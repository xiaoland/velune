#!/usr/bin/env python3
"""Manual acceptance for choosing a turn's permission mode.

Real Codex 0.159.3 and Claude Code 2.1.286 run in managed HOMEs inside a
temporary VELUNE_HOME, through Velune's gateway, against a loopback upstream
that answers every turn with text. Checks the modes each runtime type
declares (none for a generic ACP type), that an undeclared mode is refused
before anything is sent, and what each harness then runs with: the Codex
sandbox and approval policy it tells the model, and the mode Claude Code
reports in its requests. A mode is session state: a turn without one keeps
the conversation's current mode (Velune sends it again, since Codex resumes
a thread in its configured policy), and choosing another changes it. No real
model, credential, HOME or session is read; this is not an automated test.
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

REPLY = 'SYNTHETIC_REPLY'


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    for name in ('library', 'bindings', 'bundle', 'node', 'codex', 'claude'):
        parser.add_argument('--' + name, type=pathlib.Path, required=True)
    args = parser.parse_args()
    captures = []

    class Upstream(BaseHTTPRequestHandler):
        protocol_version = 'HTTP/1.1'

        def log_message(self, *_):
            pass

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
                payload = b'{"input_tokens":1}'
                self.send_response(200)
                self.send_header('Content-Type', 'application/json')
                self.send_header('Content-Length', str(len(payload)))
                self.end_headers()
                self.wfile.write(payload)
                return
            captures.append(body)
            if path.endswith('/responses'):
                # Harnesses keep the model's message id, so each reply needs its own.
                item = {'id': 'item-%d' % len(captures), 'type': 'message', 'role': 'assistant', 'status': 'completed',
                        'content': [{'type': 'output_text', 'text': REPLY, 'annotations': []}]}
                response = {'id': 'response', 'object': 'response', 'status': 'completed', 'output': [item],
                            'usage': {'input_tokens': 1, 'output_tokens': 1, 'total_tokens': 2}}
                return self.events([
                    {'type': 'response.created', 'response': {**response, 'status': 'in_progress', 'output': []}},
                    {'type': 'response.output_item.added', 'output_index': 0, 'item': item},
                    {'type': 'response.output_item.done', 'output_index': 0, 'item': item},
                    {'type': 'response.completed', 'response': response}])
            # Claude Code's auto-mode classifier asks whether to block a call.
            text = '<block>no</block>' if 'security monitor' in json.dumps(body.get('system')) else REPLY
            message = {'id': 'msg_synthetic_%d' % len(captures), 'type': 'message', 'role': 'assistant', 'model': body.get('model', 'synthetic'),
                       'content': [], 'stop_reason': None, 'stop_sequence': None, 'usage': {'input_tokens': 1, 'output_tokens': 1}}
            if not body.get('stream'):
                payload = json.dumps({**message, 'content': [{'type': 'text', 'text': text}], 'stop_reason': 'end_turn'}).encode()
                self.send_response(200)
                self.send_header('Content-Type', 'application/json')
                self.send_header('Content-Length', str(len(payload)))
                self.end_headers()
                self.wfile.write(payload)
                return
            self.events([
                {'type': 'message_start', 'message': message},
                {'type': 'content_block_start', 'index': 0, 'content_block': {'type': 'text', 'text': ''}},
                {'type': 'content_block_delta', 'index': 0, 'delta': {'type': 'text_delta', 'text': text}},
                {'type': 'content_block_stop', 'index': 0},
                {'type': 'message_delta', 'delta': {'stop_reason': 'end_turn', 'stop_sequence': None}, 'usage': {'output_tokens': 1}},
                {'type': 'message_stop'}])

    server = ThreadingHTTPServer(('127.0.0.1', 0), Upstream)
    threading.Thread(target=server.serve_forever, daemon=True).start()
    endpoint = 'http://127.0.0.1:%d/v1' % server.server_port
    spec = importlib.util.spec_from_file_location('velune_permission_modes', args.bindings / 'velune_bindings.py')
    b = importlib.util.module_from_spec(spec)
    sys.modules[spec.name] = b
    spec.loader.exec_module(b)

    with tempfile.TemporaryDirectory(prefix='velune-permission-modes-') as directory:
        root = pathlib.Path(directory).resolve()
        (root / 'project').mkdir()
        app = b.VeluneApplication.open(b.BindingOptions(home_directory=str(root / 'velune'),
                                                         resources_directory=str(args.bundle / 'Contents/Resources')))
        try:
            managed = b.BindingRuntimeExecution(transport=b.BindingRuntimeTransport.LOCAL(),
                                                home=b.BindingRuntimeHome.MANAGED(remote_root=None),
                                                remote_resources_directory=None)
            app.upsert_runtime_with_execution(b.BindingRuntimeInstance(
                enabled=True, id='codex', name='Codex', type_id='codex-0.159', gateway_id='default',
                settings={'binary': str(args.codex.resolve()), 'nodeBinary': str(args.node)}), managed)
            app.upsert_runtime_with_execution(b.BindingRuntimeInstance(
                enabled=True, id='claude', name='Claude Code', type_id='claude-code-2.1', gateway_id='default',
                settings={'binary': str(args.claude.resolve())}), managed)

            declared = {t.id: [m.id for m in t.permission_modes] for t in app.list().runtime_types}
            assert declared['codex-0.159'] == ['read-only', 'workspace', 'auto-review', 'full-access'], declared
            assert declared['claude-code-2.1'] == ['manual', 'acceptEdits', 'plan', 'auto', 'dontAsk', 'bypassPermissions'], declared
            assert declared['acp-v1'] == [] and declared['pi-1.0'] == [], declared
            print('PASS runtime types declare their own permission modes; ACP and Pi declare none', flush=True)

            def provider(identifier, protocol):
                app.save_provider('default', b.BindingProviderDraft(
                    id=identifier, name=identifier, protocol=protocol, endpoint=endpoint,
                    models=[b.BindingProviderModel(record_key='', provider_model_id='synthetic-' + identifier, nickname=identifier,
                                                   icon=None, context_window=32768, max_output_tokens=1024,
                                                   reasoning_levels=None, adapter_metadata_json=None)], quota_limits=[], execution_backend=b.BindingExecutionBackend.HTTP(), request_profile=b.BindingRequestProfile.NATIVE, authentication_source=b.BindingProviderAuthenticationSource.API_KEY, quota_source=b.BindingProviderQuotaSource.NONE),
                    b.BindingAuthenticationEdit.SET_API_KEY(value='SYNTHETIC_ONLY'))
                return next(p for p in app.list().gateways[0].providers if p.id == identifier).models[0].record_key

            keys = {'codex': provider('responses', b.BindingGatewayProtocol.RESPONSES_V1),
                    'claude': provider('messages', b.BindingGatewayProtocol.MESSAGES_V1)}

            def turn(runtime, conversation, text, mode):
                before = len(captures)
                conversation = app.send_turn_with_selection(
                    runtime, conversation, runtime, b.BindingModelSelection.MANAGED(model_record_key=keys[runtime]),
                    b.BindingComposedTurn(text=text, effort=None, permission_mode=mode, attachments=[])).snapshot.conversation.id
                deadline = time.monotonic() + 90
                while True:
                    current = app.snapshot(runtime, conversation).snapshot
                    assert current.run_state != b.BindingRunState.FAILED, (runtime, text)
                    said = [block.text for message in current.messages[-1:] for block in message.blocks
                            if isinstance(block, b.BindingMessageBlock.TEXT)]
                    if current.run_state == b.BindingRunState.IDLE and REPLY in said:
                        break
                    assert time.monotonic() < deadline, (runtime, text, current.run_state, [
                        (message.role, [getattr(block, 'text', getattr(block, 'title', type(block).__name__))[:120] for block in message.blocks])
                        for message in current.messages[-3:]])
                    time.sleep(.1)
                sent = [json.dumps(body) for body in captures[before:] if text in json.dumps(body)]
                assert sent, (runtime, text, 'no upstream request')
                return sent[-1], conversation

            def mode(body):
                """The permission mode Claude Code reports in a request."""
                found = []
                def walk(value):
                    if isinstance(value, dict):
                        found.extend(v for k, v in value.items() if k == 'permission_mode')
                        for v in value.values(): walk(v)
                    elif isinstance(value, list):
                        for v in value: walk(v)
                walk(json.loads(body))
                assert len(set(found)) == 1, found
                return found[0]

            def refused(runtime, conversation, mode):
                before = len(captures)
                try:
                    app.send_turn_with_selection(runtime, conversation, runtime, b.BindingModelSelection.MANAGED(model_record_key=keys[runtime]),
                                                 b.BindingComposedTurn(text='NEVER_SENT', effort=None, permission_mode=mode, attachments=[]))
                except b.BindingError:
                    pass
                else:
                    raise AssertionError('%s accepted permission mode %s' % (runtime, mode))
                assert len(captures) == before, 'a refused turn reached the upstream'

            conversation = app.create_conversation('codex', str(root / 'project'), keys['codex']).snapshot.conversation.id
            refused('codex', conversation, 'plan')
            refused('codex', conversation, 'bogus')
            # Codex states its sandbox and approval policy to the model in its permissions instructions.
            body, conversation = turn('codex', conversation, 'CODEX_READ_ONLY', 'read-only')
            assert '`sandbox_mode` is `read-only`' in body, 'read-only sandbox'
            body, conversation = turn('codex', conversation, 'CODEX_FULL_ACCESS', 'full-access')
            tail = body.split('CODEX_READ_ONLY')[-1]
            assert '`sandbox_mode` is `danger-full-access`' in tail and 'Approval policy is currently never' in tail, 'full access'
            body, conversation = turn('codex', conversation, 'CODEX_KEPT', None)
            tail = body.split('CODEX_FULL_ACCESS')[-1]
            assert '`sandbox_mode` is `workspace-write`' not in tail, 'a turn without a mode keeps the thread\'s mode'
            body, conversation = turn('codex', conversation, 'CODEX_WORKSPACE', 'workspace')
            tail = body.split('CODEX_KEPT')[-1]
            assert '`sandbox_mode` is `workspace-write`' in tail and 'Approval policy is currently never' not in tail, 'default preset'
            print('PASS codex: presets reach Codex as sandbox and approval policy; a turn without one keeps the thread\'s mode', flush=True)

            conversation = app.create_conversation('claude', str(root / 'project'), keys['claude']).snapshot.conversation.id
            refused('claude', conversation, 'full-access')
            body, conversation = turn('claude', conversation, 'CLAUDE_PLAN', 'plan')
            assert 'Plan mode is active' in body and mode(body) == 'plan', 'plan mode'
            # Claude Code resumes a session in its last mode, and reminds of
            # plan mode only now and then, so the request's own mode decides.
            body, conversation = turn('claude', conversation, 'CLAUDE_KEPT', None)
            assert mode(body) == 'plan', ('a turn without a mode keeps the session\'s mode', mode(body))
            body, conversation = turn('claude', conversation, 'CLAUDE_AUTO', 'auto')
            assert mode(body) == 'auto', ('a chosen mode replaces it', mode(body))
            print('PASS claude: the chosen mode starts Claude Code in it; a turn without one keeps the session\'s mode', flush=True)
        finally:
            app.shutdown()
            server.shutdown()


if __name__ == '__main__':
    main()
