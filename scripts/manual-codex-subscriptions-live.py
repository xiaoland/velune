#!/usr/bin/env python3
"""只读现有双订阅的真实手动验收；不接入 CI，不提供产品 Switcher 集成。

默认仅编译；--run --switcher-credentials 才由 Rust 只读现有 accounts.json，
只提取两份 ChatGPT access_token/account_id 到短生命周期内存。不提取或刷新
refresh token，不写原认证、Codex 文件或新应用认证目录；401 时停止并要求原来源续期。
启动时解析 --source-ref（默认 HEAD）并冻结完整提交，报告完整 SHA，真实目标不变。

默认 text-a-route 做两次推理：A 返回合成 nonce，再将真实 A 文本投影历史经本地
合成 429 切给真实 B；B 的新问题不含 nonce，答案必须精确匹配。该路径仅验证普通
文本历史，未验证工具、加密 reasoning 或文件引用；本地故障不是上游真实限流证据。
discover-models 仅查询两份模型目录；diagnose-a 做一次 A 文本诊断；basic-b-route
做 B 文本基准和本地故障切换。响应正文仅在内存，日志只含固定安全元数据和 token 计数。
"""
import argparse
import io
import json
from pathlib import Path
import os
import subprocess
import tarfile
import tempfile

ROOT = Path(__file__).resolve().parents[1]




