#!/usr/bin/env python3
"""显式将旧按轮 serviceTier 转为模型变体；默认预览，退出 Velune 后 --apply。

空档位只移除已撤销字段；非空档位仅在原生 Codex 模型身份明确时转换。
无法无损转换时整批拒绝，不猜默认模型、不丢弃已保存的档位。
文件名前缀保证先于 composer-state.py，避免旧窗口选择被重新编码时丢失档位。
"""
import argparse
import json
import os
from pathlib import Path
import tempfile
import unicodedata


PREFIX = "velune:codex-speed:"
TIER_FIELDS = {"serviceTier", "ServiceTier"}
CHOICE_FIELDS = {"model", "effort", "permissionMode", "serviceTier"}
SETTINGS_FIELDS = CHOICE_FIELDS | {"runtimeId", "modelSelection"}
WINDOWS_CHOICE_FIELDS = {"Model", "Effort", "PermissionMode", "ServiceTier"}
RUNTIME_FIELDS = ("runtimeId", "runtimeID", "RuntimeId")


def unique_object(pairs):
    value = {}
    for key, item in pairs:
        if key in value:
            raise ValueError("JSON 含重复字段，原文件未修改")
        value[key] = item
    return value


def reject_constant(_):
    raise ValueError("JSON 含非标准数值，原文件未修改")


def read_json(path):
    try:
        return json.loads(path.read_bytes(), object_pairs_hook=unique_object, parse_constant=reject_constant)
    except (json.JSONDecodeError, UnicodeError):
        # Parser excerpts can contain user content or credentials.
        raise ValueError(f"{path.name} 不是有效 UTF-8 JSON，原文件未修改") from None


def native_model(selection):
    """Return the existing native model container/key; preserve its encoding."""
    if not isinstance(selection, dict):
        return None
    if set(selection) == {"native"} and isinstance(selection["native"], dict):
        native = selection["native"]
        if set(native) <= {"provider_id", "model_id"} and "model_id" in native:
            return native, "model_id"
        if set(native) <= {"providerId", "modelId"} and "modelId" in native:
            return native, "modelId"
    if selection.get("kind") == "native" and set(selection) <= {"kind", "provider", "model"}:
        return selection, "model"
    return None


def convert(value, runtimes, local_owner, runtime=None, owner=None):
    """Visit only metadata files; a field in an unknown shape stops the batch."""
    removed, variants = 0, 0
    if isinstance(value, list):
        for item in value:
            empty, converted = convert(item, runtimes, local_owner, runtime, owner)
            removed += empty
            variants += converted
        return removed, variants
    if not isinstance(value, dict):
        return removed, variants
    runtime = next((value[key] for key in RUNTIME_FIELDS if value.get(key) is not None), runtime)
    owner = value.get("ownerId", value.get("OwnerId", owner))
    fields = TIER_FIELDS & value.keys()
    if fields:
        if len(fields) != 1 or not (
            value.keys() <= SETTINGS_FIELDS or value.keys() <= WINDOWS_CHOICE_FIELDS
        ):
            raise ValueError("旧档位位于不认识的配置结构中，原文件未修改；请先人工核对")
        field = next(iter(fields))
        tier = value[field]
        if tier is None:
            removed += 1
        else:
            if not isinstance(tier, str) or not tier or tier.strip() != tier or any(unicodedata.category(char) == "Cc" for char in tier):
                raise ValueError("旧档位值无效，原文件未修改；请先人工核对")
            if not isinstance(runtime, str) or runtimes.get(runtime) not in {"codex-0.159", "codex-0.166"} or (owner is not None and owner != local_owner):
                raise ValueError("非空旧档位缺少可核实的本机 Codex 实例，原文件未修改；请先在原执行设备核对")
            selection = value.get("modelSelection", value.get("Model", value.get("model")))
            native = native_model(selection)
            if native is None:
                raise ValueError("非空旧档位没有明确的原生模型，原文件未修改；请先将该选择改为明确的原生模型或在模型记录中配置等价请求参数")
            container, key = native
            base = container.get(key)
            if not isinstance(base, str) or not base or any(unicodedata.category(char) == "Cc" for char in base):
                raise ValueError("旧原生模型身份无效，原文件未修改")
            if base.startswith(PREFIX):
                try:
                    previous = json.loads(base[len(PREFIX):])
                except ValueError:
                    previous = None
                if not isinstance(previous, list) or len(previous) != 2 or not all(isinstance(part, str) and part for part in previous):
                    raise ValueError("模型变体身份无法核实，原文件未修改")
                if previous[1] != tier:
                    raise ValueError("模型变体与旧档位冲突，原文件未修改；请先人工核对")
                base = previous[0]
            else:
                container[key] = PREFIX + json.dumps([base, tier], ensure_ascii=False, separators=(",", ":"))
            if "modelSelection" in value:
                value["model"] = f"{base} · {tier}"
            variants += 1
        del value[field]
    for key, item in value.items():
        # Legacy Mac ConversationChoices stores a dictionary keyed by runtime ID.
        if key == "choices" and isinstance(item, dict) and all(
            isinstance(choice, dict) and choice.keys() <= CHOICE_FIELDS for choice in item.values()
        ):
            for runtime_id, choice in item.items():
                empty, converted = convert(choice, runtimes, local_owner, runtime_id, owner)
                removed += empty
                variants += converted
        else:
            empty, converted = convert(item, runtimes, local_owner, runtime, owner)
            removed += empty
            variants += converted
    return removed, variants


