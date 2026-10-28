# 分布式可观测性

## 目标、授权与边界

为 agent 提供跨客户端、FFI、QUIC、执行与受管网关的可诊断证据，同时支持生产 OTLP 三信号。默认未配置且不启用；每个客户端独立设置，可以经独立授权一次性复制保存配置。开发入口使用私有临时覆盖，不改变普通启动偏好。使用各生态官方成熟 SDK，不引入 LiteLLM、托管后台、全局自动配置或持续同步。

用户已同意方案并明确授权实现及验收；2026-10-27 又要求“整顿、清理一下，重新启动验收；别忘了你可以自由提交”。后续持续完成既定范围并可提交本任务改动，不重复申请阶段授权。用户进一步明确直接使用已安装的 Velune、可以覆盖更新；合成数据使用独立配置目录，不再安装产品副本或编写独立配对程序。真实凭据及其他任务改动不属于本任务。已按用户要求卸载 iPad 的 `app.velune.ios` 以释放免费签名名额，并覆盖更新 `/Applications/Velune.app`。

## 当前结论

2026-10-27 后续语义评审已完成：service 身份与当前 scope 可以保留，但 span 命名、原生操作分类和 kind 尚不适合作为新增埋点的范例。具体观察及建议见文末。用户随后要求进一步调查可维护性；[资料与应用判断](maintainability-research.md) 已整理事件、错误、因果、成本、演进和验证规则，并确认现有事件名依赖源码行号、原生与 Rust 事件字段不同，以及双路事件输出。本轮仅研究和检查既有数据，没有修改产品源码；以下建议尚未进入实现。已有初步接入验收结论保持原范围。

初步接入已经实现；本轮完成可用设备的关键补验并接入 SVC dev。Windows↔Android 的实际配置复制、撤销后明确拒绝与目的端原配置保持通过。Mac 和 Android 的新开发入口查到三信号，普通重启后临时配置不再生效；Windows 新入口也查到三信号，正常退出及普通重启后的双 SDK 关闭已观察。Android 的后台／强制结束／普通重启通过。已修复实际表单在本次失败时仍显示上次成功的问题，覆盖既有安装后真机复验通过。

原有真实双 SDK、FFI／QUIC／Core／Gateway、采样、代际撤销、请求结果未知、业务活跃数、三个独立 agent 诊断案例、Mac 表单保存及 Mac→iPhone Simulator 分享证据继续适用。Android 与 iPad 的真机系统证书拒绝已有证据。本轮不为未修改的 Rust 核心重复全量构建。

2026-10-27 晚间 iPad 恢复后，实际 iPad→Mac 核对／批准、完整 profile 复制、撤销后明确拒绝与目的端配置字节保持已通过。iPad 原始无 profile 的配置已恢复，并取得普通新进程双 SDK 关闭的证据。补验发现 Apple 共用设置页也会保留上次成功提示，已修复三处状态清理并覆盖更新 Mac 和 iPad。Mac 实际校验失败后的旧状态清除、iPad 新包表单与强制结束／普通重启均通过。四平台均已有实际来源与目的端入口证据，未穷举所有设备组合。长时间移动挂起也没有观察；短暂后台和强制结束不能替代该项。

## 实现与收尾

用户明确要求接入 SVC CLI，取代此前“不引入 SVC CLI”限制；文档和任务状态仍直接用 Markdown 维护。`svc.json` 声明 sfp7 LGTM、本机及 Windows SSH、Android ADB 四项能力。SVC 管理就绪与协调，SSH／ADB／Compose 保留实际生命周期；客户端启动器只负责临时 profile、应用启动与查询，没有加入产品运行依赖或另一套进程注册表。

独立产品架构审查未发现应推翻官方双 SDK、代际撤销或独立分享授权的具体问题。独立开发环境审查发现的两个问题已修复并验证：恢复上游时复用存活的 Windows SSH master，避免争抢端口；Android 转发失败仍启动业务。Mac 开发 home 改为系统私有短路径，解决实际 Unix socket 超限；移动入口默认复用已安装应用。manifest 使用 `workspaceRevision`，明确只记录启动工具工作区，不能冒充已安装产物版本。

Android 本次 UI 修复从 `1b342eb` 的原生快照加一行修复构建，复用匹配的 Rust 库和绑定，覆盖 `app.velune.android.telemetry`；APK 与 lint 通过。当前工作树其他任务已提交 compaction，另有本地配对 CLI、重新配对修复和 Codex 调查修改，本任务不夹带或回退这些修改。后续 Apple 修复从 `1b342eb` 原生快照加共用视图三处修改构建，保留 Mac 既有 CLI 接线并复用匹配的 native 库；Mac 和 iPad 均覆盖到既有安装，不声称整个并行工作树完成产品构建。

