# AI provider 执行

本 unit 实现 AI 契约的具体协议执行，包含原生 OpenAI Chat Completions、原生 Responses、原生 Anthropic Messages、Claude Code 本机执行后端与独立的 MiniMax sampling adapter。原生协议只替换路由所需的外部模型标识，保留请求字段、请求 headers、响应状态、响应 headers 和响应／SSE body；不经过 sampling mapper。它接受构造时传入的配置和一次性 `ResolvedCredential`，不读取应用配置、认证来源或 Harness 状态。

它依赖 [ai](../ai/README.md)，不拥有应用路由、自动重试或 fail-over 策略。网关组合协议执行；Claude Code 后端仅承担已明确授权的本机执行与工具桥接，不代替其他 Harness adapter。协议限制及手动 fixture 说明见 [AI service](../../docs/design/ai-service.md) 和 [开发说明](../../docs/development.md)。

原生操作收到已经解析的精确 `ProviderModelId`；本包不维护 Velune 记录到提供商型号的映射，不接受旧 `ModelMapping` 或 `ChatCompletionsOutputLimitField`。请求输出限制与推理字段由原生协议 body 决定。

Messages 保留调用方的 `anthropic-version`／beta 请求头与原生 JSON／SSE，缺少版本在派发前拒绝；HTTP API key 使用 `x-api-key`，Bearer 使用 `Authorization`；认证材料类型与协议适配分别校验。提供商 baseURL 追加 `/v1/messages`，运行时 SDK 的网关注入使用 origin。

`translation` 提供 ChatCompletions、Responses、Messages 六向 best-effort 转换。调用方只在异协议时构造请求转换计划，并用同一计划转换 JSON 或增量 SSE，保留工具别名的请求内映射。它不依赖 Harness、路由、应用配置或网络。无法对应的字段、状态引用和不透明推理信息尽量降级并给出不含业务值的 `ConversionNote`；真实错误和缺失终态不会被改造成成功。Messages 请求必须有输出上限，调用方可提供模型配置上限作为缺省值；同协议操作不经过此转换器。

转换语义由固定版本的 New API RelayKit 提供，Go 依赖见 `relaykit-bridge/go.mod` 和 `go.sum`。Rust 内部桥接管理唯一请求句柄，流转换消费请求计划，释放不会生成终态。桥接仅处理 JSON 与事件；HTTP、认证、取消和原始计量仍由 Rust 持有。不能跨提供商复用的服务端引用按 best-effort 省略并诊断，不补造历史；内联媒体在本地解析，外部媒体 URL 不触发桥接联网。诊断丢弃上游任意说明与字段值，不把原始错误送入日志。

Cargo 构建会调用 Go，以同进程库链接。Mac 使用 c-archive；Android 配置为 c-shared，需要平台打包相应共享库，不能把它作为常驻进程运行。目标映射不是运行验证，当前平台证据归 [接入任务](../../tasks/new-api-reuse/packet.md)。平台公开接口仍是 UniFFI，这个私有桥接不对 App 暴露。RelayKit 使用 AGPLv3，发行时必须连同适用源码义务处理；Mac 的原始声明和传递依赖许可保存在 [许可证目录](../../app/mac/Licenses/RelayKit/NOTICE.md)。

`metrics::ResponseMetrics` 从原生 JSON／SSE 提取用量、语义输出和业务终态，仅观察、不改写正文。独立未知字段和 Messages 缓存组成保留原始含义；协议转换占位不是其计量来源。统计关联与持久化分别由 gateway 和 application 承担。

`ProviderConfig.request_profile` 显式选择 `Native` 或 `CodexResponses`，不从认证来源推断协议行为。`ResolvedCredential` 分别携带 `CredentialMaterial`、`CredentialRecovery` 和 `RequestCapabilities`：HTTP 材料区分 Bearer／API key 与可选 account ID，本机会话材料只指定隔离的配置目录；恢复身份与是否允许显式输出上限也是独立事实。所有秘密材料不实现 Debug。`explicit_output_cap=false` 仅拒绝相应协议的输出上限字段，不隐含采样或缓存限制。

