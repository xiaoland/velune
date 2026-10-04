# 应用层

`velune-application` 是平台 app 共用的应用用例 unit。公开入口是 `Application`，提供具名、类型化的配置、连接、会话、模型选择、提供商导入与认证操作。Swift、Kotlin 等语言绑定消费这些用例，不自行协调文件提交、子进程状态与投影失败清理。

`config` 定义 schema 2 的应用配置与跨领域引用；`repository` 是内部文件设施，负责锁、加载和原子提交，不是独立 package。配置只保存普通元数据和凭据引用。根目录与资源目录由平台通过 `Options` 显式传入，unit 不读取全局环境。来源文件与 Harness 会话仍由原拥有者维护，应用不会新建会话数据库。

默认的 `local-runtime` feature 组合本机 Agent 运行时和 AI 网关。`local` 按配置用例、连接生命周期、会话操作、认证协调与 Pi 装配分模块；认证进程与 Pi SDK 来源读取由 Agent 运行时 adapter 承担，应用只协调导入提交和认证后的配置更新。Pi 元数据在装配时显式剥离为 `velune_gateway::GatewayConfig`。网关不读取 Pi 字段，AI package 不依赖 Agent 运行时。

关闭 `local-runtime` 后，配置加载、列表和修改仍可使用，原本机配置及 adapter 元数据保持原样。本机连接、会话执行、来源导入和认证明确返回不支持错误；该构建不链接 `velune-agent-runtime`、`velune-gateway`、AI provider 或 Node 资源。本 unit 当前没有远端会话装配用例，尚未建立 remote unit。

公开 API 和结果记录定义在 `api.rs`，调用方没有 JSON action dispatcher。内部局部用例仍使用旧投影形状的 JSON 转换以维持现有交互语义；它不构成平台接入契约，也不应成为新增领域功能的接口。
