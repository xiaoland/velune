#!/usr/bin/env python3
"""Read-only macOS resource snapshots, without addresses, arguments or packets.

Counters are observations, not causal attribution. Commands are not atomic;
nettop totals reset with process identity. An extension's native flows need not
belong to the client that triggered them. Absence cannot exclude a cause.
"""
import argparse
import collections
import csv
import ctypes
import datetime
import json
import os
from pathlib import Path
import platform
import re
import struct
import subprocess
import time


def command(*args, binary=False):
    """Never retain command output on failure: it could contain private data."""
    try:
        result = subprocess.run(args, capture_output=True, text=not binary, timeout=8,
                                env={**os.environ, "LC_ALL": "C"})
    except subprocess.TimeoutExpired:
        return {"error": "timeout", "source": Path(args[0]).name}
    except OSError as error:
        return {"error": "command_unavailable", "source": Path(args[0]).name, "errno": error.errno}
    if result.returncode:
        return {"error": "command_failed", "source": Path(args[0]).name, "exit_code": result.returncode}
    return result.stdout


def utc_now():
    return datetime.datetime.now(datetime.timezone.utc).isoformat()


class ProcBSDInfo(ctypes.Structure):
    # Darwin proc_bsdinfo / PROC_PIDTBSDINFO=3. No arguments or environment read.
    _fields_ = [(name, ctypes.c_uint32) for name in (
        "flags", "status", "xstatus", "pid", "ppid", "uid", "gid", "ruid",
        "rgid", "svuid", "svgid", "reserved")] + [
        ("comm", ctypes.c_char * 16), ("name", ctypes.c_char * 32)] + [
        (name, ctypes.c_uint32) for name in (
            "nfiles", "pgid", "pjobc", "tdev", "tpgid")] + [
        ("nice", ctypes.c_int32), ("start_sec", ctypes.c_uint64),
        ("start_usec", ctypes.c_uint64)]


class ProcessReader:
    def __init__(self):
        self.libproc = ctypes.CDLL("/usr/lib/libproc.dylib", use_errno=True)
        self.libproc.proc_pidinfo.argtypes = [ctypes.c_int, ctypes.c_int,
            ctypes.c_uint64, ctypes.c_void_p, ctypes.c_int]
        self.libproc.proc_pidinfo.restype = ctypes.c_int

    def identity(self, pid):
        info = ProcBSDInfo()
        size = self.libproc.proc_pidinfo(pid, 3, 0, ctypes.byref(info), ctypes.sizeof(info))
        if size != ctypes.sizeof(info) or info.pid != pid:
            return None
        return {"pid": pid, "ppid": info.ppid, "start_sec": info.start_sec,
                "start_usec": info.start_usec, "start_precision": "microseconds"}

    def processes(self, boot_id):
        self.last_seen_pids = None
        result = command("/bin/ps", "-axo", "pid=,ppid=,lstart=,comm=")
        if not isinstance(result, str):
            return {}, [result]
        processes, missing, fallback = {}, 0, 0
        self.last_seen_pids = set()
        for line in result.splitlines():
            fields = line.strip().split(None, 7)
            if len(fields) != 8 or not fields[0].isdigit() or not fields[1].isdigit():
                missing += 1
                continue
            pid = int(fields[0])
            self.last_seen_pids.add(pid)
            identity = self.identity(pid)
            if identity is None:
                # libproc denies many other-user/system processes. ps exposes
                # their start time, with explicitly weaker seconds precision.
                try:
                    start = datetime.datetime.strptime(" ".join(fields[2:7]), "%a %b %d %H:%M:%S %Y")
                    identity = {"pid": pid, "ppid": int(fields[1]),
                        "start_sec": int(start.timestamp()), "start_usec": None,
                        "start_precision": "seconds"}
                    fallback += 1
                except ValueError:
                    missing += 1
                    continue
            identity["identity"] = identity_key(boot_id, identity)
            identity["name"] = Path(fields[7]).name
            processes[pid] = identity
        errors = ([{"error": "process_identity_unavailable", "count": missing}]
                  if missing else [])
        if fallback:
            errors.append({"error": "process_start_seconds_fallback", "count": fallback,
                           "limitation": "cannot_distinguish_same_pid_reused_within_same_second"})
        return processes, errors


def identity_key(boot_id, process):
    return f"{boot_id}:{process['pid']}:{process['start_sec']}:{process['start_usec']}"


