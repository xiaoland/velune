#!/usr/bin/env python3
"""显式迁移旧 composer 文件；不读取 Harness 历史，写入前保留原数据备份。需先退出 Velune。

预览为默认，--apply 写入。已迁移或没有旧格式数据时以 0 退出，供
scripts/run-migrations.py 在重装时自动执行。
"""
import argparse
import copy
import json
import os
from pathlib import Path
import shutil
import tempfile
from urllib.parse import unquote, urlparse
import uuid
import xml.etree.ElementTree as ET


def pairs(value):
    if isinstance(value, dict):
        return value
    if len(value) % 2:
        raise ValueError("窗口字典的键值数量不匹配")
    return dict(zip(value[::2], value[1::2]))


def model(value):
    if value is None:
        return None
    if set(value) == {"managed"}:
        fields = value["managed"]
        return {"managed": {"model_record_key": fields.get("model_record_key", fields.get("modelRecordKey"))}}
    if set(value) == {"native"}:
        fields = value["native"]
        return {"native": {"model_id": fields.get("model_id", fields.get("modelId")), "provider_id": fields.get("provider_id", fields.get("providerId"))}}
    raise ValueError("无法识别保存的模型选择；原文件保持不变")


def choice(value):
    return {"model": model(value.get("model")), "effort": value.get("effort"), "permissionMode": value.get("permissionMode")}


def empty():
    return {"text": "", "attachments": [], "runtimeId": None, "runtimeOwnerId": None, "choices": [], "choiceEdited": False}


def read_versioned(path, field, default):
    if not path.exists():
        return default
    value = json.loads(path.read_text())
    if value["schemaVersion"] != 1:
        raise ValueError(f"不支持的旧格式：{path.name}")
    return value[field]


def split_combined(value):
    """Copy each bound input's execution choices to its target exactly once."""
    result = copy.deepcopy(value)
    old_inputs = {item["id"]: item for item in value["state"]["inputs"]}
    names = ("runtimeId", "runtimeOwnerId", "choices", "choiceEdited")
    targets = []
    for binding in result["state"]["bindings"]:
        content = old_inputs[binding["inputId"]]["content"]
        targets.append({"context": copy.deepcopy(binding["context"]), "revision": 1,
                        "choices": {name: copy.deepcopy(content[name]) for name in names}})
    for item in result["state"]["inputs"]:
        item["content"] = {name: item["content"][name] for name in ("text", "attachments")}
    result["schemaVersion"] = 2
    result["state"]["revision"] += 1
    result["state"]["targets"] = targets
    return result


def atomic_json(path, value):
    fd, name = tempfile.mkstemp(prefix=".composer-migration-", dir=path.parent)
    try:
        with os.fdopen(fd, "w") as output:
            json.dump(value, output, ensure_ascii=False, indent=2)
            output.flush()
            os.fsync(output.fileno())
        os.replace(name, path)
    finally:
        if os.path.exists(name):
            os.unlink(name)


def windows_caches(home, old, apply):
    # Cache contexts are hashed. A baseline is usable only when all old
    # bindings of this input resolve to the same migrated target revision.
    for path in sorted((home / "composer-cache").glob("*.json")):
        if path.name.endswith(".backup.json"):
            continue
        cache = json.loads(path.read_text())
        if cache.get("Schema") != 1:
            continue
        content = cache["Input"]["Content"]
        choices = {name: copy.deepcopy(content.get(name)) for name in ("RuntimeId", "RuntimeOwnerId", "Choices", "ChoiceEdited")}
        source = next((item for item in old["state"]["inputs"] if item["id"] == cache["Input"]["Id"]), None)
        bound = source is not None and any(item["inputId"] == source["id"] for item in old["state"]["bindings"])
        observed = cache.get("Baseline") if cache["Pending"] else cache["Input"]["Revision"]
        baseline = 1 if bound and observed == source["revision"] else None
        cache.update(Schema=2, TargetChoices=choices, TargetRevision=1 if bound else None,
                     TargetPending=bool(cache["Pending"]), TargetBaseline=baseline)
        cache["Input"]["Content"] = {name: content[name] for name in ("Text", "Attachments")}
        if cache.get("Creation"):
            cache["Creation"]["Choices"] = choices
        if apply:
            backup = path.with_suffix(".schema1.backup.json")
            if not backup.exists():
                shutil.copy2(path, backup)
            atomic_json(path, cache)
        print(f"Windows 本机缓存 {'迁移' if apply else '待迁移'}：{path.name}")


