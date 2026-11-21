#!/usr/bin/env python3
"""Run a synthetic direct-Bindings pressure check for history/delivery caches.

Python UniFFI bindings come from the existing Mac development App dylib unless
an exact matching generated binding directory is supplied. This intentionally
does not build, launch a GUI, inspect daily state, or print transcript bodies.
"""
from __future__ import annotations

import argparse
import atexit
import fcntl
import hashlib
import importlib
import json
import os
from pathlib import Path
import resource
import shutil
import subprocess
import sys
import tempfile
import time
import types
from uuid import uuid4

REPO = Path(__file__).resolve().parent.parent
RUNTIME_ID = "memory-codex"
MiB = 1024 * 1024


def safe_root(path: Path | None) -> Path:
    if path is None:
        root = Path(tempfile.mkdtemp(prefix="velune-memory-pressure-", dir="/private/tmp"))
        os.chmod(root, 0o700)
        return root
    root = path.expanduser().absolute()
    if (root != Path("/private/tmp") and root != Path("/tmp")
            and not str(root).startswith(("/private/tmp/", "/tmp/"))):
        raise SystemExit("synthetic root must be a dedicated directory under /private/tmp or /tmp")
    if path is not None and (root.exists() or root.is_symlink()):
        raise SystemExit("refusing to take over an existing synthetic fixture directory")
    root.mkdir(mode=0o700, parents=False, exist_ok=False)
    os.chmod(root, 0o700)
    return root


def output_directory() -> Path:
    base = REPO / "target/checks/macos-history-cache"
    if base.is_symlink():
        raise SystemExit("refusing a symlink at the Mac history-cache output path")
    base.mkdir(parents=True, exist_ok=True)
    stamp = time.strftime("%Y%m%d-%H%M%S")
    for attempt in range(100):
        path = base / f"run-{stamp}-{os.getpid()}-{attempt:02d}"
        try:
            path.mkdir(mode=0o700)
            return path
        except FileExistsError:
            continue
    raise SystemExit("could not allocate a unique history-cache result directory")


def matching_app_library() -> Path:
    identity = subprocess.run(
        [sys.executable, "scripts/macos_identity.py", "describe", "--purpose", "development",
         "--profile", "dev", "--root", str(REPO)],
        cwd=REPO, check=True, capture_output=True, text=True,
    )
    bundle = Path(json.loads(identity.stdout)["bundlePath"])
    library = bundle / "Contents/Frameworks/libvelune_bindings.dylib"
    if not library.is_file():
        raise SystemExit("the built Mac development App has no embedded bindings library; build it first")
    return library


def prepare_bindings(args, temporary_directory: Path) -> tuple[Path, str, Path | None]:
    if args.bindings_dir is not None:
        if not args.expected_sha256:
            raise SystemExit("--bindings-dir requires --expected-sha256")
        bindings = args.bindings_dir.expanduser().resolve(strict=True)
        dylib = bindings / "libvelune_bindings.dylib"
        if not dylib.is_file():
            raise SystemExit("--bindings-dir must contain libvelune_bindings.dylib")
        digest = hashlib.sha256(dylib.read_bytes()).hexdigest()
        if digest != args.expected_sha256:
            raise SystemExit(f"binding dylib digest mismatch: found {digest}")
        return bindings, digest, None

    library = matching_app_library()
    digest = hashlib.sha256(library.read_bytes()).hexdigest()
    if args.expected_sha256 and digest != args.expected_sha256:
        raise SystemExit(f"built App dylib digest mismatch: found {digest}")
    generator = REPO / "target/build-tools/debug/velune-bindgen"
    if not generator.is_file():
        raise SystemExit("the fixed target/build-tools/debug/velune-bindgen tool is missing; build Mac development artifacts first")
    generated = REPO / "target/bindings/python"
    subprocess.run(
        [sys.executable, "scripts/build-support.py", "bindgen", "--library", str(library),
         "--language", "python", "--generator", str(generator), "--out-dir", str(generated)],
        cwd=REPO, check=True,
    )
    modules = sorted(generated.glob("*.py"))
    if not modules:
        raise SystemExit("build-support.py did not produce Python binding modules")
    bindings = temporary_directory / "python-bindings"
    bindings.mkdir(mode=0o700)
    for module in modules:
        shutil.copyfile(module, bindings / module.name)
    shutil.copyfile(library, bindings / "libvelune_bindings.dylib")
    return bindings, digest, library


