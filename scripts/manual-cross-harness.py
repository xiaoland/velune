#!/usr/bin/env python3
"""Manual synthetic Pi -> Codex -> DSH -> Pi handoff acceptance.

This is deliberately a temporary end-to-end probe, never a test-suite entry
point.  It uses a temporary HOME, temporary harness state, and a loopback HTTP
provider.  It never reads the user's real Velune, Pi, Codex, or DSH state.
"""
import argparse
import importlib.util
import json
import os
from pathlib import Path
import shutil
import sys
import tempfile
import threading
import time
from http.server import BaseHTTPRequestHandler, ThreadingHTTPServer


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    for name in ("bundle", "bindings", "node", "pi", "codex", "dsh"):
        parser.add_argument("--" + name, required=True, type=Path)
    args = parser.parse_args()
    for path in vars(args).values():
        if not path.is_absolute() or not path.exists():
            parser.error("all paths must be absolute and exist")

    captures = []
    server_errors = []
    old_environment = dict(os.environ)

    class Upstream(BaseHTTPRequestHandler):
        def log_message(self, *_):
            pass

        def do_POST(self):
            try:
                body = json.loads(self.rfile.read(int(self.headers["Content-Length"])))
                captures.append({"path": self.path, "body": body,
                                 "authorization": self.headers.get("Authorization")})
                model = body.get("model", "synthetic")
                text = "SYNTHETIC_ANSWER_" + str(len(captures))
                if self.path.endswith("/responses"):
                    response_id = "synthetic-response-%d" % len(captures)
                    item = {"id": "synthetic-item-%d" % len(captures), "type": "message",
                            "role": "assistant", "status": "completed",
                            "content": [{"type": "output_text", "text": text,
                                          "annotations": []}]}
                    response = {"id": response_id, "object": "response",
                                "status": "completed", "model": model, "output": [item]}
                    events = [{"type": "response.created", "response": {**response, "status": "in_progress", "output": []}},
                              {"type": "response.output_item.added", "output_index": 0,
                               "item": {**item, "status": "in_progress", "content": []}},
                              {"type": "response.content_part.added", "item_id": item["id"],
                               "output_index": 0, "content_index": 0,
                               "part": {"type": "output_text", "text": "", "annotations": []}},
                              {"type": "response.output_text.delta", "item_id": item["id"],
                               "output_index": 0, "content_index": 0, "delta": text},
                              {"type": "response.output_text.done", "item_id": item["id"],
                               "output_index": 0, "content_index": 0, "text": text},
                              {"type": "response.content_part.done", "item_id": item["id"],
                               "output_index": 0, "content_index": 0,
                               "part": item["content"][0]},
                              {"type": "response.output_item.done", "output_index": 0,
                               "item": item}, {"type": "response.completed", "response": response}]
                    payload = b"".join(("event: %s\ndata: %s\n\n" %
                                        (event["type"], json.dumps(event))).encode()
                                       for event in events)
                else:
                    chunks = [{"id": "synthetic-chat", "object": "chat.completion.chunk",
                               "model": model, "choices": [{"index": 0,
                               "delta": {"role": "assistant", "content": text},
                               "finish_reason": None}]},
                              {"id": "synthetic-chat", "object": "chat.completion.chunk",
                               "model": model, "choices": [{"index": 0, "delta": {},
                               "finish_reason": "stop"}]}]
                    payload = b"".join(("data: %s\n\n" % json.dumps(chunk)).encode()
                                       for chunk in chunks) + b"data: [DONE]\n\n"
                self.send_response(200)
                self.send_header("Content-Type", "text/event-stream")
                self.send_header("Content-Length", str(len(payload)))
                self.send_header("X-Request-ID", "synthetic-%d" % len(captures))
                self.end_headers()
                self.wfile.write(payload)
                self.wfile.flush()
            except Exception as error:
                server_errors.append(repr(error))

    server = ThreadingHTTPServer(("127.0.0.1", 0), Upstream)
    threading.Thread(target=server.serve_forever, daemon=True).start()
    application = None
    report = {"acceptance": "NOT_RUN", "checks": [], "unavailable": []}
    try:
        with tempfile.TemporaryDirectory(prefix="velune-cross-harness-") as directory:
            root = Path(directory)
            for name in ("home", "project", "pi", "codex", "dsh", "bindings"):
                (root / name).mkdir()
            bindings_file = args.bindings / "velune_bindings.py"
            if not bindings_file.exists():
                raise RuntimeError("Python UniFFI bindings are missing")
            shutil.copy(bindings_file, root / "bindings" / bindings_file.name)
            library = args.bundle / "Contents/Frameworks/libvelune_bindings.dylib"
            if not library.exists():
                raise RuntimeError("macOS bindings library is missing from the bundle")
            (root / "bindings" / library.name).symlink_to(library)
            os.environ.clear()
            os.environ.update(HOME=str(root / "home"),
                              PATH=str(args.node.parent) + ":/usr/bin:/bin",
                              NO_PROXY="127.0.0.1,localhost")
            spec = importlib.util.spec_from_file_location(
                "velune_bindings", root / "bindings" / bindings_file.name)
            bindings = importlib.util.module_from_spec(spec)
            sys.modules[spec.name] = bindings
            try:
                spec.loader.exec_module(bindings)
            except (AttributeError, OSError) as error:
                raise RuntimeError(
                    "generated UniFFI bindings/library are out of sync; rebuild both before this probe: "
                    + str(error)) from error
            if not hasattr(bindings.VeluneApplication, "send_turn"):
                raise RuntimeError("generated bindings are stale: send_turn is unavailable; regenerate UniFFI bindings")
            resources = args.bundle / "Contents/Resources"
            application = bindings.VeluneApplication.open(bindings.BindingOptions(
                home_directory=str(root / "home"), resources_directory=str(resources)))
            endpoint = "http://127.0.0.1:%d/v1" % server.server_port
            protocol_chat = bindings.BindingGatewayProtocol.CHAT_COMPLETIONS_V1
            protocol_responses = bindings.BindingGatewayProtocol.RESPONSES_V1

            def model(protocol, model_id):
                return bindings.BindingProviderModel(record_key="", provider_model_id=model_id,
                    nickname="Synthetic %s" % model_id, icon=None, context_window=32768,
                    max_output_tokens=4096, reasoning_levels=None, adapter_metadata_json=None)

            def provider(identity, protocol, model_id):
                draft = bindings.BindingProviderDraft(id=identity, name=identity,
                    protocol=protocol, endpoint=endpoint,
                    models=[model(protocol, model_id)])
                application.save_provider("default", draft,
                    bindings.BindingAuthenticationEdit.SET_API_KEY(value="synthetic-only"))
                return next(p for gateway in application.list().gateways
                            for p in gateway.providers if p.id == identity)

            providers = {
                "pi": provider("synthetic-pi", protocol_chat, "pi-model"),
                "codex": provider("synthetic-codex", protocol_responses, "codex-model"),
                "dsh": provider("synthetic-dsh", protocol_chat, "dsh-model"),
            }
            types = {descriptor.id: descriptor for descriptor in application.list().runtime_types}
            binaries = {"pi": args.pi,
                        "codex": args.codex, "dsh": args.dsh}
            runtime_types = {"pi": "pi-1.0.2", "codex": "codex-0.159.3",
                             "dsh": "dsh-acp-0.2.0-rc.2"}
            for family in ("pi", "codex", "dsh"):
                if runtime_types[family] not in types:
                    raise RuntimeError("runtime descriptor missing: " + runtime_types[family])
                runtime = bindings.BindingRuntimeInstance(enabled=True, id=family,
                    name="Synthetic " + family, type_id=runtime_types[family], gateway_id="default",
                    settings={"binary": str(binaries[family]), "nodeBinary": str(args.node),
                              "agentDir": str(root / family)})
                application.upsert_runtime(runtime)

            # A logical conversation starts with one real native source.  The
            # later send_turn calls deliberately change the target runtime;
            # they must not manufacture a source by calling send first.
            initial = application.create_conversation(
                "pi", str(root / "project"), providers["pi"].models[0].record_key)
            assert initial.snapshot is not None, "Pi source conversation was not created"

            def wait_for_turn(runtime_id, before):
                deadline = time.monotonic() + 45
                while time.monotonic() < deadline:
                    snapshot = application.snapshot(runtime_id).snapshot
                    if snapshot and snapshot.run_state == bindings.BindingRunState.FAILED:
                        raise RuntimeError(runtime_id + " synthetic turn failed")
                    if snapshot and snapshot.actions.can_send and len(captures) > before:
                        return snapshot
                    time.sleep(.05)
                raise RuntimeError(runtime_id + " synthetic turn did not settle")

            # Each call is a real application -> runtime -> loopback gateway path.
            snapshots = []
            row_ids = []
            visible_turns = ("PI_ORIGIN", "CODEX_TURN", "DSH_TURN", "PI_RETURN")
            for family, text in (("pi", "PI_ORIGIN"), ("codex", "CODEX_TURN"),
                                 ("dsh", "DSH_TURN"), ("pi", "PI_RETURN")):
                before = len(captures)
                result = application.send_turn(family, providers[family].models[0].record_key, text)
                snapshot = wait_for_turn(family, before)
                snapshots.append(snapshot)
                assert snapshot.context_runtime_id == family, (
                    "snapshot context runtime drifted: %r != %r" %
                    (snapshot.context_runtime_id, family))
                assert len(captures) == before + 1, "a turn issued more than one upstream request"
                rows = application.list().conversations
                assert len(rows) == 1, "logical conversation was split into multiple visible rows"
                row_ids.append(rows[0].id)
                user_texts = [block.text for message in snapshot.messages
                              if message.role == bindings.BindingMessageRole.USER
                              for block in message.blocks
                              if isinstance(block, bindings.BindingMessageBlock.TEXT)]
                assert text in user_texts, "current user turn is missing from projection"
                assert not any("velune-context" in value or "contextMeaning" in value
                                for value in user_texts), "handoff payload leaked into visible user text"
                assert not any(value.count("PI_ORIGIN") > 1 for value in user_texts), "handoff recursively duplicated"
            report["checks"].append("four native turns issued once each")
            assert len(set(row_ids)) == 1, "logical conversation ID changed during handoff"
            report["checks"].append("native snapshots remained readable")

            # Reopen through the same public entry point.  This checks the
            # native source projection survives a process boundary without
            # keeping an in-memory application object alive.
            application.shutdown()
            application = bindings.VeluneApplication.open(bindings.BindingOptions(
                home_directory=str(root / "home"), resources_directory=str(resources)))
            reopened_rows = application.list().conversations
            assert len(reopened_rows) == 1 and reopened_rows[0].id == row_ids[0], \
                "restart changed the logical conversation row"
            assert reopened_rows[0].can_rename and not reopened_rows[0].can_delete, \
                "aggregate logical management capabilities are wrong"
            report["checks"].append("native conversation list survived restart")

            config = root / "home" / "conversation-links.json"
            if config.exists():
                raw = config.read_text()
                for forbidden in ("PI_ORIGIN", "CODEX_TURN", "DSH_TURN", "PI_RETURN",
                                  "SYNTHETIC_ANSWER"):
                    assert forbidden not in raw, "handoff text leaked into relationship metadata"
                links = json.loads(raw)
                assert links.get("schemaVersion", 1) == 1 and len(links.get("links", [])) == 1
                link = links["links"][0]
                assert link["id"] == row_ids[0] and len(link["segments"]) == 4
                assert [segment["runtimeInstanceId"] for segment in link["segments"]] == \
                    ["pi", "codex", "dsh", "pi"]
                assert all(segment["nativeConversationId"] for segment in link["segments"])
                assert all("payload" not in segment.get("handoff", {}) for segment in link["segments"]
                           if segment.get("handoff"))
                origin_native = link["segments"][0]["nativeConversationId"]
                reopened_origin = application.open_conversation("pi", origin_native).snapshot
                assert reopened_origin is not None
                original_user_texts = [block.text for message in snapshots[-1].messages
                                       if message.role == bindings.BindingMessageRole.USER
                                       for block in message.blocks
                                       if isinstance(block, bindings.BindingMessageBlock.TEXT)]
                reopened_user_texts = [block.text for message in reopened_origin.messages
                                      if message.role == bindings.BindingMessageRole.USER
                                      for block in message.blocks
                                      if isinstance(block, bindings.BindingMessageBlock.TEXT)]
                assert all(value in reopened_user_texts for value in original_user_texts), \
                    "origin text changed across restart: %r missing from %r" % (original_user_texts, reopened_user_texts)
                assert {message.id for message in snapshots[-1].messages}.issubset(
                    {message.id for message in reopened_origin.messages})
                report["checks"].append("origin native texts and projected message IDs survived restart")
                origin_path = Path(origin_native.split(":", 1)[1]) if ":" in origin_native else None
                if origin_path is None or not origin_path.exists():
                    raise AssertionError("Pi origin native file is not addressable for cutoff acceptance")
                original_bytes = origin_path.read_bytes()
                try:
                    # Appending after the sealed prefix is allowed, but the
                    # appended native record must not enter the projection.
                    origin_path.write_bytes(original_bytes + b"\n")
                    appended = application.open_conversation("pi", origin_native).snapshot
                    assert appended is not None
                    assert not any("SYNTHETIC_ANSWER" in block.text for message in appended.messages
                                   for block in message.blocks
                                   if isinstance(block, bindings.BindingMessageBlock.TEXT)
                                   and message.role == bindings.BindingMessageRole.USER)
                    report["checks"].append("closed Pi append stayed outside the sealed prefix")

                    # Rewriting the sealed prefix must be rejected rather than
                    # silently falling back to the full native history.
                    rewritten = original_bytes.replace(b"PI_ORIGIN", b"PI_REWRITE", 1)
                    assert rewritten != original_bytes, "Pi fixture did not contain the origin marker"
                    origin_path.write_bytes(rewritten)
                    try:
                        application.open_conversation("pi", origin_native)
                    except bindings.BindingError as error:
                        report["rewriteFailure"] = str(error)
                    else:
                        raise AssertionError("rewritten sealed Pi prefix was accepted")
                    report["checks"].append("rewritten closed Pi prefix was rejected")
                finally:
                    origin_path.write_bytes(original_bytes)
                report["checks"].append("relationship metadata contains no transcript text")
            else:
                raise AssertionError("conversation-links.json missing after cross-harness handoff")

            assert not server_errors, server_errors
            report["upstreamRequests"] = len(captures)
            report["acceptance"] = "PASSED"
            print(json.dumps(report, indent=2, ensure_ascii=False))
    finally:
        if application is not None:
            try:
                application.shutdown()
            except Exception:
                pass
        os.environ.clear()
        os.environ.update(old_environment)
        server.shutdown()
        server.server_close()


if __name__ == "__main__":
    main()
