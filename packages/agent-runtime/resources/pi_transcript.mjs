// Complete display history for the active native branch. Pi's session
// projection deliberately truncates older entries at compaction boundaries;
// presentation needs the full branch while preserving Pi's entry semantics.
export function projectPiDisplayHistory(sessionManager, sessionEntryToContextMessages) {
  if (typeof sessionEntryToContextMessages !== "function") {
    throw new TypeError("Pi SDK session entry converter is unavailable");
  }
  const branch = sessionManager.getBranch();
  const edits = new Map();
  for (const entry of branch) {
    if (entry.type === "context_edit") edits.set(entry.targetId, entry);
  }

  const entries = [];
  for (const sourceEntry of branch) {
    let messages = sessionEntryToContextMessages(sourceEntry);
    const edit = edits.get(sourceEntry.id);
    if (edit) {
      if (edit.replacement === null) {
        messages = [];
      } else {
        const replacement = edit.replacement;
        messages = messages.map(message => {
          if (!["user", "assistant", "toolResult", "custom"].includes(message.role)) return message;
          const content = (message.role === "assistant" || message.role === "toolResult")
            && typeof replacement.content === "string"
            ? [{type:"text", text:replacement.content}]
            : replacement.content;
          return {...message, content};
        });
      }
    }
    entries.push({sourceEntry, messages});
  }
  return {entries, messages:entries.flatMap(entry => entry.messages)};
}
