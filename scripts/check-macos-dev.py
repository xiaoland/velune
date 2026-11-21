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


if __name__ == '__main__':
    unittest.main()