def retain_metadata(root: Path, output: Path, result: dict) -> None:
    (output / "result.json").write_text(json.dumps(result, indent=2) + "\n")
    for name in ("before-load-ps.txt", "before-load-vmmap.txt", "after-load-ps.txt", "after-load-vmmap.txt",
                 "unchanged-poll.sample.txt"):
        source = root / name
        if source.is_file():
            shutil.copyfile(source, output / name)


def write_synthetic_codex(root: Path, count: int, message_mib: int) -> list[tuple[str, str, Path]]:
    agent = root / "codex-home"
    project = root / "project"
    sessions = agent / "sessions" / "2026" / "11" / "12"
    sessions.mkdir(parents=True, exist_ok=True)
    project.mkdir(parents=True, exist_ok=True)
    stamp = "2026-11-12T12:00:00.000Z"
    body = "x" * (message_mib * MiB)
    rows = []
    for index in range(count):
        native_id = str(uuid4())
        path = sessions / f"rollout-{stamp.replace(':', '-').replace('.000Z', '')}-{native_id}.jsonl"
        records = [
            {"timestamp": stamp, "ordinal": 0, "type": "session_meta", "payload": {
                "id": native_id, "session_id": native_id, "timestamp": stamp,
                "cwd": str(project), "runtime_workspace_roots": [str(project)],
                "originator": "velune", "cli_version": "0.166.0",
                "source": "vscode", "model_provider": "synthetic", "history_mode": "paginated",
            }},
            {"timestamp": stamp, "ordinal": 1, "type": "event_msg", "payload": {
                "type": "task_started", "turn_id": f"turn-{index}", "started_at": 1794484800,
            }},
            {"timestamp": stamp, "ordinal": 2, "type": "response_item", "payload": {
                "type": "message", "id": f"user-{native_id}", "role": "user",
                "content": [{"type": "input_text", "text": f"SYNTHETIC-{index}-" + body}],
            }},
            {"timestamp": stamp, "ordinal": 3, "type": "response_item", "payload": {
                "type": "message", "id": f"assistant-{native_id}", "role": "assistant",
                "content": [{"type": "output_text", "text": f"SYNTHETIC-REPLY-{index}"}],
            }},
            {"timestamp": stamp, "ordinal": 4, "type": "event_msg", "payload": {
                "type": "task_complete", "turn_id": f"turn-{index}", "completed_at": 1794484801,
            }},
        ]
        path.write_text("".join(json.dumps(row, separators=(",", ":")) + "\n" for row in records))
        rows.append((native_id, f"{RUNTIME_ID}:{native_id}", path))
    mapping_path = agent / "synthetic-threads.json"
    index = json.loads(mapping_path.read_text()) if mapping_path.exists() else {}
    index.update({native_id: {"path": str(path), "cwd": str(project)} for native_id, _, path in rows})
    mapping_path.write_text(json.dumps(index))
    return rows


def history_read_count(state: Path) -> int:
    logs = state / "logs"
    if not logs.exists():
        return 0
    count = 0
    for path in logs.glob("*.jsonl"):
        for line in path.read_text(errors="replace").splitlines():
            try:
                fields = json.loads(line).get("fields", {})
            except (ValueError, TypeError):
                continue
            if fields.get("event") == "runtime_history_read" and fields.get("phase") == "native_read":
                count += 1
    return count


def access_name(snapshot) -> str:
    value = snapshot.actions.input_access
    name = getattr(value, "name", None) or type(value).__name__
    return str(name or value).split(".")[-1].replace("_", "").lower()


