# 模型身份与网关业务模型研究

复核日期：2026-10-05。本文是任务研究证据，不替代 `docs/prd` 或 `docs/design` 的权威契约。

## 结论

`model-id` 在协议请求中是提供商定义的模型标识，必须原样传给对应提供商。它不是 Velune 的内部主键，也不能因为某个字符串适合 UI、路由或持久化就把它命名成模型 ID。

Velune 需要同时保留三种不同身份：

| 层次 | 建议语义 | 示例 | 是否发送给上游 |
| --- | --- | --- | --- |
| 提供商模型标识 | provider model ID，来自提供商目录、运行时配置或用户输入 | `gpt-5.1`, `deepseek-chat`, `claude-sonnet-4-6` | 是，原样发送 |
| Velune 配置记录 | 内部 record key，用于编辑、持久化引用、删除和迁移 | `model_record_…` | 否 |
| 网关入口选择 | route alias / selection key，用于调用方选择策略或固定路由 | `velune/auto`, `velune/<runtime>` | 否；网关解析后得到上面两项 |

一个提供商实例（endpoint、协议、认证资源和提供商配置）可以暴露多个 provider model ID；同一个提供商模型 ID 也可能在不同 endpoint、账户或协议实例中出现。按用户已确认的产品语义，Model 是独立的业务概念，表示真实可识别的模型或型号（包括版本名、滚动名等），而不是任意 capability 桶。一个 Model 可以关联多个提供商的 binding，但这种关联必须来自来源证据或用户明确选择；不能按同名或 hash 自动推断。不同 binding 仍可能有不同的参数、价格、上下文限制或协议能力。

因此，当前代码里的 `ModelDefinition.id` 与 `ProviderModelBinding.external_model_id` 已经暴露出正确方向，但命名和约束仍然错误：前者实际承担内部逻辑 ID，后者才是提供商模型 ID；`Route.model_id` 又把内部逻辑 ID用作网关入口。继续沿用这些名字会让 UI 和调用者误以为可以编辑一个“Velune model-id”。它们应在后续契约修订中拆成明确的 `record_key`、`provider_model_id` 和 `route_alias`（确切命名由实现阶段决定），不能只在界面上改标签。

## 一手资料中的共同模型

### OpenAI API

