#!/usr/bin/env python3
"""Bounded local macOS recorder, independent of Velune's lifetime.

Install explicitly: python3 scripts/network-flight-recorder-macos.py install
Inspect/stop/remove: use status, stop, uninstall. Uninstall preserves evidence.
Default: 30s resource samples, 300s process traffic, 128MiB rotating JSONL,
seven-day observation deadline (not a guarantee of seven days of history).
No network requests, packet capture, addresses, process arguments or settings.
"""
import argparse
import datetime
import fcntl
import importlib.util
import json
import os
from pathlib import Path
import platform
import plistlib
import resource
import signal
import stat
import subprocess
import sys
import tempfile
import time
import uuid


LABEL = "app.velune.network-flight-recorder"
DEFAULT_ROOT = Path.home() / "Library/Application Support/Velune/NetworkDiagnostics"
PLIST = Path.home() / "Library/LaunchAgents" / (LABEL + ".plist")
DIAGNOSTIC_NAME = "diagnose-network-macos.py"
RECORDER_NAME = "network-flight-recorder-macos.py"
FILE_BYTES = 8 * 1024 * 1024
FILE_COUNT = 16


def utc_now():
    return datetime.datetime.now(datetime.timezone.utc).isoformat()


def private_directory(path):
    path.mkdir(parents=True, exist_ok=True, mode=0o700)
    info = path.lstat()
    if not stat.S_ISDIR(info.st_mode) or info.st_uid != os.getuid():
        raise RuntimeError("unsafe_directory")
    path.chmod(0o700)


def fsync_directory(path):
    descriptor = os.open(path, os.O_RDONLY)
    try:
        os.fsync(descriptor)
    finally:
        os.close(descriptor)


def atomic_write(path, data, mode=0o600):
    if path.is_symlink():
        raise RuntimeError("unsafe_symlink")
    descriptor, temporary = tempfile.mkstemp(prefix=".write-", dir=path.parent)
    try:
        os.fchmod(descriptor, mode)
        with os.fdopen(descriptor, "wb") as output:
            output.write(data)
            output.flush()
            os.fsync(output.fileno())
        os.replace(temporary, path)
        fsync_directory(path.parent)
    finally:
        if os.path.exists(temporary):
            os.unlink(temporary)


def read_json(path):
    if path.is_symlink():
        raise RuntimeError("unsafe_symlink")
    return json.loads(path.read_text())


def lock_file(path):
    descriptor = os.open(path, os.O_RDWR | os.O_CREAT | os.O_NOFOLLOW, 0o600)
    try:
        info = os.fstat(descriptor)
        if not stat.S_ISREG(info.st_mode) or info.st_uid != os.getuid():
            raise RuntimeError("unsafe_lock")
        os.fchmod(descriptor, 0o600)
        fcntl.flock(descriptor, fcntl.LOCK_EX | fcntl.LOCK_NB)
        return descriptor
    except BaseException:
        os.close(descriptor)
        raise


class RotatingLog:
    def __init__(self, directory, max_bytes=FILE_BYTES, max_files=FILE_COUNT):
        private_directory(directory)
        self.directory = directory
        self.max_bytes, self.max_files = max_bytes, max_files
        self.run = datetime.datetime.now(datetime.timezone.utc).strftime("%Y%m%dT%H%M%S") + "-" + uuid.uuid4().hex[:8]
        self.sequence = 0
        self.output, self.size = None, 0
        self.prune()

    def prune(self):
        files = sorted(self.directory.glob("samples-*.jsonl"), key=lambda p: p.stat().st_mtime_ns)
        for path in files[:-self.max_files]:
            if path.is_symlink() or not path.is_file():
                raise RuntimeError("unsafe_log_file")
            path.unlink()
        fsync_directory(self.directory)

    def write(self, record):
        encoded = (json.dumps(record, ensure_ascii=False, separators=(",", ":")) + "\n").encode()
        if len(encoded) > self.max_bytes:
            encoded = (json.dumps({"kind": "error", "time": utc_now(),
                "error": "sample_exceeds_file_limit", "sample_bytes": len(encoded)}) + "\n").encode()
        if self.output is None or self.size + len(encoded) > self.max_bytes:
            self.close()
            self.sequence += 1
            path = self.directory / f"samples-{self.run}-{self.sequence:05d}.jsonl"
            descriptor = os.open(path, os.O_WRONLY | os.O_CREAT | os.O_EXCL | os.O_NOFOLLOW, 0o600)
            self.output = os.fdopen(descriptor, "wb")
            self.size = 0
            self.prune()
        self.output.write(encoded)
        self.output.flush()
        os.fsync(self.output.fileno())
        self.size += len(encoded)

    def close(self):
        if self.output:
            self.output.close()
            self.output = None


