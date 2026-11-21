#!/bin/bash
set -euo pipefail
cd "$(dirname "$0")/.."
build="$PWD/.build/apple/Products/Release"
bundle=$(python3 scripts/macos_identity.py describe --purpose development --profile release --root "$PWD" \
  | python3 -c 'import json,sys; print(json.load(sys.stdin)["bundlePath"])')
library="$bundle/Contents/Frameworks"
[[ -f "$build/VeluneBindings_Module.o" && -f "$library/libvelune_bindings.dylib" ]] || {
  echo '先运行 svc run build-macos-release 生成匹配的 Swift 绑定与动态库。' >&2
  exit 1
}
work=$(mktemp -d)
trap 'rm -rf "$work"' EXIT
xcrun swiftc -parse-as-library -whole-module-optimization -package-name velune -warnings-as-errors \
  app/mac/ComposerQuota.swift scripts/check-account-quota.swift \
  "$build/VeluneBindings_Module.o" -I "$build" -I target/swift-ffi \
  -L "$library" -lvelune_bindings -Xlinker -rpath -Xlinker "$library" -o "$work/check"
"$work/check"
