#!/usr/bin/env python3
"""Launch an isolated client with a private, temporary OTLP development profile."""
import argparse
import http.client
import json
import os
from pathlib import Path
import subprocess
import sys
import time
import tempfile
import urllib.parse
import uuid
from dev_services import backend_ready, ensure

ROOT = Path(__file__).resolve().parent.parent
BASE = ROOT / "target/telemetry-development"
BACKEND = ROOT / "target/otlp-backend-probe"


def private_json(path, value):
    path.parent.mkdir(parents=True, exist_ok=True, mode=0o700)
    path.write_text(json.dumps(value, ensure_ascii=False, indent=2) + "\n")
    path.chmod(0o600)


def run(*args, **kwargs):
    return subprocess.run(args, check=True, text=True, capture_output=True, timeout=kwargs.pop("timeout", 60), **kwargs).stdout.strip()


def workspace_revision():
    try:
        return run("git", "rev-parse", "HEAD", cwd=ROOT)
    except (OSError, subprocess.SubprocessError):
        # Installed or copied development artifacts need not include a Git checkout.
        return None


def backend_json(path, token=None):
    # This local connection neither uses environment proxies nor follows redirects with credentials.
    connection = http.client.HTTPConnection("127.0.0.1", 30300, timeout=10)
    try:
        connection.request("GET", path, headers={"Authorization": "Bearer " + token} if token else {})
        response = connection.getresponse()
        if response.status != 200:
            raise RuntimeError("Grafana 查询未成功")
        data = response.read(4 * 1024 * 1024 + 1)
        if len(data) > 4 * 1024 * 1024:
            raise RuntimeError("查询结果超过 4 MiB，请缩小查询范围")
        return json.loads(data)
    finally:
        connection.close()


def ready():
    if os.name != "nt":
        ensure("telemetry")
    # The Mac workspace declares the reverse forward to Windows in svc.json.
    backend_ready()


