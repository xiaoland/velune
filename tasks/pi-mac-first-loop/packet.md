# Pi 与 Mac 首循环

## 目标与当前范围

交付 Velune 原生 Mac control surface：会话列表与聊天界面、应用配置、Agent 运行时实例，以及 `Pi → Velune AI 服务网关 → AI 提供商` 的实际调用路径。开发方完成构建与隔离验证，真实登录、模型请求和最终验收由用户执行；不读取真实凭据或会话，不调用真实模型。

长期决定归属 [PRD](../../docs/prd/index.md)、[架构](../../docs/design/architecture.md) 与 [AI service](../../docs/design/ai-service.md)，运行入口归属 [开发说明](../../docs/development.md)。用户已授权实施、自主提交当前任务，并要求持续维护任务、长期文档与代码可维护性；远端发布仍需独立授权。

## 当前状态

上一切片 `5032781` 的独立 Host／IPC 接入已被用户明确纠正为跨平台 Rust lib，经 C ABI 嵌入平台 app。CoreRuntime 拥有配置校验与原子持久化、通用动作、Pi／网关与投影生命周期；平台显式提供 home、资源目录和秘密设施。Mac 不启动 Host/socket，不打开模拟 SQLite；只有外部 Pi Harness 是子进程。产品显示版本为 `0.1 beta.1`，安装到 `/Applications/Velune.app`。

advisor 已核对配置归属、ABI 所有权、单写锁与忙时退出契约。旧 Host 在确认已知 owner、无直接运行时子进程后一次性退休，未读取业务响应或真实会话；此迁移不构成通用自动终止策略。应用更新要求正常退出，忙时拒绝退出并保留 handle，不自动取消工作。

协作采用敏捷开发，需求可在实现中变化；早期以明确关键契约和维护基础优先。Mastra、HAPI、Lody 作为架构候选参考，按具体问题核对版本与前提，不据此直接采用功能或认证方案。

## 已确认的边界

- Velune 是 control surface。消息不显示头像或昵称，用户在右、LLM 在左、系统／Harness 在中间，不以 Velune 标记 LLM 作者。
- 各平台遵循原生视觉与交互；品牌仅在少量细节体现，默认 graphite logo。Apple 不设独立深色模式开发或验收项。
- core 拥有 AI 服务与 Harness 适配；app 拥有平台 UI、ABI 与秘密设施装配。Mac 不解析 Pi 协议或按提供商分支。
- 模型跨提供商独立存在，有 ID、昵称、图标、输出上限与推理级别；提供商通过外部模型 ID 关联多个模型。协议是有限选项，支持原生 OpenAI ChatCompletions v1 与 Responses v1。
- 提供商、模型路由和 fail-over 归属网关配置。首轮使用显式路由，fail-over 禁用，不擅自增加自动策略。执行 Harness 只接 Velune 网关；网关可明确接入当前支持的 Harness 认证来源，不自动读取工作环境认证。
- Agent 运行时区分类型和实例。同为 Pi、不同配置目录或工作目录形成不同实例。首轮一个活跃 runner，空闲切换实例，会话身份按实例隔离。
- 应用配置文件位于 `VELUNE_HOME`，默认 `~/.velune`。凭据值在 Keychain，文件仅保存引用。Pi 拥有会话持久化，Velune 只做 projection。

## 基线与依赖

`main` 已从 `dev/minimax-stream-fixtures` 快进至 `2684f14755bfd1e41d36d48ddbb4d68afaa0fb85`，本地和远端 main 已同步。当前开发分支 `feat/pi-mac-first-loop` 承载本切片，尚未发布。旧 MiniMax 合成 fixture 人工 replay 已匹配 expected，不能代替新网关证据。

