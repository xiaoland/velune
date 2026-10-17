"""Read the configured Claude Code session root; never load auth or settings."""
import datetime
import json
import os
import re
import sys
import uuid

AGENT_ID = re.compile(r"[A-Za-z0-9_-]+")


def timestamp(value):
    if not isinstance(value, str):
        return None
    try:
        return int(datetime.datetime.fromisoformat(value.replace("Z", "+00:00")).timestamp() * 1000)
    except ValueError:
        return None


def blocks(content, agent=None):
    """`agent` is the Velune conversation of the sub-agent a tool result started."""
    if isinstance(content, str):
        return [{"kind": "text", "text": content}]
    result = []
    for block in content if isinstance(content, list) else []:
        kind = block.get("type")
        if kind == "text":
            result.append({"kind": "text", "text": block.get("text", "")})
        elif kind == "image" and (block.get("source") or {}).get("type") == "base64":
            result.append({"kind": "image", "media_type": block["source"].get("media_type"),
                           "source": {"kind": "data", "data": block["source"].get("data", "")}})
        elif kind == "thinking":
            result.append({"kind": "reasoning", "text": block.get("thinking", "")})
        elif kind in ("tool_use", "tool_result"):
            output = block.get("content") if kind == "tool_result" else None
            result.append({"kind": "tool", "tool_id": block.get("id", block.get("tool_use_id")),
                           "title": block.get("name", "工具结果"),
                           "state": "failed" if block.get("is_error") else ("completed" if kind == "tool_result" else "running"),
                           "output": output if isinstance(output, str) or output is None else json.dumps(output, ensure_ascii=False),
                           "conversation_id": agent if kind == "tool_result" else None})
    return result


def tag(text, name):
    match = re.search(r"<%s>(.*?)</%s>" % (name, name), text, re.S)
    return match.group(1).strip() if match else None


def local_command(content):
    """A slash command as Claude Code records it: ("input", "/name args") for
    what the user typed, ("output", text or None) for a local command's output."""
    if isinstance(content, list) and len(content) == 1 and isinstance(content[0], dict) and content[0].get("type") == "text":
        content = content[0].get("text")
    if not isinstance(content, str):
        return None
    text = content.lstrip()
    for name in ("local-command-stdout", "local-command-stderr"):
        if text.startswith("<%s>" % name):
            return "output", tag(content, name) or None
    if text.startswith("<command-name>") or text.startswith("<command-message>"):
        name = tag(content, "command-name") or "/" + (tag(content, "command-message") or "")
        return "input", " ".join(part for part in (name, tag(content, "command-args")) if part)
    return None


def notification(text, link):
    """A background task's completion, which Claude Code records as a user turn."""
    status = tag(text, "status")
    return {"kind": "tool", "tool_id": tag(text, "tool-use-id"), "title": "任务通知",
            "state": "completed" if status == "completed" else "failed",
            "output": tag(text, "result") or tag(text, "summary"), "conversation_id": link(tag(text, "task-id"))}


