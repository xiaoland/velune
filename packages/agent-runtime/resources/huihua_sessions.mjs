// @ts-check
// Read-only projection through public huihua APIs. Explicit roots are mandatory;
// native IDs are resolved by scanning those roots, never by opening caller paths.
import { codexProvider } from "huihua/providers/codex";
import { deepseekProvider } from "huihua/providers/deepseek";
import { isAbsolute, join } from "node:path";
import { createReadStream } from "node:fs";
import { createInterface } from "node:readline";

class RequestError extends Error {}

/** @typedef {import('huihua').SessionEvent} Event */
/** @typedef {{kind:'text'|'reasoning'|'notice',text:string}|{kind:'tool',toolID:string|null,title:string,state:'pending'|'running'|'completed'|'failed',output:string|null}} Block */
/** @typedef {{id:string,role:string,timestamp_unix_ms:number|null,blocks:Block[]}} Message */
/** @param {unknown} value @returns {string} */
function requiredString(value) {
  if (typeof value !== "string" || !value.trim()) throw new RequestError("Expected nonempty string");
  return value;
}
/** @param {import('huihua').Timestamp|undefined} value */
function timestamp(value) {
  if (!value) return null;
  const ms = value.format === "rfc3339" ? Date.parse(value.value) : value.value;
  return Number.isFinite(ms) ? ms : null;
}
/** @param {readonly import('huihua').ContentBlock[]} content @returns {Block[]} */
function contentBlocks(content) {
  return content.map(block => block.type === "text"
    ? {kind:"text",text:block.data}
    : {kind:"notice",text:block.type === "image" ? "图片" : block.type === "file" ? "文件" : "结构化内容"});
}
/** @param {Event} event @returns {Message|null} */
function message(event) {
  const id = `history:${event.sequence}:${event.type}`;
  switch (event.type) {
    case "user_message": return {id,timestamp_unix_ms:timestamp(event.timestamp),role:"user",blocks:contentBlocks(event.data.content)};
    case "assistant_message": return {id,timestamp_unix_ms:timestamp(event.timestamp),role:"assistant",blocks:contentBlocks(event.data.content)};
    case "reasoning": {
      const text = event.data.text ?? event.data.summary;
      return text ? {id,timestamp_unix_ms:timestamp(event.timestamp),role:"assistant",blocks:[{kind:"reasoning",text}]} : null;
    }
    case "tool_call": return {id,timestamp_unix_ms:timestamp(event.timestamp),role:"tool",blocks:[{kind:"tool",toolID:event.data.callId ?? null,title:event.data.toolName,state:"pending",output:null}]};
    case "tool_result": return {id,timestamp_unix_ms:timestamp(event.timestamp),role:"tool",blocks:[{kind:"tool",toolID:event.data.callId ?? null,title:event.data.toolName ?? "工具结果",state:event.data.isError ? "failed" : "completed",output:resultText(event.data.result)}]};
    case "error": return {id,timestamp_unix_ms:timestamp(event.timestamp),role:"system",blocks:[{kind:"notice",text:event.data.message ?? "运行时记录了错误"}]};
    case "file_change": return {id,timestamp_unix_ms:timestamp(event.timestamp),role:"tool",blocks:[{kind:"notice",text:`${event.data.operation}: ${event.data.path}`}]};
    case "command": return {id,timestamp_unix_ms:timestamp(event.timestamp),role:"tool",blocks:[{kind:"notice",text:"运行时执行了命令"}]};
    case "permission_request": return {id,timestamp_unix_ms:timestamp(event.timestamp),role:"system",blocks:[{kind:"notice",text:"历史权限请求"}]};
    case "subagent": return {id,timestamp_unix_ms:timestamp(event.timestamp),role:"system",blocks:[{kind:"notice",text:`${event.data.name ?? event.data.agentId}: ${event.data.kind}`}]};
    case "unknown": return null;
    case "system": case "usage": return null;
  }
}

/** @param {unknown} result @returns {string|null} */
function resultText(result) {
  if (typeof result === "string") return result;
  if (Array.isArray(result)) return result.map(part => typeof part === "string" ? part : part?.text ?? (part?.type === "text" ? part.data : "")).filter(Boolean).join("\n");
  if (result && typeof result === "object" && "content" in result) return resultText(result.content);
  return result == null ? null : JSON.stringify(result);
}

/** Codex 0.159.3 records initialization context before its first turn_context
 * boundary, separately from actual user input. This variant adapter also handles
 * duplicated runtime events and model wire context. Prefer the native
 * user event (which excludes injected context) and complete assistant item.
 * @param {readonly Event[]} events @param {string} providerId */
function presentationEvents(events, providerId) {
  if (providerId !== "codex") return events;
  const firstTurn = events.find(event => event.type === "system" && event.data.sourceType === "turn_context");
  const nativeUsers = events.some(event => event.type === "user_message" && event.providerMetadata.type === "event_msg");
  const assistantItems = events.some(event => event.type === "assistant_message" && event.providerMetadata.type === "response_item");
  return events.filter(event => !(firstTurn && event.type === "user_message" && event.record < firstTurn.record)
    && !(nativeUsers && event.type === "user_message" && event.providerMetadata.type !== "event_msg")
    && !(assistantItems && event.type === "assistant_message" && event.providerMetadata.type === "event_msg"));
}

