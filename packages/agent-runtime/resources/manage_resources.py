"""One bounded native-resource operation; paths and contents arrive only on stdin."""
import hashlib
import json
import os
import secrets
import stat
import sys

MAX_BYTES = 16 * 1024 * 1024
MAX_NODES = 4096
MAX_DEPTH = 64
DIRECTORY = os.O_RDONLY | os.O_DIRECTORY | os.O_NOFOLLOW | os.O_CLOEXEC


class Failure(Exception):
    def __init__(self, code, message):
        self.code = code
        self.message = message


def fail(code, message):
    raise Failure(code, message)


def kind(mode):
    if stat.S_ISREG(mode):
        return "file"
    if stat.S_ISDIR(mode):
        return "directory"
    if stat.S_ISLNK(mode):
        return "symlink"
    return "other"


def components(path):
    if not isinstance(path, str) or "\0" in path or os.path.isabs(path):
        fail("invalid", "Resource path must be relative")
    parts = path.split("/") if path else []
    if any(part in ("", ".", "..") for part in parts):
        fail("invalid", "Resource path contains an invalid component")
    if len(parts) > MAX_DEPTH:
        fail("invalid", "Resource path exceeds 64 levels")
    return parts


def metadata(parent, name):
    try:
        return os.stat(name, dir_fd=parent, follow_symlinks=False)
    except FileNotFoundError:
        return None


def open_directory(parent, name, create):
    info = metadata(parent, name)
    if info is None and create:
        try:
            os.mkdir(name, mode=0o700, dir_fd=parent)
        except FileExistsError:
            pass
        info = metadata(parent, name)
    if info is None:
        return None
    if not stat.S_ISDIR(info.st_mode):
        fail("invalid", "Resource ancestor is not a directory or is a symbolic link")
    return os.open(name, DIRECTORY, dir_fd=parent)


def external_parent(root, create):
    if not isinstance(root, str) or "\0" in root or not os.path.isabs(root):
        fail("invalid", "Resource root must be absolute")
    root = os.path.normpath(root)
    parent, name = os.path.split(root)
    if not name:
        return os.open(root, DIRECTORY), "."
    # Only ancestors outside the caller's root may resolve symbolic links.
    parent = os.path.realpath(parent)
    pending = []
    while not os.path.exists(parent):
        parent, child = os.path.split(parent)
        pending.append(child)
    descriptor = os.open(parent, DIRECTORY)
    try:
        for child in reversed(pending):
            next_descriptor = open_directory(descriptor, child, create)
            os.close(descriptor)
            descriptor = next_descriptor
            if descriptor is None:
                return None, name
        return descriptor, name
    except BaseException:
        if descriptor is not None:
            os.close(descriptor)
        raise


def locate(root, path, create=False):
    parts = components(path)
    descriptor, root_name = external_parent(root, create)
    if descriptor is None:
        return None, parts[-1] if parts else root_name
    if not parts:
        return descriptor, root_name
    try:
        for name in [root_name] + parts[:-1]:
            next_descriptor = open_directory(descriptor, name, create)
            os.close(descriptor)
            descriptor = next_descriptor
            if descriptor is None:
                return None, parts[-1]
        return descriptor, parts[-1]
    except BaseException:
        if descriptor is not None:
            os.close(descriptor)
        raise


def children(descriptor):
    entries = []
    with os.scandir(descriptor) as iterator:
        for entry in iterator:
            if len(entries) >= MAX_NODES:
                fail("invalid", "Resource catalog exceeds 4096 nodes")
            name = entry.name
            try:
                name.encode("utf-8")
            except UnicodeError:
                fail("invalid", "Resource name is not UTF-8")
            info = metadata(descriptor, name)
            if info is None:
                fail("conflict", "Resource directory changed during inspection")
            entries.append((name, kind(info.st_mode)))
    return sorted(entries)


def digest_start(node, mode):
    digest = hashlib.sha256()
    digest.update((node + "\0" + format(stat.S_IMODE(mode), "o") + "\0").encode("ascii"))
    return digest


