"""Native transport actions for the capabilities declared in svc.json."""
import argparse
import http.client
import json
from pathlib import Path
import socket
import subprocess

ROOT = Path(__file__).resolve().parent.parent
STATE = Path.home() / ".local/state/velune/dev"
SSH = ["ssh", "-o", "BatchMode=yes", "-o", "ConnectTimeout=5"]


def run(*argv, timeout=15):
    return subprocess.run(argv, check=True, capture_output=True, text=True, timeout=timeout).stdout.strip()


def ensure(target):
    # SVC owns coordination and readiness; native tools retain their lifecycle.
    run("svc", "dev", "ensure", target, "--repo", str(ROOT), "--json", timeout=200)


def backend_ready():
    with socket.create_connection(("127.0.0.1", 34317), timeout=3):
        pass
    connection = http.client.HTTPConnection("127.0.0.1", 30300, timeout=3)
    try:
        connection.request("GET", "/api/health")
        response = connection.getresponse()
        if response.status != 200 or json.loads(response.read(4096)).get("database") != "ok":
            raise RuntimeError("Grafana 尚未就绪")
    finally:
        connection.close()


def tunnel(args):
    control = STATE / (args.service + ".sock")
    if args.action == "stop":
        if control.exists():
            run(*SSH, "-S", str(control), "-O", "exit", args.host)
        return
    if args.action == "status":
        run(*SSH, "-S", str(control), "-O", "check", args.host)
        if args.service == "telemetry":
            backend_ready()
        else:
            run(*SSH, args.host, "powershell -NoProfile -Command \"$ErrorActionPreference='Stop'; "
                "$c=[Net.Sockets.TcpClient]::new('127.0.0.1',34317); $c.Dispose(); "
                "if((Invoke-RestMethod -TimeoutSec 3 http://127.0.0.1:30300/api/health).database -ne 'ok'){exit 1}\"")
        return
    ensure("lgtm" if args.service == "telemetry" else "telemetry")
    existing = subprocess.run([*SSH, "-S", str(control), "-O", "check", args.host], capture_output=True, timeout=10)
    if existing.returncode == 0:
        return
    STATE.mkdir(parents=True, exist_ok=True, mode=0o700)
    direction = "-L" if args.service == "telemetry" else "-R"
    # ExitOnForwardFailure preserves an unrelated listener instead of taking its port.
    argv = [*SSH, "-M", "-S", str(control), "-fNT", "-o", "ExitOnForwardFailure=yes",
            "-o", "ControlPersist=no",
            "-o", "ServerAliveInterval=15", "-o", "ServerAliveCountMax=3",
            direction, "127.0.0.1:30300:127.0.0.1:30300",
            direction, "127.0.0.1:34317:127.0.0.1:34317", args.host]
    run(*argv)


def android(args):
    adb = [args.adb, *(["-s", args.device] if args.device else ["-d"])]
    mappings = [line.split() for line in run(*adb, "reverse", "--list").splitlines()]
    mapped = any(row[-2:] == ["tcp:34317", "tcp:34317"] for row in mappings)
    if args.action == "status":
        if not mapped:
            raise RuntimeError("Android OTLP 转发未连接")
        backend_ready()
    elif args.action == "stop":
        if mapped:
            run(*adb, "reverse", "--remove", "tcp:34317")
    else:
        ensure("telemetry")
        if any(len(row) >= 2 and row[-2] == "tcp:34317" for row in mappings) and not mapped:
            raise RuntimeError("Android 34317 已用于其它转发")
        run(*adb, "reverse", "tcp:34317", "tcp:34317")


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("service", choices=("telemetry", "windows", "android"))
    parser.add_argument("action", choices=("status", "start", "stop"))
    parser.add_argument("--host", help="SSH config 主机别名")
    parser.add_argument("--device", help="Android 序列号；默认唯一 USB 设备")
    parser.add_argument("--adb", default="adb")
    args = parser.parse_args()
    if args.service != "android" and not args.host:
        parser.error("SSH 转发需要 --host")
    try:
        (android if args.service == "android" else tunnel)(args)
    except (OSError, ValueError, RuntimeError, subprocess.SubprocessError, http.client.HTTPException) as error:
        detail = error.stderr.strip()[-1500:] if isinstance(error, subprocess.CalledProcessError) else str(error)
        print(f"{args.service} {args.action}: {detail or type(error).__name__}")
        raise SystemExit(1)


if __name__ == "__main__":
    main()
