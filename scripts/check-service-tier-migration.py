#!/usr/bin/env python3
"""用合成状态验证旧按轮档位迁移；只创建并修改临时目录。"""
import copy
import json
import os
from pathlib import Path
import subprocess
import sys
import tempfile
import unittest


SCRIPTS = Path(__file__).resolve().parent
MIGRATION = SCRIPTS / "migrations/01-service-tier-model-variants.py"
OWNER = "11111111-1111-4111-8111-111111111111"
INPUT = "22222222-2222-4222-8222-222222222222"


def choice(tier=None):
    return {"model": {"native": {"provider_id": None, "model_id": "gpt-test"}},
            "effort": "high", "permissionMode": "read-only", "serviceTier": tier}


class MigrationTests(unittest.TestCase):
    def setUp(self):
        self.temporary = tempfile.TemporaryDirectory(prefix="velune-service-tier-test-")
        self.addCleanup(self.temporary.cleanup)
        self.home = Path(self.temporary.name)
        self.write("generic-config.json", {"schemaVersion": 11, "gateways": [], "modelTemplates": [],
                   "runtimeInstances": [{"id": "codex", "typeId": "codex-0.166"}]})
        self.write("composer-inputs.json", {"schemaVersion": 1, "state": {
            "ownerId": OWNER, "revision": 9, "creations": [],
            "inputs": [{"id": INPUT, "revision": 4, "content": {
                "text": "合成草稿", "attachments": [], "runtimeId": "codex", "runtimeOwnerId": OWNER,
                "choices": [{"ownerId": OWNER, "runtimeId": "codex", "choice": choice()}], "choiceEdited": True}}],
            "bindings": [{"context": {"kind": "newConversation"}, "inputId": INPUT, "revision": 9}]}, "attachments": []})
        self.write("composer-choices.json", {"schemaVersion": 1, "choices": {"conversations": [], "runtimes": [
            {"runtimeID": "codex", "lastChosen": choice(), "newConversation": {"kind": "chosen", "choice": choice()}}]}})

    def write(self, name, value):
        path = self.home / name
        path.parent.mkdir(parents=True, exist_ok=True)
        path.write_text(json.dumps(value, ensure_ascii=False))

    def read(self, name):
        return json.loads((self.home / name).read_text())

    def invoke(self, script=MIGRATION, apply=False, success=True):
        result = subprocess.run([sys.executable, str(script), "--home", str(self.home)] + (["--apply"] if apply else []),
                                text=True, capture_output=True)
        self.assertEqual(result.returncode == 0, success, result.stdout + result.stderr)
        return result

    def sources(self):
        return {path: path.read_bytes() for path in self.home.rglob("*.json")}

    def assert_unchanged(self, original):
        self.assertEqual(self.sources(), original)
        self.assertFalse(list(self.home.glob("before-*")))

    def test_preview_backup_idempotence_and_runner(self):
        original = self.sources()
        self.invoke()
        self.assert_unchanged(original)
        self.invoke(SCRIPTS / "migrations/composer-state.py", apply=True, success=False)
        self.assert_unchanged(original)
        self.invoke(apply=True)
        backup = next(self.home.glob("before-service-tier-model-variants-*"))
        for path, data in original.items():
            if path.name == "generic-config.json":
                continue
            saved = backup / path.relative_to(self.home)
            self.assertEqual(saved.read_bytes(), data)
            if os.name != "nt":
                self.assertEqual(saved.stat().st_mode & 0o777, 0o600)
            self.assertNotIn("serviceTier", path.read_text())
        migrated = self.sources()
        self.invoke(apply=True)
        self.assertEqual(self.sources(), migrated)
        self.invoke(SCRIPTS / "run-migrations.py")
        current = self.read("composer-inputs.json")
        self.assertEqual(current["schemaVersion"], 2)
        self.assertEqual(current["state"]["ownerId"], OWNER)
        self.assertEqual(current["state"]["revision"], 10)
        self.assertEqual(current["state"]["inputs"][0], {"id": INPUT, "revision": 4,
                         "content": {"text": "合成草稿", "attachments": []}})
        expected = choice()
        expected.pop("serviceTier")
        self.assertEqual(current["state"]["targets"][0]["choices"]["choices"][0]["choice"], expected)

    def test_nonempty_native_and_platform_encodings(self):
        self.write("composer-choices.json", {"schemaVersion": 1, "choices": {"conversations": [], "runtimes": [
            {"runtimeID": "codex", "lastChosen": choice("priority"), "newConversation": {"kind": "lastChosen"}}]}})
        settings = {"runtimeId": "codex", "model": "gpt-test", "modelSelection": choice()["model"],
                    "effort": "high", "permissionMode": "read-only", "serviceTier": "priority"}
        self.write("turn-requests.json", {"schemaVersion": 1, "conversations": [
            {"conversationID": "synthetic", "turns": [{"ordinal": 0, "digest": "0123456789abcdef", "settings": settings}]}]})
        self.write("mac-window-workspaces/synthetic.json", {"composerWorkspace": {"schemaVersion": 3,
                   "pendingTargets": {"synthetic": {"choices": {"choices": {"codex": {
                       "model": {"native": {"providerId": "native-provider", "modelId": "gpt-test"}},
                       "effort": "high", "permissionMode": None, "serviceTier": "priority"}}}}}}})
        self.write("composer-cache/synthetic.json", {"Schema": 2, "TargetChoices": {"Choices": [{
            "RuntimeId": "codex", "Choice": {"Model": {"kind": "native", "provider": None, "model": "gpt-test"},
            "Effort": "high", "PermissionMode": None, "ServiceTier": "priority"}}]}})
        self.invoke(apply=True)
        variant = 'velune:codex-speed:["gpt-test","priority"]'
        selected = self.read("composer-choices.json")["choices"]["runtimes"][0]["lastChosen"]
        self.assertEqual(selected["model"]["native"]["model_id"], variant)
        recorded = self.read("turn-requests.json")["conversations"][0]["turns"][0]["settings"]
        self.assertEqual(recorded["modelSelection"]["native"]["model_id"], variant)
        self.assertEqual(recorded["model"], "gpt-test · priority")
        mac = self.read("mac-window-workspaces/synthetic.json")["composerWorkspace"]["pendingTargets"]["synthetic"]["choices"]["choices"]["codex"]["model"]["native"]
        self.assertEqual(mac, {"providerId": "native-provider", "modelId": variant})
        windows = self.read("composer-cache/synthetic.json")["TargetChoices"]["Choices"][0]["Choice"]["Model"]
        self.assertEqual(windows, {"kind": "native", "provider": None, "model": variant})

    def test_ambiguous_or_unknown_choices_refuse_whole_batch(self):
        baseline = self.read("composer-inputs.json")
        for model, extra in [
            (None, {}), ({"managed": {"model_record_key": "managed"}}, {}),
            (choice()["model"], {"unknown": True}),
            ({"native": {"provider_id": None, "model_id": 'velune:codex-speed:["gpt-test","ultrafast"]'}}, {}),
        ]:
            with self.subTest(model=model, extra=extra):
                value = copy.deepcopy(baseline)
                saved = value["state"]["inputs"][0]["content"]["choices"][0]["choice"]
                saved.update(model=model, serviceTier="priority", **extra)
                self.write("composer-inputs.json", value)
                original = self.sources()
                self.invoke(apply=True, success=False)
                self.assert_unchanged(original)

    def test_duplicate_fields_refuse_whole_batch(self):
        (self.home / "composer-choices.json").write_text('{"schemaVersion":1,"choices":{},"choices":{}}')
        original = self.sources()
        self.invoke(apply=True, success=False)
        self.assert_unchanged(original)

    @unittest.skipIf(os.name == "nt", "此锁验证使用 POSIX flock")
    def test_live_runtime_lock_refuses_write(self):
        import fcntl
        original = self.sources()
        with (self.home / "runtime.lock").open("a+b") as lock:
            fcntl.flock(lock, fcntl.LOCK_EX | fcntl.LOCK_NB)
            self.invoke(apply=True, success=False)
        self.assert_unchanged(original)


if __name__ == "__main__":
    unittest.main()
