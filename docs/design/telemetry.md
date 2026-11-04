# 遥测装配与关联

本文件维护平台 app、bindings、application、remote、gateway 共同遵守的遥测契约。产品行为归[客户端遥测](../prd/diagnostics.md#客户端遥测)，开发入口归[开发与复验](../development/telemetry.md)。

按需读取：[本机配置](#本机配置与双-sdk) · [关联与指标](#关联事件与指标) · [证据与分享](#agent-证据与分享) · [埋点与查询](#扩展埋点与读取范例) · [本地问题历史](#本地问题历史)。

## 本机配置与双 SDK

`packages/telemetry::ClientTelemetry` 是每个客户端进程的稳定 owner，不随窗口、远端选择或执行会话重建。它拥有私有版本化配置、修订和 Rust SDK 代际；原生 coordinator 拥有同一实例身份与配置下的原生 SDK。两侧都使用官方 SDK，没有全局 provider 替换、自动资源探测或通用自动埋点。

配置位于本机 home 的 `telemetry/config.json`，与远端 application 的普通配置分开。Unset、Off、On 分别表示未设置、明确关闭、明确开启。开发 profile 只为 Unset 提供本次进程覆盖；Off 优先。普通启动没有接收端、采集 provider 或发送队列。配置原子写入，Unix 使用私有目录和 0600 文件，Windows 使用当前用户的保护 DACL。移动端使用排除备份的应用私有目录。未来或损坏格式保留原文件并停用，不能套用普通业务配置的重置规则。

地址只允许 HTTP(S) gRPC origin，不允许 URL 认证、非根路径或查询参数。最多 16 个 ASCII metadata 字段，名称最多 64 字节、值最多 4096 字节，profile 最多 16 KiB；禁止传输保留字段和 `grpc-*`、`*-bin`。采样率必须是 0–1 的有限数。入口统一验证，SDK 的显式参数与批准的 metadata 排除 `OTEL_*` 的自动覆盖。HTTPS 使用平台证书验证。Android 的 Network Security Configuration 为应用级 HTTP 支持，不能声称只在遥测请求中放行。

更新顺序是准备、提交、撤销旧代、激活、确认。准备只建立未接事件源的材料；提交先完成持久化，再撤销 Rust 旧许可。原生协调器随后撤销旧传输并清理旧 provider，创建新 provider，最后确认 Rust。保存失败不改变旧代；提交后激活失败时停发并显示未应用，不能恢复旧目的地。配置修订只有原生与 Rust 一致确认后才显示应用。启动时遥测失败保留业务可用性；状态明确报告未接入。

Rust 的同一许可锁覆盖连接接纳和 TLS 下的 socket I/O。Swift 关闭专属 gRPC channel；Java 的 exporter 发送准入与 shutdown 共享排序锁；.NET 取消并释放专属 HttpClient。撤销后不迁移旧批次，已经发送的数据允许晚到。正常退出有界 flush，移动端不增加后台保活。构造中途失败也必须清理已注册的监听器、reader 和 exporter。

## 关联、事件与指标

原生与 Rust 共用 `service.name=velune.{platform}`、版本、进程随机 `service.instance.id`、平台、environment 与可选开发 run ID。service 表示客户端种类，instance 区分进程，scope 表示埋点来源，span 表示一次操作；业务模块不另拆 service。Rust trace/metric scope 是 `velune.client.rust`，日志 bridge 是 `velune_telemetry`，原生 scope 是 `velune.client.native`。不使用账户、设备名或公钥作为资源身份。

所有正式 SDK resource 带 `velune.telemetry.schema.version="2"`。无该标记的历史记录按 v1 解释；一次 trace 可以经过不同版本的设备，必须逐条按所属 resource 读取，不能只看根节点。新版本只写 v2；旧数据不重写，查询时显式选择版本。显式连通性 probe 仍属于独立的测试 scope，不代表正式埋点契约。

| Span | Kind | Owner 与结束边界 |
| --- | --- | --- |
| `ui.application.read/write`、`ui.configuration.read/write`、`ui.conversation.read/write`、`ui.turn.send/cancel/compact`、`ui.interaction.reply` | INTERNAL | 原生调用者；本次调用或等待结束，不含绘制与后台轮次持续时间。类别在调用处显式选择。 |
| `bindings.<静态方法名>` | INTERNAL | bindings；application 调用返回。方法名用于精确定位，原生类别用于四端比较。 |
| `remote.request`、`remote.query_request` | CLIENT | 控制端；一轮 Submit／Query RPC 收到回复、失败或超时。 |
| `remote.submit`、`remote.query` | SERVER | 被控端；鉴权和 carrier 验证后的受理／查询 handler 形成 Reply，不含读帧与网络写回。 |
| `remote.execute` | INTERNAL | 首次受理派生的本地任务；handler 实际完成。它没有独立消息消费契约。 |
| `execution.turn` | INTERNAL | Session；从观察开始到明确终态，实际开始另发事件。 |
| `harness.prepare/start/stop` | INTERNAL | application；对应外部 Harness 调用返回，内部执行仍不可见。 |
| `gateway.request` | SERVER | Gateway；拒绝结果形成，或 body 完成、中断、丢弃。 |
| `gateway.attempt` | CLIENT | Gateway；一个已选上游的逻辑请求，含该次准备，直到响应消费结束或失败；传输类别为 http／cli。 |
| `execution.result`、`gateway.request.result`、`gateway.attempt.result` | INTERNAL | 当前代 SDK；短暂报告旧操作结果，保留原耗时，不计作新工作。 |

原生调用类别不依赖函数名猜测或默认 read。Mac 的 OperationEffect 仍负责关闭准入与配置通知，不由遥测类别取代。新操作应先确定自己观察的边界，再选择名称与 kind；父上下文是否 remote 不决定 kind。

原生任务按本生态传播 context，FFI 显式传递 W3C traceparent/tracestate。调用视图不可变，只附加本次 carrier，共享底层关闭状态；释放视图不关闭应用。显式 shutdown 后，原对象和已有视图均不可继续调用。remote Submit/Query 在鉴权后接受 carrier。首次受理的 server span 将自身 carrier 交给本地 execute；重复 Submit 不另建执行任务，Query 使用各自 RPC 父节点并以 RequestId 关联执行；不传播 baggage，不让关联字段开启接收端遥测。

执行轮次由实际 Session owner 保留到终态，发送受理与轮次成功分别记录。实际发送成功后才发 `velune.execution.turn.started`；终态为 `velune.execution.turn.finished`，尚未开始的观察使用 `velune.execution.turn.not_started`。取消被拒绝不能留下取消标志，运行时失败优先于取消意图。控制端退出不结束被控端工作。Gateway 在请求接纳时捕获所属轮次，每次尝试分别记录；多个工作共用 runner 且归属不唯一时保留缺口，不能猜测父节点。

后台推进与新请求读取当前代际。旧 span 保持原代际；切换后的执行／Gateway 结果使用当前代的 result 操作，保留原 operation ID、原耗时和预先保存的 W3C context，标明 `retired_generation`。result span 只表示补报的短时过程，`velune.operation` 指原工作，`velune.operation.duration` 表示原耗时；这与通常名称一致的规则有明确区别。补报不再增加 counter 或 histogram，旧代际撤销后的指标允许缺失。不能在旧 SDK 关闭后才生成 context，也不能宣称切换无损。

每个 Operation 只有一个完成 owner。完成后释放自己的 span，后台子操作通过 carrier 关联，不克隆父 span 来延长时长。受控 tracing span 不采用隐式 registry 父引用，避免跨 dispatcher 析构旧父引用。父 span 可以先于子操作结束；网络超时不会终止已运行的阻塞 handler，因此 CLIENT 等待失败与 SERVER 随后成功可以同时成立。

命名业务事件只导出为关联 trace 的日志；trace layer 仅导出批准 span，不再重复生成 span events。本地完整诊断仍留在本地。标准 eventName 为 `velune.operation.completed` 和上述三个轮次事件；轮次专用终态替代通用 completed，不对同一终态重复记录。failure 使用 ERROR，其余结果与 started 使用 INFO。body 仅供阅读，不能解析源码行号或消息文本识别事件。成功 snapshot、snapshot_update、authentication_poll 和 remote 查询不产生常规完成日志；失败、取消和未知仍记录。原生成功 application.read／conversation.read 同样降噪。

| 属性 | 类型与含义 |
| --- | --- |
| `velune.operation` | string，静态操作名；补报保留原操作名。 |
| `velune.operation.id` | string，随机操作身份，补报沿用；不是 trace ID 或 RequestId。 |
| `velune.outcome` | string，success／failure／cancelled／unknown；started 不提前写终态。 |
| `velune.operation.duration` | double，完成日志中的耗时，单位秒。 |
| `velune.request.id` | string，受控 RequestId，关联受理、查询、执行。 |
| `velune.correlation.gap` | string，missing_parent／retired_generation 等已知缺口。 |
| `error.type` | string，仅 failure 使用受控类别；HTTP 错误可用状态码字符串，其余为 invalid、unsupported、io、contract、closed、unavailable、provider_import、history、runtime_management、conflict、unauthorized、connection、timeout、other。 |
| `http.response.status_code` | integer，实际已知 HTTP 状态；CLI 适配元数据不算 HTTP 响应。 |
| `velune.gateway.transport` | string，http／cli，仅上游尝试。 |

Rust 只外传固定 target、固定事件名及受控字段。`otel.name`／`otel.kind` 是固定入口的 SDK 控制字段，不开放任意属性字典。完成日志直接包含已知关联字段，不能依赖日志 bridge 复制 span 属性。原始错误、参数、正文和路径不进入此层；RequestId 和 operation ID 不成为指标标签。

结果按观察边界判断。RPC 超时是本次通信 failure，尚不确定的后台业务仍为 unknown；正常查询到 Running、Unknown 或 Expired 表示查询成功。Gateway server 正常形成 4xx 时不算服务端失败，5xx 或响应中断则为 failure，即使错误响应已完整输出。attempt 失败后重试成功，只保留该次尝试的 failure，整体请求可为 success。仅 failure 设置 span ERROR 与 error.type；取消或未知不自动等于错误。HTTP 传输完成不证明轮次成功，业务终态必须查看 Session owner。

| 范围 | 计数／时长 owner | 含义 |
| --- | --- | --- |
| 原生调用及等待 | `velune.ui.operations`、`velune.ui.operation.duration` | 固定操作类别与 success/failure/cancelled/unknown；从原生调用入口计时，不代表渲染耗时 |
| Mac 请求队列 | `velune.ui.queue.wait` | `DeviceStore`／`ConversationStore` 入队至开始执行；出队时使用同一客户端 owner 的当前代，不另计业务次数 |
| Rust 用例与执行边界 | `velune.client.operations`、`velune.client.operation.duration` | operation 区分 bindings、remote request/query/execute、execution turn、harness 与 gateway attempt；result 不重复计量 |
| 双 SDK 代际启用 | operation=`telemetry.activated` | 每次成功启用各一条，不计为业务轮次完成 |

计数和 histogram 使用 `velune.operation`／`velune.outcome` 及失败时的有界 `error.type` 标签；不同层不能相加当作业务轮次数。计数和 histogram 不依赖 trace 采样；时长单位为秒，显式桶覆盖 5 ms 到 60 s。失败、取消与结果未知保留区别，连接超时不证明业务失败。SDK 使用 512 条批队列、64 条批次、2 秒批延迟与 10 秒指标间隔；这些是有界资源设置，不是生产性能 SLO。

四端使用官方 `ParentBased(TraceIdRatioBased)`。`sampleRatio` 控制没有有效父上下文的新链路；已有父上下文时继承其采样决定，包含 FFI 和跨设备边界。0% 仍可能接续已采样链路，100% 也不把未采样父节点改成已采样；有效 sampled=0 carrier 必须继续传播，不能丢弃后另起新根。0% 不等于关闭，严格停止采集由本机 Off 开关保证。

.NET SDK 会省略未采样本地父级下的子 Activity；此时使用当前父 Activity 的上下文交给官方 propagator，不手工另起根，也不修改或结束该父 Activity。关闭分支始终不传递 carrier。该行为依据固定版本的[官方实现](https://github.com/open-telemetry/opentelemetry-dotnet/blob/core-1.20.1/src/OpenTelemetry/Trace/TracerProviderSdk.cs)。

`velune.execution.active_turns` 来自 Core 当前与停放 Session 的真实 turn 状态，跨 SDK 代际保留，并在重新启用时立即可读。它只统计会话自己的轮次；0 不表示没有后台子 Agent。`velune.execution.owner_available` 和 `velune.execution.observation.age`（秒）说明 owner 是否仍可观察、最近观测是否过期；SDK 回调只读取已发布快照，不回调或锁住 Core。尚未装配本地 Core 的手机不伪报 0。

## agent 证据与分享

`telemetry/status.json` 每 15 秒刷新，包含 PID、实例、run ID、修订、两侧应用状态、执行快照、采样和有限传输反馈，不含认证值。45 秒以上或不属于所查 run ID 的快照视为过期。`rpc_completed` 只说明 SDK RPC 完成；官方 SDK 可能不将 partial rejection 反映为失败。显式三信号测试使用官方生成的 OTLP 请求，scope 为 `velune.telemetry.test`，携带本机实例和可选 run ID，逐项检查回执。它测试所填连接参数，不启用 SDK，也不证明原生 SDK 已投递；后台是否可查询仍需查询证据。

精确实时队列深度与完整丢弃数没有统一的 SDK 公开接口，`droppedRecords=null` 表示未知。已知丢弃通过官方固定事件或 SDK 内部指标记录在本地 `telemetry/sdk-diagnostics.json`：只保留实例、原修订、native/rust、信号与数量，最多 64 条，并报告淘汰数。Rust 与 .NET 读取 shutdown 汇总；Swift 与 Java 使用不连 OTLP 的本代 meter reader。Swift SDK 没有日志丢弃 hook，此处仍是不可见边界。诊断不收集 SDK 原始消息，也不递归进入 OTLP。

分享只返回保存的 profile；临时开发覆盖、开关、修订和身份均不复制。`TelemetryProfileRead` 与控制授权独立，邀请与核对材料绑定用途。读取在入站权限锁内与撤销排序，不进入业务账本或重试缓存。目的端取得后仍走本机准备／提交路径，保留自己的偏好与 expected revision；失败保留原配置。

agent 通过本地状态与接收端的 Tempo、Loki、Prometheus API 联合判断。报告必须限定 run、实例、时间与数量，说明采样、查询截断、保留期和外部 Harness 内部不可见范围。未找到数据不足以证明没有失败。

## 扩展埋点与读取范例

新增埋点先说明谁拥有起止或终态、它回答什么诊断问题，再使用已有 owner 和官方 SDK。普通同机调用是 `ui.* → bindings.*`；发送受理后 `execution.turn` 继续到 Session 终态，UI 返回不替它结束。跨设备调用在 bindings 后经过 `remote.request → remote.submit → remote.execute`；查询是独立的 client/server 对，以 RequestId 查找执行。Gateway 在同一 request 下记录每次 attempt，因此一次失败后重试成功应看到失败 attempt、成功 attempt 和成功 request，而不能把整个 trace 归为失败。

排障先限定 run、实例、时间与契约版本，再检查 trace 的父子、kind 和结果。没有采样到 span 时，以完成日志的 operation ID、RequestId 与终态字段继续查；缺失的 finished 不能推出仍在执行，需结合 owner_available、observation.age 与业务查询。跨设备时钟可能有偏差，不能将两端时间戳相减当网络延迟。

新增字段同时维护外传准入、类型、单位、指标基数和查询方。改变名称、kind、触发或结果含义属于契约演进，需要说明版本影响；普通源码移动不能改变 eventName。实际输出检查能发现表达偏差，但是否值得新增 span、操作边界是否清楚，仍需按业务语义和维护成本审查。


## 本地问题历史

Mac 问题面板是既有 `logs/velune.<UTC日期>.jsonl` 的跨进程投影，不按 run ID 过滤。`DiagnosticProblem` 带时间、来源、详情、错误类别／代码、阶段、operation ID 和恢复时间；新增、恢复、显式清理是同一 rolling writer 的日志事件，写入后 flush。恢复不删除记录，用户清理的记录在重启后不复活。旧 ERROR 与带详情的 WARN 可回填，重复 operation ID 收敛到完整问题记录，断裂或无效单行不遮蔽其他记录。旧日志缺少原始详情时只报告原有证据。保留沿用本地滚动日志的 7 个文件上限，不另建问题数据库或保留第二份错误日志；此限制与重启无关。

这些本地详细事件不进入 OTLP 导出。FFI 提供本地 `problems/record_problem/resolve_problems/dismiss_problems`，无需应用控制锁或连接其他设备；本机状态路径区分投影 scope，日志读写故障在面板明确显示，不递归写入。
