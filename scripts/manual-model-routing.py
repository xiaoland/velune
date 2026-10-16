#!/usr/bin/env python3
"""Manual acceptance for model routes and their fail-over; never CI.

Phase 1 drives the gateway directly against a loopback upstream that answers
by bearer token: rate limits, server errors, refused accounts, connections
that fail, streams that break after they start, requests that refer to
upstream state, a member whose credential cannot be resolved, and a request
converted from another protocol. Phase 2 runs real Codex 0.159.3 and Pi 1.0.2
in temporary HOMEs through the application: two synthetic "accounts" with the
same model, the first answering 429, form a route that the runtimes select
like a model; the reply comes from the second account, the first is not asked
again while it cools down, and removing providers prunes the route.
Synthetic keys only; no real HOME, credential or model is used.
"""
import argparse
import importlib.util
import json
import os
from pathlib import Path
import socket
import subprocess
import sys
import tempfile
import threading
import time
from http.server import BaseHTTPRequestHandler, ThreadingHTTPServer

ROOT = Path(__file__).resolve().parents[1]
HITS = {}
LOCK = threading.Lock()


def event(value):
    return 'event: %s\ndata: %s\n\n' % (value['type'], json.dumps(value))


def reply_events(text, index):
    item = {'id': 'item-%d' % index, 'type': 'message', 'role': 'assistant', 'status': 'completed',
            'content': [{'type': 'output_text', 'text': text, 'annotations': []}]}
    response = {'id': 'resp-%d' % index, 'object': 'response', 'status': 'completed', 'output': [item],
                'usage': {'input_tokens': 10, 'output_tokens': 3, 'total_tokens': 13}}
    return ''.join(event(e) for e in [
        {'type': 'response.created', 'response': {**response, 'status': 'in_progress', 'output': []}},
        {'type': 'response.output_item.added', 'output_index': 0, 'item': {**item, 'status': 'in_progress', 'content': []}},
        {'type': 'response.output_text.delta', 'output_index': 0, 'content_index': 0, 'item_id': item['id'], 'delta': text},
        {'type': 'response.output_item.done', 'output_index': 0, 'item': item},
        {'type': 'response.completed', 'response': response}])


class Upstream(BaseHTTPRequestHandler):
    protocol_version = 'HTTP/1.1'

    def log_message(self, *_):
        pass

    def do_POST(self):
        self.rfile.read(int(self.headers.get('Content-Length', '0')))
        token = self.headers.get('Authorization', '').removeprefix('Bearer ')
        with LOCK:
            HITS[token] = HITS.get(token, 0) + 1
            index = sum(HITS.values())
        status = token.split('-')[0]
        if status in ('429', '503', '401'):
            payload = json.dumps({'error': {'message': 'synthetic %s' % status}}).encode()
            self.send_response(int(status))
            self.send_header('Content-Type', 'application/json')
            if status == '429':
                self.send_header('Retry-After', '120')
            self.send_header('Content-Length', str(len(payload)))
            self.end_headers()
            self.wfile.write(payload)
            return
        self.send_response(200)
        self.send_header('Content-Type', 'text/event-stream')
        self.send_header('Connection', 'close')
        self.end_headers()
        if status == 'mid':
            # Headers and one event reach the caller, then the stream breaks.
            self.wfile.write(event({'type': 'response.created', 'response': {'id': 'r', 'status': 'in_progress', 'output': []}}).encode())
            self.wfile.flush()
            self.connection.shutdown(socket.SHUT_RDWR)
            return
        self.wfile.write(reply_events('SERVED_BY:' + token, index).encode())
        self.wfile.flush()


