#!/usr/bin/env python3
"""合成输入迁移中断恢复检查，不访问日常状态目录。"""
import importlib.util
import json
from pathlib import Path
import sys
import tempfile
import unittest
from unittest.mock import patch


SCRIPT = Path(__file__).resolve().parent / "migrations/composer-state.py"
spec = importlib.util.spec_from_file_location("composer_migration", SCRIPT)
migration = importlib.util.module_from_spec(spec)
spec.loader.exec_module(migration)


class RecoveryTests(unittest.TestCase):
    def setUp(self):
        temporary = tempfile.TemporaryDirectory(prefix="velune-composer-migration-")
        self.addCleanup(temporary.cleanup)
        self.home = Path(temporary.name)
        self.write("composer-drafts.json", {"schemaVersion": 1, "drafts": [
            {"conversationID": "synthetic", "text": "未发送正文"}]})
        self.content = {"Text": "离线正文", "Attachments": [], "RuntimeId": "pi",
                        "RuntimeOwnerId": None, "Choices": [], "ChoiceEdited": True}
        self.write("composer-cache/offline.json", {"Schema": 1,
                   "Input": {"Id": "11111111-1111-4111-8111-111111111111", "Revision": 4,
                             "Content": self.content}, "Pending": True, "Baseline": 3})

    def write(self, name, value):
        path = self.home / name
        path.parent.mkdir(parents=True, exist_ok=True)
        path.write_text(json.dumps(value, ensure_ascii=False))

    def read(self, name):
        return json.loads((self.home / name).read_text())

    def run_migration(self):
        with patch.object(sys, "argv", [str(SCRIPT), "--home", str(self.home), "--apply"]):
            migration.main()

    def assert_recovered(self):
        catalog = self.read("composer-inputs.json")
        self.assertEqual(catalog["schemaVersion"], 2)
        self.assertIn("未发送正文", [entry["content"]["text"] for entry in catalog["state"]["inputs"]])
        cache = self.read("composer-cache/offline.json")
        self.assertEqual(cache["Schema"], 2)
        self.assertEqual(cache["Input"]["Content"], {"Text": "离线正文", "Attachments": []})
        self.assertEqual(cache["TargetChoices"], {key: self.content[key] for key in
                         ("RuntimeId", "RuntimeOwnerId", "Choices", "ChoiceEdited")})
        self.assertTrue(cache["TargetPending"])
        self.assertIsNone(cache["TargetBaseline"])
        snapshot = {path: path.read_bytes() for path in self.home.rglob("*.json")}
        self.run_migration()
        self.assertEqual(snapshot, {path: path.read_bytes() for path in self.home.rglob("*.json")})

    def test_interruption_before_catalog_preserves_identity_and_resumes(self):
        atomic_json = migration.atomic_json
        def interrupt_catalog(path, value):
            if path.name == "composer-inputs.json":
                raise OSError("synthetic interruption before catalog commit")
            atomic_json(path, value)
        with patch.object(migration, "atomic_json", side_effect=interrupt_catalog):
            with self.assertRaises(OSError):
                self.run_migration()
        checkpoint = self.read("composer-inputs.schema1.backup.json")
        self.assertFalse((self.home / "composer-inputs.json").exists())
        self.run_migration()
        actual = self.read("composer-inputs.json")
        self.assertEqual(actual["state"]["ownerId"], checkpoint["state"]["ownerId"])
        self.assertEqual([item["id"] for item in actual["state"]["inputs"]],
                         [item["id"] for item in checkpoint["state"]["inputs"]])
        self.assert_recovered()

    def test_interruption_after_catalog_resumes_caches(self):
        with patch.object(migration, "windows_caches", side_effect=OSError("synthetic interruption")):
            with self.assertRaises(OSError):
                self.run_migration()
        catalog = (self.home / "composer-inputs.json").read_bytes()
        self.run_migration()
        self.assertEqual(catalog, (self.home / "composer-inputs.json").read_bytes())
        self.assert_recovered()

    def test_old_interrupted_import_without_checkpoint_preserves_pending_choices(self):
        with patch.object(migration, "windows_caches", side_effect=OSError("synthetic interruption")):
            with self.assertRaises(OSError):
                self.run_migration()
        (self.home / "composer-inputs.schema1.backup.json").unlink()
        self.run_migration()
        self.assert_recovered()

    def test_unversioned_window_migrates_both_copies_of_shared_draft(self):
        shared = "11111111-1111-4111-8111-111111111111"
        state = {"text": "Mac 待同步草稿", "attachments": [], "choices": {"runtimeID": "pi", "choices": {}},
                 "choiceEdited": True}
        name = "mac-window-workspaces/synthetic.json"
        self.write(name, {"focused": shared, "panes": [], "composer": state,
                         "composerWorkspace": {"schemaVersion": 2, "shared": shared, "retained": [],
                                               "conversations": {"synthetic": shared},
                                               "drafts": [{"id": shared, "name": "Shared", "state": state, "pending": True}]}})
        original = (self.home / name).read_bytes()
        self.run_migration()
        result = self.read(name)
        workspace = result["composerWorkspace"]
        self.assertEqual(workspace["schemaVersion"], 3)
        self.assertEqual(workspace["pendingTargets"]["synthetic"]["choices"]["runtimeId"], "pi")
        self.assertEqual(workspace["drafts"][0]["state"], {"text": state["text"], "attachments": []})
        self.assertEqual(result["composer"], workspace["drafts"][0]["state"])
        self.assertEqual((self.home / "mac-window-workspaces/synthetic.schema2.backup.json").read_bytes(), original)

    def test_unknown_window_version_refuses_before_catalog_commit(self):
        for window in ({"schemaVersion": 2, "composerWorkspace": {"schemaVersion": 2}},
                       {"composerWorkspace": {"schemaVersion": 4}},
                       {"composerWorkspaceByDevice": {"peer": {"schemaVersion": 4}}}):
            with self.subTest(window=window):
                self.write("mac-window-workspaces/synthetic.json", window)
                original = {path: path.read_bytes() for path in self.home.rglob("*.json")}
                with self.assertRaisesRegex(ValueError, "schema"):
                    self.run_migration()
                self.assertEqual(original, {path: path.read_bytes() for path in self.home.rglob("*.json")})

    def test_future_android_export_refuses_before_catalog_commit(self):
        export = self.home / "synthetic-export.xml"
        export.write_text('<map><string name="future">{"version":3}</string></map>')
        original = {path: path.read_bytes() for path in self.home.rglob("*") if path.is_file()}
        with patch.object(sys, "argv", [str(SCRIPT), "--home", str(self.home),
                                        "--android-preferences", str(export), "--apply"]):
            with self.assertRaisesRegex(ValueError, "Android.*schema"):
                migration.main()
        self.assertEqual(original, {path: path.read_bytes() for path in self.home.rglob("*")
                                    if path.is_file() and path.name != "runtime.lock"})


if __name__ == "__main__":
    unittest.main()
