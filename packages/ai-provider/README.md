# AI provider 执行

本 unit 实现 AI 契约的具体协议执行，包含原生 OpenAI Chat Completions、原生 Responses、原生 Anthropic Messages 与独立的 MiniMax sampling adapter。原生协议只替换路由所需的外部模型标识，保留请求字段、请求 headers、响应状态、响应 headers 和响应／SSE body；不经过 sampling mapper。它接受构造时传入的配置和一次性 `ResolvedCredential`，不读取应用配置、认证来源或 Harness 状态。

它依赖 [ai](../ai/README.md)，不拥有应用路由、自动重试或 fail-over 策略。网关组合协议执行而不把 Harness 适配放入此包。协议限制及手动 fixture 说明见 [AI service](../../docs/design/ai-service.md) 和 [开发说明](../../docs/development.md)。

原生操作收到已经解析的精确 `ProviderModelId`；本包不维护 Velune 记录到提供商型号的映射，不接受旧 `ModelMapping` 或 `ChatCompletionsOutputLimitField`。请求输出限制与推理字段由原生协议 body 决定。

Messages 保留调用方的 `anthropic-version`／beta 请求头与原生 JSON／SSE，缺少版本在派发前拒绝；API key 使用 `x-api-key`，未确认的 subscription 认证不适用于此协议。提供商 baseURL 追加 `/v1/messages`，运行时 SDK 的网关注入使用 origin。

`translation` 提供 ChatCompletions、Responses、Messages 六向 best-effort 转换。调用方只在异协议时构造请求转换计划，并用同一计划转换 JSON 或增量 SSE，保留工具别名的请求内映射。它不依赖 Harness、路由、应用配置或网络。无法对应的字段、状态引用和不透明推理信息尽量降级并给出不含业务值的 `ConversionNote`；真实错误和缺失终态不会被改造成成功。Messages 请求必须有输出上限，调用方可提供模型配置上限作为缺省值；同协议操作不经过此转换器。

转换语义由固定版本的 New API RelayKit 提供，Go 依赖见 `relaykit-bridge/go.mod` 和 `go.sum`。Rust 内部桥接管理唯一请求句柄，流转换消费请求计划，释放不会生成终态。桥接仅处理 JSON 与事件；HTTP、认证、取消和原始计量仍由 Rust 持有。不能跨提供商复用的服务端引用按 best-effort 省略并诊断，不补造历史；内联媒体在本地解析，外部媒体 URL 不触发桥接联网。诊断丢弃上游任意说明与字段值，不把原始错误送入日志。

Cargo 构建会调用 Go，以同进程库链接。Mac 使用 c-archive；Android 配置为 c-shared，需要平台打包相应共享库，不能把它作为常驻进程运行。目标映射不是运行验证，当前平台证据归 [接入任务](../../tasks/new-api-reuse/packet.md)。平台公开接口仍是 UniFFI，这个私有桥接不对 App 暴露。RelayKit 使用 AGPLv3，发行时必须连同适用源码义务处理；Mac 的原始声明和传递依赖许可保存在 [许可证目录](../../app/mac/Licenses/RelayKit/NOTICE.md)。

`metrics::ResponseMetrics` 从原生 JSON／SSE 提取用量、语义输出和业务终态，仅观察、不改写正文。独立未知字段和 Messages 缓存组成保留原始含义；协议转换占位不是其计量来源。统计关联与持久化分别由 gateway 和 application 承担。

`ResolvedCredential.profile` 显式区分 `Standard` 和 `CodexSubscription { account_id }`。Codex profile 仅供 Responses adapter 使用，目标必须是 `https://chatgpt.com:443/backend-api/codex`；ChatCompletions／Messages 和其他 origin／路径在发送前拒绝。Responses 构造必须使用 `ResponsesClient::new(ClientBuilder)`；该类型在构造末尾强制禁用重定向和自动重试，不接受已构建的任意 Client。gateway 启动时构造并复用，独立消费方可在 builder 注入代理、DNS 或证书信任但不能覆盖这两项策略，避免自定义 account header 被重定向发送到其他目标。每次请求使用 Bearer 与敏感的 `chatgpt-account-id`，附加 Codex backend 所需的 beta、originator 和客户端标识；这是非正式 Codex 接入，不是 Velune 的正式 SIWC 客户端身份。

Codex backend 只接受流请求。adapter 设置 stream=true、store=false，缺少 instructions 时传空字符串，保留调用方已有 instructions、完整输入及工具调用／结果。字符串输入转换为用户消息数组，system 角色映射为 developer，历史项的上游 id 被移除；状态引用与 item_reference 在发送前明确拒绝，不能据此补造历史。不支持的采样／输出限制／显式缓存字段和不支持的 service_tier 省略，并发出只含固定字段名的 tracing 诊断。它不注入 Codex 系统指令，不合成孤儿工具结果。普通 Responses profile 保持原生字段与已有 subscription 限制，Codex 的适配不影响它。当前 backend 用户代理约定依据 Magpie `9b04c4e20ff8bf10c745a53f562f5007b82d007d`，上游变更需单独复核。
