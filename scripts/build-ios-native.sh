#!/bin/bash
set -euo pipefail
cd "$(dirname "$0")/.."
sdk="${PLATFORM_NAME:-iphonesimulator}"
profile=dev
[[ "${CONFIGURATION:-Debug}" != Release ]] || profile=release
while [[ $# -gt 0 ]]; do
  case "$1" in
    iphonesimulator|iphoneos) sdk="$1"; shift ;;
    --profile) [[ $# -ge 2 ]] || { echo '--profile 需要 dev 或 release。' >&2; exit 1; }; profile="$2"; shift 2 ;;
    *) echo '用法：scripts/build-ios-native.sh [iphonesimulator|iphoneos] [--profile dev|release]' >&2; exit 1 ;;
  esac
done
case "$sdk" in
  iphonesimulator) rust_target=aarch64-apple-ios-sim ;;
  iphoneos) rust_target=aarch64-apple-ios ;;
  *) echo 'iOS SDK 必须为 iphonesimulator 或 iphoneos。' >&2; exit 1 ;;
esac
case "$profile" in
  dev) output_profile=debug ;;
  release) output_profile=release ;;
  *) echo '--profile 需要 dev 或 release。' >&2; exit 1 ;;
esac
if [[ "${VELUNE_BUILD_MANAGED:-}" != ios ]]; then
  exec python3 scripts/build-support.py run --platform ios --profile "$profile" -- bash "$0" "$sdk" --profile "$profile"
fi
IFS=$'\t' read -r product_version version_core build_number < <(python3 scripts/version.py --format build-info)
output="$PWD/target/ios-generated"
mkdir -p "$output"
python3 - "$output/Info.plist" "$product_version" "$version_core" "$build_number" <<'PY' | python3 scripts/build-support.py write "$output/Info.plist"
import plistlib,sys
from pathlib import Path
version,core,build_number=sys.argv[2:]
plist=plistlib.loads(Path('app/ios/Info.plist').read_bytes())
plist['CFBundleShortVersionString']=core
plist['CFBundleVersion']=build_number
plist['VeluneDisplayVersion']=version
sys.stdout.buffer.write(plistlib.dumps(plist))
PY
export IPHONEOS_DEPLOYMENT_TARGET=18.0
cargo rustc --locked --target-dir target/cross-rust -p velune-bindings --lib \
  --target "$rust_target" --no-default-features --features remote --profile "$profile" \
  -j "${VELUNE_BUILD_JOBS:-2}" --crate-type staticlib
python3 scripts/build-support.py bindgen --library "target/cross-rust/$rust_target/$output_profile/libvelune_bindings.a" \
  --language swift --out-dir "$output"
# Each contract crate generates a Swift file and header. One module owns all
# headers; individual generator modulemaps remain unreferenced in this folder.
{
  echo 'module VeluneBindingsFFI {'
  for header in "$output"/*.h; do echo "    header \"$(basename "$header")\""; done
  echo '    export *'
  echo '}'
} | python3 scripts/build-support.py write "$output/module.modulemap"
workspace=$(mktemp -d "${TMPDIR:-/tmp}/velune-ios-native.XXXXXX")
trap 'rm -rf "$workspace"' EXIT
python3 -I scripts/android-licenses.py remote "$workspace/rust-licenses.txt" "$rust_target"
python3 scripts/build-support.py copy "$workspace/rust-licenses.txt" "$output/rust-licenses.txt"
python3 scripts/build-support.py copy LICENSE "$output/project-license.txt"
{
  cat app/ios/Licenses/MarkdownUI-LICENSE.txt app/ios/Licenses/NetworkImage-LICENSE.txt app/mac/Licenses/swift-cmark-COPYING.txt
  cat app/mac/Licenses/SwiftPackages/*.txt
} | python3 scripts/build-support.py write "$output/swift-licenses.txt"