/** Codex 0.159.3 persists explicit names outside rollout JSONL. The most
 * recent native index entry wins; an empty name clears the explicit title.
 * @param {string} home @returns {Promise<Map<string,string>>} */
async function codexNames(home) {
  const names = new Map();
  const input = createReadStream(join(home, "session_index.jsonl"));
  const lines = createInterface({input,crlfDelay:Infinity});
  try {
    for await (const line of lines) {
      if (!line.trim()) continue;
      if (line.length > 1024 * 1024) throw new RequestError("Native title index record is too large");
      let entry;
      // The native append-only index reader skips malformed records as well.
      try { entry = JSON.parse(line); } catch (error) { if (error instanceof SyntaxError) continue; throw error; }
      if (typeof entry.id === "string" && typeof entry.thread_name === "string") names.set(entry.id,entry.thread_name.trim());
    }
  } catch (error) {
    if (!error || typeof error !== "object" || !("code" in error) || error.code !== "ENOENT") throw error;
  } finally { lines.close(); input.destroy(); }
  return names;
}

async function main() {
  let input = "";
  for await (const chunk of process.stdin) input += chunk;
  /** @type {Record<string, unknown>} */
  const request = JSON.parse(input);
  const providerId = requiredString(request.provider);
  const provider = providerId === "codex" ? codexProvider : providerId === "deepseek" ? deepseekProvider : null;
  if (!provider) throw new RequestError("Unsupported history provider");
  const home = requiredString(request.home);
  if (!isAbsolute(home) || !Array.isArray(request.roots) || request.roots.length === 0) throw new RequestError("Explicit absolute home and roots are required");
  const roots = request.roots.map(requiredString);
  if (roots.some(root => !isAbsolute(root))) throw new RequestError("History roots must be absolute");
  const refs = (await provider.scan({homeDir:home,roots:{[provider.id]:roots}}))
    .filter(ref => ref.metadata.id_origin === "native");
  const nativeNames = providerId === "codex" ? await codexNames(home) : null;
  const explicitName = (/** @type {import('huihua').SessionRef} */ ref) => nativeNames ? nativeNames.get(ref.id) : ref.title;
  const summary = (/** @type {import('huihua').SessionRef} */ ref, /** @type {readonly Event[]} */ events = []) => {
    const name = explicitName(ref)?.trim();
    const first = events.find(event => event.type === "user_message");
    const firstText = first?.type === "user_message" ? first.data.content.filter(block=>block.type === "text").map(block=>block.data).join(" ") : "";
    return {nativeId:ref.id,title:name ? {source:"native",text:name} : firstText.trim() ? {source:"firstMessage",text:[...firstText.trim().replace(/\s+/g," ")].slice(0,80).join("")} : {source:"untitled"},updatedAtUnixMs:timestamp(ref.updatedAt),cwd:ref.workspace?.path ?? null};
  };
  if (request.operation === "list") {
    const sessions = [];
    for (const ref of refs) {
      if (explicitName(ref)?.trim()) sessions.push(summary(ref));
      else { const session = await provider.read(ref); sessions.push(summary(session,presentationEvents(session.events,providerId))); }
    }
    return {contractVersion:1,sessions};
  }
  if (request.operation !== "read") throw new RequestError("Unsupported history operation");
  const nativeId = requiredString(request.nativeId);
  const matches = refs.filter(ref => ref.id === nativeId);
  if (matches.length !== 1) throw new RequestError(matches.length ? "Ambiguous native session ID within configured roots" : "Native session not found in configured roots");
  const session = await provider.read(matches[0]);
  /** @type {Message[]} */
  const messages = [];
  /** @type {Map<string, Extract<Block,{kind:'tool'}>>} */
  const tools = new Map();
  for (const event of presentationEvents(session.events,providerId)) {
    const projected = message(event);
    if (!projected) continue;
    const tool = projected.blocks.find(block=>block.kind === "tool");
    if (tool?.kind === "tool" && tool.toolID) {
      const call = tools.get(tool.toolID);
      if (call) {
        if (event.type === "tool_result") { call.state = tool.state; call.output = tool.output; }
        continue;
      }
      tools.set(tool.toolID,tool);
    }
    messages.push(projected);
  }
  const codes = [...new Set(session.diagnostics.map(diagnostic => diagnostic.code))];
  if (codes.length) messages.push({id:"history:diagnostics",timestamp_unix_ms:null,role:"system",blocks:[{kind:"notice",text:`部分运行时记录未投影：${codes.join("、")}`}]});
  return {contractVersion:1,history:{...summary(session,presentationEvents(session.events,providerId)),messages}};
}
try { process.stdout.write(JSON.stringify(await main())); }
catch (error) {
  // No parser/raw payload is written to stderr: callers receive a bounded error.
  const detail = error instanceof RequestError ? error.message : "History provider could not read configured sessions";
  process.stderr.write(JSON.stringify({error:"history_read_failed",detail}) + "\n");
  process.exitCode = 1;
}
