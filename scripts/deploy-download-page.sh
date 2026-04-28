#!/usr/bin/env bash
# Render site/download-index.html with the current desktop + mobile
# version numbers (pulled live from the manifest endpoints) and upload
# to /var/www/updates/download/index.html on Hostinger.
#
# Called automatically by:
#   - scripts/release-windows.sh                (after each MSI deploy)
#   - scripts/publish-android-update-manifest.sh (after each APK manifest)
#
# Standalone-callable too if either source-of-truth lags:
#   bash scripts/deploy-download-page.sh
set -euo pipefail

REPO_ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
TEMPLATE="$REPO_ROOT/site/download-index.html"
[[ -f "$TEMPLATE" ]] || { echo "missing $TEMPLATE" >&2; exit 1; }

DESKTOP_VER=$(curl -sS "https://updates.dc-infosec.de/phantomchat/windows-x86_64/3.0.0" | jq -r .version)
MOBILE_VER=$(curl -sS "https://updates.dc-infosec.de/phantomchat/android/latest.json" | jq -r .version)
[[ -n "$DESKTOP_VER" && "$DESKTOP_VER" != "null" ]] || { echo "could not fetch desktop version" >&2; exit 1; }
[[ -n "$MOBILE_VER"  && "$MOBILE_VER"  != "null" ]] || { echo "could not fetch mobile version"  >&2; exit 1; }

echo "[page] rendering desktop=$DESKTOP_VER mobile=$MOBILE_VER"
TMP=$(mktemp)
sed -e "s/DESKTOP-VERSION-PLACEHOLDER/v${DESKTOP_VER}/g" \
    -e "s/MOBILE-VERSION-PLACEHOLDER/v${MOBILE_VER}/g" \
    "$TEMPLATE" > "$TMP"

scp "$TMP" "hostinger:/tmp/download-index.html" >/dev/null
ssh hostinger 'sudo install -m 0644 /tmp/download-index.html /var/www/updates/download/index.html && rm /tmp/download-index.html'
rm -f "$TMP"
echo "[page] deployed → https://updates.dc-infosec.de/download/"