def history(path, native_id, runtime_id, sidechain=False):
    """A main session, or with `sidechain` one of its sub-agents' sessions."""
    session = native_id.split("/")[0]
    agents = os.path.dirname(path) if sidechain else os.path.join(os.path.dirname(path), session, "subagents")

    def link(agent_id):
        if not isinstance(agent_id, str) or not AGENT_ID.fullmatch(agent_id):
            return None
        if not os.path.isfile(os.path.join(agents, "agent-%s.jsonl" % agent_id)):
            return None
        return "%s:%s/agent-%s" % (runtime_id, session, agent_id)

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
    entries = {entry["uuid"]: entry for entry in records if isinstance(entry.get("uuid"), str) and bool(entry.get("isSidechain")) == sidechain}
    leaf = next((entry["uuid"] for entry in reversed(records) if entry.get("uuid") in entries and entry.get("type") in ("user", "assistant")), None)
    chain, seen = [], set()
    while leaf in entries:
        if leaf in seen:
            raise ValueError("cycle in native session")
        seen.add(leaf)
        entry = entries[leaf]
        chain.append(entry)
        leaf = entry.get("parentUuid")
    messages, first_user, calls, turn = [], None, {}, None
    # The current turn's prompt and the usage of each model response in it,
    # by message id: a response's records repeat it, the last one complete.
    prompt, usages = None, {}
    for entry in reversed(chain):
        role = entry.get("type")
        if role not in ("user", "assistant"):
            continue
        if role == "user" and entry.get("isMeta"):
            # Context the Harness injected, which its own transcript does not show.
            continue
        content = entry.get("message", {}).get("content")
        if role == "user" and (entry.get("origin") or {}).get("kind") == "task-notification" and isinstance(content, str):
            messages.append({"id": entry["uuid"], "role": "tool", "completed": True,
                             "timestamp_unix_ms": timestamp(entry.get("timestamp")), "blocks": [notification(content, link)]})
            continue
        # A slash command reads as Claude Code shows it, never as its markup,
        # and names no conversation; a local command's output is a notice.
        command = local_command(content) if role == "user" else None
        if command is not None:
            kind, text = command
            if kind == "output":
                if text:
                    messages.append({"id": entry["uuid"], "role": "system", "completed": True,
                                     "timestamp_unix_ms": timestamp(entry.get("timestamp")), "blocks": [{"kind": "notice", "text": text}]})
                continue
            content = text
        result = entry.get("toolUseResult")
        content = blocks(content, link(result.get("agentId")) if isinstance(result, dict) else None)
        if role == "user":
            # A result completes its call's row, as the native transcript shows them.
            for block in [b for b in content if b["kind"] == "tool" and b["tool_id"] in calls]:
                calls.pop(block["tool_id"]).update({key: block[key] for key in ("state", "output", "conversation_id")})
                content.remove(block)
        else:
            calls.update((b["tool_id"], b) for b in content if b["kind"] == "tool" and b["tool_id"])
        model = entry.get("message", {}).get("model")
        # The turn's model is the first its replies name; Claude Code marks
        # replies it wrote itself (errors and the like) with "<synthetic>".
        if role == "assistant" and turn is not None and "model" not in turn and isinstance(model, str) and not model.startswith("<"):
            turn["model"] = model
        # Claude Code records the reasoning level each reply ran with.
        if role == "assistant" and turn is not None and "effort" not in turn and isinstance(entry.get("effort"), str):
            turn["effort"] = entry["effort"]
        usage = entry.get("message", {}).get("usage")
        if role == "assistant" and isinstance(usage, dict) and isinstance(entry["message"].get("id"), str):
            usages[entry["message"]["id"]] = usage
        if not content:
            continue
        if role == "user" and first_user is None and command is None:
            first_user = next((b["text"] for b in content if b["kind"] == "text"), None)
        message = {"id": entry["uuid"], "role": "tool" if role == "user" and all(b["kind"] == "tool" for b in content) else role,
                   "completed": True, "timestamp_unix_ms": timestamp(entry.get("timestamp")), "blocks": content}
        if message["role"] == "user":
            # A replacement resumes at the nearest message record before the
            # prompt (`--resume-session-at`); the first prompt has none.
            parent = entries.get(entry.get("parentUuid"))
            while parent is not None and parent.get("type") not in ("user", "assistant"):
                parent = entries.get(parent.get("parentUuid"))
            if parent is not None:
                message["branch_point"] = parent["uuid"]
            # Claude Code records the permission mode on the prompt that starts a turn.
            mode = entry.get("permissionMode")
            turn = message["turn_settings"] = {"permissionMode": mode} if isinstance(mode, str) else {}
            close_turn(prompt, usages)
            prompt, usages = message, {}
        messages.append(message)
    close_turn(prompt, usages)
    # The session's cost so far, written after each run; only while Claude
    # Code knows every model's price.
    cost = next((r for r in reversed(records) if r.get("type") == "cost-state"), None)
    if prompt is not None and "turn_usage" in prompt and cost and cost.get("hasUnknownModelCost") is False \
            and isinstance(cost.get("totalCostUSD"), (int, float)) and cost["totalCostUSD"] > 0:
        prompt["turn_usage"]["sessionCostMicroUsd"] = round(cost["totalCostUSD"] * 1_000_000)
    # The user's name for the session, else the one Claude Code gave it.
    title = next((name.strip() for kind, key in (("custom-title", "customTitle"), ("ai-title", "aiTitle"))
                  for name in (r.get(key) for r in reversed(records) if r.get("type") == kind)
                  if isinstance(name, str) and name.strip()), None)
    if sidechain:
        title = meta_title(path)
    if title is not None:
        title = title.strip() or None
    if first_user is not None:
        first_user = " ".join(first_user.split())[:80] or None
    cwd = next((r.get("cwd") for r in reversed(records) if isinstance(r.get("cwd"), str)), None)
    dates = [value for r in records if (value := timestamp(r.get("timestamp"))) is not None]
    return {"nativeId": native_id, "title": ({"source":"native", "text":title} if title else {"source":"firstMessage", "text":first_user} if first_user else {"source":"untitled"}), "cwd": cwd,
            "messages": messages, "createdAtUnixMs": min(dates) if dates else None, "updatedAtUnixMs": max(dates) if dates else None}