Pi 固定 1.0.2、commit `cd32f7725fdbddbaecdff5b1e68491563394e0ca`，SDK／CLI 安装于隔离 target 并随 bundle 放入 Resources；Node 22.19+ 由用户配置。该版没有 list_sessions RPC，列表通过 [SessionManager.list](https://github.com/earendil-works/pi/blob/cd32f7725fdbddbaecdff5b1e68491563394e0ca/packages/coding-agent/examples/sdk/11-sessions.ts)；稳定终态为 agent_settled。Pi home 映射为本版 PI_CODING_AGENT_DIR。依据见 [RPC](https://github.com/earendil-works/pi/blob/cd32f7725fdbddbaecdff5b1e68491563394e0ca/packages/coding-agent/docs/rpc.md)与 [模型配置](https://github.com/earendil-works/pi/blob/cd32f7725fdbddbaecdff5b1e68491563394e0ca/packages/coding-agent/docs/models.md)。

## 当前实现与证据

Mac 使用 SwiftUI WindowGroup／Settings、NavigationSplitView／List、系统工具栏与 Form，默认 graphite logo。隔离 preview 强制内存 Store，不打开 CoreRuntime、真实配置、Keychain 或会话；此前消息布局、原生设置、多实例界面与快捷键检查保留为界面证据。Apple 不设独立深色验收。

C ABI 1 使用 opaque handle、UTF-8 JSON 和 Rust 返回字符串释放函数；动作采用投影契约 3，配置 schema 2。Swift 所有调用串行，open 失败不留 handle，busy close 保留 handle。生产构建仅含动态库与秘密 helper；旧模拟 Host／SQLite 通过非默认 `simulation` feature 隔离。Mac 原生“关于”面板已观察显示 `Version 0.1 beta.1 (1)`，安装路径已验证。

2026-10-04 使用固定 Pi 1.0.2、临时 HOME／配置／工作／会话目录、fixture-only helper 与 loopback 合成 ChatCompletions 上游，已通过 [ABI 回归](../../scripts/check-pi-abi-loop.py)：配置→连接→创建→流式回复、两模型外部 ID 与输出上限切换、SDK 列表与当前会话身份一致、恢复会话选择、新会话默认模型、busy close→取消→idle close、重新打开配置，以及不创建 Host socket／SQLite。检查使用 bundle 内动态库与资源，未读取真实用户资料或调用真实模型。合成带签名历史验证 Pi 跨模型剥离签名、同模型保留；目录明确携带不同上下文窗口。

上一切片的模型方案经 advisor 推敲：Velune 决定对话逻辑模型和网关路由，Pi 感知模型身份及能力。该切片采用直接模型切换，当前已由下述 virtual model＋绑定身份替代。Pi `setModel` 影响 thinking level 与 transcript，`transformMessages` 按 provider/api/model 清理跨模型推理签名，压缩与输出预算使用 contextWindow；自定义目录缺省为 128000，不能当作真实能力。依据固定提交 `cd32f7725fdbddbaecdff5b1e68491563394e0ca` 的 `agent-session.js`、`provider-composer.js` 与 `pi-ai/dist/api/{transform-messages,simple-options}.js`。当前目录注入是 Pi 适配方式，不是永久产品约束；上游厂商特殊能力仍需要独立 provider adapter 验证。

上下文窗口可空保存草稿，运行时必须显式正值，输出上限不能超过窗口；缺值模型不进入 Pi 目录，不阻断完整模型。恢复前经 SessionManager 获取所选分支的模型，避免 Pi CLI 默认模型覆盖会话选择；不增加会话数据库。SDK 列表使用物理 cwd，解决 Mac 临时路径别名造成列表为空的已观察问题。

Rust 检查涵盖 ABI 配置重开／重复 home／无效输入、模型草稿与能力边界、网关真实工具 JSON 参数、非法输入、客户端断开、Runner drop、上游 500 不产生成功 DONE。Windows GNU target 的 lib 类型检查通过，未实现 Windows app 或宣称 Windows 运行验证。Swift 编译、本机 bundle 构建与 deep／strict ad-hoc 签名通过。安装器检查运行实例与 bundle identity，失败恢复旧安装，拒绝覆盖运行中应用。

历史 IPC 隔离循环与 `5032781` 的结果只证明当时 Host 接入，不作为当前 ABI 证据。任务仍开放，真实用户验收未完成；manifest 保留 native_verified=false。

## 当前切片：配置接入、认证委托、auto 与 Responses

2026-10-04 用户授权调整统一网关，保留 Harness 原配置与认证来源，默认稳定 `velune/auto`，并新增原生 Responses v1，暂不翻译协议。用户进一步澄清输出 token 上限等字段是能力参数示例，以提供商协议为权威，不要求所有提供商支持。长期意图归位 PRD，契约归位架构与 AI service 文档。

已实施独立 Responses service 操作、Rust provider adapter 与 gateway ingress，支持 foreground JSON／SSE，保留原生工具与 encrypted reasoning 数据；识别 completed／incomplete／failed／cancelled，不将 EOF 或 `[DONE]` 当 Responses 成功。ChatCompletions 与 Responses 可混合路由，协议错配明确拒绝，不添加转换、重试或 fail-over。

Pi public virtual model 保存 `velune/auto`，Core 决定逻辑模型与物理绑定，Pi 分支 state 保存选择。物理 binding ID 基于非秘密路由、上游模型与认证来源身份，使用固定 sha2 0.10.9；昵称、预算与 Node 路径不改变身份。网关派发同一次不可变绑定，Pi 原生转换跨绑定历史，Velune 不写清签名 hook。切换后尚未发送即重开可恢复；新会话仍使用运行时默认模型。

提供商可配置 Keychain 引用或显式 Pi 来源，二者互斥。来源 helper 只负责 SDK 认证解析、刷新、非秘密信息与交互登录，Rust 执行推理。来源使用原 AuthStorage 锁，不复制 refresh credential；执行 Pi 不获取上游授权。ModelRuntime 为公开 SDK，AuthStorage 未由 package root 导出，adapter 集中绑定固定 1.0.2 的 dist 文件并严格检查版本；该维护成本经 advisor 采用，不伪称完全公开稳定入口。成功登录由 Core 更新匹配来源的 credential generation、持久化并断开空闲 runner；更新 API key 同样更新 generation。外部在相同引用后换账户的身份辨识仍未覆盖，详见架构边界。

Pi 新 `openai` 订阅路径是公共 Responses，legacy `openai-codex` backend 不接入此来源。SDK 注册提示名固定 Pi，公开 LoginOptions 无应用名参数，复用的是 Pi 来源登录，不包装成 Velune 自有注册。来源能力限制在 Pi payload 构造阶段投影，gateway 不默默删字段；订阅不承诺服务端输出硬上限，并要求 stream=true、store=false、不使用 previous_response_id。官方依据见 sources S16。

当前 release bundle 的三种 ABI 隔离模式均通过：ChatCompletions、普通 Responses、Pi 订阅能力投影。验证包括 auto 与物理历史身份、模型切换／未发送恢复／新会话默认、busy close／取消、配置重开，以及经 Pi SessionManager 加入合成签名与工具历史后，同一逻辑模型改变上游绑定：旧签名不传出，工具调用／结果仍关联，wire 选中新目标。订阅投影模式只在临时受管 selection 注入合成能力，不绕过生产来源与 endpoint 校验，不代表真实 OAuth 联调。

此前认证隔离实验观察到同源并发只刷新一次、轮换仅写原来源、缺失／错误类型／刷新失败不回退环境、注销竞争不复活凭据、版本不匹配拒绝，以及合成登录写回原来源。2026-10-04 用户明确禁止新增自动化测试，本轮新增测试已撤除，后续以类型安全、静态检查、构建和临时端到端验收为准；上述为既有实验记录，不交付认证测试套件。真实账户、登录、会话与模型 API 均未由开发方读取或执行。

## 完成条件与用户验收

开发验证须证明合成请求经过真实网关并选中正确提供商，输出上限／推理级别与工具结果往返贯通，失败不产生成功终态，取消停止活跃上游请求；新配置和多运行时实例可重启恢复。最终 bundle 以 manifest 关联当前提交，保留 native_verified=false，不能因隔离验证将任务关闭为真实循环已通过。

用户随后在应用配置模型、提供商与显式路由，配置并选择运行时实例，选择／新建会话，发送消息，观察流与工具完成，取消，并重开继续观察。ChatGPT 订阅认证与未实现协议不伪装为已支持；资源例子不成为硬编码预设。
