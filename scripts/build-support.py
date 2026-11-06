#!/usr/bin/env python3
"""Stable outputs and provenance for native build tools; compilation stays theirs."""
import argparse
import fnmatch
import hashlib
import json
import os
from pathlib import Path
import shutil
import subprocess
import sys
import tempfile
import time

ROOT = Path(__file__).resolve().parent.parent
STATE = ROOT / "target/build-state"
sys.path.insert(0, str(Path(__file__).resolve().parent))


def digest(path):
    value = hashlib.sha256()
    with Path(path).open("rb") as source:
        for part in iter(lambda: source.read(1024 * 1024), b""):
            value.update(part)
    return value.hexdigest()


def write(path, data):
    path = Path(path)
    if path.is_file() and path.read_bytes() == data:
        return False
    path.parent.mkdir(parents=True, exist_ok=True)
    fd, temporary = tempfile.mkstemp(prefix=".velune-write-", dir=path.parent)
    try:
        with os.fdopen(fd, "wb") as output:
            output.write(data)
        os.replace(temporary, path)
    finally:
        if os.path.exists(temporary):
            os.unlink(temporary)
    return True


def copy(source, destination):
    source, destination = Path(source), Path(destination)
    destination.parent.mkdir(parents=True, exist_ok=True)
    if source.is_symlink():
        target = os.readlink(source)
        if destination.is_symlink() and os.readlink(destination) == target:
            return False
        if destination.exists() and destination.is_dir() and not destination.is_symlink():
            shutil.rmtree(destination)
        else:
            destination.unlink(missing_ok=True)
        destination.symlink_to(target, target_is_directory=source.is_dir())
        return True
    if destination.is_symlink():
        destination.unlink()
    if destination.is_file() and source.stat().st_size == destination.stat().st_size and digest(source) == digest(destination):
        # Copying executable permissions must not change the content timestamp.
        os.chmod(destination, source.stat().st_mode & 0o777)
        return False
    fd, temporary = tempfile.mkstemp(prefix=".velune-copy-", dir=destination.parent)
    os.close(fd)
    try:
        shutil.copy2(source, temporary)
        os.replace(temporary, destination)
    finally:
        if os.path.exists(temporary):
            os.unlink(temporary)
    return True


def sync(source, destination, prune=False, includes=()):
    source, destination = Path(source), Path(destination)
    if not source.is_dir():
        raise ValueError(f"同步来源不是目录：{source}")
    if destination.is_symlink():
        raise ValueError(f"拒绝写入符号链接目录：{destination}")
    destination.mkdir(parents=True, exist_ok=True)
    selected = lambda name: not includes or any(fnmatch.fnmatch(name, pattern) for pattern in includes)
    wanted = set()
    for directory, folders, files in os.walk(source, followlinks=False):
        base = Path(directory)
        links = [name for name in folders if (base / name).is_symlink()]
        folders[:] = [name for name in folders if name not in links]
        for name in files + links:
            relative = (base / name).relative_to(source)
            if selected(relative.as_posix()):
                wanted.add(relative.as_posix())
                copy(base / name, destination / relative)
    if prune:
        for directory, folders, files in os.walk(destination, topdown=False, followlinks=False):
            base = Path(directory)
            for name in files + [name for name in folders if (base / name).is_symlink()]:
                path = base / name
                relative = path.relative_to(destination).as_posix()
                if selected(relative) and relative not in wanted:
                    path.unlink()
            for name in folders:
                path = base / name
                if not path.is_symlink() and not any(path.iterdir()):
                    path.rmdir()


def ensure_bindgen():
    subprocess.run(["cargo", "build", "--locked", "--target-dir", "target/build-tools", "-p", "velune-build-tools", "--bin", "velune-bindgen", "-j", os.environ.get("CARGO_BUILD_JOBS", "2")], cwd=ROOT, check=True)
    return ROOT / "target/build-tools/debug" / ("velune-bindgen.exe" if os.name == "nt" else "velune-bindgen")


def binding_inputs(library, language, generator):
    return {
        "library": digest(library), "language": language,
        "generator": digest(generator), "support": digest(__file__),
        "config": {str(p.relative_to(ROOT)): digest(p) for p in sorted((ROOT / "packages").glob("*/uniffi.toml"))},
    }


def generated_valid(directory, stamp):
    return (bool(stamp.get("files"))
            and all((directory / name).is_file() and digest(directory / name) == value for name, value in stamp["files"].items())
            and all(Path(name).is_file() and digest(name) == value for name, value in stamp.get("artifacts", {}).items()))


def load_stamp(directory):
    try:
        value = json.loads((directory / ".velune-bindings.json").read_text())
        if not isinstance(value, dict) or not isinstance(value.get("files"), dict):
            return {}
        if any(Path(name).is_absolute() or ".." in Path(name).parts for name in value["files"]):
            return {}
        return value
    except (OSError, ValueError, TypeError):
        return {}


