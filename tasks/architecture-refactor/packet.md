# 架构重构、测试清理与文档更新

2026-10-28 起。用户要求在迭代加速前对全项目做一次彻底的实现重构、测试清理和文档更新，避免技术债继续堆积。

## 已确认决定

- 重构期间冻结功能开发，main 只接收重构；用户已冻结其他开发。
- 持久化结构可以 hard-cut，不写迁移；原 Harness 会话与认证不动。
- `scripts/manual-*.py` 全部删除（git 保留历史），重构完成后只为类型与静态检查无法表达的真实不变量重建少量分层检查。
- 破坏性变更被鼓励（2026-10-28）：首个 beta 发布前没有维护与交付压力，目标是尽量减轻技术债及其未来增长速度，而不是保持接口兼容。FFI 变更时 Mac 每次提交同步，Windows／移动端在阶段末一次追平。

## 诊断（基线 0af7910）

1. **核心内部仍运行已删除 IPC 的 JSON 协议。** `Command` → `dispatch_command` → `Application` 的类型化方法 → `json!({"version":3,"action":…})` → `CoreRuntime::request_inner` 按 59 个字符串 action 分派，用 `payload["…"]` 取字段（`application` 内 117 处），结果再以 JSON 后处理（手工删除 `acpLaunch`）并反序列化。`local/mod.rs` 与 `portable.rs` 各有一份字符串分派。哪些 action 需要聚焦会话由 `route` 中的字符串列表决定。
2. **同一类型在各层重复 3～4 次。** `application-api` 定义领域类型；`bindings/types.rs` 镜像约 200 个 `Binding*` 类型，经 `serde_json` 往返转换（错配只在运行时以 `Contract` 错误出现）；`remote_command`／`remote_response`／`remote_state` 再转一次；Mac `BindingMapping.swift`＋`Models.swift`（约 1 000 行）再映射一次。一条 composer 额度查询涉及 16 个源文件。
3. **`CoreRuntime` 是上帝对象。** 持久化配置的各字段在其中各存一份副本（gateways、模板、显示偏好等），另有多个 `RefCell` 缓存。
4. **Harness 没有适配器抽象。** `agent-runtime` 没有 trait；Pi、Codex、Claude Code、DeepSeek、ACP 由散落在 `application` 与 `agent-runtime` 的约 30 处按类型 `match` 区分。
5. **超大文件。** `gateway/runtime.rs` 2 358 行、Mac `Store.swift` 2 161 行（单个 `AppStore` 161 个函数）、`application/api.rs` 1 372 行等。
6. **文档成为流水账。** Markdown 共 2.1 MB；`docs/development.md` 112 KB、`docs/design/architecture.md` 99 KB，大部分是带日期的修订说明和"历史原型"段落；67 个任务目录。
7. **测试。** 约 60 个 Rust 测试；73 个一次性 `scripts/manual-*.py`，多数绑定旧 JSON 表面。
8. **基线 clippy 未通过**（`velune-gateway` 一处 `collapsible_if`），已在阶段 0 修复。

## 目标结构

- `application-api` 的 `Command`／`Response` 是唯一用例表面；本机、远控与 CLI 走同一条类型化路径，核心内部不再有字符串 action 或 JSON 信封。
- `CoreRuntime` 持有一份已解析的配置，按能力拆分用例模块；会话聚焦需求由 `Command` 变体决定。
- 跨 FFI 的类型直接由 `velune-conversation`／`application-api` 类型派生 UniFFI，不再维护镜像类型和 JSON 往返；平台端直接使用生成类型，不另建映射层。
- `agent-runtime` 以一个 Harness 适配器接口统一各运行时，按类型分支集中在构造处。
- 平台 app 按能力拆分状态与视图，不重算核心拥有的产品规则（见 [Product TDD](../../docs/product-tdd/index.md)）。
- 文档只描述当前事实；历史留在 git。

## 阶段

每个阶段结束时 workspace `cargo clippy --all-targets -D warnings`（含 bindings remote-only feature 组合）与 `cargo test --workspace` 通过，Mac 应用可构建运行。

