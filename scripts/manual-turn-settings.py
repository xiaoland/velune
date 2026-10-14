#!/usr/bin/env python3
"""Manual acceptance for the settings each transcript turn shows.

Real Codex 0.159.3, Claude Code 2.1.286 and Pi 1.0.2 run in managed HOMEs
inside a temporary VELUNE_HOME, through Velune's gateway, against a loopback
upstream that answers every turn with text. A turn Velune sends shows what it
asked for (runtime, Velune's name for the model, reasoning level, permission
mode), bound to that turn and not to the one before; a turn without a level
or mode keeps the conversation's current one, and Velune sends it again.
Each turn shows the tokens the harness reports for it, reasoning included
where the harness counts it apart. After a restart, reading the history
again shows what each harness recorded: Codex's model, level, permission
preset and usage from its rollout, Claude Code's model, level, permission
mode and usage, Pi's model and usage. Each turn also shows the context its last
request left and the model's window (the managed model's configured window,
else the harness's report), which the composer's status line reads; the
branch of the working directory and the status line's chosen items, kept in
display order across a restart, are read through the same application.
No real model, credential, HOME or session is read; this is not an
automated test.
"""
import argparse
import importlib.util
import json
import pathlib
import subprocess
import sys
import tempfile
import threading
import time
from http.server import BaseHTTPRequestHandler, ThreadingHTTPServer

REPLY = 'SYNTHETIC_REPLY'
# What the upstream reports for every request, in each protocol's own terms.
RESPONSES_USAGE = {'input_tokens': 100, 'input_tokens_details': {'cached_tokens': 40}, 'output_tokens': 30,
                   'output_tokens_details': {'reasoning_tokens': 12}, 'total_tokens': 130}
