#!/usr/bin/env python3
"""Claude 订阅手动真实验收；默认仅冻结源码编译及无网络解析自检。

--login 使用 Application 官方 CLI 登录流程；--run 显式运行所选阶段。
两者均要求仓库外专用 --home，不读取已有 Claude 或 Velune 目录；认证由 CLI
管理。保留操作系统 HOME/USER/LOGNAME，认证目录仅由 CLAUDE_CONFIG_DIR 隔离。
日志不含原始 CLI 输出、认证 URL、token、账号标识或模型正文。
text 最多一次网关请求；tool 最多两次请求、同一 CLI turn；quota 只请求一次采样。
失败不重放。工具仅执行内存中的合成纯函数，不读文件或执行系统命令。

例：本脚本 --home /专用持久目录 --claude /opt/homebrew/bin/claude --login
    本脚本 --home /专用持久目录 --claude /opt/homebrew/bin/claude --run --stage text
"""
import argparse
import io
import json
import os
from pathlib import Path
import secrets
import shutil
import subprocess
import tarfile
import tempfile

ROOT = Path(__file__).resolve().parents[1]
MARKER = '.velune-claude-live-runner.json'

PROGRAM = r'''
use std::{collections::BTreeMap,io::{self,Write},path::{Path,PathBuf},sync::{Arc,Mutex},time::{Duration,Instant}};
use serde_json::{Value,json};
use velune_application::{Application,Options,ProviderDraft,AuthenticationEdit};
use velune_gateway::*;
type Result<T> = std::result::Result<T,&'static str>;
fn emit(v:Value){println!("{v}");let _=io::stdout().flush();}
#[derive(Default)]struct Sink(Mutex<Vec<AnalyticsRecord>>);
impl AnalyticsSink for Sink{fn record(&self,r:AnalyticsRecord){emit(json!({"state":"analytics","status":r.status,"outcome":format!("{:?}",r.outcome),"usage_reported":r.usage_reported,"usage_complete":r.usage_complete,"tokens":{"input":r.usage.input,"output":r.usage.output,"cached":r.usage.cached_input,"reasoning":r.usage.reasoning_output}}));self.0.lock().unwrap().push(r);}}
struct Reply{content:Vec<Value>,stop:String}
fn frame(kind:&str,data:&str,blocks:&mut BTreeMap<usize,Value>,partial:&mut BTreeMap<usize,String>,stop:&mut String,terminal:&mut bool)->Result<()> {
 let v:Value=serde_json::from_str(data).map_err(|_|"SSE_JSON_INVALID")?;if !kind.is_empty()&&v["type"].as_str().is_some_and(|t|t!=kind){return Err("SSE_TYPE_MISMATCH");}let tag=v["type"].as_str().unwrap_or(kind);
 match tag {"error"=>return Err("SSE_ERROR"),"content_block_start"=>{let i=v["index"].as_u64().ok_or("SSE_INDEX_INVALID")? as usize;blocks.insert(i,v["content_block"].clone());},"content_block_delta"=>{let i=v["index"].as_u64().ok_or("SSE_INDEX_INVALID")? as usize;let d=&v["delta"];match d["type"].as_str(){Some("text_delta"|"thinking_delta"|"signature_delta")=>{let key=match d["type"].as_str(){Some("thinking_delta")=>"thinking",Some("signature_delta")=>"signature",_=>"text"};let block=blocks.get_mut(&i).ok_or("SSE_BLOCK_MISSING")?;let text=block[key].as_str().unwrap_or("").to_owned()+d[key].as_str().unwrap_or("");block[key]=json!(text);},Some("input_json_delta")=>partial.entry(i).or_default().push_str(d["partial_json"].as_str().unwrap_or("")),_=>{}}},"message_delta"=>{if let Some(s)=v["delta"]["stop_reason"].as_str(){*stop=s.into();}},"message_stop"=>*terminal=true,_=>{}}Ok(())
}
fn parse(bytes:&[u8])->Result<Reply>{let text=std::str::from_utf8(bytes).map_err(|_|"SSE_UTF8_INVALID")?.replace("\r\n","\n");let mut blocks=BTreeMap::new();let mut partial=BTreeMap::new();let mut stop=String::new();let mut terminal=false;let mut kind=String::new();let mut data=Vec::new();for line in text.lines().chain(std::iter::once("")){if line.is_empty(){if !data.is_empty(){frame(&kind,&data.join("\n"),&mut blocks,&mut partial,&mut stop,&mut terminal)?;}data.clear();kind.clear();}else if let Some(v)=line.strip_prefix("event:"){kind=v.trim_start().into();}else if let Some(v)=line.strip_prefix("data:"){data.push(v.strip_prefix(' ').unwrap_or(v).to_owned());}}if !terminal||!matches!(stop.as_str(),"end_turn"|"tool_use"){return Err("SSE_NO_SUCCESSFUL_TERMINAL");}for (i,input) in partial{blocks.get_mut(&i).ok_or("SSE_BLOCK_MISSING")?["input"]=serde_json::from_str(&input).map_err(|_|"SSE_TOOL_INPUT_INVALID")?;}Ok(Reply{content:blocks.into_values().collect(),stop})}
fn text(reply:&Reply)->String{reply.content.iter().filter(|b|b["type"]=="text").filter_map(|b|b["text"].as_str()).collect::<Vec<_>>().join("")}
fn parser_check()->Result<()>{let valid=b"event: content_block_start\ndata: {\"index\":0,\"content_block\":{\"type\":\"text\",\"text\":\"synthetic\"}}\n\nevent: message_delta\ndata: {\"delta\":{\"stop_reason\":\"end_turn\"}}\n\nevent: message_stop\ndata: {}\n\n";let r=parse(valid)?;if text(&r)!="synthetic"||parse(b"data: {}\n\n").is_ok()||parse(b"event: error\ndata: {}\n\n").is_ok(){return Err("PARSER_SELF_CHECK_FAILED");}let frames=|values:Vec<Value>|values.iter().map(|v|format!("data: {v}\n\n")).collect::<String>();let tool=frames(vec![json!({"type":"content_block_start","index":0,"content_block":{"type":"tool_use","id":"fixture-id","name":"acceptance_marker","input":{}}}),json!({"type":"content_block_delta","index":0,"delta":{"type":"input_json_delta","partial_json":"{\"nonce\":\"synthetic\"}"}}),json!({"type":"message_delta","delta":{"stop_reason":"tool_use"}}),json!({"type":"message_stop"})]);let parsed=parse(tool.as_bytes())?;if parsed.stop!="tool_use"||parsed.content[0]["input"]["nonce"]!="synthetic"{return Err("TOOL_PARSER_SELF_CHECK_FAILED");}let thought=frames(vec![json!({"type":"content_block_start","index":0,"content_block":{"type":"thinking","thinking":"","signature":""}}),json!({"type":"content_block_delta","index":0,"delta":{"type":"thinking_delta","thinking":"synthetic"}}),json!({"type":"content_block_delta","index":0,"delta":{"type":"signature_delta","signature":"fixture-signature"}}),json!({"type":"message_delta","delta":{"stop_reason":"end_turn"}}),json!({"type":"message_stop"})]);let parsed=parse(thought.as_bytes())?;if parsed.content[0]["thinking"]!="synthetic"||parsed.content[0]["signature"]!="fixture-signature"||parse(b"event: error\ndata: {\"type\":\"message_stop\"}\n\n").is_ok(){return Err("THINKING_PARSER_SELF_CHECK_FAILED");}if login_diagnostic(Some("Claude Code 认证未完成 [phase=status;reason=process_exit;exit_code=1]")).is_none()||login_diagnostic(Some("Claude Code 认证未完成 [phase=status;reason=untrusted_value;exit_code=1]")).is_some()||login_diagnostic(Some("Claude Code 认证未完成 [phase=status;reason=process_exit;exit_code=2147483648]")).is_some(){return Err("DIAGNOSTIC_SELF_CHECK_FAILED");}emit(json!({"state":"parser_check","fixtures":9,"passed":true,"real_requests":0}));Ok(())}
fn setup(home:&Path,binary:&str,model:&str)->Result<(Application,Options)>{let resources=home.join("resources");std::fs::create_dir_all(&resources).map_err(|_|"HOME_CREATE_FAILED")?;let options=Options{home_directory:home.to_owned(),resources_directory:resources};let mut app=Application::open(options.clone()).map_err(|_|"APPLICATION_OPEN_FAILED")?;let snapshot=app.list().map_err(|_|"CONFIGURATION_READ_FAILED")?;let key=snapshot.gateways.iter().flat_map(|g|&g.providers).find(|p|p.id=="claude-live").and_then(|p|p.models.first()).map(|m|m.record_key.clone()).unwrap_or_default();let draft:ProviderDraft=serde_json::from_value(json!({"id":"claude-live","name":"Claude live acceptance","protocol":"messagesV1","endpoint":"","executionBackend":{"claudeCli":{"binary":binary}},"requestProfile":"native","authenticationSource":"claudeCli","quotaSource":"claudeCliUsage","quotaLimits":[],"models":[{"recordKey":key,"providerModelId":model,"nickname":"Live acceptance","icon":null,"contextWindow":null,"maxOutputTokens":256,"reasoningLevels":null,"piProjection":null}]})).map_err(|_|"PROVIDER_DRAFT_INVALID")?;app.save_provider("default".into(),draft,AuthenticationEdit::Keep).map_err(|_|"PROVIDER_SAVE_FAILED")?;Ok((app,options))}

fn login_diagnostic(error:Option<&str>)->Option<Value>{let fields=error?.strip_prefix("Claude Code 认证未完成 [")?.strip_suffix(']')?;let parts:Vec<_>=fields.split(';').collect();if parts.len()!=3{return None;}let phase=parts[0].strip_prefix("phase=")?;if !matches!(phase,"login"|"status"|"worker"){return None;}let reason=parts[1].strip_prefix("reason=")?;if !matches!(reason,"process_unavailable"|"timeout"|"output_invalid"|"process_exit"|"json_invalid"|"config_directory_missing"|"reported_directory_unavailable"|"owned_directory_unavailable"|"config_directory_mismatch"|"logged_in_missing"|"not_logged_in"|"authentication_method_unexpected"|"identity_missing"|"not_configured"|"cancelled"|"runtime_unavailable"|"channel_disconnected"){return None;}let exit=parts[2].strip_prefix("exit_code=")?;let code=if exit=="none"{None}else{Some(exit.parse::<i32>().ok()?)};Some(json!({"state":"login_diagnostic","phase":phase,"reason":reason,"exit_code":code}))}

fn login(app:&mut Application)->Result<()>{emit(json!({"state":"login_started","browser_action":"complete_official_claude_login","model_requests":0}));let mut progress=app.authentication_start("default".into(),"claude-live".into()).map_err(|_|"LOGIN_START_FAILED")?;let deadline=Instant::now()+Duration::from_secs(305);let mut next_notice=Instant::now()+Duration::from_secs(20);while progress.running{if Instant::now()>deadline{let _=app.authentication_cancel();return Err("LOGIN_TIMEOUT");}if Instant::now()>next_notice{emit(json!({"state":"login_waiting","model_requests":0}));next_notice=Instant::now()+Duration::from_secs(20);}std::thread::sleep(Duration::from_millis(200));progress=app.authentication_poll().map_err(|_|"LOGIN_POLL_FAILED")?;}if !progress.events.iter().any(|e|e.ok==Some(true)){for event in &progress.events{if let Some(diagnostic)=login_diagnostic(event.error.as_deref()){emit(diagnostic);}}return Err("LOGIN_NOT_COMPLETED");}let status=app.authentication_inspect("default".into(),"claude-live".into()).map_err(|_|"LOGIN_STATUS_FAILED")?;if !status.configured{return Err("LOGIN_STATUS_NOT_CONFIGURED");}emit(json!({"state":"login_complete","configured":true,"model_requests":0}));Ok(())}
fn quota(app:&mut Application)->Result<()>{let mut q=app.refresh_provider_quota("default".into(),"claude-live".into()).map_err(|_|"QUOTA_START_FAILED")?;let deadline=Instant::now()+Duration::from_secs(40);while q.refreshing{if Instant::now()>deadline{return Err("QUOTA_TIMEOUT");}std::thread::sleep(Duration::from_millis(200));q=app.provider_quota("default".into(),"claude-live".into()).map_err(|_|"QUOTA_POLL_FAILED")?;}let fresh=matches!(q.freshness,velune_application::QuotaFreshness::Fresh);let windows:Vec<_>=q.windows.iter().map(|w|json!({"window_seconds":w.id.strip_prefix("window:").and_then(|v|v.parse::<u64>().ok()),"used_percent":w.used_percent,"reset_at_ms":w.reset_at_ms})).collect();emit(json!({"state":"quota_observation","freshness":format!("{:?}",q.freshness),"available":q.available,"windows":windows,"problem_present":q.problem.is_some(),"refresh_requests":1,"model_inference_requests":0}));if !fresh{return Err("QUOTA_NOT_FRESH");}Ok(())}
fn captured(home:&Path)->Result<velune_application::config::GatewayConfig>{let bytes=std::fs::read(home.join("generic-config.json")).map_err(|_|"OWN_CONFIGURATION_READ_FAILED")?;let value:Value=serde_json::from_slice(&bytes).map_err(|_|"OWN_CONFIGURATION_INVALID")?;let gateway=value["gateways"].as_array().and_then(|g|g.iter().find(|g|g["id"]=="default")).ok_or("OWN_GATEWAY_MISSING")?;serde_json::from_value(gateway.clone()).map_err(|_|"OWN_GATEWAY_INVALID")}
fn runner(home:&Path,options:&Options,backends:ExecutionBackends,state:SchedulingState,sink:Arc<Sink>,session:&str)->Result<Runner>{let full=captured(home)?;let p=full.providers.iter().find(|p|p.id=="claude-live").ok_or("OWN_PROVIDER_MISSING")?;let gateway=GatewayConfig{id:"default".into(),name:"live acceptance".into(),providers:vec![ProviderDefinition{id:p.id.clone(),name:p.name.clone(),protocol:GatewayProtocol::MessagesV1,endpoint:p.endpoint.clone(),execution_backend:p.execution_backend.clone(),request_profile:p.request_profile,credential_ref:Some(p.id.clone()),quota_limits:vec![],models:p.models.iter().map(|m|ProviderModel{record_key:m.record_key.clone(),provider_model_id:m.provider_model_id.clone(),nickname:m.nickname.clone(),icon:None,context_window:None,max_output_tokens:Some(256),reasoning_levels:None}).collect()}],routes:vec![]};Runner::start_with_scheduling(gateway,velune_application::authentication_resolver::ProviderResolver::capture(&full,options),BTreeMap::new(),Some(sink),state,Arc::new(UnknownQuotaSource),Some(SessionIdentity{namespace:"claude-live-acceptance".into(),id:session.into()}),backends).map_err(|_|"GATEWAY_START_FAILED")}
fn body(model:&str,messages:Vec<Value>,tools:bool)->Value{let mut b=json!({"model":model,"stream":true,"max_tokens":256,"messages":messages});if tools{b["tools"]=json!([{"name":"acceptance_marker","description":"Return an opaque synthetic acceptance marker for the supplied nonce. The caller executes this pure function.","input_schema":{"type":"object","properties":{"nonce":{"type":"string"}},"required":["nonce"],"additionalProperties":false}}]);}b}
async fn request(client:&reqwest::Client,r:&Runner,body:Value,stage:&str,sink:&Sink)->Result<Reply>{let prior=sink.0.lock().unwrap().len();emit(json!({"state":"request_started","stage":stage}));let mut response=client.post(format!("{}/messages",r.endpoint())).bearer_auth(r.token()).header("anthropic-version","2023-06-01").json(&body).send().await.map_err(|_|"GATEWAY_REQUEST_FAILED")?;let status=response.status().as_u16();let sse=response.headers().get("content-type").and_then(|v|v.to_str().ok()).is_some_and(|v|v.split(';').next().unwrap_or("").trim().eq_ignore_ascii_case("text/event-stream"));emit(json!({"state":"response_headers","stage":stage,"status":status,"sse":sse}));if status!=200||!sse{return Err("GATEWAY_RESPONSE_REJECTED");}let mut bytes=Vec::new();while let Some(chunk)=response.chunk().await.map_err(|_|"RESPONSE_STREAM_FAILED")?{if bytes.len()+chunk.len()>1024*1024{return Err("RESPONSE_LIMIT_EXCEEDED");}bytes.extend_from_slice(&chunk);}let result=parse(&bytes);emit(json!({"state":"stream_observation","stage":stage,"bytes":bytes.len(),"terminal_valid":result.is_ok()}));let reply=result?;let deadline=Instant::now()+Duration::from_secs(1);loop{let records=sink.0.lock().unwrap();if records.len()>prior{if !matches!(records.last().unwrap().outcome,AnalyticsOutcome::Completed){return Err("ANALYTICS_NOT_COMPLETED");}break;}drop(records);if Instant::now()>deadline{return Err("ANALYTICS_MISSING");}tokio::time::sleep(Duration::from_millis(10)).await;}Ok(reply)}
async fn inference(home:&Path,options:&Options,stage:&str,nonce:&str,result_marker:&str)->Result<()>{let client=reqwest::Client::builder().no_proxy().retry(reqwest::retry::never()).timeout(Duration::from_secs(90)).build().map_err(|_|"HTTP_CLIENT_FAILED")?;let backends=ExecutionBackends::default();let state=SchedulingState::default();let sink=Arc::new(Sink::default());let model={let cfg=captured(home)?;format!("velune/model/{}",cfg.providers.iter().find(|p|p.id=="claude-live").and_then(|p|p.models.first()).ok_or("MODEL_MISSING")?.record_key)};
 let result=async {if stage=="text"||stage=="all"{let r=runner(home,options,backends.clone(),state.clone(),sink.clone(),&format!("text-{nonce}"))?;let reply=request(&client,&r,body(&model,vec![json!({"role":"user","content":format!("Synthetic acceptance. Reply with exactly this text and nothing else: {nonce}")})],false),"text",&sink).await?;if reply.stop!="end_turn"||text(&reply).trim()!=nonce{return Err("TEXT_NONCE_MISMATCH");}emit(json!({"state":"text_complete","nonce_verified":true,"gateway_requests":1}));drop(r);}if stage=="tool"||stage=="all"{let session=format!("tool-{nonce}");let first_user=json!({"role":"user","content":format!("Synthetic acceptance. Call acceptance_marker exactly once with nonce {nonce}. After its result arrives, reply with exactly the marker returned by that tool, with no other text. Do not use any other tools.")});let r=runner(home,options,backends.clone(),state.clone(),sink.clone(),&session)?;let first=request(&client,&r,body(&model,vec![first_user.clone()],true),"tool_call",&sink).await?;let calls:Vec<_>=first.content.iter().filter(|b|b["type"]=="tool_use").collect();if first.stop!="tool_use"||calls.len()!=1||calls[0]["name"]!="acceptance_marker"||calls[0]["input"]["nonce"]!=nonce{return Err("EXPECTED_TOOL_CALL_MISSING");}let id=calls[0]["id"].as_str().filter(|v|!v.is_empty()).ok_or("TOOL_ID_MISSING")?.to_owned();drop(r);emit(json!({"state":"caller_tool_executed","pure_in_memory":true,"tool_calls":1}));let r=runner(home,options,backends.clone(),state,sink.clone(),&session)?;let history=vec![first_user,json!({"role":"assistant","content":first.content}),json!({"role":"user","content":[{"type":"tool_result","tool_use_id":id,"content":result_marker}]})];let reply=request(&client,&r,body(&model,history,true),"tool_result",&sink).await?;if reply.stop!="end_turn"||text(&reply).trim()!=result_marker{return Err("TOOL_RESULT_MARKER_MISMATCH");}emit(json!({"state":"tool_complete","tool_only_marker_verified":true,"gateway_requests":2,"cross_runner":true}));drop(r);}Ok(())}.await;backends.shutdown();result}
fn work()->Result<()>{let args:Vec<String>=std::env::args().collect();if args.get(1).map(String::as_str)==Some("self-check"){return parser_check();}let action=args.get(1).ok_or("ARGUMENTS_MISSING")?;let home=PathBuf::from(args.get(2).ok_or("ARGUMENTS_MISSING")?);let binary=args.get(3).ok_or("ARGUMENTS_MISSING")?;let model=args.get(4).ok_or("ARGUMENTS_MISSING")?;let stage=args.get(5).ok_or("ARGUMENTS_MISSING")?;let nonce=args.get(6).ok_or("ARGUMENTS_MISSING")?;let marker=args.get(7).ok_or("ARGUMENTS_MISSING")?;let (mut app,options)=setup(&home,binary,model)?;let result=if action=="login"{login(&mut app)}else{let status=app.authentication_inspect("default".into(),"claude-live".into()).map_err(|_|"STATUS_FAILED")?;if !status.configured{return Err("NOT_LOGGED_IN");}emit(json!({"state":"configured","configured":true}));if stage=="quota"{quota(&mut app)}else{let rt=tokio::runtime::Builder::new_current_thread().enable_all().build().map_err(|_|"RUNTIME_FAILED")?;let inferred=rt.block_on(inference(&home,&options,stage,nonce,marker));if inferred.is_ok()&&stage=="all"{quota(&mut app)}else{inferred}}};let closed=app.close().map_err(|_|"APPLICATION_CLOSE_FAILED");result.and(closed)}
fn main(){if let Err(reason)=work(){emit(json!({"state":"failed","reason":reason,"automatic_replay":false}));std::process::exit(1);}}
'''


