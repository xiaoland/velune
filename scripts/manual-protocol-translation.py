#!/usr/bin/env python3
"""Manual six-direction protocol translation acceptance.

This is an explicit loopback probe, never a CI entry point. It compiles a tiny
Rust process that starts the real velune-gateway Runner, then sends synthetic
Chat Completions, Responses, and Anthropic Messages requests through it. No
credentials, user configuration, or real provider is read.
"""
import argparse
import http.client
import json
from pathlib import Path
import subprocess
import tempfile
import threading
from http.server import BaseHTTPRequestHandler, ThreadingHTTPServer

PROBE = r'''
use std::{collections::BTreeMap,io::{BufRead,Write},sync::Arc};
struct Resolver;
impl velune_gateway::CredentialResolver for Resolver {
 fn resolve(&self,_:String,_:velune_gateway::CredentialTarget)->velune_ai::OperationFuture<Result<velune_gateway::ResolvedCredential,velune_gateway::CredentialResolutionError>> {
  Box::pin(async {Ok(velune_gateway::ResolvedCredential {token:"synthetic-only".into(),explicit_output_cap:None,subscription:false})})
 }
}
fn main()->Result<(),Box<dyn std::error::Error>> {
 let a:Vec<_>=std::env::args().collect(); let c=serde_json::from_slice(&std::fs::read(&a[1])?)?;
 let r=velune_gateway::Runner::start(c,Arc::new(Resolver),BTreeMap::new())?;
 println!("{}",serde_json::json!({"endpoint":r.endpoint(),"token":r.token()}));std::io::stdout().flush()?;
 let _=std::io::stdin().lock().lines().next(); Ok(())
}
'''

