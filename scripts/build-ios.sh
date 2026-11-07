#!/bin/bash
set -euo pipefail
cd "$(dirname "$0")/.."
sdk=iphonesimulator
profile=dev
xcode_arguments=()
while [[ $# -gt 0 ]]; do
  case "$1" in
    iphonesimulator|iphoneos) sdk="$1"; shift ;;
    --profile) [[ $# -ge 2 ]] || { echo '--profile 需要 dev 或 release。' >&2; exit 1; }; profile="$2"; shift 2 ;;
    --) shift; xcode_arguments+=("$@"); break ;;
    -configuration|CONFIGURATION=*) echo '请用 --profile dev|release 选择一致的 App／Rust 配置。' >&2; exit 1 ;;
    *) xcode_arguments+=("$1"); shift ;;
  esac
done
# A pass-through cannot change only Xcode's side of the shared profile.
for argument in ${xcode_arguments[@]+"${xcode_arguments[@]}"}; do
  case "$argument" in
    -configuration|CONFIGURATION=*) echo '请用 --profile dev|release 选择一致的 App／Rust 配置。' >&2; exit 1 ;;
  esac
done
case "$profile" in
  dev) configuration=Debug ;;
  release) configuration=Release ;;
  *) echo '用法：scripts/build-ios.sh [iphonesimulator|iphoneos] [--profile dev|release] [xcodebuild 参数]' >&2; exit 1 ;;
esac
if [[ "${VELUNE_BUILD_MANAGED:-}" != ios ]]; then
  exec python3 scripts/build-support.py run --platform ios --profile "$profile" -- bash "$0" "$sdk" --profile "$profile" -- ${xcode_arguments[@]+"${xcode_arguments[@]}"}
fi
case "$sdk" in
  iphonesimulator) destination='generic/platform=iOS Simulator'; signing=CODE_SIGNING_ALLOWED=NO ;;
  iphoneos) destination='generic/platform=iOS'; signing=CODE_SIGN_STYLE=Automatic ;;
esac
if [[ -n "${VELUNE_IOS_DEVICE_ID:-}" && "$sdk" == iphoneos ]]; then
  destination="id=$VELUNE_IOS_DEVICE_ID"
fi
# The declared Xcode phase is the sole native producer. Its output paths make
# generated Swift and licence files available even on the first build.
xcodebuild -project app/ios/Velune.xcodeproj -scheme Velune -configuration "$configuration" \
  -sdk "$sdk" -destination "$destination" -derivedDataPath "${VELUNE_IOS_DERIVED_DATA:-target/ios}" \
  -jobs "${VELUNE_BUILD_JOBS:-2}" "$signing" ${xcode_arguments[@]+"${xcode_arguments[@]}"} build
