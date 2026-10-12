#!/usr/bin/env python3
"""Manual acceptance for attaching images and files to a turn.

Real Codex 0.159.3, Claude Code 2.1.286 and Pi 1.0.2 run in temporary HOMEs
against gateway providers whose upstream is a loopback synthetic server.
Checks that an attached image reaches the upstream through each harness's own
image input, that other files are listed by absolute path in the text, that an
image alone is a turn, and that attachments which cannot be delivered (missing,
relative, a directory, an image whose content is not its type) refuse the turn
before anything is sent. No real model, credential, HOME or session is read;
this is not an automated test.
"""
import argparse
import base64
import importlib.util
import json
import pathlib
import sys
import tempfile
import threading
import time
from http.server import BaseHTTPRequestHandler, ThreadingHTTPServer

# A 1x1 PNG.
PNG = base64.b64decode('iVBORw0KGgoAAAANSUhEUgAAAAEAAAABCAYAAAAfFcSJAAAADUlEQVR42mP8z8BQDwAEhQGAhKmMIQAAAABJRU5ErkJggg==')


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

    with tempfile.TemporaryDirectory(prefix='velune-attach-') as directory:
        root = pathlib.Path(directory).resolve()
        (root / 'project').mkdir()
        image = root / 'pixel.png'
        image.write_bytes(PNG)
        notes = root / 'notes.txt'
        notes.write_text('ATTACHED_NOTES')
        fake = root / 'fake.png'
        fake.write_text('not an image')
        spec = importlib.util.spec_from_file_location('velune_attach', args.bindings / 'velune_bindings.py')
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
                                                   reasoning_levels=None, adapter_metadata_json=None)]),
                    b.BindingAuthenticationEdit.SET_API_KEY(value='SYNTHETIC_ONLY'))
                return next(p for p in app.list().gateways[0].providers if p.id == identifier).models[0].record_key

            responses = provider('responses', b.BindingGatewayProtocol.RESPONSES_V1)
            messages = provider('messages', b.BindingGatewayProtocol.MESSAGES_V1)

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
                raise AssertionError('%s did not settle: %s' % (label, current.run_state))

            def turn(runtime, conversation, key, text, attachments, marker):
                before = len(captures)
                conversation = app.send_turn_with_selection(
                    runtime, conversation, runtime, b.BindingModelSelection.MANAGED(model_record_key=key),
                    b.BindingComposedTurn(text=text, effort=None, attachments=attachments)).snapshot.conversation.id
                wait_idle(runtime, conversation, '%s %s' % (runtime, marker))
                sent = [body for _, body in captures[before:] if marker in json.dumps(body)]
                assert sent, (runtime, marker, 'no upstream request')
                return sent[0], conversation

            def refused(runtime, conversation, key, attachments, label):
                before = len(captures)
                try:
                    app.send_turn_with_selection(runtime, conversation, runtime, b.BindingModelSelection.MANAGED(model_record_key=key),
                                                 b.BindingComposedTurn(text='NEVER_SENT', effort=None, attachments=attachments))
                except b.BindingError:
                    pass
                else:
                    raise AssertionError('%s accepted %s' % (runtime, label))
                assert len(captures) == before, 'a refused turn reached the upstream'

            def last_user_parts(runtime, body):
                if runtime == 'claude':
                    message = [m for m in body['messages'] if m['role'] == 'user'][-1]
                    return message['content'] if isinstance(message['content'], list) else [{'type': 'text', 'text': message['content']}]
                items = [item for item in body['input'] if item.get('role') == 'user']
                content = items[-1]['content']
                return content if isinstance(content, list) else [{'type': 'input_text', 'text': content}]

            def images_in(parts):
                found = []
                for part in parts:
                    if part.get('type') == 'image':
                        found.append(base64.b64decode(part['source']['data']))
                    elif part.get('type') == 'input_image':
                        found.append(base64.b64decode(part['image_url'].split(',', 1)[1]))
                return found

            def text_in(parts):
                return '\n'.join(part.get('text', '') for part in parts)

            for runtime, key in (('codex', responses), ('claude', messages), ('pi', responses)):
                conversation = app.create_conversation(runtime, str(root / 'project'), key).snapshot.conversation.id
                for attachments, label in (([str(root / 'missing.png')], 'a missing file'),
                                           (['pixel.png'], 'a relative path'),
                                           ([str(root / 'project')], 'a directory'),
                                           ([str(fake)], 'an image whose content is not its type')):
                    refused(runtime, conversation, key, attachments, label)
                body, conversation = turn(runtime, conversation, key, 'LOOK_' + runtime, [str(image), str(notes)], 'LOOK_' + runtime)
                parts = last_user_parts(runtime, body)
                assert images_in(parts) == [PNG], (runtime, [part.get('type') for part in parts])
                assert ('LOOK_%s\n\n附件：\n- %s' % (runtime, notes)) in text_in(parts), (runtime, text_in(parts))
                print('PASS %s delivers the image through its own input and lists the file by path' % runtime, flush=True)

                def shown(snapshot_result, label):
                    current = snapshot_result.snapshot
                    message = next(m for m in reversed(current.messages)
                                   if m.role == b.BindingMessageRole.USER and any(
                                       isinstance(block, b.BindingMessageBlock.TEXT) and block.text == 'LOOK_' + runtime
                                       for block in m.blocks))
                    kinds = [type(block).__name__ for block in message.blocks]
                    assert [block.path for block in message.blocks if isinstance(block, b.BindingMessageBlock.FILE)] == [str(notes)], (label, kinds)
                    images = [block for block in message.blocks if isinstance(block, b.BindingMessageBlock.IMAGE)]
                    assert len(images) == 1, (label, kinds)
                    source = images[0].source
                    if isinstance(source, b.BindingImageSource.PATH):
                        assert source.path == str(image), (label, source.path)
                        how = 'path'
                    else:
                        payload = source.data
                        if isinstance(payload, b.BindingMessagePayload.DEFERRED):
                            loaded = app.message_payloads(current.context_runtime_id, current.conversation.id, [message.id])[0]
                            payload = next(block for block in loaded.blocks if isinstance(block, b.BindingMessageBlock.IMAGE)).source.data
                        assert base64.b64decode(payload.text) == PNG, label
                        how = 'content'
                    print('INFO %s %s shows text, the image by %s and the file' % (runtime, label, how), flush=True)

                shown(app.snapshot(runtime, conversation), 'live')
                shown(app.open_conversation(runtime, conversation), 'native record')
                print('PASS %s shows the attachments of a sent message, live and from its native record' % runtime, flush=True)
                before = len(captures)
                conversation = app.send_turn_with_selection(
                    runtime, conversation, runtime, b.BindingModelSelection.MANAGED(model_record_key=key),
                    b.BindingComposedTurn(text='', effort=None, attachments=[str(image)])).snapshot.conversation.id
                wait_idle(runtime, conversation, runtime + ' image only')
                def carries_image(body):
                    try:
                        return bool(images_in(last_user_parts(runtime, body)))
                    except (KeyError, IndexError):
                        return False
                sent = [body for _, body in captures[before:] if carries_image(body)]
                assert sent, (runtime, 'image-only turn sent no image')
                print('PASS %s sends a turn of an image alone' % runtime, flush=True)
            print('PASS attachments that cannot be delivered refuse the turn unsent', flush=True)
        finally:
            app.shutdown()
            server.shutdown()


if __name__ == '__main__':
    main()
