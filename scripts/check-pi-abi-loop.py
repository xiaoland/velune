#!/usr/bin/env python3
"""Temporary, manually invoked end-to-end acceptance aid; not a CI test suite.

Exercise the embedded ABI with fixed Pi and a synthetic loopback upstream.

All mutable state and HOME are temporary. No user credentials or sessions are
read. Supply absolute library, Node, and bundled Resources paths explicitly.
"""
import argparse
import ctypes
import json
import os
from pathlib import Path
import shutil
import subprocess
import tempfile
import threading
import time
from http.server import BaseHTTPRequestHandler, ThreadingHTTPServer


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('--library', required=True, type=Path)
    parser.add_argument('--resources', required=True, type=Path)
    parser.add_argument('--node', required=True, type=Path)
    parser.add_argument('--protocol', choices=('chatCompletionsV1', 'responsesV1'), default='chatCompletionsV1')
    parser.add_argument('--subscription-capability', action='store_true',
                        help='mark the managed selection as an OAuth subscription route')
    args = parser.parse_args()
    for path in (args.library, args.resources, args.node):
        if not path.is_absolute() or not path.exists():
            parser.error('paths must be absolute and exist')
    requests = []
    waiting = threading.Event()
    release = threading.Event()

    class Upstream(BaseHTTPRequestHandler):
        def log_message(self, *_):
            pass

        def do_POST(self):
            body = json.loads(self.rfile.read(int(self.headers['Content-Length'])))
            requests.append(body)
            self.send_response(200)
            self.send_header('Content-Type', 'text/event-stream')
            self.end_headers()
            if len(requests) == 3:
                waiting.set()
                release.wait(15)
                return
            if args.protocol == 'responsesV1':
                response = {'id': 'fixture', 'status': 'completed',
                            'output': [{'type': 'message', 'role': 'assistant',
                                        'content': [{'type': 'output_text', 'text': 'VELUNE_ABI_OK'}]}],
                            'usage': {'input_tokens': 4, 'output_tokens': 3, 'total_tokens': 7}}
                created_response = {'id': 'fixture', 'status': 'in_progress', 'output': []}
                item = {'type': 'message', 'id': 'msg_fixture', 'role': 'assistant',
                        'status': 'in_progress', 'content': []}
                events = [
                    ('response.created', {'type': 'response.created', 'response': created_response}),
                    ('response.output_item.added', {'type': 'response.output_item.added', 'output_index': 0, 'item': item}),
                    ('response.content_part.added', {'type': 'response.content_part.added', 'item_id': 'msg_fixture',
                                                     'output_index': 0, 'content_index': 0,
                                                     'part': {'type': 'output_text', 'text': ''}}),
                    ('response.output_text.delta', {'type': 'response.output_text.delta', 'delta': 'VELUNE_ABI_OK'}),
                    ('response.output_text.done', {'type': 'response.output_text.done', 'text': 'VELUNE_ABI_OK'}),
                    ('response.content_part.done', {'type': 'response.content_part.done', 'item_id': 'msg_fixture',
                                                    'output_index': 0, 'content_index': 0,
                                                    'part': {'type': 'output_text', 'text': 'VELUNE_ABI_OK'}}),
                    ('response.output_item.done', {'type': 'response.output_item.done', 'output_index': 0,
                                                   'item': {**item, 'status': 'completed',
                                                            'content': [{'type': 'output_text', 'text': 'VELUNE_ABI_OK'}]}}),
                    ('response.completed', {'type': 'response.completed', 'response': response}),
                ]
                for event, value in events:
                    self.wfile.write((f'event: {event}\ndata: {json.dumps(value)}\n\n').encode())
                self.wfile.flush()
                return
            chunk = {'id': 'fixture', 'object': 'chat.completion.chunk',
                     'created': 1, 'model': body['model'],
                     'choices': [{'index': 0, 'delta': {'content': 'VELUNE_ABI_OK'},
                                  'finish_reason': None}]}
            self.wfile.write(('data: ' + json.dumps(chunk) + '\n\n').encode())
            chunk['choices'][0] = {'index': 0, 'delta': {}, 'finish_reason': 'stop'}
            chunk['usage'] = {'prompt_tokens': 4, 'completion_tokens': 3, 'total_tokens': 7}
            self.wfile.write(('data: ' + json.dumps(chunk) + '\n\ndata: [DONE]\n\n').encode())
            self.wfile.flush()

    server = ThreadingHTTPServer(('127.0.0.1', 0), Upstream)
    thread = threading.Thread(target=server.serve_forever, daemon=True)
    thread.start()
    original_environment = dict(os.environ)
    with tempfile.TemporaryDirectory(prefix='velune-abi-loop-') as temporary:
        root = Path(temporary)
        for name in ('home', 'resources', 'work', 'agent', 'sessions'):
            (root / name).mkdir()
        for helper in ('pi_sessions.mjs', 'pi_virtual_model.mjs', 'pi_auth.mjs'):
            shutil.copy(args.resources / helper, root / 'resources' / helper)
        (root / 'resources/node_modules').symlink_to(args.resources / 'node_modules', target_is_directory=True)
        resolver = root / 'fixture-credential'
        resolver.write_text('#!/bin/sh\nprintf "fixture-only"\n')
        resolver.chmod(0o700)
        os.environ.clear()
        os.environ.update(HOME=str(root / 'home'), PATH=f'{args.node.parent}:/usr/bin:/bin',
                          NO_PROXY='127.0.0.1,localhost')
        library = ctypes.CDLL(str(args.library))
        library.velune_core_abi_version.restype = ctypes.c_uint32
        library.velune_core_open.argtypes = [ctypes.c_char_p, ctypes.POINTER(ctypes.c_void_p)]
        library.velune_core_open.restype = ctypes.c_void_p
        library.velune_core_request.argtypes = [ctypes.c_void_p, ctypes.c_char_p]
        library.velune_core_request.restype = ctypes.c_void_p
        library.velune_core_close.argtypes = [ctypes.POINTER(ctypes.c_void_p)]
        library.velune_core_close.restype = ctypes.c_void_p
        library.velune_core_string_free.argtypes = [ctypes.c_void_p]
        handle = ctypes.c_void_p()

        def take(pointer):
            try:
                return json.loads(ctypes.string_at(pointer))
            finally:
                library.velune_core_string_free(pointer)

        def request(action, **payload):
            payload.setdefault('runtimeInstanceID', 'fixture-pi')
            response = take(library.velune_core_request(handle, json.dumps(
                {'version': 3, 'action': action, 'payload': payload}).encode()))
            assert response['ok'], (action, response)
            return response['data']

        def snapshot_until_idle():
            deadline = time.monotonic() + 15
            while time.monotonic() < deadline:
                snapshot = request('getSnapshot')['snapshot']
                if snapshot['runState'] == 'idle':
                    return snapshot
                time.sleep(0.1)
            raise AssertionError('Pi did not settle')

        options = json.dumps({'homeDirectory': str(root / 'home'),
                              'resourcesDirectory': str(root / 'resources'),
                              'credentialResolver': str(resolver)}).encode()
        try:
            assert library.velune_core_abi_version() == 1
            assert take(library.velune_core_open(options, ctypes.byref(handle)))['ok']
            models = [{'id': name, 'nickname': name, 'icon': None,
                       'contextWindow': 32768 if name == 'first' else 65536,
                       'maxOutputTokens': cap, 'reasoningLevels': []}
                      for name, cap in [('first', 64), ('second', 80)]]
            gateway = {'id': 'fixture', 'name': 'Fixture', 'models': models,
                       'providers': [{'id': 'local', 'name': 'Local fixture',
                                      'protocol': args.protocol,
                                      'endpoint': f'http://127.0.0.1:{server.server_port}/v1',
                                      'credentialRef': 'fixture',
                                      'models': [{'modelId': item['id'], 'externalModelId': 'external-' + item['id']}
                                                 for item in models]}],
                       'routes': [{'modelId': item['id'], 'providerId': 'local'} for item in models],
                       'failover': {'mode': 'disabled'}}
            request('gateways', operation='upsert', gateway=json.dumps(gateway))
            runtime = {'id': 'fixture-pi', 'name': 'Isolated Pi', 'typeId': 'pi',
                       'gatewayId': 'fixture', 'modelId': 'first',
                       'settings': {'binary': str(root / 'resources/node_modules/@earendil-works/pi-coding-agent/dist/bundle/cli.js'),
                                    'nodeBinary': str(args.node), 'workingDir': str(root / 'work'),
                                    'agentDir': str(root / 'agent'), 'sessionDir': str(root / 'sessions')}}
            request('runtimeInstances', operation='upsert', runtimeInstance=json.dumps(runtime))
            request('runtimeAction', actionID='connect')
            catalog = json.loads((root / 'agent/models.json').read_text())
            catalog_models = catalog['providers']['velune-gateway']['models']
            assert [item['contextWindow'] for item in catalog_models] == [32768, 65536]
            physical_ids = {item['logicalModelId']: item['id'] for item in catalog_models}
            assert physical_ids['first'] != 'first' and physical_ids['second'] != 'second'
            transform = (args.resources / 'node_modules/@earendil-works/pi-ai/dist/api/transform-messages.js').resolve().as_uri()
            check = 'import {transformMessages} from ' + json.dumps(transform) + ';' + '''
              const old = {role:'assistant', provider:'velune-gateway', api:'openai-completions', model:'first',
                content:[{type:'thinking',thinking:'',thinkingSignature:'synthetic',redacted:true},{type:'text',text:'ok'}]};
              const model = {id:'second', provider:'velune-gateway', api:'openai-completions', input:['text']};
              if(transformMessages([old], model)[0].content.some(x => x.thinkingSignature)) throw Error('signature leaked');
              model.id='first';
              if(!transformMessages([old], model)[0].content.some(x => x.thinkingSignature)) throw Error('identity lost');
            '''
            subprocess.run([str(args.node), '--input-type=module', '-e', check], check=True)
            request('create')
            if args.subscription_capability:
                selection = root / 'agent/velune-selection.json'
                value = json.loads(selection.read_text())
                value['subscriptionCapability'] = True
                selection.write_text(json.dumps(value))
            started = request('send', text='Return the fixture response.')
            assert started['snapshot']['runState'] == 'running', started
            first = snapshot_until_idle()
            assert 'VELUNE_ABI_OK' in json.dumps(first), (first, requests)
            session_entries = [json.loads(line) for line in Path(first['conversation']['id'].split(':', 1)[1]).read_text().splitlines()]
            assert any(entry.get('type') == 'model_change' and entry.get('provider') == 'velune' and entry.get('modelId') == 'auto'
                       for entry in session_entries)
            assert any(entry.get('type') == 'message' and entry.get('message', {}).get('role') == 'assistant'
                       and entry['message'].get('provider') == 'velune-gateway'
                       and entry['message'].get('model') == physical_ids['first'] for entry in session_entries)
            assert requests[0]['model'] == 'external-first'
            if args.protocol == 'chatCompletionsV1':
                assert requests[0]['max_completion_tokens'] == 64
            elif args.subscription_capability:
                assert all(key not in requests[0] for key in (
                    'max_output_tokens', 'temperature', 'prompt_cache_retention',
                    'prompt_cache_options', 'prompt_cache_key'))
            else:
                assert requests[0]['max_output_tokens'] == 64
            session_id = first['conversation']['id']
            assert session_id.startswith('fixture-pi:/')
            listed = request('list')['conversations']
            assert session_id in [item['id'] for item in listed], (session_id, listed, list((root / 'sessions').rglob('*')))
            request('selectModel', modelID='second')
            restored_before_send = request('open', conversationID=session_id)['snapshot']
            assert restored_before_send['modelId'] == 'second', restored_before_send
            if args.subscription_capability:
                selection = root / 'agent/velune-selection.json'
                value = json.loads(selection.read_text())
                value['subscriptionCapability'] = True
                selection.write_text(json.dumps(value))
            request('send', text='Return another fixture response.')
            snapshot_until_idle()
            session_entries = [json.loads(line) for line in Path(session_id.split(':', 1)[1]).read_text().splitlines()]
            assert any(entry.get('type') == 'message' and entry.get('message', {}).get('role') == 'assistant'
                       and entry['message'].get('provider') == 'velune-gateway'
                       and entry['message'].get('model') == physical_ids['second'] for entry in session_entries)
            assert requests[1]['model'] == 'external-second'
            if args.protocol == 'chatCompletionsV1':
                assert requests[1]['max_completion_tokens'] == 80
            elif args.subscription_capability:
                assert all(key not in requests[1] for key in (
                    'max_output_tokens', 'temperature', 'prompt_cache_retention',
                    'prompt_cache_options', 'prompt_cache_key'))
            else:
                assert requests[1]['max_output_tokens'] == 80
            new_session = request('create')['snapshot']
            assert new_session['modelId'] == 'first'
            restored = request('open', conversationID=session_id)['snapshot']
            assert restored['modelId'] == 'second', restored
            request('send', text='Wait for cancellation.')
            assert waiting.wait(10), 'third request did not reach local upstream'
            old_handle = handle.value
            assert not take(library.velune_core_close(ctypes.byref(handle)))['ok']
            assert handle.value == old_handle
            request('cancel')
            snapshot_until_idle()
            release.set()
            assert take(library.velune_core_close(ctypes.byref(handle)))['ok']
            assert not handle.value
            # Build synthetic signed history through Pi's own session API, then
            # change only the upstream binding of the same logical model.
            sdk = (root / 'resources/node_modules/@earendil-works/pi-coding-agent/dist/index.js').resolve().as_uri()
            history = [
                {'role': 'assistant', 'provider': 'velune-gateway',
                 'api': 'openai-responses' if args.protocol == 'responsesV1' else 'openai-completions',
                 'model': physical_ids['second'], 'stopReason': 'toolUse', 'timestamp': 1,
                 'content': [{'type': 'thinking', 'thinking': '', 'redacted': True,
                              'thinkingSignature': 'SYNTHETIC_OLD_BINDING_SIGNATURE'},
                             {'type': 'toolCall', 'id': 'call_binding_fixture', 'name': 'read',
                              'arguments': {'path': 'synthetic.txt'},
                              'thoughtSignature': 'SYNTHETIC_OLD_BINDING_SIGNATURE'}],
                 'usage': {'input': 1, 'output': 1, 'cacheRead': 0, 'cacheWrite': 0,
                           'totalTokens': 2, 'cost': {'input': 0, 'output': 0, 'cacheRead': 0, 'cacheWrite': 0, 'total': 0}}},
                {'role': 'toolResult', 'toolCallId': 'call_binding_fixture', 'toolName': 'read',
                 'content': [{'type': 'text', 'text': 'SYNTHETIC_TOOL_RESULT'}],
                 'isError': False, 'timestamp': 2},
            ]
            append = ('import {SessionManager} from ' + json.dumps(sdk) + ';'
                      'const manager = SessionManager.open(' + json.dumps(session_id.split(':', 1)[1]) + ');'
                      'for (const message of ' + json.dumps(history) + ') manager.appendMessage(message);')
            subprocess.run([str(args.node), '--input-type=module', '-e', append], check=True)
            assert take(library.velune_core_open(options, ctypes.byref(handle)))['ok']
            assert request('list')['runtimeInstances'][0]['modelId'] == 'first'
            gateway['providers'][0]['models'][1]['externalModelId'] = 'external-second-rebound'
            request('gateways', operation='upsert', gateway=json.dumps(gateway))
            request('runtimeAction', actionID='connect')
            catalog = json.loads((root / 'agent/models.json').read_text())
            rebound = next(item['id'] for item in catalog['providers']['velune-gateway']['models']
                           if item['logicalModelId'] == 'second')
            assert rebound != physical_ids['second']
            assert request('open', conversationID=session_id)['snapshot']['modelId'] == 'second'
            if args.subscription_capability:
                selection = root / 'agent/velune-selection.json'
                value = json.loads(selection.read_text()); value['subscriptionCapability'] = True
                selection.write_text(json.dumps(value))
            request('send', text='Return a response after changing the route binding.')
            snapshot_until_idle()
            assert requests[3]['model'] == 'external-second-rebound'
            wire = json.dumps(requests[3])
            assert 'SYNTHETIC_OLD_BINDING_SIGNATURE' not in wire
            assert 'SYNTHETIC_TOOL_RESULT' in wire
            if args.protocol == 'responsesV1':
                items = requests[3]['input']
                call = next(item for item in items if item.get('type') == 'function_call')
                result = next(item for item in items if item.get('type') == 'function_call_output')
                assert call['call_id'] == result['call_id']
            else:
                items = requests[3]['messages']
                call = next(item['tool_calls'][0] for item in items if item.get('tool_calls'))
                result = next(item for item in items if item.get('role') == 'tool')
                assert call['id'] == result['tool_call_id']

            assert not (root / 'home/ipc.sock').exists()
            assert not (root / 'home/core.sqlite').exists()
            print('PASS: ABI streaming, auto selection/restore, binding change clears signatures and preserves tools, busy close, cancel, config reopen')
        finally:
            release.set()
            if handle.value:
                take(library.velune_core_close(ctypes.byref(handle)))
            os.environ.clear()
            os.environ.update(original_environment)
            server.shutdown()
            server.server_close()


if __name__ == '__main__':
    main()
