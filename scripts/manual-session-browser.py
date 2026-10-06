#!/usr/bin/env python3
"""Manual isolated history/lazy-preparation acceptance, never a CI entry point.

Creates history through the bundled Pi SDK in a temporary HOME. No credentials,
real histories or model requests are used. Supply matching generated bindings.
"""
import argparse
import importlib.util
import json
import os
from pathlib import Path
import shutil
import subprocess
import sys
import tempfile

SEED = r'''
import {SessionManager} from '@earendil-works/pi-coding-agent';
const [cwd, sessionDir, title] = process.argv.slice(2);
const session = SessionManager.create(cwd, sessionDir);
session.appendMessage({role:'user', content:[{type:'text',text:'SYNTHETIC_READ_ONLY_HISTORY'}], timestamp:Date.now()});
session.appendMessage({role:'assistant', content:[
  {type:'thinking',thinking:'SYNTHETIC_REASONING'},
  {type:'text',text:'## Synthetic reply\n\n- Markdown content'},
  {type:'toolCall',id:'synthetic-read',name:'read',arguments:{path:'synthetic.txt'}}
], api:'openai-completions',provider:'synthetic',model:'synthetic',stopReason:'toolUse',timestamp:Date.now()});
session.appendMessage({role:'toolResult',toolCallId:'synthetic-read',toolName:'read',content:[{type:'text',text:'SYNTHETIC_TOOL_OUTPUT'}],isError:false,timestamp:Date.now()});
if (title) session.appendSessionInfo(title);
console.log(session.getSessionFile());
'''


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    for name in ('bundle', 'bindings', 'node'):
        parser.add_argument('--' + name, type=Path, required=True)
    args = parser.parse_args()
    for path in vars(args).values():
        if not path.is_absolute() or not path.exists():
            parser.error('absolute existing paths required')
    environment = dict(os.environ)
    application = None
    with tempfile.TemporaryDirectory(prefix='velune-session-browser-') as directory:
        root = Path(directory)
        resources = args.bundle / 'Contents/Resources'
        for name in ('bindings', 'runtime', 'foreign', 'project', 'seed'):
            (root / name).mkdir()
        (root / 'seed/node_modules').symlink_to(resources / 'node_modules')
        seed = root / 'seed/create.mjs'
        seed.write_text(SEED)
        fixture_env = {'HOME': str(root), 'PATH': '/usr/bin:/bin'}
        def seed_session(home, title=''):
            sessions = home / 'sessions/project'
            sessions.mkdir(parents=True, exist_ok=True)
            return Path(subprocess.check_output([str(args.node), str(seed),
                str(root / 'project'), str(sessions), title], env=fixture_env, text=True).strip())
        history = seed_session(root / 'runtime')
        named_history = seed_session(root / 'runtime', 'SYNTHETIC_EXPLICIT_TITLE')
        placeholder_named_history = seed_session(root / 'runtime', '未命名会话')
        foreign = seed_session(root / 'foreign')
        original = history.read_bytes()
        named_original = named_history.read_bytes()
        placeholder_named_original = placeholder_named_history.read_bytes()
        shutil.rmtree(root / 'project')
        marker = root / 'execution-probes.jsonl'
        binary = root / 'synthetic-pi.mjs'
        binary.write_text('import fs from "node:fs";\n'
            + 'fs.appendFileSync(' + json.dumps(str(marker)) + ',JSON.stringify(process.argv.slice(2))+"\\n");\n'
            + 'console.log("pi 9.9.9");\n')
        shutil.copy(args.bindings / 'velune_bindings.py', root / 'bindings')
        (root / 'bindings/libvelune_bindings.dylib').symlink_to(
            args.bundle / 'Contents/Frameworks/libvelune_bindings.dylib')
        os.environ.clear()
        os.environ.update(fixture_env)
        try:
            spec = importlib.util.spec_from_file_location('velune_bindings', root / 'bindings/velune_bindings.py')
            b = importlib.util.module_from_spec(spec)
            sys.modules[spec.name] = b
            spec.loader.exec_module(b)
            application = b.VeluneApplication.open(b.BindingOptions(
                home_directory=str(root / 'home'), resources_directory=str(resources)))
            runtime = b.BindingRuntimeInstance(enabled=True, id='fixture', name='Synthetic Pi',
                type_id='pi-1.0.2', gateway_id='default', settings={
                    'binary': str(binary), 'nodeBinary': str(args.node),
                    'agentDir': str(root / 'runtime')})
            application.upsert_runtime(runtime)
            listed = application.list()
            assert not any(g.providers for g in listed.gateways)
            item = next(s for s in listed.conversations if s.id == 'fixture:' + str(history))
            named_item = next(s for s in listed.conversations if s.id == 'fixture:' + str(named_history))
            assert item.title == 'SYNTHETIC_READ_ONLY_HISTORY'
            assert named_item.title == 'SYNTHETIC_EXPLICIT_TITLE'
            placeholder_item = next(s for s in listed.conversations
                                    if s.id == 'fixture:' + str(placeholder_named_history))
            assert placeholder_item.title == '未命名会话'
            assert item.updated_at_unix_ms is not None and item.updated_at_unix_ms > 0
            assert item.created_at_unix_ms is not None and item.created_at_unix_ms > 0
            other = b.BindingRuntimeInstance(enabled=True, id='other', name='Other Pi',
                type_id='pi-1.0.2', gateway_id='default', settings=dict(runtime.settings, agentDir=str(root / 'foreign')))
            application.upsert_runtime(other)
            assert any(s.runtime_id == 'other' for s in application.list().conversations)
            selected = application.select_runtime('fixture')
            assert selected.selected_runtime_instance_id == 'fixture'
            named_opened = application.open_conversation('fixture', placeholder_item.id).snapshot
            assert named_opened.conversation.title == '未命名会话', 'native title treated as a placeholder'
            opened = application.open_conversation('fixture', item.id).snapshot
            assert opened.model_record_key is None
            assert opened.conversation.title == item.title
            assert opened.messages[0].role == b.BindingMessageRole.USER
            assert opened.messages[0].timestamp_unix_ms is not None
            texts = [block.text for message in opened.messages for block in message.blocks
                     if isinstance(block, b.BindingMessageBlock.TEXT)]
            assert 'SYNTHETIC_READ_ONLY_HISTORY' in texts
            blocks = [block for message in opened.messages for block in message.blocks]
            assert any(isinstance(block, b.BindingMessageBlock.REASONING)
                       and block.text == 'SYNTHETIC_REASONING' for block in blocks)
            tools = [block for block in blocks if isinstance(block, b.BindingMessageBlock.TOOL)
                     and block.tool_id == 'synthetic-read']
            assert len(tools) == 1, 'one native tool call/result became duplicate executions'
            assert tools[0].state == b.BindingToolState.COMPLETED
            assert tools[0].output == 'SYNTHETIC_TOOL_OUTPUT'
            assert not marker.exists(), 'browsing ran the configured execution CLI'
            def rejected(call):
                try:
                    call()
                except b.BindingError:
                    return
                raise AssertionError('operation unexpectedly succeeded')
            rejected(lambda: application.open_conversation('fixture', 'fixture:' + str(foreign)))
            assert application.snapshot('fixture').snapshot.conversation.id == item.id
            model = b.BindingProviderModel(record_key='', provider_model_id='synthetic',
                nickname='Synthetic', icon=None, context_window=8192, max_output_tokens=128,
                reasoning_levels=None, adapter_metadata_json=None)
            provider = b.BindingProviderDraft(id='fixture-provider', name='Synthetic',
                protocol=b.BindingGatewayProtocol.CHAT_COMPLETIONS_V1,
                endpoint='http://127.0.0.1:9/v1', models=[model])
            application.save_provider('default', provider, b.BindingAuthenticationEdit.KEEP())
            key = application.list().gateways[0].providers[0].models[0].record_key
            model.record_key = key
            chosen = application.select_model('fixture', key).snapshot
            assert chosen.model_record_key == key and chosen.conversation.id == item.id
            assert not marker.exists(), 'choosing a model started execution'
            rejected(lambda: application.send('fixture', 'DO_NOT_ACCEPT_THIS_MESSAGE'))
            current = application.snapshot('fixture').snapshot
            assert current.conversation.id == item.id and current.model_record_key == key
            assert current.messages == chosen.messages
            assert not marker.exists(), 'missing cwd was checked after running the CLI'
            (root / 'project').mkdir()
            application.save_provider('default', provider,
                b.BindingAuthenticationEdit.SET_API_KEY(value='SYNTHETIC_ONLY'))
            # Existing internal identity remains stable after provider edits.
            application.select_model('fixture', key)
            rejected(lambda: application.send('fixture', 'DO_NOT_ACCEPT_THIS_MESSAGE'))
            current = application.snapshot('fixture').snapshot
            assert current.conversation.id == item.id and current.model_record_key == key
            assert current.messages == chosen.messages
            bad = b.BindingRuntimeInstance(enabled=True, id='unreadable', name='Synthetic broken source',
                type_id='pi-1.0.2', gateway_id='default', settings={
                    'binary': str(binary), 'nodeBinary': str(root / 'missing-node'),
                    'agentDir': str(root / 'foreign')})
            application.upsert_runtime(bad)
            listed = application.list()
            assert any(s.id == item.id for s in listed.conversations)
            assert any(error.runtime_id == 'unreadable' for error in listed.history_failures)
            # Disable an invalid source without probing its missing Node; retain its configuration.
            bad.enabled = False
            application.upsert_runtime(bad)
            assert not any(e.runtime_id == 'unreadable' for e in application.list().history_failures)
            runtime.enabled = False
            application.upsert_runtime(runtime)
            disabled = application.list()
            assert not any(s.runtime_id == 'fixture' for s in disabled.conversations)
            assert any(s.runtime_id == 'other' for s in disabled.conversations)
            assert disabled.selected_runtime_instance_id is None
            rejected(lambda: application.open_conversation('fixture', item.id))
            rejected(lambda: application.create_conversation('fixture', str(root / 'project'), key))
            runtime.enabled = True
            application.upsert_runtime(runtime)
            assert any(s.id == item.id for s in application.list().conversations)
            application.shutdown()
            application = None
            config_path = root / 'home/generic-config.json'
            legacy = json.loads(config_path.read_text())
            for stored in legacy['runtimeInstances']: stored.pop('enabled', None)
            config_path.write_text(json.dumps(legacy))
            application = b.VeluneApplication.open(b.BindingOptions(home_directory=str(root / 'home'), resources_directory=str(resources)))
            assert all(r.enabled for r in application.list().runtime_instances)
            assert any(s.id == item.id for s in application.list().conversations)
            assert history.read_bytes() == original
            assert named_history.read_bytes() == named_original
            assert placeholder_named_history.read_bytes() == placeholder_named_original
            config = json.loads((root / 'home/generic-config.json').read_text())
            assert config['schemaVersion'] == 7
            assert all('modelRecordKey' not in runtime for runtime in config['runtimeInstances'])
            print(json.dumps({'acceptance': 'PASSED', 'schema': 7,
                'historyWithoutProvidersOrExistingCwd': True,
                'nativeTitleAndFirstMessageFallback': True,
                'typedRoleAndUnixMillis': True,
                'reasoningAndCompletedToolOutput': True,
                'modelSelectionDoesNotPrepareExecution': True,
                'foreignHistoryRejected': True,
                'preparationFailurePreservesHistoryAndChoice': True,
                'enabledSourcesAndDefaultPreserveSchema7': True,
                'nativeCreatedAndUpdatedDates': True, 'isolatedHistoryFailures': True, 'originalHistoryUnchanged': True,
                'realServicesCalled': False}))
        finally:
            if application is not None:
                application.shutdown()
            os.environ.clear()
            os.environ.update(environment)


if __name__ == '__main__':
    main()
