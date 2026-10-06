#!/usr/bin/env python3
"""Manually invoked, isolated native Messages HTTP check; no real credentials or models."""
import json
from http.server import BaseHTTPRequestHandler, ThreadingHTTPServer
from pathlib import Path
import subprocess
import tempfile
import threading

ROOT = Path(__file__).resolve().parents[1]
RESPONSE = b'{"id":"synthetic","type":"message","role":"assistant","content":[{"type":"thinking","thinking":"reason","signature":"opaque"},{"type":"text","text":"hello"},{"type":"tool_use","id":"tool1","name":"read","input":{"path":"synthetic"}}],"stop_reason":"tool_use","usage":{"input_tokens":3,"output_tokens":4},"unknown_future":{"retained":true}}'
STREAM = b'event: message_start\ndata: {"type":"message_start","message":{"id":"synthetic","usage":{"input_tokens":3}}}\n\nevent: content_block_start\ndata: {"type":"content_block_start","index":0,"content_block":{"type":"thinking","thinking":""}}\n\nevent: content_block_delta\ndata: {"type":"content_block_delta","index":0,"delta":{"type":"thinking_delta","thinking":"reason"}}\n\nevent: content_block_delta\ndata: {"type":"content_block_delta","index":0,"delta":{"type":"signature_delta","signature":"opaque"}}\n\nevent: content_block_start\ndata: {"type":"content_block_start","index":1,"content_block":{"type":"tool_use","id":"tool1","name":"read","input":{}}}\n\nevent: content_block_delta\ndata: {"type":"content_block_delta","index":1,"delta":{"type":"input_json_delta","partial_json":"{\\"path\\":\\"synthetic\\"}"}}\n\nevent: content_block_delta\ndata: {"type":"content_block_delta","index":2,"delta":{"type":"text_delta","text":"hello"}}\n\nevent: future_event\ndata: {"type":"future_event","keep":"opaque"}\n\nevent: message_delta\ndata: {"type":"message_delta","delta":{"stop_reason":"tool_use"},"usage":{"output_tokens":4}}\n\nevent: message_stop\ndata: {"type":"message_stop"}\n\n'
ERROR = b'{"type":"error","error":{"type":"rate_limit_error","message":"synthetic"},"request_id":"synthetic-error"}'
STREAM_ERROR = b'event: error\ndata: {"type":"error","error":{"type":"overloaded_error","message":"synthetic"}}\n\n'
requests = []
class Handler(BaseHTTPRequestHandler):
    def log_message(self, *_args):
        pass
    def do_POST(self):
        body = json.loads(self.rfile.read(int(self.headers['content-length'])))
        requests.append((self.path, dict(self.headers), body))
        assert self.path == '/v1/messages', self.path
        assert self.headers.get('x-api-key') == 'SYNTHETIC_UPSTREAM_KEY'
        assert self.headers.get('authorization') is None
        assert self.headers.get('anthropic-version') == '2099-01-01'
        assert self.headers.get('anthropic-beta') == 'synthetic-beta'
        assert body['model'] == 'actual-api-model'
        case = body['metadata']['case']
        wire = ERROR if case == 'error' else STREAM_ERROR if case == 'stream-error' else STREAM if body.get('stream') else RESPONSE
        self.send_response(429 if case == 'error' else 200)
        self.send_header('content-type', 'text/event-stream' if body.get('stream') else 'application/json')
        self.send_header('request-id', 'synthetic-native-request')
        self.send_header('retry-after', '7')
        self.send_header('content-length', str(len(wire)))
        self.end_headers()
        self.wfile.write(wire)

