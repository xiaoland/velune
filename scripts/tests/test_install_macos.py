import importlib.util
from pathlib import Path
import plistlib
import shutil
import tempfile
import unittest
from unittest.mock import patch


spec = importlib.util.spec_from_file_location("install_macos", Path(__file__).parents[1] / "install-macos.py")
installer = importlib.util.module_from_spec(spec)
spec.loader.exec_module(installer)


class InstallationTests(unittest.TestCase):
    def bundle(self, path, version):
        (path / "Contents").mkdir(parents=True)
        (path / "Contents/Info.plist").write_bytes(plistlib.dumps({
            "CFBundleIdentifier": "local.velune.installation-fixture",
            "CFBundleShortVersionString": version,
        }))
        return path

    def copy_bundle(self, command, **kwargs):
        self.assertEqual(command[0], "/usr/bin/ditto")
        shutil.copytree(command[1], command[2])

    def test_failed_verification_restores_previous_installation(self):
        with tempfile.TemporaryDirectory() as temporary:
            root = Path(temporary)
            source = self.bundle(root / "source.app", "new")
            destination = self.bundle(root / "installed.app", "old")
            def verify(bundle):
                if bundle == destination:
                    raise RuntimeError("synthetic signature failure")
            with patch.object(installer, "running_instances", return_value=[]), patch.object(installer, "verify", side_effect=verify), patch.object(installer.subprocess, "run", side_effect=self.copy_bundle):
                with self.assertRaisesRegex(RuntimeError, "signature failure"):
                    installer.install(source, destination)
            self.assertEqual(installer.bundle_info(destination)["CFBundleShortVersionString"], "old")

    def test_running_application_is_not_replaced(self):
        with tempfile.TemporaryDirectory() as temporary:
            root = Path(temporary)
            source = self.bundle(root / "source.app", "new")
            destination = self.bundle(root / "installed.app", "old")
            with patch.object(installer, "running_instances", return_value=[123]), patch.object(installer, "verify"), patch.object(installer.subprocess, "run") as commands:
                with self.assertRaisesRegex(RuntimeError, "尚未退出"):
                    installer.install(source, destination)
                commands.assert_not_called()
            self.assertEqual(installer.bundle_info(destination)["CFBundleShortVersionString"], "old")

    def test_native_process_detection_uses_bundle_identity(self):
        with tempfile.TemporaryDirectory(prefix="velune install ") as temporary:
            bundle = self.bundle(Path(temporary) / "fixture.app", "fixture")
            executable = bundle / "Contents/MacOS/Velune"
            rows = f"123 {executable}\n124 /usr/bin/unrelated\n"
            with patch.object(installer.subprocess, "check_output", return_value=rows) as probe:
                self.assertEqual(installer.running_instances("local.velune.installation-fixture"), [123])
                self.assertEqual(installer.running_instances("local.velune.unrelated-fixture"), [])
                probe.assert_called_with(["/bin/ps", "-axo", "pid=,comm="], text=True)


if __name__ == "__main__":
    unittest.main()
