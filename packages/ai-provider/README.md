# AI provider 执行

本 unit 实现 AI 契约的具体协议执行，包含 MiniMax／OpenAI Chat Completions 与原生 Responses。它接受构造时传入的配置和认证值，不读取应用配置或 Harness 状态。

它依赖 [ai](../ai/README.md)，不拥有应用路由、自动重试或 fail-over 策略。网关组合协议执行而不把 Harness 适配放入此包。协议限制及手动 fixture 说明见 [AI service](../../docs/design/ai-service.md) 和 [开发说明](../../docs/development.md)。

OpenAI Chat Completions 的 `ModelMapping` 通过 `ChatCompletionsOutputLimitField` 选择输出限制字段：默认使用 `MaxCompletionTokens`，也可显式使用仍属于该协议的 `MaxTokens`。每次请求仅发送选中的一个字段；此选项属于提供商模型的 wire 编码，不改变 sampling 契约，也不引用 Harness 兼容类型。
