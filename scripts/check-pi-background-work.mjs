import assert from 'node:assert/strict';
import {mkdtemp,mkdir,writeFile,rm,rename} from 'node:fs/promises';
import {tmpdir} from 'node:os';
import {join,resolve} from 'node:path';
import install from '../packages/agent-runtime/resources/pi_background_work.mjs';
const root=await mkdtemp(join(tmpdir(),'velune-pi-rpc-'));
const parent=join(root,'parent.jsonl');const child=join(root,'parent','forks','run-one','child.jsonl');
await mkdir(join(root,'parent','forks','run-one'),{recursive:true});
await writeFile(child,JSON.stringify({type:'session',id:'child',parentSession:parent})+'\n');
const listeners=new Map(), commands=new Map(), hooks=new Map(), calls=[], frames=[];
const events={on(name,fn){if(!listeners.has(name))listeners.set(name,new Set());listeners.get(name).add(fn);return()=>listeners.get(name).delete(fn)},emit(name,value){for(const fn of [...listeners.get(name)??[]]) fn(value)}};
let unknown=false,multiple=false,stopFailed=false,completed=false,runID='run-one';
let reportStartupStatus;
let backgroundTasks=[{task:{id:'shell',status:'running'}}];
const archiveEntries=[];
events.on('subagents:rpc:v1:request', req=>{
 calls.push({method:req.method,params:req.params});
 const data=req.method==='ping'?{capabilities:{asyncStatusSnapshot:{version:1}},methods:['stop']}:req.method==='status'?{asyncSnapshot:unknown?{version:99}:{version:1,runs:[{id:runID,kind:'subagent',state:completed?'completed':'running',children:multiple?[{},{}]:[{}]}]}}:{state:'stopping'};
 events.emit(`subagents:rpc:v1:reply:${req.requestId}`,{version:1,requestId:req.requestId,method:req.method,success:!(stopFailed && req.method==='stop'),data,error:{message:'synthetic stop rejected'}});
 if (req.method === 'status') reportStartupStatus?.();
});
events.on('pi-background-tasks:host:request:v1', req=>{
 const result=req.operation==='capabilities'?{api_version:1,operations:['subscribe','list']}:req.operation==='subscribe'?{leaseId:'lease',activationId:'activation'}:{activationId:'activation',revision:1,tasks:backgroundTasks};
 events.emit('pi-background-tasks:host:response:v1',{schema_version:'pi-background-tasks.host-response.v1',request_id:req.request_id,operation:req.operation,ok:true,result});
});
install({events,registerCommand:(name,value)=>commands.set(name,value),on:(name,fn)=>{hooks.set(name,fn);return()=>hooks.delete(name)}});
const ctx={sessionManager:{getSessionFile:()=>parent,getEntries:()=>archiveEntries},ui:{setStatus:(_key,text)=>frames.push(JSON.parse(text))}};
const send=async value=>commands.get('velune-background-work').handler(Buffer.from(JSON.stringify(value)).toString('base64url'),ctx);
// A child event can arrive before Core requests the initial host snapshot.
completed = true;
hooks.get('session_start')({}, ctx);
const startupStatus = new Promise(resolve => { reportStartupStatus = resolve; });
events.emit('subagent:child-status');
await startupStatus;
await new Promise(resolve => setImmediate(resolve));
assert.deepEqual(frames, [], 'child discovery must not publish an uninitialized host frame');
reportStartupStatus = undefined;
completed = false;
await send({operation:'snapshot'});assert.equal(frames.at(-1).children[0].canStop,true);assert.equal(frames.at(-1).tasks[0].id,'shell');
await send({operation:'stop_child',sessionFile:child});assert.deepEqual(calls.filter(call=>call.method==='stop'),[{method:'stop',params:{id:'run-one'}}]);assert.equal(frames.at(-1).children[0].state,'stopping');assert.equal(frames.at(-1).tasks[0].id,'shell');
await rename(child,child+'.hold');await send({operation:'snapshot'});assert.equal(frames.at(-1).childrenUnknown,true);assert.equal(frames.at(-1).children[0].sessionFile,child);assert.equal(frames.at(-1).children[0].canStop,false);await assert.rejects(send({operation:'stop_child',sessionFile:child}));await rename(child+'.hold',child);await send({operation:'snapshot'});assert.equal(frames.at(-1).childrenUnknown,false);
stopFailed=true;await assert.rejects(send({operation:'stop_child',sessionFile:child}));stopFailed=false;
multiple=true;await send({operation:'snapshot'});assert.equal(frames.at(-1).children[0].canStop,false);await assert.rejects(send({operation:'stop_child',sessionFile:child}));
unknown=true;await send({operation:'snapshot'});assert.equal(frames.at(-1).childrenUnknown,true);assert.equal(frames.at(-1).children[0].canStop,false);await assert.rejects(send({operation:'stop_child',sessionFile:child}));assert.equal(calls.filter(call=>call.method==='stop').length,2);
unknown=false;completed=true;await send({operation:'snapshot'});assert.equal(frames.at(-1).childrenUnknown,false);assert.deepEqual(frames.at(-1).children,[]);
backgroundTasks=['before-compaction','in-flight'].map(id=>({task:{id,status:'completed',notifyOnCompletion:true,notified:true}}));
archiveEntries.push({type:'custom_message',customType:'background-task-notification',details:{id:'before-compaction'}},
 {type:'compaction',summary:'A summary does not retain the old notification as an effective message'},
 {type:'custom',customType:'background-task-notification',details:{id:'not-a-delivery'}});
