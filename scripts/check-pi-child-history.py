#!/usr/bin/env python3
"""Exercise Pi SDK discovery for validated legacy and current sub-agent layouts."""
import argparse
import json
import os
from pathlib import Path
import shutil
import subprocess
import tempfile

parser = argparse.ArgumentParser(description=__doc__)
parser.add_argument('--pi', default=shutil.which('pi'), help='Configured Pi 1.x CLI installation')
args = parser.parse_args()
if not args.pi:
    parser.error('Pi CLI is required; supply --pi')
helper = Path(__file__).resolve().parents[1] / 'packages/agent-runtime/resources/pi_sessions.mjs'
with tempfile.TemporaryDirectory(prefix='velune-pi-child-history-') as directory:
    root = Path(directory)
    sessions = root / 'sessions' / '--synthetic--'
    sessions.mkdir(parents=True)
    parent = sessions / 'parent.jsonl'
    child = sessions / 'parent' / 'forks' / 'run' / 'child.jsonl'
    child.parent.mkdir(parents=True)
    ordinary = sessions / 'ordinary.jsonl'
    mismatch = child.parent / 'mismatch.jsonl'
    run_id = '01234567-89ab-4cde-8fab-0123456789ab'
    modern = sessions / 'parent' / run_id / 'run-0' / 'session.jsonl'
    modern_second = modern.parent.parent / 'run-1' / 'session.jsonl'
    overflow_slot = modern.parent.parent / 'run-4294967296' / 'session.jsonl'
    padded_slot = modern.parent.parent / 'run-01' / 'session.jsonl'
    negative_slot = modern.parent.parent / 'run--1' / 'session.jsonl'
    nested = modern.with_suffix('') / run_id / 'run-0' / 'session.jsonl'
    modern_mismatch = sessions / 'parent' / '11234567-89ab-4cde-8fab-0123456789ab' / 'run-0' / 'session.jsonl'
    stray = sessions / 'stray' / run_id / 'run-0' / 'session.jsonl'
    wrong_run = sessions / 'parent' / 'not-a-run-uuid' / 'run-0' / 'session.jsonl'
    wrong_name = modern.parent / 'intruder.jsonl'
    for path, identity, ancestor in [(parent, 'parent', None), (child, 'child', parent),
                                     (ordinary, 'ordinary', parent), (mismatch, 'mismatch', ordinary),
                                     (modern, 'modern', parent), (nested, 'nested', modern),
                                     (modern_second, 'modern-second', parent),
                                     (modern_mismatch, 'wrong-parent', ordinary),
                                     (stray, 'stray', parent), (wrong_run, 'wrong-run', parent),
                                     (wrong_name, 'wrong-name', parent),
                                     (overflow_slot, 'overflow', parent), (padded_slot, 'padded', parent),
                                     (negative_slot, 'negative', parent)]:
        path.parent.mkdir(parents=True, exist_ok=True)
        header = dict(type='session', version=3, id=identity, timestamp='2026-11-11T00:00:00Z', cwd='/synthetic')
        if ancestor:
            header['parentSession'] = str(ancestor)
        path.write_text(json.dumps(header) + '\n')
    with modern.open('a') as handle:
        records = [
            dict(type='message', id='modern-user', parentId=None, timestamp='2026-11-11T00:00:01Z',
                 message=dict(role='user', content=[dict(type='text', text='First'), dict(type='text', text='prompt')],
                              timestamp=1794355201000)),
            dict(type='session_info', id='modern-name', parentId='modern-user', timestamp='2026-11-11T00:00:02Z',
                 name='Synthetic child name'),
        ]
        handle.writelines(json.dumps(record) + '\n' for record in records)
    symlink_cases = []
    try:
        linked_run = sessions / 'parent' / '21234567-89ab-4cde-8fab-0123456789ab'
        linked_run.symlink_to(modern.parent.parent, target_is_directory=True)
        linked_workspace = root / 'sessions' / '--linked-workspace--'
        linked_workspace.symlink_to(sessions, target_is_directory=True)
        linked_file = sessions / 'parent' / '31234567-89ab-4cde-8fab-0123456789ab' / 'run-0' / 'session.jsonl'
        linked_file.parent.mkdir(parents=True)
        linked_file.symlink_to(modern)
        symlink_cases = [linked_run / 'run-0' / 'session.jsonl', linked_workspace / 'parent.jsonl', linked_file]
    except OSError:
        pass  # Windows may not permit creating symlinks.
    wanted = [parent, child, ordinary, mismatch, modern, modern_second, nested, modern_mismatch, stray, wrong_run, wrong_name, overflow_slot, padded_slot, negative_slot, *symlink_cases]
    result = subprocess.run(['node', str(helper), '--agent-dir', str(root), '--cli', args.pi,
                             '--runtime-type', 'pi-1.0'],
                            input=json.dumps(dict(want=list(map(str, wanted)))),
                            text=True, capture_output=True, check=True)
    listing = json.loads(result.stdout)
    for field in ['files', 'sessions']:
        rows = {row['path']: row for row in listing[field]}
        assert rows[str(child)]['parentPath'] == str(parent)
        assert rows[str(modern)]['parentPath'] == str(parent)
        assert rows[str(modern_second)]['parentPath'] == str(parent)
        assert rows[str(nested)]['parentPath'] == str(modern)
        assert rows[str(ordinary)]['parentPath'] is None
        for rejected in [mismatch, modern_mismatch, stray, wrong_run, wrong_name, overflow_slot, padded_slot, negative_slot, *symlink_cases]:
            assert str(rejected) not in rows, f'unverified nested source was admitted: {rejected}'

    # Test the direct read with the same synthetic archive and selector
    # metadata. Instrument the real helper to count directory enumeration.
    probe = root / 'count-directory-reads.mjs'
    probe.write_text("""
import fs from 'node:fs/promises';
import { syncBuiltinESMExports } from 'node:module';
const root = process.env.VELUNE_PI_FIXTURE;
const original = fs.readdir;
let count = 0;
fs.readdir = (...args) => {
    if (String(args[0]).startsWith(root)) count++;
    return original(...args);
};
syncBuiltinESMExports();
process.on('exit', () => console.error('fixture-directory-reads=' + count));
""")
    helper_command = ['node', '--import', str(probe), str(helper), '--agent-dir', str(root),
                      '--cli', args.pi, '--runtime-type', 'pi-1.0']
    env = dict(os.environ, VELUNE_PI_FIXTURE=str(root))
    def read(path, *options):
        return subprocess.run([*helper_command, '--inspect-session', str(path), '--with-summary', *options],
                              env=env, text=True, capture_output=True)

    summary_fields = ('path', 'parentPath', 'cwd', 'name', 'createdUnixMs', 'modifiedUnixMs', 'firstMessage')
    indexed = {row['path']: row for row in listing['sessions']}
    for valid in [parent, child, ordinary, modern, modern_second, nested]:
        inspected = read(valid)
        assert inspected.returncode == 0, inspected.stderr
        summary = json.loads(inspected.stdout)['summary']
        assert summary == {key: indexed[str(valid)][key] for key in summary_fields}, summary
        assert 'fixture-directory-reads=0' in inspected.stderr
        if valid == modern:
            assert summary['name'] == 'Synthetic child name' and summary['firstMessage'] == 'First prompt'
            assert summary['modifiedUnixMs'] == 1794355201000
    for invalid in [mismatch, modern_mismatch, stray, wrong_run, wrong_name, overflow_slot,
                    padded_slot, negative_slot, *symlink_cases]:
        assert read(invalid).returncode != 0, f'unverified direct source was admitted: {invalid}'
    custom = root / 'custom'
    custom.mkdir()
    custom_session = custom / 'custom.jsonl'
    custom_session.write_text(parent.read_text())
    assert read(custom_session).returncode != 0
    result = read(custom_session, '--session-dir', str(custom))
    assert result.returncode == 0, result.stderr
    assert json.loads(result.stdout)['summary']['path'] == str(custom_session)
    assert read(parent, '--session-dir', str(custom)).returncode != 0

    # Cold inspection must recover child metadata from persisted native
    # entries even when compaction has removed the launch from effective model
    # context. Use a distinct current-format parent and exact child header.
    parent = sessions / 'subagent-parent.jsonl'
    run_id = '41234567-89ab-4cde-8fab-0123456789ab'
    child = parent.with_suffix('') / run_id / 'run-0' / 'session.jsonl'
    child.parent.mkdir(parents=True)
    header = dict(type='session', version=3, id='51234567-89ab-4cde-8fab-0123456789ab',
                  timestamp='2026-11-11T00:00:00Z', cwd='/synthetic', parentSession=str(parent))
    native_name = f'subagent-worker-{run_id}-1'
    child_entries = [
        header,
        dict(type='session_info', id='child-name', parentId=None, timestamp='2026-11-11T00:00:01Z', name=native_name),
    ]
    child.write_text(''.join(json.dumps(entry) + '\n' for entry in child_entries))

    task = 'Review the synthetic runtime boundary'
    call_id = 'subagent-tool-call'
    entries = [
        dict(type='session', version=3, id='61234567-89ab-4cde-8fab-0123456789ab',
             timestamp='2026-11-11T00:00:00Z', cwd='/synthetic'),
        dict(type='message', id='user-before', parentId=None, timestamp='2026-11-11T00:00:01Z',
             message=dict(role='user', content='A launch that will be compacted', timestamp=1762819201000)),
        dict(type='message', id='assistant-call', parentId='user-before', timestamp='2026-11-11T00:00:02Z',
             message=dict(role='assistant', content=[dict(type='toolCall', id=call_id, name='subagent',
                                                          arguments={'agent':'worker', 'task':task, 'async':True})],
                          timestamp=1762819202000)),
        dict(type='message', id='tool-result', parentId='assistant-call', timestamp='2026-11-11T00:00:03Z',
             message=dict(role='toolResult', toolCallId=call_id, toolName='subagent',
                          content=[dict(type='text', text='Started synthetic child')], isError=False,
                          details=dict(asyncId=run_id, runId=run_id, results=[]), timestamp=1762819203000)),
        dict(type='custom_message', id='notify', parentId='tool-result', timestamp='2026-11-11T00:00:04Z',
             customType='subagent-notify', content='Synthetic child completed', display=False,
             details=dict(runs=[dict(agent='worker', status='completed', runId=run_id,
                                     sessionLabel='Session file', sessionValue=str(child))])),
        dict(type='session_info', id='parent-name', parentId='notify', timestamp='2026-11-11T00:00:05Z',
             name='Synthetic parent native name'),
        # Self-retaining compaction hides all preceding entries from current
        # context while the append-only entry list retains them for inspection.
        dict(type='compaction', id='compact', parentId='parent-name', timestamp='2026-11-11T00:00:06Z',
             summary='Earlier activity was summarized.', firstKeptEntryId='compact', tokensBefore=1),
        dict(type='message', id='current-user', parentId='compact', timestamp='2026-11-11T00:00:07Z',
             message=dict(role='user', content=[dict(type='text', text='Continue after compaction')],
                          timestamp=1762819207000)),
    ]
    parent.write_text(''.join(json.dumps(entry) + '\n' for entry in entries))
    inspected = subprocess.run(['node', str(helper), '--agent-dir', str(root), '--cli', args.pi,
                                '--runtime-type', 'pi-1.0', '--inspect-session', str(parent)],
                               text=True, capture_output=True, check=True)
    parent_view = json.loads(inspected.stdout)
    assert parent_view['name'] == 'Synthetic parent native name'
    assert parent_view['subAgents'] == [dict(sessionFile=str(child),
                                             title=f'worker：{task}', failed=False)]
    display_messages = json.dumps(parent_view['messages'], ensure_ascii=False)
    assert task in display_messages, 'cold display history must retain the pre-compaction launch'
    inspected_child = subprocess.run(['node', str(helper), '--agent-dir', str(root), '--cli', args.pi,
                                      '--runtime-type', 'pi-1.0', '--inspect-session', str(child)],
                                     text=True, capture_output=True, check=True)
    child_view = json.loads(inspected_child.stdout)
    assert child_view['name'] == native_name, 'cold sub-agent metadata must not rewrite native child name'

    print('PASS legacy/current/nested Pi child discovery; strict parent ownership; cold sub-agent metadata survives compaction; native names unchanged')
