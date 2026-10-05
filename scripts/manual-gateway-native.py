#!/usr/bin/env python3
"""Manual synthetic native-HTTP acceptance; never uses real providers or credentials.

Supply --deps absolute target/debug/deps and --rustc the absolute toolchain rustc.
Cargo selects matching artifacts before the temporary probe is compiled.
This script is not a CI/test entry point.
"""
import argparse
import http.client
import json
from pathlib import Path
import socket
import subprocess
import tempfile
import threading
import time
from http.server import BaseHTTPRequestHandler, ThreadingHTTPServer

PROBE = r'''
use std::{collections::BTreeMap,io::{BufRead,Write},sync::Arc,time::Instant};
struct Resolver;
impl velune_gateway::CredentialResolver for Resolver {
 fn resolve(&self,_reference:String,_target:velune_gateway::CredentialTarget)
 ->velune_ai::OperationFuture<Result<velune_gateway::ResolvedCredential,velune_gateway::CredentialResolutionError>> {
  Box::pin(async {Ok(velune_gateway::ResolvedCredential {token:"synthetic-only".into(),explicit_output_cap:None,subscription:false})})
 }
}
fn main()->Result<(),Box<dyn std::error::Error>> {
 let args:Vec<_>=std::env::args().collect();
 let config=serde_json::from_slice(&std::fs::read(&args[1])?)?;
 let runner=velune_gateway::Runner::start(config,Arc::new(Resolver),BTreeMap::new())?;
 println!("{}",serde_json::json!({"endpoint":runner.endpoint(),"token":runner.token()}));std::io::stdout().flush()?;
 let _=std::io::stdin().lock().lines().next();let start=Instant::now();drop(runner);
 println!("{}",serde_json::json!({"dropMs":start.elapsed().as_millis()}));
 Ok(())
}
'''



