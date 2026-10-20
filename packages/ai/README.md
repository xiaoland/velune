# AI service 契约

本 unit 定义按操作分开的 AI 契约：原生 Chat Completions、原生 Responses、原生 Anthropic Messages，以及可选的 sampling 投影。原生操作只允许路由替换必要的模型标识，保留协议 JSON、headers、流式事件和响应状态；不会经过 sampling 重建。

`SamplingService` 只代表 sampling 操作，不代表整个 AI service。`Usage` 属于 sampling 业务结果；不保留 observation 的兼容 re-export。原生操作使用 operation-specific provider binding 和有限的 HTTP envelope，不依赖 reqwest 或具体厂商。

本包不读取应用配置、全局环境、认证文件，也不依赖 Harness、网关或 UniFFI。具体协议执行由 [ai-provider](../ai-provider/README.md) 负责。跨单元设计见 [AI service](../../docs/design/ai-service.md)。

`ProviderModelId` 明确表示提供商原生协议标识，保留完整字符串；它不承载应用记录主键或网关选择别名。原生请求与 provider allowlist 使用同一提供商身份，不提供旧 `ModelId` 别名。

Messages 保留调用方的 `anthropic-version`／beta 请求头与原生 JSON／SSE，缺少版本在派发前拒绝；HTTP API key 使用 `x-api-key`，Bearer 使用 `Authorization`；具体认证材料由调用方解析。提供商 baseURL 追加 `/v1/messages`，运行时 SDK 的网关注入使用 origin。

失败 kind 表达提供商边界已确认的事实，独立于 HTTP 状态：`Authentication` 是认证失败，`Permission` 是请求／账户权限限制，`QuotaExhausted` 是明确的用量耗尽，`Overloaded` 是服务容量暂时不足。原始 HTTP 状态与正文仍保存在 failure envelope 中；消费方决定重试与切换，不从 kind 伪造上游状态。
