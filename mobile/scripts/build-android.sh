#!/usr/bin/env bash
# build-android.sh — wrapper around the repo-root scripts/build-android.sh that
# detects whether mobile/android/key.properties exists and warns loudly when
# falling back to debug-signing.
#
# Usage:
#   bash mobile/scripts/build-android.sh                # release apk
#   bash mobile/scripts/build-android.sh --debug        # debug apk
#
# For Play Store .aab uploads, use mobile/scripts/build-android-bundle.sh.

set -euo pipefail

SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
MOBILE_DIR="$(cd "$SCRIPT_DIR/.." && pwd)"
REPO_ROOT="$(cd "$MOBILE_DIR/.." && pwd)"
KEY_PROPERTIES="$MOBILE_DIR/android/key.properties"

if [[ -f "$KEY_PROPERTIES" ]]; then
    echo "[signing] Building with PRODUCTION keystore (mobile/android/key.properties found)."
else
    echo "[signing] WARNING: building with DEBUG keystore — for pilot/internal use only."
    echo "[signing]          Run mobile/scripts/generate-release-keystore.sh + populate"
    echo "[signing]          mobile/android/key.properties for a real production build."
fi

exec bash "$REPO_ROOT/scripts/build-android.sh" "$@"