def load_diagnostic():
    path = Path(__file__).absolute().with_name(DIAGNOSTIC_NAME)
    spec = importlib.util.spec_from_file_location("velune_network_snapshot", path)
    module = importlib.util.module_from_spec(spec)
    spec.loader.exec_module(module)
    return module


def usage_seconds():
    own = resource.getrusage(resource.RUSAGE_SELF)
    children = resource.getrusage(resource.RUSAGE_CHILDREN)
    return own.ru_utime + own.ru_stime + children.ru_utime + children.ru_stime


def run(args):
    os.umask(0o077)
    private_directory(args.root)
    try:
        lock = lock_file(args.root / "recorder.lock")
    except BlockingIOError:
        print("Recorder already running", file=sys.stderr)
        return 0  # A duplicate is not an abnormal exit for launchd to retry.
    log = None
    try:
        log = RotatingLog(args.root / "records")
        now = time.time()
        deadline = args.expires_at or (now + args.days * 86400)
        # Manual runs also retain their deadline over abnormal restart.
        config_path = args.root / "run.json"
        if config_path.exists() and not args.expires_at:
            deadline = read_json(config_path)["expires_at"]
        if now >= deadline:
            log.write({"kind": "stopped", "time": utc_now(), "reason": "observation_expired",
                       "expires_at": deadline})
            return 0
        diagnostic = load_diagnostic()
        boot = diagnostic.boot_identity()
        reader = diagnostic.ProcessReader()
        tracker = diagnostic.LineageTracker()
        state_path = args.root / "state.json"
        state_error = None
        previous, previous_boot = None, None
        if state_path.exists():
            try:
                state = read_json(state_path)
                previous_boot = state.get("boot_id")
                if state.get("boot_id") == boot:
                    tracker.retained = state.get("lineage", {})
                previous = state.get("last_sample_at")
            except (OSError, ValueError, TypeError):
                state_error = "previous_state_unreadable"
        atomic_write(config_path, json.dumps({"expires_at": deadline, "interval": args.interval,
                     "traffic_interval": args.traffic_interval}).encode())
        os.ftruncate(lock, 0)
        os.write(lock, json.dumps({"pid": os.getpid(), "started_at": utc_now(),
                                 "boot_id": boot, "expires_at": deadline}).encode())
        os.fsync(lock)
        stopping = []
        def stop(signum, _frame):
            stopping.append(signum)
        signal.signal(signal.SIGTERM, stop)
        signal.signal(signal.SIGINT, stop)
        previous_gap = None
        if previous:
            try:
                previous_gap = round(max(0, time.time() - datetime.datetime.fromisoformat(previous).timestamp() - args.interval), 4)
            except ValueError:
                state_error = "previous_sample_timestamp_unreadable"
        log.write({"kind": "started", "time": utc_now(), "boot_id": boot,
                   "pid": os.getpid(), "interval_seconds": args.interval,
                   "traffic_interval_seconds": args.traffic_interval,
                   "expires_at": deadline, "disk_budget_bytes": FILE_BYTES * FILE_COUNT,
                   "previous_sample_at": previous, "previous_boot_id": previous_boot,
                   "previous_sample_gap_seconds": previous_gap, "state_error": state_error,
                   "limitations": ["samples_non_atomic", "short_lived_processes_may_be_missed",
                      "ancestry_known_only_when_observed", "extension_flows_not_client_attribution",
                      "kernel_socket_last_pid_may_be_stale", "absence_does_not_exclude_cause",
                      "retention_is_byte_bounded"]})
        due = time.monotonic()
        traffic_due = due
        last_start, sample_count = None, 0
        reason = "observation_expired"
        while not stopping and time.time() < deadline:
            now_mono = time.monotonic()
            if now_mono < due:
                time.sleep(min(1, due - now_mono, max(0, deadline - time.time())))
                continue
            before_cpu = usage_seconds()
            before_wall = time.time()
            try:
                include_traffic = now_mono >= traffic_due
                sample = diagnostic.snapshot(tracker, include_traffic=include_traffic,
                                             boot_id=boot, reader=reader)
                sample["scheduled_lateness_seconds"] = round(max(0, now_mono - due), 4)
                sample["since_previous_sample_seconds"] = (round(before_wall - last_start, 4)
                                                             if last_start else None)
                sample["gap_seconds"] = round(max(0, before_wall - last_start - args.interval), 4) if last_start else None
                sample["collector_cpu_seconds"] = round(usage_seconds() - before_cpu, 6)
                log.write(sample)
                atomic_write(state_path, json.dumps({"boot_id": boot,
                    "last_sample_at": sample["sample_started_at"],
                    "last_sample_finished_at": sample["sample_finished_at"],
                    "coverage": sample["coverage"], "errors": sample["errors"],
                    "kernel_control_socket_count": len(sample["kernel_controls"].get("sockets", [])),
                    "mbufs_in_use": sample["mbufs"].get("mbufs_in_use"),
                    "sample_duration_seconds": sample["sample_duration_seconds"],
                    "lineage": tracker.retained}).encode())
                last_start = before_wall
                if include_traffic:
                    traffic_due = now_mono + args.traffic_interval
            except Exception as error:
                # Record a classified failure, never raw command text or traceback.
                log.write({"kind": "error", "time": utc_now(), "error": "sample_failed",
                           "exception_type": type(error).__name__})
            sample_count += 1
            if args.samples and sample_count >= args.samples:
                reason = "requested_samples_complete"
                break
            due += args.interval
            if due < time.monotonic():
                missed = int((time.monotonic() - due) // args.interval) + 1
                log.write({"kind": "gap", "time": utc_now(), "reason": "collector_overran_interval",
                           "missed_samples": missed})
                due += missed * args.interval
        if stopping:
            reason = "signal"
        log.write({"kind": "stopped", "time": utc_now(), "reason": reason,
                   "signal": stopping[0] if stopping else None, "samples": sample_count})
        return 0
    except Exception as error:
        if log:
            try:
                log.write({"kind": "fatal", "time": utc_now(), "error": type(error).__name__})
            except Exception:
                pass
        print("Recorder failed: " + type(error).__name__, file=sys.stderr)
        return 1
    finally:
        if log:
            log.close()
        os.close(lock)


def launchctl(*arguments):
    result = subprocess.run(["/bin/launchctl", *arguments], capture_output=True, text=True)
    return result.returncode


def service_target():
    return f"gui/{os.getuid()}/{LABEL}"


def install(args):
    if PLIST.exists():
        raise RuntimeError("already_installed_use_uninstall_first")
    private_directory(args.root)
    # Do not replace modules beneath an already-running manual recorder.
    descriptor = lock_file(args.root / "recorder.lock")
    try:
        deadline = time.time() + args.days * 86400
        source = Path(__file__).absolute()
        for path in (source, source.with_name(DIAGNOSTIC_NAME)):
            atomic_write(args.root / path.name, path.read_bytes(), mode=0o700)
        atomic_write(args.root / "run.json", json.dumps({"expires_at": deadline,
            "interval": args.interval, "traffic_interval": args.traffic_interval}).encode())
        PLIST.parent.mkdir(parents=True, exist_ok=True)
        arguments = [sys.executable, str(args.root / RECORDER_NAME), "run", "--root", str(args.root),
                     "--interval", str(args.interval), "--traffic-interval", str(args.traffic_interval),
                     "--expires-at", str(deadline)]
        definition = {"Label": LABEL, "ProgramArguments": arguments, "RunAtLoad": True,
            "KeepAlive": {"SuccessfulExit": False}, "ThrottleInterval": 60,
            "ProcessType": "Background", "LowPriorityIO": True, "Nice": 10,
            "StandardOutPath": "/dev/null", "StandardErrorPath": "/dev/null",
            "EnvironmentVariables": {"PYTHONDONTWRITEBYTECODE": "1", "LC_ALL": "C"}}
        atomic_write(PLIST, plistlib.dumps(definition))
    finally:
        os.close(descriptor)
    launchctl("enable", service_target())
    if launchctl("bootstrap", f"gui/{os.getuid()}", str(PLIST)):
        raise RuntimeError("launchd_bootstrap_failed_files_retained_for_inspection")
    print(json.dumps({"installed": True, "root": str(args.root), "plist": str(PLIST),
                      "expires_at": deadline, "disk_budget_bytes": FILE_BYTES * FILE_COUNT}))
    return 0


def status(args):
    running, metadata = False, None
    path = args.root / "recorder.lock"
    if path.exists():
        try:
            descriptor = lock_file(path)
        except BlockingIOError:
            running = True
        else:
            os.close(descriptor)
        try:
            metadata = read_json(path)
        except (OSError, ValueError):
            metadata = {"error": "lock_metadata_unreadable"}
    state, config = {}, {}
    state_error = None
    for name, target in (("state.json", state), ("run.json", config)):
        path = args.root / name
        if path.exists():
            try:
                target.update(read_json(path))
            except (OSError, ValueError, TypeError):
                state_error = "status_state_unreadable"
    age = None
    if state.get("last_sample_finished_at"):
        try:
            age = round(time.time() - datetime.datetime.fromisoformat(state["last_sample_finished_at"]).timestamp(), 2)
        except ValueError:
            state_error = "status_timestamp_unreadable"
    stale = running and age is not None and age > max(120, config.get("interval", 30) * 3)
    health = ("stale" if stale else "collecting_with_limitations" if running and state.get("errors")
              else "collecting" if running and age is not None
              else "waiting_for_first_sample" if running else "stopped")
    files = list((args.root / "records").glob("samples-*.jsonl"))
    print(json.dumps({"running": running, "installed": PLIST.exists(), "root": str(args.root),
        "health": health, "sample_age_seconds": age, "last_sample_at": state.get("last_sample_at"),
        "last_coverage": state.get("coverage"), "last_errors": state.get("errors"),
        "kernel_control_socket_count": state.get("kernel_control_socket_count"),
        "status_error": state_error,
        "record_files": len(files), "record_bytes": sum(p.stat().st_size for p in files),
        "disk_budget_bytes": FILE_BYTES * FILE_COUNT, "last_run": metadata}, ensure_ascii=False))
    return 0


def is_running(root):
    path = root / "recorder.lock"
    if not path.exists():
        return False
    try:
        descriptor = lock_file(path)
    except BlockingIOError:
        return True
    os.close(descriptor)
    return False


def stop_service(args):
    # Disable persists across login, unlike bootout alone. Re-install enables it.
    disable = launchctl("disable", service_target())
    launchctl("bootout", service_target())
    if disable:
        raise RuntimeError("launchd_disable_failed")
    wait_until = time.monotonic() + 10
    while is_running(args.root) and time.monotonic() < wait_until:
        time.sleep(0.2)
    if is_running(args.root):
        raise RuntimeError("recorder_still_running_possibly_manual_run")
    print("Recorder stopped and disabled; evidence retained")
    return 0


def uninstall(args):
    stop_service(args)
    if PLIST.exists():
        PLIST.unlink()
        fsync_directory(PLIST.parent)
    # Keep logs/state; make this operation safe to run from the installed copy.
    for name in (RECORDER_NAME, DIAGNOSTIC_NAME):
        path = args.root / name
        if path.exists():
            path.unlink()
    print("LaunchAgent and installed scripts removed; evidence retained at " + str(args.root))
    return 0


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    subparsers = parser.add_subparsers(dest="action", required=True)
    for action in ("run", "install", "status", "stop", "uninstall"):
        sub = subparsers.add_parser(action)
        sub.add_argument("--root", type=Path, default=DEFAULT_ROOT)
        if action in ("run", "install"):
            sub.add_argument("--interval", type=float, default=30)
            sub.add_argument("--traffic-interval", type=float, default=300)
            sub.add_argument("--days", type=float, default=7)
        if action == "run":
            sub.add_argument("--samples", type=int, default=0, help="0 means run until deadline or signal")
            sub.add_argument("--expires-at", type=float)
    args = parser.parse_args()
    if platform.system() != "Darwin":
        parser.error("macOS only")
    args.root = args.root.expanduser().absolute()
    if args.action in ("run", "install"):
        if not 10 <= args.interval <= 300 or not args.interval <= args.traffic_interval <= 3600 or not 0 < args.days <= 7:
            parser.error("interval: 10–300s; traffic interval: resource interval–3600s; days: >0–7")
    if args.action == "run" and (args.samples < 0 or (args.expires_at is not None and args.expires_at <= 0)):
        parser.error("invalid sample count or expiration")
    try:
        return {"run": run, "install": install, "status": status,
                "stop": stop_service, "uninstall": uninstall}[args.action](args)
    except (OSError, RuntimeError) as error:
        print("Action failed: " + (str(error) if isinstance(error, RuntimeError) else type(error).__name__), file=sys.stderr)
        return 1


if __name__ == "__main__":
    sys.exit(main())
