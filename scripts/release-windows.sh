#!/usr/bin/env bash
# release-windows.sh — wrap the manual Nexus → Hostinger Windows release
# pipeline (documented in docs/WINDOWS-BUILD.md) into one script.
#
# Usage:
#   scripts/release-windows.sh <version>            # e.g. 3.0.2
#
# Steps:
#   1. ssh nexus → cargo tauri build (default features = stt enabled)
#   2. scp the freshly-built MSI back to /tmp on the local host
#   3. scp /tmp MSI to hostinger:/tmp
#   4. ssh hostinger → tauri signer sign → mv to /var/www/updates/download/
#   5. update manifest JSON + SHA256 on hostinger
#   6. (optional) git tag + gh release create when GH_RELEASE=1
#
# Prereqs:
#   - SSH aliases `nexus` and `hostinger` already configured.
#   - On nexus: PHANTOMCHAT_PFX_PATH / PHANTOMCHAT_SIGNTOOL configured for
#     Tauri's bundle.windows.signCommand to find the cert (see WINDOWS-BUILD.md).
#   - On hostinger: tauri-cli installed for `tauri signer sign`, plus
#     /var/www/updates/download/ writable by the SSH user.

set -euo pipefail

if [[ $# -ne 1 ]]; then
    echo "Usage: $0 <version>" >&2
    echo "Example: $0 3.0.2" >&2
    exit 2
fi

VERSION="$1"

# Audit 2026-04-30 (H-9): validate VERSION strictly. The string is
# interpolated into ssh-heredoc bodies, scp paths, manifest JSON, and
# git-tag commands further down — a malicious commit message containing
# '"', '$', '`', ';' or backtick would otherwise produce shell-injection
# on hostinger or arbitrary git/gh execution. Restrict to semver-shape
# (digit.digit.digit, optional pre-release / build).
if [[ ! "$VERSION" =~ ^[0-9]+\.[0-9]+\.[0-9]+([-+][A-Za-z0-9.]+)?$ ]]; then
    echo "FAIL: VERSION '$VERSION' is not a valid semver (expected 'N.N.N[-pre][+build])'" >&2
    exit 2
fi

MSI_NAME="PhantomChat_${VERSION}_x64_en-US.msi"
NEXUS_REPO_DIR="${NEXUS_REPO_DIR:-D:/phantomchat}"
HOSTINGER_DOWNLOAD_DIR="${HOSTINGER_DOWNLOAD_DIR:-/var/www/updates/download}"
HOSTINGER_MANIFEST_PATH="${HOSTINGER_MANIFEST_PATH:-/var/www/updates/phantomchat/manifests/windows-x86_64.json}"
LOCAL_TMP="${LOCAL_TMP:-/tmp/$MSI_NAME}"

echo "[1/6] Building MSI on nexus (cargo tauri build, default features = stt)…"
ssh nexus "cd '$NEXUS_REPO_DIR' && cargo tauri build"

NEXUS_MSI_PATH="$NEXUS_REPO_DIR/target/release/bundle/msi/$MSI_NAME"

echo "[2/6] Copying MSI back from nexus → $LOCAL_TMP …"
scp "nexus:$NEXUS_MSI_PATH" "$LOCAL_TMP"

echo "[3/6] Copying MSI from $LOCAL_TMP → hostinger:/tmp/ …"
scp "$LOCAL_TMP" "hostinger:/tmp/$MSI_NAME"

echo "[4/6] On hostinger: tauri signer sign + move to $HOSTINGER_DOWNLOAD_DIR …"
ssh hostinger bash -se <<EOF
set -euo pipefail
cd /tmp
# tauri signer sign produces a .sig sidecar next to the artefact.
npx --yes @tauri-apps/cli@2 signer sign --private-key "\$(sudo cat /root/.tauri/phantom-updater-v2.key)" --password "" \
                  "/tmp/$MSI_NAME"
sudo install -m 0644 "/tmp/$MSI_NAME"     "$HOSTINGER_DOWNLOAD_DIR/$MSI_NAME"
sudo install -m 0644 "/tmp/$MSI_NAME.sig" "$HOSTINGER_DOWNLOAD_DIR/$MSI_NAME.sig"
# Audit 2026-04-30 (L-11): generate the .sha256 sidecar that the
# stable-URL symlink (\$MSI_NAME.sha256) and download-page link target.
# Without this the symlink dangles and the public download page's
# "verify hash" link 404s.
sha256sum "/tmp/$MSI_NAME" | awk '{print \$1 "  $MSI_NAME"}' | sudo tee "$HOSTINGER_DOWNLOAD_DIR/$MSI_NAME.sha256" > /dev/null
sudo chmod 0644 "$HOSTINGER_DOWNLOAD_DIR/$MSI_NAME.sha256"
rm -f "/tmp/$MSI_NAME" "/tmp/$MSI_NAME.sig"

# Auto-prune: drop every prior PhantomChat_*.msi (+ sidecars) that
# isn't the version we just published. Per Deniz: "ich möchte das
# immer nur die neusten versionen zum download bereit stehen". The
# updater channel resolves via the manifest, not the directory
# listing, so removing old MSIs has no functional impact — but it
# keeps the public /download/ index from accumulating cruft.
cd "$HOSTINGER_DOWNLOAD_DIR"
for f in PhantomChat_*_x64_en-US.msi PhantomChat_*_x64-setup.exe; do
    [ -e "\$f" ] || continue
    # Skip the latest-* symlinks below — they're not stale, they're the
    # stable-URL we WANT to keep. Same for the version we just shipped.
    case "\$f" in
        "$MSI_NAME"|"$MSI_NAME."*) ;;
        PhantomChat_latest_*) ;;
        *) sudo rm -f "\$f" "\$f.sig" "\$f.sha256" ;;
    esac
