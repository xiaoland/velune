# AI provider 执行

本 unit 实现 AI 契约的具体协议执行，包含原生 OpenAI Chat Completions、原生 Responses 与独立的 MiniMax sampling adapter。原生协议只替换路由所需的外部模型标识，保留请求字段、请求 headers、响应状态、响应 headers 和响应／SSE body；不经过 sampling mapper。它接受构造时传入的配置和一次性 `ResolvedCredential`，不读取应用配置、认证来源或 Harness 状态。

它依赖 [ai](../ai/README.md)，不拥有应用路由、自动重试或 fail-over 策略。网关组合协议执行而不把 Harness 适配放入此包。协议限制及手动 fixture 说明见 [AI service](../../docs/design/ai-service.md) 和 [开发说明](../../docs/development.md)。

旧配置中的 `ChatCompletionsOutputLimitField` 仍由 application 往返保存，但 native Chat Completions operation 不读取它，也不替请求添加任何输出限制或推理字段。它不再是 provider native path 的协议决策点。