def install_fake_codex(binary: Path, agent: Path) -> None:
    script = r'''#!/usr/bin/python3
import json, pathlib, sys
home = pathlib.Path(__file__).parent / "codex-home"
threads = json.loads((home / "synthetic-threads.json").read_text())
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
    path = pathlib.Path(threads[native_id]["path"])
    rows = [json.loads(line) for line in path.read_text().splitlines()]
    result = []
    for row in rows:
        item = row.get("payload", {})
        if row.get("type") != "response_item" or item.get("type") != "message":
            continue
        role = item.get("role")
        if role == "user":
            text = item.get("content", [{}])[0].get("text", "")
            projected = {"id":item["id"],"type":"userMessage","content":[{"type":"text","text":text}]}
        elif role == "assistant":
            text = item.get("content", [{}])[0].get("text", "")
            projected = {"id":item["id"],"type":"agentMessage","text":text}
        else:
            continue
        result.append({"turnId":"turn-0","item":projected,"startedAtMs":1794484800000})
    return result
for line in sys.stdin:
    request = json.loads(line)
    if "id" not in request:
        continue
    method = request["method"]
    params = request.get("params", {})
    native_id = params.get("threadId", "")
    result = {}
    if method == "initialize":
        result = {}
    elif method == "thread/read":
        result = {"thread":thread(native_id)}
    elif method == "thread/items/list":
        result = {"data":history(native_id),"nextCursor":None}
    elif method == "thread/turns/list":
        result = {"data":[{"id":"turn-0","status":"completed","startedAt":1,"completedAt":2}],"nextCursor":None}
    else:
        print(json.dumps({"jsonrpc":"2.0","id":request["id"],"error":{"code":-32601,"message":"synthetic history fixture is read-only"}}),flush=True)
        continue
    print(json.dumps({"jsonrpc":"2.0","id":request["id"],"result":result}),flush=True)
'''
    binary.write_text(script.replace('home = pathlib.Path(__file__).parent / "codex-home"',
                                      f'home = pathlib.Path({str(agent)!r})'))
    binary.chmod(0o700)


def poll_access(app, runtime_id: str, conversation_id: str, snapshot, wanted: str, timeout: float = 3.0):
    deadline = time.monotonic() + timeout
    cursor = snapshot.cursor
    current = snapshot
    observations = []
    while time.monotonic() < deadline:
        name = access_name(current)
        observations.append(name)
        if name == wanted:
            return current, observations
        update = app.snapshot_update(runtime_id, conversation_id, cursor)
        if update.is_CHANGED():
            current = update.snapshot
            cursor = current.cursor
        elif update.is_ABSENT():
            raise AssertionError("queried conversation became absent while polling control")
        time.sleep(0.05)
    raise AssertionError(f"control did not reach {wanted}; observed={observations[-12:]}")


def observe_conversations(app, api, conversation_ids: list[str]) -> None:
    app.set_observed_conversations([
        api.ObservedConversation(runtime_id=RUNTIME_ID, conversation_id=conversation_id)
        for conversation_id in conversation_ids
    ])


def unchanged_cursor(app, conversation_id: str, cursor):
    update = app.snapshot_update(RUNTIME_ID, conversation_id, cursor)
    if update.is_ABSENT():
        raise AssertionError("observed synthetic conversation became absent")
    return update


