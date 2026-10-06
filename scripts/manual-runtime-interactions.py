#!/usr/bin/env python3
"""Manual isolated UniFFI-to-stdio acceptance of approvals, answers, cancellation and EOF.
No real runtime credentials, history or upstream services; not a CI entry point.
"""
import argparse, importlib.util, json, os, shutil, sys, tempfile, time
from pathlib import Path

SERVER = r'''
import json, sys
if '--version' in sys.argv:
    print('codex-cli 0.159.3'); sys.exit()
def send(v): print(json.dumps(v), flush=True)
def note(method, params): send({'jsonrpc':'2.0','method':method,'params':{'threadId':'fixture',**params}})
def complete(): note('turn/completed', {'turn':{'id':turn,'status':'completed'}})
turn=''; mode=''
for line in sys.stdin:
    r=json.loads(line); method=r.get('method'); identity=r.get('id')
    if method=='initialize': result={}
    elif method=='thread/start': result={'thread':{'id':'fixture'}}
    elif method=='turn/start':
        turn='turn-'+str(identity);mode=r['params']['input'][0]['text']
        send({'jsonrpc':'2.0','id':identity,'result':{'turn':{'id':turn}}})
        if mode=='exit':
            note('item/agentMessage/delta',{'itemId':'partial','delta':'PRESERVED_PARTIAL'});sys.exit()
        send({'jsonrpc':'2.0','id':'approval','method':'item/commandExecution/requestApproval','params':{'threadId':'fixture','command':'synthetic command','availableDecisions':['accept','decline']}})
        continue
    elif method=='turn/interrupt':
        send({'jsonrpc':'2.0','id':identity,'result':{}});complete();continue
    elif identity=='approval' and 'result' in r:
        if mode=='reject':
            assert r['result']['decision']=='decline';complete();continue
        assert r['result']['decision']=='accept'
        send({'jsonrpc':'2.0','id':'question','method':'item/tool/requestUserInput','params':{'threadId':'fixture','questions':[{'id':'q','question':'Synthetic secret answer?','isSecret':True,'options':[]}]}});continue
    elif identity=='question' and 'result' in r:
        assert r['result']['answers']['q']['answers']==['SYNTHETIC_PRIVATE_ANSWER']
        note('item/agentMessage/delta',{'itemId':'answer','delta':'INTERACTION_PASSED'});complete();continue
    elif identity is None: continue
    else: raise RuntimeError('unexpected operation')
    send({'jsonrpc':'2.0','id':identity,'result':result})
'''

