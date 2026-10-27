# 遥测实现验收记录

2026-10-27。初步接入已实现；本轮补完可用设备的配置复制／撤销及开发入口验证，并接入 SVC dev。晚间 iPad 恢复后，实际 iPad→Mac 分享与撤销补验通过；长时间移动挂起仍未观察。用户已授权提交及覆盖已安装 Velune；其他任务修改按归属保留。已更新 `/Applications/Velune.app`；此前按用户要求卸载了 iPad 日常 Velune 以释放免费签名名额，既有测试包保留。

## iPad 恢复后的补验

2026-10-27 晚间，`devicectl` 确认既有 iPad 可用，原生截图确认 Velune 前台；Mac 会话也已解锁。继续复用已安装的 `app.velune.ios.telemetry` 和 `/Applications/Velune.app`。AccessibilityAudit 的默认焦点枚举没有事件，精确设置当前 Velune PID 并标记目标后即可返回控件；实际导航得到遥测表单。该问题没有通过安装运行器、串流或调整系统安全设置解决，也未归因为产品故障。

iPad 实际点击生成邀请，Mac 通过既有应用的本地 CLI 接收邀请；两端核对码一致后，iPad 实际点击批准。Mac 从遥测设置选择 iPad 并点击复制，落盘完整 profile 与 iPad 一致，本机仍为 Unset，没有导入来源的安装身份；再次复制保留既有本机身份和偏好。iPad 实际撤销后，Mac 再次读取显示“设备未获授权或授权已撤销”，目的端配置字节完全不变。证据为 `target/telemetry-native-ui/ipad-mac-final-share-evidence.json`。Mac 初始尚无保存配置，因此不把运行实例 ID 当成安装身份；身份保留由第二次复制前后的配置比较证明。

这补足了 iOS 来源与 Mac 目的端入口，与已有 Mac→iPhone Simulator、Windows↔Android 一起覆盖四平台的来源和目的端角色，不等同于穷举所有设备组合。iPad 原始备份 `ipad-ui-config-before.json` 的 profile 为 null；恢复后普通新进程的地址、认证和 run ID 均为空，双 SDK 关闭，原始偏好和安装身份保持。证据为 `ipad-resumed-restoration-evidence.json`。

实际复制失败同时暴露 Apple 共用 `TelemetrySettingsView` 的旧成功提示残留。修复在异步 action 开始及同步表单校验失败时清空上一条 status，覆盖 Mac 和 iOS，不新增状态抽象。Mac 覆盖更新后，通过真实控件保存，再添加重复认证字段并保存，观察校验错误且旧成功文字消失，文件字节不变；见 `apple-status-fix-mac-evidence.json`。另一次更新后的异步读取未取得明确拒绝终态，不计为新的撤销通过；前述实际分享／撤销证据使用修复前的同一协议实现，显示修复不改变其调用与权限逻辑。

iPad 更新安装后又执行一次强制结束和普通启动，确认新 PID 的 profile、地址、认证和 run ID 均为空，双 SDK 关闭，原始偏好与安装身份保持；随后通过精确 PID 的原生控件进入遥测页，观察“未设置／已关闭”，且已撤销权限没有恢复。证据为 `ipad-status-fix-final-evidence.json`。没有把这项检查扩大为长时间后台挂起或活动导出队列恢复的结论。

Mac SwiftPM 和 iOS 真机 Xcode 构建均通过。为避免混入并行任务，使用 `1b342eb` 原生快照加本次共用视图修复，Mac 保留既有本地 CLI 接线，复用匹配的原生库与生成绑定；不将这些产物称为整个当前工作树构建通过。Mac 安装包已覆盖并通过严格签名检查，iPad 也覆盖同一个既有 bundle ID，没有新增开发 App。构建来源与校验值见 `apple-status-fix-build-evidence.json`。已移除临时原生源码快照和 Mac 私有验收 home，正常退出验收实例并重开 Mac 日常入口；iPad 保留已恢复配置的普通实例。

## 可用设备收尾与 SVC dev

