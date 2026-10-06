#!/usr/bin/env python3
"""Explicit isolated native session rename/delete acceptance; never a CI entry point.
Pi uses its SDK, Codex uses the fixed app-server, and no model service is invoked.
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
import uuid


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    for name in ('bundle', 'bindings', 'node', 'codex'):
        parser.add_argument('--' + name, type=Path, required=True)
    args = parser.parse_args()
    for path in vars(args).values():
        if not path.is_absolute() or not path.exists():
            parser.error('absolute existing paths required')
    original_env = dict(os.environ)
    application = None
    with tempfile.TemporaryDirectory(prefix='velune-native-session-management-') as directory:
        root = Path(directory)
        resources = args.bundle / 'Contents/Resources'
        for name in ('bindings', 'project', 'home', 'seed', 'pi', 'codex', 'foreign'):
            (root / name).mkdir()
        (root / 'seed/node_modules').symlink_to(resources / 'node_modules')
        seed = root / 'seed/create.mjs'
        seed.write_text("""import {SessionManager} from '@earendil-works/pi-coding-agent';
const [cwd,dir] = process.argv.slice(2);
const manager=SessionManager.create(cwd,dir);
manager.appendMessage({role:'user',content:[{type:'text',text:'SYNTHETIC_NATIVE_HISTORY'}],timestamp:Date.now()});
manager.appendMessage({role:'assistant',content:[{type:'text',text:'SYNTHETIC_REPLY'}],api:'openai-completions',provider:'synthetic',model:'synthetic',stopReason:'stop',timestamp:Date.now()});
console.log(manager.getSessionFile());
""")
        env = {'HOME': str(root / 'home'), 'PATH': str(args.node.parent) + ':/usr/bin:/bin'}
        def pi_seed(home):
            sessions = home / 'sessions/project'
            sessions.mkdir(parents=True, exist_ok=True)
            return Path(subprocess.check_output([str(args.node), str(seed), str(root / 'project'), str(sessions)], text=True, env=env).strip())
        pi_paths = [pi_seed(root / 'pi'), pi_seed(root / 'pi')]
        foreign = pi_seed(root / 'foreign')
        def codex_seed():
            identity = str(uuid.uuid4())
            path = root / 'codex/sessions/2026/10/06' / ('rollout-2026-10-06T00-00-00-' + identity + '.jsonl')
            path.parent.mkdir(parents=True, exist_ok=True)
            records = [
                {'timestamp':'2026-10-06T00:00:00Z','type':'session_meta','payload':{'id':identity,'timestamp':'2026-10-06T00:00:00Z','cwd':str(root / 'project'),'originator':'codex_cli_rs','cli_version':'0.159.3','source':'cli','model_provider':'openai'}},
                {'timestamp':'2026-10-06T00:00:01Z','type':'turn_context','payload':{'cwd':str(root / 'project'),'approval_policy':'on-request','sandbox_policy':{'type':'workspace-write'},'model':'synthetic'}},
                {'timestamp':'2026-10-06T00:00:02Z','type':'response_item','payload':{'type':'message','role':'user','content':[{'type':'input_text','text':'SYNTHETIC_NATIVE_HISTORY'}]}}
            ]
            path.write_text(''.join(json.dumps(record) + '\n' for record in records))
            return identity, path
        codex_items = [codex_seed(), codex_seed()]
        shutil.copy(args.bindings / 'velune_bindings.py', root / 'bindings')
        (root / 'bindings/libvelune_bindings.dylib').symlink_to(args.bundle / 'Contents/Frameworks/libvelune_bindings.dylib')
        os.environ.clear(); os.environ.update(env)
        try:
            spec = importlib.util.spec_from_file_location('velune_bindings', root / 'bindings/velune_bindings.py')
            b = importlib.util.module_from_spec(spec); sys.modules[spec.name] = b; spec.loader.exec_module(b)
            options = b.BindingOptions(home_directory=str(root / 'home'), resources_directory=str(resources))
            application = b.VeluneApplication.open(options)
            def rejected(operation):
                try: operation()
                except b.BindingError: return
                raise AssertionError('invalid native mutation unexpectedly accepted')
            reports = []
            for family, type_id, binary, items in [
                ('pi', 'pi-1.0.2', resources / 'node_modules/@earendil-works/pi-coding-agent/dist/bundle/cli.js', [(str(path), path) for path in pi_paths]),
                ('codex', 'codex-0.159.3', args.codex, codex_items)
            ]:
                application.upsert_runtime(b.BindingRuntimeInstance(id=family, name=family, type_id=type_id, gateway_id='default', settings={'binary':str(binary),'nodeBinary':str(args.node),'agentDir':str(root / family)}))
                application.select_runtime(family)
                descriptor = next(item for item in application.list().runtime_types if item.id == type_id)
                assert descriptor.can_rename_conversations and descriptor.can_delete_conversations
                identities = [family + ':' + identity for identity, _ in items]
                originals = [path.read_bytes() for _, path in items]
                application.open_conversation(family, identities[0])
                rejected(lambda: application.rename_conversation(family, identities[0], '  '))
                rejected(lambda: application.delete_conversation(family, 'other:' + items[0][0]))
                assert items[0][1].read_bytes() == originals[0]
                renamed = application.rename_conversation(family, identities[0], '原生名称 · SYNTHETIC')
                assert next(item for item in renamed.conversations if item.id == identities[0]).title == '原生名称 · SYNTHETIC'
                application.rename_conversation(family, identities[1], 'OTHER_SYNTHETIC_NATIVE')
                current = application.snapshot(family).snapshot
                assert current.conversation.id == identities[0] and current.conversation.title == '原生名称 · SYNTHETIC'
                if family == 'codex':
                    assert all(path.read_bytes() == original for (_, path), original in zip(items, originals)), 'rename changed native messages'
                    assert any(json.loads(line).get('thread_name') == '原生名称 · SYNTHETIC' for line in (root / 'codex/session_index.jsonl').read_text().splitlines())
                    # An explicit empty native index name clears only that name;
                    # the adapter must derive a title from the original history.
                    index = root / 'codex/session_index.jsonl'
                    with index.open('a') as stream:
                        stream.write(json.dumps({'id':items[0][0],'thread_name':'','updated_at':'2026-10-06T01:00:00Z'}) + '\n')
                    assert next(item for item in application.list().conversations if item.id == identities[0]).title == 'SYNTHETIC_NATIVE_HISTORY'
                    application.rename_conversation(family, identities[0], '原生名称 · SYNTHETIC')
                    assert next(item for item in application.list().conversations if item.id == identities[0]).title == '原生名称 · SYNTHETIC'
                    assert items[0][1].read_bytes() == originals[0]
                else:
                    assert all(path.read_bytes().startswith(original) for (_, path), original in zip(items, originals)), 'rename rewrote existing native records'
                    rejected(lambda: application.rename_conversation('pi', 'pi:' + str(foreign), 'DO_NOT_WRITE'))
                    rejected(lambda: application.delete_conversation('pi', 'pi:' + str(foreign)))
                    outside = foreign.read_bytes()
                    link = root / 'pi/sessions/project/outside.jsonl'; link.symlink_to(foreign)
                    rejected(lambda: application.rename_conversation('pi', 'pi:' + str(link), 'DO_NOT_WRITE'))
                    rejected(lambda: application.delete_conversation('pi', 'pi:' + str(link)))
                    assert foreign.read_bytes() == outside and link.is_symlink()
                    # A multiply-linked source is refused after closing any idle
                    # writer; failed deletion must retain the loaded history.
                    hardlink = root / 'hardlink.jsonl'; os.link(items[0][1], hardlink)
                    before_failure = items[0][1].read_bytes()
                    rejected(lambda: application.delete_conversation('pi', identities[0]))
                    assert application.snapshot('pi').snapshot.conversation.id == identities[0]
                    assert items[0][1].read_bytes() == before_failure
                    hardlink.unlink()
                application.shutdown(); application = b.VeluneApplication.open(options)
                listed = application.list()
                assert next(item for item in listed.conversations if item.id == identities[0]).title == '原生名称 · SYNTHETIC'
                application.open_conversation(family, identities[0])
                application.delete_conversation(family, identities[1])
                assert application.snapshot(family).snapshot.conversation.id == identities[0]
                application.delete_conversation(family, identities[0])
                assert application.snapshot(family).snapshot is None
                assert not any(item.id in identities for item in application.list().conversations)
                assert all(not path.exists() for _, path in items), 'native source files remain after deletion'
                rejected(lambda: application.open_conversation(family, identities[0]))
                reports.append(family)
            dsh = next(item for item in application.list().runtime_types if item.id == 'dsh-acp-0.2.0-rc.2')
            assert not dsh.can_rename_conversations and not dsh.can_delete_conversations
            application.upsert_runtime(b.BindingRuntimeInstance(id='deepseek',name='deepseek',type_id=dsh.id,gateway_id='default',settings={'binary':str(root / 'unstarted-dsh'),'nodeBinary':str(args.node),'agentDir':str(root / 'dsh')}))
            rejected(lambda: application.rename_conversation('deepseek','deepseek:synthetic','DO_NOT_WRITE'))
            rejected(lambda: application.delete_conversation('deepseek','deepseek:synthetic'))
            print(json.dumps({'nativeMutationVerified':reports,'dshNativeMutationExposed':False,'foreignAndSymlinkRejected':True,'nativeIDsPreservedOnRename':True,'reopenedNamesPersist':True,'upstreamRequests':0},ensure_ascii=False))
        finally:
            if application is not None: application.shutdown()
            os.environ.clear(); os.environ.update(original_env)

if __name__ == '__main__': main()