def bindings(args, verify=False):
    directory, library = Path(args.out_dir).resolve(), Path(args.library).resolve()
    old = load_stamp(directory)
    if verify:
        # Verification must not prepare or compile any tool, especially for
        # --skip-native consumers. Its recorded producer remains explicit.
        generator = Path(args.generator or old.get("generator_path", ""))
        if not generator.is_file():
            raise ValueError("绑定生成工具不可用，请先执行原生构建")
    else:
        generator = Path(args.generator).resolve() if args.generator else ensure_bindgen()
    inputs = binding_inputs(library, args.language, generator)
    artifacts = {str(Path(name).resolve()): digest(name) for name in (args.artifact or [])}
    requested_pair_matches = args.artifact is None or old.get("artifacts", {}) == artifacts
    if old.get("inputs") == inputs and requested_pair_matches and generated_valid(directory, old):
        print(f"绑定保持：{directory}")
        return
    if verify:
        raise ValueError("原生库／绑定不是当前配套产物，请先执行原生构建")
    directory.parent.mkdir(parents=True, exist_ok=True)
    with tempfile.TemporaryDirectory(prefix=".velune-bindgen-", dir=directory.parent) as temporary:
        command = [str(generator)]
        if args.language != "cs":
            command += ["generate", "--language", args.language]
        command += ["--library", str(library), "--out-dir", temporary, "--no-format"]
        subprocess.run(command, cwd=ROOT, check=True)
        produced = {p.relative_to(temporary).as_posix(): digest(p) for p in Path(temporary).rglob("*") if p.is_file()}
        if not produced:
            raise ValueError("绑定生成器没有产生文件")
        # Only the generator's own prior files are removed. Platform-owned
        # module maps and licence files in the same directory are independent.
        for name in old.get("files", {}):
            if name not in produced:
                (directory / name).unlink(missing_ok=True)
        sync(temporary, directory)
        stamp = {"inputs": inputs, "generator_path": str(generator.resolve()), "files": produced, "artifacts": artifacts}
        write(directory / ".velune-bindings.json", (json.dumps(stamp, sort_keys=True, indent=2) + "\n").encode())
        print(f"绑定已更新：{directory}")


def source_snapshot(platform):
    paths = ["Cargo.toml", "Cargo.lock", "rust-toolchain.toml", "VERSION", ".cargo", "packages", "tools", "third-party", f"app/{platform}",
             "scripts/build-support.py", "scripts/build_lock.py", "scripts/android-licenses.py"]
    if platform == "macos":
        paths = ["app/mac" if p == "app/macos" else p for p in paths]
        paths += ["Package.swift", "Package.resolved", "assets", "scripts/build-macos.sh", "scripts/render-app-icon.swift"]
    elif platform == "windows":
        paths += ["assets", "app/mac/Licenses", "scripts/build-windows.ps1"]
    elif platform == "ios":
        paths += ["app/mac/Licenses", "app/mac/ClientTelemetry.swift", "app/mac/TelemetryExporters.swift", "app/mac/TelemetrySettings.swift", "scripts/build-ios.sh", "scripts/build-ios-native.sh"]
    elif platform == "android":
        paths += ["scripts/build-android.sh", "scripts/build-android-native.sh"]
    output = subprocess.check_output(["git", "ls-files", "--cached", "--others", "--exclude-standard", "-z", "--", *paths], cwd=ROOT)
    snapshot = {}
    for name in sorted(set(output.split(b"\0")) - {b""}):
        path = ROOT / os.fsdecode(name)
        if path.name in ("README.md", "AGENTS.md", "AGENTS.local.md"):
            continue
        key = os.fsdecode(name)
        if path.is_symlink():
            info = path.lstat()
            snapshot[key] = [os.readlink(path), info.st_mtime_ns, info.st_ino]
        elif path.is_file():
            info = path.stat()
            # Content alone misses a temporary edit restored during compile.
            snapshot[key] = [digest(path), info.st_mtime_ns, info.st_size, info.st_ino]
        else:
            snapshot[key] = ["missing"]
    # Provenance includes relevant effective compiler settings, never secrets.
    for key in ("RUSTFLAGS", "CARGO_ENCODED_RUSTFLAGS", "RUSTC_WRAPPER", "RUSTC_WORKSPACE_WRAPPER", "CARGO_PROFILE_DEV_DEBUG", "CARGO_PROFILE_DEV_OPT_LEVEL", "CARGO_PROFILE_DEV_INCREMENTAL", "GO", "CC", "SDKROOT", "MACOSX_DEPLOYMENT_TARGET", "IPHONEOS_DEPLOYMENT_TARGET", "ANDROID_NDK_HOME", "VELUNE_RELAYKIT_CC"):
        snapshot["env:" + key] = hashlib.sha256(os.environ.get(key, "").encode()).hexdigest()
    return snapshot