def boot_identity():
    result = command("/usr/sbin/sysctl", "-n", "kern.boottime")
    match = re.search(r"sec = (\d+), usec = (\d+)", result) if isinstance(result, str) else None
    if match is None:
        raise RuntimeError("boot_identity_unavailable")
    return f"{match[1]}.{match[2]}"


def is_velune(process):
    return process["name"].lower() in ("velune", "velunemac", "velune-macos")


def is_extension(process):
    name = process["name"].lower()
    return "sing-box" in name or "sing_box" in name or name == "io.nekohasekai.sfamt.system"


class LineageTracker:
    """Keep a child's identity after reparenting; never transfer across PID reuse.

    Only currently observed identities persist. A child born and reparented
    between samples cannot be recovered retrospectively from a parent PID.
    """
    def __init__(self, retained=None):
        self.retained = retained or {}

    def update(self, processes, seen_pids=None):
        # A failed inventory is not evidence of exit. Retain uncertain identities
        # until a successful ps shows absence or a different identity at that PID.
        if seen_pids is None:
            retained = dict(self.retained)
        else:
            retained = {key: value for key, value in self.retained.items()
                        if int(key.split(":")[-3]) in seen_pids}
        for process in processes.values():
            retained = {key: value for key, value in retained.items()
                        if int(key.split(":")[-3]) != process["pid"] or key == process["identity"]}
        for process in processes.values():
            if is_velune(process):
                retained[process["identity"]] = {"role": "velune", "root_identity": process["identity"]}
        changed = True
        while changed:
            changed = False
            for process in processes.values():
                key = process["identity"]
                parent = processes.get(process["ppid"])
                relation = retained.get(parent["identity"]) if parent else None
                if key not in retained and relation:
                    retained[key] = {"role": "velune_descendant",
                                     "root_identity": relation["root_identity"],
                                     "observed_parent_identity": parent["identity"]}
                    changed = True
        self.retained = retained
        for process in processes.values():
            process["relation"] = retained.get(process["identity"], {"role": "other"})
            if is_extension(process):
                process["network_extension"] = "sing_box_or_sfamt"


def parse_sockets(text, protocol):
    sockets = collections.defaultdict(collections.Counter)
    rejected = 0
    for line in text.splitlines():
        fields = line.split()
        if not fields or fields[0] not in (protocol + "4", protocol + "6", protocol + "46"):
            continue
        # Address fields are transient and never returned or logged.
        pid_column = 10 if protocol == "tcp" else 9
        try:
            pid = int(fields[pid_column])
            recv, send = int(fields[1]), int(fields[2])
            rx_column = 6 if protocol == "tcp" else 5
            rx, tx = int(fields[rx_column]), int(fields[rx_column + 1])
            if min(pid, recv, send, rx, tx) < 0:
                raise ValueError()
            state = fields[5].lower() if protocol == "tcp" else None
            if state and not re.fullmatch(r"[a-z][a-z_0-9]*", state):
                raise ValueError()
        except (ValueError, IndexError):
            rejected += 1
            continue
        counts = sockets[pid]
        counts[protocol + "_sockets"] += 1
        counts["recv_queue_bytes"] += recv
        counts["send_queue_bytes"] += send
        counts["active_socket_rx_bytes"] += rx
        counts["active_socket_tx_bytes"] += tx
        if state:
            counts["tcp_" + state] += 1
    return sockets, rejected


def parse_traffic(text):
    traffic, rejected = {}, 0
    for row in csv.reader(text.splitlines()):
        if not row or not row[0]:
            continue
        _, separator, pid = row[0].rpartition(".")
        if len(row) < 6 or not separator or not pid.isdigit() or not all(v.isdigit() for v in row[1:6]):
            rejected += 1
            continue
        traffic[int(pid)] = dict(zip(("bytes_in_total", "bytes_out_total", "duplicate_bytes_total",
                                     "out_of_order_bytes_total", "retransmitted_bytes_total"),
                                    map(int, row[1:6])))
    return traffic, rejected


