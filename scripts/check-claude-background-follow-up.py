#!/usr/bin/env python3
"""Exercise Claude background Bash and follow-up through the Velune Bindings.

Use the real Claude Code CLI on macOS and a private, synthetic loopback
provider. Pass matching generated Python modules and dylib as --bindings.
"""
from __future__ import annotations

import argparse
import hashlib
from http.server import BaseHTTPRequestHandler, ThreadingHTTPServer
import json
import os
from pathlib import Path
import runpy
import shlex
import subprocess
import sys
import tempfile
import threading
import time
from uuid import uuid4


RUNTIME = "fixture-claude"
MODEL = "claude-sonnet-4-6"
NATIVE_TOKEN = "synthetic-claude-native-token"
MANAGED_TOKEN = "synthetic-claude-managed-token"
FIRST = "SYNTHETIC-BASH-FIRST-INPUT"
SECOND = "SYNTHETIC-BASH-SECOND-INPUT"
REPLY = {"first": "SYNTHETIC-BASH-FIRST-REPLY",
         "second": "SYNTHETIC-BASH-SECOND-REPLY",
         "callback": "SYNTHETIC-BASH-CALLBACK-REPLY"}
TASK_TITLE = "Controlled background Bash"
TOOL_USE_ID = "toolu_01VeluneBackgroundBash"
COMMAND = (
    "touch started; for attempt in {1..1200}; do "
    "if [ -f release ]; then touch completed; exit 0; fi; sleep 0.05; done; exit 3"
)


def response(model, stage):
    if stage == "bash":
        block = {"type": "tool_use", "id": TOOL_USE_ID, "name": "Bash", "input": {}}
        tool_input = {"command": COMMAND, "description": TASK_TITLE, "run_in_background": True}
        delta = {"type": "input_json_delta", "partial_json": json.dumps(tool_input)}
        stop = "tool_use"
    else:
        block = {"type": "text", "text": ""}
        delta = {"type": "text_delta", "text": REPLY[stage]}
        stop = "end_turn"
    events = [
        ("message_start", {"message": {"id": "msg_" + uuid4().hex, "type": "message",
            "role": "assistant", "model": model, "content": [], "stop_reason": None,
            "usage": {"input_tokens": 10, "output_tokens": 1}}}),
        ("content_block_start", {"index": 0, "content_block": block}),
        ("content_block_delta", {"index": 0, "delta": delta}),
        ("content_block_stop", {"index": 0}),
        ("message_delta", {"delta": {"stop_reason": stop, "stop_sequence": None},
                           "usage": {"output_tokens": 5}}),
        ("message_stop", {}),
    ]
    return "".join(
        f"event: {kind}\ndata: {json.dumps(dict(type=kind, **fields))}\n\n"
        for kind, fields in events
    ).encode()


class Provider:
    def __init__(self, project, timeout):
        self.requests = []
        self.second_arrived = threading.Event()
        self.callback_arrived = threading.Event()
        self.release_second = threading.Event()
        self.release_callback = threading.Event()
        fixture = self

        class Handler(BaseHTTPRequestHandler):
            def log_message(self, *_):
                pass

            def do_POST(self):
                path = self.path.split("?", 1)[0]
                if path not in ("/v1/messages", "/managed/v1/messages"):
                    fixture.requests.append({"stage": "unexpected", "path": path})
                    self.send_error(404)
                    return
                body = json.loads(self.rfile.read(int(self.headers["Content-Length"])))
                last_user = next(item for item in reversed(body["messages"]) if item["role"] == "user")
                last_content = json.dumps(last_user["content"])
                if "<task-notification>" in last_content and TOOL_USE_ID in last_content:
                    stage = "callback"
                elif SECOND in last_content:
                    stage = "second"
                elif TOOL_USE_ID in last_content and '"type": "tool_result"' in last_content:
                    stage = "first"
                elif FIRST in last_content:
                    stage = "bash"
                else:
                    fixture.requests.append({"stage": "unexpected",
                        "message_roles": [message["role"] for message in body["messages"]]})
                    self.send_error(400, "no recognized fixture input")
                    return
                fixture.requests.append({"stage": stage, "model": body["model"],
                    "managed": path.startswith("/managed/") and self.headers.get("x-api-key") == MANAGED_TOKEN,
                    "bash_started": (project / "started").exists(),
                    "bash_completed": (project / "completed").exists()})
                if stage == "second":
                    fixture.second_arrived.set()
                    gate = fixture.release_second
                elif stage == "callback":
                    fixture.callback_arrived.set()
                    gate = fixture.release_callback
                else:
                    gate = None
                if gate is not None and not gate.wait(timeout * 2):
                    self.send_error(504, "fixture gate timed out")
                    return
                payload = response(body["model"], stage)
                self.send_response(200)
                self.send_header("Content-Type", "text/event-stream")
                self.send_header("Content-Length", str(len(payload)))
                self.end_headers()
                self.wfile.write(payload)

        self.server = ThreadingHTTPServer(("127.0.0.1", 0), Handler)
        self.thread = threading.Thread(target=self.server.serve_forever, daemon=True)
        self.thread.start()

    def close(self):
        self.release_second.set()
        self.release_callback.set()
        self.server.shutdown()
        self.server.server_close()
        self.thread.join()


