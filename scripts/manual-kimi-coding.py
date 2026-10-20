#!/usr/bin/env python3
"""显式人工 Kimi Coding 验收；临时源码、隔离 HOME、回环 HTTP，不接入 CI。

仅在临时副本把固定 Kimi 目标改为回环并公开 CoreRuntime 的真实装配入口。
覆盖账户/额度与网关错误行为；不证明真实供应商资格、TLS 或账单硬上限。
运行 python3 scripts/manual-kimi-coding.py；不读取真实凭据或访问供应商。
"""
import datetime
import json
import os
from pathlib import Path
import shutil
import subprocess
import tempfile
import threading
from http.server import BaseHTTPRequestHandler, ThreadingHTTPServer

ROOT = Path(__file__).resolve().parents[1]
LOCK = threading.Lock()
MODES, HITS, GATES = {}, {}, {}
ERRORS = []


def sse(value):
    return ('event: %s\ndata: %s\n\n' % (value['type'], json.dumps(value))).encode()


class Fixture(BaseHTTPRequestHandler):
    protocol_version = 'HTTP/1.1'
    def log_message(self, *_):
        pass
    def handle(self):
        try:
            super().handle()
        except (ConnectionResetError, BrokenPipeError):
            pass
    def reply(self, status, value, headers=None):
        body = value if isinstance(value, bytes) else json.dumps(value).encode()
        self.send_response(status)
        self.send_header('Content-Type', 'application/json')
        self.send_header('Content-Length', str(len(body)))
        for key, value in (headers or {}).items():
            self.send_header(key, value)
        self.end_headers()
        try:
            self.wfile.write(body)
        except (BrokenPipeError, ConnectionResetError):
            pass
    def do_GET(self):
        if self.path == '/stats':
            with LOCK:
                value = dict(HITS)
            self.reply(200, value)
            return
        key = self.headers.get('Authorization', '').removeprefix('Bearer ')
        kind = self.path.rsplit('/', 1)[-1]
        with LOCK:
            HITS[kind + ':' + key] = HITS.get(kind + ':' + key, 0) + 1
            mode = MODES.get(key, 'normal')
        if kind == 'me' and mode in ('me401', 'me403'):
            self.reply(int(mode[-3:]), {'error': {'message': 'synthetic metadata denial'}})
            return
        if kind == 'me':
            self.reply(200, {'user_id': 'owner-' + key.split('-')[0]})
            return
        if kind != 'usages':
            self.reply(404, {})
            return
        if mode in ('usage401', 'usage403'):
            self.reply(int(mode[-3:]), {'error': {'message': 'synthetic metadata denial'}})
            return
        reset = (datetime.datetime.now(datetime.timezone.utc) + datetime.timedelta(hours=2)).isoformat()
        windows = {name: {'used_ratio': 0.2, 'reset_time': reset} for name in ('limit_5h', 'limit_7d', 'limit_month_total', 'limit_month_code')}
        if mode == 'cap':
            windows['limit_5h']['used_ratio'] = 0.8
        elif mode == 'string':
            windows['limit_5h']['used_ratio'] = '0.25'
        elif mode == 'short':
            windows = {'limit_5h': windows['limit_5h']}
        elif mode == 'empty':
            windows = {}
        elif mode == 'missing_ratio':
            del windows['limit_5h']['used_ratio']
        elif mode == 'missing_reset':
            del windows['limit_5h']['reset_time']
        elif mode == 'bad_reset':
            windows['limit_5h']['reset_time'] = 'invalid'
        elif mode in ('negative', 'overflow'):
            windows['limit_5h']['used_ratio'] = -0.1 if mode == 'negative' else 1.1
        elif mode == 'hold':
            with LOCK:
                gate = GATES[key]
            gate.wait(12)
            windows['limit_5h']['used_ratio'] = 0.99
        self.reply(200, {'usages': windows})
    def do_POST(self):
        value = json.loads(self.rfile.read(int(self.headers.get('Content-Length', '0'))))
        if self.path == '/control':
            with LOCK:
                if value.get('release'):
                    GATES[value['key']].set()
                else:
                    MODES[value['key']] = value['mode']
                    if value['mode'] == 'hold':
                        GATES[value['key']] = threading.Event()
            self.reply(200, {})
            return
        key = self.headers.get('x-api-key') or self.headers.get('Authorization', '').removeprefix('Bearer ')
        with LOCK:
            HITS['inference:' + key] = HITS.get('inference:' + key, 0) + 1
            mode = MODES.get(key, 'normal')
        if self.path.startswith('/coding/') and (len(self.headers.get_all('User-Agent', [])) != 1 or not self.headers.get('User-Agent', '').startswith('Velune/')):
            ERRORS.append('missing honest user-agent')
        failures = {
            'quota': (403, "You've reached your 5-hour usage limit."),
            'weekly': (403, "You've reached your weekly (7-day) usage limit."),
            'monthly': (403, "You've reached your monthly usage limit for this billing cycle."),
            'plan': (401, 'Your current subscription does not have access to k3.'),
            'risk': (403, 'risk control denied'),
            'disabled': (500, 'unauthenticated: failed_precondition: 因违反用户协议，该账号已被禁用'),
            'server': (500, 'ordinary server failure'),
            'rate': (429, "We're receiving too many requests"),
            'overloaded': (429, 'The engine is currently overloaded'),
        }
        if mode in ('redirect307', 'redirect308'):
            self.reply(int(mode[-3:]), {}, {'Location': self.server.base + '/redirect-target'})
            return
        if self.path == '/redirect-target':
            ERRORS.append('redirect followed')
        if mode == 'disconnect':
            self.connection.close()
            self.close_connection = True
            return
        if mode in failures:
            status, message = failures[mode]
            self.reply(status, {'error': {'message': message}}, {'x-fixture-error': mode, 'Retry-After': '60'})
            return
        if mode == 'hold':
            with LOCK:
                gate = GATES[key]
            gate.wait(12)
        try:
            self.send_response(200)
            self.send_header('Content-Type', 'text/event-stream')
            self.send_header('Connection', 'close')
            self.end_headers()
            values = [
                {'type':'message_start','message':{'id':'fixture','type':'message','role':'assistant','model':'fixture','content':[],'usage':{'input_tokens':1,'output_tokens':0}}},
                {'type':'content_block_start','index':0,'content_block':{'type':'text','text':''}},
                {'type':'content_block_delta','index':0,'delta':{'type':'text_delta','text':'SELECTED:'+key}},
                {'type':'content_block_stop','index':0},
                {'type':'message_delta','delta':{'stop_reason':'end_turn'},'usage':{'output_tokens':1}},
                {'type':'message_stop'},
            ]
            if mode == 'submitted':
                values = values[:1]
            for item in values:
                self.wfile.write(sse(item))
            self.wfile.flush()
        except (BrokenPipeError, ConnectionResetError):
            pass


