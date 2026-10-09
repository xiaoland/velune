#!/usr/bin/env python3
"""Manual signed-app resource acceptance from Xcode-backend objects; synthetic only, never CI."""
import argparse
from pathlib import Path
import plistlib
import shutil
import subprocess
import tempfile

parser = argparse.ArgumentParser(description=__doc__)
parser.add_argument('--swift-build', type=Path, required=True)
parser.add_argument('--hide-build-root', type=Path, required=True, help='An isolated scratch directory, temporarily moved after linking and restored in finally.')
args = parser.parse_args()
build = args.swift_build.resolve()
scratch = args.hide_build_root.resolve()
assert build.is_relative_to(scratch) and scratch.name != '.build', 'Use an isolated scratch directory, not the shared .build.'
backup = scratch.with_name(scratch.name + '-resource-probe-hidden')
assert not backup.exists()
with tempfile.TemporaryDirectory(prefix='velune-markdown-resource-probe-') as directory:
    root = Path(directory)
    app = root / 'ResourceProbe.app'
    (app / 'Contents/MacOS').mkdir(parents=True)
    (app / 'Contents/Resources').mkdir()
    source = root / 'Main.swift'
    source.write_text('''import Foundation
import Highlightr
import SwiftMath
@main struct ResourceProbe {
 @MainActor static func main() {
  guard let highlighter = Highlightr(), let result = highlighter.highlight("let synthetic = 42", as: "swift") else { fatalError("Highlighter assets unavailable") }
  precondition(result.string == "let synthetic = 42")
  precondition(MTFontManager().defaultFont != nil, "Math font assets unavailable")
  print("BUNDLED_HIGHLIGHT_AND_MATH_RESOURCES_PASS")
 }
}
''')
    binary = app / 'Contents/MacOS/ResourceProbe'
    subprocess.run(['xcrun', 'swiftc', '-parse-as-library', '-warnings-as-errors', '-I', str(build), str(source), str(build/'Highlightr_Module.o'), str(build/'SwiftMath_Module.o'), '-o', str(binary)], check=True)
    (app/'Contents/Info.plist').write_bytes(plistlib.dumps({'CFBundleExecutable':'ResourceProbe', 'CFBundleIdentifier':'local.velune.synthetic-resources', 'CFBundlePackageType':'APPL'}))
    for name in ('Highlightr_Highlightr.bundle', 'SwiftMath_SwiftMath.bundle'):
        shutil.copytree(build/name, app/'Contents/Resources'/name)
    subprocess.run(['codesign', '--force', '--deep', '--sign', '-', '--timestamp=none', str(app)], check=True)
    subprocess.run(['codesign', '--verify', '--deep', '--strict', str(app)], check=True)
    scratch.rename(backup)
    try:
        subprocess.run([str(binary)], check=True, cwd=root, env={'HOME':str(root), 'PATH':'/usr/bin:/bin'})
    finally:
        backup.rename(scratch)
    print('STANDARD_RESOURCES_STRICT_SIGNING_WITHOUT_BUILD_TREE_PASS')