def fixture_binary(binary, root, port):
    # The wrapper records exact child identity and uses exec so Claude owns that
    # PID, while the actual stream-json stdin/stdout remain connected to Velune.
    sandbox = f'(version 1)(allow default)(deny network*)(allow network-outbound (remote ip "localhost:{port}"))'
    path = root / "claude"
    path.write_text(f"""#!/bin/sh
printf '%s %s\n' "$$" "$*" >> {shlex.quote(str(root / "launches"))}
if [ "$1" = "--print" ]; then
    set -- "$@" --debug-file {shlex.quote(str(root / "claude.debug"))}
fi
exec /usr/bin/sandbox-exec -p {shlex.quote(sandbox)} {shlex.quote(str(binary))} "$@"
""")
    path.chmod(0o700)
    return path


def prepare_environment(root, port, managed):
    os.environ.clear()
    os.environ.update({
        "HOME": str(root / "home"),
        "VELUNE_HOME": str(root / "state"),
        "TMPDIR": str(root / "temp"),
        "PATH": "/usr/bin:/bin:/usr/sbin:/sbin",
        "CLAUDE_CODE_DISABLE_NONESSENTIAL_TRAFFIC": "1",
        "DISABLE_AUTOUPDATER": "1",
        "SHELL": "/bin/bash",
        "TERM": "dumb",
    })
    if not managed:
        os.environ.update(ANTHROPIC_BASE_URL=f"http://127.0.0.1:{port}",
                          ANTHROPIC_AUTH_TOKEN=NATIVE_TOKEN)


def managed_selection(app, api, conversation, port):
    provider = api.ProviderDraft(
        id="synthetic", name="Synthetic provider", protocol=api.Protocol.MESSAGES,
        endpoint=f"http://127.0.0.1:{port}/managed",
        execution_backend=api.ExecutionBackend.HTTP(),
        request_profile=api.RequestProfile.NATIVE,
        authentication_source=api.ProviderAuthenticationSource.API_KEY,
        quota_source=api.ProviderQuotaSource.NONE, quota_limits=[],
        codex_client_version=None,
        models=[api.ProviderModel(record_key="", provider_model_id=MODEL,
                                 context_window=128000, max_output_tokens=4096)],
    )
    updated = app.save_provider("default", provider, api.AuthenticationEdit.SET_API_KEY(MANAGED_TOKEN))
    saved = next(provider for gateway in updated.gateways for provider in gateway.providers
                 if provider.id == "synthetic")
    return conversation.ModelSelection.MANAGED(model_record_key=saved.models[0].record_key)