def mac_workspaces(home, old, apply):
    owner = old["state"]["ownerId"]
    inputs = {item["id"]: item for item in old["state"]["inputs"]}
    for path in sorted((home / "mac-window-workspaces").glob("*.json")):
        if path.name.endswith(".backup.json"):
            continue
        window = json.loads(path.read_text())
        workspace = window.get("composerWorkspace")
        if not workspace or workspace.get("schemaVersion") == 3:
            continue
        drafts = {str(uuid.UUID(item["id"])): item for item in workspace["drafts"]}
        targets = workspace.get("pendingTargets") or {}
        for context, input_id in workspace.get("conversations", {}).items():
            if context.startswith(("side-chat:", "tab:")):
                continue
            draft = drafts.get(str(uuid.UUID(input_id)))
            if not draft or not draft.get("pending") or "choices" not in draft["state"]:
                continue
            saved = draft["state"]
            kept = saved["choices"]
            choices = {"runtimeId": kept.get("runtimeID"), "runtimeOwnerId": saved.get("runtimeOwnerID") or (owner if kept.get("runtimeID") else None),
                       "choices": (saved.get("foreignChoices") or []) + [{"ownerId": owner, "runtimeId": runtime, "choice": value} for runtime, value in sorted(kept.get("choices", {}).items())],
                       "choiceEdited": bool(saved.get("choiceEdited"))}
            source = inputs.get(str(uuid.UUID(input_id)))
            baseline = 1 if source and draft.get("revision") == source["revision"] else None
            targets[context] = {"choices": choices, "revision": baseline}
        for draft in workspace["drafts"]:
            draft["state"] = {name: draft["state"][name] for name in ("text", "attachments")}
        workspace["pendingTargets"] = targets
        workspace["schemaVersion"] = 3
        if apply:
            backup = path.with_suffix(".schema2.backup.json")
            if not backup.exists():
                shutil.copy2(path, backup)
            atomic_json(path, window)
        print(f"Mac 窗口输入 {'迁移' if apply else '待迁移'}：{path.name}")


def android_preferences(paths, old, apply):
    """Migrate explicitly exported composer-inputs.xml while Android is stopped."""
    inputs = {item["id"]: item for item in old.get("state", {}).get("inputs", [])}
    bound = {item["inputId"] for item in old.get("state", {}).get("bindings", [])}
    for path in paths:
        path = path.expanduser().resolve(strict=True)
        tree = ET.parse(path)
        changed = 0
        for entry in tree.getroot().findall("string"):
            value = json.loads(entry.text or "null")
            if value.get("version") == 2:
                continue
            if value.get("version") != 1:
                raise ValueError("不支持的 Android 本机输入格式，原文件保持不变")
            source = inputs.get(value["id"])
            observed = value.get("baseline") if value["pending"] else value["revision"]
            baseline = 1 if source and value["id"] in bound and observed is not None and int(observed) == source["revision"] else None
            target = {"runtime": value.get("runtime"), "owner": value.get("runtimeOwner"), "choices": value["choices"], "edited": value["edited"]}
            for name in ("runtime", "runtimeOwner", "choices", "edited"):
                value.pop(name, None)
            value.update(version=2, target=target, targetRevision="1" if value["id"] in bound else None,
                         targetPending=bool(value["pending"]), targetBaseline=str(baseline) if baseline is not None else None)
            if value.get("creation"):
                value["creation"]["choices"] = target
            entry.text = json.dumps(value, ensure_ascii=False)
            changed += 1
        if apply and changed:
            backup = path.with_suffix(".schema1.backup.xml")
            if not backup.exists():
                shutil.copy2(path, backup)
            fd, name = tempfile.mkstemp(prefix=".composer-migration-", dir=path.parent)
            with os.fdopen(fd, "wb") as output:
                tree.write(output, encoding="utf-8", xml_declaration=True)
                output.flush()
                os.fsync(output.fileno())
            os.replace(name, path)
        print(f"Android 本机输入 {'迁移' if apply else '待迁移'} {changed} 份：{path}")