| 阶段 | 内容 | FFI 表面 | 状态 |
| --- | --- | --- | --- |
| 0 | 修复基线 clippy；删除 `manual-*.py`；建立本任务包 | 不变 | 完成 |
| 1 | 删除核心内部 JSON 协议：`Command` 直接分派到类型化用例 | 不变 | 完成 |
| 2 | Harness 适配器接口；拆分 `gateway/runtime.rs` 等超大模块 | 不变 | 完成 |
| 3 | FFI 表面：删除 `Binding*` 镜像与 JSON 转换、合并远控转换层；四端机械更新，删除 Mac 映射层 | 变化 | 进行中 |
| 4 | 平台 app 结构：拆分 Mac `AppStore` 与 Windows 对应部分；把平台自行计算的产品规则移回核心 | 不变 | 待开始 |
| 5 | 测试：复核现有 Rust 测试，只为真实不变量重建少量分层检查 | 不变 | 待开始 |
| 6 | 文档：重写 development、architecture 等为当前事实；归档已完成任务包 | 不变 | 待开始 |

## 阶段 1 结果

- `Command` 在 `commands.rs` 中经 `CoreRuntime::serve(Focus, Login, 用例)` 直接调用类型化用例：`serve` 先聚焦所需会话、推进全部会话与协作服务，再按 `Login` 决定认证进行中是否拒绝。聚焦与认证门槛由各 `Command` 分支写明，不再由字符串 action 列表推断。
- 删除 `request_inner`、`dispatch`、`CONTRACT_VERSION`、`api.rs` 的 JSON `execute` 及其 59 个包装方法；`api.rs` 余下的分析报表移到 `analytics_report.rs`，`GatewaySummary` 转换移到 `provider_configuration.rs`。
- 核心内部重入（协作服务、发送队列、消息编辑、跨运行时接续）直接调用用例；排队消息保存 `TurnRequest`，不再保存 JSON 载荷。
- 删除从未发布的 `portable` 组合与 `app/host` 原型，以及 application 的 `local-runtime` feature。
- `agent-runtime`、`application-api` 各自定义的运行时资源、认证事件与 ACP 会话配置类型合并到 `velune-conversation`，两侧以别名再导出；原 JSON 往返曾静默丢弃 ACP `mcp_http`。
- 诊断 3 修正：`CoreRuntime` 的显示偏好字段是当前客户端视图的状态（`ClientState` 按视图换入换出，`persist` 以本机基线写回），网关、运行时与模板只有这一份内存副本，不存在重复配置。上帝对象问题留给阶段 2 按能力拆分。
- 剩余 JSON 只在真实线协议处：Pi RPC、MCP、接续上下文正文与持久化文件。
- 验证（2026-10-28）：workspace clippy／test 通过；FFI 表面未变，于是取出删除前的 `manual-*.py`（git `9339fd3^`）在临时 HOME 与回环合成上游下跑重构后的 UniFFI 库：send-queue、edit-message 与 steer（Codex 0.159.3／Claude Code 2.1.292／Pi 1.0.2）、collaboration、runtime-interactions、acp-adapter、resume-turn（Codex）、context-compaction（Pi）、session-browser、working-changes、pi-native-loop、runtime-selection 全部通过。acp-adapter 对 ACP 模型目录含“默认”项、pi-native-loop 对日志字段 `operation_id` 的断言在重构前已过时，按现行行为改写后通过。Mac 应用在隔离 `VELUNE_HOME` 后台启动，列表与活动轮询无告警。

## 阶段 2 结果

- 运行时类型是 `RuntimeKind` 枚举，能力（恢复轮次、压缩、首轮持久化、图片）集中在 `agent-runtime` 的版本表；按类型的匹配均为穷举，不再有通配分支。
- Pi 会话收拢为 `agent-runtime::PiSession`；Pi、原生 app-server 与 ACP 会话实现 `LiveConversation`，application 的发送、取消、轮询、关闭、插入、压缩与回复不再三路分支（`ActiveState` 匹配 93 → 65，余下为各类型专属操作）。原生会话中 Codex 与 Claude Code 的专属状态各自成组。
- AI 原生派发合并：`velune-ai` 一个 `Protocol` 取代六份协议枚举，三套逐协议契约合为 `NativeProvider`；ai-provider 的三个 HTTP adapter 合为 `NativeHttp`；gateway 的 `runtime.rs`（2 358 行）拆为六个模块，按协议重复三次的派发合为一条路径，切换规则对三种协议一致。删除无调用方的 sampling 栈与 MiniMax adapter。净删约 5 000 行。
- 先以本地脚本化上游为网关派发与故障转移写特征测试（`packages/gateway/tests/dispatch.rs`），再做合并；网关此前没有测试。
- 未按行数拆分的文件：`resource_library.rs`、`codex.rs`／`claude.rs` 等内聚于单一用例或单一 Harness，拆分收益低。
- 验证（2026-10-29）：workspace clippy／test 通过；12 个删除前的回归脚本对重构后的 UniFFI 库全部通过。

