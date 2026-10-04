# AI service 契约

本 unit 定义 AI 操作、提供商派发与观察契约。语言模型的消息、采样与 token 语义归对应操作，不成为所有 AI 服务的基础假设。

本包不读取应用配置、全局环境、认证文件，也不依赖 Harness、网关或 UniFFI。具体协议执行由 [ai-provider](../ai-provider/README.md) 负责。跨单元设计见 [AI service](../../docs/design/ai-service.md)。
