#!/usr/bin/env bash
# Build the Android-native phantomchat_core lib + an APK.
# Prereqs: Rust + cargo-ndk + flutter_rust_bridge_codegen 2.12.0 + Flutter SDK + Android NDK.
set -euo pipefail

REPO_ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
ANDROID_NDK_HOME="${ANDROID_NDK_HOME:-$HOME/Android/Sdk/ndk/28.2.13676358}"
export ANDROID_NDK_HOME

ABIS=("arm64-v8a")  # add "armeabi-v7a" "x86_64" once you need them
MODE="release"
[[ "${1:-}" == "--debug" ]] && MODE="debug"

echo "[1/4] Regenerating flutter_rust_bridge bindings…"
(cd "$REPO_ROOT/mobile" && flutter_rust_bridge_codegen generate)

echo "[2/4] Cross-compiling Rust core for Android (${ABIS[*]})…"
cd "$REPO_ROOT/core"
ndk_args=()
for abi in "${ABIS[@]}"; do ndk_args+=("-t" "$abi"); done
cargo ndk "${ndk_args[@]}" \
    -o "$REPO_ROOT/mobile/android/app/src/main/jniLibs" \
    build "$([ "$MODE" = release ] && echo --release)" \
    --no-default-features --features ffi-mobile
# Strip incidental transitive .so files cargo-ndk may leave behind
for abi in "${ABIS[@]}"; do
    find "$REPO_ROOT/mobile/android/app/src/main/jniLibs/$abi" \
         -name 'libif_watch-*.so' -delete 2>/dev/null || true
done

echo "[3/4] flutter pub get…"
(cd "$REPO_ROOT/mobile" && flutter pub get)

echo "[4/4] Building APK ($MODE)…"
cd "$REPO_ROOT/mobile"
if [ "$MODE" = release ]; then
    flutter build apk --release --target-platform android-arm64 --split-per-abi
else
    flutter build apk --debug --target-platform android-arm64
fi

echo
echo "Done. APK(s):"
ls -lh "$REPO_ROOT/mobile/build/app/outputs/flutter-apk/"*.apk
