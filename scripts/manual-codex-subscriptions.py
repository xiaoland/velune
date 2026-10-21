#!/usr/bin/env python3
"""手动运行的 Codex 多订阅隔离验收，不接入测试或 CI。

只使用临时 HOME 和合成凭据。临时复制 Rust 源码，把固定 OAuth/backend
目标改到 loopback，并仅公开 application resolver 供组合验收；产品源码不变。
因此证据覆盖 Rust 登录、持久化、刷新和 gateway 行为，不证明真实 OAuth、
TLS、供应商资格、模型兼容性或真实双订阅可用。所有源代码重写都核对命中数。
运行：python3 scripts/manual-codex-subscriptions.py
"""
import base64
import hashlib
import json
import os
from pathlib import Path
import shutil
import socket
import subprocess
import tempfile
import threading
import time
from http.server import BaseHTTPRequestHandler, ThreadingHTTPServer
from urllib.parse import parse_qs, urlencode, urlparse

ROOT = Path(__file__).resolve().parents[1]
LOCK = threading.Lock()
CODES = {}
REFRESH = {}
COUNTS = {}
ERRORS = []
QUOTA_MODE = 'normal'
QUOTA_GATE = threading.Event()
QUOTA_ENTERED = threading.Event()


def check(ok, message):
    if not ok:
        raise RuntimeError(message)


def b64(value):
    return base64.urlsafe_b64encode(value).decode().rstrip('=')


def jwt(label, generation):
    claims = {'exp': int(time.time()) + 3600, 'sub': 'fixture-user-' + label, 'fixture_label': label,
              'fixture_generation': generation,
              'https://api.openai.com/auth': {'chatgpt_account_id': 'account-' + label}}
    return b64(b'{"alg":"fixture"}') + '.' + b64(json.dumps(claims).encode()) + '.fixture'


def count(key):
    with LOCK:
        COUNTS[key] = COUNTS.get(key, 0) + 1


def tokens(label, generation):
    refresh = 'fixture-refresh-%s-%d' % (label, generation)
    with LOCK:
        REFRESH[refresh] = (label, generation)
    token = jwt(label, generation)
    return {'access_token': token, 'refresh_token': refresh, 'id_token': token}


def event(value):
    return 'event: %s\ndata: %s\n\n' % (value['type'], json.dumps(value))


