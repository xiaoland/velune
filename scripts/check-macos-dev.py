#!/usr/bin/env python3
"""Check development instance reuse cannot cross preview/normal mode."""
import importlib.util
from pathlib import Path
import unittest
from unittest.mock import patch

spec = importlib.util.spec_from_file_location('macos_dev', Path(__file__).with_name('macos-dev.py'))
launcher = importlib.util.module_from_spec(spec)
spec.loader.exec_module(launcher)


class PreviewIdentityTests(unittest.TestCase):
    def setUp(self):
        self.app = {'pid': 123, 'launchedAt': 1000.0}
        self.receipt = dict(self.app, home='/tmp/synthetic', source='/tmp/Dev.app',
                            profile='dev', sourceFingerprint='sha256:synthetic')
        self.marker = {'schemaVersion': 1, 'purpose': 'development', 'scope': 'scope'}

    def reuse(self, preview):
        with patch.object(launcher, 'read_private_json', return_value=self.marker):
            return launcher.reuse(self.app, self.receipt, Path('/tmp/synthetic'),
                                  'scope', Path('/tmp/Dev.app'), 'dev', 'sha256:synthetic', preview)

    def test_old_receipt_remains_normal(self):
        self.assertTrue(self.reuse(False)['reused'])
        with self.assertRaises(launcher.LauncherError):
            self.reuse(True)

    def test_preview_never_reused_as_normal(self):
        self.receipt['preview'] = True
        self.assertTrue(self.reuse(True)['reused'])
        with self.assertRaises(launcher.LauncherError):
            self.reuse(False)

    def test_invalid_mode_rejected_before_any_launch(self):
        with self.assertRaises(launcher.LauncherError):
            launcher.start(preview='--arbitrary-argument')


class RestartTests(unittest.TestCase):
    def test_other_build_is_stale(self):
        app = {'pid': 123, 'launchedAt': 1000.0}
        receipt = dict(app, home='/tmp/synthetic', source='/tmp/Dev.app', profile='dev', sourceFingerprint='sha256:old')
        with self.assertRaises(launcher.StaleInstance):
            launcher.reuse(app, receipt, Path('/tmp/synthetic'), 'scope', Path('/tmp/Dev.app'), 'dev', 'sha256:new')

    def test_unmatched_instance_is_not_stale(self):
        app = {'pid': 123, 'launchedAt': 1000.0}
        matched = dict(app, home='/tmp/synthetic', source='/tmp/Dev.app', profile='dev', sourceFingerprint='sha256:old')
        marker = dict(schemaVersion=1, purpose='development', scope='scope')
        with patch.object(launcher, 'read_private_json', return_value=marker):
            def reuse(receipt):
                return launcher.reuse(app, receipt, Path('/tmp/synthetic'), 'scope', Path('/tmp/Dev.app'), 'dev', 'sha256:old')
            self.assertTrue(reuse(matched)['reused'], 'the complete matching fixture must be admissible')
            for receipt in (dict(matched, pid=124), dict(matched, launchedAt=1001.0),
                            dict(matched, home='/tmp/another-synthetic-home'), None):
                with self.subTest(receipt=receipt):
                    with self.assertRaisesRegex(launcher.LauncherError, '没有匹配的 PID／出生时间／开发目录记录') as rejected:
                        reuse(receipt)
                    self.assertIs(type(rejected.exception), launcher.LauncherError,
                                  'an unmatched instance must never authorize stale-instance restart')

    def test_restart_quits_only_a_stale_instance(self):
        calls = []
        def start(*args, **kwargs):
            calls.append('start')
            if calls.count('start') == 1:
                raise launcher.StaleInstance('stale')
            return {'status': 'running', 'pid': 2}
        with patch.object(launcher, 'start', side_effect=start), \
                patch.object(launcher, 'stop', side_effect=lambda *a, **k: calls.append('stop') or {'pid': 1}):
            result = launcher.restart()
        self.assertEqual(calls, ['start', 'stop', 'start'])
        self.assertTrue(result['restarted'])
        calls.clear()
        with patch.object(launcher, 'start', side_effect=launcher.LauncherError('ambiguous')), \
                patch.object(launcher, 'stop', side_effect=lambda *a, **k: calls.append('stop')):
            with self.assertRaises(launcher.LauncherError):
                launcher.restart()
        self.assertEqual(calls, [])


if __name__ == '__main__':
    unittest.main()