本轮验收脚本、运行 ID、构建来源与设备结果集中在[验收记录](verification.md)。sfp7 和本机查询通道保留供开发；测试客户端、Windows 计划任务和设备转发在收尾时停止，临时构建快照删除。iPad 已用 `target/telemetry-native-ui/ipad-ui-config-before.json` 恢复原始无 profile 状态；更新安装及强制结束后的新进程仍为 Unset／双 SDK 关闭。Mac 验收实例和临时原生构建快照已清理，Mac 日常入口重新启动。

## 环境与证据限制

本轮 iPad 和 Mac 均恢复可用并完成关键补验，没有可用 iPhone 真机。macOS 15.4.1 与 Xcode 26.2（包内最低 15.6），以及 iPadOS 27.0 超过其列出的 26.2 设备支持，属于官方支持范围外的组合。它是已知环境约束，尚未证明是自动化超时的唯一原因；不进行未经授权的系统升级。Apple 测试串行，复用构建，并关闭自动系统诊断收集。不得再为自有应用使用 CUA、Android／Windows 镜像串流或新的通用自动化控制器。

主要机器：sfp7 为 `sfp7-ws.localhost`；Windows 独立目录为 `C:\Users\yyh\velune-observability`；Android 真机 `HYJ4C19524006587`，既有测试包 `app.velune.android.telemetry`；iPad `4BC38BA3-E462-5248-A300-FB90F9FB0F47`，既有测试包 `app.velune.ios.telemetry`；Mac 使用 `/Applications/Velune.app`。详细运行 ID、状态、截图和原始接收数据在 Git 忽略的 target 目录，报告不输出认证值。

排查进程时曾误输出完整环境，使 NODE_AUTH_TOKEN 出现在工具输出；已通知用户并建议轮换，轮换尚未确认。后续不使用、不复述该值，只读取指定非秘密字段；启动器使用环境白名单。

## 知识与证据入口

