# 来源：共享核心、网关与早期集成

[返回来源索引](../sources.md)。本页保留原始决定与当时的授权边界；它不是当前实现状态或本次任务的授权清单。现行规则从[文档入口](../index.md)按主题查阅。

S17 原记录连续涵盖 2026-10-05 至 2026-10-11 的多项修订，仍保留同一来源编号；按下列阶段定位：

- [共享包与 AI 网关](#共享包与-ai-网关)
- [运行时与会话集成](#运行时与会话集成)
- [浏览体验与运行时扩展](#浏览体验与运行时扩展)

## S17

### 共享包与 AI 网关

2026-10-05 用户提出：共享能力不应全部集中在巨大 core，可考虑 ai、agent-runtime、persist 等独立顶级 package，各平台通过 ABI 按需消费；Android／iOS 例如只需要 remote。用户明确这些名字、边界和逐包 ABI 均为待探讨设想，不是已批准的具体划分。产品意图归 PRD，候选方案归架构，源码观察与下一步归 package-boundaries Task Packet。用户计划次日验收现有 Mac 版本，本次讨论不授权提前重构或改变该安装产物。

S17 后续修订：用户基本认可候选职责边界，同意不建立通用存储框架，但质疑配置仓库独立 package 的必要性，并明确每个 package、app 都是一个 unit。职责边界的认可不等于所有职责必须各建一个包，本轮仍是讨论而非源码重构。

S17 跨语言接入澄清：用户认可配置职责安排，进一步说明 ABI 不是固定技术选择，真实目标是 Swift／Kotlin／C# 等使用 Rust 开发的库。底层调用约定、语言绑定和产物打包应分别讨论；本轮不要求替换现有 Mac 接入。候选工具的官方资料与未验证项留在讨论任务。

S17 实施授权：用户决定全面切换 UniFFI，并要求现在开始拆分。此授权覆盖 package 重构、生成绑定接入及 Mac 重建安装，取代前述仅讨论的阶段限制。

2026-10-05 用户纠正提供商导入入口：选择一个已配置的 Agent 运行时导入，而非重新填写运行时目录。此反馈与导入 sheet 布局修复共同归首循环 Task Packet；来源选择意图归 PRD。

2026-10-05 用户反馈 Pi 导入只有泛化错误，并要求使用 Rust／Swift 各自成熟日志生态做好可观测性，OTLP 暂不接入但保留扩展能力。用户随后确认修正目录后读取成功，要求独立 sub-agent 重新设计导入界面的功能、内容层级、布局、组件和交互。日志契约归架构／开发说明，易变验证与 UI 方案归首循环任务。

2026-10-05 用户在导入验收失败后明确：当前网关无需协议转换／翻译，仅做原生透传、路由与 fail-over。此原则同时适用于 ChatCompletions 和 Responses，产品意图归 PRD，当前实现偏差与技术边界归 AI service 设计，失败证据归首循环任务。

2026-10-05 用户要求从需求重新复核整个 AI 网关并审计实现，必要覆盖整个 AI 服务，明确名称不能限定为 Harness 网关。此授权为需求、设计与实现审计，不直接等同全部重构实施。已确认职责归 PRD 和 AI 服务设计，源码证据及方案取舍归 AI 网关审计任务。
2026-10-05，用户允许 `SamplingOutput` 作为业务表示，但要求可观测性内容与业务数据分离；指定处理方向为 HTTP → OpenAI ChatCompletions → messages、outputs／stats → SamplingOutput。后续澄清 usage、finish reason 可以同时是业务数据和观察对象。设计释义为原生协议先行，业务投影按需消费，观察独立关联并可消费业务统计。权威归属为 [AI 服务设计](../design/ai-service.md)。

2026-10-05，用户明确 AI 服务不限于 LLM，当前 gateway 仅为 LLM Gateway，gateway 是 AI 模块的一种应用模式。职责关系不推出 package 合并，也不要求所有 AI 调用经过网关。权威产品归属为 [PRD](../prd/index.md)，技术边界归 [AI 服务设计](../design/ai-service.md)。

2026-10-05，用户指出导入后“Pi Agent 认证来源”被列为认证方式，违反 AI 服务与 Harness 解耦；进一步明确仅保留外部凭据引用仍危险，要求统一集中管理。认证资源、目标授权与来源适配归 application，提供商选择已登记资源，Harness 来源不成为 AI 认证类型。既有平台秘密与原订阅认证保留约束继续适用。权威归属为 [PRD](../prd/index.md)及 [AI 服务设计](../design/ai-service.md)。


2026-10-05，用户进一步纠正 model-id：这是提供商规定的模型标识，不是内部记录键；要求核对 Vercel AI SDK、Mastra、Cloudflare AI Gateway 与 OpenAI Developer Docs 的基本概念和业务模型。产品定义归 PRD，官方资料及当前代码对照归 AI 网关审计任务；这次研究不自行授权新的模型迁移实现。


2026-10-05 用户认可模型业务复核，明确授权立即实施重构，并反馈导入详情 Disclosure 正文额外左缩进；随后明确本阶段所有重构 hard-cutoff。实施不保留旧 Velune 契约或迁移，旧普通配置直接重置，原 Harness 文件、会话与平台秘密不删除；需求归 PRD，设计归 AI 服务设计，执行证据归 AI 网关任务。


2026-10-05 用户后续纠正：ID 与 reasoning effort 藏在提供商页暴露参数归属问题；要求先厘清模型／提供商配置，认为独立模型页与统一凭据管理无必要，提供商协议／地址必须可修改，API key 允许查看和编辑。取代此前强制 Keychain 与独立认证页的产品决定；当前安装源码尚未据此重构。


2026-10-05 用户进一步解释：“模型可跨提供商存在”是现实，对 Velune 的意义是避免反复填写模型参数、快速填入模板。已确认的是模板复用价值；配置快照与模板更新不自动传播是助手建议，尚未成为用户决定。归属为 [PRD](../prd/ai-providers.md)。


2026-10-05 用户复核认可提供商／模型能力／调用参数的归属模型并授权开工；特别要求随提供商表单复杂化，重新整理功能内容需求并重新设计原生界面。实施采用模板快填形成独立配置快照、不自动传播更新作为最小技术选择，不新增模板同步能力。


2026-10-05 用户补充提供商 UI 设计方向：窗口更大不一定最优，优先隐藏次要内容、表达层级和渐进披露。作为实施中的持续反馈，已交给 UI owner 合并到现有设计。

### 运行时与会话集成

2026-10-06 用户授权继续支持 Codex app-server 与 deepseek-harness，要求直接使用 [huihua](https://github.com/wibus-wee/huihua) 的包而非复制源码，遵守声明和许可证；要求运行时类型以 regex 纳入版本兼容，避免 adapter 与运行时不匹配而未诊断。用户次日统一验收；开发方继续隔离验证和安装，不读取真实会话或凭据。

2026-10-06 用户进一步明确版本分型：不同 breaking-change 版本视为不同运行时，允许多个 Pi adapter 版本并存，不只是不匹配报错；并建议参考 obelisk。后者暂以最吻合的 tommy0103/obelisk 核对，不由参考建议推定需要会话索引数据库。

2026-10-06 用户建议重点学习 Magpie。已定位 [yetone/magpie](https://github.com/yetone/magpie)，只进行架构与实现参考复核；这不构成引入 Go／Wails、协议翻译、账号切换或自动 fail-over 的产品决定。固定源码与比较证据归 研究 packet（git 历史：tasks/magpie-reference）。

2026-10-06 用户明确“开始应用对 magpie 的吸纳”，授权将已核对机制落到当前产品。当前切片采用版本化协议能力、导入选择／替换边界和无正文的网关生命周期观测；不接入 Magpie 包或复制源码，不引入协议翻译、重试／fail-over 或 Wails。设计归架构和 AI 服务设计，实施证据归 Magpie packet（git 历史：tasks/magpie-reference）。

2026-10-06 用户验收后质疑运行时“初始模型”和“连接运行时”，并要求修复更多菜单图标、从公开目录取得模型模板。生命周期方向仍在复核，不把提问自动提升为移除现有契约的决定；模板与图标改进已实施授权。公开源核对为 [models.dev 官方 README](https://github.com/anomalyco/models.dev#api)，provider-scoped API 与模板用途归 AI 服务设计，当前方案和验收归 体验任务（git 历史：tasks/runtime-session-experience）。

2026-10-06 用户明确确认“会话选模型、自动准备运行时、独立浏览历史”，指出无初始模型导致连接失败是不正确的前置关系，并要求已添加运行时即可使用。此决定修订原运行时默认模型与手动连接流程；PRD 已更新。启用／禁用是可追加能力，当前必要切片为去除模型与准备对浏览的阻塞。实施归 体验任务（git 历史：tasks/runtime-session-experience）。

2026-10-06 用户反馈标题被 JSONL 文件名替代、时间未格式化、消息未到底部，以及气泡／对齐／Markdown／长列表性能问题；同时确认内部应有权威会话模型，UI 条目可拆分。产品展示意图归 PRD，具体模型和实现证据归 展示任务（git 历史：tasks/conversation-presentation）。

2026-10-06 用户要求删除、重命名真实运行时会话，明确不在 Velune 内覆盖；反馈切换时目标选中→原会话选中→目标选中的回跳，要求考虑加载时延。产品意图归 PRD，执行见 会话管理任务（git 历史：tasks/session-management）。

2026-10-06 用户要求会话列表 context menu 及相关操作在会话加载中保持可用；这修订此前加载时整侧栏禁用的交互选择，原生持久数据与迟到结果隔离要求继续适用。实施归 会话管理任务（git 历史：tasks/session-management）。

2026-10-06 用户追加统一会话浏览需求：单列表包含所有启用 Agent Runtime，支持 Runtime、CWD project、label／section 分组筛选及创建／更新时间排序，不按执行运行时分别显示列表。懒加载／分页允许，标签来源正澄清。归 统一浏览任务（git 历史：tasks/conversation-browser）。

同轮用户澄清 label／section 只是例子，非所有运行时均支持，Codex section 聚合 project；未来自定义 tag／label／section 应由 Velune 独立持久化组织记录。本轮先实现 runtime／project 分组筛选与时间排序，不引入标签 CRUD。

2026-10-06 用户反馈诊断18dbddc3184363d8-a9ce-3的历史详情失败，并要求提供商／运行时双击编辑、模板原生多选与独立选取sheet、运行时快速检测导入、Anthropic Messages原生支持、每组默认20可配置及会话多选。执行与诊断证据归 本轮体验与协议任务（git 历史：tasks/settings-protocol-refinement），正式产品意图归PRD。

2026-10-06 会话浏览后续：用户指出顶部运行时／模型应决定下个 turn 的执行目标，并将其关联到跨 Harness 会话；打开原生历史应由会话来源决定。来源与执行意图区分已进入 PRD；观察到的读取故障相关性仍需诊断证据，不能提升为已确认根因。

2026-10-06 用户接受“目标原生会话＋上下文交接＋只持久化关联元数据”。这授权 application 保存原生会话引用与切换位置，以恢复同一逻辑会话，消息仍归各 Harness；不重放原生工具或审批状态。范围归 PRD，实施与验证归 跨 Harness 接续任务（git 历史：tasks/cross-harness-continuation）。

2026-10-06 用户报告新的 read/ambiguous_session 诊断，授权继续修复。官方 Codex [0.159.3 recorder](https://github.com/openai/codex/blob/rust-v0.159.3/codex-rs/rollout/src/recorder.rs) 与 [revert 实现](https://github.com/openai/codex/blob/rust-v0.159.3/codex-rs/thread-store/src/local/revert_thread.rs) 明确保留 thread ID、创建新 rollout 并保留旧文件；多个文件使用同一 thread ID 是合法行为。[版本化原生协议](https://github.com/openai/codex/blob/rust-v0.159.3/codex-rs/app-server-protocol/src/protocol/v2/thread.rs) 提供逻辑历史分页；不能用物理文件扫描唯一性代替原生历史。检查日期为本日，适用该 variant；真实隔离复现与安装证据归 历史身份修复任务（git 历史：tasks/codex-history-identity），不宣称读取过用户真实会话。

2026-10-06 用户将双击进入编辑明确为项目级 UI/UX 范式，要求所有可编辑列表遵循；并禁止应用捆绑任何 Agent runtime，要求排查系统 Pi 的版本发现。产品意图归 PRD，外部安装解析、版本证据和隔离验收归 本轮 packet（git 历史：tasks/external-runtime-and-editing）。

2026-10-06 用户授权为跨Harness对话实施ChatCompletions、Responses、Anthropic Messages的全部双向转换，建议参考Magpie／LiteLLM；随后明确全部协议转换采用best-effort而不是fail-closed。用户认可Anthropic必需输出上限缺省使用所选模型配置。当前权威要求归PRD与AI服务设计，证据和实现归 转换任务（git 历史：tasks/llm-protocol-translation）。

2026-10-06 用户指出会话列表“历史读取”及 composer／设置底部错误不合理，授权移除并提供类似 VSCode Problems 的集中问题面板，入口由开发方设计。权威产品行为归 PRD，实施和隔离验收归 问题展示任务（git 历史：tasks/problems-surface）。

2026-10-06 用户反馈问题窗口原因不完整，要求核对是否存在安全／隐私过滤，并确认纯本地、用户完全控制的应用不应以此删减诊断。用户同时授权 turn 工作过程折叠、两种用户消息 outline、底部图标入口与设置偏好，并建议 canonical 契约及顺序事件投影归 agent-runtime。包归属与精确折叠边界的建议已提出，当前按用户问题／最终结果可见及轻量契约保留的建议实施，尚不把建议记成用户确认。实施与验收归 本轮任务（git 历史：tasks/transcript-turns-outline）。

2026-10-07 用户授权增加“分析”，参考 [Magpie](https://github.com/yetone/magpie)、[ccusage](https://ccusage.com/guide/all-reports)、[sub2api dashboard API](https://github.com/Wei-Shaw/sub2api/blob/main/frontend/src/api/admin/dashboard.ts)，理解 token 消耗与性能。参考分别体现网关调用、运行时历史及服务端聚合数据，不能假定它们具有相同统计覆盖或直接混算。产品意图归 [PRD](../prd/analytics.md)，数据口径归 AI 服务设计，当前范围与证据归 分析任务（git 历史：tasks/usage-analytics）。


2026-10-08 用户纠正网关的排他假设：Velune 注册网关提供商使不同 Harness 共享模型、聚合订阅与提供商，从未要求只能使用该网关；授权自查并删除无需求依据的门禁。现行产品归 [PRD](../prd/ai-providers.md)，门禁判别和实施证据归 当前清单（git 历史：tasks/composer-pi-send）。

2026-10-08 用户进一步要求边界、校验、门禁尽可能由类型检查与 lint 等静态检查表达。内部关系优先通过类型和编译器维持，外部动态输入仍在接入边界解析；实施约定归 [开发说明](../development.md)，当前切片归 任务（git 历史：tasks/composer-pi-send）。

### 浏览体验与运行时扩展

2026-10-09 用户要求链接可打开／原文复制、大纲准确定位、新会话目录默认可选、各Tab独立展示会话。产品行为归PRD，共享核心与工作区边界归架构说明，证据归 会话体验任务（git 历史：tasks/mac-conversation-usability）。

2026-10-09 用户要求替换MarkdownUI，采用[LiYanan2004/MarkdownView](https://github.com/LiYanan2004/MarkdownView)。用户以MarkdownUI维护趋于停滞为替换理由。已按3.0.0的[Package.swift](https://github.com/LiYanan2004/MarkdownView/blob/3.0.0/Package.swift)和[LICENSE](https://github.com/LiYanan2004/MarkdownView/blob/3.0.0/LICENSE)核对macOS13+、Swift6.2及MIT；API接入按锁定源码核对；实施及验证归会话体验任务（git 历史：tasks/mac-conversation-usability）。

2026-10-09 用户反馈Markdown点击重排与长列表上滑异常，要求核对我们的组件接入和虚拟化层；明确若是MarkdownView自身缺陷不必修上游，建议考虑[gonzalezreal/textual](https://github.com/gonzalezreal/textual)。随后明确允许Velune最低支持macOS15。平台底线归PRD；依赖与容器的隔离判别、适用边界归会话体验任务（git 历史：tasks/mac-conversation-usability），候选不等同于已采用。

2026-10-09 用户反馈快速上滑仍跳动甚至卡住，授权重写整个消息列表渲染实现（含必要的 agent runtime UI 事件投影）、自由本地提交，并允许更换 Markdown 渲染框架（举例 Textual）；有余力时继续优化列表与交互体验。实现选择与判别归会话体验任务（git 历史：tasks/mac-conversation-usability）。

2026-10-10 用户要求推进 Rust packages：Claude Code、运行时既有提供商模型选择、每 turn 临时 Velune Gateway 与结束恢复、SSH 远程运行时及 Velune 管理 HOME。用户明确排除本轮 app／前端实施，要求把相关交互标记待实现并沉淀文档。产品行为归 [PRD](../prd/agent-runtimes.md)，实现与证据归 运行时扩展任务（git 历史：tasks/runtime-provider-ssh）。

2026-10-10 用户授权继续实现 ACP Adapter，随后更正参考项目为 [lexoliu/acpsub](https://github.com/lexoliu/acpsub)。本轮沿用 Rust packages 范围，App 仍待实现；产品归 PRD，协议与参考证据归 ACP 任务（git 历史：tasks/acp-adapter）。

同轮用户补充：技术提议未必正确，应理解背后动机；本次目标是尽可能支持更多 Agent Harness，而不逐个适配。通用 ACP 方案必须以接入成本和真实能力覆盖判断，不以协议名称本身作为完成依据。

2026-10-11 用户进一步指出 acpsub 的重点是让不同 runtime 成为另一 runtime 的 sub-agent，并明确这是 Harness 互联互通的关键功能。此要求将工作从通用 ACP 接入扩展到实际跨 Harness 委派与结果闭环，仍沿用 Rust packages 和前端待实现边界。产品要求归 [PRD](../prd/collaboration.md)，实施归 Harness 协作任务（git 历史：tasks/harness-interoperation）。

2026-10-11 用户提出复用 New API 的 RelayKit，以参与生态并减少重复维护协议转换；明确授权持续推进至接入完成或获得无法接入的具体证据，允许自主提交，并可考虑相邻 Factory26 的真实提供商或重放入口。技术归属及接入边界归 [AI service](../design/ai-service.md)，版本、许可调查与验证归 RelayKit 任务（git 历史：tasks/new-api-reuse）。此授权不等于公开发布或替整个项目选择许可证。

- 2026-10-11 用户在 RelayKit 安装后明确要求进行真实测试，允许使用相邻 Factory26 的 MiniMax、Ark Coding Plan、Qwen Coding Plan 提供商，并可固定请求用于重放。此授权覆盖合成输入的有限真实调用及脱敏采集，不涉及既有真实会话或公开凭据。

- 同轮核对发现 Qwen 实际配置为第三方 Token Plan（qianwenaiapi.com），用户明确选择“使用现有 Qwen Token Plan，按真实名称记录”；不将其称为官方 Qwen Coding Plan。
