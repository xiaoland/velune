# 架构重构、测试清理与文档更新

2026-10-28 起。用户要求在迭代加速前对全项目做一次彻底的实现重构、测试清理和文档更新，避免技术债继续堆积。

## 已确认决定

- 重构期间冻结功能开发，main 只接收重构；用户已冻结其他开发。
- 持久化结构可以 hard-cut，不写迁移；原 Harness 会话与认证不动。
- `scripts/manual-*.py` 全部删除（git 保留历史），重构完成后只为类型与静态检查无法表达的真实不变量重建少量分层检查。

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
- 跨 FFI 的类型直接由 `application-api` 类型派生 UniFFI（feature 开关），不再维护镜像类型和 JSON 往返；平台端直接使用生成类型，不另建映射层。
- `agent-runtime` 以一个 Harness 适配器接口统一各运行时，按类型分支集中在构造处。
- 平台 app 按能力拆分状态与视图，不重算核心拥有的产品规则（见 [Product TDD](../../docs/product-tdd/index.md)）。
- 文档只描述当前事实；历史留在 git。

## 阶段

每个阶段结束时 workspace `cargo clippy --all-targets -D warnings`（含 remote-only 与 portable feature 组合）通过，Mac 应用可构建运行。

| 阶段 | 内容 | FFI 表面 | 状态 |
| --- | --- | --- | --- |
| 0 | 修复基线 clippy；删除 `manual-*.py`；建立本任务包 | 不变 | 进行中 |
| 1 | 删除核心内部 JSON 协议：`Command` 直接分派到类型化用例；`portable` 同路径；`CoreRuntime` 持有单份配置 | 不变 | 待开始 |
| 2 | Harness 适配器接口；拆分 `gateway/runtime.rs` 等超大模块 | 不变 | 待开始 |
| 3 | FFI 表面：删除 `Binding*` 镜像与 JSON 转换、合并远控转换层；四端机械更新，删除 Mac 映射层 | 变化 | 待开始 |
| 4 | 平台 app 结构：拆分 Mac `AppStore` 与 Windows 对应部分；把平台自行计算的产品规则移回核心 | 不变 | 待开始 |
| 5 | 测试：复核现有 Rust 测试，只为真实不变量重建少量分层检查 | 不变 | 待开始 |
| 6 | 文档：重写 development、architecture 等为当前事实；归档已完成任务包 | 不变 | 待开始 |

## 待用户决定

- 阶段 3 各平台同步方式（Mac 同步、Windows／移动端每阶段一次追平，或四端每次提交都通过）。
- 阶段 6 已完成任务包的处理：删除（git 保留）或合并为简短结论。
