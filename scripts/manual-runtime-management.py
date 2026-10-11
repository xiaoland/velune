#!/usr/bin/env python3
"""Manual native-loader acceptance using temporary resources and a loopback model fixture.

Requires macOS sandbox-exec, Claude Code 2.1.286, Node and Pi 1.0.2's installed
resource-loader.js. Install Pi and its dependencies outside /Users for this probe.
No real credentials, caller configuration, history or public model API is used.
"""
import argparse
import http.server
import importlib.util
import json
import os
import pathlib
import selectors
import shutil
import subprocess
import sys
import tempfile
import threading
import time


class ModelFixture(http.server.BaseHTTPRequestHandler):
    def log_message(self, *_):
        pass

    def do_POST(self):
        request = json.loads(self.rfile.read(int(self.headers['Content-Length'])))
        if self.path.split('?')[0].endswith('/count_tokens'):
            body = json.dumps({'input_tokens': 20}).encode()
            content_type = 'application/json'
        else:
            self.server.requests.append(request)
            message = {'id': 'msg_fixture', 'type': 'message', 'role': 'assistant',
                       'content': [], 'model': request['model'], 'stop_reason': None,
                       'stop_sequence': None, 'usage': {'input_tokens': 20, 'output_tokens': 0}}
            events = [
                ('message_start', {'message': message}),
                ('content_block_start', {'index': 0, 'content_block': {'type': 'text', 'text': ''}}),
                ('content_block_delta', {'index': 0, 'delta': {'type': 'text_delta', 'text': 'RESOURCE_ACK'}}),
                ('content_block_stop', {'index': 0}),
                ('message_delta', {'delta': {'stop_reason': 'end_turn', 'stop_sequence': None}, 'usage': {'output_tokens': 4}}),
                ('message_stop', {}),
            ]
            body = ''.join('event: ' + event + '\ndata: ' + json.dumps({'type': event, **data}) + '\n\n'
                           for event, data in events).encode()
            content_type = 'text/event-stream'
        self.send_response(200)
        self.send_header('Content-Type', content_type)
        self.send_header('Content-Length', str(len(body)))
        self.end_headers()
        self.wfile.write(body)


def invoke(sandbox, profile, argv, cwd, env, timeout=25):
    return subprocess.run([sandbox, '-f', str(profile), *map(str, argv)], cwd=cwd,
                          env=env, text=True, capture_output=True, timeout=timeout, check=True).stdout