def parse_mbufs(text):
    """Keep numeric resource lines, never arbitrary raw command output."""
    counters = {}
    for line in text.splitlines():
        line = line.strip()
        match = re.fullmatch(r"(\d+)/(\d+) mbufs in use:", line)
        if match:
            counters.update(mbufs_in_use=int(match[1]), mbufs_total=int(match[2]))
            continue
        match = re.fullmatch(r"(\d+)/(\d+) mbuf (\d+)KB clusters in use", line)
        if match:
            counters[f"clusters_{match[3]}kb_in_use"] = int(match[1])
            counters[f"clusters_{match[3]}kb_total"] = int(match[2])
            continue
        match = re.fullmatch(r"(\d+) (?:KB|Kbytes) allocated to network \(([\d.]+)% in use\)", line)
        if match:
            counters.update(network_allocated_kb=int(match[1]), network_in_use_percent=float(match[2]))
            continue
        match = re.fullmatch(r"(\d+) mbufs allocated to ([a-z ]+)", line)
        if match:
            counters["mbufs_" + match[2].replace(" ", "_")] = int(match[1])
            continue
        match = re.fullmatch(r"(\d+) (requests for memory denied|requests for memory delayed|calls to protocol drain routines)", line)
        if match:
            counters[match[2].replace(" ", "_")] = int(match[1])
    return counters


def parse_interfaces(text):
    interfaces, rejected = [], 0
    for line in text.splitlines():
        fields = line.split()
        if len(fields) < 3 or not re.fullmatch(r"(?:en|utun)\d+\*?", fields[0]):
            continue
        link = re.fullmatch(r"<Link#(\d+)>", fields[2])
        if not link:
            continue  # Only Link rows; never duplicate IPv4/IPv6 address rows.
        try:
            # Tunnel rows have no MAC address column. Numeric tail is stable.
            values = [int(value) for value in fields[-8:]]
            if len(fields) not in (11, 12) or min(values) < 0:
                raise ValueError()
            interfaces.append({"name": fields[0].rstrip("*"), "index": int(link[1]),
                **dict(zip(("packets_in_total", "errors_in_total", "bytes_in_total",
                            "packets_out_total", "errors_out_total", "bytes_out_total",
                            "collisions_total", "drops_total"), values))})
        except (ValueError, IndexError):
            rejected += 1
    return interfaces, rejected


KCTL_XNU = "11417.101.15"
KCTL_STATS = ("reg_total", "reg_count", "pcbcount", "gencnt", "connections",
              "conn_fail", "send_fail", "send_list_fail", "enqueue_fail",
              "enqueue_fullsock", "bad_kctlref", "tbl_size_too_big",
              "enqdata_mb_alloc_fail", "enqdata_sbappend_fail")


def parse_socket_metadata(data, position, families):
    family = struct.unpack_from("<i", data, position + 40)[0]
    if family not in families:
        raise ValueError("unsupported_socket_family")
    return {"last_pid": struct.unpack_from("<i", data, position + 68)[0],
            "effective_pid": struct.unpack_from("<i", data, position + 72)[0]}


def parse_socket_buffer(data, position):
    return dict(zip(("queue_bytes", "high_water_bytes", "mbuf_bytes", "mbuf_max_bytes"),
                    struct.unpack_from("<4I", data, position + 8)))


def parse_kernel_controls(data, statistics, xnu_version):
    """Private ABI: accept only the exact checked XNU revision and TLV layout.

    Source: apple-oss-distributions/xnu tag xnu-11417.101.15, bsd/kern/
    kern_control.c:kctl_pcblist, bsd/sys/{kern_control.h,socketvar.h}.
    Pointer/cookie fields are skipped and never retained. last_pid is the last
    socket user, not proof that the process caused all queued allocations.
    """
    if xnu_version != KCTL_XNU:
        raise ValueError("unsupported_xnu_kernel_control_abi")
    if len(statistics) != 112:
        raise ValueError("unsupported_kernel_control_stats_length")
    if len(data) < 32 or struct.unpack_from("<I", data)[0] != 32:
        raise ValueError("unsupported_kernel_control_header")
    position, rows = 32, []
    footer = None
    while position < len(data):
        if len(data) - position == 32 and struct.unpack_from("<I", data, position)[0] == 32:
            footer = data[position:]
            position += 32
            break
        row = {}
        for expected_length, expected_kind in ((128, 128), (104, 1), (32, 2), (32, 4), (136, 8)):
            if position + 8 > len(data):
                raise ValueError("truncated_kernel_control_tlv")
            length, kind = struct.unpack_from("<II", data, position)
            if length != expected_length or kind != expected_kind or position + length > len(data):
                raise ValueError("unsupported_kernel_control_tlv")
            if kind == 128:
                unit, control_id = struct.unpack_from("<II", data, position + 16)
                name = data[position + 32:position + 128].split(b"\0", 1)[0]
                row.update(unit=unit, control_id=control_id,
                           control="utun" if name == b"com.apple.net.utun_control" else "other")
                if row["control"] == "utun" and unit > 0:
                    row["interface"] = f"utun{unit - 1}"
            elif kind == 1:
                row.update(parse_socket_metadata(data, position, {32}))
            elif kind in (2, 4):
                row["recv" if kind == 2 else "send"] = parse_socket_buffer(data, position)
            position += (length + 7) & ~7
        rows.append(row)
    if position != len(data) or (rows and footer is None):
        raise ValueError("missing_kernel_control_footer")
    changed = footer is not None and data[:32] != footer
    count = struct.unpack_from("<Q", data, 8)[0]
    if not changed and count != len(rows):
        raise ValueError("kernel_control_count_mismatch")
    return {"status": "partial" if changed else "observed", "xnu_abi": KCTL_XNU,
            "sockets": rows, "generation_changed_during_read": changed,
            "error": "kernel_control_changed_during_read" if changed else None,
            "stats": dict(zip(KCTL_STATS, struct.unpack("<14Q", statistics)))}


