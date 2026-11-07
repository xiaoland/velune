"""Small cross-platform advisory file lock shared by repository build tools."""
from __future__ import annotations

import os
from pathlib import Path
import errno
import time


def _open_lock_file(path: Path):
    if os.name != "nt":
        return path.open("a+b")
    # Python's ordinary CRT open does not share DELETE access. Maintenance
    # must be able to remove an idle Cargo lock *while still holding* its
    # byte-range lock, rather than opening a race by releasing it first.
    import ctypes
    from ctypes import wintypes
    import msvcrt

    kernel = ctypes.WinDLL("kernel32", use_last_error=True)
    create = kernel.CreateFileW
    create.argtypes = [wintypes.LPCWSTR, wintypes.DWORD, wintypes.DWORD,
                       wintypes.LPVOID, wintypes.DWORD, wintypes.DWORD, wintypes.HANDLE]
    create.restype = wintypes.HANDLE
    handle = create(str(path), 0x80000000 | 0x40000000, 1 | 2 | 4,
                    None, 4, 0x80, None)  # READ|WRITE, SHARE READ|WRITE|DELETE, OPEN_ALWAYS
    if handle == wintypes.HANDLE(-1).value:
        raise ctypes.WinError(ctypes.get_last_error())
    try:
        descriptor = msvcrt.open_osfhandle(handle, os.O_RDWR | os.O_BINARY)
    except BaseException:
        kernel.CloseHandle.argtypes = [wintypes.HANDLE]
        kernel.CloseHandle(handle)
        raise
    return os.fdopen(descriptor, "r+b")


def _windows_lock(file, blocking: bool) -> None:
    import msvcrt

    file.seek(0, os.SEEK_END)
    if file.tell() == 0:
        file.write(b"\0")
        file.flush()
    busy = {errno.EACCES, errno.EAGAIN, errno.EDEADLK}
    while True:
        file.seek(0)
        try:
            msvcrt.locking(file.fileno(), msvcrt.LK_NBLCK, 1)
            return
        except OSError as error:
            if not blocking or error.errno not in busy:
                raise
            time.sleep(0.1)


class FileLock:
    """A byte-range lock on Windows and flock on Unix.

    The lock file is persistent metadata; callers must not unlink it while a
    build may be opening it, since doing so can create two independent locks.
    """

    def __init__(self, path: Path, blocking: bool = True):
        self.path = Path(path)
        self.blocking = blocking
        self.file = None

    def __enter__(self):
        self.path.parent.mkdir(parents=True, exist_ok=True)
        self.file = _open_lock_file(self.path)
        try:
            if os.name == "nt":
                _windows_lock(self.file, self.blocking)
            else:
                import fcntl

                mode = fcntl.LOCK_EX
                if not self.blocking:
                    mode |= fcntl.LOCK_NB
                fcntl.flock(self.file.fileno(), mode)
        except BaseException:
            self.file.close()
            self.file = None
            raise
        return self

    def __exit__(self, exc_type, exc_value, traceback):
        if self.file is None:
            return False
        try:
            if os.name == "nt":
                import msvcrt

                self.file.seek(0)
                msvcrt.locking(self.file.fileno(), msvcrt.LK_UNLCK, 1)
            else:
                import fcntl

                fcntl.flock(self.file.fileno(), fcntl.LOCK_UN)
        finally:
            self.file.close()
            self.file = None
        return False
