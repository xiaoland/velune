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
    def run_worker(self, body: str):
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
            worker.write_text("import json, pathlib, sys\nresult = pathlib.Path(sys.argv[4])\n" + body)
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
        status, result, output = self.run_worker(
            'result.write_text(json.dumps({"case":"synthetic","status":"passed","checks":{"saved":True},"facts":{}}))\n'
            'sys.exit(7)\n')
        self.assertEqual(status, 1, output)
        self.assertEqual(result["status"], "failed")
        self.assertIn("7", result["error"])
        self.assertIn("FAIL synthetic", output)

    def test_unreadable_worker_results_still_publish_failures(self):
        for body, diagnosis in [
            ("pass\n", "no result"),
            ("result.write_text('{broken')\n", "invalid result"),
            ("result.write_text('[]')\n", "invalid result"),
            ('result.write_text(\'{"case":"synthetic","status":"unknown"}\')\n', "invalid result"),
            ('result.write_text(\'{"case":"different","status":"passed"}\')\n', "invalid result"),
            ('result.write_text(\'{"case":"synthetic","status":"passed","checks":[]}\')\n', "invalid result"),
            ('result.write_text(\'{"case":"synthetic","status":"passed","facts":{}}\')\n', "invalid result"),
            ('result.write_text(\'{"case":"synthetic","status":"passed","checks":{},"facts":{}}\')\n', "requires recorded successful checks"),
            ('result.write_text(\'{"case":"synthetic","status":"passed","checks":{"saved":false},"facts":{}}\')\n', "requires recorded successful checks"),
        ]:
            with self.subTest(diagnosis=diagnosis, body=body):
                status, result, output = self.run_worker(body)
                self.assertEqual(status, 1, output)
                self.assertEqual(result["status"], "failed")
                self.assertIn(diagnosis, result["error"])

    def test_success_skip_and_case_failure_keep_their_distinct_meaning(self):
        for state, expected, code in [("passed", 0, 0), ("skipped", 0, 0), ("failed", 1, 1)]:
            with self.subTest(state=state):
                payload = {"case": "synthetic", "status": state, "facts": {}, "checks": {}}
                if state == "passed":
                    payload["checks"]["saved"] = True
                elif state == "skipped":
                    payload["reason"] = "requires --codex"
                else:
                    payload["error"] = "target invariant failed"
                status, result, output = self.run_worker(
                    f"result.write_text({json.dumps(payload)!r})\nsys.exit({code})\n")
                self.assertEqual(status, expected, output)
                self.assertEqual(result["status"], state)
                if state == "failed":
                    self.assertIn("target invariant failed", result["error"])


class ProvenanceTests(unittest.TestCase):
    def test_generated_constructor_repair_preserves_keyword_calls_only(self):
        sys.path.insert(0, str(HERE))
        from velune_acceptance.bindings import _repair

        namespace = {}
        exec(_repair("class Command:\n    def __init__(self, optional=None, required):\n"
                     "        self.required = required\n", "generated-command.py"), namespace)
        self.assertEqual(namespace["Command"](required="caller input").required, "caller input")
        with self.assertRaises(SyntaxError):
            _repair("def ordinary(optional=None, required):\n    pass\n", "ordinary.py")
        with self.assertRaises(SyntaxError):
            _repair("class Command:\n    def __init__(self, missing syntax):\n        pass\n", "broken.py")

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