本轮继续使用已安装 Velune，未新增辅助 App、镜像串流或配对程序。证据位于 `target/telemetry-native-ui/`，使用合成配置。产品架构与开发生命周期分别经过独立 advisor 审查；没有发现需要推翻官方双 SDK、代际撤销或独立遥测分享授权的具体问题。本轮未调整产品依赖，删去了启动器自己维护 SSH 隧道的代码，改为调用 SVC 声明的能力。

| 观察 | 结果与证据 |
| --- | --- |
| Android→Windows | 新邀请经两端实际核对、批准，Windows 从实际设置入口复制完整 profile，保留本机偏好与身份。Android 撤销后，Windows 明确显示“设备未获授权或授权已撤销”，配置字节不变。`windows-final-revoke-evidence.json` |
| Windows→Android | 复用已有授权并刷新来源定位，实际设置入口复制成功；Windows 撤销后，Android 明确显示同一拒绝，配置字节不变。本次没有重新完成反向核对码批准，不将它重复计为新配对。`android-final-revoke-evidence.json` |
| Mac 开发入口 | 通过 SVC 启动已安装 App 的私有实例，两套 SDK 激活同一修订；查询到 traces、logs、metrics。正常退出后，同一私有 home 普通重启，run ID 为空且两套 SDK 关闭。`svc-mac-launch.json`、`svc-mac-restart-evidence.json` |
| Android 开发入口 | 不传 APK，直接启动既有安装；两套 SDK 激活，查询到 2 traces、2 log streams、34 metric series。系统 activity 确认进入后台，随后强制结束并普通重启，新 PID 的 run ID 为空、两套 SDK 关闭；保存 profile、偏好、身份不变，正常装配使 revision 从 8 变为 9。`svc-android-launch.json`、`svc-android-restart-evidence.json` |
| Windows 新开发入口 | 新启动器在交互桌面运行既有 WinUI 产物，双 SDK 激活，查询到 8 traces、10 log streams、173 metric series。正常退出后同一私有 home 普通重启，两套 SDK 关闭；普通实例也已在同一交互桌面正常关闭。`svc-windows-launch.json`、`svc-windows-restart-evidence.json` |
| SVC 能力 | 四目标曾同时 healthy；实际启动／复用／停止通过。停止本机通道后，确保 Windows 目标能恢复上游，Windows SSH master PID 保持不变。`svc-status.json`、`svc-recovery-evidence.json` |

Android 实际撤销失败提示曾与上一次“已复制”并存，原因是共享 action 入口只清空 failure。现在开始下一次 action 时同时清空 status。用 `1b342eb` 原生快照加该修复、匹配的既有 Rust 库／绑定构建 APK，覆盖同一既有包后，实际成功发送三信号，再输入无效地址发送测试，观察到校验错误且旧成功文本消失；保存 profile、偏好和身份保持不变。证据为 `android-status-fix-evidence.json`。APK 与 lint 通过；lint 首次被快照中的生成绑定符号链接干扰，复制相同绑定到快照内后通过，没有修改依赖或禁用检查。初次无效地址操作仍停留在输入法／旧界面，未计作产品失败；后续确认实际字段和最终界面才作断言。

Mac 查询最后返回 100 traces（达到查询上限）、49 log streams、189 metric series，不将上限结果当成完整 trace 集。安装来源仍是前文已记录的产物；manifest 的 `workspaceRevision` 只标识启动工具所在工作区，不能代表任意已安装二进制的源码版本。Mac 途中锁屏（系统会话状态确认），因此没有把 Android→Mac 的配对完成当成 Mac 复制入口通过；已有 Mac 表单保存和 Mac→iPhone Simulator 证据继续适用。当时 iPad 不可用；晚间恢复后的补验见文首。

开发入口修复了长 checkout 路径导致 Mac Unix socket 超限的问题，使用系统私有临时 home；Android 转发失败也会继续启动业务并明确报告未接入。SSH 的 `activate` 动作在上游恢复时复用存活的 control socket，防止重复进程争抢端口。开发边界夹具覆盖短 socket 路径、移动端不重装、Android 转发失败、后端失联、查询大小、代理和重定向边界。没有为这些开发脚本引入第三方 Python 依赖。

