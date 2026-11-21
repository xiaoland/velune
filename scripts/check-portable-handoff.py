#!/usr/bin/env python3
"""Private, synthetic Codex -> Pi full-core handoff fixture.

Uses the real installed Codex CLI, Pi CLI/SDK, and the exact matching generated
Python Bindings. All model traffic goes to this fixture's loopback server.
No user credentials, daily state, real conversations, or external endpoints are
read or used.
"""
from __future__ import annotations

import argparse
import hashlib
import importlib.machinery
import importlib.util
import json
import os
from pathlib import Path
import shutil
import socket
import subprocess
import sys
import tempfile
import threading
import time
from http.server import BaseHTTPRequestHandler, ThreadingHTTPServer
from uuid import uuid4

REPO = Path(__file__).resolve().parents[1]
SCOPE: str | None = None
CODEX_DEFAULT = Path("/Applications/ChatGPT.app/Contents/Resources/codex-cli/bin/codex")
TARGET_INPUT = "SYNTHETIC HANDOFF USER INPUT ONCE"
TAIL_INPUT = "SYNTHETIC QUEUED TAIL INPUT ONCE"
SUMMARY_TEXT = "SYNTHETIC PORTABLE SUMMARY: preserve the user's goal, decisions, and constraints."
SOURCE_SEED_INPUT = "SYNTHETIC SOURCE SEED INPUT"
SOURCE_SEED_REPLY = "SYNTHETIC SOURCE SEED REPLY"
SOURCE_RETAINED_PREFIX = "SYNTHETIC RETAINED PREFIX"
SOURCE_RETAINED_TAIL = "SYNTHETIC RETAINED TAIL"
SUMMARY_MODEL = "gpt-6.2-sol"
SOURCE_MODEL = "gpt-6.1-sol"
TARGET_MODEL = "beta"
ROOT_PREFIX = "velune-handoff-fullcore-"


def safe_fixture_root(value: str | None) -> Path:
    if value is None:
        return Path(tempfile.mkdtemp(prefix=ROOT_PREFIX, dir="/private/tmp"))
    path = Path(value).expanduser().absolute()
    if not str(path).startswith("/private/tmp/" + ROOT_PREFIX):
        raise SystemExit(f"fixture root must be a new /private/tmp/{ROOT_PREFIX}* directory")
    if path.exists() or path.is_symlink():
        raise SystemExit(f"refusing to take over existing fixture root: {path}")
    path.mkdir(mode=0o700, parents=False, exist_ok=False)
    return path


def import_bindings(bindings_dir: Path, expected_sha256: str, root: Path):
    bindings_dir = bindings_dir.expanduser().resolve(strict=True)
    python_dir = bindings_dir if (bindings_dir / "velune_application_api.py").is_file() else bindings_dir / "python"
    dylib = bindings_dir / "libvelune_bindings.dylib"
    if not dylib.is_file():
        dylib = python_dir / "libvelune_bindings.dylib"
    if not dylib.is_file():
        raise SystemExit(f"missing generated Bindings library: {dylib}")
    if not (python_dir / "velune_application_api.py").is_file():
        raise SystemExit(f"missing generated Python Bindings modules: {python_dir}")
    digest = hashlib.sha256(dylib.read_bytes()).hexdigest()
    if digest != expected_sha256:
        raise SystemExit(f"Bindings dylib hash mismatch: expected {expected_sha256}, got {digest}")
    # UniFFI's Python loader resolves the dylib beside the generated modules.
    # Stage exact symlinks in this private fixture root when SVC keeps the
    # generated Python cache and dylib in sibling directories.
    staged_dir = root / "bindings"
    staged_dir.mkdir(mode=0o700)
    for name in (
        "velune_application_api.py",
        "velune_bindings.py",
        "velune_conversation.py",
        "velune_remote.py",
        "velune_telemetry.py",
    ):
        source = python_dir / name
        if not source.is_file():
            raise SystemExit(f"missing generated Bindings module: {source}")
        (staged_dir / name).symlink_to(source)
    (staged_dir / "libvelune_bindings.dylib").symlink_to(dylib)
    package = importlib.util.module_from_spec(importlib.machinery.ModuleSpec("fixture_bindings", None, is_package=True))
    package.__path__ = [str(staged_dir)]
    sys.modules["fixture_bindings"] = package
    from fixture_bindings import velune_application_api as api
    from fixture_bindings import velune_bindings as bindings
    from fixture_bindings import velune_conversation as conversation
    return bindings, api, conversation, digest


def matching_app_library() -> Path:
    identity = subprocess.run(
        [sys.executable, "scripts/macos_identity.py", "describe", "--purpose", "development",
         "--profile", "dev", "--root", str(REPO)],
        cwd=REPO, check=True, capture_output=True, text=True,
    )
    bundle = Path(json.loads(identity.stdout)["bundlePath"])
    library = bundle / "Contents/Frameworks/libvelune_bindings.dylib"
    if not library.is_file():
        raise SystemExit("the built Mac development App has no embedded Bindings library; build it first")
    return library


def prepare_bindings(args, temporary_directory: Path) -> tuple[Path, str]:
    if args.bindings_dir is not None:
        if not args.bindings_sha256:
            raise SystemExit("--bindings-dir requires --bindings-sha256")
        bindings = args.bindings_dir.expanduser().resolve(strict=True)
        dylib = bindings / "libvelune_bindings.dylib"
        if not dylib.is_file():
            raise SystemExit("--bindings-dir must contain libvelune_bindings.dylib")
        digest = hashlib.sha256(dylib.read_bytes()).hexdigest()
        if digest != args.bindings_sha256:
            raise SystemExit(f"Bindings dylib digest mismatch: found {digest}")
        if args.expected_sha256 and digest != args.expected_sha256:
            raise SystemExit(f"Bindings dylib digest did not match --expected-sha256: {digest}")
        return bindings, digest

    library = matching_app_library()
    digest = hashlib.sha256(library.read_bytes()).hexdigest()
    if args.expected_sha256 and digest != args.expected_sha256:
        raise SystemExit(f"built App Bindings digest mismatch: found {digest}")
    generator = REPO / "target/build-tools/debug/velune-bindgen"
    if not generator.is_file():
        raise SystemExit("target/build-tools/debug/velune-bindgen is missing; build Mac development artifacts first")
    generated = REPO / "target/bindings/python"
    subprocess.run(
        [sys.executable, "scripts/build-support.py", "bindgen", "--library", str(library),
         "--language", "python", "--generator", str(generator), "--out-dir", str(generated)],
        cwd=REPO, check=True,
    )
    modules = sorted(generated.glob("*.py"))
    if not modules:
        raise SystemExit("build-support.py produced no Python Bindings modules")
    staged = temporary_directory / "python-bindings"
    staged.mkdir(mode=0o700)
    for module in modules:
        shutil.copyfile(module, staged / module.name)
    shutil.copyfile(library, staged / "libvelune_bindings.dylib")
    return staged, digest


