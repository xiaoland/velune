# AI 网关

`velune-gateway` 是本机 LLM 网关 unit，也是 AI 能力的一种应用模式。它接收 OpenAI ChatCompletions v1、Responses v1 或 Anthropic Messages 请求，通过显式模型路由派发给提供商。同协议路径使用原生 operation，保留消息历史、生成参数与响应事件；异协议路径调用 ai-provider 的请求计划转换 JSON 与增量 SSE，采用 best-effort 降级和静态诊断，不经过 sampling。模型路由与认证始终根据上游目标，运行时入口协议不限制提供商协议。HTTP 的 hop-by-hop 字段、本地认证和 cookie 不转发。

调用方通过 `GatewayConfig` 提供具有模型条目的提供商、模型路由及策略，通过 `Runner::start` 提供入口模型别名。提供商配置在启动时形成不可变快照；直接调用方修改配置后须重建 Runner；application 会使执行失效并在下次发送时自动准备，用户不需要手动连接。运行时模型选择可显式替换入口 alias 到稳定模型记录的绑定，不修改在途派发已捕获的目标。此 unit 不理解 Harness 的模型目录、思考级别转换、配置实例或认证文件。模型元数据不成为原生请求参数的默认值。仅异协议转换到 Messages、原请求无输出上限时，使用所选模型的配置上限；两者都缺失时返回必要配置错误。旧 `chatCompletionsOutputLimitField` 已从新契约删除，不提供兼容入口。

提供商认证配置只有不透明 `credential_ref`。应用通过 `Runner::start(config, resolver, aliases)` 注入真正的异步 `CredentialResolver`；每次解析将资源引用与已捕获的协议／endpoint 一同传入，只返回瞬时 `ResolvedCredential`。解析器的私有认证配置、目标校验、来源 adapter、helper 与进程生命周期均归调用方。网关不接收来源 JSON、目录、CLI 路径或认证文件，不解释引用，不读取全局配置，不持久化秘密。取消派发会丢弃正在等待的解析 future；调用方必须提供符合取消合同的实现。

Axum 负责 HTTP framing，当前只监听随机 loopback 端口并提供三条 POST 创建端点。请求 body 不设 gateway 自身的大小上限，读取期限为 15 秒；入口不设置固定并发拒绝门禁。流式输出采用容量为 1 的异步通道保持背压，丢弃响应 body 会取消派发。HTTP client 禁用自动重试与重定向，连接期限为 15 秒，不设置固定读取空闲期限。`Runner` 退出停止入口并取消在途任务，不等待长流完成。取消与资源限制由调用方和上游 HTTP 客户端共同决定。

依赖方向为 `velune-gateway → velune-ai-provider → velune-ai`。它不依赖会话投影、Agent 运行时、应用层或语言绑定。调用方决定是否链接此 unit；仅连接远端的客户端不需要本机 HTTP 网关。检索、后台 Responses 生命周期、跨成员之外的自动路由与无中断热配置不属于当前实现。

提供商模型条目使用稳定 `recordKey` 和精确 `providerModelId`。入口自动派生为 `velune/model/<recordKey>`，运行时注入的别名另外传入；内部记录键本身不是 wire model 入口。网关完成目标解析后才传递 `ProviderModelId`，协议执行不再次映射。上下文／输出能力与协议推理声明均可未知，同协议网关不要求或补充请求 token 限制，也不以默认规格覆盖提供商实际能力。

模型路由（`ModelRoute`）以自己的 `recordKey` 与提供商模型同列，入口同为 `velune/model/<recordKey>`，别名也可指向它；成员是按优先顺序排列的提供商模型记录键，须同协议，校验拒绝空成员、重复成员与和模型冲突的键。一次请求按顺序捕获成员；在 429 后的冷却期内（Retry-After，缺省 60 秒、至多 24 小时）该提供商排到末尾，全部冷却时仍按顺序尝试。模型路由请求携带非空 `previous_response_id` 或 `conversation` 时在转换和派发前拒绝，要求调用方提供完整历史；直接选择提供商模型仍保留原有状态引用契约。冷却状态是调用方持有的 `RouteHealth`，传给 `Runner::start_with_analytics`，使限流跨 Runner 生效；`Runner::start` 使用独立的空状态。

