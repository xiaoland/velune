#!/usr/bin/env python3
"""Manual acceptance of agent-to-agent collaboration through UniFFI.

Two generic ACP runtimes: Alpha, a synthetic agent that calls Velune's
injected collaboration MCP tools when prompted, and Beta, the synthetic ACP
peer of manual-acp-adapter.py. Checks that Alpha starts an ordinary Beta
conversation that appears in the conversation list, that Beta's permission
request is answered through Beta's own conversation, that Alpha reads Beta's
reply, that Alpha messages an existing conversation it found, that a busy
conversation is not sent to, and that an unknown ticket is refused. With
--seed DIR, writes a VELUNE_HOME at DIR/velune where Alpha has started and
heard back from a Beta conversation, and stops there, for checking the Mac
app by hand. No real model, credential, HOME or session is
read; not an automated test.
"""
import argparse
import importlib.util
from pathlib import Path
import runpy
import sys
import tempfile
import time

ALPHA = r'''
import json, os, sys, uuid, urllib.request, urllib.error
from pathlib import Path
root=Path(os.environ['ALPHA_STATE']);root.mkdir(parents=True,exist_ok=True)
session=None; servers=[]; record=None
def emit(value): print(json.dumps(dict(jsonrpc='2.0',**value)),flush=True)
def result(identity,value): emit({'id':identity,'result':value})
def update(value):
    emit({'method':'session/update','params':{'sessionId':session,'update':value}})
    record['updates'].append(value);(root/(session+'.json')).write_text(json.dumps(record))
def say(text): update({'sessionUpdate':'agent_message_chunk','content':{'type':'text','text':text}})
def call(name, arguments, token=None):
    server=next(server for server in servers if server.get('type')=='http')
    headers={header['name']:header['value'] for header in server['headers']}
    if token: headers['Authorization']='Bearer '+token
    headers.update({'Content-Type':'application/json','Accept':'application/json, text/event-stream'})
    body=json.dumps({'jsonrpc':'2.0','id':str(uuid.uuid4()),'method':'tools/call','params':{'name':name,'arguments':arguments}}).encode()
    with urllib.request.urlopen(urllib.request.Request(server['url'],data=body,headers=headers),timeout=120) as response:
        return json.loads(response.read())['result']
def tool(name, arguments):
    value=call(name, arguments)
    if value.get('isError'): raise RuntimeError(value['structuredContent']['error'])
    return value['structuredContent']
def settle(status):
    while status['state'] in ('running','waiting_for_approval') or status.get('timed_out'):
        status=tool('wait_conversation',{'conversation_id':status['conversation_id'],'timeout_ms':30000})
    return status
for line in sys.stdin:
    request=json.loads(line);method=request.get('method');identity=request.get('id');params=request.get('params',{})
    if method=='initialize':
        result(identity,{'protocolVersion':1,'agentInfo':{'name':'alpha','title':'Synthetic Alpha','version':'1'},'authMethods':[],'agentCapabilities':{'loadSession':True,'sessionCapabilities':{'list':{}},'mcpCapabilities':{'http':True}}})
    elif method=='session/new':
        session=str(uuid.uuid4());servers=params.get('mcpServers',[]);record={'cwd':params['cwd'],'updates':[]}
        (root/(session+'.json')).write_text(json.dumps(record));result(identity,{'sessionId':session})
    elif method=='session/list':
        result(identity,{'sessions':[{'sessionId':path.stem,'cwd':json.loads(path.read_text())['cwd'],'title':'Synthetic Alpha','updatedAt':'2026-10-12T00:00:00Z'} for path in sorted(root.glob('*.json'))]})
    elif method=='session/load':
        session=params['sessionId'];servers=params.get('mcpServers',[]);record=json.loads((root/(session+'.json')).read_text())
        for value in record['updates']: emit({'method':'session/update','params':{'sessionId':session,'update':value}})
        result(identity,{})
    elif method=='session/prompt':
        update({'sessionUpdate':'user_message_chunk','content':{'type':'text','text':params['prompt'][0]['text']}})
        words=params['prompt'][0]['text'].split(' ',2)
        try:
            if words[0]=='START':
                agent=next(a for a in tool('list_agents',{})['agents'] if a['name']==words[1])
                assert agent['same_machine_as_you'] and agent['location']=='local', agent
                started=tool('start_conversation',{'runtime_id':agent['runtime_id'],'message':words[2]})
                update({'sessionUpdate':'tool_call','toolCallId':'start-'+uuid.uuid4().hex,'title':'velune: start_conversation','status':'completed','content':[{'type':'content','content':{'type':'text','text':json.dumps(started)}}]})
                status=settle(started)
                say('PEER_REPLIED: %s (%s) %s' % (status['reply'], status['state'], status['conversation_id']))
            elif words[0]=='ASK':
                found=[c for c in tool('list_conversations',{'limit':10})['conversations'] if c['runtime_name']=='Beta']
                status=settle(tool('send_message',{'conversation_id':found[0]['conversation_id'],'message':words[1]}))
                say('PEER_REPLIED: %s (%s) %s' % (status['reply'], status['state'], status['conversation_id']))
            elif words[0]=='BUSY':
                agent=next(a for a in tool('list_agents',{})['agents'] if a['name']=='Beta')
                started=tool('start_conversation',{'runtime_id':agent['runtime_id'],'message':'CANCEL'})
                try:
                    tool('send_message',{'conversation_id':started['conversation_id'],'message':'again'})
                    say('BUSY_ACCEPTED')
                except RuntimeError as error:
                    say('BUSY_REJECTED: %s' % error)
                tool('cancel_turn',{'conversation_id':started['conversation_id']})
                status=settle(started)
                say(' CANCELLED_STATE: %s' % status['state'])
            elif words[0]=='AUTHCHECK':
                try:
                    call('list_agents',{},token='0'*64);say('AUTH_ACCEPTED')
                except urllib.error.HTTPError as error:
                    say('AUTH_REFUSED %d' % error.code)
            else:
                say('ALPHA_PLAIN')
        except Exception as error:
            say('ALPHA_FAILED: %s' % error)
        result(identity,{'stopReason':'end_turn'})
    elif method=='session/cancel': pass
    elif identity is not None: emit({'id':identity,'error':{'code':-32601,'message':'unsupported'}})
'''


