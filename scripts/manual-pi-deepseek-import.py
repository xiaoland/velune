#!/usr/bin/env python3
"""Manually reproduce the Pi DeepSeek-style import gap with synthetic local data.

This is an acceptance/diagnostic aid, not an automated test or CI entry point.
It loads the explicitly selected installed bundle, never user Pi files or APIs.
The SDK oracle uses a loopback HTTP server and synthetic reasoning/tool history.
"""
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
from http.server import BaseHTTPRequestHandler, ThreadingHTTPServer

SDK_ORACLE = r'''
import { pathToFileURL } from 'node:url';
import { join } from 'node:path';
const [resources, source] = process.argv.slice(2);
const {ModelRuntime} = await import(pathToFileURL(join(resources,'node_modules/@earendil-works/pi-coding-agent/dist/index.js')));
const runtime = await ModelRuntime.create({modelsPath:join(source,'models.json'),
 credentials:{read:async()=>undefined,list:async()=>[],modify:async()=>{throw Error('unused');},delete:async()=>{throw Error('unused');}},
 refreshOnCreate:false,allowModelNetwork:false});
const model = runtime.getModel('fixture-deepseek','reasoning');
const plain = runtime.getModel('fixture-deepseek','plain');
if (!model?.reasoning || plain?.reasoning !== false) throw Error('fixture reasoning mismatch');
const tool={name:'inspect_fixture',description:'Synthetic tool; no command execution.',parameters:{type:'object',properties:{},required:[]}};
const user=(text)=>({role:'user',content:[{type:'text',text}],timestamp:1});
const context={systemPrompt:'Synthetic local oracle.',messages:[user('SDK first tool turn.')],tools:[tool]};
const first=await runtime.completeSimple(model,context,{reasoning:'high',maxTokens:64,env:{}});
if(first.stopReason!=='toolUse')throw Error('first tool response missing');
const thinking=first.content.find(item=>item.type==='thinking');
if(thinking?.thinking!=='SYNTHETIC_PLAN' || thinking.thinkingSignature!=='reasoning_content')throw Error('reasoning stream not retained');
const call=first.content.find(item=>item.type==='toolCall');
context.messages.push(first,{role:'toolResult',toolCallId:call.id,toolName:call.name,content:[{type:'text',text:'SYNTHETIC_TOOL_RESULT'}],isError:false,timestamp:2});
const second=await runtime.completeSimple(model,context,{reasoning:'high',maxTokens:64,env:{}});
if(second.stopReason!=='stop')throw Error('tool continuation did not complete');
context.messages.push(second,user('SDK next user turn, thinking off.'));
const third=await runtime.completeSimple(model,context,{maxTokens:64,env:{}});
if(third.stopReason!=='stop')throw Error('off turn did not complete');
const emptyContext={systemPrompt:'Synthetic local oracle.',messages:[user('SDK empty reasoning history.'),
 {role:'assistant',api:model.api,provider:model.provider,model:model.id,content:[{type:'text',text:'SYNTHETIC_PLAIN_HISTORY'}],stopReason:'stop',timestamp:3,
 usage:{input:1,output:1,cacheRead:0,cacheWrite:0,totalTokens:2,cost:{input:0,output:0,cacheRead:0,cacheWrite:0,total:0}}},user('SDK empty history continuation.')],tools:[]};
await runtime.completeSimple(model,emptyContext,{reasoning:'high',maxTokens:64,env:{}});
await runtime.completeSimple(plain,{messages:[user('SDK nonreasoning model.')],tools:[]},{maxTokens:64,env:{}});
process.stdout.write(JSON.stringify({reasoningEnabled:model.reasoning,plainReasoningEnabled:plain.reasoning,
 streamReasoningPreserved:true,thinkingSignatureField:thinking.thinkingSignature,requests:5})+'\n');
'''


