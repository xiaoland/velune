#!/usr/bin/env python3
"""Read the product SemVer and platform release build number."""

from __future__ import annotations

import argparse
import os
import re
import sys
from pathlib import Path


SEMVER = re.compile(
    r"^(0|[1-9][0-9]*)\.(0|[1-9][0-9]*)\.(0|[1-9][0-9]*)"
    r"(?:-([0-9A-Za-z-]+(?:\.[0-9A-Za-z-]+)*))?"
    r"(?:\+([0-9A-Za-z-]+(?:\.[0-9A-Za-z-]+)*))?$"
)
ANDROID_MAX_VERSION_CODE = 2_100_000_000


def read_version() -> tuple[str, str]:
    version = (Path(__file__).resolve().parents[1] / "version.txt").read_text().strip()
    match = SEMVER.fullmatch(version)
    if match is None:
        raise ValueError(f"version.txt must contain one valid SemVer 2.0 version, got {version!r}")
    prerelease = match.group(4)
    if prerelease is not None:
        for identifier in prerelease.split("."):
            if identifier.isdigit() and len(identifier) > 1 and identifier.startswith("0"):
                raise ValueError(f"numeric prerelease identifiers cannot have leading zeroes: {identifier!r}")
    return version, ".".join(match.group(i) for i in range(1, 4))


def read_build_number(argument: str | None) -> int:
    raw = argument if argument is not None else os.environ.get("VELUNE_BUILD_NUMBER", "1")
    if not re.fullmatch(r"[1-9][0-9]*", raw):
        raise ValueError("VELUNE_BUILD_NUMBER must be a positive decimal integer")
    number = int(raw)
    if number > ANDROID_MAX_VERSION_CODE:
        raise ValueError(
            f"VELUNE_BUILD_NUMBER must not exceed Android's versionCode limit "
            f"({ANDROID_MAX_VERSION_CODE})"
        )
    return number


def main() -> int:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument(
        "--format",
        choices=("full", "core", "build-number", "build-info"),
        default="full",
    )
    parser.add_argument("--build-number", help="positive release sequence; defaults to VELUNE_BUILD_NUMBER or 1")
    args = parser.parse_args()
    try:
        full, core = read_version()
        number = read_build_number(args.build_number)
    except (OSError, ValueError) as error:
        print(f"version: {error}", file=sys.stderr)
        return 2

    output = {
        "full": full,
        "core": core,
        "build-number": str(number),
        "build-info": "\t".join((full, core, str(number))),
    }[args.format]
    print(output)
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
