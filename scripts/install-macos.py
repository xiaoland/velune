#!/usr/bin/env python3
"""Install a verified local bundle after the application has exited normally."""

import argparse
import os
from pathlib import Path
import plistlib
import subprocess
import tempfile


def bundle_info(bundle):
    return plistlib.loads((bundle / "Contents/Info.plist").read_bytes())


def running_instances(bundle_id):
    rows = subprocess.check_output(["/bin/ps", "-axo", "pid=,comm="], text=True)
    found = []
    for row in rows.splitlines():
        parts = row.strip().split(None, 1)
        if len(parts) != 2 or not parts[1].endswith("/Contents/MacOS/Velune"):
            continue
        executable = Path(parts[1])
        try:
            info = bundle_info(executable.parents[2])
        except (OSError, ValueError):
            continue
        if info.get("CFBundleIdentifier") == bundle_id:
            found.append(int(parts[0]))
    return found


def verify(bundle):
    subprocess.run(["/usr/bin/codesign", "--verify", "--deep", "--strict", str(bundle)], check=True)


def install(source, destination):
    info = bundle_info(source)
    identifier = info["CFBundleIdentifier"]
    verify(source)
    if running_instances(identifier):
        raise RuntimeError("Velune 尚未退出。请先停止活跃任务并正常退出应用，再重新安装；安装器不会强制终止任务。")
    if destination.exists() and bundle_info(destination).get("CFBundleIdentifier") != identifier:
        raise RuntimeError("目标位置存在不同的应用，未进行覆盖。")
    destination.parent.mkdir(parents=True, exist_ok=True)
    with tempfile.TemporaryDirectory(prefix=".velune-install-", dir=destination.parent) as temporary:
        staging = Path(temporary) / "Velune.app"
        backup = Path(temporary) / "previous.app"
        subprocess.run(["/usr/bin/ditto", str(source), str(staging)], check=True)
        verify(staging)
        # Check again after copying so a launched app cannot be replaced accidentally.
        if running_instances(identifier):
            raise RuntimeError("复制期间 Velune 已启动，未替换应用。")
        if destination.exists():
            os.replace(destination, backup)
        try:
            os.replace(staging, destination)
            verify(destination)
        except Exception:
            if destination.exists():
                os.replace(destination, staging)
            if backup.exists():
                os.replace(backup, destination)
            raise
    print(f"已安装 {info.get('VeluneDisplayVersion', info['CFBundleShortVersionString'])}：{destination}")


def main():
    parser = argparse.ArgumentParser(description="安装已构建的 Velune；不强制退出应用或读取应用数据。")
    parser.add_argument("source", type=Path)
    parser.add_argument("--destination", type=Path, default=Path("/Applications/Velune.app"))
    args = parser.parse_args()
    try:
        install(args.source.resolve(), args.destination.absolute())
    except (OSError, ValueError, RuntimeError, subprocess.CalledProcessError) as error:
        parser.exit(1, f"安装未完成：{error}\n")


if __name__ == "__main__":
    main()
