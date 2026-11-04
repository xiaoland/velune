#!/usr/bin/env python3
"""Read-only, bounded network-resource snapshots; no packet or address capture."""
import argparse
import collections
import datetime
import json
import os
from pathlib import Path
import platform
import subprocess
import time


def command(*args):
    try:
        result = subprocess.run(args, capture_output=True, text=True, timeout=8)
    except subprocess.TimeoutExpired:
        return {"error": "timeout", "command": args[0]}
    if result.returncode:
        return {"error": result.stderr.strip(), "command": args[0]}
    return result.stdout


def snapshot():
    processes = command("/bin/ps", "-axo", "pid=,ppid=,comm=")
    names = {}
    if isinstance(processes, str):
        for line in processes.splitlines():
            fields = line.split(None, 2)
            if len(fields) == 3:
                names[int(fields[0])] = Path(fields[2]).name
    sockets = collections.defaultdict(lambda: collections.Counter())
    errors = []
    for protocol in ("tcp", "udp"):
        result = command("/usr/sbin/netstat", "-anv", "-p", protocol)
        if not isinstance(result, str):
            errors.append(result)
            continue
        for line in result.splitlines():
            fields = line.split()
            if not fields or fields[0] not in (protocol + "4", protocol + "6"):
                continue
            # Darwin omits the state field in UDP rows. Addresses are discarded.
            pid_column = 10 if protocol == "tcp" else 9
            if len(fields) <= pid_column or not fields[pid_column].isdigit():
                errors.append({"error": "unrecognized netstat row", "protocol": protocol})
                continue
            counts = sockets[int(fields[pid_column])]
            counts[protocol + "_sockets"] += 1
            counts["recv_bytes"] += int(fields[1])
            counts["send_bytes"] += int(fields[2])
            if protocol == "tcp":
                counts["tcp_" + fields[5].lower()] += 1
    return {
        "time": datetime.datetime.now().astimezone().isoformat(),
        "os": platform.mac_ver()[0],
        "mbufs": command("/usr/sbin/netstat", "-m"),
        "file_limits": command("/usr/sbin/sysctl", "kern.num_files", "kern.maxfiles", "kern.maxfilesperproc"),
        "interfaces": command("/sbin/ifconfig", "-l"),
        "processes": [{"pid": pid, "name": names.get(pid, "exited_or_system"), **counts}
                      for pid, counts in sorted(sockets.items())],
        "errors": errors,
    }


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--output", type=Path)
    parser.add_argument("--count", type=int, default=1)
    parser.add_argument("--interval", type=float, default=10)
    args = parser.parse_args()
    if platform.system() != "Darwin":
        parser.error("macOS only")
    if not 1 <= args.count <= 360 or not 2 <= args.interval <= 60:
        parser.error("count must be 1–360; interval must be 2–60 seconds")
    path = args.output or (Path.home() / "Library/Logs/Velune" /
                           ("network-" + datetime.datetime.now().strftime("%Y%m%d-%H%M%S") + ".jsonl"))
    path.parent.mkdir(parents=True, exist_ok=True)
    # Never overwrite an earlier incident. Flush each record before the next wait.
    with open(os.open(path, os.O_WRONLY | os.O_CREAT | os.O_EXCL, 0o600), "w") as output:
        print(path, flush=True)
        for index in range(args.count):
            output.write(json.dumps(snapshot(), ensure_ascii=False) + "\n")
            output.flush()
            if index + 1 < args.count:
                time.sleep(args.interval)


if __name__ == "__main__":
    main()
