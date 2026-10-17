#!/usr/bin/env python3
"""Manual acceptance for the send queue and continuing a stopped turn.

A synthetic Pi 1.0.2 conversation in a temporary HOME runs turns through the
UniFFI application against a loopback upstream that holds each turn until
released. Checks queueing while a turn runs, editing, removing and reordering,
sending one by one after a normal end, pausing after a stopped or failed turn,
resuming, sending at once when nothing runs, and whether continuing is
offered. No real credential, provider, configuration or session is read; this
is not an automated test.
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


GATES = ('TURN_A', 'Q_ONE', 'Q_TWO', 'Q_THREE', 'Q_EDITED', 'TURN_S', 'Q_PAUSED', 'TURN_FAIL', 'Q_FAILED', 'Q_IDLE', 'CONTINUE_GATE')


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

    with tempfile.TemporaryDirectory(prefix='velune-queue-') as directory:
        root = pathlib.Path(directory).resolve()
        for name in ('home', 'project', 'bindings', 'resources', 'runtime/sessions/project'):
            (root / name).mkdir(parents=True)
        (root / 'resources/node_modules').symlink_to(args.bundle / 'Contents/Resources/node_modules')
        for resource in (args.bundle / 'Contents/Resources').glob('*.mjs'):
            shutil.copy(resource, root / 'resources')

        # Each turn streams until its gate file disappears; the user text names the gate.
        requests = []

        class Upstream(BaseHTTPRequestHandler):
            def log_message(self, *_):
                pass

            def do_POST(self):
                body = json.loads(self.rfile.read(int(self.headers['Content-Length'])))
                users = [message for message in body['messages'] if message.get('role') == 'user']
                last = json.dumps(users[-1]['content'], ensure_ascii=False) if users else ''
                name = next((n for n in GATES if n in last), 'CONTINUE_GATE' if '继续' in last else 'OTHER')
                requests.append(name)
                if name == 'TURN_FAIL':
                    self.send_response(400)
                    self.send_header('Content-Type', 'application/json')
                    self.end_headers()
                    self.wfile.write(b'{"error":{"message":"synthetic rejection","type":"invalid_request_error"}}')
                    return
                self.send_response(200)
                self.send_header('Content-Type', 'text/event-stream')
                self.end_headers()

                def chunk(delta, finish=None):
                    return {'id': 's', 'object': 'chat.completion.chunk', 'created': 1, 'model': body['model'],
                            'choices': [{'index': 0, 'delta': delta, 'finish_reason': finish}]}
                try:
                    self.wfile.write(('data: ' + json.dumps(chunk({'role': 'assistant'})) + '\n\n').encode())
                    self.wfile.flush()
                    deadline = time.monotonic() + 30
                    while (root / f'gate-{name}').exists() and time.monotonic() < deadline:
                        time.sleep(.02)
                    for item in (chunk({'content': 'REPLY_' + name}),
                                 dict(chunk({}, 'stop'), usage={'prompt_tokens': 1, 'completion_tokens': 1, 'total_tokens': 2})):
                        self.wfile.write(('data: ' + json.dumps(item) + '\n\n').encode())
                    self.wfile.write(b'data: [DONE]\n\n')
                    self.wfile.flush()
                except (BrokenPipeError, ConnectionResetError):
                    pass  # Cancellation closes the stream.

        server = ThreadingHTTPServer(('127.0.0.1', 0), Upstream)
        threading.Thread(target=server.serve_forever, daemon=True).start()

        env = {'HOME': str(root / 'home'), 'PATH': str(args.node.parent) + ':/usr/bin:/bin'}
        seed = root / 'seed.mjs'
        seed.write_text('import {SessionManager} from ' + json.dumps(sdk.resolve().as_uri()) + ''';
const [cwd,dir,title]=process.argv.slice(2); const manager=SessionManager.create(cwd,dir);
manager.appendMessage({role:'user',content:[{type:'text',text:title}],timestamp:Date.now()});
manager.appendMessage({role:'assistant',content:[{type:'text',text:'SEED '+title}],api:'openai-completions',provider:'synthetic',model:'synthetic',stopReason:'stop',usage:{input:1,output:1,cacheRead:0,cacheWrite:0,totalTokens:2,cost:{input:0,output:0,cacheRead:0,cacheWrite:0,total:0}},timestamp:Date.now()});
manager.appendSessionInfo(title); console.log(manager.getSessionFile());
''')
        subprocess.run([str(args.node), str(seed), str(root / 'project'), str(root / 'runtime/sessions/project'), 'A'],
                       env=env, check=True, capture_output=True)

        shutil.copy2(args.bindings / 'velune_bindings.py', root / 'bindings/velune_bindings.py')
        shutil.copy2(args.library, root / 'bindings' / args.library.name)
        spec = importlib.util.spec_from_file_location('velune_queue', root / 'bindings/velune_bindings.py')
        b = importlib.util.module_from_spec(spec)
        sys.modules[spec.name] = b
        spec.loader.exec_module(b)
        app = b.VeluneApplication.open(b.BindingOptions(home_directory=str(root / 'app'), resources_directory=str(root / 'resources')))
        try:
            runtime = b.BindingRuntimeInstance(enabled=True, id='pi', name='Pi', type_id='pi-1.0.2', gateway_id='default',
                                               settings={'binary': str(args.pi), 'nodeBinary': str(args.node), 'agentDir': str(root / 'runtime')})
            app.upsert_runtime(runtime)
            app.save_provider('default', b.BindingProviderDraft(
                id='synthetic', name='Synthetic', protocol=b.BindingGatewayProtocol.CHAT_COMPLETIONS_V1,
                endpoint=f'http://127.0.0.1:{server.server_port}/v1',
                models=[b.BindingProviderModel(record_key='', provider_model_id='synthetic', nickname='Synthetic', icon=None,
                                               context_window=8192, max_output_tokens=128, reasoning_levels=None, adapter_metadata_json=None)], quota_limits=[], execution_backend=b.BindingExecutionBackend.HTTP(), request_profile=b.BindingRequestProfile.NATIVE, authentication_source=b.BindingProviderAuthenticationSource.API_KEY, quota_source=b.BindingProviderQuotaSource.NONE),
                b.BindingAuthenticationEdit.SET_API_KEY(value='SYNTHETIC_ONLY'))
            model = app.list().gateways[0].providers[0].models[0].record_key
            a = app.list().conversations[0].conversation.id
            Q = b.BindingSendQueueOperation

            def snapshot():
                return app.snapshot('pi', a).snapshot

            def wait(predicate, label, timeout=30):
                deadline = time.monotonic() + timeout
                while time.monotonic() < deadline:
                    snapshot()  # The core advances its sessions when asked, as the app's polling does.
                    value = predicate()
                    if value:
                        return value
                    time.sleep(.05)
                raise AssertionError((label, snapshot().follow_up, requests))

            def running():
                return snapshot().run_state == b.BindingRunState.RUNNING

            def idle():
                current = snapshot()
                return current.run_state != b.BindingRunState.RUNNING and current.actions.can_send

            def queue(text):
                turn = b.BindingComposedTurn(text=text, effort=None, permission_mode=None, attachments=[])
                return app.send_queue('pi', a, Q.ADD(source_runtime_id='pi', target_runtime_id='pi',
                                                     selection=b.BindingModelSelection.MANAGED(model_record_key=model), turn=turn)).snapshot

            def queued():
                return [item.text for item in snapshot().follow_up.queue]

            def users():
                return [block.text for message in snapshot().messages if message.role == b.BindingMessageRole.USER
                        for block in message.blocks if isinstance(block, b.BindingMessageBlock.TEXT)]

            def release(name):
                (root / f'gate-{name}').unlink()

            for name in GATES:
                (root / f'gate-{name}').write_text('hold')
            app.open_conversation('pi', a)
            app.send_turn('pi', a, 'pi', model, 'TURN_A')
            wait(lambda: 'TURN_A' in requests, 'A reached the upstream')
            for text in ('Q_ONE', 'Q_TWO', 'Q_THREE'):
                queue(text)
            assert queued() == ['Q_ONE', 'Q_TWO', 'Q_THREE'] and running(), queued()
            ids = {item.text: item.id for item in snapshot().follow_up.queue}
            app.send_queue('pi', a, Q.EDIT(id=ids['Q_TWO'], text='Q_EDITED'))
            app.send_queue('pi', a, Q.MOVE(id=ids['Q_THREE'], index=0))
            app.send_queue('pi', a, Q.REMOVE(id=ids['Q_ONE']))
            assert queued() == ['Q_THREE', 'Q_EDITED'], queued()
            assert not snapshot().follow_up.can_continue and snapshot().follow_up.paused is None
            print('PASS messages queue while a turn runs and can be edited, reordered and removed', flush=True)

            release('TURN_A')
            wait(lambda: 'Q_THREE' in requests, 'the first queued message went out after the turn')
            assert queued() == ['Q_EDITED'], queued()
            assert 'Q_EDITED' not in requests, 'one queued message at a time'
            release('Q_THREE')
            wait(lambda: 'Q_EDITED' in requests, 'the next queued message went out')
            release('Q_EDITED')
            wait(idle, 'queue drained')
            assert queued() == [] and users()[-3:] == ['TURN_A', 'Q_THREE', 'Q_EDITED'], users()
            assert 'Q_ONE' not in requests and 'Q_TWO' not in requests
            print('PASS queued messages go out one by one, each as its own turn, in queue order', flush=True)

            app.send_turn('pi', a, 'pi', model, 'TURN_S')
            wait(lambda: 'TURN_S' in requests, 'S reached the upstream')
            queue('Q_PAUSED')
            app.cancel('pi', a)
            wait(idle, 'S stopped')
            follow = snapshot().follow_up
            assert follow.paused == b.BindingQueuePause.TURN_STOPPED and [m.text for m in follow.queue] == ['Q_PAUSED'], follow
            assert follow.can_continue, 'continuing is offered after a stop'
            time.sleep(.5)
            assert 'Q_PAUSED' not in requests, 'a stopped turn pauses the queue'
            app.send_queue('pi', a, Q.RESUME())
            wait(lambda: 'Q_PAUSED' in requests, 'resuming sent the queued message')
            assert snapshot().follow_up.paused is None and not snapshot().follow_up.can_continue
            release('Q_PAUSED')
            wait(idle, 'resumed turn ended')
            assert not snapshot().follow_up.can_continue, 'a normal end offers no continue'
            print('PASS a stopped turn pauses the queue and offers continuing; resuming sends', flush=True)

            app.send_turn('pi', a, 'pi', model, 'TURN_FAIL')
            queue('Q_FAILED')
            wait(lambda: snapshot().follow_up.paused == b.BindingQueuePause.TURN_FAILED, 'a failed turn pauses the queue')
            assert snapshot().follow_up.can_continue and queued() == ['Q_FAILED']
            app.send_queue('pi', a, Q.REMOVE(id=snapshot().follow_up.queue[0].id))
            assert queued() == [] and snapshot().follow_up.paused is None
            assert 'Q_FAILED' not in requests
            print('PASS a failed turn pauses the queue; emptying it clears the pause', flush=True)

            app.send_turn('pi', a, 'pi', model, '继续')
            wait(lambda: 'CONTINUE_GATE' in requests, 'continue reached the upstream as a message')
            release('CONTINUE_GATE')
            wait(idle, 'continue ended')
            assert users()[-1] == '继续' and not snapshot().follow_up.can_continue
            queue('Q_IDLE')
            wait(lambda: 'Q_IDLE' in requests, 'a message queued while idle goes out at once')
            release('Q_IDLE')
            wait(idle, 'idle turn ended')
            assert queued() == []
            print('PASS continuing sends a real user message; queueing while idle sends at once', flush=True)
        finally:
            for gate in root.glob('gate-*'):
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
