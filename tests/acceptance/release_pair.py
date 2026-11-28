#!/usr/bin/env python3
"""Verify direct remote control while one already paired macOS endpoint upgrades."""

import argparse
from contextlib import ExitStack, suppress
from importlib import import_module
import json
import os
from pathlib import Path
import platform
import re
import select
import shutil
import socket
import subprocess
import sys
import tempfile
import tomllib
from uuid import uuid4
from zipfile import BadZipFile, ZipFile

from velune_acceptance import bindings
from velune_acceptance.isolation import REPO


LIBRARY = "Contents/Frameworks/libvelune_bindings.dylib"
MANIFEST = "Contents/Resources/build-manifest.json"
SHARED_PROTOCOLS = {"CHAT_COMPLETIONS", "RESPONSES", "MESSAGES"}
PROBE_NAME = "Release pair acceptance"


def require(condition, explanation):
    if not condition:
        raise RuntimeError(explanation)


def prepare_artifact(path, expected_hash, destination, generator):
    path = path.resolve(strict=True)
    destination.mkdir()
    library = destination / "libvelune_bindings.dylib"
    archive_hash = None
    if path.suffix == ".zip":
        require(expected_hash is not None, f"ZIP 需要来自发行清单的 SHA-256：{path}")
        archive_hash = bindings.digest(path)
        require(archive_hash == expected_hash.lower(), f"ZIP SHA-256 不匹配：{path}")
        with ZipFile(path) as archive:
            names = archive.namelist()
            library_name = f"Velune.app/{LIBRARY}"
            manifest_name = f"Velune.app/{MANIFEST}"
            require(names.count(library_name) == names.count(manifest_name) == 1,
                    "ZIP 必须各包含一份日常包的动态库和构建清单")
            manifest = json.loads(archive.read(manifest_name))
            with archive.open(library_name) as source, library.open("xb") as output:
                shutil.copyfileobj(source, output)
        require(bindings.digest(path) == archive_hash, "读取期间发行 ZIP 已变化")
        require(manifest.get("purpose") == "daily" and manifest.get("dirty") is False,
                "ZIP 中的应用必须来自干净的日常构建")
    else:
        require(path.suffix == ".app" and path.is_dir() and expected_hash is None,
                "输入须为 .app，或附发行 SHA-256 的 .zip")
        manifest = json.loads((path / MANIFEST).read_bytes())
        source = path / LIBRARY
        require(source.is_file() and not source.is_symlink(), f"应用缺少真实动态库：{source}")
        shutil.copyfile(source, library)
        require(bindings.digest(source) == bindings.digest(library)
                and json.loads((path / MANIFEST).read_bytes()) == manifest, "读取期间 .app 构建产物已变化")
    architecture = platform.machine()
    archs = subprocess.check_output(["lipo", "-archs", str(library)], text=True).split()
    require(architecture in archs and manifest.get("architecture") == architecture, "动态库架构与本机不匹配")
    commit = manifest.get("source_commit")
    version = manifest.get("ui_version")
    require(isinstance(commit, str) and re.fullmatch(r"[0-9a-f]{40}", commit)
            and isinstance(version, str) and version, "构建清单缺少有效的提交或版本")
    generator_manifest = tomllib.loads((REPO / "tools/bindgen/Cargo.toml").read_text())
    uniffi = generator_manifest["dependencies"]["uniffi"]["version"]
    require(uniffi == "=" + str(manifest.get("uniffi_version")),
            f"{version} 需要和此检出使用相同的 UniFFI 生成器版本")
    command = [sys.executable, "-B", "scripts/build-support.py", "bindgen",
               "--library", str(library), "--language", "python", "--out-dir", str(destination)]
    if generator:
        command.extend(["--generator", str(generator)])
    subprocess.run(command, cwd=REPO, check=True, stdout=sys.stderr)
    return {"version": version, "commit": commit, "dirty": manifest.get("dirty"),
            "purpose": manifest.get("purpose"), "architecture": architecture,
            "library_sha256": bindings.digest(library), "archive_sha256": archive_hash}


