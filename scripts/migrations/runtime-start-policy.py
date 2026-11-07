#!/usr/bin/env python3
"""显式把新会话策略归入实例配置，并为 lastChosen 标记 Adapter Type。

generic-config 保持 schema 11，策略写入实例 settings.newConversation；
composer-choices 使用 schema 2，只保存带类型的 lastChosen 和原有历史意图。
先提交实例文件，后提交 composer：中断时旧 composer 仍是恢复源，重跑只接受
与它相同的已写策略。需退出占用此目录的 Velune；默认预览，--apply 写入。
"""
import argparse
import copy
import json
import os
from pathlib import Path
import tempfile


KINDS = {"pi-1.0", "codex-0.166", "claude-code-2.1", "dsh-acp-0.2", "acp-v1"}
POLICY = "newConversation"


def unique_object(pairs):
    result = {}
    for key, value in pairs:
        if key in result:
            raise ValueError("配置含重复 JSON 字段，原文件未修改")
        result[key] = value
    return result


def load_bytes(data):
    def reject_constant(_):
        raise ValueError("配置含非标准 JSON 数值，原文件未修改")
    return json.loads(data, object_pairs_hook=unique_object, parse_constant=reject_constant)


def validate_choice(choice):
    if not isinstance(choice, dict) or choice.keys() - {"model", "effort", "permissionMode"}:
        raise ValueError("选择格式未知；请先完成旧 serviceTier 迁移")
    if choice.get("model") is not None and not isinstance(choice["model"], dict):
        raise ValueError("模型选择格式无效，原文件未修改")
    if any(choice.get(key) is not None and not isinstance(choice[key], str) for key in ("effort", "permissionMode")):
        raise ValueError("选择参数格式无效，原文件未修改")


def validate_policy(policy):
    if policy == {"kind": "lastChosen"}:
        return
    if not isinstance(policy, dict) or policy.keys() != {"kind", "choice"} or policy["kind"] != "chosen":
        raise ValueError("新会话策略格式无效，原文件未修改")
    validate_choice(policy["choice"])


def convert(config, composer):
    config, composer = copy.deepcopy(config), copy.deepcopy(composer)
    if config.get("schemaVersion") != 11 or not isinstance(config.get("runtimeInstances"), list):
        raise ValueError("普通配置不是 schema 11；请先运行完整迁移入口")
    if composer.get("schemaVersion") not in (1, 2) or not isinstance(composer.get("choices"), dict):
        raise ValueError("composer 选择格式未知，原文件未修改")
    choices = composer["choices"]
    if not isinstance(choices.get("runtimes"), list) or not isinstance(choices.get("conversations"), list):
        raise ValueError("composer 选择集合格式无效，原文件未修改")
    runtimes = {}
    for runtime in config["runtimeInstances"]:
        if not isinstance(runtime, dict) or not isinstance(runtime.get("id"), str) or not runtime["id"] or runtime["id"] in runtimes or runtime.get("typeId") not in KINDS or not isinstance(runtime.get("settings"), dict):
            raise ValueError("运行时身份、类型或设置无效，原文件未修改")
        if POLICY in runtime["settings"]:
            validate_policy(load_bytes(runtime["settings"][POLICY]))
        runtimes[runtime["id"]] = runtime
    seen = set()
    for remembered in choices["runtimes"]:
        if not isinstance(remembered, dict) or remembered.get("runtimeID") not in runtimes or remembered["runtimeID"] in seen:
            raise ValueError("运行时选择孤立或重复，需人工核对，原文件未修改")
        seen.add(remembered["runtimeID"])
        validate_choice(remembered.get("lastChosen"))
        if composer["schemaVersion"] == 2:
            if remembered.keys() != {"runtimeID", "runtimeType", "lastChosen"} or remembered.get("runtimeType") not in KINDS:
                raise ValueError("schema 2 的选择格式无效，原文件未修改")
            continue
        if remembered.keys() != {"runtimeID", "lastChosen", POLICY}:
            raise ValueError("旧运行时选择字段未知，原文件未修改")
        policy = remembered.pop(POLICY)
        validate_policy(policy)
        runtime = runtimes[remembered["runtimeID"]]
        current = runtime["settings"].get(POLICY)
        if current is not None and load_bytes(current) != policy:
            raise ValueError("实例已有不同的新会话策略，需人工核对，原文件未修改")
        runtime["settings"][POLICY] = json.dumps(policy, ensure_ascii=False, separators=(",", ":"))
        remembered["runtimeType"] = runtime["typeId"]
    if composer["schemaVersion"] == 1:
        for runtime in runtimes.values():
            runtime["settings"].setdefault(POLICY, '{"kind":"lastChosen"}')
        composer["schemaVersion"] = 2
    return config, composer


def atomic_json(path, value):
    fd, name = tempfile.mkstemp(prefix=".runtime-start-policy-", dir=path.parent)
    try:
        with os.fdopen(fd, "w", encoding="utf-8") as output:
            os.chmod(name, 0o600)
            json.dump(value, output, ensure_ascii=False, indent=2, allow_nan=False)
            output.write("\n")
            output.flush()
            os.fsync(output.fileno())
        os.replace(name, path)
    finally:
        if os.path.exists(name):
            os.unlink(name)


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
            parser.error("状态目录仍被占用，请先退出占用它的 Velune")
        config_path, composer_path = home / "generic-config.json", home / "composer-choices.json"
        if not composer_path.exists():
            print("没有旧运行时默认选择，无需迁移")
            return
        try:
            original_composer = composer_path.read_bytes()
            original_config = config_path.read_bytes() if config_path.exists() else None
            config = load_bytes(original_config) if original_config is not None else {"schemaVersion": 11, "runtimeInstances": []}
            composer = load_bytes(original_composer)
            next_config, next_composer = convert(config, composer)
        except (ValueError, TypeError, KeyError, UnicodeError) as problem:
            if isinstance(problem, (json.JSONDecodeError, UnicodeError, TypeError, KeyError)):
                parser.error("配置不是已知 UTF-8 JSON 格式，原文件未修改")
            parser.error(str(problem))
        if next_config == config and next_composer == composer:
            print("运行时策略与带类型的 lastChosen 已迁移，无需修改")
            return
        print(f"迁移 {len(next_config['runtimeInstances'])} 个实例的新会话策略；保留 lastChosen、历史目标与正文附件")
        if not args.apply:
            print("仅预览，未修改配置")
            return
        backup = Path(tempfile.mkdtemp(prefix="before-runtime-start-policy-", dir=home))
        for path, data in ((config_path, original_config), (composer_path, original_composer)):
            if data is None:
                continue
            with (backup / path.name).open("xb") as output:
                os.chmod(output.name, 0o600)
                output.write(data)
                output.flush()
                os.fsync(output.fileno())
        # Old schema 1 remains a recovery source until the complete instance policy is committed.
        if next_config != config:
            atomic_json(config_path, next_config)
        atomic_json(composer_path, next_composer)
        print(f"已迁移；原文件保存在 {backup.name}")


if __name__ == "__main__":
    main()
