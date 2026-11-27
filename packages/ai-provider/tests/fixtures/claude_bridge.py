"""Claude stream-json fixture with a transcript separate from the caller's input."""
import json
from pathlib import Path
import os
import sys
import time
import uuid

args = sys.argv[1:]
transcript = Path(args[args.index("--resume") + 1]) if "--resume" in args else None
config_dir = Path(os.environ["CLAUDE_CONFIG_DIR"])
trace = config_dir / "fixture.jsonl"
if transcript and (config_dir / "reject-resume").is_file():
    sys.exit(1)
previous = []
if transcript:
    previous = [json.loads(line) for line in transcript.read_text().splitlines()]
    assert previous[0]["sessionId"] == transcript.stem
    assert "--no-session-persistence" not in args


def emit(event):
    print(json.dumps(event), flush=True)


def append(path, value):
    with path.open("a") as stream:
        stream.write(json.dumps(value) + "\n")


for line in sys.stdin:
    frame = json.loads(line)
    if frame["type"] == "control_request":
        assert frame["request"]["subtype"] == "initialize"
        emit({"type": "control_response", "response": {
            "subtype": "success", "request_id": frame["request_id"],
        }})
        continue

    text = frame["message"]["content"][0]["text"]
    append(trace, {
        "cwd": os.getcwd(), "input": text,
        "history": [item["message"]["content"][0]["text"] for item in previous
                    if item["type"] in ("user", "assistant")],
    })
    user = {"type": "user", "uuid": str(uuid.uuid4()), "message": frame["message"]}
    previous.append(user)
    if transcript:
        append(transcript, user)
    if text == "terminate before result":
        sys.exit(1)
    assistant = {"type": "assistant", "uuid": str(uuid.uuid4()), "message": {
        "role": "assistant", "content": [{"type": "text", "text": "fixture answer"}],
    }}
    emit(assistant)
    emit({"type": "result", "subtype": "success", "usage": {}})
    # The real CLI can report result before the transcript has been flushed.
    if transcript:
        time.sleep(0.03)
        append(transcript, assistant)
    previous.append(assistant)
