#!/usr/bin/env python3
"""Explicit loopback gateway → original-protocol facts → application SQLite check.
Runs only when invoked; temporary data and synthetic credentials, never CI.
"""
import argparse
import shutil
import json
from pathlib import Path
import subprocess
import tempfile
import threading
import time
from http.server import BaseHTTPRequestHandler, ThreadingHTTPServer

ROOT = Path(__file__).resolve().parents[1]
CAPTURE = {}

def event(value):
    return 'data: ' + json.dumps(value, ensure_ascii=False) + '\r\n\r\n'

def fixture(case):
    usage = {'prompt_tokens': 10, 'completion_tokens': 5,
             'prompt_tokens_details': {'cached_tokens': 4}, 'completion_tokens_details': {'reasoning_tokens': 2}}
    chat = {'id': 'synthetic', 'object': 'chat.completion', 'model': case,
            'choices': [{'index': 0, 'message': {'role': 'assistant', 'content': '你好'}, 'finish_reason': 'stop'}], 'usage': usage}
    responses = {'id': 'synthetic', 'object': 'response', 'status': 'completed', 'model': case,
                 'output': [{'type': 'message', 'role': 'assistant', 'content': [{'type': 'output_text', 'text': '你好'}]}],
                 'usage': {'input_tokens': 20, 'output_tokens': 7, 'input_tokens_details': {'cached_tokens': 6}, 'output_tokens_details': {'reasoning_tokens': 3}}}
    messages = {'id': 'synthetic', 'type': 'message', 'role': 'assistant', 'model': case,
                'content': [{'type': 'text', 'text': '你好'}], 'stop_reason': 'end_turn',
                'usage': {'input_tokens': 5, 'cache_read_input_tokens': 3, 'cache_creation_input_tokens': 2, 'output_tokens': 6}}
    if case == 'messages-uncached-json':
        messages['usage'] = {'input_tokens': 20, 'output_tokens': 3}
    if case == 'messages-zero-json':
        messages['usage'] = {'input_tokens': 0, 'cache_read_input_tokens': 0, 'cache_creation_input_tokens': 0, 'output_tokens': 0}
    if case in ('chat-missing', 'translate-unknown'):
        del chat['usage']
        return 200, 'application/json', json.dumps(chat, ensure_ascii=False).encode()
    if case == 'chat-error':
        return 429, 'application/json', json.dumps({'error': {'message': 'synthetic rate limit'}, 'usage': {'prompt_tokens': 2, 'completion_tokens': 1}}).encode()
    if case.endswith('-json') or case == 'translate-messages':
        value = responses if case.startswith('responses') else messages if case.startswith('messages') or case == 'translate-messages' else chat
        return 200, 'application/json', json.dumps(value, ensure_ascii=False).encode()
    if case.startswith('chat'):
        chunks = ': heartbeat\r\n\r\n' + event({'choices': [{'index': 0, 'delta': {'role': 'assistant'}, 'finish_reason': None}]})
        chunks += event({'choices': [{'index': 0, 'delta': {'content': '你好'}, 'finish_reason': None}], 'usage': {'prompt_tokens': 10, 'completion_tokens': 2}})
        if case not in ('chat-truncated', 'chat-cancel'):
            chunks += event({'choices': [{'index': 0, 'delta': {}, 'finish_reason': 'stop'}]})
            chunks += event({'choices': [], 'usage': usage}) * 2 + 'data: [DONE]\r\n\r\n'
    elif case.startswith('responses'):
        chunks = event({'type': 'response.created', 'response': {'id': 'synthetic', 'status': 'in_progress'}})
        chunks += event({'type': 'response.output_text.delta', 'delta': '你好'})
        chunks += event({'type': 'response.completed', 'response': responses})
    else:
        initial = dict(messages); initial['usage'] = dict(messages['usage'], output_tokens=0)
        chunks = event({'type': 'message_start', 'message': initial})
        chunks += event({'type': 'content_block_delta', 'index': 0, 'delta': {'type': 'text_delta', 'text': '你好'}})
        chunks += event({'type': 'message_delta', 'delta': {'stop_reason': 'end_turn'}, 'usage': {'output_tokens': 6}}) * 2
        chunks += event({'type': 'message_stop'})
    return 200, 'text/event-stream', chunks.encode()

