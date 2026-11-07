#!/usr/bin/env python3
"""Launch, observe and normally quit one sealed macOS development instance."""
import argparse
import hashlib
import json
import math
import os
from pathlib import Path
import shutil
import stat
import subprocess
import sys
import tempfile

from build_lock import FileLock
import macos_identity as common

ROOT = Path(__file__).resolve().parent.parent
INHERITED = ("PATH", "USER", "LOGNAME", "SHELL", "TMPDIR", "TEMP", "TMP", "LANG",
             "LC_ALL", "LC_CTYPE", "DEVELOPER_DIR", "JAVA_HOME")
EXTRA_ENVIRONMENT = {"VELUNE_RUN_ID", "VELUNE_TELEMETRY_PROFILE"}


class LauncherError(RuntimeError):
    pass


def absolute_path(value, label):
    path = Path(value)
    if not path.is_absolute() or "\0" in str(path):
        raise LauncherError(f"{label}必须是不含 NUL 的绝对路径")
    return path.resolve()


def workspace_path(workspace, profile):
    if profile not in ("dev", "release"):
        raise LauncherError("未知的 Mac 构建优化 profile")
    path = absolute_path(workspace, "工作树")
    if not path.is_dir():
        raise LauncherError("工作树目录不存在")
    return path


def bundle_path(value):
    path = absolute_path(value, "应用路径")
    if path.suffix == ".app":
        return path
    if (path.parent.name == "MacOS" and path.parent.parent.name == "Contents"
            and path.parent.parent.parent.suffix == ".app"):
        return path.parent.parent.parent
    raise LauncherError("应用路径必须是 .app bundle 或其 Contents/MacOS 下的可执行文件")


def validated_bundle(path, scope):
    context = common.validate_bundle(path, "development")
    if context["scope"] != scope:
        raise LauncherError("开发产物 scope 与所选工作树不一致")
    return context


def private_directory(path):
    """Managed directories may never redirect a native agent into user config."""
    if path.is_symlink():
        raise LauncherError(f"私有目录不能是符号链接：{path}")
    path.mkdir(parents=True, exist_ok=True, mode=0o700)
    details = path.stat()
    if not stat.S_ISDIR(details.st_mode) or details.st_uid != os.getuid():
        raise LauncherError(f"私有目录归属无效：{path}")
    path.chmod(0o700)


def private_json(path, value):
    descriptor, temporary = tempfile.mkstemp(prefix="." + path.name + "-", dir=path.parent)
    try:
        with os.fdopen(descriptor, "w") as output:
            json.dump(value, output, ensure_ascii=False, sort_keys=True, indent=2)
            output.write("\n")
            output.flush()
            os.fsync(output.fileno())
        os.replace(temporary, path)
    finally:
        if os.path.exists(temporary):
            os.unlink(temporary)


def read_private_json(path):
    descriptor = os.open(path, os.O_RDONLY | os.O_NOFOLLOW)
    with os.fdopen(descriptor, "r") as source:
        details = os.fstat(source.fileno())
        if (not stat.S_ISREG(details.st_mode) or details.st_uid != os.getuid()
                or details.st_mode & 0o077 or details.st_size > 65536):
            raise LauncherError(f"私有记录的归属、权限或类型无效：{path}")
        value = json.load(source)
    if not isinstance(value, dict):
        raise LauncherError(f"私有记录必须是 JSON 对象：{path}")
    return value


def run(arguments, *, timeout=60, environment=None):
    try:
        completed = subprocess.run([str(value) for value in arguments], check=True,
                                   capture_output=True, text=True, timeout=timeout,
                                   env=environment)
    except subprocess.CalledProcessError as error:
        detail = (error.stderr or error.stdout or "命令执行失败").strip()
        raise LauncherError(detail) from error
    except subprocess.TimeoutExpired as error:
        raise LauncherError("工具操作超时；没有发送应用退出或强制终止") from error
    return completed.stdout.strip()


