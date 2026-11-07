#!/usr/bin/env python3
"""显式退役 codex-0.159 数据，保留实例身份与原生 HOME；默认预览。

这是本次适配器移除的一次性转换，不定义 Adapter Type 之间的关系。
旧 CLI 不支持新数据时，可用 --binary 指定已安装的 0.166 CLI。
"""
import argparse
import json
import os
from pathlib import Path
import re
import shutil
import subprocess
import sys
import tempfile


def version(binary):
    try:
        result = subprocess.run([str(binary), '--version'], check=True, text=True,
                                capture_output=True, timeout=10)
        return result.stdout.strip()
    except (OSError, subprocess.SubprocessError):
        return None


def compatible(binary):
    value = version(binary)
    return value if value and re.fullmatch(r'codex-cli 0\.166\.\d+(?:[-+][^\s]+)?', value) else None


def selected_binary(runtime, explicit):
    existing = runtime['settings'].get('binary')
    candidates = [explicit] if explicit else [existing,
        '/Applications/ChatGPT.app/Contents/Resources/codex-cli/CodexCLI.app/Contents/MacOS/codex',
        '/Applications/Codex.app/Contents/Resources/codex', shutil.which('codex')]
    for candidate in dict.fromkeys(str(p) for p in candidates if p):
        if result := compatible(candidate):
            return candidate, result
    raise ValueError('没有可用的 Codex CLI 0.166.x；请先安装该 CLI，再用 --binary <绝对路径> 重新执行，原文件未修改')


def atomic_json(path, value):
    fd, temporary = tempfile.mkstemp(prefix='.codex-166-', dir=path.parent)
    try:
        with os.fdopen(fd, 'w') as output:
            json.dump(value, output, ensure_ascii=False, indent=2)
            output.write('\n')
            output.flush()
            os.fsync(output.fileno())
        os.replace(temporary, path)
    finally:
        if os.path.exists(temporary):
            os.unlink(temporary)


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('--home', type=Path, required=True, help='Velune 状态目录，不是 CODEX_HOME')
    parser.add_argument('--runtime', help='仅转换指定实例及其关联；缺省转换全部旧记录')
    parser.add_argument('--binary', type=Path, help='替换为已安装的本机 Codex CLI 0.166 绝对路径')
    parser.add_argument('--apply', action='store_true', help='备份并原子写入；缺省只预览')
    args = parser.parse_args()
    root = args.home.expanduser().resolve(strict=True)
    if args.binary:
        args.binary = args.binary.expanduser()
        if not args.binary.is_absolute():
            parser.error('--binary 必须是绝对路径')
    with (root / 'runtime.lock').open('a+b') as lock:
        try:
            if os.name == 'nt':
                import msvcrt
                lock.seek(0)
                msvcrt.locking(lock.fileno(), msvcrt.LK_NBLCK, 1)
            else:
                import fcntl
                fcntl.flock(lock, fcntl.LOCK_EX | fcntl.LOCK_NB)
        except (BlockingIOError, OSError):
            parser.error('请先退出使用该状态目录的 Velune，再执行迁移')
        config_path = root / 'generic-config.json'
        links_path = root / 'conversation-links.json'
        if not config_path.exists():
            print('尚无应用配置，无需迁移')
            return
        try:
            config = json.loads(config_path.read_bytes())
            links = json.loads(links_path.read_bytes()) if links_path.exists() else None
            if config['schemaVersion'] != 11 or (links and links['schemaVersion'] != 1):
                raise ValueError('不支持此配置或会话关联格式，原文件未修改')
            runtimes = {r['id']: r for r in config['runtimeInstances']}
            old_segments = [s for link in (links or {'links': []})['links'] for s in link['segments']
                            if s['runtimeTypeId'] == 'codex-0.159'
                            and (not args.runtime or s['runtimeInstanceId'] == args.runtime)]
            targets = {r['id'] for r in config['runtimeInstances'] if r['typeId'] == 'codex-0.159'
                       and (not args.runtime or r['id'] == args.runtime)}
            targets |= {s['runtimeInstanceId'] for s in old_segments if s['runtimeInstanceId'] in runtimes}
            if args.runtime:
                if args.runtime not in runtimes or runtimes[args.runtime]['typeId'] not in ('codex-0.159', 'codex-0.166'):
                    raise ValueError('请选择已有的 Codex 实例')
                targets.add(args.runtime)
            if not targets and not old_segments:
                print('没有已退役的 Codex 0.159 记录，无需迁移')
                return
            changes = {}
            for identity in sorted(targets):
                runtime = runtimes[identity]
                if runtime['typeId'] not in ('codex-0.159', 'codex-0.166'):
                    raise ValueError('旧关联所指实例已改为其他类型，请先人工核对；原文件未修改')
                execution = json.loads(runtime['settings'].get('execution', '{"transport":"local"}'))
                if execution['transport'] != 'local':
                    raise ValueError('SSH 实例需先在执行机器核实 0.166 CLI，再显式修改类型、binary 和对应关联；原文件未修改')
                binary, cli_version = selected_binary(runtime, args.binary)
                if runtime['typeId'] != 'codex-0.166' or runtime['settings'].get('binary') != binary:
                    runtime['typeId'] = 'codex-0.166'
                    runtime['settings']['binary'] = binary
                    changes[config_path] = config
                print(f'{identity}: 使用 {cli_version}；实例 ID、HOME 与其余设置保持')
            for segment in old_segments:
                segment['runtimeTypeId'] = 'codex-0.166'
            if old_segments:
                changes[links_path] = links
                print(f'转换 {len(old_segments)} 个关联段（包含已移除实例的历史关联），原生会话身份保持')
            if not args.apply:
                print('仅预览，原文件未修改；加 --apply 执行')
                return
            if not changes:
                print('已经转换完成，无需修改')
                return
            backup = Path(tempfile.mkdtemp(prefix='before-codex-166-', dir=root))
            for path in changes:
                with (backup / path.name).open('xb') as output:
                    if os.name != 'nt':
                        os.fchmod(output.fileno(), 0o600)
                    output.write(path.read_bytes())
                    output.flush()
                    os.fsync(output.fileno())
            for path, value in changes.items():
                atomic_json(path, value)
            print(f'已转换，原文件完整备份于 {backup.name}')
        except (ValueError, KeyError, TypeError) as error:
            parser.error(str(error) if isinstance(error, ValueError) else '配置结构无效，原文件未修改')


if __name__ == '__main__':
    main()