RUST = r'''
use std::{collections::BTreeMap,sync::{Arc,Mutex}};
use velune_ai::{messages::*,http::Header,ids::*,Payload};
use velune_ai_provider::{config::*,messages::Messages};
use velune_gateway::{config::*,runtime::*};
use serde_json::{json,Value};
struct Auth;
impl CredentialResolver for Auth {
 fn resolve(&self,_:String,target:CredentialTarget)->velune_ai::OperationFuture<Result<ResolvedCredential,CredentialResolutionError>> {
  assert_eq!(target.protocol,GatewayProtocol::MessagesV1);
  Box::pin(async {Ok(ResolvedCredential{token:"SYNTHETIC_UPSTREAM_KEY".into(),explicit_output_cap:None,subscription:false})})
 }
}
fn headers()->Vec<Header>{ vec![Header{name:"anthropic-version".into(),value:"2099-01-01".into()},Header{name:"anthropic-beta".into(),value:"synthetic-beta".into()},Header{name:"authorization".into(),value:"Bearer CALLER_NOT_FORWARDED".into()},Header{name:"x-api-key".into(),value:"CALLER_NOT_FORWARDED".into()}] }
fn body(model:&str,case:&str,stream:bool)->Value {json!({"model":model,"stream":stream,"max_tokens":64,"system":[{"type":"text","text":"native system","cache_control":{"type":"ephemeral"}}],"messages":[{"role":"user","content":[{"type":"tool_result","tool_use_id":"before","content":"native"}]}],"tools":[{"name":"read","input_schema":{"type":"object"}}],"thinking":{"type":"enabled","budget_tokens":32},"metadata":{"case":case},"unknown_future":{"retained":true}})}
#[tokio::main(flavor="current_thread")]
async fn main(){
 let endpoint=std::env::args().nth(1).unwrap();
 let port: u16=endpoint.rsplit(':').next().unwrap().parse().unwrap();
 let client=reqwest::Client::builder().redirect(reqwest::redirect::Policy::none()).retry(reqwest::retry::never()).build().unwrap();
 let config=ProviderConfig::new(ProviderId::new("native").unwrap(),ConfigRevision::new(1).unwrap(),ProtocolConfig::Messages(MessagesConfig{endpoint:HttpEndpoint::new(Transport::Http,"127.0.0.1",port,"/").unwrap()}),CredentialRef::new("native-key").unwrap()).unwrap();
 let adapter=Messages::with_resolved_credential(config.clone(),client.clone(),ResolvedCredential{token:"SYNTHETIC_UPSTREAM_KEY".into(),explicit_output_cap:None,subscription:false}).unwrap();
 let service=DirectMessagesService::new(Arc::new(MessagesBinding::new(ProviderId::new("native").unwrap(),vec![ProviderModelId::new("actual-api-model").unwrap()],Arc::new(adapter)).unwrap()));
 for (case,stream) in [("json",false),("stream",true),("error",false),("stream-error",true)] {
  let wire=Arc::new(Mutex::new(Vec::new()));let sink=wire.clone();
  let result=service.messages(MessagesRequest{model:ProviderModelId::new("actual-api-model").unwrap(),body:Payload::new(body("actual-api-model",case,stream)),headers:headers(),stream},Box::new(move|e|{let sink=sink.clone();Box::pin(async move{if let MessagesEvent::Body(bytes)=e{sink.lock().unwrap().extend(bytes.into_inner());}Ok(())})})).await;
  match result.result {Ok(MessagesOutput::Json(response))=> {assert_eq!(response.meta.status,200);std::fs::write(format!("direct-{case}"),response.body.into_inner()).unwrap();},Ok(MessagesOutput::Stream(meta))=>{assert_eq!(meta.status,200);std::fs::write(format!("direct-{case}"),wire.lock().unwrap().as_slice()).unwrap();},Err(e)=>{assert_eq!(case,"error");assert_eq!(e.response.unwrap().status,429);std::fs::write("direct-error",e.body.unwrap().into_inner()).unwrap();}}
 }
 let bad=service.messages(MessagesRequest{model:ProviderModelId::new("actual-api-model").unwrap(),body:Payload::new(body("actual-api-model","json",false)),headers:vec![],stream:false},Box::new(|_|Box::pin(async{Ok(())}))).await;
 assert!(matches!(bad.result,Err(MessagesFailure{kind:MessagesErrorKind::InvalidInput,submitted:false,..})));
 assert!(Messages::with_resolved_credential(config,client.clone(),ResolvedCredential{token:"SYNTHETIC".into(),explicit_output_cap:None,subscription:true}).is_err());
 let gateway=Runner::start(GatewayConfig{id:"g".into(),name:"g".into(),providers:vec![ProviderDefinition{id:"p".into(),name:"p".into(),protocol:GatewayProtocol::MessagesV1,endpoint: endpoint.clone(),credential_ref:Some("key".into()),models:vec![ProviderModel{record_key:"record".into(),provider_model_id:"actual-api-model".into(),nickname:"".into(),icon:None,context_window:None,max_output_tokens:None,reasoning_levels:None}]}],failover:FailoverPolicy{mode:FailoverMode::Disabled}},Arc::new(Auth),BTreeMap::from([("velune/auto".into(),"record".into())])).unwrap();
 for (case,stream) in [("json",false),("stream",true),("error",false),("stream-error",true)] {
  let response=client.post(format!("{}/messages",gateway.endpoint())).header("x-api-key",gateway.token()).header("anthropic-version","2099-01-01").header("anthropic-beta","synthetic-beta").json(&body("velune/auto",case,stream)).send().await.unwrap();
  assert_eq!(response.status().as_u16(),if case=="error"{429}else{200}); assert_eq!(response.headers()["request-id"],"synthetic-native-request");assert_eq!(response.headers()["retry-after"],"7");
  std::fs::write(format!("gateway-{case}"),response.bytes().await.unwrap()).unwrap();
 }
 let forbidden=client.post(format!("{}/messages",gateway.endpoint())).header("x-api-key","WRONG").json(&body("velune/auto","json",false)).send().await.unwrap();assert_eq!(forbidden.status(),401);
 let wrong=client.post(format!("{}/responses",gateway.endpoint())).bearer_auth(gateway.token()).json(&body("velune/auto","json",false)).send().await.unwrap();assert_eq!(wrong.status(),400);
 println!("native Messages direct/gateway passed");
}
'''

