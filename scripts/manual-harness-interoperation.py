#!/usr/bin/env python3
"""Manual macOS acceptance: OpenCode -> Velune MCP -> DSH -> OpenCode.

Supply built UniFFI Python bindings/library and installed runtime binaries.
Everything runs in disposable HOME directories with synthetic loopback providers.
This script is not registered as a test or CI job.
"""
import argparse
import http.server
import importlib.util
import json
import os
from pathlib import Path
import shutil
import sys
import tempfile
import threading
import time

CHILD_RESULT = "CHILD_DSH_RESULT"
PARENT_RESULT = "PARENT_AFTER_CHILD"


def arguments():
    parser = argparse.ArgumentParser(description=__doc__)
    for name in ("bindings", "library", "opencode", "dsh", "node"):
        parser.add_argument("--" + name, type=Path, required=True)
    args = parser.parse_args()
    for name, path in vars(args).items():
        path = path.resolve(strict=True)
        if name == "bindings":
            path = path / "velune_bindings.py" if path.is_dir() else path
        if not path.is_file():
            parser.error(f"--{name} must identify a file")
        setattr(args, name, path)
    if sys.platform != "darwin" or not Path("/usr/bin/sandbox-exec").exists():
        parser.error("this isolated acceptance requires macOS sandbox-exec")
    return args


def load_bindings(args, root):
    directory = root / "bindings"
    directory.mkdir()
    # Always use the requested library, never an older copy beside generated bindings.
    shutil.copyfile(args.bindings, directory / "velune_bindings.py")
    shutil.copyfile(args.library, directory / "libvelune_bindings.dylib")
    spec = importlib.util.spec_from_file_location(
        "velune_bindings", directory / "velune_bindings.py"
    )
    module = importlib.util.module_from_spec(spec)
    sys.modules[spec.name] = module
    spec.loader.exec_module(module)
    return module


def chat_stream(text=None, tool=None, arguments=None, sequence=0):
    delta = {"role": "assistant"}
    if tool:
        delta["tool_calls"] = [{"index": 0, "id": f"call_{sequence}",
                                "type": "function", "function": {
                                    "name": tool, "arguments": json.dumps(arguments)}}]
    else:
        delta["content"] = text
    chunks = [{"id": f"chunk_{sequence}", "object": "chat.completion.chunk",
               "model": "fixture-model", "choices": [
                   {"index": 0, "delta": delta, "finish_reason": None}]},
              {"id": f"chunk_{sequence}", "object": "chat.completion.chunk",
               "model": "fixture-model", "choices": [
                   {"index": 0, "delta": {},
                    "finish_reason": "tool_calls" if tool else "stop"}]}]
    return b"".join(b"data: " + json.dumps(chunk).encode() + b"\n\n"
                    for chunk in chunks) + b"data: [DONE]\n\n"


def messages_stream():
    events = [
        ("message_start", {"type": "message_start", "message": {
            "id": "child-msg", "type": "message", "role": "assistant",
            "model": "synthetic-model", "content": [], "stop_reason": None,
            "stop_sequence": None, "usage": {"input_tokens": 1, "output_tokens": 0}}}),
        ("content_block_start", {"type": "content_block_start", "index": 0,
                                 "content_block": {"type": "text", "text": ""}}),
        ("content_block_delta", {"type": "content_block_delta", "index": 0,
                                 "delta": {"type": "text_delta", "text": CHILD_RESULT}}),
        ("content_block_stop", {"type": "content_block_stop", "index": 0}),
        ("message_delta", {"type": "message_delta",
                           "delta": {"stop_reason": "end_turn", "stop_sequence": None},
                           "usage": {"output_tokens": 1}}),
        ("message_stop", {"type": "message_stop"}),
    ]
    return "".join(f"event: {name}\ndata: {json.dumps(value)}\n\n"
                   for name, value in events).encode()


class Upstream(http.server.ThreadingHTTPServer):
    def __init__(self):
        super().__init__(("127.0.0.1", 0), Handler)
        self.child_requested = threading.Event()
        self.parent_received = threading.Event()
        self.failed = threading.Event()
        self.errors = []
        self.operations = set()
        self.sequence = 0