def initialize(sandbox, profile, claude, project, env):
    process = subprocess.Popen([sandbox, '-f', str(profile), str(claude), '--print', '--verbose',
                                '--input-format', 'stream-json', '--output-format', 'stream-json',
                                '--no-session-persistence', '--model', 'synthetic-model'],
                               cwd=project, env=env, stdin=subprocess.PIPE,
                               stdout=subprocess.PIPE, stderr=subprocess.DEVNULL)
    try:
        payload = {'type': 'control_request', 'request_id': 'resource-init', 'request': {'subtype': 'initialize'}}
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
                        response = message['response']['response']
                        assert any(agent.get('name') == 'velune-resource-agent' for agent in response['agents'])
                        return
        raise AssertionError('Claude did not expose the API-written native Agent within the deadline')
    finally:
        process.kill()
        process.communicate(timeout=5)


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    for name in ('library', 'bindings', 'claude', 'node', 'pi-module'):
        parser.add_argument('--' + name, required=True, type=pathlib.Path)
    args = parser.parse_args()
    for name, path in vars(args).items():
        setattr(args, name, path.resolve(strict=True))
    sandbox = shutil.which('sandbox-exec')
    if not sandbox:
        parser.error('sandbox-exec is required; there is no unsandboxed fallback')
    if str(args.pi_module).startswith('/Users/'):
        parser.error('use an isolated Pi installation outside /Users, including its dependencies')
    assert json.loads((args.pi_module.parents[2] / 'package.json').read_text())['version'] == '1.0.2'
    previous = dict(os.environ)
    try:
        with tempfile.TemporaryDirectory(prefix='velune-management-native-') as directory:
            root = pathlib.Path(directory).resolve()
            for name in ('home', 'resources', 'project', 'bindings'):
                (root / name).mkdir()
            env = {'HOME': str(root / 'home'), 'PATH': '/usr/bin:/bin', 'TMPDIR': str(root),
                   'XDG_CONFIG_HOME': str(root / 'home/config'), 'XDG_CACHE_HOME': str(root / 'home/cache'),
                   'XDG_DATA_HOME': str(root / 'home/data'), 'NO_PROXY': '127.0.0.1,localhost',
                   'CLAUDE_CODE_DISABLE_NONESSENTIAL_TRAFFIC': '1', 'DISABLE_AUTOUPDATER': '1'}
            os.environ.clear()
            os.environ.update(env)
            profile = root / 'isolation.sb'
            profile.write_text('(version 1)\n(allow default)\n'
                               '(deny file-read* (subpath "/Users"))\n'
                               '(deny file-read* (subpath "/Library/Managed Preferences"))\n'
                               '(deny file-read* (subpath "/private/var/db/ConfigurationProfiles"))\n'
                               '(allow file-read* (literal ' + json.dumps(str(args.node)) + '))\n'
                               '(allow file-read* (literal ' + json.dumps(str(args.claude)) + '))\n'
                               '(deny network-outbound)\n'
                               '(allow network-outbound (remote ip "localhost:*"))\n')
            # Check denial with a directory probe, without reading any private file.
            probe = "try { require('fs').readdirSync('/Users'); process.exit(1); } catch(e) { if (!['EPERM','EACCES'].includes(e.code)) throw e; }"
            invoke(sandbox, profile, [args.node, '-e', probe], root / 'project', env)
            assert '2.1.286' in invoke(sandbox, profile, [args.claude, '--version'], root / 'project', env)
            shutil.copy2(args.bindings / 'velune_bindings.py', root / 'bindings/velune_bindings.py')
            shutil.copy2(args.library, root / 'bindings' / args.library.name)
            spec = importlib.util.spec_from_file_location('velune_management_native', root / 'bindings/velune_bindings.py')
            b = importlib.util.module_from_spec(spec)
            sys.modules[spec.name] = b
            spec.loader.exec_module(b)
            app = b.VeluneApplication.open(b.BindingOptions(home_directory=str(root / 'app'), resources_directory=str(root / 'resources')))
            try:
                for identity, variant, path in [('claude', 'claude-code-2.1.286', root / 'home/.claude'), ('pi', 'pi-1.0.2', root / 'pi-agent')]:
                    app.upsert_runtime(b.BindingRuntimeInstance(enabled=False, id=identity, name=identity,
                                       type_id=variant, gateway_id='default', settings={'agentDir': str(path)}))
                K, S, R = b.BindingRuntimeResourceKind, b.BindingRuntimeResourceScope, b.BindingRuntimeResourceRevision
                def write(runtime, kind, path, content, project=False):
                    key = b.BindingRuntimeResourceKey(kind=kind, scope=S.PROJECT if project else S.USER, path=path)
                    cwd = str(root / 'project') if project else None
                    document = app.write_runtime_resource(runtime, cwd, key, R.MISSING(), content, None)
                    loaded = app.read_runtime_resource(runtime, cwd, key).bytes
                    assert (json.loads(loaded) == json.loads(content)) if kind == K.MCP_SERVERS else loaded == content
                    return document
                write('claude', K.USER_INSTRUCTIONS, 'CLAUDE.md', b'CLAUDE_API_INSTRUCTION_MARKER\n')
                write('claude', K.SKILLS, 'velune-resource-skill/SKILL.md',
                      b'---\nname: velune-resource-skill\ndescription: Velune isolated resource fixture\n---\nCLAUDE_API_SKILL_MARKER\nReturn RESOURCE_ACK without using tools.\n')
                write('claude', K.NATIVE_AGENT_DEFINITIONS, 'velune-resource-agent.md',
                      b'---\nname: velune-resource-agent\ndescription: Velune isolated agent fixture\n---\nCLAUDE_API_AGENT_MARKER\n')
                mcp = write('claude', K.MCP_SERVERS, 'velune-resource-mcp', b'{"type":"http","url":"http://127.0.0.1:1/mcp"}')
                claude_env = dict(env, CLAUDE_CONFIG_DIR=str(root / 'home/.claude'))
                listing = invoke(sandbox, profile, [args.claude, 'mcp', 'list'], root / 'project', claude_env)
                assert 'velune-resource-mcp' in listing
                initialize(sandbox, profile, args.claude, root / 'project', claude_env)
                # Claude updates unrelated fields in the shared config during initialization.
                mcp = app.read_runtime_resource('claude', None, mcp.entry.key)
                app.delete_runtime_resource('claude', None, mcp.entry.key, R.MATCH(mcp.entry.revision))
                with http.server.ThreadingHTTPServer(('127.0.0.1', 0), ModelFixture) as server:
                    server.requests = []
                    thread = threading.Thread(target=server.serve_forever, daemon=True)
                    thread.start()
                    try:
                        model_env = dict(claude_env, ANTHROPIC_API_KEY='synthetic-fixture-only',
                                         ANTHROPIC_BASE_URL=f'http://127.0.0.1:{server.server_port}')
                        output = invoke(sandbox, profile, [args.claude, '--print', '--verbose', '--output-format',
                                        'stream-json', '--no-session-persistence', '--model', 'synthetic-model',
                                        '--permission-mode', 'bypassPermissions', '/velune-resource-skill'], root / 'project', model_env)
                        assert 'RESOURCE_ACK' in output
                        requests = json.dumps(server.requests)
                        assert 'CLAUDE_API_INSTRUCTION_MARKER' in requests, 'Claude did not load user instructions'
                        assert 'CLAUDE_API_SKILL_MARKER' in requests, 'Claude did not load the API-written skill body'
                    finally:
                        server.shutdown()
                        thread.join(timeout=5)
                print('PASS Claude: API-written instruction and skill body reached loopback request; native Agent and MCP discovered', flush=True)
                for project, marker in [(False, 'GLOBAL'), (True, 'PROJECT')]:
                    write('pi', K.USER_INSTRUCTIONS, 'AGENTS.md', f'PI_{marker}_INSTRUCTION_MARKER\n'.encode(), project)
                    name = 'velune-' + marker.lower() + '-skill'
                    write('pi', K.SKILLS, name + '/SKILL.md', f'---\nname: {name}\ndescription: isolated fixture\n---\nPI_{marker}_SKILL_MARKER\n'.encode(), project)
                oracle = root / 'pi-loader.mjs'
                oracle.write_text("""import assert from 'node:assert/strict';
const [module, cwd, agentDir] = process.argv.slice(2);
const {DefaultResourceLoader} = await import(module);
const loader = new DefaultResourceLoader({cwd, agentDir}); await loader.reload();
const instructions = loader.getAgentsFiles().agentsFiles;
const skills = loader.getSkills().skills;
for (const scope of ['GLOBAL','PROJECT']) {
  assert(instructions.some(item => item.content.includes('PI_'+scope+'_INSTRUCTION_MARKER')));
  assert(skills.some(item => item.name === 'velune-'+scope.toLowerCase()+'-skill'));
}
console.log('PASS Pi: API-written user/project instructions and skills discovered by DefaultResourceLoader');
""")
                result = invoke(sandbox, profile, [args.node, oracle, args.pi_module.as_uri(), root / 'project', root / 'pi-agent'],
                                root / 'project', dict(env, PI_CODING_AGENT_DIR=str(root / 'pi-agent')))
                print(result.strip(), flush=True)
            finally:
                app.shutdown()
    finally:
        os.environ.clear()
        os.environ.update(previous)


if __name__ == '__main__':
    main()
