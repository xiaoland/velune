"""Export a Codex rollout's effective Responses context, not display events."""
import json
import sys

from pathlib import Path
import uuid

def option(name):
    return sys.argv[sys.argv.index(name) + 1] if name in sys.argv else None

def parse_records(data):
    return [json.loads(line) for line in data.splitlines() if line.strip()]

def resolve_records(path, data, resolving):
    records = parse_records(data)
    metadata = next((record["payload"] for record in records if record["type"] == "session_meta"), {})
    base = metadata.get("history_base")
    if not base:
        return records
    thread = str(uuid.UUID(base["thread_id"]))
    size, ordinal = base["end_byte_offset"], base["end_ordinal_exclusive"]
    if not isinstance(size, int) or size < 0 or not isinstance(ordinal, int) or ordinal < 0:
        raise ValueError("invalid Codex history base")
    home = option("--home")
    if home is None:
        raise ValueError("Codex history lineage requires its configured HOME")
    candidates = []
    for root in (Path(home) / "sessions", Path(home) / "archived_sessions"):
        for candidate in root.rglob("*" + thread + "*.jsonl"):
            candidate = candidate.resolve()
            if candidate == path or (str(candidate), size) in resolving:
                continue
            with candidate.open("rb") as source:
                prefix = source.read(size)
            if len(prefix) != size or (prefix and not prefix.endswith(b"\n")):
                continue
            try:
                prior = parse_records(prefix)
            except (ValueError, UnicodeDecodeError):
                continue
            meta = next((r["payload"] for r in prior if r["type"] == "session_meta"), {})
            if not prior or meta.get("id", meta.get("session_id")) != thread or prior[-1].get("ordinal") != ordinal - 1:
                continue
            complete = resolve_records(candidate, prefix, resolving | {(str(candidate), size)})
            if not candidates or complete != candidates[0]:
                candidates.append(complete)
    if len(candidates) != 1:
        raise ValueError("Codex history base is missing or ambiguous")
    return candidates[0] + records

