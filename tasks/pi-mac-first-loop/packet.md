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

2026-10-04 使用固定 Pi 1.0.2、临时 HOME／配置／工作／会话目录、fixture-only helper 与 loopback 合成 ChatCompletions 上游，已通过 [ABI 回归](../../scripts/check-pi-uniffi-loop.py)：配置→连接→创建→流式回复、两模型外部 ID 与输出上限切换、SDK 列表与当前会话身份一致、恢复会话选择、新会话默认模型、busy close→取消→idle close、重新打开配置，以及不创建 Host socket／SQLite。检查使用 bundle 内动态库与资源，未读取真实用户资料或调用真实模型。合成带签名历史验证 Pi 跨模型剥离签名、同模型保留；目录明确携带不同上下文窗口。

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

## 自动化测试清理（2026-10-04）

用户进一步明确既有自动化测试也必须删除。本次清理覆盖 Rust 内嵌测试、独立测试 target、安装器 Python 测试，以及专供这些测试的分支、依赖和运行说明。保留生产实现、人工模拟入口与临时端到端验收脚本；历史实验记录表示当时观察，不代表当前保留测试套件。清理后以类型检查、静态检查与 Mac bundle 构建确认交付完整性。

## 会话工作目录归属（已实现，待用户验收）

2026-10-04 用户指出工作目录应属于具体会话，而非 Agent 运行时实例。当前实现确实把 `workingDir` 设为实例必填项，在连接时固定 Pi 进程 cwd，并以该 cwd 过滤 SDK 会话列表；这使一个实例无法自然覆盖多个项目。建议运行时实例保留配置／状态根目录，创建会话时指定工作目录，恢复时读取 Pi 会话记录的 cwd，并按该目录装配执行进程及项目上下文；会话列表不能继续由实例固定 cwd 限制。运行时目录在固定 Pi 1.0.2 中映射为 `PI_CODING_AGENT_DIR`，不是会话项目目录。用户随后明确要求完成修复。经 advisor 核对固定 Pi SDK，采用统一生命周期：实例连接只启动网关与准备配置，创建／恢复时以会话 cwd 启动 Pi child，空闲切换重启 child 并复用网关。恢复不传实例名称覆盖原会话标题，失效目录明确报错。实现与临时端到端验收已完成。

## Mac 应用图标尺寸（已修复，待用户验收）

用户报告 Dock／应用库图标尺寸异常，并指出 Shenglin 曾出现相同问题。当前生成器将圆角底板画为画布的 93%，需对照系统图标的实际光学占比与透明边距，区分内容尺寸与图标缓存问题。保留 graphite 光学 SVG 的品牌几何；以生成图像人工对比、Swift 静态检查与 bundle 构建验证修复，不添加自动化测试。已定位声邻仓库的 2026-09-30 验证记录：其原图标铺满画布，而本机 macOS 15.4.1 的“信息”图标主体宽 824／1024；声邻改用同样留白，16／32 像素档缩小留白以保持识别。Velune 原底板为 93%，与相同尺寸边界不符，修正应同步缩放底板及内部标志。

图标生成器已采用上述逐档留白，并以同一比例缩放底板、内部标志与圆角。临时人工对比确认原构图保持，1024 档 alpha 主体范围为 100…924（宽 824），512 档为 50…462（宽 412）。Swift 生成与 whitespace 检查通过，未新增自动化测试；最终 bundle 安装随会话目录调整一起完成。

会话目录改造的初次临时端到端验收未通过：延迟启动后的首轮可能在初始化事件中提前显示 settled，恢复路径也出现未稳定终态。当前只能确认目录投影与跨目录列表，不能将完整收发判为通过；执行 owner 随后修复初始化 RPC readiness 与当前 turn 事件匹配：未匹配的 settled 同时从 busy 状态和投影中排除。此段保留初次失败事实，最终结果见下文。

