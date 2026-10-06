#!/usr/bin/env python3
"""Manual Codex 0.159.3 fork identity and huihua scan probe.

The probe uses a temporary CODEX_HOME and a loopback Responses server.  It
does not read the user's Codex home, credentials, or existing conversations.
It is an explicit acceptance script, never a test-suite or CI entry point.
"""
import argparse
import importlib.util
import json
import os
from pathlib import Path
import subprocess
import shutil
import sys
import tempfile
import threading
import time
from http.server import BaseHTTPRequestHandler, ThreadingHTTPServer


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--codex", type=Path, required=True)
    parser.add_argument("--node", type=Path, required=True)
    parser.add_argument("--helper", type=Path)
    parser.add_argument("--old-helper", type=Path)
    parser.add_argument("--bindings", type=Path)
    parser.add_argument("--library", type=Path)
    parser.add_argument("--resources", type=Path)
    parser.add_argument("--exercise-revert", action="store_true")
    parser.add_argument("--probe-items", action="store_true")
    parser.add_argument("--probe-many-items", action="store_true")
    args = parser.parse_args()
    for path in (args.codex, args.node, args.helper, args.old_helper):
        if path is None:
            continue
        if not path.is_absolute() or not path.exists():
            parser.error("all paths must be absolute and exist")

    class Upstream(BaseHTTPRequestHandler):
        def log_message(self, *_):
            pass

        def do_POST(self):
            self.rfile.read(int(self.headers.get("Content-Length", "0")))
            response = {"id": "synthetic-response", "object": "response",
                        "status": "completed", "output": [{
                            "id": "synthetic-item", "type": "message",
                            "role": "assistant", "status": "completed",
                            "content": [{"type": "output_text", "text": "synthetic answer",
                                          "annotations": []}]}]}
            item = response["output"][0]
            events = [
                {"type": "response.created", "response": {**response,
                    "status": "in_progress", "output": []}},
                {"type": "response.output_item.added", "output_index": 0,
                 "item": {**item, "status": "in_progress", "content": []}},
                {"type": "response.content_part.added", "item_id": item["id"],
                 "output_index": 0, "content_index": 0,
                 "part": {"type": "output_text", "text": "", "annotations": []}},
                {"type": "response.output_text.delta", "item_id": item["id"],
                 "output_index": 0, "content_index": 0, "delta": "synthetic answer"},
                {"type": "response.output_text.done", "item_id": item["id"],
                 "output_index": 0, "content_index": 0, "text": "synthetic answer"},
                {"type": "response.content_part.done", "item_id": item["id"],
                 "output_index": 0, "content_index": 0, "part": item["content"][0]},
                {"type": "response.output_item.done", "output_index": 0, "item": item},
                {"type": "response.completed", "response": response},
            ]
            payload = b"".join(("event: %s\ndata: %s\n\n" %
                                (event["type"], json.dumps(event))).encode()
                               for event in events)
            self.send_response(200)
            self.send_header("Content-Type", "text/event-stream")
            self.send_header("Content-Length", str(len(payload)))
            self.end_headers()
            self.wfile.write(payload)
            self.wfile.flush()

    server = ThreadingHTTPServer(("127.0.0.1", 0), Upstream)
    threading.Thread(target=server.serve_forever, daemon=True).start()

    def invoke(process, ident, method, params, timeout=15):
        process.stdin.write(json.dumps({"jsonrpc": "2.0", "id": ident,
                                        "method": method, "params": params}) + "\n")
        process.stdin.flush()
        deadline = time.monotonic() + timeout
        while time.monotonic() < deadline:
            line = process.stdout.readline()
            if not line:
                continue
            value = json.loads(line)
            if value.get("id") == ident:
                if "error" in value:
                    raise RuntimeError("%s failed: %s" % (method, value["error"]))
                return value["result"]
        raise RuntimeError("timed out waiting for " + method)

    try:
        with tempfile.TemporaryDirectory(prefix="velune-codex-fork-") as directory:
            root = Path(directory)
            codex_home = root / "codex"
            codex_home.mkdir()
            environment = {"PATH": str(args.codex.parent) + ":/usr/bin:/bin",
                           "HOME": str(root), "CODEX_HOME": str(codex_home),
                           "TMPDIR": str(root), "LANG": "C",
                           "SYNTHETIC_KEY": "synthetic-only"}
            process = subprocess.Popen([str(args.codex), "app-server", "--stdio"],
                env=environment, stdin=subprocess.PIPE, stdout=subprocess.PIPE,
                stderr=subprocess.DEVNULL, text=True)
            try:
                invoke(process, 1, "initialize", {"clientInfo": {"name": "velune-probe",
                    "version": "0"}, "capabilities": {"experimentalApi": True}})
                provider = {"model_provider": "synthetic", "model_providers": {
                    "synthetic": {"name": "synthetic", "base_url":
                        "http://127.0.0.1:%d/v1" % server.server_port,
                        "env_key": "SYNTHETIC_KEY", "wire_api": "responses",
                        "requires_openai_auth": False}}}
                options = {"cwd": str(root), "approvalPolicy": "never",
                           "sandbox": "read-only", "modelProvider": "synthetic",
                           "config": provider}
                parent = invoke(process, 2, "thread/start", options)["thread"]
                retained_turn = invoke(process, 3, "turn/start", {"threadId": parent["id"],
                    "input": [{"type": "text", "text": "synthetic retained prefix"}],
                    "model": "gpt-6.1-sol"})
                # The app-server acknowledges turn/start before its rollout
                # writer closes the JSONL. Give the local synthetic response
                # and writer a bounded completion window before forking.
                time.sleep(2.5)
                reverted_turn = invoke(process, 4, "turn/start", {"threadId": parent["id"],
                    "input": [{"type": "text", "text": "synthetic fork parent"}],
                    "model": "gpt-6.1-sol"})
                time.sleep(2.5)
                legacy_items = None
                if args.probe_items:
                    legacy_items = invoke(process, 5, "thread/items/list", {
                        "threadId": parent["id"], "limit": 50, "sortDirection": "asc"})
                reverted = None
                if args.exercise_revert:
                    reverted = invoke(process, 6, "thread/revert", {
                        "threadId": parent["id"], "beforeTurnId": reverted_turn["turn"]["id"]})
                    time.sleep(1)
                child = invoke(process, 7, "thread/fork", {"threadId": parent["id"],
                    "excludeTurns": False, "modelProvider": "synthetic",
                    "config": provider})["thread"]
                invoke(process, 8, "turn/start", {"threadId": child["id"],
                    "input": [{"type": "text", "text": "synthetic fork child"}],
                    "model": "gpt-6.1-sol"})
                time.sleep(2.5)
                many_page = None
                if args.probe_many_items:
                    for index in range(26):
                        invoke(process, 100 + index, "turn/start", {
                            "threadId": child["id"], "input": [{"type": "text",
                            "text": "synthetic pagination %02d" % index}],
                            "model": "gpt-6.1-sol"})
                        time.sleep(.2)
                    many_page = invoke(process, 200, "thread/items/list", {
                        "threadId": child["id"], "limit": 50, "sortDirection": "asc"})
                    assert len(many_page.get("data", [])) == 50 and many_page.get("nextCursor")
                    cursor = many_page["nextCursor"]
                    second_page = invoke(process, 201, "thread/items/list", {
                        "threadId": child["id"], "limit": 50, "cursor": cursor,
                        "sortDirection": "asc"})
                    assert second_page.get("data") and second_page.get("nextCursor") is None
                    starts = [entry.get("startedAtMs", 0) for entry in many_page["data"] + second_page["data"]]
                    assert starts == sorted(starts)
                    many_page = {"firstCount": len(many_page["data"]),
                                 "secondCount": len(second_page["data"]),
                                 "ordered": True}
                paginated_items = None
                if args.probe_items:
                    paginated_items = invoke(process, 9, "thread/items/list", {
                        "threadId": parent["id"], "limit": 50, "sortDirection": "asc"})
                    assert isinstance(paginated_items.get("data"), list)
            finally:
                process.terminate()
                process.wait(timeout=5)

            (codex_home / "config.toml").write_text(
                'model_provider = "synthetic"\n'
                '[model_providers.synthetic]\n'
                'name = "synthetic"\n'
                'base_url = "http://127.0.0.1:%d/v1"\n'
                'env_key = "SYNTHETIC_KEY"\n'
                'wire_api = "responses"\n'
                'requires_openai_auth = false\n' % server.server_port,
                encoding="utf-8")
            exec_result = subprocess.run([
                str(args.codex), "exec", "--cd", str(root),
                "--dangerously-bypass-approvals-and-sandbox",
                "--model", "gpt-6.1-sol", "synthetic exec source"],
                env=environment, capture_output=True, text=True, timeout=30)
            assert exec_result.returncode == 0, exec_result.stderr[-1000:]

            assert parent["id"] != child["id"]
            assert child["forkedFromId"] == parent["id"]
            assert child["parentThreadId"] is None
            assert Path(parent["path"]).exists() and Path(child["path"]).exists()

            def first_record(path):
                with path.open() as stream:
                    return json.loads(stream.readline())

            parent_header = first_record(Path(parent["path"]))
            child_header = first_record(Path(child["path"]))
            assert parent_header["payload"]["id"] == parent["id"]
            assert child_header["payload"]["id"] == child["id"]
            assert child_header["payload"]["forked_from_id"] == parent["id"]
            assert child_header["payload"]["forked_from_ordinal_exclusive"] >= 0

            def session_meta_ids(path):
                ids = []
                with path.open() as stream:
                    for index, line in enumerate(stream):
                        if index >= 8:
                            break
                        value = json.loads(line)
                        if value.get("type") == "session_meta":
                            payload = value.get("payload", {})
                            ids.append({"id": payload.get("id"),
                                        "forkedFrom": payload.get("forked_from_id")})
                return ids

            header_ids = {"parent": session_meta_ids(Path(parent["path"])),
                          "child": session_meta_ids(Path(child["path"]))}
            assert header_ids["parent"] and header_ids["child"]
            assert all(item["id"] == parent["id"] for item in header_ids["parent"])
            assert all(item["id"] == child["id"] for item in header_ids["child"])

            request = {"operation": "list", "provider": "codex", "home": str(root),
                       "roots": [str(codex_home / "sessions")]}
            listing = {}
            if args.helper:
                listed = subprocess.run([str(args.node), str(args.helper)],
                    input=json.dumps(request), text=True, capture_output=True, check=True)
                listing = json.loads(listed.stdout)
            ids = [session["nativeId"] for session in listing.get("sessions", [])]
            histories = {}
            helper_error = listing.get("error") if args.helper else {"code": "helper_omitted"}
            if args.helper and not helper_error:
                if args.exercise_revert:
                    assert ids.count(parent["id"]) >= 2 and child["id"] in ids, listing
                else:
                    assert set(ids) == {parent["id"], child["id"]}, listing
                    assert len(ids) == 2
                for native_id in sorted(set(ids)):
                    result = subprocess.run([str(args.node), str(args.helper)], input=json.dumps({
                        "operation": "read", "provider": "codex", "home": str(root),
                        "roots": [str(codex_home / "sessions")], "nativeId": native_id}),
                        text=True, capture_output=True, check=True)
                    histories[native_id] = json.loads(result.stdout)
                    if args.exercise_revert and native_id == parent["id"]:
                        assert histories[native_id].get("error", {}).get("code") == "ambiguous_session"
                    else:
                        assert "error" not in histories[native_id]

            old_result = {}
            old_ids = []
            if args.old_helper:
                old_listing = subprocess.run([str(args.node), str(args.old_helper)],
                    input=json.dumps(request), text=True, capture_output=True, check=True)
                old_result = json.loads(old_listing.stdout)
                old_ids = [session["nativeId"] for session in old_result.get("sessions", [])]
            old_reads = {}
            for native_id in (parent["id"], child["id"]):
                if not args.old_helper:
                    break
                result = subprocess.run([str(args.node), str(args.old_helper)], input=json.dumps({
                    "operation": "read", "provider": "codex", "home": str(root),
                    "roots": [str(codex_home / "sessions")], "nativeId": native_id}),
                    text=True, capture_output=True, check=True)
                old_reads[native_id] = json.loads(result.stdout)

            application_result = None
            if args.bindings or args.library or args.resources:
                if not (args.bindings and args.library and args.resources):
                    raise RuntimeError("--bindings, --library, and --resources must be supplied together")
                module_dir = root / "bindings"
                module_dir.mkdir()
                shutil.copy(args.bindings / "velune_bindings.py", module_dir)
                (module_dir / args.library.name).symlink_to(args.library)
                spec = importlib.util.spec_from_file_location(
                    "velune_bindings", module_dir / "velune_bindings.py")
                binding = importlib.util.module_from_spec(spec)
                sys.modules[spec.name] = binding
                spec.loader.exec_module(binding)
                saved_home = os.environ.get("HOME")
                saved_codex_home = os.environ.get("CODEX_HOME")
                os.environ["HOME"] = str(root)
                os.environ["CODEX_HOME"] = str(codex_home)
                application = binding.VeluneApplication.open(binding.BindingOptions(
                    home_directory=str(root / "application-home"),
                    resources_directory=str(args.resources)))
                application.upsert_runtime(binding.BindingRuntimeInstance(
                    enabled=True, id="codex", name="Synthetic Codex", type_id="codex-0.159.3",
                    gateway_id="default", settings={"binary": str(args.codex),
                    "nodeBinary": str(args.node), "agentDir": str(codex_home)}))
                application_rows = application.list().conversations
                codex_rows = [row for row in application_rows if row.runtime_id == "codex"]
                assert len(codex_rows) == 3, [(row.id, row.runtime_id) for row in application_rows]
                application_result = {"logicalRows": [row.id for row in codex_rows]}
                parent_row = next(row for row in codex_rows if parent["id"] in row.id)
                child_row = next(row for row in codex_rows if child["id"] in row.id)
                exec_rows = [row for row in codex_rows
                             if parent["id"] not in row.id and child["id"] not in row.id]
                assert len(exec_rows) == 1
                application_result["execRow"] = exec_rows[0].id
                parent_snapshot = application.open_conversation("codex", parent_row.id).snapshot
                child_snapshot = application.open_conversation("codex", child_row.id).snapshot
                assert parent_snapshot and child_snapshot
                assert parent_snapshot.context_runtime_id == "codex"
                assert child_snapshot.context_runtime_id == "codex"
                if args.exercise_revert:
                    parent_text = " ".join(block.text for message in parent_snapshot.messages
                                           for block in message.blocks
                                           if isinstance(block, binding.BindingMessageBlock.TEXT))
                    assert "synthetic retained prefix" in parent_text
                    assert "synthetic fork parent" not in parent_text
                application_result["parentAfterRevertHasRetainedPrefix"] = (
                    "synthetic retained prefix" in parent_text) if args.exercise_revert else None
                application_result["parentAfterRevertHasOldTurn"] = (
                    "synthetic fork parent" in parent_text) if args.exercise_revert else None
                application.shutdown()
                application = binding.VeluneApplication.open(binding.BindingOptions(
                    home_directory=str(root / "application-home"),
                    resources_directory=str(args.resources)))
                restarted_rows = [row for row in application.list().conversations
                                  if row.runtime_id == "codex"]
                assert len(restarted_rows) == 3
                restarted_parent = next(row for row in restarted_rows if parent["id"] in row.id)
                restarted_snapshot = application.open_conversation("codex", restarted_parent.id).snapshot
                assert restarted_snapshot is not None
                if args.exercise_revert:
                    restarted_text = " ".join(block.text for message in restarted_snapshot.messages
                                               for block in message.blocks
                                               if isinstance(block, binding.BindingMessageBlock.TEXT))
                    assert "synthetic retained prefix" in restarted_text
                    assert "synthetic fork parent" not in restarted_text
                application_result["restartRows"] = [row.id for row in restarted_rows]
                application.shutdown()
                if saved_home is None:
                    os.environ.pop("HOME", None)
                else:
                    os.environ["HOME"] = saved_home
                if saved_codex_home is None:
                    os.environ.pop("CODEX_HOME", None)
                else:
                    os.environ["CODEX_HOME"] = saved_codex_home

            print(json.dumps({"acceptance": "PASSED", "codexVersion": "0.159.3",
                "parentId": parent["id"], "childId": child["id"],
                "childForkedFromId": child["forkedFromId"],
                "headerSessionMetaIds": header_ids,
                "huihuaNativeIds": ids, "huihuaError": helper_error,
                "oldHelperNativeIds": old_ids if args.old_helper else None,
                "threadRevertCreatedSameCanonicalId": args.exercise_revert,
                "itemsListLegacy": legacy_items,
                "itemsListAfterRevert": paginated_items,
                "itemsListOver50": many_page,
                "oldHelperReads": old_reads,
                "application": application_result,
                "ambiguousSession": False,
                "realHomeRead": False, "realCredentials": False}, indent=2))
    finally:
        server.shutdown()
        server.server_close()


if __name__ == "__main__":
    main()
