#!/usr/bin/env python3
"""Exercise the embedded ABI with fixed Pi and a synthetic loopback upstream.

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
        shutil.copy(args.resources / 'pi_sessions.mjs', root / 'resources/pi_sessions.mjs')
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
                                      'protocol': 'chatCompletionsV1',
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
            request('send', text='Return the fixture response.')
            first = snapshot_until_idle()
            assert 'VELUNE_ABI_OK' in json.dumps(first), (first, requests)
            assert requests[0]['model'] == 'external-first'
            assert requests[0]['max_completion_tokens'] == 64
            session_id = first['conversation']['id']
            assert session_id.startswith('fixture-pi:/')
            listed = request('list')['conversations']
            assert session_id in [item['id'] for item in listed], (session_id, listed, list((root / 'sessions').rglob('*')))
            request('selectModel', modelID='second')
            request('send', text='Return another fixture response.')
            snapshot_until_idle()
            assert requests[1]['model'] == 'external-second'
            assert requests[1]['max_completion_tokens'] == 80
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
            assert take(library.velune_core_open(options, ctypes.byref(handle)))['ok']
            assert request('list')['runtimeInstances'][0]['modelId'] == 'first'
            assert not (root / 'home/ipc.sock').exists()
            assert not (root / 'home/core.sqlite').exists()
            print('PASS: ABI streaming, routed model selection, native session identity/restore, busy close, cancel, configuration reopen')
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