def files(home):
    for name in ("composer-inputs.json", "composer-choices.json", "turn-requests.json"):
        path = home / name
        if path.exists():
            yield path
    for directory in ("mac-window-workspaces", "composer-cache"):
        for path in sorted((home / directory).glob("*.json")):
            if not path.name.endswith(".backup.json"):
                yield path


def validate_schema(path, value):
    if not isinstance(value, dict):
        raise ValueError(f"{path.name} 不是已知配置结构，原文件未修改")
    if path.name in {"composer-inputs.json", "composer-choices.json", "turn-requests.json"}:
        allowed = {1, 2} if path.name == "composer-inputs.json" else {1}
        version = value.get("schemaVersion")
        if type(version) is not int or version not in allowed:
            raise ValueError(f"{path.name} 不是已知 schema，原文件未修改")
    elif path.parent.name == "composer-cache":
        if type(value.get("Schema")) is not int or value["Schema"] not in {1, 2}:
            raise ValueError("Windows 本机输入不是已知 schema，原文件未修改")
    elif path.parent.name == "mac-window-workspaces":
        workspace = value.get("composerWorkspace")
        if workspace is not None:
            if not isinstance(workspace, dict):
                raise ValueError("Mac 窗口输入不是已知结构，原文件未修改")
            version = workspace.get("schemaVersion")
            if version is not None and (type(version) is not int or version not in {1, 2, 3}):
                raise ValueError("Mac 窗口输入不是已知 schema，原文件未修改")


def atomic_json(path, value):
    fd, temporary = tempfile.mkstemp(prefix=".service-tier-model-variants-", dir=path.parent)
    try:
        with os.fdopen(fd, "w", encoding="utf-8") as output:
            json.dump(value, output, ensure_ascii=False, indent=2, allow_nan=False)
            output.write("\n")
            output.flush()
            os.fsync(output.fileno())
        os.replace(temporary, path)
    finally:
        if os.path.exists(temporary):
            os.unlink(temporary)


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--home", type=Path, required=True, help="VELUNE_HOME 状态目录")
    parser.add_argument("--apply", action="store_true", help="保留原文件备份并原子写入")
    args = parser.parse_args()
    home = args.home.expanduser().resolve(strict=True)
    try:
        with (home / "runtime.lock").open("a+b") as lock:
            try:
                if os.name == "nt":
                    import msvcrt
                    lock.seek(0)
                    msvcrt.locking(lock.fileno(), msvcrt.LK_NBLCK, 1)
                else:
                    import fcntl
                    fcntl.flock(lock, fcntl.LOCK_EX | fcntl.LOCK_NB)
            except (BlockingIOError, OSError):
                raise ValueError("请先退出使用该状态目录的 Velune，再执行迁移") from None
            config_path = home / "generic-config.json"
            config = read_json(config_path) if config_path.exists() else {}
            if not isinstance(config, dict) or not isinstance(config.get("runtimeInstances", []), list):
                raise ValueError("普通配置中的运行时目录无效，原文件未修改")
            runtimes = {}
            for item in config.get("runtimeInstances", []):
                if not isinstance(item, dict) or not isinstance(item.get("id"), str) or not isinstance(item.get("typeId"), str) or item["id"] in runtimes:
                    raise ValueError("普通配置中的运行时身份无效或重复，原文件未修改")
                runtimes[item["id"]] = item["typeId"]
            input_path = home / "composer-inputs.json"
            local_owner = None
            if input_path.exists():
                inputs = read_json(input_path)
                validate_schema(input_path, inputs)
                state = inputs.get("state")
                if not isinstance(state, dict):
                    raise ValueError("输入目录结构无效，原文件未修改")
                local_owner = state.get("ownerId")
            changes = []
            for path in files(home):
                original = path.read_bytes()
                value = read_json(path)
                validate_schema(path, value)
                empty, variants = convert(value, runtimes, local_owner)
                if empty or variants:
                    changes.append((path, original, value))
                    print(f"{path.relative_to(home)}：移除 {empty} 个空档位，转换 {variants} 个原生模型变体")
            if not changes:
                print("没有旧按轮档位，无需迁移")
                return
            if not args.apply:
                print("仅预览，未修改配置；退出 Velune 后加 --apply 执行。")
                return
            backup = Path(tempfile.mkdtemp(prefix="before-service-tier-model-variants-", dir=home))
            for path, original, _ in changes:
                target = backup / path.relative_to(home)
                target.parent.mkdir(parents=True, exist_ok=True, mode=0o700)
                with target.open("xb") as output:
                    os.chmod(target, 0o600)
                    output.write(original)
                    output.flush()
                    os.fsync(output.fileno())
            # Backup every source before the first write. Each replacement is
            # atomic; rerunning safely finishes an interrupted multi-file batch.
            for path, _, value in changes:
                atomic_json(path, value)
            print(f"已迁移；原文件保存在 {backup.name}。")
    except (ValueError, KeyError, TypeError) as error:
        parser.error(str(error) if isinstance(error, ValueError) else "配置结构无效，原文件未修改；请先人工核对")


if __name__ == "__main__":
    main()