PROGRAM = r'''
use std::{collections::BTreeMap,path::PathBuf,sync::{Arc,atomic::{AtomicUsize,Ordering}},time::{Duration,Instant}};
use serde_json::{Value,json};
use velune_gateway::*;
use velune_application::{Application,Options,ProviderDraft,AuthenticationEdit};
struct Auth;
impl CredentialResolver for Auth{fn resolve(&self,id:String,target:CredentialTarget)->velune_ai::OperationFuture<Result<ResolvedCredential,CredentialResolutionError>>{Box::pin(async move{Ok(ResolvedCredential{material:CredentialMaterial::Http{authorization:if target.protocol==GatewayProtocol::MessagesV1{HttpAuthorization::ApiKey(id)}else{HttpAuthorization::Bearer(id)},account_id:None},recovery:CredentialRecovery::None,request_capabilities:Default::default()})})}}
struct ReplacingAuth(SchedulingState);
impl CredentialResolver for ReplacingAuth{fn resolve(&self,id:String,target:CredentialTarget)->velune_ai::OperationFuture<Result<ResolvedCredential,CredentialResolutionError>>{let state=self.0.clone();Box::pin(async move{state.invalidate_credentials("fixture",&id);Auth.resolve(id,target).await})}}
#[derive(Default)]struct Quotas(AtomicUsize);
impl QuotaSource for Quotas{fn snapshot(&self,_:&str)->QuotaSnapshot{QuotaSnapshot::Unknown}fn quota_exhausted(&self,_:&str){self.0.fetch_add(1,Ordering::SeqCst);}}
fn provider(id:&str,up:&str,profile:RequestProfile)->ProviderDefinition{ProviderDefinition{id:id.into(),name:id.into(),protocol:GatewayProtocol::MessagesV1,execution_backend:ExecutionBackend::Http,request_profile:profile,endpoint:format!("{up}/{}",if profile==RequestProfile::KimiCoding{"coding"}else{"backup"}),credential_ref:Some(id.into()),quota_limits:vec![],models:vec![ProviderModel{record_key:id.into(),provider_model_id:"fixture".into(),nickname:id.into(),icon:None,context_window:None,max_output_tokens:Some(16),reasoning_levels:None}]}}
fn config(up:&str,id:&str,profile:RequestProfile)->GatewayConfig{GatewayConfig{id:"fixture".into(),name:"fixture".into(),providers:vec![provider(id,up,profile),provider("backup",up,RequestProfile::Native)],routes:vec![ModelRoute{record_key:"pool".into(),name:"pool".into(),members:vec![id.into(),"backup".into()],strategy:RoutingStrategy::Order,sticky:true,weights:BTreeMap::new()}]}}
fn runner(c:GatewayConfig,state:SchedulingState,q:Arc<Quotas>)->Runner{Runner::start_with_scheduling(c,Arc::new(Auth),BTreeMap::new(),None,state,q,Some(SessionIdentity{namespace:"fixture".into(),id:"session".into()}),ExecutionBackends::default()).unwrap()}
async fn call(c:&reqwest::Client,r:&Runner,key:&str)->(u16,String,reqwest::header::HeaderMap){let response=c.post(format!("{}/messages",r.endpoint())).bearer_auth(r.token()).header("anthropic-version","2023-06-01").header("user-agent","untrusted-caller").json(&json!({"model":format!("velune/model/{key}"),"messages":[{"role":"user","content":"synthetic"}],"max_tokens":16,"stream":true})).send().await.unwrap();let s=response.status().as_u16();let h=response.headers().clone();(s,response.text().await.unwrap_or_default(),h)}
async fn control(c:&reqwest::Client,up:&str,key:&str,mode:&str){c.post(format!("{up}/control")).json(&json!({"key":key,"mode":mode})).send().await.unwrap();}
async fn hits(c:&reqwest::Client,up:&str,key:&str)->u64{c.get(format!("{up}/stats")).send().await.unwrap().json::<Value>().await.unwrap()[key].as_u64().unwrap_or(0)}
fn mark(passed:&mut Vec<String>,value:String){eprintln!("PASS {value}");passed.push(value);}
fn ok(r:&(u16,String,reqwest::header::HeaderMap),key:&str){assert_eq!(r.0,200);assert!(r.1.contains("message_stop"));assert!(r.1.contains(&format!("SELECTED:{key}")));}
fn draft(id:&str,endpoint:&str)->ProviderDraft{serde_json::from_value(json!({"id":id,"name":id,"executionBackend":"http","requestProfile":"kimiCoding","authenticationSource":"apiKey","quotaSource":"kimiUsage","protocol":"messagesV1","endpoint":endpoint,"quotaLimits":[],"models":[{"recordKey":id,"providerModelId":"fixture","nickname":"fixture","icon":null,"contextWindow":8192,"maxOutputTokens":16,"reasoningLevels":null}]})).unwrap()}
fn save(app:&mut Application,id:&str,endpoint:&str,key:&str){app.save_provider("default".into(),draft(id,endpoint),AuthenticationEdit::SetApiKey{value:key.into()}).unwrap();}
fn quota(app:&mut Application,id:&str)->Value{serde_json::to_value(app.provider_quota("default".into(),id.into()).unwrap()).unwrap()}
fn refresh(app:&mut Application,id:&str)->Value{app.refresh_provider_quota("default".into(),id.into()).unwrap();let deadline=Instant::now()+Duration::from_secs(6);loop{let q=quota(app,id);if q["refreshing"]==false{return q;}assert!(Instant::now()<deadline,"quota refresh did not finish");std::thread::sleep(Duration::from_millis(10));}}
#[tokio::main(flavor="current_thread")]
async fn main(){let args:Vec<_>=std::env::args().collect();let up=&args[1];let home=PathBuf::from(&args[2]);let client=reqwest::Client::builder().no_proxy().redirect(reqwest::redirect::Policy::none()).retry(reqwest::retry::never()).timeout(Duration::from_secs(15)).build().unwrap();let mut passed=Vec::new();

 let state=SchedulingState::default();let conf=config(up,"await-epoch",RequestProfile::KimiCoding);let r=Runner::start_with_scheduling(conf,Arc::new(ReplacingAuth(state.clone())),BTreeMap::new(),None,state,Arc::new(Quotas::default()),None,ExecutionBackends::default()).unwrap();assert_eq!(call(&client,&r,"await-epoch").await.0,503);assert_eq!(hits(&client,up,"inference:await-epoch").await,0);mark(&mut passed,"credential resolution epoch changes reject dispatch before upstream".into());
 for mode in ["quota","weekly","monthly"]{control(&client,up,mode,mode).await;let q=Arc::new(Quotas::default());let r=runner(config(up,mode,RequestProfile::KimiCoding),SchedulingState::default(),q.clone());let before=hits(&client,up,"inference:backup").await;let direct=call(&client,&r,mode).await;assert_eq!(direct.0,403);assert_eq!(direct.2["x-fixture-error"],mode);let message=match mode{"quota"=>"You've reached your 5-hour usage limit.","weekly"=>"You've reached your weekly (7-day) usage limit.",_=>"You've reached your monthly usage limit for this billing cycle."};assert_eq!(direct.1,format!("{{\"error\": {{\"message\": \"{message}\"}}}}"),"native error bytes changed");assert_eq!(serde_json::from_str::<Value>(&direct.1).unwrap()["error"]["message"].as_str().unwrap().contains("usage limit"),true);assert_eq!(q.0.load(Ordering::SeqCst),1);assert_eq!(hits(&client,up,"inference:backup").await,before);drop(r);let r=runner(config(up,mode,RequestProfile::KimiCoding),SchedulingState::default(),q.clone());ok(&call(&client,&r,"pool").await,"backup");assert_eq!(q.0.load(Ordering::SeqCst),2);mark(&mut passed,format!("{mode}: original direct wire, callback and route fallback"));}
 for mode in ["plan","risk","disabled"]{control(&client,up,mode,mode).await;let r=runner(config(up,mode,RequestProfile::KimiCoding),SchedulingState::default(),Arc::new(Quotas::default()));let before=hits(&client,up,"inference:backup").await;let reply=call(&client,&r,"pool").await;assert_eq!(reply.0,if mode=="plan"{401}else if mode=="risk"{403}else{500});assert_eq!(reply.2["x-fixture-error"],mode);assert_eq!(hits(&client,up,"inference:backup").await,before);mark(&mut passed,format!("{mode}: stop without fallback"));}
 for mode in ["server","rate","overloaded"]{control(&client,up,mode,mode).await;let r=runner(config(up,mode,RequestProfile::KimiCoding),SchedulingState::default(),Arc::new(Quotas::default()));ok(&call(&client,&r,"pool").await,"backup");let before=hits(&client,up,&format!("inference:{mode}")).await;assert_eq!(call(&client,&r,mode).await.0,429);assert_eq!(hits(&client,up,&format!("inference:{mode}")).await,before);mark(&mut passed,format!("{mode}: eligible fallback then shared direct cooldown"));}
 control(&client,up,"native","quota").await;let r=runner(config(up,"native",RequestProfile::Native),SchedulingState::default(),Arc::new(Quotas::default()));assert_eq!(call(&client,&r,"pool").await.0,403);mark(&mut passed,"Native does not inherit Kimi quota classification".into());
 for mode in ["redirect307","redirect308"]{control(&client,up,mode,mode).await;let r=runner(config(up,mode,RequestProfile::KimiCoding),SchedulingState::default(),Arc::new(Quotas::default()));assert_ne!(call(&client,&r,mode).await.0,200);mark(&mut passed,format!("{mode}: no second hop"));}

 for protocol in [GatewayProtocol::MessagesV1,GatewayProtocol::ResponsesV1,GatewayProtocol::ChatCompletionsV1]{for mode in ["redirect307","redirect308","disconnect"]{let id=format!("security-{protocol:?}-{mode}");control(&client,up,&id,mode).await;let mut conf=config(up,&id,RequestProfile::KimiCoding);conf.providers.truncate(1);conf.routes.clear();conf.providers[0].protocol=protocol.clone();if protocol!=GatewayProtocol::MessagesV1{conf.providers[0].endpoint.push_str("/v1");}let r=runner(conf,SchedulingState::default(),Arc::new(Quotas::default()));let (path,body)=match protocol{GatewayProtocol::MessagesV1=>("/messages",json!({"model":format!("velune/model/{id}"),"messages":[{"role":"user","content":"fixture"}],"max_tokens":16,"stream":true})),GatewayProtocol::ResponsesV1=>("/responses",json!({"model":format!("velune/model/{id}"),"input":"fixture","stream":true})),_=>("/chat/completions",json!({"model":format!("velune/model/{id}"),"messages":[{"role":"user","content":"fixture"}],"stream":true}))};let response=client.post(format!("{}{path}",r.endpoint())).bearer_auth(r.token()).header("anthropic-version","2023-06-01").json(&body).send().await.unwrap();assert_ne!(response.status().as_u16(),200);let _=response.bytes().await;assert_eq!(hits(&client,up,&format!("inference:{id}")).await,1,"one logical dispatch retried automatically");}mark(&mut passed,format!("{protocol:?}: 307/308 denied and transport failure sent once"));}
 let id="cancel-held";control(&client,up,id,"hold").await;let r=runner(config(up,id,RequestProfile::KimiCoding),SchedulingState::default(),Arc::new(Quotas::default()));let endpoint=r.endpoint().to_owned();let token=r.token().to_owned();let before=hits(&client,up,"inference:backup").await;let c=client.clone();let task=tokio::spawn(async move{c.post(format!("{endpoint}/messages")).bearer_auth(token).header("anthropic-version","2023-06-01").json(&json!({"model":"velune/model/pool","messages":[{"role":"user","content":"fixture"}],"max_tokens":16,"stream":true})).send().await});let deadline=Instant::now()+Duration::from_secs(3);while hits(&client,up,"inference:cancel-held").await==0{assert!(Instant::now()<deadline);tokio::time::sleep(Duration::from_millis(10)).await;}task.abort();let _=task.await;drop(r);client.post(format!("{up}/control")).json(&json!({"key":id,"release":true})).send().await.unwrap();assert_eq!(hits(&client,up,"inference:backup").await,before);mark(&mut passed,"cancelled held dispatch never falls through to another provider".into());
 control(&client,up,"submitted","submitted").await;let r=runner(config(up,"submitted",RequestProfile::KimiCoding),SchedulingState::default(),Arc::new(Quotas::default()));let before=hits(&client,up,"inference:backup").await;let reply=call(&client,&r,"pool").await;assert_eq!(reply.0,200);assert!(!reply.1.contains("message_stop"));assert_eq!(hits(&client,up,"inference:backup").await,before);mark(&mut passed,"submitted incomplete stream never switches".into());

 // Validate production target rules before loopback substitution only changes identity.
 for protocol in [GatewayProtocol::MessagesV1,GatewayProtocol::ChatCompletionsV1,GatewayProtocol::ResponsesV1]{let mut c=config(up,"shape",RequestProfile::KimiCoding);c.providers.truncate(1);c.routes.clear();c.providers[0].protocol=protocol.clone();c.providers[0].endpoint=format!("{up}{}",if protocol==GatewayProtocol::MessagesV1{"/coding"}else{"/coding/v1"});assert!(c.validate().is_ok());for suffix in ["?key=synthetic","#fragment","/wrong"]{let mut bad=c.clone();bad.providers[0].endpoint.push_str(suffix);assert!(bad.validate().is_err());}}
 mark(&mut passed,"three protocol target combinations reject path/query/fragment drift".into());
 let templates=velune_application::provider_templates();let template=templates.iter().find(|t|t.id=="kimiCodingPlan").unwrap();assert_eq!(serde_json::to_value(template).unwrap()["quotaSource"],"kimiUsage");for id in ["codexSubscription","claudeCodeSubscription","api"]{assert!(templates.iter().any(|t|t.id==id));}mark(&mut passed,"Kimi template and existing subscription templates remain distinct".into());
 std::fs::create_dir_all(home.join("resources")).unwrap();let options=Options{home_directory:home.clone(),resources_directory:home.join("resources")};let mut app=Application::open(options.clone()).unwrap();let endpoint=format!("{up}/coding/");save(&mut app,"account",&endpoint,"alice-one");let q=refresh(&mut app,"account");assert_eq!(q["freshness"],"fresh");assert_eq!(q["windows"].as_array().unwrap().len(),4);assert_eq!(q["windows"][0]["usedPercent"],20.0);let ids:std::collections::BTreeSet<_>=q["windows"].as_array().unwrap().iter().map(|w|w["id"].as_str().unwrap()).collect();assert_eq!(ids,std::collections::BTreeSet::from(["kimi:limit_5h","kimi:limit_7d","kimi:limit_month_total","kimi:limit_month_code"]));let public=serde_json::to_string(&app.authentication_inspect("default".into(),"account".into()).unwrap()).unwrap();assert!(!public.contains("owner-alice")&&!public.contains("alice-one"));mark(&mut passed,"Application verifies owner, safe label and dynamic four-window quota".into());
 let model=app.manual_model("account");let old=app.manual_runner();ok(&call(&client,&old,&model).await,"alice-one");control(&client,up,"alice-one","rate").await;assert_eq!(call(&client,&old,&model).await.0,429);save(&mut app,"account",&endpoint,"alice-two");refresh(&mut app,"account");let rotated=app.manual_runner();assert_eq!(call(&client,&rotated,&model).await.0,429);assert_ne!(call(&client,&old,&model).await.0,200);assert_eq!(hits(&client,up,"inference:alice-two").await,0);mark(&mut passed,"same-owner key rotation preserves cooldown and invalidates captured runner".into());
 save(&mut app,"account",&endpoint,"bob-one");refresh(&mut app,"account");let changed=app.manual_runner();ok(&call(&client,&changed,&model).await,"bob-one");assert_ne!(call(&client,&rotated,&model).await.0,200);mark(&mut passed,"new owner clears old health and invalidates old dispatch".into());

 save(&mut app,"sticky",&endpoint,"delta-one");save(&mut app,"standby",&endpoint,"echo-one");refresh(&mut app,"sticky");refresh(&mut app,"standby");let sticky=app.manual_sticky();ok(&call(&client,&sticky,"sticky-route").await,"delta-one");drop(sticky);save(&mut app,"sticky",&endpoint,"delta-two");refresh(&mut app,"sticky");let sticky=app.manual_sticky();ok(&call(&client,&sticky,"sticky-route").await,"delta-two");mark(&mut passed,"same-owner key rotation preserves explicit session sticky binding".into());
 save(&mut app,"duplicate",&endpoint,"bob-two");let duplicate=refresh(&mut app,"duplicate");assert_ne!(duplicate["freshness"],"fresh");let r=app.manual_runner();let duplicate_model=app.manual_model("duplicate");assert_ne!(call(&client,&r,&duplicate_model).await.0,200);assert_eq!(hits(&client,up,"inference:bob-two").await,0);mark(&mut passed,"different keys of one owner are not independent providers".into());
 for mode in ["string","short","empty","missing_ratio","missing_reset","bad_reset","negative","overflow"]{let id=format!("parse-{mode}");let key=format!("{mode}-one");control(&client,up,&key,mode).await;save(&mut app,&id,&endpoint,&key);let q=refresh(&mut app,&id);if mode=="string"{assert_eq!(q["freshness"],"fresh");assert_eq!(q["windows"][0]["usedPercent"],25.0);}else if mode=="short"{assert_eq!(q["freshness"],"fresh");assert_eq!(q["windows"].as_array().unwrap().len(),1);}else{assert_eq!(q["freshness"],"unknown");assert!(q["windows"].as_array().unwrap().is_empty());}mark(&mut passed,format!("quota boundary {mode}"));}
 let mut capped=draft("capped",&endpoint);capped.quota_limits=vec![velune_application::config::QuotaLimit{window_id:"kimi:limit_5h".into(),max_used_percent:50}];control(&client,up,"cap-one","cap").await;app.save_provider("default".into(),capped.clone(),AuthenticationEdit::SetApiKey{value:"cap-one".into()}).unwrap();refresh(&mut app,"capped");let r=app.manual_runner();let key=app.manual_model("capped");assert_ne!(call(&client,&r,&key).await.0,200);control(&client,up,"cap-two","cap").await;app.save_provider("default".into(),capped,AuthenticationEdit::SetApiKey{value:"cap-two".into()}).unwrap();refresh(&mut app,"capped");let r=app.manual_runner();assert_ne!(call(&client,&r,&key).await.0,200);assert_eq!(hits(&client,up,"inference:cap-two").await,0);mark(&mut passed,"same-owner rotation preserves cap configuration and gate".into());

 // Hold an old observation until after save: late 99% must never publish for a replacement owner.
 control(&client,up,"bob-one","hold").await;let before=hits(&client,up,"usages:bob-one").await;app.refresh_provider_quota("default".into(),"account".into()).unwrap();let deadline=Instant::now()+Duration::from_secs(4);while hits(&client,up,"usages:bob-one").await==before{assert!(Instant::now()<deadline);tokio::time::sleep(Duration::from_millis(10)).await;}
 save(&mut app,"account",&endpoint,"carol-one");client.post(format!("{up}/control")).json(&json!({"key":"bob-one","release":true})).send().await.unwrap();let q=quota(&mut app,"account");assert_ne!(q["freshness"],"fresh");let q=refresh(&mut app,"account");assert_eq!(q["freshness"],"fresh");assert_eq!(q["windows"][0]["usedPercent"],20.0);mark(&mut passed,"generation replacement cannot publish old in-flight quota".into());

 control(&client,up,"carol-one","hold").await;let before=hits(&client,up,"usages:carol-one").await;app.refresh_provider_quota("default".into(),"account".into()).unwrap();let deadline=Instant::now()+Duration::from_secs(4);while hits(&client,up,"usages:carol-one").await==before{assert!(Instant::now()<deadline);tokio::time::sleep(Duration::from_millis(10)).await;}control(&client,up,"carol-one","normal").await;app.delete_provider("default".into(),"account".into()).unwrap();save(&mut app,"account",&endpoint,"carol-one");let recreated_model=app.manual_model("account");let q=refresh(&mut app,"account");assert_eq!(q["freshness"],"fresh");assert_eq!(q["windows"][0]["usedPercent"],20.0);client.post(format!("{up}/control")).json(&json!({"key":"carol-one","release":true})).send().await.unwrap();let q=quota(&mut app,"account");assert_eq!(q["windows"][0]["usedPercent"],20.0);mark(&mut passed,"delete/recreate with same id and key rejects old quota incarnation".into());
 let persisted=std::fs::read(home.join("generic-config.json")).unwrap();for field in ["endpoint","requestProfile","quotaSource","authenticationSource"]{let mut value=serde_json::to_value(draft("invalid",&endpoint)).unwrap();value[field]=json!(match field{"endpoint"=>"https://example.invalid/coding/","requestProfile"=>"native","quotaSource"=>"none",_=>"none"});let invalid:ProviderDraft=serde_json::from_value(value).unwrap();assert!(app.save_provider("default".into(),invalid,AuthenticationEdit::SetApiKey{value:"invalid-key".into()}).is_err());assert_eq!(std::fs::read(home.join("generic-config.json")).unwrap(),persisted);}
 mark(&mut passed,"invalid application composition leaves persisted config unchanged".into());

 for mode in ["me401","me403"]{let id=format!("boundary-{mode}");let key=format!("{mode}-one");control(&client,&up,&key,mode).await;save(&mut app,&id,&endpoint,&key);let q=refresh(&mut app,&id);assert_eq!(q["freshness"],"unknown");assert!(!app.authentication_inspect("default".into(),id.clone()).unwrap().configured);assert_eq!(app.read_provider_api_key("default".into(),id.clone()).unwrap(),key);let model=app.manual_model(&id);let r=app.manual_runner();assert_ne!(call(&client,&r,&model).await.0,200);assert_eq!(hits(&client,&up,&format!("inference:{key}")).await,0);mark(&mut passed,format!("{mode}: unverified key retained, model dispatch rejected"));}
 for mode in ["usage401","usage403"]{for capped in [false,true]{let id=format!("boundary-{mode}-{capped}");let key=format!("{mode}{capped}-one");control(&client,&up,&key,mode).await;let mut d=draft(&id,&endpoint);if capped{d.quota_limits=vec![velune_application::config::QuotaLimit{window_id:"kimi:limit_5h".into(),max_used_percent:50}];}app.save_provider("default".into(),d,AuthenticationEdit::SetApiKey{value:key.clone()}).unwrap();let q=refresh(&mut app,&id);assert_eq!(q["freshness"],"unknown");assert!(app.authentication_inspect("default".into(),id.clone()).unwrap().configured);assert_eq!(app.read_provider_api_key("default".into(),id.clone()).unwrap(),key);let model=app.manual_model(&id);let r=app.manual_runner();let reply=call(&client,&r,&model).await;if capped{assert_ne!(reply.0,200);assert_eq!(hits(&client,&up,&format!("inference:{key}")).await,0);}else{ok(&reply,&key);}mark(&mut passed,format!("{mode}: verified owner, quota unknown, cap={capped} honored"));}}
 app.close().unwrap();let mut app=Application::open(options).unwrap();let q=refresh(&mut app,"account");assert_eq!(q["freshness"],"fresh");let r=app.manual_runner();ok(&call(&client,&r,&recreated_model).await,"carol-one");app.close().unwrap();mark(&mut passed,"public save/reopen retains composition and credential binding".into());
 println!("{}",json!({"passed":passed,"network":"synthetic loopback only","real_credentials_read":false}));
}
'''


