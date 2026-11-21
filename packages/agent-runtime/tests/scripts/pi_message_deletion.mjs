// Opt-in official-SDK integration check. It opens only sessions created below
// this private temporary root; no installed user's session is inspected.
import assert from 'node:assert/strict';
import {mkdtemp, readFile, writeFile, readdir, rm, mkdir} from 'node:fs/promises';
import {tmpdir} from 'node:os';
import {join, resolve} from 'node:path';
import {createHash} from 'node:crypto';
import {spawnSync} from 'node:child_process';
import {resolvePiSdk} from '../../resources/pi_sdk.mjs';
const cli = process.argv[2];
if (!cli) throw new Error('Pass the absolute installed Pi CLI path');
const root = await mkdtemp(join(tmpdir(), 'velune-pi-delete-sdk-'));
const sessions = join(root, 'sessions');
await mkdir(sessions);
const {SessionManager} = await resolvePiSdk({binary:cli,runtimeTypeId:'pi-1.0'}).importModule('dist/core/session-manager.js');
const helper = resolve('packages/agent-runtime/resources/pi_sessions.mjs');
try {
  for (const [truncate, empty] of [[false,false], [true,false], [true,true]]) {
    const manager = SessionManager.create(root, sessions);
    const user = text => ({role:'user',content:text,timestamp:Date.now()});
    const assistant = text => ({role:'assistant',content:[{type:'text',text}],api:'openai-completions',provider:'synthetic',model:'synthetic',usage:{input:1,output:1,cacheRead:0,cacheWrite:0,totalTokens:2,cost:{input:0,output:0,cacheRead:0,cacheWrite:0,total:0}},stopReason:'stop',timestamp:Date.now()});
    const first = manager.appendMessage(user('first'));
    const firstReply = manager.appendMessage(assistant('first reply'));
    const second = manager.appendMessage(user('second'));
    const secondReply = manager.appendMessage(assistant('second reply'));
    const file = manager.getSessionFile();
    const original = await readFile(file);
    const digest = bytes => createHash('sha256').update(bytes).digest('hex');
    const run = expectedDigest => spawnSync(process.execPath,[helper,'--agent-dir',root,'--session-dir',sessions,'--cli',cli,'--runtime-type','pi-1.0','--delete-message',file], {encoding:'utf8',input:JSON.stringify({messageId:`pi:${manager.getSessionId()}:${truncate && !empty ? second : first}:0`,truncate,expectedDigest,backupDirectory:join(root,'backups')})});
    const stale = run('0'.repeat(64));
    assert.notEqual(stale.status,0);
    assert.deepEqual(await readFile(file),original);
    const result = run(digest(original));
    assert.equal(result.status,0,result.stderr);
    const backup = JSON.parse(result.stdout).backupPath;
    assert.deepEqual(await readFile(backup),original);
    const cold = SessionManager.open(file);
    const text = cold.buildSessionProjection().entries.flatMap(entry => entry.messages).map(message => typeof message.content === 'string' ? message.content : message.content.filter(block => block.type === 'text').map(block => block.text).join(''));
    assert.deepEqual(text,truncate ? (empty ? [] : ['first','first reply']) : ['first reply','second','second reply']);
    assert.equal(cold.getSessionId(),manager.getSessionId());
    const inspected = spawnSync(process.execPath,[helper,'--agent-dir',root,'--session-dir',sessions,'--cli',cli,'--runtime-type','pi-1.0','--inspect-session',file], {encoding:'utf8'});
    assert.equal(inspected.status,0,inspected.stderr);
    const saved = JSON.parse(inspected.stdout);
    assert.deepEqual(saved.retainedReplyIds, truncate ? [] : [firstReply,secondReply].map(id => `pi:${manager.getSessionId()}:${id}:0`));
    assert.equal(new Set(saved.retainedReplyIds).size,saved.retainedReplyIds.length);
  }
  // Branch-local edits written after the selected boundary must survive a
  // later rewind when their targets survive. Future targets must not follow.
  {
    const manager = SessionManager.create(root, sessions);
    const user = text => ({role:'user',content:text,timestamp:Date.now()});
    const assistant = text => ({role:'assistant',content:[{type:'text',text}],api:'openai-completions',provider:'synthetic',model:'synthetic',usage:{input:1,output:1,cacheRead:0,cacheWrite:0,totalTokens:2,cost:{input:0,output:0,cacheRead:0,cacheWrite:0,total:0}},stopReason:'stop',timestamp:Date.now()});
    const first = manager.appendMessage(user('first'));
    const firstReply = manager.appendMessage(assistant('first reply'));
    const second = manager.appendMessage(user('second'));
    const secondReply = manager.appendMessage(assistant('second reply'));
    const third = manager.appendMessage(user('third'));
    manager.appendMessage(assistant('third reply'));
    const fourth = manager.appendMessage(user('fourth'));
    manager.appendMessage(assistant('fourth reply'));
    manager.appendContextEdit(first,{content:'older replacement'});
    manager.appendContextEdit(first,{content:'first edited'});
    manager.appendContextEdit(firstReply,{content:[{type:'text',text:'first reply edited'}]});
    manager.appendContextEdit(fourth,{content:'future edit must disappear'});
    const file = manager.getSessionFile();
    const run = async (entry,truncate) => {
      const original = await readFile(file);
      const result = spawnSync(process.execPath,[helper,'--agent-dir',root,'--session-dir',sessions,'--cli',cli,'--runtime-type','pi-1.0','--delete-message',file], {encoding:'utf8',input:JSON.stringify({messageId:`pi:${manager.getSessionId()}:${entry}:0`,truncate,expectedDigest:createHash('sha256').update(original).digest('hex'),backupDirectory:join(root,'backups')})});
      assert.equal(result.status,0,result.stderr);
      assert.deepEqual(await readFile(JSON.parse(result.stdout).backupPath),original);
    };
    await run(second,false);
    // Editing a later message must export the current effective prefix,
    // including edits and deletions appended after the historical boundary.
    const exported = spawnSync(process.execPath,[helper,'--agent-dir',root,'--session-dir',sessions,'--cli',cli,'--runtime-type','pi-1.0','--export-context',file,'--before-message-id',`pi:${manager.getSessionId()}:${third}:0`],{encoding:'utf8'});
    assert.equal(exported.status,0,exported.stderr);
    assert.deepEqual(JSON.parse(exported.stdout).messages.map(message=>message.content.map(block=>block.text).join('')),['first edited','first reply edited','second reply']);
    await run(third,true);
    const cold = SessionManager.open(file);
    const projected = cold.buildSessionProjection().entries.flatMap(entry => entry.messages.map(message => ({id:entry.sourceEntry.id,message})));
    assert.deepEqual(projected.map(entry=>entry.id),[first,firstReply,secondReply]);
    assert.deepEqual(projected.map(({message})=>typeof message.content==='string' ? message.content : message.content.map(block=>block.text).join('')),['first edited','first reply edited','second reply']);
    const carried = cold.getBranch().filter(entry=>entry.type==='context_edit');
    assert.deepEqual(new Set(carried.map(entry=>entry.targetId)),new Set([first,firstReply,second]));
    assert.equal(carried.find(entry=>entry.targetId===second).replacement,null);
    const raw = (await readFile(file,'utf8')).trim().split('\n').map(line=>JSON.parse(line));
    assert.equal(raw.filter(entry=>entry.type==='message').length,8,'control edits must never fabricate model messages');
    const inspected = spawnSync(process.execPath,[helper,'--agent-dir',root,'--session-dir',sessions,'--cli',cli,'--runtime-type','pi-1.0','--inspect-session',file],{encoding:'utf8'});
    assert.equal(inspected.status,0,inspected.stderr);
    assert.deepEqual(JSON.parse(inspected.stdout).messages.map(message=>message.id),projected.map(entry=>`pi:${manager.getSessionId()}:${entry.id}:0`));
  }
  // Reject unknown/corrupt sources before SDK.open can migrate or skip lines.
  // Known old formats may migrate only after an exact fsynced original backup,
  // and require an explicit reload rather than deleting against stale IDs.
  for (const mode of ['future','malformed','version1','version2']) {
    const manager = SessionManager.create(root,sessions);
    const first = manager.appendMessage({role:'user',content:'preserve original',timestamp:Date.now()});
    manager.appendMessage({role:'assistant',content:[{type:'text',text:'original reply'}],api:'openai-completions',provider:'synthetic',model:'synthetic',usage:{input:0,output:0,cacheRead:0,cacheWrite:0,totalTokens:0,cost:{input:0,output:0,cacheRead:0,cacheWrite:0,total:0}},stopReason:'stop',timestamp:Date.now()});
    const file=manager.getSessionFile();
    const records=(await readFile(file,'utf8')).trim().split('\n').map(line=>JSON.parse(line));
    if(mode==='future') records[0].version=999;
    if(mode==='version1') {records[0].version=1;for(const record of records.slice(1)){delete record.id;delete record.parentId;}}
    if(mode==='version2') records[0].version=2;
    const original=Buffer.from(records.map(record=>JSON.stringify(record)+'\n').join('')+(mode==='malformed' ? '{malformed}\n' : ''));
    await writeFile(file,original);
    const before=new Set(await readdir(join(root,'backups')));
    const result=spawnSync(process.execPath,[helper,'--agent-dir',root,'--session-dir',sessions,'--cli',cli,'--runtime-type','pi-1.0','--delete-message',file],{encoding:'utf8',input:JSON.stringify({messageId:`pi:${manager.getSessionId()}:${first}:0`,truncate:false,expectedDigest:createHash('sha256').update(original).digest('hex'),backupDirectory:join(root,'backups')})});
    assert.notEqual(result.status,0);
    const backups=(await readdir(join(root,'backups'))).filter(name=>!before.has(name));
    if(mode==='future'||mode==='malformed') {
      assert.deepEqual(await readFile(file),original);
      assert.deepEqual(backups,[]);
    } else {
      assert.match(result.stderr,/safely migrated; reload/);
      assert.equal(backups.length,1);
      assert.deepEqual(await readFile(join(root,'backups',backups[0])),original);
      const current=(await readFile(file,'utf8')).trim().split('\n').map(line=>JSON.parse(line));
      assert.equal(current[0].version,3);
      assert.deepEqual(current.filter(entry=>entry.type==='message').map(entry=>entry.message),records.filter(entry=>entry.type==='message').map(entry=>entry.message));
      assert.equal(SessionManager.open(file).buildSessionProjection().messages.filter(message=>message.role==='user').length,1,'migration must not apply the requested deletion');
    }
  }
  console.log('Pi official SDK: only-delete, truncate, stale CAS, exact backup, cold reopen, chained edit carryover and guarded migration passed');
} finally { await rm(root,{recursive:true,force:true}); }
