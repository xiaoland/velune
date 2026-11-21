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

source_path = option("--source-path")
source = sys.stdin.buffer.read()
legacy_boundary = None
legacy_boundaries = {}
legacy_turns = {}
portable_summary = None
if "--request-envelope" in sys.argv:
    request = json.loads(source)
    source = request["rollout"].encode("utf-8")
    legacy_boundary = request.get("legacyBoundary")
    legacy_boundaries = request.get("legacyBoundaries", {})
    legacy_turns = request.get("legacyTurns", {})
    portable_summary = request.get("portableSummary")
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

origin_by_object, origin_by_item_id = {}, {}
previous_user = None
for record in records:
    payload = record["payload"]
    if record["type"] == "response_item" and payload.get("type") == "message" and payload.get("role") == "user":
        previous_user = payload
        if payload.get("id"):
            origin_by_object[id(payload)] = payload["id"]
            origin_by_item_id[payload["id"]] = payload["id"]
    elif record["type"] == "event_msg" and payload.get("type") == "item_completed" and payload.get("item", {}).get("type") == "UserMessage":
        if previous_user is not None:
            origin = payload["item"]["id"]
            origin_by_object[id(previous_user)] = origin
            if previous_user.get("id"):
                origin_by_item_id[previous_user["id"]] = origin
        previous_user = None
# Resolve all legacy prompt origins before export, including prompts other
# than the selected one, so subsequent imports retain their stable identity.
for turn, users in legacy_turns.items():
    starts = [i for i, record in enumerate(records) if record["type"] == "event_msg"
              and record["payload"].get("type") == "task_started" and record["payload"].get("turn_id") == turn]
    if not starts:
        starts = [i for i, record in enumerate(records) if record["type"] == "turn_context" and record["payload"].get("turn_id") == turn]
    if not starts:
        continue
    start = starts[-1]
    end = next((i for i in range(start + 1, len(records)) if records[i]["type"] == "event_msg" and records[i]["payload"].get("type") == "task_started"), len(records))
    inputs = [record["payload"] for record in records[start + 1:end] if record["type"] == "response_item" and record["payload"].get("type") == "message" and record["payload"].get("role") == "user"]
    if len(inputs) != len(users) or any(user_content(native["content"]) != user_content(user["content"]) for native, user in zip(inputs, users)):
        continue
    for native, user in zip(inputs, users):
        origin_by_object[id(native)] = user["id"]
        if native.get("id"):
            origin_by_item_id[native["id"]] = user["id"]
omit_only = option("--without-message-id") is not None or option("--without-message-ids") is not None
boundaries = json.loads(option("--without-message-ids")) if option("--without-message-ids") else [option("--without-message-id") if omit_only else option("--before-message-id")]
omitted_input_ids = set()
omitted_record_indices = set()
for before in dict.fromkeys(boundaries):
    if before is None:
        continue
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
        for index, item in enumerate(users):
            native = records[inputs[index]]["payload"]
            origin_by_object[id(native)] = item["id"]
            if native.get("id"):
                origin_by_item_id[native["id"]] = item["id"]
        boundary = inputs[ordinal]
    else:
        raise ValueError("Codex native user message boundary was not found")
    if omit_only:
        omitted_input_id = records[boundary]["payload"].get("id")
        if omitted_input_id:
            omitted_input_ids.add(omitted_input_id)
        later_replacements = [record["payload"].get("replacement_history")
                              for record in records[boundary + 1:] if record["type"] == "compacted"]
        if later_replacements and not omitted_input_id:
            # Native compaction may retain the old prompt alongside its summary.
            # Id-less copied inputs cannot be identified by text or ordinal.
            raise ValueError("compacted Codex input lacks stable identity for exact deletion")
        omitted_record_indices.add(boundary)
    else:
        records = records[:boundary]
if omitted_record_indices:
    records = [record for index, record in enumerate(records) if index not in omitted_record_indices]
# The newest full replacement already includes inherited and retained context.
# Older records are no longer model input, including earlier compactions.
start = next((index for index in range(len(records) - 1, -1, -1)
              if records[index]["type"] == "compacted"
              and records[index]["payload"].get("replacement_history") is not None), 0)
items = []
compaction_summary = None
for record in records[start:]:
    kind, payload = record["type"], record["payload"]
    if kind == "response_item":
        items.append(payload)
    elif kind == "compacted":
        replacement = payload.get("replacement_history")
        if replacement is None:
            raise ValueError("legacy compaction without replacement history cannot be translated reliably")
        items = replacement.copy()
        compaction_summary = payload.get("message")
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

# Codex's remote compaction item carries encrypted, provider-private state.
# It cannot be replayed by Pi or another target runtime. Transfer only when
# Codex persisted a portable plaintext summary; visible retained history is not
# equivalent to the missing compacted context.
def encrypted_agent_block(block):
    return (isinstance(block, dict) and block.get("type") == "encrypted_content"
            and set(block) == {"type", "encrypted_content"}
            and isinstance(block["encrypted_content"], str) and bool(block["encrypted_content"]))

encrypted_agent_history = any(
    item.get("type") == "agent_message" and isinstance(item.get("content"), list)
    and any(encrypted_agent_block(block) for block in item["content"])
    for item in items
)
compaction_indices = [index for index, item in enumerate(items)
                      if item.get("type") in ("compaction", "context_compaction")]
if "--requires-portable-summary" in sys.argv:
    summary = compaction_summary.strip() if isinstance(compaction_summary, str) else ""
    print(json.dumps({"required": encrypted_agent_history or bool(compaction_indices and not summary)}))
    sys.exit(0)
if compaction_indices:
    candidate_summary = portable_summary if isinstance(portable_summary, str) else compaction_summary
    summary = candidate_summary.strip() if isinstance(candidate_summary, str) else ""
    if not summary:
        print(json.dumps({"errorCode":"portable_summary_required"}))
        sys.exit(0)
    replacement = {
        "type": "message",
        "role": "assistant",
        "content": [{"type": "output_text", "text": summary}],
    }
    last_compaction = compaction_indices[-1]
    items = [replacement if index == last_compaction else item
             for index, item in enumerate(items)
             if index == last_compaction or index not in compaction_indices]
elif portable_summary is not None and not encrypted_agent_history:
    raise ValueError("portable summary requires encrypted Codex context")

if omit_only and omitted_input_ids:
    # A compact replacement can retain the original response item. Remove that
    # exact item too, while retaining all summaries and subsequent replies.
    items = [item for item in items if not (item.get("type") == "message"
             and item.get("role") == "user" and item.get("id") in omitted_input_ids)]

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
        message = {"role": role, "content": content}
        if role == "user" and kind == "message":
            origin = origin_by_object.get(id(item)) or origin_by_item_id.get(item.get("id"))
            if origin:
                message["source_message_id"] = origin
        messages.append(message)
if encrypted_agent_history:
    summary = portable_summary.strip() if isinstance(portable_summary, str) else ""
    if not summary:
        print(json.dumps({"errorCode":"portable_summary_required"}))
        sys.exit(0)
    # The source fork summarized the whole effective history, including the
    # opaque deliveries. A routing envelope cannot replace their actual text.
    messages = [{"role": "assistant", "content": [{"type": "text", "text": summary}]}]
print(json.dumps({"messages": messages}))
