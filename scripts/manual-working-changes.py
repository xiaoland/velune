#!/usr/bin/env python3
"""Manual acceptance for reviewing a conversation's working changes.

A temporary git repository gets every kind of change (modified, staged
addition, deletion, rename, untracked, binary, conflict, names with spaces
and non-ASCII characters, a missing final newline). A synthetic Pi 1.0.2
conversation whose working directory is a subdirectory of it is read through
the UniFFI application. Checks the listed changes and counts, typed diff
lines, repository root resolution, a directory outside git and refusal of
paths outside the repository. No real credential, provider, repository or
session is read; this is not an automated test.
"""
import argparse
import importlib.util
import json
import pathlib
import shutil
import subprocess
import sys
import tempfile


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    for name in ('library', 'bindings', 'bundle', 'node', 'pi'):
        parser.add_argument('--' + name, type=pathlib.Path, required=True)
    args = parser.parse_args()
    sdk = None
    for directory in args.pi.resolve().parents:
        package = directory / 'package.json'
        if package.is_file() and json.loads(package.read_text()).get('name') == '@earendil-works/pi-coding-agent':
            sdk = directory / json.loads(package.read_text())['exports']['.']['import']
            break
    if sdk is None:
        parser.error('--pi must belong to the Pi 1.0.2 package')

    with tempfile.TemporaryDirectory(prefix='velune-changes-') as directory:
        root = pathlib.Path(directory).resolve()
        repo, outside = root / 'repo', root / 'outside'
        for path in (root / 'home', root / 'bindings', root / 'resources', repo / 'app', outside):
            path.mkdir(parents=True)
        (root / 'resources/node_modules').symlink_to(args.bundle / 'Contents/Resources/node_modules')
        for resource in (args.bundle / 'Contents/Resources').glob('*.mjs'):
            shutil.copy(resource, root / 'resources')

        env = {'HOME': str(root / 'home'), 'PATH': '/usr/bin:/bin', 'GIT_CONFIG_NOSYSTEM': '1',
               'GIT_AUTHOR_NAME': 'Synthetic', 'GIT_AUTHOR_EMAIL': 'synthetic@example.invalid',
               'GIT_COMMITTER_NAME': 'Synthetic', 'GIT_COMMITTER_EMAIL': 'synthetic@example.invalid'}

        def git(*command, check=True):
            return subprocess.run(['git', '-C', str(repo), *command], env=env, check=check, capture_output=True, text=True)

        git('init', '-q', '-b', 'main')
        (repo / 'app/main.py').write_text('one\ntwo\nthree\n')
        (repo / 'removed.txt').write_text('gone\n')
        (repo / 'old name.txt').write_text('same content\n' * 5)
        (repo / 'conflict.txt').write_text('base\n')
        (repo / 'image.bin').write_bytes(bytes(range(256)))
        git('add', '-A')
        git('commit', '-q', '-m', 'base')
        git('checkout', '-q', '-b', 'other')
        (repo / 'conflict.txt').write_text('other\n')
        git('commit', '-q', '-am', 'other')
        git('checkout', '-q', 'main')
        (repo / 'conflict.txt').write_text('main\n')
        git('commit', '-q', '-am', 'main')
        git('merge', '-q', 'other', check=False)

        (repo / 'app/main.py').write_text('one\nTWO\nthree\nfour')
        (repo / 'staged.txt').write_text('new\n')
        git('add', 'staged.txt')
        (repo / 'removed.txt').unlink()
        git('mv', 'old name.txt', '新 名字.txt')
        (repo / 'image.bin').write_bytes(bytes(reversed(range(256))))
        (repo / 'app/未跟踪 文件.md').write_text('a\nb\n')

        seed = root / 'seed.mjs'
        seed.write_text('import {SessionManager} from ' + json.dumps(sdk.resolve().as_uri()) + ''';
const [cwd, dir, title] = process.argv.slice(2); const manager = SessionManager.create(cwd, dir);
const usage = {input:1,output:1,cacheRead:0,cacheWrite:0,totalTokens:2,cost:{input:0,output:0,cacheRead:0,cacheWrite:0,total:0}};
manager.appendMessage({role:'user',content:[{type:'text',text:title}],timestamp:Date.now()});
manager.appendMessage({role:'assistant',content:[{type:'text',text:'ok'}],api:'openai-completions',provider:'synthetic',model:'synthetic',stopReason:'stop',usage,timestamp:Date.now()});
manager.appendSessionInfo(title);
''')
        for cwd, title in ((repo / 'app', 'IN_REPO'), (outside, 'OUTSIDE')):
            subprocess.run([str(args.node), str(seed), str(cwd), str(root / 'runtime/sessions' / title), title],
                           env=dict(env, PATH=str(args.node.parent) + ':/usr/bin:/bin'), check=True, capture_output=True)

        shutil.copy2(args.bindings / 'velune_bindings.py', root / 'bindings/velune_bindings.py')
        shutil.copy2(args.library, root / 'bindings' / args.library.name)
        spec = importlib.util.spec_from_file_location('velune_changes', root / 'bindings/velune_bindings.py')
        b = importlib.util.module_from_spec(spec)
        sys.modules[spec.name] = b
        spec.loader.exec_module(b)
        app = b.VeluneApplication.open(b.BindingOptions(home_directory=str(root / 'app-home'), resources_directory=str(root / 'resources')))
        try:
            app.upsert_runtime(b.BindingRuntimeInstance(
                enabled=True, id='pi', name='Pi', type_id='pi-1.0', gateway_id='default',
                settings={'binary': str(args.pi), 'nodeBinary': str(args.node), 'agentDir': str(root / 'runtime')}))
            ids = {c.title: c.id for c in [_row.conversation for _row in app.list().conversations]}

            changes = app.working_changes('pi', ids['IN_REPO'])
            assert changes.repository == str(repo) and changes.directory == str(repo / 'app') and changes.branch == 'main', changes
            files = {file.path: file for file in changes.files}
            kinds = {path: file.kind for path, file in files.items() if path != '新 名字.txt'}
            assert kinds == {
                'app/main.py': b.BindingChangeKind.MODIFIED(),
                'staged.txt': b.BindingChangeKind.ADDED(),
                'removed.txt': b.BindingChangeKind.DELETED(),
                'image.bin': b.BindingChangeKind.MODIFIED(),
                'app/未跟踪 文件.md': b.BindingChangeKind.UNTRACKED(),
                'conflict.txt': b.BindingChangeKind.CONFLICTED(),
            }, kinds
            renamed = files['新 名字.txt'].kind
            assert renamed.is_renamed() and renamed._from == 'old name.txt', renamed
            assert files['staged.txt'].staged and not files['staged.txt'].unstaged
            assert files['app/main.py'].unstaged and not files['app/main.py'].staged
            assert (files['app/main.py'].additions, files['app/main.py'].deletions) == (2, 1)
            assert files['image.bin'].additions is None and files['app/未跟踪 文件.md'].additions is None
            print('PASS every kind of change is listed with staging and line counts from the repository root', flush=True)

            diff = app.file_diff('pi', ids['IN_REPO'], 'app/main.py')
            lines = [(line.kind, line.text) for line in diff.lines]
            K = b.BindingDiffLineKind
            assert lines[0][0] == K.HUNK and (K.REMOVED, 'two') in lines and (K.ADDED, 'TWO') in lines \
                and (K.ADDED, 'four') in lines and (K.CONTEXT, 'one') in lines and any(kind == K.NOTICE for kind, _ in lines), lines
            assert not any(text.startswith(('diff --git', '+++', '---', 'index ')) for _, text in lines), lines
            untracked = app.file_diff('pi', ids['IN_REPO'], 'app/未跟踪 文件.md')
            assert [(line.kind, line.text) for line in untracked.lines][1:] == [(K.ADDED, 'a'), (K.ADDED, 'b')], untracked.lines
            binary = app.file_diff('pi', ids['IN_REPO'], 'image.bin')
            assert [line.kind for line in binary.lines] == [K.NOTICE], binary.lines
            deleted = app.file_diff('pi', ids['IN_REPO'], 'removed.txt')
            assert (K.REMOVED, 'gone') in [(line.kind, line.text) for line in deleted.lines]
            print('PASS diffs are typed lines for modified, untracked, binary and deleted files', flush=True)

            for path in ('../outside', str(repo / 'app/main.py')):
                try:
                    app.file_diff('pi', ids['IN_REPO'], path)
                    raise AssertionError('path outside the repository accepted: ' + path)
                except b.BindingError:
                    pass
            plain = app.working_changes('pi', ids['OUTSIDE'])
            assert plain.repository is None and plain.files == [] and plain.directory == str(outside)
            status_before = git('status', '--porcelain=v2').stdout
            assert not (repo / '.git/index.lock').exists() and git('status', '--porcelain=v2').stdout == status_before
            print('PASS paths outside the repository are refused; a directory outside git has no changes', flush=True)
        finally:
            app.shutdown()


if __name__ == '__main__':
    main()
