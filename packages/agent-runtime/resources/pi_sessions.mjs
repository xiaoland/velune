// Pi SDK session-list projection. CoreRuntime owns process isolation;
// Pi's SessionManager remains authoritative for session metadata.

import { resolvePiSdk } from "./pi_sdk.mjs";
import { projectPiDisplayHistory } from "./pi_transcript.mjs";
import { readSubagentMetadata } from "./pi_subagents.mjs";
import { lstat, open } from "node:fs/promises";
import { isAbsolute, join, relative, resolve, sep } from "node:path";

const args = process.argv.slice(2);
const value = (name) => {
  const index = args.indexOf(name);
  return index >= 0 ? args[index + 1] : undefined;
};
const agentDir = value("--agent-dir");
if (agentDir) process.env.PI_CODING_AGENT_DIR = agentDir;
// Set the explicit instance root before SDK initialization. The Rust helper
// transport clears ambient configuration so history cannot select another HOME.
// Module loading dominates small reads and unchanged listings. Import only the
// version-pinned session module, and only when Pi must read or modify a session.
const installation = resolvePiSdk({binary:value("--cli"),runtimeTypeId:value("--runtime-type")});
const sessionManager = () => installation.importModule("dist/core/session-manager.js");
const sessionDir = value("--session-dir");
const sessionsRoot = () => resolve(agentDir, "sessions");
const inspectPath = value("--inspect-session");
const mutationPath = value("--rename-session") ?? value("--delete-session");
const exportPath = value("--export-context");
const deleteMessagePath = value("--delete-message");
const forkPath = value("--fork-session");

const maxChildDepth = 16;
const runID = /^[0-9a-f]{8}-[0-9a-f]{4}-[0-9a-f]{4}-[0-9a-f]{4}-[0-9a-f]{12}$/i;
const runSlot = name => /^run-(0|[1-9][0-9]{0,9})$/.test(name) && Number(name.slice(4)) <= 0xffffffff;
const hasSessionIdentity = header => typeof header?.id === "string" && header.id.length > 0;

async function readHeader(path) {
  const file = await open(path, "r");
  try {
    const buffer = Buffer.alloc(65536);
    const { bytesRead } = await file.read(buffer, 0, buffer.length, 0);
    const line = buffer.subarray(0, bytesRead).toString("utf8").split("\n")[0];
    const value = JSON.parse(line);
    return value?.type === "session" ? value : null;
  } catch {
    return null;
  } finally {
    await file.close();
  }
}

// Validate an exact source without enumerating projects or sibling sessions.
// Reverse the same three-component hops used by child discovery, and require
// every hop to agree with the native header of its child.
async function inspectSource(path) {
  const root = sessionDir ? resolve(sessionDir) : sessionsRoot();
  const within = relative(root, path);
  const parts = within.split(sep);
  const baseParts = sessionDir ? 1 : 2;
  const depth = (parts.length - baseParts) / 3;
  const reject = () => { throw new Error("Pi session is not in the configured history source"); };
  if (!within || parts[0] === ".." || isAbsolute(within) || join(root, within) !== path
      || !Number.isInteger(depth) || depth < 0 || depth > maxChildDepth
      || !parts.at(-1).endsWith(".jsonl")) reject();
  if (sessionDir && !(await lstat(root).catch(() => null))?.isDirectory()) reject();

  let directory = root;
  for (const part of parts.slice(0, -1)) {
    directory = join(directory, part);
    if (!(await lstat(directory).catch(() => null))?.isDirectory()) reject();
  }
  let parentPath = null, previousChild = null, source;
  for (;;) {
    const candidate = join(root, ...parts);
    const stats = await lstat(candidate).catch(() => null);
    if (!stats?.isFile()) reject();
    const header = await readHeader(candidate).catch(() => null);
    if (!hasSessionIdentity(header) || (previousChild && previousChild.parentSession !== candidate)) reject();
    if (!source) source = {header, stats};
    if (parts.length === baseParts) break;
    const [run, slot, name] = parts.splice(-3);
    if (run !== "forks" && !(runID.test(run) && runSlot(slot) && name === "session.jsonl")) reject();
    parts[parts.length - 1] += ".jsonl";
    parentPath ??= join(root, ...parts);
    previousChild = header;
  }
  return {...source, parentPath};
}

