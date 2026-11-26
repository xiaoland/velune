// Opt-in production RPC entry with an installed SDK, a loopback model, and a
// private synthetic Pi home. Runs with the installed Pi CLI's absolute path.
import assert from "node:assert/strict";
import { spawn } from "node:child_process";
import { once } from "node:events";
import { mkdtemp, mkdir, rm, writeFile } from "node:fs/promises";
import { createServer } from "node:http";
import { tmpdir } from "node:os";
import { dirname, join, delimiter } from "node:path";
import { createInterface } from "node:readline";
import { fileURLToPath } from "node:url";

const cli = process.argv[2];
assert.ok(cli, "pass the installed Pi CLI path");
const root = await mkdtemp(join(tmpdir(), "velune-pi-notification-cancel-"));
const agent = join(root, "agent"), workspace = join(root, "workspace");
const resources = fileURLToPath(new URL("../../resources/", import.meta.url));
let modelRequests = 0;
const firstModelRequest = Promise.withResolvers();
const server = createServer(async (request, response) => {
  for await (const _ of request) { /* Consume the synthetic model request. */ }
  // The first turn remains active until native abort cancels its request.
  if (++modelRequests === 1) {
    firstModelRequest.resolve();
    return;
  }
  response.writeHead(200, { "content-type": "text/event-stream" });
  const chunk = { id:"synthetic", object:"chat.completion.chunk", created:1, model:"alpha" };
  for (const choice of [
    { delta:{ role:"assistant", content:"synthetic response" }, finish_reason:null },
    { delta:{}, finish_reason:"stop" },
  ]) response.write(`data: ${JSON.stringify({...chunk, choices:[{index:0,...choice}]})}\n\n`);
  response.end("data: [DONE]\n\n");
});
let child, exited, timeout;
try {
  await mkdir(join(agent, "extensions"), { recursive:true });
  await mkdir(workspace);
  server.listen(0, "127.0.0.1");
  await once(server, "listening");
  await writeFile(join(agent, "models.json"), JSON.stringify({ providers:{ synthetic:{
    baseUrl:`http://127.0.0.1:${server.address().port}/v1`, api:"openai-completions", apiKey:"synthetic",
    models:[{ id:"alpha", name:"Synthetic", reasoning:false, input:["text"],
      cost:{input:0,output:0,cacheRead:0,cacheWrite:0}, contextWindow:32768, maxTokens:4096 }],
  }} }));
  await writeFile(join(agent, "extensions", "synthetic-notifications.ts"), `
export default function(pi) {
  const notify = (id, deliverAs, triggerTurn = true) => pi.sendMessage({
    customType:"background-task-notification", content:id, display:true, details:{id},
  }, {deliverAs,triggerTurn});
  pi.registerCommand("synthetic-notifications", {handler(args) {
    if (args === "queued") {
      notify("cleared-steer", "steer");
      notify("cleared-first", "followUp");
      notify("cleared-second", "followUp");
      notify("deferred", "followUp", false);
    } else {
      notify("later", "followUp");
    }
  }});
}
`);
  child = spawn(process.execPath, [join(resources, "pi_rpc.mjs"), "--cli", cli,
    "--runtime-type", "pi-1.0", "--agent-dir", agent, "--extension", join(resources, "pi_virtual_model.mjs"),
    "--provider", "synthetic", "--model", "alpha"], {
    cwd:workspace, env:{HOME:root, USERPROFILE:root, PATH:[dirname(process.execPath), "/usr/bin", "/bin"].join(delimiter)},
    stdio:["pipe", "pipe", "inherit"],
  });
  exited = once(child, "exit");
  timeout = setTimeout(() => child.kill("SIGKILL"), 15000);
  const lines = createInterface({ input:child.stdout })[Symbol.asyncIterator]();
  const events = [];
  const observe = async (match) => {
    const known = events.find(match);
    if (known) return known;
    for (let line = await lines.next(); !line.done; line = await lines.next()) {
      const record = JSON.parse(line.value);
      events.push(record);
      if (match(record)) return record;
    }
    throw new Error(`Pi exited before the expected RPC record. Saw ${events.map(event => event.command ?? event.type).join(", ")}`);
  };
  let nextId = 0;
  const rpc = async (type, fields = {}) => {
    const id = String(++nextId);
    child.stdin.write(JSON.stringify({id,type,...fields}) + "\n");
    const response = await observe(record => record.type === "response" && record.id === id);
    assert.equal(response.success, true, JSON.stringify(response));
    return response.data;
  };
  await rpc("prompt", {message:"run the held synthetic request"});
  await observe(record => record.type === "agent_start");
  await Promise.race([firstModelRequest.promise, exited.then(() => {
    throw new Error("Pi exited before calling the synthetic model");
  })]);
  await rpc("steer", {message:"queued user steering"});
  await rpc("follow_up", {message:"queued user follow-up"});
  await rpc("prompt", {message:"/synthetic-notifications queued"});
  const before = await rpc("get_state");
  assert.deepEqual(await rpc("clear_queue"), {
    steering:["queued user steering"], followUp:["queued user follow-up"],
    cancelledBackgroundNotificationIds:["cleared-steer", "cleared-first", "cleared-second"],
  });
  const after = await rpc("get_state");
  assert.equal(after.steeringMode, before.steeringMode);
  assert.equal(after.followUpMode, before.followUpMode);
  await rpc("abort");
  await rpc("prompt", {message:"/synthetic-notifications later"});
  await observe(record => record.type === "agent_settled" && record.aborted === false);
  const {messages} = await rpc("get_messages");
  assert.deepEqual(messages.filter(m => m.customType === "background-task-notification").map(m => m.details.id),
    ["deferred", "later"]);
  assert.equal(modelRequests, 2, "the later notification must trigger its own model turn");
  assert.deepEqual((await rpc("clear_queue")).cancelledBackgroundNotificationIds, []);
  child.stdin.end();
  assert.deepEqual(await exited, [0,null]);
  console.log("Pi native clear_queue cancelled only removed notices and preserved later delivery");
} finally {
  clearTimeout(timeout);
  if (child?.exitCode === null && child.signalCode === null) child.kill("SIGKILL");
  await exited;
  server.closeAllConnections();
  server.close();
  await rm(root, {recursive:true,force:true});
}
