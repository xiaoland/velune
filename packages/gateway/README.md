# AI 网关

`velune-gateway` 是本机 LLM 网关 unit，也是 AI 能力的一种应用模式。它接收 OpenAI ChatCompletions v1、Responses v1 或 Anthropic Messages 请求，通过显式模型路由派发给提供商。同协议路径使用原生 operation，保留消息历史、生成参数与响应事件；异协议路径调用 ai-provider 的请求计划转换 JSON 与增量 SSE，采用 best-effort 降级和静态诊断，不经过 sampling。模型路由与认证始终根据上游目标，运行时入口协议不限制提供商协议。HTTP 的 hop-by-hop 字段、本地认证和 cookie 不转发。

调用方通过 `GatewayConfig` 提供具有模型条目的提供商、模型路由及策略，通过 `Runner::start` 提供入口模型别名。提供商配置在启动时形成不可变快照；直接调用方修改配置后须重建 Runner；application 会使执行失效并在下次发送时自动准备，用户不需要手动连接。运行时模型选择可显式替换入口 alias 到稳定模型记录的绑定，不修改在途派发已捕获的目标。此 unit 不理解 Harness 的模型目录、思考级别转换、配置实例或认证文件。模型元数据不成为原生请求参数的默认值。仅异协议转换到 Messages、原请求无输出上限时，使用所选模型的配置上限；两者都缺失时返回必要配置错误。旧 `chatCompletionsOutputLimitField` 已从新契约删除，不提供兼容入口。

提供商认证配置只有不透明 `credential_ref`。应用通过 `Runner::start(config, resolver, aliases)` 注入真正的异步 `CredentialResolver`；每次解析将资源引用与已捕获的协议、执行后端、request profile 和 endpoint 一同传入，只返回瞬时 `ResolvedCredential`。解析器的私有认证配置、目标校验、来源 adapter、helper 与进程生命周期均归调用方。网关不接收来源 JSON 或认证文件，不读取它们；CLI 可执行路径来自结构化后端配置，隔离目录由解析后的本机会话材料提供。网关不解释引用，不读取全局配置，不持久化秘密。取消派发会丢弃正在等待的解析 future；调用方必须提供符合取消合同的实现。

Axum 负责 HTTP framing，当前只监听随机 loopback 端口并提供三条 POST 创建端点。请求 body 不设 gateway 自身的大小上限，读取期限为 15 秒；入口不设置固定并发拒绝门禁。流式输出采用容量为 1 的异步通道保持背压，丢弃响应 body 会取消派发。HTTP client 禁用自动重试与重定向，连接期限为 15 秒，不设置固定读取空闲期限。`Runner` 退出停止入口并取消在途任务，不等待长流完成。取消与资源限制由调用方和上游 HTTP 客户端共同决定。

依赖方向为 `velune-gateway → velune-ai-provider → velune-ai`。它不依赖会话投影、Agent 运行时、应用层或语言绑定。调用方决定是否链接此 unit；仅连接远端的客户端不需要本机 HTTP 网关。检索、后台 Responses 生命周期、跨成员之外的自动路由与无中断热配置不属于当前实现。

提供商模型条目使用稳定 `recordKey` 和精确 `providerModelId`。入口自动派生为 `velune/model/<recordKey>`，运行时注入的别名另外传入；内部记录键本身不是 wire model 入口。网关完成目标解析后才传递 `ProviderModelId`，协议执行不再次映射。上下文／输出能力与协议推理声明均可未知，同协议网关不要求或补充请求 token 限制，也不以默认规格覆盖提供商实际能力。

模型路由（`ModelRoute`）以自己的 `recordKey` 与提供商模型同列，入口同为 `velune/model/<recordKey>`，别名也可指向它。成员是提供商模型记录键，须同协议；校验拒绝空成员、重复成员与和模型冲突的键。`strategy` 有 `smart`、`order`、`rotate`、`weight` 四种，`sticky` 控制是否保持会话绑定。`weight` 必须为每个成员配置正整数权重，其它策略的 weights 必须为空。旧的、实际未参与派发的 `FailoverMode`／`FailoverPolicy`／GatewayConfig.failover 已删除，不保留兼容字段；平台配置和 bindings 消费方须采用新契约。