def output_directory() -> Path:
    base = REPO / "target/checks/portable-handoff"
    if base.is_symlink():
        raise SystemExit("refusing a symlink at target/checks/portable-handoff")
    base.mkdir(parents=True, exist_ok=True)
    stamp = time.strftime("%Y%m%d-%H%M%S")
    for attempt in range(100):
        output = base / f"run-{stamp}-{os.getpid()}-{attempt:02d}"
        try:
            output.mkdir(mode=0o700)
            return output
        except FileExistsError:
            continue
    raise SystemExit("could not allocate a unique portable-handoff evidence directory")


def import_identity():
    path = REPO / "scripts/macos_identity.py"
    spec = importlib.util.spec_from_file_location("velune_macos_identity", path)
    if spec is None or spec.loader is None:
        raise SystemExit(f"cannot import identity helper: {path}")
    module = importlib.util.module_from_spec(spec)
    spec.loader.exec_module(module)
    return module


def prepare_resources(root: Path) -> None:
    destination = root / "resources"
    destination.mkdir(mode=0o700)
    source = REPO / "packages/agent-runtime/resources"
    for entry in source.iterdir():
        if entry.is_file():
            (destination / entry.name).symlink_to(entry)
    support = REPO / "target/runtime-support"
    for name in ("node_modules", "xurl"):
        candidate = support / name
        if not candidate.exists():
            raise SystemExit(f"required runtime support is missing: {candidate}")
        (destination / name).symlink_to(candidate)


def private_process_environment(root: Path, node: Path) -> None:
    os.environ.clear()
    os.environ.update({
        "HOME": str(root / "os-home"),
        "PATH": f"{node.parent}:/usr/bin:/bin:/usr/sbin:/sbin",
        "HTTP_PROXY": "http://127.0.0.1:1",
        "HTTPS_PROXY": "http://127.0.0.1:1",
        "ALL_PROXY": "http://127.0.0.1:1",
        "NO_PROXY": "127.0.0.1,localhost",
        "http_proxy": "http://127.0.0.1:1",
        "https_proxy": "http://127.0.0.1:1",
        "all_proxy": "http://127.0.0.1:1",
        "no_proxy": "127.0.0.1,localhost",
        "DISABLE_TELEMETRY": "1",
        "DISABLE_ERROR_REPORTING": "1",
    })


class LoopbackProvider:
    def __init__(self, root: Path, hold_summary: bool, hold_target: bool):
        self.root = root
        self.hold_summary = hold_summary
        self.hold_target = hold_target
        self.bodies: list[dict] = []
        self.events: list[dict] = []
        self.guard = threading.Lock()
        self.server = ThreadingHTTPServer(("127.0.0.1", 0), self._handler())
        self.server.daemon_threads = True
        self.thread = threading.Thread(target=self.server.serve_forever, name="synthetic-loopback-provider", daemon=True)
        self.thread.start()
        self.port = self.server.server_port

    def _record(self, value: dict) -> None:
        with self.guard:
            self.bodies.append(value)
            (self.root / "provider-requests.json").write_text(json.dumps(self.bodies, indent=2))

    def event(self, name: str, **fields) -> None:
        record = {"time": time.time(), "event": name, **fields}
        with self.guard:
            self.events.append(record)
            with (self.root / "provider-events.jsonl").open("a") as stream:
                stream.write(json.dumps(record, sort_keys=True) + "\n")

    def _gate(self, stage: str, hold: bool, client: socket.socket) -> str:
        progress = self.root / f"{stage}-progress.json"
        progress.write_text(json.dumps({"stage": stage, "pid": os.getpid(), "time": time.time()}))
        self.event(f"{stage}_progress")
        if not hold:
            return "release"
        release = self.root / f"release-{stage}"
        fail = self.root / f"fail-{stage}"
        stop = self.root / "stop"
        deadline = time.monotonic() + 1800
        while time.monotonic() < deadline:
            if fail.exists():
                self.event(f"{stage}_fail_requested")
                return "fail"
            if release.exists():
                self.event(f"{stage}_released")
                return "release"
            if stop.exists():
                return "stop"
            try:
                client.setblocking(False)
                try:
                    peek = client.recv(1, socket.MSG_PEEK)
                    if peek == b"":
                        self.event(f"{stage}_client_disconnected")
                        return "disconnected"
                except BlockingIOError:
                    pass
                finally:
                    client.setblocking(True)
            except OSError:
                self.event(f"{stage}_client_disconnected")
                return "disconnected"
            time.sleep(0.1)
        self.event(f"{stage}_gate_timeout")
        return "fail"

    def _handler(self):
        provider = self

        class Handler(BaseHTTPRequestHandler):
            protocol_version = "HTTP/1.1"

            def log_message(self, *_):
                pass

            def _send_json(self, code: int, value: dict):
                payload = json.dumps(value).encode()
                self.send_response(code)
                self.send_header("Content-Type", "application/json")
                self.send_header("Content-Length", str(len(payload)))
                self.send_header("Connection", "close")
                self.end_headers()
                self.wfile.write(payload)

            def do_GET(self):
                if self.path.rstrip("/").endswith("models"):
                    self._send_json(200, {"object": "list", "data": [
                        {"id": SOURCE_MODEL, "object": "model", "owned_by": "synthetic"},
                        {"id": SUMMARY_MODEL, "object": "model", "owned_by": "synthetic"},
                    ]})
                else:
                    self._send_json(404, {"error": {"message": "synthetic endpoint not found"}})

            def do_POST(self):
                try:
                    length = int(self.headers.get("Content-Length", "0"))
                    body = json.loads(self.rfile.read(length))
                except Exception:
                    self._send_json(400, {"error": {"message": "synthetic fixture requires JSON"}})
                    return
                path = self.path.split("?", 1)[0]
                protocol = "responses" if path.endswith("/responses") else "chat_completions" if path.endswith("/chat/completions") else "unknown"
                provider._record({"path": path, "protocol": protocol, "body": body})
                if protocol == "responses":
                    data = json.dumps(body.get("input", []))
                    is_seed = SOURCE_SEED_INPUT in data
                    if is_seed:
                        self._responses("seed", body)
                    else:
                        self._responses("summary", body)
                elif protocol == "chat_completions":
                    self._chat(body)
                else:
                    self._send_json(404, {"error": {"message": "synthetic endpoint not found"}})

            def _responses(self, stage: str, body: dict):
                action = provider._gate(stage, provider.hold_summary if stage == "summary" else False, self.connection)
                if action == "stop" or (action == "fail" and stage != "summary"):
                    self._send_json(503, {"error": {"message": f"synthetic {stage} failure"}})
                    return
                if action == "disconnected":
                    return
                # An HTTP-successful but empty model answer is a deterministic
                # semantic failure, not a transport error that a CLI may retry.
                answer = "" if action == "fail" else SOURCE_SEED_REPLY if stage == "seed" else SUMMARY_TEXT
                item = {"type": "message", "id": f"synthetic-{stage}-message", "status": "completed",
                        "role": "assistant", "phase": "final_answer",
                        "content": [{"type": "output_text", "text": answer, "annotations": []}]}
                response = {"id": f"synthetic-{stage}-response", "object": "response", "status": "completed",
                            "model": body.get("model", SOURCE_MODEL), "output": [item],
                            "usage": {"input_tokens": 20, "output_tokens": 8, "total_tokens": 28}}
                events = [
                    {"type": "response.created", "response": {"id": response["id"], "object": "response", "status": "in_progress", "output": []}},
                    {"type": "response.output_item.added", "output_index": 0, "item": item},
                    {"type": "response.output_item.done", "output_index": 0, "item": item},
                    {"type": "response.completed", "response": response},
                ]
                payload = "".join(f"event: {event['type']}\ndata: {json.dumps(event)}\n\n" for event in events).encode()
                self.send_response(200)
                self.send_header("Content-Type", "text/event-stream")
                self.send_header("Cache-Control", "no-cache")
                self.send_header("Content-Length", str(len(payload)))
                self.send_header("Connection", "close")
                self.end_headers()
                self.wfile.write(payload)
                self.wfile.flush()
                provider.event(f"{stage}_response_sent", model=body.get("model"), action=action)

            def _chat(self, body: dict):
                action = provider._gate("target", provider.hold_target, self.connection)
                if action in ("fail", "stop"):
                    self._send_json(503, {"error": {"message": "synthetic target failure"}})
                    return
                if action == "disconnected":
                    return
                completion_id = "chatcmpl-synthetic-handoff"
                chunks = [
                    {"id": completion_id, "object": "chat.completion.chunk", "created": 1794484800,
                     "model": body.get("model", TARGET_MODEL), "choices": [{"index": 0, "delta": {"role": "assistant", "content": "SYNTHETIC TARGET REPLY"}, "finish_reason": None}]},
                    {"id": completion_id, "object": "chat.completion.chunk", "created": 1794484800,
                     "model": body.get("model", TARGET_MODEL), "choices": [{"index": 0, "delta": {}, "finish_reason": "stop"}]},
                ]
                payload = "".join(f"data: {json.dumps(chunk)}\n\n" for chunk in chunks) + "data: [DONE]\n\n"
                raw = payload.encode()
                self.send_response(200)
                self.send_header("Content-Type", "text/event-stream")
                self.send_header("Cache-Control", "no-cache")
                self.send_header("Content-Length", str(len(raw)))
                self.send_header("Connection", "close")
                self.end_headers()
                self.wfile.write(raw)
                self.wfile.flush()
                provider.event("target_response_sent", model=body.get("model", TARGET_MODEL), action=action)

        return Handler

    def close(self):
        self.server.shutdown()
        self.server.server_close()
        self.thread.join(timeout=5)


