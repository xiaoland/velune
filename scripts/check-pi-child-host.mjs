#!/usr/bin/env node
// Real Pi + pi-subagents, private state and a loopback-only synthetic model.
import assert from "node:assert/strict";
import {spawn} from "node:child_process";
import {createServer} from "node:http";
import {mkdtemp, mkdir, writeFile, rm, readFile} from "node:fs/promises";
import {existsSync} from "node:fs";
import {tmpdir, homedir} from "node:os";
import {delimiter, join, resolve} from "node:path";
import {createRequire} from "node:module";
import {fileURLToPath, pathToFileURL} from "node:url";
import {resolvePiSdk} from "../packages/agent-runtime/resources/pi_sdk.mjs";

const repository = resolve(fileURLToPath(new URL("..", import.meta.url)));
const argument = key => {const index = process.argv.indexOf(key); return index < 0 ? undefined : process.argv[index + 1];};
const find = name => process.env.PATH?.split(delimiter).map(directory => join(directory, name)).find(existsSync);
const pi = argument("--pi") ?? process.env.VELUNE_TEST_PI_BINARY ?? find("pi");
const plugin = resolve(argument("--subagents") ?? join(homedir(), ".pi/agent/npm/node_modules/pi-subagents"));
assert(pi, "Pass --pi /absolute/path/to/pi");
const installation = resolvePiSdk({binary:resolve(pi), runtimeTypeId:"pi-1.0"});
// Direct extension registration below uses the same SDK-bound converter as
// the production RPC host.
process.env.VELUNE_PI_SDK_PACKAGE_ROOT = installation.packageDirectory;
const pluginVersion = JSON.parse(await readFile(join(plugin, "package.json"), "utf8")).version;
const root = await mkdtemp(join(tmpdir(), "velune-pi-child-host-"));
const agent = join(root, "agent"), workspace = join(root, "workspace"), privateTmp = join(root,"tmp");
await Promise.all([mkdir(join(agent,"agents"),{recursive:true}),mkdir(workspace),mkdir(privateTmp),mkdir(join(root,"home"))]);
let rpc, failure, passed = false;
const requests = [], events = [], waiters = [];
const toolCalls = [];
const text = content => typeof content === "string" ? content : (content ?? []).map(part => part.text ?? "").join("\n");
const server = createServer(async (request, response) => {
  try {
    const chunks = []; for await (const chunk of request) chunks.push(chunk);
    const body = JSON.parse(Buffer.concat(chunks).toString());
    assert.equal(body.model,"child-host-fixture", "virtual model must route to the selected physical model");
    const last = body.messages.at(-1);
    const prompt = text(last?.content);
    let call;
    if (last?.role === "user" && prompt.includes("NESTED_ROOT")) call = {agent:"middle",task:"NESTED_MIDDLE",model:"velune/auto:high",async:false};
    else if (last?.role === "user" && prompt.includes("NESTED_MIDDLE")) call = {agent:"leaf",task:"NESTED_LEAF",model:"velune/auto:high",async:false};
    else if (last?.role === "user" && prompt.includes("PARALLEL_ROOT")) call = {workflow:"parallel",args:{tasks:[{agent:"leaf",task:"PARALLEL_A",model:"velune/auto:high"},{agent:"leaf",task:"PARALLEL_B",model:"velune/auto:high"}]},async:false};
    else if (last?.role === "user" && prompt.includes("ASYNC_ROOT")) call = {agent:"leaf",task:"ASYNC_LEAF",model:"velune/auto:high",async:true};
    const kind = prompt.includes("NESTED_LEAF") ? "nested-leaf" : prompt.includes("NESTED_MIDDLE") ? "nested-middle" : prompt.includes("ASYNC_LEAF") ? "async-leaf" : prompt.includes("PARALLEL_A") ? "parallel-a" : prompt.includes("PARALLEL_B") ? "parallel-b" : "parent-or-continuation";
    requests.push({kind,model:body.model,toolCall:call?.agent ?? (call?.workflow ? "parallel" : null)});
    response.writeHead(200,{"Content-Type":"text/event-stream"});
    const envelope = {id:`fixture-${requests.length}`,object:"chat.completion.chunk",created:1,model:body.model};
    const delta = call ? {role:"assistant",tool_calls:[{index:0,id:`call-${requests.length}`,type:"function",function:{name:"subagent",arguments:JSON.stringify(call)}}]} : {role:"assistant",content:"SYNTHETIC_CHILD_OK"};
    response.write(`data: ${JSON.stringify({...envelope,choices:[{index:0,delta,finish_reason:null}]})}\n\n`);
    response.write(`data: ${JSON.stringify({...envelope,choices:[{index:0,delta:{},finish_reason:call ? "tool_calls" : "stop"}],usage:{prompt_tokens:1,completion_tokens:1,total_tokens:2}})}\n\n`);
    response.end("data: [DONE]\n\n");
  } catch (error) { failure = error; response.writeHead(500); response.end("synthetic fixture failure"); }
});
await new Promise(resolve => server.listen(0,"127.0.0.1",resolve));
const baseUrl = `http://127.0.0.1:${server.address().port}/v1`;
const models = join(root,"gateway.json"), selection = join(root,"selection.json");
await writeFile(models,JSON.stringify({providers:{"velune-gateway":{baseUrl,api:"openai-completions",apiKey:"VELUNE_GATEWAY_TOKEN",models:["child-host-fixture","child-host-next"].map(id=>({id,name:"Synthetic child host",reasoning:true,input:["text"],contextWindow:65536,maxTokens:4096,cost:{input:0,output:0,cacheRead:0,cacheWrite:0}}))}}}));
await writeFile(selection,JSON.stringify({modelRecordKey:"synthetic-child-host",physicalModelId:"child-host-fixture"}));
const probe = join(root,"registration-probe.mjs");
const pluginAPI = createRequire(join(plugin,"package.json")).resolve("pi-subagents/required-child-extensions");
await writeFile(probe, `import {registerRequiredChildExtensions as register} from ${JSON.stringify(pathToFileURL(pluginAPI).href)};
export default pi => {
  try {register({sessionId:"synthetic-independent",extensions:[]});} catch {}
  pi.registerCommand("velune-test-registration-disposal",{handler:(oldSessionId,ctx)=>{
    const previous = register({sessionId:oldSessionId.trim(),extensions:[]}); previous.dispose();
    let independentKept=false;
    try {register({sessionId:"synthetic-independent",extensions:[]});} catch {independentKept=true;}
    ctx.ui.setStatus("velune.test.child-disposal",JSON.stringify({oldDisposed:true,independentKept}));
  }});
};`);
await writeFile(join(agent,"settings.json"),JSON.stringify({extensions:[join(plugin,"index.js"),probe],packages:[],defaultThinkingLevel:"high"}));
await writeFile(join(agent,"agents/middle.md"),"---\nname: middle\ndescription: Synthetic nested child\nmodel: velune/auto:high\ntools: subagent\nallowNestedSubagents: true\nmaxSubagentDepth: 3\nextensions: []\nacceptance: { level: none, reason: synthetic model routing check }\n---\nFollow only the synthetic task.\n");
await writeFile(join(agent,"agents/leaf.md"),"---\nname: leaf\ndescription: Synthetic leaf\nmodel: velune/auto:high\ntools: read\nextensions: []\nacceptance: { level: none, reason: synthetic model routing check }\n---\nReturn the synthetic model response.\n");