本轮收尾已停止测试 Mac／Android／Windows 客户端，移除 Windows 计划任务、临时脚本及合成 home，删除 Android 临时启动 profile 和构建快照；保留唯一原始证据。设备通道经 `svc dev stop` 停止，本机 `telemetry` 与 sfp7 `lgtm` 保持 healthy。最终 `svc dev status` 因两个设备目标被主动停止而返回 action-required，这是预期停用状态。证据为 `svc-final-*-stop.json` 和 `svc-final-status.json`。Windows 首次从非交互 SSH 上下文关闭窗口未成功，改在应用所属的交互桌面执行后正常退出；不将前一夹具失败记作产品退出缺陷。Python 编译、开发边界检查、相关文档相对链接／标题锚点和 `git diff --check` 通过。

## 共享配置、SDK 与分布式执行

| 边界 | 已观察结果 | 可重放入口／证据 |
| --- | --- | --- |
| 默认关闭、私有持久化与修订 | 无配置不激活；明确 Off 优先；原子保存失败、冲突、未来格式不破坏原文件；关闭 owner 后延迟工作不能重新激活 | `cargo test -p velune-telemetry -p velune-remote --lib`；17 项 telemetry、7 项 remote 通过（包含显式测试的 run ID／scope、旧代 context 和传输撤销） |
| 实际 Apple coordinator／FFI／业务对象 | native 与 Rust 三信号、并发不可变 parent、环境隔离、零采样保留错误日志／指标；同一业务对象更换 SDK；旧队列不迁往新出口；调用视图共享关闭 | [manual-telemetry.py](../../scripts/manual-telemetry.py)；`target/telemetry-delivery-acceptance.json` 与对应 wire／boundaries JSON |
| 实际 QUIC／Core 轮次 | 两个独立父链；选中和停放会话共 2 个活跃轮次；控制断开后继续；换代后仍为 2，成功／取消降至 1／0；旧代结果保留 operation ID 与缺口；取消写失败只产生一次 failure 终态 | 同一脚本加 `--distributed`；完整接收数据按 trace、parent、日志、gauge 和 outcome 断言 |
| 结果未知与恢复 | 接纳的命令超过控制端等待期限，返回原 RequestId 与 unknown；重连查询相同 ID 获得完成结果，没有重发 | `--distributed` 的受控 ACP 初始化门闩 |
| 实际 HTTP Gateway | 首次 503 后下一成员返回 200，每次尝试单独可见；两个并行请求保持各自接纳时的父节点；同一 Runner 换代后产生带缺口的结果；不能确定归属时明确 missing_parent | 同一脚本加 `--gateway`；[HTTP 夹具](acceptance/gateway.py) |
| 实际 Mac 请求队列 | 合成工作阻塞 Store 串行队列，后续快调用的 Transport 耗时约 1–4 ms，等待 histogram 至少 0.5 s；等待期间换代，长样本只出现在出队时的出口，业务调用次数仍为 1 | 同一脚本加 `--queue`；使用实际 Store 源码和私有调度边界，见[夹具](acceptance/queue.swift) |
| 原生取消明确拒绝 | Codex 合成 interrupt 返回 JSON-RPC error 后自然完成，业务终态为 completed；正常取消和断管失败分别保留各自语义 | [manual-runtime-interactions.py](../../scripts/manual-runtime-interactions.py)；`target/telemetry-native-interactions.log` |
| 丢弃与隐私 | 真实 SDK 队列溢出汇总保留旧 revision；完整接收三信号中无合成正文、路径、环境认证 canary；实时完整丢弃数仍为未知 | Rust wire tests、完整接收断言；SDK 公开接口边界见[装配契约](../../docs/design/telemetry.md) |

端到端测试曾触发实际 panic：tracing-subscriber 在析构旧 registry 子 span 时，通过当前 dispatcher 释放父引用。修复是受控操作不保留隐式 registry 父引用，改用显式 W3C 关联；并在创建时保存 context，避免旧 SDK 关闭后才生成上下文。原场景已重放通过。Pi stdio reader 另以独立根 span 存活，避免 localLogs=true 时把启动操作留到进程结束；实际适配器与合成 Node 进程验证了 reader 仍存活时 startup 已导出、换代后关闭不重复导出。没有用“缺少 span”替代业务终态或活跃状态。

