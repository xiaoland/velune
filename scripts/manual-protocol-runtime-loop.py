#!/usr/bin/env python3
"""Explicit synthetic actual-client protocol translation/tool continuation probe; never CI."""
import argparse
import importlib.util
import json
import os
from pathlib import Path
import shutil
import subprocess
import sys
import tempfile
import threading
import time
from http.server import BaseHTTPRequestHandler, ThreadingHTTPServer


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    for name in ('bundle', 'bindings', 'node', 'pi', 'codex'):
        parser.add_argument('--' + name, type=Path, required=True)
    args = parser.parse_args()
    for path in vars(args).values():
        if not path.is_absolute() or not path.exists():
            parser.error('absolute existing paths required')
    captures, errors, reports = [], [], []
    environment = dict(os.environ)
    developer_home = Path.home()
    repository = Path(__file__).resolve().parent.parent
    with tempfile.TemporaryDirectory(prefix='velune-runtime-translation-') as directory:
        root = Path(directory)
        for name in ('home', 'bindings', 'project'):
            (root / name).mkdir()
        tool_file = root / 'project' / 'fixture.txt'
        tool_file.write_text('SYNTHETIC_TOOL_RESULT\n')

        class Upstream(BaseHTTPRequestHandler):
            def log_message(self, *_):
                pass

            def do_POST(self):
                try:
                    body = json.loads(self.rfile.read(int(self.headers['Content-Length'])))
                    captures.append({'path': self.path, 'body': body})
                    second = any(message.get('role') == 'tool' or any(isinstance(block, dict) and block.get('type') == 'tool_result' for block in (message.get('content') if isinstance(message.get('content'), list) else [])) for message in body.get('messages', []))
                    tools = body.get('tools', [])
                    names = [tool.get('function', tool).get('name', '') for tool in tools]
                    tool_name = next((name for name in names if name.endswith('exec_command')), None)
                    namespace_case = body['model'].endswith('-namespace')
                    if namespace_case:
                        tool_name = next((tool.get('function', tool).get('name') for tool in tools
                                          if tool.get('function', tool).get('description', '').startswith('Wait for agents')), None)
                        arguments = {'targets': ['00000000-0000-0000-0000-000000000001'], 'timeout_ms': 0}
                    elif tool_name:
                        arguments = {'cmd': 'cat ' + str(tool_file), 'max_output_tokens': 1000}
                    else:
                        tool_name = next((name for name in names if name == 'read'), None)
                        arguments = {'path': str(tool_file)}
                    if not second and not tool_name:
                        raise RuntimeError('no declared executable fixture tool: ' + repr(names))
                    if self.path.endswith('/chat/completions'):
                        delta = {'content': 'SYNTHETIC_FINAL'} if second else {'role': 'assistant', 'tool_calls': [
                            {'index': 0, 'id': 'call_fixture', 'type': 'function', 'function': {'name': tool_name, 'arguments': json.dumps(arguments)}}]}
                        chunks = [{'id': 'synthetic', 'object': 'chat.completion.chunk', 'created': 1, 'model': body['model'],
                                   'choices': [{'index': 0, 'delta': delta, 'finish_reason': None}]},
                                  {'id': 'synthetic', 'object': 'chat.completion.chunk', 'created': 1, 'model': body['model'],
                                   'choices': [{'index': 0, 'delta': {}, 'finish_reason': 'stop' if second else 'tool_calls'}],
                                   'usage': {'prompt_tokens': 3, 'completion_tokens': 2, 'total_tokens': 5}}]
                        payload = ''.join('data: ' + json.dumps(c) + '\n\n' for c in chunks) + 'data: [DONE]\n\n'
                    else:
                        events = [('message_start', {'type': 'message_start', 'message': {'id': 'msg_fixture', 'type': 'message',
                                  'role': 'assistant', 'model': body['model'], 'content': [], 'usage': {'input_tokens': 3, 'output_tokens': 0}}})]
                        block = {'type': 'text', 'text': ''} if second else {'type': 'tool_use', 'id': 'call_fixture', 'name': tool_name, 'input': {}}
                        events += [('content_block_start', {'type': 'content_block_start', 'index': 0, 'content_block': block}),
                                   ('content_block_delta', {'type': 'content_block_delta', 'index': 0, 'delta':
                                    {'type': 'text_delta', 'text': 'SYNTHETIC_FINAL'} if second else {'type': 'input_json_delta', 'partial_json': json.dumps(arguments)}}),
                                   ('content_block_stop', {'type': 'content_block_stop', 'index': 0}),
                                   ('message_delta', {'type': 'message_delta', 'delta': {'stop_reason': 'end_turn' if second else 'tool_use'}, 'usage': {'output_tokens': 2}}),
                                   ('message_stop', {'type': 'message_stop'})]
                        payload = ''.join('event: %s\ndata: %s\n\n' % (name, json.dumps(event)) for name, event in events)
                    self.send_response(200)
                    self.send_header('Content-Type', 'text/event-stream')
                    self.send_header('Content-Length', str(len(payload.encode())))
                    self.end_headers()
                    self.wfile.write(payload.encode())
                    self.wfile.flush()
                except Exception as error:
                    errors.append(str(error))
                    self.send_error(500)

        server = ThreadingHTTPServer(('127.0.0.1', 0), Upstream)
        threading.Thread(target=server.serve_forever, daemon=True).start()
        app = None
        try:
            shutil.copy(args.bindings / 'velune_bindings.py', root / 'bindings')
            (root / 'bindings/libvelune_bindings.dylib').symlink_to(args.bundle / 'Contents/Frameworks/libvelune_bindings.dylib')
            os.environ.clear()
            os.environ.update(HOME=str(root / 'home'), PATH=str(args.node.parent) + ':/usr/bin:/bin', NO_PROXY='127.0.0.1,localhost')
            spec = importlib.util.spec_from_file_location('velune_bindings', root / 'bindings/velune_bindings.py')
            b = importlib.util.module_from_spec(spec)
            sys.modules[spec.name] = b
            spec.loader.exec_module(b)
            app = b.VeluneApplication.open(b.BindingOptions(home_directory=str(root / 'home'), resources_directory=str(args.bundle / 'Contents/Resources')))
            cases = [(protocol, path, tool_case) for protocol, path in
                     ((b.BindingGatewayProtocol.CHAT_COMPLETIONS_V1, '/v1/chat/completions'),
                      (b.BindingGatewayProtocol.MESSAGES_V1, '/v1/messages')) for tool_case in ('command', 'namespace')]
            for protocol, path, tool_case in cases:
                case = protocol.name.lower() + '-' + tool_case
                agent = root / case
                agent.mkdir()
                app.upsert_runtime(b.BindingRuntimeInstance(enabled=True, id=case, name=case, type_id='codex-0.159.3', gateway_id='default',
                    settings={'binary': str(args.codex), 'nodeBinary': str(args.node), 'agentDir': str(agent)}))
                model = b.BindingProviderModel(record_key='', provider_model_id='synthetic-' + case, nickname=case, icon=None,
                    context_window=32768, max_output_tokens=4096, reasoning_levels=None, adapter_metadata_json=None)
                endpoint = 'http://127.0.0.1:%d' % server.server_port + ('' if protocol == b.BindingGatewayProtocol.MESSAGES_V1 else '/v1')
                app.save_provider('default', b.BindingProviderDraft(id=case, name=case, protocol=protocol, endpoint=endpoint, models=[model]),
                    b.BindingAuthenticationEdit.SET_API_KEY(value='synthetic-only'))
                key = next(p.models[0].record_key for g in app.list().gateways for p in g.providers if p.id == case)
                app.create_conversation(case, str(root / 'project'), key)
                before = len(captures)
                app.send_turn(case, key, 'Read fixture.txt using the available command tool, then answer.')
                deadline = time.monotonic() + 60
                while time.monotonic() < deadline:
                    snapshot = app.snapshot(case).snapshot
                    if errors:
                        raise RuntimeError(errors[-1])
                    if snapshot and snapshot.run_state == b.BindingRunState.FAILED:
                        raise RuntimeError('runtime failed: ' + repr(snapshot))
                    if snapshot and snapshot.actions.can_send and len(captures) >= before + 2:
                        texts = [block.text for msg in snapshot.messages for block in msg.blocks if isinstance(block, b.BindingMessageBlock.TEXT)]
                        assert 'SYNTHETIC_FINAL' in texts, texts
                        break
                    time.sleep(.05)
                else:
                    raise RuntimeError('Codex did not complete tool continuation: ' + case)
                assert len(captures) == before + 2, 'unexpected runtime retry'
                assert all(c['path'] == path for c in captures[before:]), 'wrong upstream protocol path'
                continuation = json.dumps(captures[-1]['body'])
                if tool_case == 'command':
                    assert 'SYNTHETIC_TOOL_RESULT' in continuation, 'actual command output missing from continuation'
                else:
                    assert '00000000-0000-0000-0000-000000000001' in continuation and ('not found' in continuation.lower() or 'invalid' in continuation.lower()), 'native namespace tool failure missing from continuation'
                reports.append({'toolCase': tool_case, 'toolOutcome': 'native_command_success' if tool_case == 'command' else 'native_missing_target_error_preserved', 'wrappedCustomInputObserved': any('input' in t.get('function', t).get('parameters', t.get('input_schema', {})).get('properties', {}) for t in captures[before]['body'].get('tools', [])), 'client': 'Codex 0.159.3', 'ingress': 'Responses', 'upstream': protocol.name.lower(), 'requests': 2, 'actualToolContinuation': True})
            # Force a Responses ingress using the installed Pi SDK, instead of
            # application auto-injection's deliberate same-protocol preference.
            app.shutdown()
            app = None
            probe_source = root / 'runner.rs'
            probe_source.write_text(r'''use std::{collections::BTreeMap,io::{BufRead,Write},sync::Arc};
struct Resolver;
impl velune_gateway::CredentialResolver for Resolver {
 fn resolve(&self,_:String,_:velune_gateway::CredentialTarget)->velune_ai::OperationFuture<Result<velune_gateway::ResolvedCredential,velune_gateway::CredentialResolutionError>> {
  Box::pin(async {Ok(velune_gateway::ResolvedCredential {token:"synthetic-only".into(),explicit_output_cap:None,subscription:false})})
 }
}
fn main()->Result<(),Box<dyn std::error::Error>> {
 let a:Vec<_>=std::env::args().collect();let c=serde_json::from_slice(&std::fs::read(&a[1])?)?;
 let r=velune_gateway::Runner::start(c,Arc::new(Resolver),BTreeMap::new())?;
 println!("{}",serde_json::json!({"endpoint":r.endpoint(),"token":r.token()}));std::io::stdout().flush()?;
 let _=std::io::stdin().lock().lines().next();Ok(())
}''')
            build = subprocess.run([str(developer_home / '.cargo/bin/cargo'), 'build', '--locked', '-p', 'velune-gateway', '--message-format=json'],
                                   cwd=repository, env=environment, capture_output=True, text=True, check=True)
            artifacts = {}
            for line in build.stdout.splitlines():
                item = json.loads(line)
                if item.get('reason') == 'compiler-artifact':
                    for filename in item['filenames']:
                        if filename.endswith('.rlib'):
                            artifacts[item['target']['name']] = filename
            binary = root / 'runner'
            subprocess.run([str(developer_home / '.cargo/bin/rustc'), '--edition=2024', str(probe_source),
                '-L', 'dependency=' + str(repository / 'target/debug/deps'),
                *[argument for name in ('velune_gateway', 'velune_ai', 'serde_json') for argument in ('--extern', name + '=' + artifacts[name])],
                '-o', str(binary)], env=environment, check=True)
            config = root / 'gateway.json'
            config.write_text(json.dumps({'id': 'synthetic', 'name': 'synthetic', 'providers': [{'id': 'pi-chat', 'name': 'pi-chat',
                'protocol': 'chatCompletionsV1', 'endpoint': 'http://127.0.0.1:%d/v1' % server.server_port, 'credentialRef': 'synthetic',
                'models': [{'recordKey': 'pi-chat', 'providerModelId': 'synthetic-pi', 'nickname': 'synthetic', 'maxOutputTokens': 128}]}],
                'failover': {'mode': 'disabled'}}))
            # Locate only the selected external installation's public SDK exports.
            ai_package = next((parent / 'node_modules/@earendil-works/pi-ai' for parent in args.pi.resolve().parents
                              if (parent / 'node_modules/@earendil-works/pi-ai/package.json').exists()), None)
            if ai_package is None:
                raise RuntimeError('Pi installation does not resolve its public pi-ai package')
            ai_manifest = json.loads((ai_package / 'package.json').read_text())
            assert ai_manifest['version'] == '1.0.2', 'probe requires Pi SDK 1.0.2'
            public_index = ai_package / ai_manifest['exports']['.']['import']
            public_responses = ai_package / ai_manifest['exports']['./api/*']['import'].replace('*', 'openai-responses')
            js = root / 'pi-client.mjs'
            js.write_text(r'''import fs from 'node:fs';
import {pathToFileURL} from 'node:url';
const [indexPath,apiPath,endpoint,token,file]=process.argv.slice(2);
const {normalizeContext}=await import(pathToFileURL(indexPath));
const {stream}=await import(pathToFileURL(apiPath));
const model={id:'velune/model/pi-chat',name:'Synthetic',api:'openai-responses',provider:'synthetic',baseUrl:endpoint,
 reasoning:false,input:['text'],cost:{input:0,output:0,cacheRead:0,cacheWrite:0},contextWindow:8192,maxTokens:128};
const context={messages:[{role:'user',content:[{type:'text',text:'Read the fixture.'}],timestamp:Date.now()}],
 tools:[{name:'read',description:'Read fixture',parameters:{type:'object',properties:{path:{type:'string'}},required:['path']}}]};
const first=await stream(model,normalizeContext(context),{apiKey:token}).result();
if(first.stopReason==='error')throw Error(first.errorMessage);
const call=first.content.find(c=>c.type==='toolCall');
if(!call||call.name!=='read'||call.arguments.path!==file)throw Error('SDK failed to parse translated tool call');
const text=fs.readFileSync(call.arguments.path,'utf8');
context.messages.push(first,{role:'toolResult',toolCallId:call.id,toolName:call.name,
 content:[{type:'text',text}],isError:false,timestamp:Date.now()});
const final=await stream(model,normalizeContext(context),{apiKey:token}).result();
if(final.stopReason==='error'||!final.content.some(c=>c.type==='text'&&c.text==='SYNTHETIC_FINAL'))
 throw Error('SDK failed to complete translated continuation: '+JSON.stringify(final));
console.log(JSON.stringify({client:'Pi SDK 1.0.2',ingress:'Responses',upstream:'ChatCompletions',requests:2,actualToolContinuation:true}));
''')
            process = subprocess.Popen([str(binary), str(config)], stdin=subprocess.PIPE, stdout=subprocess.PIPE, text=True)
            try:
                info = json.loads(process.stdout.readline())
                before = len(captures)
                result = subprocess.run([str(args.node), str(js), str(public_index), str(public_responses), info['endpoint'], info['token'], str(tool_file)],
                                        capture_output=True, text=True, timeout=40, check=True)
                reports.append(json.loads(result.stdout))
                assert len(captures) == before + 2, 'Pi SDK issued unexpected retries'
                assert all(c['path'] == '/v1/chat/completions' for c in captures[before:])
                assert 'SYNTHETIC_TOOL_RESULT' in json.dumps(captures[-1]['body'])
            finally:
                process.stdin.write('stop\n')
                process.stdin.flush()
                process.wait(timeout=5)
            print(json.dumps({'acceptance': 'PASSED', 'cases': reports, 'upstreamRequests': len(captures)}))
        finally:
            if app:
                app.shutdown()
            os.environ.clear()
            os.environ.update(environment)
            server.shutdown()
            server.server_close()


if __name__ == '__main__':
    main()