- 已同意的 [技术方案](technical-design.md)、[实现计划](plan.md)和[验收判据](acceptance.md)。
- 当前结果、原始证据入口、已知失败与未验收项归[验收记录](verification.md)；工具与 SDK 可行性归[SDK 记录](sdk-evidence.md)。
- sfp7 版本、资源、访问策略、部署及查询实测归[部署记录](deployment.md)。
- 产品行为归[诊断 PRD](../../docs/prd/diagnostics.md#客户端遥测)，跨单元契约归[遥测装配](../../docs/design/telemetry.md)，操作入口归[开发文档](../../docs/development.md#开发遥测)，用户决定归[来源](../../docs/sources.md)。

实现与验收基线已提交：`1d4f512` 保存验收与部署材料，`08aa284` 为 Rust OTLP 基础，`1c77962` 为 FFI／远控／执行／网关关联，`4288414` 为四端原生接入和开发入口。`1b342eb` 收敛了已安装应用验收。本地配对 CLI／重新配对修复留在原任务工作树；compaction 已由原任务提交 `ba47054`。

本轮已提交 `a97d461`，接入 SVC dev 并补完当时可用设备；晚间 iPad 恢复后的最终证据见 `ipad-mac-final-share-evidence.json`、`ipad-status-fix-final-evidence.json` 与 `apple-status-fix-mac-evidence.json`，均位于 Git 忽略的 `target/telemetry-native-ui/`。

## 后续埋点语义评审

本轮的判断标准是：后来者只看一段现有埋点和导出的链路，能否正确判断谁在运行、谁产生了观察、正在做什么、等待哪一方，以及何时算完成。三种概念分别是 resource/service、instrumentation scope 和 span；它们不是固定三层调用栈。一条 trace 可以跨越多个 service 和 scope。

当前 `ResourceIdentity` 使用 `velune.{platform}` 和进程随机 instance ID，原生与 Rust 共用，见 [身份实现](../../packages/telemetry/src/lib.rs)。这符合当前平台客户端的部署边界；remote、Core 和嵌入式 gateway 不应仅因属于不同模块就变成不同 service。两台同平台设备以 instance 区分，不把用户设备名或安装身份编码进 service 名称。

当前 trace/metric scope 为 `velune.client.native` 和 `velune.client.rust`，Rust 日志 bridge scope 为 `velune_telemetry`。它们可以表示统一埋点实现的来源，但不能声称已按组件区分 scope。[旧方案](technical-design.md#一份本地配置两套-sdk) 曾描述 native／remote／application／gateway scope，与当前实现不同。建议暂不增加 scope 路由，先用稳定操作名表达职责；若组件将来自行维护 instruments、独立发布埋点版本或确有按组件配置的需求，再细分 scope，仍然共用 provider。advisor 独立审阅支持这一判断；scope 并非必须跟 SDK 或语言一一对应。

有三项需要在推广为范例前修正：

1. **Rust 导出名称没有区分操作。** [operation 入口](../../packages/telemetry/src/lib.rs) 把所有 span 命名为 `client.operation`，[SDK 白名单](../../packages/telemetry/src/sdk.rs) 也只接受这个静态 span。实际动作虽然保留在 `operation` 属性里，默认瀑布图和操作选择仍会重复显示同名项。建议用受控的实际操作名作为导出名称，并给裸 FFI 操作加明确前缀，例如 `bindings.list`；不把 ID、路径或错误正文拼进名称。当前依赖已有 `otel.name`／`otel.kind` 支持，不需要另写 exporter 或增加 provider。
2. **原生默认分类会误报读取。** [Android 分类](../../app/android/app/src/main/kotlin/app/velune/android/ClientTelemetry.kt) 未识别名称一律落到 `ui.application.read`；当前 `ConversationModel.compact()` 传入 `compact`，因此写操作会被标成读取。[Windows 分类](../../app/windows/ClientTelemetry.cs) 也一样，`Listings.Change/Write` 的 `Configuration` 就是具体例子。Apple 默认 `ui.action` 不会误称读取，但分类粒度与其他平台不同。建议由调用处明确给出稳定语义，四端对相同动作使用同名；不要依赖方法名猜测，也不要把未知操作默认为读取。
3. **跨设备与内部操作没有 kind 区别。** `target/telemetry-delivery-acceptance.wire.json` 中的 332 条实际 span 全部为 INTERNAL，其中 311 条 Rust span 同名为 `client.operation`，21 条 native span 有具体名称。父子关系确实包括 remote.request→remote.execute 和 gateway.request→gateway.attempt，但 kind 无法表达这些边界。该样本来自合成验收，不能据此估计生产流量占比。现有验收主要检查 operation 属性及因果关联，没有覆盖这些名称／kind 的可读性问题。

kind 修正需要尊重生命周期。[remote 客户端](../../packages/remote/src/client.rs) 的 Submit／Query 请求等待响应，可以标 CLIENT；同步 query 处理可以标 SERVER，但 [service](../../packages/remote/src/service.rs) 目前在 `ledger.status` 之前就记录 success，结束应移到结果确定之后。Submit 先受理并派生后台任务，再返回状态；`remote.execute` 应按延后执行理解，CONSUMER 比 SERVER 合适，不能把后台耗时算作受理 RPC 耗时，也不必为配齐类型机械增加 PRODUCER。

[Gateway](../../packages/gateway/src/runtime.rs) 接收 HTTP 请求，request 观察保留至响应流结束或断开，可以采用 SERVER。attempt 既覆盖 HTTP 也覆盖 Claude CLI，并包含适配准备；不能统一解释成 HTTP CLIENT。后续设计应让通用 attempt 保留重试／适配含义，并在实际外部调用边界采用合适的 CLIENT；是否复用现有 attempt 需要按所覆盖边界决定。UI、FFI、执行轮次和 SDK 换代后的 result 补记保持内部语义。当前执行轮次由实际 owner 保存至终态、控制断开不提前结束、每次 gateway 尝试单独观察的设计应保留。

建议把新增埋点规则收敛到现有[遥测装配文档](../../docs/design/telemetry.md)：span 记录能独立解释耗时或失败的操作，event 记录其间状态变化，metric 记录可聚合数量和分布；不为每个函数创建 span。不同层的操作计数不能相加冒充业务轮次数。例子应包含同机 FFI、跨设备异步执行和一次 gateway 重试，并直接展示真实导出名称、kind、父子关系及结束时间。用这几个代表性场景验证即可，无需为本次评审重跑设备安装流程。

依据为 [OTel service 定义](https://opentelemetry.io/docs/specs/semconv/resource/service/)、[scope 定义](https://opentelemetry.io/docs/concepts/instrumentation-scope/)、[SpanKind 定义](https://opentelemetry.io/docs/specs/otel/trace/api/#spankind)、[tracing-opentelemetry 0.33 特殊字段](https://docs.rs/tracing-opentelemetry/0.33.0/tracing_opentelemetry/#special-fields) 和 [Tempo service graph 配对规则](https://grafana.com/docs/tempo/latest/metrics-from-traces/service_graphs/)。本轮没有验证 Grafana 网页服务图或运行新的设备测试；也没有把语义评审扩展成所有埋点的完整覆盖审计。
