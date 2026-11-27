---
name: verify-velune-macos
description: 验证 Velune Mac 开发包中的历史消息编辑器或工作区侧栏。用于选择现有原生检查、验证构建产物归属、隔离运行并保留回执。
---

# 验证 Velune Mac 原生界面

运行前在仓库根目录阅读 [Mac Agent 指引](../../../app/mac/AGENTS.md)与[界面证据分层](../../../docs/development/validation.md#平台与界面)。本流程覆盖开发包的合成数据原生检查。真实运行时、真实会话、可见窗口、系统键盘和辅助功能验收各按对应场景执行。

## 准备与产物检查

首次准备按 [Mac 构建](../../../docs/development/platforms.md#mac-构建)初始化 runtime-support。xURL 的安装会触发编译，用 `agent-build bash scripts/install-xurl.sh`。用现有 SVC 入口构建并从同一工作树解析产物路径。

```sh
agent-build svc run build-macos
app=$(python3 scripts/macos_identity.py describe --profile dev |
  python3 -c 'import json, sys; print(json.load(sys.stdin)["bundlePath"])')
python3 scripts/macos_identity.py validate --purpose development "$app"
codesign --verify --strict "$app"
```

复用已有产物时，对照 `target/build-state/results/macos-dev-development-success.json` 的退出码、`current` 与二进制摘要，以及 bundle 中 `Contents/Resources/build-manifest.json` 的源码身份。若证据只对应另一次构建，报告其实际身份。运行器会复制并重新签名本次临时开发包，负责启动和回收。

## 选择检查

| 改动涉及 | 现有检查 | 核对的结果 |
| --- | --- | --- |
| 历史消息编辑器 | `editor` | 离屏原生编辑器中编辑正文与附件，核对独立输入、过期提交拒绝和原会话寻址。日志须有 `PASS offscreen native editor`。 |
| 工作区侧栏及工具栏 | `sidebar-toolbar` | 使用 AppKit 工作区外壳，验证侧栏按钮归属、收起与重开、宽度恢复。日志须有 `PASS sidebar toolbar`。 |

例如运行历史编辑检查。

```sh
mkdir -p target/verification
out=$(mktemp -d "$PWD/target/verification/macos-editor.XXXXXX")
python3 scripts/check-macos-composer.py editor --app "$app" --output "$out"
test -s "$out/receipt.json" && test -s "$out/native.log"
```

工作区侧栏用 `sidebar-toolbar` 并保留独立输出目录。更多原生入口由 [验证说明](../../../docs/development/validation.md#平台与界面)与运行器的 `--help` 维护。涉及工作区冷打开、恢复或标签命令时，使用现有 `experience --workspace-only` 确认对应路径实际走完。

## 证据与收尾

同时核对退出码、目标场景的通过行、`receipt.json` 中的产物摘要及导出文件。报告实际经过的是离屏渲染、本地控件调用、系统事件或 AX 查询。需要判断外观时打开当次截图。出现中途失败时记录最后通过的步骤和首个失败步骤。

运行器退出时通过自己的进程句柄结束本次实例并清理私有目录。在其退出后确认结果目录仍有回执和日志。普通 GUI 另由 `svc run start-macos-dev`、`status-macos-dev` 与 `stop-macos-dev` 管理。
