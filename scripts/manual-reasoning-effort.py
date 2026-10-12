#!/usr/bin/env python3
"""Manual acceptance for choosing a turn's reasoning level.

Real Codex 0.159.3, Claude Code 2.1.286 and Pi 1.0.2 run in temporary HOMEs
against gateway providers whose upstream is a loopback synthetic server.
Checks the levels offered for each runtime and model, that a level outside
them is refused before anything is sent, and the reasoning field each
harness then sends upstream through its own channel, for a chosen level and
for a following turn without one, which returns to the runtime's default. No real model,
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


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    for name in ('library', 'bindings', 'bundle', 'node', 'codex', 'claude', 'pi'):
        parser.add_argument('--' + name, type=pathlib.Path, required=True)
    args = parser.parse_args()
    captures = []

    class Upstream(BaseHTTPRequestHandler):
        protocol_version = 'HTTP/1.1'

        def log_message(self, *_):
            pass

        def reply(self, content_type, payload):
            self.send_response(200)
            self.send_header('Content-Type', content_type)
            self.send_header('Content-Length', str(len(payload)))
            self.send_header('Connection', 'close')
            self.end_headers()
            self.wfile.write(payload)

        def do_POST(self):
            body = json.loads(self.rfile.read(int(self.headers.get('Content-Length', '0'))) or b'{}')
            path = self.path.split('?')[0]
            if path.endswith('/count_tokens'):
                return self.reply('application/json', b'{"input_tokens":1}')
            captures.append((path, body))
            if path.endswith('/responses'):
                # Codex keys items by identity; every reply needs its own.
                item_id = 'item-%d' % len(captures)
                item = {'id': item_id, 'type': 'message', 'role': 'assistant', 'status': 'completed',
                        'content': [{'type': 'output_text', 'text': 'SYNTHETIC_REPLY', 'annotations': []}]}
                response = {'id': 'response', 'object': 'response', 'status': 'completed', 'output': [item],
                            'usage': {'input_tokens': 1, 'output_tokens': 1, 'total_tokens': 2}}
                events = [
                    {'type': 'response.created', 'response': {**response, 'status': 'in_progress', 'output': []}},
                    {'type': 'response.output_item.added', 'output_index': 0, 'item': {**item, 'status': 'in_progress', 'content': []}},
                    {'type': 'response.content_part.added', 'item_id': item_id, 'output_index': 0, 'content_index': 0,
                     'part': {'type': 'output_text', 'text': '', 'annotations': []}},
                    {'type': 'response.output_text.delta', 'item_id': item_id, 'output_index': 0, 'content_index': 0, 'delta': 'SYNTHETIC_REPLY'},
                    {'type': 'response.output_text.done', 'item_id': item_id, 'output_index': 0, 'content_index': 0, 'text': 'SYNTHETIC_REPLY'},
                    {'type': 'response.content_part.done', 'item_id': item_id, 'output_index': 0, 'content_index': 0, 'part': item['content'][0]},
                    {'type': 'response.output_item.done', 'output_index': 0, 'item': item},
                    {'type': 'response.completed', 'response': response}]
            elif path.endswith('/messages'):
                message = {'id': 'msg_synthetic_%d' % len(captures), 'type': 'message', 'role': 'assistant', 'model': body.get('model', 'synthetic'),
                           'content': [], 'stop_reason': None, 'stop_sequence': None, 'usage': {'input_tokens': 1, 'output_tokens': 1}}
                if not body.get('stream'):
                    payload = {**message, 'content': [{'type': 'text', 'text': 'SYNTHETIC_REPLY'}], 'stop_reason': 'end_turn'}
                    return self.reply('application/json', json.dumps(payload).encode())
                events = [
                    {'type': 'message_start', 'message': message},
                    {'type': 'content_block_start', 'index': 0, 'content_block': {'type': 'text', 'text': ''}},
                    {'type': 'content_block_delta', 'index': 0, 'delta': {'type': 'text_delta', 'text': 'SYNTHETIC_REPLY'}},
                    {'type': 'content_block_stop', 'index': 0},
                    {'type': 'message_delta', 'delta': {'stop_reason': 'end_turn', 'stop_sequence': None}, 'usage': {'output_tokens': 1}},
                    {'type': 'message_stop'}]
            else:
                self.send_response(404)
                self.send_header('Content-Length', '0')
                self.end_headers()
                return
            self.reply('text/event-stream', ''.join('event: %s\ndata: %s\n\n' % (event['type'], json.dumps(event))
                                                    for event in events).encode())

    server = ThreadingHTTPServer(('127.0.0.1', 0), Upstream)
    threading.Thread(target=server.serve_forever, daemon=True).start()
    endpoint = 'http://127.0.0.1:%d/v1' % server.server_port

    with tempfile.TemporaryDirectory(prefix='velune-effort-') as directory:
        root = pathlib.Path(directory).resolve()
        (root / 'project').mkdir()
        spec = importlib.util.spec_from_file_location('velune_effort', args.bindings / 'velune_bindings.py')
        b = importlib.util.module_from_spec(spec)
        sys.modules[spec.name] = b
        spec.loader.exec_module(b)
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
            app.upsert_runtime(b.BindingRuntimeInstance(
                enabled=True, id='pi', name='Pi', type_id='pi-1.0.2', gateway_id='default',
                settings={'binary': str(args.pi), 'nodeBinary': str(args.node), 'agentDir': str(root / 'pi')}))

            def provider(identifier, protocol, levels):
                app.save_provider('default', b.BindingProviderDraft(
                    id=identifier, name=identifier, protocol=protocol, endpoint=endpoint,
                    models=[b.BindingProviderModel(record_key='', provider_model_id='synthetic-' + identifier, nickname=identifier,
                                                   icon=None, context_window=32768, max_output_tokens=1024,
                                                   reasoning_levels=levels, adapter_metadata_json=None)]),
                    b.BindingAuthenticationEdit.SET_API_KEY(value='SYNTHETIC_ONLY'))
                return next(p for p in app.list().gateways[0].providers if p.id == identifier).models[0].record_key

            responses = provider('responses', b.BindingGatewayProtocol.RESPONSES_V1, ['none', 'low', 'medium', 'high', 'xhigh'])
            messages = provider('messages', b.BindingGatewayProtocol.MESSAGES_V1, ['low', 'medium', 'high', 'max'])
            plain = provider('plain', b.BindingGatewayProtocol.RESPONSES_V1, None)

            offered = {(runtime, name): app.reasoning_efforts(runtime, key)
                       for runtime, name, key in (('codex', 'responses', responses), ('claude', 'messages', messages),
                                                  ('pi', 'responses', responses), ('codex', 'plain', plain))}
            assert offered == {
                ('codex', 'responses'): ['none', 'low', 'medium', 'high', 'xhigh'],
                ('claude', 'messages'): ['low', 'medium', 'high', 'max'],
                ('pi', 'responses'): ['off', 'low', 'medium', 'high', 'xhigh'],
                ('codex', 'plain'): [],
            }, offered
            print('PASS levels offered follow the model declaration in each runtime\'s own vocabulary', flush=True)

            def wait_idle(runtime, conversation, label, timeout=60):
                deadline = time.monotonic() + timeout
                while time.monotonic() < deadline:
                    current = app.snapshot(runtime, conversation).snapshot
                    if current.run_state == b.BindingRunState.FAILED:
                        raise AssertionError(label + ' failed')
                    if current.actions.can_send and any(
                            isinstance(block, b.BindingMessageBlock.TEXT) and block.text == 'SYNTHETIC_REPLY'
                            for message in current.messages[-1:] for block in message.blocks):
                        return current
                    time.sleep(.1)
                raise AssertionError('%s did not settle: %s %r' % (label, current.run_state, [
                    (message.role, [getattr(block, 'text', type(block).__name__) for block in message.blocks])
                    for message in current.messages[-3:]]))

            def turn(runtime, conversation, key, text, effort):
                before = len(captures)
                conversation = app.send_turn_with_selection(
                    runtime, conversation, runtime, b.BindingModelSelection.MANAGED(model_record_key=key),
                    b.BindingComposedTurn(text=text, effort=effort, attachments=[])).snapshot.conversation.id
                wait_idle(runtime, conversation, '%s %s' % (runtime, text))
                sent = [body for _, body in captures[before:] if text in json.dumps(body)]
                assert sent, (runtime, text, 'no upstream request')
                return sent[-1], conversation

            def refused(runtime, conversation, key, effort):
                before = len(captures)
                try:
                    app.send_turn_with_selection(runtime, conversation, runtime, b.BindingModelSelection.MANAGED(model_record_key=key),
                                                 b.BindingComposedTurn(text="NEVER_SENT", effort=effort, attachments=[]))
                except b.BindingError:
                    pass
                else:
                    raise AssertionError('%s accepted reasoning level %s' % (runtime, effort))
                assert len(captures) == before, 'a refused turn reached the upstream'

            reasoning = {
                'codex': lambda body: (body.get('reasoning') or {}).get('effort'),
                'pi': lambda body: (body.get('reasoning') or {}).get('effort'),
                'claude': lambda body: ((body.get('output_config') or {}).get('effort'), (body.get('thinking') or {}).get('type')),
            }
            for runtime, key, chosen, outside in (('codex', responses, 'high', 'max'),
                                                  ('claude', messages, 'max', 'xhigh'),
                                                  ('pi', responses, 'high', 'max')):
                conversation = app.create_conversation(runtime, str(root / 'project'), key).snapshot.conversation.id
                refused(runtime, conversation, key, outside)
                refused(runtime, conversation, key, 'bogus')
                body, conversation = turn(runtime, conversation, key, 'BASELINE_' + runtime, None)
                baseline = reasoning[runtime](body)
                body, conversation = turn(runtime, conversation, key, 'CHOSEN_' + runtime, chosen)
                picked = reasoning[runtime](body)
                body, conversation = turn(runtime, conversation, key, 'KEPT_' + runtime, None)
                kept = reasoning[runtime](body)
                print('INFO %s upstream reasoning: default %r, chosen %r, next default %r' % (runtime, baseline, picked, kept), flush=True)
                assert (picked[0] if runtime == 'claude' else picked) == chosen, (runtime, picked)
                # Under Velune a level applies to its turn: Codex sends none
                # by default, Claude Code is launched per turn and Pi's
                # session starts each execution at off.
                assert kept == baseline != picked, (runtime, baseline, kept)
            print('PASS a chosen level reaches the upstream through each harness for its turn only; '
                  'levels outside the offer are refused unsent', flush=True)
        finally:
            app.shutdown()
            server.shutdown()


if __name__ == '__main__':
    main()
