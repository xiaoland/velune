"""Make one recognized legacy empty import resumable, preserving its bytes."""
import json
import os
import stat
import sys
import tempfile


def upgrade(path, expected, record):
    backup = path + ".velune-empty-import.bak"
    flags = os.O_RDWR | os.O_APPEND | getattr(os, "O_NOFOLLOW", 0) | getattr(os, "O_BINARY", 0)
    original = os.lstat(path)
    if not stat.S_ISREG(original.st_mode) or original.st_nlink != 1:
        raise ValueError("empty import is not a single regular source")
    with os.fdopen(os.open(path, flags), "r+b", buffering=0) as source:
        # Serialize only upgrades of this source. The OS releases the lock if
        # the helper exits; native writers need not cooperate for append safety.
        if os.name == "nt":
            import msvcrt
            msvcrt.locking(source.fileno(), msvcrt.LK_LOCK, 1)
        else:
            import fcntl
            fcntl.flock(source.fileno(), fcntl.LOCK_EX)
        opened = os.fstat(source.fileno())
        identity = (opened.st_dev, opened.st_ino)

        def verify():
            current = os.lstat(path)
            source.seek(0)
            if (current.st_dev, current.st_ino) != identity or source.read() != expected:
                raise ValueError("empty import changed; source retained")

        if (original.st_dev, original.st_ino) != identity:
            raise ValueError("empty import changed before open; source retained")
        actual = source.read()
        if actual.startswith(expected) and actual != expected:
            # A concurrent upgrader may already have completed while this
            # process waited. Reuse its UUID, never append a second root.
            suffix = actual[len(expected):].splitlines()
            if len(suffix) == 1:
                try:
                    completed = json.loads(suffix[0])
                except (ValueError, UnicodeDecodeError):
                    completed = None
                if (isinstance(completed, dict)
                        and completed.keys() == record.keys()
                        and all(completed[key] == value for key, value in record.items()
                                if key not in ("uuid", "timestamp"))
                        and isinstance(completed.get("uuid"), str)
                        and isinstance(completed.get("timestamp"), str)):
                    return
        verify()
        fd, temporary = tempfile.mkstemp(prefix=".velune-empty-import-", dir=os.path.dirname(path))
        try:
            with os.fdopen(fd, "wb") as saved:
                saved.write(expected)
                saved.flush()
                os.fsync(saved.fileno())
            try:
                os.link(temporary, backup)
            except FileExistsError:
                pass
        finally:
            os.unlink(temporary)
        info = os.lstat(backup)
        if not stat.S_ISREG(info.st_mode) or (info.st_dev, info.st_ino) == identity:
            raise ValueError("empty import backup is not an independent regular file")
        with os.fdopen(os.open(backup, os.O_RDONLY | getattr(os, "O_NOFOLLOW", 0) | getattr(os, "O_BINARY", 0)), "rb") as saved:
            opened_backup = os.fstat(saved.fileno())
            if (info.st_dev, info.st_ino) != (opened_backup.st_dev, opened_backup.st_ino) or saved.read() != expected:
                raise ValueError("empty import backup verification failed; source retained")
        if os.name != "nt":
            directory = os.open(os.path.dirname(path), os.O_RDONLY | getattr(os, "O_DIRECTORY", 0))
            try:
                os.fsync(directory)
            finally:
                os.close(directory)
        verify()
        # One append preserves a native writer's new records even if it wins
        # the race after verification. Never truncate, replace or unlink it.
        addition = (json.dumps(record, separators=(",", ":")) + "\n").encode()
        if source.write(addition) != len(addition):
            raise OSError("incomplete empty import append; recover from " + backup)
        os.fsync(source.fileno())
        source.seek(0)
        current = os.lstat(path)
        after = source.read()
        if (current.st_dev, current.st_ino) != identity or not after.startswith(expected) or addition not in after[len(expected):]:
            raise ValueError("empty import changed during append; backup retained at " + backup)


if __name__ == "__main__":
    request = json.load(sys.stdin)
    upgrade(sys.argv[1], bytes(request["expected"]), request["record"])
