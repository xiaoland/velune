#!/usr/bin/env python3
"""Manual acceptance for following real Codex and Claude Code sessions live.

Velune runtimes for Codex 0.159.3 and Claude Code 2.1.286 use managed HOMEs
inside a temporary VELUNE_HOME. The real CLIs, started separately as another
client would be, write sessions into those HOMEs against a loopback upstream
that holds each turn until released. Running comes from each Harness's own
liveness record: Codex's thread writer lock, Claude Code's session registry.
Checks that a session started outside Velune is reported running and listed,
that its end is noticed, that an open conversation is read again with a
resumed external turn running and then its reply, and that a process killed
mid-turn stops running at once. Only loopback synthetic responses are
used: no real model, credential, HOME or session is read; this is not an
automated test.
"""
import argparse
import hashlib
import importlib.util
import json
import pathlib
import subprocess
import sys
import tempfile
import threading
import time
from http.server import BaseHTTPRequestHandler, ThreadingHTTPServer

GATES = ('CODEX_ONE', 'CODEX_TWO', 'CODEX_THREE', 'CLAUDE_ONE', 'CLAUDE_TWO', 'CLAUDE_THREE')


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    for name in ('library', 'bindings', 'bundle', 'node', 'codex', 'claude'):
        parser.add_argument('--' + name, type=pathlib.Path, required=True)
    args = parser.parse_args()
    codex, claude = args.codex.resolve(), args.claude.resolve()

    with tempfile.TemporaryDirectory(prefix='velune-live-native-') as directory:
        root = pathlib.Path(directory).resolve()
        project = root / 'project'
        project.mkdir()

        def gate_of(body):
            found = [(body.rfind(gate), gate) for gate in GATES if gate in body]
            return max(found)[1] if found else None

        def hold(gate):
            deadline = time.monotonic() + 60
            while gate and (root / ('gate-' + gate)).exists() and time.monotonic() < deadline:
                time.sleep(.05)

        class Upstream(BaseHTTPRequestHandler):
            protocol_version = 'HTTP/1.1'

            def log_message(self, *_):
                pass

            def send_events(self, events):
                self.send_response(200)
                self.send_header('Content-Type', 'text/event-stream')
                self.send_header('Connection', 'close')
                self.end_headers()
                self.wfile.write(b': open\n\n')
                self.wfile.flush()
                return lambda: [self.wfile.write(('event: %s\ndata: %s\n\n' % (event['type'], json.dumps(event))).encode())
                                for event in events] and self.wfile.flush()

            def do_POST(self):
                raw = self.rfile.read(int(self.headers.get('Content-Length', '0'))).decode('utf-8', 'replace')
                body = json.loads(raw or '{}')
                gate = gate_of(raw)
                reply = 'REPLY_' + (gate or 'OTHER')
                if self.path.endswith('/responses'):
                    item = {'id': 'item', 'type': 'message', 'role': 'assistant', 'status': 'completed',
                            'content': [{'type': 'output_text', 'text': reply, 'annotations': []}]}
                    response = {'id': 'response', 'object': 'response', 'status': 'completed', 'output': [item],
                                'usage': {'input_tokens': 1, 'output_tokens': 1, 'total_tokens': 2}}
                    finish = self.send_events([
                        {'type': 'response.created', 'response': {**response, 'status': 'in_progress', 'output': []}},
                        {'type': 'response.output_item.added', 'output_index': 0, 'item': {**item, 'status': 'in_progress', 'content': []}},
                        {'type': 'response.content_part.added', 'item_id': 'item', 'output_index': 0, 'content_index': 0,
                         'part': {'type': 'output_text', 'text': '', 'annotations': []}},
                        {'type': 'response.output_text.delta', 'item_id': 'item', 'output_index': 0, 'content_index': 0, 'delta': reply},
                        {'type': 'response.output_text.done', 'item_id': 'item', 'output_index': 0, 'content_index': 0, 'text': reply},
                        {'type': 'response.content_part.done', 'item_id': 'item', 'output_index': 0, 'content_index': 0, 'part': item['content'][0]},
                        {'type': 'response.output_item.done', 'output_index': 0, 'item': item},
                        {'type': 'response.completed', 'response': response}])
                elif self.path.split('?')[0].endswith('/count_tokens'):
                    payload = b'{"input_tokens":1}'
                    self.send_response(200)
                    self.send_header('Content-Type', 'application/json')
                    self.send_header('Content-Length', str(len(payload)))
                    self.end_headers()
                    self.wfile.write(payload)
                    return
                elif self.path.split('?')[0].endswith('/messages'):
                    message = {'id': 'msg_synthetic', 'type': 'message', 'role': 'assistant', 'model': body.get('model', 'synthetic'),
                               'content': [], 'stop_reason': None, 'stop_sequence': None,
                               'usage': {'input_tokens': 1, 'output_tokens': 1}}
                    if not body.get('stream'):
                        hold(gate)
                        payload = json.dumps({**message, 'content': [{'type': 'text', 'text': reply}], 'stop_reason': 'end_turn'}).encode()
                        self.send_response(200)
                        self.send_header('Content-Type', 'application/json')
                        self.send_header('Content-Length', str(len(payload)))
                        self.end_headers()
                        self.wfile.write(payload)
                        return
                    finish = self.send_events([
                        {'type': 'message_start', 'message': message},
                        {'type': 'content_block_start', 'index': 0, 'content_block': {'type': 'text', 'text': ''}},
                        {'type': 'content_block_delta', 'index': 0, 'delta': {'type': 'text_delta', 'text': reply}},
                        {'type': 'content_block_stop', 'index': 0},
                        {'type': 'message_delta', 'delta': {'stop_reason': 'end_turn', 'stop_sequence': None}, 'usage': {'output_tokens': 1}},
                        {'type': 'message_stop'}])
                else:
                    self.send_response(404)
                    self.send_header('Content-Length', '0')
                    self.end_headers()
                    return
                hold(gate)
                try:
                    finish()
                except (BrokenPipeError, ConnectionResetError):
                    pass

        server = ThreadingHTTPServer(('127.0.0.1', 0), Upstream)
        threading.Thread(target=server.serve_forever, daemon=True).start()
        base = 'http://127.0.0.1:%d' % server.server_port

        spec = importlib.util.spec_from_file_location('velune_live_native', args.bindings / 'velune_bindings.py')
        b = importlib.util.module_from_spec(spec)
        sys.modules[spec.name] = b
        spec.loader.exec_module(b)
        app = b.VeluneApplication.open(b.BindingOptions(home_directory=str(root / 'velune'),
                                                         resources_directory=str(args.bundle / 'Contents/Resources')))
        processes = []
        try:
            managed = b.BindingRuntimeExecution(transport=b.BindingRuntimeTransport.LOCAL(),
                                                home=b.BindingRuntimeHome.MANAGED(remote_root=None),
                                                remote_resources_directory=None)
            app.upsert_runtime_with_execution(b.BindingRuntimeInstance(
                enabled=True, id='codex', name='Codex', type_id='codex-0.159.3', gateway_id='default',
                settings={'binary': str(codex), 'nodeBinary': str(args.node)}), managed)
            app.upsert_runtime_with_execution(b.BindingRuntimeInstance(
                enabled=True, id='claude', name='Claude Code', type_id='claude-code-2.1.286', gateway_id='default',
                settings={'binary': str(claude)}), managed)
            app.list()
            homes = {path.name: path for path in (root / 'velune/runtime-homes').iterdir()}
            codex_home = homes[hashlib.sha256(b'codex').hexdigest()]
            claude_home = homes[hashlib.sha256(b'claude').hexdigest()]
            (codex_home / '.codex').mkdir(parents=True, exist_ok=True)
            (codex_home / '.codex/config.toml').write_text(
                'model_provider = "synthetic"\n[model_providers.synthetic]\nname = "synthetic"\n'
                'base_url = "%s/v1"\nenv_key = "SYNTHETIC_KEY"\nwire_api = "responses"\nrequires_openai_auth = false\n' % base)
            codex_env = {'PATH': str(codex.parent) + ':/usr/bin:/bin', 'HOME': str(codex_home),
                         'CODEX_HOME': str(codex_home / '.codex'), 'SYNTHETIC_KEY': 'synthetic-only', 'LANG': 'C'}
            claude_env = {'PATH': str(args.node.parent) + ':/usr/bin:/bin', 'HOME': str(claude_home),
                          'CLAUDE_CONFIG_DIR': str(claude_home / '.claude'), 'ANTHROPIC_BASE_URL': base,
                          'ANTHROPIC_API_KEY': 'synthetic-only', 'DISABLE_AUTOUPDATER': '1', 'DISABLE_TELEMETRY': '1',
                          'DISABLE_ERROR_REPORTING': '1', 'CLAUDE_CODE_DISABLE_NONESSENTIAL_TRAFFIC': '1'}

            def external(command, env):
                process = subprocess.Popen(command, cwd=project, env=env, stdin=subprocess.DEVNULL,
                                           stdout=subprocess.PIPE, stderr=subprocess.PIPE, text=True)
                processes.append(process)
                return process

            def finished(process, label):
                out, err = process.communicate(timeout=60)
                assert process.returncode == 0, (label, out[-800:], err[-1500:])
                return out

            def wait(predicate, label, timeout=40):
                deadline = time.monotonic() + timeout
                while time.monotonic() < deadline:
                    value = predicate()
                    if value:
                        return value
                    time.sleep(.3)
                raise AssertionError(label)

            def elsewhere():
                return {item.conversation_id for item in app.conversation_activity(0).active
                        if item.run_state == b.BindingRunState.ELSEWHERE}

            def texts(runtime, conversation):
                current = app.snapshot(runtime, conversation).snapshot
                return current, [block.text for message in current.messages for block in message.blocks
                                 if isinstance(block, b.BindingMessageBlock.TEXT)]

            def follow(runtime, start, resume, first, second, third, files):
                (root / ('gate-' + first)).write_text('hold')
                process = start(first)
                path = wait(lambda: next(iter(files()), None), runtime + ' session file written')
                ids = wait(lambda: {i for i in elsewhere() if i.startswith(runtime + ':')}, runtime + ' started outside Velune is running elsewhere')
                conversation = next(iter(ids))
                # Liveness comes from the Harness's lock or registry, listing from
                # what it has written; the App lists again while they differ.
                wait(lambda: conversation in {c.id for c in [_row.conversation for _row in app.list().conversations]}, runtime + ' running session listed', timeout=10)
                (root / ('gate-' + first)).unlink()
                finished(process, first)
                wait(lambda: conversation not in elsewhere(), runtime + ' end noticed')
                print('PASS %s: a session started outside Velune runs, is listed and ends (%s)' % (runtime, path.name), flush=True)

                app.open_conversation(runtime, conversation)
                current, shown = texts(runtime, conversation)
                assert 'REPLY_' + first in shown and current.run_state == b.BindingRunState.IDLE, shown
                (root / ('gate-' + second)).write_text('hold')
                process = resume(conversation.split(':', 1)[1], second)
                wait(lambda: (lambda c, t: c.run_state == b.BindingRunState.ELSEWHERE and any(second in x for x in t))(*texts(runtime, conversation)),
                     runtime + ' resumed external turn appears running elsewhere')
                (root / ('gate-' + second)).unlink()
                finished(process, second)
                wait(lambda: (lambda c, t: c.run_state == b.BindingRunState.IDLE and 'REPLY_' + second in t)(*texts(runtime, conversation)),
                     runtime + ' external reply read again')
                print('PASS %s: an open conversation shows a resumed external turn live, then its reply' % runtime, flush=True)

                (root / ('gate-' + third)).write_text('hold')
                process = resume(conversation.split(':', 1)[1], third)
                wait(lambda: conversation in elsewhere(), runtime + ' third external turn runs')
                process.kill()
                process.wait()
                wait(lambda: conversation not in elsewhere()
                     and texts(runtime, conversation)[0].run_state == b.BindingRunState.IDLE,
                     runtime + ' killed mid-turn no longer runs', timeout=15)
                (root / ('gate-' + third)).unlink()
                print('PASS %s: a process killed mid-turn stops running' % runtime, flush=True)
                return path

            codex_args = ['--skip-git-repo-check', '--dangerously-bypass-approvals-and-sandbox', '--model', 'gpt-6.1-sol']
            rollout = follow(
                'codex',
                lambda gate: external([str(codex), 'exec', *codex_args, gate + ' prompt'], codex_env),
                lambda native, gate: external([str(codex), 'exec', 'resume', *codex_args, native, gate + ' prompt'], codex_env),
                'CODEX_ONE', 'CODEX_TWO', 'CODEX_THREE',
                lambda: sorted((codex_home / '.codex/sessions').rglob('rollout-*.jsonl')))
            kinds = [json.loads(line).get('payload', {}).get('type') for line in rollout.read_text().splitlines()
                     if json.loads(line).get('type') == 'event_msg']
            print('INFO codex rollout turn events:', [kind for kind in kinds if kind and ('task' in kind or 'turn' in kind)], flush=True)

            claude_args = ['-p', '--model', 'claude-sonnet-4-5', '--output-format', 'text']
            follow(
                'claude',
                lambda gate: external([str(claude), *claude_args, gate + ' prompt'], claude_env),
                lambda native, gate: external([str(claude), *claude_args, '--resume', native, gate + ' prompt'], claude_env),
                'CLAUDE_ONE', 'CLAUDE_TWO', 'CLAUDE_THREE',
                lambda: [path for path in (claude_home / '.claude/projects').rglob('*.jsonl') if 'subagents' not in path.parts])
        finally:
            for gate in root.glob('gate-*'):
                gate.unlink()
            for process in processes:
                if process.poll() is None:
                    process.terminate()
            app.shutdown()
            server.shutdown()


if __name__ == '__main__':
    main()
