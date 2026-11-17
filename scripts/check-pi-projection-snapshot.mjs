// Synthetic-only check of the production extension. No SessionManager.open,
// model, user session, credentials, or native output path is accessed.
import assert from "node:assert/strict";
import {pathToFileURL} from "node:url";
import {resolve} from "node:path";
delete process.env.VELUNE_PI_SELECTION_FILE;
const moduleUrl = process.argv[2] ? pathToFileURL(resolve(process.argv[2])) : new URL("../packages/agent-runtime/resources/pi_virtual_model.mjs", import.meta.url);
const install = (await import(moduleUrl.href)).default;
const commands = new Map(), events = new Map(), statuses = [];
install({
  on(name, handler) { events.set(name, [...(events.get(name) ?? []), handler]); },
  registerCommand(name, command) { commands.set(name, command); },
  events:{on(){return () => {};},emit(){}},
});
const nativeUser = {role:"user",content:"synthetic user"};
let session = "A", file = "/synthetic/A.jsonl", calls = 0;
let entries = [
  {sourceEntry:{id:"u",type:"message",message:nativeUser},messages:[nativeUser]},
  {sourceEntry:{id:"c",type:"compaction"},messages:[{role:"system",content:"checkpoint"},{role:"compactionSummary",summary:"synthetic summary"}]},
  {sourceEntry:{id:"h",type:"custom_message"},messages:[{role:"custom",customType:"hidden-control",display:false,content:"hidden"}]},
];
const ctx = {sessionManager:{
  buildSessionProjection(){calls++; return {entries};},getSessionId:()=>session,getSessionFile:()=>file,
  getCwd:()=>"/synthetic",getSessionName:()=>session,getHeader:()=>({timestamp:"2026-01-01T00:00:00Z"}),
},ui:{setStatus(key,text){statuses.push({key,text});}}};
events.get("message_start").at(-1)({message:nativeUser},ctx);
const liveId = statuses.at(-1).text;
events.get("message_end").at(-1)({message:nativeUser},ctx);
const command = commands.get("velune-projection-sync");
command.handler("query-A",ctx);
const first = JSON.parse(statuses.at(-1).text);
assert.equal(calls,1);
assert.equal(first.snapshotVersion,1);
assert.equal(first.requestId,"query-A");
assert.equal(first.messages.length,4);
assert.deepEqual(first.messageIds,first.messages.map(m=>m.id));
assert.deepEqual(first.messageIds,["pi:A:u:0","pi:A:c:0","pi:A:c:1","pi:A:h:0"]);
assert.deepEqual(first.identityConfirmations,[{previousId:liveId,currentId:"pi:A:u:0"}]);
// Later appends / in-flight agent messages cannot mutate the serialized frame.
entries.push({sourceEntry:{id:"n",type:"custom_message"},messages:[{role:"custom",customType:"background-task-notification",content:"late"}]});
const divergentAgentMessages = [...entries.flatMap(e=>e.messages),{role:"assistant",content:"in flight"}];
assert.notEqual(divergentAgentMessages.length,first.messageIds.length);
assert.equal(first.messages.length,4);
command.handler("query-append",ctx);
assert.equal(JSON.parse(statuses.at(-1).text).messages.length,5);
// Same message count on another session/branch must still change each paired ID.
session="B";file="/synthetic/B.jsonl";
entries=[{sourceEntry:{id:"other",type:"message",message:{role:"user",content:"other"}},messages:[{role:"user",content:"other"},{role:"assistant",content:"branch answer"},{role:"system",content:"branch checkpoint"},{role:"compactionSummary",summary:"branch summary"}]}];
command.handler("query-B",ctx);
const branch=JSON.parse(statuses.at(-1).text);
assert.equal(branch.messages.length,first.messages.length);
assert.equal(branch.sessionFile,"/synthetic/B.jsonl");
assert.equal(branch.messages[0].content,"other");
assert.deepEqual(branch.messageIds,branch.messages.map(m=>m.id));
assert.ok(branch.messageIds.every(id=>id.startsWith("pi:B:other:")));
assert.deepEqual(first.messageIds,["pi:A:u:0","pi:A:c:0","pi:A:c:1","pi:A:h:0"]);
console.log(JSON.stringify({passed:6,source:moduleUrl.pathname,cases:["one-projection","compound-compaction","hidden-identities","live-confirmation","concurrent-append","same-length-session-branch"]}));
