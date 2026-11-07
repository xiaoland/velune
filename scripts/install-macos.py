#!/usr/bin/env python3
"""Install a verified daily bundle after the application has exited normally."""

import argparse
import os
from pathlib import Path
import plistlib
import subprocess
import tempfile

from macos_identity import daily_home, identity, validate_bundle

ROOT = Path(__file__).resolve().parent.parent


def bundle_info(bundle):
    info = plistlib.loads((bundle / "Contents/Info.plist").read_bytes())
    if not isinstance(info, dict):
        raise ValueError("应用的 Info.plist 必须为字典。")
    return info


def running_instances(bundle_id):
    rows = subprocess.check_output(["/bin/ps", "-axo", "pid=,comm="], text=True)
    found = []
    for row in rows.splitlines():
        parts = row.strip().split(None, 1)
        if len(parts) != 2:
            continue
        executable = Path(parts[1])
        if (not executable.is_absolute() or executable.parent.name != "MacOS"
                or executable.parent.parent.name != "Contents"):
            continue
        bundle = executable.parents[2]
        try:
            info = bundle_info(bundle)
            name = info.get("CFBundleExecutable")
            if not isinstance(name, str) or not name or Path(name).name != name:
                continue
            declared_executable = bundle / "Contents/MacOS" / name
            if (executable.resolve() == declared_executable.resolve()
                    and info.get("CFBundleIdentifier") == bundle_id):
                found.append((int(parts[0]), str(executable)))
        except (OSError, ValueError):
            continue
    return found


def describe(instances):
    return "\n".join(f"  PID {pid}：{executable}" for pid, executable in instances)


def verify(bundle):
    subprocess.run(["/usr/bin/codesign", "--verify", "--deep", "--strict", str(bundle)], check=True)


def validate_destination(destination):
    if destination.is_symlink():
        raise RuntimeError("安装目标是符号链接，未进行覆盖。")
    if not destination.exists():
        return
    info = bundle_info(destination)
    if "VeluneExecutionContext" in info:
        validate_bundle(destination, "daily")
        return
    # This exception only admits the explicitly selected legacy installation.
    # New source bundles always require sealed metadata above.
    daily = identity(ROOT, "daily")
    if (info.get("CFBundleIdentifier") != daily["bundleIdentifier"]
            or info.get("CFBundleExecutable") != daily["executableName"]):
        raise RuntimeError("目标位置存在不同的应用，未进行覆盖。")


def preflight(source, destination):
    # Admission precedes codesign, process inspection, quit, and filesystem writes.
    validate_bundle(source, "daily")
    info = bundle_info(source)
    daily_home()
    validate_destination(destination)
    verify(source)
    return info


def install(source, destination):
    info = preflight(source, destination)
    identifier = info["CFBundleIdentifier"]
    found = running_instances(identifier)
    if found:
        raise RuntimeError("Velune 尚未退出。请先停止活跃任务并正常退出应用，再重新安装；安装器不会强制终止任务。仍在运行的实例：\n" + describe(found))
    destination.parent.mkdir(parents=True, exist_ok=True)
    with tempfile.TemporaryDirectory(prefix=".velune-install-", dir=destination.parent) as temporary:
        staging = Path(temporary) / "Velune.app"
        backup = Path(temporary) / "previous.app"
        subprocess.run(["/usr/bin/ditto", str(source), str(staging)], check=True)
        validate_bundle(staging, "daily")
        verify(staging)
        # Check again after copying so a launched app cannot be replaced accidentally.
        found = running_instances(identifier)
        if found:
            raise RuntimeError("复制期间 Velune 已启动，未替换应用：\n" + describe(found))
        validate_destination(destination)
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
    parser = argparse.ArgumentParser(description="安装已构建的 Velune 日常版；不强制退出应用或读取应用数据。")
    parser.add_argument("source", type=Path)
    parser.add_argument("--destination", type=Path, default=Path("/Applications/Velune.app"))
    parser.add_argument("--validate-only", action="store_true", help="只验证日常来源、安装目标与状态目录，不退出或安装应用")
    args = parser.parse_args()
    try:
        source, destination = args.source.resolve(), args.destination.absolute()
        if args.validate_only:
            preflight(source, destination)
            print("日常安装准入验证通过。")
        else:
            install(source, destination)
    except (OSError, ValueError, RuntimeError, subprocess.CalledProcessError) as error:
        parser.exit(1, f"安装未完成：{error}\n")


if __name__ == "__main__":
    main()
