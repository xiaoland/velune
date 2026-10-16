#!/usr/bin/env python3
"""Explicit, bounded Pi coding-task collection/replay through the actual Velune gateway.

Secrets are read in this process only and injected at the HTTPS boundary. Saved
fixtures contain synthetic task traffic, never headers or provider credentials.
This is a manual entry point; it is not part of CI or an automated test suite.
"""
import argparse
import http.client
import importlib.util
import json
import os
import re
from pathlib import Path
import shlex
import ssl
import subprocess
import threading
from http.server import BaseHTTPRequestHandler, ThreadingHTTPServer
from urllib.parse import urlsplit

PROTOCOLS = {'chat': 'chatCompletionsV1', 'responses': 'responsesV1', 'messages': 'messagesV1'}
PI_APIS = {'chat': 'openai-completions', 'responses': 'openai-responses', 'messages': 'anthropic-messages'}
PATHS = {'chat': '/chat/completions', 'responses': '/responses', 'messages': '/messages'}
MARKER = 'RELAYKIT_SYNTHETIC_CODE'
ROOT_MARKER = '${SYNTHETIC_ROOT}'


def environment_values(path):
    if path.stat().st_mode & 0o077:
        raise RuntimeError('provider environment must be private (mode 0600)')
    values = {}
    for line in path.read_text().splitlines():
        parts = shlex.split(line, comments=True)
        if parts and parts[0] == 'export':
            parts = parts[1:]
        if len(parts) == 1 and '=' in parts[0]:
            key, value = parts[0].split('=', 1)
            values[key] = value
    return values


def sanitized(value, root):
    if isinstance(value, dict):
        result = {key: sanitized(item, root) for key, item in value.items()
                  if key not in {'request_id', 'requestId', 'account_id', 'organization_id', 'system_fingerprint'}}
        if isinstance(result.get('messages'), list):
            result['messages'] = [message for message in result['messages'] if message.get('role') not in ('system', 'developer')]
        if 'id' in result and (value.get('object') in ('chat.completion', 'chat.completion.chunk', 'response') or value.get('type') == 'message'):
            result['id'] = 'synthetic-response'
        return result
    if isinstance(value, list):
        return [sanitized(item, root) for item in value]
    if isinstance(value, str):
        return value.replace(str(root), ROOT_MARKER).replace(str(Path.home()), '${DEVELOPER_HOME}')
    return value


def sanitize_wire(raw, root):
    text = raw.decode('utf8')
    if text.lstrip().startswith('{'):
        return json.dumps(sanitized(json.loads(text), root), ensure_ascii=False)
    output = []
    for line in text.splitlines():
        if line.startswith('data:') and line[5:].strip() not in ('', '[DONE]'):
            line = 'data: ' + json.dumps(sanitized(json.loads(line[5:]), root), ensure_ascii=False)
        output.append(line)
    return '\n'.join(output) + '\n'


