# 遥测语义技术方案

2026-10-28。用户已批准本文所述技术方案，并明确跳过独立验收设计，进入[实施准备](packet.md#实施顺序)。本文描述待实现的方案，不是当前运行行为或实现完成声明。当前契约仍由[遥测装配](../../docs/design/telemetry.md)维护，后续实现时应更新该文件及其消费者。本文不另建长期规范。

## 归属与操作边界

保留 service、instrumentation scope、span 三个不同维度。`service.name=velune.{platform}` 表示运行的客户端种类，`service.instance.id` 区分进程实例；scope 表示埋点实现来自 native 还是 Rust；span 表示一次有起止时间的操作。它们不是三层业务调用，也不要求一个 scope 对应一个 service。远控两端属于各自客户端的 service，不能为了凑出服务拓扑而把 bindings、application、gateway 虚构成独立服务。

沿用 `velune.client.native` 和 `velune.client.rust`，Rust 日志桥接仍使用 `velune_telemetry` scope。当前没有独立发布或独立控制各领域埋点的需求，因此不再按 UI、FFI、Harness 拆 scope。查询使用 resource、操作名和事件名，不假定 Rust 三信号的 scope 字符串完全相同。

下面给每种操作明确的观察对象。每个定义只有一种 kind；父上下文是否来自远端则是另一个参数，不能通过 `remote: bool` 推导 kind。

| 操作 | Kind | Owner 与起止含义 |
| --- | --- | --- |
| `ui.application.read/write`、`ui.configuration.read/write`、`ui.conversation.read/write`、`ui.turn.send/cancel/compact`、`ui.interaction.reply` | INTERNAL | 原生调用入口至本次调用或等待结束；不包含后续绘制，也不代表后台轮次完成。调用处显式选择类别。 |
| `bindings.<既有静态操作名>` | INTERNAL | bindings 入口至 application 调用返回。具体方法仍可区分，但名称有领域前缀，不再把 `list` 等裸名称当全局身份。 |
| `remote.request`、`remote.query_request` | CLIENT | 发起一轮 Submit／Query RPC，至收到回复、通信失败或本轮超时。每次查询是独立 RPC。 |
| `remote.submit`、`remote.query` | SERVER | 鉴权并验证 carrier 后，执行受理或查询 handler，至确定实际 Reply。时长不包含此前读帧和此后写回网络。 |
| `remote.execute` | INTERNAL | 首次受理后派生的本地执行任务，至 handler 的实际执行结果确定；即使控制端停止等待，仍按被控端生命周期完成。 |
| `execution.turn` | INTERNAL | Session owner 开始观察轮次，至明确终态；是否真正开始由专用事件表达。 |
| `harness.prepare/start/stop` | INTERNAL | application 调用对应 Harness 边界的持续时间，不声称观察到外部 Harness 内部步骤。 |
| `gateway.request` | SERVER | 本机 Gateway 收到 HTTP 请求，至拒绝结果形成，或响应 body 完成／中断／被丢弃。body 完成不证明客户端已收到全部字节。 |
| `gateway.attempt` | CLIENT | 对一个已选上游执行一次逻辑请求，包含该次适配和发送准备，至该次响应消费结束或失败。HTTP 和 CLI 都属于对外调用，但仅实际 HTTP 响应记录 HTTP 状态。 |
| 代际切换后的 `execution.result`、`gateway.request.result`、`gateway.attempt.result` | INTERNAL | 当前 SDK 报告旧操作结果的短操作；它不是重新执行，也不是原工作全程的 span。原耗时放在结果字段中。 |

原生类别使用各生态的枚举，在真实调用处显式指定。现有 effect 可以帮助核对读写边界，但不能代替操作身份；删除根据函数名猜测类别和“未知一律 read”的默认分支。通用 application.write 是明确选择的类别，不是遗漏分类时的兜底。

Gateway attempt 选择逻辑 CLIENT 边界，是因为同一上游请求可以通过 HTTP 或 CLI 完成，排障首先需要区分选中了哪个传输类别、哪次尝试失败。当前不再为每个适配步骤新增 span，也不把 CLI 标成 HTTP。此定义不会宣称覆盖完整的 HTTP semantic conventions；外部服务没有传播或接续 context 时，拓扑中也不会凭空出现服务节点。

## 远控因果与生命周期

一次典型调用的父子关系如下，其中同机分支没有 remote 节点：

```text
控制端 ui.turn.send
└─ bindings.send_turn_with_selection
   ├─ remote.request [CLIENT]
   │  └─ 被控端 remote.submit [SERVER]
   │     └─ remote.execute [INTERNAL]
   │        └─ execution.turn [INTERNAL]
   │           └─ gateway.request [SERVER]
   │              ├─ gateway.attempt [CLIENT，失败]
   │              └─ gateway.attempt [CLIENT，成功]
   └─ remote.query_request [CLIENT]
      └─ 被控端 remote.query [SERVER]
```

这表示因果关系，不要求父 span 一直存活到所有子操作结束。`remote.submit` 把自身已生成的 carrier 交给新执行任务；任务以本地父上下文继续，不再直接把收到的远端 carrier 当父节点。只有首次受理创建 execution，重复 Submit 返回账本状态。查询、重试与执行依靠同一个 RequestId 关联，不能把后续查询重新挂到执行 span 下，假造一次调用树。

上一轮曾建议执行任务使用 CONSUMER；此处修正为 INTERNAL。当前是 handler 派生本地任务，没有独立的消息消费契约。SERVER 负责跨进程受理，INTERNAL 负责后续本地执行，足以表达实际因果。

`serve_connection` 的外层超时不能终止已经开始的 `spawn_blocking` handler。因而不新增跨线程的完成状态来承诺 SERVER 覆盖完整网络往返；它按真实 handler 结束，CLIENT 按实际等待结束。两者可能有不同结果，这正是需要保留的事实。跨设备时间受时钟偏差影响，不能用两个设备的时间戳相减来推算网络延迟。

鉴权之前不接受对方提供的父上下文；无父、旧代际以及无法确定唯一轮次归属时保留已有的 correlation gap。继续使用官方 W3C propagator，保留 sampled=0，禁止 baggage。当前链路均有单一触发者，不为异步本身新增独立根或 links；未来遇到多个真实来源时，应在对应领域定义 link 的含义。

`Operation` 保持一个完成 owner，终态只提交一次。调用结束时及时释放 span；后台操作持有自己的 Operation，通过 carrier 关联，不能靠克隆父 span 延长其时长。`complete` 只记录结果而 span 直到最后一个引用释放才结束的现状，需要在实现中把结束与 owner 释放放在同一明确边界。异步取消后只按已知事实记录 cancelled 或 unknown，Drop 不能伪造业务成功。

## 事件与受控字段

新事件统一通过带 trace/span context 的日志记录输出。Rust trace layer 接收批准的 span，日志 bridge 接收批准的命名事件；同一次事实不再同时导出为 span event 和 log。普通本地 tracing 日志继续留在本地，不因此获得外传许可。

| 标准 eventName | 触发条件 |
| --- | --- |
| `velune.operation.completed` | 普通操作 owner 得到终态；包含该观察边界的结果及耗时。 |
| `velune.execution.turn.started` | Session 已实际接纳发送，而非 UI 点击或准备开始。 |
| `velune.execution.turn.finished` | 已开始的轮次到达 owner 可知的终态。 |
| `velune.execution.turn.not_started` | 观察结束，但轮次尚未实际开始。 |

轮次专用终态事件替代该 Operation 的通用 completed 事件，不再对同一次终态发两条完成日志。已知 failure 使用 ERROR，其余结果和 started 使用 INFO；原始异常详情留在本地。日志 body 仅供阅读；程序不解析 body、源码文件名或行号来识别事件。启用观测沿用 `telemetry.activated` 操作及通用完成事件，并在查询中明确排除业务统计，不额外创造一套启动事件。

保留当前成功读取和高频轮询的降噪原则，并将例外按明确操作定义列在权威文档中；失败、取消和未知仍输出结果。`remote.query`／`remote.query_request` 正常返回账本状态时不逐轮发通用成功日志，trace 仍遵循采样，指标仍按真实 RPC 计数。关键轮次终态不受 trace 采样影响。未收到事件可能是进程退出、停用、队列丢弃或查询范围造成，遥测不是可靠的业务账本。

自定义字段统一放在 Velune 命名空间，标准字段使用其标准含义。建议 v2 的基本字段如下：

| 字段 | 类型、用途与约束 |
| --- | --- |
| `velune.operation` | string，静态操作名，与所属 span 的语义名称一致；结果补报同时保留原操作名用于解释工作。 |
| `velune.operation.id` | string，一次被观察操作的随机身份，结果补报沿用；只进入 trace/log。 |
| `velune.outcome` | string，success／failure／cancelled／unknown；started 事件不提前写终态。 |
| `velune.operation.duration` | double，秒，完成日志中的原操作耗时。trace 持续时间仍由 span 起止表达。 |
| `velune.request.id` | string，现有受控 RequestId，关联重复受理、查询与执行；只进入 trace/log。 |
| `velune.correlation.gap` | string，受控枚举，保留缺父与 retired_generation 等已知缺口。 |
| `error.type` | string，仅已知失败使用，采用受控错误分类；没有可用细分时为 `other`，不传原始错误文本。 |
| `http.response.status_code` | integer，只记录实际已知的 HTTP 响应码，不给 CLI 或 QUIC 编造状态。 |
| `velune.gateway.transport` | string，http／cli，仅 Gateway 对外尝试使用。 |

结果补报的 span 名仍为 `*.result`，`velune.operation` 保留原工作名称，因此它是上表名称一致规则的明确例外。补报自己的短 span 时长不能替代 `velune.operation.duration`。原操作 ID 不等于 trace ID 或 RequestId，查询不能混用三种身份。完成日志直接包含已知的 operation ID、RequestId、结果与缺口，不能假定日志 bridge 自动复制 span 的属性；否则未采样时仍无法独立解释结果。

错误分类沿用现有 invalid、unsupported、io、contract、closed、unavailable、provider_import、history、runtime_management、conflict、unauthorized、connection、other，并增加明确的 timeout。unknown 的原因由 outcome 与关联缺口解释，不使用 `error.type=outcome_unknown` 把未知改称失败。由实际 HTTP 错误响应导致的 failure 可使用状态码十进制字符串作为 error.type，取值限定在有效错误状态范围；不拼接服务返回的消息。这些受控值与对应失败指标保持一致。

各信号的允许字段、类型和取值范围一同更新。Rust 的 `otel.name`、`otel.kind` 是 SDK 控制字段，只在批准 span 的固定入口设置；白名单不能因引入这些字段而静默拒绝整个 span。事件名通过固定调用点的 `tracing!(name: ...)` 设置，避免建立运行时字符串转换注册表。原生端直接使用各自 SDK。对新增隐私字段仍采取显式准入，不开放任意属性字典。

## 结果、错误和计数

结果必须针对观察边界解释。RPC 超时证明本轮通信失败，使用 failure 和有界的 timeout 分类；提交后的业务结果若尚不确定，外层等待或执行查询仍保持 unknown。`remote.query` 正常读到 Running、Unknown 或 Expired，表示查询处理成功，不能据此宣布业务成功或失败。取消请求被受理，也不等于轮次已经取消。

Gateway HTTP SERVER 正常形成 4xx 时保留 HTTP 状态，默认不将其视为 server 错误；在这个边界，success 仅表示正常处理并形成响应。5xx、上游中断或内部失败则为 failure，即使响应 body 已完整输出。该选择遵循 [OTel HTTP 状态规则](https://opentelemetry.io/docs/specs/semconv/http/http-spans/#status)。对外 attempt 按该上游请求的成功条件判断，失败后重试成功只影响失败的 attempt，最终 gateway.request 仍可成功。正常取消使用 cancelled；缺少结论使用 unknown。仅 failure 设置 span ERROR，并输出 `error.type`；success 通常保持 UNSET，cancelled／unknown 不自动等价于 ERROR。业务是否完成应查看 execution owner，不能单凭 HTTP 状态下结论。

保留现有指标名称、秒单位和桶边界，规范其标签为 `velune.operation`、`velune.outcome`；错误分类可在失败的 counter／duration 上使用相同的有界 `error.type`。operation ID、RequestId、trace ID、账户、路径和原始错误不得成为标签。各层计数分别回答各层问题，不能把 UI、bindings、RPC、attempt 的数量相加当作轮次数。业务轮次从 execution.turn 的 owner 统计，attempt 次数属于 Gateway。

代际切换后的 result 只补报结果，不再次增加操作计数或 duration histogram；旧代际被撤销后未投递的指标允许缺失，不能伪装成无损迁移或通过补报制造双计数。补报日志明确保留原耗时和 retired_generation。继续沿用 active_turns、owner_available、observation.age 的真实状态含义，不为关闭、未装配 Core 或无法观察的情况填零。

头部采样、队列上限、移动端生命周期与停用行为保持现状。新增语义应先减少重复事件和无意义轮询日志，不先引入尾部采样、长期队列、Collector 重写或另一套遥测设施。

## 实现接口与固定依赖

保留 `ClientTelemetry` 对配置、身份和 SDK 代际的所有权，保留 `Operation` 对受控生命周期的所有权。只补足明确 name、kind、父上下文来源与完成策略所需的窄接口：普通操作和执行轮次具有有限的终态事件策略，result 补报不计量。具体静态操作定义靠近其语义 owner；不集中维护所有业务函数的巨大映射，也不增加跨语言代码生成层。

Rust 用 tracing-opentelemetry 已支持的 `otel.name` 让导出 span 使用真实语义名称，并显式设置 `otel.kind`。bindings 可在已有静态调用宏中加 `bindings.` 前缀。原生入口接收枚举，四端遵守同一份操作定义；各 SDK 的 context 传播、记录和 exporter 继续直接使用官方能力。

已核对固定版本，以下能力不需要升级依赖。这些是 API／源码能力证据，尚不是修改后四端实际输出的验收证据。

| 生态 | 固定版本与核对结果 |
| --- | --- |
| Rust | OpenTelemetry 0.32.0，tracing-opentelemetry 0.33.0；桥接层以 tracing metadata.name 设置日志 eventName，显式 name 可避免文件行号身份；支持 `otel.name`／`otel.kind`。 |
| Swift | Core 与 exporter 2.6.0；本地依赖源码中 `LogRecordBuilder.setEventName`、SDK record 和 OTLP adapter 连通。 |
| Java | 1.62.0；固定版本 LogRecordBuilder 有 setEventName，已解析 exporter JAR 的日志 marshaler 使用 getEventName。 |
| .NET | 1.20.1；固定版本 OTLP serializer 将 `EventId.Name` 写入 protobuf EventName，可沿用 ILogger。 |

主要改动归原有 telemetry、bindings、remote、application、gateway 及原生 coordinator／调用入口。Rust 日志 bridge 的字段筛选与 span context 仍需检查真实导出；不能仅因两层 filter 分开，就宣称关联已正确。

## 版本、查询与文档演进

本轮属于一次遥测契约变更：新增 resource 属性 `velune.telemetry.schema.version="2"`，缺失该属性的既有数据按 legacy v1 解释。四端 native 与 Rust 设置相同标记；它不属于用户配置或分享内容，也不改变 FFI carrier 协议。应用版本继续保留，但不能单独代替遥测契约版本。当前不填写一个没有真实 schema 文件的 SchemaURL。

一次更新只写 v2，不做永久双写。滚动更新期间允许不同设备使用不同契约；同一 trace 跨版本时逐 span／log 按所属 resource 解读，不能只检查根节点。常用新查询选择 v2；历史查询显式保留 legacy 字段与名称的读取分支，不改写原始证据。操作名和标签变更后的指标不能跨版本盲目聚合，即使 metric 名称未变。

同步维护现有开发查询入口、OTLP 证据解析器、示例与断言，并在已有 `svc dev` 入口下复用服务。查询输出应先给出实例、run ID、时间范围、契约版本与查询限制，再沿 trace、operation ID、RequestId 回答受理、执行、尝试与结果；指标用于判断频率和趋势。使用原始字段保留可追溯性，不建设一个自动猜测旧语义的归一化服务。

权威文档以三条实际链路作为扩展范例：同机调用、跨设备异步执行、失败后重试成功。每个范例说明 owner、起止、事件触发、字段、失败与缺失含义。未来改变名字、kind、单位、结果或触发条件，都需要同步查看消费者并说明兼容影响；只移动源码不能改变事件身份。适用的标准 convention 可以逐步采用，但当前只承诺本文明确选择的字段与边界。

## 证据与剩余阶段

本方案建立在现有源码、历史合成 OTLP 数据、固定依赖源码及[调研记录](../otlp-observability/maintainability-research.md)上。远控边界另经 advisor 核对：采用其关于 spawn_blocking 与本地执行 kind 的判断，但没有采用“SERVER 覆盖网络回复”的扩展建议，因为已确认目标不需要新增跨线程终止状态管理。当前工作区存在其他任务的源码修改，本轮只维护本任务文件。

设计依据包括 [OTel 操作定义指导](https://opentelemetry.io/docs/specs/semconv/how-to-write-conventions/)、[SpanKind](https://opentelemetry.io/docs/specs/otel/trace/api/#spankind)、[错误记录](https://opentelemetry.io/docs/specs/semconv/general/recording-errors/)、[命名事件](https://opentelemetry.io/docs/specs/semconv/general/events/)和[遥测稳定性](https://opentelemetry.io/docs/specs/otel/telemetry-stability/)。这些资料帮助选择语义，不代替项目自己的契约。固定 API 证据见 [Rust 特殊字段](https://docs.rs/tracing-opentelemetry/0.33.0/tracing_opentelemetry/#special-fields)、[Java 1.62.0 LogRecordBuilder](https://github.com/open-telemetry/opentelemetry-java/blob/v1.62.0/api/all/src/main/java/io/opentelemetry/api/logs/LogRecordBuilder.java)、[.NET 1.20.1 OTLP serializer](https://github.com/open-telemetry/opentelemetry-dotnet/blob/core-1.20.1/src/OpenTelemetry.Exporter.OpenTelemetryProtocol/Implementation/Serializer/ProtobufOtlpLogSerializer.cs)。

用户已确认技术方案并跳过独立验收设计。后续按任务包中的实施顺序推进，以业务边界审查语义合理性，并用构建和实际输出核对实现是否表达了约定；不另设扩展练习或盲诊验收阶段。方案现已实现，具体检查结果和边界见任务包；未执行设备验收，未升级依赖或改动日常遥测配置。