class Handler(BaseHTTPRequestHandler):
    protocol_version = 'HTTP/1.1'
    def log_message(self, *_): pass
    def do_POST(self):
        body = json.loads(self.rfile.read(int(self.headers['Content-Length'])))
        case = body['model']
        status, content_type, payload = fixture(case)
        CAPTURE[case] = payload
        self.send_response(status)
        self.send_header('Content-Type', content_type)
        self.send_header('Content-Length', str(len(payload) + (1000 if case == 'chat-cancel' else 0)))
        self.end_headers()
        try:
            # Fragment across UTF-8 and SSE delimiters, not just whole JSON lines.
            for i in range(0, len(payload), 3):
                self.wfile.write(payload[i:i+3]); self.wfile.flush()
                time.sleep(0.0003)
            if case == 'chat-cancel': time.sleep(1)
        except (BrokenPipeError, ConnectionResetError): pass

RUST = r'''
use std::{collections::BTreeMap, sync::Arc, path::Path};
use futures_util::StreamExt;
use velune_gateway::*;
use velune_application::Error;
#[allow(dead_code)]
#[path="STORE_SOURCE"] mod storage;
struct Auth;
impl CredentialResolver for Auth {
 fn resolve(&self,_:String,_:CredentialTarget)->velune_ai::OperationFuture<Result<ResolvedCredential,CredentialResolutionError>> {
 Box::pin(async { Ok(ResolvedCredential { token:"SYNTHETIC".into(), explicit_output_cap:None, subscription:false }) }) }
}
fn provider(protocol:GatewayProtocol,cases:&[&str],endpoint:&str)->ProviderDefinition {
 ProviderDefinition { id:format!("{protocol:?}"),name:format!("Synthetic {protocol:?}"),protocol,endpoint:endpoint.into(),credential_ref:Some("synthetic".into()),models:cases.iter().map(|case|ProviderModel{record_key:case.to_string(),provider_model_id:case.to_string(),nickname:String::new(),icon:None,context_window:None,max_output_tokens:Some(100),reasoning_levels:None}).collect() }
}
#[tokio::main(flavor="current_thread")]
async fn main(){
 let args:Vec<String>=std::env::args().collect();let home=Path::new(&args[2]);
 let store=Arc::new(storage::AnalyticsStore::open(home).unwrap());
 let gateway=Runner::start_with_analytics(GatewayConfig{id:"g".into(),name:"Synthetic".into(),providers:vec![
 provider(GatewayProtocol::ChatCompletionsV1,&["chat-json","chat-sse","chat-missing","chat-truncated","chat-error","chat-cancel","translate-unknown"],&args[1]),
 provider(GatewayProtocol::ResponsesV1,&["responses-json","responses-sse"],&args[1]),
 provider(GatewayProtocol::MessagesV1,&["messages-json","messages-sse","translate-messages","messages-uncached-json","messages-zero-json"],&args[1])],failover:FailoverPolicy{mode:FailoverMode::Disabled}},Arc::new(Auth),BTreeMap::new(),Some(store.clone())).unwrap();
 let client=reqwest::Client::new();
 let cases=["chat-json","chat-sse","chat-missing","chat-truncated","chat-error","chat-cancel","responses-json","responses-sse","messages-json","messages-sse","translate-unknown","translate-messages","messages-uncached-json","messages-zero-json"];
 for case in cases {
  let ingress=if case.starts_with("messages") || case=="translate-unknown" {"messages"} else if case.starts_with("responses") {"responses"} else {"chat/completions"};
  let streaming=case.ends_with("sse") || case.ends_with("truncated") || case.ends_with("cancel");
  let body=if ingress=="responses" {serde_json::json!({"model":format!("velune/model/{case}"),"input":"Synthetic","stream":streaming})} else {serde_json::json!({"model":format!("velune/model/{case}"),"messages":[{"role":"user","content":"Synthetic"}],"max_tokens":100,"stream":streaming})};
  let response=client.post(format!("{}/{ingress}",gateway.endpoint())).bearer_auth(gateway.token()).header("anthropic-version","2023-06-01").json(&body).send().await.unwrap();
  assert_eq!(response.status().as_u16(),if case=="chat-error"{429}else{200},"{case}");
  if case=="chat-cancel" {let mut stream=response.bytes_stream();let mut seen=Vec::new();while let Some(Ok(chunk))=stream.next().await {seen.extend(chunk);if String::from_utf8_lossy(&seen).contains("completion_tokens") && seen.ends_with(b"\r\n\r\n") {break;}}drop(stream);tokio::time::sleep(std::time::Duration::from_millis(30)).await;}
  else {std::fs::write(home.join(format!("wire-{case}")),response.bytes().await.unwrap()).unwrap();}
 }
 drop(gateway);
 let store=Arc::try_unwrap(store).ok().expect("gateway leaked analytics sink");drop(store);
 let reopened=storage::AnalyticsStore::open(home).unwrap();
 let mut rows=Vec::new(); reopened.visit(0,i64::MAX,None,None,|row|{rows.push(row);Ok(())}).unwrap();assert_eq!(rows.len(),14);
 for row in &rows {
  match row.provider_model_id.as_str() {
   "chat-json"|"chat-sse" => {assert_eq!(row.input_tokens,Some(10));assert_eq!(row.output_tokens,Some(5));assert_eq!(row.cached_input_tokens,Some(4));assert_eq!(row.reasoning_output_tokens,Some(2));assert!(row.terminal_elapsed_ms.is_some());},
   "responses-json"|"responses-sse" => {assert_eq!(row.input_tokens,Some(20));assert_eq!(row.output_tokens,Some(7));},
   "messages-json"|"messages-sse"|"translate-messages" => {assert_eq!(row.input_tokens,Some(10));assert_eq!(row.output_tokens,Some(6));assert_eq!(row.cache_read_input_tokens,Some(3));assert_eq!(row.cache_creation_input_tokens,Some(2));},
   "messages-uncached-json" => {assert_eq!(row.input_tokens,None);assert_eq!(row.uncached_input_tokens,Some(20));assert_eq!(row.output_tokens,Some(3));},
   "messages-zero-json" => {assert_eq!(row.input_tokens,Some(0));assert_eq!(row.output_tokens,Some(0));},
   "chat-missing"|"translate-unknown" => {assert_eq!(row.input_tokens,None);assert_eq!(row.output_tokens,None);},
   "chat-error" => {assert_eq!(row.status,Some(429));assert_eq!(row.outcome,"failed");assert_eq!(row.output_tokens,Some(1));},
   "chat-truncated" => {assert_eq!(row.outcome,"incomplete");assert_eq!(row.terminal_elapsed_ms,None);assert_eq!(row.output_tokens,Some(2));},
   "chat-cancel" => {assert_eq!(row.outcome,"cancelled");assert_eq!(row.output_tokens,Some(2));},
   _=>panic!("unexpected model")
  }
  if row.provider_model_id.ends_with("-json") {assert_eq!(row.first_output_ms,None);}
  if row.provider_model_id.ends_with("sse") {assert!(row.first_output_ms.is_some());}
 }
 println!("PASS 14 original-protocol gateway → SQLite records, fragmented UTF-8/SSE, cumulative usage, conversion, errors/cancel, reopen");
}
'''

