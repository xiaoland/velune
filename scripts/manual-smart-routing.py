#!/usr/bin/env python3
"""手动调度验收：只运行临时 Rust runner 和回环合成上游，不接入 CI。

检查实际 HTTP 选择、比例分配、额度门槛、共享冷却、半开探测及显式会话黏性。
QuotaSource 返回合成供应商观察，不把本机 token 计数当真实额度。
不读取真实 HOME、认证或调用供应商；不会构建或安装 Mac app。
"""
import json
import os
from pathlib import Path
import subprocess
import tempfile
import threading
from http.server import BaseHTTPRequestHandler, ThreadingHTTPServer

ROOT = Path(__file__).resolve().parents[1]
LOCK = threading.Lock()
MODES = {}
GATES = {}
HITS = {}


def event(value):
    return 'event: %s\ndata: %s\n\n' % (value['type'], json.dumps(value))


class Fixture(BaseHTTPRequestHandler):
    protocol_version = 'HTTP/1.1'
    def log_message(self, *_):
        pass
    def reply(self, status, value, retry=1):
        data = json.dumps(value).encode()
        self.send_response(status)
        self.send_header('Content-Type', 'application/json')
        self.send_header('Content-Length', str(len(data)))
        if status == 429:
            self.send_header('Retry-After', str(retry))
        self.end_headers()
        self.wfile.write(data)
    def do_GET(self):
        with LOCK:
            snapshot = dict(HITS)
        self.reply(200, snapshot)
    def do_POST(self):
        data = json.loads(self.rfile.read(int(self.headers.get('Content-Length', '0'))))
        if self.path == '/control':
            identifier = data['id']
            with LOCK:
                if data.get('release'):
                    GATES.setdefault(identifier, threading.Event()).set()
                else:
                    MODES[identifier] = data['mode']
                    if data['mode'] in ('hold', 'stream_hold'):
                        GATES[identifier] = threading.Event()
            self.reply(200, {})
            return
        identifier = self.headers.get('Authorization', '').removeprefix('Bearer ')
        with LOCK:
            HITS[identifier] = HITS.get(identifier, 0) + 1
            mode = MODES.get(identifier, 'ok')
            gate = GATES.get(identifier)
        if mode in ('rate', 'longrate'):
            self.reply(429, {'error': {'type': 'usage_limit_reached'}}, 60 if mode == 'longrate' else 1)
            return
        if mode == 'server':
            self.reply(503, {'error': {'type': 'server_error'}})
            return
        if mode == 'hold':
            gate.wait(8)
        try:
            self.send_response(200)
            self.send_header('Content-Type', 'text/event-stream')
            self.send_header('Connection', 'close')
            self.end_headers()
            self.wfile.write(event({'type': 'response.created', 'response': {'status': 'in_progress', 'output': []}}).encode())
            self.wfile.flush()
            if mode == 'stream_hold':
                gate.wait(8)
            if mode in ('incomplete', 'failed'):
                self.wfile.write(event({'type': 'response.' + mode, 'response': {'status': mode, 'output': []}}).encode())
                self.wfile.flush()
                return
            text = 'SELECTED:' + identifier
            self.wfile.write(event({'type': 'response.output_text.delta', 'delta': text}).encode())
            self.wfile.write(event({'type': 'response.completed', 'response': {'status': 'completed', 'output': [{'type': 'message', 'role': 'assistant', 'content': [{'type': 'output_text', 'text': text}]}], 'usage': {'input_tokens': 1, 'output_tokens': 1, 'total_tokens': 2}}}).encode())
            self.wfile.flush()
        except (BrokenPipeError, ConnectionResetError):
            pass


