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
- 2026-10-19 用户建议分治：Agent 运行时的控制能力由 Rust lib 在 Windows 上直接验证，不经 GUI。临时 Rust probe 放在 win-ws 的仓库之外，以路径依赖直接驱动 `velune-agent-runtime`，未入库。
  - 单独构建暴露了打包缺陷：agent-runtime（及新的凭据助手）声明的 `windows-sys` features 不全，缺 `Win32_Security` 与 `Win32_System_Threading`。工作区构建能通过，只是因为 GPUI 的依赖启用了它们。已补齐，并在 win-ws 上逐包 `cargo check -p` 了 agent-runtime、application、remote 与 bindings。
  - Pi 1.0.2（隔离 agent dir，回环脚本上游；win-ws 上的 Pi 没有登录）：1.6 秒就绪，回合完成；bash 工具经 Git Bash 运行 ping 时停止，ping 结束、工具标为失败；同样情形下宿主硬退出，node、bash、ping 及 conhost 全部随 Job 结束；关闭后无残留。
  - Codex 0.159.3（用户已登录的 `~\.codex`，用户授权的真实额度，选最小的 gpt-5.6-luna、low）：模型目录；单词回复（约 1.6 万输入 token，用量与周额度回报正常）；read-only 下的命令审批（经 pwsh 建文件，允许后完成）；运行 ping 时停止，ping 结束；硬退出时 codex、node_repl、code-mode-host、pwsh、ping 全部结束；历史的列出与读取含 Windows 路径的 cwd 和逐轮设置。Codex 应用自带 0.163.0-alpha，0.159.3 较旧，不会前向迁移用户的状态库。测试线程随后按 scratch cwd 删除，用户其他线程未动。
  - 修正 Codex 适配器（各平台通用）：停止后，被中止命令的工具块原本一直显示运行中，因为 Codex 不为它发送 `item/completed`；现在轮次以 interrupted／failed 结束时标为失败。win-ws 复验通过。
  - 观察：Codex 的新线程在首轮之前没有 rollout，此时更换原生模型会触发重启和 resume，而 resume 失败。application 先选模型再 create，因此不受影响。

## Windows 应用证据（2026-10-19）

- `app/windows`（GPUI，gpui-kit `=0.7.3`）直接链接 bindings，入口见 [Windows README](../../app/windows/README.md)。`scripts/build-windows.ps1` 在 win-ws 上完成 release 构建，组装出可移动的应用目录。
- 在 win-ws 的交互会话中以隔离 `VELUNE_HOME` 运行，用临时脚本模拟点击（未入库），已验证：启动与设置窗口；远控服务就绪，接入开关打开；运行时发现列出用户全局安装的 pi 0.80.3、codex 0.155.0、claude 2.0.76，并标为不受支持；手动添加 Pi 1.0.2 运行时；添加合成回环提供商（Chat Completions，`127.0.0.1:18080`，非真实 key）。
- 发现 gpui-component 对话框内的输入框在 Windows 上收不到点击，点击落到遮罩上、对话框随之关闭。新会话与重命名因此改为就地编辑。
- 首次冷启动时，Pi 的版本探测超过 15 秒超时，疑为 Defender 扫描；重试成功。
- 事故：一次界面脚本运行时，Velune 窗口不存在，而用户正在 win-ws 上使用 Codex 应用，脚本按绝对坐标点击并输入，把测试路径输入了用户 Codex 的输入框（未发送）。此前的粘贴步骤也两次覆盖了用户剪贴板。之后脚本增加两道保护：用户空闲不足 10 分钟即中止，目标窗口不在前台时不发送任何输入。
- 事故后的界面脚本（2026-10-19 晚）只再启动了一次应用，主窗口与“新会话”入口正常。脚本自身的输入会重置 Windows 的空闲计时，保护随之阻止后续运行；放宽保护的修改被权限审查拒绝，开发方于是停止在 win-ws 上自动操作界面，并关闭了测试实例。
- 2026-10-19 夜，用户同意后放宽保护：上次输入若是脚本自己注入的，可以继续；运行中检测到用户输入即中止。在隔离 `VELUNE_HOME` 下验证了：新会话草稿（运行时与项目目录、最近项目）→ 选择合成模型 → Ctrl+Enter 发送 → 回合完成，消息左右布局正确；双击就地重命名，Enter 保存，标题随之更新；审批卡（ACP fixture）允许后回合完成；停止后显示“本轮已取消。”。
  - 修正：Pi 会话在 Windows 上不进列表。`pi_sessions.mjs` 用 symlink 暂存会话文件，而 Windows 只有管理员或开发者模式才允许建 symlink，每次都报 EPERM。现在遇到 EPERM 改用硬链接，再不行就复制。已在 win-ws 上热替换该文件验证，会话随即出现在列表中。
  - 修正：较长的失败信息（含路径）不会换行，主区被撑宽，侧栏被挤窄，“浏览”“发送”被推出窗口。主区改为可收缩，侧栏宽度固定；邀请文本一栏同样处理。
  - 修正：打开中的会话一直显示未读点。改为与 Mac 相同：窗口处于前台时，把打开的会话标为已读。
  - 修正：重命名时点击空白处不会取消，因为 GPUI 点到不可聚焦的区域时焦点不变。改为点击输入框以外任意位置或按 Esc 即取消。
  - 后三处只在 Mac 上通过了编译，尚未在 Windows 构建中复验。
  - 首次冷启动时 Pi 启动 RPC 超过 15 秒超时，重试成功，与此前版本探测的现象相同。
  - 中文输入法（微软拼音）：组字、候选窗位置、上屏及发送中文均正常。已知限制：组字中途切换到另一个输入框时，未上屏的拼音会留在原输入框，组字继续在新输入框进行。原因是 GPUI 只用一个原生窗口，输入法感知不到焦点变化。用户未要求验收输入法，未修。
  - 自动化注意：win-ws 上用户的 AutoHotkey 脚本对调了 LCtrl 与 LWin，对注入的按键同样生效。此前记录的“GPUI 输入框里 Ctrl+A／Ctrl+V 无效”即由此导致，不是应用缺陷。