class Handler(http.server.BaseHTTPRequestHandler):
    def log_message(self, *_args):
        pass

    def do_POST(self):
        try:
            body = json.loads(self.rfile.read(int(self.headers["content-length"])))
            if self.path.split("?", 1)[0].endswith("/messages"):
                assert body["model"] == "synthetic-model"
                assert "Velune 协作输入" in json.dumps(body, ensure_ascii=False)
                self.server.child_requested.set()
                output = messages_stream()
            else:
                assert self.path.endswith("/chat/completions")
                output = self.parent_response(body)
            self.send_response(200)
            self.send_header("Content-Type", "text/event-stream")
            self.send_header("Content-Length", str(len(output)))
            self.end_headers()
            self.wfile.write(output)
        except Exception as error:
            self.server.errors.append(type(error).__name__ + ": " + str(error))
            self.server.failed.set()
            self.send_error(500, "synthetic fixture failed")

    def parent_response(self, body):
        self.server.sequence += 1
        tools = {item["function"]["name"] for item in body.get("tools", [])}
        if not tools:
            return chat_stream(text="Synthetic title")
        results = [message for message in body["messages"] if message["role"] == "tool"]
        if not results:
            operation, args = "agents", {}
        else:
            result = json.loads(results[-1]["content"])
            if "targets" in result:
                assert [target["id"] for target in result["targets"]] == ["child-target"]
                operation, args = "spawn", {"target_id": "child-target",
                                            "prompt": "Return a short synthetic child result."}
            elif result.get("state") == "idle":
                assert result["result"] == CHILD_RESULT
                self.server.parent_received.set()
                return chat_stream(text=PARENT_RESULT, sequence=self.server.sequence)
            else:
                assert result["state"] in ("starting", "running"), result.get("state")
                operation, args = "wait", {"child_id": result["id"],
                                           "after_revision": result["revision"],
                                           "timeout_ms": 30000}
        names = [name for name in tools if name.endswith("_" + operation)]
        assert len(names) == 1, f"missing/ambiguous MCP {operation} tool"
        self.server.operations.add(operation)
        return chat_stream(tool=names[0], arguments=args, sequence=self.server.sequence)


def sandbox_profile(node):
    # Deny native config/MDM reads as well as inherited credentials. The explicit
    # Node installation is the only allowed /Users subtree needed by this fixture.
    allow_node = ""
    if node.is_relative_to("/Users"):
        installation = node.parent.parent if node.parent.name == "bin" else node
        allow_node = f"(allow file-read* (subpath {json.dumps(str(installation))}))"
    return ('(version 1) (allow default) '
            '(deny file-read* (subpath "/Users") '
            '(subpath "/Library/Managed Preferences") '
            '(subpath "/Library/Application Support/opencode")) '
            '(deny file-write* (subpath "/Users")) '
            + allow_node +
            '(deny network*) '
            '(allow network-outbound (remote ip "localhost:*")) '
            '(allow network-inbound (local ip "localhost:*"))')


