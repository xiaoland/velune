#!/usr/bin/env python3
"""Append idempotent platform signing and installation notes to release notes."""

from __future__ import annotations

import argparse
from pathlib import Path
import sys


START = "<!-- velune-installation-notes:start -->"
END = "<!-- velune-installation-notes:end -->"
NOTES = f"""{START}
## 安装说明

- **macOS** 两个压缩包分别提供 arm64 和 x86_64 应用。应用使用 ad-hoc 签名，尚未公证。若 macOS 首次启动时拦截，请在“系统设置 → 隐私与安全性”中为 Velune 选择“仍要打开”。[Apple 安装说明](https://support.apple.com/en-gb/102445)。
- **Windows** 解压 x64 压缩包并运行 `Velune\\Velune.exe`。文件为自包含版本，但没有 Authenticode 签名，Windows 可能显示未知发布者或 SmartScreen 警告。
- **Android** arm64 APK 使用 Velune 固定发行密钥签名。已安装的 pre-beta/debug APK 使用不同签名，Android 不允许直接覆盖升级。如果需要保留其私有数据，请勿卸载；卸载会删除数据，目前也没有跨签名导出／导入途径。数据转移途径落实前，只能在不含重要旧数据的设备上使用此 beta。

源码压缩包包含对应 tag 的 RelayKit vendored 源码。每个发行包配有 `.build.json`，记录源码提交、版本、构建号、架构、签名状态和 SHA-256；`SHA256SUMS.txt` 覆盖整个 Release 资产集。

## Installation notes

- **macOS**  the two archives contain separate arm64 and x86_64 apps. They are ad-hoc signed and not notarized. If macOS blocks the first launch, open **System Settings → Privacy & Security → Open Anyway** for Velune. [Apple installation guidance](https://support.apple.com/en-gb/102445).
- **Windows**  unzip the x64 archive and run `Velune\\Velune.exe`. The files are self-contained but have no Authenticode signature, so Windows may show an unknown-publisher or SmartScreen warning.
- **Android**  the arm64 APK uses Velune's fixed release signing key. An already installed pre-beta/debug APK has a different key and Android will reject an in-place update. Do not uninstall it if its private data matters: uninstalling removes that data, and there is no cross-signature export/import path yet. Use the beta only on a device without needed data until a supported transfer path exists.

The source archive contains vendored RelayKit source for the corresponding tag. Each release package has a `.build.json` file with its source commit, version, build number, architecture, signing state, and SHA-256. `SHA256SUMS.txt` covers the complete release asset set.
{END}"""


def main() -> int:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--input", required=True, help="current release notes")
    parser.add_argument("--output", required=True)
    parser.add_argument("--tag", required=True)
    args = parser.parse_args()
    body = Path(args.input).read_text()
    start = body.find(START)
    if start >= 0:
        end = body.find(END, start)
        if end < 0:
            print("existing installation notes block is not closed", file=sys.stderr)
            return 2
        body = body[:start].rstrip() + body[end + len(END):]
    source = f"velune-{args.tag}-source.tar.gz"
    source_url = f"https://github.com/xiaoland/velune/releases/download/{args.tag}/{source}"
    notes = NOTES.replace(
        "源码压缩包包含对应 tag 的 RelayKit vendored 源码。",
        f"源码压缩包 [下载 {source}，含对应 tag 的 RelayKit vendored 源码]({source_url})。",
    ).replace(
        "The source archive contains vendored RelayKit source for the corresponding tag.",
        f"The [source archive]({source_url}) contains vendored RelayKit source for the corresponding tag.",
    )
    Path(args.output).write_text(body.rstrip() + "\n\n" + notes + "\n")
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
