"""Replay the selected Magpie frames over ACP stdio and use the local MCP server."""

import http.client
import json
import sys
from pathlib import Path
from urllib.parse import urlsplit

fixture_dir = Path(__file__).parent
with (fixture_dir / "wire.json").open(encoding="utf-8") as data:
    wire = json.load(data)

account, scenario, trace_file = sys.argv[1:]
session = wire[account]
session_id = session["sessionId"]
mcp_server = None


def send(**fields):
    line = json.dumps({"jsonrpc": "2.0", **fields}, ensure_ascii=False)
    sys.stdout.buffer.write(line.encode("utf-8") + b"\n")
    sys.stdout.buffer.flush()


def chunk(text):
    template = wire["message_chunk"]
    send(
        method="session/update",
        params={
            "sessionId": session_id,
            "update": {
                **template,
                "content": {**template["content"], "text": text},
            },
        },
    )


def mcp(method, params, request_id=None):
    address = urlsplit(mcp_server["url"])
    assert address.scheme == "http" and address.hostname == "127.0.0.1"
    assert mcp_server["type"] == "http"
    headers = {header["name"]: header["value"] for header in mcp_server["headers"]}
    assert any(key.lower() == "authorization" for key in headers)
    headers.update(
        {
            "Content-Type": "application/json",
            "Accept": "application/json, text/event-stream",
        }
    )
    body = {"jsonrpc": "2.0", "method": method, "params": params}
    if request_id is not None:
        body["id"] = request_id
    connection = http.client.HTTPConnection(address.hostname, address.port, timeout=10)
    try:
        connection.request("POST", address.path, json.dumps(body), headers=headers)
        response = connection.getresponse()
        contents = response.read()
        if method == "notifications/initialized":
            assert response.status in (200, 202, 204), (response.status, contents)
            return None
        assert response.status == 200, (response.status, contents)
        reply = json.loads(contents)
        assert reply["id"] == request_id and "error" not in reply, reply
        return reply["result"]
    finally:
        connection.close()


def invoke_echo():
    mcp(
        "initialize",
        {
            "protocolVersion": "2025-03-26",
            "capabilities": {},
            "clientInfo": {"name": "fake-agy", "version": "1.5.3"},
        },
        1,
    )
    mcp("notifications/initialized", {})
    tools = mcp("tools/list", {}, 2)["tools"]
    assert [tool["name"] for tool in tools] == ["echo"], tools
    result = mcp("tools/call", wire["tool_call"], 3)
    assert not result.get("isError", False), result
    return result["content"][0]["text"]


for line in sys.stdin:
    msg = json.loads(line)
    request_id = msg.get("id")
    method = msg["method"]
    params = msg.get("params") or {}

    # Retain only the method and model selection. There are no tokens in the trace.
    trace = {"method": method}
    if method == "session/set_config_option":
        trace.update(configId=params["configId"], value=params["value"])
    elif method == "session/set_model":
        trace["modelId"] = params["modelId"]
    with open(trace_file, "a", encoding="utf-8") as log:
        print(json.dumps(trace), file=log)

    if method == "initialize":
        if scenario == "startup_exit":
            raise SystemExit(2)
        send(id=request_id, result=wire["initialize"])
    elif method == "session/new":
        if scenario == "auth":
            send(id=request_id, error=wire["authentication_required"])
            continue
        if scenario == "tool":
            (mcp_server,) = params["mcpServers"]
        send(id=request_id, result=session)
    elif method == "session/set_config_option":
        (model,) = session["configOptions"]
        assert params["configId"] == model["id"]
        assert params["value"] in {option["value"] for option in model["options"]}
        model["currentValue"] = params["value"]
        send(id=request_id, result={"configOptions": session["configOptions"]})
    elif method == "session/set_model":
        models = session["models"]
        assert params["modelId"] in {item["modelId"] for item in models["availableModels"]}
        models["currentModelId"] = params["modelId"]
        send(id=request_id, result={})
    elif method == "session/prompt":
        assert params["sessionId"] == session_id
        assert params["prompt"] == [{"type": "text", "text": "fixture prompt"}]
        if scenario == "prompt_exit":
            raise SystemExit(2)
        if scenario == "tool":
            chunk("After tools. " + invoke_echo())
        else:
            chunk(account + " " + wire["message_chunk"]["content"]["text"])
        send(id=request_id, result=wire["end_turn"])
    elif method == "session/cancel":
        if request_id is not None:
            send(id=request_id, result={})
    elif request_id is not None:
        send(id=request_id, error={"code": -32601, "message": "Method not found"})
