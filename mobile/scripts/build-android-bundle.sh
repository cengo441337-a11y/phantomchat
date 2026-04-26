#!/usr/bin/env bash
# build-android-bundle.sh — produce an Android App Bundle (.aab) signed with
# the production upload-keystore, suitable for Play Store upload.
#
# Output: mobile/build/app/outputs/bundle/release/app-release.aab
#
# Prereqs:
#   * mobile/scripts/generate-release-keystore.sh has been run once.
#   * mobile/android/key.properties exists and points at the keystore.
#   * Rust + cargo-ndk + flutter_rust_bridge_codegen + Flutter SDK + Android NDK installed.

set -euo pipefail

SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
MOBILE_DIR="$(cd "$SCRIPT_DIR/.." && pwd)"
REPO_ROOT="$(cd "$MOBILE_DIR/.." && pwd)"
KEY_PROPERTIES="$MOBILE_DIR/android/key.properties"

ANDROID_NDK_HOME="${ANDROID_NDK_HOME:-$HOME/Android/Sdk/ndk/28.2.13676358}"
export ANDROID_NDK_HOME

if [[ ! -f "$KEY_PROPERTIES" ]]; then
    echo "ERROR: mobile/android/key.properties is missing." >&2
    echo "       Play Store .aab uploads MUST be signed with the production keystore." >&2
    echo "       Run: bash mobile/scripts/generate-release-keystore.sh" >&2
    echo "       Then copy mobile/android/key.properties.template -> key.properties and fill in the password." >&2
    exit 1
fi

ABIS=("arm64-v8a" "armeabi-v7a" "x86_64")  # Play Store wants all relevant ABIs

echo "[1/4] Regenerating flutter_rust_bridge bindings…"
(cd "$MOBILE_DIR" && flutter_rust_bridge_codegen generate)

echo "[2/4] Cross-compiling Rust core for Android (${ABIS[*]})…"
cd "$REPO_ROOT/core"
ndk_args=()
for abi in "${ABIS[@]}"; do ndk_args+=("-t" "$abi"); done
cargo ndk "${ndk_args[@]}" \
    -o "$MOBILE_DIR/android/app/src/main/jniLibs" \
    build --release \
    --no-default-features --features ffi-mobile
for abi in "${ABIS[@]}"; do
    find "$MOBILE_DIR/android/app/src/main/jniLibs/$abi" \
         -name 'libif_watch-*.so' -delete 2>/dev/null || true
done

echo "[3/4] flutter pub get…"
(cd "$MOBILE_DIR" && flutter pub get)

echo "[4/4] Building Android App Bundle (.aab) signed with production keystore…"
cd "$MOBILE_DIR"
flutter build appbundle --release

echo
echo "Done. App Bundle:"
ls -lh "$MOBILE_DIR/build/app/outputs/bundle/release/"*.aab
echo
echo "Upload this .aab to the Google Play Console (Internal testing -> Production tracks)."
