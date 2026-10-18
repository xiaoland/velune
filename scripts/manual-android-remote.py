#!/usr/bin/env python3
"""显式准备 Android / Mac 人工验收的合成 ACP 宿主，不读取真实配置。

先 --prepare-only 生成独立 application HOME，或保持前台运行 iroh 宿主。
邀请只写入权限 0600 的 invitation.txt；不打印邀请或业务正文。
--transport-probe 是独立 S1 marker 宿主，不打开业务核心或 Harness。
"""
import argparse
import enum
import hashlib
import importlib.util
import json
import os
from pathlib import Path
import runpy
import shutil
import sys
import time


def private_json(path, value):
    temporary = path.with_suffix('.tmp')
    descriptor = os.open(temporary, os.O_WRONLY | os.O_CREAT | os.O_TRUNC, 0o600)
    with os.fdopen(descriptor, 'w') as output:
        json.dump(value, output, ensure_ascii=False, indent=2)
    temporary.replace(path)


def configure_questions(b, application, root):
    """真实原生 Claude 协议 fixture 提供审批后的问题回答，不调用供应商。"""
    if any(runtime.id == 'questions' for runtime in application.list().runtime_instances):
        return
    fixture = runpy.run_path(str(Path(__file__).with_name('manual-runtime-interactions.py')), run_name='fixture')
    binary = root / 'synthetic-questions'
    binary.write_text('#!' + sys.executable + '\n' + fixture['CLAUDE_SERVER'])
    binary.chmod(0o700)
    application.upsert_runtime(b.BindingRuntimeInstance(enabled=True, id='questions', name='Synthetic questions',
        type_id='claude-code-2.1', gateway_id='default', settings={
            'binary': str(binary), 'agentDir': str(root / 'questions-runtime')}))


def diagnostic_value(value):
    if value is None or isinstance(value, (str, int, float, bool)):
        return value
    if isinstance(value, enum.Enum):
        return value.name
    if isinstance(value, (list, tuple)):
        return [diagnostic_value(item) for item in value]
    if isinstance(value, dict):
        return {key: diagnostic_value(item) for key, item in value.items()}
    return {'type': type(value).__qualname__,
        **{key: diagnostic_value(item) for key, item in vars(value).items() if not key.startswith('_')}}