class Fixture(BaseHTTPRequestHandler):
    protocol_version = 'HTTP/1.1'

    def log_message(self, *_):
        pass

    def reply(self, status, value):
        payload = json.dumps(value).encode()
        self.send_response(status)
        self.send_header('Content-Type', 'application/json')
        self.send_header('Content-Length', str(len(payload)))
        if status == 429:
            self.send_header('Retry-After', '120')
        self.end_headers()
        self.wfile.write(payload)

    def do_GET(self):
        global QUOTA_MODE
        query = parse_qs(urlparse(self.path).query)
        if urlparse(self.path).path == '/quota-control':
            mode = query.get('mode', ['status'])[0]
            if mode == 'hold':
                QUOTA_MODE = 'hold'
                QUOTA_GATE.clear()
                QUOTA_ENTERED.clear()
            elif mode == 'release':
                QUOTA_GATE.set()
            elif mode == 'normal':
                QUOTA_MODE = 'normal'
            self.reply(200, {'entered': QUOTA_ENTERED.is_set()})
            return
        if urlparse(self.path).path == '/usage':
            token = self.headers.get('Authorization', '').removeprefix('Bearer ')
            claims = json.loads(base64.urlsafe_b64decode(token.split('.')[1] + '=='))
            label = claims['fixture_label']
            count('quota:' + label)
            mode = QUOTA_MODE
            if mode == 'hold':
                QUOTA_ENTERED.set()
                QUOTA_GATE.wait(10)
            used = -1 if label == 'quota-invalid' else 99 if mode == 'hold' else 37.5
            try:
                self.reply(200, {'rate_limit': {'allowed': True, 'primary_window': {'used_percent': used, 'limit_window_seconds': 18000, 'reset_at': int(time.time()) + 120}}})
            except (BrokenPipeError, ConnectionResetError):
                pass
            return
        if urlparse(self.path).path == '/fixture/status':
            self.reply(200, dict(COUNTS))
            return
        if urlparse(self.path).path == '/sink':
            count('redirect:sink')
            self.reply(200, {})
            return
        try:
            check(urlparse(self.path).path == '/oauth/authorize', 'unexpected fixture GET')
            label = query['fixture_account'][0]
            code = 'fixture-code-' + label
            check(query['code_challenge_method'] == ['S256'], 'PKCE method')
            check(query['client_id'] == ['app_EMoamEEZ73f0CkXaXp7hrann'], 'client identity')
            check(query['originator'] == ['codex_cli_rs'], 'originator')
            with LOCK:
                CODES[code] = (label, query['code_challenge'][0])
            callback = query['redirect_uri'][0] + '?' + urlencode({'state': query['state'][0], 'code': code})
            self.send_response(302)
            self.send_header('Location', callback)
            self.send_header('Content-Length', '0')
            self.end_headers()
        except Exception as error:
            ERRORS.append(str(error))
            self.reply(400, {'error': 'fixture authorization rejected'})

    def do_POST(self):
        body = self.rfile.read(int(self.headers.get('Content-Length', '0')))
        if self.path == '/sink':
            count('redirect:sink')
            self.reply(200, {})
            return
        if self.path == '/oauth/token':
            form = parse_qs(body.decode())
            try:
                if form['grant_type'] == ['authorization_code']:
                    with LOCK:
                        label, challenge = CODES.pop(form['code'][0])
                    check(b64(hashlib.sha256(form['code_verifier'][0].encode()).digest()) == challenge, 'PKCE verifier')
                    count('login:' + label)
                    self.reply(200, tokens(label, 0))
                else:
                    with LOCK:
                        previous = REFRESH.get(form['refresh_token'][0])
                    if previous is None:
                        count('refresh:reused')
                        self.reply(400, {'error': 'invalid_grant'})
                        return
                    label, generation = previous
                    count('refresh:' + label)
                    if label == 'transient' and COUNTS['refresh:transient'] == 1:
                        self.reply(503, {'error': 'temporarily_unavailable'})
                        return
                    with LOCK:
                        REFRESH.pop(form['refresh_token'][0])
                    time.sleep(.6 if label == 'cancelled' else .15)
                    self.reply(200, tokens(label, generation + 1))
            except Exception as error:
                ERRORS.append(str(error))
                self.reply(400, {'error': 'fixture token rejected'})
            return
        try:
            check(self.path == '/backend-api/codex/responses', 'unexpected inference path')
            token = self.headers.get('Authorization', '').removeprefix('Bearer ')
            if token == 'fixture-standardmissing':
                label, generation = 'standardmissing', 0
            else:
                claims = json.loads(base64.urlsafe_b64decode(token.split('.')[1] + '=='))
                label, generation = claims['fixture_label'], claims['fixture_generation']
                check(self.headers.get('ChatGPT-Account-Id') == 'account-' + label, 'account header')
                check(self.headers.get('originator') == 'codex_cli_rs', 'inference originator')
                check(self.headers.get_all('Accept') == ['text/event-stream'], 'Codex Accept must be a single SSE value')
                check(self.headers.get('version') == '0.162.0', 'Codex client version')
            count('request:' + label)
            request = json.loads(body)
            check(request.get('stream') is True and request.get('store') is False, 'Codex stream/store contract')
            if label in ('redirect302', 'redirect307'):
                self.send_response(int(label.removeprefix('redirect')))
                self.send_header('Location', 'http://127.0.0.1:%d/sink' % self.server.server_port)
                self.send_header('Content-Length', '0')
                self.end_headers()
                return
            if label == 'rate':
                self.reply(429, {'error': {'type': 'usage_limit_reached', 'message': 'synthetic quota'}})
                return
            if label == 'denied' or label == 'recover' and generation == 0:
                self.reply(401, {'error': {'message': 'synthetic rejected access'}})
                return
            if label == 'nonsse':
                self.reply(200, {'unexpected': 'synthetic JSON instead of SSE'})
                return
            self.send_response(200)
            if label not in ('missingmime', 'missingempty', 'missinghtml', 'missingincomplete', 'standardmissing'):
                self.send_header('Content-Type', '' if label == 'emptymime' else ' Text/Event-Stream ; charset=utf-8 ' if label == 'mixedmime' else 'text/event-stream')
            self.send_header('Connection', 'close')
            self.end_headers()
            if label == 'missingempty':
                return
            if label == 'missinghtml':
                self.wfile.write(b'<!doctype html><html>synthetic non-SSE</html>')
                self.wfile.flush()
                return
            self.wfile.write(event({'type': 'response.created', 'response': {'id': 'fixture', 'status': 'in_progress', 'output': []}}).encode())
            self.wfile.flush()
            if label == 'missingincomplete':
                return
            if label == 'midstream':
                self.connection.shutdown(socket.SHUT_RDWR)
                return
            self.wfile.write(event({'type': 'response.completed', 'response': {'id': 'fixture', 'status': 'completed', 'output': [], 'usage': {'input_tokens': 1, 'output_tokens': 0}}}).encode())
            self.wfile.flush()
        except Exception as error:
            ERRORS.append(str(error))
            self.reply(400, {'error': 'fixture inference rejected'})


def rewrite(path, old, new, expected=1):
    source = path.read_text()
    check(source.count(old) == expected, 'source changed: %s expected %d rewrite matches' % (path.name, expected))
    path.write_text(source.replace(old, new))


