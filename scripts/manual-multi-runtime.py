#!/usr/bin/env python3
"""Explicit isolated Codex app-server / DeepSeek ACP acceptance with loopback upstream.
Not a CI/test entry point. Runtime binaries must be the declared version variants.
"""
import argparse, importlib.util, json, os, shutil, sys, tempfile, threading, time
from pathlib import Path
from http.server import BaseHTTPRequestHandler, ThreadingHTTPServer

def main():
    parser=argparse.ArgumentParser(description=__doc__)
    for name in ['bundle','bindings','node','codex','dsh']: parser.add_argument('--'+name,required=True,type=Path)
    args=parser.parse_args()
    for path in vars(args).values():
        if not path.is_absolute() or not path.exists(): parser.error('all paths must be absolute and exist')
    captures=[]; failures=[]; environment=dict(os.environ)
    class Upstream(BaseHTTPRequestHandler):
        def log_message(self,*_): pass
        def do_POST(self):
            try:
                body=json.loads(self.rfile.read(int(self.headers['Content-Length'])))
                captures.append({'path':self.path,'body':body,'authorization':self.headers.get('Authorization')})
                text='SYNTHETIC_NATIVE_ANSWER'
                if self.path.endswith('/responses'):
                    response={'id':f'resp_synthetic_{len(captures)}','object':'response','status':'completed','model':body['model'],'output':[{'id':f'msg_synthetic_{len(captures)}','type':'message','status':'completed','role':'assistant','content':[{'type':'output_text','text':text,'annotations':[]}]}],'usage':{'input_tokens':10,'output_tokens':5,'total_tokens':15}}
                    events=[{'type':'response.created','response':{**response,'status':'in_progress','output':[]}},
                            {'type':'response.output_item.added','output_index':0,'item':{**response['output'][0],'status':'in_progress','content':[]}},
                            {'type':'response.content_part.added','item_id':f'msg_synthetic_{len(captures)}','output_index':0,'content_index':0,'part':{'type':'output_text','text':'','annotations':[]}},
                            {'type':'response.output_text.delta','item_id':f'msg_synthetic_{len(captures)}','output_index':0,'content_index':0,'delta':text},
                            {'type':'response.output_text.done','item_id':f'msg_synthetic_{len(captures)}','output_index':0,'content_index':0,'text':text},
                            {'type':'response.content_part.done','item_id':f'msg_synthetic_{len(captures)}','output_index':0,'content_index':0,'part':response['output'][0]['content'][0]},
                            {'type':'response.output_item.done','output_index':0,'item':response['output'][0]},
                            {'type':'response.completed','response':response}]
                    payload=''.join('event: '+e['type']+'\ndata: '+json.dumps(e)+'\n\n' for e in events).encode()
                else:
                    events=[{'id':'chat_synthetic','object':'chat.completion.chunk','model':body['model'],'choices':[{'index':0,'delta':{'role':'assistant','content':text},'finish_reason':None}]},
                            {'id':'chat_synthetic','object':'chat.completion.chunk','model':body['model'],'choices':[{'index':0,'delta':{},'finish_reason':'stop'}],'usage':{'prompt_tokens':10,'completion_tokens':5,'total_tokens':15}}]
                    payload=(''.join('data: '+json.dumps(e)+'\n\n' for e in events)+'data: [DONE]\n\n').encode()
                self.send_response(200);self.send_header('Content-Type','text/event-stream');self.send_header('Content-Length',str(len(payload)));self.end_headers();self.wfile.write(payload);self.wfile.flush()
            except Exception as error: failures.append(type(error).__name__)
    server=ThreadingHTTPServer(('127.0.0.1',0),Upstream);server.daemon_threads=True
    threading.Thread(target=server.serve_forever,daemon=True).start()
    application=None
    with tempfile.TemporaryDirectory(prefix='velune-multi-runtime-') as directory:
        root=Path(directory);(root/'bindings').mkdir();(root/'project').mkdir();(root/'home').mkdir()
        shutil.copy(args.bindings/'velune_bindings.py',root/'bindings')
        (root/'bindings/libvelune_bindings.dylib').symlink_to(args.bundle/'Contents/Frameworks/libvelune_bindings.dylib')
        os.environ.clear();os.environ.update(HOME=str(root/'home'),PATH=f'{args.node.parent}:/usr/bin:/bin',NO_PROXY='127.0.0.1,localhost')
        try:
            spec=importlib.util.spec_from_file_location('velune_bindings',root/'bindings/velune_bindings.py');b=importlib.util.module_from_spec(spec);sys.modules[spec.name]=b;spec.loader.exec_module(b)
            application=b.VeluneApplication.open(b.BindingOptions(home_directory=str(root/'home'),resources_directory=str(args.bundle/'Contents/Resources')))
            descriptor={d.id:d for d in application.list().runtime_types}
            expected={'pi-1.0.2':('pi',r'^1\.0\.2$'),'codex-0.159.3':('codex',r'^0\.159\.3$'),'dsh-acp-0.2.0-rc.2':('deepseek-harness',r'^0\.2\.0-rc\.2$')}
            for identity,(family,pattern) in expected.items():
                assert descriptor[identity].family_id==family and descriptor[identity].version_regex==pattern
            chat=b.BindingGatewayProtocol.CHAT_COMPLETIONS_V1;responses=b.BindingGatewayProtocol.RESPONSES_V1
            assert set(descriptor['pi-1.0.2'].supported_protocols)=={chat,responses}
            assert set(descriptor['codex-0.159.3'].supported_protocols)=={responses}
            assert set(descriptor['dsh-acp-0.2.0-rc.2'].supported_protocols)=={chat,responses}
            assert all(b.BindingGatewayProtocol.MESSAGES_V1 not in d.supported_protocols for d in descriptor.values())
            endpoint=f'http://127.0.0.1:{server.server_port}/v1'
            results={}
            for family,type_id,binary,protocol,api_id in [
                ('codex','codex-0.159.3',args.codex,b.BindingGatewayProtocol.RESPONSES_V1,'synthetic-responses'),
                ('deepseek','dsh-acp-0.2.0-rc.2',args.dsh,b.BindingGatewayProtocol.CHAT_COMPLETIONS_V1,'synthetic-chat')]:
                model=b.BindingProviderModel(record_key='',provider_model_id=api_id,nickname='Synthetic native',icon=None,context_window=32768,max_output_tokens=4096,reasoning_levels=None,adapter_metadata_json=None)
                provider=b.BindingProviderDraft(id=family,name=family,protocol=protocol,endpoint=endpoint,models=[model])
                application.save_provider('default',provider,b.BindingAuthenticationEdit.SET_API_KEY(value='synthetic-only'))
                saved=next(p for p in application.list().gateways[0].providers if p.id==family)
                alternate=b.BindingProviderDraft(id=family+'-alternate',name='Synthetic alternate',protocol=protocol,endpoint=endpoint.replace('/v1','/alternate/v1'),models=[model])
                application.save_provider('default',alternate,b.BindingAuthenticationEdit.SET_API_KEY(value='synthetic-alternate'))
                alternate_saved=next(p for p in application.list().gateways[0].providers if p.id==alternate.id)
                agent_home=root/family;agent_home.mkdir()
                if family == 'deepseek':
                    switched_model=b.BindingProviderModel(record_key='',provider_model_id=api_id,nickname='Synthetic responses',icon=None,context_window=None,max_output_tokens=None,reasoning_levels=None,adapter_metadata_json=None)
                    switched=b.BindingProviderDraft(id='protocol-switch',name='Synthetic protocol switch',protocol=responses,endpoint=endpoint+'/alternate/v1',models=[switched_model])
                    switched_saved=next(p for g in application.save_provider('default',switched,b.BindingAuthenticationEdit.SET_API_KEY(value='synthetic-alternate')).gateways for p in g.providers if p.id=='protocol-switch')
                runtime=b.BindingRuntimeInstance(enabled=True, id=family,name=family,type_id=type_id,gateway_id='default',settings={'binary':str(binary),'nodeBinary':str(args.node),'agentDir':str(agent_home)})
                application.upsert_runtime(runtime);application.select_runtime(family)
                created=application.create_conversation(family,str(root/'project'),saved.models[0].record_key).snapshot
                assert created and created.conversation.cwd==str(root/'project')
                assert created.conversation.created_at_unix_ms is not None and created.conversation.created_at_unix_ms > 0, family+' new native creation date missing'
                native_created_at = created.conversation.created_at_unix_ms
                time.sleep(1.2) # Separate native creation from first event persistence.
                before=len(captures);application.send(family,'Reply with the synthetic answer; no tools are needed.')
                deadline=time.monotonic()+45
                while time.monotonic()<deadline:
                    snapshot=application.snapshot(family).snapshot
                    if snapshot and snapshot.run_state==b.BindingRunState.FAILED: raise AssertionError(family+' run failed')
                    if snapshot and snapshot.pending_interactions: raise AssertionError('unexpected interaction in no-tool turn')
                    if snapshot and snapshot.actions.can_send and len(captures)>before: break
                    time.sleep(.05)
                else: raise AssertionError(family+' run did not settle')
                assert any('SYNTHETIC_NATIVE_ANSWER' in block.text for message in snapshot.messages for block in message.blocks if isinstance(block,b.BindingMessageBlock.TEXT)),family+' answer missing'
                assert snapshot.conversation.created_at_unix_ms == native_created_at, family+' first turn lost native creation date'
                assert snapshot.conversation.title == 'Reply with the synthetic answer; no tools are needed.', family+' first user title missing'
                assert captures[-1]['body']['model']==api_id and captures[-1]['authorization']=='Bearer synthetic-only'
                sessions=application.list().conversations
                assert any(s.id==snapshot.conversation.id for s in sessions),family+' native history missing from huihua list'
                native_summary = next(s for s in sessions if s.id == snapshot.conversation.id)
                assert native_summary.created_at_unix_ms is not None and native_summary.created_at_unix_ms > 0, family+' native creation time missing'
                assert native_summary.updated_at_unix_ms is not None and native_summary.updated_at_unix_ms > 0, family+' native activity time missing'
                reopened=application.open_conversation(family,snapshot.conversation.id).snapshot
                assert reopened and reopened.conversation.created_at_unix_ms is not None
                if family == 'codex':
                    assert 0 <= reopened.conversation.created_at_unix_ms - native_created_at < 1000, f'Codex native creation mismatch: API={native_created_at} header={reopened.conversation.created_at_unix_ms}'
                else:
                    assert reopened.conversation.created_at_unix_ms == native_created_at, family+' reopen changed native creation date'
                history_created_at = reopened.conversation.created_at_unix_ms
                assert reopened and any('SYNTHETIC_NATIVE_ANSWER' in block.text for message in reopened.messages for block in message.blocks if isinstance(block,b.BindingMessageBlock.TEXT)),family+' historical answer missing'
                assert reopened.conversation.title == snapshot.conversation.title, family+' reopened title changed: '+repr(reopened.conversation.title)+' vs '+repr(snapshot.conversation.title)
                assert reopened.model_record_key is None and not reopened.actions.can_send,family+' inferred provider from bare history model ID'
                application.select_model(family,alternate_saved.models[0].record_key)
                before=len(captures);application.send(family,'Continue after the explicitly selected provider change.')
                deadline=time.monotonic()+45
                while time.monotonic()<deadline:
                    selected_snapshot=application.snapshot(family).snapshot
                    if selected_snapshot and selected_snapshot.run_state==b.BindingRunState.FAILED: raise AssertionError(family+' selected provider run failed')
                    if selected_snapshot and selected_snapshot.actions.can_send and len(captures)>before: break
                    time.sleep(.05)
                else: raise AssertionError(family+' selected provider did not settle')
                assert captures[-1]['authorization']=='Bearer synthetic-alternate' and captures[-1]['path'].startswith('/alternate/v1/'),family+' selected provider route did not change'
                assert captures[-1]['body']['model']==api_id
                assert selected_snapshot.conversation.created_at_unix_ms == history_created_at, family+' continuation changed native creation date'
                assert selected_snapshot.conversation.title == reopened.conversation.title, family+' preparation lost title'
                assert selected_snapshot.model_record_key==alternate_saved.models[0].record_key
                cross_protocol = False
                if family == 'deepseek':
                    before=len(captures)
                    selected=application.select_model(family,switched_saved.models[0].record_key).snapshot
                    assert len(captures)==before and selected.model_record_key==switched_saved.models[0].record_key
                    assert selected.conversation.id==selected_snapshot.conversation.id and selected.messages
                    application.send(family,'Continue this native session using the explicitly chosen Responses model.')
                    deadline=time.monotonic()+45
                    while time.monotonic()<deadline:
                        switched_snapshot=application.snapshot(family).snapshot
                        if switched_snapshot and switched_snapshot.run_state==b.BindingRunState.FAILED: raise AssertionError('cross-protocol native resume failed')
                        if switched_snapshot and switched_snapshot.actions.can_send and len(captures)>before: break
                        time.sleep(.05)
                    else: raise AssertionError('cross-protocol native resume did not settle')
                    assert captures[-1]['path'].endswith('/responses') and captures[-1]['authorization']=='Bearer synthetic-alternate'
                    assert switched_snapshot.conversation.id==selected.conversation.id
                    cross_protocol=True
                results[family]={'crossProtocolSelectionDefersPreparationAndResumes':cross_protocol,'actualControlAndGateway':True,'huihuaHistoryAndResume':True,'nativeIdAndCwd':True,'sameModelIdAcrossProvidersRoutesPrecisely':True}
            bad_binary=root/'unsupported-runtime'
            bad_binary.write_text('#!/bin/sh\nprintf "99.0.0\\n"\n');bad_binary.chmod(0o700)
            bad_home=root/'unsupported-home';bad_home.mkdir()
            bad=b.BindingRuntimeInstance(enabled=True, id='unsupported',name='Unsupported synthetic',type_id='codex-0.159.3',gateway_id='default',settings={'binary':str(bad_binary),'nodeBinary':str(args.node),'agentDir':str(bad_home)})
            application.upsert_runtime(bad)
            try:application.create_conversation(bad.id,str(root/'project'),application.list().gateways[0].providers[0].models[0].record_key)
            except b.BindingError as error:
                assert 'version' in str(error).lower() or '版本' in str(error),str(error)
            else:raise AssertionError('runtime with incompatible actual version prepared')
            assert application.list().selected_runtime_instance_id=='deepseek','version rejection lost active runtime'

            assert not failures,failures
            application.shutdown();application=None
            assert json.loads((root/'home/generic-config.json').read_text())['schemaVersion']==7
            logs=''.join(path.read_text() for path in (root/'home/logs').glob('*.jsonl'))
            for forbidden in ['synthetic-only','synthetic-alternate','SYNTHETIC_NATIVE_ANSWER','synthetic-responses','synthetic-chat',endpoint,str(root)]:
                assert forbidden not in logs,'business/configuration data entered diagnostic log'
            events=[json.loads(line) for line in logs.splitlines()]
            def request_id(event):
                spans=event.get('spans',[])+[event.get('span',{})]
                return next((span.get('request_id') for span in spans if span.get('name')=='gateway_request'),None)
            requests={request_id(event) for event in events if event.get('fields',{}).get('event')=='gateway_request_received'}
            assert None not in requests and len(requests)==len(captures)
            for identity in requests:
                selected=[event for event in events if request_id(event)==identity]
                def named(name): return [event for event in selected if event.get('fields',{}).get('event')==name]
                assert len(named('gateway_route_selected'))==1
                assert len(named('gateway_attempt_started'))==len(named('gateway_attempt_finished'))==1
                assert len(named('gateway_request_finished'))==1
                assert named('gateway_attempt_finished')[0]['fields']['outcome']=='transport_completed'
                assert named('gateway_request_finished')[0]['fields']['outcome']=='transport_completed'
                assert named('gateway_response_ready')[0]['fields']['http_status']==200

            print(json.dumps({'acceptance':'PASSED','runtimeVariants':list(expected),'actualRuntimes':results,'upstreamRequests':len(captures),'versionMismatchPreservesActiveRuntime':True,'metadataOnlyCorrelatedGatewayObservations':True,'realServicesCalled':False},indent=2))
        finally:
            if application:
                try: application.shutdown()
                except Exception: pass
            os.environ.clear();os.environ.update(environment);server.shutdown();server.server_close()
if __name__=='__main__':main()
