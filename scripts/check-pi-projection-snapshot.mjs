// Synthetic-only cold and live Pi display history check. The only session file
// is created under a private temporary directory and removed in finally.
import assert from "node:assert/strict";
import {execFileSync} from "node:child_process";
import {mkdir, mkdtemp, readFile, rm} from "node:fs/promises";
import {tmpdir} from "node:os";
import {join, resolve} from "node:path";
import {pathToFileURL} from "node:url";
import {resolvePiSdk} from "../packages/agent-runtime/resources/pi_sdk.mjs";

const cli = process.argv[2];
if (!cli || !resolve(cli)) throw new Error("Pass the absolute Pi CLI path");
const installation = resolvePiSdk({binary:cli,runtimeTypeId:"pi-1.0"});
process.env.VELUNE_PI_SDK_PACKAGE_ROOT = installation.packageDirectory;
delete process.env.VELUNE_PI_SELECTION_FILE;
const sdk = await installation.importModule("dist/core/session-manager.js");
const {SessionManager,CURRENT_SESSION_VERSION} = sdk;
if (typeof sdk.sessionEntryToContextMessages !== "function") throw new Error("Pi SDK entry conversion API unavailable");
const temp = await mkdtemp(join(tmpdir(), "velune-pi-display-history-"));
process.env.PI_CODING_AGENT_DIR = temp;
const stage = join(temp, "staged-resources");
const macBuild = await readFile("scripts/build-macos.sh","utf8");
const macList = macBuild.match(/for script in ([^;]+); do/)?.[1]?.trim().split(/\s+/);
const windowsBuild = await readFile("scripts/build-windows.ps1","utf8");
const windowsList = windowsBuild.match(/^\$scripts = (.+)$/m)?.[1]?.match(/'([^']+)'/g)?.map(value => value.slice(1,-1));
const remoteInstall = await readFile("packages/agent-runtime/src/execution.rs","utf8");
const remoteList = remoteInstall.match(/pub fn install_resources[\s\S]*?let names = \[([\s\S]*?)\];/)?.[1]?.match(/"([^"]+\.mjs)"/g)?.map(value => value.slice(1,-1).replace(/\.mjs$/,""));
assert.ok(macList?.includes("pi_transcript"),"macOS resource staging omitted the shared Pi helper");
assert.deepEqual(windowsList,macList,"Windows prune/copy list differs from macOS resource staging");
assert.deepEqual(remoteList,macList,"SSH resource list differs from packaged Pi resources");
const stagedScripts = macList;
await mkdir(stage);
for (const name of stagedScripts) {
  execFileSync("python3",["scripts/build-support.py","copy",
    `packages/agent-runtime/resources/${name}.mjs`,join(stage,`${name}.mjs`)],{cwd:resolve("."),stdio:"ignore"});
}
const moduleUrl = pathToFileURL(join(stage,"pi_virtual_model.mjs"));
try {
  const manager = SessionManager.create(temp, join(temp, "sessions"));
  manager.appendMessage({role:"user",content:"synthetic history before checkpoint",timestamp:Date.now()});
  const deleted = manager.appendMessage({role:"user",content:"synthetic deleted prompt",timestamp:Date.now()});
  const liveUser = {role:"user",content:"synthetic retained prompt",timestamp:Date.now()};
  const kept = manager.appendMessage(liveUser);
  const assistant = manager.appendMessage({role:"assistant",content:[{type:"text",text:"original answer"}],
    api:"openai-completions",provider:"synthetic",model:"synthetic",usage:{input:0,output:0,cacheRead:0,cacheWrite:0,totalTokens:0,
      cost:{input:0,output:0,cacheRead:0,cacheWrite:0,total:0}},stopReason:"stop",timestamp:Date.now()});
  const branchSummary = manager.branchWithSummary(assistant,"synthetic branch summary");
  manager.appendMessage({role:"system",content:"synthetic system checkpoint",timestamp:Date.now()});
  const compaction = manager.appendCompaction("synthetic compaction summary",kept,12);
  manager.appendContextEdit(deleted,null);
  manager.appendContextEdit(assistant,{content:"superseded replacement"});
  manager.appendContextEdit(assistant,{content:"edited answer"});
  const active = manager.appendMessage({role:"user",content:"active branch continuation",timestamp:Date.now()});
  manager.appendMessage({role:"user",content:"discarded future must stay hidden",timestamp:Date.now()});
  manager.branch(active);
  manager.appendSessionInfo("synthetic");

  // The model context remains Pi's normal compaction-aware projection.
  const modelContext = manager.buildSessionContext().messages;
  assert.ok(!modelContext.some(message => message.content === "synthetic deleted prompt"));

  const statuses = [], handlers = new Map(), eventHandlers = new Map();
  const install = (await import(moduleUrl.href)).default;
  install({
    on(name, handler) { eventHandlers.set(name,[...(eventHandlers.get(name) ?? []),handler]); },
    registerCommand(name, command) { handlers.set(name,command); },
    events:{on(){return () => {};},emit(){}},
  });
  const ctx = {sessionManager:manager,ui:{setStatus(key,text){statuses.push({key,text});}}};
  eventHandlers.get("message_start").at(-1)({message:liveUser},ctx);
  const liveId = statuses.at(-1).text;
  eventHandlers.get("message_end").at(-1)({message:liveUser},ctx);
  handlers.get("velune-projection-sync").handler("synthetic",ctx);
  const live = JSON.parse(statuses.at(-1).text);

  const cold = JSON.parse(execFileSync(process.execPath,[
    join(stage,"pi_sessions.mjs"),"--inspect-session",manager.getSessionFile(),
    "--agent-dir",temp,"--runtime-type","pi-1.0","--cli",cli,
  ],{cwd:resolve("."),encoding:"utf8",env:process.env}));
  assert.equal(live.snapshotVersion,1);
  assert.ok(live.messages.length > modelContext.length,
    `display history (${live.messages.length}) must retain pre-compaction branch messages (model context ${modelContext.length})`);
  assert.deepEqual(live.messages,cold.messages,"cold and live producers must share the same full display projection");
  assert.ok(live.messages.every(message => message.id.startsWith(`pi:${manager.getSessionId()}:`)));
  assert.equal(live.messages.some(message => message.content === "synthetic deleted prompt"),false);
  assert.ok(live.messages.some(message => message.content === "synthetic history before checkpoint"));
  assert.equal(live.messages.some(message => JSON.stringify(message).includes("discarded future must stay hidden")),false);
  assert.ok(live.messages.some(message => JSON.stringify(message).includes("edited answer")));
  assert.equal(live.messages.some(message => JSON.stringify(message).includes("original answer")),false);
  assert.equal(live.messages.some(message => JSON.stringify(message).includes("superseded replacement")),false);
  assert.ok(live.messages.some(message => JSON.stringify(message).includes("synthetic branch summary")));
  assert.ok(live.messages.some(message => JSON.stringify(message).includes("synthetic compaction summary")));
  assert.ok(live.messages.some(message => message.id === `pi:${manager.getSessionId()}:${branchSummary}:0`));
  assert.ok(live.messages.some(message => message.id === `pi:${manager.getSessionId()}:${compaction}:1`));
  assert.deepEqual(live.identityConfirmations,[{previousId:liveId,currentId:`pi:${manager.getSessionId()}:${kept}:0`}]);
  assert.equal(CURRENT_SESSION_VERSION,manager.getHeader().version);
  assert.deepEqual(manager.buildSessionContext().messages,modelContext);
  console.log(JSON.stringify({passed:11,cases:["full-branch-before-compaction","context-edit-delete","context-edit-replacement",
    "compaction-and-branch-summary","source-entry-ordinal-identity","discarded-branch-exclusion","stable-live-identity",
    "cold-live-producer-equivalence","model-context-unchanged","current-sdk-api","staged-resource-import"]}));
} finally {
  await rm(temp,{recursive:true,force:true});
  delete process.env.VELUNE_PI_SDK_PACKAGE_ROOT;
}