def native_helper():
    source = ROOT / "scripts/macos-dev-native.swift"
    directory = ROOT / "target/tools"
    private_directory(directory)
    executable = directory / "macos-dev-native"
    record = directory / "macos-dev-native.build.json"
    with FileLock(directory / "macos-dev-native.lock"):
        # Both the compiler and selected SDK participate in the cache identity.
        inputs = {"sourceHash": hashlib.sha256(source.read_bytes()).hexdigest(),
                  "swiftc": run(["xcrun", "swiftc", "--version"]),
                  "sdkVersion": run(["xcrun", "--sdk", "macosx", "--show-sdk-version"]),
                  "sdkPath": run(["xcrun", "--sdk", "macosx", "--show-sdk-path"])}
        try:
            cached = read_private_json(record)
        except (OSError, ValueError, LauncherError):
            cached = None
        if (cached == inputs and not executable.is_symlink() and executable.is_file()
                and os.access(executable, os.X_OK)):
            return executable
        descriptor, temporary = tempfile.mkstemp(prefix=".macos-dev-native-", dir=directory)
        os.close(descriptor)
        try:
            run(["xcrun", "swiftc", "-O", "-sdk", inputs["sdkPath"], source,
                 "-o", temporary], timeout=120)
            os.chmod(temporary, 0o700)
            os.replace(temporary, executable)
            private_json(record, inputs)
        finally:
            if os.path.exists(temporary):
                os.unlink(temporary)
    return executable


def inherited_environment():
    return {name: os.environ[name] for name in INHERITED if name in os.environ}


def native(command, bundle, *arguments):
    output = run([native_helper(), command, "--bundle", bundle, *arguments],
                 environment=inherited_environment(), timeout=45)
    try:
        value = json.loads(output)
    except ValueError as error:
        raise LauncherError("原生实例工具返回了无效 JSON") from error
    if not isinstance(value, dict):
        raise LauncherError("原生实例工具返回值不是对象")
    return value


def application_metadata(value, bundle=None, context=None):
    if not isinstance(value, dict):
        raise LauncherError("原生应用元数据无效")
    pid, birth = value.get("pid"), value.get("launchedAt")
    if (type(pid) is not int or pid <= 0 or type(birth) not in (int, float)
            or not math.isfinite(birth) or birth <= 0):
        raise LauncherError("原生应用 PID 或出生时间无效")
    for field in ("bundlePath", "executablePath", "bundleIdentifier"):
        if not isinstance(value.get(field), str) or not value[field]:
            raise LauncherError("原生应用路径或标识缺失")
    if bundle is not None and any((value["bundlePath"] != str(bundle),
                                  value["executablePath"] != str(bundle / "Contents/MacOS" / context["executable"]),
                                  value["bundleIdentifier"] != context["bundleIdentifier"])):
        raise LauncherError("原生应用身份与准确开发 bundle 不一致")
    return value


def inspect(bundle, context):
    value = native("inspect", bundle)
    for field in ("applications", "sameIdentityApplications"):
        if not isinstance(value.get(field), list):
            raise LauncherError("原生应用检查结果缺少候选列表")
    value["applications"] = [application_metadata(app, bundle, context) for app in value["applications"]]
    value["sameIdentityApplications"] = [application_metadata(app) for app in value["sameIdentityApplications"]]
    return value


def refuse_conflicts(inspection, *, source_only=False):
    candidates = inspection["sameIdentityApplications"]
    if source_only or len(inspection["applications"]) > 1:
        candidates = candidates + inspection["applications"]
    if candidates:
        details = "; ".join(f"PID {app['pid']}：{app['bundlePath']}" for app in candidates)
        raise LauncherError("存在其它路径的同身份应用或多个实例，拒绝操作；请先核对这些实例：" + details)


def paths(workspace):
    directory = workspace / "target/development/macos"
    return directory, directory / "Velune Dev.app", directory / "instance.json"


def lifecycle_lock(workspace):
    return FileLock(workspace / "target/build-state/locks/macos-development.lock")


