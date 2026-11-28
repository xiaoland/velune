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
attempts = config_dir / "attempts.jsonl"
attempt = str(uuid.uuid4())
rejection = config_dir / "reject-resume"
mode = rejection.read_text() if rejection.is_file() else None


def emit(event):
    print(json.dumps(event), flush=True)


def append(path, value):
    with path.open("a") as stream:
        stream.write(json.dumps(value) + "\n")


append(attempts, {"event": "launch", "attempt": attempt, "resume": transcript is not None})
if transcript and mode == "exit":
    append(attempts, {"event": "reject_exit", "attempt": attempt})
    sys.exit(1)

previous = []
if transcript:
    previous = [json.loads(line) for line in transcript.read_text().splitlines()]
    assert previous[0]["sessionId"] == transcript.stem
    assert "--no-session-persistence" not in args

first_frame = True
for line in sys.stdin:
    frame = json.loads(line)
    if first_frame:
        append(attempts, {"event": "first_frame", "attempt": attempt, "frame": frame})
        first_frame = False
    if transcript and mode == "initialize":
        assert frame["type"] == "control_request"
        assert frame["request"]["subtype"] == "initialize"
        append(attempts, {"event": "reject_initialize", "attempt": attempt})
        emit({"type": "control_response", "response": {
            "subtype": "error", "request_id": frame["request_id"],
            "error": "synthetic resume rejection",
        }})
        sys.exit(0)
    if frame["type"] == "control_request":
        assert frame["request"]["subtype"] == "initialize"
        emit({"type": "control_response", "response": {
            "subtype": "success", "request_id": frame["request_id"],
        }})
        continue

    text = frame["message"]["content"][0]["text"]
    append(trace, {
        "attempt": attempt, "cwd": os.getcwd(), "input": text,
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
