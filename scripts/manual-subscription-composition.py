#!/usr/bin/env python3
"""手动验收订阅组合与 Claude CLI 桥；仅临时 HOME、合成进程及回环网络。

不接入测试或 CI，不执行已安装的 Claude，不读取真实认证。合成 CLI 遵循
stream-json 与 MCP HTTP 的公开消息边界，证据不代表真实订阅或 CLI 兼容性。
运行：python3 scripts/manual-subscription-composition.py；仅取消闭环：加 --case cancel。
暂存登录清理：--case auth-cleanup；只在临时源码副本增加结果投递观测marker，
不改产品分支或持久化决策，核对重写恰好命中一次。
"""
import argparse
import json
import os
import re
from pathlib import Path
import subprocess
import shutil
import sys
import tempfile
import threading
from http.server import BaseHTTPRequestHandler, ThreadingHTTPServer

ROOT = Path(__file__).resolve().parents[1]

# This executable is generated inside the isolated fixture directory. It records
# only synthetic command classes/PIDs so process reuse is independently observable.
CLI = r'''
import json, os, pathlib, signal, sys, threading, time, urllib.request
ROOT = pathlib.Path(FIXTURE_DIRECTORY)
LOCK = threading.Lock()
CONFIG = os.environ.get('CLAUDE_CONFIG_DIR', '')
def record(kind, **details):
    value = dict(kind=kind, pid=os.getpid(), config=CONFIG, **details)
    with (ROOT / 'cli-events.jsonl').open('a') as f:
        f.write(json.dumps(value) + '\n')
def emit(value):
    with LOCK:
        print(json.dumps(value), flush=True)
def stop(*_):
    record('terminated')
    sys.exit(0)
signal.signal(signal.SIGTERM, stop)
args = sys.argv[1:]
record('start', argv=args)
if args[:2] == ['auth', 'login']:
    pathlib.Path(CONFIG).mkdir(parents=True, exist_ok=True)
    (pathlib.Path(CONFIG) / 'fixture-signed-in').write_text('synthetic')
    record('login')
    sys.exit(0)
if args[:2] == ['auth', 'status']:
    assert args == ['auth', 'status'], 'unsupported auth status flag'
    record('status')
    print(json.dumps({'loggedIn': (pathlib.Path(CONFIG) / 'fixture-signed-in').exists(),
                      'authMethod': 'claude.ai', 'subscriptionType': 'max', 'configDirectory': CONFIG,
                      'email': 'fixture@example.invalid'}))
    sys.exit(0)
if args[:2] == ['auth', 'logout']:
    (pathlib.Path(CONFIG) / 'fixture-signed-in').unlink(missing_ok=True)
    record('logout')
    sys.exit(0)
if '/usage' in args:
    assert args[:2] == ['-p', '/usage']
    assert args[args.index('--tools') + 1] == ''
    assert args[args.index('--setting-sources') + 1] == ''
    assert '--strict-mcp-config' in args and '--no-session-persistence' in args
    record('quota')
    assert args[args.index('--output-format') + 1] == 'json'
    print(json.dumps({'type': 'result', 'subtype': 'success', 'is_error': False, 'result': 'Current session: 25% used\nCurrent week (all models): 40% used'}))
    sys.exit(0)
assert args[args.index('--input-format') + 1] == 'stream-json'
assert args[args.index('--output-format') + 1] == 'stream-json'
assert args[args.index('--tools') + 1] == ''
assert '--strict-mcp-config' in args
config_path = pathlib.Path(args[args.index('--mcp-config') + 1])
config = json.loads(config_path.read_text())
server = config['mcpServers']['velune']
assert server['type'] == 'http'
URL = server['url']
assert URL.startswith('http://127.0.0.1:')
def rpc(method, params=None):
    request = urllib.request.Request(URL, data=json.dumps({'jsonrpc': '2.0', 'id': 1,
        'method': method, 'params': params or {}}).encode(),
        headers={'Content-Type': 'application/json'})
    with urllib.request.urlopen(request, timeout=30) as response:
        return json.load(response)['result']
def complete(text):
    emit({'type': 'assistant', 'message': {'id': 'fixture-message', 'type': 'message',
          'role': 'assistant', 'model': 'fixture-model', 'content': [{'type': 'text', 'text': text}],
          'stop_reason': 'end_turn', 'usage': {'input_tokens': 3, 'output_tokens': 2}}})
    emit({'type': 'result', 'subtype': 'success', 'is_error': False,
          'result': text, 'usage': {'input_tokens': 3, 'output_tokens': 2}})
    record('completed')
def tool():
    try:
        rpc('initialize', {'protocolVersion': '2024-11-05', 'capabilities': {}, 'clientInfo': {'name': 'fixture', 'version': '1'}})
        tools = rpc('tools/list')['tools']
        assert any(t['name'] == 'echo_fixture' for t in tools)
        record('tool_wait')
        result = rpc('tools/call', {'name': 'echo_fixture', 'arguments': {'value': 'synthetic-argument'}})
        record('tool_resumed')
        assert result['content'][0]['text'] == 'caller-only-result'
        complete('tool-result-seen')
    except Exception:
        record('tool_failed')
        sys.exit(2)
for line in sys.stdin:
    value = json.loads(line)
    if value.get('type') != 'user':
        continue
    record('input')
    text = json.dumps(value.get('message', {}))
    if 'fixture-crash' in text:
        record('submitted_crash')
        sys.exit(7)
    if 'fixture-hang' in text:
        record('hanging')
        threading.Event().wait(30)
    elif 'fixture-partial' in text:
        for event in [
            {'type': 'message_start', 'message': {'id': 'fixture-partial', 'usage': {'input_tokens': 3, 'output_tokens': 0}}},
            {'type': 'content_block_start', 'index': 0, 'content_block': {'type': 'text', 'text': ''}},
            {'type': 'content_block_delta', 'index': 0, 'delta': {'type': 'text_delta', 'text': 'incremental-visible'}}]:
            emit({'type': 'stream_event', 'event': event})
        record('partial_wait')
        deadline = time.monotonic() + 5
        while not (ROOT / 'partial-observed').exists():
            assert time.monotonic() < deadline, 'partial output buffered until completion'
            time.sleep(0.01)
        emit({'type': 'stream_event', 'event': {'type': 'content_block_stop', 'index': 0}})
        emit({'type': 'result', 'subtype': 'success', 'is_error': False, 'usage': {'input_tokens': 3, 'output_tokens': 2}})
        record('completed')
    elif 'fixture-tool' in text:
        threading.Thread(target=tool, daemon=True).start()
    else:
        complete('fixture-completed')
record('eof')
'''

