# 远程控制 MVP：Windows、Android 与 Mac 接入

## 当前任务

2026-10-19 用户目标：实现远程控制系列能力的 MVP。

- Windows 端基于 MyGO 开发，可在 `win-ws.localhost` 验收。
- Android 端保持既有技术栈（Kotlin／Jetpack Compose），彻底重写；现有实现只是远控基础能力的测试原型。
- Android 不是对等端（将来的 iPad、Web 同理）：只有控制能力，不运行 agent runtime。任何一端都能控制其他端。
- SSH agent-runtime 与远程控制是两项不同能力。

2026-10-19 方案核对后用户授权实施，决定见 [PRD](../../docs/prd/index.md#后续扩展分布式能力) 与 [S43](../../docs/sources.md#s43)。授权范围记于 AGENTS 的“独立 units 与跨语言接入”。

## 已核实事实（2026-10-19，main `1bc5a11`）

- Rust 远控库已在 main 中（`85e7f89` 合入），包括 `remote`、`application-api` 和 bindings 的 `remote` feature。它提供设备身份、方向配对（八位核对码）、联系人、入站授权、iroh 连接与恢复、未决请求账本，以及 `application_for_device` 返回的完整业务代理。契约见 [remote-access](../../docs/design/remote-access.md)；库任务见 [android-minimal](../android-minimal/packet.md)。
- 平台尚未接入新契约。main 不包含 Android 工程或 Mac 远控页；Android 原型只在历史提交 `2b874b3:app/android` 中，约 1.1k 行 Kotlin，配对需粘贴邀请，没有扫码功能。
- bindings 导出约 240 个具名类型和 100 多个方法，全部是同步调用；观察通过 `next(timeout)` 拉取，没有 async 导出或 callback interface。UniFFI 固定为 `=0.32.2`。
- MyGO（[egoist/mygo](https://github.com/egoist/mygo)，站点显示 v0.5.5，MIT 许可）是纯 Go、无 cgo 的桌面框架。原生 UI 由框架在 GPU 上自绘，不使用 Win32／WinUI 控件；另有 webview 模式。
- 唯一的 Go UniFFI 生成器 [uniffi-bindgen-go](https://github.com/NordSecurity/uniffi-bindgen-go) 只支持到 uniffi 0.31，并依赖 cgo；0.32 支持仍是开放 issue。
- 核心中约 108 处 Unix 专属代码（进程组、权限、fs 等），Windows 专属分支只有 1 处。Windows 要作为执行端，必须先移植。
- `win-ws.localhost`（重试后 SSH 可用）：Windows 10 22H2 x64、Ryzen 5 3600、32 GB、RTX 2070 SUPER；已装 VS Build Tools 2022、Git、Node（nvm4w）、Python 3.14、winget、Codex（原生 exe）、Claude Code 与 Pi（pnpm 全局）；未装 Rust 与 Go。SSH 默认 shell 为 cmd，脚本经 `powershell -Command -` 从 stdin 传入可避免转义问题。
- Android 真机 JKM-AL00a，Android 9，arm64-v8a，1080×2340；本机 JDK 17、SDK、NDK 30.0.14904198，Rust 已装 aarch64-linux-android 与 x86_64-pc-windows-gnu target。
- GPUI：Zed Windows 版已稳定（DirectX 11＋DirectWrite）；Longbridge `gpui-kit`／`gpui-component` 0.7.x 在 crates.io 提供组件集。GPUI 与核心同为 Rust，可同进程直接链接 bindings 的 Rust API，无需 FFI 生成器。

## 方案（用户已认可主体，Windows 框架待 spike）

1. **端角色**：Mac 与 Windows 都是完整端，持有 ApplicationService 和 Harness，也能控制其他端。Android 是 controller-only 装配（remote-only bindings）。所有端使用同一套设备列表和配对流程。
2. **同一界面，切换目标设备**：本机与远端都通过同一个 `VeluneApplication` API 访问。会话、转录和 composer 界面只编写一份，由设备切换决定目标。
3. **Windows Go↔Rust 桥**：推荐自建 UniFFI Go 生成器，基于 purego 运行时加载 DLL、不使用 cgo。只需覆盖当前 ABI 实际用到的同步函数、record、enum、object、error 和集合类型。这样符合 MyGO 的无 cgo 构建，也能从 Mac 交叉编译。备选方案是 fork uniffi-bindgen-go 以支持 0.32，并改用 cgo 和 MinGW。
4. **Windows 执行端移植**：进程树改用 Job Object；补齐路径、权限与凭据存储；三个 Harness 原生运行于 Windows（不经 WSL 或 SSH runtime）。
5. **Mac 接入**：作为执行端时提供接入开关、邀请二维码、核对码批准和撤销；作为控制端时提供设备切换。
6. **Android 重写**：采用单 Activity Compose 加 Navigation，每个页面一个 ViewModel，由仓库层包装 UniFFI。提供扫码配对（CameraX＋ZXing）、设备列表、会话列表与转录、composer、停止和审批，并适配平板布局。

## 用户决定（2026-10-19）

- 认可“同一界面，切换目标设备”。Windows MVP 必须在本机运行 agents。
- Windows UI：MyGO 看来不太合适，可评估 GPUI 等替代，也可先试 MyGO；两者的自绘 UI 都作为原生视觉原则的例外接受。若用 Go，则坚持 purego。
- Mac 同时作为被控端与控制端。MVP 先做会话日常闭环；提供商、资源等配置要跨端同步，同步方案另行收敛后再与用户核对。
- 使用 iroh 公共 relay。remote 库可继续修改或拆分。可完全控制已连接的 Android 手机与 win-ws。

## Windows UI 框架 spike（2026-10-19）

两份样例实现同一场景：10k 行会话列表、800 条可变高度 Markdown 转录（含代码块、表格、中英混排）、多行输入与发送。均在 win-ws 的 RDP 交互会话中运行并截图。

| | GPUI（gpui-kit 0.7.3，Rust 1.99.0 MSVC） | MyGO v0.5.5（Go 1.26，CGO_ENABLED=0） |
| --- | --- | --- |
| 构建 | Windows 本机 release 首次 4 分 03 秒，exe 23.8 MB | Mac 交叉编译 8 秒，exe 15.3 MB |
| 运行内存（工作集） | 约 64 MB | 约 295 MB |
| 渲染 | 中英文、Markdown 标题／列表／代码块／表格清晰，标准 Windows 标题栏按钮 | Markdown 质量好（含语法高亮与 Copy），但按示例写法的侧栏与输入区未显示，FollowEnd 未停在末尾；可能是样例用法问题，未深究 |
| 接入核心 | 同语言，直接链接 bindings 的 Rust API | 需自建 purego UniFFI 生成器，覆盖约 240 个类型 |

结论：选 GPUI。决定性因素是同语言接入，不需要 FFI 生成器；内存也明显更低。purego 约束随之不再适用。中文输入法尚未验证，样例保留在 win-ws 的 `velune-spikes\gpui-spike\target\release\gpui-spike.exe`，可请用户试用。代价是 GPUI API 变化较快，需要固定 gpui-kit 版本，且 Windows 本机构建较慢。

## Windows 核心移植证据（2026-10-19）

- 工作区在 win-ws 的 MSVC 目标上通过 `cargo clippy --locked --workspace --all-targets --all-features -- -D warnings`。RelayKit 改为 llvm-mingw 构建的 DLL，以 raw-dylib 绑定；子进程经 `process_tree`（Job Object、无控制台窗口）创建；Python 辅助程序在本地 Windows 目标上使用 `python`。
- 隔离冒烟（临时 HOME、回环合成上游、Python UniFFI 绑定、不调用真实服务）：Codex 0.159.3（原生 exe，Responses）、Claude Code 2.1.286（原生 exe，Messages）、Pi 1.0.2（node＋cli.js，ChatCompletions）均完成创建、发送、经网关收到合成回答、列表出现与重新打开历史；关闭后不残留 codex.exe。脚本在 job 临时目录，未入库。
- 测试用 Harness 安装在 win-ws 的 `velune-spikes\harness`，用 npm overrides 将 Pi 的同族包固定为 1.0.2；不固定时 `^1.0.2` 会解析到 1.3.0，导致 Pi 启动失败。
- 用户在 win-ws 全局安装的版本不在支持区间：Codex 0.155.0（需 0.159.x）、Claude Code 2.0.76（需 2.1.x）。实际使用前需要升级，或在应用中指向受支持的安装。
- 尚未验证：轮次进行中停止时，Job Object 是否连带结束 node 孙进程；SSH 运行时（依赖 ControlMaster），Windows 宿主不支持；凭据助手（`credential_helper`）在非 Unix 上仍返回 Unavailable，因此 Pi 来源型 OAuth 解析在 Windows 不可用。

## 执行次序

1. Windows 框架 spike（GPUI 与 MyGO 各做一个最小样例：窗口、虚拟列表、多行输入与中文输入法、文本渲染），比较后定案。
2. 核心 Windows 移植，与 Mac 接入并行。
3. Android 重写，以 Mac 为执行端完成真机闭环。
4. Windows 应用。
5. 配置跨端同步方案，与用户核对后实施。