fail-over 只在尚未向调用方转发任何响应时发生：认证解析失败、提供商配置无效、尚未收到响应的连接／传输失败或超时、429、结构化额度错误（402 且 error.code/type 为 `insufficient_quota`、`usage_limit_reached` 或 `quota_exceeded`）与 5xx 改用下一个成员；403、其它请求类 4xx、取消，以及已开始转发后的中断都结束请求。成功 HTTP headers 一经交付即提交响应；200 SSE 内后续出现的额度错误或业务失败也不切换。普通认证的 401 仍结束；`CodexSubscription` 的 401 在提交响应前调用 resolver.refresh 一次，只有相同 account_id 的凭据可重发。永久认证失效或重发仍为 401 时调用 invalidate，并允许下一个成员；续期暂不可用也可尝试下一成员，但不标记会话永久失效。所有成员失败时返回最后一个失败。provider 每次调用只执行一次尝试；401 恢复由 gateway 显式组织。恢复后的同一成员只形成一条最终 exchange 的 analytics 记录，初次 401 由无正文 tracing 保留；异协议请求的所有尝试进入同一个转换。每次尝试各有一条 analytics 记录，第二次起的 `request_id` 为 `<请求>/<序号>`；tracing 在 attempt 中记录 `gateway_route_selected` 与 `gateway_failover`。直接选择的提供商模型只有一个目标，不 fail-over。

Codex／DSH 原生运行时的装配方可额外注册其执行模型字符串为 alias，并映射到已选的提供商模型记录；gateway 不据模型名推断提供商，不解析 app-server／ACP，也不读取 huihua 历史。执行模式由 application 与 adapter 负责注入网关入口和临时本地 token；是否同时保留其它运行时入口由调用方决定。审批、回答、取消 turn 与原生会话恢复不属于本 unit。

网关只发出元数据 tracing 事件，subscriber 由消费方装配。request／attempt 关联贯穿响应 body 与取消 future 的丢弃；目标使用当前快照中的序号，日志不包含模型 ID、alias、endpoint、请求正文或凭据。传输完成与模型业务成功分开，调用方断开与 Runner 关闭也有独立归因。具体字段与人工验收见 [本地诊断](../../docs/development.md#本地诊断)。

Messages 保留调用方的 `anthropic-version`／beta 请求头与原生 JSON／SSE，缺少版本在派发前拒绝；API key 使用 `x-api-key`，未确认的 subscription 认证不适用于此协议。提供商 baseURL 追加 `/v1/messages`，运行时 SDK 的网关注入使用 origin。

异协议转换复用 `eventsource-stream` 解析 SSE 分帧，支持任意网络分块。转换后的响应去除过期的长度、编码与完整性 headers；Messages 合成请求使用 API 版本 `2023-06-01`，同协议仍保留调用方的版本。输出沿用容量为 1 的通道与取消合同；具体资源预算由调用方和上游传输负责，不添加协议无关的正文总量门禁。转换无法保留的字段只记录静态 field/code，不记录正文；真实上游 HTTP 错误保留状态和正文，未出现有效业务终态的断流不会补造完成。人工端到端验证入口见 [开发说明](../../docs/development.md)。

跨协议文本增量立即交付。转换到 Messages 的工具参数须形成合法 JSON object，因此在单个工具完成时交付其有界累积参数；Responses custom 工具也在其参数完成后解除 `{input:string}` 包装。无法解析的参数保留为原始字符串并记录降级，不能补造可执行命令或整次拒绝。OpenAI 两协议间普通 function 参数继续增量交付。

调用方可通过 `Runner::start_with_analytics` 注入非阻塞 `AnalyticsSink`，在上游原生响应、协议转换之前接收实际提供商／模型与用量、时间、结果记录。接收器负责持久化；gateway 不读取存储路径，也不从 tracing 重建用量。已有 `Runner::start` 可不采集统计。

`ResolvedCredential.profile` 将普通 API／已有来源认证（`Standard`）与 Codex backend 订阅（`CodexSubscription { account_id }`）分开。profile 不改变网关路由协议；后者只允许精确的 HTTPS ChatGPT Codex Responses 目标，协议 adapter 再次校验。resolver 的 refresh／invalidate 默认分别返回不可用／无操作，现有来源不必实现续期。rejected_token 仅用于识别被拒的会话，不能写入日志。认证的创建、保存、刷新串行化与并发换号保护属于 application。

上游 HTTP 2xx 仍可能在响应提交前因协议契约不满足而失败，例如流请求收到非 SSE 类型。此时网关返回 502 合成 JSON 错误，不复用成功响应的正文或实体 headers；analytics 保留实际上游状态。这个下游状态规范化不参与 fail-over 分类，不能把上游协议错误当作上游 5xx。已提交响应后的错误仍按原有中断处理。
