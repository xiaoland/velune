"""Codex to Pi handoff through a portable summary.

A real Codex turn seeds the source, and an encrypted-only compaction is
appended so the handoff must prepare a portable summary. The handoff is then
queued, cancelled, failed and retried before it reaches a real Pi target, and
cancelled again while the Pi SDK is still starting. All model traffic goes to
a loopback provider whose summary and target answers are held behind file
gates the case releases.
"""
from __future__ import annotations

import json
import os
import socket
import sys
import threading
import time
from http.server import BaseHTTPRequestHandler, ThreadingHTTPServer
from pathlib import Path

from velune_acceptance import Context

REQUIRES = ("codex", "pi", "node")
SOURCE, TARGET = "codex-source", "pi-target"
TARGET_INPUT = "SYNTHETIC HANDOFF USER INPUT ONCE"
TAIL_INPUT = "SYNTHETIC QUEUED TAIL INPUT ONCE"
SUMMARY_TEXT = "SYNTHETIC PORTABLE SUMMARY: preserve the user's goal, decisions, and constraints."
SEED_INPUT = "SYNTHETIC SOURCE SEED INPUT"
SEED_REPLY = "SYNTHETIC SOURCE SEED REPLY"
TARGET_REPLY = "SYNTHETIC TARGET REPLY"
OPAQUE = "SYNTHETIC-OPAQUE-COMPACTION"
SUMMARY_MODEL = "gpt-6.2-sol"
SOURCE_MODEL = "gpt-6.1-sol"
TARGET_MODEL = "beta"


class LoopbackProvider:
    """Responses API for Codex (seed and summary), Chat Completions for Pi.

    A held stage waits until `release-<stage>` or `fail-<stage>` appears in
    `gates`, or until the client disconnects, which is how a cancelled
    summary is observed."""

    def __init__(self, gates: Path):
        self.gates = gates
        self.bodies: list[dict] = []
        self.events: list[str] = []
        self.guard = threading.Lock()
        self.server = ThreadingHTTPServer(("127.0.0.1", 0), self._handler())
        self.server.daemon_threads = True
        self.thread = threading.Thread(target=self.server.serve_forever, daemon=True)
        self.thread.start()
        self.endpoint = f"http://127.0.0.1:{self.server.server_port}/v1"

    def event(self, name: str) -> None:
        with self.guard:
            self.events.append(name)

    def calls(self, protocol: str, seed: bool | None = None) -> list[dict]:
        with self.guard:
            bodies = list(self.bodies)
        return [entry for entry in bodies if entry["protocol"] == protocol and (
            seed is None or (SEED_INPUT in json.dumps(entry["body"].get("input", []))) == seed)]

    def gate(self, stage: str, client: socket.socket) -> str:
        if stage == "seed":
            return "release"
        deadline = time.monotonic() + 600
        while time.monotonic() < deadline:
            if (self.gates / f"fail-{stage}").exists():
                return "fail"
            if (self.gates / f"release-{stage}").exists():
                return "release"
            try:
                client.setblocking(False)
                try:
                    if client.recv(1, socket.MSG_PEEK) == b"":
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
        return "fail"

    def _handler(self):
        provider = self

        class Handler(BaseHTTPRequestHandler):
            protocol_version = "HTTP/1.1"

            def log_message(self, *_):
                pass

            def _send(self, code: int, payload: bytes, content_type: str):
                self.send_response(code)
                self.send_header("Content-Type", content_type)
                self.send_header("Content-Length", str(len(payload)))
                self.send_header("Connection", "close")
                self.end_headers()
                self.wfile.write(payload)
                self.wfile.flush()

            def _json(self, code: int, value: dict):
                self._send(code, json.dumps(value).encode(), "application/json")

            def do_GET(self):
                if self.path.rstrip("/").endswith("models"):
                    self._json(200, {"object": "list", "data": [
                        {"id": SOURCE_MODEL, "object": "model", "owned_by": "synthetic"},
                        {"id": SUMMARY_MODEL, "object": "model", "owned_by": "synthetic"}]})
                else:
                    self._json(404, {"error": {"message": "synthetic endpoint not found"}})

            def do_POST(self):
                try:
                    body = json.loads(self.rfile.read(int(self.headers.get("Content-Length", "0"))))
                except ValueError:
                    self._json(400, {"error": {"message": "synthetic fixture requires JSON"}})
                    return
                path = self.path.split("?", 1)[0]
                protocol = ("responses" if path.endswith("/responses")
                            else "chat_completions" if path.endswith("/chat/completions") else "unknown")
                with provider.guard:
                    provider.bodies.append({"protocol": protocol, "body": body})
                if protocol == "responses":
                    self._responses("seed" if SEED_INPUT in json.dumps(body.get("input", [])) else "summary", body)
                elif protocol == "chat_completions":
                    self._chat(body)
                else:
                    self._json(404, {"error": {"message": "synthetic endpoint not found"}})

            def _responses(self, stage: str, body: dict):
                action = provider.gate(stage, self.connection)
                if action == "disconnected":
                    return
                # An HTTP-successful empty answer is a semantic failure, not a
                # transport error that a CLI may retry.
                answer = "" if action == "fail" else SEED_REPLY if stage == "seed" else SUMMARY_TEXT
                item = {"type": "message", "id": f"synthetic-{stage}-message", "status": "completed",
                        "role": "assistant", "phase": "final_answer",
                        "content": [{"type": "output_text", "text": answer, "annotations": []}]}
                response = {"id": f"synthetic-{stage}-response", "object": "response", "status": "completed",
                            "model": body.get("model", SOURCE_MODEL), "output": [item],
                            "usage": {"input_tokens": 20, "output_tokens": 8, "total_tokens": 28}}
                events = [
                    {"type": "response.created", "response": {**response, "status": "in_progress", "output": []}},
                    {"type": "response.output_item.added", "output_index": 0, "item": item},
                    {"type": "response.output_item.done", "output_index": 0, "item": item},
                    {"type": "response.completed", "response": response},
                ]
                self._send(200, "".join(f"event: {event['type']}\ndata: {json.dumps(event)}\n\n"
                                        for event in events).encode(), "text/event-stream")

            def _chat(self, body: dict):
                action = provider.gate("target", self.connection)
                if action == "disconnected":
                    return
                if action == "fail":
                    self._json(503, {"error": {"message": "synthetic target failure"}})
                    return
                model = body.get("model", TARGET_MODEL)
                chunks = [{"id": "chatcmpl-synthetic", "object": "chat.completion.chunk", "created": 1794484800,
                           "model": model, "choices": [{"index": 0, "delta": delta, "finish_reason": finish}]}
                          for delta, finish in (({"role": "assistant", "content": TARGET_REPLY}, None), ({}, "stop"))]
                self._send(200, ("".join(f"data: {json.dumps(chunk)}\n\n" for chunk in chunks)
                                 + "data: [DONE]\n\n").encode(), "text/event-stream")

        return Handler

    def close(self):
        self.server.shutdown()
        self.server.server_close()
        self.thread.join(timeout=5)


