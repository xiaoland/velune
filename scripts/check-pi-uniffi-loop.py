#!/usr/bin/env python3
"""Temporary, manually invoked end-to-end acceptance aid; not a CI test suite.

Exercise the generated UniFFI API with fixed Pi and a synthetic loopback upstream.

All mutable state and HOME are temporary. No user credentials or sessions are
read. Supply absolute library, Node, and bundled Resources paths explicitly.
"""
import argparse
import enum
import importlib.util
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
    parser.add_argument('--bindings', required=True, type=Path)
    parser.add_argument('--resources', required=True, type=Path)
    parser.add_argument('--node', required=True, type=Path)
    parser.add_argument('--protocol', choices=('chatCompletionsV1', 'responsesV1'), default='chatCompletionsV1')
    parser.add_argument('--subscription-capability', action='store_true',
                        help='mark the managed selection as an OAuth subscription route')
    args = parser.parse_args()
    for path in (args.library, args.bindings, args.resources, args.node):
        if not path.is_absolute() or not path.exists():
            parser.error('paths must be absolute and exist')
    requests = []
    tool_result_requests = []
    waiting = threading.Event()
    release = threading.Event()

    class Upstream(BaseHTTPRequestHandler):
        def log_message(self, *_):
            pass

        def do_POST(self):
            body = json.loads(self.rfile.read(int(self.headers['Content-Length'])))
            requests.append(body)
            items = body.get('messages', []) if args.protocol == 'chatCompletionsV1' else body.get('input', [])
            last_user_index = next((index for index in range(len(items) - 1, -1, -1)
                                    if items[index].get('role') == 'user'), None)
            last_user = items[last_user_index] if last_user_index is not None else None
            last_user_text = last_user.get('content', '') if last_user else ''
            self.send_response(200)
            self.send_header('Content-Type', 'text/event-stream')
            self.end_headers()
            if 'Wait for cancellation.' in str(last_user_text):
                waiting.set()
                release.wait(15)
                return
            if args.protocol == 'chatCompletionsV1' and last_user_text == 'Verify tool cwd.':
                current_items = items[last_user_index + 1:] if last_user_index is not None else []
                tool_results = [item for item in current_items if item.get('role') == 'tool']
                has_tool_result = bool(tool_results)
                if has_tool_result:
                    tool_result_requests.append(tool_results[-1].get('content', ''))
                else:
                    call_id = f'call_pwd_{len(tool_result_requests) + 1}'
                    chunk = {'id': 'fixture-tool', 'object': 'chat.completion.chunk',
                             'created': 1, 'model': body['model'],
                             'choices': [{'index': 0, 'delta': {'tool_calls': [{
                                 'index': 0, 'id': call_id, 'type': 'function',
                                 'function': {'name': 'bash', 'arguments': json.dumps({'command': 'pwd'})}}]},
                                          'finish_reason': 'tool_calls'}]}
                    self.wfile.write(('data: ' + json.dumps(chunk) + '\n\ndata: [DONE]\n\n').encode())
                    self.wfile.flush()
                    return
            if args.protocol == 'responsesV1':
                response = {'id': 'fixture', 'status': 'completed',
                            'output': [{'type': 'message', 'role': 'assistant',
                                        'content': [{'type': 'output_text', 'text': 'VELUNE_UNIFFI_OK'}]}],
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
                    ('response.output_text.delta', {'type': 'response.output_text.delta', 'delta': 'VELUNE_UNIFFI_OK'}),
                    ('response.output_text.done', {'type': 'response.output_text.done', 'text': 'VELUNE_UNIFFI_OK'}),
                    ('response.content_part.done', {'type': 'response.content_part.done', 'item_id': 'msg_fixture',
                                                    'output_index': 0, 'content_index': 0,
                                                    'part': {'type': 'output_text', 'text': 'VELUNE_UNIFFI_OK'}}),
                    ('response.output_item.done', {'type': 'response.output_item.done', 'output_index': 0,
                                                   'item': {**item, 'status': 'completed',
                                                            'content': [{'type': 'output_text', 'text': 'VELUNE_UNIFFI_OK'}]}}),
                    ('response.completed', {'type': 'response.completed', 'response': response}),
                ]
                for event, value in events:
                    self.wfile.write((f'event: {event}\ndata: {json.dumps(value)}\n\n').encode())
                self.wfile.flush()
                return
            chunk = {'id': 'fixture', 'object': 'chat.completion.chunk',
                     'created': 1, 'model': body['model'],
                     'choices': [{'index': 0, 'delta': {'content': 'VELUNE_UNIFFI_OK'},
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
    with tempfile.TemporaryDirectory(prefix='velune-uniffi-loop-') as temporary:
        root = Path(temporary)
        for name in ('home', 'resources', 'work-a', 'work-b', 'agent', 'sessions'):
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
        # Load a copied generated module next to the explicitly selected library.
        # Never mutate the build directory or search a user's library paths.
        binding_directory = root / 'bindings'
        binding_directory.mkdir()
        shutil.copy(args.bindings / 'velune_bindings.py', binding_directory)
        (binding_directory / args.library.name).symlink_to(args.library)
        spec = importlib.util.spec_from_file_location('velune_bindings', binding_directory / 'velune_bindings.py')
        bindings = importlib.util.module_from_spec(spec)
        import sys
        sys.modules[spec.name] = bindings
        spec.loader.exec_module(bindings)
        application = None

        def view(value):
            """Keep existing manual assertions readable; all calls remain typed."""
            if value is None or isinstance(value, (str, int, bool, float)):
                return value
            if isinstance(value, enum.Enum):
                return value.name.lower()
            if isinstance(value, list):
                return [view(item) for item in value]
            if isinstance(value, dict):
                return {key: view(item) for key, item in value.items()}
            def camel(name):
                first, *rest = name.split('_')
                return first + ''.join(part.title() for part in rest)
            result = {camel(key): view(item) for key, item in vars(value).items()}
            if isinstance(value, bindings.BindingMessageBlock):
                result['kind'] = type(value).__name__.lower()
            return result

        def expect_error(operation):
            try:
                operation()
            except bindings.BindingError:
                return
            raise AssertionError('operation unexpectedly succeeded')

        def gateway_record(value):
            protocols = {
                'chatCompletionsV1': bindings.BindingGatewayProtocol.CHAT_COMPLETIONS_V1,
                'responsesV1': bindings.BindingGatewayProtocol.RESPONSES_V1,
            }
            return bindings.BindingGatewayConfig(
                id=value['id'], name=value['name'],
                models=[bindings.BindingModelDefinition(
                    id=item['id'], nickname=item['nickname'], icon=item['icon'],
                    context_window=item['contextWindow'], max_output_tokens=item['maxOutputTokens'],
                    reasoning_levels=item['reasoningLevels']) for item in value['models']],
                providers=[bindings.BindingProviderDefinition(
                    id=item['id'], name=item['name'], protocol=protocols[item['protocol']],
                    endpoint=item['endpoint'], credential_ref=item['credentialRef'],
                    credential_source=None, credential_generation=0,
                    models=[bindings.BindingProviderModelBinding(
                        model_id=model['modelId'], external_model_id=model['externalModelId'],
                        adapter_metadata_json=None) for model in item['models']]) for item in value['providers']],
                routes=[bindings.BindingRoute(model_id=item['modelId'], provider_id=item['providerId'])
                        for item in value['routes']],
                failover=bindings.BindingFailoverPolicy(mode=bindings.BindingFailoverMode.DISABLED))

        def snapshot_until_idle():
            deadline = time.monotonic() + 15
            while time.monotonic() < deadline:
                snapshot = view(application.snapshot('fixture-pi'))['snapshot']
                if snapshot['runState'] == 'failed':
                    raise AssertionError(('Pi failed', snapshot))
                if snapshot['runState'] == 'idle':
                    return snapshot
                time.sleep(0.1)
            raise AssertionError('Pi did not settle')

        def snapshot_until_ready():
            deadline = time.monotonic() + 15
            while time.monotonic() < deadline:
                snapshot = view(application.snapshot('fixture-pi'))['snapshot']
                if snapshot['runState'] == 'failed':
                    raise AssertionError(('Pi failed during startup', snapshot))
                if snapshot['runState'] == 'idle' and snapshot['actions']['canSend']:
                    return snapshot
                time.sleep(0.1)
            raise AssertionError('Pi did not become ready')

        def request_for_model(model):
            return next(item for item in reversed(requests) if item.get('model') == model)

        def wait_for_requests(count):
            deadline = time.monotonic() + 15
            while time.monotonic() < deadline and len(requests) < count:
                time.sleep(0.05)
            assert len(requests) >= count, (count, requests)

        def wait_for_tool_result(cwd):
            """Wait for the upstream body carrying the current tool result.

            Inspect actual upstream request bodies rather than assuming the
            auxiliary capture list has already observed the tool result.
            """
            deadline = time.monotonic() + 15
            while time.monotonic() < deadline:
                results = [
                    item.get('content', '')
                    for body in requests
                    for item in body.get('messages', [])
                    if item.get('role') == 'tool'
                ]
                if results and cwd in str(results[-1]):
                    return results[-1]
                time.sleep(0.05)
            raise AssertionError(('tool result not observed', cwd, requests))

        options = bindings.BindingOptions(home_directory=str(root / 'home'),
                                          resources_directory=str(root / 'resources'),
                                          credential_resolver=str(resolver))
        try:
            application = bindings.VeluneApplication.open(options)
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
            application.upsert_gateway(gateway_record(gateway))
            runtime = {'id': 'fixture-pi', 'name': 'Isolated Pi', 'typeId': 'pi',
                       'gatewayId': 'fixture', 'modelId': 'first',
                       'settings': {'binary': str(root / 'resources/node_modules/@earendil-works/pi-coding-agent/dist/bundle/cli.js'),
                                    'nodeBinary': str(args.node),
                                    'agentDir': str(root / 'agent'), 'sessionDir': str(root / 'sessions')}}
            application.upsert_runtime(bindings.BindingRuntimeInstance(
                id=runtime['id'], name=runtime['name'], type_id=runtime['typeId'],
                gateway_id=runtime['gatewayId'], model_id=runtime['modelId'], settings=runtime['settings']))
            application.connect_runtime('fixture-pi')
            selection_before_session = (root / 'agent/velune-selection.json').read_bytes()
            expect_error(lambda: application.select_model('fixture-pi', 'second'))
            assert (root / 'agent/velune-selection.json').read_bytes() == selection_before_session
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
            view(application.create_conversation('fixture-pi', str(root / 'work-a')))
            snapshot_until_ready()
            if args.subscription_capability:
                selection = root / 'agent/velune-selection.json'
                value = json.loads(selection.read_text())
                value['subscriptionCapability'] = True
                selection.write_text(json.dumps(value))
            expected_request_count = len(requests) + 1
            started = view(application.send('fixture-pi', 'Return the fixture response.'))
            assert started['snapshot']['runState'] == 'running', started
            wait_for_requests(expected_request_count)
            first = snapshot_until_idle()
            assert 'VELUNE_UNIFFI_OK' in json.dumps(first), (first, requests, 'session=' + session_id if 'session_id' in locals() else 'session=not-yet')
            session_entries = [json.loads(line) for line in Path(first['conversation']['id'].split(':', 1)[1]).read_text().splitlines()]
            assert any(entry.get('type') == 'model_change' and entry.get('provider') == 'velune' and entry.get('modelId') == 'auto'
                       for entry in session_entries)
            assert any(entry.get('type') == 'message' and entry.get('message', {}).get('role') == 'assistant'
                       and entry['message'].get('provider') == 'velune-gateway'
                       and entry['message'].get('model') == physical_ids['first'] for entry in session_entries)
            first_request = request_for_model('external-first')
            if args.protocol == 'chatCompletionsV1':
                assert first_request['max_completion_tokens'] == 64
            elif args.subscription_capability:
                assert all(key not in first_request for key in (
                    'max_output_tokens', 'temperature', 'prompt_cache_retention',
                    'prompt_cache_options', 'prompt_cache_key'))
            else:
                assert requests[0]['max_output_tokens'] == 64
            session_id = first['conversation']['id']
            assert session_id.startswith('fixture-pi:/')
            listed = view(application.list())['conversations']
            assert session_id in [item['id'] for item in listed], (session_id, listed, list((root / 'sessions').rglob('*')))
            first_listing = next(item for item in listed if item['id'] == session_id)
            assert first_listing['cwd'] == str((root / 'work-a').resolve()), first_listing
            application.select_model('fixture-pi', 'second')
            restored_before_send = view(application.open_conversation('fixture-pi', session_id))['snapshot']
            assert restored_before_send['modelId'] == 'second', restored_before_send
            time.sleep(0.2)
            if args.subscription_capability:
                selection = root / 'agent/velune-selection.json'
                value = json.loads(selection.read_text())
                value['subscriptionCapability'] = True
                selection.write_text(json.dumps(value))
            expected_request_count = len(requests) + 1
            second_snapshot = view(application.send('fixture-pi', 'Return another fixture response.'))['snapshot']
            assert second_snapshot['runState'] == 'running', second_snapshot
            wait_for_requests(expected_request_count)
            second_snapshot = snapshot_until_idle()
            assert 'VELUNE_UNIFFI_OK' in json.dumps(second_snapshot), second_snapshot
            session_entries = [json.loads(line) for line in Path(session_id.split(':', 1)[1]).read_text().splitlines()]
            assert any(entry.get('type') == 'message' and entry.get('message', {}).get('role') == 'assistant'
                       and entry['message'].get('provider') == 'velune-gateway'
                       and entry['message'].get('model') == physical_ids['second'] for entry in session_entries)
            second_request = request_for_model('external-second')
            if args.protocol == 'chatCompletionsV1':
                assert second_request['max_completion_tokens'] == 80
            elif args.subscription_capability:
                assert all(key not in second_request for key in (
                    'max_output_tokens', 'temperature', 'prompt_cache_retention',
                    'prompt_cache_options', 'prompt_cache_key'))
            else:
                assert second_request['max_output_tokens'] == 80
            new_session = view(application.create_conversation('fixture-pi', str(root / 'work-b')))['snapshot']
            assert new_session['modelId'] == 'first'
            second_id = new_session['conversation']['id']
            assert new_session['conversation']['cwd'] == str((root / 'work-b').resolve()), new_session
            snapshot_until_ready()
            expected_request_count = len(requests) + 1
            second_started = view(application.send('fixture-pi', 'Create the second project session.'))['snapshot']
            assert second_started['runState'] == 'running', second_started
            wait_for_requests(expected_request_count)
            second_snapshot = snapshot_until_idle()
            assert 'VELUNE_UNIFFI_OK' in json.dumps(second_snapshot), second_snapshot
            if args.protocol == 'chatCompletionsV1':
                snapshot_until_ready()
                tool_snapshot = view(application.send('fixture-pi', 'Verify tool cwd.'))['snapshot']
                assert tool_snapshot['runState'] == 'running', tool_snapshot
                tool_snapshot = snapshot_until_idle()
                assert 'VELUNE_UNIFFI_OK' in json.dumps(tool_snapshot), tool_snapshot
                wait_for_tool_result(str((root / 'work-b').resolve()))
            listed = view(application.list())['conversations']
            deadline = time.monotonic() + 5
            while time.monotonic() < deadline and not any(item['id'] == second_id for item in listed):
                time.sleep(0.1)
                listed = view(application.list())['conversations']
            selected_cwds = {item['cwd'] for item in listed if item['id'] in (session_id, second_id)}
            assert selected_cwds == {str((root / 'work-a').resolve()), str((root / 'work-b').resolve())}, (selected_cwds, listed, session_id, second_id)
            restored = view(application.open_conversation('fixture-pi', session_id))['snapshot']
            assert restored['modelId'] == 'second', restored
            assert restored['conversation']['cwd'] == str((root / 'work-a').resolve()), restored
            if args.protocol == 'chatCompletionsV1':
                expected_request_count = len(requests) + 1
                restored_tool = view(application.send('fixture-pi', 'Verify tool cwd.'))['snapshot']
                assert restored_tool['runState'] == 'running', restored_tool
                wait_for_requests(expected_request_count)
                restored_tool = snapshot_until_idle()
                assert 'VELUNE_UNIFFI_OK' in json.dumps(restored_tool), restored_tool
                assert tool_result_requests and str((root / 'work-a').resolve()) in json.dumps(tool_result_requests[-1]), tool_result_requests
            shutil.rmtree(root / 'work-b')
            expect_error(lambda: application.open_conversation('fixture-pi', second_id))
            still_a = view(application.snapshot('fixture-pi'))['snapshot']
            assert still_a['conversation']['cwd'] == str((root / 'work-a').resolve()), still_a
            view(application.send('fixture-pi', 'Wait for cancellation.'))
            assert waiting.wait(10), 'third request did not reach local upstream'
            expect_error(lambda: application.create_conversation('fixture-pi', str(root / 'work-b')))
            expect_error(lambda: application.open_conversation('fixture-pi', session_id))
            expect_error(application.shutdown)
            assert view(application.snapshot('fixture-pi'))['snapshot']['runState'] != 'idle'
            application.cancel('fixture-pi')
            release.set()
            snapshot_until_idle()
            application.shutdown()
            expect_error(application.list)
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
            application = bindings.VeluneApplication.open(options)
            assert view(application.list())['runtimeInstances'][0]['modelId'] == 'first'
            gateway['providers'][0]['models'][1]['externalModelId'] = 'external-second-rebound'
            application.upsert_gateway(gateway_record(gateway))
            application.connect_runtime('fixture-pi')
            catalog = json.loads((root / 'agent/models.json').read_text())
            rebound = next(item['id'] for item in catalog['providers']['velune-gateway']['models']
                           if item['logicalModelId'] == 'second')
            assert rebound != physical_ids['second']
            assert view(application.open_conversation('fixture-pi', session_id))['snapshot']['modelId'] == 'second'
            if args.subscription_capability:
                selection = root / 'agent/velune-selection.json'
                value = json.loads(selection.read_text()); value['subscriptionCapability'] = True
                selection.write_text(json.dumps(value))
            view(application.send('fixture-pi', 'Return a response after changing the route binding.'))
            snapshot_until_idle()
            assert requests[-1]['model'] == 'external-second-rebound'
            wire = json.dumps(requests[-1])
            assert 'SYNTHETIC_OLD_BINDING_SIGNATURE' not in wire
            assert 'SYNTHETIC_TOOL_RESULT' in wire
            if args.protocol == 'responsesV1':
                items = requests[-1]['input']
                call = next(item for item in items if item.get('type') == 'function_call')
                result = next(item for item in items if item.get('type') == 'function_call_output')
                assert call['call_id'] == result['call_id']
            else:
                items = requests[-1]['messages']
                call = next(item['tool_calls'][0] for item in items if item.get('tool_calls'))
                result = next(item for item in items if item.get('role') == 'tool')
                assert call['id'] == result['tool_call_id']

            assert not (root / 'home/ipc.sock').exists()
            assert not (root / 'home/core.sqlite').exists()
            print('PASS: UniFFI streaming, auto selection/restore, binding change clears signatures and preserves tools, busy close, cancel, config reopen')
        finally:
            release.set()
            if application is not None:
                application.shutdown()
            os.environ.clear()
            os.environ.update(original_environment)
            server.shutdown()
            server.server_close()


if __name__ == '__main__':
    main()