最终 ChatCompletions 临时端到端验收通过：运行时没有工作目录仍可连接，且不启动 Pi child；同实例 A／B 项目分别创建会话，工具实际执行 `pwd` 并返回对应目录，列表同时出现两者；恢复 A 后工具回到 A，模型选择保持；删除 B 目录后打开明确失败并保留 A；busy 时 create／open 被拒绝，无会话选模型返回受控错误且不修改 selection。正常发送必须有新增上游请求与成功正文，不将 failed 当作 idle 成功。临时脚本不接入 CI，不新增自动化测试。Rust 静态检查、Mac 实际 Swift 编译／bundle 构建和 Windows GNU 类型检查已通过；最终产物通过 clean manifest 关联源码提交，安装版本保持 0.1 beta.1 与 native_verified=false，真实用户验收仍未执行。

## Harness 提供商配置导入（已实现，待用户验收）

用户询问后复核：当前只接入显式 Pi OpenAI 认证来源、SDK 登录／刷新和认证元数据；“读取来源信息”填入的协议与 endpoint 来自适配器声明。`ModelRuntime` 使用 `modelsPath: null`，未读取 Harness 的提供商配置，也未发现／导入其提供商列表、自定义 endpoint、模型列表与能力参数。认证来源接入不能作为完整提供商配置读取已交付的证据，该部分需求仍未完成。

用户进一步纠正：这是完整的“将 Agent Harness 提供商配置导入 Velune”功能，认证不是独立交付项。当前恢复该需求实施，先核对固定 Pi SDK 的提供商／模型定义、覆盖与凭据解析优先级，结合现有网关协议和配置边界制定可预览、可导入的契约。所有调查与实验使用代码或合成配置，不读取真实用户配置／凭据，不调用模型，也不新增自动化测试。

导入采用完整功能契约：Core 提供非秘密预览与原子应用，Mac 使用描述驱动的原生表单与模型选择列表。固定 Pi SDK 合成有效模型配置，候选来自显式模型配置及原认证目录；端点／协议不同的模型分组。认证不复制，普通 key 在原来源解析，OAuth 只接支持的 OpenAI Responses 来源并复用原刷新锁。命令凭据和未满足的环境变量不执行、不回退；预览使用只读认证存储与内存模型目录，不写原配置。

初版自定义 ChatCompletions 导入已在临时合成端到端脚本中通过真实 bundle ABI、Swift 凭据 shim、网关和 Pi：上游收到外部 demo 模型的一次请求，assistant 投影包含 IMPORT_E2E_OK 并进入 idle；重复导入跳过，旧预览 token 被拒绝，命令凭据未产生执行标记。该观察来自执行能力契约扩展前的 bundle，不能替代下面最终版本的验证。

复核内置 OpenAI 模型后发现：其 Responses compat 与 off→none 等映射不能被作为任意配置丢弃，也不能一律标成不支持。初次 advisor 建议将来源模型编码信息关联到 ProviderModelBinding，而非认证 settings；最终归属在下段进一步纠正为 Pi adapter 的投影。模型通用参数仍归全局目录，不读取真实账户或调用模型服务。

用户随后再次提醒 Harness↔AI 服务、AI 服务↔LLM 两条独立边界。经 advisor 复核，先前“通用 execution”命名会把 Pi 级别和 SDK 编码选项误归通用模型能力；调整为 Pi adapter 拥有的 piProjection，Core 装配关联，网关仅消费派生原生请求值域。AI lib 不引用 Pi 类型，Mac 不解释投影，仅保留适配元数据。旧 off→none probe 未导入来源投影，测到 manual identity map 的 off 不构成 SDK 转换能力失败证据；后续验证必须使用真实导入的合成配置。

最终隔离验证使用最新 release Core、sealed bundle 资源与真实 Swift 凭据 shim。临时脚本分别执行 ChatCompletions 和 Responses：providerImport 预览／应用后 routes 与导入前保持一致，随后通过正常配置动作显式设定 route、连接 Pi、新建会话并发送。两条路径各有一个真实 loopback 上游请求与成功 assistant 投影，Responses 上游实际 effort 为 none；重复导入跳过，旧 token 被拒绝，command key 未执行。早期脚本依赖自动设 route 的行为已删除，不能作为最终导入行为的验收。模型映射保留现有参数；Pi 与全局推理级别交集为空时明确不能运行，不自动升级或降级。

