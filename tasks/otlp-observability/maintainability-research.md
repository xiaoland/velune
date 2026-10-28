# 遥测可维护性调研

2026-10-27。用户要求继续调查事件与分布式遥测设计，重点是后来者能否持续增加、理解和修改埋点。本文件记录资料、实际观察和建议，不是已批准的新实现契约。当前行为仍归 [遥测装配](../../docs/design/telemetry.md)，前一轮 service／scope／span 评审见 [任务记录](packet.md#后续埋点语义评审)。

## 判断与资料

可维护性的主要对象是产出数据与查询方之间的契约。名称之外，还需要明确观察的业务事实、发生时刻、结束条件、字段类型、错误含义和变更方式。下面区分资料的指导与对 Velune 的建议；不把外部文章的所有做法当作项目要求。

| 资料 | 有用的指导 | 对 Velune 的建议 |
| --- | --- | --- |
| [OTel：How to write semantic conventions](https://opentelemetry.io/docs/specs/semconv/how-to-write-conventions/) | 定义 span 的起止、逻辑调用与单次尝试、kind、错误与属性，而非只列名字。 | 在现有文档中为少数领域操作定义这些要素；UI 等待、RPC 受理、轮次执行、网关尝试分别由实际 owner 记录。 |
| [OTel：Events](https://opentelemetry.io/docs/specs/semconv/general/events/) | 事件是有明确名称的时点事实；定义触发条件、发生时间、严重程度和字段。持续操作使用 span，整段操作的性质使用属性。 | started 在实际接纳后记录；finished 在 owner 得到终态后记录。说明哪些事件可能重复、缺失或乱序，不能以未收到 finished 推断仍在执行。 |
| [OTel：Deprecating Span Events API，2026-03-17](https://opentelemetry.io/blog/2026/deprecating-span-events/) | 新事件推荐使用关联 trace 的日志；现有 span event 数据仍受支持，语言 SDK 逐步迁移。 | 选择一条命名事件出口，以标准日志字段表达，并保持 trace/span context。迁移前检查现有查询，不因文章发布就整体升级四端 SDK。 |
| [Stripe：Canonical log lines，2019-07-30](https://stripe.com/blog/canonical-log-lines) | 每个工作单元的终态记录汇总必要上下文，降低排障时拼接大量日志的成本。 | 对关键轮次保留可独立理解的结果记录；沿用现有 completion 能力，不再新增平行的完成事件。字段以诊断问题为依据，不照搬文章中的用户或凭据标识。 |
| [OTel：Recording errors](https://opentelemetry.io/docs/specs/semconv/general/recording-errors/) | 已由重试恢复的失败，不应污染最终成功操作；同一异常不要重复记录。对应 span 与 metric 的错误分类应一致。 | 分开 attempt 失败与总体结果；保留 cancelled／unknown 的业务区别。按固定分类采用适用的标准 error.type，并保留本地详情边界。 |
| [OTel：Messaging trace structure](https://opentelemetry.io/docs/specs/semconv/messaging/messaging-spans/#trace-structure) | parent 只能有一个，links 可以保留多来源或另一活动上下文；单消息可以使用直接父子。 | 借鉴其因果原则，不把 QUIC 协议伪装成消息中间件。单一触发保持 parent，多来源或另有当前操作时再用 link；不能一见异步就改成独立根。 |
| [Prometheus：Instrumentation](https://prometheus.io/docs/practices/instrumentation/) | 标签组合决定序列数量和资源成本；计数时机需要一致。 | 固定类别用于指标标签，operation ID／RequestId 用于日志和 trace。不同层的计数不可相加当业务轮次；文档写清 histogram 单位及统计边界。 |
| [OTel：Sampling](https://opentelemetry.io/docs/concepts/sampling/) | 头部采样无法保证保留未来会失败的全部 trace，尾部采样有额外运维成本。 | 维持现有父采样规则，关键结果日志与指标独立可见；查询报告继续说明样本范围。先测轮询噪声和数据量，不先引入尾部采样设施。 |
| [OTel：Naming](https://opentelemetry.io/docs/specs/semconv/general/naming/) 与 [Telemetry Stability](https://opentelemetry.io/docs/specs/otel/telemetry-stability/) | 优先复用标准概念；自定义名称使用自己的命名空间。遥测变更可能破坏查询；当前也不能依赖 schema 自动变换解决所有兼容问题。 | 应用专属属性／事件采用 velune 命名空间，标准字段保持标准语义。记录约定版本及迁移，更新查询、样例和测试；混合版本客户端必须能被区分。 |
| [OTel：Don't Wrap OpenTelemetry，2026-06-24](https://opentelemetry.io/blog/2026/dont-wrap-opentelemetry/) | SDK 配置助手与再造通用埋点 API 不同，后者会增加维护负担并遮蔽原生能力。 | 保留配置、代际和隐私 owner，让受控操作明确表达 name/kind；避免任意字典、动态 instrument 注册表或第二套通用日志 API。 |
| [OTel：Weaver](https://opentelemetry.io/blog/2025/otel-weaver/) | 可以检查约定变更，并验证实际产出的遥测是否符合定义。 | 借鉴“检查真正导出的数据”这一方式。当前先扩展已有 OTLP 夹具；只有定义规模与跨语言同步成本证明有收益，再评估 Weaver，不在本轮引入依赖。 |

Events、Recording errors 等页面当前标为 Development，社区博客也不是强制规范。可以采用其清楚的设计原则，但实现时应核对固定 SDK 能力，不承诺最新资料中的所有 API 已在四端可用。

## 对现有输出的补充检查

本轮读取 `target/telemetry-delivery-acceptance.wire.json`，只统计名称、字段和记录形态，没有运行设备或发送业务请求。该文件是历史合成验收数据，不能用来估计生产流量。

| 实际观察 | 对维护的影响 |
| --- | --- |
| 341 条 Rust 日志的标准 eventName 为 `event packages/telemetry/src/lib.rs:795` 等文件／行号名称；稳定业务名在自定义 event 属性中。 | 使用标准事件名查询时，普通代码移动就可能改变类别；新人必须知道私有字段约定。 |
| 26 条 native 日志没有 eventName，body 为 operation_completed。 | 同类完成记录在不同 SDK 中需要不同字段查询。该样本的 native 是 Apple，不将计数外推到所有平台。 |
| Rust 同时输出 321 条 span events 与 341 条日志记录，事件类别包含 operation_completed、execution.turn.started／finished。 | 当前同一事件入口进入 trace 和 log 两条路径；采样和代际导致数量不同，不能把这两个总数当作逐条重复率。 |

固定版本 `opentelemetry-appender-tracing 0.32.0` 的 `on_event` 直接以 `metadata.name()` 设置 eventName；其本地测试演示 `tracing` 的显式 `name:`。因此 Rust 事件名可以在产生事件时直接确定，不需要 Collector 解析文件名或额外事件转换层。[SDK 装配](../../packages/telemetry/src/sdk.rs) 的 trace layer 和日志 bridge 当前接纳相同事件，与上述实际数据一致。

建议把批准的 span 送入 trace layer，把命名业务事件送入日志 bridge，并保留 span 上下文以关联日志。原生端按同一事件契约输出。具体四端 API 映射尚未逐个验证；在落实之前不删除所有 completion 日志，因为它们仍可能提供未采样操作的结果。

`ClientTelemetry` 与 `Operation` 不应因“不要包装”的文章标题被整体移除。它们拥有启停、SDK 代际、受控字段、FFI 关联、计数时长和单次完成语义；这些职责在当前产品中有实际用途。应收窄的是把所有操作压成一个名字、通过 remote 布尔值猜测 kind、以及默认将未知 UI 操作归为读取的做法。advisor 独立复核了该边界及本地日志 bridge，实现建议仍待用户审阅。

## 建议的落地范围

先在现有遥测文档中维护少量领域定义与三个代表性例子，不新增一套平台。每个定义说明：诊断问题与 owner；span 起止或事件触发条件；名称、kind、outcome；字段类型、单位与基数；trace／RequestId 关联方式；采样与缺失时能得出的结论。定义应贴近调用处落实，新增字段时同步检查外传白名单，避免整条记录因字段不被批准而消失。

例如 `velune.execution.turn.finished` 可以作为建议事件名：由执行 owner 在确知终态时记录，具有业务 operation ID、明确 outcome、耗时及适用的有界错误分类，并携带可用的 trace/span ID。进程被强杀可能没有该记录，不能制造一个并不存在的 finished。受理、传输结束和轮次结束仍是不同事实；遥测记录也不是必须持久可靠的业务账本。

接着在已有夹具内检查实际 OTLP：四端同类事件名称和字段类型一致；名称不依赖源码行号；一条业务事件不会未经约定同时导出为 span event 和日志；重试后的总体结果、unknown、未采样失败仍可解释；异步关联和终态时间符合业务事实。最后保留少量可执行查询，让另一个 agent 从数据回答“是否受理、是否仍执行、哪次尝试失败、最终结果是否已知”。格式校验不能替代这一诊断验证。

当前没有新增运行时依赖、调整采样策略、改变默认关闭行为或修改产品源码。研究结论为下一轮技术收敛提供依据，并不表示上述建议已经实现或验收。
