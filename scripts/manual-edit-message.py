#!/usr/bin/env python3
"""Manual acceptance for editing a user message.

Real Codex 0.159.3, Claude Code 2.1.286 and Pi 1.0.2 run in temporary HOMEs
against gateway providers whose upstream is a loopback synthetic server. Each
runtime's conversation gets three messages; then the last is edited in place,
the second is edited into a fork, the second is edited in place, and the
first is edited into a fork and (except Claude Code, which offers no record
before it) in place. Checks the messages each model request carries, what
each conversation shows and, after reopening the application, what the
native histories hold. No real model, credential, HOME or session is read;
this is not an automated test.
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
            captures.append(json.dumps(body, ensure_ascii=False))
            if path.endswith('/responses'):
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

    with tempfile.TemporaryDirectory(prefix='velune-edit-') as directory:
        root = pathlib.Path(directory).resolve()
        (root / 'project').mkdir()
        spec = importlib.util.spec_from_file_location('velune_edit', args.bindings / 'velune_bindings.py')
        b = importlib.util.module_from_spec(spec)
        sys.modules[spec.name] = b
        spec.loader.exec_module(b)
        options = b.BindingOptions(home_directory=str(root / 'velune'),
                                   resources_directory=str(args.bundle / 'Contents/Resources'))

        def close(app):
            for _ in range(100):
                try:
                    app.shutdown()
                    return
                except b.BindingError:
                    time.sleep(.1)

        app = b.VeluneApplication.open(options)
        expected = {}
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

            def wait(predicate, label, timeout=90):
                deadline = time.monotonic() + timeout
                while time.monotonic() < deadline:
                    value = predicate()
                    if value:
                        return value
                    time.sleep(.1)
                raise AssertionError(label)

            def settled(runtime, conversation):
                def check():
                    current = app.snapshot(runtime, conversation).snapshot
                    if current.run_state == b.BindingRunState.FAILED:
                        raise AssertionError((runtime, conversation, 'failed'))
                    return current if current.run_state != b.BindingRunState.RUNNING and current.actions.can_send else None
                return wait(check, (runtime, conversation, 'settled'))

            def users(snapshot):
                return [' '.join(block.text for block in message.blocks if isinstance(block, b.BindingMessageBlock.TEXT))
                        for message in snapshot.messages if message.role == b.BindingMessageRole.USER]

            def user_ids(snapshot):
                return [message.id for message in snapshot.messages if message.role == b.BindingMessageRole.USER]

            def requests_since(mark, token):
                later = [text for text in captures[mark:] if token in text]
                assert later, ('no model request carried', token)
                return later

            def edit(runtime, conversation, ordinal, placement, text, key):
                current = settled(runtime, conversation)
                message = user_ids(current)[ordinal]
                editable = {item.message_id: item.in_place for item in current.follow_up.editable}
                assert message in editable, (runtime, ordinal, 'not editable', editable)
                mark = len(captures)
                result = app.edit_message(runtime, conversation, message, placement,
                                          b.BindingModelSelection.MANAGED(model_record_key=key),
                                          b.BindingComposedTurn(text=text, effort=None, attachments=[])).snapshot
                identifier = result.conversation.id
                return identifier, settled(runtime, identifier), mark

            for runtime, key in (('codex', responses), ('claude', messages), ('pi', responses)):
                token = lambda name: '%s_%s' % (runtime.upper(), name)
                conversation = app.create_conversation(runtime, str(root / 'project'), key).snapshot.conversation.id
                for name in ('FIRST', 'SECOND', 'THIRD'):
                    conversation = app.send_turn_with_selection(runtime, conversation, runtime,
                        b.BindingModelSelection.MANAGED(model_record_key=key),
                        b.BindingComposedTurn(text=token(name), effort=None, attachments=[])).snapshot.conversation.id
                    settled(runtime, conversation)
                current = settled(runtime, conversation)
                assert users(current) == [token('FIRST'), token('SECOND'), token('THIRD')], users(current)
                in_place = [item.in_place for item in current.follow_up.editable]
                assert in_place == ([False, True, True] if runtime == 'claude' else [True, True, True]), (runtime, in_place)

                # The last message, in place.
                same, current, mark = edit(runtime, conversation, 2, b.BindingEditPlacement.IN_PLACE, token('EDITED_THIRD'), key)
                assert same == conversation, (runtime, 'in place stays in the conversation')
                for text in requests_since(mark, token('EDITED_THIRD')):
                    assert token('SECOND') in text and token('THIRD') not in text.replace(token('EDITED_THIRD'), ''), (runtime, 'last in place')
                assert users(current) == [token('FIRST'), token('SECOND'), token('EDITED_THIRD')], (runtime, users(current))

                # The second, into a fork; the original stays.
                fork, current, mark = edit(runtime, conversation, 1, b.BindingEditPlacement.FORK, token('FORKED_SECOND'), key)
                assert fork != conversation, (runtime, 'a fork is a new conversation')
                for text in requests_since(mark, token('FORKED_SECOND')):
                    assert token('FIRST') in text and token('SECOND') not in text.replace(token('FORKED_SECOND'), ''), (runtime, 'fork')
                assert users(current) == [token('FIRST'), token('FORKED_SECOND')], (runtime, users(current))
                original = settled(runtime, conversation)
                assert users(original) == [token('FIRST'), token('SECOND'), token('EDITED_THIRD')], (runtime, users(original))

                # The second, in place: what followed it goes.
                _, current, mark = edit(runtime, conversation, 1, b.BindingEditPlacement.IN_PLACE, token('REPLACED_SECOND'), key)
                for text in requests_since(mark, token('REPLACED_SECOND')):
                    assert token('FIRST') in text and token('EDITED_THIRD') not in text, (runtime, 'second in place')
                assert users(current) == [token('FIRST'), token('REPLACED_SECOND')], (runtime, users(current))

                # The first, into a fork: nothing precedes it.
                first_fork, current, mark = edit(runtime, conversation, 0, b.BindingEditPlacement.FORK, token('NEW_FIRST'), key)
                assert first_fork not in (conversation, fork), (runtime, 'first fork is new')
                for text in requests_since(mark, token('NEW_FIRST')):
                    assert token('FIRST') not in text.replace(token('NEW_FIRST'), ''), (runtime, 'first fork')
                assert users(current) == [token('NEW_FIRST')], (runtime, users(current))
                expected[(runtime, first_fork)] = [token('NEW_FIRST')]

                # The first, in place, where the runtime has a point before it.
                if runtime == 'claude':
                    current = settled(runtime, conversation)
                    try:
                        app.edit_message(runtime, conversation, user_ids(current)[0], b.BindingEditPlacement.IN_PLACE,
                                         b.BindingModelSelection.MANAGED(model_record_key=key),
                                         b.BindingComposedTurn(text='NEVER', effort=None, attachments=[]))
                        raise AssertionError('claude replaced its first message in place')
                    except b.BindingError:
                        pass
                    expected[(runtime, conversation)] = [token('FIRST'), token('REPLACED_SECOND')]
                else:
                    _, current, mark = edit(runtime, conversation, 0, b.BindingEditPlacement.IN_PLACE, token('REDONE_FIRST'), key)
                    for text in requests_since(mark, token('REDONE_FIRST')):
                        assert token('FIRST') not in text.replace(token('REDONE_FIRST'), ''), (runtime, 'first in place')
                    assert users(current) == [token('REDONE_FIRST')], (runtime, users(current))
                    expected[(runtime, conversation)] = [token('REDONE_FIRST')]
                expected[(runtime, fork)] = [token('FIRST'), token('FORKED_SECOND')]
                print('PASS', runtime, 'edits the last, a fork, earlier in place and the first message', flush=True)
        finally:
            close(app)

        # The native histories, read afresh.
        app = b.VeluneApplication.open(options)
        try:
            for (runtime, conversation), want in expected.items():
                got = users(app.read_conversation(runtime, conversation).snapshot)
                assert got == want, (runtime, conversation, got, want)
            print('PASS native histories after reopening:', len(expected), 'conversations', flush=True)
        finally:
            close(app)
            server.shutdown()


if __name__ == '__main__':
    main()
