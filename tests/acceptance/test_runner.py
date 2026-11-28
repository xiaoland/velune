"""Parent runner contracts, using real child processes without loading Rust."""
from __future__ import annotations

import contextlib
import hashlib
import importlib.util
import io
import json
import sys
import tempfile
import unittest
from pathlib import Path
from unittest.mock import patch

HERE = Path(__file__).resolve().parent
spec = importlib.util.spec_from_file_location("acceptance_runner", HERE / "run.py")
runner = importlib.util.module_from_spec(spec)
spec.loader.exec_module(runner)


class RunnerTests(unittest.TestCase):
    def run_worker(self, payload: str | None, exit_code: int = 0):
        with tempfile.TemporaryDirectory(prefix="velune-runner-test-") as temporary:
            root = Path(temporary)
            cases = root / "cases"
            cases.mkdir()
            (cases / "synthetic.py").write_text("REQUIRES = ()\n")
            bindings = root / "bindings"
            bindings.mkdir()
            (bindings / "libvelune_bindings.so").write_bytes(b"not loaded by the synthetic worker")
            resources = root / "resources"
            resources.mkdir()
            worker = root / "worker.py"
            write = f"pathlib.Path(sys.argv[4]).write_text({payload!r})\n" if payload is not None else ""
            worker.write_text(f"import pathlib, sys\n{write}sys.exit({exit_code})\n")
            output = io.StringIO()
            # Only replace the external worker executable and output locations.
            # The actual parent owns subprocess execution, result parsing and exit policy.
            with patch.object(runner, "__file__", str(worker)), \
                    patch.object(runner, "CASES", cases), \
                    patch.object(runner, "RESULTS", root / "results"), \
                    patch.object(sys, "argv", [str(HERE / "run.py"), "synthetic", "--bindings", str(bindings),
                                               "--resources", str(resources)]), \
                    contextlib.redirect_stdout(output):
                status = runner.main()
            manifests = list((root / "results").glob("*/manifest.json"))
            self.assertEqual(len(manifests), 1, output.getvalue())
            manifest = json.loads(manifests[0].read_text())
            normalized = json.loads((manifests[0].parent / "synthetic.json").read_text())
            self.assertEqual(manifest["cases"][0], normalized)
            return status, manifest["cases"][0], output.getvalue()

    def test_worker_exit_is_required_even_after_a_passed_result(self):
        valid = {"case": "synthetic", "status": "passed", "checks": {"saved": True}, "facts": {}}
        status, result, output = self.run_worker(json.dumps(valid), exit_code=7)
        self.assertEqual(status, 1, output)
        self.assertEqual(result["status"], "failed")
        self.assertEqual(result["worker_returncode"], 7)
        self.assertIn("worker exited with 7", result["error"])
        self.assertIn("FAIL synthetic", output)

    def test_unreadable_worker_results_still_publish_failures(self):
        valid = {"case": "synthetic", "status": "passed", "checks": {"saved": True}, "facts": {}}
        schema_error = "expected a case result with a known status and check records"
        cases = [
            (None, "no result"),
            ("{broken", "invalid result"),
            ("[]", schema_error),
            (json.dumps({**valid, "status": "unknown"}), schema_error),
            (json.dumps({**valid, "case": "different"}), schema_error),
            (json.dumps({**valid, "facts": []}), schema_error),
            (json.dumps({**valid, "checks": []}), schema_error),
            (json.dumps({key: value for key, value in valid.items() if key != "checks"}), schema_error),
            (json.dumps({**valid, "checks": {}}), "requires recorded successful checks"),
            (json.dumps({**valid, "checks": {"saved": False}}), "requires recorded successful checks"),
        ]
        with tempfile.TemporaryDirectory(prefix="velune-worker-results-") as temporary:
            path = Path(temporary) / "synthetic.json"
            for payload, diagnosis in cases:
                with self.subTest(payload=payload):
                    if payload is not None:
                        path.write_text(payload)
                    result = runner.worker_result("synthetic", path, 0, path.with_suffix(".log"))
                    self.assertEqual(result["status"], "failed")
                    self.assertIn(diagnosis, result["error"])
                    self.assertEqual(json.loads(path.read_text()), result)

    def test_case_failure_is_required_even_after_a_successful_worker_exit(self):
        failed = {"case": "synthetic", "status": "failed", "facts": {},
                  "checks": {}, "error": "target invariant failed"}
        status, result, output = self.run_worker(json.dumps(failed))
        self.assertEqual(status, 1, output)
        self.assertEqual(result["status"], "failed")
        self.assertIn("target invariant failed", result["error"])


class ProvenanceTests(unittest.TestCase):
    def test_existing_generation_receipt_rejects_a_partially_replaced_pair(self):
        sys.path.insert(0, str(HERE))
        from velune_acceptance import provenance

        with tempfile.TemporaryDirectory(prefix="velune-pair-test-") as temporary:
            directory = Path(temporary)
            library, module = directory / "libvelune_bindings.so", directory / "velune_bindings.py"
            library.write_bytes(b"original library")
            module.write_bytes(b"original generated module")
            receipt = {"inputs": {"library": hashlib.sha256(b"original library").hexdigest()},
                       "files": {module.name: hashlib.sha256(b"original generated module").hexdigest()}}
            (directory / ".velune-bindings.json").write_text(json.dumps(receipt))
            self.assertIsNotNone(provenance.generation_receipt(directory, library, {module.name: {}}))
            for changed in (library, module):
                with self.subTest(changed=changed.name):
                    original = changed.read_bytes()
                    changed.write_bytes(b"replacement from a different build")
                    with self.assertRaisesRegex(ValueError, "receipt does not match"):
                        provenance.generation_receipt(directory, library, {module.name: {}})
                    changed.write_bytes(original)

    def test_inputs_distinguish_bindings_helpers_and_case_source(self):
        sys.path.insert(0, str(HERE))
        from velune_acceptance import provenance

        with tempfile.TemporaryDirectory(prefix="velune-inputs-test-") as temporary:
            root = Path(temporary)
            bindings, resources, source = (root / name for name in ("bindings", "resources", "source"))
            for path in (bindings, resources, source / "cases", source / "velune_acceptance"):
                path.mkdir(parents=True)
            (bindings / "libvelune_bindings.so").write_bytes(b"synthetic-library")
            module = bindings / "velune_bindings.py"
            module.write_text("old generated module")
            helper = root / "actual-helper.py"
            helper.write_text("old helper")
            (resources / "helper.py").symlink_to(helper)
            (source / "run.py").write_text("synthetic runner")
            case = source / "cases" / "example.py"
            case.write_text("old case")
            before = provenance.capture(bindings, resources, source)
            self.assertEqual(before["resources"]["mode"], "explicit")
            self.assertEqual(before["resources"]["entries"]["bundle"]["path"], str(resources.resolve()))
            self.assertEqual(before["bindings"]["python_modules"]["velune_bindings.py"]["path"], str(module.resolve()))
            for changed, group in [(module, "bindings"), (helper, "resources"), (case, "acceptance")]:
                with self.subTest(group=group):
                    changed.write_text("changed input")
                    after = provenance.capture(bindings, resources, source)
                    self.assertNotEqual(before[group], after[group])
                    for other in {"bindings", "resources", "acceptance"} - {group}:
                        self.assertEqual(before[other], after[other])
                    before = after


if __name__ == "__main__":
    unittest.main()