GATEWAY_PROGRAM = r'''
use std::{collections::BTreeMap, sync::{Arc, Mutex}};
use velune_gateway::*;
struct Resolver;
impl CredentialResolver for Resolver {
 fn resolve(&self,reference:String,_:CredentialTarget)->velune_ai::OperationFuture<Result<ResolvedCredential,CredentialResolutionError>> {
  Box::pin(async move { if reference=="credfail" {Err(CredentialResolutionError::Unavailable)} else {Ok(ResolvedCredential{token:reference,explicit_output_cap:None,subscription:false})} })
 }
}
#[derive(Default)] struct Sink(Mutex<Vec<AnalyticsRecord>>);
impl AnalyticsSink for Sink { fn record(&self,record:AnalyticsRecord){self.0.lock().unwrap().push(record);} }
fn provider(id:&str,endpoint:&str)->ProviderDefinition {
 ProviderDefinition{id:id.into(),name:id.into(),protocol:GatewayProtocol::ResponsesV1,endpoint:endpoint.into(),credential_ref:Some(id.into()),
  models:vec![ProviderModel{record_key:id.into(),provider_model_id:"synthetic-model".into(),nickname:String::new(),icon:None,context_window:None,max_output_tokens:Some(256),reasoning_levels:None}]}
}
fn route(key:&str,members:&[&str])->ModelRoute { ModelRoute{record_key:key.into(),name:key.into(),members:members.iter().map(|m|m.to_string()).collect()} }
async fn call(client:&reqwest::Client,runner:&Runner,ingress:&str,model:&str,extra:serde_json::Value)->(u16,Result<String,String>) {
 let mut body=if ingress=="chat/completions" {serde_json::json!({"model":format!("velune/model/{model}"),"messages":[{"role":"user","content":"hi"}],"stream":true})}
  else {serde_json::json!({"model":format!("velune/model/{model}"),"input":"hi","stream":true,"store":false})};
 for (k,v) in extra.as_object().unwrap() { body[k]=v.clone(); }
 let response=client.post(format!("{}/{ingress}",runner.endpoint())).bearer_auth(runner.token()).json(&body).send().await.unwrap();
 let status=response.status().as_u16();
 (status,response.text().await.map_err(|e|e.to_string()))
}
#[tokio::main(flavor="current_thread")]
async fn main(){
 let args:Vec<String>=std::env::args().collect();let (up,dead)=(&args[1],&args[2]);
 let ids=["429-a","ok-b","503-c","ok-d","401-e","ok-f","ok-h","mid-i","ok-j","429-k","ok-l","503-m","503-n","ok-o","429-p","ok-q"];
 let mut providers:Vec<_>=ids.iter().map(|id|provider(id,up)).collect();
 providers.push(provider("dead-g",dead)); providers.push(provider("credfail",up));
 let config=GatewayConfig{id:"g".into(),name:"g".into(),providers,routes:vec![
  route("rate",&["429-a","ok-b"]),route("server",&["503-c","ok-d"]),route("refused",&["401-e","ok-f"]),route("connect",&["dead-g","ok-h"]),
  route("midstream",&["mid-i","ok-j"]),route("stateful",&["429-k","ok-l"]),route("exhausted",&["503-m","503-n"]),route("credential",&["credfail","ok-o"]),
  route("converted",&["429-p","ok-q"])],failover:FailoverPolicy{mode:FailoverMode::Disabled}};
 let mut mixed=config.clone();
 mixed.providers.push(ProviderDefinition{protocol:GatewayProtocol::ChatCompletionsV1,..provider("chat-x",up)});
 mixed.routes.push(route("mixed",&["ok-b","chat-x"]));
 assert_eq!(mixed.validate(),Err("model route members must share a protocol"));
 let mut duplicate=config.clone(); duplicate.routes.push(route("ok-b",&["ok-d"]));
 assert!(duplicate.validate().is_err(),"route key colliding with a model key");
 let sink=Arc::new(Sink::default());
 let runner=Runner::start_with_analytics(config,Arc::new(Resolver),BTreeMap::new(),Some(sink.clone()),RouteHealth::default()).unwrap();
 let client=reqwest::Client::builder().no_proxy().build().unwrap();
 let mut report=serde_json::Map::new();
 for (name,ingress,model,extra) in [
  ("rate","responses","rate",serde_json::json!({})),("rate_again","responses","rate",serde_json::json!({})),
  ("server","responses","server",serde_json::json!({})),("refused","responses","refused",serde_json::json!({})),
  ("connect","responses","connect",serde_json::json!({})),("midstream","responses","midstream",serde_json::json!({})),
  ("stateful","responses","stateful",serde_json::json!({"previous_response_id":"resp-elsewhere"})),
  ("direct","responses","503-c",serde_json::json!({})),("exhausted","responses","exhausted",serde_json::json!({})),
  ("credential","responses","credential",serde_json::json!({})),("converted","chat/completions","converted",serde_json::json!({}))] {
  let (status,body)=call(&client,&runner,ingress,model,extra).await;
  report.insert(name.into(),serde_json::json!({"status":status,"body":body.as_ref().ok(),"bodyError":body.as_ref().err().is_some()}));
 }
 drop(runner);
 let records:Vec<_>=sink.0.lock().unwrap().iter().map(|r|serde_json::json!({"request":r.request_id,"provider":r.provider_id,"status":r.status,"outcome":format!("{:?}",r.outcome)})).collect();
 report.insert("analytics".into(),serde_json::Value::Array(records));
 println!("{}",serde_json::Value::Object(report));
}
'''


def check(condition, message):
    if not condition:
        raise SystemExit('FAIL: ' + message)


