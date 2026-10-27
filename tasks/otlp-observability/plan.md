# OTLP 实现计划

状态：2026-10-26，用户已授权实现及验收；按以下顺序实施。产品范围、技术方案及验收依据沿用已形成的任务材料。第 1–6 步的产品装配已落地，第 7 步验收正在收尾；实际证据与设备阻塞见[验收记录](verification.md)，未将构建通过视为全部验收通过。

实施目标是让 agent 能诊断四端的本地操作与分布式执行，默认无配置、无外传，每端独立设置，并能按独立权限一次性复制配置。完成条件以[验收设计](acceptance.md)为准；不增加托管后台、通用自动埋点或持续同步。

## 已收敛的实现约束

四套 SDK 的三信号、排队数据撤销、重新创建出口、环境覆盖隔离已在 Mac 上的独立进程中验证；Swift→真实 UniFFI→Rust 并发调用与 Kotlin 协程隔离也已验证。具体版本、发现和证据边界见[SDK 实验记录](sdk-evidence.md)。这些结果确定接入路线，不能替代平台和产品联调。

1. **稳定客户端 owner，按代际更换出口。** `ClientTelemetry` 持有本机配置和当前代际，长期存活的业务 owner 持有它的稳定引用。每个新操作／后台推进／网关请求接纳时读取当前代际，不能在线程启动时永久捕获旧 `Dispatch`。继续使用 `tracing-opentelemetry`，不重写 tracing bridge，也不让 exporter 把旧批次改投新地址。已开始的 span 保持原代际；关闭或切换后可能不完整。之后发生的新结果事件和新操作使用新代际并标示关联缺口，不制造“全链路完整”的假象。
2. **准备阶段不采集。** 候选阶段只验证配置、创建 exporter／传输及未接入事件源的装配材料；不提前注册全局 Activity／Meter 监听器、日志入口或开始采集。保存成功后撤销两侧旧出口，再激活新一代。两侧都报告 active revision 后才显示已应用；激活异常时显示未生效并停发，不能悄悄恢复旧目的地。普通退出先有界 flush；关闭／换地址则先撤销传输，再清理 SDK。
3. **传输撤销需要不同生态的具体实现。** Swift 持有专属 gRPC channel 并关闭；Java 使用官方默认 OkHttp sender，先关闭显式持有的 exporter，再清理 providers，调用准入与关闭排序；.NET 使用专属 `HttpClientFactory` 的 client／handler，取消并释放；Rust 使用官方 `with_channel` 和 tonic connector，在连接及 I/O 边界检查同一代际许可。Rust interceptor 只负责 metadata，不把它误当成传输取消屏障。所有清理在 UI 主线程之外完成，关闭幂等。
4. **FFI 调用视图共享生命周期。** 将底层应用／远端对象的关闭状态放进共享 session owner；不可变调用视图只附加本次 trace carrier 和既有操作身份。释放视图不关闭应用；显式关闭后旧视图不能继续调用。不能仅 clone `Backend`，也不能把目前 `ScopedRemote` 的 shutdown 空操作照搬为新视图契约。
5. **分享入口独立于控制入口。** endpoint 在控制或分享需要入站时接收连接；邀请、配对、RPC 各按具体目的检查。关闭控制不误关仍获准的分享，开启分享不允许业务 Submit。移动端只增加配置 handler，不增加本地 application Handler。
6. **迁移保留授权。** remote wire 从当前 v2 升至下一版，并同步 ALPN、邀请和描述符校验。设备状态从当前 v3 显式迁移至新格式：保留身份引用、联系人、原控制 grant、pending RequestId 和 reset notice，新增分享 grant 默认为空。新版本代码遇到未知未来格式应报错且不覆写；不能复用“非 v3 就重置”的分支。升级不承诺旧二进制可以读取新状态。

两套 SDK 不能通过反复修改全局单例来切换。Java 不重复调用 `GlobalOpenTelemetry.set`；原生 facade 在新操作进入时获取当前 provider。指标的累计状态随 provider 代际重建，旧累计值不转交新目的地；后端按标准 counter reset 语义处理。

## 实施顺序

按下面的依赖顺序推进。每一步包含实现与相应反馈，不把测试留成最后一次补做。出现边界失败时修复该步，再扩大使用入口，不额外制造人工审批点。

### 1. 建立客户端本地配置与生命周期

