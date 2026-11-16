// Production Pi bridge, native-shaped records and synthetic files only.
import assert from "node:assert/strict";
import {mkdtemp, mkdir, writeFile, rm, symlink} from "node:fs/promises";
import {tmpdir} from "node:os";
import {join} from "node:path";
import install from "../packages/agent-runtime/resources/pi_background_work.mjs";
import {indexSubagentRuns, subagentSessions, readSubagentMetadata} from "../packages/agent-runtime/resources/pi_subagents.mjs";

const root = await mkdtemp(join(tmpdir(), "velune-pi-subagent-metadata-"));
const parent = join(root, "parent.jsonl");
const oldID = "11111111-1111-4111-8111-111111111111", resumedID = "22222222-2222-4222-8222-222222222222";
const child = join(root, "parent", oldID, "run-0", "session.jsonl");
await mkdir(join(root, "parent", oldID, "run-0"), {recursive:true});
await writeFile(child, JSON.stringify({type:"session",id:"native-child",parentSession:parent}) + "\n");
const entries = [];
function launch(id, args, suffix) {
  entries.push({id:`call-${suffix}`,type:"message",message:{role:"assistant",content:[{type:"toolCall",id:`tool-${suffix}`,name:"subagent",arguments:args}]}},
    {id:`result-${suffix}`,type:"message",message:{role:"toolResult",toolName:"subagent",toolCallId:`tool-${suffix}`,details:{runId:id,asyncId:id,results:[]}}});
}
launch(oldID, {agent:"worker",task:"合成子任务：保留角色与用途"}, "spawn");
let runs = [{id:oldID,kind:"subagent",label:"worker",state:"running",children:[{kind:"step",state:"running"}]}];
const listeners = new Map(), commands = new Map(), hooks = new Map(), frames = [], stops = [];
let broken = false, holdNext = false, held;
const bus = {
  on(name, fn) { if (!listeners.has(name)) listeners.set(name, new Set()); listeners.get(name).add(fn); return () => listeners.get(name).delete(fn); },
  emit(name, value) { for (const fn of [...listeners.get(name) ?? []]) fn(value); },
};
const reply = (request, data) => bus.emit(`subagents:rpc:v1:reply:${request.requestId}`, {version:1,requestId:request.requestId,method:request.method,success:true,data});
bus.on("subagents:rpc:v1:request", request => {
  if (request.method === "ping") return reply(request, {capabilities:{asyncStatusSnapshot:{version:1}},methods:["stop"]});
  if (request.method === "stop") { stops.push(request.params.id); return reply(request, {state:"stopping"}); }
  if (holdNext) { holdNext = false; held = request; return; }
  reply(request, {asyncSnapshot:{version:broken ? 99 : 1,runs}});
});
bus.on("pi-background-tasks:host:request:v1", request => {
  const result = request.operation === "capabilities" ? {api_version:1,operations:["subscribe","list"]}
    : request.operation === "subscribe" ? {leaseId:"lease",activationId:"activation"}
    : {activationId:"activation",revision:1,tasks:[{kind:"shell",task:{id:"independent-build",status:"running"}}]};
  bus.emit("pi-background-tasks:host:response:v1", {schema_version:"pi-background-tasks.host-response.v1",request_id:request.request_id,operation:request.operation,ok:true,result});
});
install({events:bus,getAllTools:() => [{name:"subagent"}],getCommands:() => [],
  registerCommand:(name, command) => commands.set(name, command),on:(name, hook) => { hooks.set(name, hook); return () => hooks.delete(name); }});
