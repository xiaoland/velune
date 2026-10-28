# AI service 契约

本 unit 定义 AI 能力的权威契约。当前唯一的操作是原生 LLM 派发：`Protocol` 区分 OpenAI Chat Completions v1、Responses v1 与 Anthropic Messages，三者共用 `NativeRequest`、`NativeEvent`、`NativeFailure` 与 `NativeCompletion`，由 `NativeProvider` 对一个确定目标执行一次请求。原生操作只允许路由替换必要的模型标识，保留协议 JSON、headers、流式事件和响应状态。

本包不读取应用配置、全局环境、认证文件，也不依赖 HTTP 客户端、Harness、网关或 UniFFI。具体协议执行由 [ai-provider](../ai-provider/README.md) 负责。跨单元设计见 [AI service](../../docs/design/ai-service.md)。

`ProviderModelId` 明确表示提供商原生协议标识，保留完整字符串；它不承载应用记录主键或网关选择别名。`Protocol` 的序列化名（`chatCompletionsV1` 等）也是网关配置与应用 API 中的协议名。

失败 kind 表达提供商边界已确认的事实，独立于 HTTP 状态：`Authentication` 是认证失败，`Permission` 是请求／账户权限限制，`QuotaExhausted` 是明确的用量耗尽，`Overloaded` 是服务容量暂时不足。原始响应头与正文仍保存在 failure 中，`submitted` 表示请求已交给传输层；消费方决定重试与切换，不从 kind 伪造上游状态。成功的 `completed` 只在协议自身报告完成时为真，目前仅 Responses 报告。
