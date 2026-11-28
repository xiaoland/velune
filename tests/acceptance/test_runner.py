"""Validate the parent runner with a real, synthetic child process."""
import contextlib
import importlib.util
import io
import json
import sys
import tempfile
import unittest
from pathlib import Path
from unittest.mock import patch


HERE = Path(__file__).resolve().parent
SPEC = importlib.util.spec_from_file_location("acceptance_runner", HERE / "run.py")
runner = importlib.util.module_from_spec(SPEC)
SPEC.loader.exec_module(runner)


class RunnerTests(unittest.TestCase):
    def run_worker(self, *, status="passed", checks=None, exit_code=0):
        with tempfile.TemporaryDirectory(prefix="velune-runner-test-") as temporary:
            root = Path(temporary)
            cases = root / "cases"
            cases.mkdir()
            (cases / "synthetic.py").touch()
            bindings = root / "bindings"
            bindings.mkdir()
            (bindings / "libvelune_bindings.so").write_bytes(b"synthetic fingerprint only")
            result = {"case": "synthetic", "status": status, "checks": checks, "facts": {}}
            worker = root / "worker.py"
            worker.write_text(
                "import pathlib, sys\n"
                f"pathlib.Path(sys.argv[4]).write_text({json.dumps(result)!r})\n"
                f"raise SystemExit({exit_code})\n"
            )
            captured = io.StringIO()
            with (
                patch.object(runner, "__file__", str(worker)),
                patch.object(runner, "CASES", cases),
                patch.object(runner, "RESULTS", root / "results"),
                patch.object(sys, "argv", [str(HERE / "run.py"), "synthetic", "--bindings", str(bindings)]),
                contextlib.redirect_stdout(captured),
            ):
                code = runner.main()
            manifest, = (root / "results").glob("*/manifest.json")
            result = json.loads(manifest.read_text())["cases"][0]
            self.assertEqual(result, json.loads((manifest.parent / "synthetic.json").read_text()))
            return code, result, captured.getvalue()

    def test_nonzero_exit_overrides_passed_result(self):
        code, result, output = self.run_worker(checks={"saved": True}, exit_code=7)
        self.assertEqual(code, 1)
        self.assertEqual(result["status"], "failed")
        self.assertIn("worker exited with 7", result["error"])
        self.assertIn("FAIL synthetic", output)

    def test_pass_requires_successful_checks(self):
        for checks in (None, {}, {"saved": False}):
            with self.subTest(checks=checks):
                code, result, output = self.run_worker(checks=checks)
                self.assertEqual(code, 1)
                self.assertEqual(result["status"], "failed")
                self.assertIn("FAIL synthetic", output)

    def test_pass_and_skip_keep_their_status(self):
        for status, checks in (("passed", {"saved": True}), ("skipped", {})):
            with self.subTest(status=status):
                code, result, _ = self.run_worker(status=status, checks=checks)
                self.assertEqual(code, 0)
                self.assertEqual(result["status"], status)


if __name__ == "__main__":
    unittest.main()
