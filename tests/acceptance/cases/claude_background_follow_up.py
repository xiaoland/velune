"""Check native Claude routing, then keep a managed gateway lease across a
background Bash, follow-up and native task callback on one real CLI child.
Private HOME and a sandbox limited to loopback.
"""
from __future__ import annotations

import json
import os
import shlex
import subprocess
import sys
import threading
from http.server import BaseHTTPRequestHandler, ThreadingHTTPServer
from uuid import uuid4

from velune_acceptance import Context, Skip
from velune_acceptance.app import Driver, texts

REQUIRES = ("claude",)
RUNTIME = "acceptance-claude"
MODEL = "claude-sonnet-4-6"
NATIVE_TOKEN = "synthetic-claude-native-token"
MANAGED_TOKEN = "synthetic-claude-managed-token"
NATIVE_PROMPT = "SYNTHETIC-NATIVE-ROUTE"
FIRST = "SYNTHETIC-BASH-FIRST-INPUT"
SECOND = "SYNTHETIC-BASH-SECOND-INPUT"
REPLY = {"native": "SYNTHETIC-NATIVE-REPLY",
         "first": "SYNTHETIC-BASH-FIRST-REPLY",
         "second": "SYNTHETIC-BASH-SECOND-REPLY",
         "callback": "SYNTHETIC-BASH-CALLBACK-REPLY"}
TASK_TITLE = "Controlled background Bash"
TOOL_USE_ID = "toolu_01VeluneBackgroundBash"
COMMAND = ("touch started; for attempt in {1..1200}; do "
           "if [ -f release ]; then touch completed; exit 0; fi; sleep 0.05; done; exit 3")


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
    return "".join(f"event: {kind}\ndata: {json.dumps(dict(type=kind, **fields))}\n\n"
                   for kind, fields in events).encode()


class MessagesProvider:
    """Anthropic Messages endpoint whose second reply and task callback are
    held until the case releases them."""

    def __init__(self, project, timeout):
        self.requests = []
        self.second_arrived, self.callback_arrived = threading.Event(), threading.Event()
        self.release_second, self.release_callback = threading.Event(), threading.Event()
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
                content = json.dumps(last_user["content"])
                if NATIVE_PROMPT in content:
                    stage = "native"
                elif "<task-notification>" in content and TOOL_USE_ID in content:
                    stage = "callback"
                elif SECOND in content:
                    stage = "second"
                elif TOOL_USE_ID in content and '"type": "tool_result"' in content:
                    stage = "first"
                elif FIRST in content:
                    stage = "bash"
                else:
                    fixture.requests.append({"stage": "unexpected",
                                             "roles": [message["role"] for message in body["messages"]]})
                    self.send_error(400, "no recognized fixture input")
                    return
                fixture.requests.append({"stage": stage, "model": body["model"],
                    "route": path,
                    "authenticated": (self.headers.get("authorization") == f"Bearer {NATIVE_TOKEN}"
                                      if path == "/v1/messages" else self.headers.get("x-api-key") == MANAGED_TOKEN),
                    "bash_started": (project / "started").exists(),
                    "bash_completed": (project / "completed").exists()})
                gate = None
                if stage == "second":
                    fixture.second_arrived.set()
                    gate = fixture.release_second
                elif stage == "callback":
                    fixture.callback_arrived.set()
                    gate = fixture.release_callback
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
        self.port = self.server.server_port
        self.thread = threading.Thread(target=self.server.serve_forever, daemon=True)
        self.thread.start()

    def release(self):
        self.release_second.set()
        self.release_callback.set()

    def close(self):
        self.release()
        self.server.shutdown()
        self.server.server_close()
        self.thread.join()


def wrapper(binary, root):
    """Record the exact child PID and exec Claude with network limited to the
    loopback interface, so the CLI owns that PID while stream-json stays connected."""
    sandbox = '(version 1)(allow default)(deny network*)(allow network-outbound (remote ip "localhost:*"))'
    path = root / "claude"
    path.write_text(f"""#!/bin/sh
printf '%s %s\\n' "$$" "$*" >> {shlex.quote(str(root / "launches"))}
exec /usr/bin/sandbox-exec -p {shlex.quote(sandbox)} {shlex.quote(str(binary))} "$@"
""")
    path.chmod(0o700)
    return path


