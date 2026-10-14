#!/usr/bin/env python3
"""Manual acceptance for conversations executing in parallel.

Two synthetic Pi 1.0.2 conversations in a temporary HOME run turns at the same
time through the UniFFI application against a loopback upstream that holds
each turn until released. Checks independent progress, active-conversation
reporting, private execution lanes, cancellation of one conversation, reopening
a running conversation, the configuration guard and reported turn outcomes. No real credential,
provider, configuration or session is read; this is not an automated test.
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


GATES = ('TURN_A', 'TURN_B', 'TURN_C', 'TURN_D', 'TURN_FAIL')


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

    with tempfile.TemporaryDirectory(prefix='velune-parallel-') as directory:
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
                last = json.dumps(users[-1]['content']) if users else ''
                name = next((n for n in GATES if n in last), 'OTHER')
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
        for title in ('A', 'B'):
            subprocess.run([str(args.node), str(seed), str(root / 'project'), str(root / 'runtime/sessions/project'), title],
                           env=env, check=True, capture_output=True)
            time.sleep(.01)

        shutil.copy2(args.bindings / 'velune_bindings.py', root / 'bindings/velune_bindings.py')
        shutil.copy2(args.library, root / 'bindings' / args.library.name)
        spec = importlib.util.spec_from_file_location('velune_parallel', root / 'bindings/velune_bindings.py')
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
                                               context_window=8192, max_output_tokens=128, reasoning_levels=None, adapter_metadata_json=None)]),
                b.BindingAuthenticationEdit.SET_API_KEY(value='SYNTHETIC_ONLY'))
            model = app.list().gateways[0].providers[0].models[0].record_key
            ids = {c.title: c.id for c in [_row.conversation for _row in app.list().conversations]}
            a, b_id = ids['A'], ids['B']

            def snapshot(conversation):
                return app.snapshot('pi', conversation).snapshot

            def wait(predicate, label, timeout=30):
                deadline = time.monotonic() + timeout
                while time.monotonic() < deadline:
                    value = predicate()
                    if value:
                        return value
                    time.sleep(.05)
                raise AssertionError(label)

            def running(conversation):
                current = snapshot(conversation)
                return current is not None and current.run_state == b.BindingRunState.RUNNING

            def replied(conversation, text):
                current = snapshot(conversation)
                return current is not None and current.actions.can_send and any(
                    isinstance(block, b.BindingMessageBlock.TEXT) and block.text == text
                    for message in current.messages for block in message.blocks)

            for name in GATES:
                (root / f'gate-{name}').write_text('hold')
            app.open_conversation('pi', a)
            app.send_turn('pi', a, 'pi', model, 'TURN_A')
            wait(lambda: 'TURN_A' in requests, 'A reached the upstream')
            app.open_conversation('pi', b_id)
            app.send_turn('pi', b_id, 'pi', model, 'TURN_B')
            wait(lambda: 'TURN_B' in requests, 'B reached the upstream while A runs')
            assert running(a) and running(b_id), 'both conversations run at once'
            active = {item.conversation_id: item for item in app.conversation_activity(0).active}
            assert set(active) == {a, b_id} and all(item.run_state == b.BindingRunState.RUNNING for item in active.values()), active
            lanes = sorted(path.parent.name for path in (root / 'app/runtime-projections').glob('*/lane-*/velune-selection.json'))
            assert lanes == ['lane-0', 'lane-1'], lanes
            print('PASS two conversations of one runtime run at once, each in its own execution lane', flush=True)

            reopened = app.open_conversation('pi', a).snapshot
            assert reopened.conversation.id == a and reopened.run_state == b.BindingRunState.RUNNING
            try:
                app.upsert_runtime(runtime)
                raise AssertionError('runtime configuration changed during execution')
            except b.BindingError:
                pass
            print('PASS reopening a running conversation shows it running; configuration stays locked meanwhile', flush=True)

            (root / 'gate-TURN_B').unlink()
            wait(lambda: replied(b_id, 'REPLY_TURN_B'), 'B finished')
            assert running(a), 'A keeps running after B finished'
            assert [item.conversation_id for item in app.conversation_activity(0).active] == [a]
            (root / 'gate-TURN_A').unlink()
            wait(lambda: replied(a, 'REPLY_TURN_A'), 'A finished')
            assert app.conversation_activity(0).active == []
            print('PASS each conversation finished independently with its own reply', flush=True)

            app.send_turn('pi', a, 'pi', model, 'TURN_C')
            app.send_turn('pi', b_id, 'pi', model, 'TURN_D')
            wait(lambda: running(a) and running(b_id), 'both run again')
            app.cancel('pi', a)
            wait(lambda: snapshot(a).actions.can_send, 'A cancelled')
            assert running(b_id), 'cancelling A leaves B running'
            (root / 'gate-TURN_D').unlink()
            wait(lambda: replied(b_id, 'REPLY_TURN_D'), 'B finished after A was cancelled')
            print('PASS cancelling one conversation leaves the other running', flush=True)

            outcomes = [(turn.conversation_id, turn.outcome) for turn in app.conversation_activity(0).finished]
            assert outcomes == [(b_id, b.BindingTurnOutcome.COMPLETED), (a, b.BindingTurnOutcome.COMPLETED),
                                (a, b.BindingTurnOutcome.CANCELLED), (b_id, b.BindingTurnOutcome.COMPLETED)], outcomes
            cursor = app.conversation_activity(0).cursor
            app.send_turn('pi', a, 'pi', model, 'TURN_FAIL')
            wait(lambda: app.conversation_activity(cursor).finished, 'failed turn ended')
            finished = app.conversation_activity(cursor).finished
            assert [(turn.conversation_id, turn.outcome) for turn in finished] == [(a, b.BindingTurnOutcome.FAILED)], finished
            assert app.conversation_activity(finished[-1].sequence).finished == []
            print('PASS ended turns are reported once per cursor as completed, cancelled or failed', flush=True)
            app.upsert_runtime(runtime)
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