// The SDK does not expose its single-file listing projection. Extract only
// the list fields the open view needs from entries already read by the SDK.
function inspectedSummary(path, source, manager, entries, name) {
  const header = manager.getHeader();
  if (header?.id !== source.header.id || header.parentSession !== source.header.parentSession) {
    throw new Error("Pi session identity changed during history read");
  }
  let firstMessage = "", lastActivity = 0;
  for (const entry of entries) {
    if (entry.type !== "message") continue;
    const message = entry.message;
    if (message?.role !== "user" && message?.role !== "assistant") continue;
    if (!("content" in message)) continue;
    const timestamp = typeof message.timestamp === "number" ? message.timestamp : Date.parse(entry.timestamp);
    if (Number.isFinite(timestamp)) lastActivity = Math.max(lastActivity, timestamp);
    if (!firstMessage && message.role === "user") {
      const content = message.content;
      firstMessage = typeof content === "string" ? content
        : (Array.isArray(content) ? content.filter(block => block.type === "text").map(block => block.text).join(" ") : "");
    }
  }
  const created = Date.parse(header.timestamp);
  return {
    path, parentPath: source.parentPath, cwd: manager.getCwd(), name,
    createdUnixMs: Number.isFinite(created) ? created : null,
    modifiedUnixMs: new Date(lastActivity || (Number.isFinite(created) ? created : source.stats.mtimeMs)).getTime(),
    firstMessage: firstMessage || "(no messages)",
  };
}

