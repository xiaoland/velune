#!/usr/bin/env python3
"""显式迁移旧 composer 文件；不读取 Harness 历史，不改原文件。需先退出 Velune。

预览为默认，--apply 写入。已迁移或没有旧格式数据时以 0 退出，供
scripts/run-migrations.py 在重装时自动执行。
"""
import argparse
import json
import os
from pathlib import Path
import shutil
import tempfile
from urllib.parse import unquote, urlparse
import uuid


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


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--home", required=True, type=Path, help="需要迁移的 VELUNE_HOME")
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
            print("composer-inputs.json 已存在，视为已迁移，未改动")
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
        print("迁移完成；旧配置、窗口快照及原附件均保留。")


if __name__ == "__main__":
    main()