def xnu_version():
    version = command("/usr/sbin/sysctl", "-n", "kern.version")
    match = re.search(r"xnu-([\d.]+)~", version) if isinstance(version, str) else None
    return match[1] if match else None


def kernel_control_snapshot(version=None):
    version = version or xnu_version()
    if version != KCTL_XNU:
        return {"status": "unsupported", "error": "unsupported_xnu_kernel_control_abi"}
    data = command("/usr/sbin/sysctl", "-b", "net.systm.kctl.pcblist", binary=True)
    statistics = command("/usr/sbin/sysctl", "-b", "net.systm.kctl.stats", binary=True)
    if not isinstance(data, bytes) or not isinstance(statistics, bytes):
        return {"status": "unavailable", "error": "kernel_control_sysctl_failed"}
    try:
        return parse_kernel_controls(data, statistics, version)
    except ValueError as error:
        return {"status": "unsupported", "error": str(error)}


def parse_internet_socket_buffers(data, protocol, version):
    """Aggregate sb_mbcnt by last_pid, discarding inpcb addresses and pointers.

    Exact XNU source: bsd/netinet/in_pcblist.c:get_pcblist_n and
    bsd/netinet/in_pcb.h:xinpgen. The TCP-only tcpcb TLV is validated then
    discarded. Payload queue bytes and queued mbuf allocation are distinct.
    The PID denotes last socket use; it can survive a process or FD transfer.
    """
    if version != KCTL_XNU or protocol not in ("tcp", "udp"):
        raise ValueError("unsupported_internet_socket_abi")
    if len(data) < 24 or struct.unpack_from("<I", data)[0] != 24:
        raise ValueError("unsupported_internet_socket_header")
    advertised = struct.unpack_from("<I", data, 4)[0]
    expected = [(104, 16), (104, 1), (32, 2), (32, 4), (136, 8)]
    if protocol == "tcp":
        expected.append((204, 32))
    position, total, null_sockets, footer = 24, 0, 0, None
    by_pid = collections.defaultdict(collections.Counter)
    while position < len(data):
        if len(data) - position == 24 and struct.unpack_from("<I", data, position)[0] == 24:
            footer = data[position:]
            position += 24
            break
        row, null_socket = {}, False
        for expected_length, expected_kind in expected:
            if position + 8 > len(data):
                raise ValueError("truncated_internet_socket_tlv")
            length, kind = struct.unpack_from("<II", data, position)
            rounded = (length + 7) & ~7
            if length != expected_length or position + rounded > len(data):
                raise ValueError("unsupported_internet_socket_tlv_length")
            # get_pcblist_n emits a length-only, otherwise zero sockbuf when
            # inp_socket is NULL. Accept precisely that documented null shape.
            known_null_buffer = (expected_kind in (2, 4) and null_socket and kind == 0
                                 and not any(data[position + 8:position + length]))
            if kind != expected_kind and not known_null_buffer:
                raise ValueError("unsupported_internet_socket_tlv_kind")
            if expected_kind == 1:
                null_socket = not any(data[position + 8:position + length])
                row.update(parse_socket_metadata(data, position, {0} if null_socket else {2, 30}))
            elif expected_kind in (2, 4):
                if null_socket and not known_null_buffer:
                    raise ValueError("unsupported_null_socket_buffer_shape")
                row["recv" if expected_kind == 2 else "send"] = parse_socket_buffer(data, position)
            position += rounded
        total += 1
        if null_socket:
            null_sockets += 1
            continue  # There is no live socket whose owner can be inferred.
        counts = by_pid[row["last_pid"]]
        counts["sockets"] += 1
        for direction in ("recv", "send"):
            counts[direction + "_mbuf_bytes"] += row[direction]["mbuf_bytes"]
    if position != len(data) or (total and footer is None) or (not total and advertised and footer is None):
        raise ValueError("missing_internet_socket_footer")
    changed = footer is not None and data[:24] != footer
    # Source deliberately skips dead/racy PCBs, so advertised count is not a
    # promise that every PCB produces a record, even with stable generations.
    return {"status": "partial" if changed else "observed", "xnu_abi": KCTL_XNU,
            "generation_changed_during_read": changed,
            "error": "internet_sockets_changed_during_read" if changed else None,
            "advertised_socket_count": advertised, "socket_count": total,
            "null_socket_count": null_sockets,
            "by_last_pid": [{"last_pid": pid, **dict(counts)} for pid, counts in sorted(by_pid.items())]}