Codex request profile 仅用于 Responses HTTP adapter，目标必须是 `https://chatgpt.com:443/backend-api/codex`，并提供 Bearer 和 account ID；其他协议或目的地址在发送前拒绝。Responses 构造必须使用 `ResponsesClient::new(ClientBuilder)`；该类型在构造末尾强制禁用重定向和自动重试，不接受已构建的任意 Client。gateway 启动时构造并复用，独立消费方可在 builder 注入代理、DNS 或证书信任但不能覆盖这两项策略，避免自定义 account header 被重定向发送到其他目标。请求使用敏感的 `chatgpt-account-id`，附加 Codex backend 所需的 beta、originator 和客户端标识；这是非正式 Codex 接入，不是 Velune 的正式 SIWC 客户端身份。

Codex backend 只接受流请求。adapter 设置 stream=true、store=false，缺少 instructions 时传空字符串，保留调用方已有 instructions、完整输入及工具调用／结果。字符串输入转换为用户消息数组，system 角色映射为 developer，历史项的上游 id 被移除；状态引用与 item_reference 在发送前明确拒绝，不能据此补造历史。不支持的采样／输出限制／显式缓存字段和不支持的 service_tier 省略，并发出只含固定字段名的 tracing 诊断。它不注入 Codex 系统指令，不合成孤儿工具结果。Native Responses profile 保持原生字段与显式请求能力限制，Codex 的适配不影响它。当前 backend 用户代理约定依据 Magpie `9b04c4e20ff8bf10c745a53f562f5007b82d007d`，上游变更需单独复核。

Responses 流响应按 MIME 规则忽略媒体类型大小写和两端空白。成功 HTTP 状态且显式声明非 SSE 类型的响应在交付 headers 前返回协议错误。仅 `CodexResponses` profile 的流请求，在成功响应真正没有 Content-Type header 时，仍进入原始 SSE 解析路径并向下游补充 `text/event-stream`；这是 2026-10-16 真实 Codex backend 响应已观察到的约定。普通 profile 仍要求声明 SSE，已存在但无效或错误的 MIME 不享有例外。解析仍要求显式业务终态，空流、HTML 或缺终态都会失败；headers 提交后不切换其他账户。`responses_stream_content_type_rejected` 诊断只记录上游状态、固定 MIME 分类与固定编码分类，不包含原始 header 值或正文。


`claude_code::ClaudeCode` 实现 MessagesProvider，输入为 Messages 请求，输出为 Messages SSE；网关负责其他入口协议及非流响应转换。`ClaudeCodePool` 由消费方跨 Runner 共享，拥有独立执行线程、CLI 进程和挂起的 MCP 工具请求。配置明确给出可执行文件、隔离 config_dir、binding 与可选 session；不通过 shell 拼接命令。后端禁用 Claude 原生工具与自主执行来源，只把调用方声明的工具转成 tool_use，等待调用方真实 tool_result。进程成功启动后或向已有进程写入本次输入前，失败的 `submitted` 即为 true，调用方不得透明重放到其他实例，即使还没有响应 headers。

CLI 子进程清空继承环境后，仅保留系统用户身份所需的 `HOME`、`USER`、`LOGNAME`、命令查找所需的 `PATH`，以及 Windows 必要系统变量。真实 `HOME` 用于 macOS 用户钥匙串定位；订阅配置仍由显式独立的 `CLAUDE_CONFIG_DIR` 隔离，不把配置目录伪装成用户主目录。API key、OAuth token 和服务端点覆盖不从父进程环境继承。原生工具、hooks、额外 settings 与 MCP 自动发现的禁用约束保持有效。

Pool 的 continuation 查询与实际工具等待状态同源，返回无约束、原 binding 归属或已失效。失效会终止对应进程并保留必要的会话失效事实；消费方负责将该约束与自己的模型资格、认证世代共同校验。`cancel_session` 也取消已返回 tool_use 后等待结果的交互；应用在明确的新用户轮调用 `begin_user_turn`，只有已终止交互可以解除失效，仍运行或 pending 时拒绝。已失效工具 ID 独立保留，`validate_result_ids` 只接收当前请求尾部结果，防止新轮错误接收迟到结果。关闭单个 HTTP Runner 不应 shutdown 共享 pool，应用生命周期结束时才显式 shutdown。
