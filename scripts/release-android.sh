#!/usr/bin/env bash
# release-android.sh — wrapper around mobile/scripts/build-android.sh +
# scripts/publish-android-update-manifest.sh that produces signed
# release APKs for all 3 ABIs and uploads them to Hostinger, then
# refreshes the auto-update manifest so existing installs see the bump.
#
# Usage:
#   scripts/release-android.sh [--notes "release notes blob"]
#
# Steps:
#   1. bash mobile/scripts/build-android.sh           # release apk, all 3 ABIs
#   2. for each ABI: scp app-<abi>-release.apk hostinger:/var/www/updates/download/
#   3. bash scripts/publish-android-update-manifest.sh --notes "..."
#
# Prereqs:
#   - SSH alias `hostinger` configured.
#   - mobile/android/key.properties present (production keystore) — otherwise
#     the wrapper falls back to debug-signing, which is not suitable for
#     publishing.

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
MOBILE_DIR="$REPO_ROOT/mobile"
APK_DIR="$MOBILE_DIR/build/app/outputs/flutter-apk"
HOSTINGER_DOWNLOAD_DIR="${HOSTINGER_DOWNLOAD_DIR:-/var/www/updates/download}"

echo "[1/3] Building release APKs for all 3 ABIs…"
bash "$MOBILE_DIR/scripts/build-android.sh"

ABIS=("arm64-v8a" "armeabi-v7a" "x86_64")

echo "[2/3] Uploading APKs to hostinger:$HOSTINGER_DOWNLOAD_DIR …"
for abi in "${ABIS[@]}"; do
    apk="$APK_DIR/app-${abi}-release.apk"
    if [[ ! -f "$apk" ]]; then
        echo "ERROR: expected APK not found: $apk" >&2
        exit 1
    fi
    echo "  - $apk"
    scp "$apk" "hostinger:/tmp/app-${abi}-release.apk"
    ssh hostinger sudo install -m 0644 \
        "/tmp/app-${abi}-release.apk" \
        "$HOSTINGER_DOWNLOAD_DIR/app-${abi}-release.apk"
    ssh hostinger rm -f "/tmp/app-${abi}-release.apk"
done

echo "[3/3] Publishing auto-update manifest…"
if [[ -n "$NOTES" ]]; then
    bash "$REPO_ROOT/scripts/publish-android-update-manifest.sh" --notes "$NOTES"
else
    bash "$REPO_ROOT/scripts/publish-android-update-manifest.sh"
fi

echo
echo "Done. Android release published to hostinger."
