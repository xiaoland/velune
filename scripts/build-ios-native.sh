#!/bin/bash
set -euo pipefail
cd "$(dirname "$0")/.."
case "${PLATFORM_NAME:-${1:-iphonesimulator}}" in
  iphonesimulator) rust_target=aarch64-apple-ios-sim ;;
  iphoneos) rust_target=aarch64-apple-ios ;;
  *) echo '用法：scripts/build-ios-native.sh [iphonesimulator|iphoneos]' >&2; exit 1 ;;
esac
export IPHONEOS_DEPLOYMENT_TARGET=18.0
profile=debug
cargo_profile=--profile=dev
if [[ "${CONFIGURATION:-Debug}" == Release ]]; then profile=release; cargo_profile=--release; fi
cargo rustc --locked --target-dir target/ios-rust -p velune-bindings --lib \
  --target "$rust_target" --no-default-features --features remote "$cargo_profile" --crate-type staticlib
output="$PWD/target/ios-generated"
mkdir -p "$output"
cargo run --locked --target-dir target/ios-bindgen -p velune-bindings --no-default-features --features cli \
  --bin velune-bindgen -- generate --library "target/ios-rust/$rust_target/$profile/libvelune_bindings.a" \
  --language swift --out-dir "$output" --no-format
cp "$output/VeluneBindingsFFI.modulemap" "$output/module.modulemap"
python3 -I scripts/android-licenses.py remote "$output/rust-licenses.txt" "$rust_target"
cat app/ios/Licenses/MarkdownUI-LICENSE.txt app/ios/Licenses/NetworkImage-LICENSE.txt app/mac/Licenses/swift-cmark-COPYING.txt > "$output/swift-licenses.txt"