def close_turn(prompt, usages):
    """The turn's usage on its prompt, and the context its last reply's
    request held. Anthropic counts input without cache reads and writes.
    Claude Code copies thinking tokens from the response and writes 0 when
    there were none to copy, so only more than 0 counts."""
    if prompt is None or not usages:
        return
    def total(key, values=None):
        values = [u.get(key) for u in usages.values()] if values is None else values
        return sum(values) if all(isinstance(v, int) for v in values) else None
    read, write, plain = total("cache_read_input_tokens") or 0, total("cache_creation_input_tokens") or 0, total("input_tokens")
    thinking = total(None, [(u.get("output_tokens_details") or {}).get("thinking_tokens") for u in usages.values()])
    last = list(usages.values())[-1]
    context = [last.get(k) for k in ("input_tokens", "cache_read_input_tokens", "cache_creation_input_tokens", "output_tokens")]
    prompt["turn_usage"] = {"inputTokens": None if plain is None else plain + read + write, "cacheReadTokens": read,
                            "cacheWriteTokens": write, "outputTokens": total("output_tokens"),
                            "reasoningTokens": thinking or None,
                            "contextTokens": sum(v or 0 for v in context) if isinstance(context[0], int) else None}


def meta_title(path):
    """The description the parent agent gave a sub-agent: its native name."""
    try:
        with open(path[:-6] + ".meta.json", encoding="utf-8") as file:
            meta = json.load(file)
    except (OSError, ValueError):
        return None
    title = meta.get("description") if isinstance(meta, dict) else None
    return title if isinstance(title, str) else None


def subagents(path):
    """The sub-agent sessions beside a main session, as (native id, path)."""
    session = os.path.basename(path)[:-6]
    directory = os.path.join(os.path.dirname(path), session, "subagents")
    if os.path.islink(os.path.join(os.path.dirname(path), session)) or os.path.islink(directory) or not os.path.isdir(directory):
        return []
    found = []
    for name in sorted(os.listdir(directory)):
        agent = name[len("agent-"):-len(".jsonl")]
        candidate = os.path.join(directory, name)
        if name.startswith("agent-") and name.endswith(".jsonl") and AGENT_ID.fullmatch(agent) and os.path.isfile(candidate) and not os.path.islink(candidate):
            found.append(("%s/agent-%s" % (session, agent), candidate))
    return found


def first_cwd(path):
    """The working directory a session started in, from its first records."""
    try:
        with open(path, encoding="utf-8") as file:
            for count, line in enumerate(file):
                if count >= 200:
                    break
                try:
                    record = json.loads(line)
                except ValueError:
                    continue
                if isinstance(record, dict) and isinstance(record.get("cwd"), str):
                    return record["cwd"]
    except OSError:
        pass
    return None


