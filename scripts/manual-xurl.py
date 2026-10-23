#!/usr/bin/env python3
"""隔离验收 xURL 地址、官方 CLI 与 MCP。只使用临时 HOME 和合成 Harness。"""
import argparse
import base64
import importlib.util
import json
import os
from pathlib import Path
import runpy
import subprocess
import sys
import tempfile
import time

SESSION = "019c871c-b1f9-7f60-9c4f-87ed09f13592"
OTHER = "019c871c-b1f9-7f60-9c4f-87ed09f13593"

CODEX = r'''
import json, os, sys
from pathlib import Path
root=Path(os.environ['CODEX_HOME'])
if '--version' in sys.argv: print('codex-cli 0.159.3');sys.exit()
if 'exec' in sys.argv:
    (root/'write-args.json').write_text(json.dumps(sys.argv[1:]))
    print(json.dumps({'type':'thread.started','thread_id':'019c871c-b1f9-7f60-9c4f-87ed09f13592'}))
    print(json.dumps({'type':'item.completed','item':{'type':'agent_message','text':'XURL_NATIVE_WRITE'}}))
    print(json.dumps({'type':'turn.completed','usage':{}}));sys.exit()
for line in sys.stdin:
    request=json.loads(line);method=request.get('method');identity=request.get('id')
    if identity is None: continue
    if method=='thread/list':
        data=json.loads((root/'threads.json').read_text())
        value={'data':data if not request['params'].get('archived') else [],'nextCursor':None}
    elif method=='initialize': value={}
    elif method=='config/read': value={'config':{}}
    else: raise RuntimeError(method)
    print(json.dumps({'jsonrpc':'2.0','id':identity,'result':value}),flush=True)
'''

