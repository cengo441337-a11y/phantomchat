#!/usr/bin/env bash
# Wave 11G — publish the in-app auto-update manifest for the Android client.
#
# Usage:
#   scripts/publish-android-update-manifest.sh [--notes "release notes blob"]
#
# Steps:
#   1. Read mobile/pubspec.yaml -> version (strip "+build" metadata).
#   2. SSH to Hostinger (alias `ssh hostinger`) and sha256sum every
#      app-<abi>-release.apk that's currently published in
#      /var/www/updates/download/.
#   3. Stat each APK to capture size_bytes (matches what curl will see).
#   4. Build the manifest JSON locally.
#   5. Atomically write it to
#      /var/www/updates/phantomchat/android/latest.json on Hostinger.
#
# Trust model: the manifest URL is HTTPS-served by the same host that
# serves the APKs, so an attacker who can MITM the download URL would
# have to MITM the manifest too — and the SHA256 in the manifest
# protects the APK download against any out-of-band swap.

set -euo pipefail

NOTES=""
while [[ $# -gt 0 ]]; do
    case "$1" in
        --notes)
            NOTES="$2"
            shift 2
            ;;
        *)
            echo "unknown arg: $1" >&2
            exit 2
            ;;
    esac
done

REPO_ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
PUBSPEC="$REPO_ROOT/mobile/pubspec.yaml"

if [[ ! -f "$PUBSPEC" ]]; then
    echo "error: $PUBSPEC not found" >&2
    exit 1
fi

# Extract `version: X.Y.Z[+build]`. The Flutter convention is
# `<semver>+<buildNumber>` and Gradle wires `+N` straight into Android's
# `versionCode`. The mobile client's UpdateService.checkForUpdate REQUIRES
# `version_code` per ABI in the manifest — without it AbiVariant.fromJson
# throws and the banner silently drops (caught at line 184). 1.0.4 through
# 1.1.1 shipped without this field; banner never showed for any user.
VERSION_RAW=$(grep -E '^version:' "$PUBSPEC" | head -n1 | awk '{print $2}')
VERSION="${VERSION_RAW%%+*}"
VERSION_CODE="${VERSION_RAW##*+}"
if [[ -z "$VERSION" || -z "$VERSION_CODE" || "$VERSION" == "$VERSION_CODE" ]]; then
    echo "error: could not parse 'X.Y.Z+N' version from $PUBSPEC (got '$VERSION_RAW')" >&2
    exit 1
fi

echo "[manifest] version = $VERSION (build = $VERSION_CODE)"

REMOTE_DIR="/var/www/updates/download"
# nginx config for updates.dc-infosec.de rewrites
# `/phantomchat/<target>/<version>` → `/phantomchat/manifests/<target>.json`
# so the canonical on-disk location of the Android manifest is
# `/var/www/updates/phantomchat/manifests/android.json` regardless of what
# the mobile client pretends the URL is. The user-facing URL is then
# `https://updates.dc-infosec.de/phantomchat/android/latest.json`
# (the trailing "latest" is consumed by the nginx regex).
REMOTE_MANIFEST_DIR="/var/www/updates/phantomchat/manifests"
REMOTE_MANIFEST_PATH="$REMOTE_MANIFEST_DIR/android.json"

echo "[manifest] fetching APK metadata from hostinger:$REMOTE_DIR …"

# One SSH round-trip: emit "<abi> <sha256> <size>" lines for each
# released APK. We grep for the canonical filenames so any extra files
# in the dir (signing intermediates, debug builds) are ignored.
APK_META=$(ssh hostinger bash <<'REMOTE'
set -euo pipefail
cd /var/www/updates/download
for abi in arm64-v8a armeabi-v7a x86_64; do
    f="app-${abi}-release.apk"
    if [[ -f "$f" ]]; then
        sha=$(sha256sum "$f" | awk '{print $1}')
        size=$(stat -c %s "$f")
        echo "$abi $sha $size"
    fi
done
REMOTE
)

if [[ -z "$APK_META" ]]; then
    echo "error: no app-*-release.apk files found in $REMOTE_DIR" >&2
    exit 1
fi

