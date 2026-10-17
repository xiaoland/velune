#!/usr/bin/env python3
"""Manual native-loader acceptance for what the resource library writes.

Library resources are saved and enabled through the UniFFI API on disabled
Claude Code 2.1.286, Codex 0.159.3 and Pi 1.0.2 instances in a temporary HOME;
each Harness's own loader then reports what it found. Requires macOS
sandbox-exec, the installed Claude and Codex CLIs, Node and Pi 1.0.2's
dist/extensions/mcp/config.js installed outside /Users. No model request is
made, and no real credential, caller configuration or session is read.
"""
import argparse
import importlib.util
import json
import os
import pathlib
import selectors
import shutil
import subprocess
import sys
import tempfile
import time


def invoke(sandbox, profile, argv, cwd, env, timeout=30):
    return subprocess.run([sandbox, '-f', str(profile), *map(str, argv)], cwd=cwd,
                          env=env, text=True, capture_output=True, timeout=timeout, check=True).stdout


def rpc(process, pending, request_id, method, params, deadline=20):
    """One JSON-RPC exchange over a line-oriented stdio process."""
    process.stdin.write((json.dumps({'id': request_id, 'method': method, 'params': params}) + '\n').encode())
    process.stdin.flush()
    end = time.monotonic() + deadline
    with selectors.DefaultSelector() as watcher:
        watcher.register(process.stdout, selectors.EVENT_READ)
        while time.monotonic() < end:
            while b'\n' in pending[0]:
                line, pending[0] = pending[0].split(b'\n', 1)
                message = json.loads(line)
                if message.get('id') == request_id:
                    if 'error' in message:
                        raise AssertionError(f'{method} failed: {message["error"]}')
                    return message['result']
            if not watcher.select(max(0, end - time.monotonic())):
                break
            data = os.read(process.stdout.fileno(), 65536)
            if not data:
                break
            pending[0] += data
    raise AssertionError(f'{method} did not answer within the deadline')