def read_receipt(path, scope, bundle, context):
    try:
        receipt = read_private_json(path)
    except FileNotFoundError:
        return None
    application_metadata(receipt, bundle, context)
    if (receipt.get("schemaVersion") != 1 or receipt.get("purpose") != "development"
            or receipt.get("scope") != scope):
        raise LauncherError("开发实例记录的用途或 scope 无效")
    for field in ("home", "source"):
        if not isinstance(receipt.get(field), str) or str(absolute_path(receipt[field], field)) != receipt[field]:
            raise LauncherError("开发实例记录的路径无效")
    return receipt


def same_instance(receipt, application):
    return (receipt is not None and receipt["pid"] == application["pid"]
            and abs(receipt["launchedAt"] - application["launchedAt"]) <= 0.000001)


def source_fingerprint(bundle, context):
    """Identify the sealed code and context copied into the running bundle."""
    files = {Path("Contents/Info.plist"), Path("Contents/MacOS") / context["executable"]}
    files.update(path.relative_to(bundle) for path in bundle.rglob("*.dylib"))
    fingerprint = hashlib.sha256(b"velune-development-bundle-source-v1\0")
    for relative in sorted(files):
        content = hashlib.sha256()
        with (bundle / relative).open("rb") as source:
            for chunk in iter(lambda: source.read(1024 * 1024), b""):
                content.update(chunk)
        fingerprint.update(os.fsencode(relative))
        fingerprint.update(b"\0" + content.digest())
    return "sha256:" + fingerprint.hexdigest()


def reuse(application, receipt, home, scope, source, profile, fingerprint):
    if not same_instance(receipt, application) or receipt["home"] != str(home):
        raise LauncherError("运行实例没有匹配的 PID／出生时间／开发目录记录；拒绝猜测环境或替换运行副本")
    differences = []
    for field, requested, label in (("profile", profile, "profile"), ("source", str(source), "source 路径"),
                                    ("sourceFingerprint", fingerprint, "产物内容指纹")):
        if receipt.get(field) != requested:
            differences.append(label)
    if differences:
        raise LauncherError("运行实例的构建来源与请求不一致（" + "、".join(differences)
                            + "）；请先显式正常停止该开发实例，再启动所需产物")
    marker = read_private_json(home / common.MARKER)
    if marker != dict(schemaVersion=1, purpose="development", scope=scope):
        raise LauncherError("运行实例的开发目录归属与 scope 不一致")
    return dict(receipt, **application, status="running", reused=True)


def validated_extra_environment(extra_environment):
    if extra_environment is not None and not isinstance(extra_environment, dict):
        raise LauncherError("额外启动环境必须是字符串映射")
    extra = {} if extra_environment is None else dict(extra_environment)
    if (set(extra) - EXTRA_ENVIRONMENT or any(not isinstance(value, str) or "\0" in value for value in extra.values())):
        raise LauncherError("额外启动环境只接受 VELUNE_RUN_ID 与 VELUNE_TELEMETRY_PROFILE 字符串")
    return extra


def launch_environment(home, scope, extra_environment):
    extra = validated_extra_environment(extra_environment)
    home = common.prepare_development_home(home, scope)
    harness = home / "harness-user"
    for directory in (harness, harness / ".codex", harness / ".claude", harness / ".pi", harness / ".pi/agent", harness / ".dsh"):
        private_directory(directory)
    environment = inherited_environment()
    environment.update(NO_PROXY="localhost,127.0.0.1,::1", VELUNE_HOME=str(home), HOME=str(harness),
                       CODEX_HOME=str(harness / ".codex"), CLAUDE_CONFIG_DIR=str(harness / ".claude"),
                       PI_CODING_AGENT_DIR=str(harness / ".pi/agent"), DSH_HOME=str(harness / ".dsh"))
    environment.update(extra)
    return environment


def verify_signature(bundle):
    run(["/usr/bin/codesign", "--verify", "--deep", "--strict", bundle])


