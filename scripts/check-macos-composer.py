#!/usr/bin/env python3
"""Run production native composer checks in an owned background-only copy."""
import argparse
import hashlib
import json
import os
from pathlib import Path
import plistlib
import shutil
import signal
import subprocess
import tempfile
import threading
import time
import uuid
from contextlib import ExitStack
from http.server import BaseHTTPRequestHandler, ThreadingHTTPServer
from macos_identity import ROOT, app_bundle_path, validate_bundle


def digest(path):
    with path.open('rb') as stream:
        return hashlib.file_digest(stream, 'sha256').hexdigest()


def prepare_claude_continuation(root, evidence, path, lifetime, editing):
    """Tiny native history where an earlier reply is outside the replay chain."""
    binary = shutil.which('claude', path=path)
    if not binary:
        raise SystemExit('Claude continuation checks require Claude Code on PATH')
    config = root / 'claude'
    project = (root / 'project').resolve()
    project.mkdir()
    source = str(uuid.uuid4())
    directory = config / 'projects' / (''.join(c if c.isascii() and c.isalnum() else '-' for c in str(project)))
    directory.mkdir(parents=True)
    rows = []

    def record(role, parent, **fields):
        identity = str(uuid.uuid4())
        rows.append(dict(type=role, uuid=identity, parentUuid=parent, isSidechain=False,
                         sessionId=source, cwd=str(project), timestamp='2026-11-01T00:00:00.000Z',
                         version='2.1.327', userType='external', **fields))
        return identity

    def message(role, parent, value, **fields):
        body = dict(id='msg_' + uuid.uuid4().hex, role=role, content=[dict(type='text', text=value)])
        if role == 'assistant':
            body.update(type='message', model='claude-sonnet-4-6', stop_reason='end_turn',
                        usage=dict(input_tokens=10, output_tokens=5))
        return record(role, parent, message=body, **fields)

    selected = message('user', None, 'oldest fork prompt')
    old_reply = message('assistant', selected, 'oldest fork reply')
    retained_parent = old_reply
    if editing:
        selected = message('user', old_reply, 'old edit prompt')
        retained_parent = message('assistant', selected, 'old edit reply')
    retained = message('user', retained_parent, 'retained later prompt')
    retained_reply = message('assistant', retained, 'retained later reply')
    compact = record('system', None, subtype='compact_boundary', logicalParentUuid=retained_reply,
                     compactMetadata=dict(trigger='manual', preTokens=300, postTokens=30))
    summary = message('user', compact, 'new compact summary', isCompactSummary=True)
    keep = [retained, retained_reply]
    rows[-2]['compactMetadata'].update(preservedMessages=dict(anchorUuid=summary, uuids=keep, allUuids=keep),
                                       preservedSegment=dict(anchorUuid=summary, headUuid=retained, tailUuid=retained_reply))
    latest = message('user', summary, 'later prompt after compact')
    message('assistant', latest, 'later reply after compact')
    archive = directory / (source + '.jsonl')
    archive.write_text(''.join(json.dumps(row) + '\n' for row in rows))
    archive_digest = digest(archive)
    requests = []
    roles = []
    rejected = []
    prompt = 'edited historical message' if editing else 'first message after fork'
    reply = 'edited reply received' if editing else 'fork reply received'
    markers = ['oldest fork prompt', 'oldest fork reply', 'old edit prompt', 'old edit reply',
               'new compact summary', 'retained later prompt', 'retained later reply',
               'later prompt after compact', 'later reply after compact', prompt]

    class Messages(BaseHTTPRequestHandler):
        def log_message(self, *_):
            pass

        def do_POST(self):
            request = json.loads(self.rfile.read(int(self.headers['Content-Length'])))
            if not self.path.split('?')[0].endswith('/messages'):
                rejected.append(self.path.split('?')[0])
                self.send_error(400)
                return
            captured = []
            for message in request['messages']:
                content = json.dumps(message['content'])
                captured.extend((message['role'], marker) for marker in markers if marker in content)
            requests.append([marker for _, marker in captured])
            roles.append([role for role, _ in captured])
            self.send_response(200)
            self.send_header('Content-Type', 'text/event-stream')
            self.end_headers()
            events = [
                ('message_start', dict(message=dict(id='msg_' + uuid.uuid4().hex, type='message', role='assistant',
                    model=request['model'], content=[], stop_reason=None, usage=dict(input_tokens=10, output_tokens=1)))),
                ('content_block_start', dict(index=0, content_block=dict(type='text', text=''))),
                ('content_block_delta', dict(index=0, delta=dict(type='text_delta', text=reply))),
                ('content_block_stop', dict(index=0)),
                ('message_delta', dict(delta=dict(stop_reason='end_turn', stop_sequence=None), usage=dict(output_tokens=5))),
                ('message_stop', {}),
            ]
            for kind, fields in events:
                self.wfile.write(f'event: {kind}\ndata: {json.dumps(dict(type=kind, **fields))}\n\n'.encode())
            self.wfile.flush()

    server = ThreadingHTTPServer(('127.0.0.1', 0), Messages)
    threading.Thread(target=server.serve_forever, daemon=True).start()
    lifetime.callback(server.server_close)
    lifetime.callback(server.shutdown)
    (evidence / 'fixture.json').write_text(json.dumps(dict(
        binary=binary, agentDirectory=str(config), cwd=str(project), sourceID='claude-check:' + source,
        userID=selected, endpoint=f'http://127.0.0.1:{server.server_port}/v1')))

    def verify(output, success):
        intact = digest(archive) == archive_digest
        (output / 'loopback.json').write_text(json.dumps(dict(originalUnchanged=intact, requests=requests,
                                                            roles=roles, rejectedPaths=rejected), indent=2))
        if success:
            assert intact, 'continuation changed the original native archive'
            assert requests == [['oldest fork prompt', 'oldest fork reply', prompt]], requests
            assert roles == [['user', 'assistant', 'user']], roles

    return verify


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('check', choices=['editor', 'runtime-editor', 'model-auto-compaction', 'fork', 'fork-send', 'claude-compact-edit', 'queued-editor', 'queue-withdrawal', 'archived', 'palette-ime', 'palette-search', 'pagination', 'continuation', 'targets', 'tabs', 'ownership', 'geometry', 'controls', 'background', 'experience', 'appearance-zoom', 'aggregated-list', 'recovery', 'compaction', 'sidebar', 'provider-analytics', 'background-notifications', 'notification-navigation', 'sidebar-toolbar', 'outgoing-submissions', 'late-send-echo', 'transcript', 'shell-motion', 'telemetry-shutdown', 'file-links', 'requests', 'interactions', 'drag-provider', 'typography', 'admission', 'navigation', 'group-creation'])
    parser.add_argument('--app', type=Path, default=app_bundle_path(ROOT, 'dev'))
    parser.add_argument('--output', type=Path)
    parser.add_argument('--dark', action='store_true')
    parser.add_argument('--child-activity', action='store_true', help='check only the child conversation activity row')
    parser.add_argument('--child-open', action='store_true', help='check targeted child history on the headless request harness')
    args = parser.parse_args()
    if args.child_activity and args.check != 'background-notifications':
        parser.error('--child-activity requires background-notifications')
    if args.child_open and args.check != 'ownership':
        parser.error('--child-open requires ownership')
    app = args.app.resolve()
    context = validate_bundle(app, purpose='development')
    output = (args.output or ROOT / 'target/checks' / ('macos-' + args.check + ('-dark' if args.dark else ''))).resolve()
    output.mkdir(parents=True, exist_ok=True)
    original = app / 'Contents/MacOS' / context['executable']
    receipt = {'check': args.check, 'sourceApp': str(app), 'executableSHA256': digest(original),
               'realModelCalls': 0, 'backgroundOnly': True, 'startedAt': time.time()}
    library = app / 'Contents/Frameworks/libvelune_bindings.dylib'
    if library.exists():
        receipt['bindingsSHA256'] = digest(library)
    with tempfile.TemporaryDirectory(prefix='velune-native-composer-', dir='/tmp') as temporary, ExitStack() as lifetime:
        root = Path(temporary)
        copied = root / 'Check.app'
        copy_flag = '-cR' if app.stat().st_dev == root.stat().st_dev else '-R'
        subprocess.run(['/bin/cp', copy_flag, str(app), str(copied)], check=True)
        info_path = copied / 'Contents/Info.plist'
        info = plistlib.loads(info_path.read_bytes())
        info['LSBackgroundOnly'] = True
        info_path.write_bytes(plistlib.dumps(info))
        subprocess.run(['/usr/bin/codesign', '--force', '--sign', '-', str(copied)], check=True, stdout=subprocess.DEVNULL)
        validate_bundle(copied, purpose='development')
        private_home = root / 'user'; private_home.mkdir(mode=0o700)
        private_tmp = root / 'tmp'; private_tmp.mkdir(mode=0o700)
        env = {key: os.environ[key] for key in ['PATH', 'LANG', 'LC_ALL', 'TMPDIR'] if key in os.environ}
        evidence = root / 'evidence'; evidence.mkdir(mode=0o700)
        env.update(HOME=str(private_home), TMPDIR=str(private_tmp) + '/', VELUNE_HOME=str(root / 'state'), VELUNE_CHECK_OUTPUT=str(evidence),
                   VELUNE_CONTINUATION_CHECK_HOME=str(root / 'continuation'), VELUNE_COMPOSER_CHECK_HOME=str(root / 'targets'),
                   VELUNE_EXPERIENCE_CHECK_HOME=str(evidence),
                   CODEX_HOME=str(root / 'codex'), PI_CODING_AGENT_DIR=str(root / 'pi'), CLAUDE_CONFIG_DIR=str(root / 'claude'),
                   XDG_CONFIG_HOME=str(root / 'config'), XDG_DATA_HOME=str(root / 'data'), XDG_CACHE_HOME=str(root / 'cache'))
        if args.check == 'editor':
            env['VELUNE_HOME'] = str(evidence / 'state')
        flags = ['--check-continuation-preparation'] if args.check == 'continuation' else ['--check-composer-ownership', '--composer-check-content-target']
        named_flags = {
            'requests': ['--check-conversation-requests'],
            'interactions': ['--check-runtime-interactions'],
            'drag-provider': ['--check-message-experience', '--experience-drag-provider-only'],
            'typography': ['--check-message-experience', '--experience-typography-only'],
            'admission': ['--check-message-experience', '--experience-admission-only'],
            'navigation': ['--check-message-experience', '--experience-navigation-only'],
            'group-creation': ['--check-group-creation'],
        }
        if args.check in named_flags:
            flags = named_flags[args.check]
            if args.check == 'interactions' and args.dark:
                flags.append('--interaction-check-dark')
        if args.check == 'editor':
            flags = ['--check-message-experience', '--experience-message-editor-offscreen']
            if args.dark:
                flags.append('--editor-check-dark')
        if args.check == 'queued-editor':
            flags = ['--check-queued-message-editor']
            if args.dark:
                flags.append('--queued-editor-dark')
        if args.check == 'runtime-editor':
            flags = ['--check-message-experience', '--experience-runtime-editor-only']
        if args.check == 'model-auto-compaction':
            flags = ['--check-message-experience', '--experience-model-auto-compaction-only']
        if args.check == 'fork':
            flags = ['--check-message-experience', '--experience-fork-only']
        if args.check == 'archived':
            flags = ['--check-archived-conversations']
            if args.dark:
                flags.append('--archived-check-dark')
        if args.check == 'palette-ime':
            flags = ['--check-palette-ime']
        if args.check == 'palette-search':
            flags = ['--check-palette-search']
        if args.check == 'pagination':
            flags = ['--check-list-pagination']
        if args.check == 'tabs':
            flags = ['--check-composer-ownership', '--composer-check-tab-state']
        if args.check == 'ownership':
            flags = ['--check-composer-ownership']
        if args.check == 'controls':
            flags = ['--check-composer-controls']
        if args.check == 'geometry':
            flags = ['--check-composer-controls', '--check-geometry']
        if args.check == 'background':
            flags = ['--check-background-tasks']
            if args.dark:
                flags.append('--background-check-dark')
        if args.check == 'experience':
            flags = ['--check-message-experience']
        if args.check == 'appearance-zoom':
            flags = ['--check-message-experience', '--experience-appearance-zoom-only']
            if args.dark:
                flags.append('--appearance-zoom-dark')
        if args.check in ['fork-send', 'claude-compact-edit']:
            editing = args.check == 'claude-compact-edit'
            flag = '--experience-claude-compact-edit-only' if editing else '--experience-fork-send-only'
            flags = ['--check-message-experience', flag]
            verify_claude = prepare_claude_continuation(root, evidence, env.get('PATH', os.defpath), lifetime, editing)
            env.update(CLAUDE_CODE_DISABLE_NONESSENTIAL_TRAFFIC='1', DISABLE_AUTOUPDATER='1',
                       HTTP_PROXY='http://127.0.0.1:9', HTTPS_PROXY='http://127.0.0.1:9', NO_PROXY='127.0.0.1,localhost')
        if args.check == 'recovery':
            flags = ['--check-message-experience', '--experience-runtime-recovery-only']
        if args.check == 'compaction':
            flags = ['--check-message-experience', '--experience-compaction-only']
        if args.check == 'background-notifications':
            flags = ['--check-background-notifications']
        if args.check == 'notification-navigation':
            flags = ['--check-notification-navigation']
        if args.child_open:
            flags = ['--check-conversation-requests', '--check-child-open']
        if args.check == 'aggregated-list':
            flags = ['--check-aggregated-list']
        if args.check == 'provider-analytics':
            flags = ['--check-provider-analytics']
        if args.check == 'outgoing-submissions':
            flags = ['--check-outgoing-submissions']
        if args.check == 'late-send-echo':
            flags = ['--check-conversation-requests', '--check-late-send-echo']
        if args.check == 'queue-withdrawal':
            flags = ['--check-conversation-requests', '--check-queue-withdrawal']
        if args.check == 'sidebar-toolbar':
            flags = ['--check-sidebar-toolbar']
        if args.check == 'telemetry-shutdown':
            flags = ['--check-telemetry-shutdown']
        if args.check == 'transcript':
            flags = ['--check-message-experience', '--experience-native-row-order-only']
        elif args.check == 'shell-motion':
            flags = ['--check-shell-motion']
        if args.check == 'file-links':
            flags = ['--check-message-experience', '--experience-file-links-only']
        if args.check == 'sidebar':
            flags = ['--check-message-experience', '--experience-sidebar-only']
        if args.dark and args.check == 'continuation':
            flags.append('--continuation-check-dark')
        if args.child_activity:
            flags.append('--check-child-activity')
            if args.dark:
                flags.append('--child-activity-dark')
        phases = [('initial', flags, 'native.log')]
        if flags == ['--check-composer-ownership']:
            phases.append(('reopen', ['--check-composer-ownership', '--composer-check-reopen'], 'reopen.log'))
        elif flags == ['--check-message-experience']:
            phases.append(('reopen', ['--check-message-experience', '--experience-reopen'], 'reopen.log'))
        receipt['phases'] = []
        # Retain the same private home across two independent processes. The
        # second phase starts only after the writer has exited successfully.
        for name, phase_flags, log_name in phases:
            phase = {'name': name, 'arguments': phase_flags, 'log': log_name, 'startedAt': time.time()}
            receipt['phases'].append(phase)
            with (output / log_name).open('w') as log:
                child = subprocess.Popen([str(copied / 'Contents/MacOS' / context['executable']), *phase_flags], env=env,
                                         cwd=ROOT, stdout=log, stderr=subprocess.STDOUT, start_new_session=True)
                phase['pid'] = child.pid
                if name == 'initial':
                    receipt['pid'] = child.pid
                try:
                    phase['exitCode'] = child.wait(timeout=180)
                    receipt['exitCode'] = phase['exitCode']
                except BaseException as error:
                    os.killpg(child.pid, signal.SIGTERM)
                    try:
                        child.wait(timeout=5)
                    except subprocess.TimeoutExpired:
                        os.killpg(child.pid, signal.SIGKILL); child.wait()
                    phase['exitCode'] = child.returncode
                    phase['error'] = type(error).__name__
                    receipt['exitCode'] = phase['exitCode']
                    raise
                finally:
                    phase['finishedAt'] = time.time()
                    receipt['finishedAt'] = phase['finishedAt']
                    (output / 'receipt.json').write_text(json.dumps(receipt, indent=2) + '\n')
            if receipt['exitCode'] != 0:
                break
        # Copy only explicit result artifacts; application state stays private.
        exported = []
        for result in evidence.iterdir():
            if result.is_file() and (result.name in ['acceptance.json', 'render-measurements.json', 'editor-wire.jsonl', 'editor-native.json', 'fixture.json', 'archived-check.json', 'continuation-check.json', 'background-check.json', 'sidebar-check.json', 'provider-analytics-check.json', 'background-notifications-check.json', 'sidebar-toolbar-check.json', 'outgoing-submissions-check.json', 'recovery-check.json'] or result.suffix in ['.png', '.csv', '.txt']):
                shutil.copy2(result, output / result.name)
                exported.append(result.name)
        if args.check == 'editor':
            for native_log in (evidence / 'state' / 'logs').glob('*.jsonl'):
                name = 'editor-core-' + native_log.name
                shutil.copy2(native_log, output / name)
                exported.append(name)
        if args.check in ['fork-send', 'claude-compact-edit']:
            verify_claude(output, receipt['exitCode'] == 0)
            exported.append('loopback.json')
            prefix = 'claude-compact-edit' if args.check == 'claude-compact-edit' else 'fork'
            for native_log in (root / 'state' / 'logs').glob('*.jsonl'):
                name = prefix + '-core-' + native_log.name
                shutil.copy2(native_log, output / name)
                exported.append(name)
        receipt['artifacts'] = sorted(exported)
        if args.check in ['fork-send', 'claude-compact-edit'] and receipt['exitCode'] == 0:
            receipt['loopbackRequests'] = 1
        (output / 'receipt.json').write_text(json.dumps(receipt, indent=2) + '\n')
        for phase in receipt['phases']:
            print((output / phase['log']).read_text(), end='')
        print('Evidence:', output)
        return receipt['exitCode']


if __name__ == '__main__':
    raise SystemExit(main())
