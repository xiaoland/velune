"""隔离移动安装的真实 SDK 网络／系统证书验收；配置由夹具预置，不计作界面保存证据。"""
import argparse
from concurrent.futures import ThreadPoolExecutor
import json
import os
from pathlib import Path
import subprocess
import threading
import time
import uuid


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('platform', choices=['ios', 'android'])
    parser.add_argument('--device', required=True)
    parser.add_argument('--host', required=True, help='设备可达的开发机 IPv4 地址')
    args = parser.parse_args()
    import grpc
    from google.protobuf.json_format import MessageToDict
    from opentelemetry.proto.collector.trace.v1 import trace_service_pb2 as traces, trace_service_pb2_grpc as trace_rpc
    from opentelemetry.proto.collector.logs.v1 import logs_service_pb2 as logs, logs_service_pb2_grpc as log_rpc
    from opentelemetry.proto.collector.metrics.v1 import metrics_service_pb2 as metrics, metrics_service_pb2_grpc as metric_rpc
    repository = Path(__file__).resolve().parents[3]
    root = repository / 'target' / ('telemetry-mobile-' + args.platform + '-' + uuid.uuid4().hex)
    root.mkdir(mode=0o700)
    package = 'app.velune.ios.telemetry' if args.platform == 'ios' else 'app.velune.android.telemetry'
    relative = 'Library/Application Support/client/telemetry/' if args.platform == 'ios' else 'no_backup/telemetry/'
    sequence = 0

    def run(command, *, check=True, data=None):
        nonlocal sequence
        sequence += 1
        result = subprocess.run(command, input=data, capture_output=True, timeout=25)
        (root / f'command-{sequence:03}.log').write_bytes(result.stdout + result.stderr)
        if check and result.returncode:
            raise RuntimeError(f'Native command failed; see command-{sequence:03}.log')
        return result

    def device(*command, **options):
        return run(['xcrun', 'devicectl', 'device', *command[:2], '--device', args.device, '--timeout', '20', *command[2:]], **options)

    def read(name, *, optional=False):
        output = root / ('read-' + name)
        output.unlink(missing_ok=True)
        if args.platform == 'ios':
            result = device('copy', 'from', '--domain-type', 'appDataContainer', '--domain-identifier', package,
                            '--source', relative + name, '--destination', str(output), check=not optional)
            if result.returncode:
                return None
            return json.loads(output.read_text())
        result = run(['adb', '-s', args.device, 'exec-out', 'run-as', package, 'cat', relative + name], check=not optional)
        if result.returncode:
            return None
        output.write_bytes(result.stdout)
        return json.loads(result.stdout)

    def write(config):
        output = root / 'fixture-config.json'
        output.write_text(json.dumps(config))
        output.chmod(0o600)
        if args.platform == 'ios':
            device('copy', 'to', '--domain-type', 'appDataContainer', '--domain-identifier', package,
                   '--source', str(output), '--destination', relative + 'config.json')
        else:
            run(['adb', '-s', args.device, 'shell', '-T', 'run-as', package, 'sh', '-c',
                 "'umask 077; cat > no_backup/telemetry/config.json'"], data=output.read_bytes())

    def launch():
        if args.platform == 'ios':
            output = root / 'launch.json'
            device('process', 'launch', '--terminate-existing', '--json-output', str(output), package)
            return json.loads(output.read_text())['result']['process']['processIdentifier']
        else:
            run(['adb', '-s', args.device, 'shell', 'am', 'force-stop', package])
            run(['adb', '-s', args.device, 'shell', 'am', 'start', '-n', package + '/app.velune.android.MainActivity'])

    def await_state(predicate, message, seconds=35):
        end = time.monotonic() + seconds
        while time.monotonic() < end:
            state = read('status.json')
            if predicate(state):
                return state
            time.sleep(1)
        raise AssertionError(message)

    received = [[], []]
    lock = threading.Lock()

    class Receiver:
        def __init__(self, index):
            self.index = index

        def Export(self, request, context):
            assert dict(context.invocation_metadata()).get('x-velune-fixture') == 'SYNTHETIC_MOBILE_NETWORK'
            with lock:
                assert len(received[self.index]) < 100, 'unbounded export loop'
                received[self.index].append(MessageToDict(request))
            if isinstance(request, traces.ExportTraceServiceRequest):
                return traces.ExportTraceServiceResponse()
            if isinstance(request, logs.ExportLogsServiceRequest):
                return logs.ExportLogsServiceResponse()
            return metrics.ExportMetricsServiceResponse()

    servers = []
    reverses = []
    initial = read('status.json')
    original = read('config.json', optional=True)
    restored = original or dict(version=1, revision=0, installationId=initial['instanceId'], preference='unset', profile=None)
    evidence = dict(platform=args.platform, package=package, initial=initial, passed=False)
    try:
        run(['openssl', 'req', '-x509', '-newkey', 'rsa:2048', '-nodes', '-keyout', str(root / 'key.pem'),
             '-out', str(root / 'cert.pem'), '-days', '1', '-subj', '/CN=Velune synthetic receiver',
             '-addext', 'subjectAltName=IP:' + args.host])
        ports = []
        for index in range(2):
            server = grpc.server(ThreadPoolExecutor(max_workers=3))
            receiver = Receiver(index)
            trace_rpc.add_TraceServiceServicer_to_server(receiver, server)
            log_rpc.add_LogsServiceServicer_to_server(receiver, server)
            metric_rpc.add_MetricsServiceServicer_to_server(receiver, server)
            if index:
                port = server.add_secure_port(args.host + ':0', grpc.ssl_server_credentials([((root / 'key.pem').read_bytes(), (root / 'cert.pem').read_bytes())]))
            else:
                port = server.add_insecure_port(args.host + ':0')
            assert port > 0
            ports.append(port)
            if args.platform == 'android':
                run(['adb', '-s', args.device, 'reverse', 'tcp:' + str(port), 'tcp:' + str(port)])
                reverses.append(port)
            server.start()
            servers.append(server)
        profile = dict(version=1, revision=restored['revision'] + 1, installationId=initial['instanceId'], preference='on',
                       profile=dict(endpoint=f'http://{args.host}:{ports[0]}', headers={'x-velune-fixture': 'SYNTHETIC_MOBILE_NETWORK'}, sampleRatio=1))
        write(profile)
        assert read('config.json') == profile, 'Fixture configuration was not transferred'
        process = launch()
        await_state(lambda s: process is None or s['processId'] == process, 'New native client did not publish its own status', seconds=8)
        active = await_state(lambda s: (process is None or s['processId'] == process) and s['applied'] and s['rustExport']['completedBatches'] > 0 and s['nativeExport']['completedBatches'] > 0,
                             'Both actual SDKs did not reach the plaintext control receiver')
        assert active['rustActiveRevision'] == active['nativeActiveRevision'] == profile['revision'] + 1
        # Require all three signals from both actual SDK scopes, not just a successful channel or old status.
        end = time.monotonic() + 20
        def scopes():
            found = {key: set() for key in ['Spans', 'Logs', 'Metrics']}
            with lock:
                for request in received[0]:
                    for signal in found:
                        for resource in request.get('resource' + signal, []):
                            for scope in resource.get('scope' + signal, []):
                                found[signal].add(scope.get('scope', {}).get('name', ''))
            return found
        expected = {'Spans': {'velune.client.native', 'velune.client.rust'}, 'Logs': {'velune.client.native', 'velune_telemetry'}, 'Metrics': {'velune.client.native', 'velune.client.rust'}}
        while time.monotonic() < end and not all(expected[k] <= names for k, names in scopes().items()):
            time.sleep(.2)
        observed = scopes()
        assert all(expected[k] <= names for k, names in observed.items()), observed
        evidence.update(plaintext=active, signalScopes={k: sorted(v) for k, v in observed.items()})
        if args.platform == 'android':
            def resumed():
                return run(['adb', '-s', args.device, 'shell', 'dumpsys activity activities | grep -m 1 mResumedActivity']).stdout.decode()
            assert package in resumed(), 'Isolated activity was not in the foreground'
            run(['adb', '-s', args.device, 'shell', 'input', 'keyevent', 'KEYCODE_HOME'])
            time.sleep(2)
            assert package not in resumed(), 'Home did not background the isolated activity'
            background = read('status.json')
            run(['adb', '-s', args.device, 'shell', 'am', 'start', '-n', package + '/app.velune.android.MainActivity'])
            assert package in resumed(), 'Isolated activity did not resume'
            foreground = read('status.json')
            assert background['processId'] == foreground['processId'] == active['processId']
            assert background['rustActiveRevision'] == foreground['rustActiveRevision'] == active['rustActiveRevision']
            evidence.update(shortBackgroundResumeSameOwner=True)

        profile['revision'] += 1
        profile['profile']['endpoint'] = f'https://{args.host}:{ports[1]}'
        write(profile)
        assert read('config.json') == profile, 'TLS fixture configuration was not transferred'
        process = launch()
        rejected = await_state(lambda s: (process is None or s['processId'] == process) and s['revision'] == profile['revision'] + 1 and s['rustExport']['failedBatches'] > 0 and s['nativeExport']['failedBatches'] > 0,
                               'Both actual SDKs did not report system-trust rejection')
        assert not received[1], 'Untrusted certificate accepted an OTLP RPC'
        assert rejected['applied'] and rejected['rustActiveRevision'] == rejected['nativeActiveRevision'] == profile['revision'] + 1
        assert 'SYNTHETIC_MOBILE_NETWORK' not in json.dumps(received), 'Authentication leaked into OTLP payload'
        evidence.update(untrustedTls=rejected, tlsAcceptedRpcs=0, passed=True)
    except Exception as error:
        evidence['failure'] = str(error)
        raise
    finally:
        try:
            write(restored)
            assert read('config.json') == restored, 'Saved configuration was not restored'
            process = launch()
            restored_revision = restored['revision'] + (restored['preference'] == 'on')
            recovered = await_state(lambda s: (process is None or s['processId'] == process) and s['revision'] == restored_revision and s['preference'] == restored['preference'], 'Saved state not restored', seconds=15)
            evidence['restored'] = recovered
            if restored['preference'] != 'on':
                assert recovered['rustActiveRevision'] is None and recovered['nativeActiveRevision'] is None
        except Exception as error:
            evidence['restorationFailure'] = str(error)
            evidence['passed'] = False
            if 'failure' not in evidence:
                raise
        finally:
            for server in servers:
                server.stop(0)
            for port in reverses:
                run(['adb', '-s', args.device, 'reverse', '--remove', 'tcp:' + str(port)], check=False)
            (root / 'wire.json').write_text(json.dumps(received, indent=2))
            (root / 'evidence.json').write_text(json.dumps(evidence, indent=2))
            print(root / 'evidence.json', flush=True)


if __name__ == '__main__':
    main()
