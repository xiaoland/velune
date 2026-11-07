#!/usr/bin/env python3
"""Small synthetic safety check for scripts/build-cache.py."""
from pathlib import Path
import shutil
import subprocess
import sys
import tempfile
import errno
import types
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
        (cache / "regenerable.o").write_bytes(b"synthetic build output" * 128)

        result = run(repo, "prune", "--domain", "host-rust", "--budget-gib", "0")
        assert result.returncode == 0, result.stderr
        assert cache.exists(), "preview changed the cache"

        with FileLock(target / "build-state/locks/macos.lock", blocking=False):
            result = run(repo, "prune", "--domain", "host-rust", "--budget-gib", "0", "--apply")
        assert result.returncode == 2, "an active platform lock must reject pruning"
        assert cache.exists(), "refused prune changed the cache"

        cargo_lock_path = cache / ".cargo-lock"
        with FileLock(cargo_lock_path, blocking=False):
            result = run(repo, "prune", "--domain", "host-rust", "--budget-gib", "0", "--apply")
        assert result.returncode == 2, "an active Cargo lock must reject pruning"
        assert cache.exists(), "refused Cargo prune changed the cache"

        cargo_lock_path.unlink()
        nested_cargo_lock = cache / "aarch64-apple-ios" / "release" / ".cargo-lock"
        nested_cargo_lock.parent.mkdir(parents=True)
        with FileLock(nested_cargo_lock, blocking=False):
            result = run(repo, "prune", "--domain", "host-rust", "--budget-gib", "0", "--apply")
        assert result.returncode == 2, "a nested cross-target Cargo lock must reject pruning"
        assert cache.exists(), "refused nested Cargo prune changed the cache"
        shutil.rmtree(cache / "aarch64-apple-ios")

        link = cache / "shared"
        try:
            link.symlink_to(repo, target_is_directory=True)
        except (OSError, NotImplementedError):
            link = None
        if link is not None:
            result = run(repo, "prune", "--domain", "host-rust", "--budget-gib", "0", "--apply")
            assert result.returncode == 2, "a cache containing a shared symlink must be refused"
            assert cache.exists() and link.is_symlink(), "symlink refusal changed the cache"
            link.unlink()

        # A real idle Cargo cache retains this file. Windows must delete it
        # without releasing the acquired lock before the removal completes.
        (cache / "debug").mkdir()
        (cache / "debug/.cargo-lock").write_bytes(b"\0")
        result = run(repo, "prune", "--domain", "host-rust", "--budget-gib", "0", "--apply")
        assert result.returncode == 0, result.stderr
        assert not cache.exists(), "explicit, unlocked apply did not remove the opted-in cache domain"

        legacy = target / "release"
        legacy.mkdir()
        result = run(repo, "prune", "--retired", "release", "--budget-gib", "0", "--apply")
        assert result.returncode == 2 and legacy.exists(), "unqualified legacy cache must remain protected"

        print("PASS synthetic cache preview, active platform/Cargo/base/cross-target locks, links, explicit domain apply, and protected legacy refusal")
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