def serve_transport_probe(b, args):
    """S1 独立 ALPN，只承载合成 marker；不打开 ApplicationService 或 Harness。"""
    relay = b.BindingRelayConfig.DEVELOPMENT_PUBLIC() if args.public_infrastructure else b.BindingRelayConfig.DISABLED()
    discovery = b.BindingDiscoveryConfig.DEVELOPMENT_PUBLIC() if args.public_infrastructure else b.BindingDiscoveryConfig.NONE()
    events = args.root / 'host-events.jsonl'
    if events.exists():
        raise ValueError('S1 每轮使用新的私有 root，保留已有 host-events.jsonl')
    private_json(args.root / 'host-info.json', {
        'library_sha256': hashlib.sha256((args.root / 'bindings' / args.library.name).read_bytes()).hexdigest(),
        'bindings_sha256': hashlib.sha256((args.root / 'bindings' / 'velune_bindings.py').read_bytes()).hexdigest(),
        'public_infrastructure': args.public_infrastructure, 'mdns': args.mdns,
        'started_unix_seconds': time.time(),
    })
    probe = b.VeluneTransportProbe.open(b.BindingTransportProbeOptions(
        relay=relay, discovery=discovery, path_policy=b.BindingRemotePathPolicy.AUTOMATIC,
        bind_addresses=['0.0.0.0:0', '[::]:0']))
    started = time.monotonic_ns()
    try:
        if args.mdns:
            probe.enable_mdns()
        descriptor = os.open(events, os.O_WRONLY | os.O_CREAT | os.O_EXCL, 0o600)
        with os.fdopen(descriptor, 'a') as output:
            announced = False
            while True:
                mdns_refresh = None
                if args.mdns:
                    try:
                        mdns_refresh = {'generation': probe.refresh_mdns_interfaces()}
                    except b.BindingError.Diagnostic as error:
                        mdns_refresh = {'error_kind': 'binding_diagnostic',
                            'binding_failure_kind': error.kind.name}
                target = probe.target()
                expected = {'endpointId': target.endpoint_id, 'relayUrls': target.relay_urls,
                    'directAddresses': target.direct_addresses}
                if args.mdns:
                    private_json(args.root / 'expected-target.json', expected)
                private_json(args.root / 'target.json', {
                    'endpointId': target.endpoint_id, 'relayUrls': [] if args.mdns else target.relay_urls,
                    'directAddresses': [] if args.mdns else target.direct_addresses, 'mdns': args.mdns,
                    'publicInfrastructure': args.public_infrastructure,
                    'intervalMs': 1000, 'payloadBytes': 256, 'timeoutMs': 2000,
                })
                sample = {'elapsed_ms': (time.monotonic_ns() - started) // 1_000_000,
                    'observed_unix_ms': time.time_ns() // 1_000_000,
                    'connections': diagnostic_value(probe.connections()),
                    'native_events': diagnostic_value(probe.native_events())}
                if args.mdns:
                    sample['mdns_refresh'] = mdns_refresh
                    controlled = {connection.status.peer_id for connection in probe.connections()}
                    mdns = probe.mdns_events()
                    sample['mdns_events'] = diagnostic_value([entry for entry in mdns
                        if not hasattr(entry, 'endpoint_id') or entry.endpoint_id in controlled])
                output.write(json.dumps(sample, ensure_ascii=True) + '\n')
                output.flush()
                if not announced:
                    print('S1 marker 宿主运行中；target 与含地址诊断只写私有文件。Ctrl-C 结束。', flush=True)
                    announced = True
                time.sleep(.5)
    except KeyboardInterrupt:
        pass
    finally:
        probe.shutdown()


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    for name in ('root', 'library', 'bindings'):
        parser.add_argument('--' + name, type=Path, required=True)
    parser.add_argument('--resources', type=Path)
    parser.add_argument('--prepare-only', action='store_true')
    parser.add_argument('--transport-probe', action='store_true')
    parser.add_argument('--public-infrastructure', action='store_true')
    parser.add_argument('--mdns', action='store_true')
    args = parser.parse_args()
    if args.transport_probe and args.prepare_only:
        parser.error('transport-probe 与 prepare-only 不可合用')
    if not args.transport_probe and args.resources is None:
        parser.error('业务宿主需要 --resources')
    if (args.public_infrastructure or args.mdns) and not args.transport_probe:
        parser.error('public-infrastructure/mdns 仅用于显式 S1 transport-probe')
    if args.mdns and args.public_infrastructure:
        parser.error('S1 mDNS 判别禁用公共设施，避免混入公网查找结果')
    for name in ('library', 'bindings') + (('resources',) if args.resources is not None else ()):
        path = getattr(args, name)
        if not path.is_absolute() or not path.exists():
            parser.error(name + ' 必须是存在的绝对路径')
    if not args.root.is_absolute():
        parser.error('root 必须是绝对路径')
    root = args.root
    root.mkdir(parents=True, exist_ok=True, mode=0o700)
    os.chmod(root, 0o700)
    for name in ('bindings', 'user-home') + (() if args.transport_probe else ('project',)):
        (root / name).mkdir(exist_ok=True)
    shutil.copy2(args.bindings / 'velune_bindings.py', root / 'bindings' / 'velune_bindings.py')
    library = root / 'bindings' / args.library.name
    if args.transport_probe:
        if library.is_symlink():
            library.unlink()
        shutil.copy2(args.library, library)
    elif not library.exists():
        library.symlink_to(args.library)
    os.environ.clear()
    os.environ.update(HOME=str(root / 'user-home'), PATH='/usr/bin:/bin',
                      NO_PROXY='127.0.0.1,localhost')
    spec = importlib.util.spec_from_file_location('velune_bindings', root / 'bindings' / 'velune_bindings.py')
    b = importlib.util.module_from_spec(spec)
    sys.modules[spec.name] = b
    spec.loader.exec_module(b)
    if args.transport_probe:
        serve_transport_probe(b, args)
        return
    fixture = runpy.run_path(str(Path(__file__).with_name('manual-remote-control.py')), run_name='fixture')
    application = b.VeluneApplication.open(b.BindingOptions(
        home_directory=str(root / 'application'), resources_directory=str(args.resources)))
    server = None
    try:
        if not (root / 'fixture.json').exists():
            fixture['configure_peers'](b, application, root)
            default = b.BindingModelSelection.RUNTIME_DEFAULT()
            snapshot = application.create_conversation_with_selection('beta', str(root / 'project'), default).snapshot
            application.send_turn_with_selection('beta', snapshot.conversation.id, 'beta', default,
                b.BindingComposedTurn(text='ANDROID_SYNTHETIC_HELLO', effort=None, attachments=[]))
            fixture['wait_for'](lambda: application.snapshot('beta', snapshot.conversation.id).snapshot.actions.can_send,
                                '合成会话完成')
            private_json(root / 'fixture.json', {'runtime': 'beta', 'conversation': snapshot.conversation.id,
                'application_home': str(root / 'application'), 'cwd': str(root / 'project')})
        configure_questions(b, application, root)
        if args.prepare_only:
            print('合成 HOME 已准备；执行端已关闭，可交给隔离 Mac app 独占打开。')
            return
        server = b.VeluneRemoteEndpoint.open(b.BindingRemoteOptions(
            identity_file=str(root / 'identity.json'), trust_file=str(root / 'trust.json'),
            device_name='Synthetic Android acceptance host', relay=b.BindingRelayConfig.DEVELOPMENT_PUBLIC(),
            discovery=b.BindingDiscoveryConfig.DEVELOPMENT_PUBLIC(), bind_addresses=['0.0.0.0:0'],
            request_timeout_ms=30000, path_policy=b.BindingRemotePathPolicy.AUTOMATIC), application)
        def invite():
            value = server.create_invitation(600)
            descriptor = os.open(root / 'invitation.txt', os.O_WRONLY | os.O_CREAT | os.O_TRUNC, 0o600)
            with os.fdopen(descriptor, 'w') as output:
                output.write(value.encoded)
        invite()
        print('合成 iroh 宿主运行中；邀请已写入私有文件。Ctrl-C 结束。', flush=True)
        while True:
            command = root / 'command.json'
            if command.exists():
                value = json.loads(command.read_text())
                command.unlink()
                if value['operation'] == 'invite':
                    invite()
                elif value['operation'] == 'release':
                    (root / 'hold').unlink(missing_ok=True)
                elif value['operation'] == 'revoke':
                    for device in server.trusted_devices():
                        server.revoke(device.id)
                else:
                    raise ValueError('未知人工控制命令')
            devices = server.trusted_devices()
            private_json(root / 'status.json', {'paired_devices': len(devices)})
            time.sleep(.2)
    except KeyboardInterrupt:
        pass
    finally:
        if server is not None:
            server.shutdown()
        application.shutdown()


if __name__ == '__main__':
    main()
