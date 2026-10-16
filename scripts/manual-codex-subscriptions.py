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


def check(ok, message):
    if not ok:
        raise RuntimeError(message)


def b64(value):
    return base64.urlsafe_b64encode(value).decode().rstrip('=')


def jwt(label, generation):
    claims = {'exp': int(time.time()) + 3600, 'fixture_label': label,
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
        query = parse_qs(urlparse(self.path).query)
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
            claims = json.loads(base64.urlsafe_b64decode(token.split('.')[1] + '=='))
            label, generation = claims['fixture_label'], claims['fixture_generation']
            count('request:' + label)
            check(self.headers.get('ChatGPT-Account-Id') == 'account-' + label, 'account header')
            check(self.headers.get('originator') == 'codex_cli_rs', 'inference originator')
            check(self.headers.get('version') == '0.162.0', 'Codex client version')
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
            self.send_response(200)
            self.send_header('Content-Type', 'text/event-stream')
            self.send_header('Connection', 'close')
            self.end_headers()
            self.wfile.write(event({'type': 'response.created', 'response': {'id': 'fixture', 'status': 'in_progress', 'output': []}}).encode())
            self.wfile.flush()
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
use velune_gateway::{CredentialTarget,GatewayProtocol,ModelRoute,Runner,RouteHealth};
use serde_json::{Value,json};
#[derive(Default)] struct Sink(Mutex<Vec<velune_gateway::AnalyticsRecord>>);
impl velune_gateway::AnalyticsSink for Sink { fn record(&self,record:velune_gateway::AnalyticsRecord) {self.0.lock().unwrap().push(record);} }
static SINK:OnceLock<Arc<Sink>>=OnceLock::new();
fn load(home:&Path)->Value { serde_json::from_slice(&fs::read(home.join("generic-config.json")).unwrap()).unwrap() }
fn config(home:&Path)->velune_application::config::GatewayConfig { serde_json::from_value(load(home)["gateways"][0].clone()).unwrap() }
fn target(endpoint:&str)->CredentialTarget { CredentialTarget {protocol:GatewayProtocol::ResponsesV1,endpoint:endpoint.into()} }
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
async fn call(client:&reqwest::Client,runner:&Runner,extra:Value)->(u16,String) {
 let mut body=json!({"model":"velune/model/pool","input":[{"role":"user","content":"fixture"}],"stream":true,"store":false});
 for (key,value) in extra.as_object().unwrap(){body[key]=value.clone();}
 let response=client.post(format!("{}/responses",runner.endpoint())).bearer_auth(runner.token()).json(&body).send().await.unwrap();
 let status=response.status().as_u16();(status,response.text().await.unwrap_or_default())
}
fn runner(home:&Path,options:&Options,labels:&[&str])->Runner {
 let full=config(home);
 let providers:Vec<_>=full.providers.iter().filter(|p|labels.contains(&p.id.as_str())).map(|p|velune_gateway::ProviderDefinition {
  id:p.id.clone(),name:p.name.clone(),protocol:GatewayProtocol::ResponsesV1,endpoint:p.endpoint.clone(),credential_ref:Some(p.id.clone()),
  models:p.models.iter().map(|m|velune_gateway::ProviderModel {record_key:m.record_key.clone(),provider_model_id:m.provider_model_id.clone(),nickname:m.nickname.clone(),icon:None,context_window:None,max_output_tokens:Some(256),reasoning_levels:None}).collect()
 }).collect();
 let members=labels.iter().map(|label|providers.iter().find(|p|p.id==*label).unwrap().models[0].record_key.clone()).collect();
 let gateway=velune_gateway::GatewayConfig{id:"default".into(),name:"fixture".into(),providers,routes:vec![ModelRoute {record_key:"pool".into(),name:"fixture".into(),members}],failover:velune_gateway::FailoverPolicy{mode:velune_gateway::FailoverMode::Disabled}};
 Runner::start_with_analytics(gateway,velune_application::authentication_resolver::ProviderResolver::capture(&full,options),BTreeMap::new(),Some(SINK.get_or_init(||Arc::new(Sink::default())).clone()),RouteHealth::default()).unwrap()
}
fn main(){
 let args:Vec<String>=std::env::args().collect();let home=PathBuf::from(&args[1]);let endpoint=&args[2];
 fs::create_dir_all(home.join("resources")).unwrap();
 let options=Options {home_directory:home.clone(),resources_directory:home.join("resources")};
 let mut app=Application::open(options.clone()).unwrap();
 for label in ["rate","good","recover","denied","concurrent","midstream","transient","cancelled","redirect302","redirect307"] {
  let draft:ProviderDraft=serde_json::from_value(json!({"id":label,"name":label,"protocol":"responsesV1","endpoint":endpoint,"models":[{"recordKey":"","providerModelId":"fixture-model","nickname":"fixture","icon":null,"contextWindow":8192,"maxOutputTokens":256,"reasoningLevels":null}]})).unwrap();
  app.save_provider("default".into(),draft,AuthenticationEdit::SetCodexSubscription).unwrap();
 }
 for label in ["rate","good","recover","denied","concurrent","midstream","transient","cancelled","redirect302","redirect307"] {login(&mut app,label);}
 app.close().unwrap();drop(app);
 let mut app=Application::open(options.clone()).unwrap();
 for label in ["rate","good","recover","denied","concurrent","midstream","transient","cancelled","redirect302","redirect307"] {assert!(app.authentication_inspect("default".into(),label.into()).unwrap().configured,"reopen lost login");}
 let public=serde_json::to_string(&app.list().unwrap()).unwrap();
 let mut labels=std::collections::BTreeSet::new();
 for label in ["rate","good"] {
  let metadata=app.authentication_inspect("default".into(),label.into()).unwrap();
  labels.insert(metadata.account_label.unwrap());
  let session:Value=serde_json::from_slice(&fs::read(session_path(&home,label)).unwrap()).unwrap();
  for field in ["access_token","refresh_token","id_token","account_id"] {assert!(!public.contains(session[field].as_str().unwrap()),"private authentication exposed in public summary");}
 }
 assert_eq!(labels.len(),2,"account labels cannot distinguish subscriptions");
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
  let mut first=None;for task in tasks{let result=task.await.unwrap();if let Some(ref old)=first{assert!(old==&result.token,"concurrent refresh yielded divergent access");}else{first=Some(result.token);}}
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
 drop(records);
 let mut app=Application::open(options.clone()).unwrap();
 let denied=app.authentication_inspect("default".into(),"denied".into()).unwrap();assert!(!denied.configured&&denied.account_label.is_some(),"invalidated account public state");
 assert!(app.authentication_inspect("default".into(),"transient".into()).unwrap().configured,"temporary failure public state");app.close().unwrap();drop(app);
 monitor_done.store(true,Ordering::SeqCst);assert!(monitor.join().unwrap()>0,"no atomic write observations");
 #[cfg(unix)]{use std::os::unix::fs::PermissionsExt;assert_eq!(fs::metadata(&concurrent).unwrap().permissions().mode()&0o777,0o600,"credential permissions");}
 println!("{}",json!({"cases":["PKCE 回调与错误 state 拒绝","独立订阅登录、持久化与重开","并发续期与原子持久化","凭据目标约束","429 切换与冷却","401 最多恢复一次","401 失效与切换","聚合拒绝账户状态引用","流式提交边界","暂时续期失败后恢复","调用方取消后完成凭据轮换","重新登录写失败保留旧认证","交互登录互斥","重定向目标隔离","公开认证描述脱敏与账户区分"]}));
}
'''


def main():
    with tempfile.TemporaryDirectory(prefix='velune-subscriptions-') as directory:
        temporary = Path(directory)
        copy = temporary / 'source'
        copy.mkdir()
        shutil.copytree(ROOT / 'packages', copy / 'packages')
        shutil.copytree(ROOT / 'app/host', copy / 'app/host')
        for name in ('Cargo.toml', 'Cargo.lock'):
            shutil.copy2(ROOT / name, copy / name)
        server = ThreadingHTTPServer(('127.0.0.1', 0), Fixture)
        threading.Thread(target=server.serve_forever, daemon=True).start()
        base = 'http://127.0.0.1:%d' % server.server_port
        endpoint = base + '/backend-api/codex'
        try:
            rewrite(copy / 'packages/application/src/subscription_auth.rs', 'https://auth.openai.com', base)
            rewrite(copy / 'packages/application/src/provider_authentication.rs', 'https://chatgpt.com/backend-api/codex', endpoint)
            # Keep every target check, substituting only this synthetic target's identity.
            rewrite(copy / 'packages/application/src/subscription_auth.rs', 'https://chatgpt.com/backend-api/codex', endpoint)
            rewrite(copy / 'packages/ai-provider/src/responses.rs',
                    'protocol.endpoint.transport() != Transport::Https\n                || protocol.endpoint.host() != "chatgpt.com"\n                || protocol.endpoint.port().get() != 443',
                    'protocol.endpoint.transport() != Transport::Http\n                || protocol.endpoint.host() != "127.0.0.1"\n                || protocol.endpoint.port().get() != %d' % server.server_port)
            rewrite(copy / 'packages/application/src/lib.rs', 'mod authentication_resolver;', 'pub mod authentication_resolver;')
            rewrite(copy / 'packages/application/src/authentication_resolver.rs', 'pub(crate) struct ProviderResolver', 'pub struct ProviderResolver')
            rewrite(copy / 'packages/application/src/authentication_resolver.rs', 'pub(crate) fn capture(', 'pub fn capture(')
            harness = temporary / 'harness'
            (harness / 'src').mkdir(parents=True)
            (harness / 'src/main.rs').write_text(PROGRAM)
            dependencies = '\n'.join('%s={path="%s"}' % (name, copy / 'packages' / folder) for name, folder in [('velune-application', 'application'), ('velune-gateway', 'gateway')])
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
            check(all(COUNTS.get('login:' + label) == 1 for label in ('rate','recover','denied','concurrent','midstream','transient','cancelled','redirect302','redirect307')), 'independent login count')
            check(COUNTS.get('login:good') == 2, 'failed relogin fixture not exercised')
            check(COUNTS.get('refresh:transient') == 2 and COUNTS.get('refresh:cancelled') == 1, 'temporary refresh and cancellation rotation')
            check(not COUNTS.get('redirect:sink'), 'redirect leaked second-hop request')
            print('PASS', json.dumps(report, ensure_ascii=False))
            print('合成隔离证据；未验证真实 OAuth/TLS、真实订阅与跨账户历史兼容性。')
        finally:
            server.shutdown()
            server.server_close()


if __name__ == '__main__':
    main()