def configure(b, app, root):
    """Both runtimes are generic ACP instances started by this Python."""
    peer = runpy.run_path(str(Path(__file__).with_name('manual-acp-adapter.py')), run_name='fixture')['PEER']
    (root / 'alpha.py').write_text(ALPHA)
    # Messages from another agent end in Velune's line naming the sender; the
    # peer keeps the whole text in its history, as a Harness would, and acts
    # on the message.
    marker = "session['records'].append(['user',text]);save()"
    assert marker in peer
    (root / 'beta.py').write_text(peer.replace(marker, marker + ";text=text.split('\\n\\n[Velune 协作消息]',1)[0]"))
    (root / 'project').mkdir(exist_ok=True)
    execution = b.BindingRuntimeExecution(transport=b.BindingRuntimeTransport.LOCAL(), home=b.BindingRuntimeHome.INHERIT(),
                                          remote_resources_directory=None)
    for identity, name, launch in (
            ('alpha', 'Alpha', b.BindingAcpLaunch(args=[str(root / 'alpha.py')], environment={'ALPHA_STATE': str(root / 'alpha-state')})),
            ('beta', 'Beta', b.BindingAcpLaunch(args=[str(root / 'beta.py'), 'literal ; $(not-a-shell)'],
                                                environment={'ACP_MODE': 'full', 'ACP_STATE': str(root / 'beta-state')}))):
        app.upsert_acp_runtime(b.BindingRuntimeInstance(enabled=True, id=identity, name=name, type_id='acp-v1', gateway_id='default',
                                                        settings={'binary': sys.executable}), execution, launch)


