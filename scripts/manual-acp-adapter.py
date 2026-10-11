#!/usr/bin/env python3
"""Explicit isolated UniFFI acceptance for generic ACP; synthetic peers only, not a CI entry point."""
import argparse
import importlib.util
import json
import os
from pathlib import Path
import runpy
import shutil
import sys
import tempfile
import time

PEER = r'''
import json, os, sys, uuid
from pathlib import Path
assert sys.argv[1:] == ['literal ; $(not-a-shell)'], 'argv changed'
root=Path(os.environ['ACP_STATE']);root.mkdir(parents=True,exist_ok=True)
mode=os.environ['ACP_MODE']; log=root/'events.jsonl'
session=None; pending=None; permission=None; model='alpha'; enabled=True

def record(kind, **values):
    with log.open('a') as stream: stream.write(json.dumps(dict(kind=kind,pid=os.getpid(),home=os.environ['HOME'],**values))+'\n')
def emit(value): print(json.dumps(dict(jsonrpc='2.0',**value)),flush=True)
def result(identity,value): emit({'id':identity,'result':value})
def update(value): emit({'method':'session/update','params':{'sessionId':session['id'],'update':value}})
def save(): (root/(session['id']+'.json')).write_text(json.dumps(session))
def options():
    if mode=='live': return []
    return [
        {'id':'primary-selector','name':'Model','category':'model','type':'select','currentValue':model,'options':[{'group':'fixture','name':'Fixture','options':[{'value':'alpha','name':'Alpha'},{'value':'beta','name':'Beta'}]}]},
        {'id':'second-selector','name':'Second model','category':'model','type':'select','currentValue':'secondary','options':[{'value':'secondary','name':'Secondary'}]},
        {'id':'flag-without-category','name':'Boolean setting','type':'boolean','currentValue':enabled},
        {'id':'future-setting','name':'Future optional configuration','type':'future_type','currentValue':'future'},
    ]
def answer(identity,text='ACP_ANSWER'):
    update({'sessionUpdate':'agent_message_chunk','content':{'type':'text','text':text}})
    session['records'].append(['assistant',text]);save();result(identity,{'stopReason':'end_turn'})
record('start')
for line in sys.stdin:
    request=json.loads(line);method=request.get('method');identity=request.get('id');params=request.get('params',{})
    if method=='initialize':
        capabilities={'loadSession':mode!='live','sessionCapabilities':{'list':{},'delete':{}} if mode=='full' else ({'delete':{}} if mode=='listless' else {})}
        result(identity,{'protocolVersion':1,'agentInfo':{'name':'fixture','title':'Synthetic ACP','version':'1'},'authMethods':[{'id':'external','name':'External login'}],'agentCapabilities':capabilities})
    elif method=='session/new':
        session={'id':str(uuid.uuid4()),'cwd':params['cwd'],'records':[]};save();record('new',session=session['id']);result(identity,{'sessionId':session['id'],'configOptions':options()})
    elif method=='session/list':
        assert mode=='full'
        values=[json.loads(path.read_text()) for path in sorted(root.glob('*.json'))];offset=int(params.get('cursor','0'));chunk=values[offset:offset+1]
        value={'sessions':[{'sessionId':item['id'],'cwd':item['cwd'],'title':'Synthetic ACP','updatedAt':'2026-10-10T00:00:00Z'} for item in chunk]}
        if offset+1<len(values): value['nextCursor']=str(offset+1)
        result(identity,value)
    elif method=='session/load':
        assert mode!='live'
        session=json.loads((root/(params['sessionId']+'.json')).read_text());assert session['cwd']==params['cwd'];record('load',session=session['id'])
        for role,text in session['records']:
            update({'sessionUpdate':'user_message_chunk' if role=='user' else 'agent_message_chunk','content':{'type':'text','text':text}})
        result(identity,{'configOptions':options()})
    elif method=='session/set_config_option':
        record('config',config=params['configId'],value=params['value'])
        if params['configId']=='primary-selector': model=params['value']
        elif params['configId']=='flag-without-category': assert params['type']=='boolean';enabled=params['value']
        else: raise AssertionError('unknown config')
        result(identity,{'configOptions':options()})
    elif method=='session/prompt':
        assert params['sessionId']==session['id'];text=params['prompt'][0]['text'];record('prompt',session=session['id'],text=text,model=model)
        session['records'].append(['user',text]);save()
        if text=='EOF':
            update({'sessionUpdate':'agent_message_chunk','content':{'type':'text','text':'PARTIAL'}});sys.exit(0)
        elif text=='PERMIT':
            pending=identity;permission='permission-'+uuid.uuid4().hex
            emit({'id':permission,'method':'session/request_permission','params':{'sessionId':session['id'],'toolCall':{'toolCallId':'tool','title':'Synthetic permission','kind':'read','status':'pending'},'options':[{'optionId':'allow','name':'Allow','kind':'allow_once'},{'optionId':'deny','name':'Deny','kind':'reject_once'}]}})
        elif text=='CANCEL': pending=identity
        else: answer(identity)
    elif method=='session/cancel':
        if pending is not None: result(pending,{'stopReason':'cancelled'});pending=None
    elif method=='session/delete':
        assert mode!='live';(root/(params['sessionId']+'.json')).unlink();result(identity,{})
    elif method is None and identity==permission:
        assert request['result']['outcome']=={'outcome':'selected','optionId':'allow'};answer(pending,'ACP_PERMISSION');pending=permission=None
    elif identity is not None: emit({'id':identity,'error':{'code':-32601,'message':'unsupported'}})
'''


