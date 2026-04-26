#!/usr/bin/env bash
# PhantomChat — Android APK build helper.
#
# Cross-compiles the `phantomchat_mobile` wrapper crate to the four NDK
# targets Flutter's Gradle plugin expects (arm64, armv7, x86_64, x86),
# stages the resulting `.so` files into Android's `jniLibs/<abi>/`, then
# kicks off the Flutter app build.
#
# Prerequisites (see mobile/README.md for install pointers):
#   - Flutter ≥ 3.41
#   - Rust toolchain with `rustup target add <…>` for each ABI
#   - cargo-ndk (`cargo install cargo-ndk`)
#   - Android SDK + NDK (NDK_HOME / ANDROID_NDK_ROOT exported)
#
# Usage:
#   mobile/scripts/build-android.sh [debug|release]   # default: release
#   mobile/scripts/build-android.sh release apk       # explicit apk
#   mobile/scripts/build-android.sh release appbundle # AAB for Play Store
#
# Outputs the APK / AAB under `mobile/build/app/outputs/`.

set -euo pipefail

PROFILE="${1:-release}"
ARTIFACT="${2:-apk}"
SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
MOBILE_DIR="$(cd "${SCRIPT_DIR}/.." && pwd)"
RUST_DIR="${MOBILE_DIR}/rust"
JNILIBS_DIR="${MOBILE_DIR}/android/app/src/main/jniLibs"

if ! command -v cargo-ndk >/dev/null 2>&1; then
    echo "ERROR: cargo-ndk not installed. Run: cargo install cargo-ndk" >&2
    exit 1
fi
if ! command -v flutter >/dev/null 2>&1; then
    echo "ERROR: flutter not on PATH" >&2
    exit 1
fi
if [ -z "${ANDROID_NDK_ROOT:-}${NDK_HOME:-}" ]; then
    echo "WARN: neither ANDROID_NDK_ROOT nor NDK_HOME is set; cargo-ndk may fail to locate the NDK." >&2
fi

echo "==> staging native libs into ${JNILIBS_DIR}"
mkdir -p "${JNILIBS_DIR}"

CARGO_PROFILE_FLAG=""
if [ "${PROFILE}" = "release" ]; then
    CARGO_PROFILE_FLAG="--release"
fi

(
    cd "${RUST_DIR}"
    # cargo-ndk drops `target/<triple>/<profile>/libphantomchat_mobile.so`
    # for each ABI AND symlinks them into the requested staging dir.
    cargo ndk \
        -t arm64-v8a \
        -t armeabi-v7a \
        -t x86_64 \
        -t x86 \
        -o "${JNILIBS_DIR}" \
        build ${CARGO_PROFILE_FLAG}
)

echo "==> built native libs:"
find "${JNILIBS_DIR}" -name 'libphantomchat_mobile.so' -printf '   %p (%s bytes)\n'

echo "==> flutter build ${ARTIFACT} (${PROFILE})"
(
    cd "${MOBILE_DIR}"
    case "${ARTIFACT}" in
        apk)
            if [ "${PROFILE}" = "release" ]; then
                flutter build apk --release --split-per-abi
            else
                flutter build apk --debug
            fi
            ;;
        appbundle|aab)
            flutter build appbundle --release
            ;;
        *)
            echo "ERROR: unknown artifact '${ARTIFACT}' (expected: apk | appbundle)" >&2
            exit 1
            ;;
    esac
)

echo "==> done. artefacts under ${MOBILE_DIR}/build/app/outputs/"