MESSAGES_USAGE = {'input_tokens': 50, 'cache_read_input_tokens': 20, 'cache_creation_input_tokens': 5, 'output_tokens': 9}


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
                            'usage': RESPONSES_USAGE}
                return self.events([
                    {'type': 'response.created', 'response': {**response, 'status': 'in_progress', 'output': []}},
                    {'type': 'response.output_item.added', 'output_index': 0, 'item': item},
                    {'type': 'response.output_item.done', 'output_index': 0, 'item': item},
                    {'type': 'response.completed', 'response': response}])
            # Claude Code's auto-mode classifier asks whether to block a call.
            text = '<block>no</block>' if 'security monitor' in json.dumps(body.get('system')) else REPLY
            message = {'id': 'msg_synthetic_%d' % len(captures), 'type': 'message', 'role': 'assistant', 'model': body.get('model', 'synthetic'),
                       'content': [], 'stop_reason': None, 'stop_sequence': None, 'usage': MESSAGES_USAGE}
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
                {'type': 'message_delta', 'delta': {'stop_reason': 'end_turn', 'stop_sequence': None}, 'usage': {'output_tokens': MESSAGES_USAGE['output_tokens']}},
                {'type': 'message_stop'}])

    server = ThreadingHTTPServer(('127.0.0.1', 0), Upstream)
    threading.Thread(target=server.serve_forever, daemon=True).start()
    endpoint = 'http://127.0.0.1:%d/v1' % server.server_port
    spec = importlib.util.spec_from_file_location('velune_turn_settings', args.bindings / 'velune_bindings.py')
    b = importlib.util.module_from_spec(spec)
    sys.modules[spec.name] = b
    spec.loader.exec_module(b)

    with tempfile.TemporaryDirectory(prefix='velune-turn-settings-') as directory:
        root = pathlib.Path(directory).resolve()
        (root / 'project').mkdir()
        subprocess.run(['git', 'init', '-q', '-b', 'status-line-check', str(root / 'project')], check=True)
        (root / 'pi').mkdir()
        options = b.BindingOptions(home_directory=str(root / 'velune'), resources_directory=str(args.bundle / 'Contents/Resources'))
        app = b.VeluneApplication.open(options)
        keys, sent = {}, {}
        ordered = [b.BindingStatusLineItem.CONTEXT_USAGE, b.BindingStatusLineItem.WORKING_DIRECTORY, b.BindingStatusLineItem.SESSION_COST]
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

            def provider(identifier, protocol, levels):
                app.save_provider('default', b.BindingProviderDraft(
                    id=identifier, name=identifier, protocol=protocol, endpoint=endpoint,
                    models=[b.BindingProviderModel(record_key='', provider_model_id='synthetic-' + identifier, nickname=identifier,
                                                   icon=None, context_window=32768, max_output_tokens=1024,
                                                   reasoning_levels=levels, adapter_metadata_json=None)]),
                    b.BindingAuthenticationEdit.SET_API_KEY(value='SYNTHETIC_ONLY'))
                return next(p for p in app.list().gateways[0].providers if p.id == identifier).models[0].record_key

            responses = provider('responses', b.BindingGatewayProtocol.RESPONSES_V1, ['none', 'low', 'medium', 'high', 'xhigh'])
            messages = provider('messages', b.BindingGatewayProtocol.MESSAGES_V1, ['low', 'medium', 'high', 'max'])
            keys = {'codex': responses, 'claude': messages, 'pi': responses}

            requests = {}

            def turn(runtime, conversation, text, effort, mode):
                before = len(captures)
                conversation = app.send_turn_with_selection(
                    runtime, conversation, runtime, b.BindingModelSelection.MANAGED(model_record_key=keys[runtime]),
                    b.BindingComposedTurn(text=text, effort=effort, permission_mode=mode, attachments=[])).snapshot.conversation.id
                deadline = time.monotonic() + 90
                while True:
                    current = app.snapshot(runtime, conversation).snapshot
                    assert current.run_state != b.BindingRunState.FAILED, (runtime, text)
                    said = [block.text for message in current.messages[-1:] for block in message.blocks
                            if isinstance(block, b.BindingMessageBlock.TEXT)]
                    if current.run_state == b.BindingRunState.IDLE and REPLY in said:
                        requests[text] = captures[before:]
                        return conversation, current
                    assert time.monotonic() < deadline, (runtime, text, current.run_state)
                    time.sleep(.1)

            def settings(snapshot, text):
                """The settings of the turn whose user message says `text`."""
                users = {message.id for message in snapshot.messages if message.role == b.BindingMessageRole.USER
                         and any(isinstance(block, b.BindingMessageBlock.TEXT) and block.text == text for block in message.blocks)}
                found = [t.settings for t in snapshot.transcript_turns if t.user_message_id in users]
                assert len(found) == 1, (text, found)
                value = found[0]
                return value and (value.runtime_id, value.model, value.effort, value.permission_mode)

            def usage(snapshot, text):
                """The usage of the turn whose user message says `text`."""
                users = {message.id for message in snapshot.messages if message.role == b.BindingMessageRole.USER
                         and any(isinstance(block, b.BindingMessageBlock.TEXT) and block.text == text for block in message.blocks)}
                value = next(t.usage for t in snapshot.transcript_turns if t.user_message_id in users)
                return value and (value.input_tokens, value.cache_read_tokens, value.cache_write_tokens, value.output_tokens, value.reasoning_tokens)

            def context(snapshot, text):
                """What the turn's last request left in the context, against the window."""
                users = {message.id for message in snapshot.messages if message.role == b.BindingMessageRole.USER
                         and any(isinstance(block, b.BindingMessageBlock.TEXT) and block.text == text for block in message.blocks)}
                value = next(t.usage for t in snapshot.transcript_turns if t.user_message_id in users)
                return value and (value.context_tokens, value.context_window)

            def expected_context(runtime):
                """One request's input, cache included, and its output."""
                if runtime == 'claude':
                    u = MESSAGES_USAGE
                    return u['input_tokens'] + u['cache_read_input_tokens'] + u['cache_creation_input_tokens'] + u['output_tokens']
                return RESPONSES_USAGE['input_tokens'] + RESPONSES_USAGE['output_tokens']

            def expected(runtime, text):
                """What the upstream reported for the turn's requests, in Velune's terms:
                input with cache reads and writes, output with reasoning."""
                n = len(requests[text])
                assert n >= 1, (runtime, text)
                if runtime == 'claude':
                    u = MESSAGES_USAGE
                    return (n * (u['input_tokens'] + u['cache_read_input_tokens'] + u['cache_creation_input_tokens']),
                            n * u['cache_read_input_tokens'], n * u['cache_creation_input_tokens'], n * u['output_tokens'], None)
                u = RESPONSES_USAGE
                return (n * u['input_tokens'], n * u['input_tokens_details']['cached_tokens'], 0, n * u['output_tokens'],
                        n * u['output_tokens_details']['reasoning_tokens'])

            def effort_sent(runtime, text):
                """The reasoning level the turn's last upstream request carried."""
                body = [c for c in requests[text] if text in json.dumps(c)][-1]
                return (body.get('reasoning') or {}).get('effort') if runtime != 'claude' else (body.get('output_config') or {}).get('effort') or (body.get('thinking') or {}).get('type')

            for runtime, first, second in (('codex', ('high', 'read-only'), (None, None)),
                                           ('claude', ('high', 'plan'), (None, None)),
                                           ('pi', ('high', None), (None, None))):
                conversation = app.create_conversation(runtime, str(root / 'project'), keys[runtime]).snapshot.conversation.id
                conversation, _ = turn(runtime, conversation, runtime.upper() + '_FIRST', *first)
                conversation, snapshot = turn(runtime, conversation, runtime.upper() + '_SECOND', *second)
                name = {'codex': 'responses · responses', 'claude': 'messages · messages', 'pi': 'responses · responses'}[runtime]
                got = settings(snapshot, runtime.upper() + '_FIRST')
                assert got[0] == runtime and got[1] == name and got[2] == first[0], got
                assert first[1] is None or got[3] == first[1], got
                got = settings(snapshot, runtime.upper() + '_SECOND')
                # Without a choice the turn keeps the conversation's level and mode, and says so.
                assert got[0] == runtime and got[1] == name and got[2] == first[0], ('a turn without a level keeps the current one', got)
                assert got[3] == first[1], ('a turn without a mode keeps the current one', got)
                if runtime != 'claude':
                    assert effort_sent(runtime, runtime.upper() + '_SECOND') == first[0], (runtime, effort_sent(runtime, runtime.upper() + '_SECOND'))
                for text in (runtime.upper() + '_FIRST', runtime.upper() + '_SECOND'):
                    assert usage(snapshot, text) == expected(runtime, text), (runtime, text, usage(snapshot, text), expected(runtime, text))
                    tokens, window = context(snapshot, text)
                    # The managed model's window, not the harness's guess at a model it does not know.
                    assert (tokens, window) == (expected_context(runtime), 32768), (runtime, text, tokens, window)
                sent[runtime] = conversation
            print('PASS live: each turn shows the runtime, model name, level and mode Velune sent for it, and its usage; '
                  'a turn without a level or mode keeps the current one', flush=True)
            print('PASS live context: each turn shows its last request\'s context against the managed model\'s window', flush=True)
            assert app.working_branch('codex', sent['codex']) == 'status-line-check'
            items = [b.BindingStatusLineItem.SESSION_COST, b.BindingStatusLineItem.CONTEXT_USAGE, b.BindingStatusLineItem.SESSION_COST,
                     b.BindingStatusLineItem.WORKING_DIRECTORY]
            assert app.list().status_line == [b.BindingStatusLineItem.CONTEXT_USAGE], app.list().status_line
            assert app.set_status_line(items) == ordered, app.list().status_line
            print('PASS status line: the working branch is read; the items default to context usage and are kept once each, in display order', flush=True)
        finally:
            app.shutdown()

        # A fresh application has no requests of its own; what remains is the native record.
        app = b.VeluneApplication.open(options)
        try:
            history = {runtime: app.read_conversation(runtime, conversation).snapshot for runtime, conversation in sent.items()}
            assert app.list().status_line == ordered, app.list().status_line
            for runtime, snapshot in history.items():
                for text in (runtime.upper() + '_FIRST', runtime.upper() + '_SECOND'):
                    tokens, window = context(snapshot, text)
                    assert (tokens, window) == (expected_context(runtime), 32768), (runtime, text, tokens, window)
            print('PASS history context: each turn\'s context comes from the native record, against the managed model\'s window', flush=True)
            got = settings(history['codex'], 'CODEX_FIRST')
            assert got == ('codex', 'synthetic-responses', 'high', 'read-only'), got
            # A turn without a mode keeps the session's: the record says so.
            got = settings(history['codex'], 'CODEX_SECOND')
            assert got == ('codex', 'synthetic-responses', 'high', 'read-only'), got
            for text in ('CODEX_FIRST', 'CODEX_SECOND'):
                assert usage(history['codex'], text) == expected('codex', text), (text, usage(history['codex'], text))
            print('PASS codex history: model, level, permission preset and usage come from the rollout', flush=True)
            got = settings(history['claude'], 'CLAUDE_FIRST')
            assert got[:2] == ('claude', 'synthetic-messages') and got[3] == 'plan', got
            got = settings(history['claude'], 'CLAUDE_SECOND')
            # Claude Code records the level each reply ran with.
            assert got == ('claude', 'synthetic-messages', 'high', 'plan'), got
            for text in ('CLAUDE_FIRST', 'CLAUDE_SECOND'):
                assert usage(history['claude'], text) == expected('claude', text), (text, usage(history['claude'], text))
            print('PASS claude history: model, level, permission mode and usage come from the session record', flush=True)
            # Pi records the gateway's route alias; the turn names the managed model.
            got = settings(history['pi'], 'PI_FIRST')
            assert got[:2] == ('pi', 'responses · responses'), got
            for text in ('PI_FIRST', 'PI_SECOND'):
                assert usage(history['pi'], text) == expected('pi', text), (text, usage(history['pi'], text))
            print('PASS pi history: the model comes from the replies, by its Velune name, and the usage from their records', flush=True)
        finally:
            app.shutdown()
            server.shutdown()


if __name__ == '__main__':
    main()
