#!/usr/bin/env python3
"""Manual acceptance of a native sub-agent that outlives its parent's turn, through UniFFI.

Velune runs real Codex 0.159.3 in a managed HOME inside a temporary
VELUNE_HOME against a loopback upstream: the parent starts a sub-agent with
`spawn_agent` and ends its turn without waiting; the upstream holds the
sub-agent's first request until the parent is idle, then the sub-agent runs a
command that needs approval and answers. Checks, on Codex's own provider (a
native turn), that the parent's snapshot names the sub-agent, that activity
reports the sub-agent running under its parent while the parent is idle, that
the sub-agent's approval comes to the idle parent and its answer reaches the
sub-agent, and that the sub-agent then ends. Through Velune's gateway (a turn
that uses the gateway only for itself) it checks that the sub-agent outlives
the parent's turn: a next turn on the same model reuses the process, one that
needs another process is refused while the sub-agent runs, and once the
sub-agent has ended the turn's app-server ends, the Harness configuration is
unchanged and the parent goes on with a native turn. With
--seed DIR the VELUNE_HOME is kept at DIR/velune for checking the Mac app by
hand. Only loopback synthetic responses are used: no real model, credential,
HOME or session is read; this is not an automated test.
"""
import argparse
import hashlib
import importlib.util
import itertools
import json
import os
import pathlib
import subprocess
import sys
import tempfile
import threading
import time
from http.server import BaseHTTPRequestHandler, ThreadingHTTPServer

# Codex keeps a response item's id as the message's.
ITEMS = itertools.count()
# Set once the parent is idle: the held sub-agent request may then go on.
RELEASE = threading.Event()


def responses_reply(body):
    """The parent spawns a sub-agent and ends; the sub-agent waits for the
    release, runs a command that needs approval, then answers."""
    text = json.dumps(body.get('input', []))
    outputs = [item for item in body.get('input', []) if item.get('type') == 'function_call_output']
    if 'SUBAGENT_TASK' in text and 'PARENT_TASK' not in text:
        if outputs:
            return {'type': 'message', 'text': 'SUB_DONE'}
        RELEASE.wait(120)
        return {'type': 'function_call', 'name': 'exec_command', 'call_id': 'call_escalate', 'namespace': None,
                'arguments': json.dumps({'cmd': 'echo SUB_APPROVED', 'sandbox_permissions': 'require_escalated',
                                         'justification': 'Synthetic sub-agent check'})}
    if 'PARENT_TASK' not in text:
        return {'type': 'message', 'text': 'OTHER'}
    if outputs:
        return {'type': 'message', 'text': 'PARENT_DONE'}
    assert 'spawn_agent' in json.dumps(body.get('tools', [])), 'Codex offered no spawn_agent tool'
    return {'type': 'function_call', 'name': 'spawn_agent', 'call_id': 'call_spawn',
            'arguments': json.dumps({'message': 'SUBAGENT_TASK'})}


class Upstream(BaseHTTPRequestHandler):
    protocol_version = 'HTTP/1.1'

    def log_message(self, *_):
        pass

    def do_POST(self):
        body = json.loads(self.rfile.read(int(self.headers.get('Content-Length', '0'))) or b'{}')
        if not self.path.split('?')[0].endswith('/responses'):
            self.send_response(404)
            self.send_header('Content-Length', '0')
            self.end_headers()
            return
        out = responses_reply(body)
        if out['type'] == 'message':
            item = {'id': 'item-%d' % next(ITEMS), 'type': 'message', 'role': 'assistant', 'status': 'completed',
                    'content': [{'type': 'output_text', 'text': out['text'], 'annotations': []}]}
        else:
            item = {'id': 'fc', 'type': 'function_call', 'status': 'completed', 'name': out['name'],
                    'arguments': out['arguments'], 'call_id': out['call_id']}
            if out.get('namespace', 'multi_agent_v1'):
                item['namespace'] = out.get('namespace', 'multi_agent_v1')
        response = {'id': 'response', 'object': 'response', 'status': 'completed', 'output': [item],
                    'usage': {'input_tokens': 1, 'output_tokens': 1, 'total_tokens': 2}}
        self.send_response(200)
        self.send_header('Content-Type', 'text/event-stream')
        self.send_header('Connection', 'close')
        self.end_headers()
        for event in [{'type': 'response.created', 'response': {**response, 'status': 'in_progress', 'output': []}},
                      {'type': 'response.output_item.added', 'output_index': 0, 'item': item},
                      {'type': 'response.output_item.done', 'output_index': 0, 'item': item},
                      {'type': 'response.completed', 'response': response}]:
            self.wfile.write(('event: %s\ndata: %s\n\n' % (event['type'], json.dumps(event))).encode())
        self.wfile.flush()


