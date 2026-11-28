#!/usr/bin/env python3
"""Exercise the xURL install boundary against a local Git source."""

import importlib.util
from pathlib import Path
import os
import subprocess
import tempfile
import unittest
from unittest.mock import patch

spec = importlib.util.spec_from_file_location("install_xurl", Path(__file__).with_name("install-xurl.py"))
install_xurl = importlib.util.module_from_spec(spec)
spec.loader.exec_module(install_xurl)

EXECUTABLE = Path("bin/xurl.exe" if os.name == "nt" else "bin/xurl")
GIT = ["git", "-c", "user.name=xURL Test", "-c", "user.email=xurl@example.invalid", "-c", "commit.gpgsign=false"]


class InstallXurlTest(unittest.TestCase):
    def setUp(self):
        temporary = tempfile.TemporaryDirectory()
        self.addCleanup(temporary.cleanup)
        root = Path(temporary.name)
        self.origin = root / "origin"
        self.destination = root / "staging/xurl"
        self.origin.mkdir()
        subprocess.run([*GIT, "init", "--quiet"], cwd=self.origin, check=True)
        (self.origin / "xurl-cli").mkdir()
        (self.origin / "xurl-cli/Cargo.toml").write_text("pinned source", encoding="utf-8")
        (self.origin / "LICENSE").write_bytes(b"pinned license")
        self.commit("pinned release")
        self.revision = subprocess.check_output([*GIT, "rev-parse", "HEAD"], cwd=self.origin, text=True).strip()
        (self.origin / "xurl-cli/Cargo.toml").write_text("new default branch", encoding="utf-8")
        (self.origin / "LICENSE").write_bytes(b"new license")
        self.commit("later release")
        self.run = subprocess.run

    def commit(self, subject):
        subprocess.run([*GIT, "add", "."], cwd=self.origin, check=True)
        subprocess.run([*GIT, "commit", "--quiet", "-m", subject], cwd=self.origin, check=True)

    def fake_cargo(self, command, **kwargs):
        if command[0] == "cargo":
            source = kwargs["cwd"]
            self.assertIn("--locked", command)
            self.assertEqual(command[command.index("--path") + 1], "xurl-cli")
            self.assertEqual((source / "xurl-cli/Cargo.toml").read_text(encoding="utf-8"), "pinned source")
            self.assertEqual((source / "LICENSE").read_bytes(), b"pinned license")
            executable = Path(command[command.index("--root") + 1]) / EXECUTABLE
            executable.parent.mkdir(parents=True)
            executable.write_bytes(b"pinned binary")
        elif command[1:] == ["--version"] and Path(command[0]).name == EXECUTABLE.name:
            self.assertEqual(Path(command[0]).read_bytes(), b"pinned binary")
        else:
            return self.run(command, **kwargs)
        return subprocess.CompletedProcess(command, 0)

    def test_build_and_license_share_the_pinned_tree(self):
        with patch.object(subprocess, "run", side_effect=self.fake_cargo):
            install_xurl.install(self.destination, str(self.origin), self.revision)
        self.assertEqual((self.destination / EXECUTABLE).read_bytes(), b"pinned binary")
        self.assertEqual((self.destination / "LICENSE").read_bytes(), b"pinned license")
        self.assertEqual((self.destination / "REVISION").read_text(encoding="utf-8"), self.revision + "\n")

    def test_build_failure_preserves_current_install(self):
        (self.destination / "bin").mkdir(parents=True)
        (self.destination / EXECUTABLE).write_bytes(b"previous binary")
        (self.destination / "LICENSE").write_bytes(b"previous license")
        (self.destination / "REVISION").write_bytes(b"previous revision")

        def fail_build(command, **kwargs):
            if command[0] == "cargo":
                raise subprocess.CalledProcessError(42, command)
            return self.fake_cargo(command, **kwargs)

        with patch.object(subprocess, "run", side_effect=fail_build):
            with self.assertRaises(subprocess.CalledProcessError) as caught:
                install_xurl.install(self.destination, str(self.origin), self.revision)
        self.assertEqual(caught.exception.returncode, 42)
        self.assertEqual((self.destination / EXECUTABLE).read_bytes(), b"previous binary")
        self.assertEqual((self.destination / "LICENSE").read_bytes(), b"previous license")
        self.assertEqual((self.destination / "REVISION").read_bytes(), b"previous revision")


if __name__ == "__main__":
    unittest.main()
