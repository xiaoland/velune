// Production extension + exact Host v1 frames; synthetic files, no model/CLI.
import assert from "node:assert/strict";
import {mkdtemp, mkdir, writeFile, rm, symlink} from "node:fs/promises";
import {tmpdir} from "node:os";
import {join} from "node:path";
import install from "../packages/agent-runtime/resources/pi_background_work.mjs";

const root = await mkdtemp(join(tmpdir(), "velune-background-tasks-"));
const listeners = new Map(), commands = new Map(), hooks = new Map(), frames = [], requests = [];
const events = {
  on(name, callback) { if (!listeners.has(name)) listeners.set(name, new Set()); listeners.get(name).add(callback); return () => listeners.get(name).delete(callback); },
  emit(name, value) { for (const callback of [...listeners.get(name) ?? []]) callback(value); },
};
const parent = join(root, "parent.jsonl"), artifacts = join(root, "delegate-task"), directory = join(artifacts, "child-session");
await mkdir(directory, {recursive: true});
const facts = {taskId:"delegate",childSessionId:"child",artifactDirAbs:artifacts,launchNonce:"launch",seedSha256:"seed"};
const manifest = {task_id:"delegate",child_session_id:"child",child_session_dir:directory,launch_nonce:"launch",seed_sha256:"seed"};
await writeFile(join(artifacts, "manifest.json"), JSON.stringify(manifest, null, 2));
let tasks = [
  {kind:"shell", task:{id:"shell-a",description:"Shell A",status:"running"}},
  {kind:"shell", task:{id:"shell-b",description:"Shell B",status:"running"}},
  {kind:"delegate", canReadResult:true, task:{id:"delegate",description:"Summarize results",status:"completed",delegate:facts}},
];
let failList = false, wrongTaskResponse = false, wrongSession = false, logBytes, logText = "bounded shell log";
events.on("subagents:rpc:v1:request", request => {
  const data = request.method === "ping" ? {capabilities:{asyncStatusSnapshot:{version:1}},methods:["stop"]} : {asyncSnapshot:{version:1,runs:[]}};
  events.emit(`subagents:rpc:v1:reply:${request.requestId}`, {version:1,requestId:request.requestId,method:request.method,success:true,data});
});
events.on("pi-background-tasks:host:request:v1", request => {
  requests.push({operation:request.operation,payload:request.payload});
  let result;
  if (request.operation === "capabilities") result = {api_version:1,operations:["subscribe","list","detail","logs","stop","result"]};
  else if (request.operation === "subscribe") result = {leaseId:"lease",activationId:"activation",sessionId:"parent"};
  else if (request.operation === "list") result = {activationId:"activation",sessionId:wrongSession ? "another-session" : "parent",revision:1,tasks};
  else if (request.operation === "detail") result = {task:tasks.find(task => task.task.id === (wrongTaskResponse ? "shell-b" : request.payload.taskId))};
  else if (request.operation === "logs") { logBytes = request.payload.maxBytes; result = {task:{id:request.payload.taskId},text:logText,truncated:true}; }
  else if (request.operation === "result") result = {taskId:request.payload.taskId,state:"committed",delivery:"inline",text:"Host-verified delegate answer"};
  else if (request.operation === "stop") {
    tasks = tasks.map(task => task.task.id === request.payload.taskId ? {...task,task:{...task.task,status:"killed"}} : task);
    result = {task:tasks.find(task => task.task.id === request.payload.taskId)?.task,alreadyTerminal:false};
  } else throw new Error(`Unexpected operation ${request.operation}`);
  events.emit("pi-background-tasks:host:response:v1", {schema_version:"pi-background-tasks.host-response.v1",request_id:request.request_id,operation:request.operation,ok:!(failList && request.operation === "list"),result,error:{code:"SYNTHETIC",message:"list unavailable"}});
});
install({events,registerCommand:(name, command) => commands.set(name, command),on:(name, handler) => { hooks.set(name, handler); return () => hooks.delete(name); }});
const context = {sessionManager:{getSessionFile:() => parent,getSessionId:() => "parent",getEntries:() => []},ui:{setStatus:(_key, text) => frames.push(JSON.parse(text))}};
const send = value => commands.get("velune-background-work").handler(Buffer.from(JSON.stringify(value)).toString("base64url"), context);
try {
  await send({operation:"snapshot"});
  assert.equal(frames.at(-1).tasks[2].kind, "delegate");
  assert.equal(frames.at(-1).tasks[2].childSessionId, "child");
  assert.equal(frames.at(-1).tasks[2].childSessionFile, undefined, "empty directory is not a conversation");
  const child = join(directory, "child.jsonl");
  await writeFile(child, JSON.stringify({type:"session",id:"child",parentSession:parent}) + "\n");
  await send({operation:"snapshot"}); assert.equal(frames.at(-1).tasks[2].childSessionFile, child);
  await writeFile(child, JSON.stringify({type:"session",id:"other-child",parentSession:parent}) + "\n");
  await send({operation:"snapshot"}); assert.equal(frames.at(-1).tasks[2].childSessionFile, undefined);
  await rm(child); await symlink(parent, child);
  await send({operation:"snapshot"}); assert.equal(frames.at(-1).tasks[2].childSessionFile, undefined);
  await rm(child);
  await send({operation:"task_output",taskId:"shell-a",requestId:"read-shell"});
  assert.equal(logBytes, 65536); assert.equal(frames.at(-1).taskOutput.requestId, "read-shell");
  assert.equal(frames.at(-1).taskOutput.text, "bounded shell log"); assert.equal(frames.at(-1).taskOutput.truncated, true);
  await send({operation:"task_output",taskId:"delegate",requestId:"read-delegate"});
  assert.equal(frames.at(-1).taskOutput.text, "Host-verified delegate answer");
  logText = "中".repeat(30_000); await send({operation:"task_output",taskId:"shell-a",requestId:"bounded-unicode"});
  assert.ok(Buffer.byteLength(frames.at(-1).taskOutput.text) <= 65536); assert.equal(frames.at(-1).taskOutput.truncated, true);
  wrongTaskResponse = true; await send({operation:"task_output",taskId:"shell-a",requestId:"wrong-task"});
  assert.match(frames.at(-1).taskOutput.error, /identity mismatch/); wrongTaskResponse = false;
  await send({operation:"stop_task",taskId:"shell-a"});
  assert.deepEqual(requests.filter(request => request.operation === "stop").map(request => request.payload.taskId), ["shell-a"]);
  assert.equal(frames.at(-1).tasks.find(task => task.id === "shell-b").status, "running");
  assert.equal(frames.at(-1).tasks.find(task => task.id === "shell-a").status, "killed");
  assert.ok(!requests.some(request => request.operation === "stop_all"));
  await send({operation:"stop_task",taskId:"unknown"}); assert.match(frames.at(-1).taskOperation.error, /cannot be stopped/);
  wrongSession = true; await send({operation:"snapshot"}); assert.equal(frames.at(-1).kind, "unknown");
  wrongSession = false; await send({operation:"snapshot"}); assert.equal(frames.at(-1).kind, "snapshot");
  failList = true; await send({operation:"snapshot"}); assert.equal(frames.at(-1).kind, "unknown");
  console.log("PASS Pi background Host: identity/kind, bounded logs, verified result, exact stop preserves siblings, missing/mismatched/symlink child never linked, unknown snapshot");
} finally {
  hooks.get("session_shutdown")?.(); await rm(root, {recursive:true,force:true});
}
