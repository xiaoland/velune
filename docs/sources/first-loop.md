# 来源：AI 服务与 Mac 首循环

[返回来源索引](../sources.md)。本页保留原始决定与当时的授权边界；它不是当前实现状态或本次任务的授权清单。现行规则从[文档入口](../index.md)按主题查阅。

## S13

- 来源：父会话 `01a0fb5e-ceba-778c-9e3d-6cbf22b340d8` 于 2026-10-03 转达的逐项讨论结果和有界实现委托；不伪造原始消息 ID
- 已确认：AI service 是独立 lib，领域不限 LLM；sampling 是其操作；service 拥有调用方／provider 两套权威契约；provider 即 adapter，自行转换外部协议；service 不依赖具体 SDK
- 配置：provider 为协议配置＋凭据引用＋模型列表，不增加账户／渠道实体；统一配置中心持久化，app main 装配；调用开始后配置稳定，新配置只影响后续调用
- 范围：只模块／类型／构造校验／必要接口与文档；不做路由、fallback、重试、网络、真实凭据、Host/UI 改造或测试，只允许静态基础检查
- 隔离：从已发布 `3bce5f9` 新 worktree／分支开始，保留暂停的 IPC／测试改动；允许新有界开发分支普通 push，不改 main、不强推、不创建 PR
- 验收补充：用户指出仅实现不足以闭环，需要验收方案；随后提出真实 AI 接入并固定 fixture。provider／model、凭据位置和预算尚未答复，未授权猜测或提前访问秘密／调用。固定 fixture 不是无限新增测试框架授权

## S14

2026-10-03，父会话 `01a0fb5e-ceba-778c-9e3d-6cbf22b340d8` 的有界委托：从 `7c6f98267a56101d2117311adeaf10b50b01d8f2` 实现 service 直接流式派发、MiniMax 普通 API adapter、合成 live fixture 与离线重复验收；预算总 ¥5，本步最多新增 8 attempts，先文本／工具各一次，工具只返回不执行。批准普通 push 新 dev 分支，禁止 tests／PR／main／force／部署／Host UI 改造／其他 provider。Networksecret 配置类型由用户配置页证据与平台占位契约确认，不输出／保存值。长期技术归属 [AI service](../product-tdd/ai-service.md)，实际用量、失败和剩余验收项归属 任务（git 历史：tasks/ai-service-contracts）。

## S15

2026-10-04，本会话用户要求将 `dev/minimax-stream-fixtures` 合并主分支，并推进 Pi agent＋Mac app 首循环；资源示例为 ChatGPT 订阅、Tokenflux、ARK Coding Plan。随后明确：资源由用户在应用中配置，不能硬编码；需要 Chatbot 界面与会话列表；会话持久化不在本任务，Velune 只是 projection；真实验收由用户做；模块至少拆为 core（AI 服务、Harness 适配器）与 app（Mac 等平台）。用户在体验后要求完全重写 Mac app，提供 Velune SVG logo，并要求避免与任何提供商或 Agent Harness 耦合。用户进一步指出该界面缺乏 Apple 原生体验，确认项目级原则：尽可能贴近所属平台原生视觉风格，品牌／软件特点在微小细节体现。随后要求 AI providers、Harness 等应用配置通过文件系统持久化，位于 `VELUNE_HOME`（默认 `~/.velune`）。用户进一步明确 Apple app 不必独立验收深色模式：使用原生组件与原生系统视觉，常规明暗适配交由系统。产品边界归属 [PRD](../prd/conversations.md#会话权威与投影)，执行证据归属 任务（git 历史：tasks/pi-mac-first-loop）。没有为当前消息伪造 ID。

S15 后续纠正：模型独立且跨提供商共享；提供商可关联多个模型，模型有参数与展示元数据；协议首个为 OpenAI ChatCompletions v1 枚举选项。提供商只是 Velune 网关配置一部分，另有路由与 fail-over，Harness 仅使用注入的网关，不利用已有上游认证。“连接”页改为 Agent 运行时，类型与配置实例分离，同为 Pi 的不同配置也形成不同实例。

S15 品牌补充：默认使用 graphite logo，不使用 theme 配色作为默认标志。

S15 产品与协作补充：Velune 是 control surface 而非 Agent；消息按用户右、LLM 左、系统／Harness 中间布局，不显示头像／昵称，不标记 LLM 为 Velune。用户明确持续迭代协作，开发方主动维护任务、长期文档和代码质量，并允许自主提交当前任务改动。此授权未扩为 push／部署。

S15 运行与交付补充：core 应为跨平台一致的 lib，由各平台 app 通过 ABI 嵌入，不能作为独立 Host 进程。Mac 安装到 Applications，修改后重新安装；当前产品显示版本为 0.1 beta.1。旧 Host 版本不匹配是接入实现的偏差，不应要求用户长期手动管理该进程。

S15 协作方式澄清：采用敏捷开发，需求在实现阶段可以变化；早期重视长期可迭代的技术架构与开发基础，不要求仓促产出。关键设计善用 advisor，发现需要用户取舍的问题时，带着证据与建议请求决定。

S15 架构参考补充：用户提出 Mastra、HAPI、Lody 等实现可用于借鉴思路与架构，不以复制功能为目标。具体技术结论需按相关设计问题检查源码版本与前提，不因列入参考就认定兼容或采用。

S15 后续配置与交付纠正：工作目录属于具体会话，运行时目录是配置／状态根目录；用户要求从运行时表单移除必填工作目录。用户允许开发方随时退出或强制退出运行中的 Velune 完成更新，并要求删除全部既有自动化测试，采用静态检查和人工／临时端到端验收。

<a id="s16"></a>

## S16：统一网关与 Harness 配置来源

2026-10-04 用户提出保留 Harness 原提供商配置，读取、接管并共享其配置与认证来源；希望复用 Pi 订阅登录，Velune 管理会话默认使用 `velune/auto` 或按 Harness 区分的稳定入口。该意图不等同于复制令牌、复用任何客户端注册身份或已授权真实登录。

官方复核：[ChatGPT 开源客户端注册与登录](https://developers.openai.com/siwc/token-sharing-open-source/sign-in)要求明确应用身份、主机标识与授权 scope；[模型与推理](https://developers.openai.com/siwc/token-sharing-open-source/models-and-inference)描述使用 public Responses 端点。2026-10-04 检查；不能将旧 Codex backend 认证与该路径视为相同。Pi 固定版本的具体能力与验证缺口留在当前 Task Packet。

用户在本轮进一步澄清：`max_output_tokens` 等具体字段用于举例能力参数，不是必须支持的需求；以提供商规定的协议为权威。原生 Responses、稳定入口和认证委托的实施已获授权，真实账户验收仍由用户进行。

S16 导入语义纠正：用户明确将 Harness 提供商配置导入 Velune 视为一项完整功能，认证来源接入属于其中，不能作为完整导入已交付的证据。

2026-10-04 用户进一步提醒两个独立边界：Agent Harness 不与 AI 服务耦合，AI 服务不与大语言模型领域耦合；导入配置属于适配／装配行为，LLM 相关字段不能成为整个 AI 服务的基础假设。需求归属 PRD“AI service 的有界实施方向”。

2026-10-04 用户补充原生视觉原则：原生控件无需专门实现深色模式，但 Core Graphics、固定色 PNG、logo、自定义阴影、渐变和边框仍需要处理暗色背景下的显示。此补充已归位 PRD 的平台体验原则。
