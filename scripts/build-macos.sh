#!/bin/bash
set -euo pipefail
cd "$(dirname "$0")/.."
[[ $# -eq 0 || ( $# -eq 1 && "$1" == "--build-only" ) ]] || { echo '用法：scripts/build-macos.sh [--build-only]' >&2; exit 1; }
[[ "$(uname -s)" == Darwin ]] || { echo 'This script requires macOS + Xcode Command Line Tools.' >&2; exit 1; }
command -v cargo >/dev/null
xcrun --find swiftc >/dev/null
[[ -d target/pi-runtime/node_modules/@earendil-works/pi-coding-agent ]] || {
  echo 'Pi runtime is missing; run ./scripts/install-pi-runtime.sh first.' >&2
  exit 1
}
[[ -d target/runtime-support/node_modules/huihua && -f target/runtime-support/THIRD_PARTY_NOTICES.md ]] || {
  echo 'Runtime support is missing; run ./scripts/install-runtime-support.sh first.' >&2
  exit 1
}
cargo build --locked --release -p velune-bindings --lib
bindings="$PWD/target/bindings/swift"
mkdir -p "$bindings"
cargo run --locked -p velune-bindings --features cli --bin velune-bindgen -- generate --library target/release/libvelune_bindings.dylib --language swift --out-dir "$bindings" --no-format
app="$PWD/target/macos/Velune.app"
rm -rf "$app"
mkdir -p "$app/Contents/MacOS" "$app/Contents/Resources" "$app/Contents/Frameworks"
cp target/release/libvelune_bindings.dylib "$app/Contents/Frameworks/libvelune_bindings.dylib"
install_name_tool -id '@rpath/libvelune_bindings.dylib' "$app/Contents/Frameworks/libvelune_bindings.dylib"
xcrun swiftc -parse-as-library -swift-version 5 -emit-library -emit-module -module-name VeluneBindings \
  -target "$(uname -m)-apple-macosx14.0" "$bindings/VeluneBindings.swift" \
  -I "$bindings" -Xcc -fmodule-map-file="$bindings/VeluneBindingsFFI.modulemap" \
  -L "$app/Contents/Frameworks" -lvelune_bindings \
  -Xlinker -rpath -Xlinker '@loader_path' \
  -emit-module-path "$bindings/VeluneBindings.swiftmodule" -o "$app/Contents/Frameworks/libVeluneBindings.dylib"
install_name_tool -id '@rpath/libVeluneBindings.dylib' "$app/Contents/Frameworks/libVeluneBindings.dylib"
xcrun swiftc -parse-as-library -swift-version 5 -target "$(uname -m)-apple-macosx14.0" -framework AppKit -framework SwiftUI \
  app/mac/main.swift app/mac/Views.swift app/mac/Theme.swift app/mac/Logo.swift \
  app/mac/Models.swift app/mac/BindingMapping.swift app/mac/Store.swift app/mac/Transport.swift app/mac/ProviderImport.swift app/mac/ExecutableDiscovery.swift app/mac/PublicModelCatalog.swift \
  -I "$bindings" -Xcc -fmodule-map-file="$bindings/VeluneBindingsFFI.modulemap" -L "$app/Contents/Frameworks" -lVeluneBindings \
  -Xlinker -rpath -Xlinker '@executable_path/../Frameworks' -o "$app/Contents/MacOS/Velune"
# JavaScript is a sealed resource, not a nested macOS executable.
cp packages/agent-runtime/resources/pi_sessions.mjs packages/agent-runtime/resources/pi_rpc.mjs packages/agent-runtime/resources/pi_virtual_model.mjs packages/agent-runtime/resources/pi_auth.mjs packages/agent-runtime/resources/pi_provider_import.mjs "$app/Contents/Resources/"
cp -R target/pi-runtime/node_modules "$app/Contents/Resources/node_modules"
cp packages/agent-runtime/resources/huihua_sessions.mjs "$app/Contents/Resources/"
cp -R target/runtime-support/node_modules/. "$app/Contents/Resources/node_modules/"
mkdir -p "$app/Contents/Resources/ThirdParty"
cp packages/agent-runtime/runtime-support/THIRD_PARTY_NOTICES.md "$app/Contents/Resources/ThirdParty/"
cp -R packages/agent-runtime/runtime-support/licenses "$app/Contents/Resources/ThirdParty/"
cp packages/agent-runtime/runtime-support/package-lock.json "$app/Contents/Resources/ThirdParty/runtime-support-package-lock.json"
cp -R app/mac/Assets/Brand "$app/Contents/Resources/Brand"
xcrun swift scripts/render-app-icon.swift app/mac/Assets/Brand target/macos/Velune.iconset
iconutil -c icns target/macos/Velune.iconset -o "$app/Contents/Resources/Velune.icns"
cat > "$app/Contents/Info.plist" <<'PLIST'
<?xml version="1.0" encoding="UTF-8"?>
<!DOCTYPE plist PUBLIC "-//Apple//DTD PLIST 1.0//EN" "http://www.apple.com/DTDs/PropertyList-1.0.dtd">
<plist version="1.0"><dict>
<key>CFBundleExecutable</key><string>Velune</string>
<key>CFBundleIdentifier</key><string>local.velune.prototype</string>
<key>CFBundleName</key><string>Velune</string>
<key>CFBundleIconFile</key><string>Velune</string>
<key>LSMinimumSystemVersion</key><string>14.0</string>
<key>CFBundleVersion</key><string>1</string>
<key>CFBundleShortVersionString</key><string>0.1</string>
<key>NSHighResolutionCapable</key><true/>
</dict></plist>
PLIST
python3 - "$app/Contents/Resources/build-manifest.json" <<'PY'
import hashlib,json,platform,plistlib,subprocess,sys
from pathlib import Path
def cmd(*args): return subprocess.check_output(args,text=True).strip()
version=Path('VERSION').read_text().strip()
plist_path=Path(sys.argv[1]).parents[1]/'Info.plist'
plist=plistlib.loads(plist_path.read_bytes())
plist['VeluneDisplayVersion']=version
plist['CFBundleGetInfoString']=version
plist_path.write_bytes(plistlib.dumps(plist))
manifest={'source_commit':cmd('git','rev-parse','HEAD'),'dirty':bool(cmd('git','status','--porcelain')),'lock_sha256':hashlib.sha256(Path('Cargo.lock').read_bytes()).hexdigest(),'bindings_version':'0.1.0','uniffi_version':'0.32.2','ui_version':version,'simulation':False,'control_transport':'uniffi','language_contract':'typed','config_schema_version':7,'supported_modes':['conversation_projection','llm_gateway','runtime_instances','synthetic_preview'],'native_verified':False,'rust':cmd('rustc','--version'),'swift':cmd('xcrun','swiftc','--version'),'xcode':cmd('xcodebuild','-version'),'architecture':platform.machine(),'macos':platform.mac_ver()[0],'build_command':'bash scripts/build-macos.sh'}
Path(sys.argv[1]).write_text(json.dumps(manifest,ensure_ascii=False,indent=2))
PY
# Ad-hoc local debug signing only. No identity/keychain selection, certificate, or notarization.
codesign --force --sign - "$app/Contents/Frameworks/libvelune_bindings.dylib"
codesign --force --sign - "$app/Contents/Frameworks/libVeluneBindings.dylib"
codesign --force --sign - "$app"
codesign --verify --deep --strict "$app"
shasum -a 256 "$app/Contents/MacOS/Velune" "$app/Contents/Frameworks/libvelune_bindings.dylib" "$app/Contents/Frameworks/libVeluneBindings.dylib" > target/macos/binary-sha256.txt
printf '已构建：%s\n' "$app"
if [[ "${1:-}" != "--build-only" ]]; then
  python3 scripts/install-macos.py "$app"
fi
