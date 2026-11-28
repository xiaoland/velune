"""Local ACP v1 peer for the generic client and model configuration checks."""
import json
from pathlib import Path
import sys
import time

mode = sys.argv[1]
root = Path(__file__).parent
session = "synthetic"
pending = None
count = 0
seen = set()
live_model = "model/a"
auto_values = {"model/a": False, "model/b": "default"}


def emit(**frame):
    print(json.dumps({"jsonrpc": "2.0", **frame}), flush=True)


def chunk(text, target=session):
    update = {"sessionUpdate": "agent_message_chunk", "content": {"type": "text", "text": text}}
    emit(method="session/update", params={"sessionId": target, "update": update})


def permission(rpc_id="p", target=session):
    emit(id=rpc_id, method="session/request_permission", params={
        "sessionId": target,
        "toolCall": {"toolCallId": "t", "title": "Synthetic tool"},
        "options": [{"optionId": "yes", "name": "Yes", "kind": "allow_once"}],
    })


def form(rpc_id="q", *, numeric=False):
    key, field = ("count", {"type": "integer", "minimum": 1}) if numeric else ("value", {"type": "string"})
    emit(id=rpc_id, method="elicitation/create", params={
        "sessionId": session, "mode": "form", "message": "Value?",
        "requestedSchema": {"type": "object", "required": [key], "properties": {key: field}},
    })


def options(model="model/a"):
    choices = [{"value": "model/a", "name": "A"}, {"value": "model/b", "name": "B"}]
    result = [{"id": "opaque/model-id", "name": "Model", "category": "model",
               "type": "select", "currentValue": model, "options": choices}]
    if mode == "model-auto-compaction":
        if model == "model/a":
            result.append({"id": "auto_compact_enabled", "name": "Automatically compact",
                           "type": "boolean", "currentValue": auto_values[model]})
        else:
            result.append({"id": "vendor.retention", "name": "Retention", "type": "select",
                           "currentValue": auto_values[model], "options": [
                               {"value": "default", "name": "Default"}, {"value": "less", "name": "Less"}]})
        result.append({"id": "opaque/safety", "name": "Permissions", "category": "_permissions",
                       "type": "boolean", "currentValue": False})
    return result


for line in sys.stdin:
    request = json.loads(line)
    rpc_id = request.get("id")
    method = request.get("method")
    params = request.get("params", {})

    if method == "initialize":
        assert params["clientCapabilities"]["elicitation"]["form"] == {}
        result = {"protocolVersion": 1, "authMethods": [], "agentCapabilities": {
            "loadSession": True, "sessionCapabilities": {"list": {}, "resume": {}, "close": {}, "delete": {}}}}
    elif method == "session/new":
        if mode == "history":
            chunk("first")
            chunk("ignored", "unknown")
            chunk(" second")
        result = {"sessionId": session, "configOptions": options()}
        if mode == "reject-open":
            result["configOptions"] += options()
        if mode == "idle-form":
            emit(id=rpc_id, result=result)
            form()
            continue
    elif method in ("session/load", "session/resume") and mode == "reject-open":
        chunk("unconfirmed history", params["sessionId"])
        emit(id=rpc_id, error={"code": -32000, "message": "not opened", "data": {"target": params["sessionId"]}})
        continue
    elif method == "session/load":
        chunk("authoritative replay", params["sessionId"])
        live_model = "model/b"
        result = {"configOptions": options(live_model)}
    elif method == "session/resume":
        result = {"configOptions": options()}
    elif method == "session/close":
        result = {}
    elif method == "session/set_config_option":
        config, value = params["configId"], params["value"]
        if mode == "model-auto-compaction":
            assert params["sessionId"] == session
            if config == "opaque/model-id":
                assert auto_values[live_model] == (False if live_model == "model/a" else "default")
                live_model = value
            else:
                if live_model == "model/a":
                    assert config == "auto_compact_enabled" and params["type"] == "boolean" and isinstance(value, bool)
                else:
                    assert config == "vendor.retention" and value in ("default", "less")
                auto_values[live_model] = value
            with (root / "setting-writes").open("a") as output:
                print(config, file=output)
            result = {"configOptions": options(live_model)}
        else:
            assert config == "opaque/model-id"
            if mode == "reject":
                emit(id=rpc_id, error={"code": -32000, "message": "synthetic rejected", "data": {"native": "kept"}})
                continue
            result = {"configOptions": options("model/b")}
            if mode == "accepted-disconnect":
                emit(id=rpc_id, result=result)
                emit(id="unsupported", method="synthetic/unsupported", params={})
                break
    elif method == "session/prompt":
        pending = rpc_id
        if mode == "model-auto-compaction":
            assert auto_values[live_model] == json.loads(params["prompt"][0]["text"])
            emit(id=rpc_id, result={"stopReason": "end_turn"})
        elif mode == "prompt-error":
            chunk("output before error")
            emit(id=rpc_id, error={"code": -32000, "message": "after execution", "data": {"toolRan": True}})
        elif mode == "disconnect":
            form()
            while not (root / "disconnect-now").exists():
                time.sleep(0.005)
            chunk("output before disconnect")
            break
        elif mode.startswith("duplicate-"):
            first, second = (permission, form) if mode == "duplicate-permission-first" else (form, permission)
            first("same")
            second("same")
        elif mode == "live":
            permission("foreign", "other")
            permission()
            form()
        elif mode == "form":
            count += 1
            form(numeric=True)
            if count == 2:
                emit(method="$/cancel_request", params={"requestId": "q"})
        else:
            raise AssertionError(f"unexpected prompt for {mode}")
        continue
    elif method == "session/cancel":
        if mode == "live":
            assert seen == {"p", "q", "foreign"}
            chunk("cancel delivered")
            emit(id=pending, result={"stopReason": "cancelled"})
        continue
    elif method is None:
        result = request["result"]
        if mode == "idle-form":
            assert rpc_id == "q" and result == {"action": "accept", "content": {"value": "still here"}}
            count += 1
            (root / "reply-count").write_text(str(count))
            chunk("idle answer delivered")
        elif mode.startswith("duplicate-"):
            (root / "duplicate-reply").write_text("unexpected response")
        elif mode == "live":
            assert rpc_id not in seen
            seen.add(rpc_id)
            if rpc_id == "foreign":
                assert result == {"outcome": {"outcome": "cancelled"}}
                continue
            if rpc_id == "p":
                assert result == {"outcome": {"outcome": "selected", "optionId": "yes"}}
            else:
                assert rpc_id == "q" and result == {"action": "accept", "content": {"value": "answer"}}
            chunk(rpc_id + " delivered")
        elif mode == "form":
            assert rpc_id == "q"
            if count == 1:
                assert result["content"]["count"] == 2
            else:
                assert result["action"] == "cancel"
            emit(id=pending, result={"stopReason": "end_turn"})
        else:
            raise AssertionError(f"unexpected client response for {mode}")
        continue
    else:
        raise AssertionError(f"unexpected method {method} for {mode}")
    emit(id=rpc_id, result=result)