def file_bytes(parent, name):
    descriptor = os.open(name, os.O_RDONLY | os.O_NOFOLLOW | os.O_NONBLOCK | os.O_CLOEXEC, dir_fd=parent)
    with os.fdopen(descriptor, "rb") as file:
        info = os.fstat(file.fileno())
        if not stat.S_ISREG(info.st_mode):
            fail("invalid", "Resource is not a regular file")
        content = file.read(MAX_BYTES + 1)
        if len(content) > MAX_BYTES:
            fail("invalid", "Resource file exceeds 16 MiB")
        return info, content


def can_write(parent, name=None):
    try:
        return os.access(".", os.W_OK | os.X_OK, dir_fd=parent, effective_ids=True) and (
            name is None or os.access(name, os.W_OK, dir_fd=parent, effective_ids=True, follow_symlinks=False))
    except (NotImplementedError, TypeError):
        fail("unsupported", "Target cannot inspect effective resource permissions")


def inspect_at(parent, name, path, include_bytes=False):
    info = metadata(parent, name) if parent is not None else None
    node = {"path": path, "node": "other", "revision": None,
            "state": {"kind": "missing"}, "access": {"kind": "writable"},
            "executable": False, "size": 0}
    if info is None:
        if parent is not None and not can_write(parent):
            node["access"] = {"kind": "readOnly", "reason": "Resource parent is not writable"}
        return node, None
    node.update(node=kind(info.st_mode), state={"kind": "present"},
                executable=bool(info.st_mode & 0o111), size=info.st_size)
    reason = None
    if node["node"] == "symlink":
        reason = "Symbolic links are visible but cannot be managed"
    elif node["node"] == "other":
        reason = "Resource is not a regular file or directory"
        node["state"] = {"kind": "invalid", "reason": reason}
    elif node["node"] == "file" and info.st_size > MAX_BYTES:
        reason = "Resource file exceeds 16 MiB"
        node["state"] = {"kind": "invalid", "reason": reason}
    if reason:
        node["access"] = {"kind": "readOnly", "reason": reason}
        return node, None
    content = None
    if node["node"] == "file":
        info, content = file_bytes(parent, name)
        digest = digest_start("file", info.st_mode)
        digest.update(content)
        node.update(size=len(content), executable=bool(info.st_mode & 0o111))
    else:
        descriptor = os.open(name, DIRECTORY, dir_fd=parent)
        try:
            digest = digest_start("directory", os.fstat(descriptor).st_mode)
            for child, child_kind in children(descriptor):
                digest.update(child.encode("utf-8") + b"\0" + child_kind.encode("ascii") + b"\0")
        finally:
            os.close(descriptor)
    node["revision"] = digest.hexdigest()
    if info.st_mode & 0o222 == 0 or not can_write(parent, name):
        node["access"] = {"kind": "readOnly", "reason": "Resource or parent is not writable"}
    return node, content if include_bytes else None


def expected_revision(node, expected):
    if expected["kind"] == "missing":
        if node["state"]["kind"] != "missing":
            fail("conflict", "Resource already exists")
    elif node["revision"] is None or node["revision"] != expected.get("revision"):
        fail("conflict", "Resource revision changed")


def editable(node):
    if node["access"]["kind"] != "writable":
        fail("invalid", node["access"]["reason"])


def collect(parent, name, path, recursive, output):
    info = metadata(parent, name)
    if info is None:
        return
    if not stat.S_ISDIR(info.st_mode):
        fail("invalid", "Resource catalog path is not a directory")
    descriptor = os.open(name, DIRECTORY, dir_fd=parent)
    try:
        for child, child_kind in children(descriptor):
            if len(output) >= MAX_NODES:
                fail("invalid", "Resource catalog exceeds 4096 nodes")
            child_path = path + "/" + child if path else child
            if len(child_path.split("/")) > MAX_DEPTH:
                fail("invalid", "Resource catalog exceeds 64 levels")
            node, _ = inspect_at(descriptor, child, child_path)
            output.append(node)
            if recursive and child_kind == "directory":
                collect(descriptor, child, child_path, True, output)
    finally:
        os.close(descriptor)