def dedicated_home(value):
    home = Path(value).expanduser().resolve()
    user_home = Path.home().resolve()
    if any((parent / '.git').exists() for parent in (home, *home.parents)):
        raise ValueError('--home 必须位于Git仓库之外')
    if home == user_home or any(home == original or original in home.parents for original in (user_home / '.velune', user_home / '.claude', user_home / '.codex', user_home / '.codex-switcher')) or home == ROOT or ROOT in home.parents:
        raise ValueError('--home 必须是仓库外的独立验收目录')
    marker = home / MARKER
    if home.exists() and any(home.iterdir()) and not marker.is_file():
        raise ValueError('拒绝使用非空且无本脚本标记的目录')
    if marker.exists() and json.loads(marker.read_text()) != {'purpose': 'velune-claude-live', 'version': 1}:
        raise ValueError('验收目录标记不匹配')
    home.mkdir(parents=True, exist_ok=True, mode=0o700)
    home.chmod(0o700)
    marker.write_text(json.dumps({'purpose': 'velune-claude-live', 'version': 1}))
    marker.chmod(0o600)
    return home


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    action = parser.add_mutually_exclusive_group()
    action.add_argument('--login', action='store_true')
    action.add_argument('--run', action='store_true')
    parser.add_argument('--home', help='仓库外专用持久验收目录；login/run必须指定')
    parser.add_argument('--claude', help='官方Claude CLI可执行文件的绝对路径')
    parser.add_argument('--model', default='sonnet')
    parser.add_argument('--stage', choices=['text', 'tool', 'quota', 'all'], default='text')
    parser.add_argument('--source-ref', default='HEAD')
    args = parser.parse_args()
    if (args.login or args.run) and not args.home:
        parser.error('--login/--run 必须显式 --home')
    if args.claude and not Path(args.claude).is_absolute():
        parser.error('--claude 必须是绝对路径')
    binary = args.claude or shutil.which('claude')
    if (args.login or args.run) and (not binary or not Path(binary).is_file()):
        parser.error('必须指定可用的官方 --claude 可执行文件')
    commit = subprocess.run(['git', 'rev-parse', '--verify', args.source_ref + '^{commit}'], cwd=ROOT, capture_output=True, text=True, check=True).stdout.strip()
    with tempfile.TemporaryDirectory(prefix='velune-claude-live-build-') as directory:
        temporary = Path(directory)
        source = temporary / 'source'
        source.mkdir()
        archive = subprocess.run(['git', 'archive', commit, 'Cargo.toml', 'Cargo.lock', 'packages'], cwd=ROOT, capture_output=True, check=True).stdout
        with tarfile.open(fileobj=io.BytesIO(archive)) as stream:
            stream.extractall(source, filter='data')
        for relative, old, new in [
            ('packages/application/src/lib.rs', 'mod authentication_resolver;', 'pub mod authentication_resolver;'),
            ('packages/application/src/authentication_resolver.rs', 'pub(crate) struct ProviderResolver', 'pub struct ProviderResolver'),
            ('packages/application/src/authentication_resolver.rs', 'pub(crate) fn capture(', 'pub fn capture('),
        ]:
            path = source / relative
            text = path.read_text()
            if text.count(old) != 1:
                raise RuntimeError('临时resolver可见性边界变更，停止验收')
            path.write_text(text.replace(old, new))
        harness = temporary / 'harness'
        (harness / 'src').mkdir(parents=True)
        (harness / 'src/main.rs').write_text(PROGRAM)
        dependencies = '\n'.join('%s={path="%s"}' % (name, source / 'packages' / folder) for name, folder in [('velune-application', 'application'), ('velune-gateway', 'gateway')])
        (harness / 'Cargo.toml').write_text('[package]\nname="manual-claude-subscription-live"\nversion="0.0.0"\nedition="2024"\n[workspace]\n[dependencies]\n' + dependencies + '\nserde_json="1"\nreqwest={version="0.13",default-features=false,features=["json"]}\ntokio={version="1",features=["rt","time"]}\n')
        target = ROOT / 'target/manual-claude-live'
        build = subprocess.run(['cargo', 'build', '--quiet', '--offline', '--manifest-path', str(harness / 'Cargo.toml'), '--target-dir', str(target)], cwd=ROOT, capture_output=True, text=True)
        if build.returncode:
            raise RuntimeError('冻结runner编译失败：\n' + build.stderr)
        executable = target / 'debug/manual-claude-subscription-live'
        subprocess.run([str(executable), 'self-check'], cwd=temporary, check=True)
        count = {'text': 1, 'tool': 2, 'quota': 0, 'all': 3}[args.stage] if args.run else 0
        print(json.dumps({'state': 'prepared', 'source_commit': commit, 'mode': 'login' if args.login else 'run' if args.run else 'compile_only', 'stage': args.stage, 'maximum_gateway_inference_requests': count, 'automatic_replay': False, 'quota_samples': int(args.run and args.stage in ('quota', 'all')), 'original_auth_directories_used': False, 'cli_internal_requests': 'managed_by_official_cli'}, ensure_ascii=False), flush=True)
        if not args.login and not args.run:
            return
        home = dedicated_home(args.home)
        environment = dict(os.environ)
        subprocess.run([str(executable), 'login' if args.login else 'run', str(home), str(Path(binary).resolve()), args.model, args.stage, secrets.token_hex(8), secrets.token_hex(12)], cwd=temporary, env=environment, check=True, timeout=370 if args.login else 380)


if __name__ == '__main__':
    try:
        main()
    except subprocess.CalledProcessError as error:
        print(json.dumps({'state': 'process_failed', 'exit_code': error.returncode}), flush=True)
        raise SystemExit(error.returncode if 1 <= error.returncode <= 255 else 1)
    except subprocess.TimeoutExpired:
        print(json.dumps({'state': 'process_failed', 'reason': 'RUNNER_TIMEOUT'}), flush=True)
        raise SystemExit(1)
    except (OSError, ValueError, RuntimeError, tarfile.TarError):
        print(json.dumps({'state': 'prepare_failed', 'reason': 'PREPARATION_FAILED'}), flush=True)
        raise SystemExit(1)
