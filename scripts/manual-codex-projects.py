#!/usr/bin/env python3
"""Verify native Codex project reads/edits and restored workspace roots using synthetic data.

Uses a temporary HOME, the real Codex app-server, and a loopback Responses fixture. No credentials, real projects or external model requests.
"""
import argparse
import importlib.util
import json
import os
from pathlib import Path
import queue
import shutil
import subprocess
import sys
import tempfile
import threading
import time
from http.server import BaseHTTPRequestHandler, ThreadingHTTPServer


def load(name, path):
    spec = importlib.util.spec_from_file_location(name, path)
    module = importlib.util.module_from_spec(spec)
    sys.modules[name] = module
    spec.loader.exec_module(module)
    return module


class Upstream(BaseHTTPRequestHandler):
    def log_message(self, *_):
        pass

    def do_POST(self):
        self.rfile.read(int(self.headers.get('Content-Length', '0')))
        ident = str(time.time_ns())
        item = {'id': 'item-' + ident, 'type': 'message', 'role': 'assistant', 'status': 'completed',
                'content': [{'type': 'output_text', 'text': 'synthetic answer', 'annotations': []}]}
        response = {'id': 'response-' + ident, 'object': 'response', 'status': 'completed', 'output': [item]}
        events = [
            {'type': 'response.created', 'response': {**response, 'status': 'in_progress', 'output': []}},
            {'type': 'response.output_item.added', 'output_index': 0, 'item': item},
            {'type': 'response.output_item.done', 'output_index': 0, 'item': item},
            {'type': 'response.completed', 'response': response},
        ]
        payload = b''.join(('event: %s\ndata: %s\n\n' % (event['type'], json.dumps(event))).encode() for event in events)
        self.send_response(200)
        self.send_header('Content-Type', 'text/event-stream')
        self.send_header('Content-Length', str(len(payload)))
        self.end_headers()
        self.wfile.write(payload)