def text_blocks(snapshot) -> list[str]:
    if snapshot is None:
        return []
    return [block.text for message in snapshot.messages for block in message.blocks if block.is_text()]


def handoff_maintenance(snapshot, work_id: str):
    matches = [] if snapshot is None else [message for message in snapshot.messages
        if message.id == f"velune-handoff-compaction:{work_id}"]
    if len(matches) != 1 or not any(variant(block, "context_compaction") for block in matches[0].blocks):
        raise AssertionError(f"expected one ordinary maintenance event for work {work_id}")
    if matches[0].role.name != "SYSTEM":
        raise AssertionError("portable summary appeared as a user/assistant message")
    return matches[0]


def wait_source_turn(app, runtime_id: str, conversation_id: str, timeout: float = 60.0):
    deadline = time.monotonic() + timeout
    while time.monotonic() < deadline:
        snapshot = app.snapshot(runtime_id, conversation_id).snapshot
        texts = text_blocks(snapshot)
        if snapshot is not None and snapshot.run_state.name == "IDLE" and SOURCE_SEED_REPLY in texts:
            return snapshot
        time.sleep(0.1)
    raise TimeoutError("synthetic Codex seed turn did not settle")


def find_rollout(codex_home: Path, native_id: str) -> Path:
    sessions = codex_home / "sessions"
    rollouts = list(sessions.rglob("*.jsonl"))
    for path in rollouts:
        if native_id in path.name or path.stem.endswith(native_id):
            return path
    # CreateConversation can retain a logical draft alias after the native
    # send creates a different physical thread. Resolve only this fixture's
    # canonical association metadata before falling back to a sole rollout.
    links_path = codex_home.parent / "state/conversation-links.json"
    if links_path.exists():
        links = json.loads(links_path.read_text()).get("links", [])
        link = next((entry for entry in links if entry.get("id") == "codex-source:" + native_id), None)
        if link and link.get("segments"):
            physical = link["segments"][-1]["nativeConversationId"].removeprefix("codex-source:")
            matches = [path for path in rollouts if physical in path.name]
            if len(matches) == 1:
                return matches[0]
    # A sole rollout is still an unambiguous synthetic match on older builds.
    if len(rollouts) == 1:
        return rollouts[0]
    raise FileNotFoundError(
        f"expected one synthetic source rollout for {native_id}; found {[path.name for path in rollouts]}"
    )


def append_encrypted_compaction(path: Path) -> bytes:
    record = {
        "timestamp": "2026-11-12T00:00:00.000Z",
        "type": "compacted",
        "payload": {
            "message": "",
            "replacement_history": [
                {"type": "message", "id": "fixture-retained-user", "role": "user",
                 "content": [{"type": "input_text", "text": SOURCE_RETAINED_PREFIX}]},
                {"type": "compaction", "id": "fixture-encrypted-only", "encrypted_content": "SYNTHETIC-OPAQUE-COMPACTION"},
                {"type": "message", "id": "fixture-retained-tail", "role": "assistant",
                 "content": [{"type": "output_text", "text": SOURCE_RETAINED_TAIL}]},
            ],
        },
    }
    with path.open("a") as stream:
        stream.write(json.dumps(record, separators=(",", ":")) + "\n")
    return path.read_bytes()


def resolve_executable(explicit: Path | None, command: str) -> Path:
    raw = str(explicit.expanduser()) if explicit is not None else shutil.which(command)
    if raw is None:
        raise SystemExit(f"{command} was not found on PATH; pass --{command} /absolute/path")
    resolved = Path(raw).resolve(strict=True)
    if not resolved.is_file():
        raise SystemExit(f"{command} is not a file: {resolved}")
    return resolved


def resolve_codex(explicit: Path | None) -> Path:
    if explicit is not None:
        return resolve_executable(explicit, "codex")
    if CODEX_DEFAULT.is_file():
        return CODEX_DEFAULT.resolve(strict=True)
    return resolve_executable(None, "codex")