def clone_bundle(source, destination, scope, context, fingerprint):
    if source == destination:
        return
    staging = Path(tempfile.mkdtemp(prefix=".prepare-", dir=destination.parent))
    staged = staging / destination.name
    try:
        try:
            run(["/bin/cp", "-cR", source, staged], timeout=120)
        except LauncherError:
            if staged.exists():
                shutil.rmtree(staged)
            run(["/usr/bin/ditto", source, staged], timeout=120)
        validated_bundle(staged, scope)
        verify_signature(staged)
        if source_fingerprint(staged, context) != fingerprint:
            raise LauncherError("准备运行副本时源产物内容发生变化；拒绝启动或替换现有运行副本")
        # Recheck immediately before changing the only managed runtime copy.
        if destination.exists():
            current = inspect(destination, context)
            refuse_conflicts(current)
            if current["applications"]:
                raise LauncherError("准备副本期间应用已启动；保留已加载的运行副本")
            previous = staging / "previous.app"
            os.replace(destination, previous)
            try:
                os.replace(staged, destination)
            except BaseException:
                os.replace(previous, destination)
                raise
        else:
            refuse_conflicts(inspect(source, context), source_only=True)
            os.replace(staged, destination)
    finally:
        shutil.rmtree(staging)


def start(workspace=ROOT, profile="dev", app=None, home=None, extra_environment=None):
    workspace = workspace_path(workspace, profile)
    extra_environment = validated_extra_environment(extra_environment)
    scope = common.identity(workspace, "development")["context"]["scope"]
    source_input = absolute_path(app if app is not None else common.app_bundle_path(workspace, profile), "应用路径")
    source = bundle_path(source_input)
    home = absolute_path(home if home is not None else common.default_development_home(scope), "开发目录")
    directory, bundle, receipt_path = paths(workspace)
    with lifecycle_lock(workspace):
        with FileLock(workspace / "target/build-state/locks/macos.lock"):
            # Wait for an in-flight build, then reject unsealed/daily/foreign
            # sources before any native application inspection.
            context = validated_bundle(source, scope)
            if source_input != source and source_input != source / "Contents/MacOS" / context["executable"]:
                raise LauncherError("提供的可执行文件路径与 bundle 用途元数据不一致")
            verify_signature(source)
            fingerprint = source_fingerprint(source, context)
            if bundle.exists() or bundle.is_symlink():
                validated_bundle(bundle, scope)
                verify_signature(bundle)
                observed = inspect(bundle, context)
                refuse_conflicts(observed)
                if observed["applications"]:
                    receipt = read_receipt(receipt_path, scope, bundle, context)
                    return reuse(observed["applications"][0], receipt, home, scope, source, profile, fingerprint)
            else:
                refuse_conflicts(inspect(source, context), source_only=True)
            private_directory(directory.parent)
            private_directory(directory)
            clone_bundle(source, bundle, scope, context, fingerprint)
        environment = launch_environment(home, scope, extra_environment)
        descriptor, environment_path = tempfile.mkstemp(prefix=".environment-", suffix=".json", dir=directory)
        try:
            with os.fdopen(descriptor, "w") as output:
                json.dump(environment, output, sort_keys=True)
            launched = native("start", bundle, "--environment", environment_path)
        finally:
            os.unlink(environment_path)
        application = application_metadata(launched.get("application"), bundle, context)
        if type(launched.get("reused")) is not bool:
            raise LauncherError("原生启动结果缺少明确的复用状态")
        if launched["reused"]:
            return reuse(application, read_receipt(receipt_path, scope, bundle, context), home, scope,
                         source, profile, fingerprint)
        receipt = dict(application, schemaVersion=1, purpose="development", scope=scope,
                       home=str(home), source=str(source), profile=profile, sourceFingerprint=fingerprint)
        try:
            private_json(receipt_path, receipt)
        except OSError as error:
            raise LauncherError(f"开发应用已启动（PID {application['pid']}），但实例记录写入失败；可用 stop 按新鲜身份正常退出") from error
        return dict(receipt, status="running", reused=False)


