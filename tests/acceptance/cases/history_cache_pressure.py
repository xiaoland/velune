"""History and delivery caches under memory pressure.

A read-only fake Codex app-server serves synthetic rollout files, so no
installed Agent or model is involved. Eight 10 MiB histories exceed the 64 MiB
byte budgets, nine tiny ones exceed the eight-view delivery cap, and two
40 MiB histories test observed panes. The fake server records each history read at the source.
"""
from __future__ import annotations

import json
import resource
import subprocess
import sys
import time
from pathlib import Path
from uuid import uuid4

from velune_acceptance import Context

REQUIRES = ()
RUNTIME = "memory-codex"
MiB = 1024 * 1024
BUDGET = 64 * MiB

FAKE_CODEX = r'''
import json, pathlib, sys
threads = json.loads((HOME / "synthetic-threads.json").read_text())
if "--version" in sys.argv:
    print("codex-cli 0.166.0")
    raise SystemExit(0)
def thread(native_id):
    row = threads[native_id]
    return {"id":native_id,"name":"Synthetic history","preview":"Synthetic pressure fixture",
            "cwd":row["cwd"],"source":"appServer","historyMode":"paginated",
            "createdAt":1,"updatedAt":2,"path":row["path"],"status":{"type":"idle"},
            "modelProvider":"synthetic"}
def history(native_id):
    with (HOME / "history-reads").open("a") as trace:
        print(native_id, file=trace)
    result = []
    for line in pathlib.Path(threads[native_id]["path"]).read_text().splitlines():
        row = json.loads(line)
        item = row.get("payload", {})
        if row.get("type") != "response_item" or item.get("type") != "message":
            continue
        text = item.get("content", [{}])[0].get("text", "")
        if item.get("role") == "user":
            projected = {"id":item["id"],"type":"userMessage","content":[{"type":"text","text":text}]}
        elif item.get("role") == "assistant":
            projected = {"id":item["id"],"type":"agentMessage","text":text}
        else:
            continue
        result.append({"turnId":"turn-0","item":projected,"startedAtMs":1794484800000})
    return result
for line in sys.stdin:
    request = json.loads(line)
    if "id" not in request:
        continue
    method, native_id = request["method"], request.get("params", {}).get("threadId", "")
    if method == "initialize":
        result = {}
    elif method == "thread/read":
        result = {"thread":thread(native_id)}
    elif method == "thread/items/list":
        result = {"data":history(native_id),"nextCursor":None}
    elif method == "thread/turns/list":
        result = {"data":[{"id":"turn-0","status":"completed","startedAt":1,"completedAt":2}],"nextCursor":None}
    else:
        print(json.dumps({"jsonrpc":"2.0","id":request["id"],"error":{"code":-32601,"message":"read-only fixture"}}),flush=True)
        continue
    print(json.dumps({"jsonrpc":"2.0","id":request["id"],"result":result}),flush=True)
'''


def write_histories(agent: Path, project: Path, count: int, message_mib: int) -> list[tuple[str, str, Path]]:
    """Write `count` one-turn Codex rollouts and register them with the fake server."""
    sessions = agent / "sessions/2026/11/12"
    sessions.mkdir(parents=True, exist_ok=True)
    stamp = "2026-11-12T12:00:00.000Z"
    body = "x" * (message_mib * MiB)
    rows = []
    for index in range(count):
        native = str(uuid4())
        path = sessions / f"rollout-2026-11-12T12-00-00-{native}.jsonl"
        records = [
            {"type": "session_meta", "payload": {
                "id": native, "session_id": native, "timestamp": stamp, "cwd": str(project),
                "runtime_workspace_roots": [str(project)], "originator": "velune", "cli_version": "0.166.0",
                "source": "vscode", "model_provider": "synthetic", "history_mode": "paginated"}},
            {"type": "event_msg", "payload": {"type": "task_started", "turn_id": f"turn-{index}",
                                              "started_at": 1794484800}},
            {"type": "response_item", "payload": {"type": "message", "id": f"user-{native}", "role": "user",
                "content": [{"type": "input_text", "text": f"SYNTHETIC-{index}-" + body}]}},
            {"type": "response_item", "payload": {"type": "message", "id": f"assistant-{native}",
                "role": "assistant", "content": [{"type": "output_text", "text": f"SYNTHETIC-REPLY-{index}"}]}},
            {"type": "event_msg", "payload": {"type": "task_complete", "turn_id": f"turn-{index}",
                                              "completed_at": 1794484801}},
        ]
        path.write_text("".join(json.dumps({"timestamp": stamp, "ordinal": ordinal, **row},
                                           separators=(",", ":")) + "\n" for ordinal, row in enumerate(records)))
        rows.append((native, f"{RUNTIME}:{native}", path))
    registry = agent / "synthetic-threads.json"
    threads = json.loads(registry.read_text()) if registry.exists() else {}
    threads.update({native: {"path": str(path), "cwd": str(project)} for native, _, path in rows})
    registry.write_text(json.dumps(threads))
    return rows