def config_runtime_bindings(api, conversation, root: Path, codex_cli: Path, node: Path, pi_cli: Path, endpoint: str):
    home = root / "os-home"
    codex_home = root / "codex-home"
    workspace = root / "workspace"
    pi_home = root / "pi-home"
    pi_sessions = pi_home / "sessions"
    for path in (home, codex_home, workspace, pi_home, pi_sessions):
        path.mkdir(parents=True, exist_ok=True, mode=0o700)
    (codex_home / "config.toml").write_text(
        f'''model = "{SOURCE_MODEL}"
model_provider = "synthetic"
approval_policy = "never"
sandbox_mode = "read-only"
[model_providers.synthetic]
name = "Synthetic local provider"
base_url = "{endpoint}"
wire_api = "responses"
experimental_bearer_token = "synthetic-fake-token"
requires_openai_auth = false
[features]
responses_websockets = false
responses_websockets_v2 = false
hooks = false
goals = false
plugins = false
apps = false
memories = false
shell_snapshot = false
'''
    )
    (pi_home / "models.json").write_text(json.dumps({"providers": {"synthetic": {
        "baseUrl": endpoint,
        "api": "openai-completions",
        "apiKey": "synthetic-fake-token",
        "models": [
            {"id": "alpha", "name": "Synthetic Alpha", "reasoning": False, "input": ["text"],
             "contextWindow": 200000, "maxTokens": 1024, "cost": {"input": 0, "output": 0, "cacheRead": 0, "cacheWrite": 0}},
            {"id": TARGET_MODEL, "name": "Synthetic Beta", "reasoning": False, "input": ["text"],
             "contextWindow": 200000, "maxTokens": 1024, "cost": {"input": 0, "output": 0, "cacheRead": 0, "cacheWrite": 0}},
        ],
    }}}, indent=2))
    (pi_home / "settings.json").write_text(json.dumps({
        "defaultProvider": "synthetic", "defaultModel": "alpha",
        "compaction": {"enabled": False}, "retry": {"enabled": False},
    }, indent=2))
    return home, codex_home, workspace, pi_home, pi_sessions


def create_application(bindings, api, conversation, root: Path, node: Path, codex_cli: Path, pi_cli: Path):
    options = bindings.BindingOptions(home_directory=str(root / "state"), resources_directory=str(root / "resources"))
    app = bindings.VeluneApplication.open(options)
    execution = api.RuntimeExecution(
        transport=api.RuntimeTransport.LOCAL(),
        home=api.RuntimeHome.EXISTING(path=str(root / "os-home")),
        remote_resources_directory=None,
    )
    codex = api.RuntimeInstance(
        id="codex-source", name="合成 Codex 来源", enabled=True,
        type_id=conversation.RuntimeKind.CODEX166, gateway_id="default",
        settings={"binary": str(codex_cli), "nodeBinary": str(node),
                  "agentDir": str(root / "codex-home")},
    )
    pi = api.RuntimeInstance(
        id="pi-target", name="合成 Pi 目标", enabled=True,
        type_id=conversation.RuntimeKind.PI, gateway_id="default",
        settings={"binary": str(pi_cli), "nodeBinary": str(node),
                  "agentDir": str(root / "pi-home"), "sessionDir": str(root / "pi-home/sessions")},
    )
    app.upsert_runtime_with_execution(codex, execution)
    app.upsert_runtime_with_execution(pi, execution)
    canonical = app.list()
    default_gateway = next((gateway for gateway in canonical.gateways if gateway.id == "default"), None)
    if default_gateway is None or default_gateway.routes:
        app.shutdown()
        raise RuntimeError("canonical test config must retain the default gateway with an empty routes list")
    (root / "configuration-proof.json").write_text(json.dumps({
        "runtimeIds": [instance.id for instance in canonical.runtime_instances],
        "defaultGatewayId": default_gateway.id,
        "defaultGatewayRoutes": len(default_gateway.routes),
        "sourceRuntimeKind": codex.type_id.name,
        "targetRuntimeKind": pi.type_id.name,
        "configurationWasWrittenThroughBindings": True,
    }, indent=2))
    return app


def variant(value, name: str) -> bool:
    method = getattr(value, f"is_{name}", None)
    if method is None:
        method = getattr(value, f"is_{name.upper()}", None)
    return bool(method and method())


def find_native_model(model) -> tuple[str | None, str] | None:
    selection = model.selection
    if variant(selection, "NATIVE"):
        return selection.provider_id, selection.model_id
    return None


def summary_model_wire(model_id: str) -> tuple[str, str | None]:
    prefix = "velune:codex-speed:"
    if model_id.startswith(prefix):
        base, tier = json.loads(model_id[len(prefix):])
        return base, tier
    return model_id, None


def poll_requirements(app, api, source_id: str, conversation_id: str, target_id: str, timeout: float = 60.0):
    request = app.read_handoff_requirements(source_id, conversation_id, target_id, None)
    deadline = time.monotonic() + timeout
    while variant(request, "CHECKING") and time.monotonic() < deadline:
        time.sleep(0.1)
        request = app.read_handoff_requirements(source_id, conversation_id, target_id, request.request_id)
    if variant(request, "FAILED"):
        raise RuntimeError(f"handoff requirements failed: {request.code}: {request.detail}")
    if not variant(request, "PORTABLE_SUMMARY_REQUIRED"):
        raise RuntimeError(f"expected encrypted-context summary requirement, got {request}")
    return request


def poll_summary(app, work_id: str, root: Path, auto_release: bool, timeout: float = 180.0):
    deadline = time.monotonic() + timeout
    released = False
    while time.monotonic() < deadline:
        status = app.read_handoff_summary(work_id)
        if variant(status, "READY"):
            return status
        if variant(status, "FAILED"):
            raise RuntimeError(f"summary work failed: {status.code}")
        if variant(status, "CANCELLED"):
            raise RuntimeError("summary work was cancelled before preparation completed")
        if auto_release and (root / "summary-progress.json").exists() and not released:
            (root / "release-summary").touch()
            released = True
        time.sleep(0.1)
    raise TimeoutError("summary preparation did not reach a terminal result")


def wait_target(app, source_id: str, conversation_id: str, root: Path, auto_release: bool, timeout: float = 30.0):
    # snapshot() reports the currently observed live session and may be None
    # after a completed handoff. ReadConversation is the durable history path
    # and also verifies that the linked target can be reopened by logical ID.
    deadline = time.monotonic() + min(timeout, 30.0)
    released = False
    while time.monotonic() < deadline:
        snapshot = app.read_conversation(source_id, conversation_id).snapshot
        texts = text_blocks(snapshot)
        if snapshot is not None and snapshot.context_runtime_id == "pi-target" and snapshot.run_state.name == "IDLE" and "SYNTHETIC TARGET REPLY" in texts:
            return snapshot
        if auto_release and (root / "target-progress.json").exists() and not released:
            (root / "release-target").touch()
            released = True
        time.sleep(0.1)
    raise TimeoutError("target Pi conversation did not complete")


def wait_source_input_ready(app, runtime_id: str, conversation_id: str, timeout: float = 30.0):
    deadline = time.monotonic() + timeout
    last_snapshot = None
    while time.monotonic() < deadline:
        last_snapshot = app.snapshot(runtime_id, conversation_id).snapshot
        if last_snapshot is not None and last_snapshot.actions.can_send:
            return last_snapshot
        time.sleep(0.1)
    access = None if last_snapshot is None else last_snapshot.actions.input_access.name
    raise TimeoutError(f"source input admission did not settle; last access={access}")