def main():
    parser=argparse.ArgumentParser(description=__doc__)
    for key in ('bundle','bindings','node'): parser.add_argument('--'+key,required=True,type=Path)
    args=parser.parse_args()
    for path in vars(args).values():
        if not path.is_absolute() or not path.exists(): parser.error('absolute existing paths required')
    environment=dict(os.environ)
    with tempfile.TemporaryDirectory(prefix='velune-native-interactions-') as directory:
        root=Path(directory)
        for name in ('bindings','home','runtime','project'): (root/name).mkdir()
        binary=root/'synthetic-codex';binary.write_text('#!'+sys.executable+'\n'+SERVER);binary.chmod(0o700)
        shutil.copy(args.bindings/'velune_bindings.py',root/'bindings')
        (root/'bindings/libvelune_bindings.dylib').symlink_to(args.bundle/'Contents/Frameworks/libvelune_bindings.dylib')
        os.environ.clear();os.environ.update(HOME=str(root/'home'),PATH='/usr/bin:/bin')
        application=None
        try:
            spec=importlib.util.spec_from_file_location('velune_bindings',root/'bindings/velune_bindings.py');b=importlib.util.module_from_spec(spec);sys.modules[spec.name]=b;spec.loader.exec_module(b)
            application=b.VeluneApplication.open(b.BindingOptions(home_directory=str(root/'home'),resources_directory=str(args.bundle/'Contents/Resources')))
            model=b.BindingProviderModel(record_key='',provider_model_id='synthetic',nickname='Synthetic',icon=None,context_window=None,max_output_tokens=None,reasoning_levels=None,adapter_metadata_json=None)
            provider=b.BindingProviderDraft(id='synthetic',name='Synthetic',protocol=b.BindingGatewayProtocol.RESPONSES_V1,endpoint='http://127.0.0.1:9/v1',models=[model])
            application.save_provider('default',provider,b.BindingAuthenticationEdit.SET_API_KEY(value='SYNTHETIC_KEY'))
            record=application.list().gateways[0].providers[0].models[0].record_key
            runtime=b.BindingRuntimeInstance(id='fixture',name='Synthetic',type_id='codex-0.159.3',gateway_id='default',settings={'binary':str(binary),'nodeBinary':str(args.node),'agentDir':str(root/'runtime')})
            application.upsert_runtime(runtime);application.select_runtime('fixture')
            try: application.select_model('fixture',record)
            except b.BindingError: pass
            else: raise AssertionError('model selection without an active conversation succeeded')
            application.create_conversation('fixture',str(root/'project'),record)
            def wait(predicate, allow_error=False):
                deadline=time.monotonic()+8
                while time.monotonic()<deadline:
                    try: snapshot=application.snapshot('fixture').snapshot
                    except b.BindingError:
                        if not allow_error: raise
                        time.sleep(.02);continue
                    if predicate(snapshot): return snapshot
                    time.sleep(.02)
                raise AssertionError('native interaction did not settle')
            def pending(): return wait(lambda s: bool(s.pending_interactions)).pending_interactions[0]
            def rejected(call):
                try: call()
                except b.BindingError: return
                raise AssertionError('invalid reply succeeded')
            application.send('fixture','approve');approval=pending()
            active_id = application.snapshot('fixture').snapshot.conversation.id
            rejected(lambda: application.rename_conversation('fixture', active_id, 'DO_NOT_CHANGE_BUSY'))
            rejected(lambda: application.delete_conversation('fixture', active_id))
            assert application.snapshot('fixture').snapshot.pending_interactions[0].id == approval.id
            assert isinstance(approval.kind,b.BindingInteractionKind.APPROVAL)
            assert [o.id for o in approval.kind.options]==['accept','decline']
            rejected(lambda: application.reply_runtime_interaction('fixture',approval.id,b.BindingRuntimeInteractionReply.DECISION(option_id='not-offered')))
            application.reply_runtime_interaction('fixture',approval.id,b.BindingRuntimeInteractionReply.DECISION(option_id='accept'))
            question=pending();assert isinstance(question.kind,b.BindingInteractionKind.USER_INPUT) and question.kind.questions[0].secret
            rejected(lambda: application.reply_runtime_interaction('fixture',question.id,b.BindingRuntimeInteractionReply.ANSWERS(answers=[])))
            application.reply_runtime_interaction('fixture',question.id,b.BindingRuntimeInteractionReply.ANSWERS(answers=[b.BindingInteractionAnswer(question_id='q',values=['SYNTHETIC_PRIVATE_ANSWER'])]))
            settled=wait(lambda s: s.actions.can_send)
            assert any(block.text=='INTERACTION_PASSED' for m in settled.messages for block in m.blocks if isinstance(block,b.BindingMessageBlock.TEXT))
            application.send('fixture','reject');approval=pending()
            application.reply_runtime_interaction('fixture',approval.id,b.BindingRuntimeInteractionReply.CANCEL())
            wait(lambda s: s.actions.can_send)
            application.send('fixture','cancel');old=pending();application.cancel('fixture');wait(lambda s: s.actions.can_send and not s.pending_interactions)
            rejected(lambda: application.reply_runtime_interaction('fixture',old.id,b.BindingRuntimeInteractionReply.DECISION(option_id='accept')))
            application.send('fixture','exit');failed=wait(lambda s: s.run_state==b.BindingRunState.FAILED,allow_error=True)
            assert any(block.text=='PRESERVED_PARTIAL' for m in failed.messages for block in m.blocks if isinstance(block,b.BindingMessageBlock.TEXT))
            assert not failed.pending_interactions and not failed.actions.can_send
            assert application.list().runtime_instances
            rejected(lambda: application.send('fixture','cannot send on failed transport'))
            application.select_runtime('fixture');application.create_conversation('fixture',str(root/'project'),record);assert application.list().selected_runtime_instance_id=='fixture'
            for path in (root/'home').rglob('*.jsonl'):
                assert 'SYNTHETIC_PRIVATE_ANSWER' not in path.read_text() and 'SYNTHETIC_KEY' not in path.read_text()
            print(json.dumps({'acceptance':'PASSED','explicitApprovalAndPrivateAnswer':True,'busySessionMutationsRejected':True,'invalidAndStaleRepliesRejected':True,'cancelUsesAdvertisedDecline':True,'turnCancellation':True,'terminalFailurePreservesPartialAndAllowsNewConversation':True,'realServicesCalled':False},indent=2))
        finally:
            if application is not None: application.shutdown()
            os.environ.clear();os.environ.update(environment)
if __name__=='__main__': main()