实际 Apple coordinator 与 Rust SDK 的自签证书拒绝检查通过：两侧记录导出失败，TLS 接收器没有接受任何三信号 RPC，业务 list 仍完成；见 `target/telemetry-tls-acceptance.json`。它不替代各移动系统的真机证书、网络及后台行为。

## 平台与开发入口

| 平台 | 构建与实际产品数据 | 已有界面／生命周期证据及限制 |
| --- | --- | --- |
| macOS | SwiftPM 完整构建通过；实际双 SDK 三信号进入 sfp7；实际 Store 队列验收通过 | 隔离 GUI 的遥测表单、双侧回执、三信号测试通过。锁屏前完成 Mac→iPhone 的独立授权、复制、撤销后拒绝和保留原配置；本轮重新启动后，通过真实控件保存地址与合成认证，普通重启读回一致，修订保持为 1、Unset 下双 SDK 关闭；可重放入口见下文 |
| Windows | 实际 Windows Rust release／WinUI publish 通过；当前开发入口在 sfp7 查询到 7 traces、10 log streams、139 metric series | 浅／深色、定时器、关闭设置后的回调、正常退出 0、未新增 crash.log、当前用户保护 DACL 通过；实际 SDK 中途构造失败清理、首次 HTTP 写出前撤销及未采样上下文传播通过。开发临时配置的强制终止后普通重启为 Unset/off，旧 manifest 不冒认新进程；该项不替代正常退出检查 |
| Android | 最新独立 application ID 的 native／APK／lint 通过；实际设备经 ADB reverse 查询到双 SDK 的 2 traces、2 log streams、34 metric series | 最新包普通重启为 Unset、两侧 inactive、run ID 空，旧 manifest 拒绝冒认。解锁后真实表单及 Android→Windows 分享已补验；Windows↔Android 双向复制已通过；真机系统证书拒绝与短暂前后台保持同 owner 已补验（见下文）；短暂后台后强制结束／普通重启已补验，长时间挂起仍未观察。使用既有测试安装 |
| iPhone／iPad | 同一 iOS target 的 arm64 Simulator 构建通过；两种专用模拟器均查到双 SDK 三信号 | 两者普通重启均为 Unset、SDK inactive、run ID 空；安装更新会改变容器路径，工具不冒认旧状态。iPhone 的上述复制流程已实际操作；iPad 真机网络及自签证书拒绝已补验；新包表单、强制结束／普通重启已补验，长时间后台未观察 |

Windows 原生 SDK 故障夹具是[windows-sdk.ps1](acceptance/windows-sdk.ps1)，它只在临时副本中注入异常或首次写出门闩，不增加产品故障开关。证据为 `target/telemetry-windows-sdk-evidence.json`；浅／深色图像及退出结果在 `target/telemetry-windows-ui/`。Windows 开发入口的当前实例、生效与普通重启数据在 `target/telemetry-windows-dev-entry/`。

Windows 额外边界测试发现 `.NET` 官方 SDK 在未采样本地父级下返回空子 Activity，原实现因而丢失 carrier。已改为传播现有父上下文，实际协调器复验覆盖根 0%、经过 await 的未采样嵌套、根 100% 继承未采样父级及 tracestate、已有 Current 时 Off 仍不传播。最新版 WinUI publish 通过。Android 最新安装／重启证据为 `target/telemetry-android-delivery-evidence.json`。

[开发入口检查](acceptance/development.py) 证明查询不使用环境代理、不带认证跟随重定向，并限制为 4 MiB；后端和 Git 元数据不可用时仍请求业务启动，未知状态明确返回 state_unavailable。这个单项检查拦截进程创建参数；实际应用启动、数据接收与重启另由上表证明。开发工具只传运行所需的环境字段，不转发无关凭据。Windows 复用明确建立的回环转发，未向 LAN 开放 LGTM。