def codex_config(endpoint: str) -> str:
    return f'''model = "{SOURCE_MODEL}"
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


def pi_config(pi_home: Path, endpoint: str) -> None:
    model = {"reasoning": False, "input": ["text"], "contextWindow": 200000, "maxTokens": 1024,
             "cost": {"input": 0, "output": 0, "cacheRead": 0, "cacheWrite": 0}}
    (pi_home / "models.json").write_text(json.dumps({"providers": {"synthetic": {
        "baseUrl": endpoint, "api": "openai-completions", "apiKey": "synthetic-fake-token",
        "models": [{"id": "alpha", "name": "Synthetic Alpha", **model},
                   {"id": TARGET_MODEL, "name": "Synthetic Beta", **model}]}}}))
    (pi_home / "settings.json").write_text(json.dumps({
        "defaultProvider": "synthetic", "defaultModel": "alpha",
        "compaction": {"enabled": False}, "retry": {"enabled": False}}))


def encrypted_compaction() -> str:
    """A compaction Pi cannot read, which forces a portable summary."""
    return json.dumps({"timestamp": "2026-11-12T00:00:00.000Z", "type": "compacted", "payload": {
        "message": "", "replacement_history": [
            {"type": "message", "id": "fixture-retained-user", "role": "user",
             "content": [{"type": "input_text", "text": "SYNTHETIC RETAINED PREFIX"}]},
            {"type": "compaction", "id": "fixture-encrypted-only", "encrypted_content": OPAQUE},
            {"type": "message", "id": "fixture-retained-tail", "role": "assistant",
             "content": [{"type": "output_text", "text": "SYNTHETIC RETAINED TAIL"}]}]}},
        separators=(",", ":")) + "\n"


def find_rollout(codex_home: Path, state: Path, native_id: str) -> Path:
    rollouts = list((codex_home / "sessions").rglob("*.jsonl"))
    for path in rollouts:
        if native_id in path.name:
            return path
    # A logical draft alias can differ from the physical thread the first
    # send created; resolve it through Velune's own association metadata.
    links_path = state / "conversation-links.json"
    if links_path.exists():
        link = next((entry for entry in json.loads(links_path.read_text()).get("links", [])
                     if entry.get("id") == f"{SOURCE}:{native_id}"), None)
        if link and link.get("segments"):
            physical = link["segments"][-1]["nativeConversationId"].removeprefix(f"{SOURCE}:")
            matches = [path for path in rollouts if physical in path.name]
            if len(matches) == 1:
                return matches[0]
    assert len(rollouts) == 1, f"expected one source rollout for {native_id}; found {[p.name for p in rollouts]}"
    return rollouts[0]


def variant(value, name: str) -> bool:
    method = getattr(value, f"is_{name}", None) or getattr(value, f"is_{name.upper()}", None)
    return bool(method and method())


def texts(snapshot) -> list[str]:
    return [] if snapshot is None else [block.text for message in snapshot.messages
                                        for block in message.blocks if block.is_text()]


def maintenance(snapshot, work_id: str):
    """The single system event that carries a handoff's preparation."""
    matches = [] if snapshot is None else [message for message in snapshot.messages
                                           if message.id == f"velune-handoff-compaction:{work_id}"]
    assert len(matches) == 1 and any(variant(block, "context_compaction") for block in matches[0].blocks), \
        f"expected one maintenance event for work {work_id}"
    assert matches[0].role.name == "SYSTEM", "portable summary appeared as a user or assistant message"
    return matches[0]


def run(ctx: Context) -> None:
    b = ctx.bindings
    bindings, api, conversation = b.bindings, b.application_api, b.conversation
    codex_cli, pi_cli, node = ctx.tool("codex"), ctx.tool("pi"), ctx.tool("node")
    os.environ["PATH"] = f"{node.parent}:{os.environ['PATH']}"
    state, home, workspace = ctx.isolated.state, ctx.isolated.home, ctx.isolated.project
    codex_home, pi_home, gates = ctx.isolated.dir("codex"), ctx.isolated.dir("pi"), ctx.isolated.dir("gates")
    (pi_home / "sessions").mkdir()
    limit = max(ctx.timeout, 60.0)
    provider = LoopbackProvider(gates)
    (codex_home / "config.toml").write_text(codex_config(provider.endpoint))
    pi_config(pi_home, provider.endpoint)
    execution = api.RuntimeExecution(transport=api.RuntimeTransport.LOCAL(),
                                     home=api.RuntimeHome.EXISTING(path=str(home)), remote_resources_directory=None)
    native = lambda model: conversation.ModelSelection.NATIVE(provider_id="synthetic", model_id=model)
    composed = lambda text: api.ComposedTurn(text=text, effort=None, permission_mode=None, attachments=[])
    app = None

    def open_app(pi_node: Path = node):
        opened = bindings.VeluneApplication.open(bindings.BindingOptions(
            home_directory=str(state), resources_directory=str(ctx.isolated.resources)))
        opened.upsert_runtime_with_execution(api.RuntimeInstance(
            id=SOURCE, name="Synthetic Codex source", enabled=True, type_id=conversation.RuntimeKind.CODEX166,
            gateway_id="default",
            settings={"binary": str(codex_cli), "nodeBinary": str(node), "agentDir": str(codex_home)}), execution)
        opened.upsert_runtime_with_execution(api.RuntimeInstance(
            id=TARGET, name="Synthetic Pi target", enabled=True, type_id=conversation.RuntimeKind.PI,
            gateway_id="default",
            settings={"binary": str(pi_cli), "nodeBinary": str(pi_node), "agentDir": str(pi_home),
                      "sessionDir": str(pi_home / "sessions")}), execution)
        gateway = next((g for g in opened.list().gateways if g.id == "default"), None)
        assert gateway is not None and not gateway.routes, "default gateway must exist with no routes"
        return opened

    def wait_for(observe, label, seconds=30.0):
        deadline = time.monotonic() + min(seconds, limit)
        while time.monotonic() < deadline:
            value = observe()
            if value:
                return value
            time.sleep(0.05)
        raise TimeoutError(label)

    def close_idle():
        # A final reply can precede native metadata settlement; a cold reopen
        # must follow an ordinary idle close, never a forced stop.
        deadline = time.monotonic() + limit
        while True:
            try:
                app.shutdown()
                return
            except bindings.BindingError.Diagnostic as error:
                if error.code != "invalid" or error.detail != "runtime is busy" or time.monotonic() >= deadline:
                    raise
            time.sleep(0.1)

    def seed_source():
        """Finish one real Codex turn, close, and make its context unportable."""
        created = app.create_conversation(SOURCE, str(workspace), native(SOURCE_MODEL)).snapshot
        assert created is not None, "Codex source conversation was not returned"
        cid = created.conversation.id
        app.send_turn(SOURCE, cid, SOURCE, None, composed(SEED_INPUT))
        wait_for(lambda: (lambda s: s is not None and s.run_state.name == "IDLE" and SEED_REPLY in texts(s))(
            app.snapshot(SOURCE, cid).snapshot), "seed Codex turn did not settle", limit)
        close_idle()
        path = find_rollout(codex_home, state, cid.rsplit(":", 1)[-1])
        with path.open("a") as stream:
            stream.write(encrypted_compaction())
        return cid, path, path.read_bytes()

    def requirements(cid):
        request = app.read_handoff_requirements(SOURCE, cid, TARGET, None)
        deadline = time.monotonic() + limit
        while variant(request, "CHECKING") and time.monotonic() < deadline:
            time.sleep(0.1)
            request = app.read_handoff_requirements(SOURCE, cid, TARGET, request.request_id)
        assert not variant(request, "FAILED"), f"handoff requirements failed: {request.code}: {request.detail}"
        assert variant(request, "PORTABLE_SUMMARY_REQUIRED"), f"expected a summary requirement, got {request}"
        return request

    def input_ready(cid):
        wait_for(lambda: (lambda s: s is not None and s.actions.can_send)(app.snapshot(SOURCE, cid).snapshot),
                 "source input admission did not settle")

    try:
        app = open_app()
        cid, source_path, source_bytes = seed_source()
        app = open_app()
        current = lambda: app.read_conversation(SOURCE, cid).snapshot
        summaries = lambda: len(provider.calls("responses", seed=False))
        targets = lambda: provider.calls("chat_completions")

        def queue_is(ids, paused=False):
            snapshot = current()
            if snapshot is None or [item.id for item in snapshot.follow_up.queue] != ids:
                return None
            return None if paused and snapshot.follow_up.paused is None else snapshot

        def work_reaches(work_id, name):
            def read():
                status = app.read_handoff_summary(work_id)
                if variant(status, name) or (name == "CANCELLED" and variant(status, "FAILED")
                                             and status.code == "cleanup_pending"):
                    return status
                return None
            return wait_for(read, f"work did not reach {name}")

        app.read_conversation(SOURCE, cid)
        input_ready(cid)
        catalog = requirements(cid).summary_basis.catalog or []
        assert any(variant(model.selection, "NATIVE") and model.selection.provider_id == "synthetic"
                   and model.selection.model_id == SUMMARY_MODEL for model in catalog), \
            f"summary picker did not offer the fixture's {SUMMARY_MODEL}"
        summary_model = native(SUMMARY_MODEL)
        ctx.facts["summary_model"] = SUMMARY_MODEL
        ctx.passed("encrypted_context_requires_a_portable_summary")

        target_selection = native(TARGET_MODEL)
        started = app.queue_handoff_summary(bindings.BindingQueuedHandoffSummary(
            source_runtime_id=SOURCE, conversation_id=cid, target_runtime_id=TARGET,
            selection=target_selection, turn=composed(TARGET_INPUT), summary_model=summary_model, effort=None))
        head = started.queued_message_id
        assert started.result.snapshot is not None and \
            [item.id for item in started.result.snapshot.follow_up.queue] == [head], \
            "queue acknowledgement did not contain its accepted item"
        before = current()
        event = maintenance(before, started.work_id)
        assert not event.completed and before.run_state.name == "IDLE", \
            "preparation did not keep the source idle with a running maintenance event"
        assert TARGET_INPUT not in texts(before), "queued input leaked into the transcript before delivery"
        tail = app.send_queue(SOURCE, cid, api.SendQueueOperation.ADD(
            source_runtime_id=SOURCE, target_runtime_id=TARGET, selection=target_selection,
            turn=composed(TAIL_INPUT))).snapshot.follow_up.queue[-1].id
        ctx.passed("queued_input_waits_for_preparation")

        wait_for(lambda: summaries() == 1, "first summary request was not observed")
        cancel_started = time.monotonic()
        app.cancel_handoff_summary(started.work_id)
        work_reaches(started.work_id, "CANCELLED")
        wait_for(lambda: "summary_client_disconnected" in provider.events,
                 "cancelled summary kept its model connection")
        seconds = time.monotonic() - cancel_started
        ctx.facts["summary_cancel_seconds"] = round(seconds, 2)
        assert seconds < 3, f"summary cancel took {seconds:.2f}s to stop its model transport"
        wait_for(lambda: queue_is([head, tail], paused=True), "cancel consumed or failed to pause the queue")
        assert not targets(), "summary cancel dispatched a target request"
        ctx.passed("summary_cancel_stops_its_model_and_keeps_the_queue")

        # An empty but successful answer fails preparation, with no hidden
        # model retry and no target.
        requirements(cid)
        (gates / "fail-summary").touch()
        failed = app.retry_queued_handoff_summary(SOURCE, cid, head, summary_model, None)
        assert failed.queued_message_id == head, "retry added a duplicate queue item"
        failed_status = work_reaches(failed.work_id, "FAILED")
        assert failed_status.code == "summary_requires_text_only_response", (failed_status.code, failed_status.detail)
        snapshot = wait_for(lambda: queue_is([head, tail], paused=True), "failed preparation lost or unpaused the queue")
        event = maintenance(snapshot, failed.work_id)
        diagnosis = next(block.text for block in event.blocks if variant(block, "CONTEXT_COMPACTION"))
        assert event.completed and all(value in diagnosis for value in
                                       (failed_status.detail, failed_status.code, failed.work_id)), \
            "failed preparation lost its diagnosis"
        assert summaries() == 2, "semantic summary failure made an extra model call"
        (gates / "fail-summary").unlink()
        ctx.passed("failed_summary_keeps_the_queue_and_its_diagnosis")

        requirements(cid)
        final = app.retry_queued_handoff_summary(SOURCE, cid, head, summary_model, None)
        assert final.queued_message_id == head, "second retry added a duplicate item"
        wait_for(lambda: summaries() == 3, "explicit retry did not make exactly one summary call")
        app.send_queue(SOURCE, cid, api.SendQueueOperation.EDIT(id=head, text=TARGET_INPUT + " temporary edit"))
        app.send_queue(SOURCE, cid, api.SendQueueOperation.EDIT(id=head, text=TARGET_INPUT))
        retained = wait_for(lambda: queue_is([head, tail]), "editing during preparation changed queue identity")
        assert retained.follow_up.context_preparation.work_id == final.work_id and summaries() == 3, \
            "a text-only edit restarted the paid preparation"
        ctx.passed("text_edit_keeps_the_running_preparation")

        (gates / "release-summary").touch()
        wait_for(lambda: len(targets()) == 1, "ready preparation did not dispatch the queue head")
        assert TAIL_INPUT not in json.dumps(targets()[0]["body"]), "the tail crossed the preparation gate"
        ctx.passed("ready_preparation_dispatches_only_the_queue_head")

        # The target accepted the head but its answer is held: cancelling the
        # preparation must stop the native target and keep the tail.
        cancel_started = time.monotonic()
        accepted = app.cancel_handoff_summary(final.work_id)
        ctx.facts["accepted_cancel_seconds"] = round(time.monotonic() - cancel_started, 2)
        assert variant(accepted, "CANCELLED"), f"accepted target cancel was not confirmed: {accepted}"
        wait_for(lambda: queue_is([tail], paused=True), "accepted cancel lost or did not pause the tail")
        wait_for(lambda: (lambda s: s and s.run_state.name not in ("RUNNING", "STOPPING"))(current()),
                 "accepted target did not stop")
        assert len(targets()) == 1, "cancel replayed the head or dispatched the tail"
        ctx.passed("accepted_target_cancel_keeps_the_tail")

        (gates / "release-target").touch()
        app.send_queue(TARGET, cid, api.SendQueueOperation.RESUME())
        # read_conversation is the durable path; snapshot() may be None after
        # a completed handoff.
        done = wait_for(lambda: (lambda s: s if s is not None and s.context_runtime_id == TARGET
                                 and s.run_state.name == "IDLE" and TARGET_REPLY in texts(s) else None)(current()),
                        "target Pi conversation did not complete")
        sent = targets()
        assert len(sent) == 2, f"expected the head and its tail exactly once, got {len(sent)} target requests"
        first = json.dumps(sent[0]["body"].get("messages", []), ensure_ascii=False)
        assert TAIL_INPUT not in first, "the tail overtook the queue head"
        assert json.dumps(sent[1]["body"].get("messages", [])).count(TAIL_INPUT) == 1, "the tail was lost or duplicated"
        assert first.count(SUMMARY_TEXT) == 1 and first.count(TARGET_INPUT) == 1, \
            "target did not receive the summary and the input exactly once"
        assert OPAQUE not in first, "target request carried the encrypted compaction"
        for request in provider.calls("responses", seed=False):
            body = request["body"]
            assert body.get("model") == SUMMARY_MODEL and body.get("service_tier") is None, \
                f"summary used {body.get('model')!r}/{body.get('service_tier')!r}"
        assert done.conversation.id == cid, f"logical identity changed: {done.conversation.id}"
        committed = maintenance(done, final.work_id)
        assert committed.completed, "committed handoff maintenance event remained running"
        assert source_path.read_bytes() == source_bytes, "handoff changed the source Codex rollout"
        ctx.passed("handoff_delivers_summary_and_input_once_to_pi")

        close_idle()
        app = open_app()
        restored = maintenance(app.read_conversation(SOURCE, cid).snapshot, final.work_id)
        assert restored.completed and restored.id == committed.id, "maintenance identity changed after reopen"
        assert source_path.read_bytes() == source_bytes, "cold reopen changed the source"
        ctx.passed("maintenance_event_survives_a_cold_reopen")

        # Cancel while the real Pi SDK process is still starting: a node
        # wrapper holds pi_rpc.mjs before it connects.
        entered, release = gates / "target-preparation-entered", gates / "release-target-preparation"
        cid, source_path, source_bytes = seed_source()
        wrapper = ctx.isolated.root / "gated-node"
        wrapper.write_text(
            f"#!{sys.executable}\nimport os, pathlib, sys, time\n"
            f"entered, release = pathlib.Path({str(entered)!r}), pathlib.Path({str(release)!r})\n"
            "if len(sys.argv) > 1 and pathlib.Path(sys.argv[1]).name == 'pi_rpc.mjs':\n"
            "    entered.write_text(str(os.getpid()))\n"
            "    deadline = time.monotonic() + 30\n"
            "    while not release.exists():\n"
            "        if time.monotonic() >= deadline: sys.exit(2)\n"
            "        time.sleep(.01)\n"
            f"os.execv({str(node)!r}, [{str(node)!r}, *sys.argv[1:]])\n")
        wrapper.chmod(0o700)
        try:
            app = open_app(pi_node=wrapper)
            app.read_conversation(SOURCE, cid)
            input_ready(cid)
            requirements(cid)
            sent_before = len(targets())
            started = app.queue_handoff_summary(bindings.BindingQueuedHandoffSummary(
                source_runtime_id=SOURCE, conversation_id=cid, target_runtime_id=TARGET,
                selection=target_selection, turn=composed("SYNTHETIC CANCEL BEFORE TARGET ACCEPT"),
                summary_model=None, effort=None))
            wait_for(entered.exists, "the Pi SDK start did not reach its gate")
            outcome = []
            cancel_call_started = threading.Event()

            def cancel():
                try:
                    cancel_call_started.set()
                    outcome.append(app.cancel_handoff_summary(started.work_id))
                except BaseException as error:  # Reported below.
                    outcome.append(error)

            canceller = threading.Thread(target=cancel)
            canceller.start()
            # Start the caller while the SDK is held. Its receipt, the paused queue
            # and the model wire below distinguish cancellation from a delivered prompt.
            assert cancel_call_started.wait(timeout=5), "cancellation caller did not start"
            release.touch()
            canceller.join(timeout=30)
            assert not canceller.is_alive(), "cancel did not finish after the SDK start resumed"
            assert len(outcome) == 1 and variant(outcome[0], "CANCELLED"), f"cancel was not confirmed: {outcome}"
            wait_for(lambda: queue_is([started.queued_message_id], paused=True),
                     "cancel before accept did not keep a paused queue head", 10.0)
            assert len(targets()) == sent_before, "a target prompt was sent after cancel during SDK start"
            assert source_path.read_bytes() == source_bytes, "cancel before accept changed the source"
        finally:
            release.touch()
        ctx.passed("cancel_before_target_accept_sends_nothing")
        ctx.facts["provider_requests"] = [
            {"protocol": entry["protocol"], "model": entry["body"].get("model"),
             "service_tier": entry["body"].get("service_tier")} for entry in provider.bodies]
    finally:
        if app is not None:
            try:
                app.shutdown_stopping_work()
            except Exception:  # Already closed by close_idle().
                pass
        provider.close()