def launch(args):
    run_id = "dev-" + uuid.uuid4().hex
    directory = BASE / run_id
    directory.mkdir(parents=True, mode=0o700)
    profile = {"endpoint": "http://127.0.0.1:34317", "headers": {}, "sampleRatio": 1.0}
    attached = True
    try:
        if args.platform == "android":
            run(sys.executable, str(ROOT / "scripts/dev_services.py"), "android", "start", "--device", args.device, "--adb", str(args.adb), timeout=200)
        else:
            ready()
    except (OSError, RuntimeError, ValueError, subprocess.SubprocessError, http.client.HTTPException) as error:
        attached = False
        print(f"开发遥测未接入（{type(error).__name__}）；继续启动业务。", file=sys.stderr)
    local_profile = directory / "profile.json"
    if attached:
        private_json(local_profile, profile)
    manifest = {"runId": run_id, "platform": args.platform, "startedUnixMs": int(time.time() * 1000),
                "appPath": str(args.app) if args.app else None,
                "endpoint": profile["endpoint"] if attached else None, "sampleRatio": 1.0,
                "attached": attached, "viewer": "http://127.0.0.1:30300", "statusPath": None,
                "manifestPath": str(directory / "run.json"), "queryTokenPath": str(BACKEND / "query-token.json"), "retentionHours": 72, "workspaceRevision": workspace_revision()}
    # Isolated clients inherit runtime paths, never unrelated credentials or agent control sockets.
    inherited = ("PATH", "SystemRoot", "windir", "TEMP", "TMP", "TMPDIR", "LANG", "LC_ALL", "LC_CTYPE", "USER", "LOGNAME", "SHELL", "DEVELOPER_DIR", "JAVA_HOME", "ANDROID_HOME", "ANDROID_SDK_ROOT")
    environment = {name: os.environ[name] for name in inherited if name in os.environ}
    environment["NO_PROXY"] = "127.0.0.1,localhost,::1"
    environment["VELUNE_RUN_ID"] = run_id
    # A missing explicit profile is observable as development_configuration_unavailable.
    environment["VELUNE_TELEMETRY_PROFILE"] = str(local_profile)
    if args.platform in ("macos", "windows"):
        # Remote control has a Unix socket; checkout/run-id paths exceed sun_path on macOS.
        home = Path(tempfile.mkdtemp(prefix="velune-dev-"))
        os_home = home / "os-home"
        os_home.mkdir()
        environment["VELUNE_HOME"] = str(home)
        environment["HOME"] = str(os_home)
        if args.platform == "windows":
            environment["USERPROFILE"] = str(os_home)
        manifest["statusPath"] = str(home / "telemetry/status.json")
        with (directory / "process.log").open("wb") as output:
            process = subprocess.Popen([str(args.app)], env=environment, stdout=output, stderr=subprocess.STDOUT, start_new_session=(os.name != "nt"))
        manifest["processId"] = process.pid
    elif args.platform == "ios-simulator":
        if args.app:
            run("xcrun", "simctl", "install", args.device, str(args.app))
        container = Path(run("xcrun", "simctl", "get_app_container", args.device, args.package, "data"))
        device_profile = container / "Library/Application Support/development-telemetry" / (run_id + ".json")
        if attached:
            private_json(device_profile, profile)
        child = {**environment, "SIMCTL_CHILD_VELUNE_TELEMETRY_PROFILE": str(device_profile), "SIMCTL_CHILD_VELUNE_RUN_ID": run_id}
        run("xcrun", "simctl", "launch", "--terminate-running-process", args.device, args.package, env=child)
        manifest["statusPath"] = str(container / "Library/Application Support/client/telemetry/status.json")
        manifest["device"] = args.device
        manifest["package"] = args.package
    else:
        adb = str(args.adb)
        if args.app:
            run(adb, "-s", args.device, "install", "-r", str(args.app))
        run(adb, "-s", args.device, "shell", "run-as", args.package, "mkdir", "-p", "no_backup/development-telemetry")
        relative = f"no_backup/development-telemetry/{run_id}.json"
        device_profile = f"/data/user/0/{args.package}/{relative}"
        if attached:
            # Authentication travels on stdin, never in an argument or world-readable /sdcard file.
            run(adb, "-s", args.device, "shell", "-T", "run-as", args.package, "sh", "-c", f"'umask 077; cat > {relative}'",
                input=json.dumps(profile))
        run(adb, "-s", args.device, "shell", "am", "start", "-S", "-n", args.package + "/app.velune.android.MainActivity",
            "--es", "velune.telemetry.profile", device_profile, "--es", "velune.telemetry.run", run_id)
        manifest.update(device=args.device, package=args.package, statusPath=f"no_backup/telemetry/status.json")
    private_json(directory / "run.json", manifest)
    print(json.dumps(manifest, ensure_ascii=False, indent=2))


def status(args):
    manifest = json.loads(args.manifest.read_text())
    try:
        if manifest["platform"] == "android":
            text = run(str(args.adb), "-s", manifest["device"], "shell", "run-as", manifest["package"], "cat", manifest["statusPath"])
        elif manifest["platform"] == "ios-simulator" and manifest.get("package"):
            container = Path(run("xcrun", "simctl", "get_app_container", manifest["device"], manifest["package"], "data"))
            text = (container / "Library/Application Support/client/telemetry/status.json").read_text()
        else:
            text = Path(manifest["statusPath"]).read_text()
        state = json.loads(text)
    except (OSError, ValueError, subprocess.SubprocessError):
        print(json.dumps({"queriedRunId": manifest["runId"], "stateAvailable": False,
                          "status": "state_unavailable", "belongsToRun": None, "stale": True}))
        return
    state["stateAvailable"] = True
    state["belongsToRun"] = state.get("runId") == manifest["runId"]
    state["stale"] = not state["belongsToRun"] or int(time.time() * 1000) - state["updatedUnixMs"] > 45000
    print(json.dumps(state, ensure_ascii=False, indent=2))


