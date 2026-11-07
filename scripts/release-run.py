#!/usr/bin/env python3
"""Create and validate the immutable build identity for one release tag."""

from __future__ import annotations

import argparse
import json
from pathlib import Path
import re
import subprocess
import sys

ROOT = Path(__file__).resolve().parents[1]
sys.path.insert(0, str(ROOT / "scripts"))
from version import read_build_number, read_version  # noqa: E402


def full_tag_commit(tag: str) -> str:
    commit = subprocess.check_output(["git", "rev-parse", f"{tag}^{{commit}}"], cwd=ROOT, text=True).strip()
    if not re.fullmatch(r"[0-9a-f]{40,64}", commit):
        raise ValueError("tag did not resolve to a full Git object ID")
    return commit


def write(args: argparse.Namespace) -> None:
    version, _ = read_version()
    if args.tag != f"v{version}":
        raise ValueError(f"release tag {args.tag} does not match version.txt {version}")
    if not re.fullmatch(r"[1-9][0-9]*", args.build_number):
        raise ValueError("build number must be a positive decimal integer")
    build_number = read_build_number(args.build_number)
    commit = full_tag_commit(args.tag)
    if commit != args.commit:
        raise ValueError(f"release tag resolves to {commit}, expected {args.commit}")
    result = {
        "tag": args.tag,
        "version": version,
        "source_commit": commit,
        "build_number": build_number,
        "workflow_run_id": args.run_id,
        "workflow_run_number": args.run_number,
    }
    output = Path(args.output)
    output.parent.mkdir(parents=True, exist_ok=True)
    output.write_text(json.dumps(result, indent=2) + "\n")


def verify(args: argparse.Namespace) -> None:
    metadata = json.loads(Path(args.metadata).read_text())
    commit = full_tag_commit(args.tag)
    tag_version = subprocess.check_output(["git", "show", f"{args.tag}:version.txt"], cwd=ROOT, text=True).strip()
    if metadata.get("tag") != args.tag:
        raise ValueError("release metadata tag does not match requested tag")
    if metadata.get("source_commit") != commit:
        raise ValueError("release metadata source commit does not match tag object")
    if tag_version != metadata.get("version") or args.tag != f"v{tag_version}":
        raise ValueError("tagged version.txt does not match release metadata")
    if args.commit and commit != args.commit:
        raise ValueError("tag commit does not match the requested source commit")
    read_build_number(str(metadata.get("build_number", "")))
    print(json.dumps(metadata, sort_keys=True))


def main() -> int:
    parser = argparse.ArgumentParser(description=__doc__)
    commands = parser.add_subparsers(dest="command", required=True)
    create = commands.add_parser("write")
    create.add_argument("--tag", required=True)
    create.add_argument("--commit", required=True)
    create.add_argument("--build-number", required=True)
    create.add_argument("--run-id", required=True)
    create.add_argument("--run-number", required=True)
    create.add_argument("--output", required=True)
    create.set_defaults(handler=write)
    check = commands.add_parser("verify")
    check.add_argument("--metadata", required=True)
    check.add_argument("--tag", required=True)
    check.add_argument("--commit")
    check.set_defaults(handler=verify)
    args = parser.parse_args()
    try:
        args.handler(args)
    except (OSError, ValueError, subprocess.CalledProcessError, json.JSONDecodeError) as error:
        print(f"release run: {error}", file=sys.stderr)
        return 2
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