def snapshot_identity(snapshot):
    return hashlib.sha256(json.dumps(snapshot, sort_keys=True).encode()).hexdigest()


def source_identity(platform):
    return snapshot_identity(source_snapshot(platform))


def artifact_identity(platform, profile, command):
    native_profile = "debug" if profile == "dev" else "release"
    ios_sdk = "iphoneos" if "iphoneos" in command or os.environ.get("PLATFORM_NAME") == "iphoneos" else "iphonesimulator"
    ios_target = "aarch64-apple-ios" if ios_sdk == "iphoneos" else "aarch64-apple-ios-sim"
    configuration = "Debug" if profile == "dev" else "Release"
    paths = {
        "macos": [f"target/host-rust/{native_profile}/libvelune_bindings.dylib", f"target/{'macos-dev' if profile == 'dev' else 'macos'}/Velune.app/Contents/MacOS/Velune"],
        "windows": [f"target/host-rust/{native_profile}/velune_bindings.dll", f"target/{'windows-dev' if profile == 'dev' else 'windows'}/Velune/Velune.exe"],
        "ios": [f"target/cross-rust/{ios_target}/{native_profile}/libvelune_bindings.a", f"target/ios/Build/Products/{configuration}-{ios_sdk}/Velune.app/Velune"],
        "android": ["target/android-native/arm64-v8a/libvelune_bindings.so", f"app/android/app/build/outputs/apk/{'debug/app-debug.apk' if profile == 'dev' else 'release/app-release-unsigned.apk'}"],
    }[platform]
    return {p: digest(ROOT / p) for p in paths if (ROOT / p).is_file()}


def run(args):
    from build_lock import FileLock
    command = args.command
    if command and command[0] == "--":
        command = command[1:]
    if not command:
        raise ValueError("run需要构建命令")
    with FileLock(STATE / "locks" / f"{args.platform}.lock"):
        started = time.time()
        before_snapshot = source_snapshot(args.platform)
        before = snapshot_identity(before_snapshot)
        environment = {**os.environ, "VELUNE_BUILD_MANAGED": args.platform}
        environment.setdefault("CARGO_BUILD_JOBS", "2")
        result = subprocess.run(command, cwd=ROOT, env=environment)
        after_snapshot = source_snapshot(args.platform)
        after = snapshot_identity(after_snapshot)
        code = result.returncode
        if code == 0 and before != after:
            code = 3
            print("构建期间输入已变化；产物保留，本次不作为当前源码的通过结果。请重新请求构建。", file=sys.stderr)
        record = {"platform": args.platform, "profile": args.profile, "command": command,
                  "input_before": before, "input_after": after, "current": before == after,
                  "exit_code": code, "duration_seconds": round(time.time() - started, 3),
                  "artifacts": artifact_identity(args.platform, args.profile, command),
                  "changed_inputs": sorted(name for name in before_snapshot.keys() | after_snapshot.keys() if before_snapshot.get(name) != after_snapshot.get(name))}
        data = (json.dumps(record, indent=2, ensure_ascii=False) + "\n").encode()
        path = STATE / "results" / f"{args.platform}-{args.profile}.json"
        write(path, data)
        if code == 0:
            write(path.with_name(path.stem + "-success.json"), data)
        return code


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    actions = parser.add_subparsers(dest="action", required=True)
    p = actions.add_parser("copy"); p.add_argument("source"); p.add_argument("destination")
    p = actions.add_parser("sync"); p.add_argument("source"); p.add_argument("destination"); p.add_argument("--prune", action="store_true"); p.add_argument("--include", action="append", default=[])
    p = actions.add_parser("write"); p.add_argument("path")
    for name in ("bindgen", "verify-bindings"):
        p = actions.add_parser(name); p.add_argument("--library", required=True); p.add_argument("--language", choices=("swift", "kotlin", "cs"), required=True); p.add_argument("--generator"); p.add_argument("--out-dir", required=True); p.add_argument("--artifact", action="append")
    p = actions.add_parser("run"); p.add_argument("--platform", choices=("macos", "ios", "android", "windows"), required=True); p.add_argument("--profile", choices=("dev", "release"), required=True); p.add_argument("command", nargs=argparse.REMAINDER)
    args = parser.parse_args()
    try:
        if args.action == "copy": copy(args.source, args.destination)
        elif args.action == "sync": sync(args.source, args.destination, args.prune, args.include)
        elif args.action == "write": write(args.path, sys.stdin.buffer.read())
        elif args.action in ("bindgen", "verify-bindings"): bindings(args, args.action == "verify-bindings")
        elif args.action == "run": return run(args)
    except (OSError, ValueError, subprocess.CalledProcessError) as error:
        print(f"构建失败：{error}", file=sys.stderr)
        return 1
    return 0


if __name__ == "__main__":
    sys.exit(main())
