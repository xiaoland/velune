// Native pi-subagents launch identities. A resumed async run has a new run ID
// while retaining the old transcript; directory names alone are not run IDs.
import {lstat, open, readdir} from "node:fs/promises";
import {basename, dirname, isAbsolute, join, relative, sep} from "node:path";

const runID = value => typeof value === "string" && /^[a-zA-Z0-9_-]+$/u.test(value);
const uuid = value => /^[0-9a-f]{8}-[0-9a-f]{4}-[0-9a-f]{4}-[0-9a-f]{4}-[0-9a-f]{12}$/iu.test(value);
const slot = value => /^run-(0|[1-9][0-9]{0,9})$/u.test(value) && Number(value.slice(4)) <= 0xffffffff;
const text = value => typeof value === "string" ? value.trim() : "";
const bounded = value => Array.from(text(value).replace(/\s+/gu, " ")).slice(0, 240).join("");
const directory = async path => lstat(path).then(stat => stat.isDirectory() && !stat.isSymbolicLink()).catch(() => false);

export function indexSubagentRuns(entries) {
  const calls = new Map(), runs = new Map();
  const get = id => {
    if (!runID(id)) return undefined;
    if (!runs.has(id)) runs.set(id, {id, sessions:[], descriptions:new Map()});
    return runs.get(id);
  };
  for (const entry of entries ?? []) {
    if (!entry || typeof entry !== "object") continue;
    const message = entry.type === "message" ? entry.message : entry;
    if (message?.role === "assistant" && Array.isArray(message.content)) {
      for (const block of message.content) {
        if (block.type === "toolCall" && block.name === "subagent" && typeof block.id === "string") {
          calls.set(block.id, block.arguments ?? {});
        }
      }
    }
    if (message?.role === "toolResult" && message.toolName === "subagent") {
      const details = message.details ?? {}, run = get(details.asyncId ?? details.runId);
      if (!run) continue;
      const args = calls.get(message.toolCallId) ?? {};
      if (text(args.agent)) run.agent = text(args.agent);
      if (text(args.task)) run.task = text(args.task);
      if (args.workflow === "parallel" && Array.isArray(args.args?.tasks)) run.steps = args.args.tasks;
      if (args.action === "resume" && text(args.id ?? args.runId)) {
        run.sourceRunId = text(args.id ?? args.runId);
        if (Number.isInteger(args.index) && args.index >= 0) run.sourceIndex = args.index;
        if (text(args.message)) run.task = text(args.message);
      }
      for (const result of Array.isArray(details.results) ? details.results : []) {
        if (!result || typeof result !== "object") continue;
        if (typeof result.sessionFile === "string") {
          run.sessions.push(result.sessionFile);
          run.descriptions.set(result.sessionFile, {agent:text(result.agent),task:text(result.task)});
        }
      }
    }
    if (entry.customType === "subagent-notify" && Array.isArray(entry.details?.runs)) {
      for (const item of entry.details.runs) {
        const run = get(item.runId);
        if (!run) continue;
        if (typeof item.sessionValue === "string" && item.sessionLabel === "Session file") {
          run.sessions.push(item.sessionValue);
          run.descriptions.set(item.sessionValue, {agent:text(item.agent),task:text(item.task),failed:["failed", "error", "rejected"].includes(item.status)});
        }
        run.failed = ["failed", "error", "rejected"].includes(item.status);
      }
    }
  }
  return runs;
}

function sourceRun(run, runs) {
  if (!run?.sourceRunId) return undefined;
  if (runs.has(run.sourceRunId)) return runs.get(run.sourceRunId);
  const candidates = [...runs.values()].filter(item => item.id.startsWith(run.sourceRunId));
  return candidates.length === 1 ? candidates[0] : undefined;
}

