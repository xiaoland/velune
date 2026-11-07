#!/usr/bin/env python3
"""Sealed macOS execution identity. Build, launch and install consume one contract."""
import argparse
import hashlib
import json
import os
from pathlib import Path
import plistlib
import pwd
import re
import sys

DAILY_BUNDLE_ID = 'local.velune.prototype'
DAILY_EXECUTABLE = 'Velune'
SCHEMA = 1
MARKER = '.velune-development.json'
ROOT = Path(__file__).resolve().parent.parent


def user_home():
    return Path(pwd.getpwuid(os.getuid()).pw_dir).resolve()


def identity(workspace, purpose):
    if purpose not in ('development', 'daily'):
        raise ValueError('未知的Mac运行用途')
    scope = hashlib.sha256(os.fsencode(str(Path(workspace).resolve()))).hexdigest()[:12]
    bundle_id = DAILY_BUNDLE_ID if purpose == 'daily' else 'local.velune.development.' + scope
    executable = DAILY_EXECUTABLE if purpose == 'daily' else 'VeluneDev'
    context = dict(schemaVersion=SCHEMA, purpose=purpose, bundleIdentifier=bundle_id, executable=executable)
    if purpose == 'development':
        context['scope'] = scope
    return dict(bundleIdentifier=bundle_id, executableName=executable,
                displayName='Velune' if purpose == 'daily' else 'Velune 开发版', context=context)


def app_bundle_path(workspace, profile, purpose='development'):
    if profile not in ('dev', 'release') or purpose not in ('development', 'daily'):
        raise ValueError('未知的Mac构建优化／用途')
    if purpose == 'daily':
        if profile != 'release':
            raise ValueError('日常分发必须使用release优化')
        return Path(workspace) / 'target/macos/Velune.app'
    folder = 'macos-dev' if profile == 'dev' else 'macos-dev-release'
    return Path(workspace) / 'target' / folder / 'Velune Dev.app'


def validate_context(info, purpose=None):
    context = info.get('VeluneExecutionContext')
    if not isinstance(context, dict) or context.get('schemaVersion') != SCHEMA:
        raise ValueError('Mac产物缺少有效运行用途；请重新构建，不回退到日常身份')
    value = context.get('purpose')
    if value not in ('development', 'daily') or (purpose is not None and value != purpose):
        raise ValueError('Mac产物运行用途不符')
    scope = context.get('scope')
    if value == 'development':
        if not isinstance(scope, str) or re.fullmatch('[0-9a-f]{12}', scope) is None:
            raise ValueError('开发产物scope无效')
        bundle_id, executable = 'local.velune.development.' + scope, 'VeluneDev'
    else:
        if scope is not None:
            raise ValueError('日常产物不能携带开发scope')
        bundle_id, executable = DAILY_BUNDLE_ID, DAILY_EXECUTABLE
    if any((info.get('CFBundleIdentifier') != bundle_id,
            info.get('CFBundleExecutable') != executable,
            context.get('bundleIdentifier') != bundle_id,
            context.get('executable') != executable)):
        raise ValueError('Mac产物系统身份与用途元数据不一致')
    schemes = [scheme for row in info.get('CFBundleURLTypes', []) for scheme in row.get('CFBundleURLSchemes', [])]
    if value == 'development' and any(s in schemes for s in ('velune', 'agents')):
        raise ValueError('开发产物不能注册日常链接协议')
    return dict(context)


def validate_bundle(bundle, purpose=None):
    bundle = Path(bundle).resolve()
    info = plistlib.loads((bundle / 'Contents/Info.plist').read_bytes())
    context = validate_context(info, purpose)
    executable = bundle / 'Contents/MacOS' / context['executable']
    if executable.is_symlink() or not executable.is_file() or not os.access(executable, os.X_OK):
        raise ValueError('Mac产物缺少匹配的实际可执行文件')
    return context


