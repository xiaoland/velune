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
- Job Object 硬退出：Python 宿主在 Pi 回合进行中 `os._exit`，Pi 的 node.exe 随之结束（临时脚本，未入库）。尚未用真实工具调用验证更深一层的孙进程。
- 凭据助手（`credential_helper`）已改为 Unix 进程组与 Windows Job Object 共用同一套超时、输出上限和进程树清理，Windows 上不再直接返回 Unavailable。目前只经静态检查：运行验证需要真实的 Pi 来源认证，开发方不读取。
- SSH 运行时依赖 ControlMaster，Windows 宿主不支持。

## Windows 应用证据（2026-10-19）

- `app/windows`（GPUI，gpui-kit `=0.7.3`）直接链接 bindings，入口见 [Windows README](../../app/windows/README.md)。`scripts/build-windows.ps1` 在 win-ws 上完成 release 构建，组装出可移动的应用目录。
- 在 win-ws 的交互会话中以隔离 `VELUNE_HOME` 运行，用临时脚本模拟点击（未入库），已验证：启动与设置窗口；远控服务就绪，接入开关打开；运行时发现列出用户全局安装的 pi 0.80.3、codex 0.155.0、claude 2.0.76，并标为不受支持；手动添加 Pi 1.0.2 运行时；添加合成回环提供商（Chat Completions，`127.0.0.1:18080`，非真实 key）。
- 发现 gpui-component 对话框内的输入框在 Windows 上收不到点击，点击落到遮罩上、对话框随之关闭。新会话与重命名因此改为就地编辑。
- 首次冷启动时，Pi 的版本探测超过 15 秒超时，疑为 Defender 扫描；重试成功。
- 事故：一次界面脚本运行时，Velune 窗口不存在，而用户正在 win-ws 上使用 Codex 应用，脚本按绝对坐标点击并输入，把测试路径输入了用户 Codex 的输入框（未发送）。此前的粘贴步骤也两次覆盖了用户剪贴板。之后脚本增加两道保护：用户空闲不足 10 分钟即中止，目标窗口不在前台时不发送任何输入。
- 事故后的界面脚本（2026-10-19 晚）只再启动了一次应用，主窗口与“新会话”入口正常。脚本自身的输入会重置 Windows 的空闲计时，保护随之阻止后续运行；放宽保护的修改被权限审查拒绝，开发方于是停止在 win-ws 上自动操作界面，并关闭了测试实例。
- 待用户验收（win-ws 上 `target\windows\Velune\velune.exe`）：发送消息并完成回合、审批、停止、就地重命名、二维码显示，与 Android／Mac 互相配对和批准对话框，以及中文输入法。

## Mac 接入证据（2026-10-19）

- 设置新增“设备”页：接入开关、设备名、邀请二维码（含复制与到期）、入站授权及撤销；“控制其他设备”列出联系人状态、打开与移除，并可粘贴邀请发起配对。设备窗口沿用同一套工作区界面，`Transport` 改为通过远程代理 `VeluneApplication` 访问对端；工具栏显示目标设备名。
- 用后台开发副本（临时 `VELUNE_HOME`）配合合成 Python 完整端（ACP fixture，iroh 公共 relay），两个方向都已验证，均使用临时 hook 驱动且未入库。Mac 作为控制端：配对时双方核对码一致，对端批准后，设备窗口显示对端的会话列表、转写与运行时。Mac 作为被控端：批准弹窗显示请求方名称和分组核对码，人工允许后对端的 `list`／`upsert_runtime` 生效；撤销授权、生成邀请均在界面中验证。
- 验证中修正了四处：
  - 批准弹窗的“允许”不再响应 Return，Esc 对应“拒绝”。
  - 发起方原本收不到对方的批准（核心需要显式调用 `pairing_status`）。现在由 remote 库在 `begin_pairing` 之后及服务打开时跟进未确认的事务，经 `observe_state` 送达；回环脚本测得批准后约 2 秒确认，Android 与 Windows 直接受益。
  - 启动时恢复的设备窗口会先等待服务就绪和确认完成，再连接。
  - 邀请输入框的宽度改为正常显示。
- 待用户验收：真实两台设备之间的配对；在设备窗口中发送消息；批准弹窗的位置（应用位于后台时，它只在激活应用后才可见，Dock 会提醒）。

## Android 证据（2026-10-19）