def replace(path, old, new):
    value = path.read_text()
    assert value.count(old) == 1, ('source rewrite changed', path.name, old[:60], value.count(old))
    path.write_text(value.replace(old, new))


def main():
    with tempfile.TemporaryDirectory(prefix='velune-kimi-manual-') as directory:
        temp = Path(directory)
        source = temp / 'source'
        source.mkdir()
        for filename in ('Cargo.toml', 'Cargo.lock'):
            shutil.copy2(ROOT / filename, source / filename)
        shutil.copytree(ROOT / 'packages', source / 'packages')
        server = ThreadingHTTPServer(('127.0.0.1', 0), Fixture)
        server.base = 'http://127.0.0.1:%d' % server.server_port
        threading.Thread(target=server.serve_forever, daemon=True).start()
        base = server.base
        # Narrow rewrites keep all other profiles' target validation intact.
        p = source/'packages/ai-provider/src/config.rs'
        value = p.read_text()
        start = value.index('if request_profile == RequestProfile::KimiCoding {')
        head, block = value[:start], value[start:]
        for old, new in [('endpoint.transport() != Transport::Https', 'endpoint.transport() != Transport::Http'), ('!matches!(endpoint.host(), "api.kimi.com" | "api.kimi.ai")', 'endpoint.host() != "127.0.0.1"'), ('endpoint.port().get() != 443', 'endpoint.port().get() != %d' % server.server_port)]:
            assert block.count(old) == 1, old
            block = block.replace(old, new)
        p.write_text(head + block)
        p = source/'packages/gateway/src/config.rs'
        value = p.read_text()
        start = value.index('if provider.request_profile == RequestProfile::KimiCoding {')
        head, block = value[:start], value[start:]
        for old, new in [('endpoint.scheme() != "https"', 'endpoint.scheme() != "http"'), ('!matches!(endpoint.host_str(), Some("api.kimi.com" | "api.kimi.ai"))','endpoint.host_str() != Some("127.0.0.1")'), ('endpoint.port_or_known_default() != Some(443)', 'endpoint.port_or_known_default() != Some(%d)' % server.server_port)]:
            assert block.count(old) == 1, old
            block = block.replace(old, new)
        p.write_text(head + block)
        replace(source/'packages/application/src/kimi_quota.rs', 'https://api.kimi.com/coding/v1', base+'/coding/v1')
        replace(source/'packages/application/src/kimi_quota.rs', 'https://api.kimi.ai/coding/v1', base+'/coding/v1')
        replace(source/'packages/application/src/config.rs', '"https://api.kimi.com/coding/" | "https://api.kimi.com/coding"', json.dumps(base+'/coding/')+' | '+json.dumps(base+'/coding'))
        with (source/'packages/application/src/lib.rs').open('a') as handle:
            handle.write('\nimpl Application { pub fn manual_model(&self,id:&str)->String{self.runtime.manual_model(id)} pub fn manual_runner(&self)->velune_gateway::Runner{self.runtime.manual_runner()} pub fn manual_sticky(&self)->velune_gateway::Runner{self.runtime.manual_sticky()} }\n')
        with (source/'packages/application/src/local/mod.rs').open('a') as handle:
            handle.write('''\nimpl CoreRuntime {
 pub fn manual_model(&self,id:&str)->String{self.gateways.iter().find(|g|g.id=="default").unwrap().providers.iter().find(|p|p.id==id).unwrap().models[0].record_key.clone()}
 pub fn manual_sticky(&self)->velune_gateway::Runner{let g=self.gateways.iter().find(|g|g.id=="default").unwrap();let mut config=g.to_gateway_config().unwrap();config.routes.push(velune_gateway::ModelRoute{record_key:"sticky-route".into(),name:"sticky-route".into(),members:vec![self.manual_model("sticky"),self.manual_model("standby")],strategy:velune_gateway::RoutingStrategy::Rotate,sticky:true,weights:Default::default()});velune_gateway::Runner::start_with_scheduling(config,crate::authentication_resolver::ProviderResolver::capture_for_dispatch(g,&self.options,self.subscription_quota.clone()),Default::default(),None,self.scheduling_state.clone(),self.subscription_quota.source(&g.id),Some(velune_gateway::SessionIdentity{namespace:"fixture".into(),id:"sticky".into()}),self.execution_backends.clone()).unwrap()}
 pub fn manual_runner(&self)->velune_gateway::Runner{let g=self.gateways.iter().find(|g|g.id=="default").unwrap();velune_gateway::Runner::start_with_scheduling(g.to_gateway_config().unwrap(),crate::authentication_resolver::ProviderResolver::capture_for_dispatch(g,&self.options,self.subscription_quota.clone()),std::collections::BTreeMap::new(),None,self.scheduling_state.clone(),self.subscription_quota.source(&g.id),None,self.execution_backends.clone()).unwrap()}
}\n''')
        harness = temp/'harness'
        (harness/'src').mkdir(parents=True)
        (harness/'src/main.rs').write_text(PROGRAM)
        deps = '\n'.join('%s={path="%s"}' % (name, source/'packages'/folder) for name, folder in [('velune-gateway','gateway'),('velune-ai','ai'),('velune-application','application')])
        (harness/'Cargo.toml').write_text('[package]\nname="manual-kimi-coding"\nversion="0.0.0"\nedition="2024"\n[workspace]\n[dependencies]\n'+deps+'\nserde_json="1"\nreqwest={version="0.13",default-features=false,features=["json"]}\ntokio={version="1",features=["macros","rt","time"]}\n')
        target = ROOT/'target/manual-kimi-coding'
        try:
            subprocess.run(['cargo','build','--quiet','--offline','--manifest-path',str(harness/'Cargo.toml'),'--target-dir',str(target)],check=True,cwd=ROOT)
            environment = dict(os.environ, HOME=str(temp/'home'), HTTP_PROXY='http://127.0.0.1:1', HTTPS_PROXY='http://127.0.0.1:1', ALL_PROXY='http://127.0.0.1:1', NO_PROXY='127.0.0.1,localhost', no_proxy='127.0.0.1,localhost')
            subprocess.run([str(target/'debug/manual-kimi-coding'),base,str(temp/'application')],check=True,cwd=temp,env=environment,timeout=150)
            assert not ERRORS, ERRORS
        finally:
            for gate in GATES.values():
                gate.set()
            server.shutdown()
            server.server_close()


if __name__ == '__main__':
    main()
