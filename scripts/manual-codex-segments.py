#!/usr/bin/env python3
"""Check that a Codex thread written in several rollout segments shows as
running elsewhere from its latest segment. Synthetic files in a temporary
CODEX_HOME only; a child process holds the thread writer locks as Codex does.

    python3 scripts/manual-codex-segments.py --bundle <Velune.app> --bindings <dir> --node <node> --codex <codex>
"""
import argparse
import importlib.util
import json
import os
import subprocess
import sys
import tempfile
import time
from pathlib import Path

HOLD = """
import fcntl, sys, time
files = [open(path, 'w') for path in sys.argv[1:]]
for file in files:
    fcntl.lockf(file, fcntl.LOCK_EX)
print('held', flush=True)
time.sleep(600)
"""


def records(*events):
    lines = [{'type': 'session_meta', 'payload': {'id': 'synthetic'}}]
    lines += [{'type': 'event_msg', 'payload': {'type': event}} for event in events]
    return ''.join(json.dumps(line) + '\n' for line in lines)


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    for name in ('bundle', 'bindings', 'node', 'codex'):
        parser.add_argument('--' + name, type=Path, required=True)
    args = parser.parse_args()
    with tempfile.TemporaryDirectory(prefix='velune-codex-segments-') as directory:
        root = Path(directory).resolve()
        codex_home = root / 'codex'
        # running: its first segment finished, the later one runs a turn.
        # finished: an older segment was cut mid-turn; the later one finished.
        running, finished = '01a12882-0000-7000-8000-000000000001', '01a12882-0000-7000-8000-000000000002'
        segment = '01a13018-0000-7000-8000-00000000000%d'
        files = [
            ('2026/10/11', 'rollout-2026-10-11T09-00-00-%s.jsonl' % running, records('task_started', 'task_complete'), 100),
            ('2026/10/12', 'rollout-2026-10-12T20-00-00-%s_%s.jsonl' % (running, segment % 1), records('task_started'), 200),
            ('2026/10/11', 'rollout-2026-10-11T09-00-00-%s.jsonl' % finished, records('task_started'), 100),
            ('2026/10/12', 'rollout-2026-10-12T20-00-00-%s_%s.jsonl' % (finished, segment % 2), records('task_started', 'turn_complete'), 200),
        ]
        base = time.time() - 1000
        for day, name, text, offset in files:
            path = codex_home / 'sessions' / day / name
            path.parent.mkdir(parents=True, exist_ok=True)
            path.write_text(text)
            os.utime(path, (base + offset, base + offset))
        locks = codex_home / 'thread-writer-locks'
        locks.mkdir()
        holder = subprocess.Popen([sys.executable, '-c', HOLD, str(locks / (running + '.lock')), str(locks / (finished + '.lock'))],
                                  stdout=subprocess.PIPE, text=True)
        try:
            assert holder.stdout.readline().strip() == 'held'
            # The bindings load the library beside them: the bundle's.
            (root / 'bindings').mkdir()
            (root / 'bindings/velune_bindings.py').write_bytes((args.bindings / 'velune_bindings.py').read_bytes())
            (root / 'bindings/libvelune_bindings.dylib').symlink_to(args.bundle / 'Contents/Frameworks/libvelune_bindings.dylib')
            spec = importlib.util.spec_from_file_location('velune_bindings', root / 'bindings/velune_bindings.py')
            b = importlib.util.module_from_spec(spec)
            sys.modules[spec.name] = b
            spec.loader.exec_module(b)
            app = b.VeluneApplication.open(b.BindingOptions(home_directory=str(root / 'velune'),
                                                             resources_directory=str(args.bundle / 'Contents/Resources')))
            app.upsert_runtime(b.BindingRuntimeInstance(enabled=True, id='codex', name='Synthetic Codex', type_id='codex-0.159.3',
                gateway_id='default', settings={'binary': str(args.codex), 'nodeBinary': str(args.node), 'agentDir': str(codex_home)}))
            elsewhere = sorted(item.conversation_id for item in app.conversation_activity(0).active
                               if item.run_state == b.BindingRunState.ELSEWHERE)
            assert elsewhere == ['codex:' + running], elsewhere
            app.shutdown()
        finally:
            holder.kill()
            holder.wait()
    print('PASS Codex rollout segments: the latest segment decides, under the thread id')


if __name__ == '__main__':
    main()
