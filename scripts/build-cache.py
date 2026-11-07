#!/usr/bin/env python3
"""Report and explicitly prune only Velune's bounded, regenerable build caches."""
from __future__ import annotations

import argparse
import os
from pathlib import Path
import shutil
import stat
import sys

from build_lock import FileLock


DOMAINS = {
    "host-rust": Path("target/host-rust"),
    "cross-rust": Path("target/cross-rust"),
    "build-tools": Path("target/build-tools"),
}

# Known historical output directories are visible in reports, but remain
# protected until a replacement build path has been qualified. Add a name
# here only with the task decision that retires that exact legacy directory.
LEGACY = {
    "debug": Path("target/debug"),
    "release": Path("target/release"),
    "ios-rust": Path("target/ios-rust"),
    "ios-bindgen": Path("target/ios-bindgen"),
    "android-bindgen": Path("target/android-bindgen"),
    "android-native": Path("target/android-native"),
    "aarch64-linux-android": Path("target/aarch64-linux-android"),
}
# Qualified by the new host dev / mobile builds and shared generator. The
# shipped applications contain their own native libraries; none uses these
# compiler caches at runtime. Keep old release and packaged outputs protected.
RETIRED_LEGACY: frozenset[str] = frozenset({"debug", "ios-rust", "ios-bindgen", "android-bindgen", "aarch64-linux-android"})
PLATFORMS = ("macos", "ios", "android", "windows")


class UnsafeCache(RuntimeError):
    pass


def is_link(path: Path) -> bool:
    junction = getattr(path, "is_junction", None)
    return path.is_symlink() or (junction is not None and junction())


def contained_path(repo: Path, relative: Path) -> Path:
    if relative.is_absolute() or ".." in relative.parts:
        raise UnsafeCache(f"不接受仓库外路径：{relative}")
    path = repo / relative
    if is_link(path):
        raise UnsafeCache(f"目标是符号链接或 junction：{relative}")
    if path.exists() and path.resolve() != (repo / relative):
        raise UnsafeCache(f"目标解析到了仓库外：{relative}")
    return path


def usage(path: Path) -> tuple[int, int, int]:
    """Return allocated bytes, regular-file count, and skipped link count."""
    if not path.exists():
        return 0, 0, 0
    if is_link(path):
        return 0, 0, 1
    total = files = links = 0
    hardlinks = set()
    stack = [path]
    while stack:
        current = stack.pop()
        with os.scandir(current) as entries:
            for entry in entries:
                info = entry.stat(follow_symlinks=False)
                if stat.S_ISLNK(info.st_mode) or getattr(info, "st_file_attributes", 0) & 0x400:
                    links += 1
                elif stat.S_ISDIR(info.st_mode):
                    stack.append(Path(entry.path))
                elif stat.S_ISREG(info.st_mode):
                    files += 1
                    identity = (info.st_dev, info.st_ino)
                    if info.st_nlink > 1:
                        if identity in hardlinks:
                            continue
                        hardlinks.add(identity)
                    total += info.st_blocks * 512 if hasattr(info, "st_blocks") else info.st_size
    return total, files, links


def gib(value: int) -> str:
    return f"{value / (1024 ** 3):.2f} GiB"


def report(repo: Path) -> None:
    print("Velune 可再生构建缓存（allocated bytes；不含外部链接目标）")
    for name, relative in DOMAINS.items():
        path = contained_path(repo, relative)
        size, files, links = usage(path)
        print(f"  domain {name}: {gib(size)}, {files} files, {links} links")
    print("历史缓存（只报告；未获退役资格前不可清理）")
    for name, relative in LEGACY.items():
        path = contained_path(repo, relative)
        if path.exists():
            status = "可按显式退役清理" if name in RETIRED_LEGACY else "受保护"
            # Some preexisting Cargo target directories contain hundreds of
            # gigabytes. Reporting their metadata must not trigger another
            # deep walk; their size is intentionally left to the operator's
            # existing disk inventory until that exact domain is retired.
            print(f"  {name}: {relative}; {status}，跳过深度扫描")