def wait_idle_shutdown(app, bindings, timeout: float):
    # A final reply can precede native metadata settlement. Cold reopen must
    # follow a normal idle close, with no forced stopping of pending work.
    started = time.monotonic()
    deadline = started + timeout
    retries = 0
    while True:
        try:
            app.shutdown()
            return retries, time.monotonic() - started
        except bindings.BindingError.Diagnostic as error:
            if error.code != "invalid" or error.detail != "runtime is busy" or time.monotonic() >= deadline:
                raise
        retries += 1
        time.sleep(0.1)


def verify_send(provider: LoopbackProvider, root: Path, source_path: Path, source_before: bytes,
                logical_id: str, snapshot, work_id: str, summary_model_id: str) -> dict:
    requests = list(provider.bodies)
    targets = [entry for entry in requests if entry["protocol"] == "chat_completions"]
    if len(targets) != 2:
        raise AssertionError(f"expected first queued input and its tail exactly once, got {len(targets)}")
    if TAIL_INPUT in json.dumps(targets[0]["body"].get("messages", [])):
        raise AssertionError("the tail overtook the queue head")
    if json.dumps(targets[1]["body"].get("messages", [])).count(TAIL_INPUT) != 1:
        raise AssertionError("the tail was lost or duplicated")
    messages = targets[0]["body"].get("messages", [])
    serialized = json.dumps(messages, ensure_ascii=False)
    summary_count = serialized.count(SUMMARY_TEXT)
    input_count = serialized.count(TARGET_INPUT)
    if summary_count != 1 or input_count != 1:
        raise AssertionError(f"expected portable summary and user input once; got summary={summary_count} input={input_count}")
    summary_requests = [entry for entry in requests
                        if entry["protocol"] == "responses"
                        and SOURCE_SEED_INPUT not in json.dumps(entry["body"].get("input", []))]
    if len(summary_requests) != 3:
        raise AssertionError(f"expected cancel, semantic failure and explicit retry only; got {len(summary_requests)} summary calls")
    expected_wire_model, expected_tier = summary_model_wire(summary_model_id)
    summary_body = summary_requests[-1]["body"]
    if summary_body.get("model") != expected_wire_model:
        raise AssertionError(f"summary request used model {summary_body.get('model')!r}, expected {expected_wire_model!r}")
    if summary_body.get("service_tier") != expected_tier:
        raise AssertionError(f"summary request used service_tier {summary_body.get('service_tier')!r}, expected {expected_tier!r}")
    if snapshot.conversation.id != logical_id:
        raise AssertionError(f"logical identity changed: {snapshot.conversation.id}")
    if snapshot.context_runtime_id != "pi-target":
        raise AssertionError(f"target context not adopted: {snapshot.context_runtime_id}")
    maintenance = handoff_maintenance(snapshot, work_id)
    if not maintenance.completed:
        raise AssertionError("committed handoff maintenance event remained running")
    after = source_path.read_bytes()
    if after != source_before:
        raise AssertionError("source Codex rollout changed during prepared handoff")
    proof = {
        "passed": True,
        "workId": work_id,
        "logicalConversationIdPreserved": snapshot.conversation.id,
        "contextRuntimeId": snapshot.context_runtime_id,
        "sourceRolloutBytesUnchanged": hashlib.sha256(source_before).hexdigest() == hashlib.sha256(after).hexdigest(),
        "targetProtocol": targets[0]["protocol"],
        "targetModel": targets[0]["body"].get("model"),
        "portableSummaryCount": summary_count,
        "originalInputCount": input_count,
        "summaryRequestModel": summary_body.get("model"),
        "summaryRequestServiceTier": summary_body.get("service_tier"),
        "targetRequestCount": len(targets),
        "explicitSummaryCallCount": len(summary_requests),
        "queuedTailDidNotOvertake": True,
        "requestContainsNoEncryptedPayload": "SYNTHETIC-OPAQUE-COMPACTION" not in serialized,
        "portableCompactionEventId": maintenance.id,
        "portableCompactionEventCompleted": maintenance.completed,
    }
    (root / "fullcore-proof.json").write_text(json.dumps(proof, indent=2))
    return proof


