#!/usr/bin/env python3
"""Select native consumers for a merge candidate using build input ownership."""

import json
import os
from pathlib import Path
import runpy
import subprocess
import sys


build = runpy.run_path(str(Path(__file__).with_name("build-support.py")))
PLATFORMS = ("macos", "ios", "android", "windows")
WORKFLOW_INPUTS = {
    ".github/workflows/pr-native-consumers.yml",
    "scripts/native-pr-inputs.py",
}


def changed_paths(base):
    names = subprocess.check_output(
        ["git", "diff", "--name-only", "--no-renames", "-z", base, "HEAD", "--"],
    )
    return {os.fsdecode(name) for name in names.split(b"\0") if name}


def consumers(names):
    if names & WORKFLOW_INPUTS:
        return set(PLATFORMS)
    paths = [name for name in names if build["is_build_input"](name)]
    return {
        platform
        for platform in PLATFORMS
        if any(
            path == root or path.startswith(root + "/")
            for root in build["source_inputs"](platform)
            for path in paths
        )
    }


def main(base):
    selected = consumers(changed_paths(base))
    apple = [platform for platform in ("macos", "ios") if platform in selected]
    print(f"apple={json.dumps(apple)}")
    for platform in ("android", "windows"):
        print(f"{platform}={str(platform in selected).lower()}")


if __name__ == "__main__":
    main(sys.argv[1])
