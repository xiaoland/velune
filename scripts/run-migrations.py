#!/usr/bin/env python3
"""Velune 退出后，手动执行旧开发格式的过渡迁移或数据恢复。

按文件名顺序对指定状态目录运行 scripts/migrations/ 下的脚本，并传入 --apply。
任一脚本失败时停止，处理所报问题后可重试。正常更新由应用在启动时迁移。
"""
import argparse
import os
from pathlib import Path
import subprocess
import sys


def probe_lock(root):
    """占用检测只用于诊断；各迁移脚本写入前自行 flock。"""
    lock_path = root / "runtime.lock"
    lock_path.touch(exist_ok=True)
    busy = False
    with lock_path.open("a+b") as lock:
        try:
            if os.name == "nt":
                import msvcrt
                lock.seek(0)
                msvcrt.locking(lock.fileno(), msvcrt.LK_NBLCK, 1)
            else:
                import fcntl
                fcntl.flock(lock.fileno(), fcntl.LOCK_EX | fcntl.LOCK_NB)
        except (BlockingIOError, OSError):
            busy = True
    if busy:
        print(f"状态目录仍被进程占用：{root}", file=sys.stderr)
        if os.name != "nt":
            subprocess.run(["/usr/sbin/lsof", str(lock_path)], check=False)
        print("请退出占用该目录的进程后重试迁移。", file=sys.stderr)
        return False
    return True


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--home", required=True, type=Path, help="VELUNE_HOME 状态目录")
    args = parser.parse_args()
    root = args.home.expanduser()
    if not root.is_dir():
        print(f"状态目录不存在：{root}，无需迁移")
        return
    root = root.resolve()
    if not probe_lock(root):
        sys.exit(1)
    migrations = sorted((Path(__file__).parent / "migrations").glob("*.py"))
    if not migrations:
        print("没有已注册的迁移，跳过。")
        return
    for script in migrations:
        print(f"迁移：{script.name}", flush=True)
        result = subprocess.run([sys.executable, str(script), "--home", str(root), "--apply"], check=False)
        if result.returncode != 0:
            sys.exit(f"迁移 {script.name} 未完成。请按上述输出处理后重试，完成后再启动新版。")


if __name__ == "__main__":
    main()