CLAUDE = r'''
import json, sys
if '--version' in sys.argv: print('2.1.286');sys.exit()
def send(value): print(json.dumps(value),flush=True)
def done():
    send({'type':'assistant','message':{'id':'answer','content':[{'type':'text','text':'URI_SENT'}]}})
    send({'type':'result','subtype':'success','is_error':False})
    send({'type':'system','subtype':'session_state_changed','state':'idle'})
servers=[]
for line in sys.stdin:
    record=json.loads(line)
    if record['type']=='control_request':
        subtype=record['request']['subtype'];response={}
        if subtype=='initialize': response={'models':[{'value':'synthetic','displayName':'Synthetic'}]}
        if subtype=='mcp_status': response={'mcpServers':[{'name':name,'status':'connected'} for name in servers]}
        if subtype=='mcp_set_servers': servers=list(record['request']['servers']);response={'errors':{}}
        send({'type':'control_response','response':{'subtype':'success','request_id':record['request_id'],'response':response}})
        if subtype=='interrupt': done()
    elif record['type']=='user':
        text=record['message']['content'][0]['text']
        send({'type':'system','subtype':'session_state_changed','state':'running'})
        if text.startswith('HOLD'): pass
        elif text.startswith('APPROVE'):
            send({'type':'control_request','request_id':'approval','request':{'subtype':'can_use_tool','tool_name':'Bash','input':{'command':'synthetic'}}})
        else: done()
    elif record['type']=='control_response': done()
'''


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    for name in ('bindings', 'resources', 'xurl', 'node', 'pi'):
        parser.add_argument('--' + name, type=Path, required=True)
    parser.add_argument('--seed', type=Path)
    args = parser.parse_args()
    spec = importlib.util.spec_from_file_location('velune_xurl_bindings', args.bindings / 'velune_bindings.py')
    b = importlib.util.module_from_spec(spec)
    sys.modules[spec.name] = b
    spec.loader.exec_module(b)
    collaboration = runpy.run_path(str(Path(__file__).with_name('manual-collaboration.py')), run_name='fixture')
    with tempfile.TemporaryDirectory(prefix='velune-xurl-') as temporary:
        root = (args.seed or Path(temporary)).resolve()
        root.mkdir(exist_ok=True)
        app = b.VeluneApplication.open(b.BindingOptions(home_directory=str(root / 'velune'), resources_directory=str(args.resources)))
        try:
            collaboration['configure'](b, app, root)
            alpha = (root / 'alpha.py').read_text().replace("if words[0]=='START':", """if words[0]=='XURL':
                import base64
                operation,arguments=json.loads(base64.b64decode(words[1]))
                say('XURL_RESULT:'+json.dumps(call(operation,arguments)))
            elif words[0]=='START':""")
            (root / 'alpha.py').write_text(alpha)
            project = root / 'project'
            execution = b.BindingRuntimeExecution(transport=b.BindingRuntimeTransport.LOCAL(), home=b.BindingRuntimeHome.EXISTING(str(root)), remote_resources_directory=None)
            for family, source in [('codex', CODEX), ('claude', CLAUDE)]:
                binary = root / ('synthetic-' + family)
                binary.write_text('#!' + sys.executable + '\n' + source)
                binary.chmod(0o700)
            for name in ('claude-a', 'claude-b'):
                directory = root / name / 'projects' / 'synthetic-project'
                directory.mkdir(parents=True)
                records = [
                    {'type':'user','uuid':'user-1','sessionId':SESSION,'cwd':str(project),'timestamp':'2026-10-23T01:00:00Z','message':{'role':'user','content':'SYNTHETIC 甲🙂 question'}},
                    {'type':'assistant','uuid':'answer-1','parentUuid':'user-1','sessionId':SESSION,'cwd':str(project),'timestamp':'2026-10-23T01:00:01Z','message':{'role':'assistant','content':[{'type':'text','text':'SYNTHETIC answer'}]}},
                    {'type':'custom-title','sessionId':SESSION,'customTitle':'xURL fixture'},
                ]
                (directory / (SESSION + '.jsonl')).write_text('\n'.join(map(json.dumps, records)) + '\n')
                child = directory / SESSION / 'subagents'
                child.mkdir(parents=True)
                (child / 'agent-a123.jsonl').write_text(json.dumps({**records[0], 'isSidechain':True, 'agentId':'a123'}) + '\n')
                app.upsert_runtime_with_execution(b.BindingRuntimeInstance(enabled=True, id=name, name=name, type_id='claude-code-2.1', gateway_id='default', settings= {'binary':str(root/'synthetic-claude'),'agentDir':str(root/name)}), execution)
            for name, identity in [('codex-a', SESSION), ('codex-b', OTHER)]:
                directory = root / name
                (directory / 'sessions').mkdir(parents=True)
                (directory / 'threads.json').write_text(json.dumps([{'id':identity,'name':'xURL Codex','preview':'synthetic','cwd':str(project),'createdAt':1,'updatedAt':2,'source':'cli'}]))
                (directory / 'sessions' / ('rollout-2026-10-23T01-00-00-' + identity + '.jsonl')).write_text('\n'.join(map(json.dumps, [
                    {'type':'session_meta','payload':{'id':identity,'cwd':str(project),'timestamp':'2026-10-23T01:00:00Z'}},
                    {'type':'response_item','payload':{'type':'message','role':'user','content':[{'type':'input_text','text':'SYNTHETIC Codex'}]}},
                ])) + '\n')
                app.upsert_runtime_with_execution(b.BindingRuntimeInstance(enabled=True, id=name, name=name, type_id='codex-0.159', gateway_id='default', settings= {'binary':str(root/'synthetic-codex'),'nodeBinary':str(args.node),'agentDir':str(directory)}), execution)
            pi_directory = root / 'pi' / 'sessions' / 'synthetic-project'
            pi_directory.mkdir(parents=True)
            (pi_directory / ('2026-10-23T01-00-00_' + SESSION + '.jsonl')).write_text('\n'.join(map(json.dumps, [
                {'type':'session','version':3,'id':SESSION,'timestamp':'2026-10-23T01:00:00Z','cwd':str(project)},
                {'type':'message','id':'1234abcd','parentId':None,'timestamp':'2026-10-23T01:00:00Z','message':{'role':'user','content':[{'type':'text','text':'SYNTHETIC Pi'}],'timestamp':1792717200000}},
            ])) + '\n')
            app.upsert_runtime_with_execution(b.BindingRuntimeInstance(enabled=True, id='pi', name='Pi', type_id='pi-1.0', gateway_id='default', settings= {'binary':str(args.pi),'nodeBinary':str(args.node),'agentDir':str(root/'pi')}), execution)
            uri = 'agents://claude/' + SESSION
            found = app.resolve_agent_uri(uri, None)
            assert not found.failures and len(found.conversations) == 2, found
            assert {item.agents_uri for item in found.conversations} == {uri}
            assert len(app.resolve_agent_uri(uri, 'claude-b').conversations) == 1
            assert len(app.resolve_agent_uri('agents://claude?q=xURL&limit=1', None).conversations) == 1
            absent = app.resolve_agent_uri('agents://claude/' + OTHER, None)
            assert not absent.conversations and not absent.failures, [f.detail for f in absent.failures]
            codex = app.resolve_agent_uri('agents://codex/' + SESSION, None)
            assert not codex.failures and len(codex.conversations) == 1, codex
            assert len(app.resolve_agent_uri('agents://codex?q=xURL', None).conversations) == 2
            missing = app.resolve_agent_uri('agents://codex/00000000-0000-0000-0000-000000000000', None)
            assert not missing.conversations and not missing.failures, missing
            pi = app.resolve_agent_uri('agents://pi/' + SESSION, 'pi')
            assert not pi.failures and len(pi.conversations) == 1, [f.detail for f in pi.failures]
            assert pi.conversations[0].agents_uri == 'agents://pi/' + SESSION
            original = next(pi_directory.glob('*.jsonl'))
            duplicate = pi_directory / ('duplicate-' + original.name)
            duplicate.write_bytes(original.read_bytes())
            assert len(app.resolve_agent_uri('agents://pi/' + SESSION, 'pi').conversations) == 2
            duplicate.unlink()
            child = app.resolve_agent_uri(uri + '/a123', 'claude-a')
            assert len(child.conversations) == 1 and child.conversations[0].conversation.parent_conversation_id, child
            environment = {'HOME':str(root), 'PATH':'/usr/bin:/bin', 'CLAUDE_CONFIG_DIR':str(root/'claude-a'), 'CODEX_HOME':str(root/'codex-a'), 'PI_CODING_AGENT_DIR':str(root/'pi'), 'XURL_CODEX_BIN':str(root/'synthetic-codex')}
            for address, expect in [(uri, 'SYNTHETIC'), ('agents://codex/'+SESSION, 'SYNTHETIC Codex'), ('agents://pi/'+SESSION, 'SYNTHETIC Pi')]:
                result = subprocess.run([str(args.xurl), address], env=environment, text=True, capture_output=True, timeout=20)
                assert result.returncode == 0, result.stderr
                assert expect in result.stdout, result.stdout
            subprocess.run([str(args.xurl), 'agents://codex/'+SESSION, '-d', 'SYNTHETIC write'], env=environment, capture_output=True, timeout=20, check=True)
            assert 'resume' in json.loads((root/'codex-a/write-args.json').read_text())
            alpha_id = app.create_conversation_with_selection('alpha', str(project), None).snapshot.conversation.id

            def call(name, arguments, error=False):
                payload = base64.b64encode(json.dumps([name, arguments]).encode()).decode()
                app.send_turn_with_selection('alpha', alpha_id, 'alpha', None, b.BindingComposedTurn(text='XURL '+payload, effort=None, attachments=[]))
                deadline = time.monotonic() + 50
                while time.monotonic() < deadline:
                    snapshot = app.snapshot('alpha', alpha_id).snapshot
                    if snapshot.actions.can_send:
                        text = next(text for text in reversed(collaboration['texts'](b, snapshot)) if text.startswith('XURL_RESULT:'))
                        value = json.loads(text[len('XURL_RESULT:'):])
                        assert bool(value.get('isError')) == error, value
                        return value['structuredContent']
                    time.sleep(.03)
                raise AssertionError('MCP response timed out')

            assert 'xURL' in call('xurl_instructions', {})['instructions']
            agents = call('list_agents', {})['agents']
            native = next(item for item in agents if item['runtime_id'] == 'claude-a')['xurl']
            assert native['executable'] == str(args.resources/'xurl/bin/xurl'), native
            assert native['environment']['CLAUDE_CONFIG_DIR'] == str(root/'claude-a')
            pi_cli = next(item for item in agents if item['runtime_id'] == 'pi')['xurl']
            assert pi_cli['environment']['PATH'].split(os.pathsep)[0] == str(args.node.parent)
            # An unreadable instance makes even a single discovered match inconclusive.
            index = root / 'codex-b/threads.json'
            saved = index.read_text()
            try:
                index.write_text('invalid synthetic index')
                partial = app.resolve_agent_uri('agents://codex/' + SESSION, None)
                assert len(partial.conversations) == 1 and len(partial.failures) == 1
                call('send_message', {'conversation_id':'agents://codex/'+SESSION,'message':'forbidden'}, error=True)
            finally:
                index.write_text(saved)
            call('send_message', {'conversation_id':uri,'message':'hello'}, error=True)
            resolved = call('resolve_agent_uri', {'uri':uri,'runtime_id':'claude-a'})
            assert len(resolved['conversations']) == 1
            first = call('read_conversation', {'conversation_id':uri,'runtime_id':'claude-a','limit':12})
            rest = call('read_conversation', {'conversation_id':uri,'runtime_id':'claude-a','offset':first['next_offset'],'expected_revision':first['revision']})
            assert 'SYNTHETIC 甲🙂 question' in first['text'] + rest['text']
            call('read_conversation', {'conversation_id':uri,'runtime_id':'claude-a','expected_revision':'stale'}, error=True)
            call('send_message', {'conversation_id':uri+'/a123','runtime_id':'claude-a','message':'forbidden'}, error=True)
            # Resume uses the native default; all native I/O goes to our synthetic executable.
            status = call('send_message', {'conversation_id':uri,'runtime_id':'claude-a','message':'hello'})
            status = call('wait_conversation', {'conversation_id':status['conversation_id'],'timeout_ms':30000})
            assert status['reply'] == 'URI_SENT', status
            call('send_message', {'conversation_id':uri,'runtime_id':'claude-a','message':'HOLD'})
            call('send_message', {'conversation_id':uri,'runtime_id':'claude-a','message':'again'}, error=True)
            call('cancel_turn', {'conversation_id':uri,'runtime_id':'claude-a'})
            call('send_message', {'conversation_id':uri,'runtime_id':'claude-a','message':'APPROVE'})
            status = call('get_conversation', {'conversation_id':uri,'runtime_id':'claude-a'})
            assert status['state'] == 'waiting_for_approval', status
            call('send_message', {'conversation_id':uri,'runtime_id':'claude-a','message':'again'}, error=True)
            call('cancel_turn', {'conversation_id':uri,'runtime_id':'claude-a'})
            print(json.dumps({'result':'通过','multi_instance':True,'pi_native_uuid':True,'official_cli_read_write':True,'mcp_uri_read_send':True,'busy_approval_child_rejected':True,'real_services':False}, ensure_ascii=False))
        finally:
            app.shutdown_stopping_work()


if __name__ == '__main__':
    main()
