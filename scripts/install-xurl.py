#!/usr/bin/env python3
"""Build and stage the official xURL CLI and license from one pinned Git tree."""

from pathlib import Path
import os
import shutil
import subprocess
import sys
import tempfile

SOURCE = "https://github.com/Xuanwo/xurl"
REVISION = "24eedb2ff2b141b025efa1a1658fd7f3e88cdd01"
ROOT = Path(__file__).resolve().parents[1]
DESTINATION = ROOT / "target/runtime-support/xurl"


def install(destination: Path, repository: str, revision: str) -> None:
    destination.parent.mkdir(parents=True, exist_ok=True)
    with tempfile.TemporaryDirectory(prefix=".xurl-", dir=destination.parent) as temporary:
        temporary = Path(temporary)
        source = temporary / "source"
        subprocess.run(["git", "init", "--quiet", str(source)], check=True)
        subprocess.run(
            ["git", "fetch", "--quiet", "--depth=1", repository, revision], cwd=source, check=True
        )
        resolved = subprocess.check_output(["git", "rev-parse", "FETCH_HEAD"], cwd=source, text=True).strip()
        if resolved != revision:
            raise ValueError(f"xURL source resolved to {resolved}, expected {revision}")
        subprocess.run(
            ["git", "-c", "core.autocrlf=false", "checkout", "--quiet", "--detach", resolved],
            cwd=source, check=True,
        )

        stage = temporary / "install"
        subprocess.run(
            ["cargo", "install", "--path", "xurl-cli", "--locked", "--root", str(stage), "--no-track"],
            cwd=source, check=True,
        )
        executable = Path("bin/xurl.exe" if os.name == "nt" else "bin/xurl")
        subprocess.run([str(stage / executable), "--version"], check=True)
        shutil.copyfile(source / "LICENSE", stage / "LICENSE")
        (stage / "REVISION").write_bytes(f"{resolved}\n".encode("ascii"))

        # A failed source lookup, build or version check leaves the current installation in place.
        (destination / "bin").mkdir(parents=True, exist_ok=True)
        for file in (executable, Path("LICENSE"), Path("REVISION")):
            (stage / file).replace(destination / file)


def main() -> int:
    try:
        install(DESTINATION, SOURCE, REVISION)
    except (OSError, ValueError, subprocess.CalledProcessError) as error:
        print(f"xURL install: {error}", file=sys.stderr)
        return 1
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
