#!/usr/bin/env python3
"""Explicit isolated native Pi continuation failure checks; never a CI/test entry point."""
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
    for key in ("bundle", "bindings", "node"):
        parser.add_argument("--" + key, required=True, type=Path)
    args = parser.parse_args()
    captures = []

    class Upstream(BaseHTTPRequestHandler):
        def log_message(self, *_):
            pass

        def do_POST(self):
            captures.append(json.loads(self.rfile.read(int(self.headers["Content-Length"]))))
            body = {"id": "synthetic", "object": "chat.completion.chunk", "model": "synthetic",
                    "choices": [{"index": 0, "delta": {"role": "assistant", "content": "BOUNDARY_ANSWER"}, "finish_reason": None}]}
            end = {**body, "choices": [{"index": 0, "delta": {}, "finish_reason": "stop"}]}
            payload = ("data: " + json.dumps(body) + "\n\ndata: " + json.dumps(end) + "\n\ndata: [DONE]\n\n").encode()
            self.send_response(200)
            self.send_header("Content-Type", "text/event-stream")
            self.send_header("Content-Length", str(len(payload)))
            self.end_headers()
            self.wfile.write(payload)

    server = ThreadingHTTPServer(("127.0.0.1", 0), Upstream)
    threading.Thread(target=server.serve_forever, daemon=True).start()
    previous_environment = dict(os.environ)
    application = None
    try:
        with tempfile.TemporaryDirectory(prefix="velune-continuation-boundaries-") as directory:
            root = Path(directory)
            for name in ("home", "project", "alpha", "beta", "bindings"):
                (root / name).mkdir()
            shutil.copy(args.bindings / "velune_bindings.py", root / "bindings")
            (root / "bindings/libvelune_bindings.dylib").symlink_to(args.bundle / "Contents/Frameworks/libvelune_bindings.dylib")
            os.environ.clear()
            os.environ.update(HOME=str(root / "home"), PATH=f"{args.node.parent}:/usr/bin:/bin", NO_PROXY="127.0.0.1,localhost")
            spec = importlib.util.spec_from_file_location("velune_bindings", root / "bindings/velune_bindings.py")
            b = importlib.util.module_from_spec(spec)
            sys.modules[spec.name] = b
            spec.loader.exec_module(b)
            resources = args.bundle / "Contents/Resources"
            options = b.BindingOptions(home_directory=str(root / "home"), resources_directory=str(resources))
            application = b.VeluneApplication.open(options)
            model = b.BindingProviderModel(record_key="", provider_model_id="synthetic", nickname="Synthetic", icon=None,
                                          context_window=32768, max_output_tokens=4096, reasoning_levels=None, adapter_metadata_json=None)
            draft = b.BindingProviderDraft(id="fixture", name="Fixture", protocol=b.BindingGatewayProtocol.CHAT_COMPLETIONS_V1,
                                          endpoint=f"http://127.0.0.1:{server.server_port}/v1", models=[model])
            application.save_provider("default", draft, b.BindingAuthenticationEdit.SET_API_KEY(value="synthetic-only"))
            key = application.list().gateways[0].providers[0].models[0].record_key
            binary = resources / "node_modules/@earendil-works/pi-coding-agent/dist/bundle/cli.js"
            for identity in ("alpha", "beta"):
                application.upsert_runtime(b.BindingRuntimeInstance(enabled=True, id=identity, name=identity,
                    type_id="pi-1.0.2", gateway_id="default", settings={"binary": str(binary), "nodeBinary": str(args.node), "agentDir": str(root / identity)}))

            def settle(identity, expected):
                deadline = time.monotonic() + 30
                while time.monotonic() < deadline:
                    snapshot = application.snapshot(identity).snapshot
                    if snapshot and snapshot.run_state == b.BindingRunState.FAILED:
                        raise AssertionError("synthetic native turn failed")
                    if snapshot and snapshot.actions.can_send and len(captures) == expected:
                        return snapshot
                    time.sleep(.05)
                raise AssertionError("native turn did not settle")

            application.create_conversation("alpha", str(root / "project"), key)
            application.send_turn("alpha", key, "BOUNDARY_ORIGIN")
            source = settle("alpha", 1)
            source_path = Path(source.conversation.id.removeprefix("alpha:"))
            source_bytes = source_path.read_bytes()
            links_path = root / "home/conversation-links.json"

            def rejected(action):
                try:
                    action()
                except b.BindingError:
                    pass
                else:
                    raise AssertionError("operation unexpectedly accepted")
                assert len(captures) == 1, "a rejected handoff dispatched upstream"
                assert source_path.read_bytes() == source_bytes, "rejected handoff changed native source"
                current = application.snapshot("alpha").snapshot
                assert current and current.conversation.id == source.conversation.id

            rejected(lambda: application.send_turn("beta", key, "x" * (256 * 1024)))
            assert not links_path.exists()
            unavailable_binary = root / "bad-runtime"
            unavailable_binary.write_text("#!/bin/sh\nprintf '99.0.0\\n'\n")
            unavailable_binary.chmod(0o700)
            application.upsert_runtime(b.BindingRuntimeInstance(enabled=True, id="bad", name="bad", type_id="codex-0.159.3",
                gateway_id="default", settings={"binary": str(unavailable_binary), "nodeBinary": str(args.node), "agentDir": str(root / "beta")}))
            # A ChatCompletions model cannot execute on Codex: reject before preparation.
            rejected(lambda: application.send_turn("bad", key, "NEVER_SENT"))
            assert not links_path.exists()
            links_path.mkdir()  # Atomic rename cannot replace this directory.
            rejected(lambda: application.send_turn("beta", key, "PERSIST_FAILURE"))
            links_path.rmdir()
            assert len(application.list().conversations) == 1
            application.send_turn("beta", key, "BOUNDARY_TARGET")
            target = settle("beta", 2)
            assert target.conversation.id == source.conversation.id and target.context_runtime_id == "beta"
            assert source_path.read_bytes() == source_bytes
            metadata = links_path.read_bytes()
            assert links_path.stat().st_mode & 0o777 == 0o600
            for text in (b"BOUNDARY_ORIGIN", b"BOUNDARY_TARGET", b"BOUNDARY_ANSWER", b"synthetic-only"):
                assert text not in metadata
            assert len(json.loads(metadata)["links"][0]["segments"]) == 2
            application.shutdown()
            application = None
            # A missing target must not hide already checked source history or
            # pretend an unreadable native context is ready to resume.
            target_id = json.loads(metadata)["links"][0]["segments"][-1]["nativeConversationId"]
            target_path = Path(target_id.removeprefix("beta:"))
            target_bytes = target_path.read_bytes()
            target_path.unlink()
            application = b.VeluneApplication.open(options)
            unavailable = application.open_conversation("alpha", source.conversation.id).snapshot
            assert unavailable and unavailable.context_runtime_id == "beta" and not unavailable.actions.can_send
            visible = [block.text for message in unavailable.messages for block in message.blocks if isinstance(block, b.BindingMessageBlock.TEXT)]
            assert "BOUNDARY_ORIGIN" in visible and "BOUNDARY_TARGET" not in visible
            try:
                application.send_turn("beta", key, "NEVER_RESEND")
            except b.BindingError:
                pass
            else:
                raise AssertionError("unreadable native context accepted a new turn")
            assert len(captures) == 2
            target_path.write_bytes(target_bytes)
            restored = application.open_conversation("alpha", source.conversation.id).snapshot
            assert restored and restored.actions.can_send and restored.context_runtime_id == "beta"
            application.shutdown()
            application = None
            # Provider schema reset must not destroy authoritative associations.
            (root / "home/generic-config.json").write_text('{"schemaVersion":6}')
            application = b.VeluneApplication.open(options)
            assert links_path.read_bytes() == metadata
            rows = application.list().conversations
            assert len(rows) == 1 and rows[0].id == source.conversation.id
            assert not rows[0].can_rename and not rows[0].can_delete
            assert len(captures) == 2, "restart dispatched an unsolicited turn"
            print(json.dumps({"oversizeRejectedBeforePrepare": True, "incompatibleTargetRejected": True,
                "failedAssociationCommitPreservesSource": True, "explicitRetryCreatesSingleLinkedTarget": True,
                "metadataContainsNoContentOrCredentials": True, "metadataMode600": True,
                "unreadableTailKeepsCheckedSourceWithoutResume": True,
                "providerSchemaResetPreservesAssociations": True, "restartNeverResends": True,
                "upstreamRequests": len(captures)}))
    finally:
        if application:
            application.shutdown()
        os.environ.clear()
        os.environ.update(previous_environment)
        server.shutdown()
        server.server_close()


if __name__ == "__main__":
    main()
