#!/usr/bin/env bash
# Build the Android-native phantomchat_core lib + an APK.
# Prereqs: Rust + cargo-ndk + flutter_rust_bridge_codegen 2.12.0 + Flutter SDK + Android NDK.
set -euo pipefail

REPO_ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
ANDROID_NDK_HOME="${ANDROID_NDK_HOME:-$HOME/Android/Sdk/ndk/28.2.13676358}"
export ANDROID_NDK_HOME

ABIS=("arm64-v8a" "armeabi-v7a" "x86_64")
MODE="release"
[[ "${1:-}" == "--debug" ]] && MODE="debug"

echo "[1/4] Regenerating flutter_rust_bridge bindings…"
(cd "$REPO_ROOT/mobile" && flutter_rust_bridge_codegen generate)

echo "[2/4] Cross-compiling phantomchat_mobile for Android (${ABIS[*]})…"
# IMPORTANT: build the mobile/rust/ wrapper crate, NOT core/. The FRB-
# generated Dart code (mobile/lib/src/rust/frb_generated.dart) dlopens
# `libphantomchat_mobile.so` (matches the wrapper's [package].name).
# If we built core/ here, RustLib.init() throws "not initialized — did
# you forget RustLib.init?" because FRB cannot find its symbols by name
# in libphantomchat_core.so.
cd "$REPO_ROOT/mobile/rust"
ndk_args=()
for abi in "${ABIS[@]}"; do ndk_args+=("-t" "$abi"); done
cargo ndk "${ndk_args[@]}" \
    -o "$REPO_ROOT/mobile/android/app/src/main/jniLibs" \
    build "$([ "$MODE" = release ] && echo --release)"
# phantomchat_core is statically linked into libphantomchat_mobile.so
# via the wrapper crate's dep; we don't need (and shouldn't ship) a
# separate libphantomchat_core.so. Also strip transient cargo-ndk debris.
for abi in "${ABIS[@]}"; do
    find "$REPO_ROOT/mobile/android/app/src/main/jniLibs/$abi" \
         \( -name 'libif_watch-*.so' -o -name 'libphantomchat_core.so' \) \
         -delete 2>/dev/null || true
done

echo "[3/4] flutter pub get…"
(cd "$REPO_ROOT/mobile" && flutter pub get)

echo "[4/4] Building APK ($MODE)…"
cd "$REPO_ROOT/mobile"
if [ "$MODE" = release ]; then
    # --obfuscate         : symbol-rename Dart code in the AOT snapshot so
    #                       reverse-engineers cannot trivially read class /
    #                       method / variable names. Combined with the Rust
    #                       core's own LTO+strip this leaves nothing readable.
    # --split-debug-info  : the original Dart symbols are still needed to map
    #                       obfuscated stack-traces back to source. Persisted
    #                       under build/symbols/ — never shipped, but checked
    #                       in via .gitignore exception so the team can decode
    #                       crash reports collected from production devices.
    mkdir -p "$REPO_ROOT/mobile/build/symbols"
    flutter build apk --release \
        --split-per-abi \
        --obfuscate \
        --split-debug-info="$REPO_ROOT/mobile/build/symbols/"
else
    flutter build apk --debug
fi

echo
echo "Done. APK(s):"
ls -lh "$REPO_ROOT/mobile/build/app/outputs/flutter-apk/"*.apk