portable_summary = None
if "--before-message-id" in sys.argv:
    source_path = option("--source-path")
    source = sys.stdin.buffer.read()
    legacy_boundary = None
    legacy_boundaries = {}
    if "--request-envelope" in sys.argv:
        request = json.loads(source)
        source = request["rollout"].encode("utf-8")
        legacy_boundary = request.get("legacyBoundary")
        legacy_boundaries = request.get("legacyBoundaries", {})
    records = resolve_records(Path(source_path).resolve(), source, {(str(Path(source_path).resolve()), len(source))}) if source_path else parse_records(source)
    active = []
    for record in records:
        if record["type"] == "event_msg" and record["payload"].get("type") == "thread_rolled_back":
            count = record["payload"].get("num_turns")
            starts = [i for i, prior in enumerate(active) if prior["type"] == "event_msg" and prior["payload"].get("type") == "task_started"]
            if not isinstance(count, int) or count < 0 or count > len(starts):
                raise ValueError("Codex rollback boundary could not be resolved")
            if count:
                active = active[:starts[-count]]
        else:
            active.append(record)
    records = active
    def user_content(content):
        if isinstance(content, str):
            content = [{"type":"text", "text":content}]
        normalized = []
        for block in content:
            kind = block["type"]
            if kind in ("text", "input_text"):
                normalized.append(("text", block["text"]))
            elif kind in ("image", "input_image"):
                url = block.get("image_url", block.get("url"))
                if not isinstance(url, str):
                    raise ValueError("Codex legacy image input cannot be associated")
                normalized.append(("image", url))
            else:
                raise ValueError("Codex legacy input cannot be associated: " + kind)
        return normalized

    before = option("--before-message-id")
    legacy_boundary = legacy_boundaries.get(before, legacy_boundary)
    # Imported messages share their response/display identity. Native prompts
    # have a separate UserMessage event identity immediately after the input.
    event = next((i for i, record in enumerate(records)
                  if record["type"] == "event_msg" and record["payload"].get("type") == "item_completed"
                  and record["payload"].get("item", {}).get("type") == "UserMessage"
                  and record["payload"]["item"].get("id") == before), None)
    direct = next((i for i, record in enumerate(records)
                   if record["type"] == "response_item" and record["payload"].get("type") == "message"
                   and record["payload"].get("role") == "user" and record["payload"].get("id") == before), None)
    if direct is not None:
        boundary = direct
    elif event is not None:
        boundary = event - 1
        while boundary >= 0 and records[boundary]["type"] != "response_item":
            boundary -= 1
        if boundary < 0 or records[boundary]["payload"].get("type") != "message" or records[boundary]["payload"].get("role") != "user":
            raise ValueError("Codex user event has no corresponding native input")
    elif legacy_boundary is not None:
        turn = legacy_boundary["turnId"]
        starts = [i for i, record in enumerate(records)
                  if record["type"] == "event_msg" and record["payload"].get("type") == "task_started"
                  and record["payload"].get("turn_id") == turn]
        if not starts:
            starts = [i for i, record in enumerate(records)
                      if record["type"] == "turn_context" and record["payload"].get("turn_id") == turn]
        if not starts:
            raise ValueError("Codex legacy user turn was not found")
        start = starts[-1]
        end = next((i for i in range(start + 1, len(records))
                    if records[i]["type"] == "event_msg" and records[i]["payload"].get("type") == "task_started"), len(records))
        inputs = [i for i in range(start + 1, end) if records[i]["type"] == "response_item"
                  and records[i]["payload"].get("type") == "message" and records[i]["payload"].get("role") == "user"]
        users = legacy_boundary["users"]
        ordinal = legacy_boundary["ordinal"]
        if not isinstance(ordinal, int) or ordinal < 0 or ordinal >= len(users) or users[ordinal]["id"] != before:
            raise ValueError("Codex legacy user identity changed")
        expected = [user_content(item["content"]) for item in users]
        actual = [user_content(records[i]["payload"]["content"]) for i in inputs]
        # Canonical native turn order establishes identity. Content only
        # validates the association; it never selects a duplicate by text.
        if len(expected) != len(actual) or expected != actual:
            raise ValueError("Codex legacy input identity cannot be associated exactly")
        boundary = inputs[ordinal]
    else:
        raise ValueError("Codex native user message boundary was not found")
    records = records[:boundary]
else:
    data = sys.stdin.buffer.read()
    if "--request-envelope" in sys.argv:
        request = json.loads(data)
        portable_summary = request.get("portableSummary")
        data = request["rollout"].encode("utf-8")
    records = parse_records(data)
# The newest full replacement already includes inherited and retained context.
# Older records are no longer model input, including earlier compactions.
start = next((index for index in range(len(records) - 1, -1, -1)
              if records[index]["type"] == "compacted"
              and records[index]["payload"].get("replacement_history") is not None), 0)
if "--before-message-id" not in sys.argv and not start and any(record["type"] == "session_meta" and record["payload"].get("history_base")
                     for record in records):
    raise ValueError("Codex inherited context without a complete compact replacement requires native lineage resolution")
items = []
for record in records[start:]:
    kind, payload = record["type"], record["payload"]
    if kind == "response_item":
        items.append(payload)
    elif kind == "compacted":
        replacement = payload.get("replacement_history")
        if replacement is None:
            raise ValueError("legacy compaction without replacement history cannot be translated reliably")
        items = replacement.copy()
    elif kind == "history_base" or (kind == "event_msg" and payload.get("type") == "thread_rolled_back"):
        raise ValueError("Codex inherited or reverted context requires native history resolution")
    elif kind == "inter_agent_communication_metadata":
        # Codex persists only the delivery/wake flag here. Model-visible
        # communication is persisted separately as a response_item; native
        # rollout reconstruction also skips this metadata. Never turn it into
        # a user prompt or relax rejection of unknown context-bearing records.
        continue
    elif kind not in ("session_meta", "turn_context", "event_msg", "world_state", "token_usage_record"):
        raise ValueError("unsupported Codex rollout record: " + kind)

