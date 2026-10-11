#!/usr/bin/env python3
"""Manual acceptance for following native conversations live.

Synthetic Pi 1.0.2 sessions in a temporary HOME are changed by a separate
process, as another Pi client would, while the UniFFI application watches
them. Checks that an open conversation is read again after a native change,
that an unfinished external turn is presented as running elsewhere and ends
when its reply is written, that a conversation started outside Velune is
reported, and that Velune's own execution is never reported as running
elsewhere. No real credential, provider, configuration or session is read;
this is not an automated test.
"""
import argparse
import importlib.util
import json
import pathlib
import shutil
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

        # Velune's own turn streams until its gate file disappears.
        class Upstream(BaseHTTPRequestHandler):
            def log_message(self, *_):
                pass

            def do_POST(self):
                body = json.loads(self.rfile.read(int(self.headers['Content-Length'])))
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
                for item in (chunk({'content': 'OWN_REPLY'}),
                             dict(chunk({}, 'stop'), usage={'prompt_tokens': 1, 'completion_tokens': 1, 'total_tokens': 2})):
                    self.wfile.write(('data: ' + json.dumps(item) + '\n\n').encode())
                self.wfile.write(b'data: [DONE]\n\n')
                self.wfile.flush()

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
                                               context_window=8192, max_output_tokens=128, reasoning_levels=None, adapter_metadata_json=None)]),
                b.BindingAuthenticationEdit.SET_API_KEY(value='SYNTHETIC_ONLY'))
            model = app.list().gateways[0].providers[0].models[0].record_key
            ids = {c.title: c.id for c in app.list().conversations}
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
            external('append', path_a, 'user', 'EXTERNAL_PROMPT')
            wait(lambda: 'EXTERNAL_PROMPT' in texts(a), 'open conversation read again after a native change')
            assert snapshot(a).run_state == b.BindingRunState.ELSEWHERE, snapshot(a).run_state
            wait(lambda: a in elsewhere(), 'conversation reported running elsewhere')
            print('PASS an external prompt appears live and the conversation runs elsewhere', flush=True)

            external('append', path_a, 'assistant', 'EXTERNAL_REPLY')
            wait(lambda: 'EXTERNAL_REPLY' in texts(a), 'external reply appears')
            assert snapshot(a).run_state == b.BindingRunState.IDLE and snapshot(a).actions.can_send
            wait(lambda: a not in elsewhere(), 'conversation no longer runs elsewhere')
            print('PASS the external reply ends running elsewhere', flush=True)

            # Pi writes a new session file once its first assistant message exists.
            path_c = external('create', 'C', 'user', 'PROMPT_C')
            external('append', path_c, 'tool-call', 'read')
            c = 'pi:' + path_c
            wait(lambda: c in elsewhere(), 'conversation started outside Velune reported')
            assert c in {item.id for item in app.list().conversations}
            print('PASS a conversation started outside Velune is reported running elsewhere and listed', flush=True)

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

            external('append', b_id.removeprefix('pi:'), 'user', 'EXTERNAL_AFTER_OWN')
            wait(lambda: 'EXTERNAL_AFTER_OWN' in texts(b_id), 'idle execution released and read again')
            assert 'OWN_REPLY' in texts(b_id) and snapshot(b_id).run_state == b.BindingRunState.ELSEWHERE
            external('append', b_id.removeprefix('pi:'), 'assistant', 'EXTERNAL_REPLY_B')
            wait(lambda: snapshot(b_id).run_state == b.BindingRunState.IDLE, 'external turn after own ended')
            app.send_turn('pi', b_id, 'pi', model, 'OWN_AGAIN')
            wait(lambda: texts(b_id).count('OWN_REPLY') == 2 and snapshot(b_id).actions.can_send, 'own turn resumes the changed conversation')
            assert 'EXTERNAL_REPLY_B' in texts(b_id)
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
