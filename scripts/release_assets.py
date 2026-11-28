#!/usr/bin/env python3
"""Own the public release targets and validate the exact upload file set."""

from __future__ import annotations

import argparse
from collections.abc import Sequence
import hashlib
import json
from pathlib import Path
import sys

# Windows packaging has its own manual entry and is outside the public matrix.
PUBLIC_TARGETS = (
    ("macos", "arm64", "macos-15"),
    ("macos", "x86_64", "macos-15-intel"),
    ("android", "arm64", "ubuntu-24.04"),
)
ANDROID_CERTIFICATE = "babd7b659b691b52ca5a7412c68f2e52abed05d0b0aeb1c56e203d548dbda4bb"
BUILD_IDENTITY = "release-build.json"
CHECKSUMS = "SHA256SUMS.txt"


def artifact_name(platform: str, architecture: str, tag: str) -> str:
    if platform == "source":
        return f"velune-{tag}-source.tar.gz"
    if platform == "macos":
        return f"Velune-{tag}-macOS-{architecture}.zip"
    if platform == "android":
        return f"Velune-{tag}-android-{architecture}.apk"
    raise ValueError(f"unknown public release platform: {platform}")


def target_matrix() -> dict[str, list[dict[str, str]]]:
    return {"include": [
        {"name": f"{platform}-{arch}", "platform": platform, "architecture": arch, "runner": runner}
        for platform, arch, runner in PUBLIC_TARGETS
    ]}


def source_platforms() -> list[str]:
    return sorted({platform for platform, _, _ in PUBLIC_TARGETS})


def sha256(path: Path) -> str:
    value = hashlib.sha256()
    with path.open("rb") as source:
        for block in iter(lambda: source.read(1024 * 1024), b""):
            value.update(block)
    return value.hexdigest()


def verify_record(path: Path, expected: dict[str, object]) -> None:
    record = json.loads(path.read_text(encoding="utf-8"))
    if not isinstance(record, dict):
        raise ValueError(f"{path.name} must contain an object")
    for name, value in expected.items():
        supplied = record.get(name)
        if type(supplied) is not type(value) or supplied != value:
            raise ValueError(f"{path.name} has an unexpected {name}")


def prepare_upload(root: Path, tag: str, version: str, commit: str, build_number: int) -> list[Path]:
    targets = [(platform, arch) for platform, arch, _ in PUBLIC_TARGETS]
    targets.append(("source", "all"))
    artifacts = {artifact_name(platform, arch, tag): (platform, arch) for platform, arch in targets}
    metadata_names = {f"{name}.build.json" for name in artifacts}
    required = set(artifacts) | metadata_names | {BUILD_IDENTITY}
    found = {path.name for path in root.iterdir()}
    missing = required - found
    unexpected = found - required - {CHECKSUMS}
    if missing or unexpected:
        raise ValueError(f"release asset set differs (missing={sorted(missing)}, unexpected={sorted(unexpected)})")
    for name in sorted(found):
        path = root / name
        if path.is_symlink() or not path.is_file() or path.stat().st_size == 0:
            raise ValueError(f"release asset must be a nonempty regular file: {name}")

    identity = {"tag": tag, "version": version, "source_commit": commit, "build_number": build_number}
    verify_record(root / BUILD_IDENTITY, identity)
    digests = {BUILD_IDENTITY: sha256(root / BUILD_IDENTITY)}
    for name, (platform, arch) in artifacts.items():
        path = root / f"{name}.build.json"
        checks: dict[str, object] = {**identity, "artifact": name, "platform": platform, "architecture": arch}
        if platform == "source":
            checks["corresponding_platforms"] = source_platforms()
        else:
            checks["binary_version"] = version
        if platform == "android":
            checks["certificate_sha256"] = ANDROID_CERTIFICATE
        digest = sha256(root / name)
        verify_record(path, {**checks, "sha256": digest})
        digests[name] = digest
        digests[path.name] = sha256(path)

    (root / CHECKSUMS).write_text(
        "".join(f"{digests[name]}  {name}\n" for name in sorted(digests)), encoding="utf-8"
    )
    return [root / name for name in sorted(required | {CHECKSUMS})]


def main(argv: Sequence[str] | None = None) -> int:
    parser = argparse.ArgumentParser(description=__doc__)
    commands = parser.add_subparsers(dest="command", required=True)
    commands.add_parser("workflow-outputs")
    upload = commands.add_parser("prepare-upload")
    upload.add_argument("--directory", type=Path, required=True)
    upload.add_argument("--tag", required=True)
    upload.add_argument("--version", required=True)
    upload.add_argument("--commit", required=True)
    upload.add_argument("--build-number", type=int, required=True)
    args = parser.parse_args(argv)

    try:
        if args.command == "workflow-outputs":
            print(f"matrix={json.dumps(target_matrix(), separators=(',', ':'))}")
            print(f"source_platforms={' '.join(source_platforms())}")
            print(f"android_certificate={ANDROID_CERTIFICATE}")
        else:
            print(*prepare_upload(args.directory, args.tag, args.version, args.commit, args.build_number), sep="\n")
    except (OSError, ValueError) as error:
        print(f"release assets: {error}", file=sys.stderr)
        return 2
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