def participant_main(artifact, directory, name, port, response_fd):
    directory.mkdir(mode=0o700, exist_ok=True)
    for name_part in ("home", "state", "resources"):
        (directory / name_part).mkdir(mode=0o700, exist_ok=True)
    # A restart reopens the same private state, but imports fresh bindings.
    with tempfile.TemporaryDirectory(prefix="participant-", dir=directory) as run, os.fdopen(response_fd, "w") as replies:
        module = bindings.load(artifact, Path(run) / "ffi", "velune_release_pair")
        api, native = module.application_api, module.bindings
        remote = import_module("velune_release_pair.velune_remote")
        app = native.VeluneApplication.open(native.BindingOptions(
            home_directory=str(directory / "state"), resources_directory=str(directory / "resources"),
        ))
        endpoint = native.VeluneRemoteEndpoint.open(native.BindingRemoteOptions(
            identity_file=str(directory / "device.json"), state_file=str(directory / "network.json"),
            device_name=name, relay=remote.RelayConfig.DISABLED(), discovery=remote.DiscoveryConfig.NONE(),
            path_policy=remote.PathPolicy.AUTOMATIC, bind_addresses=[f"127.0.0.1:{port}"], request_timeout_ms=5000,
        ), app)
        target = None
        print(json.dumps({"ok": endpoint.descriptor().endpoint_id}), file=replies, flush=True)
        for line in sys.stdin:
            command, *args = json.loads(line)
            try:
                if command == "offer":
                    value = endpoint.create_invitation(60).encoded
                elif command == "begin":
                    state = endpoint.begin_pairing(args[0])
                    value = [state.attempt_id, state.verification_code]
                elif command == "approve":
                    attempt, peer, code = args
                    pending = next(item for item in endpoint.read_state().incoming_pairings if item.attempt_id == attempt)
                    require(bool(code) and pending.peer_id == peer and pending.verification_code == code, "配对核对码不符")
                    value = endpoint.approve_pairing(attempt, peer).phase.name
                elif command == "pairing_status":
                    value = endpoint.pairing_status(args[0]).phase.name
                elif command == "connect":
                    target = endpoint.connect(args[0], str(uuid4()))
                    value = None
                elif command == "read":
                    value = sorted(item.id.name for item in target.list().protocols)
                elif command == "path":
                    path = target.connection_status()
                    value = {"peer_id": path.peer_id, "closed": path.closed,
                             "selected_paths": [item.kind.name for item in path.paths if item.selected],
                             "via_device_ids": endpoint.controller_status(args[0]).via_device_ids}
                elif command == "write":
                    content = api.ResourceTemplateContent.INSTRUCTIONS("Synthetic release pair fixture.")
                    target.save_resource_template(api.ResourceTemplate(id="", name=PROBE_NAME, content=content))
                    value = None
                elif command == "count":
                    value = sum(item.name == PROBE_NAME for item in app.list_resource_templates())
                else:
                    raise ValueError(command)
                reply = {"ok": value}
            except Exception as error:
                reply = {"error": type(error).__name__, "detail": str(error)}
            print(json.dumps(reply), file=replies, flush=True)
        app.shutdown()


class Participant:
    def __init__(self, cleanup, artifact, directory, name, port):
        read_fd, write_fd = os.pipe()
        self.replies = cleanup.enter_context(os.fdopen(read_fd))
        try:
            environment = {"HOME": str(directory / "home"), "VELUNE_HOME": str(directory / "state"),
                           "PATH": "/usr/bin:/bin:/usr/sbin:/sbin", "TMPDIR": str(directory.parent),
                           "DISABLE_TELEMETRY": "1", "DISABLE_ERROR_REPORTING": "1"}
            self.process = subprocess.Popen(
                [sys.executable, "-B", __file__, "--participant", str(artifact), str(directory), name, str(port), str(write_fd)],
                stdin=subprocess.PIPE, stdout=subprocess.DEVNULL, stderr=subprocess.DEVNULL,
                text=True, env=environment, pass_fds=(write_fd,),
            )
        finally:
            os.close(write_fd)
        cleanup.callback(self.close)
        self.identity = self.receive("start")

    def close(self):
        if not self.process.stdin.closed:
            with suppress(BrokenPipeError):
                self.process.stdin.close()
        try:
            return self.process.wait(5)
        except subprocess.TimeoutExpired:
            self.process.kill()
            self.process.wait()
            return None

    def receive(self, stage):
        require(select.select([self.replies], [], [], 15)[0], f"{stage} 超时")
        line = self.replies.readline()
        require(bool(line), f"{stage} 未收到结果，子进程退出码 {self.process.poll()}")
        reply = json.loads(line)
        require("ok" in reply, f"{stage}：{reply.get('error')} {reply.get('detail')}")
        return reply["ok"]

    def call(self, command, *args):
        self.process.stdin.write(json.dumps([command, *args]) + "\n")
        self.process.stdin.flush()
        return self.receive(command)


def ports():
    with socket.socket(socket.AF_INET, socket.SOCK_DGRAM) as host, socket.socket(socket.AF_INET, socket.SOCK_DGRAM) as controller:
        host.bind(("127.0.0.1", 0))
        controller.bind(("127.0.0.1", 0))
        return host.getsockname()[1], controller.getsockname()[1]