def stamp(info, workspace, purpose):
    value = identity(workspace, purpose)
    info = dict(info)
    info.update(CFBundleIdentifier=value['bundleIdentifier'], CFBundleExecutable=value['executableName'],
                CFBundleName=value['displayName'], CFBundleDisplayName=value['displayName'],
                VeluneExecutionContext=value['context'])
    schemes = ['velune', 'agents'] if purpose == 'daily' else ['velune-dev-' + value['context']['scope']]
    info['CFBundleURLTypes'] = [dict(CFBundleURLName=value['bundleIdentifier'] + '.conversation', CFBundleURLSchemes=schemes)]
    return info


def default_development_home(scope, os_home=None):
    if re.fullmatch('[0-9a-f]{12}', scope) is None:
        raise ValueError('无效开发scope')
    return (Path(os_home) if os_home else user_home()) / '.velune-dev' / scope


def inside(path, root):
    return path == root or root in path.parents


def prepare_development_home(directory, scope, os_home=None):
    if re.fullmatch('[0-9a-f]{12}', scope) is None:
        raise ValueError('无效开发scope')
    path = Path(directory)
    if not path.is_absolute():
        raise ValueError('开发目录必须为绝对路径')
    path = path.resolve()
    daily = ((Path(os_home) if os_home else user_home()) / '.velune').resolve()
    if inside(path, daily) or inside(daily, path):
        raise ValueError('开发实例不能使用日常目录或其上级／子目录')
    marker = path / MARKER
    expected = dict(schemaVersion=SCHEMA, purpose='development', scope=scope)
    if path.exists():
        if not path.is_dir() or path.stat().st_uid != os.getuid():
            raise ValueError('开发目录归属无效')
        if marker.exists() or marker.is_symlink():
            if marker.is_symlink() or json.loads(marker.read_text()) != expected:
                raise ValueError('开发目录属于其它运行scope')
        elif any(path.iterdir()):
            raise ValueError('拒绝接管已有、无开发归属的非空目录')
    path.mkdir(parents=True, exist_ok=True, mode=0o700)
    path.chmod(0o700)
    if not marker.exists():
        descriptor = os.open(marker, os.O_WRONLY | os.O_CREAT | os.O_EXCL | os.O_NOFOLLOW, 0o600)
        with os.fdopen(descriptor, 'w') as output:
            json.dump(expected, output, sort_keys=True)
            output.write('\n')
            output.flush()
            os.fsync(output.fileno())
    return path


def daily_home():
    home = (user_home() / '.velune').resolve()
    configured = os.environ.get('VELUNE_HOME', '').strip()
    if configured and (not Path(configured).is_absolute() or Path(configured).resolve() != home):
        raise ValueError('日常打包／重装拒绝继承开发或自定义VELUNE_HOME；请在日常环境中执行')
    if (home / MARKER).exists():
        raise ValueError('日常目录带有开发归属标记，拒绝继续')
    return home


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    commands = parser.add_subparsers(dest='command', required=True)
    for name in ('describe', 'stamp'):
        cmd = commands.add_parser(name)
        cmd.add_argument('--purpose', choices=('development', 'daily'), default='development')
        cmd.add_argument('--root', type=Path, default=ROOT)
        if name == 'describe':
            cmd.add_argument('--profile', choices=('dev', 'release'), default='dev')
    cmd = commands.add_parser('validate')
    cmd.add_argument('--purpose', choices=('development', 'daily'))
    cmd.add_argument('bundle', type=Path)
    commands.add_parser('daily-home')
    args = parser.parse_args()
    if args.command == 'describe':
        value = identity(args.root, args.purpose)
        value['bundlePath'] = str(app_bundle_path(args.root.resolve(), args.profile, args.purpose))
        print(json.dumps(value, ensure_ascii=False))
    elif args.command == 'stamp':
        sys.stdout.buffer.write(plistlib.dumps(stamp(plistlib.loads(sys.stdin.buffer.read()), args.root, args.purpose)))
    elif args.command == 'validate':
        print(json.dumps(validate_bundle(args.bundle, args.purpose), ensure_ascii=False))
    else:
        print(daily_home())


if __name__ == '__main__':
    try:
        main()
    except (OSError, ValueError, KeyError, plistlib.InvalidFileException) as error:
        print(str(error), file=sys.stderr)
        sys.exit(1)