def observed_pane_pressure_check(app, api, state: Path, rows) -> dict:
    first_id, second_id = rows[0][1], rows[1][1]
    fixture_bytes = sum(path.stat().st_size for _, _, path in rows)
    if fixture_bytes <= 64 * MiB:
        raise AssertionError("two observed Codex fixtures do not exceed the parked-history byte budget")
    # Register the exact local view contents before opening them. Both 40 MiB
    # histories then remain eligible to exceed the parked-cache byte budget.
    observe_conversations(app, api, [first_id, second_id])
    first = poll_access(app, RUNTIME_ID, first_id, open_ready(app, RUNTIME_ID, first_id),
                        "editable", timeout=10.0)[0]
    second = poll_access(app, RUNTIME_ID, second_id, open_ready(app, RUNTIME_ID, second_id),
                         "editable", timeout=10.0)[0]
    cursors = {first_id: first.cursor, second_id: second.cursor}

    # Alternate the two visible panes across multiple Core owner/trim cycles.
    # Their delivered cursors must stay stable while both are explicitly observed.
    before_reads = history_read_count(state)
    unchanged_polls = 0
    for _ in range(6):
        for conversation_id in (first_id, second_id):
            update = unchanged_cursor(app, conversation_id, cursors[conversation_id])
            if not update.is_UNCHANGED():
                raise AssertionError("an observed pane's Delivery cursor was rebuilt during polling")
            unchanged_polls += 1
    after_observed_reads = history_read_count(state)
    if after_observed_reads != before_reads:
        raise AssertionError("observed panes caused native history reads while polling")

    # Removing one pane from the complete set makes its transcript reclaimable.
    # A later re-display should cold-load that history once, while the still
    # observed pane remains hot and keeps its Delivery cursor.
    observe_conversations(app, api, [first_id])
    retained = unchanged_cursor(app, first_id, cursors[first_id])
    if not retained.is_UNCHANGED():
        raise AssertionError("the pane left in the observation set lost its stable cursor")
    reads_before_reappear = history_read_count(state)
    observe_conversations(app, api, [first_id, second_id])
    reappeared = unchanged_cursor(app, first_id, cursors[first_id])
    if not reappeared.is_UNCHANGED():
        raise AssertionError("re-observing the retained pane rebuilt its cursor")
    if history_read_count(state) != reads_before_reappear:
        raise AssertionError("restoring the observation set loaded history before the evicted pane was read")

    second_update = unchanged_cursor(app, second_id, cursors[second_id])
    reads_after_reappear = history_read_count(state)
    if reads_after_reappear != reads_before_reappear + 1:
        raise AssertionError("re-displaying one evicted pane did not cause exactly one native history read")
    if second_update.is_ABSENT():
        raise AssertionError("re-displayed pane remained absent after its native history reload")
    restored_cursor = second_update.snapshot.cursor if second_update.is_CHANGED() else cursors[second_id]
    if not unchanged_cursor(app, second_id, restored_cursor).is_UNCHANGED():
        raise AssertionError("re-displayed pane did not settle to an unchanged Delivery cursor")
    if not unchanged_cursor(app, first_id, cursors[first_id]).is_UNCHANGED():
        raise AssertionError("the continuously observed pane was evicted during its sibling reload")
    if history_read_count(state) != reads_after_reappear:
        raise AssertionError("polling the re-displayed pane repeated its one necessary reload")

    observe_conversations(app, api, [])

    return {
        "panes": 2,
        "message_mib_each": 40,
        "fixture_jsonl_bytes": fixture_bytes,
        "observed_polls": unchanged_polls,
        "source_read_events_before_after_observed_polls": [before_reads, after_observed_reads],
        "retained_pane_unchanged_after_sibling_removal": True,
        "reappeared_pane_native_reads": reads_after_reappear - reads_before_reappear,
        "reappeared_pane_delivery_state": "changed" if second_update.is_CHANGED() else "unchanged",
        "reappeared_pane_settled_unchanged": True,
    }


def open_ready(app, runtime_id: str, conversation_id: str, timeout: float = 30.0):
    result = app.open_conversation(runtime_id, conversation_id)
    deadline = time.monotonic() + timeout
    while result.snapshot is None and time.monotonic() < deadline:
        try:
            result = app.snapshot(runtime_id, conversation_id)
        except Exception:
            pass
        time.sleep(0.05)
    if result.snapshot is None:
        raise AssertionError(f"open did not yield a snapshot for synthetic conversation {conversation_id}")
    return result.snapshot


def cold_read_ready(app, runtime_id: str, conversation_id: str, timeout: float = 30.0):
    result = app.snapshot(runtime_id, conversation_id)
    deadline = time.monotonic() + timeout
    while result.snapshot is None and time.monotonic() < deadline:
        try:
            result = app.snapshot(runtime_id, conversation_id)
        except Exception:
            pass
        time.sleep(0.05)
    if result.snapshot is None:
        raise AssertionError(f"cold snapshot did not reload {conversation_id}")
    return result.snapshot


def spawn_lock_holder(path: Path):
    code = "import fcntl,sys,time; f=open(sys.argv[1],'a+'); fcntl.flock(f,fcntl.LOCK_EX); print('LOCKED',flush=True); time.sleep(60)"
    process = subprocess.Popen([sys.executable, "-c", code, str(path)], stdout=subprocess.PIPE, text=True)
    if process.stdout.readline().strip() != "LOCKED":
        process.terminate()
        raise RuntimeError("synthetic Codex writer lock helper failed")
    return process