资格检查先于策略和会话黏性，也适用于直接选择的模型。提供商的 `quota_limits` 指定额度窗口与允许使用百分比上限；观测达到上限即暂停。它是基于新鲜观察的准入阈值，不预估单次消费或预留预算；并发请求和采集间隔可能使供应商实际使用越过阈值。中立只读 `QuotaSource` 返回 `Known`、`Unknown`、`Stale` 或 `Unsupported`，已知／过期状态可保留窗口和显式可用性。窗口包含适用的提供商模型 ID、实际使用百分比、重置时间、观察时间和有效期。gateway 不采集网络额度、不持久化、不修改来源；source 的 snapshot 必须是非阻塞读取。窗口只约束其声明适用的模型。已过有效期或重置时间的观察不能据此推断额度恢复；有上限却缺少新鲜适用窗口时暂停，无上限的未知额度成员作为 smart 后备。显式 `available=false` 也暂停，不能伪造为 100% 使用。

`smart` 在健康／额度资格过滤后，优先已绑定且仍合格的会话成员，再按最紧窗口的归一化余量 `min((cap-used)/cap)` 降序排列；未配置 cap 的新鲜窗口按 100% 计算，相同余量保留成员顺序，未知额度排在已知成员之后。`order` 使用配置顺序；`rotate` 轮询；`weight` 使用平滑加权轮询。轮询和权重在新会话或关闭黏性时分配，已有合格会话不会推进分配计数。成员或权重变化会重置轮询／权重状态，不迁移旧余额。

调用方用 `SessionIdentity { namespace, id }` 显式注入会话身份，路由记录键与 gateway ID 由当前目标补足。没有会话 ID 时不从 prompt、HTTP headers 或 Harness 私有内容猜测。只有显式业务 Completed 且传输成功后才绑定，替补成功后更新绑定；失败、incomplete、取消和断流不建立绑定。此判定不依赖是否注入 analytics sink。绑定只在运行期的共享 `SchedulingState` 内保存。直接模型仍受健康与额度检查，但不建立路由黏性。模型路由请求携带非空 `previous_response_id` 或 `conversation` 时在转换和派发前拒绝，要求调用方提供完整历史；直接选择提供商模型仍保留协议原有状态引用契约。

fail-over 只在尚未转发任何响应时发生。429 与结构化 402 额度错误（error.code/type 为 `insufficient_quota`、`usage_limit_reached` 或 `quota_exceeded`）按 Retry-After 冷却，缺省 60 秒、至多 24 小时；5xx、连接／传输失败与超时采用 5 秒健康冷却；提交后发生传输故障只冷却后续请求，不切换当前流。HTTP 错误没有可靠窗口作用域，因此该冷却按提供商实例生效，区别于额度快照按模型过滤窗口。冷却期间所有路由和直接调用都不穿透；没有合格成员时返回 429 与最早 Retry-After，纯粹缺少必要额度事实时返回 503。重置到期只允许重新读取新鲜观察，不视为满额。冷却期满仅放行一个探测，RAII permit 在取消、失败和正常退出时释放，真正 Completed 后恢复正常排序；探测遭遇相同临时故障会重新冷却。

