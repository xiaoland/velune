#!/usr/bin/env python3
"""按文件名顺序执行 scripts/migrations/ 的一次性迁移；全部自检且幂等。

重装脚本在安装新版后、启动前对本机状态目录调用本入口；也可单独显式执行。
每个迁移脚本接受 --home 与 --apply，无事可迁移时以 0 退出；任一脚本失败
即停止并以非 0 退出，由调用方决定中止启动。
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
        print("请退出占用该目录的进程后再重装或启动新版；占用期间新版将无法打开此目录。", file=sys.stderr)
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
            sys.exit(f"迁移 {script.name} 未完成；按其上输出处理后再重装或启动新版。")


if __name__ == "__main__":
    main()
