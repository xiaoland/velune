#!/usr/bin/env python3
"""Manually verify native input-free recovery through the application API.

Uses a real Codex or Claude Code CLI in a temporary managed HOME and a loopback synthetic
Responses upstream. No existing configuration, credentials or sessions are
read. Run explicitly with matching generated Python bindings and library.
"""
import argparse
import importlib.util
import json
from pathlib import Path
import shutil
import subprocess
import sys
import tempfile
import threading
import time
from http.server import BaseHTTPRequestHandler, ThreadingHTTPServer


def check_claude_history(root):
    """Native interruption markers must not hide a real user with the same text."""
    session = '12345678-1234-4234-8234-123456789abc'
    directory = root / 'history-fixture'
    directory.mkdir()
    path = directory / (session + '.jsonl')
    content = [{'type': 'text', 'text': '[Request interrupted by user]'}]
    original = {'uuid': 'original', 'parentUuid': None, 'type': 'user', 'promptId': 'prompt',
                'promptSource': 'sdk', 'turnPosition': {'promptIndex': 0, 'turnIndex': 1},
                'cwd': str(root), 'message': {'role': 'user', 'content': content}}
    marker = {'uuid': 'interruption', 'parentUuid': 'original', 'type': 'user', 'promptId': 'prompt',
              'message': {'role': 'user', 'content': content}}
    helper = Path(__file__).resolve().parents[1] / 'packages/agent-runtime/resources/claude_history.py'

    def read(records):
        path.write_text(''.join(json.dumps(record) + '\n' for record in records))
        request = {'roots': [str(directory)], 'runtimeId': 'fixture', 'operation': 'read', 'nativeId': session}
        return json.loads(subprocess.check_output([sys.executable, str(helper)], input=json.dumps(request).encode()))['history']

    stopped = read([original, marker])
    assert stopped['canResume'] and len(stopped['messages']) == 1
    typed = read([original, {**marker, 'promptId': 'next', 'promptSource': 'sdk', 'turnPosition': {'turnIndex': 2}}])
    assert not typed['canResume'] and len(typed['messages']) == 2
    finished = read([original, marker, {'uuid': 'answer', 'parentUuid': 'interruption', 'type': 'assistant',
        'message': {'role': 'assistant', 'model': 'synthetic', 'content': [{'type': 'text', 'text': 'done'}]}}])
    assert not finished['canResume'] and len(finished['messages']) == 2


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    for name in ('library', 'bindings', 'resources', 'node', 'binary'):
        parser.add_argument('--' + name, type=Path, required=True)
    parser.add_argument('--runtime', choices=('codex', 'claude'), default='codex')
    args = parser.parse_args()
    runtime = args.runtime
    mode = 'read-only' if runtime == 'codex' else 'plan'
    captures = []
    held = threading.Event()
    release = threading.Event()

    class Upstream(BaseHTTPRequestHandler):
        def log_message(self, *_):
            pass

        def do_POST(self):
            if self.path.endswith('/count_tokens'):
                data = b'{"input_tokens":1}'
                self.rfile.read(int(self.headers['Content-Length']))
                self.send_response(200)
                self.send_header('Content-Type', 'application/json')
                self.send_header('Content-Length', str(len(data)))
                self.end_headers()
                self.wfile.write(data)
                return
            assert self.path in ('/v1/responses', '/v1/messages'), self.path
            body = json.loads(self.rfile.read(int(self.headers['Content-Length'])))
            captures.append(body)
            number = len(captures)
            if number == 7:
                data = b'{"error":{"type":"invalid_request_error","message":"SYNTHETIC_REJECTION"}}'
                self.send_response(400)
                self.send_header('Content-Type', 'application/json')
                self.send_header('Content-Length', str(len(data)))
                self.end_headers()
                self.wfile.write(data)
                return
            if number in (2, 5):
                held.set()
                release.wait(45)
            if number == 1:
                item = {'id': 'fc_once', 'type': 'function_call', 'name': 'exec_command',
                        'call_id': 'call_once', 'status': 'completed',
                        'arguments': json.dumps({'cmd': 'echo SYNTHETIC_TOOL_ONCE', 'max_output_tokens': 100})}
            else:
                item = {'id': 'reply-%d' % number, 'type': 'message', 'role': 'assistant', 'status': 'completed',
                        'content': [{'type': 'output_text', 'text': 'SYNTHETIC_REPLY_%d' % number, 'annotations': []}]}
            response = {'id': 'response-%d' % number, 'object': 'response', 'status': 'completed', 'output': [item],
                        'usage': {'input_tokens': number + 1, 'output_tokens': 1, 'total_tokens': number + 2,
                                  'input_tokens_details': {'cached_tokens': 0}, 'output_tokens_details': {'reasoning_tokens': 0}}}
            events = [
                {'type': 'response.created', 'response': {**response, 'status': 'in_progress', 'output': []}},
                {'type': 'response.output_item.added', 'output_index': 0, 'item': item},
                {'type': 'response.output_item.done', 'output_index': 0, 'item': item},
                {'type': 'response.completed', 'response': response}]
            if self.path == '/v1/messages':
                content = [{'type': 'tool_use', 'id': 'call_once', 'name': 'Read', 'input': {'file_path': str(root / 'project/fixture.txt')}}] if number == 1 else [{'type': 'text', 'text': 'SYNTHETIC_REPLY_%d' % number}]
                message = {'id': 'message-%d' % number, 'type': 'message', 'role': 'assistant', 'model': body['model'],
                    'content': [], 'stop_reason': None, 'stop_sequence': None,
                    'usage': {'input_tokens': number + 1, 'output_tokens': 0, 'cache_read_input_tokens': 0, 'cache_creation_input_tokens': 0}}
                start = {**content[0], 'input': {}} if number == 1 else {'type': 'text', 'text': ''}
                delta = {'type': 'input_json_delta', 'partial_json': json.dumps(content[0]['input'])} if number == 1 else {'type': 'text_delta', 'text': content[0]['text']}
                events = [{'type': 'message_start', 'message': message}, {'type': 'content_block_start', 'index': 0, 'content_block': start},
                    {'type': 'content_block_delta', 'index': 0, 'delta': delta},
                    {'type': 'content_block_stop', 'index': 0},
                    {'type': 'message_delta', 'delta': {'stop_reason': 'tool_use' if number == 1 else 'end_turn', 'stop_sequence': None}, 'usage': {'output_tokens': 1}},
                    {'type': 'message_stop'}]
            data = ''.join('event: %s\ndata: %s\n\n' % (event['type'], json.dumps(event)) for event in events).encode()
            self.send_response(200)
            self.send_header('Content-Type', 'text/event-stream')
            self.send_header('Content-Length', str(len(data)))
            self.end_headers()
            try:
                self.wfile.write(data)
            except (BrokenPipeError, ConnectionResetError):
                pass  # The interrupted request no longer has a reader.

    server = ThreadingHTTPServer(('127.0.0.1', 0), Upstream)
    threading.Thread(target=server.serve_forever, daemon=True).start()
    with tempfile.TemporaryDirectory(prefix='velune-resume-turn-') as directory:
        root = Path(directory)
        check_claude_history(root)
        (root / 'bindings').mkdir()
        (root / 'project').mkdir()
        (root / 'project/fixture.txt').write_text('SYNTHETIC_TOOL_ONCE')
        shutil.copy2(args.library, root / 'bindings' / args.library.name)
        shutil.copy2(args.bindings / 'velune_bindings.py', root / 'bindings/velune_bindings.py')
        spec = importlib.util.spec_from_file_location('velune_resume', root / 'bindings/velune_bindings.py')
        b = importlib.util.module_from_spec(spec)
        sys.modules[spec.name] = b
        spec.loader.exec_module(b)
        options = b.BindingOptions(home_directory=str(root / 'velune'), resources_directory=str(args.resources))
        app = b.VeluneApplication.open(options)
        try:
            app.upsert_runtime_with_execution(b.BindingRuntimeInstance(
                enabled=True, id=runtime, name=runtime, type_id='codex-0.159' if runtime == 'codex' else 'claude-code-2.1', gateway_id='default',
                settings={'binary': str(args.binary.resolve()), **({'nodeBinary': str(args.node)} if runtime == 'codex' else {})}),
                b.BindingRuntimeExecution(transport=b.BindingRuntimeTransport.LOCAL(),
                    home=b.BindingRuntimeHome.MANAGED(remote_root=None), remote_resources_directory=None))
            app.save_provider('default', b.BindingProviderDraft(
                id='synthetic', name='Synthetic', protocol=b.BindingGatewayProtocol.RESPONSES_V1 if runtime == 'codex' else b.BindingGatewayProtocol.MESSAGES_V1,
                endpoint=('http://127.0.0.1:%d' % server.server_port) + ('/v1' if runtime == 'codex' else ''),
                models=[b.BindingProviderModel(record_key='', provider_model_id='synthetic-' + name, nickname=name,
                    icon=None, context_window=32768, max_output_tokens=1024, reasoning_levels=['low', 'high']) for name in ('first', 'resume', 'next')],
                quota_limits=[], execution_backend=b.BindingExecutionBackend.HTTP(), request_profile=b.BindingRequestProfile.NATIVE,
                authentication_source=b.BindingProviderAuthenticationSource.API_KEY, quota_source=b.BindingProviderQuotaSource.NONE),
                b.BindingAuthenticationEdit.SET_API_KEY(value='SYNTHETIC_ONLY'))
            keys = {m.nickname: m.record_key for p in app.list().gateways[0].providers if p.id == 'synthetic' for m in p.models}
            selections = {name: b.BindingModelSelection.MANAGED(model_record_key=key) for name, key in keys.items()}
            conversation = app.create_conversation(runtime, str(root / 'project'), keys['first']).snapshot.conversation.id

            def snapshot():
                return app.snapshot(runtime, conversation).snapshot

            def wait(predicate, label):
                deadline = time.monotonic() + 40
                while time.monotonic() < deadline:
                    current = snapshot()
                    if predicate(current):
                        return current
                    time.sleep(.05)
                raise AssertionError((label, snapshot().run_state, len(captures)))

            def rejected(action):
                before = len(captures)
                try:
                    action()
                except b.BindingError:
                    assert len(captures) == before, 'rejected operation dispatched a request'
                else:
                    raise AssertionError('operation should be rejected')

            def resume(selection=selections['resume'], permission=mode):
                return app.resume_turn(runtime, conversation, selection, 'high', permission)

            def users(current):
                return [message for message in current.messages if message.role == b.BindingMessageRole.USER]

            def first_turn(current):
                return next(turn for turn in current.transcript_turns if turn.user_message_id == users(current)[0].id)

            rejected(resume)  # An unsent conversation is not a stopped context.
            app.send_turn_with_selection(runtime, conversation, runtime, selections['first'],
                b.BindingComposedTurn(text='SYNTHETIC_ORIGINAL', effort='low', permission_mode=mode, attachments=[]))
            wait(lambda _: held.is_set(), 'tool completed and second request held')
            rejected(resume)  # No concurrent resume while the turn runs.
            queued = app.send_queue(runtime, conversation, b.BindingSendQueueOperation.ADD(
                source_runtime_id=runtime, target_runtime_id=runtime, selection=selections['first'],
                turn=b.BindingComposedTurn(text='SYNTHETIC_QUEUED', effort=None, permission_mode=None, attachments=[]))).snapshot
            queue_id = queued.follow_up.queue[0].id
            app.cancel(runtime, conversation)
            stopped = wait(lambda s: s.follow_up.can_resume, 'stopped context is resumable')
            assert len(users(stopped)) == 1
            assert stopped.follow_up.paused == b.BindingQueuePause.TURN_STOPPED
            original_settings = first_turn(stopped).settings
            rejected(lambda: resume(permission='invalid'))
            rejected(lambda: resume(b.BindingModelSelection.MANAGED(model_record_key='missing-model')))
            rejected(lambda: app.send_turn_with_selection(runtime, conversation, runtime, selections['first'],
                b.BindingComposedTurn(text='', effort=None, permission_mode=None, attachments=[])))
            assert snapshot().follow_up.can_resume, 'unaccepted requests must preserve recovery'
            resume()
            resumed = wait(lambda s: s.actions.can_send and not s.follow_up.can_continue, 'resumed turn completed')
            release.set()
            assert len(captures) == 3, [(body.get('model'), [(m.get('role'), str(m.get('content'))[:180]) for m in body.get('messages', [])[-2:]]) for body in captures]
            assert len(users(resumed)) == 1, 'resume added a user message'
            assert first_turn(resumed).settings == original_settings, 'resume changed original requested settings'
            assert first_turn(resumed).usage.input_tokens == 6, first_turn(resumed).usage
            assert resumed.follow_up.paused == b.BindingQueuePause.TURN_STOPPED
            assert len(resumed.follow_up.queue) == 1, 'resume sent a paused queued message'
            if runtime == 'codex':
                outputs = [item for item in captures[2]['input'] if item.get('type') == 'function_call_output' and item.get('call_id') == 'call_once']
            else:
                outputs = [block for message in captures[2]['messages'] for block in message.get('content', []) if isinstance(block, dict) and block.get('type') == 'tool_result' and block.get('tool_use_id') == 'call_once']
            assert len(outputs) == 1, outputs
            assert 'SYNTHETIC_TOOL_ONCE' in json.dumps(outputs), 'resume lost completed tool output'
            rejected(resume)  # A completed turn is not offered recovery.
            app.send_queue(runtime, conversation, b.BindingSendQueueOperation.REMOVE(id=queue_id))
            app.send_turn_with_selection(runtime, conversation, runtime, selections['next'],
                b.BindingComposedTurn(text='SYNTHETIC_NEXT', effort='low', permission_mode=mode, attachments=[]))
            final = wait(lambda s: s.actions.can_send and len(users(s)) == 2, 'next normal message completed')
            turns = [turn for turn in final.transcript_turns if turn.user_message_id in {message.id for message in users(final)}]
            assert turns[-1].settings.model_selection == selections['next'], 'resume settings leaked into next message'
            assert turns[-1].settings.effort == 'low'
            assert len(captures) == 4
            expected = [(turn.settings, turn.usage) for turn in turns]
            app.shutdown()
            app = b.VeluneApplication.open(options)
            reopened = app.open_conversation(runtime, conversation).snapshot
            assert len(users(reopened)) == 2
            actual = [(turn.settings, turn.usage) for turn in reopened.transcript_turns if turn.user_message_id in {message.id for message in users(reopened)}]
            assert actual == expected, ([(vars(settings), vars(usage)) for settings, usage in expected], [(vars(settings), vars(usage)) for settings, usage in actual])
            held.clear()
            release.clear()
            conversation = app.create_conversation(runtime, str(root / 'project'), keys['first']).snapshot.conversation.id
            app.send_turn(runtime, conversation, runtime, keys['first'], 'SYNTHETIC_COLD_STOP')
            wait(lambda _: held.is_set(), 'second conversation reached upstream')
            app.cancel(runtime, conversation)
            wait(lambda s: s.follow_up.can_resume, 'second conversation stopped')
            app.shutdown()
            app = b.VeluneApplication.open(options)
            reopened = app.open_conversation(runtime, conversation).snapshot
            assert reopened.follow_up.can_resume, 'cold-open lost the native interruption state'
            resume(selections['first'])
            cold = wait(lambda s: s.actions.can_send and not s.follow_up.can_continue, 'cold-open context resumed')
            assert len(users(cold)) == 1
            assert len(captures) == 6
            release.set()
            if runtime == 'codex':
                conversation = app.create_conversation(runtime, str(root / 'project'), keys['first']).snapshot.conversation.id
                app.send_turn(runtime, conversation, runtime, keys['first'], 'SYNTHETIC_FAIL')
                wait(lambda s: s.run_state == b.BindingRunState.FAILED and s.follow_up.can_resume, 'failed context is resumable')
                resume(selections['first'])
                recovered = wait(lambda s: s.actions.can_send and s.run_state != b.BindingRunState.FAILED, 'failed context recovered')
                assert len(users(recovered)) == 1
                assert len(captures) == 8
            print('PASS:', runtime, 'input-free recovery has no new user message or tool replay; rejects preserve recovery; paused queue, subsequent send and reopened settings/usage remain correct.')
        finally:
            release.set()
            current = snapshot()
            if current.actions.can_cancel:
                app.cancel(runtime, conversation)
                wait(lambda s: not s.actions.can_cancel, 'cleanup stopped the runtime')
            app.shutdown()
            server.shutdown()
            server.server_close()


if __name__ == '__main__':
    main()