PROGRAM = r'''
use std::{collections::BTreeMap,fs::File,io::{self,BufReader,Write},path::Path,sync::{Arc,Mutex},time::Duration};
use serde_json::{Value,json};
use serde::Deserialize;
use base64::{Engine as _,engine::general_purpose::URL_SAFE_NO_PAD};
use velune_gateway::*;
struct SafeResponseWarnings;
#[derive(Default)]struct SafeWarning{selected:bool,status:Option<u64>,mime:Option<&'static str>,encoding:Option<&'static str>}
impl tracing::field::Visit for SafeWarning{
 fn record_debug(&mut self,_:&tracing::field::Field,_:&dyn std::fmt::Debug){}
 fn record_u64(&mut self,field:&tracing::field::Field,value:u64){match field.name(){"upstream_status" if (100..600).contains(&value)=>self.status=Some(value),_=>{}}}
 fn record_str(&mut self,field:&tracing::field::Field,value:&str){match field.name(){"event"=>self.selected=value=="responses_stream_content_type_rejected","mime"=>self.mime=["absent","sse","json","html","plain","other"].into_iter().find(|tag|*tag==value),"encoding"=>self.encoding=["absent","identity","gzip","br","deflate","zstd","other"].into_iter().find(|tag|*tag==value),_=>{}}}
}
impl<S:tracing::Subscriber>tracing_subscriber::Layer<S> for SafeResponseWarnings{
 fn on_event(&self,event:&tracing::Event<'_>,_:tracing_subscriber::layer::Context<'_,S>){let metadata=event.metadata();if metadata.target()!="velune_ai_provider::responses"||*metadata.level()!=tracing::Level::WARN{return;}let mut safe=SafeWarning::default();event.record(&mut safe);if safe.selected{emit(json!({"state":"provider_diagnostic","event":"responses_stream_content_type_rejected","upstream_status":safe.status,"mime":safe.mime,"encoding":safe.encoding}));}}
}
type Result<T> = std::result::Result<T,String>;
const ENDPOINT:&str="https://chatgpt.com/backend-api/codex";
fn fail(stage:&str)->String {stage.into()}
fn emit(value:Value){println!("{value}");let _=io::stdout().flush();}
#[derive(Deserialize)]struct Store{accounts:Vec<Stored>}
#[derive(Deserialize)]struct Stored{auth_data:Auth}
#[derive(Deserialize)]struct Auth{#[serde(rename="type")]kind:String,access_token:Option<String>,account_id:Option<String>}
struct SourceCredentials{credentials:BTreeMap<String,ResolvedCredential>}
impl CredentialResolver for SourceCredentials {
 fn resolve(&self,reference:String,target:CredentialTarget)->velune_ai::OperationFuture<std::result::Result<ResolvedCredential,CredentialResolutionError>> {
  let value=if target.endpoint!=ENDPOINT||!matches!(target.protocol,GatewayProtocol::ResponsesV1){Err(CredentialResolutionError::UnauthorizedTarget)}else{self.credentials.get(&reference).map(|c|ResolvedCredential{token:c.token.clone(),explicit_output_cap:c.explicit_output_cap,subscription:c.subscription,profile:match &c.profile{CredentialProfile::Standard=>CredentialProfile::Standard,CredentialProfile::CodexSubscription{account_id}=>CredentialProfile::CodexSubscription{account_id:account_id.clone()}}}).ok_or(CredentialResolutionError::Unavailable)};
  Box::pin(async move{value})
 }
 // Default refresh/invalidate do not mutate or renew this external source.
}
fn load(path:&Path)->Result<Arc<dyn CredentialResolver>>{
 // Streaming selective deserialization ignores refresh/id tokens and account metadata.
 let file=File::open(path).map_err(|_|fail("无法只读打开指定认证来源"))?;
 let store:Store=serde_json::from_reader(BufReader::new(file)).map_err(|_|fail("认证来源结构不符合公开格式"))?;
 let auth:Vec<_>=store.accounts.into_iter().filter_map(|a|if a.auth_data.kind=="chat_g_p_t"{Some(a.auth_data)}else{None}).collect();
 if auth.len()!=2{return Err(fail("验收要求恰好两个 ChatGPT 订阅"));}
 let now=std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map_err(|_|fail("时钟无效"))?.as_secs();
 let mut credentials=BTreeMap::new();
 for (id,auth) in ["account-a","account-b"].into_iter().zip(auth){
  let token=auth.access_token.filter(|s|!s.is_empty()&&!s.chars().any(char::is_control)).ok_or_else(||fail("现有 access credential 缺失"))?;
  let account_id=auth.account_id.filter(|s|!s.is_empty()&&!s.chars().any(char::is_control)).ok_or_else(||fail("现有账户绑定缺失"))?;
  let encoded=token.split('.').nth(1).ok_or_else(||fail("现有 access credential 格式不支持"))?;
  let bytes=URL_SAFE_NO_PAD.decode(encoded).map_err(|_|fail("现有 access credential 格式不支持"))?;
  let claims:Value=serde_json::from_slice(&bytes).map_err(|_|fail("现有 access credential 格式不支持"))?;
  if claims["exp"].as_u64().is_none_or(|exp|exp<=now){return Err(fail("现有 access credential 已过期；请先在原来源正常刷新，本脚本不续期"));}
  credentials.insert(id.into(),ResolvedCredential{token,explicit_output_cap:Some(false),subscription:true,profile:CredentialProfile::CodexSubscription{account_id}});
 }
 emit(json!({"state":"source_loaded_read_only","account_count":2,"authentication_types":["chat_g_p_t","chat_g_p_t"],"refresh_enabled":false}));
 Ok(Arc::new(SourceCredentials{credentials}))
}
#[derive(Default)]struct Sink(Mutex<Vec<AnalyticsRecord>>);
impl AnalyticsSink for Sink{fn record(&self,record:AnalyticsRecord){emit(json!({"state":"analytics_observation","provider":record.provider_id,"status":record.status,"outcome":format!("{:?}",record.outcome),"usage_reported":record.usage_reported,"usage_complete":record.usage_complete}));if let Ok(mut records)=self.0.lock(){records.push(record);}}}
struct InjectedResolver{real:Arc<dyn CredentialResolver>,endpoint:String}
impl CredentialResolver for InjectedResolver {
 fn resolve(&self,reference:String,target:CredentialTarget)->velune_ai::OperationFuture<std::result::Result<ResolvedCredential,CredentialResolutionError>> {
  if reference=="local-injected-a" {
   let allowed=target.endpoint==self.endpoint && matches!(target.protocol,GatewayProtocol::ResponsesV1);
   return Box::pin(async move{if !allowed{Err(CredentialResolutionError::UnauthorizedTarget)}else{Ok(ResolvedCredential{token:"fixture-local-429-only".into(),explicit_output_cap:None,subscription:false,profile:CredentialProfile::Standard})}});
  }self.real.resolve(reference,target)
 }
 fn refresh(&self,reference:String,target:CredentialTarget,rejected:String)->velune_ai::OperationFuture<std::result::Result<ResolvedCredential,CredentialResolutionError>> {self.real.refresh(reference,target,rejected)}
 fn invalidate(&self,reference:String,target:CredentialTarget,rejected:String)->velune_ai::OperationFuture<()> {self.real.invalidate(reference,target,rejected)}
}
fn runner(real:Arc<dyn CredentialResolver>,model:&str,ids:&[&str],injection:Option<&str>,sink:Arc<Sink>)->Result<Runner>{
 let providers:Vec<_>=ids.iter().map(|id|ProviderDefinition{id:(*id).into(),name:(*id).into(),protocol:GatewayProtocol::ResponsesV1,endpoint:if *id=="account-a"{injection.unwrap_or(ENDPOINT).into()}else{ENDPOINT.into()},credential_ref:Some(if *id=="account-a"&&injection.is_some(){"local-injected-a".into()}else{(*id).into()}),quota_limits:vec![],models:vec![ProviderModel{record_key:(*id).into(),provider_model_id:model.into(),nickname:"合成验收".into(),icon:None,context_window:None,max_output_tokens:None,reasoning_levels:None}]}).collect();
 let routes=if ids.len()>1{vec![ModelRoute{record_key:"live-pool".into(),name:"验收订阅池".into(),members:providers.iter().map(|p|p.models[0].record_key.clone()).collect(),strategy:RoutingStrategy::Order,sticky:false,weights:BTreeMap::new()}]}else{Vec::new()};
 let resolver:Arc<dyn CredentialResolver>=if let Some(endpoint)=injection{Arc::new(InjectedResolver{real,endpoint:endpoint.into()})}else{real};
 Runner::start_with_analytics(GatewayConfig{id:"default".into(),name:"隔离验收".into(),providers,routes},resolver,BTreeMap::new(),Some(sink),SchedulingState::default()).map_err(|_|fail("启动隔离网关失败"))
}
async fn diagnostic(response:&mut reqwest::Response)->Value {
 let mut bytes=Vec::new();while let Ok(Some(chunk))=response.chunk().await{if bytes.len()+chunk.len()>16384{return json!({"structured_error":"too_large"});}bytes.extend_from_slice(&chunk);}
 let Ok(value)=serde_json::from_slice::<Value>(&bytes)else{return json!({"structured_error":"not_json"});};
 let error=&value["error"];
 let kind=error["type"].as_str().filter(|s|matches!(*s,"invalid_request_error"|"authentication_error"|"permission_error"|"not_found_error"|"rate_limit_error"|"server_error"|"api_error"|"invalid_request"|"bad_request"));
 let code=error["code"].as_str().filter(|s|matches!(*s,"model_not_found"|"unsupported_model"|"unsupported_parameter"|"invalid_value"|"invalid_request"|"invalid_json"|"context_length_exceeded"|"insufficient_quota"|"usage_limit_reached"|"quota_exceeded"|"account_deactivated"|"token_expired"|"invalid_token"|"invalid_api_key"|"access_denied"|"unauthorized"|"feature_not_supported"));
 let param=error["param"].as_str().filter(|s|s.len()<=64&&s.bytes().all(|b|b.is_ascii_alphanumeric()||b"_.-[]".contains(&b))).filter(|s|matches!(s.split(['.','[']).next().unwrap_or(""),"model"|"input"|"instructions"|"tools"|"tool_choice"|"parallel_tool_calls"|"include"|"reasoning"|"max_output_tokens"|"temperature"|"top_p"|"stream"|"store"|"previous_response_id"|"conversation"|"text"|"service_tier"));
 let message=error["message"].as_str().or_else(||value["detail"].as_str()).or_else(||value["message"].as_str()).unwrap_or("").to_ascii_lowercase();
 let mut tags=Vec::new();
 for (field,needles) in [("tool_choice",vec!["tool_choice","tool choice"]),("parallel_tool_calls",vec!["parallel_tool_calls"]),("text",vec!["text"]),("verbosity",vec!["verbosity"]),("tools",vec!["tools"]),("stream",vec!["stream"]),("store",vec!["store"]),("instructions",vec!["instructions"]),("input",vec!["input"]),("include",vec!["include"]),("reasoning",vec!["reasoning"]),("model",vec!["model"]),("session",vec!["session"]),("user-agent",vec!["user-agent","user agent"]),("max_output_tokens",vec!["max_output_tokens"]),("temperature",vec!["temperature"]),("top_p",vec!["top_p"]),("service_tier",vec!["service_tier"])]{if needles.iter().any(|n|message.contains(n)){tags.push(format!("field:{field}"));}}
 for (tag,needles) in [("value:auto",vec!["auto"]),("unsupported",vec!["unsupported","not supported"]),("required",vec!["required"]),("invalid",vec!["invalid"]),("missing",vec!["missing"]),("not_allowed",vec!["not allowed"]),("constraint",vec!["must ","must be"]),("unknown",vec!["unknown"]),("unavailable",vec!["unavailable","not available","not found","does not exist"]),("array",vec!["array"]),("string",vec!["string"]),("boolean",vec!["boolean"])]{if needles.iter().any(|n|message.contains(n)){tags.push(tag.into());}}
 json!({"type":kind,"code":code,"param":param,"message_tags":tags,"unrecognized_fields_omitted":true})
}
fn safe_usage(value:&Value)->Value{
 json!({"input_tokens":value["input_tokens"].as_u64(),"output_tokens":value["output_tokens"].as_u64(),"total_tokens":value["total_tokens"].as_u64(),"cached_tokens":value["input_tokens_details"]["cached_tokens"].as_u64().or_else(||value["cached_tokens"].as_u64()),"reasoning_tokens":value["output_tokens_details"]["reasoning_tokens"].as_u64().or_else(||value["reasoning_tokens"].as_u64())})
}
struct Reply{text:String,usage:Value}
const EVENT_TAGS:&[&str]=&["response.created","response.in_progress","response.queued","response.completed","response.failed","response.incomplete","response.cancelled","response.canceled","response.output_item.added","response.output_item.done","response.content_part.added","response.content_part.done","response.output_text.delta","response.output_text.done","response.function_call_arguments.delta","response.function_call_arguments.done","response.reasoning_summary_text.delta","response.reasoning_summary_text.done","response.reasoning_text.delta","response.reasoning_text.done","error"];
fn protocol_tag(value:&str)->&'static str{EVENT_TAGS.iter().copied().find(|tag|*tag==value).unwrap_or("other")}
fn status_tag(value:&str)->&'static str{match value{"completed"=>"completed","failed"=>"failed","incomplete"=>"incomplete","cancelled"|"canceled"=>"cancelled","queued"=>"queued","in_progress"=>"in_progress",_=>"other"}}
#[derive(Default)]struct StreamParser{event:Option<String>,data:Vec<String>,event_counts:BTreeMap<&'static str,u64>,type_counts:BTreeMap<&'static str,u64>,status_counts:BTreeMap<&'static str,u64>,frames:u64,json_invalid:u64,text:String,complete:Option<Value>,failed:bool,error_tags:Vec<String>}
impl StreamParser{
 fn frame(&mut self){
  if self.event.is_none()&&self.data.is_empty(){return;}self.frames+=1;
  let event=self.event.take();if let Some(name)=&event{*self.event_counts.entry(protocol_tag(name)).or_default()+=1;}
  let data=std::mem::take(&mut self.data).join("\n");if data.is_empty()||data=="[DONE]"{return;}
  let Ok(value)=serde_json::from_str::<Value>(&data)else{self.json_invalid+=1;return;};
  let kind=value["type"].as_str();if let Some(kind)=kind{*self.type_counts.entry(protocol_tag(kind)).or_default()+=1;}
  let response=value.get("response").unwrap_or(&value);
  if let Some(status)=response["status"].as_str(){*self.status_counts.entry(status_tag(status)).or_default()+=1;}
  let names=[event.as_deref(),kind];
  if names.iter().flatten().any(|name|matches!(*name,"response.failed"|"response.incomplete"|"response.cancelled"|"response.canceled"|"error"))||response["status"].as_str().is_some_and(|s|matches!(s,"failed"|"incomplete"|"cancelled"|"canceled")){self.failed=true;let message=response["error"]["message"].as_str().or_else(||value["error"]["message"].as_str()).or_else(||value["message"].as_str()).unwrap_or("").to_ascii_lowercase();for tag in ["model","tool_choice","input","instructions","reasoning","encrypted","unsupported","required","invalid","quota","limit"]{if message.contains(tag)&&!self.error_tags.iter().any(|old|old==tag){self.error_tags.push(tag.into());}}}
  if names.iter().flatten().any(|name|*name=="response.output_text.delta")&&let Some(delta)=value["delta"].as_str(){self.text.push_str(delta);}
  if names.iter().flatten().any(|name|*name=="response.completed")||response["status"]=="completed"{self.complete=Some(response.clone());}
 }
 fn parse(mut self,text:&str)->(Result<Reply>,Value){
  for line in text.lines(){let line=line.strip_suffix('\r').unwrap_or(line);if line.is_empty(){self.frame();continue;}if line.starts_with(':'){continue;}let (field,value)=line.split_once(':').unwrap_or((line,""));let value=value.strip_prefix(' ').unwrap_or(value);match field{"event"=>self.event=Some(value.into()),"data"=>self.data.push(value.into()),_=>{}}}
  self.frame();
  let metadata=json!({"frames":self.frames,"event_counts":self.event_counts,"data_type_counts":self.type_counts,"response_status_counts":self.status_counts,"invalid_json_frames":self.json_invalid,"failure_message_tags":self.error_tags,"explicit_completed":self.complete.is_some(),"failure_terminal":self.failed});
  if self.failed{return(Err(fail("提供商显式失败、不完整或取消终态")),metadata);}if self.json_invalid>0{return(Err(fail("SSE JSON 帧无效")),metadata);}
  let Some(response)=self.complete else{return(Err(fail("没有明确 completed 业务终态")),metadata);};
  let output=response["output"].as_array().cloned().unwrap_or_default();
  if self.text.is_empty(){for item in &output{if let Some(content)=item["content"].as_array(){for block in content{if block["type"]=="output_text"&&let Some(text)=block["text"].as_str(){self.text.push_str(text);}}}}}
  (Ok(Reply{text:self.text,usage:safe_usage(&response["usage"])}),metadata)
 }
}
async fn invoke(client:&reqwest::Client,runner:&Runner,key:&str,mut body:Value,stage:&str)->Result<Reply>{
 body["model"]=json!(format!("velune/model/{key}"));body["stream"]=json!(true);body["store"]=json!(false);
 emit(json!({"state":"inference_started","stage":stage}));
 let response=client.post(format!("{}/responses",runner.endpoint())).bearer_auth(runner.token()).header("accept","*/*").json(&body).send().await;
 let mut response=response.map_err(|_|fail(&format!("{stage}：传输失败")))?;
 let status=response.status().as_u16();
 let content_type_header=response.headers().get("content-type").and_then(|h|h.to_str().ok()).and_then(|v|v.split(';').next()).unwrap_or("").trim().to_ascii_lowercase();
 let content_type=match content_type_header.as_str(){"text/event-stream"=>"text/event-stream","application/json"=>"application/json","text/plain"=>"text/plain","application/octet-stream"=>"application/octet-stream",_=>"other"};
 emit(json!({"state":"inference_headers","stage":stage,"status":status,"natural_429":status==429}));
 if status!=200{let error=diagnostic(&mut response).await;emit(json!({"state":"upstream_error","stage":stage,"status":status,"error":error}));}
 if status==401{return Err(fail(&format!("{stage}：HTTP 401，现有认证被拒绝；请先在原来源正常刷新，本脚本不续期")));}
 if status!=200{return Err(fail(&format!("{stage}：HTTP {status}，停止，不自动重跑")));}
 let mut bytes=Vec::new();while let Some(chunk)=response.chunk().await.map_err(|_|fail(&format!("{stage}：流中断")))?{if bytes.len()+chunk.len()>512*1024{return Err(fail("验收响应超出内存边界"));}bytes.extend_from_slice(&chunk);}
 let text=match std::str::from_utf8(&bytes){Ok(text)=>text,Err(_)=>{emit(json!({"state":"stream_observation","stage":stage,"content_type":content_type,"bytes":bytes.len(),"utf8_valid":false}));return Err(fail("响应 UTF-8 无效"));}};
 let (reply,metadata)=StreamParser::default().parse(text);
 let envelope=serde_json::from_str::<Value>(text).ok();let json_error_envelope=envelope.as_ref().is_some_and(|v|v.get("error").is_some());
 let envelope_message=envelope.as_ref().and_then(|v|v["error"]["message"].as_str()).unwrap_or("").to_ascii_lowercase();
 let envelope_tags:Vec<_>=["stream","content-type","unsupported","upstream","failed","model","invalid"].into_iter().filter(|tag|envelope_message.contains(tag)).collect();
 emit(json!({"state":"stream_observation","stage":stage,"content_type":content_type,"bytes":bytes.len(),"json_error_envelope":json_error_envelope,"json_error_message_tags":envelope_tags,"protocol":metadata}));
 let reply=reply.map_err(|reason|fail(&format!("{stage}：{reason}")))?;
 emit(json!({"state":"inference_complete","stage":stage,"usage":reply.usage}));Ok(reply)
}

async fn discover(credentials:[ResolvedCredential;2])->Result<Value>{
 let client=reqwest::Client::builder().redirect(reqwest::redirect::Policy::none()).retry(reqwest::retry::never()).timeout(Duration::from_secs(60)).build().map_err(|_|fail("创建目录客户端失败"))?;
 let mut catalogs=Vec::new();
 for (index,credential) in credentials.into_iter().enumerate(){
  let CredentialProfile::CodexSubscription{account_id}=credential.profile else{return Err(fail("模型目录要求订阅认证"));};
  let mut response=client.get(format!("{ENDPOINT}/models?client_version=0.162.0")).bearer_auth(credential.token).header("chatgpt-account-id",account_id).header("originator","codex_cli_rs").header("user-agent","codex_cli_rs/0.162.0 Velune/0.1").header("version","0.162.0").send().await.map_err(|_|fail("模型目录请求失败"))?;
  let status=response.status().as_u16();emit(json!({"state":"model_directory_headers","account":if index==0{"account-a"}else{"account-b"},"status":status}));
  if status!=200{let error=diagnostic(&mut response).await;emit(json!({"state":"model_directory_error","status":status,"error":error}));return Err(fail("真实模型目录不可用；停止，不猜模型"));}
  let mut bytes=Vec::new();while let Some(chunk)=response.chunk().await.map_err(|_|fail("模型目录流中断"))?{if bytes.len()+chunk.len()>1024*1024{return Err(fail("模型目录超过 1 MiB"));}bytes.extend_from_slice(&chunk);}
  let value:Value=serde_json::from_slice(&bytes).map_err(|_|fail("模型目录不是 JSON"))?;
  let models=value["models"].as_array().ok_or_else(||fail("模型目录缺少 models"))?;let mut catalog=BTreeMap::new();
  for model in models{let slug=model["slug"].as_str().or_else(||model["model_id"].as_str()).or_else(||model["id"].as_str()).unwrap_or("");
   if slug.is_empty()||slug.len()>128||!slug.bytes().all(|b|b.is_ascii_alphanumeric()||b"._-".contains(&b))||model["visibility"]=="hide"{continue;}
   let visibility=model["visibility"].as_str().filter(|v|matches!(*v,"list"|"show"|"visible"|"public")).unwrap_or("unspecified");
   catalog.insert(slug.to_owned(),(model["priority"].as_i64().unwrap_or(0),visibility.to_owned()));
  }catalogs.push(catalog);
 }
 let mut common=Vec::new();for (slug,(priority_a,visibility_a)) in &catalogs[0]{if let Some((priority_b,visibility_b))=catalogs[1].get(slug){common.push(json!({"slug":slug,"priority_a":priority_a,"priority_b":priority_b,"visibility_a":visibility_a,"visibility_b":visibility_b}));}}
 common.sort_by_key(|v|v["priority_a"].as_i64().unwrap_or(0).max(v["priority_b"].as_i64().unwrap_or(0)));
 if common.is_empty(){return Err(fail("两个账户没有共同公开模型；停止，不猜模型"));}
 Ok(json!({"case":"discover-models","logical_requests":0,"metadata_requests":2,"different_accounts":true,"common_models":common}))
}
async fn live(real:Arc<dyn CredentialResolver>,model:&str,fixture:&str,nonce:&str,case:&str)->Result<Value>{
 let target=||CredentialTarget{protocol:GatewayProtocol::ResponsesV1,endpoint:ENDPOINT.into()};
 let a=real.resolve("account-a".into(),target()).await.map_err(|_|fail("A 凭据不可用"))?;let b=real.resolve("account-b".into(),target()).await.map_err(|_|fail("B 凭据不可用"))?;
 let different=matches!((&a.profile,&b.profile),(CredentialProfile::CodexSubscription{account_id:a},CredentialProfile::CodexSubscription{account_id:b}) if a!=b);
 if !different{return Err(fail("两个登录属于同一账户，停止调用"));}emit(json!({"state":"distinct_accounts_verified","different_accounts":true}));
 if case=="discover-models"{return discover([a,b]).await;}drop(a);drop(b);
 let client=reqwest::Client::builder().no_proxy().redirect(reqwest::redirect::Policy::none()).retry(reqwest::retry::never()).timeout(Duration::from_secs(60)).build().map_err(|_|fail("创建验收客户端失败"))?;
 let sink=Arc::new(Sink::default());
 if case=="diagnose-a"||case=="basic-b-route"||case=="text-a-route" {
  let id=if case=="diagnose-a"||case=="text-a-route"{"account-a"}else{"account-b"};
  let plain=json!({"instructions":"Keep output minimal.","input":[{"role":"user","content":format!("Synthetic acceptance. Reply only with {nonce}.")}]});
  let baseline=runner(real.clone(),model,&[id],None,sink.clone())?;
  let reply=invoke(&client,&baseline,id,plain.clone(),if id=="account-a"{"A 极简文本诊断"}else{"B 极简文本基准"}).await?;drop(baseline);
  if reply.text.trim()!=nonce{return Err(fail("极简文本未精确返回合成 nonce"));}
  if !sink.0.lock().map_err(|_|fail("分析记录不可用"))?.iter().any(|r|r.provider_id==id&&r.outcome==AnalyticsOutcome::Completed){return Err(fail("生产分析未确认基准 Completed"));}
  if case=="diagnose-a"{return Ok(json!({"case":case,"logical_requests":1,"maximum_inference_http_attempts":1,"account_a_plain_baseline":true,"history_scope":if case=="text-a-route"{"ordinary_text"}else{"none"},"usage":[reply.usage]}));}
  let injected=runner(real.clone(),model,&["account-a","account-b"],Some(fixture),sink.clone())?;
  let routed_body=if case=="text-a-route"{json!({"instructions":"Keep output minimal.","input":[plain["input"][0].clone(),{"role":"assistant","content":[{"type":"output_text","text":reply.text}]},{"role":"user","content":"For this synthetic acceptance, reply only with the exact nonce from the previous assistant message."}]})}else{plain};
  let routed=invoke(&client,&injected,"live-pool",routed_body,"本地 429 注入后真实 B").await?;drop(injected);
  if routed.text.trim()!=nonce{return Err(fail("基础切换未精确返回合成 nonce"));}
  let records=sink.0.lock().map_err(|_|fail("分析记录不可用"))?;
  if !records.iter().any(|r|r.provider_id=="account-b"&&r.status==Some(200)&&r.outcome==AnalyticsOutcome::Completed){return Err(fail("生产分析未确认切换后的 B Completed"));}
  if records.iter().filter(|r|r.provider_id=="account-a"&&r.status==Some(429)).count()!=1{return Err(fail("本地故障注入不唯一"));}
  let attempts:Vec<_>=records.iter().map(|r|json!({"provider":r.provider_id,"status":r.status,"outcome":format!("{:?}",r.outcome)})).collect();
  return Ok(json!({"case":case,"different_accounts":true,"logical_requests":2,"maximum_inference_http_attempts":2,"account_a_plain_baseline":case=="text-a-route","account_b_plain_baseline":case=="basic-b-route","ordinary_text_history_after_failover":case=="text-a-route","opaque_or_tool_history_tested":false,"basic_failover":true,"fault":"local_synthetic_429","natural_rate_limit_failover_verified":false,"history_scope":if case=="text-a-route"{"ordinary_text"}else{"none"},"usage":[reply.usage,routed.usage],"attempts":attempts}));
 }
 Err(fail("未知验收用例"))
}

fn run()->Result<()> {
 use tracing_subscriber::layer::SubscriberExt;
 let _=tracing::subscriber::set_global_default(tracing_subscriber::registry().with(SafeResponseWarnings));
 let args:Vec<String>=std::env::args().collect();
 if args.get(1).is_some_and(|s|s=="parser-check"){
  let cases=[("event: response.completed\ndata: {\"response\":\ndata: {\"status\":\"completed\",\"output\":[]}}\n\n",true),("data: {\"type\":\"response.completed\",\"response\":{\"status\":\"completed\",\"output\":[]}}\n\n",true),("event: response.failed\ndata: {\"response\":{\"status\":\"failed\"}}\n\n",false),("data: [DONE]\n\n",false),("event: response.in_progress\ndata: {\"response\":{\"status\":\"in_progress\"}}\n\n",false)];
  for (fixture,completed) in cases{let (reply,_)=StreamParser::default().parse(fixture);if reply.is_ok()!=completed{return Err(fail("本地 SSE 解析自检失败"));}}
  let usage=safe_usage(&json!({"input_tokens":1,"output_tokens":2,"total_tokens":3,"input_tokens_details":{"cached_tokens":1},"output_tokens_details":{"reasoning_tokens":2},"attribution":{"items":[{"id":"synthetic-private-id"}]}}));
  if usage.as_object().is_none_or(|v|v.len()!=5)||usage.to_string().contains("synthetic-private-id")||usage["cached_tokens"]!=1||usage["reasoning_tokens"]!=2{return Err(fail("usage 白名单自检失败"));}
  emit(json!({"state":"parser_checked","fixture_cases":5,"usage_allowlist_checked":true,"real_requests":0}));return Ok(());
 }
 let source=Path::new(args.get(1).ok_or_else(||fail("缺少只读来源路径"))?);let model=args.get(2).ok_or_else(||fail("缺少模型 slug"))?;let fixture=args.get(3).ok_or_else(||fail("缺少故障注入地址"))?;
 let case=args.get(4).ok_or_else(||fail("缺少验收用例"))?;
 let credentials=load(source)?;
 let nonce=format!("VELUNE_FIXTURE_{}",std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map_err(|_|fail("时钟无效"))?.as_nanos());
 let runtime=tokio::runtime::Builder::new_current_thread().enable_all().build().map_err(|_|fail("创建调用运行时失败"))?;
 let evidence=runtime.block_on(live(credentials,model,fixture,&nonce,case))?;emit(json!({"state":"complete","evidence":evidence}));Ok(())
}
fn main(){if let Err(stage)=run(){emit(json!({"state":"failed","stage":stage}));std::process::exit(1);}}
'''


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('--source-ref', default='HEAD', help='解析并冻结的完整 Git 提交；默认 HEAD')
    parser.add_argument('--run', action='store_true', help='执行所选用例；默认两次普通文本历史/切换推理，不登录或续期')
    parser.add_argument('--switcher-credentials', action='store_true', help='只读现有两份 access credential，不作为产品集成')
    parser.add_argument('--case', choices=['diagnose-a','basic-b-route','discover-models','text-a-route'], default='text-a-route', help='discover-models不推理；text-a-route两请求验证文本历史与切换；其他为诊断入口')
    parser.add_argument('--model', help='仅推理case需要；必须从共同目录明确选择，不设默认模型')
    args = parser.parse_args()
    if args.run and not args.switcher_credentials:
        parser.error('--run 需要明确指定 --switcher-credentials')
    if args.run and args.case != 'discover-models' and not args.model:
        parser.error('推理 case 必须显式 --model，先用 discover-models 获取共同目录')
    source_commit = subprocess.run(['git','rev-parse','--verify',args.source_ref + '^{commit}'],cwd=ROOT,capture_output=True,text=True,check=True).stdout.strip()
    credential_source = Path.home() / '.codex-switcher/accounts.json'
    with tempfile.TemporaryDirectory(prefix='velune-subscriptions-live-') as directory:
        temporary = Path(directory)
        source = temporary / 'source'
        source.mkdir()
        archive = subprocess.run(['git', 'archive', source_commit, 'Cargo.toml', 'Cargo.lock', 'packages', 'app/host'], cwd=ROOT, capture_output=True, check=True).stdout
        with tarfile.open(fileobj=io.BytesIO(archive)) as stream:
            stream.extractall(source, filter='data')
        harness = temporary / 'harness'
        (harness / 'src').mkdir(parents=True)
        (harness / 'src/main.rs').write_text(PROGRAM)
        dependencies = '\n'.join('%s={path="%s"}' % (name, source / 'packages' / folder) for name, folder in [('velune-gateway','gateway'),('velune-ai','ai')])
        (harness / 'Cargo.toml').write_text('[package]\nname="manual-codex-subscriptions-live"\nversion="0.0.0"\nedition="2024"\n[workspace]\n[dependencies]\n' + dependencies + '\nreqwest={version="0.13",default-features=false,features=["json"]}\ntokio={version="1",features=["rt","time"]}\nserde_json="1"\nserde={version="1",features=["derive"]}\nbase64="0.23"\ntracing="0.1"\ntracing-subscriber="0.3"\n')
        target = ROOT / 'target/manual-subscriptions-live'
        build = subprocess.run(['cargo','build','--quiet','--offline','--manifest-path',str(harness / 'Cargo.toml'),'--target-dir',str(target)], cwd=ROOT, capture_output=True, text=True)
        if build.returncode:
            raise RuntimeError('冻结 Rust runner 编译失败：\n' + build.stderr)
        parser_check = subprocess.run([str(target/'debug/manual-codex-subscriptions-live'),'parser-check'],cwd=temporary,capture_output=True,text=True)
        if parser_check.returncode:
            raise RuntimeError('本地 SSE 解析自检失败（未读取凭据/调用模型）')
        print(parser_check.stdout.strip(),flush=True)
        request_count = {'diagnose-a':1,'basic-b-route':2,'discover-models':0,'text-a-route':2}[args.case]
        print(json.dumps({'state':'prepared','source_commit':source_commit,'real_endpoint_unchanged':True,'logical_requests':request_count,'maximum_inference_http_attempts':request_count,'case':args.case,'metadata_requests':2 if args.case == 'discover-models' else 0,'refresh_enabled':False,'mode':'read_only_existing_source'},ensure_ascii=False),flush=True)
        if not args.run:
            return
        from http.server import BaseHTTPRequestHandler, ThreadingHTTPServer
        import threading
        class Inject429(BaseHTTPRequestHandler):
            def log_message(self,*_):
                pass
            def do_POST(self):
                self.rfile.read(int(self.headers.get('Content-Length','0')))
                if self.headers.get('Authorization') != 'Bearer fixture-local-429-only':
                    self.send_error(400)
                    return
                self.server.injected += 1
                payload=b'{"error":{"type":"usage_limit_reached","message":"local synthetic acceptance fault"}}'
                self.send_response(429)
                self.send_header('Content-Type','application/json')
                self.send_header('Content-Length',str(len(payload)))
                self.send_header('Retry-After','60')
                self.end_headers()
                self.wfile.write(payload)
        server=ThreadingHTTPServer(('127.0.0.1',0),Inject429)
        server.injected=0
        threading.Thread(target=server.serve_forever,daemon=True).start()
        process_home=temporary/'process-home'
        process_home.mkdir()
        environment=dict(os.environ,HOME=str(process_home))
        try:
            result=subprocess.run([str(target/'debug/manual-codex-subscriptions-live'),str(credential_source),args.model or 'unused-model','http://127.0.0.1:%d/v1'%server.server_port,args.case],cwd=temporary,env=environment)
            if result.returncode:
                raise SystemExit(result.returncode)
            if server.injected != (0 if args.case in ('diagnose-a','discover-models') else 1):
                raise RuntimeError('本地故障注入次数不是一次，不能认定切换验收通过。')
        finally:
            server.shutdown()
            server.server_close()


if __name__=='__main__':
    main()
