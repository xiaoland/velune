#!/usr/bin/env python3
"""Manually accept configured Pi import and native gateway with a synthetic local loop.

Explicit invocation only; not a test suite or CI entry point. All configuration,
credentials, conversations, tools and upstream responses live in a temporary home.
"""
import argparse
import importlib.util
import http.client
import json
import os
from pathlib import Path
import shutil
import sys
import tempfile
import threading
import time
from http.server import BaseHTTPRequestHandler, ThreadingHTTPServer


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('--bundle', required=True, type=Path)
    parser.add_argument('--bindings', required=True, type=Path)
    parser.add_argument('--node', required=True, type=Path)
    args = parser.parse_args()
    for path in (args.bundle, args.bindings, args.node):
        if not path.is_absolute() or not path.exists():
            parser.error('all paths must be absolute and exist')
    resources = args.bundle / 'Contents/Resources'
    manifest = json.loads((resources / 'build-manifest.json').read_text())
    captures = []
    auth_headers = []
    request_paths = []
    upstream_errors = []
    environment = dict(os.environ)
    with tempfile.TemporaryDirectory(prefix='velune-native-loop-') as directory:
        root = Path(directory)
        for name in ('home', 'source', 'bindings', 'project'):
            (root / name).mkdir()
        tool_file = root / 'project' / 'synthetic.txt'
        tool_file.write_text('SYNTHETIC_TOOL_RESULT\n')

        class Upstream(BaseHTTPRequestHandler):
            def log_message(self, *_):
                pass

            def do_POST(self):
                try:
                    body = json.loads(self.rfile.read(int(self.headers['Content-Length'])))
                    captures.append(body)
                    auth_headers.append(self.headers.get("Authorization"))
                    request_paths.append(self.path)
                    first = len(captures) == 1
                    delta = {'reasoning_content': 'SYNTHETIC_PLAN' if first else 'SYNTHETIC_REVIEW'}
                    if first:
                        delta['tool_calls'] = [{'index': 0, 'id': 'call_fixture', 'type': 'function',
                            'function': {'name': 'read', 'arguments': json.dumps({'path': str(tool_file)})}}]
                    else:
                        delta['content'] = 'SYNTHETIC_ANSWER'
                    self.send_response(200)
                    self.send_header('Content-Type', 'text/event-stream')
                    self.send_header('X-Request-ID', 'synthetic-request')
                    self.end_headers()
                    chunks = [
                        {'id': 'synthetic', 'object': 'chat.completion.chunk', 'created': 1,
                         'model': body['model'], 'choices': [{'index': 0, 'delta': delta, 'finish_reason': None}]},
                        {'id': 'synthetic', 'object': 'chat.completion.chunk', 'created': 1,
                         'model': body['model'], 'choices': [{'index': 0, 'delta': {},
                            'finish_reason': 'tool_calls' if first else 'stop'}],
                         'usage': {'prompt_tokens': 1, 'completion_tokens': 2, 'total_tokens': 3}},
                    ]
                    for chunk in chunks:
                        self.wfile.write(('data: ' + json.dumps(chunk) + '\n\n').encode())
                    self.wfile.write(b'data: [DONE]\n\n')
                    self.wfile.flush()
                except Exception as error:
                    upstream_errors.append(type(error).__name__)

        server = ThreadingHTTPServer(('127.0.0.1', 0), Upstream)
        thread = threading.Thread(target=server.serve_forever, daemon=True)
        thread.start()
        endpoint = f'http://127.0.0.1:{server.server_port}/v1'
        compat = {'thinkingFormat': 'deepseek', 'requiresReasoningContentOnAssistantMessages': True,
                  'maxTokensField': 'max_tokens', 'supportsStore': False}
        def provider(identity):
            return {'baseUrl': endpoint, 'api': 'openai-completions', 'apiKey': 'synthetic-only',
                    'models': [{'id': identity, 'name': identity, 'reasoning': True, 'input': ['text'],
                                'contextWindow': 16384, 'maxTokens': 128, 'compat': compat}]}
        selected_source = provider('reasoning')
        selected_source['models'].append({'id': 'plain', 'name': 'plain', 'reasoning': False,
            'input': ['text'], 'contextWindow': 16384, 'maxTokens': 128, 'compat': compat})
        source_files = {'models.json': json.dumps({'providers': {
            'fixture-selected': selected_source, 'fixture-unselected': provider('other')}}).encode(),
            'auth.json': b'{}', 'settings.json': b'{"synthetic":true}'}
        for name, content in source_files.items():
            (root / 'source' / name).write_bytes(content)
        library = args.bundle / 'Contents/Frameworks/libvelune_bindings.dylib'
        shutil.copy(args.bindings / 'velune_bindings.py', root / 'bindings')
        (root / 'bindings' / library.name).symlink_to(library)
        os.environ.clear()
        os.environ.update(HOME=str(root / 'home'), PATH=f'{args.node.parent}:/usr/bin:/bin',
                          NO_PROXY='127.0.0.1,localhost')
        application = None
        try:
            spec = importlib.util.spec_from_file_location('velune_bindings', root / 'bindings/velune_bindings.py')
            bindings = importlib.util.module_from_spec(spec)
            sys.modules[spec.name] = bindings
            spec.loader.exec_module(bindings)
            application = bindings.VeluneApplication.open(bindings.BindingOptions(
                home_directory=str(root / 'home'), resources_directory=str(resources)))
            runtime = bindings.BindingRuntimeInstance(
                id='fixture-runtime', name='Synthetic Pi', type_id='pi-1.0.2', gateway_id='default',
                settings={'agentDir': str(root / 'source'), 'nodeBinary': str(args.node),
                          'binary': str(resources / 'node_modules/@earendil-works/pi-coding-agent/dist/bundle/cli.js')})
            application.upsert_runtime(runtime)
            source = bindings.BindingProviderImportSource(kind='harness', harness_type_id='pi',
                source_instance_id=runtime.id, provider_id=None, settings={})
            preview = application.preview_provider_import('default', source)
            selected = next(item for item in preview.providers if item.source_provider_id == 'fixture-selected')
            candidate = next(item for item in selected.models if item.provider_model_id == 'reasoning')
            plain = next(item for item in selected.models if item.provider_model_id == 'plain')
            if not selected.can_import or not candidate.can_import:
                raise RuntimeError(f'synthetic import still blocked: {selected.issues}, {candidate.issues}')
            assert plain.can_import, 'plain model with native compatibility was blocked'
            saved_before_import = (root / 'home/generic-config.json').read_bytes()
            try:
                application.apply_provider_import('default', source, preview.token,
                    [bindings.BindingImportSelection(provider_id=selected.id,
                        candidate_keys=[candidate.candidate_key]),
                     bindings.BindingImportSelection(provider_id=selected.source_provider_id,
                        candidate_keys=[plain.candidate_key])], False)
            except bindings.BindingError:
                assert (root / 'home/generic-config.json').read_bytes() == saved_before_import
            else:
                raise AssertionError('duplicate provider selections were silently accepted')
            models_path = root / 'source/models.json'
            changed_source = json.loads(source_files['models.json'])
            changed_source['providers']['fixture-selected']['models'][0]['contextWindow'] = 32768
            models_path.write_text(json.dumps(changed_source))
            try:
                application.apply_provider_import('default', source, preview.token,
                    [bindings.BindingImportSelection(provider_id=selected.id,
                        candidate_keys=[candidate.candidate_key])], False)
            except bindings.BindingError:
                assert (root / 'home/generic-config.json').read_bytes() == saved_before_import
            else:
                raise AssertionError('changed source accepted an old preview')
            finally:
                models_path.write_bytes(source_files['models.json'])
            imported = application.apply_provider_import('default', source, preview.token,
                [bindings.BindingImportSelection(provider_id=selected.id, candidate_keys=[candidate.candidate_key])], False)
            assert imported.imported_provider_ids == [selected.id], 'unselected provider was imported'
            gateway, = imported.gateways
            assert len(gateway.providers) == 1, 'unselected provider was saved'
            assert gateway.providers[0].authentication.method == bindings.BindingAuthenticationMethod.API_KEY
            assert application.read_provider_api_key('default', selected.id) == 'synthetic-only'
            saved = json.loads((root / 'home/generic-config.json').read_text())
            assert saved['schemaVersion'] == 7 and 'authenticationBindings' not in saved
            binding, = gateway.providers[0].models
            assert binding.provider_model_id == 'reasoning' and binding.record_key != 'reasoning'
            assert binding.context_window == candidate.context_window
            assert binding.max_output_tokens == candidate.max_output_tokens
            application.upsert_runtime(runtime)
            application.select_runtime(runtime.id)
            application.create_conversation(runtime.id, str(root / 'project'), binding.record_key)
            for turn in ('Read the synthetic local file.', 'Continue the synthetic conversation.'):
                before = len(captures)
                application.send(runtime.id, turn)
                deadline = time.monotonic() + 30
                while time.monotonic() < deadline:
                    snapshot = application.snapshot(runtime.id).snapshot
                    if snapshot and snapshot.run_state == bindings.BindingRunState.FAILED:
                        raise RuntimeError('synthetic Pi run failed')
                    if snapshot and snapshot.actions.can_send and len(captures) > before:
                        break
                    time.sleep(0.05)
                else:
                    raise RuntimeError('synthetic Pi did not settle within 30 seconds')
            assert len(captures) == 3, f'expected tool continuation plus next turn, got {len(captures)}'
            assert not upstream_errors, upstream_errors
            for request in captures:
                assert request['model'] == 'reasoning', 'logical model was not routed'
                assert 'max_completion_tokens' not in request, 'SDK compat maxTokensField changed'
                assert request.get('thinking') == {'type': 'disabled'}, 'explicit off selection changed'
            continuation = captures[1]
            assistant = next(item for item in continuation['messages'] if item.get('tool_calls'))
            assert assistant.get('reasoning_content') == 'SYNTHETIC_PLAN', 'reasoning history was lost'
            assert any(item.get('role') == 'tool' and item.get('tool_call_id') == 'call_fixture'
                       and 'SYNTHETIC_TOOL_RESULT' in str(item.get('content')) for item in continuation['messages'])
            assert all('reasoning_content' in item for item in captures[2]['messages'] if item.get('role') == 'assistant')
            assistant_text = [block.text for message in snapshot.messages if message.role == 'assistant'
                              for block in message.blocks if isinstance(block, bindings.BindingMessageBlock.TEXT)]
            assert any('SYNTHETIC_ANSWER' in text for text in assistant_text), 'final projection missing'
            preview = application.preview_provider_import('default', source)
            skipped = application.apply_provider_import('default', source, preview.token,
                [bindings.BindingImportSelection(provider_id=selected.id, candidate_keys=[candidate.candidate_key])], False)
            assert not skipped.execution_invalidated, 'unchanged skipped import invalidated execution'
            assert application.list().selected_runtime_instance_id == runtime.id
            preview = application.preview_provider_import('default', source)
            changed = application.apply_provider_import('default', source, preview.token,
                [bindings.BindingImportSelection(provider_id=selected.id,
                    candidate_keys=[candidate.candidate_key, plain.candidate_key])], True)
            assert changed.execution_invalidated, 'changed active gateway did not report stale execution'
            assert application.list().selected_runtime_instance_id == runtime.id, 'gateway edit lost browse selection'
            assert all((root / 'source' / name).read_bytes() == content for name, content in source_files.items())
            stale = application.preview_provider_import('default', source)
            current = application.list().gateways[0].providers[0]
            draft = bindings.BindingProviderDraft(id=current.id, name=current.name, protocol=current.protocol,
                endpoint=current.endpoint, models=current.models)
            application.save_provider('default', draft,
                bindings.BindingAuthenticationEdit.SET_API_KEY(value='synthetic-replacement'))
            saved_after_replacement = (root / 'home/generic-config.json').read_bytes()
            try:
                application.apply_provider_import('default', source, stale.token,
                    [bindings.BindingImportSelection(provider_id=selected.id,
                        candidate_keys=[candidate.candidate_key])], True)
            except bindings.BindingError:
                assert (root / 'home/generic-config.json').read_bytes() == saved_after_replacement
            else:
                raise AssertionError('old preview overwrote edited provider authentication')
            assert application.read_provider_api_key('default', selected.id) == 'synthetic-replacement'
            def send_after_edit():
                application.select_runtime(runtime.id)
                application.create_conversation(runtime.id, str(root / 'project'), binding.record_key)
                before = len(captures)
                application.send(runtime.id, 'Confirm the synthetic edited configuration.')
                deadline = time.monotonic() + 30
                while time.monotonic() < deadline:
                    current_snapshot = application.snapshot(runtime.id).snapshot
                    if current_snapshot and current_snapshot.run_state == bindings.BindingRunState.FAILED:
                        raise RuntimeError('edited synthetic provider run failed')
                    if current_snapshot and current_snapshot.actions.can_send and len(captures) > before:
                        return
                    time.sleep(.05)
                raise RuntimeError('edited configuration did not settle')
            send_after_edit()
            assert auth_headers[-1] == 'Bearer synthetic-replacement'
            current = application.list().gateways[0].providers[0]
            edited_model = next(item for item in current.models if item.record_key == binding.record_key)
            edited_model.provider_model_id = 'edited-api-model-id'
            edited_model.adapter_metadata_json = None
            edited_model.reasoning_levels = []
            changed_endpoint = endpoint.replace('/v1', '/changed/v1')
            draft = bindings.BindingProviderDraft(id=current.id, name=current.name,
                protocol=current.protocol, endpoint=changed_endpoint, models=current.models)
            application.save_provider('default', draft, bindings.BindingAuthenticationEdit.KEEP())
            send_after_edit()
            assert captures[-1]['model'] == 'edited-api-model-id'
            assert request_paths[-1] == '/changed/v1/chat/completions'
            assert auth_headers[-1] == 'Bearer synthetic-replacement'
            assert not hasattr(application.list().runtime_instances[0], 'model_record_key')

            application.shutdown()
            application = None
            assert all((root / 'source' / name).read_bytes() == content for name, content in source_files.items())
            print(json.dumps({'acceptance': 'PASSED', 'bundle': {'sourceCommit': manifest['source_commit'],
                'dirty': manifest['dirty'], 'uiVersion': manifest['ui_version']},
                'normalPath': 'configured runtime → import → choose conversation model → automatic prepare → create → send → tool → continuation → next turn',
                'upstreamRequests': len(captures), 'onlySelectedProviderSaved': True, 'providerOwnedAuthentication': True,
                'editedKeyIdEndpointUsedUpstream': True,
                'authenticationReplacementInvalidatesImportPreview': True,
                'changedSourceInvalidatesImportPreview': True,
                'duplicateProviderSelectionsRejectedBeforeMutation': True,
                'nativeReasoningHistoryPreserved': True, 'sourceFilesUnchanged': True,
                'unchangedImportKeepsExecution': True, 'changedImportInvalidatesExecutionAndKeepsHistory': True,
                'realServicesCalled': False}, ensure_ascii=False, indent=2))
        finally:
            try:
                if application:
                    application.shutdown()
            finally:
                os.environ.clear()
                os.environ.update(environment)
                server.shutdown()
                server.server_close()
                thread.join(timeout=2)


if __name__ == '__main__':
    main()
