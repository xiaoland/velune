#!/usr/bin/env python3
"""Exercise the same existing-conversation lifecycle on matched Velune builds.

Run once per build with its generated Python modules and dylib, then compare
the JSON checks. Uses a real Codex CLI, private HOME, and loopback model; never
opens a GUI or reads a user's credentials, configuration, or conversations.

Generate the Python modules beside the exact dylib with velune-bindgen
generate --library --no-format --language python --out-dir DIR DYLIB.
Pass DIR as --bindings and that same build's runtime resources as --resources.
Use --history-items 2467 to reproduce a paginated snapshot of 2,469 messages.
The JSON includes the dylib digest so runs can be matched to build receipts.
"""
from __future__ import annotations

import argparse
import ast
from collections import Counter
import fcntl
import hashlib
from http.server import BaseHTTPRequestHandler, ThreadingHTTPServer
import importlib
import inspect
import json
import os
from pathlib import Path
import sys
import tempfile
import threading
import time
import types


RUNTIME = "parity-codex"
MODEL = "gpt-6.1-sol"
STAGES = ("seed", "resume", "queue", "stop", "recover")
PROMPT = {stage: f"PARITY-INPUT-{stage}" for stage in STAGES}
REPLY = {stage: f"PARITY-OUTPUT-{stage}" for stage in STAGES}


def history_item(index):
    return f"PARITY-HISTORY-{index:04d} " + "abcd" * 48


class Provider:
    def __init__(self, history_items):
        self.started = {stage: threading.Event() for stage in STAGES}
        self.released = {stage: threading.Event() for stage in STAGES}
        self.calls = Counter()
        self.title_requests = 0
        for stage in ("seed", "queue", "recover"):
            self.released[stage].set()
        provider = self

        class Handler(BaseHTTPRequestHandler):
            def log_message(self, *_):
                pass

            def write(self, content_type, value):
                self.send_response(200)
                self.send_header("Content-Type", content_type)
                self.send_header("Content-Length", str(len(value)))
                self.end_headers()
                try:
                    self.wfile.write(value)
                except OSError:
                    pass  # Expected if the held request was interrupted.

            def do_GET(self):
                if self.path.rstrip("/").endswith("models"):
                    value = {"object": "list", "data": [{"id": MODEL, "object": "model", "owned_by": "synthetic"}]}
                    self.write("application/json", json.dumps(value).encode())
                else:
                    self.send_error(404)

            def do_POST(self):
                if not self.path.split("?", 1)[0].endswith("/responses"):
                    self.send_error(404)
                    return
                body = json.loads(self.rfile.read(int(self.headers["Content-Length"])))
                title = ("Generate task titles only" in str(body.get("instructions", ""))
                         or "title" in (body.get("text", {}).get("format", {}).get("schema", {}).get("properties", {})))
                users = [item for item in body.get("input", []) if item.get("role") == "user"]
                request = json.dumps(users[-1]) if users else ""
                stage = "title" if title else next((stage for stage in STAGES if PROMPT[stage] in request), None)
                if stage is None:
                    self.send_error(400, "fixture requires a recognized final user prompt")
                    return
                if title:
                    provider.title_requests += 1
                    answer = json.dumps({"title": "Synthetic parity conversation"})
                else:
                    provider.calls[stage] += 1
                    provider.started[stage].set()
                    if not provider.released[stage].wait(60):
                        self.send_error(504, "fixture request held too long")
                        return
                    answer = REPLY[stage]
                item = {"type": "message", "id": f"message-{stage}", "status": "completed",
                        "role": "assistant", "phase": "final_answer",
                        "content": [{"type": "output_text", "text": answer, "annotations": []}]}
                items = [dict(item, id=f"history-{index:04d}", phase="commentary",
                              content=[dict(type="output_text", text=history_item(index), annotations=[])])
                         for index in range(history_items if stage == "seed" else 0)] + [item]
                response = {"id": f"response-{stage}", "object": "response", "status": "completed",
                            "model": MODEL, "output": items,
                            "usage": {"input_tokens": 16, "output_tokens": 4, "total_tokens": 20}}
                events = [{"type": "response.created", "response": dict(response, status="in_progress", output=[])}]
                for index, entry in enumerate(items):
                    events.append({"type": "response.output_item.added", "output_index": index, "item": entry})
                    events.append({"type": "response.output_item.done", "output_index": index, "item": entry})
                events.append({"type": "response.completed", "response": response})
                payload = "".join(f"event: {event['type']}\ndata: {json.dumps(event)}\n\n" for event in events)
                self.write("text/event-stream", payload.encode())

        self.server = ThreadingHTTPServer(("127.0.0.1", 0), Handler)
        self.thread = threading.Thread(target=self.server.serve_forever, daemon=True)
        self.thread.start()

    def close(self):
        for gate in self.released.values():
            gate.set()
        self.server.shutdown()
        self.server.server_close()
        self.thread.join()