OpenAI 的模型对象把 `id` 定义为可以在 API endpoint 中引用的模型标识；`GET /models/{model}` 的路径参数 `model` 就是该 ID。[OpenAI Models API Reference](https://developers.openai.com/api/reference/resources/models) 也把模型对象描述为可用于 API 的模型 offering，并将 `owned_by` 作为单独字段。Responses 请求同样使用 `model` 字符串；响应记录的 `model` 表示实际用于生成的模型。

这说明 `model` 是请求协议字段，模型目录是提供商的事实来源。它不是客户端数据库的主键，也不是一个可由网关为了“统一”而改写的名字。提供商支持 alias 时，请求的 alias 和返回的实际 model 还可能不同，观测数据应同时保存 requested 与 effective model。

### Vercel AI SDK

AI SDK 将语言模型对象表示成 `{ provider, modelId }`，而生成结果会优先采用提供商响应里的 `modelId`；文档还明确指出提供商支持 alias 时，响应中的实际模型可能不同。[generateText reference](https://ai-sdk.dev/docs/reference/ai-sdk-core/generate-text)

AI SDK 的 provider registry 使用 `providerId:modelId` 作为应用侧查找字符串；文档明确说 provider ID 会成为 model ID 的前缀。[Provider & Model Management](https://ai-sdk.dev/docs/ai-sdk-core/provider-management) 这是 SDK registry 的命名空间，不是上游 HTTP `model` 字段的替代品。`customProvider` 还允许把短名称映射到真实模型并预置 provider settings。[customProvider reference](https://ai-sdk.dev/docs/reference/ai-sdk-core/custom-provider) 因而 `opus`、`sonnet` 这类短名是应用 alias，真正的 provider model ID 仍由底层 provider model 实例持有。

AI SDK 把常见设置（如 `maxOutputTokens`、`temperature`）放在统一调用面，同时保留 provider-specific options；这是一种调用抽象，不表示每个提供商都有相同字段或语义。Velune 的协议层应以提供商协议和模型能力为权威，不能把统一 UI 字段当成所有 provider model 的事实模型。

### Mastra

Mastra 的 Agent 示例使用 `openai/gpt-5.6-sol`、`anthropic/claude-sonnet-4-6` 这样的 model router 字符串；这是 Mastra model router 的 provider/model 命名空间，目的是让 Agent 选择模型并由路由器接入多个提供商。[Model Router](https://mastra.ai/blog/model-router) Mastra 的 Agent Controller 也把默认模型、会话模型切换和按 agent type 的模型选择作为运行时设置，而不是把模型名当作 Agent 的身份。[Agent Controller](https://mastra.ai/docs/harness/agent-controller)

Mastra 的 `ModelSelectionProcessor` 进一步说明“模型选择”和“模型身份”是两件事：选择规则可以按请求在 `openai/...` 与另一个模型之间路由，模型字符串仍然指向具体 provider/model 入口。[ModelSelectionProcessor](https://mastra.ai/reference/processors/model-selection-processor) 这支持 Velune 将 route alias、选择策略与 provider model ID 分开。

Mastra Gateway 的业务定位是单一接入点、模型选择和跨提供商 failover；它对任何 Agent stack 工作，而非 Agent 本身。[Mastra Gateway](https://mastra.ai/ai-gateway) 这与 Velune 的 LLM Gateway 方向一致，但不意味着 Velune 可以把 Mastra 的 `provider/model` 字符串直接当作自己的内部主键。

### Cloudflare AI Gateway

Cloudflare 的 Unified API 示例把 `custom-some-provider/model-name` 放在请求的 `model` 字段中；其中前缀是网关 custom provider slug，后半段是上游模型名。[Custom Providers](https://developers.cloudflare.com/ai-gateway/configuration/custom-providers/) 对 provider-specific endpoint 的示例则直接把 `model-name` 传给上游，文档明确展示了 gateway URL 到 upstream URL 的映射。

Dynamic route 使用 `dynamic/<route-name>` 作为 `model`，响应通过 `cf-aig-model` 和 `cf-aig-provider` 告知实际选择结果。[Dynamic routing](https://developers.cloudflare.com/ai-gateway/features/dynamic-routing/usage/) 这是 route alias 的清晰实例：请求中出现的是网关选择键，响应中另有实际 provider 与 model。它不能证明 `dynamic/foo` 是提供商 model ID，反而证明两者必须分离。

## 对 Velune 当前实现的审计

当前 `packages/application/src/config.rs`：

* `ModelDefinition.id` 被用于 `Route.model_id`、运行时选择和持久化引用，并在 Pi 导入中由 `stable_id("pi_model", ...)` 生成。这是内部记录身份，不是 provider model ID。
* `ProviderModelBinding.external_model_id` 保存了来源模型的 `model.id`，并在 `packages/application/src/config.rs` 组装 gateway 时传入 gateway binding；它实际承担 provider model ID，但 `external` 命名会掩盖其协议职责。
* `provider_import.rs` 的 `model_id` 变量有时指候选内部 ID，有时指来源 `model.id`。这会让选择、映射和持久化边界混淆，也是导入 UI 容易出现错误语义的根因。

当前 `packages/gateway/src/config.rs` 与 `runtime.rs`：

* gateway route map 以 `Route.model_id` 为 key，同时 binding 里另存 `external_model_id`，转发时再用后者。这是“内部 route key → provider model ID”的必要映射，但命名应明确表达映射，而不是把 route 的 key 称作 model ID。
* `runtime.rs` 的 `aliases: BTreeMap<String, String>` 已经是独立的 route alias 机制。它应保持为入口选择键，不能回写成 ModelDefinition 的 ID，也不能让 alias 取代 provider model ID。

当前 `app/mac/Models.swift` 与 `Views.swift` 把 `AIModel.id` 作为模型编辑、展示、route picker 和删除的值；这可以作为内部记录 key，但 UI 应展示 provider model ID 作为只读或提供商信息，并把 nickname 作为本地显示字段。让用户编辑“ID”会误导他们以为是在修改提供商目录里的 model ID；如果确实需要修改 provider model ID，应明确是 provider binding 的协议字段，并影响路由校验。

## 可执行的契约判断

1. 保存 provider binding 时，`provider_model_id` 必须是该提供商协议所需的精确字符串；网关原生透传时不得把它替换成 Velune record key 或 route alias。
2. 内部 record key 可以存在，因为编辑、迁移和跨提供商展示需要稳定引用；但它不应叫 `model_id`，也不应出现在发给上游的请求中。
3. Model 保留为独立业务实体；最小结构是内部 `record_key` 加一个或多个 `ProviderModelBinding`。binding 的 `provider_model_id` 和实际协议能力/限制属于提供商绑定，不新增独立 Capability/Offering 实体，也不因同名或 hash 自动合并跨提供商模型。
4. `velune/auto`、`velune/<agent-harness-id>` 这类值是网关路由入口或选择策略；解析结果必须记录 effective provider、endpoint 和 provider model ID，便于观测和诊断。
5. 模型参数和能力以协议/provider/model offering 为事实来源。统一层只承诺协议允许且该 offering 声明支持的字段；provider-specific fields 应原样保留在对应协议配置中。
6. 任何 SDK 的 alias、gateway prefix 或 Agent framework 字符串都只能作为调用方命名空间的参考，不能作为“模型 ID 由 Velune 自行生成”的依据。

## 参数作用域的具体错误

当前配置把“模型能力元数据”和“本次请求默认参数”混成了硬必填字段：`packages/application/src/config.rs` 的 `ModelDefinition.max_output_tokens: u32` 与 `reasoning_levels: Vec<String>` 没有协议或来源能力的可选性；`packages/gateway/src/config.rs::validate` 又要求每个模型的 `max_output_tokens > 0`，并在有 context window 时强制比较二者。于是一个原生 Chat Completions 或 Responses provider，即使只需要透传请求、没有可发现的最大输出上限或推理等级，也无法成为合法 gateway model。

同样，Pi 导入在 `packages/application/src/provider_import.rs` 中只接受 `context_window > 0` 与 `max_tokens > 0` 的来源模型，随后把 `model.max_tokens` 写进通用 `ModelDefinition`；`packages/application/src/local/pi_composition.rs` 又把这个值投影成 Pi 的 `maxTokens`，把 Pi 适配器需要的字段误升格成所有协议的模型必备字段。`app/mac/Views.swift` 的模型编辑器也把“最大输出 Token 数”作为保存模型的必填校验。这里至少应区分：

* provider/model 的声明能力与限制：来源提供商或协议目录给出的可选 metadata；缺失表示未知或该协议不声明，不等于零；
* 某次请求的参数：由调用方、运行时或 provider-specific defaults 决定，只在请求确实携带时透传；
* Pi projection 的兼容字段：只在 Pi 适配器需要时生成，不回写成通用 AI Model 的硬约束。

这是结合用户需求得出的 Velune 修正建议，不是上述官方资料共同规定的字段命名。官方资料只证明 provider model ID、实际响应模型和 provider-specific settings 的边界；具体数据结构仍由 Velune 契约决定。

## 后续实施归属

本研究保存重构前证据，不描述当前源码。后续已确认保留真实 Model 与 ProviderModelBinding 两层，内部 recordKey 自动生成且隐藏，route alias 自动派生，来源导入可显式关联跨提供商模型。没有将 Model 改成 capability 容器，也没有预建协议目录发现。当前权威契约归 [AI 服务设计](../../docs/design/ai-service.md)，实施与验收归 [模型重构记录](model-implementation.md)。
