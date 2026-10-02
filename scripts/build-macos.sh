#!/bin/bash
set -euo pipefail
cd "$(dirname "$0")/.."
[[ "$(uname -s)" == Darwin ]] || { echo 'This script requires macOS + Xcode Command Line Tools.' >&2; exit 1; }
command -v cargo >/dev/null
xcrun --find swiftc >/dev/null
cargo build --locked --release
app="$PWD/target/macos/Velune.app"
mkdir -p "$app/Contents/MacOS" "$app/Contents/Helpers" "$app/Contents/Resources"
cp target/release/velune-core "$app/Contents/Helpers/velune-core"
xcrun swiftc -swift-version 5 -framework AppKit native/macos/main.swift -o "$app/Contents/MacOS/Velune"
cat > "$app/Contents/Info.plist" <<'PLIST'
<?xml version="1.0" encoding="UTF-8"?>
<!DOCTYPE plist PUBLIC "-//Apple//DTD PLIST 1.0//EN" "http://www.apple.com/DTDs/PropertyList-1.0.dtd">
<plist version="1.0"><dict>
<key>CFBundleExecutable</key><string>Velune</string>
<key>CFBundleIdentifier</key><string>local.velune.prototype</string>
<key>CFBundleName</key><string>Velune</string>
<key>CFBundleVersion</key><string>1</string>
<key>CFBundleShortVersionString</key><string>0.1.0</string>
<key>NSHighResolutionCapable</key><true/>
</dict></plist>
PLIST
python3 - "$app/Contents/Resources/build-manifest.json" <<'PY'
import hashlib,json,platform,subprocess,sys
from pathlib import Path
def cmd(*args): return subprocess.check_output(args,text=True).strip()
manifest={'source_commit':cmd('git','rev-parse','HEAD'),'dirty':bool(cmd('git','status','--porcelain')),'lock_sha256':hashlib.sha256(Path('Cargo.lock').read_bytes()).hexdigest(),'core_version':'0.1.0','contract_version':1,'schema_version':1,'ui_version':'0.1.0','simulation':True,'rust':cmd('rustc','--version'),'swift':cmd('xcrun','swiftc','--version'),'xcode':cmd('xcodebuild','-version'),'architecture':platform.machine(),'macos':platform.mac_ver()[0],'build_command':'bash scripts/build-macos.sh'}
Path(sys.argv[1]).write_text(json.dumps(manifest,ensure_ascii=False,indent=2))
PY
# Ad-hoc local debug signing only. No identity/keychain selection, certificate, or notarization.
codesign --force --sign - "$app/Contents/Helpers/velune-core"
codesign --force --sign - "$app"
codesign --verify --deep --strict "$app"
shasum -a 256 "$app/Contents/MacOS/Velune" "$app/Contents/Helpers/velune-core" > target/macos/binary-sha256.txt
printf 'Built local debug app: %s\nRun: open "%s"\n' "$app" "$app"
