// Opt-in native Pi tree semantics and Velune's before-mutation fallback.
// All sessions are synthetic and held in memory; no model is invoked.
import assert from "node:assert/strict";
import {mkdtemp, rm} from "node:fs/promises";
import {tmpdir} from "node:os";
import {join} from "node:path";
import {resolvePiSdk} from "../../resources/pi_sdk.mjs";
import install from "../../resources/pi_virtual_model.mjs";

const root = await mkdtemp(join(tmpdir(), "velune-pi-edit-sdk-"));
process.env.PI_CODING_AGENT_DIR = root;
delete process.env.VELUNE_PI_SELECTION_FILE;
const sdk = resolvePiSdk({binary:process.argv[2], runtimeTypeId:"pi-1.0"});
const {SessionManager} = await sdk.importModule("dist/core/session-manager.js");
const {AgentSession} = await sdk.importModule("dist/core/agent-session.js");
const commands = new Map();
install({registerCommand:(name, command) => commands.set(name, command), on:() => {},
  events:{on:() => () => {}, emit:() => {}}});
const user = content => ({role:"user", content, timestamp:Date.now()});
const reply = text => ({role:"assistant", content:[{type:"text", text}],
  api:"openai-completions", provider:"synthetic", model:"synthetic",
  usage:{input:0, output:0, cacheRead:0, cacheWrite:0, totalTokens:0,
    cost:{input:0, output:0, cacheRead:0, cacheWrite:0, total:0}},
  stopReason:"stop", timestamp:Date.now()});
const text = manager => manager.buildSessionContext().messages.map(message =>
  typeof message.content === "string" ? message.content : message.content.map(block => block.text).join(""));
// Exercise the installed SDK's actual navigateTree method, supplying only the
// non-model host hooks that its summarize:false path needs.
const navigate = (manager, target) => AgentSession.prototype.navigateTree.call({
  sessionManager:manager, isStreaming:false, isCompacting:false,
  _extensionRunner:{hasHandlers:() => false, emit:async () => {}},
  _refreshFinalizedContext:() => {}, _restoreToolsFromTranscript:() => {},
  _resolveIdleWaitIfIdle:() => {},
}, target, {summarize:false});
try {
  for (const mode of ["ordinary", "leaf", "carry"]) {
    const manager = SessionManager.inMemory(root);
    const first = manager.appendMessage(user("retained prompt"));
    const firstReply = manager.appendMessage(reply("retained reply"));
    const selected = manager.appendMessage(user("selected"));
    if (mode !== "leaf") manager.appendMessage(reply("discarded reply"));
    if (mode === "carry") {
      manager.appendContextEdit(first, null);
      manager.appendContextEdit(firstReply, {content:"retained reply edited"});
    }
    const originalLeaf = manager.getLeafId();
    if (mode !== "ordinary") {
      await navigate(manager, selected);
      if (mode === "leaf") assert.equal(manager.getLeafId(), selected, "SDK user-leaf navigation is a successful no-op");
      else assert.deepEqual(text(manager), ["retained prompt", "retained reply"], "SDK rewind loses later prefix edits");
      manager.branch(originalLeaf);
    }
    let outcome, navigations = 0;
    const before = text(manager);
    await commands.get("velune-rewind").handler(selected, {
      sessionManager:manager,
      navigateTree:async target => { navigations++; return navigate(manager, target); },
      ui:{setStatus:(_key, value) => { outcome = JSON.parse(value); }},
    });
    if (mode === "ordinary") {
      assert.equal(outcome.entryId, selected);
      assert.equal(navigations, 1);
      assert.equal(manager.getLeafId(), firstReply);
      assert.deepEqual(text(manager), ["retained prompt", "retained reply"]);
    } else {
      assert.equal(typeof outcome.fallback, "string");
      assert.equal(navigations, 0, "fallback must decline before selecting another leaf");
      assert.equal(manager.getLeafId(), originalLeaf);
      assert.deepEqual(text(manager), before);
    }
  }
  console.log("Pi official SDK: ordinary rewind, user-leaf no-op, later prefix edits, and mutation-free fallback passed");
} finally {
  await rm(root, {recursive:true, force:true});
}