def run(args, root):
    for name in ("project", "resources", "parent", "child", "home", "tmp"):
        (root / name).mkdir()
    os.environ.clear()
    os.environ.update(PATH="/usr/bin:/bin", HOME=str(root / "home"),
                      TMPDIR=str(root / "tmp"), NO_PROXY="127.0.0.1,localhost",
                      OTEL_SDK_DISABLED="true")
    os.chdir(root / "project")
    bindings = load_bindings(args, root)
    upstream = Upstream()
    thread = threading.Thread(target=upstream.serve_forever, daemon=True)
    thread.start()
    app = None
    identity = None
    try:
        base_url = f"http://127.0.0.1:{upstream.server_port}/v1"
        config = root / "opencode.json"
        config.write_text(json.dumps({
            "enabled_providers": ["fixture"], "model": "fixture/fixture-model",
            "small_model": "fixture/fixture-model", "share": "disabled",
            "permission": {"*": "allow"}, "provider": {"fixture": {
                "npm": "@ai-sdk/openai-compatible", "name": "Synthetic loopback",
                "options": {"baseURL": base_url, "apiKey": "synthetic-only"},
                "models": {"fixture-model": {"name": "Fixture", "limit": {
                    "context": 8192, "output": 1024}}}}}}))
        patch = root / "dsh.patch.yml"
        patch.write_text(f"""- insert:
    - id: llm-pi-ai
      name: '@deepseek-ai/dsh-llm-pi-ai'
      config:
        providers:
          synthetic:
            api: anthropic-messages
            apiKeyEnv: SYNTHETIC_KEY
            baseURL: {base_url}
            models:
              - id: synthetic-model
                contextWindow: 8192
""")
        profile = sandbox_profile(args.node)
        app = bindings.VeluneApplication.open(bindings.BindingOptions(
            home_directory=str(root / "application"), resources_directory=str(root / "resources")))

        def register(runtime_id, command, environment):
            runtime_home = root / runtime_id
            for key, name in (("XDG_CONFIG_HOME", "config"), ("XDG_DATA_HOME", "data"),
                              ("XDG_CACHE_HOME", "cache"), ("XDG_STATE_HOME", "state")):
                environment[key] = str(runtime_home / name)
            app.upsert_acp_runtime(
                bindings.BindingRuntimeInstance(enabled=True, id=runtime_id, name=runtime_id,
                    type_id="acp-v1", gateway_id="default",
                    settings={"binary": "/usr/bin/sandbox-exec"}),
                bindings.BindingRuntimeExecution(transport=bindings.BindingRuntimeTransport.LOCAL(),
                    home=bindings.BindingRuntimeHome.EXISTING(path=str(runtime_home)),
                    remote_resources_directory=None),
                bindings.BindingAcpLaunch(args=["-p", profile, *map(str, command)],
                                          environment=environment))

        register("parent", [args.opencode, "acp"], {
            "OPENCODE_CONFIG": str(config), "OPENCODE_TEST_HOME": str(root / "parent"),
            "OPENCODE_TEST_MANAGED_CONFIG_DIR": str(root / "parent" / "managed"),
            "OPENCODE_DISABLE_MODELS_FETCH": "1", "OPENCODE_DISABLE_AUTOUPDATE": "1",
            "OPENCODE_DISABLE_PROJECT_CONFIG": "1",
            "OPENCODE_EXPERIMENTAL_DISABLE_FILEWATCHER": "1", "OPENCODE_DISABLE_FFF": "1"})
        register("child", [args.node, args.dsh, "--profile", "acp", "--patch", patch], {
            "DSH_HOME": str(root / "child" / "dsh"), "SYNTHETIC_KEY": "synthetic-only"})
        default = bindings.BindingModelSelection.RUNTIME_DEFAULT()
        target = bindings.BindingCollaborationTarget(
            id="child-target", name="DSH child", runtime_id="child", cwd=str(root / "project"),
            model_selection=bindings.BindingModelSelection.NATIVE(
                None, '["synthetic","synthetic-model"]'))
        result = app.create_collaborative_conversation(
            "parent", str(root / "project"), default, [target],
            bindings.BindingCollaborationLimits(max_children=1, turn_timeout_ms=20000))
        identity = result.snapshot.conversation.id
        app.send_turn_with_selection("parent", identity, "parent", default,
                                     "Delegate this task to the child, wait, then report its result.")
        # Do not call any application polling API until the parent has received
        # the child's result. The child must make progress on its own worker.
        deadline = time.monotonic() + 45
        while not upstream.parent_received.wait(0.05):
            assert not upstream.failed.is_set(), upstream.errors
            assert time.monotonic() < deadline, "parent did not receive the child result"
        assert upstream.child_requested.is_set()
        assert {"agents", "spawn", "wait"} <= upstream.operations
        deadline = time.monotonic() + 10
        while True:
            snapshot = app.snapshot("parent", identity).snapshot
            if snapshot.run_state == bindings.BindingRunState.IDLE:
                break
            assert snapshot.run_state != bindings.BindingRunState.FAILED
            assert time.monotonic() < deadline, "parent did not settle"
            time.sleep(0.05)
        texts = [block.text for message in snapshot.messages for block in message.blocks
                 if isinstance(block, bindings.BindingMessageBlock.TEXT)]
        assert PARENT_RESULT in texts, "parent continuation text missing"
        children = app.collaboration_children()
        assert len(children) == 1 and children[0].state == bindings.BindingChildState.IDLE
        assert children[0].result == CHILD_RESULT
        child = app.child_snapshot(children[0].id)
        assert child.snapshot is not None and child.snapshot.messages
        assert all(message.stamp > 0 for message in child.snapshot.messages)
        stamps = [(message.id, message.stamp) for message in child.snapshot.messages]
        again = app.child_snapshot(children[0].id)
        assert stamps == [(message.id, message.stamp) for message in again.snapshot.messages]
        app.close_collaboration()
        app.shutdown()
        app = None
        print("PASS: real OpenCode -> Velune MCP -> real DSH -> parent continuation; "
              "no application polling during delegation; child snapshot stamps stable")
    finally:
        try:
            if app is not None:
                if identity is not None:
                    try:
                        app.cancel("parent", identity)
                    except Exception:
                        pass
                try:
                    app.close_collaboration()
                    app.shutdown()
                finally:
                    del app
        finally:
            upstream.shutdown()
            upstream.server_close()
            thread.join()


def main():
    args = arguments()
    previous_cwd = Path.cwd()
    try:
        with tempfile.TemporaryDirectory(prefix="velune-interop-") as directory:
            run(args, Path(directory).resolve())
    finally:
        os.chdir(previous_cwd)


if __name__ == "__main__":
    main()