def run_api_flow(bindings, api, conversation, app, provider: LoopbackProvider, root: Path,
                 source_conversation_id: str, source_native_id: str, source_path: Path,
                 source_before: bytes, auto_release: bool, timeout: float):
    req = poll_requirements(app, api, "codex-source", source_conversation_id, "pi-target", timeout)
    basis = req.summary_basis
    catalog = basis.catalog or []
    candidate_options = []
    for model in catalog:
        selection = find_native_model(model)
        if selection is not None:
            provider_id, model_id = selection
            if provider_id == "synthetic" and model_id != SOURCE_MODEL:
                candidate_options.append((provider_id, model_id, model.name))
    if not candidate_options:
        available = [(item.selection.provider_id, item.selection.model_id, item.name)
                     for item in catalog if variant(item.selection, "NATIVE")]
        raise RuntimeError(f"summary picker exposed no distinct source-provider model: {available}")
    summary_provider_id, summary_model_id, summary_model_name = candidate_options[0]
    selected_summary = conversation.ModelSelection.NATIVE(
        provider_id=summary_provider_id, model_id=summary_model_id,
    )
    def summary_count():
        return sum(entry["protocol"] == "responses" and SOURCE_SEED_INPUT not in json.dumps(entry["body"].get("input", [])) for entry in provider.bodies)

    def wait_for(predicate, label, limit=30):
        deadline = time.monotonic() + min(limit, timeout)
        while time.monotonic() < deadline:
            value = predicate()
            if value:
                return value
            time.sleep(0.05)
        raise TimeoutError(label)

    def current():
        return app.read_conversation("codex-source", source_conversation_id).snapshot

    def work_reaches(work_id, name):
        def read():
            status = app.read_handoff_summary(work_id)
            if variant(status, name):
                return status
            if name == "CANCELLED" and variant(status, "FAILED") and status.code == "cleanup_pending":
                return status
            return None
        return wait_for(read, f"work did not reach {name}")

    def queue_matches(ids, paused=False):
        snapshot = current()
        if snapshot is None or [item.id for item in snapshot.follow_up.queue] != ids:
            return None
        if paused and snapshot.follow_up.paused is None:
            return None
        return snapshot

    app.read_conversation("codex-source", source_conversation_id)
    wait_source_input_ready(app, "codex-source", source_conversation_id)
    target_selection = conversation.ModelSelection.NATIVE(provider_id="synthetic", model_id=TARGET_MODEL)
    turn = api.ComposedTurn(text=TARGET_INPUT, effort=None, permission_mode=None, attachments=[])
    started = app.queue_handoff_summary(
        "codex-source", source_conversation_id, "pi-target", target_selection, turn, selected_summary, None,
    )
    head = started.queued_message_id
    if started.result.snapshot is None or [item.id for item in started.result.snapshot.follow_up.queue] != [head]:
        raise AssertionError("atomic queue acknowledgement did not contain its accepted first item")
    before = current()
    maintenance = handoff_maintenance(before, started.work_id)
    if maintenance.completed or before.run_state.name != "IDLE":
        raise AssertionError("portable preparation did not preserve source idle with a running maintenance event")
    if TARGET_INPUT in text_blocks(before):
        raise AssertionError("queued input leaked into the transcript before delivery")
    tail_turn = api.ComposedTurn(text=TAIL_INPUT, effort=None, permission_mode=None, attachments=[])
    tail_result = app.send_queue("codex-source", source_conversation_id, api.SendQueueOperation.ADD(
        source_runtime_id="codex-source", target_runtime_id="pi-target", selection=target_selection, turn=tail_turn,
    ))
    tail = tail_result.snapshot.follow_up.queue[-1].id
    wait_for(lambda: summary_count() == 1, "first summary request was not observed")
    cancel_started = time.monotonic()
    app.cancel_handoff_summary(started.work_id)
    cancel_status = work_reaches(started.work_id, "CANCELLED")
    wait_for(lambda: any(event["event"] == "summary_client_disconnected" for event in provider.events),
             "cancelled summary model connection did not stop")
    cancellation_seconds = time.monotonic() - cancel_started
    if not variant(cancel_status, "CANCELLED") or cancellation_seconds >= 3:
        raise AssertionError(f"summary cancellation did not stop its own model transport promptly: {cancellation_seconds:.2f}s")
    wait_for(lambda: queue_matches([head, tail], paused=True), "cancel consumed the queue or failed to pause it")
    if any(entry["protocol"] == "chat_completions" for entry in provider.bodies):
        raise AssertionError("summary cancellation dispatched a target request")

    # Retry is explicit and reuses the original queued item; an empty successful
    # answer fails preparation without a hidden model retry or target creation.
    poll_requirements(app, api, "codex-source", source_conversation_id, "pi-target", timeout)
    (root / "fail-summary").touch()
    failed = app.retry_queued_handoff_summary("codex-source", source_conversation_id, head, selected_summary, None)
    if failed.queued_message_id != head:
        raise AssertionError("retry added a duplicate queue item")
    failed_status = work_reaches(failed.work_id, "FAILED")
    failed_snapshot = wait_for(
        lambda: queue_matches([head, tail], paused=True),
        "failed preparation lost or unpaused queued input",
    )
    failed_event = handoff_maintenance(failed_snapshot, failed.work_id)
    failure_text = next(block.text for block in failed_event.blocks if variant(block, "CONTEXT_COMPACTION"))
    if not failed_event.completed or not all(
        value in failure_text
        for value in (failed_status.detail, failed_status.code, failed.work_id)
    ):
        raise AssertionError("failed preparation lost its diagnosis in transcript delivery")
    if summary_count() != 2:
        raise AssertionError("semantic summary failure made an implicit extra model call")
    (root / "fail-summary").unlink()

    poll_requirements(app, api, "codex-source", source_conversation_id, "pi-target", timeout)
    final = app.retry_queued_handoff_summary("codex-source", source_conversation_id, head, selected_summary, None)
    if final.queued_message_id != head:
        raise AssertionError("second explicit retry added a duplicate item")
    wait_for(lambda: summary_count() == 3, "explicit retry did not make exactly its one summary call")
    app.send_queue("codex-source", source_conversation_id, api.SendQueueOperation.EDIT(id=head, text=TARGET_INPUT + " temporary edit"))
    app.send_queue("codex-source", source_conversation_id, api.SendQueueOperation.EDIT(id=head, text=TARGET_INPUT))
    retained = wait_for(lambda: queue_matches([head, tail]), "editing during preparation changed queue identity")
    if retained.follow_up.context_preparation.work_id != final.work_id or summary_count() != 3:
        raise AssertionError("a text-only edit restarted paid preparation")
    (root / "release-summary").touch()
    wait_for(lambda: sum(entry["protocol"] == "chat_completions" for entry in provider.bodies) == 1,
             "Ready did not automatically dispatch exactly the queue head")
    if TAIL_INPUT in json.dumps([entry for entry in provider.bodies if entry["protocol"] == "chat_completions"][0]["body"]):
        raise AssertionError("the tail crossed the preparation gate")

    # The first target has accepted but its model response is held. Cancel
    # immediately from the preparation action without polling background work.
    # The response must confirm the native target stop and preserve the tail.
    accepted_cancel_started = time.monotonic()
    accepted_cancel = app.cancel_handoff_summary(final.work_id)
    accepted_cancellation_seconds = time.monotonic() - accepted_cancel_started
    if not variant(accepted_cancel, "CANCELLED"):
        raise AssertionError(f"accepted target cancellation was not confirmed: {accepted_cancel}")
    stopped = wait_for(lambda: queue_matches([tail], paused=True), "accepted cancellation lost the tail or did not pause it")
    wait_for(lambda: (lambda snapshot: snapshot and snapshot.run_state.name not in ("RUNNING", "STOPPING"))(current()),
             "accepted target did not stop")
    if sum(entry["protocol"] == "chat_completions" for entry in provider.bodies) != 1:
        raise AssertionError("cancel replayed the head or dispatched its tail")
    (root / "release-target").touch()
    app.send_queue("pi-target", source_conversation_id, api.SendQueueOperation.RESUME())
    snapshot = wait_target(app, "codex-source", source_conversation_id, root, True, timeout)
    proof = verify_send(provider, root, source_path, source_before, source_conversation_id,
                        snapshot, final.work_id, summary_model_id)
    proof["initialQueuedMessageId"] = head
    proof["retryPreservedQueueIdentity"] = True
    proof["cancelAndFailurePreservedQueue"] = True
    proof["summaryCancellationStatus"] = "cancelled" if variant(cancel_status, "CANCELLED") else cancel_status.code
    proof["summaryCancellationSeconds"] = cancellation_seconds
    proof["summaryCleanupIsSeparateFromCancellation"] = True
    proof["textEditDidNotRegenerateSummary"] = True
    proof["acceptedCancellationPausedTail"] = True
    proof["acceptedCancellationStatus"] = "cancelled"
    proof["acceptedCancellationSeconds"] = accepted_cancellation_seconds
    proof["summaryModelSelected"] = summary_model_id
    proof["summaryModelName"] = summary_model_name
    proof["summaryProviderSelected"] = summary_provider_id
    proof["summaryCandidateIds"] = list(dict.fromkeys(item[1] for item in candidate_options))
    proof["sourceNativeId"] = source_native_id
    (root / "fullcore-proof.json").write_text(json.dumps(proof, indent=2))
    print("FULLCORE_HANDOFF_PASSED", json.dumps(proof, ensure_ascii=False), flush=True)
    return proof



