#!/usr/bin/env python3
"""显式准备 Android / Mac 人工验收的合成 ACP 宿主，不读取真实配置。

先 --prepare-only 生成独立 application HOME，或保持前台运行 iroh 宿主。
邀请只写入权限 0600 的 invitation.txt；不打印邀请或业务正文。
"""
import argparse
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


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    for name in ('root', 'library', 'bindings', 'resources'):
        parser.add_argument('--' + name, type=Path, required=True)
    parser.add_argument('--prepare-only', action='store_true')
    args = parser.parse_args()
    for name in ('library', 'bindings', 'resources'):
        path = getattr(args, name)
        if not path.is_absolute() or not path.exists():
            parser.error(name + ' 必须是存在的绝对路径')
    if not args.root.is_absolute():
        parser.error('root 必须是绝对路径')
    root = args.root
    root.mkdir(parents=True, exist_ok=True, mode=0o700)
    os.chmod(root, 0o700)
    for name in ('bindings', 'user-home', 'project'):
        (root / name).mkdir(exist_ok=True)
    shutil.copy2(args.bindings / 'velune_bindings.py', root / 'bindings' / 'velune_bindings.py')
    library = root / 'bindings' / args.library.name
    if not library.exists():
        library.symlink_to(args.library)
    os.environ.clear()
    os.environ.update(HOME=str(root / 'user-home'), PATH='/usr/bin:/bin',
                      NO_PROXY='127.0.0.1,localhost')
    spec = importlib.util.spec_from_file_location('velune_bindings', root / 'bindings' / 'velune_bindings.py')
    b = importlib.util.module_from_spec(spec)
    sys.modules[spec.name] = b
    spec.loader.exec_module(b)
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