def gateway_phase(upstream, dead):
    with tempfile.TemporaryDirectory(prefix='velune-routing-gateway-') as directory:
        root = Path(directory)
        (root / 'src').mkdir()
        (root / 'src/main.rs').write_text(GATEWAY_PROGRAM)
        dependencies = '\n'.join('%s={path="%s"}' % (name, ROOT / 'packages' / folder) for name, folder in [('velune-gateway', 'gateway'), ('velune-ai', 'ai')])
        (root / 'Cargo.toml').write_text('[package]\nname="manual-model-routing"\nversion="0.0.0"\nedition="2024"\n[dependencies]\n' + dependencies +
            '\nreqwest={version="0.13",default-features=false,features=["json"]}\ntokio={version="1",features=["macros","rt"]}\nserde_json="1"\n')
        output = subprocess.run(['cargo', 'run', '--quiet', '--offline', '--manifest-path', str(root / 'Cargo.toml'), '--target-dir', str(ROOT / 'target/manual-routing'),
                                 '--', upstream, dead], check=True, capture_output=True, text=True, cwd=ROOT).stdout
    report = json.loads(output.strip().splitlines()[-1])
    served = lambda name: report[name]['status'] == 200 and report[name]['body'] or ''
    check('SERVED_BY:ok-b' in served('rate'), 'rate limit fails over: %s' % report['rate'])
    check('SERVED_BY:ok-b' in served('rate_again') and HITS.get('429-a') == 1, 'rate-limited provider cools down: %s' % HITS)
    check('SERVED_BY:ok-d' in served('server'), 'server error fails over')
    check(report['refused']['status'] == 401 and 'ok-f' not in HITS, 'refused account stops')
    check('SERVED_BY:ok-h' in served('connect'), 'failed connection fails over')
    check(report['midstream']['status'] == 200 and 'SERVED_BY' not in (report['midstream']['body'] or '') and 'ok-j' not in HITS, 'started stream is not switched: %s' % report['midstream'])
    check(report['stateful']['status'] == 429 and 'ok-l' not in HITS, 'stateful request keeps its account')
    check(report['direct']['status'] == 503 and HITS.get('503-c') == 2, 'direct model does not fail over')
    check(report['exhausted']['status'] == 503 and HITS.get('503-m') == 1 and HITS.get('503-n') == 1, 'last failure returned')
    check('SERVED_BY:ok-o' in served('credential'), 'unresolved credential fails over')
    check('SERVED_BY:ok-q' in served('converted') and 'chat.completion' in served('converted'), 'converted request fails over: %s' % report['converted'])
    rate = [r for r in report['analytics'] if r['provider'] in ('429-a', 'ok-b')]
    first = next(r for r in rate if r['provider'] == '429-a')
    second = [r for r in rate if r['provider'] == 'ok-b' and r['request'].startswith(first['request'] + '/')]
    check(first['status'] == 429 and first['outcome'] == 'Failed' and len(second) == 1 and second[0]['request'] == first['request'] + '/2'
          and second[0]['outcome'] == 'Completed', 'one analytics record per attempt: %s' % rate)
    return {'cases': sorted(k for k in report if k != 'analytics'), 'upstreamHits': dict(HITS)}


