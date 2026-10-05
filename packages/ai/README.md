# AI service 契约

本 unit 定义按操作分开的 AI 契约：原生 Chat Completions、原生 Responses，以及可选的 sampling 投影。原生操作只允许路由替换必要的模型标识，保留协议 JSON、headers、流式事件和响应状态；不会经过 sampling 重建。

`SamplingService` 只代表 sampling 操作，不代表整个 AI service。`Usage` 属于 sampling 业务结果；`observation::Usage` 仅为旧调用方提供兼容 re-export。原生操作使用 operation-specific provider binding 和有限的 HTTP envelope，不依赖 reqwest 或具体厂商。

本包不读取应用配置、全局环境、认证文件，也不依赖 Harness、网关或 UniFFI。具体协议执行由 [ai-provider](../ai-provider/README.md) 负责。跨单元设计见 [AI service](../../docs/design/ai-service.md)。