403、请求类 4xx、取消与已开始转发后的中断结束请求。成功 HTTP headers 一经交付即提交；200 SSE 内后续出现的额度错误或业务失败也不切换。没有恢复能力的 HTTP 认证收到 401 时结束；声明 `CredentialRecovery::Refreshable` 的 HTTP 认证在提交前调用 resolver.refresh 一次，只有相同恢复身份和 account_id 的凭据可重发。永久认证失效或重发仍为 401 时调用 invalidate，允许下一个成员；临时续期失败可尝试下一成员，但不标记会话永久失效。provider 每次调用只执行一次尝试，401 恢复由 gateway 显式组织。所有候选尝试失败时保留最后一个上游状态和正文，有健康／额度等待事实时附加当前候选最早 Retry-After。该时间表示重新检查的机会，不保证来源已刷新或额度已恢复。每个成员尝试各有一条 analytics，401 恢复保留该成员最终 exchange；tracing 记录首次 401 与切换，日志没有正文和认证值。

`Runner::start_with_scheduling` 注入共享 `SchedulingState`、只读 source、可选会话身份与共享 `ExecutionBackends`；现有 start／start_with_analytics 是无额度来源、无会话身份的便捷入口，使用同一调度实现。应用应跨 Runner 持有状态。健康状态按 gateway ID 与 provider ID 隔离；配置的认证身份变化、换号或退出登录时，调用 `invalidate_provider` 清旧冷却与绑定并递增 epoch，普通 token refresh 不调用。Runner 捕获启动时的 epoch，旧 Runner 和迟到结果不能准入或改写新身份状态；额度来源也必须由应用按同一 gateway／认证绑定隔离。

Codex／DSH 原生运行时的装配方可额外注册其执行模型字符串为 alias，并映射到已选的提供商模型记录；gateway 不据模型名推断提供商，不解析 app-server／ACP，也不读取 huihua 历史。执行模式由 application 与 adapter 负责注入网关入口和临时本地 token；是否同时保留其它运行时入口由调用方决定。审批、回答、取消 turn 与原生会话恢复不属于本 unit。

