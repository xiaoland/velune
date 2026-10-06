#!/usr/bin/env python3
"""Manual real Pi/DSH adapters against isolated native Messages loopback, including Pi import."""
import argparse, importlib.util, json, os, shutil, sys, tempfile, threading, time
from pathlib import Path
from http.server import BaseHTTPRequestHandler, ThreadingHTTPServer

def main():
    parser=argparse.ArgumentParser(description=__doc__)
    for name in ['bundle','bindings','node','pi','dsh']: parser.add_argument('--'+name,required=True,type=Path)
    args=parser.parse_args()
    captures=[]; failures=[]; environment=dict(os.environ)
    class Upstream(BaseHTTPRequestHandler):
        def log_message(self,*_): pass
        def do_POST(self):
            try:
                body=json.loads(self.rfile.read(int(self.headers['content-length'])))
                captures.append((self.path,dict(self.headers),body))
                assert self.path=='/v1/messages',self.path
                assert self.headers.get('x-api-key')=='synthetic-only'
                assert self.headers.get('authorization') is None
                assert self.headers.get('anthropic-version')
                assert body['model']=='synthetic-native-model'
                events=[('message_start',{'type':'message_start','message':{'id':'synthetic','type':'message','role':'assistant','model':body['model'],'content':[],'stop_reason':None,'stop_sequence':None,'usage':{'input_tokens':3,'output_tokens':0}}}),('content_block_start',{'type':'content_block_start','index':0,'content_block':{'type':'text','text':''}}),('content_block_delta',{'type':'content_block_delta','index':0,'delta':{'type':'text_delta','text':'SYNTHETIC_MESSAGES_ANSWER'}}),('content_block_stop',{'type':'content_block_stop','index':0}),('message_delta',{'type':'message_delta','delta':{'stop_reason':'end_turn','stop_sequence':None},'usage':{'output_tokens':5}}),('message_stop',{'type':'message_stop'})]
                wire=''.join('event: '+name+'\ndata: '+json.dumps(value)+'\n\n' for name,value in events).encode()
                self.send_response(200);self.send_header('content-type','text/event-stream');self.send_header('content-length',str(len(wire)));self.end_headers();self.wfile.write(wire)
            except Exception as error:
                failures.append(str(error));self.send_error(500)
    server=ThreadingHTTPServer(('127.0.0.1',0),Upstream);threading.Thread(target=server.serve_forever,daemon=True).start()
    try:
        with tempfile.TemporaryDirectory(prefix='velune-runtime-messages-') as directory:
            root=Path(directory)
            for name in ['home','bindings','project','pi','dsh','resources']:(root/name).mkdir()
            resources=root/'resources'
            source_resources=args.bundle/'Contents/Resources'
            for item in source_resources.iterdir():
                if item.name == 'node_modules':
                    continue
                if item.is_dir():(resources/item.name).symlink_to(item)
                else:shutil.copy(item,resources/item.name)
            # DSH's read-only huihua adapter still needs its public JS
            # dependencies. Link those from the app fixture, while keeping
            # every Pi package external through --pi.
            source_modules = source_resources / 'node_modules'
            if source_modules.is_dir():
                (resources / 'node_modules').mkdir()
                for item in source_modules.iterdir():
                    if item.name == '@earendil-works':
                        continue
                    (resources / 'node_modules' / item.name).symlink_to(item,
                        target_is_directory=item.is_dir())
            # Use current authored source helpers, without changing an installed app or its resources.
            for name in ['pi_sdk.mjs','pi_auth.mjs','pi_provider_import.mjs','pi_sessions.mjs','pi_virtual_model.mjs','pi_rpc.mjs','huihua_sessions.mjs']:
                shutil.copy(Path(__file__).resolve().parents[1]/'packages/agent-runtime/resources'/name,resources/name)
            shutil.copy(args.bindings/'velune_bindings.py',root/'bindings')
            (root/'bindings/libvelune_bindings.dylib').symlink_to(args.bundle/'Contents/Frameworks/libvelune_bindings.dylib')
            os.environ.clear();os.environ.update(HOME=str(root/'home'),PATH=f'{args.node.parent}:/usr/bin:/bin',NO_PROXY='127.0.0.1,localhost')
            spec=importlib.util.spec_from_file_location('velune_bindings',root/'bindings/velune_bindings.py');b=importlib.util.module_from_spec(spec);sys.modules[spec.name]=b;spec.loader.exec_module(b)
            app=b.VeluneApplication.open(b.BindingOptions(home_directory=str(root/'home'),resources_directory=str(resources)))
            endpoint=f'http://127.0.0.1:{server.server_port}'
            runtimes=[b.BindingRuntimeInstance(enabled=True,id='pi',name='Synthetic Pi',type_id='pi-1.0.2',gateway_id='default',settings={'agentDir':str(root/'pi'),'nodeBinary':str(args.node),'binary':str(args.pi)}), b.BindingRuntimeInstance(enabled=True,id='dsh',name='Synthetic DSH',type_id='dsh-acp-0.2.0-rc.2',gateway_id='default',settings={'agentDir':str(root/'dsh'),'nodeBinary':str(args.node),'binary':str(args.dsh)})]
            for runtime in runtimes:app.upsert_runtime(runtime)
            model={'id':'synthetic-native-model','name':'Synthetic native','reasoning':False,'input':['text'],'contextWindow':32768,'maxTokens':1024,'compat':{'supportsStrictTools':False,'sendSessionAffinityHeaders':True}}
            original=json.dumps({'providers':{'synthetic-source':{'api':'anthropic-messages','baseUrl':endpoint,'apiKey':'synthetic-only','models':[model]}}}).encode()
            (root/'pi/models.json').write_bytes(original);(root/'pi/auth.json').write_text('{}')
            source=b.BindingProviderImportSource(kind='harness',harness_type_id='pi',source_instance_id='pi',provider_id=None,settings={})
            preview=app.preview_provider_import('default',source)
            provider=next(p for p in preview.providers if p.source_provider_id=='synthetic-source')
            candidate=provider.models[0];assert candidate.can_import,candidate.issues
            assert provider.protocol=="messagesV1"
            app.apply_provider_import('default',source,preview.token,[b.BindingImportSelection(provider_id=provider.id,candidate_keys=[candidate.candidate_key])],False)
            imported=next(p for p in app.list().gateways[0].providers if p.id==provider.id)
            assert imported.protocol==b.BindingGatewayProtocol.MESSAGES_V1
            assert (root/'pi/models.json').read_bytes()==original
            for runtime in runtimes:
                assert b.BindingGatewayProtocol.MESSAGES_V1 in next(d for d in app.list().runtime_types if d.id==runtime.type_id).supported_protocols
                app.create_conversation(runtime.id,str(root/'project'),imported.models[0].record_key)
                for turn in range(2):
                    before=len(captures);app.send_turn(runtime.id, imported.models[0].record_key, 'Reply with the synthetic answer, no tools.')
                    deadline=time.monotonic()+45
                    while time.monotonic()<deadline:
                        snapshot=app.snapshot(runtime.id).snapshot
                        if snapshot and snapshot.run_state==b.BindingRunState.FAILED:raise AssertionError(runtime.id+' run failed')
                        if snapshot and snapshot.actions.can_send and len(captures)>before:break
                        time.sleep(.05)
                    else:raise AssertionError(runtime.id+' did not settle')
                    assert any(isinstance(block,b.BindingMessageBlock.TEXT) and 'SYNTHETIC_MESSAGES_ANSWER' in block.text for message in snapshot.messages for block in message.blocks)
                app.open_conversation(runtime.id,snapshot.conversation.id)
            app.shutdown()
            assert not failures,failures
            assert len(captures)==4,len(captures)
            print('PASS: Pi import + Pi/DSH 2-turn native Messages through gateway, 4 isolated upstream requests.')
    finally:
        os.environ.clear();os.environ.update(environment);server.shutdown();server.server_close()
if __name__=='__main__':main()
