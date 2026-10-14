#!/usr/bin/env python3
"""Manual conversation list management acceptance, never a CI entry point.

Seeds Pi sessions in several projects through the selected Pi SDK and Codex
threads through a loopback Responses server and synthetic Claude Code JSONL,
all in a temporary HOME, then checks the list's window (groups, pages,
total), that only the sessions it shows are read, pins, read state, moves,
project names, Velune and Codex archives, search, and lookup through the
generated bindings. Wrappers of node and python3 record which sessions the
history helpers are asked to read. No credentials, real histories, or model
requests.
"""
import argparse
import importlib.util
import json
import os
from pathlib import Path
import subprocess
import sys
import tempfile
import threading
import time
from http.server import BaseHTTPRequestHandler, ThreadingHTTPServer

SEED = r'''
import {SessionManager} from '@earendil-works/pi-coding-agent';
const specs = JSON.parse(process.argv[2]);
const sleep = (ms) => new Promise((done) => setTimeout(done, ms));
const paths = [];
for (const spec of specs) {
  if (spec.append) {
    const session = SessionManager.open(spec.append);
    session.appendMessage({role:'user', content:[{type:'text',text:'SYNTHETIC_FOLLOW_UP'}], timestamp:Date.now()});
    paths.push(spec.append);
  } else {
    const session = SessionManager.create(spec.cwd, spec.dir);
    session.appendMessage({role:'user', content:[{type:'text',text:spec.title}], timestamp:Date.now()});
    session.appendMessage({role:'assistant', content:[{type:'text',text:'synthetic reply'}],
      api:'openai-completions',provider:'synthetic',model:'synthetic',stopReason:'stop',timestamp:Date.now()});
    session.appendSessionInfo(spec.title);
    paths.push(session.getSessionFile());
  }
  await sleep(5);
}
console.log(JSON.stringify(paths));
'''


class Upstream(BaseHTTPRequestHandler):
    def log_message(self, *_):
        pass

    def do_POST(self):
        self.rfile.read(int(self.headers.get("Content-Length", "0")))
        item = {"id": "synthetic-item", "type": "message", "role": "assistant", "status": "completed",
                "content": [{"type": "output_text", "text": "synthetic answer", "annotations": []}]}
        response = {"id": "synthetic-response", "object": "response", "status": "completed", "output": [item]}
        events = [
            {"type": "response.created", "response": {**response, "status": "in_progress", "output": []}},
            {"type": "response.output_item.added", "output_index": 0, "item": {**item, "status": "in_progress", "content": []}},
            {"type": "response.content_part.added", "item_id": item["id"], "output_index": 0, "content_index": 0,
             "part": {"type": "output_text", "text": "", "annotations": []}},
            {"type": "response.output_text.delta", "item_id": item["id"], "output_index": 0, "content_index": 0, "delta": "synthetic answer"},
            {"type": "response.output_text.done", "item_id": item["id"], "output_index": 0, "content_index": 0, "text": "synthetic answer"},
            {"type": "response.content_part.done", "item_id": item["id"], "output_index": 0, "content_index": 0, "part": item["content"][0]},
            {"type": "response.output_item.done", "output_index": 0, "item": item},
            {"type": "response.completed", "response": response},
        ]
        payload = b"".join(("event: %s\ndata: %s\n\n" % (e["type"], json.dumps(e))).encode() for e in events)
        self.send_response(200)
        self.send_header("Content-Type", "text/event-stream")
        self.send_header("Content-Length", str(len(payload)))
        self.end_headers()
        self.wfile.write(payload)