PROGRAM = r'''
use std::{collections::BTreeMap,sync::{Arc,Mutex},time::{Duration,SystemTime,UNIX_EPOCH}};
use serde_json::{Value,json};
use velune_gateway::*;
fn now()->i64{SystemTime::now().duration_since(UNIX_EPOCH).unwrap().as_millis() as i64}
struct Auth;
impl CredentialResolver for Auth{fn resolve(&self,id:String,_:CredentialTarget)->velune_ai::OperationFuture<Result<ResolvedCredential,CredentialResolutionError>>{Box::pin(async move{Ok(ResolvedCredential{token:id,explicit_output_cap:None,subscription:false,profile:CredentialProfile::Standard})})}}
#[derive(Default)]struct Quotas(Mutex<BTreeMap<String,QuotaSnapshot>>);
impl QuotaSource for Quotas{fn snapshot(&self,id:&str)->QuotaSnapshot{self.0.lock().unwrap().get(id).cloned().unwrap_or(QuotaSnapshot::Unknown)}}
impl Quotas{fn set(&self,id:&str,value:QuotaSnapshot){self.0.lock().unwrap().insert(id.into(),value);}}
fn window(id:&str,used:f64)->QuotaWindow{let t=now();QuotaWindow{id:id.into(),model_ids:None,used_percent:used,reset_at_ms:Some(t+60_000),observed_at_ms:t,expires_at_ms:t+60_000}}
fn known(used:f64)->QuotaSnapshot{QuotaSnapshot::Known{available:Some(true),windows:vec![window("short",used)]}}
fn cap(value:u8)->Vec<QuotaLimit>{vec![QuotaLimit{window_id:"short".into(),max_used_percent:value}]}
fn provider(id:&str,endpoint:&str,limits:Vec<QuotaLimit>)->ProviderDefinition{ProviderDefinition{id:id.into(),name:id.into(),protocol:GatewayProtocol::ResponsesV1,endpoint:format!("{endpoint}/v1"),credential_ref:Some(id.into()),quota_limits:limits,models:vec![ProviderModel{record_key:id.into(),provider_model_id:"fixture-model".into(),nickname:id.into(),icon:None,context_window:None,max_output_tokens:None,reasoning_levels:None}]}}
fn route(key:&str,members:&[&str],strategy:RoutingStrategy,sticky:bool)->ModelRoute{ModelRoute{record_key:key.into(),name:key.into(),members:members.iter().map(|id|id.to_string()).collect(),strategy,sticky,weights:if strategy==RoutingStrategy::Weight{BTreeMap::from([(members[0].into(),1),(members[1].into(),3)])}else{BTreeMap::new()}}}
fn config(endpoint:&str,ids:&[(&str,Vec<QuotaLimit>)],routes:Vec<ModelRoute>)->GatewayConfig{GatewayConfig{id:"fixture".into(),name:"fixture".into(),providers:ids.iter().map(|(id,limits)|provider(id,endpoint,limits.clone())).collect(),routes}}
fn runner(config:GatewayConfig,state:SchedulingState,quotas:Arc<Quotas>,session:Option<(&str,&str)>)->Runner{Runner::start_with_scheduling(config,Arc::new(Auth),BTreeMap::new(),None,state,quotas,session.map(|(namespace,id)|SessionIdentity{namespace:namespace.into(),id:id.into()})).unwrap()}
async fn response(client:&reqwest::Client,endpoint:&str,token:&str,key:&str)->reqwest::Response{client.post(format!("{endpoint}/responses")).bearer_auth(token).json(&json!({"model":format!("velune/model/{key}"),"input":"synthetic","stream":true,"store":false})).send().await.unwrap()}
async fn call(client:&reqwest::Client,runner:&Runner,key:&str)->(u16,String,Option<u64>){let r=response(client,runner.endpoint(),runner.token(),key).await;let status=r.status().as_u16();let retry=r.headers().get("retry-after").and_then(|v|v.to_str().ok()).and_then(|s|s.parse().ok());(status,r.text().await.unwrap_or_default(),retry)}
fn selected(reply:&(u16,String,Option<u64>),id:&str){assert_eq!(reply.0,200,"request was not admitted");assert!(reply.1.contains(&format!("SELECTED:{id}")),"wrong observable provider selection");assert!(reply.1.contains("response.completed"),"missing explicit completion");}
async fn control(client:&reqwest::Client,up:&str,id:&str,mode:&str){client.post(format!("{up}/control")).json(&json!({"id":id,"mode":mode})).send().await.unwrap();}
async fn release(client:&reqwest::Client,up:&str,id:&str){client.post(format!("{up}/control")).json(&json!({"id":id,"release":true})).send().await.unwrap();}
async fn hits(client:&reqwest::Client,up:&str,id:&str)->u64{client.get(format!("{up}/stats")).send().await.unwrap().json::<Value>().await.unwrap()[id].as_u64().unwrap_or(0)}
async fn wait_hits(client:&reqwest::Client,up:&str,id:&str,count:u64){let deadline=std::time::Instant::now()+Duration::from_secs(3);while hits(client,up,id).await<count{assert!(std::time::Instant::now()<deadline,"upstream was not reached");tokio::time::sleep(Duration::from_millis(10)).await;}}
#[tokio::main(flavor="current_thread")]
async fn main(){let up=std::env::args().nth(1).unwrap();let client=reqwest::Client::builder().no_proxy().timeout(Duration::from_secs(10)).build().unwrap();let mut passed=Vec::new();
 // Different caps distinguish relative headroom from absolute remaining percentage.
 let q=Arc::new(Quotas::default());q.set("smart-a",known(25.));q.set("smart-b",known(60.));
 let smart=config(&up,&[("smart-a",cap(50)),("smart-b",cap(100))],vec![route("smart",&["smart-b","smart-a"],RoutingStrategy::Smart,false)]);
 let r=runner(smart,SchedulingState::default(),q.clone(),None);selected(&call(&client,&r,"smart").await,"smart-a");drop(r);passed.push("smart uses relative cap headroom");
 let order=config(&up,&[("order-a",vec![]),("order-b",vec![])],vec![route("order",&["order-b","order-a"],RoutingStrategy::Order,false)]);
 let r=runner(order,SchedulingState::default(),q.clone(),None);selected(&call(&client,&r,"order").await,"order-b");drop(r);passed.push("order respects explicit priority");
 for strategy in [RoutingStrategy::Rotate,RoutingStrategy::Weight]{let ids=[("dist-a",vec![]),("dist-b",vec![])];let conf=config(&up,&ids,vec![route("distribution",&["dist-a","dist-b"],strategy,false)]);let state=SchedulingState::default();let mut counts=[0u32;2];for turn in 0..128{let r=runner(conf.clone(),state.clone(),q.clone(),None);let reply=call(&client,&r,"distribution").await;assert_eq!(reply.0,200);if reply.1.contains("SELECTED:dist-a"){counts[0]+=1;}else{assert!(reply.1.contains("SELECTED:dist-b"));counts[1]+=1;}drop(r);if turn==127{if strategy==RoutingStrategy::Rotate{assert!(counts[0].abs_diff(counts[1])<=1,"rotation starved a member");}else{assert!((13..=45).contains(&counts[0])&&counts[1]>counts[0],"weight proportions lost");}}}passed.push(if strategy==RoutingStrategy::Rotate{"rotate distributes across recreated runners"}else{"weight preserves configured proportions"});}
 // Caps are provider-wide and never bypassed by a different route or direct model.
 let q=Arc::new(Quotas::default());let mut conf=config(&up,&[("capped",cap(50)),("fallback",vec![])],vec![route("cap-route",&["capped","fallback"],RoutingStrategy::Order,false)]);let second=conf.providers[0].models[0].clone();conf.providers[0].models.push(ProviderModel{record_key:"capped-second".into(),provider_model_id:"fixture-model-two".into(),..second});
 let r=runner(conf.clone(),SchedulingState::default(),q.clone(),None);
 for snapshot in [QuotaSnapshot::Unknown,QuotaSnapshot::Unsupported,QuotaSnapshot::Stale{available:Some(true),windows:vec![window("short",0.)]}]{q.set("capped",snapshot);assert_ne!(call(&client,&r,"capped").await.0,200);assert_ne!(call(&client,&r,"capped-second").await.0,200);selected(&call(&client,&r,"cap-route").await,"fallback");}
 q.set("capped",known(50.));assert_ne!(call(&client,&r,"capped").await.0,200);selected(&call(&client,&r,"cap-route").await,"fallback");
 let mut reset=window("short",50.);reset.reset_at_ms=Some(now()-1);q.set("capped",QuotaSnapshot::Known{available:Some(true),windows:vec![reset]});assert_ne!(call(&client,&r,"capped").await.0,200,"reset fabricated fresh capacity");
 let mut expired=window("short",1.);expired.expires_at_ms=now()-1;q.set("capped",QuotaSnapshot::Known{available:Some(true),windows:vec![expired]});assert_ne!(call(&client,&r,"capped").await.0,200,"stale observation admitted a capped model");
 assert_eq!(hits(&client,&up,"capped").await,0,"cap-rejected provider was called");q.set("capped",known(49.));selected(&call(&client,&r,"capped").await,"capped");drop(r);passed.push("all routes/direct share caps; unknown stale reset never assume full");
 // Same-period global and per-model windows must all satisfy the provider cap.
 let mut scoped=window("short",99.);scoped.model_ids=Some(vec!["fixture-model".into()]);q.set("scoped",QuotaSnapshot::Known{available:Some(true),windows:vec![window("short",10.),scoped]});let mut conf=config(&up,&[("scoped",cap(50))],vec![]);let second=conf.providers[0].models[0].clone();conf.providers[0].models.push(ProviderModel{record_key:"scoped-other".into(),provider_model_id:"other-model".into(),..second});let r=runner(conf,SchedulingState::default(),q.clone(),None);assert_ne!(call(&client,&r,"scoped").await.0,200);assert_eq!(hits(&client,&up,"scoped").await,0);selected(&call(&client,&r,"scoped-other").await,"scoped");drop(r);passed.push("same-period model windows enforce the tightest applicable cap");
 q.set("unknown",QuotaSnapshot::Unknown);q.set("fresh",known(90.));let conf=config(&up,&[("unknown",vec![]),("fresh",vec![])],vec![route("fresh-first",&["unknown","fresh"],RoutingStrategy::Smart,false)]);let r=runner(conf,SchedulingState::default(),q.clone(),None);selected(&call(&client,&r,"fresh-first").await,"fresh");drop(r);passed.push("uncapped unknown remains lower-priority fallback");
 // Bindings are explicit (namespace, session, route); eligibility wins and success rebinds.
 let q=Arc::new(Quotas::default());q.set("sticky-a",known(10.));q.set("sticky-b",known(20.));let conf=config(&up,&[("sticky-a",cap(80)),("sticky-b",cap(80))],vec![route("sticky",&["sticky-a","sticky-b"],RoutingStrategy::Smart,true),route("other",&["sticky-b","sticky-a"],RoutingStrategy::Order,true)]);let state=SchedulingState::default();
 let r=runner(conf.clone(),state.clone(),q.clone(),Some(("project-a","session")));selected(&call(&client,&r,"sticky").await,"sticky-a");drop(r);q.set("sticky-b",known(0.));let r=runner(conf.clone(),state.clone(),q.clone(),Some(("project-a","session")));selected(&call(&client,&r,"sticky").await,"sticky-a");selected(&call(&client,&r,"other").await,"sticky-b");drop(r);
 let r=runner(conf.clone(),state.clone(),q.clone(),Some(("project-b","session")));selected(&call(&client,&r,"sticky").await,"sticky-b");drop(r);
 q.set("sticky-a",known(80.));let r=runner(conf.clone(),state.clone(),q.clone(),Some(("project-a","session")));selected(&call(&client,&r,"sticky").await,"sticky-b");drop(r);q.set("sticky-a",known(0.));q.set("sticky-b",known(70.));let r=runner(conf,state,q.clone(),Some(("project-a","session")));selected(&call(&client,&r,"sticky").await,"sticky-b");drop(r);passed.push("namespace/route sticky survives runners but cannot bypass cap; replacement rebinds");
 // Without an analytics sink, HTTP 200 with an explicit failed/incomplete business terminal must not bind.
 for terminal in ["incomplete","failed"]{let q=Arc::new(Quotas::default());let conf=config(&up,&[("no-bind-a",vec![]),("no-bind-b",vec![])],vec![route("no-bind",&["no-bind-a","no-bind-b"],RoutingStrategy::Rotate,true)]);let state=SchedulingState::default();control(&client,&up,"no-bind-a",terminal).await;let r=runner(conf.clone(),state.clone(),q.clone(),Some(("no-sink","session")));let failed=call(&client,&r,"no-bind").await;assert_eq!(failed.0,200);assert!(!failed.1.contains("response.completed"));control(&client,&up,"no-bind-a","ok").await;selected(&call(&client,&r,"no-bind-a").await,"no-bind-a");drop(r);let r=runner(conf.clone(),state.clone(),q.clone(),Some(("no-sink","session")));selected(&call(&client,&r,"no-bind").await,"no-bind-b");drop(r);let r=runner(conf,state,q,Some(("no-sink","session")));selected(&call(&client,&r,"no-bind").await,"no-bind-b");drop(r);}passed.push("without analytics only true Completed creates sticky binding");
 // A cold account is never penetrated by direct requests, including another Runner.
 let q=Arc::new(Quotas::default());let state=SchedulingState::default();let conf=config(&up,&[("cold",vec![])],vec![]);control(&client,&up,"cold","rate").await;let r=runner(conf.clone(),state.clone(),q.clone(),None);assert_eq!(call(&client,&r,"cold").await.0,429);drop(r);let r=runner(conf.clone(),state.clone(),q.clone(),None);let blocked=call(&client,&r,"cold").await;assert_eq!(blocked.0,429);assert!(blocked.2.is_some_and(|s|s>=1));assert_eq!(hits(&client,&up,"cold").await,1);drop(r);
 tokio::time::sleep(Duration::from_millis(1100)).await;control(&client,&up,"cold","hold").await;let r=runner(conf.clone(),state.clone(),q.clone(),None);let endpoint=r.endpoint().to_owned();let token=r.token().to_owned();let c=client.clone();let leader=tokio::spawn(async move{let result=response(&c,&endpoint,&token,"cold").await;let s=result.status().as_u16();let body=result.text().await.unwrap_or_default();(s,body,None)});wait_hits(&client,&up,"cold",2).await;
 let mut tasks=Vec::new();for _ in 0..12{let c=client.clone();let endpoint=r.endpoint().to_owned();let token=r.token().to_owned();tasks.push(tokio::spawn(async move{response(&c,&endpoint,&token,"cold").await.status().as_u16()}));}for task in tasks{assert_eq!(task.await.unwrap(),429,"parallel half-open penetration");}assert_eq!(hits(&client,&up,"cold").await,2);release(&client,&up,"cold").await;selected(&leader.await.unwrap(),"cold");selected(&call(&client,&r,"cold").await,"cold");drop(r);passed.push("shared cooldown retry time; expiry permits only one probe; success releases");
 // Retry-After describes the earliest eligible member, not the last failure's longer cooldown.
 let state=SchedulingState::default();let q=Arc::new(Quotas::default());let conf=config(&up,&[("earliest-a",vec![]),("earliest-b",vec![])],vec![route("earliest",&["earliest-a","earliest-b"],RoutingStrategy::Order,false)]);let r=runner(conf,state,q.clone(),None);control(&client,&up,"earliest-b","server").await;assert_eq!(call(&client,&r,"earliest-b").await.0,503);control(&client,&up,"earliest-a","longrate").await;let blocked=call(&client,&r,"earliest").await;assert_eq!(blocked.0,429);assert!(blocked.2.is_some_and(|s|(1..=5).contains(&s)),"retry time ignored the earlier member");assert_eq!(hits(&client,&up,"earliest-b").await,1,"cold fallback was penetrated");drop(r);passed.push("all-cold Retry-After uses earliest member without penetration");
 // Client cancellation must release a half-open permit even while upstream sends no new chunk.
 let state=SchedulingState::default();let conf=config(&up,&[("cancel-probe",vec![])],vec![]);control(&client,&up,"cancel-probe","rate").await;let r=runner(conf,state,Arc::new(Quotas::default()),None);assert_eq!(call(&client,&r,"cancel-probe").await.0,429);tokio::time::sleep(Duration::from_millis(1100)).await;control(&client,&up,"cancel-probe","stream_hold").await;let held=response(&client,r.endpoint(),r.token(),"cancel-probe").await;assert_eq!(held.status().as_u16(),200);drop(held);tokio::time::sleep(Duration::from_millis(150)).await;control(&client,&up,"cancel-probe","ok").await;let next=tokio::time::timeout(Duration::from_secs(2),call(&client,&r,"cancel-probe")).await.expect("cancelled probe kept its slot");selected(&next,"cancel-probe");release(&client,&up,"cancel-probe").await;drop(r);passed.push("cancelled streaming probe releases before upstream resumes");
 // Replacing an account invalidates old captured runners, not ordinary refreshes.
 let state=SchedulingState::default();let conf=config(&up,&[("epoch",vec![])],vec![]);let old=runner(conf.clone(),state.clone(),q.clone(),None);state.invalidate_provider("fixture","epoch");let new=runner(conf,state,q,None);assert_ne!(call(&client,&old,"epoch").await.0,200);assert_eq!(hits(&client,&up,"epoch").await,0);selected(&call(&client,&new,"epoch").await,"epoch");drop(old);drop(new);passed.push("account replacement invalidates captured runner epochs");
 println!("{}",json!({"passed":passed,"network":"loopback only","real_quota_verified":false}));
}
'''


