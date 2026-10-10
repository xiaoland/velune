"""Read the configured Claude Code session root; never load auth or settings."""
import datetime
import json
import os
import sys
import uuid


def timestamp(value):
    if not isinstance(value, str):
        return None
    try:
        return int(datetime.datetime.fromisoformat(value.replace("Z", "+00:00")).timestamp() * 1000)
    except ValueError:
        return None


def blocks(content):
    if isinstance(content, str):
        return [{"kind": "text", "text": content}]
    result = []
    for block in content if isinstance(content, list) else []:
        kind = block.get("type")
        if kind == "text":
            result.append({"kind": "text", "text": block.get("text", "")})
        elif kind == "thinking":
            result.append({"kind": "reasoning", "text": block.get("thinking", "")})
        elif kind in ("tool_use", "tool_result"):
            output = block.get("content") if kind == "tool_result" else None
            result.append({"kind": "tool", "tool_id": block.get("id", block.get("tool_use_id")),
                           "title": block.get("name", "工具结果"),
                           "state": "failed" if block.get("is_error") else ("completed" if kind == "tool_result" else "running"),
                           "output": output if isinstance(output, str) or output is None else json.dumps(output, ensure_ascii=False)})
    return result


def history(path):
    records = []
    with open(path, encoding="utf-8") as file:
        for line in file:
            if line.strip():
                try:
                    record = json.loads(line)
                except json.JSONDecodeError:
                    # The native process can be appending its final JSONL record during a read.
                    if not line.endswith("\n"):
                        break
                    raise
                if isinstance(record, dict):
                    records.append(record)
    native_id = os.path.basename(path)[:-6]
    entries = {entry["uuid"]: entry for entry in records if isinstance(entry.get("uuid"), str) and not entry.get("isSidechain")}
    leaf = next((entry["uuid"] for entry in reversed(records) if entry.get("uuid") in entries and entry.get("type") in ("user", "assistant")), None)
    chain, seen = [], set()
    while leaf in entries:
        if leaf in seen:
            raise ValueError("cycle in native session")
        seen.add(leaf)
        entry = entries[leaf]
        chain.append(entry)
        leaf = entry.get("parentUuid")
    messages, first_user = [], None
    for entry in reversed(chain):
        role = entry.get("type")
        if role not in ("user", "assistant"):
            continue
        content = blocks(entry.get("message", {}).get("content"))
        if not content:
            continue
        if role == "user" and first_user is None:
            first_user = next((b["text"] for b in content if b["kind"] == "text"), None)
        messages.append({"id": entry["uuid"], "role": "tool" if role == "user" and all(b["kind"] == "tool" for b in content) else role,
                         "completed": True, "timestamp_unix_ms": timestamp(entry.get("timestamp")), "blocks": content})
    title = next((r.get("customTitle") for r in reversed(records) if r.get("type") == "custom-title" and isinstance(r.get("customTitle"), str)), None)
    if title is not None:
        title = title.strip() or None
    if first_user is not None:
        first_user = " ".join(first_user.split())[:80] or None
    cwd = next((r.get("cwd") for r in reversed(records) if isinstance(r.get("cwd"), str)), None)
    dates = [value for r in records if (value := timestamp(r.get("timestamp"))) is not None]
    return {"nativeId": native_id, "title": ({"source":"native", "text":title} if title else {"source":"firstMessage", "text":first_user} if first_user else {"source":"untitled"}), "cwd": cwd,
            "messages": messages, "createdAtUnixMs": min(dates) if dates else None, "updatedAtUnixMs": max(dates) if dates else None}


def main():
    request = json.load(sys.stdin)
    roots = request["roots"]
    if not roots or any(not os.path.isabs(root) for root in roots):
        raise ValueError("absolute session roots required")
    requested = request.get("nativeId")
    if requested:
        uuid.UUID(requested)
    found = {}
    for root in roots:
        if not os.path.exists(root):
            continue
        for directory, children, files in os.walk(root, followlinks=False):
            children[:] = [name for name in children if name != "subagents" and not os.path.islink(os.path.join(directory, name))]
            for name in files:
                if not name.endswith(".jsonl") or (requested and name != requested + ".jsonl"):
                    continue
                try:
                    uuid.UUID(name[:-6])
                except ValueError:
                    continue
                path = os.path.join(directory, name)
                if os.path.islink(path):
                    continue
                if name in found:
                    raise ValueError("ambiguous_session")
                found[name] = path
    if request["operation"] == "read":
        path = found.get(requested + ".jsonl")
        if path is None:
            return {"error": {"code": "session_not_found"}}
        return {"contractVersion": 1, "history": history(path)}
    sessions = []
    for path in found.values():
        item = history(path)
        item.pop("messages")
        sessions.append(item)
    return {"contractVersion": 1, "sessions": sessions, "failures": []}


try:
    print(json.dumps(main(), ensure_ascii=False))
except Exception as error:
    print(json.dumps({"error": {"code": "provider_read", "detail": type(error).__name__}}))
    sys.exit(1)