def migrate_combined(home, target, value, apply):
    if value.get("schemaVersion") == 2:
        # A prior interruption may have committed the catalog before caches.
        backup = home / "composer-inputs.schema1.backup.json"
        if backup.exists():
            windows_caches(home, json.loads(backup.read_text()), apply)
            mac_workspaces(home, json.loads(backup.read_text()), apply)
        print("composer-inputs.json 已为分离格式，未覆盖")
        return
    if value.get("schemaVersion") != 1:
        raise ValueError("不支持的 composer 目录格式，原文件保持不变")
    result = split_combined(value)
    print(f"内容 {len(result['state']['inputs'])} 份；目标配置 {len(result['state']['targets'])} 份；孤立内容保留，旧选择保存在完整备份")
    if apply:
        backup = home / "composer-inputs.schema1.backup.json"
        if backup.exists() and json.loads(backup.read_text()) != value:
            raise ValueError("已有不同的 schema1 备份，未覆盖；请先人工核对")
        if not backup.exists():
            shutil.copy2(target, backup)
        atomic_json(target, result)
    windows_caches(home, value, apply)
    mac_workspaces(home, value, apply)
    print("迁移完成；原目录已完整备份。" if apply else "仅检查，未写入；加 --apply 执行。")


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--home", required=True, type=Path, help="需要迁移的 VELUNE_HOME")
    parser.add_argument("--android-preferences", type=Path, action="append", default=[], help="已显式导出的 Android composer-inputs.xml；应用须停止，迁移后由操作者放回")
    parser.add_argument("--apply", action="store_true", help="写入迁移结果；缺省只检查与统计")
    args = parser.parse_args()
    home = args.home.expanduser().resolve(strict=True)
    target = home / "composer-inputs.json"
    with (home / "runtime.lock").open("a+b") as lock:
        if os.name == "nt":
            import msvcrt
            lock.seek(0)
            msvcrt.locking(lock.fileno(), msvcrt.LK_NBLCK, 1)
        else:
            import fcntl
            fcntl.flock(lock.fileno(), fcntl.LOCK_EX | fcntl.LOCK_NB)
        if target.exists():
            value = json.loads(target.read_text())
            migrate_combined(home, target, value, args.apply)
            backup = home / "composer-inputs.schema1.backup.json"
            source = value if value.get("schemaVersion") == 1 else json.loads(backup.read_text()) if backup.exists() else {}
            android_preferences(args.android_preferences, source, args.apply)
            return
        journal = home / "mac-window-workspaces"
        legacy = [(home / name).exists() for name in ("composer-choices.json", "composer-drafts.json")]
        if not any(legacy):
            for path in sorted(journal.glob("*.json")):
                window = json.loads(path.read_text())
                if window.get("composerWorkspace") or window.get("composer"):
                    legacy.append(True)
                    break
        if not any(legacy):
            print("没有旧格式的 composer 数据，无需迁移")
            return
        owner = str(uuid.uuid4())
        records, edited, links, link_dates = {}, {}, {}, {}
        origins = {}
        attachments, sources = {}, {}
        missing = 0

        def context(conversation):
            return None if conversation.startswith("draft:") else conversation

        def bind(key, input_id, timestamp):
            if timestamp >= link_dates.get(key, -1):
                links[key], link_dates[key] = input_id, timestamp

        def file_reference(value):
            nonlocal missing
            parsed = urlparse(value)
            if parsed.scheme and parsed.scheme != "file":
                raise ValueError("旧附件不是本地文件，未迁移")
            path = Path(unquote(parsed.path) if parsed.scheme else value)
            if os.name == "nt" and parsed.scheme and path.as_posix().startswith("/"):
                path = Path(str(path)[1:])
            if not path.is_absolute():
                raise ValueError("旧附件路径必须为绝对路径，原文件保持不变")
            key = str(path)
            if key in sources:
                return sources[key][0]
            name = path.name
            if not name or name in (".", "..") or "/" in name or "\\" in name or len(name.encode()) > 240 or any(ord(c) < 32 for c in name):
                raise ValueError("旧附件名称无效，原文件保持不变")
            exists = path.is_file()
            size = path.stat().st_size if exists else 0
            if size > 20 * 1024 * 1024:
                raise ValueError(f"附件 {name} 超过现有 20 MiB 传输上限，未丢弃；请保留旧版并处理该附件后再迁移")
            if not exists:
                missing += 1
            reference = {"id": str(uuid.uuid4()), "name": name, "bytes": size}
            attachments[reference["id"]] = reference
            sources[key] = (reference, path if exists else None)
            return reference

        for path in sorted(journal.glob("*.json")):
            window = json.loads(path.read_text())
            workspace = window.get("composerWorkspace")
            if not workspace:
                if window.get("composer"):
                    raise ValueError("发现更早的无输入身份窗口格式，请先保留并人工核对，未重置")
                continue
            when = path.stat().st_mtime
            for draft in workspace["drafts"]:
                input_id = str(uuid.UUID(draft["id"]))
                timestamp = float(draft["editedAt"]) + 978307200 if draft.get("editedAt") is not None else when
                if timestamp >= edited.get(input_id, -1):
                    saved = draft["state"]
                    kept = saved["choices"]
                    content = {"text": saved["text"], "attachments": [file_reference(value) for value in saved["attachments"]],
                               "runtimeId": kept.get("runtimeID"), "runtimeOwnerId": owner if kept.get("runtimeID") else None, "choices": [{"ownerId": owner, "runtimeId": key, "choice": choice(value)} for key, value in sorted(kept.get("choices", {}).items())],
                               "choiceEdited": bool(saved.get("choiceEdited"))}
                    records[input_id] = {"id": input_id, "revision": 1, "content": content}
                    edited[input_id] = timestamp
            for conversation, input_id in workspace["conversations"].items():
                input_id = str(uuid.UUID(input_id))
                if input_id in records and not conversation.startswith(("side-chat:", "tab:")):
                    origins.setdefault(context(conversation), set()).add(input_id)
                    bind(context(conversation), input_id, when)
            # A shared input is the actual work at the focused target, even when
            # the old per-session association contains another input.
            panes = pairs(window["panes"])
            focused = next((value for key, value in panes.items() if key.lower() == window["focused"].lower()), None)
            tab_conversations = pairs(window.get("conversations", {}))
            conversation = next((value for key, value in tab_conversations.items() if focused and key.lower() == focused["selected"].lower()), None)
            shared_id = str(uuid.UUID(workspace["shared"]))
            tools = pairs(window.get("tools", {}).get("tools", {}))
            # Absent placement metadata is not evidence that the unused shared
            # buffer should replace an inline conversation's input.
            composer = tools.get("composer", {})
            place = composer.get("place", {})
            shared_visible = any(key in place for key in ("docked", "floating", "window"))
            if shared_visible and conversation and shared_id in records:
                bind(context(conversation), shared_id, when + 0.001)

        intents = read_versioned(home / "composer-choices.json", "choices", {"conversations": []})["conversations"]
        choices = {item["conversationID"]: item for item in intents}
        texts = {item["conversationID"]: item["text"] for item in read_versioned(home / "composer-drafts.json", "drafts", [])}
        for conversation in sorted(set(choices) | set(texts)):
            key = context(conversation)
            text = texts.get(conversation, "")
            if key in links:
                known = origins.get(key, set()) | {links[key]}
                if not text or any(records[input]["content"]["text"] == text for input in known):
                    continue
            kept = choices.get(conversation, {})
            content = empty()
            content.update(text=text, runtimeId=kept.get("runtimeID"), runtimeOwnerId=owner if kept.get("runtimeID") else None, choices=[{"ownerId": owner, "runtimeId": item["runtimeID"], "choice": item["choice"]} for item in kept.get("choices", [])], choiceEdited=bool(kept.get("choices")))
            input_id = str(uuid.uuid4())
            records[input_id] = {"id": input_id, "revision": 1, "content": content}
            if key not in links:
                links[key] = input_id
        if None not in links:
            input_id = str(uuid.uuid4())
            records[input_id] = {"id": input_id, "revision": 1, "content": empty()}
            links[None] = input_id
        bindings = [{"context": {"kind": "newConversation"} if key is None else {"kind": "conversation", "ownerId": owner, "conversationId": key}, "inputId": input_id, "revision": 1} for key, input_id in links.items()]
        result = {"schemaVersion": 1, "state": {"ownerId": owner, "revision": 1, "inputs": list(records.values()), "bindings": bindings, "creations": []}, "attachments": list(attachments.values())}
        combined = result
        result = split_combined(result)
        print(f"输入 {len(records)} 份；上下文 {len(bindings)} 个；全局新建项 1 个；附件 {len(attachments)} 个（保留失效引用 {missing} 个）")
        if not args.apply:
            print("仅检查，未写入输入或附件；加 --apply 执行。")
            return
        root = home / "composer-attachments"
        for reference, source in sources.values():
            if source is None:
                continue
            directory = root / reference["id"]
            directory.mkdir(parents=True, mode=0o700)
            destination = directory / reference["name"]
            shutil.copyfile(source, destination)
            destination.chmod(0o600)
        fd, name = tempfile.mkstemp(prefix=".composer-inputs-", dir=home)
        with os.fdopen(fd, "w") as output:
            json.dump(result, output, ensure_ascii=False, indent=2)
            output.flush()
            os.fsync(output.fileno())
        os.replace(name, target)
        android_preferences(args.android_preferences, combined, True)
        mac_workspaces(home, combined, True)
        windows_caches(home, combined, True)
        print("迁移完成；旧配置、窗口快照及原附件均保留。")


if __name__ == "__main__":
    main()
