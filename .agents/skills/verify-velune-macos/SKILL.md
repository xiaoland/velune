---
name: verify-velune-macos
description: 运行 Velune Mac 开发包的历史编辑、工作区恢复、侧栏或消息列表原生检查，核对构建输入并保留可复查的证据。
---

# 验证 Velune Mac 原生界面

从仓库根目录执行。用[场景索引](features/README.md)选择经过受影响路径的检查。[Mac Agent 指引](../../../app/mac/AGENTS.md)规定焦点、可见窗口与进程边界，[界面证据分层](../../../docs/development/validation.md#平台与界面)规定可以据此得出的结论。涉及系统键盘、前台切换或真实运行时的改动，按对应场景补验。

## 构建与诊断

首次构建按 [Mac 构建](../../../docs/development/platforms.md#mac-构建)准备 runtime-support。xURL 安装会触发编译，使用 `agent-build bash scripts/install-xurl.sh`。

```sh
agent-build svc run build-macos
mkdir -p target/verification
out=$(mktemp -d "$PWD/target/verification/macos-native.XXXXXX")
python3 scripts/build-support.py verify-macos-build > "$out/build.json"
app=$(python3 -c 'import json, sys; print(json.load(open(sys.argv[1]))["app"])' "$out/build.json")
```

复用开发包时从创建输出目录和诊断开始。诊断检查签名、工作树 scope、当前源码输入及实际 bundle 的摘要。失败时重新请求构建。构建时显式设置的编译参数须在同一环境下诊断。

## 启动与操作

每条场景命令会启动本次开发包的后台副本。运行器创建空的私有状态和 Harness 目录，准备该场景的合成数据，再驱动真实 AppKit 控件或工作区外壳。检查成功和场景指定的通过行一起证明启动抵达可用状态。举例，验证历史编辑器时执行：

```sh
python3 scripts/check-macos-composer.py editor --app "$app" --output "$out"
```

## 证据和收尾

核对退出码、`native.log` 的场景通过行和导出文件。`receipt.json` 的 `executableSHA256`、`bindingsSHA256` 须与 `build.json` 一致。保留首次失败及其前一条通过记录。报告检查实际经过的是离屏渲染、本地控件调用、系统事件还是 AppKit 辅助功能。判断外观时查看当次导出的图片。

运行器通过自己持有的子进程句柄结束实例，并删除私有状态与临时 App。退出后确认 `build.json`、`receipt.json` 和 `native.log` 仍在输出目录。每条场景单独创建目录。普通开发 GUI 使用 `svc run start-macos-dev`、`status-macos-dev` 与 `stop-macos-dev`。
