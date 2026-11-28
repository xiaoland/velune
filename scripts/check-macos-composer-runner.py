#!/usr/bin/env python3
"""Check native-check process orchestration with real, non-GUI workers."""
import contextlib
import importlib.util
import io
import json
from pathlib import Path
import plistlib
import sys
import tempfile
import unittest
from unittest.mock import patch

spec = importlib.util.spec_from_file_location('macos_composer', Path(__file__).with_name('check-macos-composer.py'))
runner = importlib.util.module_from_spec(spec)
spec.loader.exec_module(runner)


class NativeCheckRunnerChecks(unittest.TestCase):
    def run_fixture(self, check, *, first_exit=0, reopen_exit=0, extra=()):
        with tempfile.TemporaryDirectory(prefix='velune-native-runner-') as temporary:
            root = Path(temporary)
            app = root / 'Fixture.app'
            executable = app / 'Contents/MacOS/Fixture'
            executable.parent.mkdir(parents=True)
            (app / 'Contents/Info.plist').write_bytes(plistlib.dumps({'CFBundleExecutable': 'Fixture'}))
            # Only bundle identity/signing is replaced. The wrapper launches and
            # reaps real workers, which exchange state solely through its HOME.
            executable.write_text(f'''#!/usr/bin/env python3
import json, os, pathlib, sys
output = pathlib.Path(os.environ['VELUNE_CHECK_OUTPUT'])
(output / 'worker-arguments.txt').write_text(json.dumps(sys.argv[1:]))
state = pathlib.Path(os.environ['VELUNE_COMPOSER_CHECK_HOME']) / 'saved.json'
state.parent.mkdir(parents=True, exist_ok=True)
reopen = any(flag in sys.argv for flag in ['--composer-check-reopen', '--experience-reopen'])
if reopen:
    saved = json.loads(state.read_text())
    assert saved['pid'] != os.getpid()
    try:
        os.kill(saved['pid'], 0)
    except ProcessLookupError:
        pass
    else:
        raise AssertionError('first process must have exited before reopening')
    assert saved['text'] == 'durable synthetic draft'
    (output / 'worker-reopen.txt').write_text('reopened in a new process')
    sys.exit({reopen_exit})
state.write_text(json.dumps(dict(pid=os.getpid(), text='durable synthetic draft')))
(output / 'worker-first.txt').write_text('saved')
sys.exit({first_exit})
''')
            executable.chmod(0o700)
            output = root / 'output'
            real_run = runner.subprocess.run

            def run(command, **kwargs):
                if command[0] == '/usr/bin/codesign':
                    return runner.subprocess.CompletedProcess(command, 0)
                if command[0] == '/bin/cp':
                    # macOS's clone-copy flag is an optimization; use portable
                    # copying so the process boundary also runs in Linux CI.
                    command = [command[0], '-R', *command[2:]]
                return real_run(command, **kwargs)

            argv = ['check-macos-composer.py', check, '--app', str(app), '--output', str(output), *extra]
            with patch.object(sys, 'argv', argv), patch.object(runner, 'validate_bundle', return_value={'executable': 'Fixture'}), \
                    patch.object(runner.subprocess, 'run', side_effect=run), contextlib.redirect_stdout(io.StringIO()):
                status = runner.main()
            files = {path.name: path.read_text() for path in output.iterdir()}
            return status, json.loads(files['receipt.json']), files

    def test_owned_state_is_reopened_only_after_first_process_exits(self):
        for check in ('ownership', 'experience'):
            with self.subTest(check=check):
                status, receipt, files = self.run_fixture(check)
                self.assertEqual(status, 0)
                self.assertIn('worker-reopen.txt', files, 'the persisted state must be consumed in a second process')
                self.assertEqual([phase['exitCode'] for phase in receipt['phases']], [0, 0])
                self.assertNotEqual(receipt['phases'][0]['pid'], receipt['phases'][1]['pid'])

    def test_reopen_failure_is_not_hidden_by_successful_save(self):
        status, receipt, files = self.run_fixture('ownership', reopen_exit=9)
        self.assertEqual(status, 9)
        self.assertEqual(receipt['exitCode'], 9)
        self.assertIn('reopen.log', files)

    def test_failed_save_never_starts_reopen(self):
        status, receipt, files = self.run_fixture('experience', first_exit=7)
        self.assertEqual(status, 7)
        self.assertNotIn('worker-reopen.txt', files)
        self.assertEqual(receipt['exitCode'], 7)

    def test_other_checks_and_workspace_only_do_not_use_reopen_fixture(self):
        for check, extra in [('controls', ()), ('experience', ('--workspace-only',)),
                             ('ownership', ('--child-open',))]:
            with self.subTest(check=check, extra=extra):
                status, _, files = self.run_fixture(check, extra=extra)
                self.assertEqual(status, 0)
                self.assertNotIn('worker-reopen.txt', files)

    def test_named_checks_select_their_native_entry(self):
        cases = [
            ('requests', (), ['--check-conversation-requests']),
            ('interactions', (), ['--check-runtime-interactions']),
            ('interactions', ('--dark',), ['--check-runtime-interactions', '--interaction-check-dark']),
            ('fork', (), ['--check-message-experience', '--experience-fork-only']),
            ('drag-provider', (), ['--check-message-experience', '--experience-drag-provider-only']),
            ('typography', (), ['--check-message-experience', '--experience-typography-only']),
            ('admission', (), ['--check-message-experience', '--experience-admission-only']),
            ('navigation', (), ['--check-message-experience', '--experience-navigation-only']),
            ('group-creation', (), ['--check-group-creation']),
        ]
        for check, extra, expected in cases:
            with self.subTest(check=check, extra=extra):
                status, receipt, files = self.run_fixture(check, extra=extra)
                self.assertEqual(status, 0)
                self.assertEqual(json.loads(files['worker-arguments.txt']), expected)
                self.assertEqual(receipt['phases'][0]['arguments'], expected)
                self.assertEqual(len(receipt['phases']), 1)

    def test_child_open_rejects_a_mislabeled_check_before_bundle_access(self):
        for check in ('fork', 'aggregated-list', 'experience', 'fork-send'):
            with self.subTest(check=check), \
                    patch.object(sys, 'argv', ['check-macos-composer.py', check, '--child-open']), \
                    patch.object(runner, 'validate_bundle', side_effect=AssertionError('bundle accessed before argument rejection')), \
                    contextlib.redirect_stderr(io.StringIO()) as error:
                with self.assertRaises(SystemExit) as rejected:
                    runner.main()
                self.assertEqual(rejected.exception.code, 2)
                self.assertIn('--child-open requires ownership', error.getvalue())


if __name__ == '__main__':
    unittest.main()