PROTOCOL_PATH = {
    "chat": "/v1/chat/completions",
    "responses": "/v1/responses",
    "messages": "/v1/messages",
}


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--deps", required=True, type=Path)
    parser.add_argument("--rustc", required=True, type=Path)
    args = parser.parse_args()
    for path in (args.deps, args.rustc):
        if not path.is_absolute() or not path.exists():
            parser.error("absolute existing paths are required")
    captures = []
    cancel_seen = threading.Event()
    truncated_reads = []

    class Upstream(BaseHTTPRequestHandler):
        def log_message(self, *_):
            pass

        def do_POST(self):
            length = int(self.headers["Content-Length"])
            body = json.loads(self.rfile.read(length))
            captures.append({"path": self.path, "body": body})
            case = self.headers.get("X-Fixture-Case", "normal")
            if case == "error":
                self.send_response(422)
                self.send_header("Content-Type", "application/json")
                self.end_headers()
                self.wfile.write(json.dumps({"error": {"type": "synthetic_error", "vendor_field": "preserve"}}).encode())
                return
            if case == "cancel":
                self.send_response(200)
                self.send_header("Content-Type", "text/event-stream")
                self.end_headers()
                try:
                    self.wfile.write(b"event: response.created\ndata: {\"type\":\"response.created\",\"response\":{\"id\":\"cancel-1\",\"status\":\"in_progress\"}}\n\n")
                    self.wfile.flush()
                    while True:
                        self.wfile.write(b"event: response.output_text.delta\ndata: {\"type\":\"response.output_text.delta\",\"delta\":\"x\"}\n\n")
                        self.wfile.flush()
                except (BrokenPipeError, ConnectionResetError):
                    cancel_seen.set()
                    return
            if case == "truncate-before":
                self.send_response(200)
                self.send_header("Content-Type", "text/event-stream")
                self.end_headers()
                self.wfile.write(b"event: response.created\r\ndata: {\"type\":\"response.created\",\"response\":{\"id\":\"truncated\",\"status\":\"in_progress\"}}\r\n\r\n")
                self.wfile.flush()
                return
            serialized = json.dumps(body, ensure_ascii=False)
            second = any(marker in serialized for marker in ("function_call_output", "tool_result", '"role": "tool"'))
            if self.path.endswith("/chat/completions"):
                if second:
                    payload = ("data: " + json.dumps({"choices": [{"index": 0, "delta": {"content": "SYNTHETIC_FINAL 🌿"}, "finish_reason": None}]})
                               + "\n\ndata: " + json.dumps({"choices": [{"index": 0, "delta": {}, "finish_reason": "stop"}], "usage": {"prompt_tokens": 5, "completion_tokens": 4}})
                               + "\n\ndata: [DONE]\n\n").encode()
                else:
                    payload = (
                        "data: " + json.dumps({"choices": [{"index": 0, "delta": {
                            "role": "assistant", "tool_calls": [{"index": 0, "id": "call-1",
                            "type": "function", "function": {"name": "lookup", "arguments": '{"input":"x 🌿"}'}},
                            {"index": 1, "id": "call-2", "type": "function",
                             "function": {"name": "search", "arguments": '{"query":"x"}'}}]}}]})
                        + "\n\ndata: " + json.dumps({"choices": [{"index": 0, "delta": {}, "finish_reason": "tool_calls"}],
                            "usage": {"prompt_tokens": 3, "completion_tokens": 2}}) + "\n\ndata: [DONE]\n\n"
                    ).encode()
                content_type = "text/event-stream"
            elif self.path.endswith("/messages"):
                if second:
                    events = [("message_start", {"type": "message_start", "message": {"id": "msg-2", "type": "message", "role": "assistant", "content": [], "usage": {"input_tokens": 5, "output_tokens": 0}}}),
                              ("content_block_start", {"type": "content_block_start", "index": 0, "content_block": {"type": "text", "text": ""}}),
                              ("content_block_delta", {"type": "content_block_delta", "index": 0, "delta": {"type": "text_delta", "text": "SYNTHETIC_FINAL 🌿"}}),
                              ("message_delta", {"type": "message_delta", "delta": {"stop_reason": "end_turn"}, "usage": {"output_tokens": 4}}),
                              ("message_stop", {"type": "message_stop"})]
                else:
                    events = [
                        ("message_start", {"type": "message_start", "message": {"id": "msg-1", "type": "message", "role": "assistant", "content": [], "usage": {"input_tokens": 3, "output_tokens": 0}}}),
                        ("content_block_start", {"type": "content_block_start", "index": 0, "content_block": {"type": "tool_use", "id": "call-1", "name": "lookup", "input": {}}}),
                        ("content_block_delta", {"type": "content_block_delta", "index": 0, "delta": {"type": "input_json_delta", "partial_json": '{"input":"x 🌿"}'}}),
                        ("content_block_stop", {"type": "content_block_stop", "index": 0}),
                        ("content_block_start", {"type": "content_block_start", "index": 1, "content_block": {"type": "tool_use", "id": "call-2", "name": "search", "input": {}}}),
                        ("content_block_delta", {"type": "content_block_delta", "index": 1, "delta": {"type": "input_json_delta", "partial_json": '{"query":"x"}'}}),
                        ("content_block_stop", {"type": "content_block_stop", "index": 1}),
                        ("message_delta", {"type": "message_delta", "delta": {"stop_reason": "tool_use"}, "usage": {"output_tokens": 2}}),
                        ("message_stop", {"type": "message_stop"}),
                    ]
                payload = "".join("event: %s\ndata: %s\n\n" % (name, json.dumps(data)) for name, data in events).encode()
                content_type = "text/event-stream"
            else:
                output = [{"type": "message", "id": "item-2", "role": "assistant", "content": [{"type": "output_text", "text": "SYNTHETIC_FINAL 🌿"}]}] if second else [{"type": "function_call", "id": "item-1", "call_id": "call-1", "name": "lookup", "arguments": '{"input":"x 🌿"}'}, {"type": "function_call", "id": "item-2", "call_id": "call-2", "name": "search", "arguments": '{"query":"x"}'}]
                response = {"id": "resp-1", "object": "response", "status": "completed", "output": output, "usage": {"input_tokens": 3, "output_tokens": 2}, "vendor_unknown": {"keep": True}}
                created = {**response, "status": "in_progress", "output": []}
                events = [("response.created", {"type": "response.created", "response": created})]
                for index, item in enumerate(output):
                    events.append(("response.output_item.added", {"type": "response.output_item.added", "output_index": index, "item": {**item, "status": "in_progress"}}))
                    events.append(("response.output_item.done", {"type": "response.output_item.done", "output_index": index, "item": item}))
                events.append(("response.completed", {"type": "response.completed", "response": response}))
                payload = "".join("event: %s\ndata: %s\n\n" % (name, json.dumps(data)) for name, data in events).encode()
                content_type = "text/event-stream"
            if not body.get("stream", True):
                if self.path.endswith("/chat/completions"):
                    response = {"id": "json-1", "object": "chat.completion", "model": body.get("model", "fixture"), "choices": [{"index": 0, "message": {"role": "assistant", "tool_calls": [{"id": "call-1", "type": "function", "function": {"name": "lookup", "arguments": '{"input":"x 🌿"}'}}, {"id": "call-2", "type": "function", "function": {"name": "search", "arguments": '{"query":"x"}'}}]}, "finish_reason": "tool_calls"}], "usage": {"prompt_tokens": 3, "completion_tokens": 2}}
                    if second:
                        response["choices"][0] = {"index": 0, "message": {"role": "assistant", "content": "SYNTHETIC_FINAL 🌿"}, "finish_reason": "stop"}
                elif self.path.endswith("/messages"):
                    response = {"id": "json-1", "type": "message", "role": "assistant", "model": body.get("model", "fixture"), "content": [{"type": "tool_use", "id": "call-1", "name": "lookup", "input": {"input": "x 🌿"}}, {"type": "tool_use", "id": "call-2", "name": "search", "input": {"query": "x"}}], "stop_reason": "tool_use", "usage": {"input_tokens": 3, "output_tokens": 2}}
                    if second:
                        response["content"] = [{"type": "text", "text": "SYNTHETIC_FINAL 🌿"}]
                        response["stop_reason"] = "end_turn"
                else:
                    response = {"id": "json-1", "object": "response", "status": "completed", "model": body.get("model", "fixture"), "output": [{"type": "function_call", "id": "item-1", "call_id": "call-1", "name": "lookup", "arguments": '{"input":"x 🌿"}'}, {"type": "function_call", "id": "item-2", "call_id": "call-2", "name": "search", "arguments": '{"query":"x"}'}], "usage": {"input_tokens": 3, "output_tokens": 2}}
                    if second:
                        response["output"] = [{"type": "message", "id": "item-2", "role": "assistant", "content": [{"type": "output_text", "text": "SYNTHETIC_FINAL 🌿"}]}]
                payload = json.dumps(response, ensure_ascii=False).encode()
                content_type = "application/json"
            elif content_type == "text/event-stream":
                if case == "unknown-tail":
                    payload += b"event: vendor.unknown\ndata: {\"type\":\"vendor.unknown\",\"opaque\":true}\n\n"
                if case == "incomplete":
                    if self.path.endswith("/chat/completions"):
                        payload = b'data: {"choices":[{"index":0,"delta":{"content":"partial"},"finish_reason":"length"}],"usage":{"prompt_tokens":2,"completion_tokens":1}}\n\ndata: [DONE]\n\n'
                    elif self.path.endswith("/messages"):
                        payload = b'event: message_start\ndata: {"type":"message_start","message":{"id":"incomplete","type":"message","role":"assistant","content":[],"usage":{"input_tokens":2,"output_tokens":1}}}\n\nevent: message_delta\ndata: {"type":"message_delta","delta":{"stop_reason":"max_tokens"},"usage":{"output_tokens":1}}\n\nevent: message_stop\ndata: {"type":"message_stop"}\n\n'
                    else:
                        payload = b'event: response.incomplete\ndata: {"type":"response.incomplete","response":{"id":"incomplete","status":"incomplete","incomplete_details":{"reason":"max_output_tokens"},"output":[],"usage":{"input_tokens":2,"output_tokens":1}}}\n\n'
                payload = payload.replace(b"\\ud83c\\udf3f", "🌿".encode()).replace(b"\n", b"\r\n")
            self.send_response(200)
            self.send_header("Content-Type", content_type)
            self.send_header("X-Request-ID", "synthetic-translation")
            self.end_headers()
            if content_type == "text/event-stream":
                for offset in range(0, len(payload), 7):
                    self.wfile.write(payload[offset:offset + 7])
                    self.wfile.flush()
            else:
                self.wfile.write(payload)
                self.wfile.flush()

    server = ThreadingHTTPServer(("127.0.0.1", 0), Upstream)
    threading.Thread(target=server.serve_forever, daemon=True).start()
    with tempfile.TemporaryDirectory(prefix="velune-protocol-translation-") as directory:
        root = Path(directory)
        source = root / "probe.rs"
        source.write_text(PROBE)
        config = root / "gateway.json"
        providers = []
        for target in ("chat", "responses", "messages"):
            protocol = {"chat": "chatCompletionsV1", "responses": "responsesV1", "messages": "messagesV1"}[target]
            providers.append({"id": target, "name": target, "protocol": protocol,
                              "endpoint": f"http://127.0.0.1:{server.server_port}/v1",
                              "credentialRef": "synthetic", "models": ([{"recordKey": target, "providerModelId": "fixture-" + target, "nickname": target, "maxOutputTokens": 128}, {"recordKey": "messages-nocap", "providerModelId": "fixture-messages-nocap", "nickname": "messages-nocap"}] if target == "messages" else [{"recordKey": target, "providerModelId": "fixture-" + target, "nickname": target}])})
        config.write_text(json.dumps({"id": "translation", "name": "translation", "providers": providers, "failover": {"mode": "disabled"}}))
        build = subprocess.run(["cargo", "build", "--locked", "-p", "velune-gateway", "--message-format=json"], capture_output=True, text=True, check=True)
        artifacts = {}
        for line in build.stdout.splitlines():
            item = json.loads(line)
            if item.get("reason") == "compiler-artifact":
                for filename in item["filenames"]:
                    if filename.endswith(".rlib"):
                        artifacts[item["target"]["name"]] = filename
        binary = root / "gateway-probe"
        subprocess.run([str(args.rustc), "--edition=2024", str(source), "-L", f"dependency={args.deps}",
                        "--extern", f"velune_gateway={artifacts['velune_gateway']}",
                        "--extern", f"velune_ai={artifacts['velune_ai']}",
                        "--extern", f"serde_json={artifacts['serde_json']}", "-o", str(binary)], check=True)
        process = subprocess.Popen([str(binary), str(config)], stdin=subprocess.PIPE, stdout=subprocess.PIPE, text=True)
        try:
            info = json.loads(process.stdout.readline())
            port = int(info["endpoint"].rsplit(":", 1)[1].split("/", 1)[0])
            directions = [(source_name, target) for source_name in PROTOCOL_PATH for target in ("chat", "responses", "messages") if source_name != target]
            def request_body(source_name, target, second=False):
                tools = [{"type": "function", "function": {"name": "lookup", "parameters": {"type": "object"}}}, {"type": "function", "function": {"name": "search", "parameters": {"type": "object"}}}]
                body = {"model": "velune/model/" + target, "stream": True, "max_tokens": 128, "messages": [{"role": "user", "content": "synthetic 🌿"}], "tools": tools, "metadata": {"fixture_case": "unknown", "state_ref": "synthetic-response"}}
                if second:
                    body["messages"] = [{"role": "assistant", "tool_calls": [{"id": "call-1", "type": "function", "function": {"name": "lookup", "arguments": '{"input":"x 🌿"}'}}, {"id": "call-2", "type": "function", "function": {"name": "search", "arguments": '{"query":"x"}'}}]}, {"role": "tool", "tool_call_id": "call-1", "content": "SYNTHETIC_TOOL_RESULT"}, {"role": "tool", "tool_call_id": "call-2", "content": "SYNTHETIC_SEARCH_RESULT"}]
                if source_name == "responses":
                    body = {"model": body["model"], "stream": True, "input": [{"type": "function_call", "call_id": "call-1", "name": "lookup", "arguments": '{"input":"x 🌿"}'}, {"type": "function_call_output", "call_id": "call-1", "output": "SYNTHETIC_TOOL_RESULT"}, {"type": "function_call", "call_id": "call-2", "name": "search", "arguments": '{"query":"x"}'}, {"type": "function_call_output", "call_id": "call-2", "output": "SYNTHETIC_SEARCH_RESULT"}] if second else [{"type": "message", "role": "user", "content": [{"type": "input_text", "text": "synthetic 🌿"}]}], "tools": [{"type": "function", "name": "lookup", "parameters": {"type": "object"}}, {"type": "function", "name": "search", "parameters": {"type": "object"}}], "previous_response_id": "resp-previous", "metadata": {"fixture_case": "unknown", "state_ref": "synthetic-response"}}
                if source_name == "messages":
                    body = {"model": body["model"], "max_tokens": 128, "stream": True, "messages": [{"role": "assistant", "content": [{"type": "tool_use", "id": "call-1", "name": "lookup", "input": {"input": "x 🌿"}}, {"type": "tool_use", "id": "call-2", "name": "search", "input": {"query": "x"}}]}, {"role": "user", "content": [{"type": "tool_result", "tool_use_id": "call-1", "content": "SYNTHETIC_TOOL_RESULT"}, {"type": "tool_result", "tool_use_id": "call-2", "content": "SYNTHETIC_SEARCH_RESULT"}]}] if second else [{"role": "user", "content": "synthetic 🌿"}], "tools": [{"name": "lookup", "input_schema": {"type": "object"}}, {"name": "search", "input_schema": {"type": "object"}}], "metadata": {"fixture_case": "unknown", "state_ref": "synthetic-response"}}
                return body
            def send_json(source_name, body, fixture_case="normal", fragmented=False):
                raw = json.dumps(body, ensure_ascii=False).encode()
                connection = http.client.HTTPConnection("127.0.0.1", port, timeout=8)
                headers = {"Authorization": "Bearer " + info["token"], "Content-Type": "application/json", "X-Fixture-Case": fixture_case}
                if not fragmented:
                    connection.request("POST", PROTOCOL_PATH[source_name], raw, headers)
                else:
                    connection.putrequest("POST", PROTOCOL_PATH[source_name])
                    for name, value in headers.items():
                        connection.putheader(name, value)
                    connection.putheader("Transfer-Encoding", "chunked")
                    connection.endheaders()
                    split = raw.find("🌿".encode()) + len("🌿".encode()) // 2
                    for chunk in (raw[:split], raw[split:]):
                        connection.send((hex(len(chunk))[2:] + "\r\n").encode() + chunk + b"\r\n")
                    connection.send(b"0\r\n\r\n")
                response = connection.getresponse()
                try:
                    payload = response.read()
                except http.client.IncompleteRead as error:
                    if fixture_case != "truncate-before":
                        raise
                    truncated_reads.append(fixture_case)
                    payload = error.partial
                connection.close()
                return response.status, payload
            for source_name, target in directions:
                body = request_body(source_name, target)
                status, payload = send_json(source_name, body, fragmented=(source_name == "chat"))
                assert status == 200, (source_name, target, status, payload[:300])
                assert payload and b"call-1" in payload and b"call-2" in payload and b"\xf0\x9f\x8c\xbf" in payload, (source_name, target, payload[:300])
                body = request_body(source_name, target, second=True)
                status, payload = send_json(source_name, body)
                assert status == 200 and b"SYNTHETIC_FINAL" in payload and b"\xf0\x9f\x8c\xbf" in payload, (source_name, target, payload[:300])
            for source_name, target in directions:
                body = request_body(source_name, target)
                body["stream"] = False
                status, payload = send_json(source_name, body)
                assert status == 200 and b"call-1" in payload and b"\xf0\x9f\x8c\xbf" in payload, ("json", source_name, target, status, payload[:300])
                body = request_body(source_name, target, second=True)
                body["stream"] = False
                status, payload = send_json(source_name, body)
                assert status == 200 and b"SYNTHETIC_FINAL" in payload and b"usage" in payload and b"\xf0\x9f\x8c\xbf" in payload, ("json", source_name, target, status, payload[:300])
            status, payload = send_json("chat", request_body("chat", "responses"), fixture_case="unknown-tail")
            assert status == 200 and b"call-1" in payload, ("unknown-tail", status, payload[:300])
            status, payload = send_json("chat", request_body("chat", "responses"), fixture_case="incomplete")
            assert status == 200 and (b"length" in payload or b"max_tokens" in payload), ("incomplete", status, payload[:300])
            status, payload = send_json("chat", request_body("chat", "responses"), fixture_case="truncate-before")
            assert status == 200 and b"response.completed" not in payload and b"[DONE]" not in payload, ("truncate-before", status, payload[:300])
            assert truncated_reads == ["truncate-before"], truncated_reads
            nocap_body = request_body("chat", "messages-nocap")
            nocap_body.pop("max_tokens", None)
            status, payload = send_json("chat", nocap_body)
            assert status == 400 and b"max_tokens_required" in payload, (status, payload[:300])
            status, payload = send_json("chat", request_body("chat", "responses"), fixture_case="error")
            assert status == 422 and b"synthetic_error" in payload and b"vendor_field" in payload, (status, payload[:300])
            cancel_connection = http.client.HTTPConnection("127.0.0.1", port, timeout=3)
            raw = json.dumps(request_body("chat", "responses"), ensure_ascii=False).encode()
            cancel_connection.request("POST", PROTOCOL_PATH["chat"], raw, {"Authorization": "Bearer " + info["token"], "Content-Type": "application/json", "X-Fixture-Case": "cancel"})
            cancel_response = cancel_connection.getresponse()
            cancel_response.read(128)
            cancel_connection.close()
            assert cancel_seen.wait(3), "cancel did not reach synthetic upstream"
            print(json.dumps({"acceptance": "PASSED", "directions": len(directions), "jsonDirections": len(directions), "upstreamRequests": len(captures), "twoTurnToolResults": True, "parallelToolFixture": True, "unknownFieldsAndStateRefsSent": True, "utf8FragmentFixture": True, "sseCrLfAndByteFragments": True, "jsonUsageAndFinish": True, "errorPreserved": True, "cancelObserved": True, "incompleteTerminal": True, "truncationBoundary": True, "unknownTailBestEffort": True}))
        finally:
            process.stdin.write("stop\n")
            process.stdin.flush()
            process.wait(timeout=5)
    server.shutdown()
    server.server_close()


if __name__ == "__main__":
    main()
