"""Export a Codex rollout's effective Responses context, not display events."""
import json
import sys

records = [json.loads(line) for line in sys.stdin if line.strip()]
# The newest full replacement already includes inherited and retained context.
# Older records are no longer model input, including earlier compactions.
start = next((index for index in range(len(records) - 1, -1, -1)
              if records[index]["type"] == "compacted"
              and records[index]["payload"].get("replacement_history") is not None), 0)
if not start and any(record["type"] == "session_meta" and record["payload"].get("history_base")
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
    elif kind not in ("session_meta", "turn_context", "event_msg", "world_state", "token_usage_record"):
        raise ValueError("unsupported Codex rollout record: " + kind)

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
    elif kind in ("function_call", "custom_tool_call"):
        arguments = json.loads(item["arguments"]) if kind == "function_call" else {"input": item["input"]}
        role, content = "assistant", [{"type": "tool_call", "id": item["call_id"], "name": item["name"], "arguments": arguments}]
    elif kind in ("function_call_output", "custom_tool_call_output"):
        output = item["output"]
        if isinstance(output, dict):
            output = output["content"]
        role, content = "user", [{"type": "tool_result", "id": item["call_id"], "content": blocks(output), "is_error": False}]
    elif kind in ("reasoning", "additional_tools"):
        # Tool availability belongs to the target runtime, not historical calls.
        continue
    else:
        raise ValueError("unsupported Codex context item: " + kind)
    if content:
        messages.append({"role": role, "content": content})
print(json.dumps({"messages": messages}))
