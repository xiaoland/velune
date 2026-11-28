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
import time
import tomllib
from uuid import uuid4
from zipfile import BadZipFile, ZipFile

from velune_acceptance import bindings
from velune_acceptance.isolation import REPO


LIBRARY = "Contents/Frameworks/libvelune_bindings.dylib"
MANIFEST = "Contents/Resources/build-manifest.json"
SHARED_PROTOCOLS = {"CHAT_COMPLETIONS", "RESPONSES", "MESSAGES"}
PROBE_NAME = "Release pair acceptance"
PROVIDER_ID = "release-pair-v3-refusal"


def require(condition, explanation):
    if not condition:
        raise RuntimeError(explanation)


class ParticipantError(RuntimeError):
    def __init__(self, stage, reply):
        super().__init__(f"{stage}：{reply['error']} {reply.get('detail')}")
        self.code = reply.get("code")


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


def participant_main(artifact, directory, name, port, blocked_port, response_fd):
    directory.mkdir(mode=0o700, exist_ok=True)
    for name_part in ("home", "state", "resources"):
        (directory / name_part).mkdir(mode=0o700, exist_ok=True)
    # A restart reopens the same private state, but imports fresh bindings.
    with tempfile.TemporaryDirectory(prefix="participant-", dir=directory) as run, os.fdopen(response_fd, "w") as replies:
        if blocked_port:
            with socket.socket(socket.AF_INET, socket.SOCK_DGRAM) as probe:
                try:
                    probe.sendto(b"Velune test isolation", ("127.0.0.1", blocked_port))
                except PermissionError:
                    pass
                else:
                    print(json.dumps({"error": "Unisolated", "detail": "直达 UDP 仍可用"}), file=replies, flush=True)
                    return
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
                elif command == "authorized_contact":
                    value = any(contact.peer_id == args[0] and contact.control_authorized
                                and isinstance(contact.confirmation, remote.ContactConfirmation.CONFIRMED)
                                for contact in endpoint.read_state().contacts)
                elif command == "connect":
                    target = endpoint.connect(args[0], str(uuid4()))
                    value = None
                elif command == "activate":
                    endpoint.set_controller_activity(args[0], True, True)
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
                elif command == "save_provider":
                    owner = app if args[0] == "local" else target
                    provider = api.ProviderDraft(
                        id=PROVIDER_ID, name="Synthetic release pair provider", protocol=api.Protocol.GEMINI,
                        endpoint="http://127.0.0.1:1", execution_backend=api.ExecutionBackend.HTTP(),
                        request_profile=api.RequestProfile.NATIVE, authentication_source=api.ProviderAuthenticationSource.API_KEY,
                        quota_source=api.ProviderQuotaSource.NONE, quota_limits=[], codex_client_version=None,
                        models=[api.ProviderModel(record_key="", provider_model_id="gemini-synthetic",
                                                  context_window=128000, max_output_tokens=4096)],
                    )
                    try:
                        owner.save_provider("default", provider, api.AuthenticationEdit.SET_API_KEY("synthetic-pair-fixture"))
                    except (native.BindingError.RemoteRequest, native.BindingError.Diagnostic) as error:
                        value = {"kind": error.kind.name, "detail": error.detail, "code": error.code}
                    else:
                        value = {"kind": "ACCEPTED"}
                elif command == "provider_count":
                    value = sum(provider.id == PROVIDER_ID for gateway in app.list().gateways for provider in gateway.providers)
                else:
                    raise ValueError(command)
                reply = {"ok": value}
            except Exception as error:
                reply = {"error": type(error).__name__, "detail": str(error), "code": getattr(error, "code", None)}
            print(json.dumps(reply), file=replies, flush=True)
        app.shutdown()