前轮基线的全 workspace all-targets／all-features check 与 clippy 通过；相应 telemetry、agent-runtime、gateway、bindings library clippy（`-D warnings`）、格式检查及 17＋7 项 Rust 测试通过。实际 AppStore 的 100 次空闲回调不重复发布、问题恢复和真实配置更新仍传播的检查通过，见 `target/telemetry-store-idle-delivery.log`。macOS 隔离 bundle 已刷新为最新构建并通过签名检查；模拟器与测试客户端已停止，sfp7 开发接收端保留。

当时的 Python 编译、两个 Apple 构建脚本的 shell 语法、开发入口边界检查和 `git diff --check` 通过；21 份相关文档的 380 个相对链接／标题锚点检查无误。已移除本任务 Windows 验收计划任务、Android reverse 与 Windows 临时反向隧道；sfp7 接收端及 Mac 查询隧道保留。后续正式 Mac 更新曾正常退出日常客户端，不声称日常客户端始终未停止。

## agent 诊断演练

独立 advisor 只读取三个限定 JSON，未读取源码或故障注入记录。输入包含实际 OTLP 接收数据、时间窗、实例、采样率、100 条上限及是否截断；B 另含断开边界的实际 FFI 状态断言。由[案例生成器](acceptance/agent-cases.py)从 wire／boundaries 文件生成，输出在 `target/telemetry-agent-cases/`。

| 案例 | 注入账本与实际判断 |
| --- | --- |
| A | 合成 ACP 入口不可用。agent 定位到同 trace 的 harness.start→remote.execute 失败，指出传输／结果查询成功不能抵消业务失败；没有凭上层 invalid 猜测具体文件系统原因 |
| B | 控制断开，两个轮次保持门闩未结束。agent 结合 started 事件与 activeTurns=2 判断工作仍在继续，并说明聚合数不能严格替代逐轮次状态，最终结果仍未知 |
| C | Gateway 首成员 503、次成员 200。agent 正确识别同请求下先失败后成功，说明不能据此断言 LLM 业务完成或精确区分首次失败原因 |

三案与独立注入账本相符。另已通过 sampled=1→本端 0%、sampled=0→本端 100% 的实际 native／FFI／QUIC 用例；后者仍保留有效 carrier 和关联日志，没有另起已采样根。诊断能力仍有明确边界：受限元数据用于定位阶段，完整启动原因需按操作编号／RequestId 读取获授权的本地问题或业务结果；查询未截断、全采样也不证明未埋点或时间窗外的事件不存在。

## 解锁后的原生验收与工具链边界

本轮遵循用户要求，改用 Android UIAutomator／instrumentation、Windows UI Automation 和 Apple XCTest／XCUITest。准备过的 scrcpy 没有启动串流，已卸载。Android 真实表单保存含合成 metadata 的配置、关闭状态下显式三信号测试均通过。Android→Windows、Windows→Android 均已实际核对双方相同代码、批准独立遥测权限、复制合成认证字段，并验证目的端仍为 Unset；随后撤销来源权限，目的端保存配置及修订未改变。证据在 `target/telemetry-native-ui/`。当时尚未确认撤销后的界面错误；本轮已由文首的实际拒绝反馈与配置不变证据补足。

iPad 真机的独立 `app.velune.ios.telemetry` 包已构建、安装并普通启动：默认 Unset、无 endpoint、双 SDK 未激活、已完成批次均为 0。XCUITest 运行器先遇到 Apple 免费签名最多三个 App 限制；用户明确授权后仅卸载了日常 `app.velune.ios`，隔离包与声邻 Pad 保留。运行器随后仍在启用 automation mode 时超时；设备 lockState 显示不需要解锁，不能归因于锁屏。

用户完成 Mac 的系统认证后，Mac XCUITest 已进入测试方法。当时失败来自验收运行器带有 App Sandbox，却试图向其容器外写控制文件；这是验收夹具问题，不是产品保存失败。后续应使用运行器自己的容器，不能据此要求放开系统安全。iPad 模拟器已运行 XCUITest 并生成实际树／截图，但点击后尚未观察到预期导航，不计为通过。