def managed_selection(app, api, conversation, port):
    provider = api.ProviderDraft(
        id="synthetic", name="Synthetic provider", protocol=api.Protocol.MESSAGES,
        endpoint=f"http://127.0.0.1:{port}/managed",
        execution_backend=api.ExecutionBackend.HTTP(),
        request_profile=api.RequestProfile.NATIVE,
        authentication_source=api.ProviderAuthenticationSource.API_KEY,
        quota_source=api.ProviderQuotaSource.NONE, quota_limits=[], codex_client_version=None,
        models=[api.ProviderModel(record_key="", provider_model_id=MODEL,
                                  context_window=128000, max_output_tokens=4096)])
    updated = app.save_provider("default", provider, api.AuthenticationEdit.SET_API_KEY(MANAGED_TOKEN))
    saved = next(item for gateway in updated.gateways for item in gateway.providers if item.id == "synthetic")
    return conversation.ModelSelection.MANAGED(model_record_key=saved.models[0].record_key)


def run(ctx: Context) -> None:
    if sys.platform != "darwin":
        raise Skip("needs sandbox-exec to confine the real Claude CLI to the fixture")
    b = ctx.bindings
    api, conversation = b.application_api, b.conversation
    project = ctx.isolated.project
    provider = MessagesProvider(project, ctx.timeout)
    os.environ.update(CLAUDE_CODE_DISABLE_NONESSENTIAL_TRAFFIC="1", DISABLE_AUTOUPDATER="1",
                      SHELL="/bin/bash", TERM="dumb")
    os.environ.update(ANTHROPIC_BASE_URL=f"http://127.0.0.1:{provider.port}",
                      ANTHROPIC_AUTH_TOKEN=NATIVE_TOKEN)
    # Velune allocates its gateway port independently of the provider.
    binary = wrapper(ctx.tool("claude"), ctx.isolated.root)
    ctx.facts["claude"] = subprocess.check_output([str(binary), "--version"], text=True).strip()

    options = b.bindings.BindingOptions(home_directory=str(ctx.isolated.state),
                                        resources_directory=str(ctx.isolated.resources))
    runtime = api.RuntimeInstance(id=RUNTIME, name="Synthetic Claude", enabled=True,
        type_id=conversation.RuntimeKind.CLAUDE_CODE, gateway_id="default",
        settings={"binary": str(binary), "agentDir": str(ctx.isolated.dir("claude-home"))})
    execution = api.RuntimeExecution(transport=api.RuntimeTransport.LOCAL(),
        home=api.RuntimeHome.EXISTING(path=str(ctx.isolated.home)), remote_resources_directory=None)
    model = conversation.ModelSelection.NATIVE(provider_id=None, model_id=MODEL)
    driver = Driver(b, options, RUNTIME, ctx.timeout)
    app = driver.app
    requests = lambda: f"requests={provider.requests}"

    def finished():
        return [event for event in app.conversation_activity(0).finished
                if event.conversation_id == driver.key and event.context_runtime_id == RUNTIME]

    def check_finished(count):
        turns = finished()
        assert len(turns) == count, [(event.sequence, event.outcome.name) for event in turns]
        assert all(event.outcome.name == "COMPLETED" and not event.silent for event in turns), turns
        assert len({event.sequence for event in turns}) == count
        return [event.sequence for event in turns]

    def background(status, output_ready=False):
        return next((task for task in app.background_tasks(RUNTIME, driver.key).tasks
                     if task.title == TASK_TITLE and task.status.name == status
                     and (not output_ready or task.can_read_output)), None)

    def until(label, observe):
        """Wait for a non-snapshot condition and return what it observed."""
        found = None

        def ready(_snapshot):
            nonlocal found
            found = observe()
            return bool(found)

        driver.wait(label, ready, requests)
        return found

    def submit(text):
        turn = api.ComposedTurn(text=text, effort=None, permission_mode="bypassPermissions", attachments=[])
        return app.send_turn(RUNTIME, driver.key, RUNTIME, model, turn)

    def launches():
        native_id = driver.key.split(":", 1)[1]
        lines = (ctx.isolated.root / "launches").read_text().splitlines()
        traced = [int(pid) for pid, arguments in (line.split(" ", 1) for line in lines)
                  if native_id in arguments.split()]
        assert traced, f"no CLI invocation for native session {native_id}: {lines}"
        return traced

    def exited(pid):
        try:
            os.kill(pid, 0)
        except ProcessLookupError:
            return True
        return False

    try:
        app.upsert_runtime_with_execution(runtime, execution)
        driver.key = app.create_conversation(RUNTIME, str(project), model).snapshot.conversation.id
        submit(NATIVE_PROMPT)
        driver.wait("native credentials did not deliver a reply",
                    lambda s: REPLY["native"] in texts(s) and s.run_state.name == "IDLE", requests)
        native_request, = provider.requests
        assert native_request["route"] == "/v1/messages" and native_request["authenticated"], native_request
        assert native_request["model"] == MODEL, native_request
        ctx.passed("native_endpoint_and_credentials_deliver_the_selected_model")

        model = managed_selection(app, api, conversation, provider.port)
        driver.key = app.create_conversation(RUNTIME, str(project), model).snapshot.conversation.id
        submit(FIRST)
        driver.wait("first foreground result missing",
                    lambda s: REPLY["first"] in texts(s) and finished(), requests)
        first_sequence = check_finished(1)
        running = until("Bash should survive the first foreground result", lambda: background("RUNNING"))
        assert (project / "started").exists() and not (project / "completed").exists()
        first_pid, = launches()
        ctx.passed("background_bash_outlives_foreground_result")

        submit(SECOND)
        until("follow-up should reach the model during background Bash", provider.second_arrived.is_set)
        assert launches() == [first_pid], launches()
        assert provider.requests[-1]["bash_started"] and not provider.requests[-1]["bash_completed"]
        assert driver.read().run_state.name == "RUNNING"
        ctx.passed("follow_up_reuses_live_child")

        (project / "release").touch()
        completed = until("Bash completion should be visible during the held second turn",
                          lambda: background("COMPLETED", output_ready=True))
        assert completed.id == running.id
        output = app.background_task_output(RUNTIME, driver.key, completed.id, None)
        assert output.status.name == "READY" and TASK_TITLE in output.text, output
        assert driver.read().run_state.name == "RUNNING"
        assert check_finished(1) == first_sequence and not provider.callback_arrived.is_set()
        ctx.passed("bash_completion_keeps_user_turn_running")

        provider.release_second.set()
        until("task callback should reach the model after the second reply", provider.callback_arrived.is_set)
        driver.wait("second foreground result missing during the held callback",
                    lambda s: REPLY["second"] in texts(s) and len(finished()) >= 2, requests)
        assert check_finished(2)[:1] == first_sequence
        assert launches() == [first_pid] and not exited(first_pid)
        assert REPLY["callback"] not in texts(driver.read())
        provider.release_callback.set()
        final = driver.wait("callback should deliver a separate reply",
                            lambda s: REPLY["callback"] in texts(s) and s.run_state.name == "IDLE", requests)
        until("Claude should release its child after the callback settles", lambda: exited(first_pid))
        assert [text for text in texts(final) if text in {*REPLY.values(), FIRST, SECOND}] == [
            FIRST, REPLY["first"], SECOND, REPLY["second"], REPLY["callback"]]
        assert [block.text for message in final.messages if message.role.name == "USER"
                for block in message.blocks if block.is_text()] == [FIRST, SECOND]
        check_finished(2)
        assert launches() == [first_pid], launches()
        assert [request["stage"] for request in provider.requests] == ["native", "bash", "first", "second", "callback"]
        assert all(request["route"] == "/managed/v1/messages" and request["authenticated"]
                   and request["model"] == MODEL for request in provider.requests[1:])
        ctx.passed("callback_is_a_separate_reply_on_the_same_child")
        driver.close_when_idle()
    except BaseException:
        (project / "release").touch()
        provider.release()
        driver.abandon()
        raise
    finally:
        ctx.facts["requests"] = provider.requests
        provider.close()
