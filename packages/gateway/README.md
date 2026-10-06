# AI 网关

`velune-gateway` 是本机 LLM 网关 unit，也是 AI 能力的一种应用模式。它接收 OpenAI ChatCompletions v1、Responses v1 或 Anthropic Messages 请求，通过显式模型路由派发给提供商。同协议路径使用原生 operation，保留消息历史、生成参数与响应事件；异协议路径调用 ai-provider 的请求计划转换 JSON 与增量 SSE，采用 best-effort 降级和静态诊断，不经过 sampling。模型路由与认证始终根据上游目标，运行时入口协议不限制提供商协议。HTTP 的 hop-by-hop 字段、本地认证和 cookie 不转发。

调用方通过 `GatewayConfig` 提供具有模型条目的提供商及策略，通过 `Runner::start` 提供入口模型别名。提供商配置在启动时形成不可变快照，当前 fail-over 明确禁用；直接调用方修改配置后须重建 Runner；application 会使执行失效并在下次发送时自动准备，用户不需要手动连接。运行时模型选择可显式替换入口 alias 到稳定模型记录的绑定，不修改在途派发已捕获的目标。此 unit 不理解 Harness 的模型目录、思考级别转换、配置实例或认证文件。模型元数据不成为原生请求参数的默认值。仅异协议转换到 Messages、原请求无输出上限时，使用所选模型的配置上限；两者都缺失时返回必要配置错误。旧 `chatCompletionsOutputLimitField` 已从新契约删除，不提供兼容入口。

提供商认证配置只有不透明 `credential_ref`。应用通过 `Runner::start(config, resolver, aliases)` 注入真正的异步 `CredentialResolver`；每次解析将资源引用与已捕获的协议／endpoint 一同传入，只返回瞬时 `ResolvedCredential`。解析器的私有认证配置、目标校验、来源 adapter、helper 与进程生命周期均归调用方。网关不接收来源 JSON、目录、CLI 路径或认证文件，不解释引用，不读取全局配置，不持久化秘密。取消派发会丢弃正在等待的解析 future；调用方必须提供符合取消合同的实现。

Axum 负责 HTTP framing，当前只监听随机 loopback 端口并提供三条 POST 创建端点。请求 body 上限为 256 KiB，读取期限为 15 秒；最多同时处理 16 个已进入 handler 的请求，不承诺限制所有原始 TCP 连接。流式输出采用容量为 1 的异步通道保持背压，丢弃响应 body 会取消派发。HTTP client 禁用自动重试与重定向，连接期限为 15 秒，读取空闲期限为 60 秒。`Runner` 退出停止入口并取消在途任务，不等待长流完成。资源策略当前是固定上限，尚无应用内配置入口。

依赖方向为 `velune-gateway → velune-ai-provider → velune-ai`。它不依赖会话投影、Agent 运行时、应用层或语言绑定。调用方决定是否链接此 unit；仅连接远端的客户端不需要本机 HTTP 网关。检索、后台 Responses 生命周期、自动路由、fail-over 与无中断热配置不属于当前实现。

提供商模型条目使用稳定 `recordKey` 和精确 `providerModelId`。入口自动派生为 `velune/model/<recordKey>`，运行时注入的别名另外传入；内部记录键本身不是 wire model 入口。网关完成目标解析后才传递 `ProviderModelId`，协议执行不再次映射。上下文／输出能力与协议推理声明均可未知，同协议网关不要求或补充请求 token 限制，也不以默认规格覆盖提供商实际能力。

Codex／DSH 原生运行时的装配方可额外注册其执行模型字符串为 alias，并映射到已选的提供商模型记录；gateway 不据模型名推断提供商，不解析 app-server／ACP，也不读取 huihua 历史。执行模式由 application 与 adapter 保证原生 Harness 只获得网关入口与临时本地 token。审批、回答、取消 turn 与原生会话恢复不属于本 unit。

网关只发出元数据 tracing 事件，subscriber 由消费方装配。request／attempt 关联贯穿响应 body 与取消 future 的丢弃；目标使用当前快照中的序号，日志不包含模型 ID、alias、endpoint、请求正文或凭据。传输完成与模型业务成功分开，调用方断开与 Runner 关闭也有独立归因。具体字段与人工验收见 [本地诊断](../../docs/development.md#本地诊断)。

Messages 保留调用方的 `anthropic-version`／beta 请求头与原生 JSON／SSE，缺少版本在派发前拒绝；API key 使用 `x-api-key`，未确认的 subscription 认证不适用于此协议。提供商 baseURL 追加 `/v1/messages`，运行时 SDK 的网关注入使用 origin。

异协议转换复用 `eventsource-stream` 解析 SSE 分帧，支持任意网络分块。转换后的响应去除过期的长度、编码与完整性 headers；Messages 合成请求使用 API 版本 `2023-06-01`，同协议仍保留调用方的版本。转换正文累计上限为 16 MiB，输出沿用容量为 1 的通道与取消合同。转换无法保留的字段只记录静态 field/code，不记录正文；真实上游 HTTP 错误保留状态和正文，未出现有效业务终态的断流不会补造完成。人工端到端验证入口见 [开发说明](../../docs/development.md)。

跨协议文本增量立即交付。转换到 Messages 的工具参数须形成合法 JSON object，因此在单个工具完成时交付其有界累积参数；Responses custom 工具也在其参数完成后解除 `{input:string}` 包装。无法解析的参数保留为原始字符串并记录降级，不能补造可执行命令或整次拒绝。OpenAI 两协议间普通 function 参数继续增量交付。
