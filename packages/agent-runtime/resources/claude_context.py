"""Claude Code's effective branch, including compact summaries and retained segments.
Loaded with claude_history.py so source selection has the same ownership checks.
"""
def claude_native_blocks(entry):
    content = (entry.get("message") or {}).get("content")
    return content if isinstance(content, list) else []


def claude_tool_result_branches(chain, entries, records, after_boundary=None):
    """Complete a selected native tool batch, not arbitrary sibling prompts.

    Parallel results can each name their own assistant fragment as parent.
    The next response follows only the final result, so a linear parent walk
    misses its siblings. Require the exact call owner, an on-chain result of
    that API response, and its saved frontier; never revive later branches or
    results summarized away by compaction. Ambiguous results stay unresolved.
    """
    order = {r.get("uuid"): i for i, r in enumerate(records) if "uuid" in r}
    floor = order[after_boundary] if after_boundary is not None else 0
    selected = {r["uuid"] for r in chain}
    calls, last_fragment = {}, {}
    for index, entry in enumerate(chain):
        if entry.get("type") != "assistant":
            continue
        message = entry.get("message", {})
        group = message.get("id") or entry["uuid"]
        last_fragment[group] = index
        for block in claude_native_blocks(entry):
            if block.get("type") == "tool_use":
                calls[block["id"]] = (entry["uuid"], group)
    present, frontiers, result_rows = set(), {}, {}
    for index, entry in enumerate(chain):
        for block in claude_native_blocks(entry) if entry.get("type") == "user" else []:
            if block.get("type") != "tool_result":
                continue
            tool = block.get("tool_use_id")
            present.add(tool)
            if tool in calls:
                group = calls[tool][1]
                position = order[entry["uuid"]]
                frontiers[group] = max(frontiers.get(group, -1), position)
                result_rows.setdefault(group, []).append((index, position))
    candidates = {}
    for entry in entries.values():
        if entry["uuid"] in selected or entry.get("type") != "user" or order[entry["uuid"]] < floor:
            continue
        for block in claude_native_blocks(entry):
            tool = block.get("tool_use_id")
            owner = calls.get(tool)
            if (block.get("type") == "tool_result" and tool not in present and owner
                    and entry.get("parentUuid") == owner[0]
                    and order[entry["uuid"]] <= frontiers.get(owner[1], -1)):
                candidates.setdefault(tool, []).append((entry, block))
    recovered = {}
    for matches in candidates.values():
        if len(matches) == 1:
            entry, block = matches[0]
            recovered.setdefault(entry["uuid"], (entry, []))[1].append(block)
    insertions = {}
    for entry, blocks in recovered.values():
        group = calls[blocks[0]["tool_use_id"]][1]
        position = order[entry["uuid"]]
        rows = [(index, ordinal) for index, ordinal in result_rows[group]
                if index > last_fragment[group]]
        following = [(index, ordinal) for index, ordinal in rows if ordinal > position]
        previous = [(index, ordinal) for index, ordinal in rows if ordinal < position]
        insertion = (min(following, key=lambda row: row[1])[0] if following else
                     max(previous, key=lambda row: row[1])[0] + 1 if previous else last_fragment[group] + 1)
        # Off-chain human text remains off-chain. Only proven tool results
        # belong to the selected batch; their nested output stays untouched.
        result = dict(entry, message=dict(entry["message"], content=blocks))
        insertions.setdefault(insertion, []).append((position, result))
    result = []
    for index in range(len(chain) + 1):
        result.extend(entry for _, entry in sorted(insertions.get(index, []), key=lambda item: item[0]))
        if index < len(chain):
            result.append(chain[index])
    return result


def claude_replay_entries(records, sidechain):
    entries = {r["uuid"]: r for r in records if "uuid" in r and bool(r.get("isSidechain")) == sidechain}
    leaf = next((r["uuid"] for r in reversed(records) if r.get("uuid") in entries and r.get("type") in ("user", "assistant")), None)
    return entries, leaf


def claude_replay_chain(entries, leaf):
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
    return chain