用户报告本机卡顿后，已停止本任务 Apple 测试及专用模拟器。实测交换空间使用约 19.3 GiB，当时 memory_pressure 空闲指标为 38%；没有据此宣称已确定全部卡顿来源。后续 Apple 验收串行复用已有构建，减少全层级快照，只补尚缺的关键 UI 流程。Apple 官方建议以逻辑／集成测试为主、少量 UI 端到端测试，并支持分别构建和运行测试，见[测试策略](https://developer.apple.com/documentation/xcode/testing)、[运行器说明](https://help.apple.com/xcode/mac/current/en.lproj/dev6061748b5.html)和[命令行构建与测试](https://developer.apple.com/library/archive/technotes/tn2339/_index.html)。

已核实本机 macOS 15.4.1、Xcode 26.2，Xcode 包内最低系统版本为 15.6；iPad 为 iPadOS 27.0。[Apple 支持表](https://developer.apple.com/xcode/system-requirements/)列出的 Xcode 26.2 主机要求为 macOS 15.6 以上，对应设备支持至 iPadOS 26.2。当前组合不受官方支持，但尚未证明它就是自动化超时或导航问题的根因。真机 XCUITest 缺项需先在匹配工具链的环境补验，未进行系统升级。Mac 另以独立 XCTest 小程序调用公开 AX API，复用已经获准的 AX 能力，绑定隔离应用的确切 PID；权限探针通过，测试进程 RSS 约 13 MiB，不安装额外 App、不启动模拟器或 xcodebuild。其早期保存烟测没有得到预期配置落盘，不能计作保存通过；后续真实输入／提交／重启已补足产品证据，见下一节。框架和 API 仅是诊断手段的记录，不能成为产品通过项目或证据强度的分类。

## 清理后重新启动验收

2026-10-27。已停止本任务隔离 Mac／Android 客户端，移除 Windows `VeluneTelemetryNativeAcceptance` 计划任务、Android 辅助 instrumentation APK、ADB reverse 与 Windows 临时反向隧道。移除五份重复 Apple DerivedData，保留唯一证据；没有运行中的模拟器。sfp7 开发服务及 Mac 查询隧道保留。iPad 辅助 UI runner 已卸载，隔离产品包保留。

Mac 原生保存／重启闭环已经通过，见 [mac-ui.py](acceptance/mac-ui.py) 和[原生用例](acceptance/mac-ui.swift)。最新结果直接运行 `/Applications/Velune.app`（`local.velune.prototype`），创建私有合成配置，从普通入口启动；通过实际文本框、键盘输入与保存按钮写入指定地址和合成认证字段，然后正常退出并重新启动，验证界面读回、落盘修订及 Unset 下两套 SDK 仍关闭。证据为 `target/telemetry-mac-ui/mac-save-evidence.json`，实际用例耗时约 5 秒。它不验证配置复制、点击命中区域或未执行的移动生命周期。

已安装 Mac 的原生源码基线为 `4288414`，保留工作树既有 `RemoteDevices.swift` 本地 CLI 接线，复用本轮已完成的单 worker Rust release 库。另一任务的 compaction 改动使随后生成绑定时的 debug 重编译失败，故直接用已有相同 UniFFI 版本的 bindgen 生成绑定，并从提交快照完成原生构建；没有修补或回退另一任务源码。Swift native/debug 构建完成、严格签名校验通过，并用仓库安装脚本覆盖正式路径。构建日志为 `target/telemetry-native-ui/installed-macos-stable-build.log`，安装来源记录为 `target/telemetry-native-ui/installed-macos-provenance.json`。该产物不代表正在变化的整个工作树已构建通过。

本轮排除了夹具中的预览模式和旧窗口，并读取 macOS `AXTitleUIElement` 提供的控件关联标签；不会把这些夹具失败记为产品缺陷。此前旧的直接 `AXValue` 保存检查未独立复现其唯一原因，已由上述真实输入／提交／重启结果补足所需产品证据。标准 UI runner 本轮再次停在系统 Enable UI Automation 认证，未进入产品用例；没有继续循环重试或修改系统安全设置。

用户随后指出 ProfilePair 辅助程序造成打扰，并明确可以直接使用／覆盖已安装 Velune。已停止该路线，删除本轮新增的配对辅助源码及执行产物，保留历史断言 JSON。辅助程序曾完成的独立 SDK／QUIC 观察只作为边界证据；后续实际应用验收不依赖这些程序，也不再为凑 16 格制造新的产品副本。其 Windows→Mac 首次握手失败尚未归因，不冒充实际 Velune 已复现的产品缺陷。

实际应用当前已观察 Mac→iPhone Simulator、Windows→Android、Android→Windows。剩余关键分享路径是 iPad→Mac，它与前述路径共同覆盖四平台的来源与目的端角色；每端设置入口和共享权限／原子保存／协议边界分别核对。未观察的组合不宣称逐对实测。

[移动网络用例](acceptance/mobile-network.py)预置隔离安装的合成配置，使用实际应用 SDK，不能冒充 UI 保存。Android 与 iPad 真机均已从原生／Rust 的三个官方 scope 收到全部三信号，且两侧状态给出真实接收回执；随后连接带正确 IP SAN 的自签证书接收端，服务器出现证书告警、两侧导出失败、零 OTLP RPC 被接受。配置已恢复原值并普通重启。Android 最新证据为 `target/telemetry-mobile-android-54f093b2af25486da0194497a45a9b56/evidence.json`，包含系统 resumed activity 确认切后台／返回，同一 PID 与 SDK 修订保持，恢复后原有 revision=7、Unset／双 inactive；不据短暂后台推断长时间挂起行为。iPad 证据为 `target/telemetry-mobile-ios-3db9815cb411451bbbb853de54bfede0/evidence.json`，恢复后本轮 PID 2598、Unset／双 inactive。此前新进程未发布状态的尝试未计通过；用户确认设备亮屏且 Velune 前台后，重新取得本轮 PID 的新鲜状态再执行了成功的检查。没有把工具 launch 回执或旧状态当成就绪。

随后使用设备原生 AccessibilityAudit 服务操作已安装 iPad App，已观察“添加认证字段”实际增加表单行；没有安装新运行器或启动媒体串流。为余下分享流程预置了 Unset 的合成来源 profile，但重新启动时设备服务连接中断。Apple `devicectl` 显示 unavailable，原生设备发现和 Bonjour 发现均为空；该条件阻止继续操作，尚无产品分享失败证据。测试包目前仍留有此合成来源 profile，待重连后恢复此前无 profile 的状态；不把它写成 UI 保存或已完成分享。

设备恢复后只补 iPad→Mac 的实际分享、撤销拒绝与原配置保持。既有共享配置、SDK 撤销、并发 FFI／QUIC／Core／Gateway 和独立 agent 诊断证据继续复用，相关实现变化时才重放。设备不可用不阻止交付已实现的初步接入，但阻止“全部验收通过”的声明。

## 尚缺的验收

移动长时间挂起仍未观察；Android 的后台／强制结束／普通重启，以及 iPad→Mac 分享已通过。系统自签证书拒绝已有上述真机结果。共享 QUIC 权限／迁移测试已经覆盖手机无 Handler 双向分享、无隐式凭据授权、撤销和旧控制关系保留，但不能替代这些平台入口。

晚间 iPad 已恢复并完成上述补验；没有可用 iPhone 真机。Apple UI 的当前夹具和工具链限制见上一节；专用模拟器已关闭以释放资源。sfp7 的有限负载与重建持久化通过，不代表长期生产负载或保留期删除已经经过实际等待。

排查进程时曾误输出完整环境，使 NODE_AUTH_TOKEN 出现在工具输出。已通知用户并建议轮换，未复述或使用其值；轮换尚未确认。后续仅读取指定非秘密字段，开发启动环境已改为白名单。

清理时也修正了本任务 Swift／.NET SDK 边界用例的入口：断言失败写到 stderr 并返回非零退出码，避免把正常的检查失败变成系统崩溃弹窗。它们仅在需要验证 SDK 故障边界时运行，不作为新的产品应用或实际设置验收入口。