def cargo_lock(path: Path):
    """Acquire all Cargo locks in the bounded target/profile directory layout."""
    locks = []
    paths = [path / ".cargo-lock"]
    try:
        with os.scandir(path) as entries:
            first_level = [
                Path(entry.path)
                for entry in entries
                if entry.is_dir(follow_symlinks=False) and not entry.is_symlink()
            ]
        for directory in first_level:
            # Cargo's ordinary target-dir lock sits at target-dir/{debug,release}.
            paths.append(directory / ".cargo-lock")
            # Cross-target outputs use target-dir/<triple>/{debug,release}.
            # Only inspect known profile leaves; never walk deps/build artifacts.
            if directory.name not in {"debug", "release", "check"}:
                paths.extend(directory / profile / ".cargo-lock" for profile in ("debug", "release", "check"))

        for lock_path in sorted(set(paths)):
            if not lock_path.exists():
                continue
            if is_link(lock_path):
                raise UnsafeCache(f"Cargo 锁文件是符号链接：{lock_path}")
            lock = FileLock(lock_path, blocking=False)
            try:
                lock.__enter__()
            except (BlockingIOError, OSError):
                raise UnsafeCache(f"Cargo 正在使用 {lock_path}") from None
            locks.append(lock)
        return locks
    except BaseException:
        for lock in reversed(locks):
            lock.__exit__(None, None, None)
        raise


def prune(repo: Path, relative: Path, name: str, budget: int, apply: bool, retired: bool) -> None:
    if retired:
        if name not in RETIRED_LEGACY or LEGACY.get(name) != relative:
            raise UnsafeCache(f"历史目录尚未登记为可替代并退役：{name}")
    elif DOMAINS.get(name) != relative:
        raise UnsafeCache("只能按名称选择已登记的构建域")

    path = contained_path(repo, relative)
    if not path.exists():
        print(f"{relative} 不存在，无需回收")
        return
    if not apply:
        size, files, links = usage(path)
        print(f"{relative}: {gib(size)}, {files} files, {links} links；预算 {gib(budget)}")
        if size <= budget:
            print("未超过预算，不回收")
            return
        if links:
            raise UnsafeCache(f"缓存域内含 {links} 个符号链接或 junction，拒绝整体回收")
        print(f"预览：apply 将整体移除 {relative}；下次相应平台构建会重新生成该缓存")
        return

    target = contained_path(repo, Path("target"))
    acquired = []
    try:
        # A cache may feed more than one platform build, so prune only while
        # every known platform build is idle.
        for platform in PLATFORMS:
            lock = FileLock(target / "build-state" / "locks" / f"{platform}.lock", blocking=False)
            try:
                lock.__enter__()
            except (BlockingIOError, OSError):
                raise UnsafeCache(f"平台构建仍在运行：{platform}") from None
            acquired.append(lock)
        acquired.extend(cargo_lock(path))
        # Recheck identity and size after acquiring the shared build locks.
        if contained_path(repo, relative) != path or is_link(path):
            raise UnsafeCache("锁定后缓存路径身份改变")
        current, files, links = usage(path)
        print(f"{relative}: {gib(current)}, {files} files, {links} links；预算 {gib(budget)}")
        if links:
            raise UnsafeCache(f"缓存域内含 {links} 个符号链接或 junction")
        if current <= budget:
            print("获取锁后缓存已低于预算，不回收")
            return
        shutil.rmtree(path)
        print(f"已回收 {relative}（{gib(current)}）；这是再生缓存，下次相应构建会重新生成")
    finally:
        for lock in reversed(acquired):
            lock.__exit__(None, None, None)


def main(argv: list[str] | None = None) -> int:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--repo", type=Path, default=Path(__file__).resolve().parents[1])
    sub = parser.add_subparsers(dest="action", required=True)
    sub.add_parser("report", help="报告登记的构建缓存和历史目录")
    clean = sub.add_parser("prune", help="按缓存域预算预览或回收")
    clean.add_argument("--domain", choices=DOMAINS)
    clean.add_argument("--retired", choices=LEGACY, help="仅清理已验证退役并登记的历史域")
    clean.add_argument("--budget-gib", type=float, required=True, help="缓存保留预算；超过后整体回收该域")
    clean.add_argument("--apply", action="store_true", help="执行预览中的回收")
    args = parser.parse_args(argv)
    try:
        repo = args.repo.resolve(strict=True)
        target = contained_path(repo, Path("target"))
        if not target.is_dir():
            raise UnsafeCache("仓库 target 目录不存在")
        if args.action == "report":
            report(repo)
            return 0
        if bool(args.domain) == bool(args.retired):
            raise UnsafeCache("prune 必须且只能选择 --domain 或 --retired")
        if not (0 <= args.budget_gib <= 1_048_576):
            raise UnsafeCache("预算须为 0 到 1,048,576 GiB")
        name = args.domain or args.retired
        relative = DOMAINS[name] if args.domain else LEGACY[name]
        prune(repo, relative, name, int(args.budget_gib * 1024 ** 3), args.apply, bool(args.retired))
        if not args.apply:
            print("预览结束，磁盘内容未修改")
        return 0
    except (OSError, UnsafeCache) as error:
        print(f"无法安全处理构建缓存：{error}", file=sys.stderr)
        return 2


if __name__ == "__main__":
    raise SystemExit(main())
