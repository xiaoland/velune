#!/usr/bin/env python3
"""按需人工验收：隔离源码/配置、合成 AK/SK、回环 HTTP；不接入测试或 CI。"""
import hashlib
import hmac
import json
import os
from pathlib import Path
import shutil
import subprocess
import tempfile
import threading
import time
from http.server import BaseHTTPRequestHandler, ThreadingHTTPServer

ROOT = Path(__file__).resolve().parents[1]
STATE = {'quota': 'normal', 'models': {}, 'hits': {}, 'gate': threading.Event()}
ERRORS = []

class Fixture(BaseHTTPRequestHandler):
    protocol_version = 'HTTP/1.1'
    def log_message(self, *_): pass
    def handle(self):
        try: super().handle()
        except (ConnectionResetError, BrokenPipeError): pass
    def reply(self, status, value, headers=None):
        body = value if isinstance(value, bytes) else json.dumps(value).encode()
        self.send_response(status)
        self.send_header('Content-Type', 'application/json')
        self.send_header('Content-Length', str(len(body)))
        for k,v in (headers or {}).items(): self.send_header(k,v)
        self.end_headers()
        try: self.wfile.write(body)
        except (BrokenPipeError, ConnectionResetError): pass
    def do_GET(self): self.reply(200, STATE['hits'])
    def do_POST(self):
        body = self.rfile.read(int(self.headers['Content-Length']))
        value = json.loads(body)
        if self.path == '/control':
            STATE.update(value)
            if value.get('release'): STATE['gate'].set()
            self.reply(200, {})
            return
        if self.path.startswith('/?Action='):
            STATE['hits']['quota'] = STATE['hits'].get('quota',0)+1
            auth = self.headers.get('Authorization','')
            try:
                credential = auth.split('Credential=')[1].split(',')[0]
                access,scope=credential.split('/',1)
                secret={'fixture-ak':'fixture-secret','second-ak':'second-secret'}[access]
                stamp=self.headers['X-Date']
                digest=hashlib.sha256(body).hexdigest()
                signed='content-type;host;x-content-sha256;x-date'
                canonical='POST\n/\nAction=GetCodingPlanUsage&Version=2024-01-01\ncontent-type:application/json\nhost:'+self.headers['Host']+'\nx-content-sha256:'+digest+'\nx-date:'+stamp+'\n\n'+signed+'\n'+digest
                key=secret.encode()
                for part in [stamp[:8],'cn-beijing','ark','request']:
                    key=hmac.new(key,part.encode(),hashlib.sha256).digest()
                expected=hmac.new(key,('HMAC-SHA256\n'+stamp+'\n'+scope+'\n'+hashlib.sha256(canonical.encode()).hexdigest()).encode(),hashlib.sha256).hexdigest()
                assert auth.endswith('Signature='+expected)
                assert self.headers['X-Content-Sha256']==digest and value=={}
                assert self.headers['User-Agent'].startswith('Velune/')
            except Exception:
                ERRORS.append('control-plane signature or credential isolation failed')
                self.reply(403,{})
                return
            mode=STATE['quota']
            if mode=='hold':
                STATE['gate'].clear()
                STATE['gate'].wait(12)
            if mode=='denied': self.reply(403,{}); return
            if mode=='envelope': self.reply(200,{'ResponseMetadata':{'Error':{'Code':'AccessDenied'}}}); return
            if mode=='redirect': self.reply(307,{}, {'Location':self.server.base+'/redirect'}); return
            if mode=='oversized': self.reply(200,b' '*270000); return
            percent={'high':80,'negative':-1,'overflow':101,'string':'0.5','nan':'NaN','hold':99}.get(mode,20)
            windows=[{'Level':level,'Percent':percent,'ResetTimestamp':int(time.time())+3600} for level in ['session','weekly','monthly']]
            if mode=='missing': del windows[0]['Percent']
            if mode=='empty': windows=[]
            if mode=='duplicate': windows.append(dict(windows[0]))
            if mode=='sentinel': windows[0]['ResetTimestamp']=-1
            self.reply(200,{'Result':{'QuotaUsage':windows}})
            return
        if self.path=='/redirect': ERRORS.append('redirect followed');self.reply(500,{});return
        key=self.headers.get('Authorization','').removeprefix('Bearer ')
        if not key: key=self.headers.get('x-api-key','')
        assert key.startswith('model-'), 'query credentials reached inference'
        assert self.headers.get('User-Agent','').startswith('Velune/')
        assert not any(secret in str(self.headers)+body.decode() for secret in ['fixture-ak','fixture-secret','second-ak','second-secret'])
        STATE['hits'][key]=STATE['hits'].get(key,0)+1
        mode=STATE['models'].get(key,'normal')
        failures={'quota':(429,'QuotaExceeded','You have exceeded the 5-hour usage quota. It will reset later.'),'queue':(429,'QuotaExceeded','The request has exceeded the quota.'),'plan':(400,'InvalidSubscription','subscription expired'),'model':(404,'UnsupportedModel','model unsupported'),'access':(403,'AccessDenied','not allowed'),'burst':(429,'RequestBurstTooFast','busy')}
        if mode in failures:
            status,code,message=failures[mode]
            self.reply(status,{'error':{'code':code,'message':message}}, {'x-fixture-error':'preserved','Retry-After':'60'})
            return
        if self.path.endswith('/chat/completions'):
            self.reply(200,{'id':'fixture','object':'chat.completion','model':value['model'],'choices':[{'index':0,'message':{'role':'assistant','content':key},'finish_reason':'stop'}],'usage':{'prompt_tokens':1,'completion_tokens':1,'total_tokens':2}})
            return
        if self.path.endswith('/messages'):
            self.reply(200,{'id':'fixture','type':'message','role':'assistant','model':value['model'],'content':[{'type':'text','text':key}],'stop_reason':'end_turn','usage':{'input_tokens':1,'output_tokens':1}})
            return
        self.reply(200,{'id':'fixture','object':'response','status':'completed','model':value['model'],'output':[{'id':'msg','type':'message','role':'assistant','status':'completed','content':[{'type':'output_text','text':key,'annotations':[]}]}],'usage':{'input_tokens':1,'output_tokens':1,'total_tokens':2}})