PROGRAM = r'''
use std::{collections::BTreeMap, fs, path::{Path,PathBuf}, sync::{Arc,Mutex,OnceLock,atomic::{AtomicBool, Ordering}}, time::{Duration,Instant}};
use velune_application::{Application,Options,AuthenticationEdit,ProviderDraft};
use velune_gateway::{CredentialTarget,GatewayProtocol,ModelRoute,Runner,SchedulingState};
use serde_json::{Value,json};
#[derive(Default)] struct Sink(Mutex<Vec<velune_gateway::AnalyticsRecord>>);
impl velune_gateway::AnalyticsSink for Sink { fn record(&self,record:velune_gateway::AnalyticsRecord) {self.0.lock().unwrap().push(record);} }
static SINK:OnceLock<Arc<Sink>>=OnceLock::new();
fn load(home:&Path)->Value { serde_json::from_slice(&fs::read(home.join("generic-config.json")).unwrap()).unwrap() }
fn config(home:&Path)->velune_application::config::GatewayConfig { serde_json::from_value(load(home)["gateways"][0].clone()).unwrap() }
fn target(endpoint:&str)->CredentialTarget { CredentialTarget {protocol:GatewayProtocol::ResponsesV1,endpoint:endpoint.into(),execution_backend:velune_gateway::ExecutionBackend::Http,request_profile:velune_gateway::RequestProfile::CodexResponses} }
fn session_path(home:&Path,label:&str)->PathBuf {
 let value=load(home);
 let provider=value["gateways"][0]["providers"].as_array().unwrap().iter().find(|p|p["id"]==label).unwrap();
 let id=provider["authentication"]["material"]["credentialId"].as_str().unwrap();
 home.join("subscription-auth").join(format!("{id}.json"))
}
fn login(app:&mut Application,label:&str) {
 let mut progress=app.authentication_start("default".into(),label.into()).unwrap();
 let other=if label=="rate" {"good"} else {"rate"};
 assert!(app.authentication_start("default".into(),other.into()).is_err(),"second provider concurrent login accepted");
 let deadline=Instant::now()+Duration::from_secs(15);
 let url=loop {
  if let Some(url)=progress.events.iter().find_map(|e|e.notification.as_ref().and_then(|n| {assert_eq!(e.type_id,"notify","login event contract");assert_eq!(n.kind,"url","login URL kind");n.url.clone()})) {break url;}
  assert!(Instant::now()<deadline,"login URL missing");
  std::thread::sleep(Duration::from_millis(20));progress=app.authentication_poll().unwrap();
 };
 let mut url=url::Url::parse(&url).unwrap();
 let state=url.query_pairs().find(|(k,_)|k=="state").unwrap().1.into_owned();
 assert!(state.len()>=32,"state entropy");
 let redirect=url.query_pairs().find(|(k,_)|k=="redirect_uri").unwrap().1.into_owned();
 let client=reqwest::blocking::Client::builder().no_proxy().build().unwrap();
 let rejected=client.get(format!("{redirect}?state=wrong&code=wrong")).send().unwrap();
 assert_eq!(rejected.status().as_u16(),400,"invalid state accepted");
 url.query_pairs_mut().append_pair("fixture_account",label);
 assert!(client.get(url).send().unwrap().status().is_success(),"callback failed");
 loop {
  progress=app.authentication_poll().unwrap();
  if !progress.running {assert!(progress.events.iter().any(|e|e.ok==Some(true)),"login not persisted");break;}
  assert!(Instant::now()<deadline,"login timed out");std::thread::sleep(Duration::from_millis(20));
 }
}
async fn call(client:&reqwest::Client,runner:&Runner,extra:Value)->(u16,String,reqwest::header::HeaderMap) {
 let mut body=json!({"model":"velune/model/pool","input":[{"role":"user","content":"fixture"}],"stream":true,"store":false});
 for (key,value) in extra.as_object().unwrap(){body[key]=value.clone();}
 let response=client.post(format!("{}/responses",runner.endpoint())).bearer_auth(runner.token()).header("accept","*/*").json(&body).send().await.unwrap();
 let status=response.status().as_u16();let headers=response.headers().clone();(status,response.text().await.unwrap_or_default(),headers)
}
fn runner(home:&Path,options:&Options,labels:&[&str])->Runner {
 let full=config(home);
 let providers:Vec<_>=full.providers.iter().filter(|p|labels.contains(&p.id.as_str())).map(|p|velune_gateway::ProviderDefinition {execution_backend:velune_gateway::ExecutionBackend::Http,request_profile:match p.request_profile{velune_application::config::RequestProfile::Native=>velune_gateway::RequestProfile::Native,velune_application::config::RequestProfile::CodexResponses=>velune_gateway::RequestProfile::CodexResponses,velune_application::config::RequestProfile::KimiCoding=>velune_gateway::RequestProfile::KimiCoding,velune_application::config::RequestProfile::ArkCoding=>velune_gateway::RequestProfile::ArkCoding},
  id:p.id.clone(),name:p.name.clone(),protocol:GatewayProtocol::ResponsesV1,endpoint:p.endpoint.clone(),credential_ref:Some(p.id.clone()),quota_limits:vec![],
  models:p.models.iter().map(|m|velune_gateway::ProviderModel {record_key:m.record_key.clone(),provider_model_id:m.provider_model_id.clone(),nickname:m.nickname.clone(),icon:None,context_window:None,max_output_tokens:Some(256),reasoning_levels:None}).collect()
 }).collect();
 let members=labels.iter().map(|label|providers.iter().find(|p|p.id==*label).unwrap().models[0].record_key.clone()).collect();
 let gateway=velune_gateway::GatewayConfig{id:"default".into(),name:"fixture".into(),providers,routes:vec![ModelRoute {record_key:"pool".into(),name:"fixture".into(),members,strategy:velune_gateway::RoutingStrategy::Order,sticky:false,weights:BTreeMap::new()}]};
 Runner::start_with_analytics(gateway,velune_application::authentication_resolver::ProviderResolver::capture(&full,options),BTreeMap::new(),Some(SINK.get_or_init(||Arc::new(Sink::default())).clone()),SchedulingState::default()).unwrap()
}
fn main(){
 let args:Vec<String>=std::env::args().collect();let home=PathBuf::from(&args[1]);let endpoint=&args[2];
 fs::create_dir_all(home.join("resources")).unwrap();
 let options=Options {home_directory:home.clone(),resources_directory:home.join("resources")};
 let mut app=Application::open(options.clone()).unwrap();
 for label in ["rate","good","recover","denied","concurrent","midstream","transient","cancelled","redirect302","redirect307","nonsse","mixedmime","missingmime","missingempty","missinghtml","missingincomplete","emptymime","quota-invalid"] {
  let draft:ProviderDraft=serde_json::from_value(json!({"id":label,"name":label,"executionBackend":"http","requestProfile":"codexResponses","authenticationSource":"codexOAuth","quotaSource":"codexUsage","protocol":"responsesV1","endpoint":endpoint,"quotaLimits":[],"models":[{"recordKey":"","providerModelId":"fixture-model","nickname":"fixture","icon":null,"contextWindow":8192,"maxOutputTokens":256,"reasoningLevels":null}]})).unwrap();
  app.save_provider("default".into(),draft,AuthenticationEdit::Clear,None).unwrap();
 }
 let standard:ProviderDraft=serde_json::from_value(json!({"id":"standardmissing","name":"standardmissing","executionBackend":"http","requestProfile":"native","authenticationSource":"apiKey","quotaSource":"none","protocol":"responsesV1","endpoint":endpoint,"quotaLimits":[],"models":[{"recordKey":"","providerModelId":"fixture-model","nickname":"fixture","icon":null,"contextWindow":8192,"maxOutputTokens":256,"reasoningLevels":null}]})).unwrap();
 app.save_provider("default".into(),standard,AuthenticationEdit::SetApiKey{value:"fixture-standardmissing".into()},None).unwrap();
 for label in ["rate","good","recover","denied","concurrent","midstream","transient","cancelled","redirect302","redirect307","nonsse","mixedmime","missingmime","missingempty","missinghtml","missingincomplete","emptymime","quota-invalid"] {login(&mut app,label);}
 app.close().unwrap();drop(app);
 let mut app=Application::open(options.clone()).unwrap();
 for label in ["rate","good","recover","denied","concurrent","midstream","transient","cancelled","redirect302","redirect307","nonsse","mixedmime","missingmime","missingempty","missinghtml","missingincomplete","emptymime","quota-invalid"] {assert!(app.authentication_inspect("default".into(),label.into()).unwrap().configured,"reopen lost login");}
 let public=serde_json::to_string(&app.list().unwrap()).unwrap();
 let mut labels=std::collections::BTreeSet::new();
 for label in ["rate","good"] {
  let metadata=app.authentication_inspect("default".into(),label.into()).unwrap();
  labels.insert(metadata.account_label.unwrap());
  let session:Value=serde_json::from_slice(&fs::read(session_path(&home,label)).unwrap()).unwrap();
  for field in ["access_token","refresh_token","id_token","account_id"] {assert!(!public.contains(session[field].as_str().unwrap()),"private authentication exposed in public summary");}
 }
 assert_eq!(labels.len(),2,"account labels cannot distinguish subscriptions");
 // Public configuration and quota collection stay provider-owned and survive reopening.
 let saved=config(&home);let rate=saved.providers.iter().find(|p|p.id=="rate").unwrap();let good=saved.providers.iter().find(|p|p.id=="good").unwrap();
 let rate_key=rate.models[0].record_key.clone();let good_key=good.models[0].record_key.clone();
 app.save_provider("default".into(),ProviderDraft{execution_backend:rate.execution_backend.clone(),request_profile:rate.request_profile,authentication_source:velune_application::ProviderAuthenticationSource::CodexOAuth,quota_source:rate.quota_source,id:rate.id.clone(),name:rate.name.clone(),protocol:rate.protocol.clone(),endpoint:rate.endpoint.clone(),models:rate.models.iter().map(Into::into).collect(),quota_limits:vec![velune_application::config::QuotaLimit{window_id:"window:18000".into(),max_used_percent:75}]},AuthenticationEdit::Keep,None).unwrap();
 for strategy in [velune_application::config::RoutingStrategy::Smart,velune_application::config::RoutingStrategy::Order,velune_application::config::RoutingStrategy::Rotate,velune_application::config::RoutingStrategy::Weight]{
  let weights=if strategy==velune_application::config::RoutingStrategy::Weight{BTreeMap::from([(rate_key.clone(),1),(good_key.clone(),3)])}else{BTreeMap::new()};
  app.save_model_route("default".into(),velune_application::config::ModelRoute{record_key:String::new(),name:format!("Fixture {strategy:?}"),members:vec![rate_key.clone(),good_key.clone()],strategy,sticky:true,weights}).unwrap();
 }
 let sample=app.refresh_provider_quota("default".into(),"rate".into()).unwrap();assert!(sample.refreshing);
 let deadline=Instant::now()+Duration::from_secs(5);loop{let sample=app.provider_quota("default".into(),"rate".into()).unwrap();if !sample.refreshing{assert!(matches!(sample.freshness,velune_application::QuotaFreshness::Fresh));assert_eq!(sample.windows.len(),1);assert_eq!(sample.windows[0].id,"window:18000");assert_eq!(sample.windows[0].used_percent,37.5);break;}assert!(Instant::now()<deadline,"quota collection timeout");std::thread::sleep(Duration::from_millis(10));}
 app.refresh_provider_quota("default".into(),"quota-invalid".into()).unwrap();let deadline=Instant::now()+Duration::from_secs(5);loop{let sample=app.provider_quota("default".into(),"quota-invalid".into()).unwrap();if !sample.refreshing{assert!(matches!(sample.freshness,velune_application::QuotaFreshness::Unknown));assert!(sample.windows.is_empty()&&sample.problem.is_some(),"invalid supplier percentage became fresh");break;}assert!(Instant::now()<deadline);std::thread::sleep(Duration::from_millis(10));}
 app.close().unwrap();drop(app);let mut app=Application::open(options.clone()).unwrap();let snapshot=app.list().unwrap();assert_eq!(snapshot.gateways[0].routes.len(),4);assert!(snapshot.gateways[0].routes.iter().all(|r|r.sticky));assert_eq!(snapshot.gateways[0].providers.iter().find(|p|p.id=="rate").unwrap().quota_limits[0].max_used_percent,75);
 assert_eq!(load(&home)["schemaVersion"],10,"configuration schema boundary");
 let before=fs::read(home.join("generic-config.json")).unwrap();
 let old_session=session_path(&home,"good");
 let old_bytes=fs::read(&old_session).unwrap();
 let mut progress=app.authentication_start("default".into(),"good".into()).unwrap();
 let url=loop {if let Some(url)=progress.events.iter().find_map(|e|e.notification.as_ref().and_then(|n| {assert_eq!(e.type_id,"notify","login event contract");assert_eq!(n.kind,"url","login URL kind");n.url.clone()})) {break url;}std::thread::sleep(Duration::from_millis(20));progress=app.authentication_poll().unwrap();};
 let mut url=url::Url::parse(&url).unwrap();url.query_pairs_mut().append_pair("fixture_account","good");
 assert!(reqwest::blocking::Client::builder().no_proxy().build().unwrap().get(url).send().unwrap().status().is_success());
 fs::create_dir(home.join("generic-config.json.tmp")).unwrap();
 assert!(app.authentication_poll().is_err(),"injected persistence failure accepted");
 fs::remove_dir(home.join("generic-config.json.tmp")).unwrap();
 assert_eq!(fs::read(home.join("generic-config.json")).unwrap(),before,"failed relogin changed binding");
 assert_eq!(fs::read(&old_session).unwrap(),old_bytes,"failed relogin destroyed old session");
 app.close().unwrap();drop(app);
 for label in ["transient","cancelled"] {
  let path=session_path(&home,label);let mut value:Value=serde_json::from_slice(&fs::read(&path).unwrap()).unwrap();value["expires_at"]=json!(0);fs::write(path,serde_json::to_vec(&value).unwrap()).unwrap();
 }
 let concurrent=session_path(&home,"concurrent");
 let mut session:Value=serde_json::from_slice(&fs::read(&concurrent).unwrap()).unwrap();session["expires_at"]=json!(0);
 fs::write(&concurrent,serde_json::to_vec(&session).unwrap()).unwrap();
 let monitor_done=Arc::new(AtomicBool::new(false));let monitor_flag=monitor_done.clone();let monitor_path=concurrent.clone();
 let monitor=std::thread::spawn(move|| {let mut observations=0;while !monitor_flag.load(Ordering::SeqCst){let v:Value=serde_json::from_slice(&fs::read(&monitor_path).unwrap()).unwrap();assert!(v["access_token"].is_string()&&v["refresh_token"].is_string(),"partial credential write");observations+=1;}observations});
 let rt=tokio::runtime::Builder::new_current_thread().enable_all().build().unwrap();
 rt.block_on(async {
  let resolver=velune_application::authentication_resolver::ProviderResolver::capture(&config(&home),&options);
  let mut tasks=Vec::new();for _ in 0..12 {let r=resolver.clone();let t=target(endpoint);tasks.push(tokio::spawn(async move{r.resolve("concurrent".into(),t).await.unwrap()}));}
  let mut first=None;for task in tasks{let result=task.await.unwrap();if let Some(ref old)=first{assert!(old==result.token().unwrap(),"concurrent refresh yielded divergent access");}else{first=Some(result.token().unwrap().to_owned());}}
  assert!(resolver.resolve("good".into(),target("http://127.0.0.1:1/elsewhere")).await.is_err(),"credential escaped its target");
  let client=reqwest::Client::builder().no_proxy().redirect(reqwest::redirect::Policy::none()).build().unwrap();
  assert!(resolver.resolve("transient".into(),target(endpoint)).await.is_err(),"synthetic temporary refresh should fail");
  let transient:Value=serde_json::from_slice(&fs::read(session_path(&home,"transient")).unwrap()).unwrap();assert_eq!(transient["invalidated"],false,"temporary refresh invalidated session");
  assert!(resolver.resolve("transient".into(),target(endpoint)).await.is_ok(),"temporary refresh could not recover");
  let r=resolver.clone();let t=target(endpoint);let cancelled=tokio::spawn(async move {r.resolve("cancelled".into(),t).await});
  let deadline=Instant::now()+Duration::from_secs(5);
  loop {let status:Value=client.get(format!("{}/fixture/status",endpoint.trim_end_matches("/backend-api/codex"))).send().await.unwrap().json().await.unwrap();if status["refresh:cancelled"]==1 {break;}assert!(Instant::now()<deadline,"refresh not sent");tokio::time::sleep(Duration::from_millis(10)).await;}
  cancelled.abort();let _=cancelled.await;tokio::time::sleep(Duration::from_millis(850)).await;
  assert!(resolver.resolve("cancelled".into(),target(endpoint)).await.is_ok(),"cancelled caller lost durable refresh");
  for (label,status) in [("redirect302",302),("redirect307",307)] {let redirect=runner(&home,&options,&[label,"good"]);assert_eq!(call(&client,&redirect,json!({})).await.0,status,"redirect followed");drop(redirect);}
  let nonsse=runner(&home,&options,&["nonsse","good"]);let invalid=call(&client,&nonsse,json!({})).await;assert_eq!(invalid.0,502,"non-SSE HTTP 200 became false success");assert_eq!(invalid.2.get("content-type").unwrap(),"application/json","gateway error JSON content type");assert!(invalid.2.get("content-encoding").is_none(),"upstream entity encoding retained");assert!(serde_json::from_str::<Value>(&invalid.1).unwrap()["error"].is_object(),"gateway error envelope missing");drop(nonsse);
  let mime=runner(&home,&options,&["mixedmime","good"]);assert_eq!(call(&client,&mime,json!({})).await.0,200,"legal case/whitespace MIME rejected");drop(mime);
  let missing=runner(&home,&options,&["missingmime","good"]);let response=call(&client,&missing,json!({})).await;assert_eq!(response.0,200,"Codex missing MIME SSE failed");assert_eq!(response.2.get("content-type").unwrap(),"text/event-stream","missing Codex MIME not supplied downstream");drop(missing);
  for label in ["missingempty","missinghtml","missingincomplete"]{let missing=runner(&home,&options,&[label,"good"]);assert_eq!(call(&client,&missing,json!({})).await.0,200,"already committed headers changed");drop(missing);}
  for label in ["standardmissing","emptymime"]{let invalid=runner(&home,&options,&[label,"good"]);assert_eq!(call(&client,&invalid,json!({})).await.0,502,"missing standard MIME or present empty Codex MIME was accepted");drop(invalid);}
  let rate=runner(&home,&options,&["rate","good"]);assert_eq!(call(&client,&rate,json!({})).await.0,200,"429 failover");assert_eq!(call(&client,&rate,json!({})).await.0,200,"cooldown next request");drop(rate);
  let recover=runner(&home,&options,&["recover","good"]);assert_eq!(call(&client,&recover,json!({})).await.0,200,"401 refresh recovery");drop(recover);
  let denied=runner(&home,&options,&["denied","good"]);assert_eq!(call(&client,&denied,json!({})).await.0,200,"401 exhausted failover");drop(denied);
  assert!(resolver.resolve("denied".into(),target(endpoint)).await.is_err(),"rejected session not invalidated");
  let state=runner(&home,&options,&["good","rate"]);for extra in [json!({"previous_response_id":"foreign"}),json!({"conversation":"foreign"})]{assert_eq!(call(&client,&state,extra).await.0,400,"stateful aggregation accepted");}drop(state);
  let mid=runner(&home,&options,&["midstream","good"]);let response=call(&client,&mid,json!({})).await;assert_eq!(response.0,200,"stream response must preserve committed headers");drop(mid);
 });
 let records=SINK.get().unwrap().0.lock().unwrap();
 let recovered:Vec<_>=records.iter().filter(|r|r.provider_id=="recover").collect();
 assert_eq!(recovered.len(),1,"401 recovery must produce one attempt record");
 assert_eq!(recovered[0].outcome,velune_gateway::AnalyticsOutcome::Completed,"recovered stream analytics");
 assert!(records.iter().any(|r|r.provider_id=="nonsse"&&r.status==Some(200)&&r.outcome==velune_gateway::AnalyticsOutcome::Failed),"non-SSE failure analytics");
 assert!(records.iter().any(|r|r.provider_id=="mixedmime"&&r.outcome==velune_gateway::AnalyticsOutcome::Completed),"MIME-normalized success analytics");
 assert!(records.iter().any(|r|r.provider_id=="missingmime"&&r.outcome==velune_gateway::AnalyticsOutcome::Completed),"Codex missing MIME SSE not completed");
 for label in ["missingempty","missinghtml","missingincomplete","standardmissing","emptymime"]{assert!(records.iter().any(|r|r.provider_id==label&&r.outcome==velune_gateway::AnalyticsOutcome::Failed),"missing/invalid MIME produced false completion");}
 drop(records);
 let mut app=Application::open(options.clone()).unwrap();
 let control=reqwest::blocking::Client::builder().no_proxy().build().unwrap();let quota_base=endpoint.trim_end_matches("/backend-api/codex");
 control.get(format!("{quota_base}/quota-control?mode=hold")).send().unwrap();app.refresh_provider_quota("default".into(),"rate".into()).unwrap();let deadline=Instant::now()+Duration::from_secs(5);loop{let status:Value=control.get(format!("{quota_base}/quota-control")).send().unwrap().json().unwrap();if status["entered"]==true{break;}assert!(Instant::now()<deadline);std::thread::sleep(Duration::from_millis(10));}
 let old=config(&home).providers.into_iter().find(|p|p.id=="rate").unwrap();app.save_provider("default".into(),ProviderDraft{execution_backend:old.execution_backend.clone(),request_profile:old.request_profile,authentication_source:velune_application::ProviderAuthenticationSource::CodexOAuth,quota_source:old.quota_source,id:old.id,name:old.name,protocol:old.protocol,endpoint:old.endpoint,models:old.models.iter().map(Into::into).collect(),quota_limits:old.quota_limits},AuthenticationEdit::Clear,None).unwrap();control.get(format!("{quota_base}/quota-control?mode=release")).send().unwrap();std::thread::sleep(Duration::from_millis(150));let replaced=app.provider_quota("default".into(),"rate".into()).unwrap();assert!(matches!(replaced.freshness,velune_application::QuotaFreshness::Unknown)&&replaced.windows.is_empty(),"old-account quota returned after authentication replacement");
 let denied=app.authentication_inspect("default".into(),"denied".into()).unwrap();assert!(!denied.configured&&denied.account_label.is_some(),"invalidated account public state");
 assert!(app.authentication_inspect("default".into(),"transient".into()).unwrap().configured,"temporary failure public state");app.close().unwrap();drop(app);
 monitor_done.store(true,Ordering::SeqCst);assert!(monitor.join().unwrap()>0,"no atomic write observations");
 #[cfg(unix)]{use std::os::unix::fs::PermissionsExt;assert_eq!(fs::metadata(&concurrent).unwrap().permissions().mode()&0o777,0o600,"credential permissions");}
 {
  use velune_bindings::*;
  let binding_home=home.join("binding-config");let bound=VeluneApplication::open(BindingOptions{home_directory:binding_home.to_string_lossy().into_owned(),resources_directory:options.resources_directory.to_string_lossy().into_owned()}).unwrap();
  let mut keys=Vec::new();for id in ["binding-a","binding-b"]{let draft:BindingProviderDraft=serde_json::from_value(json!({"id":id,"name":id,"executionBackend":"http","requestProfile":"native","authenticationSource":"apiKey","quotaSource":"none","protocol":"responsesV1","endpoint":endpoint,"quotaLimits":[{"windowId":"window:18000","maxUsedPercent":75}],"models":[{"recordKey":"","providerModelId":"fixture-model","nickname":"fixture","icon":null,"contextWindow":null,"maxOutputTokens":null,"reasoningLevels":null}]})).unwrap();let update=bound.save_provider("default".into(),draft,BindingAuthenticationEdit::SetApiKey{value:"fixture-key".into()},None).unwrap();keys.push(update.gateways[0].providers.iter().find(|p|p.id==id).unwrap().models[0].record_key.clone());}
  bound.save_model_route("default".into(),BindingModelRoute{record_key:String::new(),name:"Weighted".into(),members:keys.clone(),strategy:BindingRoutingStrategy::Weight,sticky:true,weights:std::collections::HashMap::from([(keys[0].clone(),1),(keys[1].clone(),3)])}).unwrap();assert!(matches!(bound.provider_quota("default".into(),"binding-a".into()).unwrap().freshness,BindingQuotaFreshness::Unsupported));bound.shutdown().unwrap();drop(bound);
  let bound=VeluneApplication::open(BindingOptions{home_directory:binding_home.to_string_lossy().into_owned(),resources_directory:options.resources_directory.to_string_lossy().into_owned()}).unwrap();let snapshot=bound.list().unwrap();assert_eq!(snapshot.gateways[0].routes[0].weights[&keys[1]],3);assert!(snapshot.gateways[0].routes[0].sticky);bound.shutdown().unwrap();
 }
 println!("{}",json!({"cases":["PKCE 回调与错误 state 拒绝","独立订阅登录、持久化与重开","并发续期与原子持久化","凭据目标约束","429 切换与冷却","401 最多恢复一次","401 失效与切换","聚合拒绝账户状态引用","流式提交边界","暂时续期失败后恢复","调用方取消后完成凭据轮换","重新登录写失败保留旧认证","交互登录互斥","重定向目标隔离","公开认证描述脱敏与账户区分","非 SSE 200 映射为 502 且不切换","SSE MIME 大小写与空白规范化","Codex 缺 MIME 有效流完成并补下游 MIME","缺 MIME 空流 HTML 无终态均失败且不切换","Standard 缺 MIME 与显式空 MIME 保持拒绝","四策略 cap 配置保存重开","公开额度采集与非法窗口拒绝","账户替换拒绝旧额度回写","bindings 类型门面保存重开"]}));
}
'''