await send({operation:'snapshot'});
assert.deepEqual(frames.at(-1).persistedNotificationIds,['before-compaction']);
assert.equal(frames.at(-1).tasks.length,2);
archiveEntries.push({type:'custom_message',customType:'background-task-notification',details:{id:'in-flight'}});
await send({operation:'snapshot'});
assert.deepEqual(frames.at(-1).persistedNotificationIds,['before-compaction','in-flight']);
completed=false;multiple=false;runID='11111111-1111-4111-8111-111111111111';
const currentDirectory=join(root,'parent',runID,'run-0'), currentChild=join(currentDirectory,'session.jsonl');
await mkdir(currentDirectory,{recursive:true});await rename(child,currentChild);
for(const slot of ['run-00','run-4294967296']) { const directory=join(root,'parent',runID,slot);await mkdir(directory,{recursive:true});await writeFile(join(directory,'session.jsonl'),JSON.stringify({type:'session',id:slot,parentSession:parent})+'\n'); }
await send({operation:'snapshot'});
assert.equal(frames.at(-1).children.length,1);assert.equal(frames.at(-1).children[0].sessionFile,currentChild);assert.equal(frames.at(-1).children[0].canStop,true);
await send({operation:'stop_child',sessionFile:currentChild});assert.equal(frames.at(-1).children[0].state,'stopping');
await writeFile(currentChild,JSON.stringify({type:'session',id:'child',parentSession:join(root,'another-parent.jsonl')})+'\n');
await send({operation:'snapshot'});assert.equal(frames.at(-1).childrenUnknown,true);assert.equal(frames.at(-1).children[0].canStop,false);
hooks.get('session_shutdown')();await rm(root,{recursive:true});console.log('PASS native EventBus: exact single child stop, shell preserved, multi-child refused, unknown state retained, failed stop rejected, missing child file keeps pending metadata');
console.log('PASS pi-subagents run/step layout: exact parent header required; legacy forks layout retained');
console.log('PASS delivery receipts: SDK archive survives compaction; notified without a persisted message is not acknowledged');

// Initial failures from an installed responder must remain unknown and retry;
// only a silent, unregistered extension is classified as absent.
for (const initial of ['rejected', 'malformed', 'silent-installed', 'silent-registry-error', 'absent']) {
  const listeners = new Map(), commands = new Map(), hooks = new Map(), frames = [];
  let recovered = false, pings = 0;
  const bus = {
    on(name, fn) {
      if (!listeners.has(name)) listeners.set(name, new Set());
      listeners.get(name).add(fn);
      return () => listeners.get(name).delete(fn);
    },
    emit(name, value) { for (const fn of [...listeners.get(name) ?? []]) fn(value); },
  };
  bus.on('subagents:rpc:v1:request', request => {
    if (request.method === 'ping') pings++;
    if (!recovered && ['silent-installed', 'silent-registry-error', 'absent'].includes(initial)) return;
    const data = request.method === 'ping'
      ? (!recovered && initial === 'malformed' ? {} : {capabilities:{asyncStatusSnapshot:{version:1}},methods:['stop']})
      : {asyncSnapshot:{version:1,runs:[]}};
    bus.emit(`subagents:rpc:v1:reply:${request.requestId}`, {
      version:1, requestId:request.requestId, method:request.method,
      success:recovered || initial !== 'rejected', data, error:{message:'initial probe rejected'},
    });
  });
  bus.on('pi-background-tasks:host:request:v1', request => {
    const result = request.operation === 'capabilities' ? {api_version:1,operations:['subscribe','list']}
      : request.operation === 'subscribe' ? {leaseId:'lease',activationId:'activation'}
      : {activationId:'activation',revision:1,tasks:[]};
    bus.emit('pi-background-tasks:host:response:v1', {schema_version:'pi-background-tasks.host-response.v1',
      request_id:request.request_id, operation:request.operation,ok:true,result});
  });
  install({events:bus, getAllTools:() => { if(initial === 'silent-registry-error') throw new Error('registry unavailable'); return initial === 'absent' ? [] : [{name:'subagents_enable'}]; }, getCommands:() => [],
    registerCommand:(name, value) => commands.set(name, value), on:(name, fn) => { hooks.set(name, fn); return () => hooks.delete(name); }});
  const context = {sessionManager:{getSessionFile:() => undefined},ui:{setStatus:(_key,text) => frames.push(JSON.parse(text))}};
  const snapshot = () => commands.get('velune-background-work').handler(Buffer.from('{"operation":"snapshot"}').toString('base64url'), context);
  await snapshot();
  assert.equal(frames.at(-1).childrenUnknown, initial !== 'absent', initial);
  recovered = true;
  // Recovery must happen without a new turn, explicit snapshot command or
  // plugin event. Also prove an absent plugin is not repeatedly probed.
  const deadline = Date.now() + 2600;
  while (Date.now() < deadline && (initial === 'absent' || frames.at(-1).childrenUnknown)) {
    await new Promise(resolve => setTimeout(resolve, 25));
  }
  assert.equal(frames.at(-1).childrenUnknown, false, `${initial} recovered`);
  assert.equal(pings, initial === 'absent' ? 1 : 2, `${initial} probe count`);
  hooks.get('session_shutdown')();
}
console.log('PASS initial subagent detection: rejection, malformed capabilities and installed timeout recover; absent plugin settles');
