# AI 网关与 AI 服务：需求复核及实现审计

审计日期为 2026-10-05。源码基线为 f7d4b0d，产品实现等同其前序 7345ba8；本轮只修改文档，不修改产品源码。AI 网关、应用装配和 AI／provider 分别由独立 owner 调查，advisor 负责目标职责判断，主执行者整合并纠正未经证实的推论。需求权威归 [PRD](../../docs/prd/index.md#ai-网关与-agent-运行时配置)，目标职责归 [AI 服务设计](../../docs/design/ai-service.md)，本文件只保存基线审计与迁移依据。

结论：需要重划执行职责，不能只补 DeepSeek 参数。当前 ChatCompletions 是有损的 sampling 协议桥；Responses 只有部分原生保真；应用装配仍以一个 Pi 首循环的生命周期组织 AI 网关。现有独立 packages 可以保留，不需要重新创建巨大 core 或万能 provider 框架。

## 需求与实际能力

| 已确认需求 | 当前观察 | 判断 |
| --- | --- | --- |
| 同协议原生请求、响应及流事件保真 | Chat 经过 SamplingInput／Delta；Responses 保 body／SSE，但 HTTP envelope 不完整 | Chat 是架构偏差，Responses 需补齐边界 |
| AI 网关负责路由与 fail-over | 启动时捕获唯一静态目标；FailoverMode 只有 Disabled | 静态路由已实现，自动路由／fail-over 未实现，不能宣称交付 |
| AI 服务不限 LLM，sampling 是独立操作 | AiService 名称只暴露 sampling；通用 observation 引用 model、token 与 SamplingErrorKind；Responses 没有相同调用／尝试观测 | 范围与命名需要收敛，不能把 sampling 类型扩成所有操作的基底 |
| AI 服务不依赖 Harness | ai、ai-provider、gateway 无 Pi crate 依赖；跨域转换在 application | 基础依赖方向正确；应用投影和通用 wire 配置仍有反向耦合 |
| 提供商、逻辑模型、提供商模型映射分别表达 | 已有三个配置层；全局模型 output limit 成为所有调用的闸门 | 实体分离可保留，能力和策略需区分 |
| 参数以提供商协议为权威 | 网关强制调用含 output limit；Pi selection 固定 off；订阅能力按名称猜测 | 有效请求被限制或参数被改写，不能视为原生透传 |
| 导入仅应用明确选择，保留原来源 | 来源保留、fingerprint 校验已有；未选 B 仍写入空模型 provider | 配置选择语义存在已复现缺陷 |
| 配置由 application 持久化，调用保持不可变快照 | 原子保存已有；导入要求重连却保留旧 Runner，普通编辑又全局断连 | 生效版本／状态合同缺失 |
| 取消、断连与退出保持明确且有界 | 异步 HTTP 阶段能检查断连；同步凭据 helper 无期限，Drop 等线程 | 取消不覆盖完整链路，退出可能无界 |
| 开发方验收合理功能边界，UI 与真实服务由用户验收 | 已有已安装库导入失败、SDK 多轮和网关诊断；本轮新增选择探针 | 证据可用，不把脚本成功当产品通过 |

## 实际数据路径

application 读取保存的运行时实例，选择 AI 网关和固定 provider-model binding，生成 Pi 私有模型投影与物理绑定 ID。Pi SDK 构造请求；AI 网关将逻辑模型或别名解析为启动时冻结的 RouteTarget。Chat 路径转为 SamplingInput，再经 DirectAiService 和 provider 重建上游 JSON；返回事件再次由网关重建为 SSE。Responses 路径传原生 body 与 SSE。两条路径都通过应用提供的外部 resolver 获取认证。

sampling 的实际非网关调用方是 [MiniMax 手动入口](../../packages/ai-provider/examples/minimax_manual.rs)。其有界采样用途合理，不能以保留网关调用为理由把原生协议削减成采样子集。原生 Responses 有独立 binding 本身也合理；问题是跨操作生命周期与观测合同不一致，不是必须把所有操作塞进一个大 AiService trait。

## 架构偏差与具体缺陷

### P1：ChatCompletions 经过有损转换

[gateway runtime](../../packages/gateway/src/runtime.rs) 行 295、941、799 分别装配 DirectAiService、解析 SamplingInput 和重建 SSE。[OpenAI adapter](../../packages/ai-provider/src/openai.rs) 行 88–163 重建 messages，固定 stream 与 include_usage，并从内部字段生成 output limit／reasoning_effort。[Sampling Message](../../packages/ai/src/sampling.rs) 行 39–50 只有文本、assistant tool calls 和 tool result；无法保留原生内容、推理历史与扩展。

[gateway runtime](../../packages/gateway/src/runtime.rs) 行 970 还将多条 system／developer 指令合入单一位置，后者覆盖前者。合法角色位置、多模态内容、未知扩展及其它选择字段也无法原样通过。既有 [导入验收](../pi-mac-first-loop/deepseek-import-acceptance.md) 已实际确认 thinking 与历史 reasoning_content 丢失，推理 SSE 触发 terminal error。

应替换 AI 网关中的 sampling 往返路径，增加独立原生 ChatCompletions 操作。sampling 保留自己的明确语义，不继续添加厂商字段来模拟透传。

### P1：通用 OpenAI 响应受有界 MiniMax decoder 限制

[openai.rs](../../packages/ai-provider/src/openai.rs) 行 10–11、171–183、322–324 依赖 minimax::Decoder 与 mapping；[MiniMax mapping](../../packages/ai-provider/src/minimax/mapping.rs) 只接受有界 choice、delta、finish 与 usage 形状。未支持的推理 delta 会失败，其它原生扩展无法进入 sampling 输出。

同一代码复用不是天然错误，但此处复用了限制语义的 decoder，影响另一个 adapter 的协议合同。保留 MiniMax sampling decoder；原生 Chat decoder／观察器由对应协议操作负责，未知合法扩展不因采样类型缺槽而丢弃或拒绝。

### P1：上游 HTTP 结果被网关提前替换

[gateway runtime](../../packages/gateway/src/runtime.rs) 行 517 在上游结果到达前发送下游 200。Chat 的错误 body、状态、响应头、事件身份和完整 usage 不通过原生 envelope 交付。[openai.rs](../../packages/ai-provider/src/openai.rs) 行 260–283 也不保留非成功错误 body，且只支持 SSE。

这不仅影响客户端错误解释，也破坏 fail-over 的提交边界。应由原生操作提供状态、可转发响应头、body／SSE 和执行阶段，由 AI 网关决定下游提交；不得在未知上游结果时先承诺成功。stream=false 也需作为明确的协议能力验收。

### P1：凭据 helper 和清理没有有界取消合同

[gateway runtime](../../packages/gateway/src/runtime.rs) 行 264 使用同步 Command::output，没有期限、取消或 stdout 上限；行 109 的 current-thread executor 无法在阻塞期间调度断连检查；行 152 的 Drop 等待执行线程。同步 helper 挂起即可使关闭长期等待。

保留显式委托认证设施，给认证、请求读取、连接、流和关闭阶段分别建立有界、可取消合同。不能以主 HTTP timeout 代替完整清理保证。本项由代码确定，未启动真实认证进程验证。

### P1：导入写入未选提供商

[provider_import.rs](../../packages/application/src/provider_import.rs) 行 360 遍历所有 preview provider，模型选择为空时仍在行 507 写入 provider。最小探针用已安装 7345ba8 clean bundle，仅选择 A 的一个模型：apply 返回 A、B 均导入，实际读取保存配置为 A models=1、未选 B models=0。三份合成来源文件逐字节不变，未连接或调用上游。修正探针清理为 shutdown 后执行退出 0，临时目录删除。

这是独立配置事务缺陷：只处理明确选择且具有有效选中模型的 provider。修复不依赖重建整个 AI 服务。

### P1：订阅能力由来源名称推断

[pi_composition.rs](../../packages/application/src/local/pi_composition.rs) 行 219 仅用 Pi、openai 与 Responses 判定 subscriptionCapability，没有核对 OAuth／订阅认证类型；[pi_virtual_model.mjs](../../packages/agent-runtime/resources/pi_virtual_model.mjs) 行 35–51 据此删除 output、temperature 和 cache 参数。

API-key 来源也可能被套用订阅限制。这是实际请求改写条件，不只是 UI 文案。应由认证来源 adapter 返回明确能力，不能按名称猜测。真正的提供商限制仍需保留，不能为透传而绕过认证合同。

### P2：Responses 的 HTTP envelope 不完整

[gateway runtime](../../packages/gateway/src/runtime.rs) 行 615 只承接 status 与 content_type，行 662 固定非流成功为 200，行 685 固定错误 Content-Type。限流 Retry-After、请求追踪等响应头不在操作合同里；[ResponsesEvent](../../packages/ai/src/responses.rs) 行 22–28 同样没有完整头集合。

原生 body／SSE 路径可保留，补齐 HTTP envelope 和明确头转发规则。认证头、Host 与 hop-by-hop 字段不能作为普通扩展直接透传；原生保真不等于任意 HTTP proxy。

### P2：模型能力、策略和协议参数混用

[gateway config](../../packages/gateway/src/config.rs) 行 9 的 max_output_tokens 必填，runtime 行 952 强制 Chat 请求含上限并按全局模型值拒绝，Responses 行 708 也使用模型能力校验 body。这里没有 clamp，而是拒绝；仍会拒绝未含可选上限的有效调用。应分别表达逻辑身份、提供商执行能力及明确的调用策略，不能从示例参数推出所有提供商必填同一字段。

[application config](../../packages/application/src/config.rs) 行 553 又从 pi_projection 派生通用 max_tokens wire 选项。非 Pi 来源无法通过对应普通字段独立表达它。迁移为原生请求后重新核对这个网关选项是否仍必要；直接 sampling 若仍需要选择字段，归该操作的 provider 配置，Pi metadata 只保存 Pi 投影。

### P2：配置生效、网关与运行时生命周期混在一起

[configuration.rs](../../packages/application/src/local/configuration.rs) 行 88 的导入保存配置并提示 requiresReconnect，却保留旧 Runner；行 151 的普通网关编辑会停止任何活动运行时，即使修改无关网关。[connection.rs](../../packages/application/src/local/connection.rs) 行 71 在候选验证前停止旧连接，行 72 只在运行时连接时启动 AI 网关。

明确持久配置、运行快照和每次调用版本，以及新请求采用新版的时间。分开 AI 网关资源与运行时连接所有权，候选准备成功后再切换。当前单活运行时是已标注的范围限制，不直接判为缺陷；但不应成为 AI 网关 unit 的永久能力前提。

### P2：执行模型与资源限制未形成服务合同

[gateway runtime](../../packages/gateway/src/runtime.rs) 行 116 在完整处理一条流后才 accept 下一连接，行 185 固定连接／整体期限，行 884 的手写 HTTP parser 只处理 Content-Length 和固定 body 上限。

有界资源限制是必要的，常数本身不等于错误；缺少的是并发／队列、总期限／空闲期限、取消及请求读取能力的明确合同。不要为扩功能继续发展自制 HTTP 协议栈；实现时评估成熟 HTTP server，依实际需求决定依赖，不在此次审计直接引入框架。

### P2：AI 服务宽泛命名掩盖操作范围

[ai/lib.rs](../../packages/ai/src/lib.rs) 行 17 的 AiService 只有 sampling；[observation.rs](../../packages/ai/src/observation.rs) 行 28–54 的 Operation、Usage、AttemptContext 和错误引用仅适用于采样与语言模型。Responses 的独立契约缺少相应 call／attempt 观测，不利于网关一致记录尝试与执行状态。

以操作命名接口和 binding，共享必要的生命周期识别规则；操作定义自己的 usage／目标和错误语义，不把 model、token 或 SamplingErrorKind 变成 AI 服务所有能力的基础。无需万能操作注册器或新的巨大 AiService trait。

## 范围缺口与尚待验证

静态 model→provider 路由、禁止隐式 retry／redirect 是当前真实能力；自动路由与 fail-over 尚未实现。Pi 物理身份 hash 包含提供商、端点、外部模型、projection 和认证身份，有防止历史签名误复用的价值。但 alias 当前复制唯一启动目标，未来切换不能只加候选表，需要处理请求的能力与状态约束。相同协议不证明 previous_response_id、文件、缓存、加密推理或账号绑定可迁移。

Responses 目前只覆盖 POST 创建请求，background 被拒绝；其它状态型端点不属已验收范围。provider 发现终态后返回是代码事实，但未证实终态后还有合法事件必须保留，不直接判为已复现截断缺陷。应在实现切片中明确终态与传输结束的规则、帧边界和未知合法事件；协议规定 JSON 的字段收到 malformed JSON 不自动成为透传缺陷。

Messages 仅为预留配置形状。application 的协议描述只暴露 Chat／Responses，dispatch 校验拒绝 Messages，因此不构成当前 UI 伪支持。删除或保留预留形状依实际消费者决定，不能以审计名义擅自声称已实现或机械删除。

Pi selection 当前固定 off，virtual route 优先使用它；推理能力与本次选择混在投影里。产品已经要求参数遵守提供商协议，但本次推理选项由用户、会话或策略怎样选择仍需明确，不能以 capability 的存在推定默认高推理或永久关闭。

## 建议的保留、替换和演进

保留现有 unit 依赖方向、application 配置仓库、UniFFI 类型接口、原子保存、导入 fingerprint／来源执行校验、原目录和运行投影分离、Payload 默认脱敏、本地入口认证、禁用隐式 retry／redirect，以及 MiniMax 有界采样的独立用途。

替换 Chat 网关的 SamplingInput／Delta 路径及由 MiniMax decoder 定义原生 OpenAI 语义的耦合。不要继续用 Pi metadata 给网关填补可选 wire 字段；保留 Pi 投影所属的运行时规则，并由明确认证能力驱动订阅限制。

建议按以下可独立验收的切片实施；此处是重构方案，不是本轮已完成能力：

1. 先确定原生 Chat／Responses 请求、HTTP envelope、事件与尝试阶段合同，明确 sampling 的操作范围和命名；冻结非路由内容保真标准。
2. 建立单次协议执行与 AI 网关原生入口，覆盖 JSON／SSE、未知合法扩展、历史、多个指令、usage、非成功状态／body／必要头和中断；退出旧采样桥。
3. 修复导入选择与认证能力来源，保留有效 compat、重新验收已配置运行时导入到真实 loopback 多轮请求；不将预览放行作为完成。
4. 明确配置版本生效、网关与运行时资源所有权，补齐认证和传输的有界取消／关闭，再人工验收失败切换与取消各阶段。
5. 在上述单次执行与观察可信后，实现明确配置候选的 fail-over；先决定跨模型／账号／提供商范围及不确定提交是否允许重复执行。未决定前保持 Disabled，不自动补入策略。

每个切片优先类型和静态检查，再用人工临时脚本在真实产品入口作合成端到端验收；不新增自动化测试、真实模型调用或 UI E2E。协议参考：[OpenAI ChatCompletions](https://developers.openai.com/api/reference/resources/chat/subresources/completions/methods/create)、[Responses events](https://developers.openai.com/api/reference/resources/responses/streaming-events)、[DeepSeek thinking mode](https://api-docs.deepseek.com/guides/thinking_mode/)，于 2026-10-05 复核；这些资料支持协议与参数含义，不作为未经执行的本地验收证据。