def capture_metadata(root: Path, label: str, pid: int) -> None:
    (root / f"{label}-ps.txt").write_text(
        subprocess.run(["/bin/ps", "-o", "pid=,rss=,%cpu=", "-p", str(pid)],
                       check=True, capture_output=True, text=True).stdout
    )
    result = subprocess.run(["/usr/bin/vmmap", "-summary", str(pid)], capture_output=True, text=True)
    (root / f"{label}-vmmap.txt").write_text(result.stdout + result.stderr)


def main() -> None:
    parser = argparse.ArgumentParser()
    parser.add_argument("--bindings-dir", type=Path,
                        help="advanced: matching generated Python modules and dylib")
    parser.add_argument("--expected-sha256",
                        help="advanced: require this exact embedded bindings library digest")
    parser.add_argument("--root", type=Path, help="fresh, non-existing private fixture path under /private/tmp or /tmp")
    parser.add_argument("--keep-fixture", action="store_true", help="keep the synthetic history files for debugging")
    parser.add_argument("--message-mib", type=int, default=10)
    parser.add_argument("--byte-conversations", type=int, default=9)
    parser.add_argument("--count-conversations", type=int, default=9)
    parser.add_argument("--poll-rounds", type=int, default=6)
    parser.add_argument("--sample-seconds", type=int, default=2,
                        help="sample this process during unchanged polling; use 0 to skip")
    args = parser.parse_args()
    if sys.platform != "darwin":
        parser.error("this pressure check requires macOS vmmap and sample tools")
    if args.message_mib < 1 or args.byte_conversations < 9 or args.count_conversations < 9 or args.poll_rounds < 1:
        parser.error("the workload requires at least nine histories in each phase, positive message size and poll rounds")
    if args.message_mib * args.byte_conversations <= 64:
        parser.error("the large-history phase must exceed the 64 MiB cache budget")
    if args.sample_seconds < 0:
        parser.error("--sample-seconds must be zero or greater")
    binding_workspace = Path(tempfile.mkdtemp(prefix="velune-history-bindings-", dir="/private/tmp"))
    atexit.register(shutil.rmtree, binding_workspace, ignore_errors=True)
    bindings, digest, app_library = prepare_bindings(args, binding_workspace)
    package_name = "_velune_pressure_bindings"
    package = types.ModuleType(package_name)
    package.__path__ = [str(bindings)]
    sys.modules[package_name] = package
    b = importlib.import_module(f"{package_name}.velune_bindings")
    a = importlib.import_module(f"{package_name}.velune_application_api")
    c = importlib.import_module(f"{package_name}.velune_conversation")
    if not hasattr(c.RuntimeKind, "CODEX166"):
        raise SystemExit("bindings do not expose the Codex 0.166 runtime used by this fixture")

    root = safe_root(args.root)
    if not args.keep_fixture:
        atexit.register(shutil.rmtree, root, ignore_errors=True)
    state, os_home, resources = root / "state", root / "os-home", root / "resources"
    agent = root / "codex-home"
    for path in (state, os_home, resources, agent):
        path.mkdir(parents=True, exist_ok=True)
    # A fake read-only Codex app-server serves fixtures read from the synthetic
    # native rollout files; it never invokes an installed Agent or model.
    binary = root / "synthetic-codex"
    install_fake_codex(binary, agent)
    os.environ["HOME"] = str(os_home)
    os.environ["CODEX_HOME"] = str(agent)
    os.environ["DISABLE_TELEMETRY"] = "1"
    os.environ["DISABLE_ERROR_REPORTING"] = "1"
    os.environ["HTTP_PROXY"] = "http://127.0.0.1:1"
    os.environ["HTTPS_PROXY"] = "http://127.0.0.1:1"
    os.environ["ALL_PROXY"] = "http://127.0.0.1:1"
    os.environ["NO_PROXY"] = "127.0.0.1,localhost"

    byte_rows = write_synthetic_codex(root, args.byte_conversations, args.message_mib)
    observed_rows = write_synthetic_codex(root, 2, 40)
    count_rows = write_synthetic_codex(root, args.count_conversations, 0)
    options = b.BindingOptions(home_directory=str(state), resources_directory=str(resources))
    app = None
    lock_holder = None
    try:
        app = b.VeluneApplication.open(options)
        execution = a.RuntimeExecution(
            transport=a.RuntimeTransport.LOCAL(),
            home=a.RuntimeHome.EXISTING(path=str(os_home)),
            remote_resources_directory=None,
        )
        runtime = a.RuntimeInstance(
            id=RUNTIME_ID, name="Synthetic memory pressure", enabled=True,
            type_id=c.RuntimeKind.CODEX166, gateway_id="default",
            settings={"binary": str(binary), "agentDir": str(agent), "nodeBinary": "/usr/bin/false"},
        )
        app.upsert_runtime_with_execution(runtime, execution)

        capture_metadata(root, "before-load", os.getpid())
        first_native, first_id, _ = byte_rows[0]
        first = open_ready(app, RUNTIME_ID, first_id)
        if access_name(first) != "checkingcontrol":
            raise AssertionError(f"first Codex watch did not start pending: {access_name(first)}")
        byte_payload_ids = {}
        for native_id, conversation_id, _ in byte_rows:
            snapshot = first if conversation_id == first_id else open_ready(app, RUNTIME_ID, conversation_id)
            message_id = f"user-{native_id}"
            if message_id not in [message.id for message in snapshot.messages]:
                raise AssertionError("large synthetic user message missing from delivered snapshot")
            byte_payload_ids[conversation_id] = message_id

        # Nine 10 MiB conversations exceed both 64 MiB byte budgets. Delivery
        # payload reads expose eviction without re-reading source history.
        oldest_payload = app.message_payloads(RUNTIME_ID, first_id, [byte_payload_ids[first_id]])
        newest_id = byte_rows[-1][1]
        newest_payload = app.message_payloads(RUNTIME_ID, newest_id, [byte_payload_ids[newest_id]])
        if oldest_payload or len(newest_payload) != 1:
            raise AssertionError("delivery byte budget did not evict the oldest complete message payload")

        before_reads = history_read_count(state)
        cold = cold_read_ready(app, RUNTIME_ID, first_id)
        after_reads = history_read_count(state)
        if after_reads <= before_reads:
            raise AssertionError("oldest history remained available without a source reload after byte pressure")
        if f"user-{first_native}" not in [message.id for message in cold.messages]:
            raise AssertionError("cold reload lost the complete synthetic history message")
        cold, control_states = poll_access(app, RUNTIME_ID, first_id, cold, "editable")
        if not control_states or control_states[-1] != "editable":
            raise AssertionError("evicted ControlWatch did not converge after cold reload")

        # Explicit refresh must discard the retired Available observation. A
        # local synthetic flock makes the next Target.observe report HeldElsewhere.
        lock_path = agent / "thread-writer-locks" / f"{first_native}.lock"
        lock_path.parent.mkdir(parents=True, exist_ok=True)
        lock_holder = spawn_lock_holder(lock_path)
        # Evict the watched history again before refresh so its observer enters
        # the independent retired-watch cache.
        for _, conversation_id, _ in byte_rows[1:]:
            open_ready(app, RUNTIME_ID, conversation_id)
        refreshed = open_ready(app, RUNTIME_ID, first_id)
        if access_name(refreshed) not in {"checkingcontrol", "externalcontroller"}:
            raise AssertionError("explicit refresh reused the retired editable observation")
        refreshed, refresh_states = poll_access(app, RUNTIME_ID, first_id, refreshed,
                                                "externalcontroller", timeout=10.0)
        lock_holder.terminate()
        lock_holder.wait(timeout=5)
        lock_holder = None

        observed_panes = observed_pane_pressure_check(app, a, state, observed_rows)

        # Nine tiny views isolate the existing eight-view delivery cap from
        # its byte cap; the session byte/count cache is far below either limit.
        count_payload_ids = {}
        for native_id, conversation_id, _ in count_rows:
            snapshot = open_ready(app, RUNTIME_ID, conversation_id)
            message_id = f"user-{native_id}"
            if message_id not in [message.id for message in snapshot.messages]:
                raise AssertionError("small synthetic message missing")
            count_payload_ids[conversation_id] = message_id
        if app.message_payloads(RUNTIME_ID, count_rows[0][1], [count_payload_ids[count_rows[0][1]]]):
            raise AssertionError("delivery retained more than eight view payloads")
        if len(app.message_payloads(RUNTIME_ID, count_rows[-1][1], [count_payload_ids[count_rows[-1][1]]])) != 1:
            raise AssertionError("delivery count cap discarded the newest payload")

        # Stable unchanged polling uses the actual Application/Bindings path.
        # A growing source-read count would reveal cold-read churn; CPU is
        # reported as evidence, with no machine-independent limit claimed.
        stable_rows = byte_rows[-6:]
        stable = []
        for _, conversation_id, _ in stable_rows:
            snapshot = open_ready(app, RUNTIME_ID, conversation_id)
            snapshot, _ = poll_access(app, RUNTIME_ID, conversation_id, snapshot, "editable", timeout=10.0)
            stable.append((conversation_id, snapshot.cursor))
        reads_before_poll = history_read_count(state)
        usage_before = resource.getrusage(resource.RUSAGE_SELF)
        poll_start = time.monotonic()
        unchanged = 0
        sample_path = root / "unchanged-poll.sample.txt"
        sampler = None
        if args.sample_seconds:
            sampler = subprocess.Popen(["/usr/bin/sample", str(os.getpid()), str(args.sample_seconds), "-file", str(sample_path)],
                                       stdout=subprocess.DEVNULL, stderr=subprocess.DEVNULL)
        try:
            for _ in range(args.poll_rounds):
                for conversation_id, cursor in stable:
                    update = app.snapshot_update(RUNTIME_ID, conversation_id, cursor)
                    if not update.is_UNCHANGED():
                        raise AssertionError("stable synthetic snapshot poll was not unchanged")
                    unchanged += 1
                    time.sleep(0.02)
        finally:
            if sampler is not None and sampler.poll() is None:
                sampler.wait(timeout=args.sample_seconds + 4)
        elapsed = time.monotonic() - poll_start
        usage_after = resource.getrusage(resource.RUSAGE_SELF)
        reads_after_poll = history_read_count(state)
        if reads_after_poll != reads_before_poll:
            raise AssertionError("unchanged polls unexpectedly re-read native history")
        capture_metadata(root, "after-load", os.getpid())
        result = {
            "passed": True,
            "binding_sha256": digest,
            "binding_source": str(app_library) if app_library else "explicit --bindings-dir",
            "process_id": os.getpid(),
            "byte_pressure": {"conversations": len(byte_rows), "message_mib_each": args.message_mib,
                              "oldest_delivery_payload_evicted": True,
                              "cold_reload_read_events_before_after": [before_reads, after_reads],
                              "control_resumed": control_states},
            "explicit_refresh": {"synthetic_writer_lock_used": True, "observations": refresh_states},
            "observed_panes": observed_panes,
            "delivery_count_cap": {"views": len(count_rows), "retained_count": 8,
                                    "oldest_evicted": True, "newest_retained": True},
            "unchanged_poll_load": {"sessions": len(stable), "rounds": args.poll_rounds,
                                    "unchanged_updates": unchanged, "wall_seconds": elapsed,
                                    "process_cpu_seconds": ((usage_after.ru_utime + usage_after.ru_stime)
                                                             - (usage_before.ru_utime + usage_before.ru_stime)),
                                    "source_read_events_before_after": [reads_before_poll, reads_after_poll]},
            "note": "serialized-size budgets are estimates; this report records actual process metadata separately",
        }
        output = output_directory()
        result["result_directory"] = str(output)
        retain_metadata(root, output, result)
        print(json.dumps(result, indent=2))
    finally:
        if lock_holder is not None:
            lock_holder.terminate()
            lock_holder.wait(timeout=5)
        if app is not None:
            app.shutdown_stopping_work()


if __name__ == "__main__":
    try:
        main()
    except SystemExit:
        raise
    except Exception as error:
        print(json.dumps({"passed": False, "error_type": type(error).__name__}), file=sys.stderr)
        raise SystemExit(1) from None
