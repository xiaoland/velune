#!/usr/bin/env python3
"""Manual acceptance that a resource library sub-agent reaches the Harness.

Velune writes a skill and a sub-agent (description over several lines,
model, reasoning level, the skill) from its resource library into real
Codex 0.159.3 and Claude Code 2.1.286 managed HOMEs inside a temporary
VELUNE_HOME. Checks the native files and the notes for what a Harness
cannot hold, and that Codex has no place for the skill. Then a parent turn at a different level, through Velune's gateway against a loopback
upstream, starts the defined sub-agent (Codex `spawn_agent` with its role;
Claude Code's `Agent` tool with its type), and the sub-agent's own upstream
request shows its instructions and level, and on Claude Code the skill's
content. Only loopback synthetic responses are used: no real model,
credential, HOME or session is read; this is not an automated test.
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

INSTRUCTIONS = 'PROBE_INSTRUCTIONS'
SKILL = 'SKILL_MARKER_TEXT'
captures = []


def responses_reply(body):
    """Codex: the parent spawns the probe role and waits; the sub-agent answers."""
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
    return {'type': 'function_call', 'name': 'spawn_agent', 'call_id': 'call_spawn',
            'arguments': json.dumps({'message': 'SUBAGENT_TASK', 'agent_type': 'probe'})}


def messages_reply(body):
    """Claude Code: the parent starts the probe agent; the sub-agent answers."""
    messages = body.get('messages', [])
    text = json.dumps(messages)
    if 'security monitor' in json.dumps(body.get('system')):
        return [{'type': 'text', 'text': '<block>no</block>'}], 'end_turn'
    agent = next((tool['name'] for tool in body.get('tools', []) if tool.get('name') in ('Agent', 'Task')), None)
    if 'SUBAGENT_TASK' in json.dumps(messages[:1]) and 'PARENT_TASK' not in text:
        return [{'type': 'text', 'text': 'SUB_DONE'}], 'end_turn'
    if 'PARENT_TASK' in text and 'tool_result' in text:
        return [{'type': 'text', 'text': 'PARENT_DONE'}], 'end_turn'
    if 'PARENT_TASK' in text and agent:
        return [{'type': 'tool_use', 'id': 'toolu_probe', 'name': agent,
                 'input': {'description': 'Probe', 'prompt': 'SUBAGENT_TASK', 'subagent_type': 'probe'}}], 'tool_use'
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
        captures.append(body)
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
                enabled=True, id='codex', name='Codex', type_id='codex-0.159.3', gateway_id='default',
                settings={'binary': str(args.codex.resolve()), 'nodeBinary': str(args.node)}), managed)
            app.upsert_runtime_with_execution(b.BindingRuntimeInstance(
                enabled=True, id='claude', name='Claude Code', type_id='claude-code-2.1.286', gateway_id='default',
                settings={'binary': str(args.claude.resolve())}), managed)

            def provider(identifier, protocol):
                app.save_provider('default', b.BindingProviderDraft(
                    id=identifier, name=identifier, protocol=protocol, endpoint=endpoint,
                    models=[b.BindingProviderModel(record_key='', provider_model_id='synthetic-' + identifier, nickname=identifier,
                                                   icon=None, context_window=32768, max_output_tokens=1024,
                                                   reasoning_levels=['low', 'medium', 'high'], adapter_metadata_json=None)]),
                    b.BindingAuthenticationEdit.SET_API_KEY(value='SYNTHETIC_ONLY'))
                return next(p for p in app.list().gateways[0].providers if p.id == identifier).models[0].record_key

            keys = {'codex': provider('responses', b.BindingGatewayProtocol.RESPONSES_V1),
                    'claude': provider('messages', b.BindingGatewayProtocol.MESSAGES_V1)}
            targets = [b.BindingTemplateApplyTarget(runtime_id=runtime, overwrite=False) for runtime in ('codex', 'claude')]
            skill = app.save_resource_template(b.BindingResourceTemplate(id='', name='probe-skill', content=content.SKILL(files=[
                b.BindingTemplateSkillFile(path='SKILL.md', executable=False, bytes=(
                    '---\nname: probe-skill\ndescription: Probe skill\n---\n\n%s\n' % SKILL).encode())]))).summary
            app.apply_resource_template(skill.id, targets)
            probe = app.save_resource_template(b.BindingResourceTemplate(id='', name='probe', content=content.SUBAGENT(
                description='Probe agent.\nUse it for the probe task.', instructions=INSTRUCTIONS, model=None, tools=[],
                effort='high', skills=['probe-skill']))).summary
            notes = {report.runtime_id: report.notes for report in app.apply_resource_template(probe.id, targets)}
            assert notes['claude'] == [] and any('技能' in note for note in notes['codex']), notes
            files = {path.name: path.read_text() for path in (root / 'velune').rglob('probe.*') if path.parent.name == 'agents'}
            assert 'model_reasoning_effort = "high"' in files['probe.toml'] and 'Use it for the probe task.' in files['probe.toml'], files
            assert 'effort: high' in files['probe.md'] and '- probe-skill' in files['probe.md'], files
            print('PASS the definition is written in each Harness\'s own fields; Codex says it has no place for the skill', flush=True)

            def run(runtime):
                conversation = app.create_conversation(runtime, str(root / 'project'), keys[runtime]).snapshot.conversation.id
                conversation = app.send_turn_with_selection(
                    runtime, conversation, runtime, b.BindingModelSelection.MANAGED(model_record_key=keys[runtime]),
                    b.BindingComposedTurn(text='PARENT_TASK', effort='low', attachments=[])).snapshot.conversation.id
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

            for runtime in ('codex', 'claude'):
                before = len(captures)
                run(runtime)
                requests = [body for body in captures[before:] if 'SUBAGENT_TASK' in json.dumps(body) and 'PARENT_TASK' not in json.dumps(body)]
                assert requests, (runtime, 'the sub-agent made no request')
                for body in requests:
                    text = json.dumps(body)
                    level = (body.get('reasoning') or {}).get('effort') if runtime == 'codex' else (body.get('output_config') or {}).get('effort')
                    assert INSTRUCTIONS in text and level == 'high', (runtime, level)
                    assert runtime == 'codex' or SKILL in text, (runtime, 'the skill was not loaded')
                print('PASS %s: the sub-agent runs with its own instructions and level%s' % (runtime, ' and the skill' if runtime == 'claude' else ''), flush=True)
        finally:
            app.shutdown()
            server.shutdown()


if __name__ == '__main__':
    main()
