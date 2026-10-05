# AI service 的权威契约与 provider 边界

状态：2026-10-03 有界 MiniMax 流式实现；文本终止问题已定位修复并由真实诊断记录离线复验，工具 live 与重复 replay 成功。service 是通用独立 lib，sampling 是其首个操作；不引入账户／渠道实体。授权与运行证据见 [任务](../../tasks/ai-service-contracts/packet.md)。旧 Host／UI／mock Router 未接线。

## 网关配置与首循环修订

2026-10-04 用户纠正首循环：AI 提供商只是 Velune AI 服务网关配置的一部分。全局模型目录保存模型身份、昵称、图标、输出上限和支持的推理级别；提供商通过模型映射关联目录项。协议由有限枚举表达，首先支持 OpenAI ChatCompletions v1；不将自由文本当作协议契约。模型路由与 fail-over 策略属于网关，不塞入提供商配置。

执行 Harness 不直接使用上游认证。嵌入式 CoreRuntime 装配本机 Velune 网关，向选定的 Agent 运行时实例注入网关端点、模型目录与本地访问凭据；上游授权只在提供商装配边界使用。用户可以明确委托 Harness 的认证来源；原存储保留，同一来源负责刷新，不复制为第二份认证权威。适配器不向 Harness 传递上游端点或 Keychain 引用。此次实现先提供显式模型到提供商的路由；自动策略尚未定义，默认禁用 fail-over，不展示可启用的空策略。

