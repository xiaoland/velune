# AI provider 执行

本 unit 实现 AI 契约的具体协议执行，包含原生 OpenAI Chat Completions、原生 Responses、原生 Anthropic Messages 与独立的 MiniMax sampling adapter。原生协议只替换路由所需的外部模型标识，保留请求字段、请求 headers、响应状态、响应 headers 和响应／SSE body；不经过 sampling mapper。它接受构造时传入的配置和一次性 `ResolvedCredential`，不读取应用配置、认证来源或 Harness 状态。

它依赖 [ai](../ai/README.md)，不拥有应用路由、自动重试或 fail-over 策略。网关组合协议执行而不把 Harness 适配放入此包。协议限制及手动 fixture 说明见 [AI service](../../docs/design/ai-service.md) 和 [开发说明](../../docs/development.md)。

原生操作收到已经解析的精确 `ProviderModelId`；本包不维护 Velune 记录到提供商型号的映射，不接受旧 `ModelMapping` 或 `ChatCompletionsOutputLimitField`。请求输出限制与推理字段由原生协议 body 决定。

Messages 保留调用方的 `anthropic-version`／beta 请求头与原生 JSON／SSE，缺少版本在派发前拒绝；API key 使用 `x-api-key`，未确认的 subscription 认证不适用于此协议。提供商 baseURL 追加 `/v1/messages`，运行时 SDK 的网关注入使用 origin。