if (forkPath) {
  const sdk = await sessionManager();
  const { SessionManager, CURRENT_SESSION_VERSION, loadEntriesFromFile, buildSessionProjection, sessionEntryToContextMessages } = sdk;
  const { existsSync, writeFileSync, readFileSync, openSync, fsyncSync, closeSync } = await import("node:fs");
  const { dirname } = await import("node:path");
  const { createHash } = await import("node:crypto");
  const { isDeepStrictEqual } = await import("node:util");
  let input = "";
  for await (const chunk of process.stdin) input += chunk;
  let copyStarted = false;
  let targetPath;
  try {
    const { position, entryId, preservePrefixEdits = false, expectedDigest } = JSON.parse(input);
    if (preservePrefixEdits && position !== "before") throw new Error("Edit fork requires a before boundary");
    const original = preservePrefixEdits ? readFileSync(forkPath) : undefined;
    if (original) {
      if (typeof expectedDigest !== "string" || !/^[0-9a-f]{64}$/.test(expectedDigest)
          || createHash("sha256").update(original).digest("hex") !== expectedDigest) {
        throw new Error("Pi edit source differs from the authorized byte digest");
      }
      const text = new TextDecoder("utf-8", {fatal:true}).decode(original);
      if (!text.endsWith("\n")) throw new Error("Incomplete Pi source");
      // SDK's tolerant reader must not authorize an edit through malformed input.
      for (const line of text.split("\n").filter(line => line.trim())) JSON.parse(line);
    }
    // Opening current native format is read-only. Do not let an SDK open
    // migrate the source as a side effect of creating a different session.
    const entries = loadEntriesFromFile(forkPath);
    if (entries[0]?.type !== "session" || entries[0].version !== CURRENT_SESSION_VERSION) {
      throw new Error("Pi source must use the current native format before forking");
    }
    const source = SessionManager.open(forkPath, sessionDir);
    const entry = source.getEntry(entryId);
    if (!entry || !["at", "before"].includes(position)) throw new Error("Invalid Pi fork boundary");
    const leaf = position === "at" ? entry.id : entry.parentId;
    let carry = [], expectedProjection, expectedDisplay, retainedEntries = [];
    if (preservePrefixEdits) {
      const branch = source.getBranch();
      if (entry.type !== "message" || entry.message.role !== "user" || !branch.some(item => item.id === entry.id)) {
        throw new Error("Edit boundary is not an active native user entry");
      }
      retainedEntries = leaf ? source.getBranch(leaf) : [];
      const retained = new Set(retainedEntries.map(item => item.id));
      const latest = new Map(branch.filter(item => item.type === "context_edit").map(item => [item.targetId, item]));
      carry = [...latest.values()].filter(item => retained.has(item.targetId) && !retained.has(item.id));
      const prefix = source.getEntries().slice(0, source.getEntries().findIndex(item => item.id === entry.id));
      let projectedLeaf = leaf;
      for (const edit of carry) {
        prefix.push({...edit, parentId:projectedLeaf});
        projectedLeaf = edit.id;
      }
      expectedProjection = buildSessionProjection(prefix, projectedLeaf);
      // Model compaction and complete display history are distinct contracts.
      // Use the same full-branch producer as cold/live readers for identities
      // and every displayed message before the selected native entry.
      expectedDisplay = projectPiDisplayHistory(
        {getBranch:() => [...retainedEntries, ...carry]}, sessionEntryToContextMessages);
      if (!readFileSync(forkPath).equals(original)) throw new Error("Pi edit source changed before fork");
    }
    copyStarted = true;
    if (leaf) {
      targetPath = source.createBranchedSession(leaf);
    } else {
      const target = SessionManager.create(source.getCwd(), source.getSessionDir());
      target.newSession({parentSession:forkPath});
      targetPath = target.getSessionFile();
      // An empty fork is still an addressable target before its first turn.
      writeFileSync(targetPath, `${JSON.stringify(target.getHeader())}\n`, {flag:"wx", mode:0o600});
    }
    if (!targetPath) throw new Error("Pi did not create a fork identity");
    if (!existsSync(targetPath)) {
      // SDK persistence normally waits for conversation content. Materialize
      // only the SDK-produced new records, never rewrite the source file.
      const records = [source.getHeader(), ...source.getEntries()];
      writeFileSync(targetPath, records.map(record => JSON.stringify(record)).join("\n") + "\n", {flag:"wx", mode:0o600});
    }
    if (preservePrefixEdits) {
      const target = SessionManager.open(targetPath, sessionDir);
      for (const edit of carry) target.appendContextEdit(edit.targetId, edit.replacement);
      // Persist the SDK-selected leaf even when the effective prefix is empty.
      target.appendSessionInfo(target.getSessionName() ?? "");
      const reopened = SessionManager.open(targetPath, sessionDir);
      // The SDK creates a new session header and new carry-edit entry IDs.
      // Compare effective native message payloads, including images/tool pairs
      // and custom messages, without confusing those new identities with loss.
      const normalized = projection => projection.entries.filter(item => item.messages.length)
        .map(item => item.messages);
      if (!isDeepStrictEqual(normalized(reopened.buildSessionProjection()), normalized(expectedProjection))) {
        throw new Error("Pi edit fork did not preserve the effective native prefix");
      }
      const displayIdentity = projection => projection.entries.flatMap(entry =>
        entry.messages.map((message, ordinal) => ({sourceEntryId:entry.sourceEntry.id, ordinal, message})));
      if (!isDeepStrictEqual(displayIdentity(projectPiDisplayHistory(reopened, sessionEntryToContextMessages)),
          displayIdentity(expectedDisplay))) {
        throw new Error("Pi edit fork did not preserve the complete display prefix or native message identities");
      }
      // Native custom payloads can affect extensions without rendering a row.
      for (const entry of retainedEntries.filter(item => item.type === "custom" || item.type === "custom_message")) {
        if (!isDeepStrictEqual(reopened.getEntry(entry.id), entry)) throw new Error("Pi edit fork lost native custom data");
      }
      if (!readFileSync(forkPath).equals(original)) throw new Error("Pi edit source changed during fork; recovery copy retained");
      // Reopening proves visibility, not durability. Flush the complete SDK
      // target (including carry edits), then its directory, before links may
      // refer to it. Any error below retains this fork as fork_created.
      const file = openSync(targetPath, "r+");
      try { fsyncSync(file); } finally { closeSync(file); }
      const directory = openSync(dirname(targetPath), "r");
      try { fsyncSync(directory); } finally { closeSync(directory); }
      if (!readFileSync(forkPath).equals(original)) throw new Error("Pi edit source changed before durable fork completion; recovery copy retained");
    }
    process.stdout.write(JSON.stringify({path:targetPath}));
  } catch (error) {
    const code = targetPath ? "fork_created" : copyStarted ? "fork_outcome_unknown" : "fork_rejected";
    process.stdout.write(JSON.stringify({error:`Pi fork${targetPath ? ` ${targetPath}` : ""}: ${error.message}`, code}));
  }
} else if (deleteMessagePath) {
  const { SessionManager, CURRENT_SESSION_VERSION, assertValidSessionId } = await sessionManager();
  const { isDeepStrictEqual } = await import("node:util");
  const { realpath, lstat, readFile, mkdir, open } = await import("node:fs/promises");
  const { resolve, relative, isAbsolute, join } = await import("node:path");
  const { createHash, randomUUID } = await import("node:crypto");
  let input = "";
  for await (const chunk of process.stdin) input += chunk;
  const { messageId, truncate, expectedDigest, backupDirectory } = JSON.parse(input);
  const root = await realpath(sessionDir ?? resolve(agentDir, "sessions"));
  const actual = await realpath(deleteMessagePath);
  const within = relative(root, actual);
  const stat = await lstat(deleteMessagePath);
  if (stat.isSymbolicLink() || stat.nlink !== 1 || !within || within.startsWith("../") || isAbsolute(within)) throw new Error("Session source escaped configured root");
  const original = await readFile(actual);
  const digest = bytes => createHash("sha256").update(bytes).digest("hex");
  if (digest(original) !== expectedDigest) throw new Error("Session changed before deletion");
  // SessionManager.open migrates known old formats in place and its parser
  // skips malformed JSONL lines. Validate every original line before giving
  // the SDK write access; future/unknown formats must remain byte-for-byte.
  const parseStrict = bytes => {
    const text = new TextDecoder("utf-8", {fatal:true}).decode(bytes);
    if (!text.endsWith("\n")) throw new Error("Incomplete Pi session JSONL");
    const records = text.split("\n").filter(line => line.trim()).map(line => JSON.parse(line));
    const header = records[0];
    if (!header || header.type !== "session") throw new Error("Invalid Pi session header");
    const version = header.version ?? 1;
    if (!Number.isInteger(version) || version < 1 || version > CURRENT_SESSION_VERSION) throw new Error("Unsupported Pi session version");
    assertValidSessionId(header.id);
    const known = new Set(["message", "thinking_level_change", "model_change", "usage", "compaction", "branch_summary", "custom", "custom_message", "context_edit", "label", "session_info"]);
    const ids = new Set();
    for (const entry of records.slice(1)) {
      if (!entry || typeof entry !== "object" || !known.has(entry.type)) throw new Error("Unknown Pi session entry");
      if (version >= 2) {
        if (typeof entry.id !== "string" || !entry.id || ids.has(entry.id)
            || (entry.parentId !== null && !ids.has(entry.parentId))) throw new Error("Invalid Pi session entry identity or parent");
      }
      if (entry.type === "message" && (!entry.message || typeof entry.message.role !== "string")) throw new Error("Invalid Pi message entry");
      if (entry.type === "context_edit" && (!ids.has(entry.targetId)
          || (entry.replacement !== null && (!entry.replacement || typeof entry.replacement !== "object"
            || (typeof entry.replacement.content !== "string" && !Array.isArray(entry.replacement.content)))))) throw new Error("Invalid Pi context edit");
      if (typeof entry.id === "string") ids.add(entry.id);
    }
    return records;
  };
  const originalRecords = parseStrict(original);
  const prefix = `pi:${originalRecords[0].id}:`;
  if (typeof messageId !== "string" || !messageId.startsWith(prefix)) throw new Error("Invalid Pi message identity");
  const parts = messageId.slice(prefix.length).split(":");
  if (parts.length !== 2 || parts[1] !== "0" || typeof truncate !== "boolean" || !isAbsolute(backupDirectory)) throw new Error("Invalid deletion request");
  await mkdir(backupDirectory, {recursive:true, mode:0o700});
  const directory = await lstat(backupDirectory);
  if (!directory.isDirectory() || directory.isSymbolicLink()) throw new Error("Invalid backup directory");
  const backupPath = join(backupDirectory, `${randomUUID()}.pi.jsonl`);
  const backup = await open(backupPath, "wx", 0o600);
  try { await backup.writeFile(original); await backup.sync(); } finally { await backup.close(); }
  const directoryHandle = await open(backupDirectory, "r");
  try { await directoryHandle.sync(); } finally { await directoryHandle.close(); }
  if (digest(await readFile(actual)) !== expectedDigest) throw new Error("Session changed during deletion");
  if (!(await SessionManager.listAll(sessionDir)).some(session => session.path === deleteMessagePath)) throw new Error("Session not in configured source");
  let manager;
  try { manager = SessionManager.open(actual); }
  catch (error) { throw new Error(`Pi could not open the source safely; original backup: ${backupPath}`, {cause:error}); }
  const afterOpen = await readFile(actual);
  if (digest(afterOpen) !== expectedDigest) {
    let upgraded;
    try { upgraded = parseStrict(afterOpen); }
    catch (error) { throw new Error(`Pi source changed while opening; original backup: ${backupPath}`, {cause:error}); }
    const sourceVersion = originalRecords[0].version ?? 1;
    const messages = records => records.filter(entry => entry.type === "message").map(entry => {
      const message = structuredClone(entry.message);
      if (message.role === "hookMessage") message.role = "custom";
      return message;
    });
    if (sourceVersion >= CURRENT_SESSION_VERSION || upgraded.length !== originalRecords.length
        || upgraded[0].version !== CURRENT_SESSION_VERSION || !isDeepStrictEqual(messages(originalRecords), messages(upgraded))) {
      throw new Error(`Pi source changed while opening; deletion was not applied. Original backup: ${backupPath}`);
    }
    const upgradedHandle = await open(actual, "r+");
    try { await upgradedHandle.sync(); } finally { await upgradedHandle.close(); }
    throw new Error(`Pi session was safely migrated; reload it before retrying deletion. Original backup: ${backupPath}`);
  }
  const projection = manager.buildSessionProjection();
  const projected = projection.entries.find(entry => entry.sourceEntry.id === parts[0]);
  // A tombstone applies to a whole source entry. Refuse compound entries rather
  // than silently deleting adjacent projected messages.
  if (projected?.messages.length !== 1 || projected.messages[0].role !== "user") throw new Error("Pi deletion needs an exact single user entry");
  if (digest(await readFile(actual)) !== expectedDigest) throw new Error("Session changed during deletion");
  if (truncate) {
    // Edits live on the active branch's tip, often after the truncation target.
    // Rewinding must retain their latest effect on surviving source entries,
    // while dropping edits whose targets belong to the discarded future.
    const retained = new Set(projected.sourceEntry.parentId
      ? manager.getBranch(projected.sourceEntry.parentId).map(entry => entry.id) : []);
    const edits = new Map(manager.getBranch().filter(entry => entry.type === "context_edit")
      .map(entry => [entry.targetId, entry]));
    const carry = [...edits.values()].filter(entry => retained.has(entry.targetId) && !retained.has(entry.id));
    if (projected.sourceEntry.parentId) manager.branch(projected.sourceEntry.parentId);
    else manager.resetLeaf();
    for (const edit of carry) manager.appendContextEdit(edit.targetId, edit.replacement);
    // branch() only moves the in-memory leaf. Persist a native metadata entry,
    // so cold readers and the next runtime resume follow the selected branch.
    manager.appendSessionInfo(manager.getSessionName() ?? "");
  } else {
    manager.appendContextEdit(projected.sourceEntry.id, null);
  }
  const handle = await open(actual, "r+");
  try { await handle.sync(); } finally { await handle.close(); }
  const fresh = SessionManager.open(actual).buildSessionProjection();
  if (fresh.entries.some(entry => entry.sourceEntry.id === projected.sourceEntry.id && entry.messages.length)) throw new Error("Pi deletion did not persist; original source is backed up");
  process.stdout.write(JSON.stringify({ok:true, backupPath}));
} else if (exportPath) {
  const { SessionManager, buildSessionProjection } = await sessionManager();
  const { convertToLlm } = await installation.importModule("dist/core/messages.js");
  const manager = SessionManager.open(exportPath);
  const before = value("--before-message-id");
  let context;
  if (before === undefined) {
    context = manager.buildSessionContext();
  } else {
    const prefix = `pi:${manager.getSessionId()}:`;
    if (!before.startsWith(prefix)) throw new Error("Pi message belongs to another session");
    const parts = before.slice(prefix.length).split(":");
    if (parts.length !== 2 || !/^\d+$/.test(parts[1])) throw new Error("Invalid Pi message boundary");
    const [entryId, ordinalText] = parts;
    const ordinal = Number(ordinalText);
    const entries = manager.getEntries();
    const boundary = entries.findIndex(entry => entry.id === entryId);
    if (boundary < 0 || !manager.getBranch().some(entry => entry.id === entryId)) throw new Error("Pi user message is no longer on the active branch");
    const selected = entries[boundary];
    // Reconstruct the boundary with the active branch's latest edits to its
    // surviving entries. Those edits may have been appended in the future we
    // are discarding; replay only their effects, never future messages/tools.
    const projectAt = (leaf, end) => {
      const retained = new Set(leaf ? manager.getBranch(leaf).map(entry => entry.id) : []);
      const latest = new Map(manager.getBranch().filter(entry => entry.type === "context_edit")
        .map(entry => [entry.targetId, entry]));
      const prefix = entries.slice(0, end);
      for (const edit of latest.values()) {
        if (!retained.has(edit.targetId) || retained.has(edit.id)) continue;
        prefix.push({...edit, parentId:leaf});
        leaf = edit.id;
      }
      return buildSessionProjection(prefix, leaf);
    };
    const through = projectAt(selected.id, boundary + 1);
    const projected = through.entries.find(entry => entry.sourceEntry.id === entryId);
    if (projected?.messages[ordinal]?.role !== "user") throw new Error("Pi boundary is not a user message");
    if (ordinal === 0) {
      context = projectAt(selected.parentId, boundary);
    } else {
      const at = through.entries.indexOf(projected);
      context = {messages: through.entries.slice(0, at).flatMap(entry => entry.messages).concat(projected.messages.slice(0, ordinal))};
    }
  }
  const blocks = content => typeof content === "string" ? [{type:"text", text:content}] : content.flatMap(block => {
    switch (block.type) {
      case "text": return [{type:"text", text:block.text}];
      case "image": return [{type:"image", media_type:block.mimeType, data:block.data}];
      case "toolCall": return [{type:"tool_call", id:block.id, name:block.name, arguments:block.arguments}];
      case "thinking": return [];
      default: throw new Error(`Unsupported Pi context block: ${block.type}`);
    }
  });
  const messages = convertToLlm(context.messages).flatMap(message => {
    if (message.role === "system") return [];
    if (message.role === "toolResult") return [{role:"user",content:[{type:"tool_result",id:message.toolCallId,content:blocks(message.content),is_error:message.isError}]}];
    if (!["user", "assistant"].includes(message.role)) throw new Error(`Unsupported Pi context role: ${message.role}`);
    const content = blocks(message.content);
    return content.length ? [{role:message.role,content}] : [];
  });
  process.stdout.write(JSON.stringify({messages}));
} else if (mutationPath) {
  const { SessionManager } = await sessionManager();
  const { realpath, lstat, unlink } = await import("node:fs/promises");
  const { resolve, relative, isAbsolute } = await import("node:path");
  const root = await realpath(sessionDir ?? resolve(agentDir, "sessions"));
  const sessions = await SessionManager.listAll(sessionDir);
  if (!sessions.some(session => session.path === mutationPath)) throw new Error("Session not in configured source");
  const actual = await realpath(mutationPath);
  const within = relative(root, actual);
  const stat = await lstat(mutationPath);
  if (stat.isSymbolicLink() || stat.nlink !== 1 || !within || within.startsWith("../") || isAbsolute(within)) throw new Error("Session source escaped configured root");
  if (args.includes("--rename-session")) {
    let input = "";
    for await (const chunk of process.stdin) input += chunk;
    const {title} = JSON.parse(input);
    if (typeof title !== "string" || !title.trim()) throw new Error("Nonempty title required");
    const manager = SessionManager.open(mutationPath);
    manager.appendSessionInfo(title.trim());
    if (SessionManager.open(mutationPath).getSessionName() !== title.trim()) throw new Error("Session rename was not persisted");
  } else {
    // Pi's own selector uses unlink when trash is unavailable. Velune's native
    // confirmation explicitly describes permanent deletion of this source file.
    await unlink(mutationPath);
  }
  process.stdout.write(JSON.stringify({ok:true}));
} else
if (inspectPath) {
  const source = args.includes("--with-summary") ? await inspectSource(inspectPath) : null;
  const sdk = await sessionManager();
  const { SessionManager } = sdk;
  const manager = SessionManager.open(inspectPath);
  const entries = manager.getEntries();
  const name = manager.getSessionName() ?? null;
  const summary = source ? inspectedSummary(inspectPath, source, manager, entries, name) : undefined;
  const context = manager.buildSessionContext();
  const projected = projectPiDisplayHistory(manager, sdk.sessionEntryToContextMessages);
  const branch = manager.getBranch();
  const edits = new Map(branch.filter(entry => entry.type === "context_edit").map(entry => [entry.targetId, entry]));
  const deletedUser = branch.some(entry => entry.type === "message" && entry.message.role === "user"
    && edits.get(entry.id)?.replacement === null);
  const retainedReplyIds = [];
  if (deletedUser) {
    // The SDK's raw branch retains tombstoned prompts and therefore their
    // original turn boundaries. Only expose replies still present in its
    // authoritative effective projection, using the same native identities.
    const visibleReplies = new Set(projected.entries.flatMap(entry => entry.messages.flatMap((message, ordinal) =>
      message.role === "assistant" && Array.isArray(message.content)
        && message.content.at(-1)?.type === "text" && message.content.at(-1).text?.trim()
        ? [`pi:${manager.getSessionId()}:${entry.sourceEntry.id}:${ordinal}`] : [])));
    let closing;
    const close = () => {
      if (closing && visibleReplies.has(closing)) retainedReplyIds.push(closing);
      closing = undefined;
    };
    for (const entry of branch) {
      if (entry.type !== "message") continue;
      if (entry.message.role === "user") close();
      else if (entry.message.role === "assistant") {
        const last = Array.isArray(entry.message.content) ? entry.message.content.at(-1) : undefined;
        closing = last?.type === "text" && last.text?.trim()
          ? `pi:${manager.getSessionId()}:${entry.id}:0` : undefined;
      }
    }
    close();
  }
  const virtualEntry = [...manager.getBranch()]
    .reverse()
    .find((entry) => entry.type === "custom" && entry.customType === "pi.virtual-model-state");
  process.stdout.write(JSON.stringify({
    name,
    cwd: manager.getCwd(),
    summary,
    subAgents: await readSubagentMetadata(inspectPath, entries),
    retainedReplyIds,
    messages: projected.entries.flatMap(entry =>
      entry.messages.map((message, ordinal) => ({...message, id:`pi:${manager.getSessionId()}:${entry.sourceEntry.id}:${ordinal}`}))),
    model: context.model,
    thinkingLevel: context.thinkingLevel,
    virtualState: virtualEntry?.data ?? null,
  }));
} else {
  // Discovery mirrors SessionManager.listAll. Every file is reported with
  // its identity and, from its first line, the working directory and start
  // time it records, so the list can group and count without reading
  // sessions. Only the files the caller names are parsed, by Pi's own
  // metadata reader through a private directory of links, so Pi stays
  // authoritative for every listed field.
  const { readdir, stat, mkdtemp, symlink, link, copyFile, rm } = await import("node:fs/promises");
  const { dirname, basename } = await import("node:path");
  const { tmpdir } = await import("node:os");
  let input = "";
  for await (const chunk of process.stdin) input += chunk;
  const want = new Set(JSON.parse(input).want);
  const root = sessionsRoot();
  const directories = sessionDir
    ? [sessionDir]
    : await readdir(root, { withFileTypes: true })
        .then((entries) => entries.filter((entry) => entry.isDirectory()).map((entry) => join(root, entry.name)))
        .catch(() => []);
  const headers = new Map();
  const header = path => {
    if (!headers.has(path)) headers.set(path, readHeader(path).catch(() => null));
    return headers.get(path);
  };
  const parentPaths = new Map();
  // Recognize only the two native child layouts. A directory alone is not
  // ownership: every nested file must name this already discovered parent.
  const children = async (directory, depth = 0, parent = null, modern = false) => {
    if (depth > maxChildDepth || !(await lstat(directory).catch(() => null))?.isDirectory()) return [];
    const entries = await readdir(directory, { withFileTypes: true }).catch(() => []);
    const candidates = entries.filter(entry => entry.isFile()
      && (modern ? entry.name === "session.jsonl" : entry.name.endsWith(".jsonl")));
    const files = [];
    for (const entry of candidates) {
      const file = join(directory, entry.name);
      const start = await header(file);
      if (parent && (start?.parentSession !== parent || !hasSessionIdentity(start))) continue;
      files.push(file);
      if (parent) parentPaths.set(file, parent);
      if (!hasSessionIdentity(start)) continue;
      const stem = join(dirname(file), basename(file, ".jsonl"));
      if (!(await lstat(stem).catch(() => null))?.isDirectory()) continue;
      const forks = join(stem, "forks");
      if ((await lstat(forks).catch(() => null))?.isDirectory()) {
        const runs = await readdir(forks, { withFileTypes: true }).catch(() => []);
        for (const run of runs.filter(item => item.isDirectory())) {
          files.push(...await children(join(forks, run.name), depth + 1, file));
        }
      }
      // pi-subagents 0.80 native foreground/detached child session layout.
      const runs = await readdir(stem, { withFileTypes: true }).catch(() => []);
      for (const run of runs.filter(item => item.isDirectory() && runID.test(item.name))) {
        const directory = join(stem, run.name);
        const slots = await readdir(directory, { withFileTypes: true }).catch(() => []);
        for (const slot of slots.filter(item => item.isDirectory() && runSlot(item.name))) {
          files.push(...await children(join(directory, slot.name), depth + 1, file, true));
        }
      }
    }
    return files;
  };
  const paths = [...new Set((await Promise.all(directories.map(directory => children(directory)))).flat())];
  // Nanoseconds as a decimal string survive the JSON round trip exactly.
  const files = (await Promise.all(paths.map(async (path) => {
    try {
      const stats = await stat(path, { bigint: true });
      const start = await header(path);
      const created = typeof start?.timestamp === "string" ? Date.parse(start.timestamp) : NaN;
      return { path, sessionId: typeof start?.id === "string" ? start.id : null, size: String(stats.size), mtimeNs: String(stats.mtimeNs),
        modifiedAtUnixMs: Number(stats.mtimeMs),
        cwd: typeof start?.cwd === "string" && start.cwd ? start.cwd : null,
        parentSession: typeof start?.parentSession === "string" ? start.parentSession : null,
        createdAtUnixMs: Number.isFinite(created) ? created : null };
    } catch {
      return null;
    }
  }))).filter((file) => file !== null);
  const knownPaths = new Set(files.map(file => file.path));
  for (const file of files) {
    const parent = parentPaths.get(file.path);
    file.parentPath = parent && knownPaths.has(parent) ? parent : null;
  }
  const wanted = files.filter((file) => want.has(file.path));
  let sessions = [];
  if (wanted.length) {
    const { SessionManager } = await sessionManager();
    const links = await mkdtemp(join(tmpdir(), "velune-pi-list-"));
    // Windows grants symlinks only to administrators or in Developer Mode; a hard link
    // (same volume) or, failing that, a copy is read the same way.
    const stage = async (source, target) => {
      try {
        await symlink(source, target);
      } catch (error) {
        if (error?.code !== "EPERM") throw error;
        await link(source, target).catch(() => copyFile(source, target));
      }
    };
    try {
      await Promise.all(wanted.map((file, index) => stage(file.path, join(links, `${index}.jsonl`))));
      sessions = (await SessionManager.listAll(links)).map((session) => {
        const index = Number(session.path.slice(links.length + 1, -".jsonl".length));
        return {
          path: wanted[index].path,
          parentPath: wanted[index].parentPath,
          id: session.id,
          cwd: session.cwd,
          name: session.name ?? null,
          createdUnixMs: session.created.getTime(),
          modifiedUnixMs: session.modified.getTime(),
          messageCount: session.messageCount,
          firstMessage: session.firstMessage,
        };
      });
    } finally {
      await rm(links, { recursive: true, force: true });
    }
  }
  process.stdout.write(JSON.stringify({ contractVersion: 3, files, sessions }));
}