class Participant:
    def __init__(self, cleanup, artifact, directory, name, port, blocked_port=0):
        read_fd, write_fd = os.pipe()
        self.replies = cleanup.enter_context(os.fdopen(read_fd))
        try:
            environment = {"HOME": str(directory / "home"), "VELUNE_HOME": str(directory / "state"),
                           "PATH": "/usr/bin:/bin:/usr/sbin:/sbin", "TMPDIR": str(directory.parent),
                           "DISABLE_TELEMETRY": "1", "DISABLE_ERROR_REPORTING": "1"}
            command = [sys.executable, "-B", __file__, "--participant", str(artifact), str(directory), name,
                       str(port), str(blocked_port), str(write_fd)]
            if blocked_port:
                policy = f'(version 1)(allow default)(deny network-outbound (remote ip "*:{blocked_port}"))'
                command = ["/usr/bin/sandbox-exec", "-p", policy, *command]
            self.process = subprocess.Popen(
                command,
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
        if "error" in reply:
            raise ParticipantError(stage, reply)
        return reply["ok"]

    def call(self, command, *args):
        self.process.stdin.write(json.dumps([command, *args]) + "\n")
        self.process.stdin.flush()
        return self.receive(command)


def ports(count):
    with ExitStack() as pending:
        sockets = [pending.enter_context(socket.socket(socket.AF_INET, socket.SOCK_DGRAM)) for _ in range(count)]
        for endpoint in sockets:
            endpoint.bind(("127.0.0.1", 0))
        return [endpoint.getsockname()[1] for endpoint in sockets]


def pair(controller, host):
    invitation = host.call("offer")
    attempt, code = controller.call("begin", invitation)
    require(host.call("approve", attempt, controller.identity, code) == "GRANTED"
            and controller.call("pairing_status", host.identity) == "GRANTED", "旧版两端未完成配对")


def wait_for_member(controller, host):
    deadline = time.monotonic() + 30
    while not controller.call("authorized_contact", host.identity):
        require(time.monotonic() < deadline, "中转设备未使两端自动取得互控资格")
        time.sleep(0.1)


def v3_write_refusal(controller, host):
    require(controller.call("provider_count") == host.call("provider_count") == 0, "探针提供商已存在")
    require(controller.call("save_provider", "local") == {"kind": "ACCEPTED"}
            and controller.call("provider_count") == 1, "新版本本地对照未接受相同提供商配置")
    result = controller.call("save_provider", "remote")
    result["host_write_count"] = host.call("provider_count")
    return result


def observe(connection, host_id, result):
    # Opening a proxy and reading configuration are both observations.
    # Across a routed host restart the older controller may still be retiring
    # its previous path. Keep the rejection in the receipt and never retry a write.
    started = time.monotonic()
    while True:
        try:
            connection.call("connect", host_id)
            return connection.call("read")
        except ParticipantError as error:
            if error.code != "server_restarted":
                raise
            elapsed = time.monotonic() - started
            result.setdefault("reconnect_observations", []).append({"code": error.code, "seconds": round(elapsed, 3)})
            if elapsed >= 15:
                raise
            time.sleep(0.25)


def checked_path(controller, host, relay):
    path = controller.call("path", host.identity)
    require(path == {"peer_id": host.identity, "closed": False,
                     "selected_paths": ["OTHER"] if relay else ["DIRECT"],
                     "via_device_ids": [relay.identity] if relay else []},
            f"未观测到指定的路径和目标设备：{path}")
    return path


def scenario(older, newer, directory, upgraded, transport, check_v3):
    directory.mkdir()
    result = {"upgraded": upgraded, "transport": transport, "status": "failed"}
    host_port, controller_port, relay_port = ports(3)
    routed = transport == "routed"
    with ExitStack() as cleanup:
        try:
            def start(artifact, side, port, blocked_port):
                return Participant(cleanup, artifact, directory / side, f"Acceptance {side}", port,
                                   blocked_port if routed else 0)

            host = start(older, "host", host_port, controller_port)
            controller = start(older, "controller", controller_port, host_port)
            result["before"] = {"host": host.identity, "controller": controller.identity}
            relay = None
            if routed:
                relay = Participant(cleanup, older, directory / "relay", "Acceptance relay", relay_port)
                result["relay_id"] = relay.identity
                pair(controller, relay)
                pair(relay, host)
                wait_for_member(controller, host)
                wait_for_member(host, controller)
            else:
                pair(controller, host)
            controller.call("connect", host.identity)
            controller.call("activate", host.identity)
            result["before_protocols"] = controller.call("read")
            require(SHARED_PROTOCOLS.issubset(result["before_protocols"]), "旧版工作基线缺少共享业务协议")
            result["before_path"] = checked_path(controller, host, relay)
            if upgraded == "host":
                require(host.close() == 0, "旧执行端未正常退出")
                host = start(newer, "host", host_port, controller_port)
            else:
                require(controller.close() == 0, "旧控制端未正常退出")
                controller = start(newer, "controller", controller_port, host_port)
            result["after"] = {"host": host.identity, "controller": controller.identity}
            require(result["before"] == result["after"], "升级后设备身份变化")
            result["host_write_count"] = host.call("count")
            require(result["host_write_count"] == 0, "测试写入前资源库已有记录")
            try:
                result["after_protocols"] = observe(controller, host.identity, result)
                require(SHARED_PROTOCOLS.issubset(result["after_protocols"]), "升级后未读出共享业务协议")
                result["path"] = checked_path(controller, host, relay)
                controller.call("write")
            finally:
                result["host_write_count"] = host.call("count")
            require(result["host_write_count"] == 1, "执行端应恰好创建一份资源模板")
            if check_v3 and upgraded == "controller":
                result["v3_write_refusal"] = v3_write_refusal(controller, host)
                refusal = result["v3_write_refusal"]
                require(refusal["kind"] == "UNSUPPORTED" and "需要两端更新" in refusal.get("detail", "")
                        and refusal["host_write_count"] == 0, "v3 执行端没有明确拒绝或产生了配置写入")
            if relay:
                result["relay_write_count"] = relay.call("count")
                require(result["relay_write_count"] == 0 and relay.close() == 0, "中转端执行了写入或退出异常")
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
    parser.add_argument("--routed", action="store_true", help="用两端进程隔离 UDP 直达，再验收指定中转设备")
    parser.add_argument("--v3-write-refusal", action="store_true", help="已知旧包仅用 wire v3 时验证升级提示和零写入")
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
            transports = ("direct", "routed") if args.routed else ("direct",)
            report["scenarios"] = []
            for transport in transports:
                cases = [scenario(old, new, workspace / f"{transport}-{side}", side, transport,
                                  args.v3_write_refusal) for side in ("host", "controller")]
                report["scenarios"].extend(cases)
                report["transport"][transport] = "passed" if all(case["status"] == "passed" for case in cases) else "failed"
            if all(case["status"] == "passed" for case in report["scenarios"]):
                report["status"] = "passed"
    except (BadZipFile, OSError, RuntimeError, ValueError, subprocess.CalledProcessError) as error:
        report["error"] = str(error)
    args.output.parent.mkdir(parents=True, exist_ok=True)
    args.output.write_text(json.dumps(report, ensure_ascii=False, indent=2) + "\n", encoding="utf-8")
    print(f"{report['status']}: {args.output}")
    return 0 if report["status"] == "passed" else 1


if __name__ == "__main__":
    if sys.argv[1:2] == ["--participant"]:
        artifact, directory, name, port, blocked_port, response_fd = sys.argv[2:]
        participant_main(Path(artifact), Path(directory), name, int(port), int(blocked_port), int(response_fd))
    else:
        sys.exit(main())
