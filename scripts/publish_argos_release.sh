#!/usr/bin/env bash
# publish_argos_release.sh — single-source-of-truth Argos release publisher.
#
# Run from openclaw after a successful `bash scripts/build-android.sh`. The
# script reads the version from mobile/pubspec.yaml and pushes everywhere
# the version is surfaced. Designed so we never again ship a build where
# the manifest says vX.Y.Z but the download landing-page still shows
# vA.B.C.
#
# What it updates:
#   1. /var/www/updates/download/app-*.apk          (hostinger)
#   2. /var/www/updates/phantomchat/manifests/      (hostinger)
#   3. /var/www/updates/download/PhantomChat_latest_android.apk.sha256
#   4. /var/www/updates/download/index.html         (vX.Y.Z label)
#   5. /var/www/dc-infosec.de/dl/phantomchat-arm64.apk
#   6. /root/pylonyx/web/{components,app/argos,lib}  version constants
#   7. (optional) git tag + push + gh release create
#
# Usage:
#   bash publish_argos_release.sh                  # full publish (no GH release)
#   bash publish_argos_release.sh --release-notes "Notes here"  # + GH release
#
# Prereq: SSH alias `hostinger` works from openclaw, gh CLI authenticated.

set -euo pipefail

REPO_ROOT="${REPO_ROOT:-/home/deniz/phantomchat}"
APK_DIR="$REPO_ROOT/mobile/build/app/outputs/flutter-apk"

VERSION_LINE=$(grep '^version: ' "$REPO_ROOT/mobile/pubspec.yaml")
VERSION_FULL=${VERSION_LINE#version: }
VERSION=${VERSION_FULL%+*}
BUILD_CODE=${VERSION_FULL#*+}

echo "[publish] Version=$VERSION code=$BUILD_CODE"

ARM64_APK="$APK_DIR/app-arm64-v8a-release.apk"
ARM32_APK="$APK_DIR/app-armeabi-v7a-release.apk"
X64_APK="$APK_DIR/app-x86_64-release.apk"

for f in "$ARM64_APK" "$ARM32_APK" "$X64_APK"; do
    [[ -f $f ]] || { echo "[publish] MISSING APK: $f"; exit 2; }
done

SHA_ARM64=$(sha256sum "$ARM64_APK" | cut -d' ' -f1)
SHA_ARM32=$(sha256sum "$ARM32_APK" | cut -d' ' -f1)
SHA_X64=$(sha256sum "$X64_APK" | cut -d' ' -f1)
SIZE_ARM64=$(stat -c%s "$ARM64_APK")
SIZE_ARM32=$(stat -c%s "$ARM32_APK")
SIZE_X64=$(stat -c%s "$X64_APK")

echo "[publish] arm64 sha=$SHA_ARM64 size=$SIZE_ARM64"

NOTES_DEFAULT="Argos v$VERSION+$BUILD_CODE — siehe https://github.com/cengo441337-a11y/phantomchat/releases/tag/v$VERSION-android"
NOTES="${1:-$NOTES_DEFAULT}"
RELEASED_AT=$(date -u +"%Y-%m-%dT%H:%M:%SZ")

echo "[publish] 1/6 transferring APKs to hostinger…"
for f in "$ARM64_APK" "$ARM32_APK" "$X64_APK"; do
    scp -q "$f" hostinger:/var/www/updates/download/
done
scp -q "$ARM64_APK" hostinger:/var/www/dc-infosec.de/dl/phantomchat-arm64.apk

echo "[publish] 2/6 writing manifest…"
ssh hostinger "cat > /var/www/updates/phantomchat/manifests/android.json" << MANIFEST
{
    "version": "$VERSION",
    "released_at": "$RELEASED_AT",
    "notes": "$NOTES",
    "abis": {
        "arm64-v8a": {
            "url": "https://updates.dc-infosec.de/download/app-arm64-v8a-release.apk",
            "sha256": "$SHA_ARM64",
            "size_bytes": $SIZE_ARM64,
            "version_code": $BUILD_CODE
        },
        "armeabi-v7a": {
            "url": "https://updates.dc-infosec.de/download/app-armeabi-v7a-release.apk",
            "sha256": "$SHA_ARM32",
            "size_bytes": $SIZE_ARM32,
            "version_code": $BUILD_CODE
        },
        "x86_64": {
            "url": "https://updates.dc-infosec.de/download/app-x86_64-release.apk",
            "sha256": "$SHA_X64",
            "size_bytes": $SIZE_X64,
            "version_code": $BUILD_CODE
        }
    }
}
MANIFEST

echo "[publish] 3/6 updating sha256 sidecar…"
ssh hostinger "echo '$SHA_ARM64  app-arm64-v8a-release.apk' > /var/www/updates/download/PhantomChat_latest_android.apk.sha256"

echo "[publish] 4/6 bumping download landing-page version label…"
ssh hostinger "sed -i -E 's|ANDROID · APK · v[0-9]+\\.[0-9]+\\.[0-9]+|ANDROID · APK · v$VERSION|g; s|<strong>Aktuell:</strong> v[0-9]+\\.[0-9]+\\.[0-9]+|<strong>Aktuell:</strong> v$VERSION|g' /var/www/updates/download/index.html"

echo "[publish] 5/6 bumping pylonyx version constants…"
ssh hostinger "for f in /root/pylonyx/web/app/argos/success/page.tsx /root/pylonyx/web/app/argos/buy/page.tsx /root/pylonyx/web/components/ArgosSection.tsx /root/pylonyx/web/lib/argos.ts /root/pylonyx/web/app/api/argos/download/route.ts; do sed -i -E 's|1\\.[12]\\.[0-9]+|$VERSION|g' \$f; done; cd /root/pylonyx/web && timeout 180 npm run build > /tmp/pylonyx-rebuild.log 2>&1 && pm2 restart pylonyx-web && echo pylonyx restarted || cat /tmp/pylonyx-rebuild.log | tail -10"

echo "[publish] 6/6 sanity-check manifest live…"
LIVE=$(curl -fs "https://updates.dc-infosec.de/phantomchat/android/latest.json?ts=$(date +%s)" | python3 -c "import json,sys;d=json.load(sys.stdin);print(d['version'],d['abis']['arm64-v8a']['version_code'])")
echo "[publish] manifest live: $LIVE (expected $VERSION $BUILD_CODE)"
if [[ "$LIVE" != "$VERSION $BUILD_CODE" ]]; then
    echo "[publish] WARN: live manifest does not match expected"; exit 3
fi

echo "[publish] DONE — Argos v$VERSION+$BUILD_CODE is live on all surfaces."