Mac 对未知 adapter 元数据的临时编译／往返实验确认 null、boolean 与嵌套映射保持；平台不解析 piProjection。固定 OpenAI SDK 目录的合成 OAuth 元数据预览得到 44 个可导入模型，GPT-5.6 保留 off→none 与 strict 工具参数，原目录只保留原合成 auth.json，无模型缓存或配置写入。上述均未读取真实账户、认证文件或会话，未进行真实登录／模型调用，未增加自动化测试。

最终 Rust workspace 类型检查、fmt、clippy 与 Windows GNU 类型检查通过，Mac 实际 Swift 类型检查、bundle 构建与签名校验通过。产品重新安装仍需 clean manifest 对应最终提交，native_verified 保持 false；真实资源配置、订阅登录和产品验收归用户。

用户补充：原生深色适配不涵盖自绘内容。Mac 当前固定颜色绘制仅出现在双片 logo；消息边框使用系统 separator，未发现自定义阴影、渐变或固定色界面位图。应用内 logo 按 SwiftUI colorScheme 使用素材中 graphite／dark 两套中性色，保持原光学几何；Dock 图标有固定浅色底板，保持原版本。临时 SwiftUI ImageRenderer 按系统浅／深背景渲染 20、32、64 点 logo，人工确认两片与间距可辨；实际 Mac bundle 编译通过。只检查这处自绘内容，不引入完整深色验收流程。

收尾复核撤掉临时固定延时以及未有因果证据的额外 state RPC。Core 保留已有启动 RPC readiness、发送前 set_model matching response 与 turn 事件匹配；重复验收的空 system／user 快照曾被假设为 bootstrap 与 settled 误匹配；实际 RPC 捕获随后否定其作为根因的解释。Core 仍收紧运行边界：只用固定 SDK 的 agent_start 授权 settled，不再用 message_start／turn_start 作为 busy 生命周期起点。人工 ABI 脚本以状态轮询及实际上游请求中的工具结果为依据，ChatCompletions／Responses 原循环和双协议导入均通过，不以 idle 单独判定发送成功。

上述单次成功不能证明启动可靠性。最终重复验收仍观察到普通 ChatCompletions 首个 prompt 在 10 秒后未到达 loopback 上游，快照只有空 system 与 user、idle、无 assistant 错误。正在捕获固定 Pi RPC preflight 之后的异常：该 SDK 在 preflight 已回复 started 后吞掉 prompt promise 的异常，只发 settled；必须定位实际异常并处理，不能用脚本延时或重复通过掩盖。

根执行者接手后仅在临时 SDK／工作 bundle 加入非秘密 RPC 事件捕获，最终观察到 assistant error：网关返回 400、Resource temporarily unavailable (os error 35)，尚未转发上游。临时 std-only TCP 实验直接观察到 accepted socket 的首次读取为 WouldBlock／os error 35，显式 blocking 后正常收到延迟发送的字节，证实本机继承监听器的非阻塞模式，而 read_request 使用阻塞 read_exact；为每条请求显式 set_nonblocking(false)，保留读取／写入超时。临时 SDK 修改已撤回，不交付补丁、诊断日志或固定延时。最终无诊断版运行结果以修复后的 bundle 复核。

修复 accepted socket 后，无诊断 SDK 的实际 Mac bundle 四项端到端全部通过：ChatCompletions／Responses 原 ABI 循环、两种协议的导入→显式路由→真实 loopback dispatch。原循环覆盖工具 cwd、模型切换／恢复、取消与配置重开；导入保持原 routes，Responses effort 实际为 none。源码以本切片提交关联 clean build manifest，安装保持 0.1 beta.1；仍由用户验收真实资源和原生界面。

## 2026-10-05 共享 units 与 UniFFI 迁移

用户授权的拆分与全面 UniFFI 接入已实施，方案、依赖裁剪和隔离证据归 [package-boundaries](../package-boundaries/packet.md)。本地产品仍为同进程 Rust lib＋原生 Mac app；会话历史归 Pi，配置 schema 2 保持。版本继续为 0.1 beta.1，Mac 修改后重建并安装。该架构迁移不替代用户的真实提供商与体验验收。

## 2026-10-05 提供商导入窗口布局修复