PROGRAM = r'''
use std::{path::{Path,PathBuf},sync::{Arc,Mutex},time::{Duration,Instant}};
use serde_json::{Value,json};
use velune_ai::{Payload,messages::*,ids::ProviderModelId};
use velune_ai_provider::claude_code::*;
fn provider(pool:Arc<ClaudeCodePool>,root:&Path,session:&str,binding:&str)->ClaudeCode{
 ClaudeCode::new(pool,ClaudeCodeConfig{executable:root.join("claude-fixture"),config_dir:root.join("account"),session:Some(session.into()),binding:binding.into()}).unwrap()
}
fn request(messages:Vec<Value>,tools:bool)->MessagesRequest{
 let mut body=json!({"model":"fixture-model","max_tokens":32,"stream":true,"messages":messages});
 if tools {body["tools"]=json!([{"name":"echo_fixture","description":"pure synthetic echo","input_schema":{"type":"object","properties":{"value":{"type":"string"}},"required":["value"]}}]);}
 MessagesRequest{model:ProviderModelId::new("fixture-model").unwrap(),body:Payload::new(body),headers:vec![],stream:true}
}
async fn call(p:&ClaudeCode,messages:Vec<Value>,tools:bool)->(MessagesCompletion,String){
 let bytes=Arc::new(Mutex::new(Vec::new()));let capture=bytes.clone();
 let result=tokio::time::timeout(Duration::from_secs(5),p.messages(request(messages,tools),Box::new(move |event|{if let MessagesEvent::Body(body)=event {capture.lock().unwrap().extend_from_slice(body.get());}Box::pin(async{Ok(())})}))).await.expect("CLI operation timed out");
 let text=String::from_utf8(bytes.lock().unwrap().clone()).unwrap();(result,text)
}
fn log(root:&Path)->Vec<Value>{std::fs::read_to_string(root.join("cli-events.jsonl")).unwrap_or_default().lines().map(|s|serde_json::from_str(s).unwrap()).collect()}
fn count(root:&Path,kind:&str)->usize{log(root).iter().filter(|v|v["kind"]==kind).count()}
async fn wait(root:&Path,kind:&str,minimum:usize){let deadline=Instant::now()+Duration::from_secs(3);while count(root,kind)<minimum{assert!(Instant::now()<deadline,"missing fixture stage {kind}");tokio::time::sleep(Duration::from_millis(10)).await;}}
fn user(text:&str)->Value{json!({"role":"user","content":text})}
fn text_blocks(text:&str)->Value{json!({"role":"assistant","content":[{"type":"text","text":text}]})}
fn success(reply:&(MessagesCompletion,String),text:&str){assert!(reply.0.result.is_ok(),"operation failed");assert!(reply.1.contains(text),"missing synthetic reply");assert!(reply.1.contains("message_stop"),"missing terminal");}
fn alive(pid:u64)->bool{std::process::Command::new("/bin/kill").args(["-0",&pid.to_string()]).stderr(std::process::Stdio::null()).status().unwrap().success()}
async fn dead(pid:u64){let deadline=Instant::now()+Duration::from_secs(3);while alive(pid){assert!(Instant::now()<deadline,"synthetic child leaked");tokio::time::sleep(Duration::from_millis(10)).await;}}

struct Auth{root:PathBuf}
impl velune_gateway::CredentialResolver for Auth{fn resolve(&self,id:String,target:velune_gateway::CredentialTarget)->velune_ai::OperationFuture<Result<velune_gateway::ResolvedCredential,velune_gateway::CredentialResolutionError>>{let root=self.root.clone();Box::pin(async move{if matches!(target.execution_backend,velune_gateway::ExecutionBackend::Http){return Ok(velune_gateway::ResolvedCredential{material:velune_gateway::CredentialMaterial::Http{authorization:velune_gateway::HttpAuthorization::Bearer("synthetic-http".into()),account_id:None},recovery:velune_gateway::CredentialRecovery::None,request_capabilities:Default::default()});}Ok(velune_gateway::ResolvedCredential{material:velune_gateway::CredentialMaterial::LocalSession{config_dir:root.join(format!("account-{id}")).to_string_lossy().into_owned()},recovery:velune_gateway::CredentialRecovery::None,request_capabilities:velune_gateway::RequestCapabilities::default()})})}}
fn gateway_config(root:&Path)->velune_gateway::GatewayConfig{use velune_gateway::*;GatewayConfig{id:"bridge".into(),name:"bridge".into(),providers:["cli-a","cli-b"].into_iter().map(|id|ProviderDefinition{id:id.into(),name:id.into(),protocol:GatewayProtocol::MessagesV1,execution_backend:ExecutionBackend::ClaudeCli{binary:root.join("claude-fixture").to_string_lossy().into_owned()},request_profile:RequestProfile::Native,endpoint:String::new(),credential_ref:Some(id.into()),quota_limits:vec![],models:vec![ProviderModel{record_key:id.into(),provider_model_id:"fixture-model".into(),nickname:id.into(),icon:None,context_window:None,max_output_tokens:Some(32),reasoning_levels:None}]}).collect(),routes:vec![ModelRoute{record_key:"bridge-pool".into(),name:"bridge-pool".into(),members:vec!["cli-a".into(),"cli-b".into()],strategy:RoutingStrategy::Order,sticky:true,weights:std::collections::BTreeMap::new()}]}}
fn gateway(root:&Path,config:velune_gateway::GatewayConfig,state:velune_gateway::SchedulingState,backends:velune_gateway::ExecutionBackends,session:&str)->velune_gateway::Runner{velune_gateway::Runner::start_with_scheduling(config,Arc::new(Auth{root:root.to_owned()}),std::collections::BTreeMap::new(),None,state,Arc::new(velune_gateway::UnknownQuotaSource),Some(velune_gateway::SessionIdentity{namespace:"fixture".into(),id:session.into()}),backends).unwrap()}
async fn http(client:&reqwest::Client,gateway:&velune_gateway::Runner,messages:Vec<Value>,tools:bool)->(u16,String){let mut body=request(messages,tools).body.into_inner();body["model"]=json!("velune/model/bridge-pool");let response=client.post(format!("{}/messages",gateway.endpoint())).bearer_auth(gateway.token()).header("anthropic-version","2023-06-01").json(&body).send().await.unwrap();let status=response.status().as_u16();(status,response.text().await.unwrap_or_default())}
fn tool_blocks(text:&str)->Vec<Value>{text.lines().filter_map(|l|l.strip_prefix("data: ")).filter_map(|s|serde_json::from_str::<Value>(s).ok()).filter_map(|v|v.get("content_block").cloned()).filter(|v|v["type"]=="tool_use").collect()}
async fn gateway_cases(root:&Path)->Vec<&'static str>{use velune_gateway::*;let mut passed=Vec::new();let backends=ExecutionBackends::default();let state=SchedulingState::default();let config=gateway_config(root);config.validate().unwrap();let client=reqwest::Client::builder().no_proxy().timeout(Duration::from_secs(5)).build().unwrap();
 let starts=count(root,"start");let first=gateway(root,config.clone(),state.clone(),backends.clone(),"cross-runner-tools");let reply=http(&client,&first,vec![user("fixture-tool")],true).await;assert_eq!(reply.0,200);let blocks=tool_blocks(&reply.1);assert_eq!(blocks.len(),1);let id=blocks[0]["id"].clone();drop(first);let next=gateway(root,config.clone(),state.clone(),backends.clone(),"cross-runner-tools");let result=http(&client,&next,vec![user("fixture-tool"),json!({"role":"assistant","content":blocks}),json!({"role":"user","content":[{"type":"tool_result","tool_use_id":id,"content":"caller-only-result"}]})],true).await;assert_eq!(result.0,200);assert!(result.1.contains("tool-result-seen")&&result.1.contains("message_stop"));assert_eq!(count(root,"start"),starts+1,"new Runner lost original CLI process");drop(next);passed.push("HTTP tool result crosses Runner destruction and resumes original PID");
 let before=count(root,"start");let crash=gateway(root,config.clone(),state.clone(),backends.clone(),"unsafe-crash");let result=http(&client,&crash,vec![user("fixture-crash")],false).await;assert_ne!(result.0,200);assert_eq!(count(root,"start"),before+1,"post-spawn failure replayed through fallback");drop(crash);passed.push("gateway never fails over a submitted CLI crash");
 let pending=gateway(root,config.clone(),SchedulingState::default(),backends.clone(),"pending-invalidated");let reply=http(&client,&pending,vec![user("fixture-tool")],true).await;assert_eq!(reply.0,200);let blocks=tool_blocks(&reply.1);assert_eq!(blocks.len(),1);let id=blocks[0]["id"].clone();drop(pending);let pid=log(root).iter().rev().find(|v|v["kind"]=="tool_wait").unwrap()["pid"].as_u64().unwrap();backends.invalidate_provider("bridge","cli-a");dead(pid).await;let before=count(root,"start");let changed=gateway(root,config.clone(),SchedulingState::default(),backends.clone(),"pending-invalidated");let reply=http(&client,&changed,vec![user("fixture-tool"),json!({"role":"assistant","content":blocks}),json!({"role":"user","content":[{"type":"tool_result","tool_use_id":id,"content":"caller-only-result"}]})],true).await;assert_eq!(reply.0,409);assert_eq!(count(root,"start"),before,"invalid pending ownership started replacement");drop(changed);passed.push("invalidated pending ownership returns conflict without replay");
 let mut invalid=config.clone();invalid.providers[0].endpoint="https://example.invalid".into();assert!(invalid.validate().is_err());let mut invalid=config.clone();invalid.providers[0].protocol=GatewayProtocol::ResponsesV1;assert!(invalid.validate().is_err());let mut invalid=config.clone();invalid.providers[0].request_profile=RequestProfile::CodexResponses;assert!(invalid.validate().is_err());passed.push("gateway rejects incoherent CLI protocol/profile/endpoint combinations");
 backends.shutdown();for event in log(root).iter().filter(|v|v["kind"]=="start"){dead(event["pid"].as_u64().unwrap()).await;}passed
}



async fn cancellation_cases(root:&Path,endpoint:&str)->Vec<&'static str>{use velune_gateway::*;let backends=ExecutionBackends::default();let state=SchedulingState::default();let mut config=gateway_config(root);config.providers[1].execution_backend=ExecutionBackend::Http;config.providers[1].endpoint=endpoint.into();let identity=SessionIdentity{namespace:"fixture".into(),id:"cancel-pending-tool".into()};let client=reqwest::Client::builder().no_proxy().timeout(Duration::from_secs(5)).build().unwrap();let first=gateway(root,config.clone(),state.clone(),backends.clone(),&identity.id);let reply=http(&client,&first,vec![user("fixture-tool")],true).await;assert_eq!(reply.0,200);let blocks=tool_blocks(&reply.1);assert_eq!(blocks.len(),1);let id=blocks[0]["id"].clone();assert!(backends.begin_user_turn("bridge",&identity).is_err(),"new turn silently discarded pending tool");drop(first);let pid=log(root).iter().rev().find(|v|v["kind"]=="tool_wait").unwrap()["pid"].as_u64().unwrap();backends.cancel_session("bridge",&identity);dead(pid).await;backends.begin_user_turn("bridge",&identity).unwrap();let next=gateway(root,config.clone(),state.clone(),backends.clone(),&identity.id);let before=count(root,"start");let reply=http(&client,&next,vec![user("fixture-text")],false).await;assert_eq!(reply.0,200);assert!(reply.1.contains("fixture-completed"));assert_eq!(count(root,"start"),before+1,"explicit new user turn did not start a new process");drop(next);let starts=count(root,"start");let input=count(root,"input");let http_before=count(root,"http-request");let late=gateway(root,config,state,backends.clone(),&identity.id);let reply=http(&client,&late,vec![user("fixture-tool"),json!({"role":"assistant","content":blocks}),json!({"role":"user","content":[{"type":"tool_result","tool_use_id":id,"content":"caller-only-result"}]})],true).await;assert_eq!(reply.0,409,"late cancelled result was admitted");assert_eq!(count(root,"start"),starts);assert_eq!(count(root,"input"),input);assert_eq!(count(root,"http-request"),http_before,"late result reached HTTP fallback");drop(late);backends.shutdown();for event in log(root).iter().filter(|v|v["kind"]=="start"){dead(event["pid"].as_u64().unwrap()).await;}vec!["explicit pending cancellation kills MCP process; new turn works; late tool result reaches no backend"]}



async fn finishing_cases(root:&Path)->Vec<&'static str>{use velune_gateway::*;let backends=ExecutionBackends::default();let state=SchedulingState::default();let config=gateway_config(root);let client=reqwest::Client::builder().no_proxy().timeout(Duration::from_secs(5)).build().unwrap();let pending=SessionIdentity{namespace:"fixture".into(),id:"finish-pending".into()};let first=gateway(root,config.clone(),state.clone(),backends.clone(),&pending.id);let reply=http(&client,&first,vec![user("fixture-tool")],true).await;assert_eq!(reply.0,200);assert_eq!(tool_blocks(&reply.1).len(),1);drop(first);let pid=log(root).iter().rev().find(|v|v["kind"]=="tool_wait").unwrap()["pid"].as_u64().unwrap();backends.finish_user_turn("bridge",&pending);dead(pid).await;backends.begin_user_turn("bridge",&pending).unwrap();let next=gateway(root,config.clone(),state.clone(),backends.clone(),&pending.id);let starts=count(root,"start");let reply=http(&client,&next,vec![user("fixture-text")],false).await;assert_eq!(reply.0,200);assert!(reply.1.contains("fixture-completed"));assert_eq!(count(root,"start"),starts+1);drop(next);
 let completed=SessionIdentity{namespace:"fixture".into(),id:"finish-completed".into()};let first=gateway(root,config.clone(),state.clone(),backends.clone(),&completed.id);let reply=http(&client,&first,vec![user("fixture-text")],false).await;assert_eq!(reply.0,200);assert!(reply.1.contains("message_stop"));let starts=count(root,"start");let pid=log(root).iter().rev().find(|v|v["kind"]=="completed").unwrap()["pid"].as_u64().unwrap();drop(first);backends.finish_user_turn("bridge",&completed);assert!(alive(pid),"finish killed completed idle CLI session");backends.begin_user_turn("bridge",&completed).unwrap();let next=gateway(root,config,state,backends.clone(),&completed.id);let reply=http(&client,&next,vec![user("fixture-text"),text_blocks("fixture-completed"),user("fixture-next")],false).await;assert_eq!(reply.0,200);assert!(reply.1.contains("fixture-completed"));assert_eq!(count(root,"start"),starts,"completed idle CLI was restarted");assert!(alive(pid));drop(next);backends.shutdown();for event in log(root).iter().filter(|v|v["kind"]=="start"){dead(event["pid"].as_u64().unwrap()).await;}vec!["finish pending outer turn kills MCP child and permits explicit new turn", "finish completed outer turn preserves same PID across next Runner"]}

fn wait_staging(root:&Path,prior:usize)->PathBuf{let deadline=Instant::now()+Duration::from_secs(5);loop{let events=log(root);let logins:Vec<_>=events.iter().filter(|v|v["kind"]=="login").collect();if logins.len()>prior{let dir=PathBuf::from(logins.last().unwrap()["config"].as_str().unwrap());if std::fs::read(dir.join("fixture-result-delivered")).ok().as_deref()==Some(b"success"){return dir;}}assert!(Instant::now()<deadline,"successful staging result never reached receiver");std::thread::sleep(Duration::from_millis(10));}}
fn wait_staging_cleanup(root:&Path,dir:&Path){let deadline=Instant::now()+Duration::from_secs(5);while dir.exists()||!log(root).iter().any(|v|v["kind"]=="logout"&&v["config"].as_str()==dir.to_str()){assert!(Instant::now()<deadline,"staging was not logged out and removed");std::thread::sleep(Duration::from_millis(10));}}
fn authentication_cleanup_cases(root:&Path)->Vec<&'static str>{use velune_application::{Application,Options,ProviderDraft,AuthenticationEdit};let home=root.join("cleanup-application");let resources=root.join("resources");std::fs::create_dir_all(&resources).unwrap();let mut app=Application::open(Options{home_directory:home.clone(),resources_directory:resources}).unwrap();let draft:ProviderDraft=serde_json::from_value(json!({"id":"claude","name":"Synthetic Claude","protocol":"messagesV1","endpoint":"","executionBackend":{"claudeCli":{"binary":root.join("claude-fixture")}},"requestProfile":"native","authenticationSource":"claudeCli","quotaSource":"none","quotaLimits":[],"models":[{"recordKey":"","providerModelId":"fixture-model","nickname":"fixture","icon":null,"contextWindow":8192,"maxOutputTokens":32,"reasoningLevels":null,"piProjection":null}]})).unwrap();app.save_provider("default".into(),draft,AuthenticationEdit::Keep).unwrap();let logins=count(root,"login");let mut progress=app.authentication_start("default".into(),"claude".into()).unwrap();let old=wait_staging(root,logins);let deadline=Instant::now()+Duration::from_secs(5);while progress.running{assert!(Instant::now()<deadline);std::thread::sleep(Duration::from_millis(10));progress=app.authentication_poll().unwrap();}assert!(progress.events.iter().any(|e|e.ok==Some(true)));let before=std::fs::read(home.join("generic-config.json")).unwrap();assert!(old.exists());
 let logins=count(root,"login");app.authentication_start("default".into(),"claude".into()).unwrap();let rejected=wait_staging(root,logins);assert_ne!(old,rejected);std::fs::create_dir(home.join("generic-config.json.tmp")).unwrap();assert!(app.authentication_poll().is_err(),"blocked configuration persistence falsely succeeded");std::fs::remove_dir(home.join("generic-config.json.tmp")).unwrap();wait_staging_cleanup(root,&rejected);assert_eq!(std::fs::read(home.join("generic-config.json")).unwrap(),before);assert!(old.exists(),"failed relogin removed old CLI login directory");assert!(app.authentication_inspect("default".into(),"claude".into()).unwrap().configured);
 let logins=count(root,"login");app.authentication_start("default".into(),"claude".into()).unwrap();let cancelled=wait_staging(root,logins);assert_ne!(old,cancelled);let progress=app.authentication_cancel().unwrap();assert!(!progress.running);assert!(progress.events.iter().any(|e|e.cancelled==Some(true)&&e.ok==Some(false)),"already delivered success was committed despite cancellation");wait_staging_cleanup(root,&cancelled);assert_eq!(std::fs::read(home.join("generic-config.json")).unwrap(),before);assert!(old.exists());assert!(app.authentication_inspect("default".into(),"claude".into()).unwrap().configured);app.close().unwrap();vec!["failed CLI login persistence logs out/removes staging while retaining old binding", "cancel after delivered success cleans staging before Application drop and never commits"]}

fn application_cases(root:&Path)->Vec<&'static str>{use velune_application::{Application,Options,ProviderDraft,AuthenticationEdit};let mut passed=Vec::new();let home=root.join("application");let resources=root.join("resources");std::fs::create_dir_all(&resources).unwrap();let options=Options{home_directory:home.clone(),resources_directory:resources};let mut app=Application::open(options.clone()).unwrap();let value=json!({"id":"claude","name":"Synthetic Claude","protocol":"messagesV1","endpoint":"","executionBackend":{"claudeCli":{"binary":root.join("claude-fixture")}},"requestProfile":"native","authenticationSource":"claudeCli","quotaSource":"claudeCliUsage","quotaLimits":[],"models":[{"recordKey":"","providerModelId":"fixture-model","nickname":"fixture","icon":null,"contextWindow":8192,"maxOutputTokens":32,"reasoningLevels":null,"piProjection":null}]});let draft:ProviderDraft=serde_json::from_value(value.clone()).unwrap();app.save_provider("default".into(),draft,AuthenticationEdit::Keep).unwrap();let before=std::fs::read(home.join("generic-config.json")).unwrap();for (field,invalid) in [("endpoint",json!("https://example.invalid")),("requestProfile",json!("codexResponses")),("authenticationSource",json!("apiKey")),("quotaSource",json!("codexUsage"))]{let mut rejected=value.clone();rejected[field]=invalid;let draft:ProviderDraft=serde_json::from_value(rejected).unwrap();assert!(app.save_provider("default".into(),draft,AuthenticationEdit::Keep).is_err());assert_eq!(std::fs::read(home.join("generic-config.json")).unwrap(),before,"invalid composition mutated saved state");}passed.push("Application rejects invalid combinations without changing persisted configuration");
 assert!(!app.authentication_inspect("default".into(),"claude".into()).unwrap().configured);let mut progress=app.authentication_start("default".into(),"claude".into()).unwrap();let deadline=Instant::now()+Duration::from_secs(5);while progress.running {assert!(Instant::now()<deadline);std::thread::sleep(Duration::from_millis(10));progress=app.authentication_poll().unwrap();}assert!(progress.events.iter().any(|event|event.type_id=="result"&&event.ok==Some(true)));assert!(app.authentication_inspect("default".into(),"claude".into()).unwrap().configured);assert!(count(root,"login")>0&&count(root,"status")>0);passed.push("Application login and status delegate to CLI in isolated account directory");
 app.refresh_provider_quota("default".into(),"claude".into()).unwrap();let deadline=Instant::now()+Duration::from_secs(5);let quota=loop{let q=app.provider_quota("default".into(),"claude".into()).unwrap();if !q.refreshing&&matches!(q.freshness,velune_application::QuotaFreshness::Fresh){break q;}assert!(Instant::now()<deadline,"CLI quota never became fresh");std::thread::sleep(Duration::from_millis(10));};assert!(quota.windows.iter().any(|w|w.id=="window:18000"&&w.used_percent==25.));assert!(quota.windows.iter().any(|w|w.id=="window:604800"&&w.used_percent==40.));assert!(count(root,"quota")>0);passed.push("CLI usage command publishes independently observed quota windows");
 app.close().unwrap();drop(app);let mut app=Application::open(options).unwrap();let snapshot=app.list().unwrap();let provider=snapshot.gateways.iter().find(|g|g.id=="default").unwrap().providers.iter().find(|p|p.id=="claude").unwrap();assert!(matches!(provider.execution_backend,velune_application::config::ExecutionBackend::ClaudeCli{..}));assert_eq!(provider.request_profile,velune_application::config::RequestProfile::Native);assert_eq!(provider.authentication_source,velune_application::ProviderAuthenticationSource::ClaudeCli);assert_eq!(provider.quota_source,velune_application::config::ProviderQuotaSource::ClaudeCliUsage);assert!(provider.authentication.configured);passed.push("composition and CLI login binding survive Application reopen");app.authentication_logout("default".into(),"claude".into()).unwrap();assert!(!app.authentication_inspect("default".into(),"claude".into()).unwrap().configured);assert!(count(root,"logout")>0);app.close().unwrap();passed.push("Application logout delegates to CLI and clears binding");passed
}

#[tokio::main(flavor="current_thread")]
async fn main(){let root=PathBuf::from(std::env::args().nth(1).unwrap());let selected=std::env::args().nth(2).unwrap();let endpoint=std::env::args().nth(3).unwrap();if selected=="auth-cleanup"{let r=root.clone();let passed=std::thread::spawn(move||authentication_cleanup_cases(&r)).join().unwrap();println!("{}",json!({"passed":passed,"real_cli_executed":false,"temporary_observation_patch":true}));return;}if selected=="cancel"{let mut passed=cancellation_cases(&root,&endpoint).await;passed.extend(finishing_cases(&root).await);println!("{}",json!({"passed":passed,"real_cli_executed":false}));return;}let pool=Arc::new(ClaudeCodePool::new());let mut passed=Vec::new();
 let partial=provider(pool.clone(),&root,"namespace/partial","provider/epoch-a");let marker=root.join("partial-observed");let result=tokio::time::timeout(Duration::from_secs(5),partial.messages(request(vec![user("fixture-partial")],false),Box::new(move|event|{if let MessagesEvent::Body(bytes)=event{if String::from_utf8_lossy(bytes.get()).contains("incremental-visible"){std::fs::write(&marker,"observed before terminal").unwrap();}}Box::pin(async{Ok(())})}))).await.unwrap();assert!(result.result.is_ok());passed.push("stream delta reaches caller before CLI terminal is released");
 let a=provider(pool.clone(),&root,"namespace/session-a","provider/epoch-a");let first=call(&a,vec![user("fixture-text")],false).await;success(&first,"fixture-completed");let before=count(&root,"start");
 let history=vec![user("fixture-text"),text_blocks("fixture-completed"),user("fixture-next")];success(&call(&a,history,false).await,"fixture-completed");assert_eq!(count(&root,"start"),before,"same session restarted");passed.push("stream completion and same-session process reuse");
 let b=provider(pool.clone(),&root,"namespace/session-b","provider/epoch-a");success(&call(&b,vec![user("fixture-text")],false).await,"fixture-completed");assert_eq!(count(&root,"start"),before+1,"sessions shared process");passed.push("explicit sessions own independent processes");
 let t=provider(pool.clone(),&root,"namespace/tools","provider/epoch-a");let first=call(&t,vec![user("fixture-tool")],true).await;assert!(first.0.result.is_ok());let blocks:Vec<Value>=first.1.lines().filter_map(|l|l.strip_prefix("data: ")).filter_map(|s|serde_json::from_str::<Value>(s).ok()).filter_map(|v|v.get("content_block").cloned()).filter(|v|v["type"]=="tool_use").collect();assert_eq!(blocks.len(),1,"tool call not surfaced");assert_eq!(count(&root,"tool_resumed"),0,"tool ran without caller result");let call_id=blocks[0]["id"].as_str().unwrap().to_owned();let starts=count(&root,"start");let continuation=vec![user("fixture-tool"),json!({"role":"assistant","content":blocks}),json!({"role":"user","content":[{"type":"tool_result","tool_use_id":call_id,"content":"caller-only-result"}]})];success(&call(&t,continuation,true).await,"tool-result-seen");assert_eq!(count(&root,"start"),starts);assert_eq!(count(&root,"tool_resumed"),1);passed.push("caller tool result resumes the original blocked MCP call and process");
 let c=Arc::new(provider(pool.clone(),&root,"namespace/cancel","provider/epoch-a"));let task=tokio::spawn({let c=c.clone();async move{call(&c,vec![user("fixture-hang")],false).await}});wait(&root,"hanging",1).await;let pid=log(&root).iter().rev().find(|v|v["kind"]=="hanging").unwrap()["pid"].as_u64().unwrap();task.abort();let _=task.await;dead(pid).await;passed.push("cancelled operation kills its hanging child");
 let obsolete=Arc::new(provider(pool.clone(),&root,"namespace/replace","provider/obsolete"));let task=tokio::spawn({let obsolete=obsolete.clone();async move{call(&obsolete,vec![user("fixture-hang")],false).await}});wait(&root,"hanging",2).await;let pid=log(&root).iter().rev().find(|v|v["kind"]=="hanging").unwrap()["pid"].as_u64().unwrap();pool.invalidate("provider/obsolete");let old=task.await.unwrap();assert!(old.0.result.is_err(),"invalidated binding delivered success");dead(pid).await;let replacement=provider(pool.clone(),&root,"namespace/replace","provider/replacement");success(&call(&replacement,vec![user("fixture-text")],false).await,"fixture-completed");passed.push("binding invalidation kills in-flight process and rejects old result");
 let crash=provider(pool.clone(),&root,"namespace/crash","provider/epoch-a");let result=call(&crash,vec![user("fixture-crash")],false).await;assert!(result.0.result.is_err_and(|f|f.submitted),"post-spawn failure was marked safe to retry");passed.push("post-spawn failure carries submitted state");
 pool.shutdown();for event in log(&root).iter().filter(|v|v["kind"]=="start"){dead(event["pid"].as_u64().unwrap()).await;}passed.push("shared pool shutdown releases retained processes");passed.extend(gateway_cases(&root).await);passed.extend(cancellation_cases(&root,&endpoint).await);passed.extend(finishing_cases(&root).await);let app_root=root.clone();passed.extend(std::thread::spawn(move||application_cases(&app_root)).join().unwrap());println!("{}",json!({"passed":passed,"real_cli_executed":false}));
}
'''


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('--case', choices=['all', 'cancel', 'auth-cleanup'], default='all')
    args = parser.parse_args()
    with tempfile.TemporaryDirectory(prefix='velune-composition-') as directory:
        temporary = Path(directory)
        cli = '#!' + sys.executable + '\n' + CLI.replace('FIXTURE_DIRECTORY', repr(directory))
        executable = temporary / 'claude-fixture'
        executable.write_text(cli)
        executable.chmod(0o700)
        (temporary / 'src').mkdir()
        (temporary / 'src/main.rs').write_text(PROGRAM)
        source = ROOT
        if args.case == 'auth-cleanup':
            source = temporary / 'snapshot'
            for unit in ['ai', 'ai-provider', 'conversation', 'agent-runtime', 'gateway', 'application']:
                shutil.copytree(ROOT / 'packages' / unit, source / 'packages' / unit, ignore=shutil.ignore_patterns('target'))
            observed = source / 'packages/application/src/claude_cli.rs'
            text = observed.read_text()
            marker = r'let delivered\s*=\s*sender\.send\(outcome\)\.is_ok\(\);'
            text, count = re.subn(marker, lambda match: match.group(0) + '\n            if successful && delivered { std::fs::write(dir.join("fixture-result-delivered"), b"success").unwrap(); }', text)
            assert count == 1, 'staging observation boundary changed'
            observed.write_text(text)
        dependencies = '\n'.join('%s={path="%s"}' % (name, source / 'packages' / folder)
                                 for name, folder in [('velune-ai-provider', 'ai-provider'), ('velune-ai', 'ai'), ('velune-gateway', 'gateway'), ('velune-application', 'application')])
        (temporary / 'Cargo.toml').write_text('[package]\nname="manual-subscription-composition"\nversion="0.0.0"\nedition="2024"\n[workspace]\n[dependencies]\n' + dependencies + '\ntokio={version="1",features=["macros","rt","time","sync"]}\nserde_json="1"\nreqwest={version="0.13",default-features=false,features=["json"]}\n')
        target = ROOT / 'target/manual-subscription-composition'
        subprocess.run(['cargo', 'build', '--quiet', '--offline', '--manifest-path', str(temporary / 'Cargo.toml'), '--target-dir', str(target)], cwd=ROOT, check=True)
        home = temporary / 'home'
        home.mkdir()
        environment = dict(os.environ, HOME=str(home), HTTP_PROXY='http://127.0.0.1:1', HTTPS_PROXY='http://127.0.0.1:1', ALL_PROXY='http://127.0.0.1:1', NO_PROXY='127.0.0.1,localhost', no_proxy='127.0.0.1,localhost')
        class HttpFallback(BaseHTTPRequestHandler):
            def log_message(self, *_):
                pass
            def do_POST(self):
                self.rfile.read(int(self.headers.get('Content-Length', '0')))
                with (temporary / 'cli-events.jsonl').open('a') as events:
                    events.write(json.dumps({'kind': 'http-request'}) + '\n')
                self.send_response(503)
                self.send_header('Content-Length', '0')
                self.end_headers()
        server = ThreadingHTTPServer(('127.0.0.1', 0), HttpFallback)
        threading.Thread(target=server.serve_forever, daemon=True).start()
        try:
            subprocess.run([str(target / 'debug/manual-subscription-composition'), directory, args.case, 'http://127.0.0.1:%d/v1' % server.server_port], cwd=temporary, env=environment, check=True, timeout=60)
        finally:
            server.shutdown()
            server.server_close()


if __name__ == '__main__':
    main()