def seed_codex(codex, root, codex_home, port, cwds):
    environment = {"PATH": str(codex.parent) + ":/usr/bin:/bin", "HOME": str(root), "CODEX_HOME": str(codex_home),
                   "TMPDIR": str(root), "LANG": "C", "SYNTHETIC_KEY": "synthetic-only"}
    process = subprocess.Popen([str(codex), "app-server", "--stdio"], env=environment, stdin=subprocess.PIPE,
                               stdout=subprocess.PIPE, stderr=subprocess.DEVNULL, text=True)
    counter = iter(range(1, 1000))

    def invoke(method, params, timeout=15):
        ident = next(counter)
        process.stdin.write(json.dumps({"jsonrpc": "2.0", "id": ident, "method": method, "params": params}) + "\n")
        process.stdin.flush()
        deadline = time.monotonic() + timeout
        while time.monotonic() < deadline:
            line = process.stdout.readline()
            if line and json.loads(line).get("id") == ident:
                value = json.loads(line)
                if "error" in value:
                    raise RuntimeError("%s failed: %s" % (method, value["error"]))
                return value["result"]
        raise RuntimeError("timed out waiting for " + method)
    try:
        invoke("initialize", {"clientInfo": {"name": "velune-probe", "version": "0"}, "capabilities": {"experimentalApi": True}})
        provider = {"model_provider": "synthetic", "model_providers": {"synthetic": {
            "name": "synthetic", "base_url": "http://127.0.0.1:%d/v1" % port, "env_key": "SYNTHETIC_KEY",
            "wire_api": "responses", "requires_openai_auth": False}}}
        ids = []
        for cwd in cwds:
            thread = invoke("thread/start", {"cwd": str(cwd), "approvalPolicy": "never", "sandbox": "read-only",
                                             "modelProvider": "synthetic", "config": provider})["thread"]
            invoke("turn/start", {"threadId": thread["id"], "input": [{"type": "text", "text": "synthetic codex"}],
                                  "model": "gpt-6.1-sol"})
            time.sleep(2.5)
            ids.append(thread["id"])
        return ids
    finally:
        process.terminate()
        process.wait(timeout=5)


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    for name in ('bundle', 'bindings', 'node', 'pi', 'codex', 'claude'):
        parser.add_argument('--' + name, type=Path, required=True)
    args = parser.parse_args()
    for path in vars(args).values():
        if not path.is_absolute() or not path.exists():
            parser.error('absolute existing paths required')
    server = ThreadingHTTPServer(("127.0.0.1", 0), Upstream)
    threading.Thread(target=server.serve_forever, daemon=True).start()
    with tempfile.TemporaryDirectory(prefix='velune-conversation-list-') as directory:
        root = Path(directory).resolve()
        for name in ('bindings', 'seed', 'pi', 'codex', 'claude', 'tools', 'alpha', 'beta', 'gamma', 'delta'):
            (root / name).mkdir()
        package = args.pi.resolve()
        while not (package / 'package.json').is_file():
            package = package.parent
        (root / 'seed/node_modules/@earendil-works').mkdir(parents=True)
        (root / 'seed/node_modules/@earendil-works/pi-coding-agent').symlink_to(package)
        (root / 'seed/create.mjs').write_text(SEED)
        fixture_env = {'HOME': str(root), 'PATH': '/usr/bin:/bin'}
        # The history helpers read their request from stdin; the wrappers
        # keep a copy of each one before running the real interpreter.
        requests = root / 'requests.jsonl'
        node = root / 'tools/node'
        node.write_text('#!/bin/bash\ncase "$*" in *pi_sessions.mjs*) input=$(/bin/cat); printf \'%%s\\n\' "$input" >> %s;'
                        ' exec %s "$@" <<< "$input";; esac\nexec %s "$@"\n' % (requests, args.node, args.node))
        python = root / 'tools/python3'
        python.write_text('#!/bin/bash\ninput=$(/bin/cat); printf \'%%s\\n\' "$input" >> %s; exec /usr/bin/python3 "$@" <<< "$input"\n' % requests)
        node.chmod(0o755)
        python.chmod(0o755)

        def read_requests():
            # The paths and session ids the helpers were asked to read since last asked.
            if not requests.exists():
                return set()
            asked = set()
            for line in requests.read_text().splitlines():
                request = json.loads(line)
                asked.update(request.get('want') or request.get('nativeIds') or [])
            requests.unlink()
            return asked

        def pi_dir(cwd):
            return root / 'pi/sessions' / ('--' + str(cwd).lstrip('/').replace('/', '-') + '--')

        def seed(specs):
            return json.loads(subprocess.check_output([str(args.node), str(root / 'seed/create.mjs'), json.dumps(specs)],
                                                      env=fixture_env, text=True))
        # Oldest first: gamma, then beta, then alpha is the most recent project.
        specs = [{'cwd': str(root / 'gamma'), 'dir': str(pi_dir(root / 'gamma')), 'title': 'gamma %d' % i} for i in range(3)]
        specs += [{'cwd': str(root / 'beta'), 'dir': str(pi_dir(root / 'beta')), 'title': 'beta %d' % i} for i in range(4)]
        specs += [{'cwd': str(root / 'alpha'), 'dir': str(pi_dir(root / 'alpha')), 'title': 'alpha %d' % i} for i in range(7)]
        paths = seed(specs)
        codex_ids = seed_codex(args.codex, root, root / 'codex', server.server_port, [root / 'beta', root / 'beta'])

        (root / 'bindings/velune_bindings.py').write_bytes((args.bindings / 'velune_bindings.py').read_bytes())
        (root / 'bindings/libvelune_bindings.dylib').symlink_to(args.bundle / 'Contents/Frameworks/libvelune_bindings.dylib')
        os.environ.clear()
        os.environ.update(fixture_env)
        spec = importlib.util.spec_from_file_location('velune_bindings', root / 'bindings/velune_bindings.py')
        b = importlib.util.module_from_spec(spec)
        sys.modules[spec.name] = b
        spec.loader.exec_module(b)
        resources = str(args.bundle / 'Contents/Resources')
        home = root / 'home'

        def open_application():
            return b.VeluneApplication.open(b.BindingOptions(home_directory=str(home), resources_directory=resources))
        application = open_application()
        results = {}
        K = b.BindingConversationGroupKey
        C = b.BindingConversationOrganizationChange
        os.environ['PATH'] = str(root / 'tools') + ':/usr/bin:/bin'

        def preferences(**changes):
            value = dict(grouping=b.BindingConversationBrowserGrouping.NONE, sort=b.BindingConversationBrowserSort.UPDATED,
                         oldest_first=False, runtime_id=None, project=b.BindingConversationBrowserProject.ALL(), collapsed=[])
            value.update(changes)
            return b.BindingConversationBrowserPreferences(**value)

        # A small window before the runtimes join: only what it shows is read.
        application.set_conversation_browser_limits(b.BindingConversationBrowserLimits(page=3, total=5))
        application.set_conversation_browser_preferences(preferences(grouping=b.BindingConversationBrowserGrouping.PROJECT))
        application.upsert_runtime(b.BindingRuntimeInstance(enabled=True, id='pi', name='Synthetic Pi', type_id='pi-1.0.2',
            gateway_id='default', settings={'binary': str(args.pi), 'nodeBinary': str(node), 'agentDir': str(root / 'pi')}))
        application.upsert_runtime(b.BindingRuntimeInstance(enabled=True, id='codex', name='Synthetic Codex', type_id='codex-0.159.3',
            gateway_id='default', settings={'binary': str(args.codex), 'nodeBinary': str(args.node), 'agentDir': str(root / 'codex')}))
        lazy = application.list()
        loads = [(g.key, g.loaded, g.total) for g in lazy.conversation_groups]
        assert loads == [(K.PROJECT(str(root / 'beta')), 3, 6), (K.PROJECT(str(root / 'alpha')), 2, 7),
                         (K.PROJECT(str(root / 'gamma')), 0, 3)], loads
        shown_pi = {c.native_id for c in lazy.conversations if c.group is not None and c.conversation.runtime_id == 'pi'}
        asked = read_requests()
        assert len([c for c in lazy.conversations if c.group is not None]) == 5 and shown_pi <= set(paths[3:])
        assert asked == shown_pi, (asked, shown_pi)
        assert all(c.conversation.title for c in lazy.conversations)
        more = application.load_more_conversations(K.PROJECT(str(root / 'gamma')))
        assert read_requests() == set(paths[0:3])
        assert [c.conversation.title for c in more.conversations if c.group == K.PROJECT(str(root / 'gamma'))] == ['gamma 2', 'gamma 1', 'gamma 0']
        application.set_conversation_browser_limits(b.BindingConversationBrowserLimits(page=20, total=200))
        application.set_conversation_browser_preferences(preferences())
        results['lazy'] = 'ok'

        def shown(snapshot):
            return [c for c in snapshot.conversations if c.group is not None]

        def titles(snapshot, key):
            return [c.conversation.title for c in snapshot.conversations if c.group == key]

        def ref(item):
            return b.BindingConversationReference(runtime_id=item.conversation.runtime_id, conversation_id=item.conversation.id)

        def find(snapshot, title):
            return next(c for c in snapshot.conversations if c.conversation.title == title)

        listed = application.list()
        assert len(shown(listed)) == 16, [c.conversation.title for c in shown(listed)]
        assert [g.key for g in listed.conversation_groups] == [K.ALL()]
        assert not any(c.unread or c.pinned or c.archived or c.moved for c in listed.conversations)
        assert {p.path for p in listed.conversation_projects} == {str(root / n) for n in ('alpha', 'beta', 'gamma')}
        pi_row = find(listed, 'alpha 6')
        assert pi_row.native_id == paths[-1] and pi_row.project == str(root / 'alpha')
        codex_rows = [c for c in listed.conversations if c.conversation.runtime_id == 'codex']
        assert sorted(c.native_id for c in codex_rows) == sorted(codex_ids)
        results['initial'] = 'ok'

        # Pages: the total is shared out in group order; groups beyond it wait.
        application.set_conversation_browser_limits(b.BindingConversationBrowserLimits(page=3, total=5))
        grouped = application.set_conversation_browser_preferences(preferences(grouping=b.BindingConversationBrowserGrouping.PROJECT))
        order = [g.key for g in grouped.conversation_groups]
        # The Codex threads are newer than every Pi session, so beta leads.
        assert order == [K.PROJECT(str(root / 'beta')), K.PROJECT(str(root / 'alpha')), K.PROJECT(str(root / 'gamma'))], order
        loads = [(g.loaded, g.total) for g in grouped.conversation_groups]
        assert loads == [(3, 6), (2, 7), (0, 3)], loads
        more = application.load_more_conversations(K.PROJECT(str(root / 'gamma')))
        assert [(g.loaded, g.total) for g in more.conversation_groups][2] == (3, 3)
        assert titles(more, K.PROJECT(str(root / 'gamma'))) == ['gamma 2', 'gamma 1', 'gamma 0']
        folded = application.set_conversation_browser_preferences(preferences(
            grouping=b.BindingConversationBrowserGrouping.PROJECT, collapsed=[K.PROJECT(str(root / 'beta'))]))
        loads = [(g.loaded, g.total, g.collapsed) for g in folded.conversation_groups]
        # Folding keeps the extra page loaded for gamma; changing grouping would reset it.
        assert loads == [(0, 6, True), (3, 7, False), (3, 3, False)], loads
        results['window'] = 'ok'

        # Pins lead the list and do not use the total.
        pinned = application.organize_conversations([ref(find(folded, 'gamma 0'))], C.PIN())
        assert pinned.conversation_groups[0].key == K.PINNED() and titles(pinned, K.PINNED()) == ['gamma 0']
        assert [(g.loaded, g.total) for g in pinned.conversation_groups[1:]] == [(0, 6), (3, 7), (2, 2)]
        results['pin'] = 'ok'

        # Read state: new activity after the baseline is unread until marked.
        seed([{'append': paths[0]}])
        refreshed = application.set_conversation_browser_preferences(preferences())
        follow_up = next(c for c in refreshed.conversations if c.native_id == paths[0])
        assert follow_up.unread and follow_up.conversation.title == 'gamma 0'
        assert sum(c.unread for c in refreshed.conversations) == 1
        read = application.organize_conversations([ref(follow_up)], C.MARK_READ())
        assert not next(c for c in read.conversations if c.native_id == paths[0]).unread
        unread = application.organize_conversations([ref(follow_up)], C.MARK_UNREAD())
        application.set_conversation_browser_limits(b.BindingConversationBrowserLimits(page=3, total=100))
        unread = application.list()
        assert next(c for c in unread.conversations if c.native_id == paths[0]).unread
        results['read'] = 'ok'

        # Moving files it under another project only in the list.
        moved = application.organize_conversations([ref(find(unread, 'alpha 0'))], C.MOVE_TO_PROJECT(str(root / 'gamma')))
        row = find(moved, 'alpha 0')
        assert row.moved and row.project == str(root / 'gamma') and row.conversation.cwd == str(root / 'alpha')
        back = application.organize_conversations([ref(row)], C.MOVE_TO_PROJECT(None))
        assert not find(back, 'alpha 0').moved
        results['move'] = 'ok'

        aliased = application.set_project_alias(str(root / 'alpha'), '  Alpha 项目  ')
        assert next(p for p in aliased.conversation_projects if p.path == str(root / 'alpha')).alias == 'Alpha 项目'
        found = application.search_conversations('alpha 项目')
        assert sorted(c.conversation.title for c in found.conversations) == ['alpha %d' % i for i in range(7)]
        assert sorted(c.conversation.title for c in application.search_conversations('BETA 3').conversations) == ['beta 3']
        results['alias_search'] = 'ok'

        # A Velune archive hides the conversation and keeps the session.
        archived = application.organize_conversations([ref(find(aliased, 'beta 1'))], C.ARCHIVE())
        assert all(c.conversation.title != 'beta 1' for c in shown(archived))
        assert Path(paths[4]).exists()
        # The runtime's own archive: Codex lists the thread as archived.
        codex_row = next(c for c in archived.conversations if c.native_id == codex_ids[0])
        application.organize_conversations([ref(codex_row)], C.PIN())
        after = application.organize_conversations([ref(codex_row)], C.ARCHIVE())
        assert all(c.native_id != codex_ids[0] for c in shown(after))
        organization = json.loads((home / 'conversation-organization.json').read_text())['organization']
        codex_marks = [m for m in organization['conversations'] if m['conversationID'] == codex_row.conversation.id]
        assert not codex_marks, codex_marks  # pin cleared, archive kept by Codex
        assert any(m.get('archivedAtUnixMs') for m in organization['conversations'])
        view = application.show_archived_conversations(True)
        assert view.shows_archived
        assert sorted(c.conversation.title for c in shown(view)) == sorted(['beta 1', codex_row.conversation.title])
        assert all(c.archived for c in shown(view))
        opened = application.open_conversation('codex', codex_row.conversation.id)
        assert opened.snapshot.conversation.id == codex_row.conversation.id
        restored = application.organize_conversations([ref(c) for c in shown(view)], C.UNARCHIVE())
        assert not shown(restored)
        back = application.show_archived_conversations(False)
        assert len(shown(back)) == 16 and not any(c.archived for c in back.conversations)
        results['archive'] = 'ok'

        assert application.find_conversation('pi:' + paths[1]).conversation.title == 'gamma 1'
        assert application.find_conversation('pi:/missing') is None

        # Claude Code: indexed from its project folders; the window reads its
        # sessions by recency of their files, and a session whose records are
        # older than its file moves out once read.
        project = root / 'claude/projects' / str(root / 'delta').replace('/', '-')
        project.mkdir(parents=True)
        claude_ids = []
        base = int(time.time()) - 1000
        for index in range(6):
            native = '00000000-0000-4000-8000-%012d' % index
            claude_ids.append(native)
            # delta 0's file is the newest, its records the oldest.
            at = base + (10 if index == 0 else 100 + index * 10)
            stamp = time.strftime('%Y-%m-%dT%H:%M:%S.000Z', time.gmtime(at))
            records = [{'type': 'user', 'uuid': native[:-1] + 'a', 'parentUuid': None, 'sessionId': native, 'cwd': str(root / 'delta'),
                        'timestamp': stamp, 'message': {'role': 'user', 'content': 'synthetic question %d' % index}},
                       {'type': 'custom-title', 'customTitle': 'delta %d' % index, 'sessionId': native}]
            file = project / (native + '.jsonl')
            file.write_text(''.join(json.dumps(record) + '\n' for record in records))
            os.utime(file, (base + 900 if index == 0 else at, base + 900 if index == 0 else at))
        read_requests()
        application.set_conversation_browser_limits(b.BindingConversationBrowserLimits(page=2, total=2))
        application.upsert_runtime(b.BindingRuntimeInstance(enabled=True, id='claude', name='Synthetic Claude', type_id='claude-code-2.1.286',
            gateway_id='default', settings={'binary': str(args.claude), 'agentDir': str(root / 'claude')}))
        window = application.set_conversation_browser_preferences(preferences(runtime_id='claude'))
        assert [c.conversation.title for c in shown(window)] == ['delta 5', 'delta 4'], [c.conversation.title for c in shown(window)]
        assert [(g.loaded, g.total) for g in window.conversation_groups] == [(2, 6)]
        assert read_requests() == {claude_ids[0], claude_ids[5], claude_ids[4]}
        more = application.load_more_conversations(K.ALL())
        assert [c.conversation.title for c in shown(more)] == ['delta 5', 'delta 4', 'delta 3', 'delta 2']
        assert read_requests() == {claude_ids[3], claude_ids[2]}
        found = application.search_conversations('delta 1')
        assert [c.conversation.title for c in shown(found)] == ['delta 1']
        assert read_requests() == {claude_ids[1]}
        assert application.find_conversation('claude:' + claude_ids[0]).conversation.title == 'delta 0'
        application.set_conversation_browser_limits(b.BindingConversationBrowserLimits(page=3, total=100))
        application.set_conversation_browser_preferences(preferences())
        results['claude_lazy'] = 'ok'
        application.shutdown()
        application = open_application()
        reopened = application.list()
        assert next(p for p in reopened.conversation_projects if p.path == str(root / 'alpha')).alias == 'Alpha 项目'
        assert titles(reopened, K.PINNED()) == ['gamma 0']
        assert next(c for c in reopened.conversations if c.native_id == paths[0]).unread
        assert reopened.conversation_browser_limits.page == 3 and reopened.conversation_browser_limits.total == 100
        results['persistence'] = 'ok'
        application.shutdown()
        print(json.dumps(results, ensure_ascii=False))
    server.shutdown()


if __name__ == '__main__':
    main()
