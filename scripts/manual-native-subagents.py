#!/usr/bin/env python3
"""Manual acceptance of native sub-agents in the transcript, through UniFFI.

Velune runs real Codex 0.159.3 and Claude Code 2.1.286 turns in managed HOMEs
inside a temporary VELUNE_HOME, through its gateway, against a loopback
upstream that has the parent agent start a native sub-agent (Codex
`spawn_agent` then `wait_agent`; Claude Code's `Agent` tool). Checks, for the
live turn and again for the conversation read from native history, that the
sub-agent call links the sub-agent's conversation, each call row is settled
by its result, the sub-agent's own messages stay out of the parent's
transcript, and a Codex sub-agent's approval request is answered in the
parent's conversation; that the sub-agent's
conversation is listed with its parent, opens with the sub-agent's reply, and
refuses sending. With --seed DIR
the VELUNE_HOME is kept at DIR/velune for checking the Mac app by hand. Only
loopback synthetic responses are used: no real model, credential, HOME or
session is read; this is not an automated test.
"""
import argparse
import importlib.util
import json
import pathlib
import sys
import tempfile
import threading
import time
from http.server import BaseHTTPRequestHandler, ThreadingHTTPServer


def responses_reply(body):
    """Codex: the parent spawns a sub-agent and waits for it; the sub-agent runs
    a command that needs approval, then answers."""
    text = json.dumps(body.get('input', []))
    outputs = [item for item in body.get('input', []) if item.get('type') == 'function_call_output']
    if 'SUBAGENT_TASK' in text and 'PARENT_TASK' not in text:
        if outputs:
            return {'type': 'message', 'text': 'SUB_DONE'}
        return {'type': 'function_call', 'name': 'exec_command', 'call_id': 'call_escalate', 'namespace': None,
                'arguments': json.dumps({'cmd': 'echo SUB_APPROVED', 'sandbox_permissions': 'require_escalated',
                                         'justification': 'Synthetic sub-agent check'})}
    if 'PARENT_TASK' not in text:
        return {'type': 'message', 'text': 'OTHER'}
    if len(outputs) >= 2:
        return {'type': 'message', 'text': 'PARENT_DONE'}
    if outputs:
        spawned = json.loads(outputs[-1]['output'])
        target = spawned.get('agent_id') or spawned.get('id') or spawned.get('task_name')
        return {'type': 'function_call', 'name': 'wait_agent', 'call_id': 'call_wait',
                'arguments': json.dumps({'targets': [target], 'timeout_ms': 30000})}
    names = json.dumps(body.get('tools', []))
    assert 'spawn_agent' in names, 'Codex offered no spawn_agent tool'
    return {'type': 'function_call', 'name': 'spawn_agent', 'call_id': 'call_spawn',
            'arguments': json.dumps({'message': 'SUBAGENT_TASK'})}


def messages_reply(body):
    """Claude Code: the parent starts an Agent; the sub-agent answers."""
    messages = body.get('messages', [])
    text = json.dumps(messages)
    if 'security monitor' in json.dumps(body.get('system')):
        # Velune runs Claude Code in auto mode; its classifier asks whether to block the call.
        return [{'type': 'text', 'text': '<block>no</block>'}], 'end_turn'
    agent = next((tool['name'] for tool in body.get('tools', []) if tool.get('name') in ('Agent', 'Task')), None)
    if 'SUBAGENT_TASK' in json.dumps(messages[:1]) and 'PARENT_TASK' not in text:
        return [{'type': 'text', 'text': 'SUB_DONE'}], 'end_turn'
    if 'PARENT_TASK' in text and 'tool_result' in text:
        return [{'type': 'text', 'text': 'PARENT_DONE'}], 'end_turn'
    if 'PARENT_TASK' in text and agent:
        return [{'type': 'tool_use', 'id': 'toolu_subagent', 'name': agent,
                 'input': {'description': 'Synthetic helper', 'prompt': 'SUBAGENT_TASK', 'subagent_type': 'general-purpose'}}], 'tool_use'
    return [{'type': 'text', 'text': 'OTHER'}], 'end_turn'


