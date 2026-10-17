#!/usr/bin/env python3
"""Manual acceptance for steering a message into a running turn.

Real Codex 0.159.3, Claude Code 2.1.286 and Pi 1.0.2 run in temporary HOMEs
against gateway providers whose upstream is a loopback synthetic server that
holds each runtime's first model request. While it is held, a message is
steered in directly and another from the send queue. Checks that steering is
offered while the turn runs, that both messages reach the model within the
same turn (no new turn starts and exactly one turn ends) and that the queue
is left empty. No real model, credential, HOME or session is read; this is
not an automated test.
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
    held = {'codex', 'claude', 'pi'}
    gates = pathlib.Path(tempfile.mkdtemp(prefix='velune-steer-gates-'))

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
            # The first request of each runtime's held turn waits for its gate.
            text = json.dumps(body)
            for runtime in list(held):
                if 'HOLD_' + runtime in text and 'STEER_' + runtime not in text:
                    held.discard(runtime)
                    gate = gates / runtime
                    deadline = time.monotonic() + 60
                    while gate.exists() and time.monotonic() < deadline:
                        time.sleep(.05)
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

    with tempfile.TemporaryDirectory(prefix='velune-steer-') as directory:
        root = pathlib.Path(directory).resolve()
        (root / 'project').mkdir()
        spec = importlib.util.spec_from_file_location('velune_steer', args.bindings / 'velune_bindings.py')
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

            def provider(identifier, protocol):
                app.save_provider('default', b.BindingProviderDraft(
                    id=identifier, name=identifier, protocol=protocol, endpoint=endpoint,
                    models=[b.BindingProviderModel(record_key='', provider_model_id='synthetic-' + identifier, nickname=identifier,
                                                   icon=None, context_window=32768, max_output_tokens=1024,
                                                   reasoning_levels=None, adapter_metadata_json=None)], quota_limits=[], execution_backend=b.BindingExecutionBackend.HTTP(), request_profile=b.BindingRequestProfile.NATIVE, authentication_source=b.BindingProviderAuthenticationSource.API_KEY, quota_source=b.BindingProviderQuotaSource.NONE),
                    b.BindingAuthenticationEdit.SET_API_KEY(value='SYNTHETIC_ONLY'))
                return next(p for p in app.list().gateways[0].providers if p.id == identifier).models[0].record_key

            responses = provider('responses', b.BindingGatewayProtocol.RESPONSES_V1)
            messages = provider('messages', b.BindingGatewayProtocol.MESSAGES_V1)

            def snapshot(runtime, conversation):
                return app.snapshot(runtime, conversation).snapshot

            def wait(predicate, label, timeout=60):
                deadline = time.monotonic() + timeout
                while time.monotonic() < deadline:
                    value = predicate()
                    if value:
                        return value
                    time.sleep(.1)
                raise AssertionError(label)

            Q = b.BindingSendQueueOperation
            for runtime, key in (('codex', responses), ('claude', messages), ('pi', responses)):
                (gates / runtime).write_text('hold')
                conversation = app.create_conversation(runtime, str(root / 'project'), key).snapshot.conversation.id
                selection = b.BindingModelSelection.MANAGED(model_record_key=key)
                conversation = app.send_turn_with_selection(runtime, conversation, runtime, selection,
                    b.BindingComposedTurn(text='HOLD_' + runtime, effort=None, attachments=[])).snapshot.conversation.id
                wait(lambda: runtime not in held, runtime + ' reached the upstream')
                cursor = app.conversation_activity(0).cursor
                current = wait(lambda: (lambda s: s if s.run_state == b.BindingRunState.RUNNING and s.follow_up.can_steer else None)(snapshot(runtime, conversation)),
                               runtime + ' offers steering while running')
                before = len(captures)
                app.steer_turn(runtime, conversation, b.BindingComposedTurn(text='STEER_' + runtime, effort=None, attachments=[]))
                queued = app.send_queue(runtime, conversation, Q.ADD(source_runtime_id=runtime, target_runtime_id=runtime, selection=selection,
                    turn=b.BindingComposedTurn(text='QSTEER_' + runtime, effort=None, attachments=[]))).snapshot
                app.send_queue(runtime, conversation, Q.STEER(id=queued.follow_up.queue[0].id))
                assert snapshot(runtime, conversation).follow_up.queue == [], 'a steered message leaves the queue'
                (gates / runtime).unlink()

                def settled():
                    current = snapshot(runtime, conversation)
                    if current.run_state == b.BindingRunState.FAILED:
                        raise AssertionError(runtime + ' failed')
                    return current.run_state != b.BindingRunState.RUNNING and current.actions.can_send
                wait(settled, runtime + ' settled')
                later = [json.dumps(body) for _, body in captures[before:]]
                assert any('STEER_' + runtime in text and 'QSTEER_' + runtime in text for text in later), (runtime, 'steered messages did not reach the model')
                finished = [turn.outcome for turn in app.conversation_activity(cursor).finished if turn.conversation_id == conversation]
                assert finished == [b.BindingTurnOutcome.COMPLETED], (runtime, finished)
                users = [block.text for message in snapshot(runtime, conversation).messages if message.role == b.BindingMessageRole.USER
                         for block in message.blocks if isinstance(block, b.BindingMessageBlock.TEXT)]
                print('PASS', runtime, 'takes direct and queued messages into the running turn; one turn ended; transcript users:', users, flush=True)
        finally:
            for _ in range(100):
                try:
                    app.shutdown()
                    break
                except b.BindingError:
                    time.sleep(.1)
            server.shutdown()


if __name__ == '__main__':
    main()