新增 `packages/telemetry`，加入 workspace 和锁文件，承接 Rust SDK、私有配置、代际许可、资源身份和无秘密状态快照；不依赖 application 或 remote。bindings 装配客户端 owner，提供本机配置快照、带 expected revision 的保存准备／提交、关闭、导出状态与测试接口。

配置区分 unset／off／on，保存字段和临时开发覆盖分开；认证只在私有文件、原生 SDK 装配和获授权分享路径中流动。端点、metadata、采样率、长度与数量在入口校验。未配置和明确关闭时不创建采集队列或网络客户端。两侧准备、保存、撤销、激活由原生客户端协调，Rust 保持配置写入和修订的唯一 owner。

同时整理 `VeluneApplication` 的共享 session 与不可变调用视图，覆盖 local、remote、`VeluneRemoteOperation`，重新生成 Swift／Kotlin／C#／Python 消费的 bindings。配置不得存入当前远端应用的 generic config。CLI／Rust-only 测试使用显式的单 Rust 装配；原生 app 必须报告原生和 Rust 两侧状态。

反馈：配置原子写入、修订冲突、off 优先、失败不损坏旧配置、共享关闭语义、候选阶段不积攒数据。使用合成目录与接收器；与 A01、A03–A05、A11 对应。

### 2. 在 Mac 接通一条完整的本地路径

在 `app/mac` 的稳定 AppStore／Transport 生命周期中持有原生 coordinator，添加 Swift SDK 精确依赖，接通本机遥测设置、状态和显式测试操作。设置不跟随当前远控选择切换。先覆盖启动、列表／正文读取和一条发送操作，保留 OSLog、JSONL 与问题详情。

在 `packages/application/src/service.rs`、bindings 诊断装配、runtime reader 和 gateway 的新工作边界传递稳定 telemetry owner。请求携带本次 `Dispatch` 与 context，后台 `advance` 每轮读取当前代际。保留 `tracing`、现成 bridge 与官方 propagator；字段按事件契约逐项允许，不能仅按 target 前缀全部外传。

Swift 对批准的 metric instruments 注册明确 views，三信号使用显式资源和配置。Rust 在 interceptor 中重建批准的 metadata，排除环境自动合并；SDK 自身的诊断不回灌 OTLP。测试操作使用官方生成的 OTLP/gRPC 类型检查实际回执与 partial rejection，不能只返回 SDK `forceFlush` 的布尔值；测试成功不宣称后台已可查询。

反馈：同一个未重建的 ApplicationService 和网关监听器换目的地后，新操作及后台推进出现在新端；旧批次不迁移，旧 span 缺口可解释。验证两侧关闭、错误地址／认证、采样与敏感 canary；从 Mac 通过已有 sfp7 隧道查到真实原生与 Rust 三信号。对应 A01–A06、A11、A13。

### 3. 一次完成远控协议、分享权限与状态迁移

集中修改 `packages/remote` 的 protocol、pairing、store、service、endpoint、controller 及对应 bindings。trace carrier 是独立的有界字段，不进入业务 Command、幂等身份或授权判断。先鉴权再建立受信的关联；使用标准 W3C 校验，不传播 baggage。

保留现有控制 grants，增加独立的 telemetry profile grants。邀请目的、核对材料和批准记录绑定 Control 或 TelemetryProfileRead；旧控制权限不升级为分享权限。transport 入口按两种用途合并接纳，具体 RPC 分开鉴权；关闭／撤销一种用途后，另一种用途的连接和权限仍按自身规则运行。

配置读取直接调用本地 profile owner，仅返回保存的连接、认证和采样参数，不返回开关、身份或开发覆盖；不经过业务结果账本、普通 snapshot 或持久重试。目的端取得配置后走第 1 步同一准备／提交路径。源配置离线、撤销和导入冲突明确失败，不留下半份配置。

为现有 v3 编写明确迁移函数及合成旧状态夹具，原子提交新格式。保留 pending 请求身份和未知结果，不因协议升级自动重新执行。同步旧版本拒绝行为与错误映射，更新相关人工脚本的消费契约。对应 A07、A09–A12。

### 4. 贯通执行轮次和网关的实际因果

在 `packages/application`、`packages/agent-runtime`、`packages/gateway` 的真实执行 owner 上保存轮次关联；在受理、准备、开始执行、取消、终态、网关请求和每次重试边界记录。继续使用现有 RequestId 查询恢复，不以连接 span 或 reader span 充当业务父节点。