def preaccepted_cancellation_flow(bindings, api, conversation, app, provider, root, node, codex_cli, pi_cli, timeout):
    """Gate the real Pi SDK process before connect/accept, not an API mock."""
    source = app.create_conversation("codex-source", str(root / "workspace"),
        conversation.ModelSelection.NATIVE(provider_id="synthetic", model_id=SOURCE_MODEL)).snapshot
    cid = source.conversation.id
    native_id = cid.rsplit(":", 1)[-1]
    app.send_turn("codex-source", cid, "codex-source", None,
        api.ComposedTurn(text=SOURCE_SEED_INPUT, effort=None, permission_mode=None, attachments=[]))
    wait_source_turn(app, "codex-source", cid, timeout)
    seed_close_retries, seed_close_seconds = wait_idle_shutdown(app, bindings, timeout)
    path = find_rollout(root / "codex-home", native_id)
    original = append_encrypted_compaction(path)
    app = create_application(bindings, api, conversation, root, node, codex_cli, pi_cli)
    try:
        wrapper = root / "gated-node"
        entered = root / "target-preparation-entered"
        release = root / "release-target-preparation"
        wrapper.write_text("#!/usr/bin/python3\n" +
            "import os, pathlib, sys, time\n" +
            "root=pathlib.Path(" + repr(str(root)) + ")\n" +
            "if len(sys.argv)>1 and pathlib.Path(sys.argv[1]).name=='pi_rpc.mjs':\n" +
            "    (root/'target-preparation-entered').write_text(str(os.getpid()))\n" +
            "    deadline=time.monotonic()+30\n" +
            "    while not (root/'release-target-preparation').exists() and time.monotonic()<deadline: time.sleep(.01)\n" +
            "os.execv(" + repr(str(node)) + ", [" + repr(str(node)) + ", *sys.argv[1:]])\n")
        wrapper.chmod(0o700)
        execution = api.RuntimeExecution(transport=api.RuntimeTransport.LOCAL(),
            home=api.RuntimeHome.EXISTING(path=str(root / "os-home")), remote_resources_directory=None)
        target = api.RuntimeInstance(id="pi-target",name="合成 Pi 目标",enabled=True,
            type_id=conversation.RuntimeKind.PI,gateway_id="default",
            settings={"binary":str(pi_cli),"nodeBinary":str(wrapper),"agentDir":str(root / "pi-home"),"sessionDir":str(root / "pi-home/sessions")})
        app.upsert_runtime_with_execution(target,execution)
        app.read_conversation("codex-source",cid)
        wait_source_input_ready(app,"codex-source",cid)
        poll_requirements(app,api,"codex-source",cid,"pi-target",timeout)
        target_count=sum(entry["protocol"]=="chat_completions" for entry in provider.bodies)
        started=app.queue_handoff_summary("codex-source",cid,"pi-target",
            conversation.ModelSelection.NATIVE(provider_id="synthetic",model_id=TARGET_MODEL),
            api.ComposedTurn(text="SYNTHETIC CANCEL BEFORE TARGET ACCEPT",effort=None,permission_mode=None,attachments=[]),
            None,None)
        deadline=time.monotonic()+min(timeout,30)
        while not entered.exists() and time.monotonic()<deadline: time.sleep(.05)
        if not entered.exists(): raise TimeoutError("real target SDK preparation did not enter its gate")
        outcome=[]
        def cancel():
            try: outcome.append(app.cancel_handoff_summary(started.work_id))
            except BaseException as error: outcome.append(error)
        worker=threading.Thread(target=cancel,name="queued-handoff-cancel")
        worker.start()
        # The request thread sets its shared cancellation flag before waiting
        # for the occupied owner. Release only after it has reached that wait.
        time.sleep(.15)
        release.touch()
        worker.join(timeout=30)
        if worker.is_alive(): raise TimeoutError("cancel did not finish after target preparation resumed")
        if not outcome or isinstance(outcome[0],BaseException): raise RuntimeError(f"cancel failed: {outcome}")
        deadline=time.monotonic()+10
        pending=None
        while time.monotonic()<deadline:
            pending=app.read_conversation("codex-source",cid).snapshot
            if pending and pending.follow_up.paused is not None and [item.id for item in pending.follow_up.queue]==[started.queued_message_id]: break
            time.sleep(.05)
        else: raise AssertionError("pre-accept cancellation did not preserve its paused queue head")
        if sum(entry["protocol"]=="chat_completions" for entry in provider.bodies)!=target_count:
            raise AssertionError("a target prompt was sent after cancellation during SDK preparation")
        if path.read_bytes()!=original: raise AssertionError("pre-accept cancellation changed the source")
        return app, {"preacceptedCancellationBlockedTarget":True,"preacceptedCancellationPreservedQueue":True,
                     "preacceptedSeedIdleCloseRetries":seed_close_retries,"preacceptedSeedIdleCloseSeconds":seed_close_seconds,
                     "committedTargetRecoveryUsesOrdinaryQueue":pending.context_runtime_id=="pi-target" and pending.follow_up.context_preparation is None}
    except BaseException:
        (root / "release-target-preparation").touch()
        app.shutdown_stopping_work()
        raise

def cleanup_fixture(root: Path) -> None:
    root = root.expanduser().resolve(strict=True)
    if not str(root).startswith("/private/tmp/" + ROOT_PREFIX):
        raise SystemExit("refusing cleanup outside fixture-specific /private/tmp root")
    marker = root / "state" / ".velune-development.json"
    expected = {"schemaVersion": 1, "purpose": "development", "scope": SCOPE}
    if json.loads(marker.read_text()) != expected:
        raise SystemExit("refusing cleanup: development-state marker does not match this fixture")
    manifest_path = root / "manifest.json"
    stopped_receipt = root / "provider-stopped.json"
    if manifest_path.is_file():
        manifest = json.loads(manifest_path.read_text())
        if manifest.get("root") != str(root) or manifest.get("scope", SCOPE) != SCOPE:
            raise SystemExit("refusing cleanup: manifest root/scope does not match this fixture")
    elif not stopped_receipt.is_file():
        raise SystemExit("refusing cleanup: incomplete fixture has no stopped-provider receipt")
    if (root / "provider.pid").exists():
        raise SystemExit("stop the exact fixture provider before cleanup")
    shutil.rmtree(root)
    print(f"removed synthetic fixture root: {root}")