def access(snapshot) -> str:
    value = snapshot.actions.input_access
    name = getattr(value, "name", None) or type(value).__name__
    return str(name or value).split(".")[-1].replace("_", "").lower()


def run(ctx: Context) -> None:
    b = ctx.bindings
    api, conversation = b.application_api, b.conversation
    state, project = ctx.isolated.state, ctx.isolated.project
    agent = ctx.isolated.dir("codex")
    fake = ctx.isolated.root / "synthetic-codex"
    fake.write_text(f"#!{sys.executable}\nimport pathlib\nHOME = pathlib.Path({str(agent)!r})\n" + FAKE_CODEX)
    fake.chmod(0o700)

    byte_rows = write_histories(agent, project, 8, 10)
    pane_rows = write_histories(agent, project, 2, 40)
    count_rows = write_histories(agent, project, 9, 0)
    app = b.bindings.VeluneApplication.open(b.bindings.BindingOptions(
        home_directory=str(state), resources_directory=str(ctx.isolated.resources)))
    lock_holder = None

    def source_reads():
        trace = agent / "history-reads"
        return trace.read_text().splitlines() if trace.exists() else []

    def wait_snapshot(read, label):
        result, deadline = read(), time.monotonic() + 2 * ctx.timeout
        while result.snapshot is None and time.monotonic() < deadline:
            time.sleep(0.05)
            try:
                result = app.snapshot(RUNTIME, label)
            except Exception:  # The history may still be loading.
                pass
        assert result.snapshot is not None, f"no snapshot for {label}"
        return result.snapshot

    def open_ready(conversation_id):
        return wait_snapshot(lambda: app.open_conversation(RUNTIME, conversation_id), conversation_id)

    def cold_ready(conversation_id):
        return wait_snapshot(lambda: app.snapshot(RUNTIME, conversation_id), conversation_id)

    def poll_access(conversation_id, snapshot, wanted, timeout):
        deadline, current, seen = time.monotonic() + timeout, snapshot, []
        while time.monotonic() < deadline:
            seen.append(access(current))
            if seen[-1] == wanted:
                return current, seen
            update = app.snapshot_update(RUNTIME, conversation_id, current.cursor)
            assert not update.is_ABSENT(), "conversation became absent while polling control"
            if update.is_CHANGED():
                current = update.snapshot
            time.sleep(0.05)
        raise AssertionError(f"control did not reach {wanted}; observed={seen[-12:]}")

    def poll(conversation_id, cursor):
        update = app.snapshot_update(RUNTIME, conversation_id, cursor)
        assert not update.is_ABSENT(), "observed conversation became absent"
        return update

    def observe(conversation_ids):
        app.set_observed_conversations([api.ObservedConversation(runtime_id=RUNTIME, conversation_id=cid)
                                        for cid in conversation_ids])

    def payload_kept(conversation_id, native):
        return bool(app.message_payloads(RUNTIME, conversation_id, [f"user-{native}"]))

    try:
        app.upsert_runtime_with_execution(
            api.RuntimeInstance(id=RUNTIME, name="Synthetic memory pressure", enabled=True,
                type_id=conversation.RuntimeKind.CODEX166, gateway_id="default",
                settings={"binary": str(fake), "agentDir": str(agent), "nodeBinary": "/usr/bin/false"}),
            api.RuntimeExecution(transport=api.RuntimeTransport.LOCAL(),
                home=api.RuntimeHome.EXISTING(path=str(ctx.isolated.home)), remote_resources_directory=None))

        # Eight views fit the count cap. Only the byte budget can evict a payload.
        first_native, first_id, _ = byte_rows[0]
        first = open_ready(first_id)
        assert access(first) == "checkingcontrol", f"first watch did not start pending: {access(first)}"
        for native, conversation_id, _ in byte_rows:
            snapshot = first if conversation_id == first_id else open_ready(conversation_id)
            assert f"user-{native}" in [m.id for m in snapshot.messages], "large message missing"
        assert not payload_kept(first_id, first_native), "byte budget kept the oldest payload"
        assert payload_kept(byte_rows[-1][1], byte_rows[-1][0]), "byte budget dropped the newest payload"
        ctx.passed("byte_budget_evicts_the_oldest_payload")

        before = source_reads()
        cold = cold_ready(first_id)
        assert source_reads() == [*before, first_native], "evicted history was served without a source reload"
        assert f"user-{first_native}" in [m.id for m in cold.messages], "cold reload lost the message"
        _, resumed = poll_access(first_id, cold, "editable", 3.0)
        ctx.facts["control_after_cold_reload"] = resumed
        ctx.passed("evicted_history_reloads_and_regains_control")

        # Refresh discards the retired Available observation: a held writer
        # lock must now surface as an external controller.
        lock = agent / "thread-writer-locks" / f"{first_native}.lock"
        lock.parent.mkdir(parents=True, exist_ok=True)
        lock_holder = subprocess.Popen(
            [sys.executable, "-c", "import fcntl,sys,time; f=open(sys.argv[1],'a+'); "
             "fcntl.flock(f,fcntl.LOCK_EX); print('LOCKED',flush=True); time.sleep(60)", str(lock)],
            stdout=subprocess.PIPE, text=True)
        assert lock_holder.stdout.readline().strip() == "LOCKED", "writer lock helper failed"
        for _, conversation_id, _ in byte_rows[1:]:
            open_ready(conversation_id)  # Evict the watched history again.
        refreshed = open_ready(first_id)
        assert access(refreshed) in {"checkingcontrol", "externalcontroller"}, \
            "refresh reused the retired editable observation"
        poll_access(first_id, refreshed, "externalcontroller", 10.0)
        lock_holder.terminate()
        lock_holder.wait(timeout=5)
        lock_holder = None
        ctx.passed("refresh_drops_a_retired_control_observation")

        # Two observed 40 MiB panes exceed the parked budget yet stay hot.
        (_, a_id, _), (_, b_id, _) = pane_rows
        assert sum(path.stat().st_size for *_, path in pane_rows) > BUDGET
        observe([a_id, b_id])
        cursors = {cid: poll_access(cid, open_ready(cid), "editable", 10.0)[0].cursor for cid in (a_id, b_id)}
        before = source_reads()
        for _ in range(6):
            for cid in (a_id, b_id):
                assert poll(cid, cursors[cid]).is_UNCHANGED(), "observed pane cursor was rebuilt"
        assert source_reads() == before, "observed panes re-read history while polling"
        ctx.passed("observed_panes_stay_hot_over_budget")

        # Dropping a pane makes it reclaimable; showing it again reloads it
        # exactly once while its sibling stays hot.
        observe([a_id])
        assert poll(a_id, cursors[a_id]).is_UNCHANGED(), "remaining pane lost its cursor"
        before = source_reads()
        observe([a_id, b_id])
        assert poll(a_id, cursors[a_id]).is_UNCHANGED(), "re-observing rebuilt the retained pane"
        assert source_reads() == before, "re-observing loaded history before it was read"
        update = poll(b_id, cursors[b_id])
        assert source_reads() == [*before, pane_rows[1][0]], "re-displayed pane did not reload exactly once"
        settled = update.snapshot.cursor if update.is_CHANGED() else cursors[b_id]
        assert poll(b_id, settled).is_UNCHANGED(), "re-displayed pane did not settle"
        assert poll(a_id, cursors[a_id]).is_UNCHANGED(), "sibling reload evicted the observed pane"
        assert source_reads() == [*before, pane_rows[1][0]], "polling repeated the reload"
        observe([])
        ctx.passed("redisplayed_pane_reloads_once")

        # Count cap: nine tiny views keep only the newest eight payloads.
        for native, conversation_id, _ in count_rows:
            assert f"user-{native}" in [m.id for m in open_ready(conversation_id).messages], "small message missing"
        retained = [native for native, cid, _ in count_rows if payload_kept(cid, native)]
        assert retained == [native for native, _, _ in count_rows[1:]], f"wrong set of retained views: {retained}"
        ctx.passed("count_cap_keeps_the_newest_eight_views")

        stable = []
        for _, conversation_id, _ in byte_rows[-6:]:
            snapshot, _ = poll_access(conversation_id, open_ready(conversation_id), "editable", 10.0)
            stable.append((conversation_id, snapshot.cursor))
        before, usage, started = source_reads(), resource.getrusage(resource.RUSAGE_SELF), time.monotonic()
        for _ in range(6):
            for conversation_id, cursor in stable:
                assert app.snapshot_update(RUNTIME, conversation_id, cursor).is_UNCHANGED(), "stable poll changed"
                time.sleep(0.02)
        after = resource.getrusage(resource.RUSAGE_SELF)
        assert source_reads() == before, "unchanged polls re-read native history"
        # CPU is evidence only; no machine-independent limit is claimed.
        ctx.facts["unchanged_poll"] = {
            "updates": 6 * len(stable), "wall_seconds": round(time.monotonic() - started, 3),
            "cpu_seconds": round(after.ru_utime + after.ru_stime - usage.ru_utime - usage.ru_stime, 3)}
        ctx.passed("unchanged_polls_read_no_history")
    finally:
        if lock_holder is not None:
            lock_holder.terminate()
            lock_holder.wait(timeout=5)
        app.shutdown_stopping_work()
