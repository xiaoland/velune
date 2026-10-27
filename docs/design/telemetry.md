# 遥测装配与关联

本文件维护平台 app、bindings、application、remote、gateway 共同遵守的遥测契约。产品行为归[客户端遥测](../prd/diagnostics.md#客户端遥测)，开发入口归[开发与复验](../development.md#开发遥测)，实测范围归[任务](../../tasks/otlp-observability/packet.md)。

## 本机配置与双 SDK

`packages/telemetry::ClientTelemetry` 是每个客户端进程的稳定 owner，不随窗口、远端选择或执行会话重建。它拥有私有版本化配置、修订和 Rust SDK 代际；原生 coordinator 拥有同一实例身份与配置下的原生 SDK。两侧都使用官方 SDK，没有全局 provider 替换、自动资源探测或通用自动埋点。

配置位于本机 home 的 `telemetry/config.json`，与远端 application 的普通配置分开。Unset、Off、On 分别表示未设置、明确关闭、明确开启。开发 profile 只为 Unset 提供本次进程覆盖；Off 优先。普通启动没有接收端、采集 provider 或发送队列。配置原子写入，Unix 使用私有目录和 0600 文件，Windows 使用当前用户的保护 DACL。移动端使用排除备份的应用私有目录。未来或损坏格式保留原文件并停用，不能套用普通业务配置的重置规则。

地址只允许 HTTP(S) gRPC origin，不允许 URL 认证、非根路径或查询参数。最多 16 个 ASCII metadata 字段，名称最多 64 字节、值最多 4096 字节，profile 最多 16 KiB；禁止传输保留字段和 `grpc-*`、`*-bin`。采样率必须是 0–1 的有限数。入口统一验证，SDK 的显式参数与批准的 metadata 排除 `OTEL_*` 的自动覆盖。HTTPS 使用平台证书验证。Android 的 Network Security Configuration 为应用级 HTTP 支持，不能声称只在遥测请求中放行。

更新顺序是准备、提交、撤销旧代、激活、确认。准备只建立未接事件源的材料；提交先完成持久化，再撤销 Rust 旧许可。原生协调器随后撤销旧传输并清理旧 provider，创建新 provider，最后确认 Rust。保存失败不改变旧代；提交后激活失败时停发并显示未应用，不能恢复旧目的地。配置修订只有原生与 Rust 一致确认后才显示应用。启动时遥测失败保留业务可用性；状态明确报告未接入。

Rust 的同一许可锁覆盖连接接纳和 TLS 下的 socket I/O。Swift 关闭专属 gRPC channel；Java 的 exporter 发送准入与 shutdown 共享排序锁；.NET 取消并释放专属 HttpClient。撤销后不迁移旧批次，已经发送的数据允许晚到。正常退出有界 flush，移动端不增加后台保活。构造中途失败也必须清理已注册的监听器、reader 和 exporter。

## 关联、事件与指标

原生与 Rust 共用 `service.name`、版本、进程随机 `service.instance.id`、平台、environment 与可选开发 run ID；scope 区分 native 和 Rust。Rust trace/metric scope 是 `velune.client.rust`，日志 bridge 使用批准 target `velune_telemetry`，原生 scope 是 `velune.client.native`。不使用账户、设备名或公钥作为资源身份。

原生任务按本生态传播 context，FFI 显式传递 W3C traceparent/tracestate。调用视图不可变，只附加本次 carrier，共享底层关闭状态；释放视图不关闭应用。显式 shutdown 后，原对象和已有视图均不可继续调用。remote Submit/Query 在鉴权后接受 carrier，执行线程接续同一因果；不传播 baggage，不让关联字段开启接收端遥测。

执行轮次由实际 Session owner 保留到终态，发送受理与轮次成功分别记录。实际发送成功后才发 `execution.turn.started`；终态为 `execution.turn.finished`，尚未开始的观察使用 `execution.turn.not_started`。取消被拒绝不能留下取消标志，运行时失败优先于取消意图。控制端退出不结束被控端工作。Gateway 在请求接纳时捕获所属轮次，每次尝试分别记录；多个工作共用 runner 且归属不唯一时保留缺口，不能猜测父节点。

后台推进与新请求读取当前代际。旧 span 保持原代际；切换后的执行／Gateway 结果使用当前代的 result 操作，保留原 operation ID、起始时间和预先保存的 W3C context，并标明 `retired_generation`。不能在旧 SDK 关闭后才生成 context。受控操作的 tracing registry span 不采用隐式父引用，因果只通过 W3C context 建立，避免跨 dispatcher 析构旧父引用。远控 Submit 客户端为 `remote.request`，结果轮询为 `remote.query_request`，执行端分别为 `remote.execute` 和 `remote.query`；查询成功只证明查询完成。

Rust 只外传 `velune_telemetry` 的 `client.operation` span 和批准事件，字段白名单为 operation、operation_id、outcome、event、elapsed_ms、request_id、correlation_gap、failure_kind。原始错误、参数、正文和路径不进入此层。RequestId 只用于 trace/log 关联，不能成为指标标签。

| 范围 | 计数／时长 owner | 含义 |
| --- | --- | --- |
| 原生调用及等待 | `velune.ui.operations`、`velune.ui.operation.duration` | 固定操作类别与 success/failure/cancelled/unknown；从原生调用入口计时，不代表渲染耗时 |
| Mac 请求队列 | `velune.ui.queue.wait` | AppStore 入队至开始执行；出队时使用同一客户端 owner 的当前代，不另计业务次数 |
| Rust 用例与执行边界 | `velune.client.operations`、`velune.client.operation.duration` | operation 区分 bindings、remote request/query/execute、execution turn/result、harness 与 gateway attempt |
| 双 SDK 代际启用 | operation=`telemetry.activated` | 每次成功启用各一条，不计为业务轮次完成 |

计数和 histogram 不依赖 trace 采样；时长单位为秒，显式桶覆盖 5 ms 到 60 s。失败、取消与结果未知保留区别，连接超时不证明业务失败。SDK 使用 512 条批队列、64 条批次、2 秒批延迟与 10 秒指标间隔；这些是有界资源设置，不是生产性能 SLO。

四端使用官方 `ParentBased(TraceIdRatioBased)`。`sampleRatio` 控制没有有效父上下文的新链路；已有父上下文时继承其采样决定，包含 FFI 和跨设备边界。0% 仍可能接续已采样链路，100% 也不把未采样父节点改成已采样；有效 sampled=0 carrier 必须继续传播，不能丢弃后另起新根。0% 不等于关闭，严格停止采集由本机 Off 开关保证。

.NET SDK 会省略未采样本地父级下的子 Activity；此时使用当前父 Activity 的上下文交给官方 propagator，不手工另起根，也不修改或结束该父 Activity。关闭分支始终不传递 carrier。该行为依据固定版本的[官方实现](https://github.com/open-telemetry/opentelemetry-dotnet/blob/core-1.20.1/src/OpenTelemetry/Trace/TracerProviderSdk.cs)。

`velune.execution.active_turns` 来自 Core 当前与停放 Session 的真实 turn 状态，跨 SDK 代际保留，并在重新启用时立即可读。它只统计会话自己的轮次；0 不表示没有后台子 Agent。`velune.execution.owner_available` 和 `velune.execution.observation.age`（秒）说明 owner 是否仍可观察、最近观测是否过期；SDK 回调只读取已发布快照，不回调或锁住 Core。尚未装配本地 Core 的手机不伪报 0。

## agent 证据与分享

`telemetry/status.json` 每 15 秒刷新，包含 PID、实例、run ID、修订、两侧应用状态、执行快照、采样和有限传输反馈，不含认证值。45 秒以上或不属于所查 run ID 的快照视为过期。`rpc_completed` 只说明 SDK RPC 完成；官方 SDK 可能不将 partial rejection 反映为失败。显式三信号测试使用官方生成的 OTLP 请求，scope 为 `velune.telemetry.test`，携带本机实例和可选 run ID，逐项检查回执。它测试所填连接参数，不启用 SDK，也不证明原生 SDK 已投递；后台是否可查询仍需查询证据。

精确实时队列深度与完整丢弃数没有统一的 SDK 公开接口，`droppedRecords=null` 表示未知。已知丢弃通过官方固定事件或 SDK 内部指标记录在本地 `telemetry/sdk-diagnostics.json`：只保留实例、原修订、native/rust、信号与数量，最多 64 条，并报告淘汰数。Rust 与 .NET 读取 shutdown 汇总；Swift 与 Java 使用不连 OTLP 的本代 meter reader。Swift SDK 没有日志丢弃 hook，此处仍是不可见边界。诊断不收集 SDK 原始消息，也不递归进入 OTLP。

分享只返回保存的 profile；临时开发覆盖、开关、修订和身份均不复制。`TelemetryProfileRead` 与控制授权独立，邀请与核对材料绑定用途。读取在入站权限锁内与撤销排序，不进入业务账本或重试缓存。目的端取得后仍走本机准备／提交路径，保留自己的偏好与 expected revision；失败保留原配置。

agent 通过本地状态与接收端的 Tempo、Loki、Prometheus API 联合判断。报告必须限定 run、实例、时间与数量，说明采样、查询截断、保留期和外部 Harness 内部不可见范围。未找到数据不足以证明没有失败。