def application_phase(args, endpoint):
    HITS.clear()
    spec = importlib.util.spec_from_file_location('velune_model_routing', args.bindings / 'velune_bindings.py')
    b = importlib.util.module_from_spec(spec)
    sys.modules[spec.name] = b
    spec.loader.exec_module(b)
    with tempfile.TemporaryDirectory(prefix='velune-routing-app-') as directory:
        root = Path(directory).resolve()
        (root / 'project').mkdir()
        (root / 'pi').mkdir()
        app = b.VeluneApplication.open(b.BindingOptions(home_directory=str(root / 'velune'), resources_directory=str(args.bundle / 'Contents/Resources')))
        try:
            managed = b.BindingRuntimeExecution(transport=b.BindingRuntimeTransport.LOCAL(), home=b.BindingRuntimeHome.MANAGED(remote_root=None), remote_resources_directory=None)
            app.upsert_runtime_with_execution(b.BindingRuntimeInstance(enabled=True, id='codex', name='Codex', type_id='codex-0.159.3', gateway_id='default',
                settings={'binary': str(args.codex.resolve()), 'nodeBinary': str(args.node)}), managed)
            app.upsert_runtime(b.BindingRuntimeInstance(enabled=True, id='pi', name='Pi', type_id='pi-1.0.2', gateway_id='default',
                settings={'binary': str(args.pi), 'nodeBinary': str(args.node), 'agentDir': str(root / 'pi')}))

            def provider(identifier, protocol, key):
                app.save_provider('default', b.BindingProviderDraft(id=identifier, name=identifier, protocol=protocol, endpoint=endpoint,
                    models=[b.BindingProviderModel(record_key='', provider_model_id='synthetic-model', nickname='Synthetic', icon=None,
                        context_window=32768, max_output_tokens=1024, reasoning_levels=None, adapter_metadata_json=None)]),
                    b.BindingAuthenticationEdit.SET_API_KEY(value=key))
                return next(p for p in app.list().gateways[0].providers if p.id == identifier).models[0].record_key

            first = provider('account-a', b.BindingGatewayProtocol.RESPONSES_V1, '429-account-a')
            second = provider('account-b', b.BindingGatewayProtocol.RESPONSES_V1, 'ok-account-b')
            chat = provider('chat', b.BindingGatewayProtocol.CHAT_COMPLETIONS_V1, 'ok-chat')
            try:
                app.save_model_route('default', b.BindingModelRoute(record_key='', name='Mixed', members=[first, chat]))
                check(False, 'mixed-protocol route accepted')
            except b.BindingError as error:
                check('同一协议' in str(error), 'mixed-protocol message: %s' % error)
            update = app.save_model_route('default', b.BindingModelRoute(record_key='', name='订阅池', members=[first, second]))
            route = update.gateways[0].routes[0]
            check(route.name == '订阅池' and route.members == [first, second], 'saved route')
            for runtime in ('codex', 'pi'):
                names = [m.name for m in app.runtime_models(runtime)]
                check('订阅池 · 模型路由' in names, '%s lists the route: %s' % (runtime, names))

            replies = {}
            for runtime in ('codex', 'pi'):
                conversation = app.create_conversation(runtime, str(root / 'project'), route.record_key).snapshot.conversation.id
                for text in ('first turn', 'second turn'):
                    conversation = app.send_turn_with_selection(runtime, conversation, runtime, b.BindingModelSelection.MANAGED(model_record_key=route.record_key),
                        b.BindingComposedTurn(text=text, effort=None, permission_mode=None, attachments=[])).snapshot.conversation.id
                    deadline = time.monotonic() + 90
                    while True:
                        current = app.snapshot(runtime, conversation).snapshot
                        check(current.run_state != b.BindingRunState.FAILED, '%s turn failed: %s' % (runtime, [str(x) for m in current.messages for x in m.blocks]))
                        said = [block.text for message in current.messages[-1:] for block in message.blocks if isinstance(block, b.BindingMessageBlock.TEXT)]
                        if current.run_state == b.BindingRunState.IDLE and any('SERVED_BY:ok-account-b' in s for s in said):
                            replies[runtime + ' ' + text] = said
                            break
                        check(time.monotonic() < deadline, '%s turn timed out: %s' % (runtime, current.run_state))
                        time.sleep(.1)
                    models = [t.settings.model for t in current.transcript_turns if t.settings]
                    check('订阅池 · 模型路由' in models, '%s names the turn by the route: %s' % (runtime, models))
            # One 429 for the whole run: later requests go to the cooled-down account last.
            check(HITS.get('429-account-a') == 1, 'first account asked once: %s' % HITS)
            check(HITS.get('ok-account-b', 0) >= 4, 'second account served every turn: %s' % HITS)
            app.delete_provider('default', 'account-b')
            check([r.members for r in app.list().gateways[0].routes] == [[first]], 'deleting a member prunes the route')
            app.delete_provider('default', 'account-a')
            check(app.list().gateways[0].routes == [], 'a route without members is removed')
            saved = json.loads((root / 'velune' / 'generic-config.json').read_text())
            check(saved['schemaVersion'] == 8 and 'routes' not in saved['gateways'][0], 'empty routes are not written')
            return {'replies': replies, 'upstreamHits': dict(HITS)}
        finally:
            app.shutdown()


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    for name in ('bundle', 'bindings', 'node', 'codex', 'pi'):
        parser.add_argument('--' + name, type=Path, required=True)
    args = parser.parse_args()
    for path in vars(args).values():
        if not path.is_absolute() or not path.exists():
            parser.error('absolute existing paths required')
    server = ThreadingHTTPServer(('127.0.0.1', 0), Upstream)
    threading.Thread(target=server.serve_forever, daemon=True).start()
    closed = socket.socket()
    closed.bind(('127.0.0.1', 0))
    dead = 'http://127.0.0.1:%d/v1' % closed.getsockname()[1]
    closed.close()
    upstream = 'http://127.0.0.1:%d/v1' % server.server_port
    environment = dict(os.environ)
    try:
        gateway = gateway_phase(upstream, dead)
        print('PASS gateway fail-over matrix', json.dumps(gateway, ensure_ascii=False))
        os.environ.update(NO_PROXY='127.0.0.1,localhost', no_proxy='127.0.0.1,localhost')
        application = application_phase(args, upstream)
        print('PASS application routes through Codex and Pi', json.dumps(application, ensure_ascii=False))
    finally:
        os.environ.clear()
        os.environ.update(environment)
        server.shutdown()
        server.server_close()


if __name__ == '__main__':
    main()
