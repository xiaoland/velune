#!/usr/bin/env python3
"""Run production native composer checks in an owned background-only copy."""
import argparse
import hashlib
import json
import os
from pathlib import Path
import plistlib
import shutil
import signal
import subprocess
import tempfile
import time
from macos_identity import ROOT, app_bundle_path, validate_bundle


def digest(path):
    with path.open('rb') as stream:
        return hashlib.file_digest(stream, 'sha256').hexdigest()


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('check', choices=['provider-analytics', 'archived', 'palette-ime', 'pagination', 'tabs', 'targets', 'ownership', 'background', 'experience', 'recovery', 'background-notifications'])
    parser.add_argument('--app', type=Path, default=app_bundle_path(ROOT, 'dev'))
    parser.add_argument('--output', type=Path)
    parser.add_argument('--dark', action='store_true')
    args = parser.parse_args()
    app = args.app.resolve()
    context = validate_bundle(app, purpose='development')
    output = (args.output or ROOT / 'target/checks' / ('macos-' + args.check + ('-dark' if args.dark else ''))).resolve()
    output.mkdir(parents=True, exist_ok=True)
    original = app / 'Contents/MacOS' / context['executable']
    receipt = {'check': args.check, 'sourceApp': str(app), 'executableSHA256': digest(original),
               'realModelCalls': 0, 'backgroundOnly': True, 'startedAt': time.time()}
    library = app / 'Contents/Frameworks/libvelune_bindings.dylib'
    if library.exists():
        receipt['bindingsSHA256'] = digest(library)
    with tempfile.TemporaryDirectory(prefix='velune-native-composer-', dir='/tmp') as temporary:
        root = Path(temporary)
        copied = root / 'Check.app'
        copy_flag = '-cR' if app.stat().st_dev == root.stat().st_dev else '-R'
        subprocess.run(['/bin/cp', copy_flag, str(app), str(copied)], check=True)
        info_path = copied / 'Contents/Info.plist'
        info = plistlib.loads(info_path.read_bytes())
        info['LSBackgroundOnly'] = True
        info_path.write_bytes(plistlib.dumps(info))
        subprocess.run(['/usr/bin/codesign', '--force', '--sign', '-', str(copied)], check=True, stdout=subprocess.DEVNULL)
        validate_bundle(copied, purpose='development')
        private_home = root / 'user'; private_home.mkdir(mode=0o700)
        private_tmp = root / 'tmp'; private_tmp.mkdir(mode=0o700)
        env = {key: os.environ[key] for key in ['PATH', 'LANG', 'LC_ALL', 'TMPDIR'] if key in os.environ}
        evidence = root / 'evidence'; evidence.mkdir(mode=0o700)
        env.update(HOME=str(private_home), TMPDIR=str(private_tmp) + '/', VELUNE_HOME=str(root / 'state'), VELUNE_CHECK_OUTPUT=str(evidence),
                   VELUNE_CONTINUATION_CHECK_HOME=str(root / 'continuation'), VELUNE_COMPOSER_CHECK_HOME=str(root / 'targets'),
                   VELUNE_EXPERIENCE_CHECK_HOME=str(evidence),
                   CODEX_HOME=str(root / 'codex'), PI_CODING_AGENT_DIR=str(root / 'pi'), CLAUDE_CONFIG_DIR=str(root / 'claude'),
                   XDG_CONFIG_HOME=str(root / 'config'), XDG_DATA_HOME=str(root / 'data'), XDG_CACHE_HOME=str(root / 'cache'))
        flags = ['--check-background-tasks'] if args.check == 'background' else ['--check-message-experience']
        if args.check == 'archived':
            flags = ['--check-archived-conversations']
            if args.dark:
                flags.append('--archived-check-dark')
        if args.check == 'background-notifications':
            flags = ['--check-background-notifications']
        if args.check == 'provider-analytics':
            flags = ['--check-provider-analytics']
        if args.check == 'palette-ime':
            flags = ['--check-palette-ime']
        if args.check == 'pagination':
            flags = ['--check-list-pagination']
        if args.check in ['tabs', 'targets', 'ownership']:
            flags = ['--check-composer-ownership']
            if args.check == 'tabs':
                flags.append('--composer-check-tab-state')
            if args.check == 'targets':
                flags.append('--composer-check-content-target')
        if args.check == 'recovery':
            flags.append('--experience-runtime-recovery-only')
        if args.dark and args.check == 'background':
            flags.append('--background-check-dark')
        with (output / 'native.log').open('w') as log:
            child = subprocess.Popen([str(copied / 'Contents/MacOS' / context['executable']), *flags], env=env,
                                     cwd=ROOT, stdout=log, stderr=subprocess.STDOUT, start_new_session=True)
            receipt['pid'] = child.pid
            try:
                receipt['exitCode'] = child.wait(timeout=180)
            except BaseException:
                os.killpg(child.pid, signal.SIGTERM)
                try:
                    child.wait(timeout=5)
                except subprocess.TimeoutExpired:
                    os.killpg(child.pid, signal.SIGKILL); child.wait()
                raise
            finally:
                receipt['finishedAt'] = time.time()
                (output / 'receipt.json').write_text(json.dumps(receipt, indent=2) + '\n')
        # Copy only explicit result artifacts; application state stays private.
        exported = []
        for result in evidence.iterdir():
            if result.is_file() and (result.name in ['archived-check.json', 'continuation-check.json', 'background-check.json', 'provider-analytics-check.json', 'background-notifications-check.json'] or result.suffix in ['.png', '.csv', '.txt']):
                shutil.copy2(result, output / result.name)
                exported.append(result.name)
        receipt['artifacts'] = sorted(exported)
        (output / 'receipt.json').write_text(json.dumps(receipt, indent=2) + '\n')
        print((output / 'native.log').read_text(), end='')
        print('Evidence:', output)
        return receipt['exitCode']


if __name__ == '__main__':
    raise SystemExit(main())
