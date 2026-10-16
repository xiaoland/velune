#!/usr/bin/env python3
"""Manual acceptance for following native conversations live.

Pi 1.0.2 sessions in a temporary HOME are seeded through the Pi SDK, and the
real Pi CLI, started separately as another client would be, runs turns in
them against a loopback upstream that holds each turn until released, while
the UniFFI application watches. Pi keeps no liveness record, so Velune takes
a session as running while a live Pi process in its working directory has an
unfinished turn. Checks that an open conversation is read again after a
native change, that an external turn is presented as running and ends with
its reply, that a conversation started outside Velune is reported and stops
running when its process is killed mid-turn, and that Velune's own execution
is never reported as running elsewhere. No real credential, provider,
configuration or session is read; this is not an automated test.
"""
import argparse
import importlib.util
import json
import os
import pathlib
import shutil
import signal
import subprocess
import sys
import tempfile
import threading
import time
from http.server import BaseHTTPRequestHandler, ThreadingHTTPServer


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    for name in ('library', 'bindings', 'bundle', 'node', 'pi'):
        parser.add_argument('--' + name, type=pathlib.Path, required=True)
    args = parser.parse_args()
    sdk = None
    for directory in args.pi.resolve().parents:
        package = directory / 'package.json'
        if package.is_file() and json.loads(package.read_text()).get('name') == '@earendil-works/pi-coding-agent':
            metadata = json.loads(package.read_text())
            if metadata['version'] != '1.0.2':
                parser.error('this fixture requires external Pi 1.0.2')
            sdk = directory / metadata['exports']['.']['import']
            break
    if sdk is None:
        parser.error('--pi must belong to the Pi 1.0.2 package')

    with tempfile.TemporaryDirectory(prefix='velune-live-') as directory:
        root = pathlib.Path(directory).resolve()
        sessions = root / 'runtime/sessions/project'
        for name in ('home', 'project', 'bindings', 'resources'):
            (root / name).mkdir(parents=True)
        sessions.mkdir(parents=True)
        (root / 'resources/node_modules').symlink_to(args.bundle / 'Contents/Resources/node_modules')
        for resource in (args.bundle / 'Contents/Resources').glob('*.mjs'):
            shutil.copy(resource, root / 'resources')

        # Every turn streams until the gate file disappears.
        class Upstream(BaseHTTPRequestHandler):
            def log_message(self, *_):
                pass

            def do_POST(self):
                raw = self.rfile.read(int(self.headers['Content-Length'])).decode()
                body = json.loads(raw)
                reply = 'EXTERNAL_REPLY' if 'EXTERNAL_' in raw.rsplit('OWN_', 1)[-1] else 'OWN_REPLY'
                self.send_response(200)
                self.send_header('Content-Type', 'text/event-stream')
                self.end_headers()

                def chunk(delta, finish=None):
                    return {'id': 's', 'object': 'chat.completion.chunk', 'created': 1, 'model': body['model'],
                            'choices': [{'index': 0, 'delta': delta, 'finish_reason': finish}]}
                self.wfile.write(('data: ' + json.dumps(chunk({'role': 'assistant'})) + '\n\n').encode())
                self.wfile.flush()
                deadline = time.monotonic() + 30
                while (root / 'gate').exists() and time.monotonic() < deadline:
                    time.sleep(.02)
                try:
                    for item in (chunk({'content': reply}),
                                 dict(chunk({}, 'stop'), usage={'prompt_tokens': 1, 'completion_tokens': 1, 'total_tokens': 2})):
                        self.wfile.write(('data: ' + json.dumps(item) + '\n\n').encode())
                    self.wfile.write(b'data: [DONE]\n\n')
                    self.wfile.flush()
                except (BrokenPipeError, ConnectionResetError):
                    pass  # a client killed mid-turn

        server = ThreadingHTTPServer(('127.0.0.1', 0), Upstream)
        threading.Thread(target=server.serve_forever, daemon=True).start()

        # Another Pi client: creates sessions and appends messages through the SDK.
        env = {'HOME': str(root / 'home'), 'PATH': str(args.node.parent) + ':/usr/bin:/bin'}
        native = root / 'native.mjs'
        native.write_text('import {SessionManager} from ' + json.dumps(sdk.resolve().as_uri()) + ''';
const [operation, target, role, text] = process.argv.slice(2);
const usage = {input:1,output:1,cacheRead:0,cacheWrite:0,totalTokens:2,cost:{input:0,output:0,cacheRead:0,cacheWrite:0,total:0}};
const manager = operation === 'create'
  ? SessionManager.create(process.env.PROJECT, process.env.SESSIONS)
  : SessionManager.open(target, process.env.SESSIONS);
if (operation === 'create') manager.appendSessionInfo(target);
if (role === 'user') manager.appendMessage({role:'user',content:[{type:'text',text}],timestamp:Date.now()});
if (role === 'assistant') manager.appendMessage({role:'assistant',content:[{type:'text',text}],api:'openai-completions',provider:'synthetic',model:'synthetic',stopReason:'stop',usage,timestamp:Date.now()});
if (role === 'tool-call') manager.appendMessage({role:'assistant',content:[{type:'toolCall',id:'call-1',name:text,arguments:{}}],api:'openai-completions',provider:'synthetic',model:'synthetic',stopReason:'toolUse',usage,timestamp:Date.now()});
console.log(manager.getSessionFile());
''')
        native_env = dict(env, PROJECT=str(root / 'project'), SESSIONS=str(sessions))

        def external(operation, target, role, text):
            result = subprocess.run([str(args.node), str(native), operation, target, role, text],
                                    env=native_env, check=True, capture_output=True, text=True)
            return result.stdout.strip()

        # The real Pi CLI as another client, with its own agent directory and a
        # loopback provider, writing into the runtime's sessions. A shell
        # parents it: this process hosts the application, whose own Pi
        # children are its execution, not another client.
        external_agent = root / 'external-agent'
        external_agent.mkdir()
        (external_agent / 'models.json').write_text(json.dumps({'providers': {'synthetic': {
            'baseUrl': f'http://127.0.0.1:{server.server_port}/v1', 'api': 'openai-completions', 'apiKey': 'SYNTHETIC_ONLY',
            'models': [{'id': 'synthetic', 'name': 'Synthetic', 'contextWindow': 8192, 'maxTokens': 128}]}}}))
        pi_env = dict(env, PI_CODING_AGENT_DIR=str(external_agent))

        def pi_client(prompt, session=None):
            command = [str(args.node), str(args.pi), '--session-dir', str(sessions),
                       *(['--session', session] if session else []),
                       '--provider', 'synthetic', '--model', 'synthetic', '-p', prompt]
            return subprocess.Popen(['/bin/sh', '-c', '"$@"; exit $?', 'sh', *command], cwd=root / 'project', env=pi_env,
                                    stdin=subprocess.DEVNULL, stdout=subprocess.PIPE, stderr=subprocess.PIPE,
                                    text=True, start_new_session=True)

        def ended(process, label):
            out, err = process.communicate(timeout=30)
            assert process.returncode == 0, (label, out[-800:], err[-1500:])

        for title in ('A', 'B'):
            path = external('create', title, 'user', 'PROMPT_' + title)
            external('append', path, 'assistant', 'SEED_' + title)
            time.sleep(.01)

        shutil.copy2(args.bindings / 'velune_bindings.py', root / 'bindings/velune_bindings.py')
        shutil.copy2(args.library, root / 'bindings' / args.library.name)
        spec = importlib.util.spec_from_file_location('velune_live', root / 'bindings/velune_bindings.py')
        b = importlib.util.module_from_spec(spec)
        sys.modules[spec.name] = b
        spec.loader.exec_module(b)
        app = b.VeluneApplication.open(b.BindingOptions(home_directory=str(root / 'app'), resources_directory=str(root / 'resources')))
        try:
            app.upsert_runtime(b.BindingRuntimeInstance(
                enabled=True, id='pi', name='Pi', type_id='pi-1.0.2', gateway_id='default',
                settings={'binary': str(args.pi), 'nodeBinary': str(args.node), 'agentDir': str(root / 'runtime')}))
            app.save_provider('default', b.BindingProviderDraft(
                id='synthetic', name='Synthetic', protocol=b.BindingGatewayProtocol.CHAT_COMPLETIONS_V1,
                endpoint=f'http://127.0.0.1:{server.server_port}/v1',
                models=[b.BindingProviderModel(record_key='', provider_model_id='synthetic', nickname='Synthetic', icon=None,
                                               context_window=8192, max_output_tokens=128, reasoning_levels=None, adapter_metadata_json=None)], quota_limits=[]),
                b.BindingAuthenticationEdit.SET_API_KEY(value='SYNTHETIC_ONLY'))
            model = app.list().gateways[0].providers[0].models[0].record_key
            ids = {c.title: c.id for c in [_row.conversation for _row in app.list().conversations]}
            a, b_id = ids['A'], ids['B']
            path_a = a.removeprefix('pi:')

            def snapshot(conversation):
                return app.snapshot('pi', conversation).snapshot

            def texts(conversation):
                current = snapshot(conversation)
                return [block.text for message in current.messages for block in message.blocks
                        if isinstance(block, b.BindingMessageBlock.TEXT)]

            def elsewhere():
                return {item.conversation_id for item in app.conversation_activity(0).active
                        if item.run_state == b.BindingRunState.ELSEWHERE}

            def wait(predicate, label, timeout=20):
                deadline = time.monotonic() + timeout
                while time.monotonic() < deadline:
                    value = predicate()
                    if value:
                        return value
                    time.sleep(.2)
                raise AssertionError(label)

            app.open_conversation('pi', a)
            assert snapshot(a).run_state == b.BindingRunState.IDLE
            (root / 'gate').write_text('hold')
            client = pi_client('EXTERNAL_PROMPT', path_a)
            wait(lambda: 'EXTERNAL_PROMPT' in texts(a), 'open conversation read again after a native change')
            wait(lambda: snapshot(a).run_state == b.BindingRunState.ELSEWHERE, 'external turn runs')
            # Its unfinished turn is presented as running, untimed.
            last = snapshot(a).transcript_turns[-1]
            assert last.is_running and last.duration_ms is None, last
            wait(lambda: a in elsewhere(), 'conversation reported running elsewhere')
            print('PASS an external Pi turn appears live and the conversation runs, its turn running', flush=True)

            (root / 'gate').unlink()
            ended(client, 'external turn')
            wait(lambda: 'EXTERNAL_REPLY' in texts(a), 'external reply appears')
            wait(lambda: snapshot(a).run_state == b.BindingRunState.IDLE, 'external turn ended')
            assert snapshot(a).actions.can_send and not snapshot(a).transcript_turns[-1].is_running
            wait(lambda: a not in elsewhere(), 'conversation no longer runs elsewhere')
            print('PASS the external reply ends the run', flush=True)

            (root / 'gate').write_text('hold')
            client = pi_client('EXTERNAL_PROMPT_C')
            started = wait(lambda: {item for item in elsewhere() if item not in (a, b_id)}, 'conversation started outside Velune reported')
            c = next(iter(started))
            assert c in {item.id for item in [_row.conversation for _row in app.list().conversations]}
            print('PASS a conversation started outside Velune is reported running and listed', flush=True)
            os.killpg(client.pid, signal.SIGKILL)
            client.wait()
            wait(lambda: c not in elsewhere(), 'a Pi process killed mid-turn no longer runs', timeout=15)
            (root / 'gate').unlink()
            print('PASS a turn whose Pi process ended mid-turn is no longer running', flush=True)

            (root / 'gate').write_text('hold')
            app.open_conversation('pi', b_id)
            app.send_turn('pi', b_id, 'pi', model, 'OWN_PROMPT')
            wait(lambda: snapshot(b_id).run_state == b.BindingRunState.RUNNING, 'own turn runs')
            time.sleep(6)
            assert b_id not in elsewhere(), 'own execution is not running elsewhere'
            assert {item.conversation_id: item.run_state for item in app.conversation_activity(0).active}[b_id] == b.BindingRunState.RUNNING
            (root / 'gate').unlink()
            wait(lambda: 'OWN_REPLY' in texts(b_id) and snapshot(b_id).actions.can_send, 'own turn finished')
            time.sleep(7)
            assert snapshot(b_id).run_state == b.BindingRunState.IDLE and b_id not in elsewhere()
            print('PASS own execution and its trailing writes are never running elsewhere', flush=True)

            (root / 'gate').write_text('hold')
            client = pi_client('EXTERNAL_AFTER_OWN', b_id.removeprefix('pi:'))
            wait(lambda: 'EXTERNAL_AFTER_OWN' in texts(b_id), 'idle execution released and read again')
            wait(lambda: snapshot(b_id).run_state == b.BindingRunState.ELSEWHERE, 'external turn after own runs')
            assert 'OWN_REPLY' in texts(b_id)
            (root / 'gate').unlink()
            ended(client, 'external turn after own')
            wait(lambda: 'EXTERNAL_REPLY' in texts(b_id) and snapshot(b_id).run_state == b.BindingRunState.IDLE, 'external turn after own ended')
            app.send_turn('pi', b_id, 'pi', model, 'OWN_AGAIN')
            wait(lambda: texts(b_id).count('OWN_REPLY') == 2 and snapshot(b_id).actions.can_send, 'own turn resumes the changed conversation')
            print('PASS an external change after own execution is read and the next turn continues it', flush=True)
        finally:
            for gate in root.glob('gate'):
                gate.unlink()
            for _ in range(100):
                try:
                    app.shutdown()
                    break
                except b.BindingError:
                    time.sleep(.1)
            server.shutdown()


if __name__ == '__main__':
    main()
