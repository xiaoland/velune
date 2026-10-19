# 远程控制 MVP：Windows、Android 与 Mac 接入

## 当前任务

2026-10-19 用户目标：实现远程控制系列能力的 MVP。

- Windows 端基于 MyGO 开发，可在 `win-ws.localhost` 验收。
- Android 端保持既有技术栈（Kotlin／Jetpack Compose），彻底重写；现有实现只是远控基础能力的测试原型。
- Android 不是对等端（将来的 iPad、Web 同理）：只有控制能力，不运行 agent runtime。任何一端都能控制其他端。
- SSH agent-runtime 与远程控制是两项不同能力。

用户要求先收敛产品与技术方案并核对，然后再实施。本 packet 当前只记录事实、候选方案和待决问题，尚未获得实施授权。

## 已核实事实（2026-10-19，main `1bc5a11`）

- Rust 远控库已在 main 中（`85e7f89` 合入），包括 `remote`、`application-api` 和 bindings 的 `remote` feature。它提供设备身份、方向配对（八位核对码）、联系人、入站授权、iroh 连接与恢复、未决请求账本，以及 `application_for_device` 返回的完整业务代理。契约见 [remote-access](../../docs/design/remote-access.md)；库任务见 [android-minimal](../android-minimal/packet.md)。
- 平台尚未接入新契约。main 不包含 Android 工程或 Mac 远控页；Android 原型只在历史提交 `2b874b3:app/android` 中，约 1.1k 行 Kotlin，配对需粘贴邀请，没有扫码功能。
- bindings 导出约 240 个具名类型和 100 多个方法，全部是同步调用；观察通过 `next(timeout)` 拉取，没有 async 导出或 callback interface。UniFFI 固定为 `=0.32.2`。
- MyGO（[egoist/mygo](https://github.com/egoist/mygo)，站点显示 v0.5.5，MIT 许可）是纯 Go、无 cgo 的桌面框架。原生 UI 由框架在 GPU 上自绘，不使用 Win32／WinUI 控件；另有 webview 模式。
- 唯一的 Go UniFFI 生成器 [uniffi-bindgen-go](https://github.com/NordSecurity/uniffi-bindgen-go) 只支持到 uniffi 0.31，并依赖 cgo；0.32 支持仍是开放 issue。
- 核心中约 108 处 Unix 专属代码（进程组、权限、fs 等），Windows 专属分支只有 1 处。Windows 要作为执行端，必须先移植。
- `win-ws.localhost` 本次探测 SSH 22 端口时返回 No route to host，Windows 工具链尚未核实。

## 候选方案（待用户确认）

1. **端角色**：Mac 与 Windows 都是完整端，持有 ApplicationService 和 Harness，也能控制其他端。Android 是 controller-only 装配（remote-only bindings）。所有端使用同一套设备列表和配对流程。
2. **同一界面，切换目标设备**：本机与远端都通过同一个 `VeluneApplication` API 访问。会话、转录和 composer 界面只编写一份，由设备切换决定目标。
3. **Windows Go↔Rust 桥**：推荐自建 UniFFI Go 生成器，基于 purego 运行时加载 DLL、不使用 cgo。只需覆盖当前 ABI 实际用到的同步函数、record、enum、object、error 和集合类型。这样符合 MyGO 的无 cgo 构建，也能从 Mac 交叉编译。备选方案是 fork uniffi-bindgen-go 以支持 0.32，并改用 cgo 和 MinGW。
4. **Windows 执行端移植**：进程树改用 Job Object；补齐路径、权限与凭据存储；三个 Harness 原生运行于 Windows（不经 WSL 或 SSH runtime）。
5. **Mac 接入**：作为执行端时提供接入开关、邀请二维码、核对码批准和撤销；作为控制端时提供设备切换。
6. **Android 重写**：采用单 Activity Compose 加 Navigation，每个页面一个 ViewModel，由仓库层包装 UniFFI。提供扫码配对（CameraX＋ZXing）、设备列表、会话列表与转录、composer、停止和审批，并适配平板布局。

## 待用户决定

见本轮回复中的问题。用户作出决定后，先更新 PRD 和 remote-access，再更新本 packet。