用户截图显示导入 sheet 的来源表单有内部滚动条、与标题和操作区边距不一致，空状态占用过多空间。当前 ProviderImportView 给 grouped Form 固定 260 高度，再叠加另一个可滚动预览列表；本轮修正内容布局与滚动归属，保持原生控件和既有导入行为。Mac 修复后构建并安装，使用隔离原生窗口检查空状态和合成预览，不读取用户来源或真实会话。

截图底部另有 runtime model id 配置错误。先核对源码来源，将配置失败与布局缺陷分开；本轮不凭截图推断用户实际配置内容。

用户进一步纠正导入来源选择：仅选择已有运行时实例，复用其配置。UI 不解析 Harness 路径字段；application 在 preview/apply 时从保存的实例解析权威来源，实例或来源变化使旧预览失效。无默认模型／路由可读取配置；空默认网关先保存后配置运行时的既有流程保持，不放宽跨配置引用约束。

调查另发现 Pi 原配置目录与受管网关 catalog 写入目录重叠：已有 models.json 无受管标记时 connect 拒绝覆盖。需要在不破坏原配置、认证来源和会话归属的前提下核对固定 Pi SDK 的 catalog 注入方式；advisor 负责判别路径，开发方不读取真实配置或认证。

固定 Pi 1.0.2 SDK 的隔离 RPC 实验证实：ModelRuntime 可使用独立 catalog，同时保留原 agentDir 和 SessionManager。原合成 models/auth/settings 文件逐字节不变，get_state 仅暴露受管模型；不用整体隔离 PI_HOME 或复制认证。实现由 application 生成 runtime-projections 路径，agent-runtime 的固定 SDK launcher 校验所选安装、复用项目信任逻辑并注入空认证存储。旧受管源文件保留，导入排除能确认归属 Velune 的网关配置，防止递归导入。

Mac 隔离原生窗口已人工截图检查空状态和八组提供商长预览：仅一个内容滚动区，操作区固定，运行时 Picker 使用已配置实例，无重复卡片背景。CUA 服务不可用时改用系统窗口截图，只捕获独立 fixture，不读取真实数据；临时进程已清理。

最终 workspace fmt/check/clippy（all-targets、all-features）、bindings 无 local-runtime 的 Clippy、Windows GNU 类型检查，以及 Mac Swift 构建与签名校验均通过。人工 UniFFI 端到端三条流程覆盖 ChatCompletions、Responses 与订阅能力限制，实际使用新的 SDK 装配并检查原 models/auth/settings 字节未变。导入仍不自动设置路由；完整配置导入后的显式路由与实际发送由独立合成流程复核，不读取真实账户。真实提供商和最终产品体验仍归用户验收。

最后的配置导入合成流程已使用新增 launcher 的提供商注册边界：已有实例预览／导入、显式路由、connect、实际流式回复与 shutdown 全部通过，原三份文件逐字节保留。开发方未调用真实模型。完成后以本切片提交构建 clean bundle 并重新安装，版本保持 0.1 beta.1。

## 2026-10-05 Pi Node 配置与路径发现

用户要求 Pi Node 必填并支持自动发现，询问会话目录语义。Node 由平台发现并验证后填入明确绝对路径，已有选择不覆盖；最低版本由运行时描述提供。共享配置保存边界同步必填。会话存储目录仍是 Pi 历史存储位置，不是任务工作目录；留空采用原 Pi home 下按 cwd 分组的 sessions，改善标签与帮助，不改变会话归属。仅做静态和隔离人工验证，不添加自动化测试。

共享配置与通用 UniFFI 字段增加 executableDiscovery 提示，由平台消费 command/minimumVersion。Mac 隔离探针实际发现本机 mise 管理的 Node 24.15.0，并拒绝合成的 999.0.0 最低版本；探测在后台有时限，失败可手选、已有值和探测期间手动输入保留。固定 Pi SDK 探针确认默认与覆盖会话存储目录。workspace fmt/check/clippy、无 local-runtime Clippy、Windows 类型检查及 Mac typed bindings 构建通过。

最终生成 Python UniFFI 绑定的合成端到端会话循环通过，覆盖新配置字段往返、流式发送、模型恢复、取消及配置重开。Mac 重新构建安装，版本仍为 0.1 beta.1；未添加自动化测试，未使用真实提供商。
