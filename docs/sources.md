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
