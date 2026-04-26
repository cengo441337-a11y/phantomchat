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
echo "[2/5] downloading $SUMS_URL"
if ! curl -fsSL "$SUMS_URL" -o "$WORKDIR/SHA256SUMS.txt"; then
    echo "FAIL: could not download SHA256SUMS.txt for $TAG" >&2
    exit 2
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
