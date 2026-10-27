#!/usr/bin/env python3
"""Verify manual native compaction using isolated HOMEs and a synthetic loopback model.

Run explicitly with a matching library, generated Python bindings, runtime resources,
Node, and a supported Codex, Claude Code or Pi CLI. No real accounts or sessions.
"""
import argparse
import hashlib
from contextlib import nullcontext
import importlib.util
import json
import os
from pathlib import Path
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
    for name in ('library', 'bindings', 'resources', 'node', 'binary'):
        parser.add_argument('--' + name, type=Path, required=True)
    parser.add_argument('--runtime', choices=('codex', 'claude', 'pi'), required=True)
    parser.add_argument('--disconnect', action='store_true', help='On POSIX, terminate only this isolated runner\'s runtime child to verify disconnect handling')
    parser.add_argument("--keep", type=Path, help="Keep a new isolated synthetic workspace for diagnosis or native UI inspection")
    args = parser.parse_args()
    if args.disconnect and os.name != 'posix':
        parser.error('--disconnect requires POSIX process inspection')
    for name in ("library", "bindings", "resources", "node", "binary"):
        setattr(args, name, getattr(args, name).resolve())
    captures = []
    control = {'mode': 'reply'}
    held, release = threading.Event(), threading.Event()

    class Upstream(BaseHTTPRequestHandler):
        def log_message(self, *_):
            pass

        def data(self, status, value):
            body = json.dumps(value).encode()
            self.send_response(status)
            self.send_header('Content-Type', 'application/json')
            self.send_header('Content-Length', str(len(body)))
            self.end_headers()
            self.wfile.write(body)

        def do_POST(self):
            body = json.loads(self.rfile.read(int(self.headers['Content-Length'])))
            if self.path.endswith('/count_tokens'):
                return self.data(200, {'input_tokens': 2000})
            captures.append(body)
            mode = control['mode']
            if mode == 'hold':
                held.set()
                release.wait(40)
            if mode == 'fail':
                return self.data(400, {'error': {'type': 'invalid_request_error', 'message': 'SYNTHETIC_COMPACTION_REJECTED'}})
            text = 'SYNTHETIC_SUMMARY: preserve the project and next action.'
            number = len(captures)
            if self.path.endswith('/responses'):
                item = {'id': f'item-{number}', 'type': 'message', 'role': 'assistant', 'status': 'completed',
                        'content': [{'type': 'output_text', 'text': text, 'annotations': []}]}
                response = {'id': f'response-{number}', 'object': 'response', 'status': 'completed', 'output': [item],
                            'usage': {'input_tokens': 2000, 'output_tokens': 20, 'total_tokens': 2020,
                                      'input_tokens_details': {'cached_tokens': 0}, 'output_tokens_details': {'reasoning_tokens': 0}}}
                events = [{'type': 'response.created', 'response': {**response, 'status': 'in_progress', 'output': []}},
                          {'type': 'response.output_item.added', 'output_index': 0, 'item': item},
                          {'type': 'response.output_item.done', 'output_index': 0, 'item': item},
                          {'type': 'response.completed', 'response': response}]
            else:
                message = {'id': f'message-{number}', 'type': 'message', 'role': 'assistant', 'model': body['model'],
                           'content': [], 'stop_reason': None, 'stop_sequence': None,
                           'usage': {'input_tokens': 2000, 'output_tokens': 0, 'cache_read_input_tokens': 0, 'cache_creation_input_tokens': 0}}
                if not body.get('stream'):
                    return self.data(200, {**message, 'content': [{'type': 'text', 'text': text}], 'stop_reason': 'end_turn'})
                events = [{'type': 'message_start', 'message': message},
                          {'type': 'content_block_start', 'index': 0, 'content_block': {'type': 'text', 'text': ''}},
                          {'type': 'content_block_delta', 'index': 0, 'delta': {'type': 'text_delta', 'text': text}},
                          {'type': 'content_block_stop', 'index': 0},
                          {'type': 'message_delta', 'delta': {'stop_reason': 'end_turn', 'stop_sequence': None}, 'usage': {'output_tokens': 20}},
                          {'type': 'message_stop'}]
            data = ''.join(f'event: {event["type"]}\ndata: {json.dumps(event)}\n\n' for event in events).encode()
            self.send_response(200)
            self.send_header('Content-Type', 'text/event-stream')
            self.send_header('Content-Length', str(len(data)))
            self.end_headers()
            try:
                self.wfile.write(data)
            except (BrokenPipeError, ConnectionResetError):
                pass

    server = ThreadingHTTPServer(('127.0.0.1', 0), Upstream)
    threading.Thread(target=server.serve_forever, daemon=True).start()
    if args.keep:
        args.keep.mkdir(parents=True, exist_ok=False)
    workspace = nullcontext(str(args.keep.resolve())) if args.keep else tempfile.TemporaryDirectory(prefix='velune-context-compaction-')
    with workspace as directory:
        root = Path(directory)
        (root / 'bindings').mkdir()
        (root / 'project').mkdir()
        (root / 'pi').mkdir()
        (root / 'pi/settings.json').write_text(json.dumps({'compaction': {'enabled': False, 'keepRecentTokens': 128, 'reserveTokens': 1024}}))
        shutil.copy2(args.library, root / 'bindings' / args.library.name)
        shutil.copy2(args.bindings / 'velune_bindings.py', root / 'bindings/velune_bindings.py')
        spec = importlib.util.spec_from_file_location('velune_compaction', root / 'bindings/velune_bindings.py')
        b = importlib.util.module_from_spec(spec)
        sys.modules[spec.name] = b
        spec.loader.exec_module(b)
        app = b.VeluneApplication.open(b.BindingOptions(home_directory=str(root / 'velune'), resources_directory=str(args.resources)))
        runtime = args.runtime
        try:
            instance = b.BindingRuntimeInstance(enabled=True, id=runtime, name=runtime,
                type_id={'codex': 'codex-0.159', 'claude': 'claude-code-2.1', 'pi': 'pi-1.0'}[runtime], gateway_id='default',
                settings={'binary': str(args.binary.resolve()), 'nodeBinary': str(args.node), **({'agentDir': str(root / 'pi')} if runtime == 'pi' else {})})
            app.upsert_runtime_with_execution(instance, b.BindingRuntimeExecution(transport=b.BindingRuntimeTransport.LOCAL(),
                home=b.BindingRuntimeHome.MANAGED(remote_root=None), remote_resources_directory=None))
            if runtime == 'pi':
                agent = root / 'velune/runtime-homes' / hashlib.sha256(runtime.encode()).hexdigest() / '.pi/agent'
                agent.mkdir(parents=True, exist_ok=True)
                (agent / 'settings.json').write_text((root / 'pi/settings.json').read_text())
            app.save_provider('default', b.BindingProviderDraft(id='synthetic', name='Synthetic',
                protocol=b.BindingGatewayProtocol.MESSAGES_V1 if runtime == 'claude' else b.BindingGatewayProtocol.RESPONSES_V1,
                endpoint=f'http://127.0.0.1:{server.server_port}/v1',
                models=[b.BindingProviderModel(record_key='', provider_model_id='synthetic', nickname='Synthetic', icon=None,
                    context_window=32768, max_output_tokens=1024, reasoning_levels=[])], quota_limits=[],
                execution_backend=b.BindingExecutionBackend.HTTP(), request_profile=b.BindingRequestProfile.NATIVE,
                authentication_source=b.BindingProviderAuthenticationSource.API_KEY, quota_source=b.BindingProviderQuotaSource.NONE),
                b.BindingAuthenticationEdit.SET_API_KEY(value='SYNTHETIC_ONLY'))
            key = next(p for p in app.list().gateways[0].providers if p.id == 'synthetic').models[0].record_key
            selection = b.BindingModelSelection.MANAGED(model_record_key=key)
            conversation = app.create_conversation(runtime, str(root / 'project'), key).snapshot.conversation.id

            def snapshot():
                return app.snapshot(runtime, conversation).snapshot

            def wait(predicate, label):
                deadline = time.monotonic() + 45
                while time.monotonic() < deadline:
                    current = snapshot()
                    if predicate(current):
                        return current
                    time.sleep(.05)
                raise AssertionError((label, current.run_state, [(message.id, message.completed, block.text) for message, block in notices(current)], len(captures)))

            def notices(current):
                return [(message, block) for message in current.messages for block in message.blocks
                        if isinstance(block, b.BindingMessageBlock.CONTEXT_COMPACTION)]

            def send(index):
                app.send_turn_with_selection(runtime, conversation, runtime, selection,
                    b.BindingComposedTurn(text=f'SYNTHETIC_{index}: ' + 'context evidence ' * 1000,
                        effort=None, permission_mode='plan' if runtime == 'claude' else 'read-only' if runtime == 'codex' else None, attachments=[]))
                return wait(lambda s: s.actions.can_send, 'normal turn completed')

            assert not snapshot().follow_up.can_compact
            for index in range(3):
                before = send(index)
            assert before.follow_up.can_compact
            old_turn = before.transcript_turns[-1]
            other = app.create_conversation(runtime, str(root / 'project'), key).snapshot.conversation.id
            app.send_turn_with_selection(runtime, other, runtime, selection,
                b.BindingComposedTurn(text='OTHER_SESSION_SYNTHETIC', effort=None, permission_mode=None, attachments=[]))
            deadline = time.monotonic() + 45
            while not app.snapshot(runtime, other).snapshot.actions.can_send:
                assert time.monotonic() < deadline
                time.sleep(.05)
            control['mode'] = 'hold'
            started_at = time.monotonic()
            accepted = app.compact_conversation(runtime, conversation, selection).snapshot
            assert accepted.conversation.id == conversation, 'compaction targeted another session'
            assert time.monotonic() - started_at < 10, 'compaction call waited for the model'
            running = wait(lambda s: held.is_set() and any(not m.completed for m, _ in notices(s)), 'compaction is visibly running')
            assert running.follow_up.is_compacting and running.actions.can_cancel
            assert not running.follow_up.can_steer and not running.follow_up.can_compact
            assert not running.transcript_turns[-1].is_running
            assert running.transcript_turns[-1].duration_ms == old_turn.duration_ms
            assert running.transcript_turns[-1].last_message_id == old_turn.last_message_id
            notice_id = notices(running)[-1][0].id
            assert all(notice_id not in turn.work_message_ids for turn in running.transcript_turns)
            release.set()
            finished = wait(lambda s: s.actions.can_send and not s.follow_up.is_compacting, 'compaction completed')
            assert any(block.text == '上下文已压缩' for _, block in notices(finished)), notices(finished)
            assert not any(not message.completed for message, _ in notices(finished))
            assert '/compact' not in [getattr(block, 'text', '') for message in finished.messages
                if message.role == b.BindingMessageRole.USER for block in message.blocks], 'command became a user message'
            print(f'PASS {runtime}: native compaction, visible progress, polling, completion, independent transcript and no user command')

            control['mode'] = 'reply'
            send(3)
            control['mode'] = 'hold'
            held.clear(); release.clear()
            app.compact_conversation(runtime, conversation, selection)
            wait(lambda s: held.is_set() and s.actions.can_cancel, 'second compaction can be stopped')
            app.cancel(runtime, conversation)
            release.set()
            cancelled = wait(lambda s: s.actions.can_send and not s.follow_up.is_compacting, 'compaction cancelled')
            assert not any(not message.completed for message, _ in notices(cancelled))
            assert any('取消' in block.text or '中断' in block.text for _, block in notices(cancelled)), [(message.completed, block.text) for message, block in notices(cancelled)]
            print(f'PASS {runtime}: cancellation clears progress and restores controls')

            control['mode'] = 'reply'
            normal = send(4)
            baseline = normal.transcript_turns[-1]
            control['mode'] = 'fail'
            app.compact_conversation(runtime, conversation, selection)
            failed = wait(lambda s: s.actions.can_send and not s.follow_up.is_compacting, 'compaction failure is terminal')
            assert any('失败' in block.text for _, block in notices(failed)), [(message.completed, block.text) for message, block in notices(failed)]
            assert failed.transcript_turns[-1].last_message_id == baseline.last_message_id, (baseline.last_message_id, failed.transcript_turns[-1].last_message_id, [(message.id, message.role, [(type(block).__name__, getattr(block, 'text', '')[:180]) for block in message.blocks]) for message in failed.messages[-5:]])
            assert failed.transcript_turns[-1].duration_ms == baseline.duration_ms
            assert failed.follow_up.can_resume == normal.follow_up.can_resume
            print(f'PASS {runtime}: failure preserves the previous reply, duration and recovery eligibility')

            control['mode'] = 'reply'
            app.shutdown()
            app = b.VeluneApplication.open(b.BindingOptions(home_directory=str(root / 'velune'), resources_directory=str(args.resources)))
            reopened = app.open_conversation(runtime, conversation).snapshot
            assert not any(not message.completed for message, _ in notices(reopened))
            assert '/compact' not in [getattr(block, 'text', '') for message in reopened.messages
                if message.role == b.BindingMessageRole.USER for block in message.blocks]
            print(f'PASS {runtime}: native history reopens without a command user message or stale progress')

            held.clear(); release.clear(); control['mode'] = 'hold'
            app.send_turn_with_selection(runtime, conversation, runtime, selection,
                b.BindingComposedTurn(text='SYNTHETIC_PAUSED: ' + 'context ' * 1000, effort=None, permission_mode=None, attachments=[]))
            wait(lambda s: held.is_set() and s.actions.can_cancel, 'user turn can be stopped')
            queued = app.send_queue(runtime, conversation, b.BindingSendQueueOperation.ADD(
                source_runtime_id=runtime, target_runtime_id=runtime, selection=selection,
                turn=b.BindingComposedTurn(text='MUST_STAY_QUEUED', effort=None, permission_mode=None, attachments=[]))).snapshot
            app.cancel(runtime, conversation)
            release.set()
            paused = wait(lambda s: s.actions.can_send and s.follow_up.paused is not None, 'send queue paused')
            count = len(captures)
            control['mode'] = 'reply'
            app.compact_conversation(runtime, conversation, selection)
            kept = wait(lambda s: s.actions.can_send and not s.follow_up.is_compacting, 'compaction preserves queue pause')
            assert kept.follow_up.paused == paused.follow_up.paused
            assert [entry.id for entry in kept.follow_up.queue] == [entry.id for entry in queued.follow_up.queue]
            assert not any('MUST_STAY_QUEUED' in json.dumps(body) for body in captures[count:])
            print(f'PASS {runtime}: paused send queue remains paused after compaction')

            if args.disconnect:
                for entry in kept.follow_up.queue:
                    app.send_queue(runtime, conversation, b.BindingSendQueueOperation.REMOVE(id=entry.id))
                baseline = send(5).transcript_turns[-1]
                control['mode'] = 'hold'
                held.clear(); release.clear()
                app.compact_conversation(runtime, conversation, selection)
                wait(lambda s: held.is_set() and any(not m.completed for m, _ in notices(s)), 'compaction before disconnect')
                children = []
                for line in subprocess.check_output(['ps', '-axo', 'pid=,ppid=,command='], text=True).splitlines():
                    pid, parent, command = line.strip().split(None, 2)
                    if int(parent) == os.getpid() and (str(args.binary) in command or 'pi_rpc.mjs' in command):
                        children.append(int(pid))
                assert len(children) == 1, ('expected one isolated runtime child', children)
                os.kill(children[0], signal.SIGKILL)
                release.set()
                try:
                    disconnected = wait(lambda s: not s.follow_up.is_compacting, 'runtime disconnect settles compaction')
                except b.BindingError:
                    # Native polling reports EOF once before returning the settled snapshot.
                    disconnected = wait(lambda s: not s.follow_up.is_compacting, 'snapshot after reported disconnect')
                assert disconnected.run_state == b.BindingRunState.FAILED
                assert all(message.completed for message, _ in notices(disconnected))
                assert any('中断' in block.text for _, block in notices(disconnected)), [(m.id, block.text) for m, block in notices(disconnected)]
                assert disconnected.transcript_turns[-1].last_message_id == baseline.last_message_id
                assert disconnected.transcript_turns[-1].duration_ms == baseline.duration_ms
                print(f'PASS {runtime}: runtime disconnect preserves the previous reply and completes compaction status')
        finally:
            release.set()
            app.shutdown_stopping_work()
            server.shutdown()
            server.server_close()


if __name__ == '__main__':
    main()
