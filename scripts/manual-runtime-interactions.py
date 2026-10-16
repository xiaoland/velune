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
    elif method=='config/read': result={'config':{'model_provider':'openai','model':'synthetic'}}
    elif method in ('thread/start','thread/resume'): result={'thread':{'id':'fixture'}}
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

CLAUDE_SERVER = r'''
import json, sys
if '--version' in sys.argv:
    print('2.1.286'); sys.exit()
def send(value): print(json.dumps(value), flush=True)
def complete():
    send({'type':'result','subtype':'success','is_error':False})
    send({'type':'system','subtype':'session_state_changed','state':'idle'})
def permission(identity, tool, value):
    send({'type':'control_request','request_id':identity,'request':{'subtype':'can_use_tool','tool_name':tool,'input':value}})
mode=''
for line in sys.stdin:
    record=json.loads(line)
    if record['type']=='control_request':
        subtype=record['request']['subtype']
        response={'models':[{'value':'synthetic','displayName':'Synthetic'}]} if subtype=='initialize' else {}
        send({'type':'control_response','response':{'subtype':'success','request_id':record['request_id'],'response':response}})
        if subtype=='interrupt': complete()
    elif record['type']=='user':
        mode=record['message']['content'][0]['text']
        send({'type':'system','subtype':'session_state_changed','state':'running'})
        permission('approval','Bash',{'command':'synthetic command'})
    elif record['type']=='control_response':
        identity=record['response']['request_id'];response=record['response']['response']
        if identity=='approval':
            if mode=='reject':
                assert response['behavior']=='deny' and response['interrupt'] is True
                complete();continue
            assert response=={'behavior':'allow','updatedInput':{'command':'synthetic command'}}
            permission('question','AskUserQuestion',{'questions':[{'question':'Choose a synthetic value?','options':[{'label':'fixture-answer'}]}]})
        elif identity=='question':
            assert response['behavior']=='allow'
            assert response['updatedInput']['answers']=={'Choose a synthetic value?':'fixture-answer'}
            send({'type':'assistant','message':{'id':'answer','content':[{'type':'text','text':'CLAUDE_INTERACTION_PASSED'}]}})
            complete()
        else: raise RuntimeError('unexpected control response')
    else: raise RuntimeError('unexpected record')
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
            provider=b.BindingProviderDraft(id='synthetic',name='Synthetic',protocol=b.BindingGatewayProtocol.RESPONSES_V1,endpoint='http://127.0.0.1:9/v1',models=[model], quota_limits=[])
            application.save_provider('default',provider,b.BindingAuthenticationEdit.SET_API_KEY(value='SYNTHETIC_KEY'))
            record=application.list().gateways[0].providers[0].models[0].record_key
            runtime=b.BindingRuntimeInstance(enabled=True, id='fixture',name='Synthetic',type_id='codex-0.159.3',gateway_id='default',settings={'binary':str(binary),'nodeBinary':str(args.node),'agentDir':str(root/'runtime')})
            application.upsert_runtime(runtime);application.select_runtime('fixture')
            conversation_id=application.create_conversation('fixture',str(root/'project'),record).snapshot.conversation.id
            def wait(predicate, allow_error=False, runtime_id='fixture'):
                deadline=time.monotonic()+8
                while time.monotonic()<deadline:
                    try: snapshot=application.snapshot(runtime_id,conversation_id).snapshot
                    except b.BindingError:
                        if not allow_error: raise
                        time.sleep(.02);continue
                    if predicate(snapshot): return snapshot
                    time.sleep(.02)
                raise AssertionError('native interaction did not settle')
            def pending(runtime_id='fixture'): return wait(lambda s: bool(s.pending_interactions),runtime_id=runtime_id).pending_interactions[0]
            def rejected(call):
                try: call()
                except b.BindingError: return
                raise AssertionError('invalid reply succeeded')
            application.send_turn('fixture',conversation_id,'fixture',record,'approve');approval=pending()
            active_id = application.snapshot('fixture',conversation_id).snapshot.conversation.id
            rejected(lambda: application.rename_conversation('fixture', active_id, 'DO_NOT_CHANGE_BUSY'))
            rejected(lambda: application.delete_conversation('fixture', active_id))
            assert application.snapshot('fixture',conversation_id).snapshot.pending_interactions[0].id == approval.id
            assert isinstance(approval.kind,b.BindingInteractionKind.APPROVAL)
            assert [o.id for o in approval.kind.options]==['accept','decline']
            rejected(lambda: application.reply_runtime_interaction('fixture',conversation_id,approval.id,b.BindingRuntimeInteractionReply.DECISION(option_id='not-offered')))
            application.reply_runtime_interaction('fixture',conversation_id,approval.id,b.BindingRuntimeInteractionReply.DECISION(option_id='accept'))
            question=pending();assert isinstance(question.kind,b.BindingInteractionKind.USER_INPUT) and question.kind.questions[0].secret
            rejected(lambda: application.reply_runtime_interaction('fixture',conversation_id,question.id,b.BindingRuntimeInteractionReply.ANSWERS(answers=[])))
            application.reply_runtime_interaction('fixture',conversation_id,question.id,b.BindingRuntimeInteractionReply.ANSWERS(answers=[b.BindingInteractionAnswer(question_id='q',values=['SYNTHETIC_PRIVATE_ANSWER'])]))
            settled=wait(lambda s: s.actions.can_send)
            assert any(block.text=='INTERACTION_PASSED' for m in settled.messages for block in m.blocks if isinstance(block,b.BindingMessageBlock.TEXT))
            application.send_turn('fixture',conversation_id,'fixture',record,'reject');approval=pending()
            application.reply_runtime_interaction('fixture',conversation_id,approval.id,b.BindingRuntimeInteractionReply.CANCEL())
            wait(lambda s: s.actions.can_send)
            application.send_turn('fixture',conversation_id,'fixture',record,'cancel');old=pending();application.cancel('fixture',conversation_id);wait(lambda s: s.actions.can_send and not s.pending_interactions)
            rejected(lambda: application.reply_runtime_interaction('fixture',conversation_id,old.id,b.BindingRuntimeInteractionReply.DECISION(option_id='accept')))
            application.send_turn('fixture',conversation_id,'fixture',record,'exit');failed=wait(lambda s: s.run_state==b.BindingRunState.FAILED,allow_error=True)
            assert any(block.text=='PRESERVED_PARTIAL' for m in failed.messages for block in m.blocks if isinstance(block,b.BindingMessageBlock.TEXT))
            assert not failed.pending_interactions and not failed.actions.can_send
            assert application.list().runtime_instances
            rejected(lambda: application.send_turn('fixture',conversation_id,'fixture',record,'cannot send on failed transport'))
            application.select_runtime('fixture');application.create_conversation('fixture',str(root/'project'),record);assert application.list().selected_runtime_instance_id=='fixture'
            claude_binary=root/'synthetic-claude';claude_binary.write_text('#!'+sys.executable+'\n'+CLAUDE_SERVER);claude_binary.chmod(0o700)
            claude=b.BindingRuntimeInstance(enabled=True,id='claude',name='Synthetic Claude',type_id='claude-code-2.1.286',gateway_id='default',settings={'binary':str(claude_binary),'agentDir':str(root/'claude-runtime')})
            application.upsert_runtime(claude)
            native=b.BindingModelSelection.NATIVE(provider_id=None,model_id='synthetic')
            conversation_id=application.create_conversation_with_selection('claude',str(root/'project'),native).snapshot.conversation.id
            application.send_turn_with_selection('claude',conversation_id,'claude',native,'approve');approval=pending('claude')
            assert isinstance(approval.kind,b.BindingInteractionKind.APPROVAL)
            assert [option.id for option in approval.kind.options]==['allow','deny']
            rejected(lambda: application.reply_runtime_interaction('claude',conversation_id,approval.id,b.BindingRuntimeInteractionReply.DECISION(option_id='not-offered')))
            application.reply_runtime_interaction('claude',conversation_id,approval.id,b.BindingRuntimeInteractionReply.DECISION(option_id='allow'))
            question=pending('claude');assert isinstance(question.kind,b.BindingInteractionKind.USER_INPUT)
            rejected(lambda: application.reply_runtime_interaction('claude',conversation_id,question.id,b.BindingRuntimeInteractionReply.ANSWERS(answers=[])))
            application.reply_runtime_interaction('claude',conversation_id,question.id,b.BindingRuntimeInteractionReply.ANSWERS(answers=[b.BindingInteractionAnswer(question_id='Choose a synthetic value?',values=['fixture-answer'])]))
            settled=wait(lambda s: s.actions.can_send,runtime_id='claude')
            assert any(block.text=='CLAUDE_INTERACTION_PASSED' for message in settled.messages for block in message.blocks if isinstance(block,b.BindingMessageBlock.TEXT))
            application.send_turn_with_selection('claude',conversation_id,'claude',native,'reject');approval=pending('claude')
            application.reply_runtime_interaction('claude',conversation_id,approval.id,b.BindingRuntimeInteractionReply.CANCEL())
            wait(lambda s: s.actions.can_send,runtime_id='claude')
            application.send_turn_with_selection('claude',conversation_id,'claude',native,'cancel');old=pending('claude')
            application.cancel('claude',conversation_id);wait(lambda s: s.actions.can_send and not s.pending_interactions,runtime_id='claude')
            rejected(lambda: application.reply_runtime_interaction('claude',conversation_id,old.id,b.BindingRuntimeInteractionReply.DECISION(option_id='allow')))
            for path in (root/'home').rglob('*.jsonl'):
                assert 'SYNTHETIC_PRIVATE_ANSWER' not in path.read_text() and 'SYNTHETIC_KEY' not in path.read_text()
            print(json.dumps({'acceptance':'PASSED','explicitApprovalAndPrivateAnswer':True,'busySessionMutationsRejected':True,'invalidAndStaleRepliesRejected':True,'cancelUsesAdvertisedDecline':True,'turnCancellation':True,'terminalFailurePreservesPartialAndAllowsNewConversation':True,'claudeSyntheticToolApprovalAndQuestion':True,'claudeSyntheticDenyAndInterrupt':True,'realServicesCalled':False},indent=2))
        finally:
            if application is not None: application.shutdown()
            os.environ.clear();os.environ.update(environment)
if __name__=='__main__': main()