def internet_socket_buffer_snapshot(version):
    result = {}
    for protocol in ("tcp", "udp"):
        if version != KCTL_XNU:
            result[protocol] = {"status": "unsupported", "error": "unsupported_internet_socket_abi"}
            continue
        data = command("/usr/sbin/sysctl", "-b", f"net.inet.{protocol}.pcblist_n", binary=True)
        if not isinstance(data, bytes):
            result[protocol] = {"status": "unavailable", "error": "internet_socket_sysctl_failed"}
            continue
        try:
            result[protocol] = parse_internet_socket_buffers(data, protocol, version)
        except ValueError as error:
            result[protocol] = {"status": "unsupported", "error": str(error)}
    return result


def snapshot(tracker=None, include_traffic=True, boot_id=None, reader=None):
    started, monotonic = utc_now(), time.monotonic()
    boot_id = boot_id or boot_identity()
    reader = reader or ProcessReader()
    processes, errors = reader.processes(boot_id)
    tracker = tracker or LineageTracker()
    tracker.update(processes, getattr(reader, "last_seen_pids", set(processes)))
    sockets = collections.defaultdict(collections.Counter)
    coverage = {}
    for protocol in ("tcp", "udp"):
        result = command("/usr/sbin/netstat", "-anv", "-p", protocol)
        if not isinstance(result, str):
            errors.append(result)
            coverage[protocol] = "unavailable"
            continue
        parsed, rejected = parse_sockets(result, protocol)
        coverage[protocol] = "partial" if rejected else "observed"
        if rejected:
            errors.append({"error": "unrecognized_netstat_rows", "protocol": protocol, "count": rejected})
        for pid, counts in parsed.items():
            sockets[pid].update(counts)
    if include_traffic:
        traffic = command("/usr/bin/nettop", "-P", "-L", "1", "-n", "-x", "-c",
                          "-J", "bytes_in,bytes_out,rx_dupe,rx_ooo,re-tx")
        if isinstance(traffic, str):
            parsed, rejected = parse_traffic(traffic)
            coverage["traffic"] = "partial" if rejected else "observed"
            for pid, counts in parsed.items():
                sockets[pid].update(counts)
            if rejected:
                errors.append({"error": "unrecognized_nettop_rows", "count": rejected})
        else:
            coverage["traffic"] = "unavailable"
            errors.append(traffic)
    else:
        coverage["traffic"] = "not_scheduled"
    mbufs_result = command("/usr/sbin/netstat", "-m")
    mbufs = parse_mbufs(mbufs_result) if isinstance(mbufs_result, str) else {}
    coverage["mbufs"] = "observed" if "mbufs_in_use" in mbufs else "unavailable"
    if not mbufs:
        errors.append(mbufs_result if isinstance(mbufs_result, dict) else {"error": "unrecognized_mbuf_counters"})
    file_limits = command("/usr/sbin/sysctl", "kern.num_files", "kern.maxfiles", "kern.maxfilesperproc")
    limits = {}
    if isinstance(file_limits, str):
        for line in file_limits.splitlines():
            key, _, value = line.partition(": ")
            if key in ("kern.num_files", "kern.maxfiles", "kern.maxfilesperproc") and value.isdigit():
                limits[key] = int(value)
    else:
        errors.append(file_limits)
    interface_result = command("/usr/sbin/netstat", "-ibdn")
    interfaces = []
    if isinstance(interface_result, str):
        interfaces, rejected = parse_interfaces(interface_result)
        coverage["interfaces"] = "partial" if rejected else "observed"
        if rejected:
            errors.append({"error": "unrecognized_interface_rows", "count": rejected})
    else:
        coverage["interfaces"] = "unavailable"
        errors.append(interface_result)
    version = xnu_version()
    kernel_controls = kernel_control_snapshot(version)
    coverage["kernel_controls"] = kernel_controls["status"]
    if kernel_controls["status"] != "observed":
        errors.append({"error": kernel_controls["error"]})
    internet_buffers = internet_socket_buffer_snapshot(version)
    for protocol, buffers in internet_buffers.items():
        coverage[protocol + "_socket_buffers"] = buffers["status"]
        if buffers["status"] != "observed":
            errors.append({"error": buffers["error"], "protocol": protocol})
    # Network owners, watched idle processes, and their ancestors.
    selected = set(sockets) | {pid for pid, p in processes.items()
                               if p["relation"]["role"] != "other" or p.get("network_extension")}
    selected.update(row["last_pid"] for row in kernel_controls.get("sockets", []) if row["last_pid"] > 0)
    selected.update(row["last_pid"] for buffers in internet_buffers.values()
                    for row in buffers.get("by_last_pid", []) if row["last_pid"] > 0)
    for pid in list(selected):
        seen = set()
        while pid in processes and pid not in seen:
            seen.add(pid)
            pid = processes[pid]["ppid"]
            if pid in processes:
                selected.add(pid)
    # Validate again after counters: PID reuse must not inherit old ownership.
    current_processes, _ = reader.processes(boot_id)
    output, unstable = [], 0
    for pid in sorted(selected):
        process = processes.get(pid)
        current = current_processes.get(pid)
        if process and current and current["identity"] == process["identity"]:
            row = dict(process)
            parent = processes.get(row["ppid"])
            row["parent_identity"] = parent["identity"] if parent else None
            if pid in sockets:
                row["counters"] = dict(sockets[pid])
            elif coverage.get("tcp") == "observed" and coverage.get("udp") == "observed":
                row["counters"] = {"tcp_sockets": 0, "udp_sockets": 0,
                                   "recv_queue_bytes": 0, "send_queue_bytes": 0}
            output.append(row)
        elif pid in sockets:
            output.append({"pid": pid, "identity": None, "name": "kernel_or_owner_unavailable",
                           "counters": dict(sockets[pid])})
            if pid:
                unstable += 1
    if unstable:
        errors.append({"error": "socket_owner_identity_unavailable_or_changed", "count": unstable})
    validated = {row["pid"]: row["identity"] for row in output if row.get("identity")}
    for row in kernel_controls.get("sockets", []):
        # Kernel last_pid itself has no process start time and can be stale.
        row["current_process_at_last_pid_identity"] = validated.get(row["last_pid"])
    for buffers in internet_buffers.values():
        for row in buffers.get("by_last_pid", []):
            row["current_process_at_last_pid_identity"] = validated.get(row["last_pid"])
    return {"schema": 2, "kind": "sample", "time": started, "sample_started_at": started,
            "sample_finished_at": utc_now(), "sample_monotonic_seconds": monotonic,
            "sample_duration_seconds": round(time.monotonic() - monotonic, 4),
            "boot_id": boot_id, "os": platform.mac_ver()[0], "mbufs": mbufs,
            "file_limits": limits, "interfaces": interfaces, "kernel_controls": kernel_controls,
            "internet_socket_buffers": internet_buffers,
            "processes": output, "coverage": coverage,
            "process_inventory_count": len(processes), "errors": errors}


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
    tracker = LineageTracker()
    with open(os.open(path, os.O_WRONLY | os.O_CREAT | os.O_EXCL, 0o600), "w") as output:
        print(path, flush=True)
        for index in range(args.count):
            output.write(json.dumps(snapshot(tracker), ensure_ascii=False, separators=(",", ":")) + "\n")
            output.flush()
            os.fsync(output.fileno())
            if index + 1 < args.count:
                time.sleep(args.interval)


if __name__ == "__main__":
    main()