def operation(request):
    action, path = request["operation"], request["path"]
    parent, name = locate(request["root"], path, action == "write")
    try:
        if action == "list":
            output = []
            if parent is not None:
                collect(parent, name, path, request["recursive"], output)
            return output
        node, content = inspect_at(parent, name, path, action == "read")
        if action == "inspect":
            return node
        if action == "read":
            if node["state"]["kind"] == "missing":
                fail("missing", "Resource does not exist")
            if node["node"] != "file" or content is None:
                fail("invalid", "Resource is not a readable regular file")
            return {"node": node, "hex": content.hex()}
        if not path:
            fail("invalid", "Resource root cannot be replaced or deleted")
        expected_revision(node, request["expected"])
        editable(node)
        if action == "delete":
            if node["state"]["kind"] == "missing":
                fail("missing", "Resource does not exist")
            if node["node"] == "directory":
                os.rmdir(name, dir_fd=parent)
            elif node["node"] == "file":
                os.unlink(name, dir_fd=parent)
            else:
                fail("invalid", "Resource cannot be deleted")
            os.fsync(parent)
            return None
        if action != "write":
            fail("invalid", "Unknown resource operation")
        if node["state"]["kind"] != "missing" and node["node"] != "file":
            fail("invalid", "Only regular files can be written")
        content = bytes.fromhex(request["hex"])
        if len(content) > MAX_BYTES:
            fail("invalid", "Resource file exceeds 16 MiB")
        old = metadata(parent, name)
        mode = stat.S_IMODE(old.st_mode) if old is not None else 0o600
        executable = request.get("executable")
        if executable is not None:
            mode = mode | 0o100 if executable else mode & ~0o111
        temporary = ".velune-resource-" + secrets.token_hex(12)
        descriptor = os.open(temporary, os.O_WRONLY | os.O_CREAT | os.O_EXCL | os.O_NOFOLLOW | os.O_CLOEXEC, 0o600, dir_fd=parent)
        try:
            with os.fdopen(descriptor, "wb") as file:
                file.write(content)
                file.flush()
                os.fchmod(file.fileno(), mode)
                os.fsync(file.fileno())
                mode = stat.S_IMODE(os.fstat(file.fileno()).st_mode)
            # Optimistic revision check, not a lock shared with external editors.
            current, _ = inspect_at(parent, name, path)
            expected_revision(current, request["expected"])
            if request["expected"]["kind"] == "missing":
                try:
                    os.link(temporary, name, src_dir_fd=parent, dst_dir_fd=parent, follow_symlinks=False)
                except FileExistsError:
                    fail("conflict", "Resource already exists")
            else:
                os.replace(temporary, name, src_dir_fd=parent, dst_dir_fd=parent)
            os.fsync(parent)
        finally:
            try:
                os.unlink(temporary, dir_fd=parent)
            except FileNotFoundError:
                pass
        digest = digest_start("file", mode)
        digest.update(content)
        return {"path": path, "node": "file", "revision": digest.hexdigest(),
                "state": {"kind": "present"}, "access": {"kind": "writable"},
                "executable": bool(mode & 0o111), "size": len(content)}
    finally:
        if parent is not None:
            os.close(parent)


try:
    request = json.load(sys.stdin)
    print(json.dumps({"ok": operation(request)}))
except Failure as error:
    print(json.dumps({"error": {"code": error.code, "message": error.message}}))
except OSError as error:
    import errno
    code = "missing" if error.errno == errno.ENOENT else "invalid" if error.errno in (errno.ELOOP, errno.ENOTDIR, errno.ENOTEMPTY, errno.EISDIR) else "unavailable"
    print(json.dumps({"error": {"code": code, "message": "Resource filesystem operation failed: " + (error.strerror or "I/O error")}}))
except (ValueError, KeyError, TypeError, UnicodeError):
    print(json.dumps({"error": {"code": "invalid", "message": "Invalid resource request"}}))
