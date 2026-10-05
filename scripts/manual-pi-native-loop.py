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
        helper = root / 'synthetic-credential'
        helper.write_text(f'#!{sys.executable}\nraise SystemExit(1)\n')
        helper.chmod(0o700)
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
                home_directory=str(root / 'home'), resources_directory=str(resources),
                credential_resolver=str(helper)))
            application.upsert_gateway(bindings.BindingGatewayConfig(
                id='fixture', name='Synthetic gateway', models=[], providers=[], routes=[],
                failover=bindings.BindingFailoverPolicy(mode=bindings.BindingFailoverMode.DISABLED)))
            runtime = bindings.BindingRuntimeInstance(
                id='fixture-runtime', name='Synthetic Pi', type_id='pi', gateway_id='fixture', model_record_key=None,
                settings={'agentDir': str(root / 'source'), 'nodeBinary': str(args.node),
                          'binary': str(resources / 'node_modules/@earendil-works/pi-coding-agent/dist/bundle/cli.js')})
            application.upsert_runtime(runtime)
            source = bindings.BindingProviderImportSource(kind='harness', harness_type_id='pi',
                source_instance_id=runtime.id, provider_id=None, settings={})
            preview = application.preview_provider_import('fixture', source)
            selected = next(item for item in preview.providers if item.source_provider_id == 'fixture-selected')
            candidate = next(item for item in selected.models if item.provider_model_id == 'reasoning')
            plain = next(item for item in selected.models if item.provider_model_id == 'plain')
            if not selected.can_import or not candidate.can_import:
                raise RuntimeError(f'synthetic import still blocked: {selected.issues}, {candidate.issues}')
            assert plain.can_import, 'plain model with native compatibility was blocked'
            imported = application.apply_provider_import('fixture', source, preview.token,
                [bindings.BindingImportSelection(provider_id=selected.id, candidate_keys=[candidate.candidate_key], model_record_mappings={})], False)
            assert imported.imported_provider_ids == [selected.id], 'unselected provider was imported'
            gateway, = imported.gateways
            assert len(gateway.providers) == 1, 'unselected provider was saved'
            authentication_id = gateway.providers[0].authentication_id
            resource = next(item for item in application.list().authentication_bindings if item.id == authentication_id)
            assert resource.method == bindings.BindingAuthenticationMethod.API_KEY
            assert resource.provenance.runtime_type_id == 'pi'
            saved = json.loads((root / 'home/generic-config.json').read_text())
            assert saved['schemaVersion'] == 4 and len(saved['authenticationBindings']) == 1
            assert not any(key.startswith('credential') for key in saved['gateways'][0]['providers'][0])
            binding, = gateway.providers[0].models
            assert binding.provider_model_id == 'reasoning' and binding.model_record_key != 'reasoning'
            assert binding.context_window == candidate.context_window
            assert binding.max_output_tokens == candidate.max_output_tokens
            saved_before_invalid = (root / 'home/generic-config.json').read_bytes()
            invalid_preview = application.preview_provider_import('fixture', source)
            try:
                application.apply_provider_import('fixture', source, invalid_preview.token,
                    [bindings.BindingImportSelection(provider_id=selected.id,
                        candidate_keys=[candidate.candidate_key],
                        model_record_mappings={candidate.candidate_key: 'unknown-record'})], True)
            except bindings.BindingError:
                assert (root / 'home/generic-config.json').read_bytes() == saved_before_invalid
            else:
                raise AssertionError('unknown explicit model mapping accepted')

            gateway.routes = [bindings.BindingRoute(model_record_key=binding.model_record_key, provider_id=selected.id)]
            application.upsert_gateway(gateway)
            runtime.model_record_key = binding.model_record_key
            application.upsert_runtime(runtime)
            application.connect_runtime(runtime.id)
            application.create_conversation(runtime.id, str(root / 'project'))
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
            preview = application.preview_provider_import('fixture', source)
            skipped = application.apply_provider_import('fixture', source, preview.token,
                [bindings.BindingImportSelection(provider_id=selected.id, candidate_keys=[candidate.candidate_key], model_record_mappings={})], False)
            assert not skipped.requires_reconnect, 'unchanged skipped import disconnected runtime'
            assert application.list().active_runtime_instance_id == runtime.id
            preview = application.preview_provider_import('fixture', source)
            changed = application.apply_provider_import('fixture', source, preview.token,
                [bindings.BindingImportSelection(provider_id=selected.id,
                    candidate_keys=[candidate.candidate_key, plain.candidate_key], model_record_mappings={})], True)
            assert changed.requires_reconnect, 'changed active gateway did not report stale connection'
            assert application.list().active_runtime_instance_id is None, 'stale gateway remained active'
            assert all((root / 'source' / name).read_bytes() == content for name, content in source_files.items())
            stale = application.preview_provider_import('fixture', source)
            current_resource = next(item for item in application.list().authentication_bindings
                                    if item.id == authentication_id)
            application.configure_api_key_binding(authentication_id, 'Synthetic replacement', 'synthetic-ref',
                bindings.BindingGatewayProtocol.CHAT_COMPLETIONS_V1, endpoint, False, current_resource.generation)
            saved_after_replacement = (root / 'home/generic-config.json').read_bytes()
            try:
                application.apply_provider_import('fixture', source, stale.token,
                    [bindings.BindingImportSelection(provider_id=selected.id,
                        candidate_keys=[candidate.candidate_key], model_record_mappings={})], True)
            except bindings.BindingError:
                assert (root / 'home/generic-config.json').read_bytes() == saved_after_replacement
            else:
                raise AssertionError('old preview overwrote a newly managed authentication resource')
            # Exercise helper lifetime through the application-managed registry,
            # not a gateway-specific source protocol. Only this synthetic helper
            # and its synthetic Node descendant are started or terminated.
            pid_file = root / 'helper-node.pid'
            node_code = 'require("fs").writeFileSync(' + json.dumps(str(pid_file)) + ',String(process.pid));setInterval(()=>{},1000)'
            helper.write_text(f'#!{sys.executable}\nimport subprocess\nsubprocess.run([{str(args.node)!r}, "-e", {node_code!r}])\n')
            managed = application.configure_api_key_binding('hanging-fixture', 'Synthetic helper', 'synthetic-ref',
                bindings.BindingGatewayProtocol.CHAT_COMPLETIONS_V1, endpoint, False, None)
            gateway, = application.list().gateways
            gateway.providers[0].authentication_id = managed.binding.id
            application.upsert_gateway(gateway)
            # The managed catalog contains an environment placeholder, never a
            # stored gateway token. Capture only this fixture's ephemeral token
            # from its own Node launcher; no real process or secret is inspected.
            token_file = root / 'synthetic-gateway-token'
            launcher = root / 'synthetic-node'
            launcher.write_text(f'#!{sys.executable}\nimport os,sys\nfrom pathlib import Path\n'
                f'if "VELUNE_GATEWAY_TOKEN" in os.environ:\n'
                f' p=Path({str(token_file)!r});p.write_text(os.environ["VELUNE_GATEWAY_TOKEN"]);p.chmod(0o600)\n'
                f'os.execv({str(args.node)!r}, [{str(args.node)!r}, *sys.argv[1:]])\n')
            launcher.chmod(0o700)
            runtime.settings['nodeBinary'] = str(launcher)
            application.upsert_runtime(runtime)
            application.connect_runtime(runtime.id)
            application.create_conversation(runtime.id, str(root / 'project'))
            deadline = time.monotonic() + 3
            while not token_file.exists() and time.monotonic() < deadline:
                time.sleep(.02)
            assert token_file.exists(), 'synthetic runtime did not receive local gateway credentials'
            projection, = (root / 'home/runtime-projections').iterdir()
            injected = json.loads((projection / 'models.json').read_text())['providers']['velune-gateway']
            port = int(injected['baseUrl'].split(':')[2].split('/')[0])

            def pending_request():
                connection = http.client.HTTPConnection('127.0.0.1', port, timeout=35)
                connection.request('POST', '/v1/chat/completions', json.dumps({'model': 'velune/model/' + binding.model_record_key,
                    'messages': [{'role': 'user', 'content': 'synthetic'}]}),
                    {'Authorization': 'Bearer ' + token_file.read_text(), 'Content-Type': 'application/json'})
                deadline = time.monotonic() + 3
                while not pid_file.exists() and time.monotonic() < deadline:
                    time.sleep(.02)
                assert pid_file.exists(), 'managed helper did not start'
                return connection, int(pid_file.read_text())

            def descendant_gone(pid):
                deadline = time.monotonic() + 3
                while time.monotonic() < deadline:
                    try:
                        os.kill(pid, 0)
                    except ProcessLookupError:
                        pid_file.unlink()
                        return
                    time.sleep(.02)
                raise AssertionError('managed helper descendant survived cancellation')

            connection, pid = pending_request()
            connection.close()
            descendant_gone(pid)
            connection, pid = pending_request()
            started = time.monotonic()
            response = connection.getresponse()
            assert response.status == 503
            response.read()
            assert 25 < time.monotonic() - started < 33
            connection.close()
            descendant_gone(pid)
            connection, pid = pending_request()
            started = time.monotonic()
            application.shutdown()
            application = None
            connection.close()
            descendant_gone(pid)
            assert time.monotonic() - started < 3, 'managed helper blocked application shutdown'
            assert all((root / 'source' / name).read_bytes() == content for name, content in source_files.items())
            print(json.dumps({'acceptance': 'PASSED', 'bundle': {'sourceCommit': manifest['source_commit'],
                'dirty': manifest['dirty'], 'uiVersion': manifest['ui_version']},
                'normalPath': 'configured runtime → import → route → connect → create → send → tool → continuation → next turn',
                'upstreamRequests': len(captures), 'onlySelectedProviderSaved': True, 'centralAuthenticationResource': True,
                'actualPiSourceCredentialAdapter': True,
                'authenticationReplacementInvalidatesImportPreview': True,
                'managedHelperDisconnectDeadlineShutdownCleanup': True,
                'nativeReasoningHistoryPreserved': True, 'sourceFilesUnchanged': True,
                'unchangedImportKeepsConnection': True, 'changedImportDisconnectsStaleGateway': True,
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
