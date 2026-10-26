#!/usr/bin/env python3
"""Exercise the complete iPad installation flow against isolated tool stubs."""
from pathlib import Path
import json
import os
import plistlib
import subprocess
import sys
import tempfile

root = Path(__file__).resolve().parents[1]
core_id = "01234567-89AB-CDEF-0123-456789ABCDEF"
udid = "00008110-0012345678901234"
team_id = "ABCDEFGHIJ"

with tempfile.TemporaryDirectory(prefix="velune-ipad-install-test-") as directory:
    base = Path(directory)
    repo = base / "repo"
    tools = base / "tools"
    tools.mkdir()
    (repo / "app/ios/Velune.xcodeproj").mkdir(parents=True)
    scripts = repo / "scripts"
    scripts.mkdir()
    build_core = scripts / "build-ios.sh"
    build_core.write_text("""#!/bin/sh
set -e
echo '{"tool":"rust"}' >> "$STUB_LOG"
test "$RUST_FAIL" != yes
"$STUB_XCODEBUILD" -destination "id=$VELUNE_IOS_DEVICE_ID" -derivedDataPath "$VELUNE_IOS_DERIVED_DATA" -allowProvisioningDeviceRegistration
""")
    build_core.chmod(0o755)

    def stub(name, body):
        path = tools / name
        path.write_text(f"#!{sys.executable}\n" + body)
        path.chmod(0o755)
        return path

    xcrun = stub("xcrun", r'''import json, os, pathlib, sys
args = sys.argv[1:]
def event(tool):
 with open(os.environ["STUB_LOG"], "a") as f: f.write(json.dumps({"tool":tool,"args":args})+"\n")
def output_path(): return pathlib.Path(args[args.index("--json-output") + 1])
if args[:3] == ["devicectl", "list", "devices"]:
 event("devices")
 output_path().write_text(json.dumps({"info":{"commandType":"devicectl.list.devices","outcome":"success"},"result":{"devices":[{"identifier":"01234567-89AB-CDEF-0123-456789ABCDEF","hardwareProperties":{"platform":"iOS","deviceType":"iPad","udid":"00008110-0012345678901234"},"connectionProperties":{"pairingState":"paired","transportType":"localNetwork","tunnelState":"disconnected"},"deviceProperties":{"name":"Test iPad"}}]}}))
elif args[:4] == ["devicectl", "device", "install", "app"]:
 event("install")
 mode = os.environ.get("INSTALL_MODE", "success")
 if mode == "fail":
  sys.stdout.write("o" * 2000000); sys.stderr.write("e" * 2000000); sys.exit(9)
 outcome = "failure" if mode == "bad-result" else "success"
 output_path().write_text(json.dumps({"info":{"commandType":"devicectl.device.install.app","outcome":outcome},"result":{"deviceIdentifier":"01234567-89AB-CDEF-0123-456789ABCDEF","installedApplications":[{"bundleID":"app.velune.ios"}]}}))
else:
 raise SystemExit("unexpected xcrun arguments")
''')

    xcodebuild = stub("xcodebuild", r'''import json, os, pathlib, plistlib, sys
args = sys.argv[1:]
with open(os.environ["STUB_LOG"], "a") as f: f.write(json.dumps({"tool":"xcodebuild","args":args})+"\n")
if os.environ.get("XCODE_MODE") == "fail":
 sys.stdout.write("x" * 2000000); sys.stderr.write("y" * 2000000); sys.exit(8)
derived = pathlib.Path(args[args.index("-derivedDataPath") + 1])
app = derived / "Build/Products/Debug-iphoneos/Velune.app"
app.mkdir(parents=True)
(app / "Velune").write_text("stub executable")
with (app / "Info.plist").open("wb") as f: plistlib.dump({"CFBundleIdentifier":"app.velune.ios"}, f)
(app / "embedded.mobileprovision").write_bytes(b"stub profile")
''')

    codesign = stub("codesign", r'''import json, os, sys
args = sys.argv[1:]
with open(os.environ["STUB_LOG"], "a") as f: f.write(json.dumps({"tool":"codesign","args":args})+"\n")
if os.environ.get("SIGN_MODE") == "fail" and args[0] == "--verify": sys.exit(6)
if args[0] == "--display": sys.stderr.write("TeamIdentifier=ABCDEFGHIJ\n")
''')

    security = stub("security", r'''import datetime, os, plistlib, sys
profile = {"TeamIdentifier":["ABCDEFGHIJ"],"ProvisionedDevices":["00008110-0012345678901234"],"Entitlements":{"application-identifier":"ABCDEFGHIJ.app.velune.ios"},"ExpirationDate":datetime.datetime.now(datetime.timezone.utc).replace(tzinfo=None)+datetime.timedelta(days=3)}
sys.stdout.buffer.write(plistlib.dumps(profile, fmt=plistlib.FMT_XML))
''')

    installer = (root / "app/mac/IPadInstaller.swift").read_text()
    for original, replacement in {
        "/usr/bin/xcrun": str(xcrun),
        "/usr/bin/xcodebuild": str(xcodebuild),
        "/usr/bin/codesign": str(codesign),
        "/usr/bin/security": str(security),
    }.items():
        installer = installer.replace(f'"{original}"', f'"{replacement}"')

    harness = r'''import Foundation
@main struct Check {
 static func main() {
  let shouldSucceed = CommandLine.arguments[1] == "success"
  do {
   try IPadInstaller.install(repository: URL(fileURLWithPath: CommandLine.arguments[2]), deviceID: "01234567-89AB-CDEF-0123-456789ABCDEF", teamID: "ABCDEFGHIJ")
   guard shouldSucceed else { fputs("unexpected success\n", stderr); exit(1) }
   print("success")
  } catch {
   guard !shouldSucceed else { fputs("unexpected failure\n", stderr); exit(1) }
   print("failure")
  }
 }
}
'''
    swift_source = base / "IPadInstaller.swift"
    swift_source.write_text(installer)
    main_source = base / "main.swift"
    main_source.write_text(harness)
    executable = base / "check"
    subprocess.run(["swiftc", "-parse-as-library", str(swift_source), str(main_source), "-o", str(executable)], check=True)

    def run_case(name, expected, **modes):
        log = base / f"{name}.jsonl"
        env = {**os.environ, "STUB_LOG": str(log), "RUST_FAIL": "no", "STUB_XCODEBUILD": str(xcodebuild), **modes}
        result = subprocess.run([str(executable), expected, str(repo)], env=env,
                                capture_output=True, text=True, timeout=30, check=True)
        assert result.stdout.strip() == expected, result.stderr
        return [json.loads(line) for line in log.read_text().splitlines()]

    rust = run_case("rust-fail", "failure", RUST_FAIL="yes")
    assert [event["tool"] for event in rust] == ["devices", "rust"]

    xcode = run_case("xcode-fail", "failure", XCODE_MODE="fail")
    assert [event["tool"] for event in xcode] == ["devices", "rust", "xcodebuild"]

    signing = run_case("sign-fail", "failure", SIGN_MODE="fail")
    assert [event["tool"] for event in signing if event["tool"] in {"install", "codesign"}] == ["codesign"]

    success = run_case("success", "success")
    build = next(event for event in success if event["tool"] == "xcodebuild")["args"]
    install = next(event for event in success if event["tool"] == "install")["args"]
    assert "-allowProvisioningDeviceRegistration" in build
    assert build[build.index("-destination") + 1] == f"id={udid}"
    assert install[install.index("--device") + 1] == core_id
    assert not any("uninstall" in event.get("args", []) or "launch" in event.get("args", []) for event in success)

    install_failure = run_case("install-fail", "failure", INSTALL_MODE="fail")
    assert any(event["tool"] == "install" for event in install_failure)

    bad_result = run_case("install-bad-result", "failure", INSTALL_MODE="bad-result")
    assert any(event["tool"] == "install" for event in bad_result)

print("iPad 安装器执行链检查通过：Rust/Xcode/签名失败阻断安装，UDID 与 CoreDevice UUID 正确传递，安装失败和错误结果不报告成功，大输出无 Pipe 阻塞。")