网关只发出元数据 tracing 事件，subscriber 由消费方装配。request／attempt 关联贯穿响应 body 与取消 future 的丢弃；目标使用当前快照中的序号，日志不包含模型 ID、alias、endpoint、请求正文或凭据。传输完成与模型业务成功分开，调用方断开与 Runner 关闭也有独立归因。具体字段与人工验收见 [本地诊断](../../docs/development.md#本地诊断)。

Messages 保留调用方的 `anthropic-version`／beta 请求头与原生 JSON／SSE，缺少版本在派发前拒绝；HTTP API key 使用 `x-api-key`，Bearer 使用 `Authorization`。提供商 baseURL 追加 `/v1/messages`，运行时 SDK 的网关注入使用 origin。

异协议转换复用 `eventsource-stream` 解析 SSE 分帧，支持任意网络分块。转换后的响应去除过期的长度、编码与完整性 headers；Messages 合成请求使用 API 版本 `2023-06-01`，同协议仍保留调用方的版本。输出沿用容量为 1 的通道与取消合同；具体资源预算由调用方和上游传输负责，不添加协议无关的正文总量门禁。转换无法保留的字段只记录静态 field/code，不记录正文；真实上游 HTTP 错误保留状态和正文，未出现有效业务终态的断流不会补造完成。人工端到端验证入口见 [开发说明](../../docs/development.md)。

跨协议文本增量立即交付。转换到 Messages 的工具参数须形成合法 JSON object，因此在单个工具完成时交付其有界累积参数；Responses custom 工具也在其参数完成后解除 `{input:string}` 包装。无法解析的参数保留为原始字符串并记录降级，不能补造可执行命令或整次拒绝。OpenAI 两协议间普通 function 参数继续增量交付。

调用方可通过 `Runner::start_with_analytics` 注入非阻塞 `AnalyticsSink`，在上游原生响应、协议转换之前接收实际提供商／模型与用量、时间、结果记录。接收器负责持久化；gateway 不读取存储路径，也不从 tracing 重建用量。已有 `Runner::start` 可不采集统计。

`ProviderDefinition.execution_backend` 显式选择 `Http` 或 `ClaudeCli { binary }`，`request_profile` 独立选择 `Native`、`CodexResponses`、`KimiCoding` 或 `ArkCoding`。HTTP 必须提供 endpoint；CLI 必须使用 Messages、Native profile 和空 endpoint，binary 是可执行文件路径而非 shell 命令。订阅 profile 只允许各自的固定 HTTPS 目标，协议 adapter 再次校验；ArkCoding 拒绝普通按量 API 地址。认证来源不决定这些选择。`ResolvedCredential` 的材料、恢复身份和请求能力彼此独立，resolver 的 refresh／invalidate 默认分别返回不可用／无操作，rejected_token 不写日志。认证的创建、保存、刷新串行化与并发换号保护属于 application；独立额度查询认证不进入网关的模型凭据。

`ExecutionBackends` 由应用跨 Runner 持有，与 `SchedulingState` 配合。它维护 Claude Code 共享 pool 与唯一 binding 编码；认证身份或后端配置变化时，应用同时调用两者的 `invalidate_provider(gateway_id, provider_id)`，普通 token refresh 不调用。应用退出才 shutdown 共享后端。CLI 的 stream／nonstream 以及三个入口协议使用既有 Messages 转换路径；没有会话 ID 时不复用 CLI 会话。

Claude CLI 的执行提交早于 HTTP 响应提交。成功启动进程后或向已存在进程写入本次输入前，后端标记 submitted；之后即使尚未返回 headers，也禁止透明切换。CLI 退出码不伪装成 HTTP 401，也不触发认证刷新；只有明确的限流、超时或传输事实更新相应健康冷却。工具执行只由调用方承担：CLI 后端转交 tool_use 并等待真实 tool_result。

有挂起工具调用的显式会话，必须使用后端登记的原 provider 与认证 epoch。这是执行归属，先于策略和软黏性；其成员被移除、换绑、会话失效或额度／健康不合格时，返回明确的不可接续错误，不切到另一实例。普通文本也不能绕过尚未完成的工具归属，不从请求正文或 tool_use_id 猜测会话。外层 Harness 回合结束时调用 `ExecutionBackends::finish_user_turn`，清理仍在执行或等待工具的交互，保留已完成的空闲会话供复用。应用在用户取消时调用 `ExecutionBackends::cancel_session`，在显式新用户轮入口调用 `begin_user_turn`；后者拒绝仍运行或挂起的交互，仅解除已终止交互的失效限制，不在每次 HTTP 请求或运行时准备时重置。后端保留已失效工具 ID，网关在选择任何候选前校验本次请求尾部的工具结果，拒绝迟到结果；完整历史中的旧结果不会阻止新轮。

上游 HTTP 2xx 仍可能在响应提交前因协议契约不满足而失败，例如流请求收到非 SSE 类型。此时网关返回 502 合成 JSON 错误，不复用成功响应的正文或实体 headers；analytics 保留实际上游状态。这个下游状态规范化不参与 fail-over 分类，不能把上游协议错误当作上游 5xx。已提交响应后的错误仍按原有中断处理。


网关消费中立失败事实而不读取供应商文案。Permission 即使伴随 401／500 也停止，不触发认证恢复或通用 5xx 切换；只有 Authentication 才进入已声明的恢复能力。QuotaExhausted 在提交前按限额冷却并允许下一成员，同时调用非阻塞 `QuotaSource::quota_exhausted(provider_id)`，由 application 按需重取，不能在网关推测重置或修改额度缓存。Overloaded 沿用限流冷却。原始状态与正文继续透传；已提交响应、取消和 CLI 执行提交边界保持不变。

应用轮换 Key 时可调用 `SchedulingState::invalidate_credentials` 使捕获旧凭据的 Runner 和迟到结果失效，保留已验证账户的冷却及黏性；该方法同时迁移当前绑定的 epoch，不让旧请求回写新世代。新 Key 重新验证前必须由来源层阻止借用旧 owner 准入。验证为不同 owner 时再调用完整 `invalidate_provider` 清除账户事实。
