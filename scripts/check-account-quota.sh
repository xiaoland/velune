#!/bin/bash
set -euo pipefail
cd "$(dirname "$0")/.."
build="$PWD/.build/apple/Products/Release"
library="$PWD/target/macos/Velune.app/Contents/Frameworks"
[[ -f "$build/VeluneBindings_Module.o" && -f "$library/libvelune_bindings.dylib" ]] || {
  echo '先运行 bash scripts/build-macos.sh --build-only 生成匹配的 Swift 绑定与动态库。' >&2
  exit 1
}
work=$(mktemp -d)
trap 'rm -rf "$work"' EXIT
xcrun swiftc -parse-as-library -whole-module-optimization -warnings-as-errors \
  app/mac/Models.swift app/mac/TokenCount.swift app/mac/ComposerQuota.swift scripts/check-account-quota.swift \
  "$build/VeluneBindings_Module.o" -I "$build" -I target/swift-ffi \
  -L "$library" -lvelune_bindings -Xlinker -rpath -Xlinker "$library" -o "$work/check"
"$work/check"
