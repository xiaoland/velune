#!/usr/bin/env python3
"""显式迁移普通配置 schema 10 → 11；默认预览，退出 Velune 后 --apply。

保留模型认证、查询 AK/SK、关联修订及其余配置。只将 quotaAuthentication
中的 accessKey 包装为有类型的 credential；不创建账单配置，不读取原生会话。
文件名前缀使本步骤先于要求 schema 11 的 Codex 适配器迁移执行。
"""
import argparse
import json
import os
from pathlib import Path
import re
import tempfile
import unicodedata


def object_fields(value, allowed, required, label):
    if not isinstance(value, dict) or not required <= value.keys() or value.keys() - allowed:
        raise ValueError(f"{label} 结构不属于已知 schema 10，原文件未修改")


def integer(value):
    return type(value) is int and 0 <= value <= 2**64 - 1


def unique_object(pairs):
    value = {}
    for key, item in pairs:
        if key in value:
            raise ValueError("配置含重复 JSON 字段，原文件未修改")
        value[key] = item
    return value


def reject_constant(_):
    raise ValueError("配置含非标准 JSON 数值，原文件未修改")


def migrate(config):
    object_fields(config, {
        "schemaVersion", "gateways", "runtimeInstances", "modelTemplates",
        "conversationBrowserGroupLimit", "conversationBrowserTotalLimit",
        "outlinePresentation", "linkedConversationOpening", "usageDisplay",
        "contextUsageDisplay", "conversationBrowserPreferences", "statusLine",
    }, {"schemaVersion", "gateways", "runtimeInstances", "modelTemplates"}, "普通配置")
    for field in ("gateways", "runtimeInstances", "modelTemplates"):
        if not isinstance(config[field], list):
            raise ValueError("配置集合格式无效，原文件未修改")
    converted = 0
    for gateway in config["gateways"]:
        object_fields(gateway, {"id", "name", "providers", "routes"}, {"id", "name", "providers"}, "网关")
        if not isinstance(gateway["providers"], list):
            raise ValueError("提供商集合格式无效，原文件未修改")
        for provider in gateway["providers"]:
            object_fields(provider, {
                "id", "name", "protocol", "endpoint", "executionBackend", "requestProfile",
                "quotaSource", "authentication", "quotaAuthentication", "models", "quotaLimits",
            }, {
                "id", "name", "protocol", "endpoint", "executionBackend", "requestProfile",
                "quotaSource", "authentication", "models", "quotaLimits",
            }, "提供商")
            if "quotaAuthentication" not in provider:
                continue  # Both versions default an absent query credential.
            query = provider["quotaAuthentication"]
            object_fields(query, {"accessKey", "generation", "confirmedModelGeneration"}, {"generation"}, "额度查询认证")
            generation = query["generation"]
            confirmed = query.get("confirmedModelGeneration")
            model_generation = provider["authentication"].get("generation") if isinstance(provider["authentication"], dict) else None
            if not integer(generation) or not integer(model_generation) or (confirmed is not None and (not integer(confirmed) or confirmed > model_generation)):
                raise ValueError("查询认证修订无效，原文件未修改")
            key = query.pop("accessKey", None)
            if key is not None:
                object_fields(key, {"id", "secret"}, {"id", "secret"}, "查询 AK/SK")
                identity, secret = key["id"], key["secret"]
                if not isinstance(identity, str) or re.fullmatch(r"[A-Za-z0-9_.-]{1,256}", identity) is None:
                    raise ValueError("查询 AK 标识无效，原文件未修改")
                if not isinstance(secret, str) or not secret or secret.strip() != secret or len(secret.encode()) > 4096 or any(unicodedata.category(char) == "Cc" for char in secret):
                    raise ValueError("查询 SK 格式无效，原文件未修改")
                if provider["quotaSource"] != "arkCodingUsage":
                    raise ValueError("查询 AK/SK 与额度来源不匹配，原文件未修改")
                query["credential"] = {"kind": "accessKeyPair", "key": key}
                converted += 1
            else:
                if confirmed is not None:
                    raise ValueError("没有查询凭据的关联确认无效，原文件未修改")
                query["credential"] = None
    config["schemaVersion"] = 11
    return converted


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--home", type=Path, required=True, help="VELUNE_HOME 状态目录")
    parser.add_argument("--apply", action="store_true", help="保留原文件备份并原子写入")
    args = parser.parse_args()
    root = args.home.expanduser().resolve(strict=True)
    with (root / "runtime.lock").open("a+b") as lock:
        try:
            if os.name == "nt":
                import msvcrt
                lock.seek(0)
                msvcrt.locking(lock.fileno(), msvcrt.LK_NBLCK, 1)
            else:
                import fcntl
                fcntl.flock(lock, fcntl.LOCK_EX | fcntl.LOCK_NB)
        except (BlockingIOError, OSError):
            parser.error("请先退出使用该状态目录的 Velune，再执行迁移")
        path = root / "generic-config.json"
        if not path.exists():
            print("尚无普通配置，无需迁移")
            return
        original = path.read_bytes()
        try:
            config = json.loads(original, object_pairs_hook=unique_object, parse_constant=reject_constant)
            version = config.get("schemaVersion") if isinstance(config, dict) else None
            if type(version) is not int:
                raise ValueError("配置版本缺失或无效，原文件未修改")
            if version == 11:
                print("普通配置已是 schema 11，无需迁移")
                return
            if version != 10:
                raise ValueError("仅支持 schema 10 → 11；请保留原文件并人工核对其他版本")
            converted = migrate(config)
        except (ValueError, UnicodeError) as error:
            # JSON parser excerpts can contain secrets; only report our own errors.
            if isinstance(error, (json.JSONDecodeError, UnicodeError)):
                parser.error("配置不是有效 UTF-8 JSON，原文件未修改")
            parser.error(str(error))
        print(f"普通配置 schema 10 → 11；保留 {converted} 组查询 AK/SK 及原有关联修订")
        if not args.apply:
            print("仅预览，未修改配置；退出 Velune 后加 --apply 执行。")
            return
        backup = Path(tempfile.mkdtemp(prefix="before-config-schema-11-", dir=root))
        with (backup / path.name).open("xb") as output:
            os.chmod(output.name, 0o600)
            output.write(original)
            output.flush()
            os.fsync(output.fileno())
        temporary = None
        try:
            fd, temporary = tempfile.mkstemp(prefix=".config-schema-11-", dir=root)
            with os.fdopen(fd, "w", encoding="utf-8") as output:
                json.dump(config, output, ensure_ascii=False, indent=2, allow_nan=False)
                output.write("\n")
                output.flush()
                os.fsync(output.fileno())
            os.replace(temporary, path)
        finally:
            if temporary and Path(temporary).exists():
                Path(temporary).unlink()
        print(f"已迁移；原文件保存在 {backup.name}。")


if __name__ == "__main__":
    main()