GATEWAY_PROBE = r'''
use std::{collections::BTreeMap, fs, io::{Read,Write}, net::TcpStream, path::PathBuf};
use serde_json::{json, Value};
use velune_gateway::{GatewayConfig, Runner};
fn main() -> Result<(),Box<dyn std::error::Error>> {
 let args:Vec<_>=std::env::args().collect();
 let config:GatewayConfig=serde_json::from_slice(&fs::read(&args[1])?)?;
 let runner=Runner::start(config,Some(PathBuf::from(&args[2])),BTreeMap::new(),BTreeMap::new())?;
 let address=runner.endpoint().strip_prefix("http://").unwrap().strip_suffix("/v1").unwrap();
 let mut observations=Vec::new();
 for path in &args[3..] {
  let payload=fs::read_to_string(path)?;
  let mut connection=TcpStream::connect(address)?;
  connection.set_read_timeout(Some(std::time::Duration::from_secs(10)))?;
  write!(connection,"POST /v1/chat/completions HTTP/1.1\r\nHost: {}\r\nAuthorization: Bearer {}\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{}",address,runner.token(),payload.len(),payload)?;
  let mut response=String::new();connection.read_to_string(&mut response)?;
  observations.push(json!({"reasoningDeltaDelivered":response.contains("\"reasoning_content\""),"terminalError":response.contains("\"error\""),"http200":response.starts_with("HTTP/1.1 200")}));
 }
 println!("{}",json!({"diagnosticOnly":true,"responses":observations}));
 Ok(())
}
'''


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('--bundle', required=True, type=Path)
    parser.add_argument('--bindings', required=True, type=Path)
    parser.add_argument('--node', required=True, type=Path)
    parser.add_argument('--probe-deps', type=Path, help='optional matching release deps for isolated gateway loss probe')
    parser.add_argument('--rustc', type=Path, help='absolute real rustc binary; used only with --probe-deps')
    args = parser.parse_args()
    for path in (args.bundle, args.bindings, args.node):
        if not path.is_absolute() or not path.exists():
            parser.error('all paths must be absolute and exist')
    if bool(args.probe_deps) != bool(args.rustc):
        parser.error('--probe-deps and --rustc must be supplied together')
    resources = args.bundle / 'Contents/Resources'
    library = args.bundle / 'Contents/Frameworks/libvelune_bindings.dylib'
    manifest = json.loads((resources / 'build-manifest.json').read_text())
    captures = []

    class Upstream(BaseHTTPRequestHandler):
        def log_message(self, *_):
            pass

        def do_POST(self):
            body = json.loads(self.rfile.read(int(self.headers['Content-Length'])))
            captures.append(body)
            first_tool = len(captures) == 1
            delta = {'reasoning_content': 'SYNTHETIC_PLAN' if first_tool else 'SYNTHETIC_REVIEW'}
            if first_tool:
                delta['tool_calls'] = [{'index': 0, 'id': 'call_fixture', 'type': 'function',
                                        'function': {'name': 'inspect_fixture', 'arguments': '{}'}}]
            else:
                delta['content'] = 'SYNTHETIC_ANSWER'
            self.send_response(200)
            self.send_header('Content-Type', 'text/event-stream')
            self.end_headers()
            chunks = [
                {'id': 'synthetic', 'object': 'chat.completion.chunk', 'created': 1, 'model': body['model'],
                 'choices': [{'index': 0, 'delta': delta, 'finish_reason': None}]},
                {'id': 'synthetic', 'object': 'chat.completion.chunk', 'created': 1, 'model': body['model'],
                 'choices': [{'index': 0, 'delta': {}, 'finish_reason': 'tool_calls' if first_tool else 'stop'}],
                 'usage': {'prompt_tokens': 1, 'completion_tokens': 2, 'total_tokens': 3}},
            ]
            for chunk in chunks:
                self.wfile.write(('data: ' + json.dumps(chunk) + '\n\n').encode())
            self.wfile.write(b'data: [DONE]\n\n')
            self.wfile.flush()

    server = ThreadingHTTPServer(('127.0.0.1', 0), Upstream)
    thread = threading.Thread(target=server.serve_forever, daemon=True)
    thread.start()
    previous_environment = dict(os.environ)
    try:
        with tempfile.TemporaryDirectory(prefix='velune-deepseek-acceptance-') as temporary:
            root = Path(temporary)
            for name in ('home', 'source', 'bindings'):
                (root / name).mkdir()
            compat = {'thinkingFormat': 'deepseek', 'requiresReasoningContentOnAssistantMessages': True,
                      'maxTokensField': 'max_tokens', 'supportsStore': False}
            models = [{'id': identity, 'name': identity, 'reasoning': reasoning, 'input': ['text'],
                       'contextWindow': 4096, 'maxTokens': 64, 'compat': compat}
                      for identity, reasoning in (('reasoning', True), ('plain', False))]
            source_files = {
                'models.json': json.dumps({'providers': {'fixture-deepseek': {
                    'baseUrl': f'http://127.0.0.1:{server.server_port}/v1', 'api': 'openai-completions',
                    'apiKey': 'synthetic-only', 'models': models}}}).encode(),
                'auth.json': b'{}', 'settings.json': b'{"synthetic":true}',
            }
            for name, content in source_files.items():
                (root / 'source' / name).write_bytes(content)
            os.environ.clear()
            os.environ.update(HOME=str(root / 'home'), PATH=f'{args.node.parent}:/usr/bin:/bin',
                              NO_PROXY='127.0.0.1,localhost')
            shutil.copy(args.bindings / 'velune_bindings.py', root / 'bindings')
            (root / 'bindings' / library.name).symlink_to(library)
            spec = importlib.util.spec_from_file_location('velune_bindings', root / 'bindings/velune_bindings.py')
            bindings = importlib.util.module_from_spec(spec)
            sys.modules[spec.name] = bindings
            spec.loader.exec_module(bindings)
            application = bindings.VeluneApplication.open(bindings.BindingOptions(
                home_directory=str(root / 'home'), resources_directory=str(resources), credential_resolver=None))
            try:
                application.upsert_gateway(bindings.BindingGatewayConfig(
                    id='fixture', name='Synthetic gateway', models=[], providers=[], routes=[],
                    failover=bindings.BindingFailoverPolicy(mode=bindings.BindingFailoverMode.DISABLED)))
                application.upsert_runtime(bindings.BindingRuntimeInstance(
                    id='fixture-runtime', name='Synthetic Pi', type_id='pi', gateway_id='fixture', model_id=None,
                    settings={'agentDir': str(root / 'source'), 'nodeBinary': str(args.node),
                              'binary': str(resources / 'node_modules/@earendil-works/pi-coding-agent/dist/bundle/cli.js')}))
                source = bindings.BindingProviderImportSource(kind='harness', harness_type_id='pi',
                    source_instance_id='fixture-runtime', provider_id=None, settings={})
                preview = application.preview_provider_import('fixture', source)
                provider, = preview.providers
                candidate = next(item for item in provider.models if item.external_model_id == 'reasoning')
                if provider.can_import or candidate.can_import:
                    raise AssertionError('baseline unexpectedly allows the blocked import')
                try:
                    application.apply_provider_import('fixture', source, preview.token,
                        [bindings.BindingImportSelection(provider_id=provider.id, model_ids=[candidate.id], model_mappings={})], False)
                except bindings.BindingError:
                    apply_rejected = True
                else:
                    raise AssertionError('unsupported selection unexpectedly imported')
                normal = {'acceptance': 'FAILED', 'configuredRuntime': True, 'previewSelectable': candidate.can_import,
                          'applyRejected': apply_rejected, 'reasoningModelIssues': candidate.issues,
                          'plainModelSelectable': next(item.can_import for item in provider.models if item.external_model_id == 'plain'),
                          'downstreamRouteConnectSend': 'not reachable through normal import'}
            finally:
                application.shutdown()
            sdk = root / 'sdk-oracle.mjs'
            sdk.write_text(SDK_ORACLE)
            process = subprocess.run([str(args.node), str(sdk), str(resources), str(root / 'source')],
                                     env=dict(os.environ), capture_output=True, timeout=30)
            if process.returncode:
                raise RuntimeError('synthetic SDK oracle failed; child messages intentionally not printed')
            sdk_result = json.loads(process.stdout)
            if len(captures) != 5:
                raise AssertionError('SDK request count mismatch')
            first, tool_continuation, next_turn, empty_history, plain = captures
            assert first['thinking'] == {'type': 'enabled'}
            assert first['reasoning_effort'] == 'high'
            assert first['max_tokens'] > 0 and 'max_completion_tokens' not in first
            assistant = next(item for item in tool_continuation['messages'] if item.get('tool_calls'))
            assert assistant['reasoning_content'] == 'SYNTHETIC_PLAN'
            assert any(item.get('role') == 'tool' and item['tool_call_id'] == 'call_fixture'
                       and item['content'] == 'SYNTHETIC_TOOL_RESULT' for item in tool_continuation['messages'])
            assert next_turn['thinking'] == {'type': 'disabled'} and 'reasoning_effort' not in next_turn
            assert all('reasoning_content' in item for item in next_turn['messages'] if item.get('role') == 'assistant')
            empty_assistant = next(item for item in empty_history['messages'] if item.get('role') == 'assistant')
            assert empty_assistant['reasoning_content'] == ''
            assert 'thinking' not in plain and 'reasoning_effort' not in plain
            gateway_result = {'diagnosticOnly': True, 'executed': False}
            if args.probe_deps:
                dependencies = args.probe_deps
                gateway_library = max(dependencies.glob('libvelune_gateway-*.rlib'), key=lambda path: path.stat().st_mtime)
                serde_library = max(dependencies.glob('libserde_json-*.rlib'), key=lambda path: path.stat().st_mtime)
                probe_source = root / 'gateway-probe.rs'
                probe_source.write_text(GATEWAY_PROBE)
                probe_binary = root / 'gateway-probe'
                compiled = subprocess.run([str(args.rustc), '--edition=2024', str(probe_source),
                    '--extern', f'velune_gateway={gateway_library}', '--extern', f'serde_json={serde_library}',
                    '-L', f'dependency={dependencies}', '-o', str(probe_binary)],
                    env=dict(os.environ), capture_output=True, timeout=60)
                if compiled.returncode:
                    raise RuntimeError('isolated gateway diagnostic compilation failed')
                resolver = root / 'synthetic-resolver'
                resolver.write_text('#!/bin/sh\nprintf synthetic-only\n')
                resolver.chmod(0o700)
                config = {'id':'diagnostic','name':'diagnostic','models':[{'id':'logical','nickname':'fixture','icon':None,
                    'maxOutputTokens':64,'contextWindow':16384,'reasoningLevels':['high']}],
                    'providers':[{'id':'fixture','name':'fixture','protocol':'chatCompletionsV1',
                    'endpoint':f'http://127.0.0.1:{server.server_port}/v1','credentialRef':'fixture',
                    'models':[{'modelId':'logical','externalModelId':'reasoning','chatCompletionsOutputLimitField':'max_tokens'}]}],
                    'routes':[{'modelId':'logical','providerId':'fixture'}],'failover':{'mode':'disabled'}}
                config_path = root / 'diagnostic-config.json'
                config_path.write_text(json.dumps(config))
                base = {'model':'logical','stream':True,'max_tokens':64,'reasoning_effort':'high',
                        'thinking':{'type':'enabled'},'messages':[{'role':'user','content':'Gateway diagnostic first turn.'}]}
                paths = []
                for index in range(2):
                    body = json.loads(json.dumps(base))
                    if index:
                        body['messages'] += [{'role':'assistant','content':None,'reasoning_content':'SYNTHETIC_PLAN',
                            'tool_calls':[{'id':'call_fixture','type':'function','function':{'name':'inspect_fixture','arguments':'{}'}}]},
                            {'role':'tool','tool_call_id':'call_fixture','content':'SYNTHETIC_TOOL_RESULT'}]
                    path = root / f'diagnostic-{index}.json'
                    path.write_text(json.dumps(body))
                    paths.append(str(path))
                probed = subprocess.run([str(probe_binary),str(config_path),str(resolver),*paths],
                                       env=dict(os.environ), capture_output=True, timeout=30)
                if probed.returncode:
                    raise RuntimeError('isolated gateway diagnostic invocation failed')
                gateway_result = json.loads(probed.stdout)
                assert len(captures) == 7
                downstream_first, downstream_second = captures[5:]
                assert 'thinking' not in downstream_first
                downstream_assistant = next(item for item in downstream_second['messages'] if item.get('tool_calls'))
                assert 'reasoning_content' not in downstream_assistant
                assert any(item.get('role') == 'tool' for item in downstream_second['messages'])
                assert all(not item['reasoningDeltaDelivered'] and item['terminalError'] for item in gateway_result['responses'])
                gateway_result.update({'executed':True,'incomingThinkingLost':True,'incomingReasoningHistoryLost':True,
                                       'toolCorrelationStillPresent':True,'reasoningStreamRejected':True,
                                       'normalImportAcceptance':False})
            assert all((root / 'source' / name).read_bytes() == content for name, content in source_files.items())
            print(json.dumps({'bundle': {'sourceCommit': manifest['source_commit'], 'dirty': manifest['dirty'],
                                        'uiVersion': manifest['ui_version'], 'installedLibraryUsed': True},
                              'normalImport': normal, 'sdkDiagnostic': {**sdk_result,
                                  'toolHistoryReasoningExact': True, 'toolCorrelationPreserved': True,
                                  'nextTurnThinkingDisabled': True, 'emptyReasoningFieldPreserved': True,
                                  'plainModelHasNoThinkingParameter': True}, 'gatewayDiagnostic':gateway_result, 'originalSourceFilesUnchanged': True},
                             ensure_ascii=False, indent=2))
    finally:
        os.environ.clear()
        os.environ.update(previous_environment)
        server.shutdown()
        server.server_close()
        thread.join(timeout=2)


if __name__ == '__main__':
    main()