def encrypted_agent_block(block):
    return (isinstance(block, dict) and block.get("type") == "encrypted_content"
            and set(block) == {"type", "encrypted_content"}
            and isinstance(block["encrypted_content"], str) and bool(block["encrypted_content"]))

encrypted_agent_history = any(
    item.get("type") == "agent_message" and isinstance(item.get("content"), list)
    and any(encrypted_agent_block(block) for block in item["content"])
    for item in items
)
if "--requires-portable-summary" in sys.argv:
    print(json.dumps({"required": encrypted_agent_history}))
    sys.exit(0)
if portable_summary is not None and not encrypted_agent_history:
    raise ValueError("portable summary requires encrypted Codex context")

def blocks(content):
    if isinstance(content, str):
        return [{"type": "text", "text": content}]
    result = []
    for block in content:
        kind = block["type"]
        if kind in ("input_text", "output_text", "text"):
            result.append({"type": "text", "text": block["text"]})
        elif kind in ("input_image", "image"):
            url = block["image_url"]
            if not url.startswith("data:") or ";base64," not in url:
                raise ValueError("non-embedded Codex image cannot be translated")
            media, data = url[5:].split(";base64,", 1)
            result.append({"type": "image", "media_type": media, "data": data})
        else:
            raise ValueError("unsupported Codex context content: " + kind)
    return result

messages = []
for item in items:
    kind = item["type"]
    if kind == "message":
        if item["role"] in ("system", "developer"):
            continue
        role, content = item["role"], blocks(item["content"])
    elif kind == "agent_message":
        # The portable context has no native inter-agent role. Keep plaintext
        # communication as attributed assistant context, not a human prompt.
        # Encrypted content cannot be replaced by its visible envelope alone.
        author, recipient = item["author"], item["recipient"]
        if not isinstance(author, str) or not author or not isinstance(recipient, str) or not recipient:
            raise ValueError("Codex agent message lacks author or recipient")
        if not isinstance(item.get("content"), list):
            raise ValueError("unsupported Codex agent message content")
        encrypted = False
        for block in item["content"]:
            if encrypted_agent_block(block):
                encrypted = True
                continue
            if block["type"] != "input_text" or not isinstance(block.get("text"), str):
                raise ValueError("unsupported Codex agent message content: " + block["type"])
        if encrypted:
            continue
        role = "assistant"
        content = [{"type": "text", "text": f"Agent message from {author} to {recipient}:\n"}] + blocks(item["content"])
    elif kind in ("function_call", "custom_tool_call"):
        arguments = json.loads(item["arguments"]) if kind == "function_call" else {"input": item["input"]}
        role, content = "assistant", [{"type": "tool_call", "id": item["call_id"], "name": item["name"], "arguments": arguments}]
    elif kind in ("function_call_output", "custom_tool_call_output"):
        output = item["output"]
        if isinstance(output, dict):
            output = output["content"]
        role, content = "user", [{"type": "tool_result", "id": item["call_id"], "content": blocks(output), "is_error": False}]
    elif kind == "configuration_update":
        # Codex persists a positional reasoning-effort control for its backend.
        # The target runtime gets its reasoning effort from the new request.
        reasoning = item.get("reasoning")
        if (set(item) != {"type", "reasoning"} or not isinstance(reasoning, dict)
                or set(reasoning) != {"effort"} or not isinstance(reasoning["effort"], str)
                or not reasoning["effort"].strip()):
            raise ValueError("unsupported Codex configuration update")
        continue
    elif kind in ("reasoning", "additional_tools"):
        # Tool availability belongs to the target runtime, not historical calls.
        continue
    else:
        raise ValueError("unsupported Codex context item: " + kind)
    if content:
        messages.append({"role": role, "content": content})
if encrypted_agent_history:
    summary = portable_summary.strip() if isinstance(portable_summary, str) else ""
    if not summary:
        print(json.dumps({"errorCode":"portable_summary_required"}))
        sys.exit(0)
    # The source fork summarized the whole effective history, including the
    # opaque deliveries. A routing envelope cannot replace their actual text.
    messages = [{"role": "assistant", "content": [{"type": "text", "text": summary}]}]
print(json.dumps({"messages": messages}))