def main():
    with tempfile.TemporaryDirectory(prefix='velune-smart-routing-') as directory:
        temporary = Path(directory)
        (temporary / 'src').mkdir()
        (temporary / 'src/main.rs').write_text(PROGRAM)
        dependencies = '\n'.join('%s={path="%s"}' % (name, ROOT / 'packages' / folder) for name, folder in [('velune-gateway','gateway'),('velune-ai','ai')])
        (temporary / 'Cargo.toml').write_text('[package]\nname="manual-smart-routing"\nversion="0.0.0"\nedition="2024"\n[workspace]\n[dependencies]\n' + dependencies + '\nreqwest={version="0.13",default-features=false,features=["json"]}\ntokio={version="1",features=["macros","rt","time","sync"]}\nserde_json="1"\n')
        target = ROOT / 'target/manual-smart-routing'
        subprocess.run(['cargo','build','--quiet','--offline','--manifest-path',str(temporary / 'Cargo.toml'),'--target-dir',str(target)],cwd=ROOT,check=True)
        server = ThreadingHTTPServer(('127.0.0.1',0),Fixture)
        threading.Thread(target=server.serve_forever,daemon=True).start()
        process_home = temporary / 'home'
        process_home.mkdir()
        environment = dict(os.environ,HOME=str(process_home),HTTP_PROXY='http://127.0.0.1:1',HTTPS_PROXY='http://127.0.0.1:1',ALL_PROXY='http://127.0.0.1:1',NO_PROXY='127.0.0.1,localhost',no_proxy='127.0.0.1,localhost')
        try:
            subprocess.run([str(target/'debug/manual-smart-routing'),'http://127.0.0.1:%d'%server.server_port],cwd=temporary,env=environment,check=True,timeout=90)
        finally:
            for gate in GATES.values():
                gate.set()
            server.shutdown()
            server.server_close()


if __name__ == '__main__':
    main()
