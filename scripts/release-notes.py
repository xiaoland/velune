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
- **Windows** 首版暂缓发布，现有依赖和项目许可保留，跨协议转换能力保留。安装器与更新代码已准备，依赖许可问题解决后再提供预构建版本。
- **Android** arm64 APK 沿用 Velune 的固定发行签名，可覆盖升级同签名的已发布版本。使用其他签名的调试安装应保留，等待数据转移入口。卸载会删除私有数据。
- **iPad** 在 Mac 应用“设置 → 设备 → 安装到 iPad…”中，使用自己的 Apple 签名团队一键构建、签名并安装。首次需准备 Xcode、Rust 工具链及对应版本源码，连接并信任 Mac、开启 iPad 开发者模式。选择源码、团队和设备后点击“安装”。

源码压缩包包含对应 tag 的 RelayKit vendored 源码。每个发行包配有 `.build.json`，记录源码提交、版本、构建号、架构、签名状态和 SHA-256；`SHA256SUMS.txt` 覆盖整个 Release 资产集。

## Installation notes

- **macOS**  the two archives contain separate arm64 and x86_64 apps. They are ad-hoc signed and not notarized. If macOS blocks the first launch, open **System Settings → Privacy & Security → Open Anyway** for Velune. [Apple installation guidance](https://support.apple.com/en-gb/102445).
- **Windows**  prebuilt distribution is deferred while dependency licensing is resolved. Existing dependencies, the project license and cross-protocol conversion remain in place. Installer and update code is prepared for a later release.
- **Android**  the arm64 APK uses Velune's existing release signing key and can update releases signed with the same key. Keep debug installations signed with a different key until a data transfer path is available. Uninstalling removes private data.
- **iPad**  in the Mac app, open **Settings → Devices → Install on iPad…** to build, sign with your own Apple team, and install in one action after setup. Prepare Xcode, Rust and the matching source version, connect and trust your Mac, and enable Developer Mode on the iPad. Select your source folder, team and device, then click **Install**.

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
