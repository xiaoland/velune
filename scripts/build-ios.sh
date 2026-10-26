#!/bin/bash
set -euo pipefail
cd "$(dirname "$0")/.."
sdk="${1:-iphonesimulator}"
if [[ $# -gt 0 ]]; then shift; fi
case "$sdk" in
  iphonesimulator) destination='generic/platform=iOS Simulator'; signing=CODE_SIGNING_ALLOWED=NO ;;
  iphoneos) destination='generic/platform=iOS'; signing=CODE_SIGN_STYLE=Automatic ;;
  *) echo '用法：scripts/build-ios.sh [iphonesimulator|iphoneos] [xcodebuild 参数]' >&2; exit 1 ;;
esac
if [[ -n "${VELUNE_IOS_DEVICE_ID:-}" && "$sdk" == iphoneos ]]; then
  destination="id=$VELUNE_IOS_DEVICE_ID"
fi
# Generate sources before Xcode plans its first build.
scripts/build-ios-native.sh "$sdk"
xcodebuild -project app/ios/Velune.xcodeproj -scheme Velune -configuration Debug \
  -sdk "$sdk" -destination "$destination" -derivedDataPath "${VELUNE_IOS_DERIVED_DATA:-target/ios}" "$signing" "$@" build