const notify = event => {
  events.push(event);
  if (event.type === "tool_execution_end" && event.toolName === "subagent") toolCalls.push(event);
  for (let index=waiters.length-1;index>=0;index--) if (waiters[index].predicate(event)) waiters.splice(index,1)[0].resolve(event);
};
const wait = (predicate, timeout=30000) => new Promise((resolve,reject) => {
  const existing = events.find(predicate); if (existing) return resolve(existing);
  const entry={predicate,resolve:event=>{clearTimeout(deadline);resolve(event);}};
  const deadline=setTimeout(()=>{const index=waiters.indexOf(entry);if(index>=0)waiters.splice(index,1);reject(new Error("Timed out waiting for synthetic Pi observation"));},timeout);
  waiters.push(entry);
});
let requestID=0;
const send = async command => {
  const id=`child-host-${++requestID}`;
  rpc.stdin.write(JSON.stringify({...command,id})+"\n");
  const answer=await wait(event=>event.type==="response"&&event.id===id);
  assert.notEqual(answer.success,false,JSON.stringify(answer));return answer;
};
const delay = ms => new Promise(resolve=>setTimeout(resolve,ms));
const stopRPC = async () => {
  if(!rpc||rpc.exitCode!==null||rpc.signalCode!==null)return;
  const exited=new Promise(resolve=>rpc.once("exit",resolve));
  rpc.stdin.end();rpc.kill("SIGTERM");
  let deadline;
  await Promise.race([exited,new Promise(resolve=>{deadline=setTimeout(resolve,5000);})]);
  clearTimeout(deadline);
  if(rpc.exitCode===null&&rpc.signalCode===null){rpc.kill("SIGKILL");await exited;}
};
let stderr="";
try {
  // Exercise the real extension's route ownership directly: the parent's
  // explicit sync changes only that instance, not a previously launched child.
  const previousSelection = process.env.VELUNE_PI_SELECTION_FILE;
  const previousModels = process.env.VELUNE_PI_MODELS_PATH;
  process.env.VELUNE_PI_SELECTION_FILE=selection; process.env.VELUNE_PI_MODELS_PATH=models;
  const install = (await import("../packages/agent-runtime/resources/pi_virtual_model.mjs")).default;
  const instance = () => {
    let definition; const commands=new Map();
    install({on:()=>{},events:{on:()=>{},emit:()=>{}},registerCommand:(name,command)=>commands.set(name,command),
      registerProvider:()=>{},registerVirtualModel:value=>definition=value,getThinkingLevel:()=>"high",appendEntry:()=>{}});
    return {route:()=>definition.route({thinkingLevel:"high"},{modelRegistry:{find:(provider,id)=>({provider,id})}}).model.id,commands};
  };
  const parentRoute=instance(), childRoute=instance();
  await writeFile(selection,JSON.stringify({modelRecordKey:"synthetic-next",physicalModelId:"child-host-next"}));
  assert.equal(childRoute.route(),"child-host-fixture","an existing child followed the parent's mutable selection file");
  parentRoute.commands.get("velune-sync-selection").handler();
  assert.equal(parentRoute.route(),"child-host-next");
  assert.equal(childRoute.route(),"child-host-fixture");
  assert.equal(instance().route(),"child-host-next","a new child did not inherit the new route");
  await writeFile(selection,JSON.stringify({modelRecordKey:"synthetic-child-host",physicalModelId:"child-host-fixture"}));
  if(previousSelection===undefined)delete process.env.VELUNE_PI_SELECTION_FILE;else process.env.VELUNE_PI_SELECTION_FILE=previousSelection;
  if(previousModels===undefined)delete process.env.VELUNE_PI_MODELS_PATH;else process.env.VELUNE_PI_MODELS_PATH=previousModels;
  const env = Object.fromEntries(["PATH","LANG","LC_ALL"].filter(key=>process.env[key]).map(key=>[key,process.env[key]]));
  Object.assign(env,{HOME:join(root,"home"),TMPDIR:privateTmp+"/",PI_CODING_AGENT_DIR:agent,VELUNE_GATEWAY_TOKEN:"synthetic-only",XDG_CONFIG_HOME:join(root,"config"),XDG_DATA_HOME:join(root,"data"),XDG_CACHE_HOME:join(root,"cache")});
  const startRPC = () => {
    rpc=spawn(process.execPath,[join(repository,"packages/agent-runtime/resources/pi_rpc.mjs"),"--cli",resolve(pi),"--runtime-type","pi-1.0","--agent-dir",agent,"--models-path",models,"--selection-file",selection,"--extension",join(repository,"packages/agent-runtime/resources/pi_virtual_model.mjs"),"--provider","velune","--model","auto"],{cwd:workspace,env,stdio:["pipe","pipe","pipe"]});
    let pending="";rpc.stdout.setEncoding("utf8");rpc.stdout.on("data",chunk=>{pending+=chunk;for(;;){const index=pending.indexOf("\n");if(index<0)break;const line=pending.slice(0,index);pending=pending.slice(index+1);try{notify(JSON.parse(line));}catch{}}});
    rpc.stderr.setEncoding("utf8");rpc.stderr.on("data",chunk=>{stderr=(stderr+chunk).slice(-32000);});
  };
  startRPC();
  const initialState=await send({type:"get_state"});
  await wait(event=>event.type==="extension_ui_request"&&event.method==="setStatus"&&event.statusKey==="velune.child-capabilities"&&JSON.parse(event.statusText??"{}").requiredExtensions==="supported");
  const afterNested = events.length;
  await send({type:"prompt",message:"NESTED_ROOT"});
  await wait(event=>events.indexOf(event)>=afterNested&&event.type==="agent_end",60000);
  assert(!failure,failure?.message);
  assert(requests.some(item=>item.kind==="nested-middle"),"foreground child never reached the loopback model");
  assert(requests.some(item=>item.kind==="nested-leaf"),"nested child did not inherit the required virtual-model extension");
  assert(toolCalls.every(event=>!event.isError),"a synthetic subagent tool call failed");
  const afterParallel=events.length;
  await send({type:"prompt",message:"PARALLEL_ROOT"});
  await wait(event=>events.indexOf(event)>=afterParallel&&event.type==="agent_end",60000);
  assert(requests.some(item=>item.kind==="parallel-a")&&requests.some(item=>item.kind==="parallel-b"),"both parallel children must reach the loopback model");
  const parallel=toolCalls.find(event=>events.indexOf(event)>=afterParallel)?.result?.details?.results;
  assert.equal(parallel?.length,2);
  const afterAsync = events.length;
  await send({type:"prompt",message:"ASYNC_ROOT"});
  await wait(event=>events.indexOf(event)>=afterAsync&&event.type==="agent_end",60000);
  for(let attempt=0;attempt<120&&!requests.some(item=>item.kind==="async-leaf");attempt++) await delay(100);
  assert(requests.some(item=>item.kind==="async-leaf"),"detached child did not resolve the selected host SDK / virtual model");
  assert(toolCalls.every(event=>!event.isError),"async child launch failed");
  const asyncTask=toolCalls.find(event=>events.indexOf(event)>=afterAsync&&event.result?.details?.asyncDir)?.result.details;
  assert(asyncTask?.asyncDir?.startsWith(root+"/"),"async task is not owned by this fixture");
  let terminal;
  for(let attempt=0;attempt<100;attempt++) {
    try { terminal=JSON.parse(await readFile(join(asyncTask.asyncDir,"status.json"),"utf8")); } catch {}
    if(terminal?.processTerminal?.state==="observed")break;
    await delay(100);
  }
  assert(["complete","completed"].includes(terminal?.state),terminal?.error??"async child did not complete");
  assert.equal(terminal?.processTerminal?.state,"observed","runner process exit was not observed");
  const listing=await new Promise((resolve,reject)=>{
    const child=spawn(process.execPath,[join(repository,"packages/agent-runtime/resources/pi_sessions.mjs"),"--agent-dir",agent,"--cli",installation.cli,"--runtime-type","pi-1.0"],{env,stdio:["pipe","pipe","pipe"]});
    let output="",errors="";
    const deadline=setTimeout(()=>{child.kill("SIGTERM");reject(new Error("History discovery timed out"));},30000);
    child.stdout.setEncoding("utf8");child.stdout.on("data",chunk=>{output+=chunk;});
    child.stderr.setEncoding("utf8");child.stderr.on("data",chunk=>{errors+=chunk;});
    child.on("error",reject);child.on("close",code=>{clearTimeout(deadline);if(code!==0)return reject(new Error(errors));try{resolve(JSON.parse(output));}catch(error){reject(error);}});
    child.stdin.end(JSON.stringify({want:[]}));
  });
  const linked=listing.files.filter(file=>file.parentPath);
  assert(linked.length>=3,"real foreground/nested/detached child session layouts were not discovered");
  assert(linked.some(file=>linked.some(parent=>parent.path===file.parentPath)),"nested child ownership was lost");
  assert(linked.some(file=>file.path===terminal.steps[0].sessionFile&&file.parentPath===initialState.data.sessionFile));
  for(const child of parallel)assert(linked.some(file=>file.path===child.sessionFile),"parallel child missing from discovery: "+child.sessionFile);
  assert(!listing.files.some(file=>file.path.includes("subagent-artifacts")),"artifact transcript leaked into the conversation list");
  await send({type:"new_session"});
  await send({type:"prompt",message:"/velune-test-registration-disposal "+initialState.data.sessionId});
  await wait(event=>event.type==="extension_ui_request"&&event.statusKey==="velune.test.child-disposal"&&JSON.parse(event.statusText??"{}").oldDisposed&&JSON.parse(event.statusText).independentKept);
  await stopRPC();
  // A loaded older optional plugin without the public registration export
  // must not prevent ordinary Pi requests from running.
  const legacy=join(root,"legacy-subagents");await mkdir(legacy);
  await writeFile(join(legacy,"package.json"),JSON.stringify({name:"pi-subagents",type:"module",exports:{".":"./index.mjs"}}));
  await writeFile(join(legacy,"index.mjs"),"export default () => {};\n");
  await writeFile(join(agent,"settings.json"),JSON.stringify({extensions:[join(legacy,"index.mjs")],packages:[]}));
  const afterLegacy=events.length;startRPC();await send({type:"get_state"});
  await wait(event=>events.indexOf(event)>=afterLegacy&&event.type==="extension_ui_request"&&event.statusKey==="velune.child-capabilities"&&JSON.parse(event.statusText??"{}").requiredExtensions==="unavailable");
  const afterPlain=events.length;await send({type:"prompt",message:"PLAIN_ROOT"});
  await wait(event=>events.indexOf(event)>=afterPlain&&event.type==="agent_end");
  assert(requests.length>=9,"ordinary Pi did not reach the loopback model with the old optional plugin");
  if(failure)throw failure;
  const result={passed:true,piVersion:installation.version,subagentsVersion:pluginVersion,realModelCalls:0,loopbackRequests:requests,foregroundAndNestedVirtualRoute:true,parallelVirtualRouteAndDiscovery:true,detachedSdkHostAndVirtualRoute:true,childRouteIsolatedFromParentSelection:true,sessionRegistrationDisposalIsolated:true,asyncRunnerExitObserved:true,olderOptionalPluginPreservesOrdinaryPi:true,currentLayoutDiscovery:true,discoveredChildCount:linked.length};
  const output=resolve(argument("--output")??join(repository,"target/checks/pi-child-host"));await mkdir(output,{recursive:true});await writeFile(join(output,"result.json"),JSON.stringify(result,null,2)+"\n");
  console.log(JSON.stringify(result));
  passed = true;
} catch(error) {
  await writeFile(join(root,"events.json"),JSON.stringify(events,null,2));
  await writeFile(join(root,"requests.json"),JSON.stringify(requests,null,2));
  console.error(stderr);
  console.error("Synthetic fixture retained:",root);
  throw error;
} finally {
  await stopRPC();
  await new Promise(resolve=>server.close(resolve));
  // Keep failed state for exact-process diagnosis; no real state is involved.
  if(passed)await rm(root,{recursive:true,force:true});
}