# Build the abis JSON object. Iterate the lines and produce
# "<abi>": { "url": …, "sha256": …, "size_bytes": … } entries joined by
# commas.
RELEASED_AT=$(date -u +"%Y-%m-%dT%H:%M:%SZ")

ABIS_JSON=""
while read -r abi sha size; do
    [[ -z "$abi" ]] && continue
    url="https://updates.dc-infosec.de/download/app-${abi}-release.apk"
    # `version_code` is REQUIRED by mobile/lib/services/update_service.dart's
    # AbiVariant.fromJson — omitting it makes the in-app update-banner
    # silently drop the manifest. We use the same VERSION_CODE for every
    # ABI because Gradle bakes the same +N from pubspec into all three
    # APK variants in build-android.sh.
    entry=$(printf '"%s":{"url":"%s","sha256":"%s","size_bytes":%s,"version_code":%s}' \
        "$abi" "$url" "$sha" "$size" "$VERSION_CODE")
    if [[ -z "$ABIS_JSON" ]]; then
        ABIS_JSON="$entry"
    else
        ABIS_JSON="$ABIS_JSON,$entry"
    fi
done <<< "$APK_META"

# Escape NOTES for embedding in JSON. We use python3 because anything
# else (sed/awk) would mishandle quotes / newlines in release notes.
ESCAPED_NOTES=$(python3 -c 'import json,sys; print(json.dumps(sys.argv[1]))' "$NOTES")

MANIFEST=$(printf '{"version":"%s","released_at":"%s","notes":%s,"abis":{%s}}\n' \
    "$VERSION" "$RELEASED_AT" "$ESCAPED_NOTES" "$ABIS_JSON")

echo "[manifest] generated:"
echo "$MANIFEST" | python3 -m json.tool

# Pretty-print on the remote too so devs can curl it and read it.
PRETTY=$(echo "$MANIFEST" | python3 -m json.tool)

echo "[manifest] uploading to hostinger:$REMOTE_MANIFEST_PATH …"
ssh hostinger bash -s <<REMOTE
set -euo pipefail
mkdir -p "$REMOTE_MANIFEST_DIR"
tmp=\$(mktemp)
cat > "\$tmp" <<'EOF'
$PRETTY
EOF
mv "\$tmp" "$REMOTE_MANIFEST_PATH"
chmod 0644 "$REMOTE_MANIFEST_PATH"
echo "[manifest] published $REMOTE_MANIFEST_PATH"

# Friendly-named stable-URL symlinks. The canonical APK filenames
# (\`app-<abi>-release.apk\`) are Flutter-codegen and unreadable for
# users sharing a download link. The symlinks below give every
# version a clickable URL that doesn't 404 across deploys AND that
# reads as "obviously the Android build of PhantomChat":
#
#   PhantomChat_latest_android.apk            → arm64-v8a (modern phones)
#   PhantomChat_latest_android_arm64.apk      → arm64-v8a (explicit)
#   PhantomChat_latest_android_arm32.apk      → armeabi-v7a (legacy)
#   PhantomChat_latest_android_x86_64.apk     → x86_64 (emulators)
#
# APK filenames are constant per-ABI (no version suffix) so the
# symlinks survive every deploy without re-pointing — but ln -sfn
# is idempotent, doesn't hurt to refresh.
cd /var/www/updates/download
sudo ln -sfn app-arm64-v8a-release.apk         PhantomChat_latest_android.apk
sudo ln -sfn app-arm64-v8a-release.apk.sha256  PhantomChat_latest_android.apk.sha256
sudo ln -sfn app-arm64-v8a-release.apk         PhantomChat_latest_android_arm64.apk
sudo ln -sfn app-armeabi-v7a-release.apk       PhantomChat_latest_android_arm32.apk
sudo ln -sfn app-x86_64-release.apk            PhantomChat_latest_android_x86_64.apk
echo "[manifest] friendly symlinks refreshed"
REMOTE

echo "[manifest] done. URL: https://updates.dc-infosec.de/phantomchat/android/latest.json"
echo "[manifest] friendly direct-download:"
echo "    https://updates.dc-infosec.de/download/PhantomChat_latest_android.apk"