def scenario(older, newer, directory, upgraded):
    directory.mkdir()
    result = {"upgraded": upgraded, "transport": "direct", "status": "failed"}
    host_port, controller_port = ports()
    with ExitStack() as cleanup:
        try:
            host = Participant(cleanup, older, directory / "host", "Acceptance host", host_port)
            controller = Participant(cleanup, older, directory / "controller", "Acceptance controller", controller_port)
            result["before"] = {"host": host.identity, "controller": controller.identity}
            invitation = host.call("offer")
            attempt, code = controller.call("begin", invitation)
            require(host.call("approve", attempt, controller.identity, code) == "GRANTED"
                    and controller.call("pairing_status", host.identity) == "GRANTED", "旧版两端未完成配对")
            controller.call("connect", host.identity)
            result["before_protocols"] = controller.call("read")
            require(SHARED_PROTOCOLS.issubset(result["before_protocols"]), "旧版工作基线缺少共享业务协议")
            if upgraded == "host":
                require(host.close() == 0, "旧执行端未正常退出")
                host = Participant(cleanup, newer, directory / "host", "Acceptance host", host_port)
            else:
                require(controller.close() == 0, "旧控制端未正常退出")
                controller = Participant(cleanup, newer, directory / "controller", "Acceptance controller", controller_port)
            result["after"] = {"host": host.identity, "controller": controller.identity}
            require(result["before"] == result["after"], "升级后设备身份变化")
            result["host_write_count"] = host.call("count")
            require(result["host_write_count"] == 0, "测试写入前资源库已有记录")
            try:
                controller.call("connect", host.identity)
                result["after_protocols"] = controller.call("read")
                require(SHARED_PROTOCOLS.issubset(result["after_protocols"]), "升级后未读出共享业务协议")
                result["path"] = controller.call("path", host.identity)
                require(result["path"] == {"peer_id": host.identity, "closed": False,
                                          "selected_paths": ["DIRECT"], "via_device_ids": []},
                        f"未观测到直连目标设备：{result['path']}")
                controller.call("write")
            finally:
                result["host_write_count"] = host.call("count")
            require(result["host_write_count"] == 1, "执行端应恰好创建一份资源模板")
            require(controller.close() == 0 and host.close() == 0, "验证子进程未正常退出")
            result["status"] = "passed"
        except (OSError, RuntimeError, ValueError) as error:
            result["error"] = str(error)
    return result


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--older", type=Path, required=True)
    parser.add_argument("--newer", type=Path, required=True)
    parser.add_argument("--older-sha256")
    parser.add_argument("--newer-sha256")
    parser.add_argument("--generator", type=Path, help="复用当前检出对应的 velune-bindgen")
    parser.add_argument("--output", type=Path, required=True, help="写入结果 JSON 的位置")
    args = parser.parse_args()
    report = {"status": "failed", "transport": {"direct": "not_run", "routed": "not_configured"}}
    try:
        require(sys.platform == "darwin", "此入口验证实际 macOS 构建动态库")
        generator = args.generator.resolve(strict=True) if args.generator else None
        with tempfile.TemporaryDirectory(prefix="velune-release-pair-") as workspace:
            workspace = Path(workspace).resolve()
            old, new = workspace / "older", workspace / "newer"
            report["older"] = prepare_artifact(args.older, args.older_sha256, old, generator)
            report["newer"] = prepare_artifact(args.newer, args.newer_sha256, new, generator)
            require(report["older"]["library_sha256"] != report["newer"]["library_sha256"], "两侧不能使用同一份动态库")
            report["scenarios"] = [scenario(old, new, workspace / side, side) for side in ("host", "controller")]
            report["transport"]["direct"] = "attempted"
            if all(case["status"] == "passed" for case in report["scenarios"]):
                report["status"] = "passed"
                report["transport"]["direct"] = "passed"
    except (BadZipFile, OSError, RuntimeError, ValueError, subprocess.CalledProcessError) as error:
        report["error"] = str(error)
    args.output.parent.mkdir(parents=True, exist_ok=True)
    args.output.write_text(json.dumps(report, ensure_ascii=False, indent=2) + "\n", encoding="utf-8")
    print(f"{report['status']}: {args.output}")
    return 0 if report["status"] == "passed" else 1


if __name__ == "__main__":
    if sys.argv[1:2] == ["--participant"]:
        artifact, directory, name, port, response_fd = sys.argv[2:]
        participant_main(Path(artifact), Path(directory), name, int(port), int(response_fd))
    else:
        sys.exit(main())
