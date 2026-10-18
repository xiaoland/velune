#!/usr/bin/env python3
"""Manual acceptance that resource library sub-agents reach the Harness.

Velune's resource library holds a skill and two sub-agents for real Codex
0.159.3 and Claude Code 2.1.286 managed HOMEs inside a temporary
VELUNE_HOME. `probe` names a native model ID: it is written into each
Harness's definition with its level and skill (Codex says it has no place
for the skill), and a turn through Velune's gateway routes that ID to the
one Velune model with the same provider model ID. `vprobe` names a Velune
model: nothing of it is written where the Harness keeps its definitions,
and only turns through the gateway receive it (Codex as a role in the
thread's config, Claude Code through `--agents`). For each, a parent turn
at another level starts the sub-agent through Velune's gateway against a
loopback upstream, and the sub-agent's own request reaches the provider of
its model with its instructions and level (between protocols, as the
gateway translates it), and on Claude Code the skill's content. Only loopback synthetic responses are used: no real model,
credential, HOME or session is read; this is not an automated test.
"""
import argparse
import importlib.util
import json
import pathlib
import re
import sys
import tempfile
import threading
import time
from http.server import BaseHTTPRequestHandler, ThreadingHTTPServer

SKILL = 'SKILL_MARKER_TEXT'
captures = []


def instructions(agent):
    return agent.upper() + '_INSTRUCTIONS'


def asked(text):
    """The sub-agent a parent turn asks for."""
    return re.search(r'PARENT_TASK=(\w+);', text).group(1)


def responses_reply(body):
    """The parent spawns the asked role and waits; the sub-agent answers."""
    text = json.dumps(body.get('input', []))
    outputs = [item for item in body.get('input', []) if item.get('type') == 'function_call_output']
    if 'SUBAGENT_TASK' in text and 'PARENT_TASK' not in text:
        return {'type': 'message', 'text': 'SUB_DONE'}
    if 'PARENT_TASK' not in text:
        return {'type': 'message', 'text': 'OTHER'}
    if len(outputs) >= 2:
        return {'type': 'message', 'text': 'PARENT_DONE'}
    if outputs:
        spawned = json.loads(outputs[-1]['output'])
        target = spawned.get('agent_id') or spawned.get('id') or spawned.get('task_name')
        return {'type': 'function_call', 'name': 'wait_agent', 'call_id': 'call_wait',
                'arguments': json.dumps({'targets': [target], 'timeout_ms': 30000})}
    agent = asked(text)
    return {'type': 'function_call', 'name': 'spawn_agent', 'call_id': 'call_spawn',
            'arguments': json.dumps({'message': 'SUBAGENT_TASK=%s;' % agent, 'agent_type': agent})}