- 按新契约重写 `app/android`：保留构建脚手架、`NativePlatform` 初始化和 `NetworkMonitor`，其余全部新写。界面包括设备列表、扫码或粘贴配对与核对码、按设备切换的会话列表、新建会话（选择运行时与项目）、转写、审批与提问、模型选择、发送和停止。只读写 `noBackupFilesDir/remote`。
- 在 JKM-AL00a（Android 9）上，以合成 Python 完整端（ACP fixture、iroh 公共 relay，同一 LAN）走真实 App 验收。邀请经临时 hook 写入应用私有目录，hook 未入库。已验证：配对（核对码 9992 8401 双方一致，批准后由 Rust 跟进确认、自动进入设备）、已连接状态、会话列表、转写左右布局、发送、审批卡“Allow”后回合完成、新建 Beta 会话并发送、停止请求被受理（运行状态离开 Running）。由于测试中放开了 fixture，这次没有区分停止后的终态。
- 修正 remote 库缺陷：`set_controller_activity` 可能启动恢复任务，但未进入 Tokio runtime，于是持锁时 `tokio::spawn` panic，锁被毒化，此后每次 `connect` 都返回 Unavailable。Mac 先 connect 后报告活动，所以没有触发；Android 先选设备才触发。bindings 现在调用前进入 runtime。
- 未验证：扫码（手机相机对不准 Mac 屏幕，只确认了相机预览启动）、跨网 NAT、后台恢复和长时间运行。
- 手机上仍留有旧原型的身份文件（`no_backup/remote/identity`、`trust.json`、`files/s1-probe`），新 App 不读取它们，清除应用数据即可删除。

## 配置跨端同步方案（候选，待用户核对）

依据：配置由 `application` 持有，见 `repository.rs` 中的 `PersistedConfig`、`resource-templates.json` 与 `resource-bindings.json`。只有完整端（Mac、Windows）持有配置；Android 只是控制端，直接读写被控端的配置，不参与同步。

**同步范围**

| 对象 | 处理 | 理由 |
|---|---|---|
| 网关与 AI 提供商：协议、地址、模型条目、额度、请求配置、路由 | 同步，以提供商为单位，路由随所属网关 | 用户明确要求 |
| API key | 建议同步（经 iroh TLS 传输，各端写入自己的私有文件） | 不同步时，每台设备仍需手填，同步就失去意义 |
| 设备绑定的认证（Codex／Claude 订阅登录、Harness 来源认证） | 不同步。条目同步过去后标为“需在本机登录” | 凭据属于本机 Harness 账户，复制凭据违反认证边界 |
| 模型参数模板 | 同步 | 与设备无关 |
| 资源模板（skills、MCP、指令），含其中的秘密 | 同步 | 用户明确要求 |
| 资源应用记录（`resource-bindings`）、Agent 运行时实例 | 不同步 | 指向本机的路径、可执行文件与 Harness 目录 |
| 界面偏好、会话整理、草稿 | 不同步 | 随设备和本机会话而变；会话本身经远控访问 |

**机制**：每个同步对象带 `(逻辑时钟, 设备 ID)` 修订号；删除留墓碑。两台完整端连上后交换修订摘要，只传较新的对象；本地修改后推送给在线的同步对端。同一对象被并发修改时，按修订号以“后写者胜”裁决，被覆盖的版本保留一份并提示用户。同步复用现有远控授权：能控制对方，本来就能读写它的全部配置，同步并不扩大权限。传输走 remote 协议中新增的配置同步请求，不另起服务。

**待用户决定**

1. 范围：上表是否正确，界面偏好是否也要同步？
2. API key 与资源模板中的秘密是否随同步传输（建议是）？
3. 同步关系：凡已配对的完整端自动同步，还是对每台设备单独开启“同步配置”（建议单独开启，并要求本端对对方有控制授权）？
4. 冲突：采用“后写者胜，保留被覆盖版本并提示”，还是每次冲突都让用户选择？

## 执行次序

1. Windows 框架 spike（GPUI 与 MyGO 各做一个最小样例：窗口、虚拟列表、多行输入与中文输入法、文本渲染），比较后定案。
2. 核心 Windows 移植，与 Mac 接入并行。
3. Android 重写，以 Mac 为执行端完成真机闭环。
4. Windows 应用。
5. 配置跨端同步方案，与用户核对后实施。
