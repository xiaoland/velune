#!/usr/bin/env python3
"""Manual acceptance of projects with several folders, never a CI entry point.

Seeds Pi sessions through the selected Pi SDK in a temporary HOME: in a Git
repository, in a linked worktree `git worktree add` made, in a Claude Code
`.claude/worktrees/<name>` folder that is then removed, and in two other
folders. Checks through the generated bindings that the worktrees' sessions
are listed under the repository's project, that a folder taken into a
project brings its sessions and that the project filter, a folded group and
a move follow a new primary folder, that a folder belongs to one project
only, and that an organization file of the older schema starts afresh. With
--seed DIR the fixture stays in DIR, its repository's project named and
holding another folder, for checking the Mac app by hand with
VELUNE_HOME=DIR/home. No credentials, real histories, or model requests.
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
const specs = JSON.parse(process.argv[2]);
const paths = [];
for (const spec of specs) {
  const session = SessionManager.create(spec.cwd, spec.dir);
  session.appendMessage({role:'user', content:[{type:'text',text:spec.title}], timestamp:Date.now()});
  session.appendMessage({role:'assistant', content:[{type:'text',text:'synthetic reply'}],
    api:'openai-completions',provider:'synthetic',model:'synthetic',stopReason:'stop',timestamp:Date.now()});
  paths.push(session.getSessionFile());
}
console.log(JSON.stringify(paths));
'''


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    for name in ('bundle', 'bindings', 'node', 'pi'):
        parser.add_argument('--' + name, type=Path, required=True)
    parser.add_argument('--seed', type=Path, help='keep the fixture in this new folder, its project shaped, for the Mac app')
    args = parser.parse_args()
    for name in ('bundle', 'bindings', 'node', 'pi'):
        path = getattr(args, name)
        if not path.is_absolute() or not path.exists():
            parser.error('absolute existing paths required')
    with tempfile.TemporaryDirectory(prefix='velune-project-folders-') as directory:
        root = args.seed.resolve() if args.seed else Path(directory).resolve()
        root.mkdir(parents=True, exist_ok=True)
        for name in ('bindings', 'seed', 'pi', 'repo', 'other', 'extra'):
            (root / name).mkdir()
        package = args.pi.resolve()
        while not (package / 'package.json').is_file():
            package = package.parent
        (root / 'seed/node_modules/@earendil-works').mkdir(parents=True)
        (root / 'seed/node_modules/@earendil-works/pi-coding-agent').symlink_to(package)
        (root / 'seed/create.mjs').write_text(SEED)
        fixture_env = {'HOME': str(root), 'PATH': '/usr/bin:/bin'}

        repo, other, extra = root / 'repo', root / 'other', root / 'extra'
        linked = root / 'linked'
        job = repo / '.claude/worktrees/job-a'
        git = ['git', '-c', 'user.name=Synthetic', '-c', 'user.email=synthetic@example.invalid']
        subprocess.check_call(git + ['init', '-q', str(repo)], env=fixture_env)
        subprocess.check_call(git + ['-C', str(repo), 'commit', '-q', '--allow-empty', '-m', 'synthetic'], env=fixture_env)
        subprocess.check_call(git + ['-C', str(repo), 'worktree', 'add', '-q', str(linked)], env=fixture_env)
        job.mkdir(parents=True)

        def pi_dir(cwd):
            return root / 'pi/sessions' / ('--' + str(cwd).lstrip('/').replace('/', '-') + '--')
        specs = [{'cwd': str(cwd), 'dir': str(pi_dir(cwd)), 'title': title}
                 for cwd, title in ((repo, 'repo 0'), (repo, 'repo 1'), (job, 'job a'), (linked, 'linked'),
                                    (other, 'other 0'), (extra, 'extra 0'))]
        subprocess.check_output([str(args.node), str(root / 'seed/create.mjs'), json.dumps(specs)], env=fixture_env, text=True)
        # Claude Code removes a worktree once its work is done; the layout still tells.
        shutil.rmtree(repo / '.claude')

        (root / 'bindings/velune_bindings.py').write_bytes((args.bindings / 'velune_bindings.py').read_bytes())
        (root / 'bindings/libvelune_bindings.dylib').symlink_to(args.bundle / 'Contents/Frameworks/libvelune_bindings.dylib')
        os.environ.clear()
        os.environ.update(fixture_env)
        spec = importlib.util.spec_from_file_location('velune_bindings', root / 'bindings/velune_bindings.py')
        b = importlib.util.module_from_spec(spec)
        sys.modules[spec.name] = b
        spec.loader.exec_module(b)
        home = root / 'home'

        def open_application():
            return b.VeluneApplication.open(b.BindingOptions(home_directory=str(home),
                                                             resources_directory=str(args.bundle / 'Contents/Resources')))
        K = b.BindingConversationGroupKey
        P = b.BindingConversationBrowserProject

        def preferences(**changes):
            value = dict(grouping=b.BindingConversationBrowserGrouping.PROJECT, sort=b.BindingConversationBrowserSort.UPDATED,
                         oldest_first=False, runtime_id=None, project=P.ALL(), collapsed=[])
            value.update(changes)
            return b.BindingConversationBrowserPreferences(**value)

        def groups(snapshot):
            # By each project group's primary folder.
            return {g.key.path: sorted(c.conversation.title for c in snapshot.conversations if c.group == g.key)
                    for g in snapshot.conversation_groups}

        def project(snapshot, path):
            return next(p for p in snapshot.conversation_projects if p.path == str(path))

        application = open_application()
        try:
            application.set_conversation_browser_preferences(preferences())
            application.upsert_runtime(b.BindingRuntimeInstance(enabled=True, id='pi', name='Synthetic Pi', type_id='pi-1.0',
                gateway_id='default', settings={'binary': str(args.pi), 'nodeBinary': str(args.node), 'agentDir': str(root / 'pi')}))
            listed = application.list()
            assert groups(listed) == {str(repo): ['job a', 'linked', 'repo 0', 'repo 1'],
                                      str(other): ['other 0'], str(extra): ['extra 0']}, groups(listed)
            assert project(listed, repo).worktrees == sorted([str(job), str(linked)]), project(listed, repo).worktrees
            print('PASS worktrees: a linked worktree and a removed .claude/worktrees folder list under the repository', flush=True)

            application.set_conversation_browser_preferences(
                preferences(project=P.PATH(str(extra)), collapsed=[K.PROJECT(str(extra))]))
            taken = application.set_project(str(repo), str(repo), ' Repo ', [str(extra)])
            # Folded, the group holds its conversations unloaded.
            assert [(g.key.path, g.total, g.collapsed) for g in taken.conversation_groups] == [(str(repo), 5, True)]
            assert taken.conversation_browser_preferences.project == P.PATH(str(repo))
            assert taken.conversation_browser_preferences.collapsed == [K.PROJECT(str(repo))]
            shaped = project(taken, repo)
            assert (shaped.name, shaped.folders) == ('Repo', [str(extra)]), shaped
            print('PASS folders: a folder taken in brings its sessions; the filter and the folded group follow', flush=True)

            application.set_conversation_browser_preferences(preferences())
            other_row = next(c for c in application.list().conversations if c.conversation.title == 'other 0')
            reference = b.BindingConversationReference(runtime_id='pi', conversation_id=other_row.conversation.id)
            moved = application.organize_conversations([reference], b.BindingConversationOrganizationChange.MOVE_TO_PROJECT(str(extra)))
            row = next(c for c in moved.conversations if c.conversation.title == 'other 0')
            assert row.moved and row.project == str(repo), (row.moved, row.project)

            application.set_conversation_browser_preferences(preferences(project=P.PATH(str(repo))))
            primary = application.set_project(str(repo), str(extra), 'Repo', [str(repo)])
            assert groups(primary) == {str(extra): ['extra 0', 'job a', 'linked', 'other 0', 'repo 0', 'repo 1']}, groups(primary)
            assert primary.conversation_browser_preferences.project == P.PATH(str(extra))
            assert next(c for c in primary.conversations if c.conversation.title == 'other 0').project == str(extra)
            print('PASS primary: the project, its filter, and a move into it follow the new primary folder', flush=True)

            try:
                application.set_project(str(other), str(other), None, [str(repo)])
            except b.BindingError as error:
                assert '已属于项目「Repo」' in str(error), error
            else:
                raise AssertionError('a folder joined two projects')

            application.set_conversation_browser_preferences(preferences())
            apart = application.set_project(str(extra), str(extra), None, [])
            assert groups(apart) == {str(repo): ['job a', 'linked', 'repo 0', 'repo 1'],
                                     str(extra): ['extra 0', 'other 0']}, groups(apart)
            assert not any(p.name or p.folders for p in apart.conversation_projects)
            print('PASS one project per folder; a project left plain gives its folders back', flush=True)
        finally:
            application.shutdown()

        organization = home / 'conversation-organization.json'
        organization.write_text(json.dumps({'schemaVersion': 1, 'organization': {
            'readBaselineUnixMs': 0, 'conversations': [], 'projectAliases': [{'path': str(repo), 'alias': 'Old'}]}}))
        application = open_application()
        try:
            assert not any(p.name for p in application.list().conversation_projects)
        finally:
            application.shutdown()
        assert json.loads(organization.read_text())['schemaVersion'] == 2
        print('PASS schema: an organization file of the older schema starts afresh', flush=True)
        if args.seed:
            application = open_application()
            try:
                application.set_project(str(repo), str(repo), 'Repo', [str(extra)])
            finally:
                application.shutdown()
            print('SEEDED', home, flush=True)


if __name__ == '__main__':
    main()