def main():
    parser=argparse.ArgumentParser(description=__doc__)
    for key in ('library','resources','bindings'): parser.add_argument('--'+key,required=True,type=Path)
    args=parser.parse_args()
    for path in vars(args).values():
        if not path.is_absolute() or not path.exists(): parser.error('absolute existing paths required')
    environment=dict(os.environ);application=None
    with tempfile.TemporaryDirectory(prefix='velune-acp-') as directory:
        root=Path(directory)
        for name in ('bindings','home','project','empty-resources'): (root/name).mkdir()
        peer=root/'peer.py';peer.write_text(PEER)
        shutil.copy(args.bindings/'velune_bindings.py',root/'bindings')
        (root/'bindings'/args.library.name).symlink_to(args.library)
        os.environ.clear();os.environ.update(HOME=str(root/'home'),PATH='/usr/bin:/bin')
        try:
            spec=importlib.util.spec_from_file_location('velune_bindings',root/'bindings/velune_bindings.py');b=importlib.util.module_from_spec(spec);sys.modules[spec.name]=b;spec.loader.exec_module(b)
            options=b.BindingOptions(home_directory=str(root/'application'),resources_directory=str(root/'empty-resources'))
            application=b.VeluneApplication.open(options)
            default=b.BindingModelSelection.RUNTIME_DEFAULT()
            execution=b.BindingRuntimeExecution(transport=b.BindingRuntimeTransport.LOCAL(),home=b.BindingRuntimeHome.MANAGED(remote_root=None),remote_resources_directory=None)
            for identity in ('full','listless','live'):
                runtime=b.BindingRuntimeInstance(enabled=True,id=identity,name=identity,type_id='acp-v1',gateway_id='default',settings={'binary':sys.executable})
                application.upsert_acp_runtime(runtime,execution,b.BindingAcpLaunch(args=[str(peer),'literal ; $(not-a-shell)'],environment={'ACP_MODE':identity,'ACP_STATE':str(root/identity)}))
                assert application.acp_launch(identity).args==[str(peer),'literal ; $(not-a-shell)']
            def records(identity):
                path=root/identity/'events.jsonl'
                return [json.loads(line) for line in path.read_text().splitlines()] if path.exists() else []
            def count(identity,kind): return sum(record['kind']==kind for record in records(identity))
            def reject(call,contains=None):
                try: call()
                except b.BindingError as error:
                    if contains: assert contains in str(error),str(error)
                    return
                raise AssertionError('expected rejection')
            def wait(runtime,identity,predicate=lambda snapshot:snapshot.actions.can_send,allow_error=False):
                deadline=time.monotonic()+10
                while time.monotonic()<deadline:
                    try: snapshot=application.snapshot(runtime,identity).snapshot
                    except b.BindingError:
                        if not allow_error: raise
                        time.sleep(.02);continue
                    if snapshot and predicate(snapshot): return snapshot
                    time.sleep(.02)
                raise AssertionError('ACP operation did not settle')
            info=application.acp_runtime_info('full')
            assert info.capabilities.load_session and info.capabilities.list_sessions
            assert info.agent_info.name=='fixture' and info.auth_methods[0].id=='external'
            reject(lambda:application.runtime_models('full'),'会话')
            assert count('full','new')==0
            created=application.create_conversation_with_selection('live',str(root/'project'),default).snapshot;identity=created.conversation.id;live_id=identity
            assert not application.acp_runtime_info('live').capabilities.load_session
            assert application.runtime_models('live')==[]
            for text in ('FIRST','SECOND'):
                application.send_turn_with_selection('live',identity,'live',default,text);wait('live',identity)
            assert count('live','new')==1 and count('live','load')==0
            assert len({record['pid'] for record in records('live') if record['kind']=='prompt'})==1
            reject(lambda:application.send_turn_with_selection('live',identity,'full',default,'NO_HANDOFF'),'历史')
            assert count('full','new')==0
            application.send_turn_with_selection('live',identity,'live',default,'STILL_LIVE');wait('live',identity)
            created=application.create_conversation_with_selection('full',str(root/'project'),default).snapshot;identity=created.conversation.id;first_full_id=identity
            models=application.runtime_models('full');assert [model.selection.model_id for model in models]==['alpha','beta']
            config=application.acp_session_config('full',identity);assert config[0].choices[0].group=='fixture'
            application.set_acp_session_config('full',identity,'flag-without-category',b.BindingAcpConfigValue.BOOLEAN(False))
            reject(lambda:application.set_acp_session_config('full',identity,'primary-selector',b.BindingAcpConfigValue.SELECT('missing')))
            beta=b.BindingModelSelection.NATIVE(provider_id=None,model_id='beta')
            for text in ('MODEL_SELECTION','SECOND_TURN'):
                application.send_turn_with_selection('full',identity,'full',beta,text);wait('full',identity)
            assert count('full','new')==1 and count('full','load')==0
            assert len({record['pid'] for record in records('full') if record['kind']=='prompt'})==1
            reject(lambda:application.send_turn_with_selection('full',identity,'full',b.BindingModelSelection.MANAGED(model_record_key='irrelevant'),'NO_MANAGED'),'managed')
            application.send_turn_with_selection('full',identity,'full',default,'PERMIT')
            pending=wait('full',identity,lambda snapshot:bool(snapshot.pending_interactions)).pending_interactions[0]
            reject(lambda:application.reply_runtime_interaction('full',identity,pending.id,b.BindingRuntimeInteractionReply.DECISION(option_id='missing')))
            application.reply_runtime_interaction('full',identity,pending.id,b.BindingRuntimeInteractionReply.DECISION(option_id='allow'));wait('full',identity)
            application.send_turn_with_selection('full',identity,'full',default,'CANCEL');application.cancel('full',identity);wait('full',identity)
            reject(lambda:application.reply_runtime_interaction('full',identity,pending.id,b.BindingRuntimeInteractionReply.DECISION(option_id='allow')))
            before=count('live','new')
            reject(lambda:application.send_turn_with_selection('full',identity,'live',default,'NO_TARGET'),'历史')
            assert count('live','new')==before and application.snapshot('full',identity).snapshot.actions.can_send
            created=application.create_conversation_with_selection('listless',str(root/'project'),default).snapshot;listless_id=created.conversation.id
            application.send_turn_with_selection('listless',listless_id,'listless',default,'RELOAD_ME');live_replay_ids=[message.id for message in wait('listless',listless_id).messages]
            application.shutdown();application=b.VeluneApplication.open(options)
            reject(lambda:application.open_acp_conversation('live',live_id.split(':',1)[1],str(root/'project')),'历史')
            assert count('live','new')==1
            reopened=application.open_acp_conversation('listless',listless_id.split(':',1)[1],str(root/'project')).snapshot
            assert reopened.conversation.id==listless_id and any(block.text=='RELOAD_ME' for message in reopened.messages for block in message.blocks if isinstance(block,b.BindingMessageBlock.TEXT))
            loaded_ids=[message.id for message in reopened.messages]
            assert loaded_ids==live_replay_ids, 'ACP live-to-replay message IDs changed'
            reloaded=application.open_acp_conversation('listless',listless_id.split(':',1)[1],str(root/'project')).snapshot
            assert [message.id for message in reloaded.messages]==loaded_ids, 'identical ACP replay changed message IDs'
            assert count('listless','new')==1
            application.send_turn_with_selection('listless',listless_id,'listless',default,'AFTER_REOPEN');wait('listless',listless_id)
            application.send_turn_with_selection('listless',listless_id,'listless',default,'EOF')
            failed=wait('listless',listless_id,lambda snapshot:snapshot.run_state==b.BindingRunState.FAILED,True)
            assert not failed.actions.can_send and any(block.text=='PARTIAL' for message in failed.messages for block in message.blocks if isinstance(block,b.BindingMessageBlock.TEXT))
            reject(lambda:application.send_turn_with_selection('listless',listless_id,'listless',default,'NO_RETRY'))
            application.shutdown();application=b.VeluneApplication.open(b.BindingOptions(home_directory=str(root/'application'),resources_directory=str(args.resources)))
            claude_peer=runpy.run_path(str(Path(__file__).with_name('manual-runtime-selection.py')))['PEER']
            claude=root/'claude-peer';claude.write_text(claude_peer.replace('__PYTHON__',sys.executable).replace('__LOG__',repr(str(root/'claude-events.jsonl'))));claude.chmod(0o700)
            application.upsert_runtime(b.BindingRuntimeInstance(enabled=True,id='claude',name='Claude fixture',type_id='claude-code-2.1.286',gateway_id='default',settings={'binary':str(claude),'agentDir':str(root/'claude')}))
            source=application.create_conversation_with_selection('full',str(root/'project'),default).snapshot;identity=source.conversation.id
            application.send_turn_with_selection('full',identity,'full',default,'ACP_ORIGIN');wait('full',identity)
            native=b.BindingModelSelection.NATIVE(provider_id=None,model_id='native-model')
            application.send_turn_with_selection('full',identity,'claude',native,'CLAUDE_HANDOFF');wait('claude',identity)
            application.send_turn_with_selection('full',identity,'full',default,'ACP_RETURN');wait('full',identity)
            application.shutdown();application=b.VeluneApplication.open(b.BindingOptions(home_directory=str(root/'application'),resources_directory=str(args.resources)))
            reopened=application.open_conversation('full',identity).snapshot
            texts=[block.text for message in reopened.messages for block in message.blocks if isinstance(block,b.BindingMessageBlock.TEXT)]
            assert all(text in texts for text in ('ACP_ORIGIN','CLAUDE_HANDOFF','ACP_RETURN'))
            links=json.loads((root/'application/conversation-links.json').read_text())['links'];assert all(Path(segment['cwd']).resolve()==(root/'project').resolve() for segment in links[0]['segments']), [(segment['runtimeInstanceId'],segment['cwd']) for segment in links[0]['segments']]
            assert not any('ACP_ORIGIN' in json.dumps(link) for link in links)
            rows=application.list().conversations
            assert any(row.id==first_full_id and row.can_delete for row in rows)
            application.delete_conversation('full',first_full_id)
            assert not any(row.id==first_full_id for row in application.list().conversations)
            draft=application.create_conversation_with_selection('claude',str(root/'project'),native).snapshot
            application.send_turn_with_selection('claude',draft.conversation.id,'listless',default,'LISTLESS_ALIAS_DELETE')
            wait('listless',draft.conversation.id)
            application.delete_conversation('listless',draft.conversation.id)
            assert not any(row.id==draft.conversation.id for row in application.list().conversations)
            application.shutdown();application=None
            assert all(Path(record['home']).is_relative_to(root/'application/runtime-homes') for mode in ('live','full','listless') for record in records(mode))
            print('PASS: generic command/args, managed HOME, negotiated capabilities, no hidden new, live no-load multi-turn, session config/model in same process, permissions/cancel/EOF, no-list explicit load, unsupported durable handoff retains source, ACP<->Claude handoff/reopen')
        finally:
            if application is not None:
                try: application.shutdown()
                except Exception: pass
            os.environ.clear();os.environ.update(environment)

if __name__=='__main__': main()
