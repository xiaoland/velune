#!/usr/bin/env python3
"""Test native PR routing against committed changes, including removed inputs."""

from pathlib import Path
import runpy
import subprocess
import sys
import tempfile
import unittest


SCRIPT = Path(__file__).with_name("native-pr-inputs.py").resolve()
consumers = runpy.run_path(str(SCRIPT))["consumers"]


class NativeInputsTests(unittest.TestCase):
    def test_docs_do_not_schedule_compilers(self):
        self.assertEqual(consumers({
            "docs/development/platforms.md",
            "app/mac/AGENTS.md",
            "app/windows/PARITY.md",
            "packages/bindings/README.md",
        }), set())

    def test_shared_and_platform_inputs(self):
        checks = {
            "packages/application-api/src/command.rs": {"macos", "ios", "android", "windows"},
            "packages/application/src/local/collaboration/xurl.md": {"macos", "ios", "android", "windows"},
            "app/mac/RemoteNetworkSettings.swift": {"macos", "ios"},
            "app/mac/LibraryWindow.swift": {"macos"},
            "app/ios/Velune.xcodeproj/project.pbxproj": {"ios"},
            "app/android/gradle/wrapper/gradle-wrapper.properties": {"android"},
            "app/windows/packages.lock.json": {"windows"},
            "global.json": {"windows"},
            ".gitattributes": {"windows"},
            "scripts/render-windows-icon.ps1": {"windows"},
            "scripts/install-runtime-support.sh": {"macos", "windows"},
            "scripts/install-xurl.py": {"macos", "windows"},
            ".github/workflows/pr-native-consumers.yml": {"macos", "ios", "android", "windows"},
        }
        for path, expected in checks.items():
            with self.subTest(path=path):
                self.assertEqual(consumers({path}), expected)

    def test_merge_candidate_routes_both_sides_of_a_rename(self):
        with tempfile.TemporaryDirectory(prefix="velune-native-pr-") as directory:
            root = Path(directory)

            def git(*args):
                return subprocess.check_output(["git", *args], cwd=root, text=True)

            git("init", "-q")
            git("config", "user.name", "Test")
            git("config", "user.email", "test@example.invalid")
            source = root / "app/mac/Probe.swift"
            source.parent.mkdir(parents=True)
            source.write_text("probe\n")
            git("add", "app")
            git("commit", "-qm", "base")
            base = git("rev-parse", "HEAD").strip()
            (root / "app/ios").mkdir()
            git("mv", "app/mac/Probe.swift", "app/ios/Probe.swift")
            git("commit", "-qm", "move")

            output = subprocess.check_output([sys.executable, "-B", str(SCRIPT), base], cwd=root, text=True)
            self.assertEqual(output.splitlines(), ['apple=["macos", "ios"]', "android=false", "windows=false"])


if __name__ == "__main__":
    unittest.main()