def load_bindings(directory, scratch):
    scratch.mkdir(mode=0o700)
    for source in sorted(directory.glob("velune_*.py")):
        code = source.read_text()
        # UniFFI 0.32.2 can put a default before required arguments in
        # generated Command subclasses. This check uses exported methods only.
        while True:
            try:
                ast.parse(code, filename=str(source))
                break
            except SyntaxError as error:
                lines = code.splitlines(keepends=True)
                line = lines[error.lineno - 1]
                if error.msg != "parameter without a default follows parameter with a default" or "def __init__(self, " not in line:
                    raise
                lines[error.lineno - 1] = line.replace("def __init__(self, ", "def __init__(self, *, ", 1)
                code = "".join(lines)
        (scratch / source.name).write_text(code)
    (scratch / "libvelune_bindings.dylib").symlink_to(directory / "libvelune_bindings.dylib")
    package = types.ModuleType("parity_bindings")
    package.__path__ = [str(scratch)]
    sys.modules[package.__name__] = package
    return [importlib.import_module(f"{package.__name__}.velune_{name}")
            for name in ("bindings", "application_api", "conversation")]


def prepare(root, provider, resources):
    home, codex, project = (root / part for part in ("home", "codex", "project"))
    for directory in (home, codex, project, root / "state"):
        directory.mkdir(mode=0o700)
    (root / "resources").symlink_to(resources, target_is_directory=True)
    (codex / "config.toml").write_text(f'''model = "{MODEL}"
model_provider = "synthetic"
approval_policy = "never"
sandbox_mode = "read-only"
model_post_turn_compact_threshold_percent = 0
[model_providers.synthetic]
name = "Synthetic local provider"
base_url = "http://127.0.0.1:{provider.server.server_port}/v1"
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
''')
    inherited_tmp = os.environ.get("TMPDIR", "/tmp")
    os.environ.clear()
    os.environ.update(HOME=str(home), CODEX_HOME=str(codex), VELUNE_HOME=str(root / "state"),
                      TMPDIR=inherited_tmp, PATH="/usr/bin:/bin:/usr/sbin:/sbin",
                      DISABLE_TELEMETRY="1", DISABLE_ERROR_REPORTING="1")
    for key in ("HTTP_PROXY", "HTTPS_PROXY", "ALL_PROXY"):
        os.environ[key] = os.environ[key.lower()] = "http://127.0.0.1:1"
    os.environ["NO_PROXY"] = os.environ["no_proxy"] = "localhost,127.0.0.1"
    return home, codex, project