const context = {sessionManager:{getSessionFile:() => parent,getEntries:() => entries},ui:{setStatus:(_key,text) => frames.push(JSON.parse(text))}};
const send = value => commands.get("velune-background-work").handler(Buffer.from(JSON.stringify(value)).toString("base64url"), context);
async function until(condition) {
  const end = Date.now() + 1000;
  while (!condition() && Date.now() < end) await new Promise(resolve => setTimeout(resolve, 5));
  assert(condition(), "Synthetic event did not arrive");
}
try {
  await send({operation:"snapshot"});
  assert.equal(frames.at(-1).children[0].sessionFile, child);
  assert.match(frames.at(-1).children[0].title, /worker.*保留角色与用途/u);
  assert.match(frames.at(-1).subAgents[0].title, /worker.*保留角色与用途/u);

  launch(resumedID, {action:"resume",id:oldID,message:"合成续作：核对后续修改"}, "resume");
  runs = [{id:resumedID,kind:"subagent",label:"worker",state:"running",children:[{kind:"step",state:"running"}]}];
  await send({operation:"snapshot"});
  const current = frames.at(-1).children;
  assert.equal(current.length, 1);
  assert.equal(current[0].sessionFile, child, "Resume must retain the old transcript identity");
  assert.equal(current[0].runId, resumedID);
  assert.equal(current[0].canStop, true);
  assert.match(current[0].title, /worker.*核对后续修改/u);
  await send({operation:"stop_child",sessionFile:child});
  assert.deepEqual(stops, [resumedID], "Stop must address the active run, not its old directory UUID");
  assert.equal(frames.at(-1).tasks[0].id, "independent-build");

  broken = true;
  await send({operation:"snapshot"});
  assert.equal(frames.at(-1).childrenUnknown, true);
  assert.equal(frames.at(-1).children[0].sessionFile, child);
  assert.equal(frames.at(-1).children[0].canStop, false);
  broken = false;

  // The parent/run can report work after its sole child has completed.
  runs = [{id:resumedID,kind:"subagent",label:"worker",state:"running",children:[{kind:"step",state:"complete"}]}];
  await send({operation:"snapshot"});
  assert.deepEqual(frames.at(-1).children, []);
  assert.equal(frames.at(-1).childrenUnknown, false);
  assert.equal(frames.at(-1).subAgents.length, 1, "Historical identity is independent of activity");
  assert.equal(frames.at(-1).tasks[0].status, "running", "Ordinary background work remains independent");

  holdNext = true;
  const pending = send({operation:"snapshot"});
  await until(() => held);
  const late = held;
  const before = frames.length;
  runs = [];
  bus.emit("subagent:async-complete", {});
  await until(() => frames.length > before && frames.at(-1).children.length === 0);
  reply(late, {asyncSnapshot:{version:1,runs:[{id:resumedID,kind:"subagent",label:"worker",state:"running",children:[{state:"running"}]}]}});
  await pending;
  assert.deepEqual(frames.at(-1).children, [], "A late observation cannot resurrect completed work");

  // Session switches reuse the installed extension, not its parent-owned data.
  holdNext = true;
  held = undefined;
  const oldParentRead = send({operation:"snapshot"});
  await until(() => held);
  const oldParentReply = held;
  const parentB = join(root, "parent-b.jsonl");
  const contextB = {sessionManager:{getSessionFile:() => parentB,getEntries:() => []},ui:context.ui};
  hooks.get("session_start")({}, contextB);
  runs = [];
  await commands.get("velune-background-work").handler(Buffer.from('{"operation":"snapshot"}').toString("base64url"), contextB);
  assert.equal(frames.at(-1).parentSessionFile, parentB);
  assert.deepEqual(frames.at(-1).subAgents, []);
  reply(oldParentReply, {asyncSnapshot:{version:1,runs:[{id:resumedID,kind:"subagent",label:"worker",state:"running",children:[{state:"running"}]}]}});
  await oldParentRead;
  assert.equal(frames.at(-1).parentSessionFile, parentB);
  assert.deepEqual(frames.at(-1).subAgents, [], "An old parent response must not attach its children to the new parent");
  hooks.get("session_start")({}, context);
  await send({operation:"snapshot"});
  assert.equal(frames.at(-1).parentSessionFile, parent);
  assert.equal(frames.at(-1).subAgents.length, 1, "Returning to A must restore A's native identities without duplicates");

  // Also cover a switch without a B command: polling's continuation must
  // not stamp A's last background-task frame with B's owner.
  holdNext = true;
  held = undefined;
  bus.emit("subagent:child-status", {});
  await until(() => held);
  const stalePoll = held, beforeSwitch = frames.length;
  hooks.get("session_start")({}, contextB);
  reply(stalePoll, {asyncSnapshot:{version:1,runs:[]}});
  await new Promise(resolve => setTimeout(resolve, 0));
  assert.equal(frames.length, beforeSwitch, "A's delayed poll cannot publish for B before B has read its own state");
  hooks.get("session_start")({}, context);
  await send({operation:"snapshot"});

  const metadata = await readSubagentMetadata(parent, entries);
  assert.equal(metadata.length, 1);
  assert.match(metadata[0].title, /核对后续修改/u);
  const wrong = join(root, "parent", "33333333-3333-4333-8333-333333333333", "run-0", "session.jsonl");
  await mkdir(join(root, "parent", "33333333-3333-4333-8333-333333333333", "run-0"), {recursive:true});
  await writeFile(wrong, JSON.stringify({type:"session",id:"not-owned",parentSession:join(root,"different-parent.jsonl")}) + "\n");
  assert.deepEqual(await subagentSessions(parent, "33333333-3333-4333-8333-333333333333", indexSubagentRuns(entries)), []);
  const linkedID = "44444444-4444-4444-8444-444444444444";
  try {
    await symlink(join(root, "parent", oldID), join(root, "parent", linkedID), "dir");
    assert.deepEqual(await subagentSessions(parent, linkedID, new Map()), []);
  } catch (error) { if (!["EPERM", "EACCES"].includes(error.code)) throw error; }

  const parallelID = "55555555-5555-4555-8555-555555555555";
  launch(parallelID, {workflow:"parallel",args:{tasks:[{agent:"worker",task:"合成实现"},{agent:"reviewer",task:"合成审阅"}]}}, "parallel");
  const parallel = [];
  for (const index of [0, 1]) {
    const file = join(root, "parent", parallelID, `run-${index}`, "session.jsonl");
    await mkdir(join(root, "parent", parallelID, `run-${index}`), {recursive:true});
    await writeFile(file, JSON.stringify({type:"session",id:`parallel-${index}`,parentSession:parent}) + "\n");
    parallel.push(file);
  }
  runs = [{id:parallelID,kind:"workflow",label:"parallel",state:"running",children:[{state:"running"},{state:"running"}]}];
  await send({operation:"snapshot"});
  assert.equal(frames.at(-1).children.length, 2);
  assert.match(frames.at(-1).children.find(row => row.sessionFile === parallel[0]).title, /worker.*合成实现/u);
  assert.match(frames.at(-1).children.find(row => row.sessionFile === parallel[1]).title, /reviewer.*合成审阅/u);
  assert(frames.at(-1).children.every(row => !row.canStop), "A multi-child workflow is not a single-child stop target");
  entries.push({id:"parallel-notice",type:"custom_message",customType:"subagent-notify",details:{runs:parallel.map((file,index) => ({
    runId:parallelID,agent:index ? "reviewer" : "worker",status:index ? "completed" : "failed",sessionLabel:"Session file",sessionValue:file,
  }))}});
  const completedParallel = await readSubagentMetadata(parent, entries);
  assert.equal(completedParallel.find(row => row.sessionFile === parallel[0]).failed, true);
  assert.equal(completedParallel.find(row => row.sessionFile === parallel[1]).failed, false);
  console.log("PASS Pi subagent metadata: native roles/tasks, resumed transcript mapping, exact new-run stop, terminal children, independent background work, stale observation, parent switching, workflow steps, unknown retention and ownership checks");
} finally {
  hooks.get("session_shutdown")();
  await rm(root, {recursive:true,force:true});
}