- 待验收：设置 › 设备的二维码，与 Android／Mac 互相配对及批准对话框。2026-10-19 用户把优先级调整为 Android 控制 Mac 可用，Windows 剩余项延后。

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
- 未验证：扫码（手机相机对不准 Mac 屏幕，只确认了相机预览启动）、后台恢复和长时间运行。跨网见下节。
- 2026-10-19 夜，验收结束后清除了手机上的应用数据（测试配对与旧原型的 `identity`、`trust.json`、`files/s1-probe`），并安装了 `1949ad8` 的 APK，用户真实配对从空状态开始。

## Android 控制 Mac 真机证据（2026-10-19 夜）

用户把优先级调整为 Android 控制 Mac 可用。用 Mac 开发副本（临时 `VELUNE_HOME`，内置磁盘，后台运行，种入 ACP fixture 运行时）和 JKM-AL00a 上的真实 App 走通：

- 配对：手机读取 Mac 生成的邀请，显示 Mac 名称；连接后 Mac 收到“JKM-AL00a”的请求（核对码 8933 3846），批准后手机自动进入设备并显示“已连接”。邀请和批准经临时 hook 传递（Mac 开发副本启动即生成邀请写入文件、并自动批准；Android 粘贴先读应用私有文件）。hook 只存在于构建产物中，源码已还原、未入库。二维码扫描与 Mac 批准弹窗沿用此前的验证。
- 手机上：Mac 会话列表与转写；发送；Mac 运行时的审批在手机上允许后回合完成；停止后显示“本轮已取消。”；新建会话（选择 Mac 上的运行时与项目）并发送，回答正常。
- Mac 窗口同步显示手机发起的回合。Mac 退出重启后，手机短暂显示“远控入口已关闭”与“正在重新连接”，随后自动恢复为“已连接”，无需重新配对。
- 修正：手机新建并很快完成的会话不进 Mac 侧栏。Mac 只在活动轮询看到“未列出且仍在运行”的会话时重读列表，回合在两次轮询（0.6 秒）之间结束就被漏掉。现在轮询返回的已结束回合属于未列出的会话时，也重读列表。复验时，Mac 应用在后台，手机新建的会话随即出现在侧栏中。
- 待用户验收：已安装的 `/Applications/Velune.app` 与手机 App 用真实运行时配对并使用；扫码配对。

### 跨网络（2026-10-19 夜）

用户让手机（Wi-Fi 10.33.56.0/24）与 Mac mini（172.16.249.154，出网经本机代理 TUN）处于不同网络，手机 ping 不到 Mac。同样的开发副本与 hook 下：