def app_servers():
    """Codex app-servers Velune runs for this script."""
    children = {}
    for line in subprocess.run(['ps', '-A', '-o', 'pid=,ppid=,command='], capture_output=True, text=True).stdout.splitlines():
        pid, ppid, command = line.split(None, 2)
        children.setdefault(ppid, []).append((pid, command))
    found, stack = [], [str(os.getpid())]
    while stack:
        for pid, command in children.get(stack.pop(), []):
            stack.append(pid)
            if 'app-server' in command:
                found.append(pid)
    return found


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    for name in ('library', 'bindings', 'bundle', 'node', 'codex'):
        parser.add_argument('--' + name, type=pathlib.Path, required=True)
    parser.add_argument('--seed', type=pathlib.Path)
    args = parser.parse_args()
    server = ThreadingHTTPServer(('127.0.0.1', 0), Upstream)
    threading.Thread(target=server.serve_forever, daemon=True).start()
    endpoint = 'http://127.0.0.1:%d/v1' % server.server_port
    spec = importlib.util.spec_from_file_location('velune_subagent_activity', args.bindings / 'velune_bindings.py')
    b = importlib.util.module_from_spec(spec)
    sys.modules[spec.name] = b
    spec.loader.exec_module(b)

    with tempfile.TemporaryDirectory(prefix='velune-subagent-activity-') as directory:
        root = args.seed.resolve() if args.seed else pathlib.Path(directory).resolve()
        (root / 'project').mkdir(parents=True, exist_ok=True)
        app = b.VeluneApplication.open(b.BindingOptions(home_directory=str(root / 'velune'),
                                                         resources_directory=str(args.bundle / 'Contents/Resources')))
        try:
            app.upsert_runtime_with_execution(b.BindingRuntimeInstance(
                enabled=True, id='codex', name='Codex', type_id='codex-0.159.3', gateway_id='default',
                settings={'binary': str(args.codex.resolve()), 'nodeBinary': str(args.node)}),
                b.BindingRuntimeExecution(transport=b.BindingRuntimeTransport.LOCAL(),
                                          home=b.BindingRuntimeHome.MANAGED(remote_root=None),
                                          remote_resources_directory=None))
            app.list()
            codex_home = root / 'velune/runtime-homes' / hashlib.sha256(b'codex').hexdigest() / '.codex'
            codex_home.mkdir(parents=True, exist_ok=True)
            harness_config = ('model_provider = "synthetic"\nmodel = "synthetic-model"\n[model_providers.synthetic]\nname = "synthetic"\n'
                              'base_url = "%s"\nwire_api = "responses"\nrequires_openai_auth = false\n' % endpoint)
            (codex_home / 'config.toml').write_text(harness_config)
            app.save_provider('default', b.BindingProviderDraft(
                id='responses', name='responses', protocol=b.BindingGatewayProtocol.RESPONSES_V1, endpoint=endpoint,
                models=[b.BindingProviderModel(record_key='', provider_model_id='synthetic-gateway', nickname='gateway',
                                               icon=None, context_window=32768, max_output_tokens=1024,
                                               reasoning_levels=None, adapter_metadata_json=None)]),
                b.BindingAuthenticationEdit.SET_API_KEY(value='SYNTHETIC_ONLY'))
            gateway_key = next(p for p in app.list().gateways[0].providers if p.id == 'responses').models[0].record_key
            selections = {'native': b.BindingModelSelection.NATIVE(provider_id='synthetic', model_id='synthetic-model'),
                          'gateway': b.BindingModelSelection.MANAGED(model_record_key=gateway_key)}

            def snapshot(conversation):
                return app.snapshot('codex', conversation).snapshot

            def texts(current):
                return [block.text for message in current.messages for block in message.blocks
                        if isinstance(block, b.BindingMessageBlock.TEXT)]

            def wait(condition, what, seconds=60):
                deadline = time.monotonic() + seconds
                while True:
                    found = condition()
                    if found:
                        return found
                    assert time.monotonic() < deadline, 'timed out: ' + what
                    time.sleep(.2)

            def running_sub_agents(parent):
                return [item.conversation_id for item in app.conversation_activity(0).active
                        if item.parent_conversation_id == parent
                        and item.run_state in (b.BindingRunState.RUNNING, b.BindingRunState.ELSEWHERE)]

            def turn(conversation, selection, text):
                return app.send_turn_with_selection('codex', conversation, 'codex', selection, b.BindingComposedTurn(
                    text=text, effort=None, attachments=[]))

            def replies(conversation):
                return texts(snapshot(conversation)).count('PARENT_DONE')

            def idle_after(conversation, count, what):
                wait(lambda: snapshot(conversation).run_state == b.BindingRunState.IDLE and replies(conversation) == count, what)

            def gateway_turn(conversation, child, selection):
                wait(lambda: child in running_sub_agents(conversation), 'sub-agent running after the gateway turn')
                servers = app_servers()
                print('PASS gateway: the sub-agent keeps running after the parent\'s gateway turn ended', flush=True)
                refused = None
                try:
                    turn(conversation, selections['native'], 'NATIVE_NEXT')
                except Exception as error:  # the bindings' error variants
                    refused = error
                assert refused is not None and '子 Agent' in str(refused), refused
                assert child in running_sub_agents(conversation) and app_servers() == servers
                print('PASS gateway: a turn needing another process is refused and the sub-agent goes on', flush=True)
                turn(conversation, selection, 'SAME_MODEL_NEXT')
                idle_after(conversation, 2, 'same-model turn while the sub-agent runs')
                assert child in running_sub_agents(conversation) and app_servers() == servers
                print('PASS gateway: a next turn on the same model reused the process', flush=True)
                RELEASE.set()
                pending = wait(lambda: snapshot(conversation).pending_interactions, 'sub-agent approval after the gateway turn')
                app.reply_runtime_interaction('codex', conversation, pending[0].id, b.BindingRuntimeInteractionReply.DECISION(
                    option_id=pending[0].kind.options[0].id))
                wait(lambda: child not in running_sub_agents(conversation), 'gateway sub-agent ends', 90)
                wait(lambda: len(app_servers()) == len(servers) - 1, 'gateway turn\'s app-server ends')
                assert (codex_home / 'config.toml').read_text() == harness_config
                print('PASS gateway: after the sub-agent ended the app-server ended and the Harness configuration is unchanged', flush=True)
                app.open_conversation('codex', child)
                assert 'SUB_DONE' in texts(snapshot(child))
                app.open_conversation('codex', conversation)
                turn(conversation, selections['native'], 'NATIVE_AFTER')
                idle_after(conversation, 3, 'native turn after the gateway turn')
                print('PASS gateway: the sub-agent completed and the parent went on with a native turn', flush=True)

            seeded = {}
            for mode in ('native', 'gateway'):
                RELEASE.clear()
                selection = selections[mode]
                conversation = app.create_conversation_with_selection('codex', str(root / 'project'), selection).snapshot.conversation.id
                conversation = app.send_turn_with_selection('codex', conversation, 'codex', selection, b.BindingComposedTurn(
                    text='PARENT_TASK', effort=None, attachments=[])).snapshot.conversation.id
                parent = wait(lambda: (lambda current: current if current.run_state == b.BindingRunState.IDLE
                                       and 'PARENT_DONE' in texts(current) else None)(snapshot(conversation)),
                              mode + ' parent turn ends')
                assert len(parent.sub_agents) == 1, (mode, parent.sub_agents)
                child = parent.sub_agents[0].conversation_id
                assert child.startswith('codex:') and child != conversation and parent.sub_agents[0].title, (mode, parent.sub_agents)
                print('PASS %s: the idle parent names its sub-agent %s' % (mode, child), flush=True)
                if mode == 'gateway':
                    gateway_turn(conversation, child, selection)
                    seeded[mode] = (conversation, child)
                    continue
                wait(lambda: child in running_sub_agents(conversation), 'native sub-agent reported running under its parent')
                print('PASS native: activity reports the sub-agent running under the idle parent', flush=True)
                RELEASE.set()
                pending = wait(lambda: snapshot(conversation).pending_interactions, 'sub-agent approval on the idle parent')
                assert snapshot(conversation).run_state == b.BindingRunState.IDLE
                assert pending[0].kind.title.startswith('子 Agent：'), pending[0].kind.title
                app.reply_runtime_interaction('codex', conversation, pending[0].id, b.BindingRuntimeInteractionReply.DECISION(
                    option_id=pending[0].kind.options[0].id))
                print('PASS native: the sub-agent\'s approval came to the idle parent and was answered', flush=True)
                wait(lambda: child not in running_sub_agents(conversation), 'sub-agent ends', 90)
                app.list()
                current = snapshot(conversation)
                assert [agent.conversation_id for agent in current.sub_agents] == [child] and not current.sub_agents[0].failed, current.sub_agents
                app.open_conversation('codex', child)
                outputs = [block.output.text for message in snapshot(child).messages for block in message.blocks
                           if isinstance(block, b.BindingMessageBlock.TOOL) and isinstance(block.output, b.BindingMessagePayload.INLINE)]
                assert any('SUB_APPROVED' in output for output in outputs) and 'SUB_DONE' in texts(snapshot(child)), outputs
                print('PASS native: the approved command ran in the sub-agent, which then ended without failure', flush=True)
                app.open_conversation('codex', conversation)
                seeded[mode] = (conversation, child)
            if args.seed:
                print('SEEDED', root / 'velune', json.dumps(seeded), flush=True)
        finally:
            RELEASE.set()
            app.shutdown()
            server.shutdown()


if __name__ == '__main__':
    main()