export function subagentTitle(run, runs, fallback = "子 Agent", sessionFile) {
  let agent = "", task = "", current = run;
  const seen = new Set();
  while (current && !seen.has(current.id)) {
    seen.add(current.id);
    const descriptor = current.descriptions?.get(sessionFile);
    const index = typeof sessionFile === "string" ? /^run-(\d+)$/u.exec(basename(dirname(sessionFile)))?.[1] : undefined;
    const step = index === undefined ? undefined : current.steps?.[Number(index)];
    agent ||= text(current.agent) || text(descriptor?.agent) || text(step?.agent);
    task ||= text(current.task) || text(descriptor?.task) || text(step?.task);
    current = sourceRun(current, runs);
  }
  if (!agent && fallback !== "子 Agent") agent = text(fallback);
  return bounded(task ? `${agent ? `${agent}：` : ""}${task}` : agent || fallback);
}

async function verifiedChild(parent, path) {
  if (!isAbsolute(path)) return false;
  const stem = join(dirname(parent), basename(parent, ".jsonl"));
  const parts = relative(stem, path).split(sep);
  const modern = parts.length === 3 && uuid(parts[0]) && slot(parts[1]) && parts[2] === "session.jsonl";
  const legacy = parts.length === 3 && parts[0] === "forks" && runID(parts[1]) && parts[2].endsWith(".jsonl");
  if (!modern && !legacy || !await directory(stem)) return false;
  let ancestor = stem;
  for (const part of parts.slice(0, -1)) {
    ancestor = join(ancestor, part);
    if (!await directory(ancestor)) return false;
  }
  const stat = await lstat(path).catch(() => undefined);
  if (!stat?.isFile() || stat.isSymbolicLink()) return false;
  try {
    const file = await open(path, "r");
    try {
      const buffer = Buffer.alloc(65536), {bytesRead} = await file.read(buffer, 0, buffer.length, 0);
      const data = buffer.subarray(0, bytesRead).toString("utf8");
      if (!data.includes("\n")) return false;
      const header = JSON.parse(data.split("\n", 1)[0]);
      return header.type === "session" && text(header.id).length > 0 && header.parentSession === parent;
    } finally { await file.close(); }
  } catch { return false; }
}

export async function subagentSessions(parent, id, runs, seen = new Set()) {
  if (typeof parent !== "string" || !isAbsolute(parent) || !runID(id) || seen.has(id)) return [];
  seen.add(id);
  const run = runs.get(id), stem = join(dirname(parent), basename(parent, ".jsonl"));
  const candidates = [...(run?.sessions ?? [])], forks = join(stem, "forks");
  if (await directory(stem)) {
    if (await directory(forks) && await directory(join(forks, id))) {
      for (const entry of await readdir(join(forks, id), {withFileTypes:true})) {
        if (entry.isFile() && entry.name.endsWith(".jsonl")) candidates.push(join(forks, id, entry.name));
      }
    }
    const current = join(stem, id);
    if (uuid(id) && await directory(current)) {
      for (const entry of await readdir(current, {withFileTypes:true})) {
        if (entry.isDirectory() && slot(entry.name)) candidates.push(join(current, entry.name, "session.jsonl"));
      }
    }
  }
  const found = [];
  for (const path of new Set(candidates)) if (await verifiedChild(parent, path)) found.push(path);
  if (found.length) return found;
  const source = sourceRun(run, runs);
  if (!source) return [];
  const inherited = await subagentSessions(parent, source.id, runs, seen);
  // Without an exact step selector, a resume of a multi-session run cannot
  // turn its idle siblings into active children or be stopped as one child.
  if (run.sourceIndex !== undefined) return inherited.filter(path => basename(dirname(path)) === `run-${run.sourceIndex}`);
  return inherited.length === 1 ? inherited : [];
}

export async function readSubagentMetadata(parent, entries) {
  const runs = indexSubagentRuns(entries), found = new Map();
  for (const run of runs.values()) {
    try {
      for (const sessionFile of await subagentSessions(parent, run.id, runs)) {
        found.set(sessionFile, {sessionFile, title:subagentTitle(run, runs, "子 Agent", sessionFile), failed:run.descriptions.get(sessionFile)?.failed ?? run.failed === true});
      }
    } catch { /* Missing optional child metadata must not hide parent history. */ }
  }
  return [...found.values()];
}
