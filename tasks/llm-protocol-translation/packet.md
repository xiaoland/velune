# LLM 三协议 best-effort 转换

状态：2026-10-06 实现与隔离验收已完成第一轮，最终干净构建、安装及该产物的客户端复核待完成。

用户授权为跨 Harness 对话实施 ChatCompletions v1、Responses v1 与 Anthropic Messages 全部双向转换，明确采用 best-effort 而不是 fail-closed。同协议继续原生透传；异协议尽量保留消息、工具与参数，无法对应的字段可降级或省略并提供不含业务值的静态诊断，不伪造历史、工具执行或成功终态。Messages 输出上限优先取请求，缺省使用目标模型配置；两者都没有时提示必要配置缺失。长期意图归 [PRD](../../docs/prd/index.md)，职责和取舍归 [AI 服务设计](../../docs/design/ai-service.md)。

root 负责 gateway、application、bindings 的接线、长期文档和构建安装；llm_translation owner 负责 ai-provider 的转换模块；ai_service_audit owner 负责官方源码研究与 manual-protocol-translation.py；session_management owner 负责 Mac 接线和 manual-protocol-runtime-loop.py 实际客户端验收。advisor 已复核私有语义表示、best-effort 和工具映射决策。仓库缺失 guides/delegation.md，仍依据共享规则按稳定 owner 协调。没有新增或运行自动化测试、读取真实配置／凭据／会话或调用真实提供商。

实现使用现有 ai-provider unit 的请求范围转换计划与私有 LLM 表示，不通过 SamplingOutput 重建，不新增 package、协议翻译运行时或会话正文数据库。SSE 分帧复用已有 eventsource-stream。文本即时交付；目标 Messages 与 custom 工具参数需在工具完成时转换合法结构，累积仍有界。无法解析的参数保留 raw 内容并诊断。合法终态后的未知尾事件忽略并诊断；没有终态的 EOF 仍中断，不能补造完成。缺失计量保持未知，Messages 的必需数字占位有单独诊断；生成的转换 ID 只在本次响应中有效。

application 区分运行时入口和提供商上游：Codex 继续发 Responses，DSH 继续发 ChatCompletions，Pi 优先同协议。UI 用瞬态 supportedProviderProtocols 判断网关可达模型，supportedProtocols 继续表示版本化 runtime 的原生 wire 能力。不修改 schema 7，认证始终按上游协议与端点解析，fail-over／自动重试仍 Disabled。

第一轮 fmt、workspace check、workspace/all-targets/all-features clippy -D warnings、bindings no-default-features check/clippy 和 Mac release warnings-as-errors 均通过。Mac owner 另完成严格 debug 构建。转换 owner 的临时 Rust 人工可执行检查六向 JSON／SSE、custom namespace、并行工具最终 item 身份一致性、未知 usage、唯一 ID、终态与 raw 参数降级，均通过；不将直接转换器检查当成端到端证据。

原生回归已通过：manual-gateway-native.py 的九个 JSON／SSE 原生路径及取消／关闭；manual-pi-native-loop.py 的六次合成请求覆盖导入→创建→工具→续轮；manual-cross-harness.py 的四次 Pi→Codex→DSH→Pi 原生 turn、重启投影与关联正文隔离。这些仅证明原生回归。

实际外部客户端第一轮已通过：Codex 0.159.3 通过 application 发 Responses，分别路由到 ChatCompletions／Messages 合成上游，执行真实文件读取工具与续轮；namespace 工具的不存在目标错误结果也能返回并续轮。外部 Pi 1.0.2 公共 SDK 强制 Responses 入口、ChatCompletions 上游，解析工具调用、读取合成文件并续轮。合计五条闭环十次上游请求，没有隐藏重试。该隔离 Codex 声明没有 custom 工具，因此 custom codec 只具转换器合成证据，不冒充真实客户端验收。最终构建后仍须重跑该脚本。

人工入口：manual-protocol-translation.py 接受 --deps 与 --rustc；manual-protocol-runtime-loop.py 接受 --bundle、--bindings、--node、--pi、--codex，均显式要求已有绝对路径。使用临时 HOME 与 loopback，不加入 CI。最终产品保持 0.1 beta.1、不捆绑 Agent、不发布远端；真实提供商与 UI 体验由用户验收。

六向真实 Runner 验收最终通过：manual-protocol-translation.py 共29次合成上游请求，覆盖六向流式及六向 JSON、两轮并行工具、CRLF 和 UTF-8 字节中间分片、usage／finish、未知字段／状态引用、合法 incomplete、终态前截断、终态后未知尾事件、上游 HTTP 错误和取消。正常流不得吞 IncompleteRead，截断 fixture 明确检查连接中断，不以“收到若干 bytes”冒充成功。Messages 输出上限模型缺省与缺少必要配置两种路径均有断言。

源码研究日期为本日。LiteLLM 固定 d4619c499a6052913c8fa548192966dd76c3fec9：[Anthropic Chat](https://github.com/BerriAI/litellm/blob/d4619c499a6052913c8fa548192966dd76c3fec9/litellm/llms/anthropic/chat/transformation.py)、[OpenAI Chat](https://github.com/BerriAI/litellm/blob/d4619c499a6052913c8fa548192966dd76c3fec9/litellm/llms/openai/chat/gpt_transformation.py)、[Responses](https://github.com/BerriAI/litellm/blob/d4619c499a6052913c8fa548192966dd76c3fec9/litellm/llms/openai/responses/transformation.py)、[Anthropic Responses adapter](https://github.com/BerriAI/litellm/blob/d4619c499a6052913c8fa548192966dd76c3fec9/litellm/llms/anthropic/pass_through/responses_adapters/transformation.py)。借鉴按协议拆分 request／response／stream 转换与请求范围状态，不复用 Python 运行时、全局 provider 状态或隐藏重试。Magpie 沿用此前固定 db55bc8b7a70126fd3a7bede95a56b31c6f20c5f 的 [研究证据](../magpie-reference/packet.md)，不引入其厂商专用改写和400重试。协议依据仍为 [OpenAI Chat](https://developers.openai.com/api/reference/resources/chat)、[Responses streaming](https://platform.openai.com/docs/api-reference/responses-streaming)、[Anthropic Messages](https://docs.anthropic.com/en/api/messages)；跨协议不同能力只能作 best-effort，具体提供商行为仍由用户实测。
