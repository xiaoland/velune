# 来源索引

这是关键决定的精简定位表，不是聊天记录导出。消息 ID 用于回到原对话核对；没有为它们编造可公开访问的 URL。未提供精确时间的消息不补猜时间。

## S1

- 消息：`Sentinel_14f3d0e763e08191a2024621ccb3e8c5`
- 时间：2026-10-01 08:47 UTC
- 决定：停止进一步 Factory26 开发，回到用户自己的 coding agent 产品

## S2

- 消息：`Sentinel_9f2c607f16748191a9a563bee396dacf`
- 时间：2026-10-01 08:49 UTC
- 决定：首要核心是多订阅、多模型供应商、自动路由、多 harness，以及跨 harness 会话感知、交流和协作；远程控制在此基础上通过分布式架构实现
- 纠正：此前远程／手机优先的讨论不再作为优先级依据

## S3

- 消息：`Sentinel_ce566f9e822881918fdfbebeedfcc75f`
- 决定：认可持久化任务—会话模型和任务树视图

## S4

- 消息：`Sentinel_06d6bb74003081919fbdccc180dfe423`
- 决定：HAPI／Lody 属说明性参考，不要求照搬其功能或实现

## S5

- 消息：`Sentinel_c03879c98b7881919e313d14c34d8c0a`
- 决定：Claude Code 订阅认证处理方式可以作为参考研究例外；尚未确定本产品认证架构

## S6

- 消息：`Sentinel_ce7bb3296a948191b830282c7b25b744`
- 决定：排除 GitHub Copilot

## S7

- 消息：`Sentinel_d84f6f77f16881919cce80d5d5ec86e4`
- 时间：2026-10-01 08:58 UTC
- 请求：先初始化 Git 仓库，包括 AGENTS.md、xiaoland/svc 文档知识系统和 Task Packet；完成后再继续讨论

## S8

- 消息：`Sentinel_fd6950f44cc48191b07f66cb0747c17f`
- 日期：2026-10-01
- 澄清：需要持久化仓库，不需要打包交付；SVC 只采用文档导航和 Task Packet，不安装或保留其 CLI 及额外工程
- 授权：可以使用用户明确给出的 Git 身份作本地提交；此授权不自动包含远端创建、push 或部署

## S9

- 消息：`Sentinel_55f5e7324b108191bc208034542f9f2b`
- 时间：2026-10-01 10:03 UTC
- 决定：首批 Harness 固定 Codex、Claude Code（语音中的 Cloud Code）、Pi；以全部可接管 LLM 路由为设计目标，调查如何实现，不以支持度重新开启选择
- 参考：跨 Harness 会话通信可研究 lexoliu/acphub，也可采用其他方案
- 委托：由设计方收敛方案，用户复核；本轮产出为仓库内方案与研究文档，不是产品实现

## S10

- 消息：`Sentinel_2ff262054c108191b25740ff1e05f8ba`
- 时间：2026-10-01 11:28 UTC
- 复核：认可上一轮四项产品设计；明确同意 MCP 暴露协作工具、ACP 作为可替换接入
- 纠正：保存任务树、消息、权限和恢复状态本身不能说明如何完成跨 Harness 协作，需要区分协调机制与支撑设施
- 技术方向：认可 SQLite，采用 Rust 核心；Apple 原生 Swift／UIKit、Android 原生 Kotlin／Jetpack Compose，替换 Web 前端；接受实验项目带来的开发不便
- 边界：未逐项批准 FFI 工具、Apple 平台覆盖／Mac Catalyst 选择、服务打包或所有详细实现契约；未授权产品实现、登录或费用测试

## 其他上下文的证据等级

Codex／Claude Code／Pi 的目标范围、LLM 路由与设备调度的区分、用户有两个 ChatGPT 账户，以及完整稳定可恢复、避免锁定等背景，来自初始化时传入的已整理上下文；当前没有为每一句提供独立消息 ID。本仓库保留其含义，不伪造逐句引文。若后来出现冲突，以最新用户明确决定为准并补充来源。

## SVC 官方来源

