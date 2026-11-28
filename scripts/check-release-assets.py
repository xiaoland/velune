#!/usr/bin/env python3
"""Exercise release preparation with valid and invalid upload directories."""

import hashlib
import io
import json
from pathlib import Path
import subprocess
import sys
import tarfile
import tempfile
import unittest

from release_assets import ANDROID_CERTIFICATE

SCRIPTS = Path(__file__).resolve().parent
VERSION = (SCRIPTS.parent / "version.txt").read_text(encoding="utf-8").strip()
TAG = f"v{VERSION}"
COMMIT = "a" * 40
BUILD_NUMBER = 37


class ReleaseAssetsTest(unittest.TestCase):
    def setUp(self):
        temporary = tempfile.TemporaryDirectory()
        self.addCleanup(temporary.cleanup)
        self.root = Path(temporary.name) / "release"
        self.root.mkdir()
        self.identity = {
            "tag": TAG, "version": VERSION, "source_commit": COMMIT, "build_number": BUILD_NUMBER,
        }
        self.identity_path = self.root / "release-build.json"
        self.write_record(self.identity_path, self.identity)
        self.android = self.add_binary(f"Velune-{TAG}-android-arm64.apk", "android", "arm64")
        self.add_binary(f"Velune-{TAG}-macOS-arm64.zip", "macos", "arm64")
        self.add_binary(f"Velune-{TAG}-macOS-x86_64.zip", "macos", "x86_64")
        self.source = self.make_source_package(Path(temporary.name) / "candidate.tar.gz")

    def write_record(self, path, record):
        path.write_text(json.dumps(record), encoding="utf-8")

    def update_record(self, asset, **changes):
        path = self.root / f"{asset.name}.build.json"
        record = json.loads(path.read_text(encoding="utf-8"))
        self.write_record(path, {**record, **changes})

    def add_binary(self, name, platform, architecture):
        asset = self.root / name
        asset.write_bytes(name.encode())
        record = {
            **self.identity, "artifact": name, "platform": platform, "architecture": architecture,
            "binary_version": VERSION, "sha256": hashlib.sha256(asset.read_bytes()).hexdigest(),
        }
        if platform == "android":
            record["certificate_sha256"] = ANDROID_CERTIFICATE
        self.write_record(self.root / f"{name}.build.json", record)
        return asset

    def make_source_package(self, archive):
        manifest = json.dumps({
            "source_tag": TAG, "source_commit": COMMIT, "corresponding_platforms": ["macos", "android"],
        }).encode()
        with tarfile.open(archive, "w:gz") as tar:
            member = tarfile.TarInfo(f"velune-{TAG}/SOURCE-ARCHIVE.json")
            member.size = len(manifest)
            tar.addfile(member, io.BytesIO(manifest))
        subprocess.run(
            [sys.executable, "-I", str(SCRIPTS / "package-release.py"), "source", "--input", str(archive),
             "--output-dir", str(self.root), "--tag", TAG, "--version", VERSION, "--commit", COMMIT,
             "--build-number", str(BUILD_NUMBER), "--arch", "all"],
            check=True, capture_output=True, text=True,
        )
        return self.root / f"velune-{TAG}-source.tar.gz"

    def prepare(self):
        return subprocess.run(
            [sys.executable, str(SCRIPTS / "release_assets.py"), "prepare-upload", "--directory", str(self.root),
             "--tag", TAG, "--version", VERSION, "--commit", COMMIT, "--build-number", str(BUILD_NUMBER)],
            capture_output=True, text=True,
        )

    def assert_rejected(self, diagnostic):
        result = self.prepare()
        self.assertEqual(result.returncode, 2)
        self.assertIn(diagnostic, result.stderr)
        self.assertEqual(result.stdout, "")
        self.assertFalse((self.root / "SHA256SUMS.txt").exists())

    def test_complete_set_and_checksums(self):
        expected = sorted(self.root.iterdir())
        result = self.prepare()
        self.assertEqual(result.returncode, 0, result.stderr)
        checksum_file = self.root / "SHA256SUMS.txt"
        self.assertEqual(sorted(map(Path, result.stdout.splitlines())), sorted([*expected, checksum_file]))
        expected_lines = [
            f"{hashlib.sha256(asset.read_bytes()).hexdigest()}  {asset.name}" for asset in expected
        ]
        self.assertEqual(checksum_file.read_text(encoding="utf-8").splitlines(), expected_lines)
        self.assertEqual(self.prepare().stdout, result.stdout)

    def test_rejects_unexpected_file(self):
        (self.root / "diagnostics.log").write_text("unexpected", encoding="utf-8")
        self.assert_rejected("unexpected=['diagnostics.log']")

    def test_rejects_missing_binary(self):
        self.android.unlink()
        self.assert_rejected(f"missing=['{self.android.name}']")

    def test_rejects_changed_binary(self):
        self.android.write_bytes(b"different")
        self.assert_rejected(f"{self.android.name}.build.json has an unexpected sha256")

    def test_rejects_wrong_certificate(self):
        self.update_record(self.android, certificate_sha256="b" * 64)
        self.assert_rejected("unexpected certificate_sha256")

    def test_rejects_incomplete_source_coverage(self):
        self.update_record(self.source, corresponding_platforms=["macos"])
        self.assert_rejected("unexpected corresponding_platforms")

    def test_rejects_wrong_release_build_number(self):
        self.write_record(self.identity_path, {**self.identity, "build_number": BUILD_NUMBER + 1})
        self.assert_rejected("release-build.json has an unexpected build_number")


if __name__ == "__main__":
    unittest.main()
