# AI 网关

`velune-gateway` 是本机 LLM 网关 unit，也是 AI 能力的一种应用模式。它接收 OpenAI ChatCompletions v1 或 Responses v1 请求，通过显式模型路由派发给提供商。两条路径均使用原生协议 operation，不经过 sampling，不翻译消息历史、生成参数或响应事件。网关仅改写路由模型和上游认证；安全的协议请求头、响应状态、响应头与 body/SSE 由原生传输保留。HTTP 的 hop-by-hop 字段、本地认证和 cookie 不转发。

调用方通过 `GatewayConfig` 提供具有模型条目的提供商及策略，通过 `Runner::start` 提供入口模型别名。配置在启动时形成不可变快照，当前 fail-over 明确禁用；修改应用配置后需要重新连接。此 unit 不理解 Harness 的模型目录、思考级别转换、配置实例或认证文件。模型元数据不成为原生请求参数的默认值；网关不强制请求包含输出上限，也不按该元数据补删生成参数。旧 `chatCompletionsOutputLimitField` 已从新契约删除，不提供兼容入口。

提供商认证配置只有不透明 `credential_ref`。应用通过 `Runner::start(config, resolver, aliases)` 注入真正的异步 `CredentialResolver`；每次解析将资源引用与已捕获的协议／endpoint 一同传入，只返回瞬时 `ResolvedCredential`。解析器的私有认证配置、目标校验、来源 adapter、helper 与进程生命周期均归调用方。网关不接收来源 JSON、目录、CLI 路径或认证文件，不解释引用，不读取全局配置，不持久化秘密。取消派发会丢弃正在等待的解析 future；调用方必须提供符合取消合同的实现。

Axum 负责 HTTP framing，当前只监听随机 loopback 端口并提供两条 POST 创建端点。请求 body 上限为 256 KiB，读取期限为 15 秒；最多同时处理 16 个已进入 handler 的请求，不承诺限制所有原始 TCP 连接。流式输出采用容量为 1 的异步通道保持背压，丢弃响应 body 会取消派发。HTTP client 禁用自动重试与重定向，连接期限为 15 秒，读取空闲期限为 60 秒。`Runner` 退出停止入口并取消在途任务，不等待长流完成。资源策略当前是固定上限，尚无应用内配置入口。

依赖方向为 `velune-gateway → velune-ai-provider → velune-ai`。它不依赖会话投影、Agent 运行时、应用层或语言绑定。调用方决定是否链接此 unit；仅连接远端的客户端不需要本机 HTTP 网关。检索、后台 Responses 生命周期、自动路由、fail-over 与无中断热配置不属于当前实现。

提供商模型条目使用稳定 `recordKey` 和精确 `providerModelId`。入口自动派生为 `velune/model/<recordKey>`，运行时注入的别名另外传入；内部记录键本身不是 wire model 入口。网关完成目标解析后才传递 `ProviderModelId`，协议执行不再次映射。上下文／输出能力与协议推理声明均可未知，网关不要求或补充请求 token 限制，也不以默认规格覆盖提供商实际能力。