def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('--deps', required=True, type=Path)
    parser.add_argument('--rustc', required=True, type=Path)
    args = parser.parse_args()
    for path in (args.deps, args.rustc):
        if not path.is_absolute() or not path.exists():
            parser.error('paths must be absolute and exist')
    captures = []
    peer_closed = threading.Event()
    requested = threading.Event()
    stream_body = b'data: {"vendor_event":true,"choices":[{"delta":{"reasoning_content":"SYNTHETIC"},"finish_reason":"vendor_stop"}],"usage":{"vendor_counter":9}}\n\ndata: [DONE]\n\n'
    response_stream = b'event: response.completed\ndata: {"type":"response.completed","response":{"status":"completed","vendor_field":9}}\n\n'

    incomplete_body = b'{"status":"incomplete","incomplete_details":{"reason":"max_output_tokens"},"vendor_field":9}'
    failed_stream = b'event: response.failed\ndata: {"type":"response.failed","response":{"status":"failed","error":{"code":"synthetic_failure"},"vendor_field":9}}\n\n'

    class Upstream(BaseHTTPRequestHandler):
        def log_message(self, *_):
            pass

        def do_POST(self):
            body = json.loads(self.rfile.read(int(self.headers['Content-Length'])))
            captures.append({'path': self.path, 'body': body, 'headers': {name.lower(): value for name,value in self.headers.items()}})
            mode = body['fixture_case']
            if mode == 'slow_headers':
                requested.set()
                self.connection.settimeout(5)
                try:
                    if self.connection.recv(1) == b'':
                        peer_closed.set()
                except socket.timeout:
                    pass
                return
            if mode == 'slow_stream':
                self.send_response(200)
                self.send_header('Content-Type', 'text/event-stream')
                self.end_headers()
                requested.set()
                try:
                    while True:
                        self.wfile.write(b'data: {"choices":[]}\n\n')
                        self.wfile.flush()
                        time.sleep(.05)
                except (BrokenPipeError, ConnectionResetError):
                    peer_closed.set()
                return
            status = 429 if mode == 'error' else 201 if mode in ['json', 'incomplete_json'] else 200
            payload = b'{"error":{"vendor_code":"synthetic_limit"}}' if mode == 'error' else (response_stream if self.path.endswith('/responses') else stream_body) if mode == 'stream' else b'{"status":"completed","choices":[],"vendor_field":{"retained":true}}'
            if mode == 'incomplete_json':
                payload = incomplete_body
            elif mode == 'failed_stream':
                payload = failed_stream
            self.send_response(status)
            self.send_header('Content-Type', 'text/event-stream' if mode in ['stream', 'failed_stream'] else 'application/json')
            self.send_header('Retry-After', '7')
            self.send_header('X-Request-ID', 'synthetic-id')
            self.send_header('X-Vendor-Field', 'retained')
            self.send_header('Set-Cookie', 'synthetic-not-forwarded')
            self.end_headers()
            self.wfile.write(payload)
            self.wfile.flush()

    server = ThreadingHTTPServer(('127.0.0.1', 0), Upstream)
    server.daemon_threads = True
    threading.Thread(target=server.serve_forever, daemon=True).start()
    processes = []
    with tempfile.TemporaryDirectory(prefix='velune-native-http-') as temporary:
        root = Path(temporary)
        source = root / 'probe.rs'
        source.write_text(PROBE)
        # Directory timestamps can mix incompatible Cargo feature builds. Read
        # the artifacts from this dependency graph instead of choosing newest.
        build = subprocess.run(['cargo', 'build', '--locked', '-p', 'velune-gateway',
            '--manifest-path', str(Path(__file__).resolve().parents[1] / 'Cargo.toml'),
            '--target-dir', str(args.deps.parents[1]), '--message-format=json'],
            check=True, capture_output=True, text=True)
        artifacts = {}
        for line in build.stdout.splitlines():
            item = json.loads(line)
            if item.get('reason') == 'compiler-artifact':
                for filename in item['filenames']:
                    if filename.endswith('.rlib'):
                        artifacts[item['target']['name']] = filename
        def artifact(name):
            return artifacts[name]
        subprocess.run([str(args.rustc), '--edition=2024', str(source), '-L', f'dependency={args.deps}', '--extern', f'velune_gateway={artifact("velune_gateway")}', '--extern', f'velune_ai={artifact("velune_ai")}', '--extern', f'serde_json={artifact("serde_json")}', '-o', str(root / 'probe')], check=True)
        def start(reference='synthetic'):
            pairs = [('chat', 'chatCompletionsV1'), ('responses', 'responsesV1')]
            config = {'id': 'fixture', 'name': 'fixture',
                'models': [{'recordKey': model, 'nickname': model} for model, _ in pairs],
                'providers': [{'id': protocol, 'name': protocol, 'protocol': protocol,
                    'endpoint': f'http://127.0.0.1:{server.server_port}/v1', 'credentialRef': reference,
                    'models': [{'modelRecordKey': model, 'providerModelId': 'external-' + model}]}
                    for model, protocol in pairs],
                'routes': [{'modelRecordKey': model, 'providerId': protocol} for model, protocol in pairs],
                'failover': {'mode': 'disabled'}}
            path = root / f'config-{len(processes)}.json'
            path.write_text(json.dumps(config))
            process = subprocess.Popen([str(root / 'probe'), str(path)], stdin=subprocess.PIPE, stdout=subprocess.PIPE, stderr=subprocess.PIPE, text=True)
            processes.append(process)
            info = json.loads(process.stdout.readline())
            info['process'] = process
            info['port'] = int(info['endpoint'].split(':')[2].split('/')[0])
            return info
        def stop(info):
            process = info['process']
            process.stdin.write('stop\n')
            process.stdin.flush()
            output, error = process.communicate(timeout=5)
            if process.returncode:
                raise RuntimeError('isolated gateway process failed')
            result = json.loads(output)
            if result['dropMs'] > 2000:
                raise AssertionError('Runner drop exceeded two seconds')
            return result['dropMs']
        def open_request(info, mode, model='chat', streaming=False, chunked=False, wire_model=None):
            connection = http.client.HTTPConnection('127.0.0.1', info['port'], timeout=8)
            body = {'model': wire_model or 'velune/model/' + model, 'fixture_case': mode, 'stream': streaming, 'vendor_option': {'unchanged': True}, 'messages': [{'role': 'system', 'content': 'first'}, {'role': 'developer', 'content': 'second'}, {'role': 'assistant', 'content': 'history', 'reasoning_content': 'SYNTHETIC_HISTORY'}], 'input': 'synthetic'}
            encoded = json.dumps(body).encode()
            encoded = [encoded[:12], encoded[12:]] if chunked else encoded
            connection.request('POST', '/v1/responses' if model == 'responses' else '/v1/chat/completions', encoded, {'Authorization': 'Bearer ' + info['token'], 'Content-Type': 'application/json', 'X-Session-Affinity': 'synthetic-session'}, encode_chunked=chunked)
            return connection, body
        try:
            info = start()
            for model in ['chat', 'responses']:
                for mode in ['json', 'stream', 'error']:
                    connection, original = open_request(info, mode, model, mode == 'stream')
                    response = connection.getresponse()
                    data = response.read()
                    assert response.status == {'json': 201, 'stream': 200, 'error': 429}[mode]
                    assert response.getheader('Retry-After') == '7' and response.getheader('X-Request-ID') == 'synthetic-id'
                    assert response.getheader('X-Vendor-Field') == 'retained' and response.getheader('Set-Cookie') is None
                    expected = b'{"error":{"vendor_code":"synthetic_limit"}}' if mode == 'error' else (response_stream if model == 'responses' else stream_body) if mode == 'stream' else b'{"status":"completed","choices":[],"vendor_field":{"retained":true}}'
                    assert data == expected
                    actual = captures[-1]
                    assert actual['body'] == {**original, 'model': 'external-' + model}
                    assert actual['headers'].get('x-session-affinity') == 'synthetic-session'
                    connection.close()
            connection, _ = open_request(info, 'json', chunked=True)
            response = connection.getresponse()
            assert response.status == 201
            assert json.loads(response.read())['vendor_field']['retained']
            connection.close()
            for mode, expected in [('incomplete_json', incomplete_body), ('failed_stream', failed_stream)]:
                connection, _ = open_request(info, mode, 'responses', mode == 'failed_stream')
                response = connection.getresponse()
                assert response.status == (201 if mode == 'incomplete_json' else 200)
                assert response.read() == expected
                connection.close()
            print(json.dumps({'nativeCases': 9, 'chunkedRequestAccepted': True, 'responsesBusinessTerminalsPreserved': True, 'jsonAndSseBytesPreserved': True, 'unknownFieldsAndHistoryPreserved': True, 'httpStatusAndHeadersPreserved': True, 'outputLimitOptional': True}))
            before = len(captures)
            connection, _ = open_request(info, 'json', wire_model='chat')
            response = connection.getresponse()
            assert response.status == 400 and 'route' in response.read().decode()
            assert len(captures) == before, 'internal key leaked upstream'
            connection.close()
            print(json.dumps({'internalRecordKeyIsNotWireModel': True, 'unknownCapabilitiesDoNotBlockNativeCall': True}))
            for mode in ['slow_headers', 'slow_stream']:
                peer_closed.clear(); requested.clear()
                connection, _ = open_request(info, mode, streaming=True)
                assert requested.wait(2)
                if mode == 'slow_stream':
                    response = connection.getresponse()
                    response.read(1)
                connection.close()
                assert peer_closed.wait(3), mode + ' upstream did not cancel after disconnect'
            print(json.dumps({'cancelBeforeHeaders': True, 'cancelDuringStream': True, 'dropIdleMs': stop(info)}))
        finally:
            for process in processes:
                if process.poll() is None:
                    try:
                        process.communicate(input='stop\n', timeout=3)
                    except subprocess.TimeoutExpired:
                        process.kill()
                        process.wait()
    server.shutdown()
    server.server_close()
    print(json.dumps({'temporaryDirectoryRemoved': not Path(temporary).exists()}))


if __name__ == '__main__':
    main()