控制端离开不结束被控端工作；发送受理与轮次完成分别计量。网关在请求接纳时捕获所属轮次，不能事后从“当前轮次”猜测旧请求。无法唯一证明归属时记录缺口，native 直连和外部 Harness 的内部过程不伪装成已观测。

按 UI 等待、远控请求、执行轮次、网关尝试分别确定指标 owner；记录成功、失败、取消、结果未知，避免原生与 Rust 重复累计。以两个并发轮次、回应丢失、重连查询、重试后成功和长任务关闭控制端验证 A06–A08、A13。

### 5. 覆盖 Windows、Android 和 iOS 原生端

Mac 的本机设置与本地 owner 契约稳定后，Windows 用 `ActivitySource`／`Meter`／`ILogger`，Android 用 Java SDK 1.62.0 和官方 Kotlin context extension，iOS 使用与 Mac 相同的 Swift SDK 与 Apple 遥测装配代码。共享 Apple 的配置／SDK生命周期实现，界面仍遵循各端原生结构；不复制成两套独立版本。

各端在顶层客户端 owner 装配，包含尚未连接任何远端的手机。设置、测试、错误状态、来源授权／撤销、来源选择和一次性复制全部可用。按用户纠偏后的验收口径覆盖四个平台各自的真实来源／目的角色，不为穷举组合另造程序；手机前台可以分享，后台不保活。原生异步任务使用本生态 context 传播，并在 FFI 显式传递不可变 carrier。

同步 `Package.swift`、iOS 工程包依赖、Android Gradle、Windows csproj、绑定生成与许可清单，更新平台 README 和 Windows 对齐表。按 Windows AGENTS 在 win-ws 构建并实际验证浅／深色、回调运行和正常退出；iPhone／iPad 与 Android 分别验证生命周期、系统证书和设置持久化。共享 Rust 通过不代替这些证据。

### 6. 完成开发启动与 agent 使用入口

复用仓库现有构建／启动脚本增加显式开发遥测入口。连接当前 sfp7 服务并检查就绪，输出无秘密连接描述和 run ID；不要每次重建或清空 LGTM。只向本次隔离应用传私有临时 profile 路径，不写日常设置或全局 `OTEL_*`。现有 `scripts/velune` 属于其他正在进行的工作，扩展时保留其命令与改动，不覆盖重建。

Android 通过验证过的 ADB reverse 接入，模拟器和真机分别解决路径；不自动把 localhost 改写成源设备地址，不开放 sfp7 到整个 LAN。启动失败允许业务继续，明确显示未接入。agent 复用 Tempo、Loki、Prometheus 的现成机器 API与独立查询凭据，不增加一套 Velune 查询协议。

发布无秘密遥测状态快照并标明进程、代际、更新时间和过期状态。给出当前运行的应用入口、查看器入口、查询时间范围、参与实例、采样及缺口，支持 A02、A13、A14。

### 7. 完成验收与权威文档更新

沿用前面已有效的证据，补齐[验收矩阵](acceptance.md)尚缺的真实设备、故障和授权边界，再做三种已知根因的 agent 诊断演练。全新安装默认无外传、关闭后旧队列不复活、复制不提升权限、两侧不分叉是交付约束，不作为可选收尾。

把实际产品行为写入 `docs/prd/diagnostics.md`、相关设备权限分册和 Product TDD；开发命令与 sfp7 使用入口进入 `docs/development.md`。同步各 unit README、依赖许可及来源记录，检查相对链接、锚点和 `git diff --check`。只在用户明确要求时提交；当前工作区已有其他任务改动必须保留。

## 验证边界与调整条件

本轮的 .NET 实验运行在 Mac 的 net8 控制台，Java／Kotlin 实验运行在 JDK 17，Swift 实验是 SwiftPM 程序；它们验证 SDK 接口和可观察的数据，不证明 WinUI net10、Android 打包、iOS 真机或系统证书已经通过。平台完整验证属于第 5 步，不能取消。

关闭的实现必须测试 SDK 队列、进入 exporter 但尚未发送、已被接收端接纳三种边界。当前已取得四端 SDK 队列证据及 Rust 连接等待证据，其余传输竞态在对应装配步骤用可控接收器补齐。遇到旧请求在撤销后首次外传时，应修复实际传输屏障，不能改成“SDK shutdown 尽力处理”。

如果实际平台发现所选库不满足已批准语义，先在当前步骤修复或修订具体适配，不能带着不成立的前提继续铺开。新的产品取舍才需要交还用户；正常编译、API适配和故障修复在获授权的实现范围内自主完成。
