"""Account-free ACP peer. Wire records acknowledge reads, never elapsed sleeps."""
import json
import pathlib
import sys

MODE = None
ROOT = None
root = pathlib.Path(ROOT)
if "--version" in sys.argv:
    print("0.2.0-rc.2")
    sys.exit(0)


def log(value):
    with (root / "wire.jsonl").open("a") as stream:
        stream.write(json.dumps(value) + "\n")


if "--dump-config" in sys.argv:
    log({"configDump": sys.argv})
    print((root / "composed.yml").read_text())
    sys.exit(0)

launch = {"launch": sys.argv}
for index, argument in enumerate(sys.argv[:-1]):
    if argument == "--patch":
        path = pathlib.Path(sys.argv[index + 1])
        if path.name == "compaction.patch.yml":
            launch["compactionPatch"] = path.read_text()
        else:
            launch.setdefault("patch", json.loads(path.read_text()))
log(launch)
catalog = [{"id": "model", "currentValue": '[ "provider", "model" ]', "options": [
    {"options": [{"value": '[ "provider", "model" ]', "name": "Model"},
                 {"value": '[ "provider", "other" ]', "name": "Other"}]}]}]
turn = None


def emit(value):
    print(json.dumps(dict(jsonrpc="2.0", **value)), flush=True)


def result(request, value):
    emit({"id": request["id"], "result": value})


def update(text, session="session"):
    emit({"method": "session/update", "params": {"sessionId": session, "update": {
        "sessionUpdate": "agent_message_chunk", "content": {"text": text}}}})


def permission(session, identity):
    emit({"id": identity, "method": "session/request_permission", "params": {
        "sessionId": session, "toolCall": {"title": "Synthetic tool"},
        "options": [{"optionId": "once", "name": "Allow", "kind": "allow_once"}]}})


for line in sys.stdin:
    request = json.loads(line)
    log(request)
    method = request.get("method")
    if method == "initialize":
        handshake = root / "handshake.json"
        value = json.loads(handshake.read_text()) if handshake.exists() else {
            "protocolVersion": 1, "agentCapabilities": {
                "sessionCapabilities": {"resume": {}}, "mcpCapabilities": {"http": True}},
            "agentInfo": {"name": "synthetic-plugin", "version": "999.0.0"}}
        result(request, value)
    elif method in ("session/new", "session/resume"):
        if MODE in ("early", "resume-reject"):
            permission("wrong", "early-wrong")
            permission("session", "early-session")
            update("Wrong session must stay invisible", "wrong")
            update("Before identity response")
        if MODE == "reject" or (MODE == "resume-reject" and method == "session/resume"):
            emit({"id": request["id"], "error": {"code": -32602,
                  "message": "Synthetic rejection", "data": {"source": "fixture"}}})
            continue
        result(request, {"sessionId": request["params"].get("sessionId", "session"),
                         "configOptions": catalog})
    elif method == "session/set_config_option":
        catalog[0]["currentValue"] = request["params"]["value"]
        result(request, {"configOptions": catalog})
    elif method == "session/close":
        result(request, {})
    elif method == "session/prompt":
        turn = request["id"]
        update("Streaming")
        if MODE in ("late", "turn-error"):
            emit({"id": "unsupported", "method": "elicitation/create",
                  "params": {"sessionId": "session"}})
            permission("wrong", "wrong-permission")
            permission("session", "permission")
        elif MODE != "cancel":
            result(request, {"stopReason": "end_turn"})
    elif "result" in request and request["id"] == "permission" and turn:
        if MODE == "turn-error":
            emit({"id": turn, "error": {"code": -32603, "message": "Late tool failure"}})
        else:
            emit({"id": turn, "result": {"stopReason": "end_turn"}})
    elif method == "session/cancel" and turn:
        emit({"id": turn, "result": {"stopReason": "cancelled"}})
