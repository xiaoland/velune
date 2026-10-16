#!/usr/bin/env python3
"""Explicit isolated acceptance of model provenance and managed HOME, using a synthetic Claude peer."""
import argparse
import importlib.util
import json
import os
from pathlib import Path
import shutil
import sys
import tempfile
import time

PEER = r'''#!__PYTHON__
import json, os, sys, uuid
from pathlib import Path
if '--version' in sys.argv:
    print('2.1.286')
    raise SystemExit(0)
def emit(value):
    print(json.dumps(value), flush=True)
def option(flag):
    return sys.argv[sys.argv.index(flag) + 1] if flag in sys.argv else None
log = Path(__LOG__)
if (log.parent / 'fail-launch').exists():
    raise SystemExit(2)
if option('--resume') and not (Path(os.environ['CLAUDE_CONFIG_DIR']) / 'projects' / 'synthetic' / (option('--resume') + '.jsonl')).exists():
    raise SystemExit(3)
def capture(kind):
    with log.open('a') as file:
        file.write(json.dumps({'kind':kind,'pid':os.getpid(),'home':os.environ.get('HOME'),'agentDir':os.environ.get('CLAUDE_CONFIG_DIR'),'managed':os.environ.get('CLAUDE_CODE_PROVIDER_MANAGED_BY_HOST') == '1','model':option('--model')}) + '\n')
capture('start')
def settle(failed=False):
    emit({'type':'result','subtype':'error_during_execution' if failed else 'success','is_error':failed})
    emit({'type':'system','subtype':'session_state_changed','state':'idle'})
for line in sys.stdin:
    request = json.loads(line)
    if request['type'] == 'control_request':
        response = {'models':[{'value':'native-model','displayName':'Native model'}]} if request['request']['subtype'] == 'initialize' else {}
        emit({'type':'control_response','response':{'subtype':'success','request_id':request['request_id'],'response':response}})
        if request['request']['subtype'] == 'interrupt': settle()
    elif request['type'] == 'user':
        capture('turn')
        emit({'type':'system','subtype':'session_state_changed','state':'running'})
        content = request['message']['content']
        text = content[0]['text']
        if text == 'CANCEL_SYNTHETIC': continue
        if text == 'NO_HISTORY_FAILURE':
            settle(True)
            continue
        session = option('--session-id') or option('--resume')
        directory = Path(os.environ['CLAUDE_CONFIG_DIR']) / 'projects' / 'synthetic'
        directory.mkdir(parents=True, exist_ok=True)
        path = directory / (session + '.jsonl')
        previous = None
        if path.exists():
            records = [json.loads(line) for line in path.read_text().splitlines()]
            previous = records[-1]['uuid']
        user_id, assistant_id = str(uuid.uuid4()), str(uuid.uuid4())
        with path.open('a') as file:
            for entry in [
                {'uuid':user_id,'parentUuid':previous,'type':'user','message':{'content':content}},
                {'uuid':assistant_id,'parentUuid':user_id,'type':'assistant','message':{'content':[{'type':'text','text':'SYNTHETIC_ANSWER'}]}}
            ]:
                entry.update(cwd=os.getcwd(), timestamp='2026-10-10T00:00:00Z', sessionId=session)
                file.write(json.dumps(entry) + '\n')
        emit({'type':'assistant','message':{'id':assistant_id,'content':[{'type':'text','text':'SYNTHETIC_ANSWER'}]}})
        settle(text == 'FAIL_SYNTHETIC')
'''


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    for name in ('library', 'resources', 'bindings'):
        parser.add_argument('--' + name, required=True, type=Path)
    args = parser.parse_args()
    for path in vars(args).values():
        if not path.is_absolute() or not path.exists():
            parser.error('All paths must be absolute and exist')
    previous_environment = dict(os.environ)
    application = None
    with tempfile.TemporaryDirectory(prefix='velune-runtime-selection-') as temporary:
        root = Path(temporary)
        for directory in ('bindings', 'user-home', 'application', 'project', 'existing-agent'):
            (root / directory).mkdir()
        log = root / 'peer.jsonl'
        peer = root / 'claude-peer'
        peer.write_text(PEER.replace('__PYTHON__', sys.executable).replace('__LOG__', repr(str(log))))
        peer.chmod(0o700)
        sentinel = root / 'existing-agent' / 'settings.json'
        sentinel.write_text('{"syntheticProvider":"existing"}\n')
        shutil.copy(args.bindings / 'velune_bindings.py', root / 'bindings')
        (root / 'bindings' / args.library.name).symlink_to(args.library)
        os.environ.clear()
        os.environ.update(HOME=str(root / 'user-home'), PATH='/usr/bin:/bin', NO_PROXY='127.0.0.1,localhost')
        try:
            spec = importlib.util.spec_from_file_location('velune_bindings', root / 'bindings' / 'velune_bindings.py')
            b = importlib.util.module_from_spec(spec)
            sys.modules[spec.name] = b
            spec.loader.exec_module(b)
            application = b.VeluneApplication.open(b.BindingOptions(home_directory=str(root / 'application'), resources_directory=str(args.resources)))
            managed_execution = b.BindingRuntimeExecution(transport=b.BindingRuntimeTransport.LOCAL(), home=b.BindingRuntimeHome.MANAGED(remote_root=None), remote_resources_directory=None)
            for identity in ('first', 'second'):
                settings = {'binary':str(peer)}
                if identity == 'first':
                    settings['agentDir'] = str(root / 'existing-agent')
                runtime = b.BindingRuntimeInstance(enabled=True, id=identity, name=identity, type_id='claude-code-2.1.286', gateway_id='default', settings=settings)
                if identity == 'first':
                    application.upsert_runtime(runtime)
                else:
                    application.upsert_runtime_with_execution(runtime, managed_execution)
            assert isinstance(application.runtime_execution('second').home, b.BindingRuntimeHome.MANAGED)
            models = application.runtime_models('first')
            native = next(model.selection for model in models if isinstance(model.selection, b.BindingModelSelection.NATIVE))
            assert native.model_id == 'native-model' and native.provider_id is None
            model = b.BindingProviderModel(record_key='', provider_model_id='managed-model', nickname='Managed model', icon=None, context_window=32768, max_output_tokens=4096, reasoning_levels=None, adapter_metadata_json=None)
            provider = b.BindingProviderDraft(id='managed', name='Managed', protocol=b.BindingGatewayProtocol.MESSAGES_V1, endpoint='http://127.0.0.1:9/v1', models=[model], quota_limits=[])
            saved = application.save_provider('default', provider, b.BindingAuthenticationEdit.SET_API_KEY(value='synthetic-only'))
            key = next(provider.models[0].record_key for gateway in saved.gateways for provider in gateway.providers if provider.id == 'managed')
            managed = b.BindingModelSelection.MANAGED(model_record_key=key)
            assert any(isinstance(model.selection, b.BindingModelSelection.MANAGED) for model in application.runtime_models('first'))
            snapshot = application.create_conversation_with_selection('first', str(root / 'project'), native).snapshot
            conversation = snapshot.conversation.id

            def records():
                return [json.loads(line) for line in log.read_text().splitlines()]

            def turn(target, selection, text='SYNTHETIC_REQUEST'):
                application.send_turn_with_selection('first', conversation, target, selection, b.BindingComposedTurn(text=text, effort=None, attachments=[]))
                deadline = time.monotonic() + 10
                while time.monotonic() < deadline:
                    value = application.snapshot(target, conversation).snapshot
                    if value and value.actions.can_send:
                        return value
                    time.sleep(0.02)
                raise AssertionError('Synthetic turn did not settle')

            turn('first', native)
            assert not records()[-1]['managed']
            managed_snapshot = turn('first', managed)
            assert managed_snapshot.model_record_key == key
            assert isinstance(application.next_turn_model_selection(), b.BindingModelSelection.MANAGED)
            managed_turn = next(record for record in reversed(records()) if record['kind'] == 'turn')
            assert managed_turn['managed'] and managed_turn['model'] == 'managed-model'
            try:
                os.kill(managed_turn['pid'], 0)
            except ProcessLookupError:
                pass
            else:
                raise AssertionError('Managed turn process survived settlement')
            turn('first', native)
            assert not records()[-1]['managed'] and records()[-1]['model'] == 'native-model'
            failed = turn('first', managed, 'FAIL_SYNTHETIC')
            assert failed.run_state == b.BindingRunState.FAILED
            turn('first', native)
            application.send_turn_with_selection('first', conversation, 'first', managed, b.BindingComposedTurn(text='CANCEL_SYNTHETIC', effort=None, attachments=[]))
            application.cancel('first', conversation)
            deadline = time.monotonic() + 10
            while time.monotonic() < deadline:
                cancelled = application.snapshot('first', conversation).snapshot
                if cancelled.actions.can_send:
                    break
                time.sleep(0.02)
            else:
                raise AssertionError('Cancelled managed turn did not settle')
            cancelled_turn = next(record for record in reversed(records()) if record['kind'] == 'turn')
            try:
                os.kill(cancelled_turn['pid'], 0)
            except ProcessLookupError:
                pass
            else:
                raise AssertionError('Managed turn process survived cancellation')
            turn('first', native)
            target = turn('second', native)
            assert target.context_runtime_id == 'second' and target.conversation.id == conversation
            last_turn = next(record for record in reversed(records()) if record['kind'] == 'turn')
            assert Path(last_turn['home']).is_relative_to(root / 'application' / 'runtime-homes')
            assert Path(last_turn['agentDir']) == Path(last_turn['home']) / '.claude'
            assert sentinel.read_text() == '{"syntheticProvider":"existing"}\n'
            assert application.read_conversation('first', conversation).snapshot.messages
            def wait_snapshot(runtime_id, public_id):
                deadline = time.monotonic() + 10
                while time.monotonic() < deadline:
                    current = application.snapshot(runtime_id, public_id).snapshot
                    if current and current.actions.can_send:
                        return current
                    time.sleep(0.02)
                raise AssertionError('Fresh draft turn did not settle')

            for initial, selected, target_runtime in ((native, managed, 'first'), (managed, native, 'first'), (native, native, 'second')):
                draft = application.create_conversation_with_selection('first', str(root / 'project'), initial).snapshot
                result = application.send_turn_with_selection('first', draft.conversation.id, target_runtime, selected, b.BindingComposedTurn(text='FRESH_SELECTION', effort=None, attachments=[]))
                current = wait_snapshot(target_runtime, draft.conversation.id)
                assert current.conversation.id == draft.conversation.id
                assert current.conversation.runtime_id == target_runtime
                assert not any(message.id == 'segment-0:boundary' for message in current.messages)
                reopened = application.read_conversation(target_runtime, draft.conversation.id).snapshot
                assert reopened and reopened.conversation.id == draft.conversation.id
                links = json.loads((root / 'application' / 'conversation-links.json').read_text())['links']
                alias = next(link for link in links if link['id'] == draft.conversation.id)
                assert len(alias['segments']) == 1 and alias['segments'][0]['nativeConversationId'] != draft.conversation.id
                if target_runtime == 'second':
                    application.send_turn_with_selection('second', draft.conversation.id, 'first', managed, b.BindingComposedTurn(text='ALIAS_HANDOFF', effort=None, attachments=[]))
                    extended = wait_snapshot('first', draft.conversation.id)
                    assert extended.conversation.runtime_id == 'second'
                    assert any(message.id == 'segment-1:boundary' for message in extended.messages)
                    links = json.loads((root / 'application' / 'conversation-links.json').read_text())['links']
                    alias = next(link for link in links if link['id'] == draft.conversation.id)
                    assert len(alias['segments']) == 2

            draft = application.create_conversation_with_selection('first', str(root / 'project'), native).snapshot
            before = sum(record['kind'] == 'turn' for record in records())
            launch_failure = root / 'fail-launch'
            launch_failure.touch()
            try:
                application.send_turn_with_selection('first', draft.conversation.id, 'first', managed, b.BindingComposedTurn(text='SETUP_RETRY', effort=None, attachments=[]))
            except b.BindingError:
                pass
            else:
                raise AssertionError('Injected setup failure was ignored')
            launch_failure.unlink()
            assert sum(record['kind'] == 'turn' for record in records()) == before
            application.send_turn_with_selection('first', draft.conversation.id, 'first', managed, b.BindingComposedTurn(text='SETUP_RETRY', effort=None, attachments=[]))
            wait_snapshot('first', draft.conversation.id)

            draft = application.create_conversation_with_selection('first', str(root / 'project'), native).snapshot
            blocked_alias = root / 'application' / 'conversation-links.json.tmp'
            blocked_alias.mkdir()
            before = sum(record['kind'] == 'turn' for record in records())
            try:
                application.send_turn_with_selection('first', draft.conversation.id, 'first', managed, b.BindingComposedTurn(text='ALIAS_RETRY', effort=None, attachments=[]))
            except b.BindingError:
                pass
            else:
                raise AssertionError('Injected alias commit failure was ignored')
            blocked_alias.rmdir()
            assert sum(record['kind'] == 'turn' for record in records()) == before
            starts = sum(record['kind'] == 'start' for record in records())
            application.send_turn_with_selection('first', draft.conversation.id, 'first', managed, b.BindingComposedTurn(text='ALIAS_RETRY', effort=None, attachments=[]))
            wait_snapshot('first', draft.conversation.id)
            assert sum(record['kind'] == 'start' for record in records()) == starts + 1, 'Alias retry recreated the prepared target'

            draft = application.create_conversation_with_selection('first', str(root / 'project'), native).snapshot
            application.send_turn_with_selection('first', draft.conversation.id, 'first', managed, b.BindingComposedTurn(text='NO_HISTORY_FAILURE', effort=None, attachments=[]))
            wait_snapshot('first', draft.conversation.id)
            application.shutdown()
            application = b.VeluneApplication.open(b.BindingOptions(home_directory=str(root / 'application'), resources_directory=str(args.resources)))
            missing = application.open_conversation('first', draft.conversation.id).snapshot
            assert missing and not missing.actions.can_send
            assert next(row for row in [_row.conversation for _row in application.list().conversations] if row.id == draft.conversation.id).can_delete
            application.delete_conversation('first', draft.conversation.id)
            assert all(row.id != draft.conversation.id for row in [_row.conversation for _row in application.list().conversations])
            remote = b.BindingRuntimeExecution(transport=b.BindingRuntimeTransport.SSH(binary='/usr/bin/ssh', destination='synthetic-unused', port=2222), home=b.BindingRuntimeHome.MANAGED(remote_root='/remote/velune-homes'), remote_resources_directory='/remote/velune-resources')
            application.configure_runtime_execution('second', remote)
            assert application.runtime_execution('second').transport.destination == 'synthetic-unused'
            application.configure_runtime_execution('second', managed_execution)
            application.shutdown()
            application = None
            print('PASS: native catalog/dispatch; managed override and process cleanup; native restoration; failure/cancel cleanup; cross-instance continuation; managed HOME; SSH typed configuration; unchanged source settings; fresh selection/handoff; setup/alias failure retry; unavailable alias reopen/delete')
        finally:
            if application is not None:
                try:
                    application.shutdown()
                except Exception:
                    pass
            os.environ.clear()
            os.environ.update(previous_environment)


if __name__ == '__main__':
    main()