def query(args):
    manifest = json.loads(args.manifest.read_text())
    ready()
    token = json.loads(Path(manifest["queryTokenPath"]).read_text())["key"]
    start = manifest["startedUnixMs"] // 1000
    end = int(time.time())
    run_id = manifest["runId"]
    schema = args.schema_version
    trace_filter = ' && resource.velune.telemetry.schema.version = "2"' if schema == "2" else (' && resource.velune.telemetry.schema.version = nil' if schema == "legacy" else "")
    log_filter = ' | velune_telemetry_schema_version="2"' if schema == "2" else (' | velune_telemetry_schema_version=""' if schema == "legacy" else "")
    metric_filter = ',velune_telemetry_schema_version="2"' if schema == "2" else (',velune_telemetry_schema_version=""' if schema == "legacy" else "")
    requests = {
        "traces": ("tempo/api/search", {"q": '{ resource.velune.run.id = "' + run_id + '"' + trace_filter + ' }', "start": start, "end": end, "limit": 100}),
        "logs": ("loki/loki/api/v1/query_range", {"query": '{service_name=~"velune[.].*"} | velune_run_id="' + run_id + '"' + log_filter, "start": str(start * 10**9), "end": str(end * 10**9), "limit": 100}),
        "metrics": ("prometheus/api/v1/query", {"query": '{__name__=~"velune_.*"} and on(job,instance) target_info{velune_run_id="' + run_id + '"' + metric_filter + '}', "time": end}),
    }
    path, parameters = requests[args.signal]
    data = backend_json("/api/datasources/proxy/uid/" + path + "?" + urllib.parse.urlencode(parameters), token)
    print(json.dumps({"start": start, "end": end, "limit": 100 if args.signal != "metrics" else None,
                      "runId": run_id, "schemaVersion": schema, "data": data}, ensure_ascii=False))


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    commands = parser.add_subparsers(dest="command", required=True)
    launch_parser = commands.add_parser("launch")
    launch_parser.add_argument("platform", choices=("macos", "windows", "android", "ios-simulator"))
    launch_parser.add_argument("--app", type=Path, help="桌面可执行文件；移动端省略则使用已安装应用")
    launch_parser.add_argument("--device", help="已启动且专用于合成验收的 Android / Simulator 设备")
    launch_parser.add_argument("--package", help="已安装移动应用的 bundle/application ID")
    for command in (launch_parser, commands.add_parser("status")):
        command.add_argument("--adb", type=Path, default=Path(os.environ.get("ANDROID_HOME", str(Path.home() / "Library/Android/sdk"))) / "platform-tools/adb")
        if command is not launch_parser:
            command.add_argument("manifest", type=Path)
    query_parser = commands.add_parser("query")
    query_parser.add_argument("manifest", type=Path)
    query_parser.add_argument("signal", choices=("traces", "logs", "metrics"))
    query_parser.add_argument("--schema-version", choices=("2", "legacy", "all"), default="2", help="遥测契约版本；legacy 为无版本标记的旧数据，all 保留混合原始结果")
    args = parser.parse_args()
    if args.command == "launch":
        if args.platform in ("macos", "windows") and not args.app:
            parser.error("桌面启动需要 --app")
        if args.app and (not args.app.is_absolute() or not args.app.exists()):
            parser.error("--app 必须是构建产物的绝对路径")
        if args.platform in ("android", "ios-simulator") and not (args.device and args.package):
            parser.error("移动启动需要 --device 和 --package")
    {"launch": launch, "status": status, "query": query}[args.command](args)


if __name__ == "__main__":
    try:
        main()
    except (OSError, RuntimeError, ValueError, subprocess.SubprocessError, http.client.HTTPException) as error:
        # Subprocess stderr can contain a receiver's response; keep credentials out of command output.
        print(f"开发遥测操作失败：{type(error).__name__}", file=sys.stderr)
        sys.exit(1)
