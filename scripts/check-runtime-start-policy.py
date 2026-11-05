#!/usr/bin/env python3
"""运行时新建策略的合成迁移与提交中断检查。"""
import importlib.util
import json
from pathlib import Path
import subprocess
import sys
import tempfile
import unittest
from unittest.mock import patch


SCRIPT = Path(__file__).resolve().parent / "migrations/runtime-start-policy.py"
spec = importlib.util.spec_from_file_location("runtime_start_migration", SCRIPT)
migration = importlib.util.module_from_spec(spec)
spec.loader.exec_module(migration)


class RuntimePolicyMigrationTests(unittest.TestCase):
    def setUp(self):
        temporary = tempfile.TemporaryDirectory(prefix="velune-runtime-policy-")
        self.addCleanup(temporary.cleanup)
        self.home = Path(temporary.name)
        self.policy = {"kind": "chosen", "choice": {"model": None, "effort": None, "permissionMode": None}}
        self.last = {"model": {"native": {"model_id": "synthetic-model", "provider_id": None}}, "effort": None, "permissionMode": None}
        self.intent = {"conversationID": "synthetic", "runtimeID": "agent", "choices": [{"runtimeID": "agent", "choice": self.last}]}
        self.write("generic-config.json", {"schemaVersion": 11, "gateways": [{"id": "default", "name": "Default", "providers": [], "routes": []}], "modelTemplates": [],
            "runtimeInstances": [{"enabled": False, "id": "agent", "name": "Synthetic ACP", "typeId": "acp-v1", "gatewayId": "default", "settings": {"binary": "/synthetic/agent", "execution": '{"transport":"local","home":"inherit","remoteResourcesDirectory":null}', "acpLaunch": '{"args":["synthetic"],"environment":{}}'}}]})
        self.write("composer-choices.json", {"schemaVersion": 1, "choices": {"conversations": [self.intent], "runtimes": [{"runtimeID": "agent", "lastChosen": self.last, "newConversation": self.policy}]}})
        self.write("composer-inputs.json", {"schemaVersion": 2, "state": {"ownerId": "11111111-1111-4111-8111-111111111111", "revision": 7, "inputs": [], "bindings": [], "targets": [], "creations": []}, "attachments": []})
        self.input = (self.home / "composer-inputs.json").read_bytes()

    def write(self, name, value):
        (self.home / name).write_text(json.dumps(value, ensure_ascii=False))

    def read(self, name):
        return json.loads((self.home / name).read_bytes())

    def run_migration(self, apply=True):
        args = [str(SCRIPT), "--home", str(self.home)] + (["--apply"] if apply else [])
        with patch.object(sys, "argv", args):
            migration.main()

    def assert_migrated(self):
        config = self.read("generic-config.json")
        self.assertEqual(config["schemaVersion"], 11)
        runtime = config["runtimeInstances"][0]
        self.assertEqual(json.loads(runtime["settings"]["newConversation"]), self.policy)
        self.assertIn("acpLaunch", runtime["settings"])
        self.assertIn("execution", runtime["settings"])
        composer = self.read("composer-choices.json")
        self.assertEqual(composer, {"schemaVersion": 2, "choices": {"conversations": [self.intent], "runtimes": [{"runtimeID": "agent", "runtimeType": "acp-v1", "lastChosen": self.last}]}})
        self.assertEqual((self.home / "composer-inputs.json").read_bytes(), self.input)

    def test_preview_and_interrupted_commit_resume_preserving_originals(self):
        original = {name: (self.home / name).read_bytes() for name in ("generic-config.json", "composer-choices.json")}
        self.run_migration(apply=False)
        self.assertEqual(original, {name: (self.home / name).read_bytes() for name in original})
        atomic_json = migration.atomic_json
        def fail_after_config(path, value):
            if path.name == "composer-choices.json":
                raise OSError("synthetic interruption after instance commit")
            atomic_json(path, value)
        with patch.object(migration, "atomic_json", side_effect=fail_after_config):
            with self.assertRaises(OSError):
                self.run_migration()
        backups = {path: path.read_bytes() for path in self.home.glob("before-runtime-start-policy-*/*.json")}
        self.assertEqual(self.read("composer-choices.json")["schemaVersion"], 1)
        self.assertEqual(json.loads(self.read("generic-config.json")["runtimeInstances"][0]["settings"]["newConversation"]), self.policy)
        self.run_migration()
        self.assert_migrated()
        self.assertEqual(backups, {path: path.read_bytes() for path in backups})
        current = {path: path.read_bytes() for path in self.home.rglob("*.json")}
        self.run_migration()
        self.assertEqual(current, {path: path.read_bytes() for path in self.home.rglob("*.json")})

    def test_conflicting_instance_policy_refuses_before_writes(self):
        config = self.read("generic-config.json")
        config["runtimeInstances"][0]["settings"]["newConversation"] = '{"kind":"lastChosen"}'
        self.write("generic-config.json", config)
        before = {path: path.read_bytes() for path in self.home.glob("*.json")}
        with self.assertRaises(SystemExit):
            self.run_migration()
        self.assertEqual(before, {path: path.read_bytes() for path in self.home.glob("*.json")})
        self.assertFalse(list(self.home.glob("before-runtime-start-policy-*")))

    def test_full_runner_orders_existing_migrations_and_remains_idempotent(self):
        for _ in range(2):
            result = subprocess.run([sys.executable, str(SCRIPT.parent.parent / "run-migrations.py"), "--home", str(self.home)], capture_output=True, text=True)
            self.assertEqual(result.returncode, 0, result.stdout + result.stderr)
        self.assert_migrated()


if __name__ == "__main__":
    unittest.main()