def messages_reply(body):
    """The parent starts the asked agent; the sub-agent answers."""
    messages = body.get('messages', [])
    text = json.dumps(messages)
    if 'security monitor' in json.dumps(body.get('system')):
        return [{'type': 'text', 'text': '<block>no</block>'}], 'end_turn'
    tool = next((tool['name'] for tool in body.get('tools', []) if tool.get('name') in ('Agent', 'Task')), None)
    if 'SUBAGENT_TASK' in text and 'PARENT_TASK' not in text:
        return [{'type': 'text', 'text': 'SUB_DONE'}], 'end_turn'
    if 'PARENT_TASK' in text and 'tool_result' in text:
        return [{'type': 'text', 'text': 'PARENT_DONE'}], 'end_turn'
    if 'PARENT_TASK' in text and tool:
        agent = asked(text)
        return [{'type': 'tool_use', 'id': 'toolu_probe', 'name': tool,
                 'input': {'description': 'Probe', 'prompt': 'SUBAGENT_TASK=%s;' % agent, 'subagent_type': agent}}], 'tool_use'
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
        captures.append((path, body))
        if path.endswith('/responses'):
            out = responses_reply(body)
            if out['type'] == 'message':
                item = {'id': 'item-%d' % len(captures), 'type': 'message', 'role': 'assistant', 'status': 'completed',
                        'content': [{'type': 'output_text', 'text': out['text'], 'annotations': []}]}
            else:
                item = {'id': 'fc-%d' % len(captures), 'type': 'function_call', 'status': 'completed', 'name': out['name'],
                        'arguments': out['arguments'], 'call_id': out['call_id'], 'namespace': 'multi_agent_v1'}
            response = {'id': 'response', 'object': 'response', 'status': 'completed', 'output': [item],
                        'usage': {'input_tokens': 1, 'output_tokens': 1, 'total_tokens': 2}}
            return self.events([
                {'type': 'response.created', 'response': {**response, 'status': 'in_progress', 'output': []}},
                {'type': 'response.output_item.added', 'output_index': 0, 'item': item},
                {'type': 'response.output_item.done', 'output_index': 0, 'item': item},
                {'type': 'response.completed', 'response': response}])
        content, stop = messages_reply(body)
        message = {'id': 'msg_synthetic_%d' % len(captures), 'type': 'message', 'role': 'assistant', 'model': body.get('model', 'synthetic'),
                   'content': content, 'stop_reason': stop, 'stop_sequence': None, 'usage': {'input_tokens': 1, 'output_tokens': 1}}
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


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    for name in ('library', 'bindings', 'bundle', 'node', 'codex', 'claude'):
        parser.add_argument('--' + name, type=pathlib.Path, required=True)
    args = parser.parse_args()
    server = ThreadingHTTPServer(('127.0.0.1', 0), Upstream)
    threading.Thread(target=server.serve_forever, daemon=True).start()
    endpoint = 'http://127.0.0.1:%d/v1' % server.server_port
    spec = importlib.util.spec_from_file_location('velune_subagent_definitions', args.bindings / 'velune_bindings.py')
    b = importlib.util.module_from_spec(spec)
    sys.modules[spec.name] = b
    spec.loader.exec_module(b)
    content = b.BindingResourceTemplateContent

    with tempfile.TemporaryDirectory(prefix='velune-subagent-definitions-') as directory:
        root = pathlib.Path(directory).resolve()
        (root / 'project').mkdir()
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
                                                   reasoning_levels=['low', 'medium', 'high'])], quota_limits=[], execution_backend=b.BindingExecutionBackend.HTTP(), request_profile=b.BindingRequestProfile.NATIVE, authentication_source=b.BindingProviderAuthenticationSource.API_KEY, quota_source=b.BindingProviderQuotaSource.NONE),
                    b.BindingAuthenticationEdit.SET_API_KEY(value='SYNTHETIC_ONLY'))
                return next(p for p in app.list().gateways[0].providers if p.id == identifier).models[0].record_key

            keys = {'codex': provider('responses', b.BindingGatewayProtocol.RESPONSES_V1),
                    'claude': provider('messages', b.BindingGatewayProtocol.MESSAGES_V1)}
            targets = [b.BindingTemplateApplyTarget(runtime_id=runtime, overwrite=False) for runtime in ('codex', 'claude')]
            skill = app.save_resource_template(b.BindingResourceTemplate(id='', name='probe-skill', content=content.SKILL(files=[
                b.BindingTemplateSkillFile(path='SKILL.md', executable=False, bytes=(
                    '---\nname: probe-skill\ndescription: Probe skill\n---\n\n%s\n' % SKILL).encode())]))).summary
            app.apply_resource_template(skill.id, targets)

            def subagent(name, model, velune_model, effort):
                saved = app.save_resource_template(b.BindingResourceTemplate(id='', name=name, content=content.SUBAGENT(
                    description='%s agent.\nUse it for the %s task.' % (name, name), instructions=instructions(name),
                    model=model, velune_model=velune_model, tools=[], effort=effort, skills=['probe-skill']))).summary
                return {report.runtime_id: report.notes for report in app.apply_resource_template(saved.id, targets)}

            # A native ID the messages provider also serves; a Velune model on the responses provider.
            notes = subagent('probe', 'synthetic-messages', None, 'high')
            assert notes['claude'] == [] and any('技能' in note for note in notes['codex']), notes
            notes = subagent('vprobe', None, keys['codex'], 'medium')
            assert all(any('网关' in note for note in found) for found in notes.values()), notes
            definitions = {path.name: path for path in (root / 'velune').rglob('probe.*') if path.parent.name == 'agents'}
            native_dirs = {path.parent for path in definitions.values()}
            text = {name: path.read_text() for name, path in definitions.items()}
            assert 'model = "synthetic-messages"' in text['probe.toml'] and 'model_reasoning_effort = "high"' in text['probe.toml'], text
            assert 'model: synthetic-messages' in text['probe.md'] and 'effort: high' in text['probe.md'] and '- probe-skill' in text['probe.md'], text

            def unwritten():
                return not any(path.name.startswith('vprobe.') for directory in native_dirs for path in directory.iterdir())
            assert unwritten()
            print('PASS a native model ID is written into each definition; a Velune model is not written at all', flush=True)

            def run(runtime, agent):
                conversation = app.create_conversation(runtime, str(root / 'project'), keys[runtime]).snapshot.conversation.id
                conversation = app.send_turn_with_selection(
                    runtime, conversation, runtime, b.BindingModelSelection.MANAGED(model_record_key=keys[runtime]),
                    b.BindingComposedTurn(text='PARENT_TASK=%s;' % agent, effort='low', attachments=[])).snapshot.conversation.id
                deadline = time.monotonic() + 120
                while True:
                    current = app.snapshot(runtime, conversation).snapshot
                    texts = [block.text for message in current.messages for block in message.blocks
                             if isinstance(block, b.BindingMessageBlock.TEXT)]
                    for pending in current.pending_interactions:
                        options = pending.kind.options if isinstance(pending.kind, b.BindingInteractionKind.APPROVAL) else []
                        app.reply_runtime_interaction(runtime, conversation, pending.id,
                                                      b.BindingRuntimeInteractionReply.DECISION(option_id=options[0].id))
                    if current.run_state == b.BindingRunState.IDLE and 'PARENT_DONE' in texts:
                        return
                    assert current.run_state != b.BindingRunState.FAILED, (runtime, texts)
                    assert time.monotonic() < deadline, (runtime, current.run_state, texts)
                    time.sleep(.2)

            # Where each sub-agent's requests go, and the level they carry there.
            expected = {'probe': ('/messages', 'synthetic-messages', 'high'), 'vprobe': ('/responses', 'synthetic-responses', 'medium')}
            for runtime in ('codex', 'claude'):
                for agent, (path, model, level) in expected.items():
                    before = len(captures)
                    run(runtime, agent)
                    requests = [(where, body) for where, body in captures[before:]
                                if 'SUBAGENT_TASK=%s;' % agent in json.dumps(body) and 'PARENT_TASK' not in json.dumps(body)]
                    assert requests, (runtime, agent, 'the sub-agent made no request')
                    for where, body in requests:
                        dumped = json.dumps(body)
                        sent = (body.get('reasoning') or {}).get('effort') if where.endswith('/responses') else (body.get('output_config') or {}).get('effort')
                        assert where.endswith(path) and body.get('model') == model, (runtime, agent, where, body.get('model'))
                        assert instructions(agent) in dumped, (runtime, agent)
                        # Responses to Messages carries a level as a thinking budget, best effort.
                        translated = runtime == 'codex' and where.endswith('/messages')
                        assert (body.get('thinking') or {}).get('type') == 'enabled' if translated else sent == level, (runtime, agent, sent, body.get('thinking'))
                        assert runtime == 'codex' or SKILL in dumped, (runtime, agent, 'the skill was not loaded')
                    print('PASS %s %s: the sub-agent reaches %s with its instructions and level%s' % (
                        runtime, agent, model, ' and the skill' if runtime == 'claude' else ''), flush=True)
            assert unwritten()
            print('PASS the Velune-model sub-agent was supplied to gateway turns without being written where the Harness keeps its definitions', flush=True)
        finally:
            app.shutdown()
            server.shutdown()


if __name__ == '__main__':
    main()
