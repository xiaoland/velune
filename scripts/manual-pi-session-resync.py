#!/usr/bin/env python3
"""Explicit SDK 1.0.2 branch/session resync acceptance. All history is synthetic.
No provider requests are permitted; a local server counts unexpected attempts.
"""
import argparse, importlib.util, json, os, pathlib, shutil, subprocess, sys, tempfile, threading
from http.server import BaseHTTPRequestHandler, ThreadingHTTPServer
parser=argparse.ArgumentParser(description=__doc__)
for name in ['bundle','bindings','node']: parser.add_argument('--'+name,required=True,type=pathlib.Path)
args=parser.parse_args()
requests=[]
class Upstream(BaseHTTPRequestHandler):
    def log_message(self,*_): pass
    def do_POST(self): requests.append(self.path);self.send_response(500);self.end_headers()
server=ThreadingHTTPServer(('127.0.0.1',0),Upstream)
threading.Thread(target=server.serve_forever,daemon=True).start()
environment=dict(os.environ)
with tempfile.TemporaryDirectory(prefix='velune-pi-resync-') as directory:
    root=pathlib.Path(directory)
    for name in ['home','runtime','project','other-project','resources','bindings']: (root/name).mkdir()
    resources=root/'resources'
    for source in (args.bundle/'Contents/Resources').glob('*.mjs'): shutil.copy2(source,resources/source.name)
    (resources/'node_modules').symlink_to(args.bundle/'Contents/Resources/node_modules',target_is_directory=True)
    sdk=resources/'seed.mjs'
    sdk.write_text('''import {SessionManager} from '@earendil-works/pi-coding-agent';
const root=process.argv[2];
const manager=SessionManager.create(root+'/project');
const user=manager.appendMessage({role:'user',content:'Synthetic first task',timestamp:1000});
const a=manager.appendMessage({role:'assistant',content:[{type:'text',text:'BRANCH_A'}],api:'openai-completions',provider:'velune',model:'auto',timestamp:2000,usage:{input:1,output:1,cacheRead:0,cacheWrite:0,totalTokens:2,cost:{input:0,output:0,cacheRead:0,cacheWrite:0,total:0}},stopReason:'stop'});
manager.branch(user);
manager.appendMessage({role:'assistant',content:[{type:'text',text:'BRANCH_B'}],api:'openai-completions',provider:'velune',model:'auto',timestamp:3000,usage:{input:1,output:1,cacheRead:0,cacheWrite:0,totalTokens:2,cost:{input:0,output:0,cacheRead:0,cacheWrite:0,total:0}},stopReason:'stop'});
manager.appendSessionInfo('未命名会话');
manager.branch(a);
const other=SessionManager.create(root+'/other-project');
other.appendMessage({role:'user',content:'Other synthetic task',timestamp:4000});
other.appendMessage({role:'assistant',content:[{type:'text',text:'OTHER_SESSION'}],api:'openai-completions',provider:'velune',model:'auto',timestamp:5000,usage:{input:1,output:1,cacheRead:0,cacheWrite:0,totalTokens:2,cost:{input:0,output:0,cacheRead:0,cacheWrite:0,total:0}},stopReason:'stop'});
other.appendSessionInfo('Second native title');
console.log(JSON.stringify({path:manager.getSessionFile(),other:other.getSessionFile()}));
''')
    env={**environment,'HOME':str(root/'home'),'PI_CODING_AGENT_DIR':str(root/'runtime')}
    paths=json.loads(subprocess.check_output([str(args.node),str(sdk),str(root)],env=env,cwd=resources,text=True))
    extension=resources/'pi_virtual_model.mjs'
    extension.write_text(extension.read_text().replace('export default function (pi) {','''export default function (pi) {
pi.registerCommand('manual-tree',{handler:async (_args,ctx)=>{const target=ctx.sessionManager.getEntries().find(entry=>entry.type==='message'&&entry.message.role==='assistant'&&entry.message.content.some(block=>block.type==='text'&&block.text==='BRANCH_B'));await ctx.navigateTree(target.id,{summarize:false});}});
pi.registerCommand('manual-switch',{handler:async (_args,ctx)=>{await ctx.switchSession('''+json.dumps(paths['other'])+''');}});
'''))
    subprocess.run([str(args.node),'--check',str(extension)],check=True)
    shutil.copy2(args.bindings/'velune_bindings.py',root/'bindings/velune_bindings.py')
    shutil.copy2(args.bundle/'Contents/Frameworks/libvelune_bindings.dylib',root/'bindings/libvelune_bindings.dylib')
    spec=importlib.util.spec_from_file_location('velune_bindings',root/'bindings/velune_bindings.py');b=importlib.util.module_from_spec(spec);sys.modules[spec.name]=b;spec.loader.exec_module(b)
    os.environ['HOME']=str(root/'home');os.environ['PATH']=str(args.node.parent)+':/usr/bin:/bin';os.environ['VELUNE_HOME']=str(root/'home/velune')
    app=b.VeluneApplication.open(b.BindingOptions(home_directory=str(root/'home/velune'),resources_directory=str(resources)))
    try:
        model=b.BindingProviderModel(record_key='',provider_model_id='synthetic',nickname='Synthetic',icon=None,context_window=8192,max_output_tokens=4096,reasoning_levels=None,adapter_metadata_json=None)
        draft=b.BindingProviderDraft(id='synthetic',name='Synthetic',protocol=b.BindingGatewayProtocol.CHAT_COMPLETIONS_V1,endpoint=f'http://127.0.0.1:{server.server_port}/v1',models=[model])
        saved=app.save_provider('default',draft,b.BindingAuthenticationEdit.SET_API_KEY(value='synthetic-only')).gateways[0].providers[0]
        runtime=b.BindingRuntimeInstance(enabled=True, id='pi',name='Synthetic Pi',type_id='pi-1.0.2',gateway_id='default',settings={'binary':str(resources/'node_modules/@earendil-works/pi-coding-agent'/json.loads((resources/'node_modules/@earendil-works/pi-coding-agent/package.json').read_text())['bin']['pi']),'nodeBinary':str(args.node),'agentDir':str(root/'runtime')})
        app.upsert_runtime(runtime)
        session=next(item for item in app.list().conversations if item.id=='pi:'+paths['path'])
        before=app.open_conversation('pi',session.id).snapshot
        app.select_model('pi',saved.models[0].record_key)
        first=app.send('pi','/manual-tree').snapshot
        text=lambda view:' '.join(block.text for message in view.messages for block in message.blocks if isinstance(block,b.BindingMessageBlock.TEXT))
        assert first.conversation.id==before.conversation.id and len(first.messages)==len(before.messages)
        assert 'BRANCH_B' in text(first) and 'BRANCH_A' not in text(first) and '/manual-tree' not in text(first)
        assert [message.id for message in first.messages]==[message.id for message in before.messages]
        # Native name may coincide with the displayed untitled placeholder.
        assert first.conversation.title=='未命名会话'
        second=app.send('pi','/manual-switch').snapshot
        assert second.conversation.id=='pi:'+paths['other'] and second.conversation.cwd==str(root/'other-project'), repr((second.conversation.id,second.conversation.cwd,second.conversation.title,paths))
        assert second.conversation.title=='Second native title' and 'OTHER_SESSION' in text(second)
        assert '/manual-switch' not in text(second) and not requests
        revision=second.revision
        for _ in range(3): assert app.snapshot('pi').snapshot.revision==revision
        print(json.dumps({'acceptance':'PASSED','sameIdSameCountBranchResync':True,'stableMessageIds':True,'nativePlaceholderNamePreserved':True,'handledCommandsNotUserMessages':True,'changedSessionIdAndCwd':True,'upstreamRequests':len(requests)}))
    finally: app.shutdown();os.environ.clear();os.environ.update(environment);server.shutdown()