def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--fixture-out", type=Path, help="Optional explicit copy of the synthetic SQLite for downstream manual checks")
    args = parser.parse_args()
    server = ThreadingHTTPServer(('127.0.0.1', 0), Handler)
    threading.Thread(target=server.serve_forever, daemon=True).start()
    try:
        with tempfile.TemporaryDirectory(prefix='velune-analytics-manual-') as directory:
            root = Path(directory); (root/'src').mkdir(); (root/'home').mkdir()
            (root/'src/main.rs').write_text(RUST.replace('STORE_SOURCE',str(ROOT/'packages/application/src/analytics.rs')))
            dependencies = '\n'.join(f'{name}={{path="{ROOT / "packages" / folder}"}}' for name,folder in [('velune-gateway','gateway'),('velune-ai','ai'),('velune-application','application')])
            (root/'Cargo.toml').write_text('[package]\nname="manual-gateway-analytics"\nversion="0.0.0"\nedition="2024"\n[dependencies]\n'+dependencies+'\nrusqlite={version="0.37",features=["bundled"]}\ntracing="0.1"\nreqwest={version="0.13",default-features=false,features=["json","stream"]}\ntokio={version="1",features=["macros","rt","time"]}\nfutures-util="0.3"\nserde_json="1"\n')
            subprocess.run(['cargo','run','--offline','--manifest-path',str(root/'Cargo.toml'),'--target-dir',str(ROOT/'target/manual-analytics'),'--',f'http://127.0.0.1:{server.server_port}',str(root/'home')],check=True,cwd=ROOT)
            for case,payload in CAPTURE.items():
                if case in ('translate-unknown','translate-messages','chat-cancel'): continue
                assert (root/'home'/f'wire-{case}').read_bytes()==payload, f'native body changed: {case}'
            print('PASS native response bytes unchanged; no real upstream calls')
            if args.fixture_out:
                args.fixture_out.parent.mkdir(parents=True, exist_ok=True)
                shutil.copy2(root/'home/analytics.sqlite', args.fixture_out)
                print('Synthetic downstream fixture:', args.fixture_out.resolve())
    finally:
        server.shutdown(); server.server_close()

if __name__ == '__main__': main()