def sessions(roots, only=None):
    """Every main session file under the roots, by file name; with `only`,
    just that session's."""
    found = {}
    for root in roots:
        if not os.path.exists(root):
            continue
        for directory, children, files in os.walk(root, followlinks=False):
            children[:] = [name for name in children if name != "subagents" and not os.path.islink(os.path.join(directory, name))]
            for name in files:
                if not name.endswith(".jsonl") or (only and name != only + ".jsonl"):
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
    return found


def summary(path, native_id, runtime_id):
    item = history(path, native_id, runtime_id)
    item.pop("messages")
    return item


def subagent_summary(native_id, candidate, parent):
    title = (meta_title(candidate) or "").strip()
    return {"nativeId": native_id, "parentNativeId": parent["nativeId"], "cwd": parent["cwd"],
            "title": {"source": "native", "text": title} if title else {"source": "untitled"},
            "createdAtUnixMs": None, "updatedAtUnixMs": None}


def main():
    request = json.load(sys.stdin)
    roots = request["roots"]
    if not roots or any(not os.path.isabs(root) for root in roots):
        raise ValueError("absolute session roots required")
    runtime_id = request["runtimeId"]
    operation = request["operation"]
    requested = request.get("nativeId")
    found = sessions(roots, requested.partition("/")[0] if operation == "read" else None)
    if operation == "index":
        # File state only: which sessions exist, when their files changed,
        # and each project folder's working directory from its newest session.
        entries, cwds = [], {}
        for name, path in found.items():
            try:
                status = os.stat(path)
            except OSError:
                continue
            entries.append({"nativeId": name[:-6], "directory": os.path.dirname(path), "size": status.st_size,
                            "modifiedAtUnixMs": int(status.st_mtime * 1000),
                            "createdAtUnixMs": int(status.st_birthtime * 1000) if hasattr(status, "st_birthtime") else None})
        newest = {}
        for entry in entries:
            if entry["modifiedAtUnixMs"] >= newest.get(entry["directory"], (-1, None))[0]:
                newest[entry["directory"]] = (entry["modifiedAtUnixMs"], entry["nativeId"])
        for directory, (_, native_id) in newest.items():
            cwds[directory] = first_cwd(os.path.join(directory, native_id + ".jsonl"))
        for entry in entries:
            entry["cwd"] = cwds.get(entry.pop("directory"))
        return {"contractVersion": 1, "entries": entries}
    if operation == "summaries":
        result, failures = [], []
        for requested in request["nativeIds"]:
            session, _, agent = requested.partition("/")
            try:
                uuid.UUID(session)
                path = found.get(session + ".jsonl")
                if path is None:
                    continue
                if not agent:
                    result.append(summary(path, session, runtime_id))
                    continue
                candidate = dict(subagents(path)).get(requested)
                if candidate is not None:
                    result.append(subagent_summary(requested, candidate, {"nativeId": session, "cwd": first_cwd(path)}))
            except (ValueError, OSError) as error:
                failures.append({"code": "provider_read", "detail": type(error).__name__})
        return {"contractVersion": 1, "sessions": result, "failures": failures}
    if operation == "read":
        session, _, agent = requested.partition("/")
        uuid.UUID(session)
        if agent and not (agent.startswith("agent-") and AGENT_ID.fullmatch(agent[len("agent-"):])):
            raise ValueError("invalid sub-agent id")
        path = found.get(session + ".jsonl")
        if path is not None and agent:
            path = dict(subagents(path)).get(requested)
        if path is None:
            return {"error": {"code": "session_not_found"}}
        return {"contractVersion": 1, "history": history(path, requested, runtime_id, sidechain=bool(agent))}
    raise ValueError("unknown operation")

try:
    print(json.dumps(main(), ensure_ascii=False))
except Exception as error:
    print(json.dumps({"error": {"code": "provider_read", "detail": type(error).__name__}}))
    sys.exit(1)
