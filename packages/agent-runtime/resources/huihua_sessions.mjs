// @ts-check
// DSH read-only projection through public huihua APIs. Explicit roots are mandatory;
// native IDs are resolved by scanning those roots, never by opening caller paths.
import { deepseekProvider } from "huihua/providers/deepseek";
import { isAbsolute } from "node:path";

class RequestError extends Error {
  constructor(code) { super(code); this.code = code; }
}

/** @typedef {import('huihua').SessionEvent} Event */
/** @typedef {{kind:'text'|'reasoning'|'notice',text:string}|{kind:'tool',toolID:string|null,title:string,state:'pending'|'running'|'completed'|'failed',output:string|null}} Block */
/** @typedef {{id:string,role:string,timestamp_unix_ms:number|null,blocks:Block[]}} Message */
/** @param {unknown} value @returns {string} */
function requiredString(value) {
  if (typeof value !== "string" || !value.trim()) throw new RequestError("invalid_request");
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

async function main() {
  let input = "";
  for await (const chunk of process.stdin) input += chunk;
  /** @type {Record<string, unknown>} */
  const request = JSON.parse(input);
  const providerId = requiredString(request.provider);
  const provider = providerId === "deepseek" ? deepseekProvider : null;
  if (!provider) throw new RequestError("unsupported_provider");
  const home = requiredString(request.home);
  if (!isAbsolute(home) || !Array.isArray(request.roots) || request.roots.length === 0) throw new RequestError("invalid_history_roots");
  const roots = request.roots.map(requiredString);
  if (roots.some(root => !isAbsolute(root))) throw new RequestError("invalid_history_roots");
  const refs = (await provider.scan({homeDir:home,roots:{[provider.id]:roots}}))
    .filter(ref => ref.metadata.id_origin === "native");
  const summary = (/** @type {import('huihua').SessionRef} */ ref, /** @type {readonly Event[]} */ events = []) => {
    const name = ref.title?.trim();
    const first = events.find(event => event.type === "user_message");
    const firstText = first?.type === "user_message" ? first.data.content.filter(block=>block.type === "text").map(block=>block.data).join(" ") : "";
    const createdAtUnixMs = timestamp(ref.createdAt);
    return {nativeId:ref.id,title:name ? {source:"native",text:name} : firstText.trim() ? {source:"firstMessage",text:[...firstText.trim().replace(/\s+/g," ")].slice(0,80).join("")} : {source:"untitled"},updatedAtUnixMs:timestamp(ref.updatedAt),createdAtUnixMs,cwd:ref.workspace?.path ?? null};
  };
  if (request.operation === "list") {
    const sessions = [];
    const failures = [];
    for (const ref of refs) {
      // scan only knows header facts; read obtains the native last activity time.
      try {
        const session = await provider.read(ref);
        sessions.push(summary(session,session.events));
      } catch (error) {
        // One damaged or concurrently deleted native file must not hide every
        // other session. Native IDs are returned only as existing opaque IDs.
        failures.push({code:"provider_read",detail:error instanceof Error ? `${error.name}: ${error.message}\n${error.stack ?? ""}` : String(error)});
      }
    }
    return {contractVersion:1,sessions,failures};
  }
  if (request.operation !== "read") throw new RequestError("unsupported_operation");
  const nativeId = requiredString(request.nativeId);
  const matches = refs.filter(ref => ref.id === nativeId);
  if (matches.length !== 1) throw new RequestError(matches.length ? "ambiguous_session" : "session_not_found");
  const session = await provider.read(matches[0]);
  /** @type {Message[]} */
  const messages = [];
  /** @type {Map<string, Extract<Block,{kind:'tool'}>>} */
  const tools = new Map();
  for (const event of session.events) {
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
  return {contractVersion:1,history:{...summary(session,session.events),messages}};
}
try { process.stdout.write(JSON.stringify(await main())); }
catch (error) {
  // The application owns this local diagnostic data; preserve the underlying
  // parser/provider cause while keeping it inside the structured response.
  const code = error instanceof RequestError ? error.code : "provider_read";
  const detail = error instanceof Error ? `${error.name}: ${error.message}\n${error.stack ?? ""}` : String(error);
  process.stdout.write(JSON.stringify({contractVersion:1,error:{code,detail}}));
}
