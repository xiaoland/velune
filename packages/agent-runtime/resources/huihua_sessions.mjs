// @ts-check
// Read-only projection through public huihua APIs. Explicit roots are mandatory;
// native IDs are resolved by scanning those roots, never by opening caller paths.
import { codexProvider } from "huihua/providers/codex";
import { deepseekProvider } from "huihua/providers/deepseek";
import { isAbsolute } from "node:path";

class RequestError extends Error {}

/** @typedef {import('huihua').SessionEvent} Event */
/** @typedef {{kind:'text',text:string}|{kind:'notice',text:string}|{kind:'tool',toolID:string|null,title:string,state:string|null}} Block */
/** @typedef {{id:string,role:string,blocks:Block[]}} Message */
/** @param {unknown} value @returns {string} */
function requiredString(value) {
  if (typeof value !== "string" || !value.trim()) throw new RequestError("Expected nonempty string");
  return value;
}
/** @param {import('huihua').Timestamp|undefined} value */
function timestamp(value) {
  if (!value) return null;
  return value.format === "rfc3339" ? value.value : new Date(value.value).toISOString();
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
    case "user_message": return {id,role:"user",blocks:contentBlocks(event.data.content)};
    case "assistant_message": return {id,role:"assistant",blocks:contentBlocks(event.data.content)};
    case "reasoning": {
      const text = event.data.text ?? event.data.summary;
      return text ? {id,role:"assistant",blocks:[{kind:"notice",text}]} : null;
    }
    case "tool_call": return {id,role:"system",blocks:[{kind:"tool",toolID:event.data.callId ?? null,title:event.data.toolName,state:"running"}]};
    case "tool_result": return {id,role:"system",blocks:[{kind:"tool",toolID:event.data.callId ?? null,title:event.data.toolName ?? "工具结果",state:event.data.isError ? "failed" : "completed"}]};
    case "error": return {id,role:"system",blocks:[{kind:"notice",text:event.data.message ?? "运行时记录了错误"}]};
    case "file_change": return {id,role:"system",blocks:[{kind:"notice",text:`${event.data.operation}: ${event.data.path}`}]};
    case "command": return {id,role:"system",blocks:[{kind:"notice",text:"运行时执行了命令"}]};
    case "permission_request": return {id,role:"system",blocks:[{kind:"notice",text:"历史权限请求"}]};
    case "subagent": return {id,role:"system",blocks:[{kind:"notice",text:`${event.data.name ?? event.data.agentId}: ${event.data.kind}`}]};
    case "unknown": return null;
    case "system": case "usage": return null;
  }
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
  const summary = (/** @type {import('huihua').SessionRef} */ ref) => ({nativeId:ref.id,title:ref.title ?? ref.id,updatedAt:timestamp(ref.updatedAt),cwd:ref.workspace?.path ?? null});
  if (request.operation === "list") return {contractVersion:1,sessions:refs.map(summary)};
  if (request.operation !== "read") throw new RequestError("Unsupported history operation");
  const nativeId = requiredString(request.nativeId);
  const matches = refs.filter(ref => ref.id === nativeId);
  if (matches.length !== 1) throw new RequestError(matches.length ? "Ambiguous native session ID within configured roots" : "Native session not found in configured roots");
  const session = await provider.read(matches[0]);
  const messages = session.events.map(message).filter(item => item !== null);
  const codes = [...new Set(session.diagnostics.map(diagnostic => diagnostic.code))];
  if (codes.length) messages.push({id:"history:diagnostics",role:"system",blocks:[{kind:"notice",text:`部分运行时记录未投影：${codes.join("、")}`}]});
  return {contractVersion:1,history:{...summary(session),messages}};
}
try { process.stdout.write(JSON.stringify(await main())); }
catch (error) {
  // No parser/raw payload is written to stderr: callers receive a bounded error.
  const detail = error instanceof RequestError ? error.message : "History provider could not read configured sessions";
  process.stderr.write(JSON.stringify({error:"history_read_failed",detail}) + "\n");
  process.exitCode = 1;
}
