# Harness 互联互通：跨运行时子 Agent

## 目标与授权

2026-10-11 用户要求实现不同 Agent runtime 互为 sub-agent，指出这是 Harness 互联互通的关键功能。承接既有 ACP 和运行时工作；仅修改 Rust packages、必要辅助资源、手动验收入口和文档，不修改／构建 App。允许本地提交，不 push，不读取真实凭据或会话，不登录或调用真实模型。

产品意图归 [PRD](../../docs/prd/collaboration.md)。父 Agent 必须实际获得协作工具，委派给另一 runtime、等待或查询可关联的结果，并在自身原生会话继续；只提供 Host API 或保存任务树不算完成。子任务的后续输入、取消、错误与审批边界必须可观察。App 展示和操作明确待实现。

## 方案与范围

任务开始时已有通用 ACP v1 Client 和 Pi／Codex／Claude／DSH 专用适配器；真实 DSH、OpenCode 闭环及 Devin 无认证会话控制证据归 [ACP 任务](../acp-adapter/packet.md)。缺口是 ACP `mcpServers` 为空、application 只有前台单活动执行器、没有产品 MCP 协作服务。历史模拟 Host 不是可复用的产品执行入口。

参考 acpsub `259db0d08f43941d3742d3a4710fd89f0adc603b` 的 MCP→ACP 分工、可继续会话和等待结果方式，不引入其 daemon 或 transcript 副本。已选择嵌入式 MCP 服务、独立子执行器与会话作用域授权；不为协作重构前台浏览／composer。父 turn 等待时子执行器自主轮询，结果回到原工具调用。首轮固定一层、多子并发和空闲后续输入；不增加递归、busy 队列、自动恢复或重试。

runtime_design 负责关键设计判断；runtime_adapter 负责运行时工具注入与原生接入；runtime_application 负责应用协调与 bindings；主执行者负责共享契约汇合、MCP 入口、文档和采用验证。已有 `tasks/new-api-reuse/` 属于其它任务，不纳入本次提交。

验收采用隔离 HOME、合成 MCP/ACP peer 和本机合成模型服务，优先真实 Harness 父→工具→异种子 Harness→结果→父继续。覆盖并发等待、后续消息、失败、取消与作用域隔离；运行 fmt/check/clippy 和必要构建，不增加或运行自动化测试。

## 实施与当前验证

已实现 official rmcp `3.5.1` 的会话独立 loopback MCP server、七个协作工具、宿主 target profile 授权和随机访问票据。application 的 Coordinator 不回调前台锁，每个 ChildRuntime 独占 Pi／Native／ACP client、网关、转发和临时模型投影；typed UniFFI 创建协作父会话并提供子状态、投影、人工审批回复和显式关闭入口。提供商／运行配置修改要求先结束协作 scope，避免授权快照继续指向已修改配置。

[运行时注入证据](runtime-injection-evidence.md)已覆盖四个真实专用父 runtime 的 MCP initialize／工具发现；Codex／Claude 保留合成原服务且不写永久配置。通用 ACP new/load/resume 按 HTTP 能力选择直连或 stdio relay；真实 WSL 完成反向转发、远端 relay 和资源清理。[端到端验收](interoperation-evidence.md)已通过真实 OpenCode→Velune MCP→真实 DSH→结果→父继续，委派期间不调用 application 轮询 API。最终动态库同时验证子快照消息 stamp 的解码与稳定性。

[应用执行验收](application-evidence.md)覆盖真实 Pi／Codex 同实例双子任务并发、managed gateway 及同一个子会话第二轮续发，源会话身份和上下文保持。合成 ACP 覆盖权限回复、审批等待、EOF、期限、启动中取消、配置授权冻结和析构清理；实际 MCP 入口另验证票据、Origin／Host、参数范围和 scope 撤销。

## 交付检查

已通过 `cargo fmt --all -- --check`、`cargo check --locked --workspace --all-targets --all-features`、`cargo clippy --locked --workspace --all-targets --all-features -- -D warnings` 与 `cargo build --locked -p velune-bindings`；bindings 无默认 features 的 check／clippy 亦通过。未新增或运行自动化测试。

本轮实现范围已完成，关键产品意图归 PRD、跨包契约归架构与 unit README、操作入口归开发说明。仅本地提交当前任务；不包含 `tasks/new-api-reuse/`，不改 App，不 push。首轮一层委派，无递归、自动恢复或未知结果重试；同步 adapter 请求可能延后取消与期限收口，不能承诺硬实时中断。
