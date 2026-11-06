#!/bin/bash
set -euo pipefail
cd "$(dirname "$0")/.."
profile=dev
build_only=false
while [[ $# -gt 0 ]]; do
  case "$1" in
    --profile)
      [[ $# -ge 2 ]] || { echo '缺少 --profile 的值' >&2; exit 2; }
      profile="$2"; shift 2 ;;
    --build-only) build_only=true; shift ;;
    *) echo '用法：scripts/build-macos.sh [--profile dev|release] [--build-only]' >&2; exit 2 ;;
  esac
done
[[ "$profile" == dev || "$profile" == release ]] || { echo 'profile 必须是 dev 或 release' >&2; exit 2; }
[[ "$(uname -s)" == Darwin ]] || { echo 'This script requires macOS + Xcode Command Line Tools.' >&2; exit 1; }
command -v cargo >/dev/null
command -v go >/dev/null
xcrun --find swiftc >/dev/null
if [[ "${VELUNE_BUILD_MANAGED:-}" != macos ]]; then
  args=(--profile "$profile")
  [[ "$build_only" == true ]] && args+=(--build-only)
  exec python3 scripts/build-support.py run --platform macos --profile "$profile" -- bash "$0" "${args[@]}"
fi
[[ -d target/runtime-support/node_modules/huihua && -f target/runtime-support/THIRD_PARTY_NOTICES.md ]] || {
  echo 'Runtime support is missing; run ./scripts/install-runtime-support.sh first.' >&2
  exit 1
}
[[ -x target/runtime-support/xurl/bin/xurl && -f target/runtime-support/xurl/LICENSE && -f target/runtime-support/xurl/REVISION ]] || {
  echo 'xURL is missing; run bash scripts/install-xurl.sh first.' >&2
  exit 1
}
cargo build --locked --target-dir target/host-rust --profile "$profile" -p velune-bindings --lib
bindings="$PWD/target/bindings/swift"
mkdir -p "$bindings"
native_configuration=debug
[[ "$profile" == release ]] && native_configuration=release
native_library="target/host-rust/$native_configuration/libvelune_bindings.dylib"
python3 scripts/build-support.py bindgen --library "$native_library" --language swift --out-dir "$bindings"
app_base=target/macos
[[ "$profile" == dev ]] && app_base=target/macos-dev
app="$PWD/$app_base/Velune.app"
mkdir -p "$app/Contents/MacOS" "$app/Contents/Resources" "$app/Contents/Frameworks"
# The bundle identity goes first: a build failing later still leaves a
# launchable app (direct execution needs it, e.g. UNUserNotificationCenter).
cat <<'PLIST' | python3 scripts/build-support.py write "$app/Contents/Info.plist"
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
</dict><dict>
<key>UTTypeIdentifier</key><string>local.velune.conversation-group</string>
<key>UTTypeDescription</key><string>Velune conversation list group</string>
<key>UTTypeConformsTo</key><array><string>public.data</string></array>
</dict></array>
<key>CFBundleURLTypes</key><array><dict>
<key>CFBundleURLName</key><string>local.velune.conversation</string>
<key>CFBundleURLSchemes</key><array><string>velune</string><string>agents</string></array>
</dict></array>
</dict></plist>
PLIST
prepared="$PWD/target/build-state/prepared/macos-$profile/libvelune_bindings.dylib"
prepared_stamp="$PWD/target/build-state/prepared/macos-$profile/signature.json"
mkdir -p "$(dirname "$prepared")"
identity="${VELUNE_CODESIGN_IDENTITY:--}"
source_hash=$(shasum -a 256 "$native_library" | awk '{print $1}')
prepared_hash=$(shasum -a 256 "$prepared" 2>/dev/null | awk '{print $1}' || true)
prepared_is_current=$(python3 - "$prepared_stamp" "$source_hash" "$identity" "$prepared_hash" <<'PY'
import json,sys
from pathlib import Path
try:
    stamp=json.loads(Path(sys.argv[1]).read_text())
except (OSError, ValueError):
    stamp={}
print('yes' if stamp.get('source_sha256') == sys.argv[2]
      and stamp.get('signing_identity') == sys.argv[3]
      and stamp.get('prepared_sha256') == sys.argv[4]
      and sys.argv[4] else 'no')
PY
)
if [[ "$prepared_is_current" != yes ]]; then
  python3 scripts/build-support.py copy "$native_library" "$prepared"
  install_name_tool -id '@rpath/libvelune_bindings.dylib' "$prepared"
  codesign --force --sign "$identity" "$prepared"
  codesign --verify --strict "$prepared"
  prepared_hash=$(shasum -a 256 "$prepared" | awk '{print $1}')
  python3 - "$source_hash" "$identity" "$prepared_hash" <<'PY' | python3 scripts/build-support.py write "$prepared_stamp"
import json,sys
print(json.dumps({'source_sha256':sys.argv[1],
                  'signing_identity':sys.argv[2],
                  'prepared_sha256':sys.argv[3]},sort_keys=True))
PY
fi
python3 scripts/build-support.py copy "$prepared" "$app/Contents/Frameworks/libvelune_bindings.dylib"
# Each crate with UniFFI types generates its own Swift file and header; all of
# them form the VeluneBindings module over one VeluneBindingsFFI C module.
mkdir -p target/swift-ffi target/swift-bindings
python3 scripts/build-support.py sync "$bindings" target/swift-bindings --include '*.swift' --prune
python3 scripts/build-support.py sync "$bindings" target/swift-ffi --include '*.h' --prune
{
  echo 'module VeluneBindingsFFI {'
  for header in target/swift-ffi/*.h; do echo "    header \"$(basename "$header")\""; done
  echo '    export *'
  echo '    use "Darwin"'
  echo '    use "_Builtin_stdbool"'
  echo '    use "_Builtin_stdint"'
  echo '}'
} | python3 scripts/build-support.py write target/swift-ffi/module.modulemap
configuration=debug
if [[ "$profile" == release ]]; then configuration=release; fi
swift build --build-system xcode --arch "$(uname -m)" --configuration "$configuration" --product VeluneMac -j "${CARGO_BUILD_JOBS:-2}" --force-resolved-versions \
  -Xlinker -L -Xlinker "$app/Contents/Frameworks" \
  -Xlinker -lvelune_bindings -Xlinker -rpath -Xlinker '@executable_path/../Frameworks'
swift_binary_directory=$(swift build --build-system xcode --arch "$(uname -m)" --configuration "$configuration" --product VeluneMac --show-bin-path)
python3 scripts/build-support.py copy "$swift_binary_directory/VeluneMac" "$app/Contents/MacOS/Velune"
# JavaScript is a sealed resource, not a nested macOS executable.
resources="$app/Contents/Resources"
for script in pi_sdk pi_sessions pi_rpc pi_virtual_model pi_auth pi_provider_import huihua_sessions; do
  python3 scripts/build-support.py copy "packages/agent-runtime/resources/$script.mjs" "$resources/$script.mjs"
done
python3 scripts/build-support.py sync target/runtime-support/node_modules "$resources/node_modules" --include '*' --prune
python3 scripts/build-support.py copy target/runtime-support/xurl/bin/xurl "$resources/xurl/bin/xurl"
python3 scripts/build-support.py copy target/runtime-support/xurl/LICENSE "$resources/xurl/LICENSE"
python3 scripts/build-support.py copy target/runtime-support/xurl/REVISION "$resources/xurl/REVISION"
python3 scripts/build-support.py copy target/runtime-support/xurl/LICENSE "$resources/ThirdParty/xurl/LICENSE"
python3 scripts/build-support.py copy target/runtime-support/xurl/REVISION "$resources/ThirdParty/xurl/REVISION"
python3 - <<'PY' | python3 scripts/build-support.py write "$resources/ThirdParty/THIRD_PARTY_NOTICES.md"
from pathlib import Path
value = Path('packages/agent-runtime/runtime-support/THIRD_PARTY_NOTICES.md').read_text()
value += '''\n
## xURL

本应用包含 Xuanwo 与贡献者开发的官方 xURL CLI，未修改其源码。
来源：https://github.com/Xuanwo/xurl
许可证：Apache-2.0。完整许可证与固定源码提交见 xurl/。
'''
print(value, end='')
PY
licenses="target/build-state/licenses/macos-$profile/rust-licenses.txt"
mkdir -p "$(dirname "$licenses")"
python3 -I scripts/android-licenses.py local-runtime,remote "$licenses" "$(rustc -vV | awk '/^host:/ {print $2}')"
python3 scripts/build-support.py copy "$licenses" "$resources/ThirdParty/rust-licenses.txt"
python3 scripts/build-support.py copy packages/agent-runtime/runtime-support/package-lock.json "$resources/ThirdParty/runtime-support-package-lock.json"
python3 scripts/build-support.py copy Package.resolved "$resources/ThirdParty/swift-package-resolved.json"
python3 scripts/build-support.py sync packages/agent-runtime/runtime-support/licenses "$resources/ThirdParty/licenses" --include '*' --prune
python3 scripts/build-support.py sync app/mac/Licenses "$resources/ThirdParty/Swift" --include '*.txt' --include 'NOTICE.md' --prune
python3 scripts/build-support.py sync app/mac/Licenses/SwiftPackages "$resources/ThirdParty/Swift/SwiftPackages" --include '*.txt' --prune
python3 scripts/build-support.py sync app/mac/Licenses/RelayKit "$resources/ThirdParty/RelayKit" --include '*.txt' --include '*.md' --prune
python3 scripts/build-support.py copy packages/ai-provider/relaykit-bridge/go.mod "$resources/ThirdParty/RelayKit/go.mod"
python3 scripts/build-support.py copy packages/ai-provider/relaykit-bridge/go.sum "$resources/ThirdParty/RelayKit/go.sum"
python3 scripts/build-support.py sync app/mac/Licenses/Mermaid "$resources/ThirdParty/Mermaid" --include '*' --prune
python3 scripts/build-support.py sync assets/brand "$resources/Brand" --include '*' --prune
python3 scripts/build-support.py sync app/mac/Assets/Artifacts "$resources/Artifacts" --include '*' --prune
iconset="target/build-state/icons/macos-$profile.iconset"
mkdir -p "$(dirname "$iconset")"
icon="$app_base/Velune.icns"
icon_inputs=$(find assets/brand scripts/render-app-icon.swift -type f -print0 | sort -z | xargs -0 shasum -a 256 | shasum -a 256 | awk '{print $1}')
icon_stamp="target/build-state/icons/macos-$profile.sha256"
if [[ "$(cat "$icon_stamp" 2>/dev/null || true)" != "$icon_inputs" || ! -f "$icon" ]]; then
  xcrun swift scripts/render-app-icon.swift assets/brand "$iconset"
  iconutil -c icns "$iconset" -o "$icon"
  printf '%s\n' "$icon_inputs" | python3 scripts/build-support.py write "$icon_stamp"
fi
python3 scripts/build-support.py copy "$icon" "$resources/Velune.icns"
python3 - "$app/Contents/Info.plist" <<'PY' | python3 scripts/build-support.py write "$app/Contents/Info.plist"
import plistlib,sys
from pathlib import Path
plist=plistlib.loads(Path(sys.argv[1]).read_bytes())
version=Path('VERSION').read_text().strip()
plist['VeluneDisplayVersion']=version
plist['CFBundleGetInfoString']=version
sys.stdout.buffer.write(plistlib.dumps(plist))
PY
python3 - "$profile" <<'PY' | python3 scripts/build-support.py write "$resources/build-manifest.json"
import hashlib,json,platform,plistlib,re,subprocess,sys
from pathlib import Path
def cmd(*args): return subprocess.check_output(args,text=True).strip()
profile=sys.argv[1]
version=Path('VERSION').read_text().strip()
schema=int(re.search(r'const CURRENT_SCHEMA_VERSION: u32 = (\d+);', Path('packages/application/src/repository.rs').read_text()).group(1))
manifest={'source_commit':cmd('git','rev-parse','HEAD'),'dirty':bool(cmd('git','status','--porcelain')),'lock_sha256':hashlib.sha256(Path('Cargo.lock').read_bytes()).hexdigest(),'bindings_version':'0.1.0','uniffi_version':'0.32.2','ui_version':version,'simulation':False,'control_transport':'uniffi','language_contract':'typed','config_schema_version':schema,'supported_modes':['conversation_projection','llm_gateway','runtime_instances','synthetic_preview'],'native_verified':False,'rust':cmd('rustc','--version'),'swift':cmd('xcrun','swiftc','--version'),'xcode':cmd('xcodebuild','-version'),'architecture':platform.machine(),'macos':platform.mac_ver()[0],'build_command':f'bash scripts/build-macos.sh --profile {profile}'}
manifest['go']=cmd('go','version')
manifest['xurl_revision']=Path('target/runtime-support/xurl/REVISION').read_text().strip()
manifest['relaykit_go_mod_sha256']=hashlib.sha256(Path('packages/ai-provider/relaykit-bridge/go.mod').read_bytes()).hexdigest()
manifest['relaykit_go_sum_sha256']=hashlib.sha256(Path('packages/ai-provider/relaykit-bridge/go.sum').read_bytes()).hexdigest()
print(json.dumps(manifest,ensure_ascii=False,indent=2))
PY
# Local signing, not notarization. VELUNE_CODESIGN_IDENTITY names a code
# signing identity in the keychain (e.g. an Apple Development certificate):
# its signature stays the same app to macOS privacy permissions across
# builds. Unset, the build is signed ad hoc, a new app to them each build.
codesign --force --sign "$identity" "$app/Contents/Resources/xurl/bin/xurl"
codesign --force --sign "$identity" "$app"
codesign --verify --deep --strict "$app"
shasum -a 256 "$app/Contents/MacOS/Velune" "$app/Contents/Frameworks/libvelune_bindings.dylib" | python3 scripts/build-support.py write "$app_base/binary-sha256.txt"
printf '已构建：%s\n' "$app"