PROGRAM=r'''
use std::{path::PathBuf,time::{Duration,Instant}};
use serde_json::{Value,json};
use velune_application::{Application,Options,ProviderDraft,AuthenticationEdit};
async fn control(c:&reqwest::Client,up:&str,v:Value){c.post(format!("{up}/control")).json(&v).send().await.unwrap();}
fn draft(id:&str,up:&str,cap:bool,edit:Value)->ProviderDraft{serde_json::from_value(json!({"id":id,"name":id,"protocol":"responsesV1","endpoint":format!("{up}/api/coding/v3"),"executionBackend":"http","requestProfile":"arkCoding","authenticationSource":"apiKey","quotaSource":"arkCodingUsage","models":[{"recordKey":id,"providerModelId":"fixture","nickname":"fixture","icon":null,"contextWindow":8192,"maxOutputTokens":16,"reasoningLevels":null}],"quotaLimits":if cap{json!([{"windowId":"ark:session","maxUsedPercent":50}])}else{json!([])},"quotaAuthenticationEdit":edit})).unwrap()}
fn access(id:&str,secret:&str,confirmed:bool)->Value{json!({"kind":"setAccessKey","accessKeyId":id,"secretAccessKey":secret,"confirmed":confirmed})}
fn save(a:&mut Application,up:&str,key:Option<&str>,cap:bool,edit:Value){a.save_provider("default".into(),draft("ark",up,cap,edit),key.map_or(AuthenticationEdit::Keep,|v|AuthenticationEdit::SetApiKey{value:v.into()})).unwrap();}
fn description(a:&mut Application)->Value{let l=serde_json::to_value(a.list().unwrap()).unwrap(); l["gateways"][0]["providers"].as_array().unwrap().iter().find(|p|p["id"]=="ark").unwrap()["quotaAuthentication"].clone()}
fn refresh(a:&mut Application)->Value{a.refresh_provider_quota("default".into(),"ark".into()).unwrap();let end=Instant::now()+Duration::from_secs(12);loop{let q=serde_json::to_value(a.provider_quota("default".into(),"ark".into()).unwrap()).unwrap();if !q["refreshing"].as_bool().unwrap(){return q}assert!(Instant::now()<end);std::thread::sleep(Duration::from_millis(15));}}
async fn call(c:&reqwest::Client,r:&velune_gateway::Runner,model:&str)->(u16,Vec<u8>){let x=c.post(format!("{}/responses",r.endpoint())).bearer_auth(r.token()).json(&json!({"model":format!("velune/model/{model}"),"input":"synthetic fixture","stream":false})).send().await.unwrap();let status=x.status().as_u16();(status,x.bytes().await.unwrap().to_vec())}
fn ok(reply:(u16,Vec<u8>),key:&str){assert_eq!(reply.0,200);let v:Value=serde_json::from_slice(&reply.1).unwrap();assert_eq!(v["status"],"completed");assert_eq!(v["output"][0]["content"][0]["text"],key);}
#[tokio::main(flavor="current_thread")]
async fn main(){let args:Vec<_>=std::env::args().collect();let up=&args[1];let home=PathBuf::from(&args[2]);std::fs::create_dir_all(home.join("resources")).unwrap();let opts=Options{home_directory:home.clone(),resources_directory:home.join("resources")};let mut a=Application::open(opts.clone()).unwrap();let c=reqwest::Client::new();let mut passed=Vec::new();
 let templates=velune_application::provider_templates();let t=templates.iter().find(|t|t.id=="arkCodingPlan").unwrap();assert_eq!(serde_json::to_value(t).unwrap()["protocol"],"responsesV1");passed.push("template");
 save(&mut a,up,Some("model-one"),false,Value::Null);ok(call(&c,&a.manual_runner(),"ark").await,"model-one");passed.push("no query credentials: uncapped inference only");
 save(&mut a,up,None,true,access("fixture-ak","fixture-secret",false));let q=refresh(&mut a);assert_eq!(q["freshness"],"fresh");assert_eq!(q["windows"].as_array().unwrap().len(),3);assert_eq!(description(&mut a)["bindingConfirmed"],false);assert_eq!(call(&c,&a.manual_runner(),"ark").await.0,503);passed.push("query valid but unconfirmed cap pauses");
 save(&mut a,up,None,true,json!({"kind":"confirmBinding"}));refresh(&mut a);ok(call(&c,&a.manual_runner(),"ark").await,"model-one");passed.push("explicit confirmation admits observed quota");
 for (id,protocol,path) in [("native-chat",velune_application::config::GatewayProtocol::ChatCompletionsV1,"chat/completions"),("native-messages",velune_application::config::GatewayProtocol::MessagesV1,"messages")] { let mut d=draft(id,up,false,Value::Null);d.protocol=protocol;if path=="messages"{d.endpoint=format!("{up}/api/coding");}let key=format!("model-{id}");a.save_provider("default".into(),d,AuthenticationEdit::SetApiKey{value:key.clone()}).unwrap();let r=a.manual_runner();let body=if path=="messages"{json!({"model":format!("velune/model/{id}"),"messages":[{"role":"user","content":"fixture"}],"max_tokens":16,"stream":false})}else{json!({"model":format!("velune/model/{id}"),"messages":[{"role":"user","content":"fixture"}],"stream":false})};let response=c.post(format!("{}/{path}",r.endpoint())).bearer_auth(r.token()).header("anthropic-version","2023-06-01").json(&body).send().await.unwrap();assert_eq!(response.status(),200);let value:Value=response.json().await.unwrap();assert_eq!(if path=="messages"{&value["content"][0]["text"]}else{&value["choices"][0]["message"]["content"]},&json!(key));passed.push(id); }
 let public=serde_json::to_string(&a.list().unwrap()).unwrap();assert!(!public.contains("fixture-secret")&&!public.contains("fixture-ak"));passed.push("public description excludes query secrets");
 control(&c,up,json!({"quota":"high"})).await;let q=refresh(&mut a);assert_eq!(q["windows"][0]["usedPercent"],80.0);assert_eq!(call(&c,&a.manual_runner(),"ark").await.0,429);passed.push("fresh cap blocks");
 control(&c,up,json!({"quota":"normal"})).await;save(&mut a,up,Some("model-two"),true,Value::Null);refresh(&mut a);assert_eq!(description(&mut a)["bindingConfirmed"],false);assert_eq!(call(&c,&a.manual_runner(),"ark").await.0,503);save(&mut a,up,None,true,json!({"kind":"confirmBinding"}));refresh(&mut a);ok(call(&c,&a.manual_runner(),"ark").await,"model-two");passed.push("model rotation revokes, explicit reconfirm restores");
 let old=a.manual_runner();save(&mut a,up,None,true,access("second-ak","second-secret",false));refresh(&mut a);assert_eq!(call(&c,&old,"ark").await.0,503);assert_eq!(call(&c,&a.manual_runner(),"ark").await.0,503);passed.push("query rotation invalidates captured runner and association");
 save(&mut a,up,None,true,json!({"kind":"confirmBinding"}));refresh(&mut a);
 for mode in ["string","sentinel","negative","overflow","nan","missing","empty","duplicate","denied","envelope","redirect","oversized"]{control(&c,up,json!({"quota":mode})).await;let q=refresh(&mut a);if ["string","sentinel"].contains(&mode){assert_eq!(q["freshness"],"fresh");if mode=="string"{assert_eq!(q["windows"][0]["usedPercent"],0.5);}else{assert!(q["windows"][0]["resetAtMs"].is_null());}}else{assert_ne!(q["freshness"],"fresh","quota mode {mode}");assert_eq!(call(&c,&a.manual_runner(),"ark").await.0,503);}passed.push(mode);}
 control(&c,up,json!({"quota":"normal"})).await;save(&mut a,up,None,false,Value::Null);refresh(&mut a);
 // Reject a late reading from a replaced query credential.
 control(&c,up,json!({"quota":"hold"})).await;let before:Value=c.get(up).send().await.unwrap().json().await.unwrap();a.refresh_provider_quota("default".into(),"ark".into()).unwrap();let deadline=Instant::now()+Duration::from_secs(3);loop{let n:Value=c.get(up).send().await.unwrap().json().await.unwrap();if n["quota"].as_u64().unwrap()>before["quota"].as_u64().unwrap(){break;}assert!(Instant::now()<deadline);tokio::time::sleep(Duration::from_millis(10)).await;}
 save(&mut a,up,None,false,access("fixture-ak","fixture-secret",true));control(&c,up,json!({"quota":"normal","release":true})).await;let q=refresh(&mut a);assert_eq!(q["windows"][0]["usedPercent"],20.0);passed.push("old query generation cannot overwrite replacement");
 // A quota refusal invalidates even a sampling request that was already in flight.
 control(&c,up,json!({"quota":"hold"})).await;let before:Value=c.get(up).send().await.unwrap().json().await.unwrap();a.refresh_provider_quota("default".into(),"ark".into()).unwrap();let deadline=Instant::now()+Duration::from_secs(3);loop{let n:Value=c.get(up).send().await.unwrap().json().await.unwrap();if n["quota"].as_u64().unwrap()>before["quota"].as_u64().unwrap(){break;}assert!(Instant::now()<deadline);tokio::time::sleep(Duration::from_millis(10)).await;}
 control(&c,up,json!({"models":{"model-two":"quota"}})).await;let wire=call(&c,&a.manual_fresh_runner(),"ark").await;assert_eq!(wire.0,429);assert_eq!(wire.1,br#"{"error": {"code": "QuotaExceeded", "message": "You have exceeded the 5-hour usage quota. It will reset later."}}"#);passed.push("quota classification preserves exact wire bytes");control(&c,up,json!({"quota":"normal","models":{},"release":true})).await;let deadline=Instant::now()+Duration::from_secs(3);loop{let q=serde_json::to_value(a.provider_quota("default".into(),"ark".into()).unwrap()).unwrap();assert_ne!(q["windows"][0]["usedPercent"],99.0);if q["freshness"]=="fresh"{assert_eq!(q["windows"][0]["usedPercent"],20.0);break;}assert!(Instant::now()<deadline);tokio::time::sleep(Duration::from_millis(15)).await;}passed.push("refusal discards older inflight observation and resamples");
 let persisted=std::fs::read(home.join("generic-config.json")).unwrap();let duplicate=a.save_provider("default".into(),draft("duplicate",up,false,access("fixture-ak","fixture-secret",true)),AuthenticationEdit::SetApiKey{value:"model-other".into()});assert!(duplicate.is_err());assert_eq!(std::fs::read(home.join("generic-config.json")).unwrap(),persisted);passed.push("duplicate query binding rejected atomically");
 let persisted=std::fs::read(home.join("generic-config.json")).unwrap();let duplicate=a.save_provider("default".into(),draft("duplicate-model",up,false,Value::Null),AuthenticationEdit::SetApiKey{value:"model-two".into()});assert!(duplicate.is_err());let mut bad=draft("invalid-target",up,false,Value::Null);bad.endpoint="https://ark.cn-beijing.volces.com/api/v3".into();assert!(a.save_provider("default".into(),bad,AuthenticationEdit::SetApiKey{value:"model-invalid".into()}).is_err());assert_eq!(std::fs::read(home.join("generic-config.json")).unwrap(),persisted);passed.push("duplicate model key and paid target rejected atomically");
 // Actual gateway route: permission failures stop; exhausted quota and transient failures switch.
 a.save_provider("default".into(),draft("backup",up,false,Value::Null),AuthenticationEdit::SetApiKey{value:"model-backup".into()}).unwrap();a.save_model_route("default".into(),velune_application::config::ModelRoute{record_key:"pool".into(),name:"pool".into(),members:vec!["ark".into(),"backup".into()],strategy:velune_application::config::RoutingStrategy::Order,weights:Default::default(),sticky:false}).unwrap();
 for (mode,status) in [("plan",400),("model",404),("access",403),("quota",200),("queue",200),("burst",200)]{control(&c,up,json!({"models":{"model-two":mode}})).await;let r=a.manual_fresh_runner();let reply=call(&c,&r,"pool").await;assert_eq!(reply.0,status);if status==200{ok(reply,"model-backup");}else{let v:Value=serde_json::from_slice(&reply.1).unwrap();assert!(v["error"]["code"].is_string());}passed.push(mode);}
 control(&c,up,json!({"models":{"model-two":"burst"}})).await;let _=call(&c,&a.manual_runner(),"ark").await;save(&mut a,up,None,false,access("fixture-ak","fixture-secret",true));refresh(&mut a);assert_eq!(call(&c,&a.manual_runner(),"ark").await.0,429);passed.push("query rotation retains model cooldown");
 save(&mut a,up,None,true,json!({"kind":"clear"}));assert_eq!(description(&mut a)["configured"],false);assert_eq!(call(&c,&a.manual_fresh_runner(),"ark").await.0,503);passed.push("clear preserves model auth but cap pauses");
 a.close().unwrap();let mut a=Application::open(opts).unwrap();assert_eq!(description(&mut a)["configured"],false);save(&mut a,up,None,false,access("fixture-ak","fixture-secret",true));refresh(&mut a);control(&c,up,json!({"models":{}})).await;ok(call(&c,&a.manual_runner(),"ark").await,"model-two");a.close().unwrap();passed.push("reopen and independent credentials persist");
 let config=home.join("generic-config.json");let mut value:Value=serde_json::from_slice(&std::fs::read(&config).unwrap()).unwrap();for gateway in value["gateways"].as_array_mut().unwrap(){for provider in gateway["providers"].as_array_mut().unwrap(){provider.as_object_mut().unwrap().remove("quotaAuthentication");}}std::fs::write(&config,serde_json::to_vec(&value).unwrap()).unwrap();let mut a=Application::open(Options{home_directory:home.clone(),resources_directory:home.join("resources")}).unwrap();assert_eq!(description(&mut a)["configured"],false);ok(call(&c,&a.manual_runner(),"ark").await,"model-two");a.close().unwrap();passed.push("absent optional query config does not reset existing models or key");
 println!("{}",json!({"passed":passed,"count":passed.len(),"real_requests":false}));
}
'''