def main():
    with tempfile.TemporaryDirectory(prefix='velune-subscriptions-') as directory:
        temporary = Path(directory)
        copy = temporary / 'source'
        copy.mkdir()
        shutil.copytree(ROOT / 'packages', copy / 'packages')
        shutil.copytree(ROOT / 'third-party', copy / 'third-party')
        shutil.copytree(ROOT / 'app/host', copy / 'app/host')
        for name in ('Cargo.toml', 'Cargo.lock'):
            shutil.copy2(ROOT / name, copy / name)
        server = ThreadingHTTPServer(('127.0.0.1', 0), Fixture)
        threading.Thread(target=server.serve_forever, daemon=True).start()
        base = 'http://127.0.0.1:%d' % server.server_port
        endpoint = base + '/backend-api/codex'
        try:
            rewrite(copy / 'packages/application/src/subscription_auth.rs', 'https://auth.openai.com', base)
            rewrite(copy / 'packages/application/src/subscription_auth.rs', 'https://chatgpt.com/backend-api/wham/usage', base + '/usage')
            rewrite(copy / 'packages/application-api/src/authentication.rs', 'https://chatgpt.com/backend-api/codex', endpoint)
            # Keep every target check, substituting only this synthetic target's identity.
            rewrite(copy / 'packages/ai-provider/src/config.rs',
                    'config.endpoint.transport() != Transport::Https\n                || config.endpoint.host() != "chatgpt.com"\n                || config.endpoint.port().get() != 443',
                    'config.endpoint.transport() != Transport::Http\n                || config.endpoint.host() != "127.0.0.1"\n                || config.endpoint.port().get() != %d' % server.server_port)
            config_path = copy / 'packages/gateway/src/config.rs'
            config_source = config_path.read_text()
            start = config_source.index('if provider.request_profile == RequestProfile::CodexResponses {')
            end = config_source.index('if provider.request_profile == RequestProfile::ArkCoding {', start)
            block = config_source[start:end]
            for old, new in [('endpoint.scheme() != "https"', 'endpoint.scheme() != "http"'), ('endpoint.host_str() != Some("chatgpt.com")', 'endpoint.host_str() != Some("127.0.0.1")'), ('endpoint.port_or_known_default() != Some(443)', 'endpoint.port_or_known_default() != Some(%d)' % server.server_port)]:
                check(block.count(old) == 1, 'Codex target rewrite count changed')
                block = block.replace(old, new)
            config_path.write_text(config_source[:start] + block + config_source[end:])
            rewrite(copy / 'packages/application/src/lib.rs', 'mod authentication_resolver;', 'pub mod authentication_resolver;')
            rewrite(copy / 'packages/application/src/authentication_resolver.rs', 'pub(crate) struct ProviderResolver', 'pub struct ProviderResolver')
            rewrite(copy / 'packages/application/src/authentication_resolver.rs', 'pub(crate) fn capture(', 'pub fn capture(')
            harness = temporary / 'harness'
            (harness / 'src').mkdir(parents=True)
            (harness / 'src/main.rs').write_text(PROGRAM)
            dependencies = '\n'.join('%s={path="%s"}' % (name, copy / 'packages' / folder) for name, folder in [('velune-application', 'application'), ('velune-gateway', 'gateway'), ('velune-bindings', 'bindings')])
            dependencies = dependencies.replace('velune-bindings={path="', 'velune-bindings={default-features=false,features=["local-runtime"],path="')
            (harness / 'Cargo.toml').write_text('[package]\nname="manual-codex-subscriptions"\nversion="0.0.0"\nedition="2024"\n[workspace]\n[dependencies]\n' + dependencies + '\nreqwest={version="0.13",default-features=false,features=["json","blocking"]}\ntokio={version="1",features=["macros","rt","sync"]}\nserde_json="1"\nurl="2"\n')
            environment = dict(os.environ)
            # An accidentally unrewritten remote URL must fail through a dead proxy.
            environment.update(HTTP_PROXY='http://127.0.0.1:1', HTTPS_PROXY='http://127.0.0.1:1', ALL_PROXY='http://127.0.0.1:1',
                               http_proxy='http://127.0.0.1:1', https_proxy='http://127.0.0.1:1', all_proxy='http://127.0.0.1:1',
                               NO_PROXY='127.0.0.1,localhost', no_proxy='127.0.0.1,localhost')
            target = ROOT / 'target/manual-subscriptions'
            build = subprocess.run(['cargo', 'build', '--quiet', '--offline', '--manifest-path', str(harness / 'Cargo.toml'), '--target-dir', str(target)], cwd=ROOT, env=environment, capture_output=True, text=True, timeout=300)
            check(build.returncode == 0, 'Rust harness build failed:\n' + build.stderr)
            process_home = temporary / 'process-home'
            process_home.mkdir()
            environment.update(HOME=str(process_home), VELUNE_HOME=str(temporary / 'home'))
            result = subprocess.run([str(target / 'debug/manual-codex-subscriptions'), str(temporary / 'home'), endpoint], cwd=temporary, env=environment, capture_output=True, text=True, timeout=60)
            if result.returncode:
                raise RuntimeError('Rust harness failed (fixture counts %s; errors %s):\n%s' % (COUNTS, ERRORS, result.stderr))
            report = json.loads(result.stdout.strip().splitlines()[-1])
            check(not ERRORS, 'fixture contract failures: %s' % ERRORS)
            check(COUNTS.get('refresh:concurrent') == 1 and not COUNTS.get('refresh:reused'), 'concurrent refresh rotation count')
            check(COUNTS.get('refresh:recover') == 1 and COUNTS.get('request:recover') == 2, '401 recovery upper bound')
            check(COUNTS.get('refresh:denied') == 1 and COUNTS.get('request:denied') == 2, '401 refusal upper bound')
            check(COUNTS.get('request:rate') == 1, '429 cooldown')
            check(COUNTS.get('request:good') == 3, 'stateful or committed stream unexpectedly reached fallback')
            check(COUNTS.get('request:midstream') == 1, 'midstream fixture unused')
            check(all(COUNTS.get('login:' + label) == 1 for label in ('rate','recover','denied','concurrent','midstream','transient','cancelled','redirect302','redirect307','nonsse','mixedmime','missingmime','missingempty','missinghtml','missingincomplete','emptymime','quota-invalid')), 'independent login count')
            check(COUNTS.get('login:good') == 2, 'failed relogin fixture not exercised')
            check(COUNTS.get('refresh:transient') == 2 and COUNTS.get('refresh:cancelled') == 1, 'temporary refresh and cancellation rotation')
            check(COUNTS.get('request:nonsse') == 1 and COUNTS.get('request:mixedmime') == 1, 'MIME boundary fixtures not exercised')
            check(all(COUNTS.get('request:' + label) == 1 for label in ('missingmime','missingempty','missinghtml','missingincomplete','standardmissing','emptymime')), 'missing MIME fixtures not exercised')
            check(not COUNTS.get('redirect:sink'), 'redirect leaked second-hop request')
            print('PASS', json.dumps(report, ensure_ascii=False))
            print('合成隔离证据；未验证真实 OAuth/TLS、真实订阅与跨账户历史兼容性。')
        finally:
            server.shutdown()
            server.server_close()


if __name__ == '__main__':
    main()
