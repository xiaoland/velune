#!/usr/bin/env python3
"""Small synthetic safety check for scripts/build-cache.py."""
from pathlib import Path
import shutil
import subprocess
import sys
import tempfile
import errno
from unittest.mock import patch

from build_lock import FileLock, _windows_lock


SCRIPT = Path(__file__).with_name("build-cache.py")


def run(repo: Path, *arguments: str) -> subprocess.CompletedProcess[str]:
    return subprocess.run(
        [sys.executable, str(SCRIPT), "--repo", str(repo), *arguments],
        capture_output=True,
        text=True,
        check=False,
    )


def main() -> int:
    class FakeWindowsLock:
        LK_NBLCK = 1

        def __init__(self):
            self.attempts = 0

        def locking(self, _fd, _mode, _size):
            self.attempts += 1
            if self.attempts < 3:
                raise OSError(errno.EACCES, "synthetic lock contention")

    with tempfile.TemporaryFile() as file:
        fake = FakeWindowsLock()
        with patch.dict(sys.modules, {"msvcrt": fake}), patch("build_lock.time.sleep") as sleep:
            _windows_lock(file, blocking=True)
        assert fake.attempts == 3 and sleep.call_count == 2, "Windows blocking lock should wait and retry"

    with tempfile.TemporaryDirectory(prefix="velune-build-cache-") as temporary:
        repo = Path(temporary)
        target = repo / "target"
        cache = target / "host-rust"
        cache.mkdir(parents=True)
        payload = b"synthetic build output" * 128
        artifact = cache / "regenerable.o"
        artifact.write_bytes(payload)

        def assert_refused(result, reason):
            assert result.returncode == 2 and reason in result.stderr, result
            assert artifact.read_bytes() == payload, "refused prune changed the build output"

        result = run(repo, "prune", "--domain", "host-rust", "--budget-gib", "0")
        assert result.returncode == 0, result.stderr
        assert artifact.read_bytes() == payload, "preview changed the build output"

        with FileLock(target / "build-state/locks/macos.lock", blocking=False):
            result = run(repo, "prune", "--domain", "host-rust", "--budget-gib", "0", "--apply")
        assert_refused(result, "平台构建仍在运行：macos")

        cargo_lock_path = cache / ".cargo-lock"
        with FileLock(cargo_lock_path, blocking=False):
            result = run(repo, "prune", "--domain", "host-rust", "--budget-gib", "0", "--apply")
        assert_refused(result, f"Cargo 正在使用 {cargo_lock_path.resolve()}")

        cargo_lock_path.unlink()
        nested_cargo_lock = cache / "aarch64-apple-ios" / "release" / ".cargo-lock"
        nested_cargo_lock.parent.mkdir(parents=True)
        with FileLock(nested_cargo_lock, blocking=False):
            result = run(repo, "prune", "--domain", "host-rust", "--budget-gib", "0", "--apply")
        assert_refused(result, f"Cargo 正在使用 {nested_cargo_lock.resolve()}")
        shutil.rmtree(cache / "aarch64-apple-ios")

        link = cache / "shared"
        try:
            link.symlink_to(repo, target_is_directory=True)
        except (OSError, NotImplementedError) as error:
            print(f"SKIP symlink refusal: could not create the fixture link ({error})")
        else:
            result = run(repo, "prune", "--domain", "host-rust", "--budget-gib", "0", "--apply")
            assert_refused(result, "缓存域内含 1 个符号链接或 junction")
            assert link.is_symlink(), "symlink refusal removed the link"
            link.unlink()
            print("PASS refusal leaves the symlink and cached artifact intact")

        # A real idle Cargo cache retains this file. Windows must delete it
        # without releasing the acquired lock before the removal completes.
        (cache / "debug").mkdir()
        (cache / "debug/.cargo-lock").write_bytes(b"\0")
        result = run(repo, "prune", "--domain", "host-rust", "--budget-gib", "0", "--apply")
        assert result.returncode == 0, result.stderr
        assert not cache.exists(), "explicit, unlocked apply did not remove the opted-in cache domain"

        # build-state/ already holds the platform locks used above.
        (target / "task-output").mkdir()
        result = run(repo, "report")
        assert result.returncode == 0 and "用途域以外：2 个目录" in result.stdout, "report must count directories outside the domains"

        print("PASS synthetic cache preview, active platform/Cargo/base/cross-target locks, explicit domain apply, and directories outside the domains")
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