def main() -> int:
    parser = argparse.ArgumentParser(description="Verify encrypted-context Codex to Pi handoff through production Bindings/Core")
    parser.add_argument("--bindings-dir", type=Path)
    parser.add_argument("--bindings-sha256")
    parser.add_argument("--codex", type=Path, help="Codex CLI; default: bundled ChatGPT CLI, then PATH")
    parser.add_argument("--node", type=Path, help="Node executable; default: discover node on PATH")
    parser.add_argument("--pi", type=Path, help="Pi CLI executable; default: discover pi on PATH")
    parser.add_argument("--expected-sha256", help="optionally require this exact Bindings library hash")
    parser.add_argument("--keep-fixture", action="store_true", help="retain private synthetic fixture root for diagnosis")
    parser.add_argument("--timeout", type=float, default=180.0)
    args = parser.parse_args()

    node = resolve_executable(args.node, "node")
    pi_cli = resolve_executable(args.pi, "pi")
    codex_cli = resolve_codex(args.codex)
    if not codex_cli.is_file():
        raise SystemExit(f"Codex CLI is missing: {codex_cli}")
    if not node.is_file() or not pi_cli.is_file():
        raise SystemExit("Node/Pi CLI path could not be resolved")
    identity = import_identity()
    global SCOPE
    SCOPE = identity.identity(REPO, "development")["context"]["scope"]
    root = safe_fixture_root(None)
    root = root.resolve()
    output = output_directory()
    bindings_dir, binding_sha = prepare_bindings(args, root)
    bindings, api, conversation, binding_sha = import_bindings(bindings_dir, binding_sha, root)
    (root / "provider.pid").write_text(str(os.getpid()) + "\n")
    for directory in ("os-home", "codex-home", "workspace", "pi-home"):
        (root / directory).mkdir(mode=0o700, exist_ok=True)
    identity.prepare_development_home(root / "state", SCOPE, os_home=root / "os-home")
    (root / "manifest.json").write_text(json.dumps({
        "root": str(root), "scope": SCOPE, "phase": "setup", "providerPid": os.getpid(),
    }, indent=2))
    prepare_resources(root)
    private_process_environment(root, node)
    # Hold the summary until the canonical snapshot has exposed its maintenance
    # row. The API-flow poll releases it after the observation, without timing races.
    provider = LoopbackProvider(root, True, True)
    endpoint = f"http://127.0.0.1:{provider.port}/v1"

    app = None
    try:
        home, codex_home, workspace, pi_home, pi_sessions = config_runtime_bindings(
            api, conversation, root, codex_cli, node, pi_cli, endpoint,
        )
        app = create_application(bindings, api, conversation, root, node, codex_cli, pi_cli)
        # Create and finish a real Codex turn through the canonical application
        # API; no mock native RPC client or provider is involved.
        created = app.create_conversation(
            "codex-source", str(workspace),
            conversation.ModelSelection.NATIVE(provider_id="synthetic", model_id=SOURCE_MODEL),
        ).snapshot
        if created is None:
            raise RuntimeError("Codex source conversation was not returned")
        source_conversation_id = created.conversation.id
        source_native_id = source_conversation_id.rsplit(":", 1)[-1]
        seed_turn = api.ComposedTurn(text=SOURCE_SEED_INPUT, effort=None, permission_mode=None, attachments=[])
        app.send_turn("codex-source", source_conversation_id, "codex-source", None, seed_turn)
        seeded = wait_source_turn(app, "codex-source", source_conversation_id, args.timeout)
        seed_close_retries, seed_close_seconds = wait_idle_shutdown(app, bindings, args.timeout)
        app = None

        source_path = find_rollout(codex_home, source_native_id)
        source_before = append_encrypted_compaction(source_path)
        provider.event("source_ready", source_native_id=source_native_id,
                       source_sha256=hashlib.sha256(source_before).hexdigest())

        manifest = {
            "root": str(root),
            "state": str(root / "state"),
            "stateScope": SCOPE,
            "resources": str(root / "resources"),
            "osHome": str(home),
            "codexHome": str(codex_home),
            "workspace": str(workspace),
            "sourceRuntimeId": "codex-source",
            "targetRuntimeId": "pi-target",
            "sourceConversationId": source_conversation_id,
            "sourceNativeId": source_native_id,
            "sourceRollout": str(source_path),
            "sourceRolloutSha256": hashlib.sha256(source_before).hexdigest(),
            "sourceModel": SOURCE_MODEL,
            "targetModel": TARGET_MODEL,
            "piHome": str(pi_home),
            "piSessionDirectory": str(pi_sessions),
            "node": str(node),
            "pi": str(pi_cli),
            "codex": str(codex_cli),
            "loopback": endpoint,
            "bindingDirectory": str(bindings_dir),
            "bindingLibrarySha256": binding_sha,
            "providerPid": os.getpid(),
            "providerStartedAtUnix": time.time(),
            "configurationCreatedThroughPythonBindings": True,
            "encryptedOnlyCompaction": True,
        }
        (root / "manifest.json").write_text(json.dumps(manifest, indent=2))
        print("HANDOFF_FIXTURE_READY", json.dumps(manifest, ensure_ascii=False), flush=True)

        app = create_application(bindings, api, conversation, root, node, codex_cli, pi_cli)
        proof = run_api_flow(
            bindings, api, conversation, app, provider, root,
            source_conversation_id, source_native_id, source_path, source_before,
            True, args.timeout,
        )
        proof["seedIdleCloseRetries"] = seed_close_retries
        proof["seedIdleCloseSeconds"] = seed_close_seconds
        proof["idleCloseRetries"], proof["idleCloseElapsedSeconds"] = wait_idle_shutdown(app, bindings, args.timeout)
        app = None
        app = create_application(bindings, api, conversation, root, node, codex_cli, pi_cli)
        reopened = app.read_conversation("codex-source", source_conversation_id).snapshot
        restored = handoff_maintenance(reopened, proof["workId"])
        if not restored.completed or restored.id != proof["portableCompactionEventId"]:
            raise AssertionError("portable maintenance identity changed after cold reopen")
        if source_path.read_bytes() != source_before:
            raise AssertionError("cold maintenance projection changed the original source")
        proof["portableCompactionEventColdReopen"] = True
        app, cancellation_proof = preaccepted_cancellation_flow(bindings,api,conversation,app,provider,root,node,codex_cli,pi_cli,args.timeout)
        proof.update(cancellation_proof)
        proof["bindingLibrarySha256"] = binding_sha
        proof["codexCli"] = str(codex_cli)
        proof["node"] = str(node)
        proof["piCli"] = str(pi_cli)
        proof["sourceRolloutSha256"] = hashlib.sha256(source_before).hexdigest()
        request_metadata = []
        for entry in provider.bodies:
            body = entry.get("body", {})
            request_metadata.append({
                "protocol": entry.get("protocol"),
                "model": body.get("model"),
                "serviceTier": body.get("service_tier"),
                "inputItemCount": len(body.get("input", [])),
                "messageCount": len(body.get("messages", [])),
            })
        proof["providerRequests"] = request_metadata
        (output / "fullcore-proof.json").write_text(json.dumps(proof, indent=2) + "\n")
        (output / "configuration-proof.json").write_text(
            (root / "configuration-proof.json").read_text()
        )
        print("PORTABLE_HANDOFF_EVIDENCE", str(output), flush=True)
        print("PORTABLE_HANDOFF_PASSED", json.dumps(proof, ensure_ascii=False), flush=True)
        return 0
    finally:
        if app is not None:
            try:
                app.shutdown_stopping_work()
            except Exception:
                pass
        provider.close()
        (root / "provider.pid").unlink(missing_ok=True)
        (root / "provider-stopped.json").write_text(json.dumps({"pid": os.getpid(), "stoppedAtUnix": time.time()}, indent=2))
        if not args.keep_fixture and (root / "state" / ".velune-development.json").is_file():
            try:
                cleanup_fixture(root)
            except Exception as error:
                print(f"synthetic fixture cleanup retained for diagnosis: {error}", file=sys.stderr)


if __name__ == "__main__":
    raise SystemExit(main())