def replace(path,old,new):
    text=path.read_text();assert text.count(old)==1,(path.name,old[:70],text.count(old));path.write_text(text.replace(old,new))

def main():
    with tempfile.TemporaryDirectory(prefix='velune-ark-manual-') as directory:
        root=Path(directory);source=root/'source';source.mkdir()
        shutil.copytree(ROOT/'packages',source/'packages')
        shutil.copy2(ROOT/'Cargo.toml',source/'Cargo.toml');shutil.copy2(ROOT/'Cargo.lock',source/'Cargo.lock')
        server=ThreadingHTTPServer(('127.0.0.1',0),Fixture);server.base='http://127.0.0.1:%d'%server.server_port
        threading.Thread(target=server.serve_forever,daemon=True).start();base=server.base
        replace(source/'packages/application/src/ark_quota.rs','open.volcengineapi.com','127.0.0.1:%d'%server.server_port)
        replace(source/'packages/application/src/ark_quota.rs','https://{HOST}/?{QUERY}','http://{HOST}/?{QUERY}')
        replace(source/'packages/application/src/config.rs','https://ark.cn-beijing.volces.com/api/coding/v3',base+'/api/coding/v3')
        replace(source/'packages/application/src/config.rs','https://ark.cn-beijing.volces.com/api/coding',base+'/api/coding')
        for filename,begin,end,changes in [
            ('ai-provider/src/config.rs','if request_profile == RequestProfile::ArkCoding {','if request_profile == RequestProfile::KimiCoding {',[('endpoint.transport() != Transport::Https','endpoint.transport() != Transport::Http'),('endpoint.host() != "ark.cn-beijing.volces.com"','endpoint.host() != "127.0.0.1"'),('endpoint.port().get() != 443','endpoint.port().get() != %d'%server.server_port)]),
            ('gateway/src/config.rs','if provider.request_profile == RequestProfile::ArkCoding {','if provider.request_profile == RequestProfile::KimiCoding {',[('endpoint.scheme() != "https"','endpoint.scheme() != "http"'),('endpoint.host_str() != Some("ark.cn-beijing.volces.com")','endpoint.host_str() != Some("127.0.0.1")'),('endpoint.port_or_known_default() != Some(443)','endpoint.port_or_known_default() != Some(%d)'%server.server_port)])]:
            p=source/'packages'/filename;text=p.read_text();start=text.index(begin);stop=text.index(end,start);block=text[start:stop]
            for old,new in changes: assert block.count(old)==1,(filename,old);block=block.replace(old,new)
            p.write_text(text[:start]+block+text[stop:])
        with (source/'packages/application/src/lib.rs').open('a') as f:f.write('\nimpl Application { pub fn manual_runner(&self)->velune_gateway::Runner{self.runtime.manual_runner(false)} pub fn manual_fresh_runner(&self)->velune_gateway::Runner{self.runtime.manual_runner(true)} }\n')
        with (source/'packages/application/src/local/mod.rs').open('a') as f:f.write('''\nimpl CoreRuntime { pub fn manual_runner(&self,fresh:bool)->velune_gateway::Runner { let g=self.gateways.iter().find(|g|g.id=="default").unwrap();velune_gateway::Runner::start_with_scheduling(g.to_gateway_config().unwrap(),crate::authentication_resolver::ProviderResolver::capture_for_dispatch(g,&self.options,self.subscription_quota.clone()),Default::default(),None,if fresh{Default::default()}else{self.scheduling_state.clone()},self.subscription_quota.source(&g.id),None,self.execution_backends.clone()).unwrap() } }\n''')
        harness=root/'harness';(harness/'src').mkdir(parents=True);(harness/'src/main.rs').write_text(PROGRAM)
        deps='\n'.join('%s={path="%s"}'%(name,source/'packages'/folder) for name,folder in [('velune-application','application'),('velune-gateway','gateway')])
        (harness/'Cargo.toml').write_text('[package]\nname="manual-ark-coding"\nversion="0.0.0"\nedition="2024"\n[workspace]\n[dependencies]\n'+deps+'\nserde_json="1"\nreqwest={version="0.13",default-features=false,features=["json"]}\ntokio={version="1",features=["macros","rt","time"]}\n')
        try:
            target=ROOT/'target/manual-kimi-coding'
            subprocess.run(['cargo','build','--quiet','--offline','--manifest-path',str(harness/'Cargo.toml'),'--target-dir',str(target)],check=True)
            env=dict(os.environ,HOME=str(root/'home'),HTTP_PROXY='http://127.0.0.1:1',HTTPS_PROXY='http://127.0.0.1:1',ALL_PROXY='http://127.0.0.1:1',NO_PROXY='127.0.0.1,localhost',no_proxy='127.0.0.1,localhost')
            subprocess.run([str(target/'debug/manual-ark-coding'),base,str(root/'application')],env=env,cwd=root,timeout=180,check=True)
            assert not ERRORS,ERRORS
        finally:
            STATE['gate'].set();server.shutdown();server.server_close()

if __name__=='__main__': main()