class Rpc:
    def __init__(self, binary, env):
        self.process = subprocess.Popen([str(binary), 'app-server', '--stdio'], env=env,
            stdin=subprocess.PIPE, stdout=subprocess.PIPE, stderr=subprocess.DEVNULL, text=True)
        self.messages = queue.Queue()
        self.ident = 0
        def read():
            for line in self.process.stdout:
                self.messages.put(json.loads(line))
        threading.Thread(target=read, daemon=True).start()
        self.call('initialize', {'clientInfo': {'name': 'velune-project-check', 'version': '0'},
            'capabilities': {'experimentalApi': True}})

    def wait(self, predicate):
        deadline = time.monotonic() + 30
        while time.monotonic() < deadline:
            message = self.messages.get(timeout=max(0.1, deadline - time.monotonic()))
            if predicate(message):
                return message
        raise TimeoutError('Codex response')

    def call(self, method, params):
        self.ident += 1
        self.process.stdin.write(json.dumps({'jsonrpc': '2.0', 'id': self.ident, 'method': method, 'params': params}) + '\n')
        self.process.stdin.flush()
        result = self.wait(lambda m: m.get('id') == self.ident)
        if 'error' in result:
            raise RuntimeError((method, result['error']))
        return result['result']

    def stop(self):
        self.process.terminate()
        self.process.wait(timeout=10)


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    for name in ('codex', 'node', 'bindings', 'library', 'resources'):
        parser.add_argument('--' + name, type=Path, required=True)
    parser.add_argument("--seed", type=Path, help="Keep a new synthetic fixture directory for UI checks")
    args = parser.parse_args()
    server = ThreadingHTTPServer(('127.0.0.1', 0), Upstream)
    threading.Thread(target=server.serve_forever, daemon=True).start()
    with tempfile.TemporaryDirectory(prefix='velune-native-projects-') as temp:
        root = args.seed.resolve() if args.seed else Path(temp).resolve()
        if args.seed:
            root.mkdir(parents=True, exist_ok=False)
        for folder in ('codex', 'one', 'two', 'outside', 'bindings'):
            (root / folder).mkdir()
        env = {'HOME': str(root), 'CODEX_HOME': str(root / 'codex'), 'PATH': '/opt/homebrew/bin:/usr/bin:/bin',
               'SYNTHETIC_KEY': 'synthetic-only', 'TMPDIR': str(root)}
        provider = {'model_provider': 'synthetic', 'model_providers': {'synthetic': {
            'name': 'synthetic', 'base_url': f'http://127.0.0.1:{server.server_port}/v1',
            'env_key': 'SYNTHETIC_KEY', 'wire_api': 'responses', 'requires_openai_auth': False}},
            'features': {'responses_websockets': False, 'responses_websockets_v2': False}}
        # Native continuation must keep using the synthetic provider after the seed process exits.
        (root / 'codex/config.toml').write_text(f'''model = "gpt-6.1-sol"
model_provider = "synthetic"
[model_providers.synthetic]
name = "synthetic"
base_url = "http://127.0.0.1:{server.server_port}/v1"
env_key = "SYNTHETIC_KEY"
wire_api = "responses"
requires_openai_auth = false
[features]
responses_websockets = false
responses_websockets_v2 = false
''')
        rpc = Rpc(args.codex, env)
        ids = []
        roots = [str(root / 'one'), str(root / 'two')]
        try:
            projects = [rpc.call('project/create', {'idempotencyKey': name, 'name': name, 'metadata': {'synthetic-marker': 'preserved'}, 'roots': [{'path': path} for path in paths]})['project']
                        for name, paths in [('Alpha', roots), ('Beta', roots[:1]), ('Empty', [])]]
            for project, cwd in zip(projects[:2], [root / 'two', root / 'outside']):
                thread = rpc.call('thread/start', {'cwd': str(cwd), 'projectId': project['id'],
                    'runtimeWorkspaceRoots': roots, 'modelProvider': 'synthetic', 'config': provider,
                    'approvalPolicy': 'never', 'sandbox': 'workspace-write'})['thread']
                ids.append(thread['id'])
                rpc.call('turn/start', {'threadId': thread['id'], 'model': 'gpt-6.1-sol',
                    'input': [{'type': 'text', 'text': 'synthetic project seed'}]})
                rpc.wait(lambda m: m.get('method') == 'turn/completed')
        finally:
            rpc.stop()
        shutil.copy(args.bindings / 'velune_bindings.py', root / 'bindings')
        (root / 'bindings/libvelune_bindings.dylib').symlink_to(args.library)
        b = load('velune_project_bindings', root / 'bindings/velune_bindings.py')
        os.environ.clear()
        os.environ.update(env)
        app = b.VeluneApplication.open(b.BindingOptions(home_directory=str(root / 'velune'), resources_directory=str(args.resources)))
        try:
            app.upsert_runtime(b.BindingRuntimeInstance(enabled=True, id='codex', name='Synthetic Codex',
                type_id='codex-0.159', gateway_id='default', settings={'binary': str(args.codex), 'nodeBinary': str(args.node), 'agentDir': str(root / 'codex')}))
            listing = app.list()
            native = {p.name: p for p in listing.conversation_projects if p.native_runtime_id == 'codex'}
            assert set(native) == {'Alpha', 'Beta', 'Empty'}, native
            assert native['Alpha'].path != native['Beta'].path
            assert native['Alpha'].primary_folder == native['Beta'].primary_folder == roots[0]
            assert native['Alpha'].folders == roots[1:]
            assert native['Empty'].primary_folder is None and not native['Empty'].folders
            rows = {r.native_id: r for r in listing.conversations}
            assert rows[ids[0]].project == native['Alpha'].path
            assert rows[ids[1]].project == native['Beta'].path
            print('PASS shared roots, second-root/outside cwd assignment, zero-root project', flush=True)
            app.set_project(native['Alpha'].path, roots[1], 'Renamed', [str(root / 'outside')])
            app.set_project(native['Empty'].path, '', 'Still empty', [])
            refreshed = app.list()
            renamed = next(p for p in refreshed.conversation_projects if p.name == 'Renamed')
            assert renamed.path == native['Alpha'].path and renamed.primary_folder == roots[1]
            assert renamed.folders == [str(root / 'outside')]
            print('PASS native project edits retain stable identity', flush=True)
            # A Velune move remains organization-only and does not mutate native assignment.
            app.organize_conversations([b.BindingConversationReference(runtime_id='codex', conversation_id=rows[ids[0]].conversation.id)],
                b.BindingConversationOrganizationChange.MOVE_TO_PROJECT(path=native['Beta'].path))
            assert next(r for r in app.list().conversations if r.native_id == ids[0]).project == native['Beta'].path
            app.open_conversation('codex', rows[ids[0]].conversation.id)
            app.send_turn_with_selection('codex', rows[ids[0]].conversation.id, 'codex',
                b.BindingModelSelection.NATIVE(provider_id='synthetic', model_id='gpt-6.1-sol'),
                b.BindingComposedTurn(text='synthetic continuation', effort=None, permission_mode='workspace', attachments=[]))
            deadline = time.monotonic() + 30
            while time.monotonic() < deadline:
                status = app.snapshot('codex', rows[ids[0]].conversation.id).snapshot
                if status and not status.actions.can_cancel:
                    break
                time.sleep(0.1)
            else:
                raise TimeoutError('Velune turn')
            archived_ref = b.BindingConversationReference(runtime_id='codex', conversation_id=rows[ids[1]].conversation.id)
            app.organize_conversations([archived_ref], b.BindingConversationOrganizationChange.ARCHIVE())
            archived = app.show_archived_conversations(True)
            assert next(r for r in archived.conversations if r.native_id == ids[1]).project == native['Beta'].path
            app.organize_conversations([archived_ref], b.BindingConversationOrganizationChange.UNARCHIVE())
            restored = app.show_archived_conversations(False)
            assert next(r for r in restored.conversations if r.native_id == ids[1]).project == native['Beta'].path
            print('PASS archive/unarchive retain native project membership', flush=True)
            app.upsert_runtime(b.BindingRuntimeInstance(enabled=True, id='codex-other', name='Second synthetic instance',
                type_id='codex-0.159', gateway_id='default', settings={'binary': str(args.codex), 'nodeBinary': str(args.node), 'agentDir': str(root / 'codex')}))
            all_native = [p for p in app.list().conversation_projects if p.native_runtime_id is not None]
            assert len(all_native) == 6 and len({p.path for p in all_native}) == 6
            print('PASS identical native project IDs from separate runtime instances stay distinct', flush=True)
        finally:
            app.shutdown()
        rpc = Rpc(args.codex, env)
        try:
            project = rpc.call('project/read', {'projectId': projects[0]['id']})['project']
            assert project['metadata'] == {'synthetic-marker': 'preserved'}
            assert project['name'] == 'Renamed' and [r['path'] for r in project['roots']] == [roots[1], str(root / 'outside')]
            resumed = rpc.call('thread/resume', {'threadId': ids[0], 'excludeTurns': True})
            assert resumed['thread']['projectId'] == projects[0]['id']
            assert resumed['thread']['cwd'] == roots[1]
            assert resumed['runtimeWorkspaceRoots'] == roots, resumed.get('runtimeWorkspaceRoots')
            print('PASS Velune continuation and organization preserve native assignment, cwd, and original workspace roots', flush=True)
        finally:
            rpc.stop()
    server.shutdown()


if __name__ == '__main__':
    main()