def claude_initialize(sandbox, profile, claude, project, env):
    process = subprocess.Popen([sandbox, '-f', str(profile), str(claude), '--print', '--verbose',
                                '--input-format', 'stream-json', '--output-format', 'stream-json',
                                '--no-session-persistence', '--model', 'synthetic-model'],
                               cwd=project, env=env, stdin=subprocess.PIPE,
                               stdout=subprocess.PIPE, stderr=subprocess.DEVNULL)
    try:
        payload = {'type': 'control_request', 'request_id': 'library-init', 'request': {'subtype': 'initialize'}}
        process.stdin.write(json.dumps(payload).encode() + b'\n')
        process.stdin.flush()
        pending = b''
        deadline = time.monotonic() + 20
        with selectors.DefaultSelector() as watcher:
            watcher.register(process.stdout, selectors.EVENT_READ)
            while time.monotonic() < deadline:
                if not watcher.select(max(0, deadline - time.monotonic())):
                    break
                data = os.read(process.stdout.fileno(), 65536)
                if not data:
                    break
                pending += data
                while b'\n' in pending:
                    line, pending = pending.split(b'\n', 1)
                    message = json.loads(line)
                    if message.get('type') == 'control_response':
                        return message['response']['response']
        raise AssertionError('Claude did not answer initialize within the deadline')
    finally:
        process.kill()
        process.communicate(timeout=5)


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    for name in ('library', 'bindings', 'claude', 'codex', 'node', 'pi-mcp-config'):
        parser.add_argument('--' + name, required=True, type=pathlib.Path)
    args = parser.parse_args()
    for name, path in vars(args).items():
        setattr(args, name, path.resolve(strict=True))
    sandbox = shutil.which('sandbox-exec')
    if not sandbox:
        parser.error('sandbox-exec is required; there is no unsandboxed fallback')
    if str(args.pi_mcp_config).startswith('/Users/'):
        parser.error('use an isolated Pi installation outside /Users')
    assert json.loads((args.pi_mcp_config.parents[3] / 'package.json').read_text())['version'] == '1.0.2'
    with tempfile.TemporaryDirectory(prefix='velune-library-native-') as directory:
        root = pathlib.Path(directory).resolve()
        for name in ('home', 'resources', 'project', 'bindings'):
            (root / name).mkdir()
        env = {'HOME': str(root / 'home'), 'PATH': '/usr/bin:/bin', 'TMPDIR': str(root), 'LANG': 'C',
               'XDG_CONFIG_HOME': str(root / 'home/config'), 'XDG_CACHE_HOME': str(root / 'home/cache'),
               'XDG_DATA_HOME': str(root / 'home/data'), 'NO_PROXY': '127.0.0.1,localhost',
               'CLAUDE_CODE_DISABLE_NONESSENTIAL_TRAFFIC': '1', 'DISABLE_AUTOUPDATER': '1'}
        profile = root / 'isolation.sb'
        profile.write_text('(version 1)\n(allow default)\n'
                           '(deny file-read* (subpath "/Users"))\n'
                           '(deny file-read* (subpath "/Library/Managed Preferences"))\n'
                           '(deny file-read* (subpath "/private/var/db/ConfigurationProfiles"))\n'
                           '(allow file-read* (literal ' + json.dumps(str(args.node)) + '))\n'
                           '(deny network-outbound)\n'
                           '(allow network-outbound (remote ip "localhost:*"))\n')
        probe = "try { require('fs').readdirSync('/Users'); process.exit(1); } catch(e) { if (!['EPERM','EACCES'].includes(e.code)) throw e; }"
        invoke(sandbox, profile, [args.node, '-e', probe], root / 'project', env)
        assert '2.1.286' in invoke(sandbox, profile, [args.claude, '--version'], root / 'project', env)
        assert '0.159.3' in invoke(sandbox, profile, [args.codex, '--version'], root / 'project', env)

        shutil.copy2(args.bindings / 'velune_bindings.py', root / 'bindings/velune_bindings.py')
        shutil.copy2(args.library, root / 'bindings' / args.library.name)
        spec = importlib.util.spec_from_file_location('velune_library_native', root / 'bindings/velune_bindings.py')
        b = importlib.util.module_from_spec(spec)
        sys.modules[spec.name] = b
        spec.loader.exec_module(b)
        dirs = {'claude': root / 'home/.claude', 'codex': root / 'codex', 'pi': root / 'pi-agent'}
        app = b.VeluneApplication.open(b.BindingOptions(home_directory=str(root / 'app'), resources_directory=str(root / 'resources')))
        try:
            for identity, variant in [('claude', 'claude-code-2.1'), ('codex', 'codex-0.159'), ('pi', 'pi-1.0')]:
                app.upsert_runtime(b.BindingRuntimeInstance(enabled=False, id=identity, name=identity, type_id=variant,
                                   gateway_id='default', settings={'agentDir': str(dirs[identity])}))
            C, T = b.BindingResourceTemplateContent, b.BindingTemplateMcpTransport

            def enable(name, content, runtimes):
                summary = app.save_resource_template(b.BindingResourceTemplate(id='', name=name, content=content)).summary
                reports = app.apply_resource_template(summary.id, [b.BindingTemplateApplyTarget(runtime_id=r, overwrite=False) for r in runtimes])
                assert all(isinstance(r.outcome, b.BindingTemplateApplyOutcome.CREATE) for r in reports), reports
                assert all(isinstance(s.state, b.BindingResourceSyncState.SYNCED)
                           for s in app.resource_sync_status() if s.template_id == summary.id and s.runtime_id in runtimes)

            everyone = ['claude', 'codex', 'pi']
            enable('Library rules', C.INSTRUCTIONS(body='LIBRARY_INSTRUCTION_MARKER\n'), everyone)
            enable('velune-library-skill', C.SKILL(files=[b.BindingTemplateSkillFile(
                path='SKILL.md', executable=False,
                bytes=b'---\nname: velune-library-skill\ndescription: Velune library fixture\n---\nLIBRARY_SKILL_MARKER\n')]), everyone)
            enable('velune-library-stdio', C.MCP(transport=T.STDIO(command='/usr/bin/true', args=['--fixture'],
                   env={'FIXTURE': 'synthetic'}, cwd='/tmp'), timeout_seconds=30), everyone)
            enable('velune-library-http', C.MCP(transport=T.HTTP(url='http://127.0.0.1:1/mcp',
                   headers={'X-Fixture': 'synthetic'}), timeout_seconds=None), everyone)
            enable('velune-library-agent', C.SUBAGENT(description='Velune library agent fixture',
                   instructions='LIBRARY_AGENT_MARKER', model=None, tools=[]), ['claude', 'codex'])

            # Claude: strict .claude.json parsing, MCP discovery, and the initialize catalog.
            claude_env = dict(env, CLAUDE_CONFIG_DIR=str(dirs['claude']))
            listing = invoke(sandbox, profile, [args.claude, 'mcp', 'list'], root / 'project', claude_env, timeout=60)
            assert 'velune-library-stdio' in listing and 'velune-library-http' in listing, listing
            detail = invoke(sandbox, profile, [args.claude, 'mcp', 'get', 'velune-library-stdio'], root / 'project', claude_env, timeout=60)
            assert '/usr/bin/true' in detail, detail
            catalog = claude_initialize(sandbox, profile, args.claude, root / 'project', claude_env)
            assert any(agent.get('name') == 'velune-library-agent' for agent in catalog['agents']), catalog.keys()
            commands = [command.get('name') for command in catalog.get('commands', [])]
            assert 'velune-library-skill' in commands, commands
            print('PASS Claude: library MCP servers listed (timeout in ms, no cwd); Agent and skill in the initialize catalog', flush=True)

            # Codex: config.toml with cwd and tool_timeout_sec, skills and Agent roles via app-server.
            codex_env = dict(env, CODEX_HOME=str(dirs['codex']))
            servers = {entry['name']: entry for entry in json.loads(invoke(sandbox, profile, [args.codex, 'mcp', 'list', '--json'],
                                                                              root / 'project', codex_env))}
            stdio, http = servers['velune-library-stdio'], servers['velune-library-http']
            assert stdio['transport'].get('cwd') == '/tmp' and stdio['transport'].get('command') == '/usr/bin/true', stdio
            assert stdio.get('tool_timeout_sec') == 30, stdio
            assert http['transport'].get('url') == 'http://127.0.0.1:1/mcp', http
            process = subprocess.Popen([sandbox, '-f', str(profile), str(args.codex), 'app-server', '--stdio'],
                                       cwd=root / 'project', env=codex_env, stdin=subprocess.PIPE,
                                       stdout=subprocess.PIPE, stderr=subprocess.DEVNULL)
            try:
                pending = [b'']
                rpc(process, pending, 1, 'initialize', {'clientInfo': {'name': 'velune-probe', 'version': '0'},
                                                         'capabilities': {'experimentalApi': True}})
                skills = rpc(process, pending, 2, 'skills/list', {'cwds': [str(root / 'project')]})
                assert 'velune-library-skill' in json.dumps(skills), skills
            finally:
                process.kill()
                process.communicate(timeout=5)
            print('PASS Codex: library MCP servers parsed with cwd and tool_timeout_sec; skill listed by app-server', flush=True)

            # Pi: the fixed package's own mcp.json loader and validator.
            oracle = root / 'pi-mcp.mjs'
            oracle.write_text("""import assert from 'node:assert/strict';
const [module, cwd, agentDir] = process.argv.slice(2);
const {loadMcpConfig} = await import(module);
const config = loadMcpConfig({cwd, agentDir, projectTrusted: false});
assert.deepEqual(config.errors, []);
const byName = Object.fromEntries(config.servers.map(server => [server.name, server.config]));
assert.equal(byName['velune-library-stdio'].command, '/usr/bin/true');
assert.equal(byName['velune-library-stdio'].cwd, '/tmp');
assert.equal(byName['velune-library-stdio'].timeout, 30);
assert.equal(byName['velune-library-http'].url, 'http://127.0.0.1:1/mcp');
console.log('PASS Pi: library MCP servers accepted by loadMcpConfig with cwd and timeout');
""")
            print(invoke(sandbox, profile, [args.node, oracle, args.pi_mcp_config.as_uri(), root / 'project', dirs['pi']],
                         root / 'project', env).strip(), flush=True)
        finally:
            app.shutdown()


if __name__ == '__main__':
    main()
