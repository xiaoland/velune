"""Velune's stdin-only launch and application-owned file protocol."""
import json
import os
import sys
import tempfile
import signal
import shutil
import socket
import subprocess
import select
import time


def receive():
    # Exact-length reads preserve the first runtime stdin record across exec.
    header = bytearray()
    while True:
        byte = os.read(0, 1)
        if not byte or len(header) > 12:
            raise ValueError("invalid request length")
        if byte == b"\n":
            break
        header.extend(byte)
    remaining = int(header)
    if remaining < 0:
        raise ValueError("invalid request length")
    chunks = []
    while remaining:
        chunk = os.read(0, min(remaining, 65536))
        if not chunk:
            raise ValueError("incomplete request")
        chunks.append(chunk)
        remaining -= len(chunk)
    return json.loads(b"".join(chunks))


def absolute(value):
    if not isinstance(value, str) or not os.path.isabs(value):
        raise ValueError("absolute path required")
    return value


def write_file(path, content):
    path = absolute(path)
    parent = os.path.dirname(path)
    os.makedirs(parent, mode=0o700, exist_ok=True)
    fd, temporary = tempfile.mkstemp(prefix=".velune-", dir=parent)
    try:
        with os.fdopen(fd, "wb") as file:
            file.write(bytes.fromhex(content))
            file.flush()
            os.fsync(file.fileno())
        os.replace(temporary, path)
    finally:
        if os.path.exists(temporary):
            os.unlink(temporary)


def path_metadata(path, directory):
    import stat
    try:
        metadata = os.stat(absolute(path))
    except FileNotFoundError:
        return False
    return not directory or stat.S_ISDIR(metadata.st_mode)


def terminate_group(child):
    try:
        os.killpg(child.pid, signal.SIGTERM)
    except ProcessLookupError:
        return
    try:
        child.wait(timeout=1)
    except subprocess.TimeoutExpired:
        pass
    try:
        os.killpg(child.pid, signal.SIGKILL)
    except ProcessLookupError:
        pass
    child.wait()


def launch(request):
    environment = os.environ.copy()
    for key, value in request["environment"].items():
        if value is None:
            environment.pop(key, None)
        else:
            environment[key] = value
    cwd = None if request["cwd"] is None else absolute(request["cwd"])
    child = subprocess.Popen(request["argv"], env=environment, cwd=cwd,
                             stdin=subprocess.PIPE, start_new_session=True)
    interactive = request["interactive"]
    deadline = time.monotonic() + 15
    def disconnected(_signal, _frame):
        raise InterruptedError("SSH transport closed")
    signal.signal(signal.SIGHUP, disconnected)
    signal.signal(signal.SIGTERM, disconnected)
    input_open = True
    pending = bytearray()
    os.set_blocking(child.stdin.fileno(), False)
    try:
        while child.poll() is None:
            if not interactive and time.monotonic() >= deadline:
                raise TimeoutError("runtime helper exceeded 15 seconds")
            if not input_open and not pending:
                if not child.stdin.closed:
                    child.stdin.close()
                time.sleep(0.02)
                continue
            ready, writable, _ = select.select([0] if input_open else [], [child.stdin] if pending else [], [], 0.05)
            if ready:
                data = os.read(0, 65536)
                if not data:
                    input_open = False
                    if interactive:
                        child.stdin.close()
                        try:
                            child.wait(timeout=1)
                        except subprocess.TimeoutExpired:
                            pass
                        break
                else:
                    pending.extend(data)
            if writable:
                try:
                    count = os.write(child.stdin.fileno(), pending)
                    del pending[:count]
                except BrokenPipeError:
                    pending.clear()
                    input_open = False
            if not input_open and not pending:
                child.stdin.close()
    finally:
        signal.signal(signal.SIGHUP, signal.SIG_IGN)
        signal.signal(signal.SIGTERM, signal.SIG_IGN)
        terminate_group(child)
    return child.returncode


def verify_loopback(port):
    found = []
    if os.path.exists("/proc/net/tcp"):
        for filename in ("/proc/net/tcp", "/proc/net/tcp6"):
            try:
                with open(filename, encoding="ascii") as table:
                    rows = table.readlines()[1:]
            except FileNotFoundError:
                continue
            for row in rows:
                columns = row.split()
                address, candidate = columns[1].split(":")
                if columns[3] != "0A" or int(candidate, 16) != port:
                    continue
                raw = b"".join(int(address[i:i+8], 16).to_bytes(4, sys.byteorder)
                               for i in range(0, len(address), 8))
                found.append(socket.inet_ntop(socket.AF_INET if len(raw) == 4 else socket.AF_INET6, raw))
    else:
        result = subprocess.run(["lsof", "-nP", "-iTCP:" + str(port), "-sTCP:LISTEN", "-Fn"],
                                capture_output=True, check=False, timeout=5, text=True)
        if result.returncode not in (0, 1):
            raise RuntimeError("cannot inspect gateway listener")
        for line in result.stdout.splitlines():
            if line.startswith("n"):
                found.append(line[1:].rsplit(":", 1)[0].strip("[]"))
    if not found or any(address not in ("127.0.0.1", "::1") for address in found):
        raise RuntimeError("gateway listener is not verifiably loopback; use sshd GatewayPorts=no or clientspecified")


def main():
    request = receive()
    operation = request["operation"]
    if operation == "launch":
        sys.exit(launch(request))
    elif operation == "verify_loopback":
        verify_loopback(request["port"])
    elif operation == "mkdir":
        os.makedirs(absolute(request["path"]), mode=0o700, exist_ok=True)
    elif operation == "remove_directory":
        try:
            shutil.rmtree(absolute(request["path"]))
        except FileNotFoundError:
            pass
    elif operation == "read":
        with open(absolute(request["path"]), "rb") as file:
            sys.stdout.buffer.write(file.read())
    elif operation == "write":
        for file in request["files"]:
            write_file(file["path"], file["content"])
    elif operation == "is_directory":
        print(json.dumps(path_metadata(request["path"], True)))
    elif operation == "exists":
        print(json.dumps(path_metadata(request["path"], False)))
    else:
        raise ValueError("unknown operation")


try:
    main()
except OSError as error:
    print("Velune remote bootstrap failed: errno=" + str(error.errno) +
          "; " + (error.strerror or str(error)) + "; path=" + str(error.filename), file=sys.stderr)
    sys.exit(1)
except (RuntimeError, TimeoutError) as error:
    print("Velune remote bootstrap failed: " + str(error), file=sys.stderr)
    sys.exit(1)
except Exception as error:
    # The request can contain a short-lived gateway token; never echo its data.
    print("Velune remote bootstrap failed: " + type(error).__name__, file=sys.stderr)
    sys.exit(1)