def run(args, root, provider, report):
    port = provider.server.server_port
    prepare_environment(root, port, args.managed)
    # Velune allocates the local gateway port independently of the provider.
    binary = fixture_binary(args.claude, root, "*" if args.managed else port)
    report["claude"] = subprocess.check_output([str(binary), "--version"], text=True).strip()
    parity = runpy.run_path(str(Path(__file__).with_name("check-conversation-parity.py")))
    bindings, api, conversation = parity["load_bindings"](args.bindings, root / "bindings")
    app = bindings.VeluneApplication.open(bindings.BindingOptions(
        home_directory=str(root / "state"), resources_directory=str(args.resources)))
    runtime = api.RuntimeInstance(id=RUNTIME, name="Synthetic Claude", enabled=True,
        type_id=conversation.RuntimeKind.CLAUDE_CODE, gateway_id="default",
        settings={"binary": str(binary), "agentDir": str(root / "claude-home")})
    execution = api.RuntimeExecution(transport=api.RuntimeTransport.LOCAL(),
        home=api.RuntimeHome.EXISTING(path=str(root / "home")), remote_resources_directory=None)
    model = conversation.ModelSelection.NATIVE(provider_id=None, model_id=MODEL)
    key = None

    def snapshot():
        return app.snapshot(RUNTIME, key).snapshot

    def texts(frame):
        return [block.text for message in frame.messages for block in message.blocks if block.is_text()]

    def finished():
        return [event for event in app.conversation_activity(0).finished
                if event.conversation_id == key and event.context_runtime_id == RUNTIME]

    def check_finished(count):
        turns = finished()
        assert len(turns) == count, [(event.sequence, event.outcome.name) for event in turns]
        assert all(event.outcome.name == "COMPLETED" and not event.silent for event in turns), turns
        assert len({event.sequence for event in turns}) == count
        return [event.sequence for event in turns]

    def phases():
        frame = snapshot()
        return {"state": frame.run_state.name, "can_send": frame.actions.can_send,
                "texts": texts(frame), "queue": [item.text for item in frame.follow_up.queue],
                "finished": [event.outcome.name for event in finished()]}

    def wait(label, observe):
        deadline = time.monotonic() + args.timeout
        while time.monotonic() < deadline:
            value = observe()
            if value:
                return value
            time.sleep(0.025)
        raise TimeoutError(f"{label}; latest={phases()}")

    def has_text(text):
        frame = snapshot()
        return frame if text in texts(frame) else None

    def background(status, output_ready=False):
        return next((task for task in app.background_tasks(RUNTIME, key).tasks
                     if task.title == TASK_TITLE and task.status.name == status
                     and (not output_ready or task.can_read_output)), None)

    def submit(text):
        turn = api.ComposedTurn(text=text, effort=None, permission_mode="bypassPermissions", attachments=[])
        return app.send_turn(RUNTIME, key, RUNTIME, model, turn)

    def launches():
        native_id = key.split(":", 1)[1]
        invocations = [line.split(" ", 1) for line in (root / "launches").read_text().splitlines()]
        traced = [int(pid) for pid, arguments in invocations if native_id in arguments.split()]
        if not traced:
            raise AssertionError(f"no CLI invocation for native session {native_id}: {invocations}")
        return traced

    def child_exited(pid):
        try:
            os.kill(pid, 0)
        except ProcessLookupError:
            return True
        return False

    try:
        app.upsert_runtime_with_execution(runtime, execution)
        if args.managed:
            model = managed_selection(app, api, conversation, port)
        key = app.create_conversation(RUNTIME, str(root / "project"), model).snapshot.conversation.id
        submit(FIRST)
        wait("first foreground result missing", lambda: has_text(REPLY["first"]) and finished())
        first_sequence = check_finished(1)
        running_task = wait("Bash should survive the first foreground result", lambda: background("RUNNING"))
        assert (root / "project/started").exists() and not (root / "project/completed").exists()
        first_pid, = launches()
        report["first_pid"] = first_pid
        report["first_result_while_bash_active"] = True

        submit(SECOND)
        wait("unchanged follow-up should reach the model during background Bash", provider.second_arrived.is_set)
        assert launches() == [first_pid], launches()
        assert provider.requests[-1]["bash_started"] and not provider.requests[-1]["bash_completed"]
        assert snapshot().run_state.name == "RUNNING"
        report["second_request_reused_live_child"] = True

        (root / "project/release").touch()
        completed_task = wait("Bash completion should be visible during the held second turn",
                              lambda: background("COMPLETED", output_ready=True))
        assert completed_task.id == running_task.id
        output = app.background_task_output(RUNTIME, key, completed_task.id, None)
        assert output.status.name == "READY" and TASK_TITLE in output.text, output
        assert snapshot().run_state.name == "RUNNING"
        assert check_finished(1) == first_sequence and not provider.callback_arrived.is_set()
        report["bash_completion_preserved_running_user_turn"] = True

        provider.release_second.set()
        wait("native task callback should reach its model after the second reply", provider.callback_arrived.is_set)
        wait("second foreground result missing during the held callback", lambda: has_text(REPLY["second"]) and len(finished()) >= 2)
        assert check_finished(2)[:1] == first_sequence
        assert launches() == [first_pid], launches()
        assert not child_exited(first_pid)
        assert not has_text(REPLY["callback"])
        provider.release_callback.set()
        wait("callback should deliver a separate reply", lambda: has_text(REPLY["callback"]))
        wait("Claude should release its child after the callback settles", lambda: child_exited(first_pid))
        final = snapshot()
        assert final.conversation.id == key
        assert final.run_state.name == "IDLE"
        assert [text for text in texts(final) if text in {*REPLY.values(), FIRST, SECOND}] == [
            FIRST, REPLY["first"], SECOND, REPLY["second"], REPLY["callback"],
        ]
        assert [block.text for message in final.messages if message.role.name == "USER"
                for block in message.blocks if block.is_text()] == [FIRST, SECOND]
        report["user_finish_sequences"] = check_finished(2)
        assert launches() == [first_pid], launches()
        assert [request["stage"] for request in provider.requests] == ["bash", "first", "second", "callback"]
        assert all(request["managed"] == args.managed and request["model"] == MODEL for request in provider.requests)
        report["callback_delivered_separately_on_same_child"] = True
        report["child_released_after_callback"] = True
    finally:
        (root / "project/release").touch()
        provider.release_second.set()
        provider.release_callback.set()
        failing = sys.exc_info()[0] is not None
        deadline = time.monotonic() + args.timeout
        while True:
            try:
                app.shutdown()
                break
            except bindings.BindingError as error:
                if getattr(error, "detail", "") == "runtime is busy" and time.monotonic() < deadline:
                    if not failing:
                        check_finished(2)
                    time.sleep(0.05)
                    continue
                if failing:
                    try:
                        app.shutdown_stopping_work()
                    except bindings.BindingError:
                        pass  # Preserve the regression's original failure.
                    break
                raise


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--bindings", type=Path, required=True)
    parser.add_argument("--resources", type=Path, required=True)
    parser.add_argument("--claude", type=Path, default=Path("/opt/homebrew/bin/claude"))
    parser.add_argument("--timeout", type=float, default=10)
    parser.add_argument("--managed", action="store_true", help="route Claude through the Velune gateway")
    args = parser.parse_args()
    if args.timeout <= 0:
        parser.error("--timeout must be positive")
    for field in ("bindings", "resources", "claude"):
        setattr(args, field, getattr(args, field).expanduser().resolve(strict=True))
    with (args.bindings / "libvelune_bindings.dylib").open("rb") as library:
        report = {"bindings_sha256": hashlib.file_digest(library, "sha256").hexdigest(),
                  "managed": args.managed}
    with tempfile.TemporaryDirectory(prefix="velune-claude-background-", dir="/private/tmp") as temp:
        root = Path(temp)
        for name in ("project", "home", "state", "claude-home", "temp"):
            (root / name).mkdir(mode=0o700)
        provider = Provider(root / "project", args.timeout)
        try:
            run(args, root, provider, report)
        except Exception as error:
            report["error"] = f"{type(error).__name__}: {error}"
            debug = root / "claude.debug"
            if debug.exists():
                report["claude_errors"] = [line for line in debug.read_text().splitlines()
                                           if "[ERROR]" in line][-8:]
            raise
        finally:
            provider.close()
            report["requests"] = provider.requests
            print(json.dumps(report, indent=2))


if __name__ == "__main__":
    main()