## 阶段 3 进展

- 契约类型直接派生 UniFFI：`velune-conversation` 与 `application-api` 无条件依赖 `uniffi` 并各自 `setup_scaffolding`，每个 crate 生成一份语言文件（Swift 合入同一 `VeluneBindings` 模块，C 头共用 `VeluneBindingsFFI`）。原计划的 feature 开关没有必要：这些 crate 唯一的产物是 bindings 动态库。`velune-ai` 保持无 FFI 依赖，`Protocol` 在 `application-api` 以 `#[uniffi::remote]` 声明；Swift 中 `X.Protocol` 是元类型语法，故 Swift 名为 `ApiProtocol`。
- 删除 `bindings/types.rs`（191 个镜像）、`remote_command.rs`／`remote_response.rs` 与 `convert` 的契约用法；`request!` 直接返回领域值。`CommandKind` 直接派生，删除远控状态里的镜像。
- 领域类型顺带收紧：`SettingField.kind` 为 `SettingKind`、`ProtocolDescriptor.id` 为 `Protocol`、`RuntimeInstance.type_id`／`RuntimeTypeDescriptor.id` 对平台为 `RuntimeKind`（导出 Display）、`TranscriptTurn.reply_start_block` 为 `u32`、`SnapshotUpdate` 不再装箱；`ConversationTitle` 导出 `displayText()`；`BindingResolution` 改名 `ResourceBindingResolution`；有序 map 以自定义类型 `StringMap`／`WeightMap` 映射为字典。
- 删除 `Command::SendTurn`／`CreateConversation` 旧变体，`…WithSelection` 改名为 `SendTurn`／`CreateConversation`（平台方法同名）。`SendQueue`／`SteerTurn` 的 `runtime_id` 实际供远控 pending 记录定位会话，保留。
- 删除没有平台调用的预分配请求 ID 入口：`prepare_remote_operation`／`VeluneRemoteOperation`、`next_request_id`、`submit_request`，以及 remote 的 `RemoteOperation` 与 `OperationUsed`；服务级 `query_request`／`acknowledge_request` 保留，`BindingRemoteRequestStatus.Completed` 直接携带 `Response`。
- Mac 构建脚本生成合并 modulemap；Mac 代码按新类型名机械更新，`RuntimeInstance.typeID` 等改为 `RuntimeKind`。
- 验证（2026-10-29）：workspace clippy／test、bindings remote-only clippy 通过；12 个删除前回归脚本按新名机械改写（另以临时 Python shim 兼容字符串类型 ID 与标题比较）后全部通过；Mac 应用构建并在隔离 `VELUNE_HOME` 后台启动，无告警。
- 无平台调用的其他导出（2026-10-29 统计）：`configure_runtime_execution`、`next_turn_model_selection`、`open_acp_conversation`、`runtime_quota`、运行时资源四个读写方法，以及远控 `acknowledge`、`acknowledge_reset`、`apply_network`、`controller_status`、`disconnect`、`expect_revision`、`list_with_revision`、`network_change`、`network_state`、`observe_paths`、`observed_revision`、`reconnect`、`record_legacy_state_reset`、`refresh_local_discovery`、`remote_descriptor`、`retry_network`、`update_contact_locator`。它们对应已设计的能力，阶段 3 末与平台追平一起决定删除或保留。

## 留待后续阶段

- 运行时设置仍是 `BTreeMap<String, String>`（如 `acpLaunch`）；`AuthenticationEvent` 是松散结构；`ProviderImportSource` 可收窄；三个 “Source” 类型并存。
- `repository.rs` 仍为旧配置补 `contextUsageDisplay`，配套测试在阶段 5 复核。
- `application-api` 的 `ExecutionBackend`、`RequestProfile`、`RoutingStrategy`、`QuotaLimit` 与 gateway 配置各有一份，由 application 逐字段转换；`AnalyticsProtocol` 与 `Protocol` 只差序列化名。随阶段 3 FFI 表面一起收敛。
- `service::usage_tests` 曾在一次 workspace 测试中于 `open().unwrap()` 失败、重跑通过，阶段 5 复核是否有共享状态。

## 待用户决定

- 阶段 6 已完成任务包的处理：删除（git 保留）或合并为简短结论。