def texts(b, snapshot):
    return [block.text for message in snapshot.messages for block in message.blocks
            if isinstance(block, b.BindingMessageBlock.TEXT)]


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    for name in ('library', 'bindings', 'resources'):
        parser.add_argument('--' + name, type=Path, required=True)
    parser.add_argument('--seed', type=Path)
    args = parser.parse_args()
    spec = importlib.util.spec_from_file_location('velune_collaboration', args.bindings / 'velune_bindings.py')
    b = importlib.util.module_from_spec(spec)
    sys.modules[spec.name] = b
    spec.loader.exec_module(b)
    seeding = args.seed is not None
    with tempfile.TemporaryDirectory(prefix='velune-collaboration-') as directory:
        root = args.seed.resolve() if seeding else Path(directory).resolve()
        app = b.VeluneApplication.open(b.BindingOptions(home_directory=str(root / 'velune'), resources_directory=str(args.resources)))
        default = b.BindingModelSelection.RUNTIME_DEFAULT()

        def turn(identity, text, expect, timeout=60, on_poll=None):
            """Send to Alpha and advance the application until Alpha's reply contains `expect`."""
            app.send_turn_with_selection('alpha', identity, 'alpha', default, b.BindingComposedTurn(text=text, effort=None, attachments=[]))
            deadline = time.monotonic() + timeout
            while True:
                snapshot = app.snapshot('alpha', identity).snapshot
                said = texts(b, snapshot)
                if snapshot.run_state == b.BindingRunState.IDLE and said and expect in said[-1]:
                    return said[-1]
                assert not any('ALPHA_FAILED' in item for item in said), said
                assert time.monotonic() < deadline, (snapshot.run_state, said)
                if on_poll:
                    on_poll()
                time.sleep(.05)

        try:
            configure(b, app, root)
            alpha = app.create_conversation_with_selection('alpha', str(root / 'project'), default).snapshot.conversation.id
            answered = []

            def approve():
                for item in app.conversation_activity(0).active:
                    if item.context_runtime_id == 'beta' and item.pending_interactions and not answered:
                        pending = app.snapshot('beta', item.conversation_id).snapshot.pending_interactions
                        app.reply_runtime_interaction('beta', item.conversation_id, pending[0].id,
                                                      b.BindingRuntimeInteractionReply.DECISION(option_id='allow'))
                        answered.append(item.conversation_id)

            said = turn(alpha, 'START Beta PERMIT', 'PEER_REPLIED', on_poll=approve)
            assert answered, 'Beta asked for approval in its own conversation'
            beta = answered[0]
            assert said == 'PEER_REPLIED: ACP_PERMISSION (idle) %s' % beta, said
            links = [block.linked_conversation_id for message in app.snapshot('alpha', alpha).snapshot.messages
                     for block in message.blocks if isinstance(block, b.BindingMessageBlock.TOOL)]
            assert links == [beta], links
            listed = [c.id for c in app.list().conversations]
            assert beta in listed and alpha in listed, listed
            first = app.snapshot('beta', beta).snapshot.messages[0]
            assert first.role == b.BindingMessageRole.USER, first
            sender, body = first.blocks
            assert isinstance(sender, b.BindingMessageBlock.AGENT_SENDER) and sender.conversation_id == alpha \
                and sender.runtime_name == 'Alpha' and sender.title == 'START Beta PERMIT', sender
            assert isinstance(body, b.BindingMessageBlock.TEXT) and body.text == 'PERMIT', body
            title = next(c.title for c in app.list().conversations if c.id == beta)
            assert 'Velune' not in str(title), title
            if seeding:
                # Both conversations stay for the Mac app; Alpha's composer offers the next step.
                app.set_composer_draft(alpha, 'ASK hello')
                print('SEEDED', root / 'velune', 'alpha', alpha, 'beta', beta, flush=True)
                return
            said = turn(alpha, 'ASK hello', 'PEER_REPLIED')
            assert said == 'PEER_REPLIED: ACP_ANSWER (idle) %s' % beta, said
            said = turn(alpha, 'BUSY', 'CANCELLED_STATE')
            assert 'BUSY_REJECTED' in said and 'CANCELLED_STATE: idle' in said, said
            said = turn(alpha, 'AUTHCHECK', 'AUTH_')
            assert said == 'AUTH_REFUSED 401', said
            print('PASS: Alpha started and messaged ordinary Beta conversations, read their replies; '
                  'approval stayed in Beta; busy and unknown tickets were refused', flush=True)
        finally:
            app.shutdown()


if __name__ == '__main__':
    main()
