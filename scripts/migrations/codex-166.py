#!/usr/bin/env python3
"""显式迁移本机 Codex 实例到 codex-0.166；预览为默认，退出 Velune 后 --apply。

不给 --runtime 时自检并处理全部待迁移的本机实例与关联段；没有待迁移内容
时以 0 退出，供 scripts/run-migrations.py 在重装时自动执行。
"""
import argparse
import fcntl
import json
import os
from pathlib import Path
import re
import subprocess
import sys
import tempfile

parser = argparse.ArgumentParser(description=__doc__)
parser.add_argument('--home', type=Path, required=True, help='Velune 状态目录（不是 CODEX_HOME）')
parser.add_argument('--runtime', help='保留原实例 ID；缺省时处理全部待迁移的本机实例')
parser.add_argument('--apply', action='store_true', help='在原目录保留备份并写入迁移结果')
args = parser.parse_args()
root = args.home.expanduser().resolve(strict=True)
with (root / 'runtime.lock').open('a+b') as lock:
    # Match the application's exclusive installation lock; never edit a running core.
    try:
        fcntl.flock(lock, fcntl.LOCK_EX | fcntl.LOCK_NB)
    except BlockingIOError:
        parser.error('请先退出使用该状态目录的 Velune，再执行迁移')
    config_path = root / 'generic-config.json'
    if not config_path.exists():
        print('尚无应用配置，无需迁移')
        sys.exit(0)
    config = json.loads(config_path.read_bytes())
    links_path = root / 'conversation-links.json'
    links = json.loads(links_path.read_bytes()) if links_path.exists() else None
    if links is not None and links['schemaVersion'] != 1:
        parser.error('不支持此会话关联格式，原文件未修改')
    if args.runtime:
        targets = [args.runtime]
    else:
        pending = {r['id'] for r in config['runtimeInstances'] if r['typeId'] == 'codex-0.159'}
        for link in (links or {'links': []})['links']:
            for segment in link['segments']:
                if segment['runtimeTypeId'] == 'codex-0.159':
                    pending.add(segment['runtimeInstanceId'])
        known = {r['id'] for r in config['runtimeInstances']}
        for missing in sorted(pending - known):
            print(f'关联段引用的实例 {missing} 已不存在，跳过其修复')
        pending &= known
        if not pending:
            print('没有 Codex 0.159 实例或关联段，无需迁移')
            sys.exit(0)
        targets = sorted(pending)
    if config['schemaVersion'] != 11:
        parser.error('此脚本仅处理 schema 11，不执行其他配置升级')
    changes = {}
    for runtime_id in targets:
        runtime = next((r for r in config['runtimeInstances'] if r['id'] == runtime_id), None)
        if runtime is None or runtime['typeId'] not in ('codex-0.159', 'codex-0.166'):
            parser.error(f'请选择已有的 Codex 实例：{runtime_id}')
        execution = json.loads(runtime['settings'].get('execution', '{"transport":"local"}'))
        if execution['transport'] != 'local':
            parser.error(f'实例 {runtime_id} 在 SSH 执行机器上；先在该机器确认 CLI 0.166，再按开发文档手动修改类型与关联段')
        version = subprocess.run([runtime['settings']['binary'], '--version'], check=True, text=True, capture_output=True, timeout=10).stdout.strip()
        if not re.fullmatch(r'codex-cli 0\.166\.\d+(?:[-+][^\s]+)?', version):
            parser.error(f'实例 {runtime_id} 所配置的可执行文件不是 Codex CLI 0.166.x')
        if runtime['typeId'] != 'codex-0.166':
            runtime['typeId'] = 'codex-0.166'
            changes[config_path] = config
        count = 0
        for link in (links or {'links': []})['links']:
            for segment in link['segments']:
                if segment['runtimeInstanceId'] == runtime_id and segment['runtimeTypeId'] == 'codex-0.159':
                    segment['runtimeTypeId'] = 'codex-0.166'
                    count += 1
        if count:
            changes[links_path] = links
        print(f'{runtime_id}: {version} → codex-0.166；更新 {count} 个关联段')
    if args.apply and changes:
        backup = Path(tempfile.mkdtemp(prefix='before-codex-166-', dir=root))
        for path in changes:
            target = backup / path.name
            with target.open('xb') as output:
                os.fchmod(output.fileno(), 0o600)
                output.write(path.read_bytes())
        # Each replacement is atomic. An interrupted multi-file update can be
        # rerun: already-updated instances still repair their remaining segments.
        for path, value in changes.items():
            fd, temporary = tempfile.mkstemp(prefix='.codex-166-', dir=root)
            with os.fdopen(fd, 'w') as output:
                json.dump(value, output, ensure_ascii=False, indent=2)
                output.write('\n')
                output.flush()
                os.fsync(output.fileno())
            os.replace(temporary, path)
        print(f'已迁移；原文件保存在 {backup.name}。请使用包含新适配器的 Velune 打开。')
    elif not args.apply:
        print('仅预览，未修改配置；安装包含新适配器的 Velune 后，退出应用并加 --apply 执行。')