def main():
    server = ThreadingHTTPServer(('127.0.0.1', 0), Handler)
    thread = threading.Thread(target=server.serve_forever, daemon=True)
    thread.start()
    try:
        with tempfile.TemporaryDirectory(prefix='velune-messages-manual-') as raw:
            root = Path(raw)
            (root / 'src').mkdir()
            (root / 'src/main.rs').write_text(RUST)
            dependencies = '\n'.join(f'{name} = {{ path = {json.dumps(str(ROOT / "packages" / directory))} }}' for name, directory in [('velune-ai','ai'),('velune-ai-provider','ai-provider'),('velune-gateway','gateway')])
            (root / 'Cargo.toml').write_text('[package]\nname="velune-messages-manual"\nversion="0.0.0"\nedition="2024"\n[dependencies]\n' + dependencies + '\nserde_json="1"\ntokio={version="1",features=["rt","macros"]}\nreqwest={version="0.13",default-features=false,features=["json","native-tls"]}\n')
            subprocess.run(['cargo','run','--offline','--quiet','--manifest-path',str(root/'Cargo.toml'),'--target-dir',str(ROOT/'target/manual-messages'),'--',f'http://127.0.0.1:{server.server_port}'], cwd=root, check=True)
            for prefix in ['direct','gateway']:
                for case, expected in [('json',RESPONSE),('stream',STREAM),('error',ERROR),('stream-error',STREAM_ERROR)]:
                    assert (root/f'{prefix}-{case}').read_bytes() == expected, (prefix,case)
            assert len(requests) == 8, len(requests)
            original = requests[0][2].copy()
            assert all(record[2] == {**original,'stream':record[2]['stream'],'metadata':record[2]['metadata']} for record in requests)
            print('PASS: 8 loopback requests; native JSON/SSE/text/tool/thinking/errors/headers preserved; version missing and subscription rejected before dispatch.')
    finally:
        server.shutdown()
        server.server_close()

if __name__ == '__main__':
    main()