- [官方仓库](https://github.com/xiaoland/svc)
- 检查时间：2026-10-01 UTC
- 源提交：[4fe4c66ac4deb35209069c00b1bbdc1b22aae3af](https://github.com/xiaoland/svc/commit/4fe4c66ac4deb35209069c00b1bbdc1b22aae3af)
- [Task Packet 语义](https://github.com/xiaoland/svc/blob/4fe4c66ac4deb35209069c00b1bbdc1b22aae3af/corpus/task-packet/index.md)
- [知识归属规范](https://github.com/xiaoland/svc/blob/4fe4c66ac4deb35209069c00b1bbdc1b22aae3af/corpus/specs/index.md)
- [PRD 规范](https://github.com/xiaoland/svc/blob/4fe4c66ac4deb35209069c00b1bbdc1b22aae3af/corpus/specs/prd/index.md)

仅采用上述文档方法，不把 SVC CLI 纳入本仓库。PRD 放在 `docs/prd/index.md`，遵从该提交的 PRD 规范入口；不另建旧模板路径的副本。

## S11

- 来源：父会话 `01a0f01b-c0bb-719d-af64-671b0563103f` 的本次 Codex Cloud 委托及后续转达；不伪造原始消息 ID
- 时间：2026-10-02；用户 06:27 同意先做无凭据最小原型
- 授权：Rust＋SQLite 核心、本地确定性模拟、代码／文档／测试、官方开发工具链及正常 registry 依赖、本地 Git 提交
- 仍禁止：读取秘密、登录、真实模型／付费请求、未授权 push／PR／部署、Factory26、SVC CLI
- 后续要求：Codex Cloud 持续开发 → Mac mini 原生安装体验 → 用户反馈 → 修复复验；Mac mini 首个目标，移动端后续 LAN 连接，不改 VPN。后续父会话报告 Mac 构建条件可用并获准 Swift＋AppKit 薄壳；06:43 UTC 用户明确要求先完成核心验证再接 UI，批准设备构建使用；不自动批准远端 push

## S13

- 来源：父会话 `01a0fb5e-ceba-778c-9e3d-6cbf22b340d8` 于 2026-10-03 转达的逐项讨论结果和有界实现委托；不伪造原始消息 ID
- 已确认：AI service 是独立 lib，领域不限 LLM；sampling 是其操作；service 拥有调用方／provider 两套权威契约；provider 即 adapter，自行转换外部协议；service 不依赖具体 SDK
- 配置：provider 为协议配置＋凭据引用＋模型列表，不增加账户／渠道实体；统一配置中心持久化，app main 装配；调用开始后配置稳定，新配置只影响后续调用
- 范围：只模块／类型／构造校验／必要接口与文档；不做路由、fallback、重试、网络、真实凭据、Host/UI 改造或测试，只允许静态基础检查
- 隔离：从已发布 `3bce5f9` 新 worktree／分支开始，保留暂停的 IPC／测试改动；允许新有界开发分支普通 push，不改 main、不强推、不创建 PR
- 验收补充：用户指出仅实现不足以闭环，需要验收方案；随后提出真实 AI 接入并固定 fixture。provider／model、凭据位置和预算尚未答复，未授权猜测或提前访问秘密／调用。固定 fixture 不是无限新增测试框架授权


## S14

2026-10-03，父会话 `01a0fb5e-ceba-778c-9e3d-6cbf22b340d8` 的有界委托：从 `7c6f98267a56101d2117311adeaf10b50b01d8f2` 实现 service 直接流式派发、MiniMax 普通 API adapter、合成 live fixture 与离线重复验收；预算总 ¥5，本步最多新增 8 attempts，先文本／工具各一次，工具只返回不执行。批准普通 push 新 dev 分支，禁止 tests／PR／main／force／部署／Host UI 改造／其他 provider。Networksecret 配置类型由用户配置页证据与平台占位契约确认，不输出／保存值。长期技术归属 [AI service](design/ai-service.md)，实际用量、失败和剩余验收项归属 [任务](../tasks/ai-service-contracts/packet.md)。

## S15

2026-10-04，本会话用户要求将 `dev/minimax-stream-fixtures` 合并主分支，并推进 Pi agent＋Mac app 首循环；资源示例为 ChatGPT 订阅、Tokenflux、ARK Coding Plan。随后明确：资源由用户在应用中配置，不能硬编码；需要 Chatbot 界面与会话列表；会话持久化不在本任务，Velune 只是 projection；真实验收由用户做；模块至少拆为 core（AI 服务、Harness 适配器）与 app（Mac 等平台）。用户在体验后要求完全重写 Mac app，提供 Velune SVG logo，并要求避免与任何提供商或 Agent Harness 耦合。用户进一步指出该界面缺乏 Apple 原生体验，确认项目级原则：尽可能贴近所属平台原生视觉风格，品牌／软件特点在微小细节体现。随后要求 AI providers、Harness 等应用配置通过文件系统持久化，位于 `VELUNE_HOME`（默认 `~/.velune`）。用户进一步明确 Apple app 不必独立验收深色模式：使用原生组件与原生系统视觉，常规明暗适配交由系统。产品边界归属 [PRD](prd/index.md#pi-与-mac-首循环)，执行证据归属 [任务](../tasks/pi-mac-first-loop/packet.md)。没有为当前消息伪造 ID。

S15 后续纠正：模型独立且跨提供商共享；提供商可关联多个模型，模型有参数与展示元数据；协议首个为 OpenAI ChatCompletions v1 枚举选项。提供商只是 Velune 网关配置一部分，另有路由与 fail-over，Harness 仅使用注入的网关，不利用已有上游认证。“连接”页改为 Agent 运行时，类型与配置实例分离，同为 Pi 的不同配置也形成不同实例。

S15 品牌补充：默认使用 graphite logo，不使用 theme 配色作为默认标志。

S15 产品与协作补充：Velune 是 control surface 而非 Agent；消息按用户右、LLM 左、系统／Harness 中间布局，不显示头像／昵称，不标记 LLM 为 Velune。用户明确持续迭代协作，开发方主动维护任务、长期文档和代码质量，并允许自主提交当前任务改动。此授权未扩为 push／部署。

S15 运行与交付补充：core 应为跨平台一致的 lib，由各平台 app 通过 ABI 嵌入，不能作为独立 Host 进程。Mac 安装到 Applications，修改后重新安装；当前产品显示版本为 0.1 beta.1。旧 Host 版本不匹配是接入实现的偏差，不应要求用户长期手动管理该进程。

S15 协作方式澄清：采用敏捷开发，需求在实现阶段可以变化；早期重视长期可迭代的技术架构与开发基础，不要求仓促产出。关键设计善用 advisor，发现需要用户取舍的问题时，带着证据与建议请求决定。

S15 架构参考补充：用户提出 Mastra、HAPI、Lody 等实现可用于借鉴思路与架构，不以复制功能为目标。具体技术结论需按相关设计问题检查源码版本与前提，不因列入参考就认定兼容或采用。

S15 后续配置与交付纠正：工作目录属于具体会话，运行时目录是配置／状态根目录；用户要求从运行时表单移除必填工作目录。用户允许开发方随时退出或强制退出运行中的 Velune 完成更新，并要求删除全部既有自动化测试，采用静态检查和人工／临时端到端验收。

## S16：统一网关与 Harness 配置来源

2026-10-04 用户提出保留 Harness 原提供商配置，读取、接管并共享其配置与认证来源；希望复用 Pi 订阅登录，Velune 管理会话默认使用 `velune/auto` 或按 Harness 区分的稳定入口。该意图不等同于复制令牌、复用任何客户端注册身份或已授权真实登录。

官方复核：[ChatGPT 开源客户端注册与登录](https://developers.openai.com/siwc/token-sharing-open-source/sign-in)要求明确应用身份、主机标识与授权 scope；[模型与推理](https://developers.openai.com/siwc/token-sharing-open-source/models-and-inference)描述使用 public Responses 端点。2026-10-04 检查；不能将旧 Codex backend 认证与该路径视为相同。Pi 固定版本的具体能力与验证缺口留在当前 Task Packet。

用户在本轮进一步澄清：`max_output_tokens` 等具体字段用于举例能力参数，不是必须支持的需求；以提供商规定的协议为权威。原生 Responses、稳定入口和认证委托的实施已获授权，真实账户验收仍由用户进行。

S16 导入语义纠正：用户明确将 Harness 提供商配置导入 Velune 视为一项完整功能，认证来源接入属于其中，不能作为完整导入已交付的证据。

2026-10-04 用户进一步提醒两个独立边界：Agent Harness 不与 AI 服务耦合，AI 服务不与大语言模型领域耦合；导入配置属于适配／装配行为，LLM 相关字段不能成为整个 AI 服务的基础假设。需求归属 PRD“AI service 的有界实施方向”。

2026-10-04 用户补充原生视觉原则：原生控件无需专门实现深色模式，但 Core Graphics、固定色 PNG、logo、自定义阴影、渐变和边框仍需要处理暗色背景下的显示。此补充已归位 PRD 的平台体验原则。

## S17

2026-10-05 用户提出：共享能力不应全部集中在巨大 core，可考虑 ai、agent-runtime、persist 等独立顶级 package，各平台通过 ABI 按需消费；Android／iOS 例如只需要 remote。用户明确这些名字、边界和逐包 ABI 均为待探讨设想，不是已批准的具体划分。产品意图归 PRD，候选方案归架构，源码观察与下一步归 package-boundaries Task Packet。用户计划次日验收现有 Mac 版本，本次讨论不授权提前重构或改变该安装产物。

S17 后续修订：用户基本认可候选职责边界，同意不建立通用存储框架，但质疑配置仓库独立 package 的必要性，并明确每个 package、app 都是一个 unit。职责边界的认可不等于所有职责必须各建一个包，本轮仍是讨论而非源码重构。

S17 跨语言接入澄清：用户认可配置职责安排，进一步说明 ABI 不是固定技术选择，真实目标是 Swift／Kotlin／C# 等使用 Rust 开发的库。底层调用约定、语言绑定和产物打包应分别讨论；本轮不要求替换现有 Mac 接入。候选工具的官方资料与未验证项留在讨论任务。

S17 实施授权：用户决定全面切换 UniFFI，并要求现在开始拆分。此授权覆盖 package 重构、生成绑定接入及 Mac 重建安装，取代前述仅讨论的阶段限制。

2026-10-05 用户纠正提供商导入入口：选择一个已配置的 Agent 运行时导入，而非重新填写运行时目录。此反馈与导入 sheet 布局修复共同归首循环 Task Packet；来源选择意图归 PRD。

2026-10-05 用户反馈 Pi 导入只有泛化错误，并要求使用 Rust／Swift 各自成熟日志生态做好可观测性，OTLP 暂不接入但保留扩展能力。用户随后确认修正目录后读取成功，要求独立 sub-agent 重新设计导入界面的功能、内容层级、布局、组件和交互。日志契约归架构／开发说明，易变验证与 UI 方案归首循环任务。

2026-10-05 用户在导入验收失败后明确：当前网关无需协议转换／翻译，仅做原生透传、路由与 fail-over。此原则同时适用于 ChatCompletions 和 Responses，产品意图归 PRD，当前实现偏差与技术边界归 AI service 设计，失败证据归首循环任务。

2026-10-05 用户要求从需求重新复核整个 AI 网关并审计实现，必要覆盖整个 AI 服务，明确名称不能限定为 Harness 网关。此授权为需求、设计与实现审计，不直接等同全部重构实施。已确认职责归 PRD 和 AI 服务设计，源码证据及方案取舍归 AI 网关审计任务。
2026-10-05，用户允许 `SamplingOutput` 作为业务表示，但要求可观测性内容与业务数据分离；指定处理方向为 HTTP → OpenAI ChatCompletions → messages、outputs／stats → SamplingOutput。后续澄清 usage、finish reason 可以同时是业务数据和观察对象。设计释义为原生协议先行，业务投影按需消费，观察独立关联并可消费业务统计。权威归属为 [AI 服务设计](design/ai-service.md)。

2026-10-05，用户明确 AI 服务不限于 LLM，当前 gateway 仅为 LLM Gateway，gateway 是 AI 模块的一种应用模式。职责关系不推出 package 合并，也不要求所有 AI 调用经过网关。权威产品归属为 [PRD](prd/index.md)，技术边界归 [AI 服务设计](design/ai-service.md)。

2026-10-05，用户指出导入后“Pi Agent 认证来源”被列为认证方式，违反 AI 服务与 Harness 解耦；进一步明确仅保留外部凭据引用仍危险，要求统一集中管理。认证资源、目标授权与来源适配归 application，提供商选择已登记资源，Harness 来源不成为 AI 认证类型。既有平台秘密与原订阅认证保留约束继续适用。权威归属为 [PRD](prd/index.md)及 [AI 服务设计](design/ai-service.md)。


2026-10-05，用户进一步纠正 model-id：这是提供商规定的模型标识，不是内部记录键；要求核对 Vercel AI SDK、Mastra、Cloudflare AI Gateway 与 OpenAI Developer Docs 的基本概念和业务模型。产品定义归 PRD，官方资料及当前代码对照归 AI 网关审计任务；这次研究不自行授权新的模型迁移实现。


2026-10-05 用户认可模型业务复核，明确授权立即实施重构，并反馈导入详情 Disclosure 正文额外左缩进；随后明确本阶段所有重构 hard-cutoff。实施不保留旧 Velune 契约或迁移，旧普通配置直接重置，原 Harness 文件、会话与平台秘密不删除；需求归 PRD，设计归 AI 服务设计，执行证据归 AI 网关任务。


2026-10-05 用户后续纠正：ID 与 reasoning effort 藏在提供商页暴露参数归属问题；要求先厘清模型／提供商配置，认为独立模型页与统一凭据管理无必要，提供商协议／地址必须可修改，API key 允许查看和编辑。取代此前强制 Keychain 与独立认证页的产品决定；当前安装源码尚未据此重构。


2026-10-05 用户进一步解释：“模型可跨提供商存在”是现实，对 Velune 的意义是避免反复填写模型参数、快速填入模板。已确认的是模板复用价值；配置快照与模板更新不自动传播是助手建议，尚未成为用户决定。归属为 [PRD](prd/index.md#ai-网关与-agent-运行时配置)。


2026-10-05 用户复核认可提供商／模型能力／调用参数的归属模型并授权开工；特别要求随提供商表单复杂化，重新整理功能内容需求并重新设计原生界面。实施采用模板快填形成独立配置快照、不自动传播更新作为最小技术选择，不新增模板同步能力。


2026-10-05 用户补充提供商 UI 设计方向：窗口更大不一定最优，优先隐藏次要内容、表达层级和渐进披露。作为实施中的持续反馈，已交给 UI owner 合并到现有设计。

2026-10-06 用户授权继续支持 Codex app-server 与 deepseek-harness，要求直接使用 [huihua](https://github.com/wibus-wee/huihua) 的包而非复制源码，遵守声明和许可证；要求运行时类型以 regex 纳入版本兼容，避免 adapter 与运行时不匹配而未诊断。用户次日统一验收；开发方继续隔离验证和安装，不读取真实会话或凭据。

2026-10-06 用户进一步明确版本分型：不同 breaking-change 版本视为不同运行时，允许多个 Pi adapter 版本并存，不只是不匹配报错；并建议参考 obelisk。后者暂以最吻合的 tommy0103/obelisk 核对，不由参考建议推定需要会话索引数据库。

2026-10-06 用户建议重点学习 Magpie。已定位 [yetone/magpie](https://github.com/yetone/magpie)，只进行架构与实现参考复核；这不构成引入 Go／Wails、协议翻译、账号切换或自动 fail-over 的产品决定。固定源码与比较证据归 [研究 packet](../tasks/magpie-reference/packet.md)。

2026-10-06 用户明确“开始应用对 magpie 的吸纳”，授权将已核对机制落到当前产品。当前切片采用版本化协议能力、导入选择／替换边界和无正文的网关生命周期观测；不接入 Magpie 包或复制源码，不引入协议翻译、重试／fail-over 或 Wails。设计归架构和 AI 服务设计，实施证据归 [Magpie packet](../tasks/magpie-reference/packet.md)。

2026-10-06 用户验收后质疑运行时“初始模型”和“连接运行时”，并要求修复更多菜单图标、从公开目录取得模型模板。生命周期方向仍在复核，不把提问自动提升为移除现有契约的决定；模板与图标改进已实施授权。公开源核对为 [models.dev 官方 README](https://github.com/anomalyco/models.dev#api)，provider-scoped API 与模板用途归 AI 服务设计，当前方案和验收归 [体验任务](../tasks/runtime-session-experience/packet.md)。

2026-10-06 用户明确确认“会话选模型、自动准备运行时、独立浏览历史”，指出无初始模型导致连接失败是不正确的前置关系，并要求已添加运行时即可使用。此决定修订原运行时默认模型与手动连接流程；PRD 已更新。启用／禁用是可追加能力，当前必要切片为去除模型与准备对浏览的阻塞。实施归 [体验任务](../tasks/runtime-session-experience/packet.md)。

2026-10-06 用户反馈标题被 JSONL 文件名替代、时间未格式化、消息未到底部，以及气泡／对齐／Markdown／长列表性能问题；同时确认内部应有权威会话模型，UI 条目可拆分。产品展示意图归 PRD，具体模型和实现证据归 [展示任务](../tasks/conversation-presentation/packet.md)。

2026-10-06 用户要求删除、重命名真实运行时会话，明确不在 Velune 内覆盖；反馈切换时目标选中→原会话选中→目标选中的回跳，要求考虑加载时延。产品意图归 PRD，执行见 [会话管理任务](../tasks/session-management/packet.md)。

2026-10-06 用户要求会话列表 context menu 及相关操作在会话加载中保持可用；这修订此前加载时整侧栏禁用的交互选择，原生持久数据与迟到结果隔离要求继续适用。实施归 [会话管理任务](../tasks/session-management/packet.md)。

2026-10-06 用户追加统一会话浏览需求：单列表包含所有启用 Agent Runtime，支持 Runtime、CWD project、label／section 分组筛选及创建／更新时间排序，不按执行运行时分别显示列表。懒加载／分页允许，标签来源正澄清。归 [统一浏览任务](../tasks/conversation-browser/packet.md)。

同轮用户澄清 label／section 只是例子，非所有运行时均支持，Codex section 聚合 project；未来自定义 tag／label／section 应由 Velune 独立持久化组织记录。本轮先实现 runtime／project 分组筛选与时间排序，不引入标签 CRUD。

2026-10-06 用户反馈诊断18dbddc3184363d8-a9ce-3的历史详情失败，并要求提供商／运行时双击编辑、模板原生多选与独立选取sheet、运行时快速检测导入、Anthropic Messages原生支持、每组默认20可配置及会话多选。执行与诊断证据归 [本轮体验与协议任务](../tasks/settings-protocol-refinement/packet.md)，正式产品意图归PRD。

2026-10-06 会话浏览后续：用户指出顶部运行时／模型应决定下个 turn 的执行目标，并将其关联到跨 Harness 会话；打开原生历史应由会话来源决定。来源与执行意图区分已进入 PRD；观察到的读取故障相关性仍需诊断证据，不能提升为已确认根因。

2026-10-06 用户接受“目标原生会话＋上下文交接＋只持久化关联元数据”。这授权 application 保存原生会话引用与切换位置，以恢复同一逻辑会话，消息仍归各 Harness；不重放原生工具或审批状态。范围归 PRD，实施与验证归 [跨 Harness 接续任务](../tasks/cross-harness-continuation/packet.md)。

2026-10-06 用户报告新的 read/ambiguous_session 诊断，授权继续修复。官方 Codex [0.159.3 recorder](https://github.com/openai/codex/blob/rust-v0.159.3/codex-rs/rollout/src/recorder.rs) 与 [revert 实现](https://github.com/openai/codex/blob/rust-v0.159.3/codex-rs/thread-store/src/local/revert_thread.rs) 明确保留 thread ID、创建新 rollout 并保留旧文件；多个文件使用同一 thread ID 是合法行为。[版本化原生协议](https://github.com/openai/codex/blob/rust-v0.159.3/codex-rs/app-server-protocol/src/protocol/v2/thread.rs) 提供逻辑历史分页；不能用物理文件扫描唯一性代替原生历史。检查日期为本日，适用该 variant；真实隔离复现与安装证据归 [历史身份修复任务](../tasks/codex-history-identity/packet.md)，不宣称读取过用户真实会话。

2026-10-06 用户将双击进入编辑明确为项目级 UI/UX 范式，要求所有可编辑列表遵循；并禁止应用捆绑任何 Agent runtime，要求排查系统 Pi 的版本发现。产品意图归 PRD，外部安装解析、版本证据和隔离验收归 [本轮 packet](../tasks/external-runtime-and-editing/packet.md)。

2026-10-06 用户授权为跨Harness对话实施ChatCompletions、Responses、Anthropic Messages的全部双向转换，建议参考Magpie／LiteLLM；随后明确全部协议转换采用best-effort而不是fail-closed。用户认可Anthropic必需输出上限缺省使用所选模型配置。当前权威要求归PRD与AI服务设计，证据和实现归 [转换任务](../tasks/llm-protocol-translation/packet.md)。

2026-10-06 用户指出会话列表“历史读取”及 composer／设置底部错误不合理，授权移除并提供类似 VSCode Problems 的集中问题面板，入口由开发方设计。权威产品行为归 PRD，实施和隔离验收归 [问题展示任务](../tasks/problems-surface/packet.md)。

2026-10-06 用户反馈问题窗口原因不完整，要求核对是否存在安全／隐私过滤，并确认纯本地、用户完全控制的应用不应以此删减诊断。用户同时授权 turn 工作过程折叠、两种用户消息 outline、底部图标入口与设置偏好，并建议 canonical 契约及顺序事件投影归 agent-runtime。包归属与精确折叠边界的建议已提出，当前按用户问题／最终结果可见及轻量契约保留的建议实施，尚不把建议记成用户确认。实施与验收归 [本轮任务](../tasks/transcript-turns-outline/packet.md)。

2026-10-07 用户授权增加“分析”，参考 [Magpie](https://github.com/yetone/magpie)、[ccusage](https://ccusage.com/guide/all-reports)、[sub2api dashboard API](https://github.com/Wei-Shaw/sub2api/blob/main/frontend/src/api/admin/dashboard.ts)，理解 token 消耗与性能。参考分别体现网关调用、运行时历史及服务端聚合数据，不能假定它们具有相同统计覆盖或直接混算。产品意图归 [PRD](prd/index.md#分析)，数据口径归 AI 服务设计，当前范围与证据归 [分析任务](../tasks/usage-analytics/packet.md)。


2026-10-08 用户纠正网关的排他假设：Velune 注册网关提供商使不同 Harness 共享模型、聚合订阅与提供商，从未要求只能使用该网关；授权自查并删除无需求依据的门禁。现行产品归 [PRD](prd/index.md#ai-网关与-agent-运行时配置)，门禁判别和实施证据归 [当前清单](../tasks/composer-pi-send/gate-audit.md)。

2026-10-08 用户进一步要求边界、校验、门禁尽可能由类型检查与 lint 等静态检查表达。内部关系优先通过类型和编译器维持，外部动态输入仍在接入边界解析；实施约定归 [开发说明](development.md)，当前切片归 [任务](../tasks/composer-pi-send/packet.md)。

2026-10-09 用户要求链接可打开／原文复制、大纲准确定位、新会话目录默认可选、各Tab独立展示会话。产品行为归PRD，共享核心与工作区边界归架构说明，证据归 [会话体验任务](../tasks/mac-conversation-usability/packet.md)。

2026-10-09 用户要求替换MarkdownUI，采用[LiYanan2004/MarkdownView](https://github.com/LiYanan2004/MarkdownView)。用户以MarkdownUI维护趋于停滞为替换理由。已按3.0.0的[Package.swift](https://github.com/LiYanan2004/MarkdownView/blob/3.0.0/Package.swift)和[LICENSE](https://github.com/LiYanan2004/MarkdownView/blob/3.0.0/LICENSE)核对macOS13+、Swift6.2及MIT；API接入按锁定源码核对；实施及验证归[会话体验任务](../tasks/mac-conversation-usability/packet.md)。

2026-10-09 用户反馈Markdown点击重排与长列表上滑异常，要求核对我们的组件接入和虚拟化层；明确若是MarkdownView自身缺陷不必修上游，建议考虑[gonzalezreal/textual](https://github.com/gonzalezreal/textual)。随后明确允许Velune最低支持macOS15。平台底线归PRD；依赖与容器的隔离判别、适用边界归[会话体验任务](../tasks/mac-conversation-usability/packet.md)，候选不等同于已采用。

2026-10-09 用户反馈快速上滑仍跳动甚至卡住，授权重写整个消息列表渲染实现（含必要的 agent runtime UI 事件投影）、自由本地提交，并允许更换 Markdown 渲染框架（举例 Textual）；有余力时继续优化列表与交互体验。实现选择与判别归[会话体验任务](../tasks/mac-conversation-usability/packet.md)。

2026-10-10 用户要求推进 Rust packages：Claude Code、运行时既有提供商模型选择、每 turn 临时 Velune Gateway 与结束恢复、SSH 远程运行时及 Velune 管理 HOME。用户明确排除本轮 app／前端实施，要求把相关交互标记待实现并沉淀文档。产品行为归 [PRD](prd/index.md#ai-网关与-agent-运行时配置)，实现与证据归 [运行时扩展任务](../tasks/runtime-provider-ssh/packet.md)。

2026-10-10 用户授权继续实现 ACP Adapter，随后更正参考项目为 [lexoliu/acpsub](https://github.com/lexoliu/acpsub)。本轮沿用 Rust packages 范围，App 仍待实现；产品归 PRD，协议与参考证据归 [ACP 任务](../tasks/acp-adapter/packet.md)。

同轮用户补充：技术提议未必正确，应理解背后动机；本次目标是尽可能支持更多 Agent Harness，而不逐个适配。通用 ACP 方案必须以接入成本和真实能力覆盖判断，不以协议名称本身作为完成依据。

2026-10-11 用户进一步指出 acpsub 的重点是让不同 runtime 成为另一 runtime 的 sub-agent，并明确这是 Harness 互联互通的关键功能。此要求将工作从通用 ACP 接入扩展到实际跨 Harness 委派与结果闭环，仍沿用 Rust packages 和前端待实现边界。产品要求归 [PRD](prd/index.md#ai-网关与-agent-运行时配置)，实施归 [Harness 协作任务](../tasks/harness-interoperation/packet.md)。

2026-10-11 用户提出复用 New API 的 RelayKit，以参与生态并减少重复维护协议转换；明确授权持续推进至接入完成或获得无法接入的具体证据，允许自主提交，并可考虑相邻 Factory26 的真实提供商或重放入口。技术归属及接入边界归 [AI service](design/ai-service.md)，版本、许可调查与验证归 [RelayKit 任务](../tasks/new-api-reuse/packet.md)。此授权不等于公开发布或替整个项目选择许可证。

- 2026-10-11 用户在 RelayKit 安装后明确要求进行真实测试，允许使用相邻 Factory26 的 MiniMax、Ark Coding Plan、Qwen Coding Plan 提供商，并可固定请求用于重放。此授权覆盖合成输入的有限真实调用及脱敏采集，不涉及既有真实会话或公开凭据。

- 同轮核对发现 Qwen 实际配置为第三方 Token Plan（qianwenaiapi.com），用户明确选择“使用现有 Qwen Token Plan，按真实名称记录”；不将其称为官方 Qwen Coding Plan。

## S18

2026-10-12 用户检查开发中的协作界面后指出：子 Agent（sub-agent）与跨 Harness 协作（A2A）不同，二者本质是否一样不是 Velune 这个 shell 要考虑的；2026-10-11 的 PRD 表述由另一会话编撰，可能有误。用户确认：A2A 参与方是普通会话（出现在会话列表、可单独打开，Agent 可联系已有会话）；联系授权若需要则归运行时设置，但倾向不做任何限制；原生 sub-agent 属于会话体验，暂时只需在消息列表中看到并可单独打开其会话。用户另提示 composer 之后需要简化（已有三个弹出按钮）。产品归 [PRD](prd/index.md#ai-网关与-agent-运行时配置)，实施归 [Agent 间协作任务](../tasks/agent-collaboration/packet.md)。

## S19

2026-10-13 用户在原生 sub-agent 切片之后要求继续解决已报告的缺口，并确认收到的协作消息放在用户侧，标出来源会话即可。同条消息指出 composer 还缺少权限模式选择器。产品归 [PRD](prd/index.md#ai-网关与-agent-运行时配置)，实施归 [Agent 间协作任务](../tasks/agent-collaboration/packet.md)。

## S20

2026-10-13 用户要求继续改进 composer 的运行时、模型、推理、权限选择器（并指出它们在深色模式下没有跟随系统外观），发送后立即在消息列表中显示用户消息与“发送中”之类的状态，以及可以查阅各 turn（含当前 turn）使用的配置。用户选择“两个下拉”的组织方式，并选择在工作过程行右侧显示轮次配置，同时要求再收拢：悬停时展开，或保留一个 plain 图标按钮。同日用户验收新样式后指出下拉的三角离标题太近，且“运行时 · 模型 · 推理强度”下拉的选项太长、太复杂，建议考虑二级菜单等方案。产品归 [PRD](prd/index.md#ai-网关与-agent-运行时配置)，实施归 [composer 与轮次配置任务](../tasks/composer-turn-settings/packet.md)。

同日实测 Claude Code 2.1.286 恢复会话时沿用上次的权限模式（CLI 没有“回到默认”的取值），Codex 的轮次覆盖在 Velune 的托管路径下不会保留。用户据此选择“沿用会话当前模式”，不读取 Harness 配置来回到默认。

## S21

2026-10-13 用户验收 composer 子菜单后提出下一批改进：composer 的选择进一步按运行时保存，切换运行时而未发送时回到当前／上一轮使用的运行时；推理强度不做本地化，按实际发给运行时的值显示；询问推理强度的“默认”是否真的发送 `default`；轮次配置悬停时只显示运行时与模型，详情增加该轮 usage（特别要包含 reasoning tokens）；此前认为资源库不需要把原生改动回填到资源，实际使用后认为需要。助手说明“默认”不发送任何值、三种 Harness 结果不一致后，用户选择推理强度与权限模式一样“沿用会话当前”，并补充模型也应默认跟随当前／上一轮；对“未发送的选择离开会话后是否保留”，用户选择只有运行时回到最近一轮，各运行时的选择（含配置）按运行时保存。产品归 [PRD](prd/index.md#ai-网关与-agent-运行时配置) 与资源库段落，实施归 [composer 与轮次配置任务](../tasks/composer-turn-settings/packet.md)。

## S22

2026-10-14 用户验收 S21 各项后，要求在 composer（会话详情视图）中增加 status line，出发点是显示上下文用量，可按需扩展。三种位置方案（composer 下方状态行、操作行右侧紧凑显示、窗口底部状态栏）中用户偏好操作行右侧的紧凑方式，依据是避免信息过载、保持界面干净，同时不损害信息披露，并表示三种都不完全满意。其余候选（本会话累计 token、工作目录与 git 分支、Codex 账户额度、Harness 报告的费用）都可以有，但建议默认不放入 status line，由用户在会话设置中自定义；重点是位置与呈现方式。产品归 [PRD](prd/index.md#ai-网关与-agent-运行时配置)，实施归 [status line 任务](../tasks/composer-status-line/packet.md)。

## S23

2026-10-14 用户验收 status line 后，对资源库的子 Agent 提出：描述也应是多行文本编辑器；应可配置推理强度；Codex 的子 Agent 支持模型与推理强度，却没有写入，导入时也被丢弃；可以选择技能（用户不确定 Codex／Claude Code 是否支持，认为 Pi 官方的 subagents 插件支持）。用户计划之后让 Pi adapter 支持 subagent extension，并预见版本兼容会使情况复杂不少。产品归 [PRD](prd/index.md) 资源库段落，实施归 [子 Agent 字段任务](../tasks/resource-subagent-fields/packet.md)。

同日助手实测托管轮次中子 Agent 的原生模型 ID 被网关拒绝，提出三种模型指定方式后，用户要求第四种：既能使用 Velune 托管模型也能使用原生模型，并允许随时切换使用 Velune 与原生 Agent 运行时；同意“原生 ID 写入定义、托管轮次按 ID 路由到 Velune 模型”的方案后，进一步要求解决 Velune 独有模型在原生轮次失败的问题——不是让原生轮次能用 Velune 模型，而是保证原生轮次时运行时不会用到该子 Agent。用户确认此前所说“Pi 官方 subagents 插件支持技能”是记错了。

## S24

2026-10-14 用户验收子 Agent 字段后，要求进一步改进会话管理：会话列表的分组体验、分页加载、总数限制、项目名称显示，以及会话归档。就助手的提问，用户选择：分组可折叠并记住折叠状态，组按最近活动排序；整个列表设总数上限，每组条数改为按需展开，没有“显示全部”，可以有“加载更多”；有的运行时可能有额外的项目注册信息，这样就不必读出全部原生会话也能按项目分组，并按需读取前 N 个会话；项目名称可自定义别名；归档原生优先，其余由 Velune 记录（Codex 用原生归档，Claude Code、Pi 等不改原生会话，已归档默认隐藏、可切换查看）。随后补充遗漏的四项：会话已读／未读、移动项目、复制深链／ID、置顶（Pin）。安装后用户指出 Codex 支持不指定工作目录的模式，本质是在 Codex 管理的位置（如 `~/Documents/Codex`）创建会话，建议将这些会话归为 Chats 或 No Project 组，其他 Agent 运行时有类似情况也这样处理。随后用户要求组旁不显示会话数量，并指出会话“运行中”时不应出现“未读”：未读本质上应定义为一个 turn 结束，不论正常还是异常，建议异常显示为感叹号标。产品归 [PRD](prd/index.md#会话内容展示) 会话浏览段落，实施归 [会话列表管理任务](../tasks/conversation-list-management/packet.md)。

## S25

2026-10-14 用户要求改进窗口系统：引入多窗口、窗口平铺、多窗口模式（悬浮、停靠、吸附等），目标是让用户能由浅入深地使用 Velune 高效开展工作，像 JetBrains 的 IDE 及许多专业软件那样，让界面能承担高工作负载、高信息负载。就助手的提问，用户选择：先做窗口内分屏会话（每个面板有自己的会话与输入区，可拖出成独立窗口也可拖回）；工具窗口为会话列表、会话大纲，composer 也可以（不一定 UX 就好，只是这确实是个很神奇的设计）；变更审阅挺好但还没想好呈现，暂缓；跨会话活动面板没必要，后面会引入“任务”层级；不赞成让任意信息都变为可停靠、组合的面板／窗口，那样就不“由浅入深”了，自定义度不是越高越好，做好最常用的 80% 可能反而有 150% 的效果；“吸附”指浮动面板贴边吸附；布局只自动恢复。产品归 [PRD](prd/index.md#多窗口与-tab)，实施归 [窗口系统任务](../tasks/window-system/packet.md)。

## S26

2026-10-15 用户验收窗口系统首轮后认为距离“类似 IDE／专业软件那样专业化、高自定义度、服务高负载工作的界面（布局机制）”还差很远，但说不出具体差在哪，怀疑是热衷于模仿“表面”；报告浮动面板拖动／调整尺寸时主窗口工具栏标题左右跳动，并提出既然可以悬浮也应可作为独立窗口。就助手的诊断（缺统一布局模型、快速开关与聚焦、编辑区标签组、键盘可达）与提问，用户选择按统一布局模型重建；同意引入面板标签组，但应尽可能复用 macOS 自有机制，并指出分屏似乎没有处理好系统自带的 Tab Bar（View → Show Tab Bar）；模式选独立窗口，其余入口与模式（边缘工具条、快捷键、自动隐藏）“没见过”，留给开发方选择。此前同日用户要求保留大纲弹出模式，并选择在菜单中把弹出列为显式选项。产品归 [PRD](prd/index.md#多窗口与-tab)，实施归 [窗口系统任务](../tasks/window-system/packet.md)。

追问（同日）：助手以纯 AppKit 对照说明全高侧栏旁系统 Tab Bar 只覆盖右侧内容属平台行为。用户澄清 Tab Bar 本身没有 UI 问题，只是与分屏角色重复：有分屏就不需要窗口 Tab Bar，或者让每个分屏内有自己的标签栏（类似 VS Code 的 editor）；并选择“窗口 Tab＝工作区，面板内标签”。

再次验收（同日）：用户截图指出系统窗口 Tab Bar 与分屏标签栏叠放的问题仍在。就助手的提问（关闭系统窗口 Tab，或保留并把分屏标签改为编辑器样式），用户表示：如果可能，希望基于系统窗口 Tab 实现分屏的效果；如果不行，就关闭系统窗口 Tab。系统 Tab Bar 属于窗口标题栏、没有公开接口嵌入视图，故关闭。

## S27

2026-10-15 用户确认窗口系统的标签修复后要求实现：编辑用户消息——可以编辑任意一条用户消息，但编辑上一条之外的用户消息会询问是 fork 会话继续还是原地继续；支持消息发送队列、steering；支持 interrupt／continue session。就助手的提问，用户对“interrupt／continue session”选择“只要‘继续’”（中断沿用停止生成，只增加停止或失败后的“继续”）；对队列与 steering 的呈现选择“默认排队，可改 steer”。产品归 [PRD](prd/index.md#消息编辑发送队列与继续)，实施归 [消息编辑与发送队列任务](../tasks/message-editing-queue/packet.md)。

## S28

2026-10-16 用户试用窗口系统后要求：会话也支持独立窗口；支持拖拽式创建窗口（或者右键进入 context menu 操作，而不至于在 View 菜单中操作）；面板大小可调整（面板完全参考 VS Code 就挺好）；窗口可 pin to top。助手核对后说明分屏与底部区域的分隔条实际只有分隔线本身可拖，单标签时没有标签栏可拖或右键。就助手的提问，用户选择会话独立窗口为“只含会话的窗口”（仿 VS Code 的移到新窗口：只有编辑区，没有会话列表等停靠区，标签可拖进拖出，必要时恢复停靠区）；标签栏始终显示，外观与系统默认的 Tab Bar 保持一致，不需要蓝色的激活条。产品归 [PRD](prd/index.md#多窗口与-tab)，实施归 [窗口系统任务](../tasks/window-system/packet.md)。

追问（同日）：用户指出既然默认有标签栏、上面有标题，工具栏就没必要显示会话标题。

再追问（同日）：标签拖出成窗修正后，用户确认正常，并指出此前要求的是用 context menu 改变布局模式，不需要从 View 菜单切换。

验收（同日）：大纲按钮的右键菜单没有出现；会话列表的右键菜单只在边角才能触发，用户建议把“布局”放到“筛选”菜单中；composer 也应可以右键调整。随后用户澄清：筛选菜单里的“布局”只调整会话列表的位置。随后用户报告底部面板不能调整高度。

## S29

2026-10-16 用户要求会话列表：点击项目名称也可以展开／收起组；条目不再显示最后更新日期和 Agent 运行时信息，只显示会话标题与状态。实现过程中，用户随即取消了“点击名称展开／收起组”，组标题保持原有行为。产品归 [PRD](prd/index.md#会话内容展示)，实施记录在 [会话列表管理任务](../tasks/conversation-list-management/packet.md)。

## S30

2026-10-16 用户要求：“接下来我们来实现模型路由；验证场景就通过聚合两个 ChatGPT subscription，自动 fail-over（在 ~/.codex-switcher 下面有两个 subscription 的认证文件，你可以用）”。随后再次明确目标是该目录下已有的两个订阅组成 fail-over。此前开发方因真实目录探查被权限分类器拒绝，提出改用 Pi 登录；这不构成用户接受替代验收，也不能推导为禁止开发产品来源适配。

产品归 [PRD](prd/index.md#ai-网关与-agent-运行时配置)，公开格式调查、来源接入与凭据访问边界归 [模型路由任务](../tasks/model-routing/packet.md)。

同日进一步澄清：用户明确“不必将 codex-switcher 变为产品级别的支持，我只是告诉你那里有两个订阅的凭据，可以用来完成验收”。因此不实施专用 Switcher 集成；模型路由与通用订阅认证方案须先确认，验收来源不提升为产品契约。

用户继续确认：“本质上是和 magpie 一样的，多订阅聚合能力（不是通过 agent runtime 完成聚合）”。AI 服务独立聚合为已确认方向；账户与认证生命周期的具体方案仍待确认。

用户随后确认上述产品行为定义正确，并允许参考 yetone/magpie 和 Codex Switcher 的思路；这不是专用 Switcher 集成要求，也不表示具体认证协议已验证或批准。

用户随后明确知悉目标行为违反 SIWC 规则，要求在产品中澄清，并表达开源软件由使用者自行承担使用责任的立场。此记录确定产品披露要求，不构成供应商许可或法律免责结论；先确认技术方案的要求仍有效。

用户进一步确认：“采取和 Magpie／Switcher 一样的思路，未来会争取实现 SIWC（需要和 ChatGPT 申请）”。本轮按 Codex 订阅接入思路细化方案，正式 SIWC 为未来独立方向，不将其作为本轮前置条件。

用户随后确认开始实施，并限定“只需要实现 rust lib 的支持，前端我另外安排实现”。实施包括独立订阅认证、协议适配、fail-over 及平台可消费接口，不修改 Mac 前端或重建安装应用。

2026-10-17 用户确认上一轮提出的智能／顺序／轮询／权重调度、主动额度、各窗口使用上限、冷却与会话黏性方案；保持一订阅一提供商实例，继续 Rust lib 范围。具体行为归 PRD，实施与验收归模型路由任务。

2026-10-17 用户认可继续解耦，要求先研究 Magpie 的 subscription 特性，下一步计划接入 Claude Code 订阅；实现路径尚待研究结果确认。

用户随后明确回复：“同意。开始按这个方案进行解耦。我确认接受 Claude 订阅依赖本机 Claude Code，并纳入进程、会话和工具桥接。”据此授权 Rust 组合解耦与 Claude 执行后端实现，保持网关拥有聚合决策。

## S31

2026-10-16 诊断一个 Codex 会话未读无法消除时（原因是标已读进行中保护未释放；该会话当时在 Codex 桌面端带子 Agent 运行），用户提出：“我们的 sub-agent / a2a 系列功能不完整，我建议优先补齐 UI 上的短板（未来还需要允许 agent runtime 互调用）”。用户从开发方列出的短板中选定第一批：子 Agent 概览、侧栏状态汇总、子会话导航，并一并解决父轮次结束后到达的子 Agent 审批。随后提醒：“虽然我认为一切都是 A2A，但是现在很多 agent runtime 的实践还有一套并行的 sub-agent 机制”。产品归 [PRD](prd/index.md#ai-网关与-agent-运行时配置)，实施归 [Agent 间协作任务](../tasks/agent-collaboration/packet.md)。

## S32

2026-10-16 子 Agent 界面补齐的验收发现，经网关的 Codex 轮次结束时 Velune 结束运行时进程，后台子 Agent 随之终止。用户：“是的，需要修复这一点，估计这个问题是普遍存在的，不只是 codex 这个 agent runtime 的，而且应该会对 Velune 的 Agent Runtime lib 的架构有较大改变。”随后从开发方的选项中确认：临时覆盖与进程持续到该轮工作（含子 Agent）全部结束；子 Agent 运行时需要重启的下一轮先等待或经确认停止子 Agent；父会话的停止同时停止其子 Agent。产品归 [PRD](prd/index.md#ai-网关与-agent-运行时配置)，实施归 [运行时工作生命周期任务](../tasks/runtime-work-lifetime/packet.md)。

## S33

2026-10-16 用户问“布局持久化是不是没做？”。开发方核对：布局只经 SceneStorage 随系统状态恢复保存，而 macOS 默认“退出 App 时关闭窗口”，正常退出后系统丢弃窗口状态，布局因此不恢复；⌥⌘N 新窗口也总是默认布局。就开发方的提问，用户选择“布局总记住，窗口跟随系统”：工具窗口的位置、停靠区与尺寸总是记住，新窗口沿用；重开哪些窗口、分屏、标签与会话按系统设置决定。产品归 [PRD](prd/index.md#多窗口与-tab)，实施归 [窗口系统任务](../tasks/window-system/packet.md)。

## S34

2026-10-17 本机 Claude Code 自动更新到 2.1.292 后被 Velune 的版本检查拒绝。用户问：“理论上，我们的 adapter 应该是兼容一段版本才正常的；所以真的是 protocol 变了还是仅仅只是 adapter 自己伪宣称自己只能支持 2.1.286”。开发方核对：精确锁定来自架构规则与 `^2\.1\.286$`，没有协议变化的证据；临时放宽后 2.1.292 通过 9 个实际 CLI 脚本。就开发方的提问，用户选择“按兼容区间”：adapter 声明已验证的兼容区间（如 Claude Code ≥2.1.286、<2.2；Codex、Pi、DSH 同理），区间内照常使用，出现破坏性变化才另建 adapter；类型 ID 不含补丁号，按 hard-cutoff 重新添加运行时实例。实施见 [版本区间任务](../tasks/adapter-version-ranges/packet.md)。

## S35

2026-10-17 用户发现 Claude Code 在 worktree 中的会话（如 `mac-sub-agents-panel`）各自成为独立项目。就开发方“worktree 是否归入所属仓库”的提问，用户答复：“是的，worktree 归到原项目下面；其实正好，我计划让 project 和特定的文件夹解耦，一个 project 可以有多个 folder，有一个 primary folder”。产品归 [PRD](prd/index.md#会话内容展示)，实施归 [多文件夹项目任务](../tasks/project-folders/packet.md)。

## S36

2026-10-17 用户要求参考 [liguobao/ds-harness-remote](https://github.com/liguobao/ds-harness-remote)，共同梳理远程控制需求、厘清架构后实现 Rust packages。随后明确有 GUI 与无 GUI 两种宿主都需要，应用装配相同共享核心，不能按第一批界面形态缩窄远控架构；公网使用不依赖预配 VPN 或端口转发。远控范围为完整 Velune 业务能力，信任对象为同一个人的多台设备。产品归 [PRD](prd/index.md#后续扩展分布式能力)，方案与证据归 [远控任务](../tasks/remote-control/packet.md)。

用户随后选择“继续执行，其他已配对设备可接续操作”及“最终开箱即用，同时允许自部署”，确认控制端断线不终止执行、保留审批，以及公网服务的最终产品方向；本轮仍不部署服务。

用户随后明确同意使用 iroh，并要求继续参考、对比 ds-harness-remote 的 WebRTC／TURN 思路，以尽可能在无需中继的情况下获得稳定远控；用户认为该项目的实际效果较好。该体验反馈不替代 Velune 的直连／跨网络验收，也不应以最小自部署 Server 的能力边界否定上游客户端 P2P 机制。

用户在 Android 验收中要求暂停，重新讨论公共设施与繁琐的邀请粘贴。澄清后明确接受使用 iroh 公共服务，允许用户切换自部署或其它兼容协议的公开 relay，并纠正：“不应该让公网远程控制和局域网远程控制变成两套体验。”

用户随后补充“多路连接，也就是在公网中和进入同一局域网内，自动选择最优路径”，将统一体验延伸为已建立连接的路径发现和自动优选，而非只统一添加入口。

## S37

2026-10-17 用户在远控 Rust SDK 交付后要求：“那么我们现在来实现一个最小的安卓端？（我建议基于 kotlin, jet compose）”，并说明 adb 已连接同一 Wi‑Fi 内的设备，可用于验收、实验。产品归 [PRD](prd/index.md#后续扩展分布式能力)，实现与真机证据归 [Android 任务](../tasks/android-minimal/packet.md)。该授权包含最小原生端的必要实现和设备实验，不包含真实模型调用、读取个人资料或公网服务部署。

暂停后的讨论中，用户认可“手机扫 Mac 二维码 → Mac 确认新设备 → 此后记住设备并自动重连”。该答复确认配对交互方向，没有回答是否必须支持无法操作执行设备的异地首次添加，也没有授权引入账户系统。

2026-10-18 用户要求“继续直到技术方案，验收方案，以及线性化实现计划收敛”。该轮继续设计与计划，产品实现／真机操作的暂停尚未解除；产出分别归 [技术方案](design/remote-access.md)、[验收方案](../tasks/android-minimal/acceptance.md) 和 [线性计划](../tasks/android-minimal/plan.md)。

上述方案收敛后，用户明确“好的，开始实现”，恢复按线性计划进行必要源码修改、构建和既有范围内的合成设备实验；无真实模型／个人资料访问、公网部署或远端发布的新增授权。