done

# Stable-URL symlink: PhantomChat_latest_x64_en-US.msi (+ .sig + .sha256)
# always points at the version we just shipped. Closes the
# stale-bookmark / old-GH-release-page hole that 3.0.6's prune opened —
# any link pointing at "PhantomChat_latest_x64_en-US.msi" keeps
# resolving to the current MSI without manual update on the linker's
# side.
sudo ln -sfn "$MSI_NAME"          PhantomChat_latest_x64_en-US.msi
sudo ln -sfn "$MSI_NAME.sig"      PhantomChat_latest_x64_en-US.msi.sig
sudo ln -sfn "$MSI_NAME.sha256"   PhantomChat_latest_x64_en-US.msi.sha256
EOF

echo "[5/6] Updating manifest JSON + SHA256 …"
LOCAL_SHA="$(sha256sum "$LOCAL_TMP" | awk '{print $1}')"
LOCAL_SIZE="$(stat -c%s "$LOCAL_TMP")"
PUB_DATE="$(date -u +%Y-%m-%dT%H:%M:%SZ)"

ssh hostinger bash -se <<EOF
set -euo pipefail

# Audit 2026-04-30 (C-8): \`tauri signer sign\` produces a .sig file
# that contains a header line and embedded newlines. Interpolating the
# raw blob into a JSON heredoc as a string-literal value would emit
# syntactically broken JSON whenever the sig has a newline or quote
# character — Tauri's updater client then silently fails the
# signature parse. Use \`jq -Rs '.'\` to JSON-encode the entire file
# (raw-input + slurp = one JSON-string with newlines escaped as \\n,
# quotes as \\", etc.) and \`--rawfile\` so we don't have to shell-quote
# the value either.
TMP_MANIFEST="\$(mktemp)"
jq -n \\
  --arg version "$VERSION" \\
  --arg pub_date "$PUB_DATE" \\
  --arg url "https://updates.dc-infosec.de/download/$MSI_NAME" \\
  --rawfile signature "$HOSTINGER_DOWNLOAD_DIR/$MSI_NAME.sig" \\
  --arg sha256 "$LOCAL_SHA" \\
  --argjson size_bytes "$LOCAL_SIZE" \\
  '{
    version: \$version,
    pub_date: \$pub_date,
    platforms: {
      "windows-x86_64": {
        url: \$url,
        signature: \$signature,
        sha256: \$sha256,
        size_bytes: \$size_bytes
      }
    }
  }' > "\$TMP_MANIFEST"

# Sanity: the just-written manifest must round-trip through jq again.
# jq exits non-zero on malformed JSON, so a corrupt heredoc fails fast
# before the install rather than after the file is on the public path.
jq -e '.version, .platforms."windows-x86_64".signature' "\$TMP_MANIFEST" >/dev/null

sudo install -m 0644 "\$TMP_MANIFEST" "$HOSTINGER_MANIFEST_PATH"
rm -f "\$TMP_MANIFEST"
EOF

echo "[5b/6] Re-deploying /download/index.html with live version strings …"
bash "$(dirname "${BASH_SOURCE[0]}")/deploy-download-page.sh"

echo "[6/6] Optional: git tag + gh release create …"
if [[ "${GH_RELEASE:-0}" == "1" ]]; then
    git tag -a "v$VERSION" -m "Windows desktop $VERSION"
    git push origin "v$VERSION"
    gh release create "v$VERSION" "$LOCAL_TMP" \
        --title "PhantomChat v$VERSION (Windows)" \
        --notes "Windows desktop release $VERSION."
else
    echo "  (skipped — set GH_RELEASE=1 to create a GitHub release)"
fi

echo
echo "Done. Published $MSI_NAME (sha256 $LOCAL_SHA, $LOCAL_SIZE bytes)."