- 配对（核对码 8742 1569）、会话列表、转写、发送、审批允许、停止均走通。邀请里没有 relay URL（hook 在启动后立即生成，home relay 尚未就绪），手机靠 pkarr（dns.iroh.link）找到 Mac。
- 路径：Mac 的 home relay 是 `usw1-1.relay.n0.iroh.link`；连上后业务流量主要走 iroh 的 UDP socket（20 秒空闲采样中 UDP 约 43 KB 出、relay TCP 约 4 KB），即打洞直连。手机到该 relay 的 RTT 190–450 ms、丢包 25–33%，所以冷启动“正在连接”约 20 秒，连上后请求即时。缩短首连需要就近或自建 relay，属于部署决定，未做。
- 修正：Mac 重启后手机停在“无法连接（protocol）”，不再重试。原因是 remote 的 `read_frame` 把任何流读取失败（对端 reset、连接丢失）都报为 `Protocol`，而控制端恢复循环把 `Protocol` 当作不可恢复（Blocked），Android 又没有显式重连入口。现在只有对端发来的字节无法解码才算 `Protocol`；流被 reset 或连接丢失为可重试的 `Unavailable`，超长为 `FrameTooLarge`。复验：Mac 重启后手机约一分钟内自动恢复“已连接”，错误提示随下次轮询消失，发送正常。
- 观察（与网络无关）：Mac 重启后 fixture 会话按 ACP 回放重建，被取消回合的用户消息与下一条用户消息合并显示为一条（“CANCELPERMIT”），待查是 fixture 回放还是投影合并规则。
- 未做：邀请生成时若 home relay 未就绪则不含 relay URL，只靠 pkarr；正常经设置界面生成时 relay 一般已就绪。
- 结束后安装了 `1f51260` 的 Mac 应用（manifest `dirty: false`）与 APK，并再次清除手机应用数据，用户真实配对从空状态开始。

### 跨网复验：连不上与很慢（2026-10-19 夜至 10-20）

用户用已安装应用真实配对后反馈：先出现“StandaloneCoroutine was cancelled”，几秒后会话才出现；点开会话消息加载很慢。另装到第二台手机（24069RA21C，Android 16）。

- “StandaloneCoroutine was cancelled”：离开或切换界面时轮询协程被取消，`catch (Exception)` 把 `CancellationException` 当作失败显示，直到下次轮询成功才消失。Android 各协程 catch 现在先重抛取消。
- 有时数分钟停在“正在连接”。用临时构建（未入库）在两端把 iroh 事件写入文件后定位：Mac 出网经 sing-box TUN，QAD 观察到的“公网地址”其实是代理的 UDP 映射（198.20.155.137），并随 Hello 写进手机保存的 locator。手机首包同时发往 relay 与该地址；经代理回到 Mac 的那份被改写成 relay QAD 端口来源（`Ip(198.18.0.204:7842)`），若先到，Mac 把连接绑在这个地址，随后同一连接 ID 的 relay 副本也并入，握手应答永远发错地方，30 秒超时后重试，形成竞态。修正：设备描述只携带本机网卡地址，QAD 映射交给连上后的打洞验证（与设计文档“只广告本机实际持有或已验证映射的地址”一致）。复验：Mac 换到 euc1 relay 后，手机经 pkarr 找到并在约 16 秒内连上。
- 慢：手机设备标题现在显示所选路径与 RTT。实测“已连接 · 中继 1145 ms”。手机网络是对称 NAT（QAD 地址随目的变化），Mac 的 UDP 全部经代理出境，没有可打洞的真实公网地址，所以始终走 relay；而 Mac 的 home relay 按代理出口延迟选择（先后为 usw1、aps1、euc1），每次请求都要往返境外两段。打开会话需要多次请求，所以很慢。
- 对照 liguobao/ds-harness-remote（`docs/network.md`、`docs/server.md`，2026-10-20 查阅）：自部署 Remote Server 负责 signaling、短期 TURN 与 WSS Relay，数据面用 WebRTC ICE，按 LAN → P2P → TURN → Relay 降级。与 iroh 的打洞（QAD≈STUN）加 relay 是同一类方案，不是“无 relay”；其低延迟来自打洞成功或自部署的就近服务器。该项目文档自述 Android 真机跨网 P2P/TURN 与代理环境仍待验证。本机条件下（Mac 无全局 IPv6，IPv4 与 IPv6 都经 sing-box TUN 出网；手机对称 NAT），WebRTC 同样只能拿到代理映射，打洞失败后同样依赖中继。
- 代码无法单独消除这段延迟。可选：用户让 sing-box 对 Velune 直连（进程规则），使 Mac 用真实宽带地址打洞、按真实延迟选 relay；或自建就近 iroh relay（核心已支持 `RelayConfig::Custom`，应用目前固定公共 relay），属部署决定，待用户决定。

## 配置跨端同步方案（候选，待用户核对）

2026-10-19 用户决定：同步另开任务实施，本任务只需保留扩展性。现状不妨碍后续实施：远控请求按操作分类型，可以增量新增同步请求；修订号与墓碑由同步任务引入。需要注意：模型记录键由纳秒时间加进程内序号生成（`application` 的 `new_record_key`），跨设备碰撞的可能性很低但不保证，同步任务应改为带设备限定或随机的键。下列候选方案与四个待决问题转交该任务。

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
