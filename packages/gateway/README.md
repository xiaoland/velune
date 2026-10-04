# AI 网关

`velune-gateway` 是本机 AI 请求入口 unit。它接收 OpenAI ChatCompletions v1 或 Responses v1 请求，校验已配置模型及其协议参数，再将请求派发给显式选中的提供商。当前只监听随机 loopback 端口，不实现自动 fallback、协议翻译或远程服务。

调用方通过 `GatewayConfig` 提供模型、提供商和路由，通过 `Runner::start` 提供入口模型别名与 Responses 原生推理参数约束。别名和约束由调用方的装配逻辑生成；此 unit 不理解 Harness 的模型目录、思考级别转换、配置实例或认证文件。

提供商认证配置只能选择 `credential_ref` 或 `credential_source` 之一。前者是平台秘密设施的引用，后者是不透明 JSON。网关在请求派发时调用装配方指定的凭据 helper；helper 接收引用或 `--source-json`，不透明来源的返回值须包含 `contractVersion: 1`、`bearer` 和与目标一致的 `capabilities.protocol`／`capabilities.endpoint`。网关不解释来源内容，不读取用户的全局配置，不持久化配置或秘密。来源的字段校验、可用性检查、认证流程与生命周期由应用装配方负责。

依赖方向为 `velune-gateway → velune-ai-provider → velune-ai`，并直接使用 AI 服务契约。它不依赖会话投影、Agent 运行时、应用层或语言绑定。Rust public API 可由应用层组合为各平台 SDK；package 本身不要求独立动态库。

原生 Responses 请求保留协议字段，不将其转换为 Harness 的能力模型。ChatCompletions 使用当前受支持的 sampling 操作范围。调用方决定是否链接此 unit；仅连接远端的客户端不需要本机 HTTP 网关。