def run(args, root, provider, proof):
    home, codex, project = prepare(root, provider, args.resources)
    bindings, api, conversation = load_bindings(args.bindings, root / "bindings")
    options = bindings.BindingOptions(home_directory=str(root / "state"), resources_directory=str(root / "resources"))
    runtime = api.RuntimeInstance(id=RUNTIME, name="Synthetic parity Codex", enabled=True,
        type_id=conversation.RuntimeKind.CODEX166, gateway_id="default",
        settings={"binary": str(args.codex), "agentDir": str(codex), "nodeBinary": "/usr/bin/false"})
    execution = api.RuntimeExecution(transport=api.RuntimeTransport.LOCAL(),
        home=api.RuntimeHome.EXISTING(path=str(home)), remote_resources_directory=None)
    model = conversation.ModelSelection.NATIVE(provider_id="synthetic", model_id=MODEL)
    app = bindings.VeluneApplication.open(options)
    supports_submission_id = "submission_id" in inspect.signature(app.send_turn).parameters
    key = None

    def texts(snapshot):
        return [block.text for message in snapshot.messages for block in message.blocks if block.is_text()]

    def read():
        return app.snapshot(RUNTIME, key).snapshot

    def wait(label, predicate):
        deadline = time.monotonic() + args.timeout
        while time.monotonic() < deadline:
            snapshot = read()
            if snapshot is not None and predicate(snapshot):
                return snapshot
            time.sleep(0.05)
        snapshot = read()
        observed = None if snapshot is None else {
            "state": snapshot.run_state.name, "access": snapshot.actions.input_access.name,
            "canSend": snapshot.actions.can_send, "canCancel": snapshot.actions.can_cancel,
            "messageCount": len(snapshot.messages), "recentText": [text[:100] for text in texts(snapshot)[-8:]],
            "queueLength": len(snapshot.follow_up.queue),
        }
        raise TimeoutError(f"{label}; last snapshot={observed}, provider calls={provider.calls}")

    def compose(stage):
        return api.ComposedTurn(text=PROMPT[stage], effort=None, permission_mode=None, attachments=[])

    def submit(stage):
        tracking = {"submission_id": f"parity-{stage}"} if supports_submission_id else {}
        return app.send_turn(RUNTIME, key, RUNTIME, None, compose(stage), **tracking)

    def close_when_idle():
        deadline = time.monotonic() + args.timeout
        while True:
            try:
                app.shutdown()
                return
            except bindings.BindingError as error:
                # Older builds retain the isolated automatic title turn after
                # the conversation itself has already published Idle.
                if getattr(error, "detail", "") != "runtime is busy" or time.monotonic() >= deadline:
                    raise
                read()
                time.sleep(0.05)

    try:
        app.upsert_runtime_with_execution(runtime, execution)
        created = app.create_conversation(RUNTIME, str(project), model).snapshot
        key = created.conversation.id
        submit("seed")
        wait("new conversation must send and settle", lambda s: s.run_state.name == "IDLE" and REPLY["seed"] in texts(s))
        proof["new_send_completed"] = True
        close_when_idle()

        rollouts = list((codex / "sessions").rglob("rollout-*.jsonl"))
        assert len(rollouts) == 1, rollouts
        app = bindings.VeluneApplication.open(options)
        app.open_conversation(RUNTIME, key)
        reopened = wait("cold reopen must keep the same conversation and history", lambda s: s.actions.can_send and REPLY["seed"] in texts(s))
        assert reopened.conversation.id == key and reopened.context_runtime_id == RUNTIME
        assert texts(reopened).count(PROMPT["seed"]) == texts(reopened).count(REPLY["seed"]) == 1
        if args.history_items:
            proof["cold_reopen_message_count"] = len(reopened.messages)
            history = texts(reopened)
            assert history.count(history_item(0)) == 1, [text[:35] for text in history[:8]]
            assert history.count(history_item(args.history_items - 1)) == 1, [text[:35] for text in history[-8:]]
            assert len(reopened.messages) >= args.history_items + 2, len(reopened.messages)
        proof["cold_reopen_history_and_send_access"] = True

        # Hold the precise local writer lock that the Codex adapter observes.
        # This models an external controller without running an external turn.
        with rollouts[0].open() as rollout:
            native_id = json.loads(rollout.readline())["payload"]["id"]
        lock = codex / "thread-writer-locks" / f"{native_id}.lock"
        lock.parent.mkdir(exist_ok=True)
        with lock.open("a+") as writer:
            fcntl.flock(writer, fcntl.LOCK_EX | fcntl.LOCK_NB)
            app.open_conversation(RUNTIME, key)
            held = wait("refresh must publish the external writer", lambda s: s.actions.input_access.name == "EXTERNAL_CONTROLLER")
            assert not held.actions.can_send and not held.actions.can_cancel
            assert REPLY["seed"] in texts(held)
        app.open_conversation(RUNTIME, key)
        wait("release and refresh must restore send access", lambda s: s.actions.can_send)
        proof["external_writer_refresh_recovers"] = True

        submit("resume")
        assert provider.started["resume"].wait(args.timeout), "resume did not reach the model"
        active = wait("active turn must offer cancel", lambda s: s.run_state.name == "RUNNING" and s.actions.can_cancel)
        assert PROMPT["seed"] in texts(active) and REPLY["seed"] in texts(active), "resume dropped history while running"
        if args.history_items:
            assert texts(active).count(history_item(args.history_items - 1)) == 1
        proof["resumed_turn_preserves_history_and_is_stoppable"] = True
        operation = api.SendQueueOperation.ADD(source_runtime_id=RUNTIME, target_runtime_id=RUNTIME,
                                               selection=None, turn=compose("queue"))
        tracking = {"submission_id": "parity-queue"} if supports_submission_id else {}
        result = app.send_queue(RUNTIME, key, operation, **tracking)
        if supports_submission_id:
            assert result.submission_receipt.is_QUEUED(), result.submission_receipt
        queued = wait("queued input must have a visible queue entry", lambda s: bool(s.follow_up.queue))
        assert [item.text for item in queued.follow_up.queue] == [PROMPT["queue"]]
        assert PROMPT["queue"] not in texts(queued) and not provider.started["queue"].is_set()
        proof["accepted_queue_has_no_early_transcript_entry"] = True
        provider.released["resume"].set()
        drained = wait("successful turn must drain its queued input", lambda s: s.run_state.name == "IDLE" and REPLY["queue"] in texts(s))
        history = texts(drained)
        expected = [value for stage in ("seed", "resume", "queue") for value in (PROMPT[stage], REPLY[stage])]
        assert [text for text in history if text in expected] == expected and not drained.follow_up.queue
        proof["queue_drains_once_in_native_order"] = True

        submit("stop")
        assert provider.started["stop"].wait(args.timeout), "stop turn did not reach the model"
        wait("pending synthetic answer must be cancellable", lambda s: s.actions.can_cancel)
        app.cancel(RUNTIME, key)
        cancelled = wait("interrupt must release active state", lambda s: s.run_state.name == "IDLE" and not s.actions.can_cancel)
        assert REPLY["stop"] not in texts(cancelled)
        proof["stop_releases_active_state_without_an_answer"] = True
        close_when_idle()

        app = bindings.VeluneApplication.open(options)
        app.open_conversation(RUNTIME, key)
        after_stop = wait("restart after stop must restore history and access", lambda s: s.actions.can_send and REPLY["queue"] in texts(s))
        assert [text for text in texts(after_stop) if text in expected] == expected
        assert REPLY["stop"] not in texts(after_stop)
        submit("recover")
        recovered = wait("reopened conversation must accept the next turn", lambda s: s.run_state.name == "IDLE" and REPLY["recover"] in texts(s))
        assert recovered.conversation.id == key and recovered.context_runtime_id == RUNTIME
        assert texts(recovered).count(PROMPT["recover"]) == texts(recovered).count(REPLY["recover"]) == 1
        assert provider.calls == Counter({stage: 1 for stage in STAGES}), provider.calls
        proof["reopen_after_stop_and_send_once"] = True
    except Exception:
        if key is not None:
            try:
                app.cancel(RUNTIME, key)
            except bindings.BindingError:
                pass
        for gate in provider.released.values():
            gate.set()
        try:
            app.shutdown()
        except bindings.BindingError:
            pass  # Preserve the original failure while the owned handle drops.
        raise
    else:
        close_when_idle()


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--bindings", type=Path, required=True, help="matching Python modules and libvelune_bindings.dylib")
    parser.add_argument("--codex", type=Path, required=True, help="Codex 0.166+ CLI binary")
    parser.add_argument("--resources", type=Path, required=True, help="runtime resources belonging to the tested build")
    parser.add_argument("--label", required=True, help="source revision or build identity to include in output")
    parser.add_argument("--timeout", type=float, default=15.0, help="per-phase timeout in seconds")
    parser.add_argument("--history-items", type=int, default=0, help="additional persisted model output items in the seed turn")
    args = parser.parse_args()
    if args.timeout <= 0 or args.history_items < 0:
        parser.error("timeout must be positive and history items must be nonnegative")
    for name in ("bindings", "codex", "resources"):
        setattr(args, name, getattr(args, name).expanduser().resolve(strict=True))
    proof = {}
    with (args.bindings / "libvelune_bindings.dylib").open("rb") as library:
        digest = hashlib.file_digest(library, "sha256").hexdigest()
    result = {"build": args.label, "bindings_sha256": digest,
              "fixture_history_items": args.history_items, "checks": proof}
    provider = Provider(args.history_items)
    try:
        with tempfile.TemporaryDirectory(prefix="velune-conversation-parity-") as temporary:
            run(args, Path(temporary).resolve(), provider, proof)
    except Exception as error:
        result["error"] = f"{type(error).__name__}: {error}"
        raise
    finally:
        provider.close()
        result["provider_calls"] = dict(provider.calls)
        result["metadata_title_requests"] = provider.title_requests
        print(json.dumps(result, ensure_ascii=False, indent=2))


if __name__ == "__main__":
    main()
