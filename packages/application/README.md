# 应用层

`velune-application` 是平台 app 共用的应用用例 unit。公开入口是 `Application`，提供具名、类型化的配置、连接、会话、模型选择、提供商导入与认证操作。Swift、Kotlin 等语言绑定消费这些用例，不自行协调文件提交、子进程状态与投影失败清理。

`config` 定义 schema 3 的应用配置与跨领域引用；`repository` 是内部文件设施，负责锁、加载和原子提交，不是独立 package。配置只保存普通元数据和认证资源登记，不保存秘密。提供商只持有中央登记表的 `authenticationId`，不能携带来源目录、原提供商认证设置或任意 Keychain 引用。`authentication_resources` 管理资源名称、获取方式、目标授权、私有定位与修订；登记、替换、改名、删除及所有配置写入共用约束。认证资源的 API key／OAuth 获取方式与来源 provenance 分开表示。根目录与资源目录由平台通过 `Options` 显式传入，unit 不读取全局环境。来源文件与 Harness 会话仍由原拥有者维护，应用不会新建会话数据库。提供商导入选择已保存的 Agent 运行时实例，应用从其配置确定目录及 Node 路径；调用方不能另填或覆盖来源配置。预览与提交都会重新解析实例，并以运行时配置与源目录元数据共同判定预览是否过期。导入不要求已连接、默认模型或可运行的模型路由。

默认的 `local-runtime` feature 组合本机 Agent 运行时和 AI 网关。`local` 按配置用例、连接生命周期、会话操作、认证协调与 Pi 装配分模块；Pi SDK 来源解析与交互式登录由运行时 adapter 承担；应用保存中央认证资源，并捕获登记表快照装配异步解析器。解析器只接受已登记 ID 和已授权的协议／endpoint，未知 ID 不回退到平台秘密库。Unix helper 的期限、输出上限与进程组取消由应用负责；网关不解释来源或调用 helper。Pi 元数据在装配时显式剥离为 `velune_gateway::GatewayConfig`。网关不读取 Pi 字段，AI package 不依赖 Agent 运行时。

关闭 `local-runtime` 后，配置加载、列表和修改仍可使用，原本机配置及 adapter 元数据保持原样。中央认证登记、改名、替换和删除同样可用；本机连接、会话执行、来源导入及需要 adapter 的认证检查／登录明确返回不支持错误；该构建不链接 `velune-agent-runtime`、`velune-gateway`、AI provider 或 Node 资源。本 unit 当前没有远端会话装配用例，尚未建立 remote unit。

公开 API 和结果记录定义在 `api.rs`，调用方没有 JSON action dispatcher。内部局部用例仍使用旧投影形状的 JSON 转换以维持现有交互语义；它不构成平台接入契约，也不应成为新增领域功能的接口。

旧 schema 2 配置在加载时仅根据原普通元数据迁移为 schema 3，并在完整验证后原子保存。每个旧提供商获得独立认证资源，不按相同目录或 Keychain 名自动合并；迁移不访问原认证文件、秘密库或会话。凭据替换采用平台先写新秘密、登记表原子更新、成功后清理旧拥有项的顺序。`AuthenticationMutation` 返回精确可清理引用；未提交错误允许平台删除新项，提交后的退出问题通过 warning 表达并保留新旧项。被提供商引用的资源不能删除，原委托来源文件永不由资源删除操作移除。
