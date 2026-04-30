#!/usr/bin/env bash
# verify-release.sh — verify a published PhantomChat release matches its
# advertised SHA-256 checksums. See docs/REPRODUCIBLE-BUILDS.md for the
# why and the trust model.
#
# Usage:
#   bash scripts/verify-release.sh v3.0.0
#   bash scripts/verify-release.sh v3.0.0 --keep   # keep work dir on exit
#
# Exit codes:
#   0  every artifact matched
#   1  one or more mismatches (artifact tampered or build diverged)
#   2  could not download checksums file or any artifact
#   3  bad CLI usage

set -euo pipefail

GH_REPO="${GH_REPO:-cengo441337-a11y/phantomchat}"
CDN_BASE="${CDN_BASE:-https://updates.dc-infosec.de/download}"

# ── arg parse ────────────────────────────────────────────────────────────
TAG="${1:-}"
KEEP=0
if [ "${2:-}" = "--keep" ]; then KEEP=1; fi

if [ -z "$TAG" ]; then
    echo "usage: $0 <tag> [--keep]" >&2
    echo "  e.g. $0 v3.0.0" >&2
    exit 3
fi

WORKDIR="$(mktemp -d -t phantomchat-verify-XXXXXX)"
cleanup() {
    if [ "$KEEP" -eq 1 ]; then
        echo "keeping work dir: $WORKDIR"
    else
        rm -rf "$WORKDIR"
    fi
}
trap cleanup EXIT

echo "[1/5] verifying PhantomChat release: $TAG"
echo "      work dir: $WORKDIR"

# ── download SHA256SUMS.txt from GitHub Release ──────────────────────────
SUMS_URL="https://github.com/${GH_REPO}/releases/download/${TAG}/SHA256SUMS.txt"
SUMS_SIG_URL="${SUMS_URL}.asc"
echo "[2/5] downloading $SUMS_URL"
if ! curl -fsSL "$SUMS_URL" -o "$WORKDIR/SHA256SUMS.txt"; then
    echo "FAIL: could not download SHA256SUMS.txt for $TAG" >&2
    exit 2
fi

# Audit 2026-04-30 (H-4): the SHA256SUMS.txt file lives on the same
# GitHub Release as the artefacts it claims to certify — same trust
# root, no air-gap. A repo-wide compromise (stolen GH token, hijacked
# release-yml workflow) lets an attacker swap both at once and the
# verify-release exit code says OK on a tampered binary. Add an
# out-of-band GPG-signature check against the project's published key
# (keys/security.asc, fingerprint 0F8DA258 1B8A1428 9F0F2FD7 EF086D82
# 9914A0E3, expires 2027-10-26).
#
# The signature file (`SHA256SUMS.txt.asc`) is OPTIONAL during the
# transition: releases that were cut before this script existed don't
# have one. We verify when present, warn when absent, and only HARD-
# FAIL when verification fails.
SIG_STATUS="missing"
if curl -fsSL "$SUMS_SIG_URL" -o "$WORKDIR/SHA256SUMS.txt.asc" 2>/dev/null; then
    if command -v gpg >/dev/null 2>&1; then
        # Auto-import the project key from keys/security.asc if it's
        # not already in the user's keyring. Path-resolves relative to
        # this script so it works whether invoked from a repo checkout
        # or a stand-alone curl|bash run.
        SCRIPT_DIR="$(cd "$(dirname "$0")" && pwd)"
        REPO_KEY_PATH="$SCRIPT_DIR/../keys/security.asc"
        if [ -r "$REPO_KEY_PATH" ]; then
            gpg --batch --quiet --import "$REPO_KEY_PATH" 2>/dev/null || true
        fi

        if gpg --batch --quiet --verify \
                "$WORKDIR/SHA256SUMS.txt.asc" \
                "$WORKDIR/SHA256SUMS.txt" 2>/dev/null; then
            SIG_STATUS="ok"
            echo "      OK   SHA256SUMS.txt.asc — GPG signature verified"
        else
            echo "FAIL: SHA256SUMS.txt.asc did NOT verify against the project key." >&2
            echo "      The artefacts may be tampered. ABORTING." >&2
            exit 1
        fi
    else
        SIG_STATUS="skipped"
        echo "      SKIP SHA256SUMS.txt.asc found, but \`gpg\` is not installed —"
        echo "           install gpg for end-to-end signature verification."
    fi
else
    echo "      WARN SHA256SUMS.txt.asc not present on the release."
    echo "           Falling back to SHA-256-only verification (same trust"
    echo "           root as the artefacts — no out-of-band check)."
fi

# Sanity: file should have at least one "<sha>  <filename>" line
if ! grep -Eq '^[0-9a-f]{64}  ' "$WORKDIR/SHA256SUMS.txt"; then
    echo "FAIL: SHA256SUMS.txt is malformed (no sha256<space><space>name lines)" >&2
    exit 2
fi

n_expected=$(grep -Ec '^[0-9a-f]{64}  ' "$WORKDIR/SHA256SUMS.txt")
echo "      $n_expected artifacts listed in SHA256SUMS.txt"

# ── download every artifact (CDN first, GH Release fallback) ────────────
echo "[3/5] downloading $n_expected artifacts"
mkdir -p "$WORKDIR/dl"
cd "$WORKDIR/dl"

n_dl_ok=0
n_dl_fail=0
while read -r _sha name; do
    [ -z "$name" ] && continue
    cdn_url="${CDN_BASE}/${name}"
    gh_url="https://github.com/${GH_REPO}/releases/download/${TAG}/${name}"
    if curl -fsSL "$cdn_url" -o "$name" 2>/dev/null; then
        echo "      OK (cdn) $name"
        n_dl_ok=$((n_dl_ok + 1))
    elif curl -fsSL "$gh_url" -o "$name" 2>/dev/null; then
        echo "      OK (gh)  $name"
        n_dl_ok=$((n_dl_ok + 1))
    else
        echo "      MISS     $name (not on CDN or GitHub Release)"
        n_dl_fail=$((n_dl_fail + 1))
    fi
done < <(awk '/^[0-9a-f]{64}  / { print $1, $2 }' "$WORKDIR/SHA256SUMS.txt")

if [ "$n_dl_ok" -eq 0 ]; then
    echo "FAIL: could not download any artifact" >&2
    exit 2
fi

# ── verify checksums ─────────────────────────────────────────────────────
echo "[4/5] verifying SHA-256 of $n_dl_ok downloaded artifacts"
if sha256sum -c "$WORKDIR/SHA256SUMS.txt" --ignore-missing; then
    sha_ok=1
else
    sha_ok=0
fi

# ── summary ──────────────────────────────────────────────────────────────
echo
echo "[5/5] summary"
echo "      tag         : $TAG"
echo "      listed      : $n_expected"
echo "      downloaded  : $n_dl_ok"
echo "      missing     : $n_dl_fail"
echo "      gpg-sig     : $SIG_STATUS"

if [ "$sha_ok" -eq 1 ] && [ "$n_dl_fail" -eq 0 ]; then
    echo "OK: all artifacts match published checksums."
    exit 0
elif [ "$sha_ok" -eq 1 ] && [ "$n_dl_fail" -gt 0 ]; then
    echo "PARTIAL: downloaded artifacts match, but $n_dl_fail were not reachable." >&2
    exit 2
else
    echo "FAIL: at least one artifact did NOT match its published SHA-256." >&2
    exit 1
fi