def usage_events(raw):
    values = []
    for line in raw.splitlines():
        if line.startswith('data:') and line[5:].strip() not in ('', '[DONE]'):
            values.append(json.loads(line[5:]))
    if not values and raw.lstrip().startswith('{'):
        values = [json.loads(raw)]
    usages = []
    for value in values:
        for owner in (value, value.get('message') or {}, value.get('response') or {}):
            if owner.get('usage'):
                usages.append(owner['usage'])
    return usages


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('mode', choices=('collect', 'replay'))
    parser.add_argument('--output', type=Path, required=True)
    parser.add_argument('--fixture', type=Path)
    parser.add_argument('--env-file', type=Path)
    parser.add_argument('--ca-file', type=Path, help='explicit trusted CA bundle when Python has no system certificate bundle')
    parser.add_argument('--prefix')
    parser.add_argument('--allowed-host')
    parser.add_argument('--provider')
    parser.add_argument('--model')
    parser.add_argument('--upstream', choices=PROTOCOLS, default='chat')
    parser.add_argument('--ingress', choices=PROTOCOLS, default='responses')
    parser.add_argument('--node', type=Path, required=True)
    parser.add_argument('--pi', type=Path, required=True)
    parser.add_argument('--rustc', type=Path, required=True)
    args = parser.parse_args()
    repository = Path(__file__).resolve().parent.parent
    root = args.output.resolve()
    if not root.is_relative_to(repository / 'target') or root.exists():
        parser.error('output must be a new directory beneath this repository target/')
    root.mkdir(parents=True, mode=0o700)
    root.chmod(0o700)
    records, errors, attempts = [], [], []
    lock = threading.Lock()
    saved = None
    key = None
    if args.mode == 'collect':
        if not all((args.env_file, args.prefix, args.allowed_host, args.provider, args.model)):
            parser.error('collect requires env-file, prefix, allowed-host, provider and model')
        values = environment_values(args.env_file)
        key = values.get(args.prefix + '_API_KEY')
        endpoint = urlsplit(values.get(args.prefix + '_BASE_URL', ''))
        if not key:
            raise RuntimeError('selected provider key is not configured')
        if endpoint.scheme != 'https' or endpoint.hostname != args.allowed_host or endpoint.username or endpoint.password or endpoint.query or endpoint.fragment:
            raise RuntimeError('provider endpoint does not match the explicitly selected HTTPS host')
        del values
    else:
        if not args.fixture:
            parser.error('replay requires fixture')
        saved = json.loads(args.fixture.read_text())
        args.provider, args.model, args.upstream, args.ingress = (saved[k] for k in ('provider', 'model', 'upstream', 'ingress'))

    class Proxy(BaseHTTPRequestHandler):
        def log_message(self, *_):
            pass

        def do_POST(self):
            connection = None
            try:
                request = json.loads(self.rfile.read(int(self.headers['Content-Length'])))
                with lock:
                    index = len(attempts)
                    attempts.append(self.path)
                # Harness retries remain local after a provider failure or the bounded task budget.
                if errors or index >= 3:
                    self.send_error(502, 'bounded provider probe stopped')
                    return
                if not self.path.endswith(PATHS[args.upstream]):
                    raise RuntimeError('unexpected upstream protocol path')
                if saved:
                    if index >= len(saved['records']):
                        raise RuntimeError('replay issued more requests than the captured coding task')
                    fixture = saved['records'][index]
                    def relevant(body):
                        result = {field: body.get(field) for field in ('model', 'tools', 'tool_choice', 'max_tokens', 'max_completion_tokens', 'max_output_tokens', 'stream', 'input')}
                        result['messages'] = [message for message in body.get('messages', []) if message.get('role') not in ('system', 'developer')]
                        return result
                    if relevant(sanitized(request, root)) != relevant(fixture['request']):
                        raise RuntimeError('replay upstream request differed from captured model, conversation or tools')
                    status, content_type = fixture['status'], fixture['contentType']
                    raw = fixture['response'].replace(ROOT_MARKER, str(root)).replace('${DEVELOPER_HOME}', str(Path.home())).encode()
                    self.send_response(status)
                    self.send_header('Content-Type', content_type)
                    self.send_header('Content-Length', str(len(raw)))
                    self.end_headers()
                    self.wfile.write(raw)
                else:
                    # HTTPSConnection never follows redirects, so authorization cannot cross hosts.
                    connection = http.client.HTTPSConnection(endpoint.hostname, endpoint.port or 443, timeout=120,
                                                            context=ssl.create_default_context(cafile=args.ca_file))
                    path = endpoint.path.rstrip('/') + PATHS[args.upstream]
                    headers = {'Content-Type': 'application/json', 'Authorization': 'Bearer ' + key}
                    if args.upstream == 'messages':
                        headers['x-api-key'] = key
                        headers['anthropic-version'] = '2023-06-01'
                    connection.request('POST', path, json.dumps(request).encode(), headers)
                    response = connection.getresponse()
                    status = response.status
                    content_type = response.getheader('Content-Type', 'application/json')
                    self.send_response(status)
                    self.send_header('Content-Type', content_type)
                    self.end_headers()
                    chunks = []
                    while chunk := response.read1(65536):
                        chunks.append(chunk)
                        self.wfile.write(chunk)
                        self.wfile.flush()
                    raw = b''.join(chunks)
                    if status != 200:
                        failure = {'status': status}
                        if key not in raw.decode('utf8', errors='replace'):
                            try:
                                error = json.loads(raw).get('error', {})
                                if isinstance(error, dict):
                                    for field in ('code', 'type', 'message'):
                                        if isinstance(error.get(field), str):
                                            failure[field] = re.sub(r'(?i)(request[_ -]?id|account[_ -]?id)[\s:=]+[\w-]+', r'\1=[redacted]', error[field])
                            except (ValueError, AttributeError):
                                pass
                        (root / 'provider-failure.json').write_text(json.dumps(failure, indent=2))
                        raise RuntimeError('provider HTTP ' + str(status))
                text = raw.decode('utf8')
                if key and (key in text or key in json.dumps(request)):
                    raise RuntimeError('credential found in traffic; fixture was not saved')
                record = {'path': self.path, 'request': sanitized(request, root), 'status': status,
                          'contentType': content_type, 'response': sanitize_wire(raw, root)}
                with lock:
                    records.append(record)
                    partial = {'schema': 1, 'provider': args.provider, 'model': args.model, 'ingress': args.ingress,
                               'upstream': args.upstream, 'redactions': ['system/developer prompts omitted', 'provider response identifiers canonicalized', 'local paths replaced'], 'records': records}
                    (root / 'fixture.partial.json').write_text(json.dumps(partial, ensure_ascii=False, indent=2))
            except Exception as error:
                # Do not serialize arbitrary provider exceptions or response bodies.
                with lock:
                    errors.append(str(error) if isinstance(error, RuntimeError) else type(error).__name__)
                try:
                    self.send_error(502, 'provider probe interrupted')
                except OSError:
                    pass
            finally:
                if connection:
                    connection.close()

    server = ThreadingHTTPServer(('127.0.0.1', 0), Proxy)
    threading.Thread(target=server.serve_forever, daemon=True).start()
    runner = None
    try:
        module_path = repository / 'scripts/manual-protocol-translation.py'
        spec = importlib.util.spec_from_file_location('manual_translation', module_path)
        module = importlib.util.module_from_spec(spec)
        spec.loader.exec_module(module)
        source = root / 'runner.rs'
        source.write_text(module.PROBE)
        build = subprocess.run(['cargo', 'build', '--locked', '-p', 'velune-gateway', '--message-format=json'],
                               cwd=repository, capture_output=True, text=True, check=True)
        artifacts = {}
        for line in build.stdout.splitlines():
            item = json.loads(line)
            if item.get('reason') == 'compiler-artifact':
                for filename in item['filenames']:
                    if filename.endswith('.rlib'):
                        artifacts[item['target']['name']] = filename
        binary = root / 'runner'
        subprocess.run([str(args.rustc), '--edition=2024', str(source), '-L', 'dependency=' + str(repository / 'target/debug/deps'),
                        *[argument for name in ('velune_gateway', 'velune_ai', 'serde_json')
                          for argument in ('--extern', name + '=' + artifacts[name])], '-o', str(binary)], check=True)
        config = {'id': 'live-probe', 'name': 'Live probe', 'providers': [{'id': 'provider', 'name': args.provider,
                  'protocol': PROTOCOLS[args.upstream], 'endpoint': 'http://127.0.0.1:' + str(server.server_port) + '/v1',
                  'credentialRef': 'synthetic', 'models': [{'recordKey': 'fixture', 'providerModelId': args.model,
                  'nickname': 'fixture', 'contextWindow': 32768, 'maxOutputTokens': 2048}]}]}
        (root / 'gateway.json').write_text(json.dumps(config))
        runner = subprocess.Popen([str(binary), str(root / 'gateway.json')], stdin=subprocess.PIPE, stdout=subprocess.PIPE,
                                  stderr=subprocess.PIPE, text=True)
        info = json.loads(runner.stdout.readline())
        home, project = root / 'home', root / 'project'
        home.mkdir()
        project.mkdir()
        guard = root / 'read-guard.mjs'
        guard.write_text("import path from 'node:path'; export default function(pi) { pi.on('tool_call', (event, ctx) => { if (event.toolName !== 'read' || path.resolve(ctx.cwd, event.input.path) !== path.join(ctx.cwd, 'fixture.py')) return {block:true, reason:'Only the synthetic fixture may be read'}; }); }\n")
        (project / 'fixture.py').write_text('def multiply():\n    return 17 * 23\n\n# ' + MARKER + '\n')
        model_config = {'providers': {'velune-probe': {'baseUrl': info['endpoint'].removesuffix('/v1') if args.ingress == 'messages' else info['endpoint'], 'api': PI_APIS[args.ingress],
                        'apiKey': info['token'], 'models': [{'id': 'velune/model/fixture', 'name': 'Fixture', 'reasoning': False,
                        'input': ['text'], 'contextWindow': 32768, 'maxTokens': 2048}]}}}
        (home / 'models.json').write_text(json.dumps(model_config))
        environment = {'HOME': str(home), 'PI_CODING_AGENT_DIR': str(home), 'PI_OFFLINE': '1',
                       'PATH': str(args.node.parent) + ':/usr/bin:/bin', 'NO_PROXY': '127.0.0.1,localhost'}
        prompt = 'Read fixture.py with the read tool exactly once. Inspect its multiply function. Then answer only RESULT=<the integer returned by multiply()>. Do not edit files or call other tools.'
        result = subprocess.run([str(args.node), str(args.pi), '--offline', '--no-session', '--no-extensions', '--no-skills',
                                 '--extension', str(guard), '--no-prompt-templates', '--no-themes', '--no-context-files', '--no-approve', '--tools', 'read',
                                 '--provider', 'velune-probe', '--model', 'velune/model/fixture', '--thinking', 'off', '--mode', 'json',
                                 '--print', '--', prompt], cwd=project, env=environment, capture_output=True, text=True, timeout=180)
        diagnostic = result.stdout + '\nSTDERR:\n' + result.stderr
        if key and key in diagnostic:
            raise RuntimeError('credential found in client output; not saved')
        (root / 'client-output.txt').write_text(diagnostic.replace(info['token'], 'synthetic-gateway-token').replace(str(root), ROOT_MARKER))
        events = [json.loads(line) for line in result.stdout.splitlines() if line.startswith('{')]
        messages = [e['message'] for e in events if e.get('type') == 'message_end']
        calls = [part for message in messages if message.get('role') == 'assistant' for part in message.get('content', [])
                 if part.get('type') == 'toolCall']
        tool_results = [message for message in messages if message.get('role') == 'toolResult']
        final = [message for message in messages if message.get('role') == 'assistant' and message.get('stopReason') == 'stop']
        if errors or result.returncode or len(records) != 2 or len(calls) != 1 or len(tool_results) != 1 or not final:
            raise RuntimeError('coding task did not complete one tool continuation; requests=' + str(len(records)) + '; failures=' + repr(errors) + '; exit=' + str(result.returncode))
        assert calls[0]['name'] == 'read' and calls[0]['id'] == tool_results[0]['toolCallId']
        assert not tool_results[0].get('isError') and MARKER in json.dumps(tool_results[0])
        assert MARKER in json.dumps(records[1]['request']) and calls[0]['id'] in json.dumps(records[1]['request'])
        assert any('RESULT=391' in block.get('text', '') for block in final[-1]['content'])
        payload = {'schema': 1, 'provider': args.provider, 'model': args.model, 'ingress': args.ingress,
                   'upstream': args.upstream, 'redactions': ['system/developer prompts omitted', 'provider response identifiers canonicalized', 'local paths replaced'], 'records': records}
        encoded = json.dumps(payload, ensure_ascii=False, indent=2)
        if key and key in encoded:
            raise RuntimeError('credential found in fixture; not saved')
        (root / 'fixture.json').write_text(encoded)
        report = {'mode': args.mode, 'provider': args.provider, 'model': args.model, 'ingress': args.ingress,
                  'upstream': args.upstream, 'requests': len(records), 'toolIdPreserved': True, 'actualReadResultContinued': True,
                  'normalTerminal': True, 'upstreamUsageEvents': [usage_events(record['response']) for record in records],
                  'sourceCommit': subprocess.check_output(['git', 'rev-parse', 'HEAD'], cwd=repository, text=True).strip()}
        (root / 'report.json').write_text(json.dumps(report, indent=2))
        print(json.dumps(report))
    finally:
        if runner:
            runner.stdin.write('stop\n')
            runner.stdin.flush()
            runner.wait(timeout=10)
        server.shutdown()


if __name__ == '__main__':
    main()
