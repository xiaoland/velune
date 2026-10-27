"""通过已安装 Mac 应用的真实控件和合成配置验证保存与普通重启。"""
import argparse
import os
from pathlib import Path
import subprocess


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('--app', required=True, type=Path, help='已安装的 Velune.app')
    args = parser.parse_args()
    app = args.app.resolve()
    bundle_id = subprocess.check_output(['/usr/libexec/PlistBuddy', '-c', 'Print CFBundleIdentifier', str(app / 'Contents/Info.plist')], text=True).strip()
    if bundle_id != 'local.velune.prototype':
        parser.error('请使用已安装的正式 Velune（local.velune.prototype）')
    repository = Path(__file__).resolve().parents[3]
    output = repository / 'target/telemetry-mac-ui'
    output.mkdir(parents=True, exist_ok=True, mode=0o700)
    developer = Path(subprocess.check_output(['xcode-select', '-p'], text=True).strip())
    platform = developer / 'Platforms/MacOSX.platform/Developer'
    frameworks, library = platform / 'Library/Frameworks', platform / 'usr/lib'
    binary = output / 'MacTelemetryUI'
    command = ['xcrun', 'swiftc', str(Path(__file__).with_suffix('.swift')), '-o', str(binary),
               '-F', str(frameworks), '-I', str(library), '-L', str(library),
               '-lXCTestSwiftSupport', '-framework', 'XCTest',
               '-Xlinker', '-rpath', '-Xlinker', str(library),
               '-Xlinker', '-rpath', '-Xlinker', str(frameworks)]
    with (output / 'build.log').open('w') as log:
        subprocess.run(command, check=True, stdout=log, stderr=subprocess.STDOUT, timeout=45)
    environment = {k: v for k, v in os.environ.items() if k in {'PATH', 'TMPDIR', 'USER', 'LOGNAME', 'LANG'}}
    with (output / 'test.log').open('w') as log:
        subprocess.run([str(binary), str(output), str(app)], check=True, env=environment,
                       stdout=log, stderr=subprocess.STDOUT, timeout=45)
    print(output / 'mac-save-evidence.json')


if __name__ == '__main__':
    main()