网关的参数编码依据 [OpenAI Chat Completions 官方参考](https://developers.openai.com/api/reference/resources/chat/subresources/completions/methods/create)，运行时配置注入依据 [Pi 1.0.2 模型配置源码](https://github.com/earendil-works/pi/blob/cd32f7725fdbddbaecdff5b1e68491563394e0ca/packages/coding-agent/docs/models.md)，于 2026-10-04 复核。内部输出上限与推理级别由 provider 转换为所选协议的字段，不以 UI 参数名称代替 wire 验证。

下文 MiniMax 的固定 host／模型、采集预算与历史验收仍只描述原有有界 adapter，不是通用 Chat Completions 网关的能力证据。新实现与隔离假上游验证状态归属 [首循环任务](../../tasks/pi-mac-first-loop/packet.md)，不能沿用 MiniMax fixture 宣称新网关已通过。

## Harness 提供商配置导入

提供商配置导入是一项完整功能，覆盖来源中的提供商、协议、端点、有效模型及能力参数，并包含认证来源。认证解析不是另一项可以代替导入的交付。Core 适配器提供非秘密预览与应用动作，平台 UI 依据通用描述展示来源和候选项，不解释 Pi 配置。

首个适配器固定读取 Pi 1.0.2 的有效配置。用户选择已配置的运行时实例；application 在预览和应用时重新解析该实例的目录与执行配置，不接受调用方覆盖路径。实例或来源变化使旧预览失效。读取不要求运行时先连接或已有默认模型。预览不执行配置中的凭据命令、不刷新认证、不访问模型服务。提供商按实际端点与协议分组；无法由当前网关保持语义的配置须显示原因，不能默默剥离后声称支持。导入保留原文件；普通凭据引用原来源，OAuth 仍由来源适配器在原存储的锁内刷新，Velune 配置不保存秘密值。

Core 装配配置中的提供商模型映射可以携带 `piProjection`，其类型和转换归 Pi adapter；它不是通用 AI 模型能力。全局目录保留本轮对话操作的身份、显示和预算，Pi adapter 将允许级别与来源 Pi 级别求交，并保留 off→none 等 SDK 映射及九项 Responses 编码选项。适配器同时派生原生 Responses 的允许 effort 值域；网关只校验 wire 值，不理解 Pi 七级、不执行级别转换。投影变化进入物理绑定身份，来源派发重新核对同一投影。未知 compat、自定义 headers、采样参数及启用 session affinity 请求头的配置继续明确标记不支持，不声称完整请求头透传。

Chat Completions 导入接受与当前网关请求行为相容的已知兼容设置，例如 `supportsStore: false`、`maxTokensField: "max_completion_tokens"` 和 `thinkingFormat: "openai"`，不因兼容对象非空而整体拒绝。网关经 sampling 契约重新编码请求，提供商模型绑定以类型化 wire 选项选择 `max_tokens` 或默认的 `max_completion_tokens`；Pi 来源字段由 application 显式转换，不进入通用 AI 模型参数。禁用 `stream_options.include_usage`、禁用 `reasoning_effort` 或使用尚未实现的供应商推理参数格式的来源仍显示具体原因。适配器同时检查 Pi 1.0.2 从原提供商与 URL 推断的相关默认值；改成受管提供商和本地网关地址不能消除来源的请求要求。这一判定绑定固定 SDK 版本，升级时须重新核对其默认推断与网关编码，不能沿用假定。Pi 新 ChatGPT 订阅仍使用受支持的 `openai-responses`；旧 `openai-codex-responses` 使用另一后端与认证契约，不作为普通 Responses 的别名。未实现的 Pi 协议按实际 API 标识说明，不将其附加为 Chat Completions 兼容问题。

`packages/ai` 与 AI provider lib 不引用 Pi 类型或任何 Harness 配置，独立 Responses 是已实现的一项协议操作，sampling 是另一项操作；这些具体操作不成为整个 AI 服务领域的基础模型。Mac 只编辑通用绑定字段并往返保留 Core 验证的适配元数据，不解释 Pi 投影。

来源配置在导入时形成快照，不做双向同步。重复项默认跳过，替换须明确选择；来源模型可以关联已有全局模型，但不覆盖该模型参数。导入不自动改变路由或运行时默认模型。应用前重新核对来源和目标配置，变化后要求重新预览；派发时核对保存的来源执行绑定，避免来源端点改变后将凭据发送到另一个目标。当前实现与人工验证记录见 [首循环任务](../../tasks/pi-mac-first-loop/packet.md)。

## 原生 Responses 操作

2026-10-04 用户授权支持 OpenAI Responses v1，暂不翻译协议。Responses 使用 AI service 自有的独立操作契约，不能经 `SamplingInput`／`SamplingDelta` 往返转换：这些采样类型无法表达完整 output items、encrypted reasoning 与原生事件。协议传输由 ai-provider 实现，网关负责入口、模型路由与访问校验。

首轮覆盖 foreground `POST /v1/responses` 的 JSON 与 SSE。每次调用捕获不可变目标，将 Velune 逻辑 ID 或 Pi 绑定 ID 解析为同一逻辑路由，再替换为外部模型 ID；其它请求字段按原生契约保留或明确拒绝。协议不匹配、未支持的状态型请求与无效能力不能隐式转为 ChatCompletions。原生 completed、incomplete、failed 与 cancelled 各自保留；HTTP 200、EOF 或 `[DONE]` 不独自证明 Responses 完整成功。断连和取消停止本次上游等待，不伪造成功终态。

认证方式可能进一步限制协议能力。Pi 新 ChatGPT 订阅 adapter 明确省略普通 Responses 的部分参数；共享其认证来源不使这些参数获得支持。限制必须由来源能力投影给 Harness 或在边界拒绝，不能靠网关悄悄删字段。当前实现与隔离证据仍以首循环 Task Packet 为准。

## 两套契约与依赖

```text
app main → AiService::sampling(request, attempt_id, event_sink)
                  ↓ DirectAiService（单个 immutable binding）
       service-owned SamplingProvider::sampling(request, delta_sink)
                  ↑ MiniMax adapter implements
                  ↓ reqwest / SSE / Chat Completions
```

[velune-ai](../../packages/ai/src/lib.rs) 拥有调用方与 provider 两套权威契约，不依赖 provider crate、HTTP、SSE、厂商 JSON 或 SDK。provider 单向依赖 service；[MiniMax](../../packages/ai-provider/src/minimax/mod.rs) 映射外部协议，不再增加平级 sampling 执行服务。serde_json 在 service 中只表达调用方工具 schema／参数，不表示厂商 envelope。

`AiService::sampling` 同步校验目标并捕获 ProviderBinding，返回 Future；Future 被 poll 才派发。调用方传入独立 CallId 和 AttemptId；此步无全局 ID 生成器／去重库，调用方保证唯一。DirectAiService 不选择 provider、不重试／fallback。一个 provider Future 完成后，service 发恰好一个 Terminal 并返回 SamplingCompletion、CallObservation 和可选 AttemptObservation。未 poll／drop／panic 路径没有终态保证，不标为已验收的取消／恢复协议。

## 配置与装配

统一配置中心拥有持久配置，app main 创建新的 immutable provider／binding／service。两个 lib 不读 env、文件、全局配置，也不保存配置。[手动入口](../../packages/ai-provider/examples/minimax_manual.rs) 是本步 composition root：读取获准 Networksecret 占位、创建带标准环境代理及系统 TLS 校验的客户端，禁 retry／redirect，注入内存凭据。不是正式配置中心或产品入口。

ProviderConfig 保留协议、CredentialRef、模型映射、ProviderId／ConfigRevision。新增独立 ChatCompletionsConfig；Messages 仍仅配置形状；Responses 原生操作正在本轮实施。MiniMax adapter 限定 HTTPS `api.minimax.cn:443/v1`、`MiniMax-M3`，`thinking:disabled`、`service_tier:standard`，不启用内置收费工具。HTTP 客户端的安全装配属于 app；adapter 接受已装配 client，不能从类型上证明任意第三方传入的 client 均关闭重试。

ProviderBinding::prepare 捕获 Arc 与 revision；adapter 自身也持有 immutable 配置和凭据，不重读配置。Arc 不证明其他 trait 实现无内部可变性；并发热切换、凭据轮换尚未实测。HttpEndpoint／ID 构造器仍是语法边界，不是完整网络安全策略。

## 流事件、结果与观测

SamplingDelta 仅表达 Text、ToolIdentity、ToolArguments、Finish、Usage；service 统一包装 AttemptContext 与递增 sequence。工具 index 是 attempt 内流组装索引；ID／名称与参数片段分开传递。参数仅在完成时解析为 JSON object，工具调用只返回、不执行。sink 同步消费提供自然背压，不启后台任务或无界 channel；用户 sink 应短小且不 panic。

Finish 是模型停止生成的原因，可能先于最后 usage；Terminal 表示整个操作结束。adapter 接受两种经校验终止：已解码的 `[DONE]`，或 clean transport EOF＋所有 SSE 行／帧闭合＋显式 finish＋已报告 input/output usage；两者都必须有完整有效输出／工具参数。后者由真实字节层诊断与官方推荐 OpenAI SDK 的自然迭代结束行为支持，并非把任意 EOF 当成功。缺少 finish／usage、未闭合帧、解析／传输错误、unsupported delta、identity 冲突等返回 typed failure，保留已知 usage 与 PartialSamplingOutput；半截参数保持字符串，不伪造完成工具调用或 finish。output limit 且工具参数不完整仍失败并保留 partial。

每个 usage 值独立 Unknown／Reported／Estimated，缺失不填零。当前 service 映射 prompt_tokens→input、completion_tokens→output；total_tokens 留 fixture，不当成另一个计费数量。其他 usage 扩展字段只记录存在性，未映射，不宣称无损覆盖缓存计量。

`submitted` 表示已调用 transport execute，不等于证明服务端接收；网络异常 execution 为 Unknown，HTTP 200 后 Accepted。无发送的 preflight 失败计 0 attempt，真实 transport submission 计 1。拒绝响应不保留原始错误 body。elapsed 用单调时钟；当前单次直接派发 call／attempt 耗时共享起点。

Payload Debug 脱敏正文及工具参数；观测无正文／headers／原始错误。显式内容访问与捕获仍由调用方授权，这不是防恶意调用者的安全沙箱。

## 依赖与运行边界

固定 reqwest 0.13.5、eventsource-stream 0.2.3（见 Cargo.lock），没有厂商 SDK 或 Web 框架。reqwest 默认会重试部分协议错误，所以手动入口显式 `retry(never)`，并 `redirect(none)`、HTTP/1、60 秒总期限、15 秒连接期限。SSE 前限制总字节 256 KiB；文本与单工具参数各 64 KiB，最多 16 个工具索引。请求 JSON 最大 4096 bytes、输出最大 1024 tokens；手动两例实际为 64／256。HTTP 请求和模型语义失败均不自动重派。

[官方 MiniMax 参数](https://platform.minimax.cn/docs/api-reference/text-openai-api)、[reqwest client](https://docs.rs/reqwest/0.13.5/reqwest/struct.ClientBuilder.html)、[SSE parser](https://docs.rs/eventsource-stream/0.2.3/eventsource_stream/) 于 2026-10-03 复核。代理占位按 [Cloud 官方契约](https://learn.chatgpt.com/docs/environments/cloud-environments#configure-environment-variables-and-network-secrets) 消费，不将变量名 literal 当凭据。

## 验收范围

静态检查与人工 live／replay 分开记录。fixture 是白名单协议记录，不是完整抓包：去除响应 ID、时间戳、fingerprint、headers、cookies、raw errors 及未知字段值；保留合成请求、model、文本／工具分片、finish、usage 与 `[DONE]` 顺序。捕获回调只缓存在内存；写 fixture 前按解码字符串、完整文本、按 index 拼接的工具参数及其内嵌 JSON 再检查凭据／占位回显。嵌入字符串解码失败或引号未闭合时 fail closed，不补引号猜测，也不跳过检查。拒绝时只保存固定元数据、typed usage／attempt 数及 capture_rejected 状态，不写正文、不格式化异常，保留预留。这个保守规则可能拒绝合法流中的不完整字符串片段；不影响模型输出契约，也不把 capture 拒绝改写成 provider 成功。此保护不声称能识别代理端未知真实值或任意编码的秘密。

手动 replay 在分支入口不构造 HTTP client、不读凭据，只把已保存的 projected records 送入同一个 Decoder，再经 service 包装；source 标为 replay，网络 attempt=0。expected 是独立人工审阅文件，不能由 mapper 自动更新。诊断样本保留旧 source mapping 的失败结果，新 expected 明确记录修正后的映射；replay 采用记录的 clean EOF／typed error 分类，保留实际 Decoder error kind 与 HTTP 接收状态。精确内容只是这次真实样本的回放 oracle，不是未来随机生成的质量断言。

未验收：并发热更新／凭据轮换、取消和 drop／panic 后终态、真实中断／429／重试故障注入、工具参数多片段／多工具并发、工具结果往返、厂商新增字段兼容、三 Harness／三 provider、全局预算协调／账单核对、正式配置中心和 Mac。静态通过或成功样本不替代这些运行证据。
