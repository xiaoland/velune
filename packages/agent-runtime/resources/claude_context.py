"""Claude Code's effective branch, including compact summaries and retained segments.
Loaded with claude_history.py so source selection has the same ownership checks.
"""
def export_context(path, sidechain=False):
    with open(path, encoding="utf-8") as file:
        records = [json.loads(line) for line in file if line.strip()]
    entries = {r["uuid"]: r for r in records if "uuid" in r and bool(r.get("isSidechain")) == sidechain}
    leaf = next((r["uuid"] for r in reversed(records) if r.get("uuid") in entries and r.get("type") in ("user", "assistant")), None)
    chain, seen = [], set()
    while leaf is not None:
        if leaf in seen or leaf not in entries:
            raise ValueError("incomplete native context branch")
        seen.add(leaf)
        entry = entries[leaf]
        chain.append(entry)
        leaf = entry.get("parentUuid")
    chain.reverse()
    # Compaction can retain selected records outside the summary's parent chain.
    for boundary in list(chain):
        if boundary.get("subtype") != "compact_boundary":
            continue
        meta = boundary.get("compactMetadata", {})
        kept = meta.get("preservedMessages")
        segment = meta.get("preservedSegment")
        if kept:
            ids, anchor = kept["uuids"], kept["anchorUuid"]
        elif segment:
            ids, anchor = [], segment["anchorUuid"]
            cursor = segment["tailUuid"]
            while True:
                if cursor not in entries or cursor in ids:
                    raise ValueError("invalid compact retained segment")
                ids.append(cursor)
                if cursor == segment["headUuid"]:
                    break
                cursor = entries[cursor].get("parentUuid")
            ids.reverse()
        else:
            continue
        insertion = next(i for i, entry in enumerate(chain) if entry["uuid"] == anchor) + 1
        retained = [entries[key] for key in ids if key not in {r["uuid"] for r in chain}]
        chain[insertion:insertion] = retained
    def content_blocks(content):
        if isinstance(content, str):
            return [{"type": "text", "text": content}]
        result = []
        for block in content:
            kind = block["type"]
            if kind == "text":
                result.append({"type": "text", "text": block["text"]})
            elif kind == "image" and block["source"]["type"] == "base64":
                result.append({"type": "image", "media_type": block["source"]["media_type"], "data": block["source"]["data"]})
            elif kind == "tool_use":
                result.append({"type": "tool_call", "id": block["id"], "name": block["name"], "arguments": block["input"]})
            elif kind == "tool_result":
                result.append({"type": "tool_result", "id": block["tool_use_id"], "content": content_blocks(block.get("content", "")), "is_error": block.get("is_error", False)})
            elif kind not in ("thinking", "redacted_thinking"):
                raise ValueError("unsupported Claude context block: " + kind)
        return result
    messages = []
    for entry in chain:
        if entry.get("type") not in ("user", "assistant") or entry.get("isApiErrorMessage"):
            continue
        # Local command echoes and transcript-only records are not model input;
        # a compact summary is model input even when marked transcript-only.
        if entry.get("isVisibleInTranscriptOnly") and not entry.get("isCompactSummary"):
            continue
        content = entry["message"]["content"]
        if local_command(content) is not None:
            continue
        blocks = content_blocks(content)
        if blocks:
            messages.append({"role": entry["type"], "content": blocks})
    return {"messages": messages}
