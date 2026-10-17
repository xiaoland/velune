#!/bin/bash
set -euo pipefail
cd "$(dirname "$0")/.."
[[ $# -eq 0 || ( $# -eq 1 && "$1" == "--build-only" ) ]] || { echo '用法：scripts/build-macos.sh [--build-only]' >&2; exit 1; }
[[ "$(uname -s)" == Darwin ]] || { echo 'This script requires macOS + Xcode Command Line Tools.' >&2; exit 1; }
command -v cargo >/dev/null
command -v go >/dev/null
xcrun --find swiftc >/dev/null
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
mkdir -p target/swift-ffi target/swift-bindings
cp "$bindings/VeluneBindings.swift" target/swift-bindings/
cp "$bindings/VeluneBindingsFFI.h" target/swift-ffi/
cp "$bindings/VeluneBindingsFFI.modulemap" target/swift-ffi/module.modulemap
swift build --build-system xcode --arch "$(uname -m)" --configuration release --product VeluneMac --force-resolved-versions \
  -Xlinker -L -Xlinker "$app/Contents/Frameworks" \
  -Xlinker -lvelune_bindings -Xlinker -rpath -Xlinker '@executable_path/../Frameworks'
swift_binary_directory=$(swift build --build-system xcode --arch "$(uname -m)" --configuration release --show-bin-path)
cp "$swift_binary_directory/VeluneMac" "$app/Contents/MacOS/Velune"
# JavaScript is a sealed resource, not a nested macOS executable.
cp packages/agent-runtime/resources/pi_sdk.mjs packages/agent-runtime/resources/pi_sessions.mjs packages/agent-runtime/resources/pi_rpc.mjs packages/agent-runtime/resources/pi_virtual_model.mjs packages/agent-runtime/resources/pi_auth.mjs packages/agent-runtime/resources/pi_provider_import.mjs "$app/Contents/Resources/"
cp packages/agent-runtime/resources/huihua_sessions.mjs "$app/Contents/Resources/"
mkdir -p "$app/Contents/Resources/node_modules"
cp -R target/runtime-support/node_modules/. "$app/Contents/Resources/node_modules/"
mkdir -p "$app/Contents/Resources/ThirdParty"
cp packages/agent-runtime/runtime-support/THIRD_PARTY_NOTICES.md "$app/Contents/Resources/ThirdParty/"
cp -R packages/agent-runtime/runtime-support/licenses "$app/Contents/Resources/ThirdParty/"
cp packages/agent-runtime/runtime-support/package-lock.json "$app/Contents/Resources/ThirdParty/runtime-support-package-lock.json"
mkdir -p "$app/Contents/Resources/ThirdParty/Swift"
cp app/mac/Licenses/*.txt app/mac/Licenses/NOTICE.md "$app/Contents/Resources/ThirdParty/Swift/"
cp -R app/mac/Licenses/RelayKit "$app/Contents/Resources/ThirdParty/RelayKit"
cat app/mac/Licenses/RelayKit/NOTICE.md >> "$app/Contents/Resources/ThirdParty/THIRD_PARTY_NOTICES.md"
cp packages/ai-provider/relaykit-bridge/go.mod packages/ai-provider/relaykit-bridge/go.sum "$app/Contents/Resources/ThirdParty/RelayKit/"
cp Package.resolved "$app/Contents/Resources/ThirdParty/swift-package-resolved.json"
cp -R assets/brand "$app/Contents/Resources/Brand"
xcrun swift scripts/render-app-icon.swift assets/brand target/macos/Velune.iconset
iconutil -c icns target/macos/Velune.iconset -o "$app/Contents/Resources/Velune.icns"
cat > "$app/Contents/Info.plist" <<'PLIST'
<?xml version="1.0" encoding="UTF-8"?>
<!DOCTYPE plist PUBLIC "-//Apple//DTD PLIST 1.0//EN" "http://www.apple.com/DTDs/PropertyList-1.0.dtd">
<plist version="1.0"><dict>
<key>CFBundleExecutable</key><string>Velune</string>
<key>CFBundleIdentifier</key><string>local.velune.prototype</string>
<key>CFBundleName</key><string>Velune</string>
<key>CFBundleIconFile</key><string>Velune</string>
<key>LSMinimumSystemVersion</key><string>15.0</string>
<key>CFBundleVersion</key><string>1</string>
<key>CFBundleShortVersionString</key><string>0.1</string>
<key>NSHighResolutionCapable</key><true/>
<key>UTExportedTypeDeclarations</key><array><dict>
<key>UTTypeIdentifier</key><string>local.velune.conversation-link</string>
<key>UTTypeDescription</key><string>Velune conversation link</string>
<key>UTTypeConformsTo</key><array><string>public.data</string></array>
</dict></array>
<key>CFBundleURLTypes</key><array><dict>
<key>CFBundleURLName</key><string>local.velune.conversation</string>
<key>CFBundleURLSchemes</key><array><string>velune</string></array>
</dict></array>
</dict></plist>
PLIST
python3 - "$app/Contents/Resources/build-manifest.json" <<'PY'
import hashlib,json,platform,plistlib,re,subprocess,sys
from pathlib import Path
def cmd(*args): return subprocess.check_output(args,text=True).strip()
version=Path('VERSION').read_text().strip()
plist_path=Path(sys.argv[1]).parents[1]/'Info.plist'
plist=plistlib.loads(plist_path.read_bytes())
plist['VeluneDisplayVersion']=version
plist['CFBundleGetInfoString']=version
plist_path.write_bytes(plistlib.dumps(plist))
schema=int(re.search(r'const CURRENT_SCHEMA_VERSION: u32 = (\d+);', Path('packages/application/src/repository.rs').read_text()).group(1))
manifest={'source_commit':cmd('git','rev-parse','HEAD'),'dirty':bool(cmd('git','status','--porcelain')),'lock_sha256':hashlib.sha256(Path('Cargo.lock').read_bytes()).hexdigest(),'bindings_version':'0.1.0','uniffi_version':'0.32.2','ui_version':version,'simulation':False,'control_transport':'uniffi','language_contract':'typed','config_schema_version':schema,'supported_modes':['conversation_projection','llm_gateway','runtime_instances','synthetic_preview'],'native_verified':False,'rust':cmd('rustc','--version'),'swift':cmd('xcrun','swiftc','--version'),'xcode':cmd('xcodebuild','-version'),'architecture':platform.machine(),'macos':platform.mac_ver()[0],'build_command':'bash scripts/build-macos.sh'}
manifest['go']=cmd('go','version')
manifest['relaykit_go_mod_sha256']=hashlib.sha256(Path('packages/ai-provider/relaykit-bridge/go.mod').read_bytes()).hexdigest()
manifest['relaykit_go_sum_sha256']=hashlib.sha256(Path('packages/ai-provider/relaykit-bridge/go.sum').read_bytes()).hexdigest()
Path(sys.argv[1]).write_text(json.dumps(manifest,ensure_ascii=False,indent=2))
PY
# Ad-hoc local debug signing only. No identity/keychain selection, certificate, or notarization.
codesign --force --sign - "$app/Contents/Frameworks/libvelune_bindings.dylib"
codesign --force --sign - "$app"
codesign --verify --deep --strict "$app"
shasum -a 256 "$app/Contents/MacOS/Velune" "$app/Contents/Frameworks/libvelune_bindings.dylib" > target/macos/binary-sha256.txt
printf '已构建：%s\n' "$app"
if [[ "${1:-}" != "--build-only" ]]; then
  python3 scripts/install-macos.py "$app"
fi