def export_context(path, sidechain=False, before_message_id=None, without_message_id=None, without_message_ids=None):
    if before_message_id is not None and without_message_id is not None:
        raise ValueError("conflicting native context boundaries")
    with open(path, encoding="utf-8") as file:
        records = [json.loads(line) for line in file if line.strip()]
    boundary_parent = None
    if before_message_id is not None:
        boundary = next((i for i, entry in enumerate(records)
                         if entry.get("uuid") == before_message_id
                         and bool(entry.get("isSidechain")) == sidechain
                         and entry.get("type") == "user"), None)
        if boundary is None:
            raise ValueError("native user message boundary was not found")
        selected = records[boundary]
        boundary_parent = selected.get("parentUuid")
        # Tool results in this same native entry belong to preceding calls,
        # not to the selected human prompt. Preserve them at the exact cut.
        content = selected.get("message", {}).get("content")
        results = [block for block in content if block.get("type") == "tool_result"] if isinstance(content, list) else []
        records = records[:boundary]
        if results:
            records.append(dict(selected, message=dict(selected["message"], content=results)))
            boundary_parent = selected["uuid"]
    entries, workspace_leaf = claude_replay_entries(records, sidechain)
    leaf = boundary_parent if before_message_id is not None else workspace_leaf
    chain = claude_replay_chain(entries, leaf)
    for without_message_id in without_message_ids or ([without_message_id] if without_message_id is not None else []):
        matches = [i for i, entry in enumerate(chain)
                   if entry.get("uuid") == without_message_id and entry.get("type") == "user"]
        source_matches = [entry for entry in records if entry.get("uuid") == without_message_id
                          and entry.get("type") == "user" and bool(entry.get("isSidechain")) == sidechain]
        if not source_matches:
            raise ValueError("native user prompt was not found")
        # A compacted-away prompt is already absent from active input. Keep the
        # summary/replies: deleting a record does not promise model forgetting.
        if matches:
            if len(matches) != 1:
                raise ValueError("ambiguous native user identity")
            selected_index = matches[0]
            selected = chain[selected_index]
            content = selected.get("message", {}).get("content")
            results = [block for block in content if block.get("type") == "tool_result"] if isinstance(content, list) else []
            if isinstance(content, list) and not any(block.get("type") in ("text", "image") for block in content):
                raise ValueError("selected native entry has no human prompt")
            if results:
                chain[selected_index] = dict(selected, message=dict(selected["message"], content=results))
            else:
                del chain[selected_index]
    compact = next((entry["uuid"] for entry in reversed(chain) if entry.get("subtype") == "compact_boundary"), None)
    chain = claude_tool_result_branches(chain, entries, records, after_boundary=compact)
    # Native replay drops an explicitly interrupted, wholly unanswered tool
    # response. Do not infer cancellation from human-authored marker text,
    # discard a completed result, or fabricate an output for a missing tool.
    answered = {block.get("tool_use_id") for entry in chain
                for block in claude_native_blocks(entry)
                if block.get("type") == "tool_result"}
    interrupted = {entry["interruptedMessageId"] for entry in chain
                   if entry.get("type") == "user" and isinstance(entry.get("interruptedMessageId"), str)
                   and "promptSource" not in entry and "turnPosition" not in entry
                   and entry.get("message", {}).get("content") in (
                       [{"type": "text", "text": "[Request interrupted by user]"}],
                       [{"type": "text", "text": "[Request interrupted by user for tool use]"}])}
    api_calls = {}
    for entry in chain:
        if entry.get("type") == "assistant":
            message = entry.get("message", {})
            for block in claude_native_blocks(entry):
                if block.get("type") == "tool_use":
                    api_calls.setdefault(message.get("id"), set()).add(block["id"])
    discarded = {identity for identity in interrupted if api_calls.get(identity)
                 and not api_calls[identity].intersection(answered)}

    def content_blocks(content, inside_tool_result=False):
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
                result.append({"type": "tool_result", "id": block["tool_use_id"], "content": content_blocks(block.get("content", ""), inside_tool_result=True), "is_error": block.get("is_error", False)})
            elif kind == "tool_reference" and inside_tool_result and isinstance(block.get("tool_name"), str) and block["tool_name"].strip():
                # A portable target has its own tool catalog. Preserve which
                # tool was found without asking it to expand a Claude reference.
                result.append({"type": "text", "text": "Tool reference: " + block["tool_name"]})
            elif kind not in ("thinking", "redacted_thinking"):
                raise ValueError("unsupported Claude context block: " + kind)
        return result
    messages, assistant_fragments = [], {}
    for entry in chain:
        if entry.get("type") not in ("user", "assistant") or entry.get("isApiErrorMessage"):
            continue
        if entry.get("type") == "assistant" and entry.get("message", {}).get("id") in discarded:
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
            api_id = entry["message"].get("id") if entry["type"] == "assistant" else None
            if api_id and api_id in assistant_fragments:
                assistant_fragments[api_id]["content"].extend(blocks)
                continue
            message = {"role": entry["type"], "content": blocks}
            if (entry["type"] == "user" and any(block["type"] in ("text", "image") for block in blocks)
                    and not entry.get("isMeta") and not entry.get("isCompactSummary")
                    and (entry.get("origin") or {}).get("kind") != "task-notification"):
                message["source_message_id"] = entry["uuid"]
            messages.append(message)
            if api_id:
                assistant_fragments[api_id] = message
    for message in assistant_fragments.values():
        # A streamed assistant response can finish its text after a parallel
        # tool result. Native replay places that response's text before calls.
        message["content"] = ([block for block in message["content"] if block["type"] != "tool_call"]
                              + [block for block in message["content"] if block["type"] == "tool_call"])
    return {"messages": messages}