def status(workspace=ROOT, profile="dev"):
    workspace = workspace_path(workspace, profile)
    scope = common.identity(workspace, "development")["context"]["scope"]
    source = common.app_bundle_path(workspace, profile)
    _, bundle, receipt_path = paths(workspace)
    result = dict(status="notBuilt", purpose="development", scope=scope, profile=profile,
                  pid=None, launchedAt=None, bundlePath=str(bundle), executablePath=None,
                  home=None, source=str(source), sourceFingerprint=None, receiptState="missing")
    with lifecycle_lock(workspace):
        runtime_exists = bundle.exists() or bundle.is_symlink()
        candidate = bundle if runtime_exists else source
        if not candidate.exists() and not candidate.is_symlink():
            return result
        context = validated_bundle(candidate, scope)
        observed = inspect(candidate, context)
        result.update(status="notRunning", bundleIdentifier=context["bundleIdentifier"],
                      executablePath=str(bundle / "Contents/MacOS" / context["executable"]))
        conflicts = observed["sameIdentityApplications"]
        if not runtime_exists or len(observed["applications"]) > 1:
            conflicts += observed["applications"]
        if conflicts:
            return dict(result, status="identityConflict", sameIdentityApplications=conflicts)
        try:
            receipt = read_receipt(receipt_path, scope, bundle, context)
        except (OSError, ValueError, LauncherError) as error:
            receipt = None
            result.update(receiptState="invalid", receiptError=str(error))
        if runtime_exists and observed["applications"]:
            application = observed["applications"][0]
            result.update(application, status="running", source=None)
            if same_instance(receipt, application):
                result.update(home=receipt["home"], source=receipt["source"],
                              profile=receipt.get("profile"), sourceFingerprint=receipt.get("sourceFingerprint"),
                              receiptState="matched")
            elif receipt is not None:
                result["receiptState"] = "mismatch"
        elif receipt is not None:
            result.update(home=receipt["home"], source=receipt["source"],
                          sourceFingerprint=receipt.get("sourceFingerprint"), receiptState="stale")
        return result


def stop(workspace=ROOT, profile="dev"):
    workspace = workspace_path(workspace, profile)
    scope = common.identity(workspace, "development")["context"]["scope"]
    _, bundle, _ = paths(workspace)
    with lifecycle_lock(workspace):
        if not bundle.exists() and not bundle.is_symlink():
            source = common.app_bundle_path(workspace, profile)
            if source.exists() or source.is_symlink():
                context = validated_bundle(source, scope)
                refuse_conflicts(inspect(source, context), source_only=True)
            return dict(status="notRunning", purpose="development", scope=scope, stopped=True,
                        alreadyStopped=True, pid=None, launchedAt=None, bundlePath=str(bundle))
        context = validated_bundle(bundle, scope)
        observed = inspect(bundle, context)
        refuse_conflicts(observed)
        if not observed["applications"]:
            return dict(status="notRunning", purpose="development", scope=scope, stopped=True,
                        alreadyStopped=True, pid=None, launchedAt=None, bundlePath=str(bundle))
        application = observed["applications"][0]
        stopped = native("stop", bundle, "--pid", application["pid"], "--launched-at", repr(application["launchedAt"]))
        if (stopped.get("stopped") is not True or stopped.get("pid") != application["pid"]
                or stopped.get("launchedAt") != application["launchedAt"]):
            raise LauncherError("原生退出结果与本次新鲜检查的实例不一致")
        return dict(stopped, status="notRunning", purpose="development", scope=scope, bundlePath=str(bundle))


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    commands = parser.add_subparsers(dest="command", required=True)
    for name in ("start", "status", "stop"):
        command = commands.add_parser(name)
        command.add_argument("--workspace", type=Path, default=ROOT)
        command.add_argument("--profile", choices=("dev", "release"), default="dev")
        if name == "start":
            command.add_argument("--app", type=Path)
            command.add_argument("--home", type=Path)
    args = vars(parser.parse_args())
    command = args.pop("command")
    print(json.dumps(globals()[command](**args), ensure_ascii=False, sort_keys=True))


if __name__ == "__main__":
    try:
        main()
    except (OSError, ValueError, LauncherError) as error:
        print(str(error), file=sys.stderr)
        sys.exit(1)
