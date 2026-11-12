"""Build the committed Velune revision on YYH-WS and update its per-user install."""
import base64
from pathlib import Path
import re
import subprocess
import tempfile
import zipfile
from uuid import uuid4


ROOT = Path(__file__).resolve().parent.parent
HOST = "win-ws.localhost"
SSH_OPTIONS = ["-o", "BatchMode=yes", "-o", "ConnectTimeout=5"]
BOOTSTRAP = r"""
$ErrorActionPreference = 'Stop'
$ProgressPreference = 'SilentlyContinue'
if ($env:COMPUTERNAME -ne 'YYH-WS') { throw 'Expected YYH-WS.' }
$zip = Join-Path $env:USERPROFILE 'UPLOAD_NAME'
$transfer = Join-Path $env:TEMP 'UPLOAD_NAME'
New-Item -ItemType Directory -Path $transfer | Out-Null
try {
    Add-Type -AssemblyName System.IO.Compression.FileSystem
    [IO.Compression.ZipFile]::ExtractToDirectory($zip, $transfer)
    & (Join-Path $transfer 'reinstall-yyh-win.ps1') -Revision 'SOURCE_COMMIT'
} finally {
    Remove-Item -LiteralPath $transfer -Recurse -Force
    Remove-Item -LiteralPath $zip -Force
}
"""


def main():
    revision = subprocess.check_output(["git", "rev-parse", "HEAD"], cwd=ROOT, text=True).strip()
    if not re.fullmatch(r"[0-9a-f]{40}", revision):
        raise SystemExit("无法确认当前提交。")
    hostname = subprocess.check_output(["ssh", "-T", *SSH_OPTIONS, HOST, "hostname"], text=True, timeout=15)
    if hostname.strip().upper() != "YYH-WS":
        raise SystemExit(f"{HOST} 没有指向 YYH-WS。")
    print(f"YYH-WS 将从提交 {revision} 构建。", flush=True)
    with tempfile.TemporaryDirectory(prefix="velune-yyh-win-") as directory:
        directory = Path(directory)
        bundle = directory / "source.bundle"
        subprocess.run(["git", "bundle", "create", str(bundle), "HEAD"], cwd=ROOT, check=True)
        heads = subprocess.check_output(["git", "bundle", "list-heads", str(bundle), "HEAD"], text=True)
        if heads.strip() != f"{revision} HEAD":
            raise SystemExit("创建 bundle 时 HEAD 已变化，请重新执行。")
        transfer = directory / "transfer.zip"
        with zipfile.ZipFile(transfer, "w") as archive:
            archive.write(bundle, "source.bundle")
            archive.write(ROOT / "scripts/reinstall-yyh-win.ps1", "reinstall-yyh-win.ps1")
        upload = f"velune-update-{uuid4().hex}.zip"
        subprocess.run(["scp", "-q", *SSH_OPTIONS, str(transfer), f"{HOST}:{upload}"], check=True, timeout=120)
        command = BOOTSTRAP.replace("SOURCE_COMMIT", revision).replace("UPLOAD_NAME", upload)
        encoded = base64.b64encode(command.encode("utf-16le")).decode("ascii")
        subprocess.run([
            "ssh", "-nT", *SSH_OPTIONS, "-o", "ServerAliveInterval=15", "-o", "ServerAliveCountMax=3",
            HOST, "powershell.exe", "-NoProfile", "-NonInteractive",
            "-ExecutionPolicy", "Bypass", "-EncodedCommand", encoded,
        ], check=True)


if __name__ == "__main__":
    try:
        main()
    except subprocess.CalledProcessError as error:
        raise SystemExit(f"YYH-WS 更新失败（{error.cmd[0]} 退出码 {error.returncode}）。请查看上方输出。") from None
    except subprocess.TimeoutExpired as error:
        raise SystemExit(f"YYH-WS 更新超时（{error.cmd[0]}）。") from None
    except OSError as error:
        raise SystemExit(f"YYH-WS 更新失败。{error}") from None
