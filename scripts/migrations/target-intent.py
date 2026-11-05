#!/usr/bin/env python3
"""显式移出未手选目标中由旧客户端固化的默认值，保留真实选择与输入。

数据结构仍为 schema 2。只处理 choiceEdited=false 的已知目标记录，
启动新版前执行，默认预览。原文件在任何写入前完整私有备份。
"""
import argparse
import copy
import json
import os
from pathlib import Path
import tempfile


def unique_object(pairs):
    value = {}
    for key, item in pairs:
        if key in value:
            raise ValueError("JSON 含重复字段，原文件未修改")
        value[key] = item
    return value


def load(path):
    def reject_constant(_):
        raise ValueError("JSON 含非标准数值，原文件未修改")
    return json.loads(path.read_bytes(), object_pairs_hook=unique_object, parse_constant=reject_constant)


def inherited(value, pascal=False):
    if not isinstance(value, dict):
        return None
    runtime, owner, choices, edited = (
        ("RuntimeId", "RuntimeOwnerId", "Choices", "ChoiceEdited")
        if pascal else ("runtimeId", "runtimeOwnerId", "choices", "choiceEdited")
    )
    if value.get(edited) is not False:
        return None
    if not isinstance(value.get(choices), list):
        raise ValueError("目标选择集合格式无效，原文件未修改")
    result = copy.deepcopy(value)
    result.update({runtime: None, owner: None, choices: []})
    return result


def directory(value):
    result = copy.deepcopy(value)
    if result.get("schemaVersion") != 2:
        raise ValueError("输入目录不是 schema 2，请先完成 composer-state 迁移")
    state = result["state"]
    changed = False
    for target in state["targets"]:
        choices = inherited(target["choices"])
        if choices is not None and choices != target["choices"]:
            target["choices"] = choices
            target["revision"] = advance_revision(target["revision"])
            changed = True
    if changed:
        state["revision"] = advance_revision(state["revision"])
    return result


def advance_revision(value):
    if type(value) is not int or not 0 <= value < 2**64 - 1:
        raise ValueError("修订值无法推进，原文件未修改")
    return value + 1


def workspace(value):
    result = copy.deepcopy(value)
    saved = result.get("composerWorkspace")
    if not isinstance(saved, dict):
        return result
    pending = saved.get("pendingTargets")
    if isinstance(pending, dict):
        saved["pendingTargets"] = {
            key: record for key, record in pending.items()
            if inherited(record.get("choices")) is None
        }
    for promotion in (saved.get("pendingPromotions") or {}).values():
        choices = inherited(promotion.get("choices"))
        if choices is not None:
            promotion["choices"] = choices
    return result


def windows_cache(value):
    result = copy.deepcopy(value)
    if result.get("Schema") != 2:
        return result
    choices = inherited(result.get("TargetChoices"), pascal=True)
    if choices is not None:
        result["TargetChoices"] = choices
        result["TargetPending"] = False
        result["TargetBaseline"] = None
    creation = result.get("Creation")
    if isinstance(creation, dict):
        choices = inherited(creation.get("Choices"), pascal=True)
        if choices is not None:
            creation["Choices"] = choices
    return result


def atomic(path, data):
    descriptor, temporary = tempfile.mkstemp(prefix=".target-intent-", dir=path.parent)
    try:
        with os.fdopen(descriptor, "wb") as output:
            os.chmod(temporary, 0o600)
            output.write(data)
            output.flush()
            os.fsync(output.fileno())
        os.replace(temporary, path)
    finally:
        if os.path.exists(temporary):
            os.unlink(temporary)


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--home", type=Path, required=True)
    parser.add_argument("--apply", action="store_true")
    args = parser.parse_args()
    home = args.home.expanduser().resolve(strict=True)
    with (home / "runtime.lock").open("a+b") as lock:
        try:
            if os.name == "nt":
                import msvcrt
                lock.seek(0)
                msvcrt.locking(lock.fileno(), msvcrt.LK_NBLCK, 1)
            else:
                import fcntl
                fcntl.flock(lock.fileno(), fcntl.LOCK_EX | fcntl.LOCK_NB)
        except (BlockingIOError, OSError):
            parser.error("状态目录仍被占用，请正常退出 Velune 后执行迁移")
        candidates = [(home / "composer-inputs.json", directory)]
        candidates += [(path, workspace) for path in sorted((home / "mac-window-workspaces").glob("*.json"))
                       if not path.name.endswith(".backup.json")]
        candidates += [(path, windows_cache) for path in sorted((home / "composer-cache").glob("*.json"))
                       if not path.name.endswith(".backup.json")]
        changes = []
        try:
            for path, convert in candidates:
                if not path.exists():
                    continue
                original = path.read_bytes()
                value = load(path)
                converted = convert(value)
                if converted != value:
                    data = (json.dumps(converted, ensure_ascii=False, indent=2, allow_nan=False) + "\n").encode()
                    changes.append((path, original, data))
        except (ValueError, TypeError, KeyError, UnicodeError):
            parser.error("目标记录不是已知格式，原文件未修改；请核对输入目录及平台缓存")
        if not changes:
            print("未手选目标已沿用默认，无需迁移")
            return
        print(f"移出 {len(changes)} 个文件中的旧自动选择；正文、附件、绑定与明确选择保留")
        if not args.apply:
            print("仅预览，未修改配置")
            return
        backup = Path(tempfile.mkdtemp(prefix="before-target-intent-", dir=home))
        for path, original, _ in changes:
            destination = backup / path.relative_to(home)
            destination.parent.mkdir(parents=True, exist_ok=True, mode=0o700)
            with destination.open("xb") as output:
                os.chmod(output.name, 0o600)
                output.write(original)
                output.flush()
                os.fsync(output.fileno())
        for path, _, data in changes:
            atomic(path, data)
        print(f"已迁移，完整原文件保存在 {backup.name}")


if __name__ == "__main__":
    main()
