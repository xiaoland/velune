#!/bin/bash
set -euo pipefail
cd "$(dirname "$0")/.."
[[ $# -eq 0 ]] || { echo '用法：scripts/build-android-native.sh' >&2; exit 1; }

sdk="${ANDROID_SDK_ROOT:-${ANDROID_HOME:-}}"
if [[ -z "$sdk" ]]; then
  case "$(uname -s)" in
    Darwin) sdk="$HOME/Library/Android/sdk" ;;
    Linux) sdk="$HOME/Android/Sdk" ;;
    *) echo '需要 macOS 或 Linux 上的 Android NDK。' >&2; exit 1 ;;
  esac
fi
ndk="${ANDROID_NDK_HOME:-${ANDROID_NDK_ROOT:-$sdk/ndk/${ANDROID_NDK_VERSION:-30.0.14904198}}}"
case "$(uname -s)" in
  Darwin) host_tag=darwin-x86_64 ;;
  Linux) host_tag=linux-x86_64 ;;
  *) echo '需要 macOS 或 Linux 上的 Android NDK。' >&2; exit 1 ;;
esac
toolchain="$ndk/toolchains/llvm/prebuilt/$host_tag/bin"
compiler="$toolchain/aarch64-linux-android28-clang"
[[ -x "$compiler" ]] || { echo "找不到 API 28 的 arm64 NDK 编译器：$compiler" >&2; exit 1; }
rustup target list --installed | grep -Fxq aarch64-linux-android || {
  echo '请先运行 rustup target add aarch64-linux-android。' >&2
  exit 1
}

export CC_aarch64_linux_android="$compiler"
export AR_aarch64_linux_android="$toolchain/llvm-ar"
export CARGO_TARGET_AARCH64_LINUX_ANDROID_LINKER="$compiler"
cargo build --locked --release --target-dir target -p velune-bindings --lib \
  --target aarch64-linux-android --no-default-features --features remote

bindings="$PWD/target/android-bindings"
native="$PWD/target/android-native/arm64-v8a"
mkdir -p "$bindings" "$native"
cargo run --locked --target-dir target -p velune-bindings --no-default-features --features cli \
  --bin velune-bindgen -- generate \
  --library target/aarch64-linux-android/release/libvelune_bindings.so \
  --language kotlin --out-dir "$bindings" --no-format
cp target/aarch64-linux-android/release/libvelune_bindings.so "$native/"
"$toolchain/llvm-strip" --strip-unneeded "$native/libvelune_bindings.so"

# The Kotlin verifier must match Cargo.lock; its official Maven artifact is hash-pinned.
verifier_version=$(awk '/name = "rustls-platform-verifier-android"/ { getline; gsub(/"/, "", $3); print $3 }' Cargo.lock)
[[ "$verifier_version" == 0.2.0 ]] || {
  echo 'rustls Android 组件版本已变化，请同步构建脚本中的 AAR 与 SHA256。' >&2
  exit 1
}
verifier="$PWD/target/android-native/rustls-platform-verifier-$verifier_version.aar"
verifier_sha256=aa021794230fbc2f0be355999e2cf67398dd563de066a2e17001ffbd0b69101b
if [[ ! -f "$verifier" ]]; then
  curl --fail --location --silent --show-error \
    "https://github.com/rustls/rustls-platform-verifier/raw/maven-archive/android-release-support/maven/org/rustls/rustls-platform-verifier/$verifier_version/rustls-platform-verifier-$verifier_version.aar" \
    --output "$verifier.download"
  printf '%s  %s\n' "$verifier_sha256" "$verifier.download" | shasum -a 256 --check --status
  mv "$verifier.download" "$verifier"
fi
printf '%s  %s\n' "$verifier_sha256" "$verifier" | shasum -a 256 --check --status
printf '已构建：%s\nKotlin 绑定：%s\n' "$native/libvelune_bindings.so" "$bindings"