class Upstream(BaseHTTPRequestHandler):
    protocol_version = 'HTTP/1.1'

    def log_message(self, *_):
        pass

    def payload(self, value):
        data = json.dumps(value).encode()
        self.send_response(200)
        self.send_header('Content-Type', 'application/json')
        self.send_header('Content-Length', str(len(data)))
        self.end_headers()
        self.wfile.write(data)

    def events(self, events):
        self.send_response(200)
        self.send_header('Content-Type', 'text/event-stream')
        self.send_header('Connection', 'close')
        self.end_headers()
        for event in events:
            self.wfile.write(('event: %s\ndata: %s\n\n' % (event['type'], json.dumps(event))).encode())
        self.wfile.flush()

    def do_POST(self):
        body = json.loads(self.rfile.read(int(self.headers.get('Content-Length', '0'))) or b'{}')
        path = self.path.split('?')[0]
        if path.endswith('/count_tokens'):
            return self.payload({'input_tokens': 1})
        if path.endswith('/responses'):
            out = responses_reply(body)
            if out['type'] == 'message':
                item = {'id': 'item', 'type': 'message', 'role': 'assistant', 'status': 'completed',
                        'content': [{'type': 'output_text', 'text': out['text'], 'annotations': []}]}
            else:
                item = {'id': 'fc', 'type': 'function_call', 'status': 'completed', 'name': out['name'],
                        'arguments': out['arguments'], 'call_id': out['call_id']}
                if out.get('namespace', 'multi_agent_v1'):
                    item['namespace'] = out.get('namespace', 'multi_agent_v1')
            response = {'id': 'response', 'object': 'response', 'status': 'completed', 'output': [item],
                        'usage': {'input_tokens': 1, 'output_tokens': 1, 'total_tokens': 2}}
            return self.events([
                {'type': 'response.created', 'response': {**response, 'status': 'in_progress', 'output': []}},
                {'type': 'response.output_item.added', 'output_index': 0, 'item': item},
                {'type': 'response.output_item.done', 'output_index': 0, 'item': item},
                {'type': 'response.completed', 'response': response}])
        if path.endswith('/messages'):
            content, stop = messages_reply(body)
            message = {'id': 'msg_synthetic', 'type': 'message', 'role': 'assistant', 'model': body.get('model', 'synthetic'),
                       'content': content, 'stop_reason': stop, 'stop_sequence': None,
                       'usage': {'input_tokens': 1, 'output_tokens': 1}}
            if not body.get('stream'):
                return self.payload(message)
            events = [{'type': 'message_start', 'message': {**message, 'content': [], 'stop_reason': None}}]
            for index, block in enumerate(content):
                if block['type'] == 'text':
                    events += [{'type': 'content_block_start', 'index': index, 'content_block': {'type': 'text', 'text': ''}},
                               {'type': 'content_block_delta', 'index': index, 'delta': {'type': 'text_delta', 'text': block['text']}}]
                else:
                    events += [{'type': 'content_block_start', 'index': index, 'content_block': {**block, 'input': {}}},
                               {'type': 'content_block_delta', 'index': index,
                                'delta': {'type': 'input_json_delta', 'partial_json': json.dumps(block['input'])}}]
                events.append({'type': 'content_block_stop', 'index': index})
            events += [{'type': 'message_delta', 'delta': {'stop_reason': stop, 'stop_sequence': None}, 'usage': {'output_tokens': 1}},
                       {'type': 'message_stop'}]
            return self.events(events)
        self.send_response(404)
        self.send_header('Content-Length', '0')
        self.end_headers()


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    for name in ('library', 'bindings', 'bundle', 'node', 'codex', 'claude'):
        parser.add_argument('--' + name, type=pathlib.Path, required=True)
    parser.add_argument('--seed', type=pathlib.Path)
    args = parser.parse_args()
    server = ThreadingHTTPServer(('127.0.0.1', 0), Upstream)
    threading.Thread(target=server.serve_forever, daemon=True).start()
    endpoint = 'http://127.0.0.1:%d/v1' % server.server_port
    spec = importlib.util.spec_from_file_location('velune_native_subagents', args.bindings / 'velune_bindings.py')
    b = importlib.util.module_from_spec(spec)
    sys.modules[spec.name] = b
    spec.loader.exec_module(b)

    with tempfile.TemporaryDirectory(prefix='velune-native-subagents-') as directory:
        root = args.seed.resolve() if args.seed else pathlib.Path(directory).resolve()
        (root / 'project').mkdir(parents=True, exist_ok=True)
        app = b.VeluneApplication.open(b.BindingOptions(home_directory=str(root / 'velune'),
                                                         resources_directory=str(args.bundle / 'Contents/Resources')))
        try:
            managed = b.BindingRuntimeExecution(transport=b.BindingRuntimeTransport.LOCAL(),
                                                home=b.BindingRuntimeHome.MANAGED(remote_root=None),
                                                remote_resources_directory=None)
            app.upsert_runtime_with_execution(b.BindingRuntimeInstance(
                enabled=True, id='codex', name='Codex', type_id='codex-0.159', gateway_id='default',
                settings={'binary': str(args.codex.resolve()), 'nodeBinary': str(args.node)}), managed)
            app.upsert_runtime_with_execution(b.BindingRuntimeInstance(
                enabled=True, id='claude', name='Claude Code', type_id='claude-code-2.1', gateway_id='default',
                settings={'binary': str(args.claude.resolve())}), managed)

            def provider(identifier, protocol):
                app.save_provider('default', b.BindingProviderDraft(
                    id=identifier, name=identifier, protocol=protocol, endpoint=endpoint,
                    models=[b.BindingProviderModel(record_key='', provider_model_id='synthetic-' + identifier, nickname=identifier,
                                                   icon=None, context_window=32768, max_output_tokens=1024,
                                                   reasoning_levels=None, adapter_metadata_json=None)], quota_limits=[]),
                    b.BindingAuthenticationEdit.SET_API_KEY(value='SYNTHETIC_ONLY'))
                return next(p for p in app.list().gateways[0].providers if p.id == identifier).models[0].record_key

            keys = {'codex': provider('responses', b.BindingGatewayProtocol.RESPONSES_V1),
                    'claude': provider('messages', b.BindingGatewayProtocol.MESSAGES_V1)}

            def view(runtime, conversation):
                current = app.snapshot(runtime, conversation).snapshot
                texts = [block.text for message in current.messages for block in message.blocks
                         if isinstance(block, b.BindingMessageBlock.TEXT)]
                links = [block.linked_conversation_id for message in current.messages for block in message.blocks
                         if isinstance(block, b.BindingMessageBlock.TOOL) and block.linked_conversation_id]
                return current, texts, links

            def run(runtime):
                conversation = app.create_conversation(runtime, str(root / 'project'), keys[runtime]).snapshot.conversation.id
                conversation = app.send_turn_with_selection(
                    runtime, conversation, runtime, b.BindingModelSelection.MANAGED(model_record_key=keys[runtime]),
                    b.BindingComposedTurn(text='PARENT_TASK', effort=None, attachments=[])).snapshot.conversation.id
                deadline = time.monotonic() + 120
                seen, asked = set(), []
                while True:
                    current, texts, links = view(runtime, conversation)
                    seen.update(links)
                    for pending in current.pending_interactions:
                        # The synthetic agents only start a sub-agent; allow it once.
                        options = pending.kind.options if isinstance(pending.kind, b.BindingInteractionKind.APPROVAL) else []
                        asked.append(pending.kind.title)
                        app.reply_runtime_interaction(runtime, conversation, pending.id,
                                                      b.BindingRuntimeInteractionReply.DECISION(option_id=options[0].id))
                    if current.run_state == b.BindingRunState.IDLE and 'PARENT_DONE' in texts:
                        return conversation, texts, seen, asked
                    assert current.run_state != b.BindingRunState.FAILED, (runtime, texts)
                    assert time.monotonic() < deadline, (runtime, current.run_state, texts)
                    time.sleep(.2)

            def check(runtime, conversation, texts, links, phase):
                if not links:
                    # A blocked or failed call shows why in its tool output.
                    tools = [(block.title, block.output.text[:300] if isinstance(block.output, b.BindingMessagePayload.INLINE) else None)
                             for message in app.snapshot(runtime, conversation).snapshot.messages for block in message.blocks
                             if isinstance(block, b.BindingMessageBlock.TOOL)]
                    raise AssertionError((runtime, phase, 'no sub-agent link', texts, tools))
                assert 'SUB_DONE' not in texts, (runtime, phase, 'sub-agent message in parent transcript', texts)
                states = [block.state for message in app.snapshot(runtime, conversation).snapshot.messages for block in message.blocks
                          if isinstance(block, b.BindingMessageBlock.TOOL)]
                assert all(state in (b.BindingToolState.COMPLETED, b.BindingToolState.FAILED) for state in states), \
                    (runtime, phase, 'a call is not settled by its result', states)
                assert len(links) == 1, (runtime, phase, links)
                child = next(iter(links))
                assert child.startswith(runtime + ':') and child != conversation, (runtime, phase, child)
                return child

            seeded = {}
            for runtime in ('codex', 'claude'):
                conversation, texts, links, asked = run(runtime)
                child = check(runtime, conversation, texts, links, 'live')
                if runtime == 'codex':
                    # The sub-agent's own approval is answered in the parent's conversation.
                    assert any(title.startswith('子 Agent：') for title in asked), asked
                print('PASS %s live: the sub-agent call links %s; its messages stay in its own conversation%s' % (runtime, child, '; its approval was answered here' if runtime == 'codex' else ''), flush=True)

                app.open_conversation(runtime, conversation)
                current, texts, links = view(runtime, conversation)
                assert check(runtime, conversation, texts, set(links), 'history') == child
                assert current.actions.can_send, (runtime, 'parent stays sendable')
                # The list leaves sub-agents to their parent; lookup still finds them.
                listed = {item.conversation.id: item for item in app.list().conversations}
                assert child not in listed or listed[child].group is None
                assert app.find_conversation(child).conversation.parent_conversation_id == conversation, runtime
                assert listed[conversation].conversation.parent_conversation_id is None
                print('PASS %s history: the link survives reading native history; the sub-agent is found under its parent' % runtime, flush=True)

                app.open_conversation(runtime, child)
                current, texts, _ = view(runtime, child)
                assert 'SUB_DONE' in texts and 'SUBAGENT_TASK' in ' '.join(texts), (runtime, texts)
                if runtime == 'codex':
                    outputs = [block.output.text for message in current.messages for block in message.blocks
                               if isinstance(block, b.BindingMessageBlock.TOOL) and isinstance(block.output, b.BindingMessagePayload.INLINE)]
                    assert any('SUB_APPROVED' in output for output in outputs), outputs
                assert not current.actions.can_send, (runtime, 'sub-agent conversation is read-only')
                try:
                    app.send_turn_with_selection(runtime, child, runtime, b.BindingModelSelection.MANAGED(model_record_key=keys[runtime]),
                                                 b.BindingComposedTurn(text='NEVER_SENT', effort=None, attachments=[]))
                except b.BindingError as error:
                    assert 'sub-agent' in str(error), error
                else:
                    raise AssertionError('%s sent to a sub-agent conversation' % runtime)
                print('PASS %s: the sub-agent conversation opens with its reply and refuses sending' % runtime, flush=True)
                seeded[runtime] = (conversation, child)
            if args.seed:
                print('SEEDED', root / 'velune', json.dumps(seeded), flush=True)
        finally:
            app.shutdown()
            server.shutdown()


if __name__ == '__main__':
    main()
