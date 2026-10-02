# 多 Harness 路由与协作架构

状态：**核心产品设计及 Rust／SQLite／原生 UI 方向已认可；细化方案待验证**。更新于 2026-10-02 UTC。首批 Codex、Claude Code、Pi 已由用户固定；本页负责收敛实现方案，不重新评选 Harness，也不表示产品已经实现。需求权威见 [PRD](../prd/index.md)，来源和版本见[研究证据](../../tasks/harness-routing-feasibility/evidence.md)。

## 1. 已认可方向与收敛方案

做一个**本地优先的任务控制服务**，掌握任务树、会话身份、路由策略、权限、消息和恢复；下挂三个原生 Harness Adapter。LLM 路由采用“统一决策、分协议执行”：Codex 接 Responses 路径，Claude Code 接 Messages 路径，Pi 优先接原生逐请求路由钩子。订阅身份保持独立，使用各供应商允许的原生或正式集成路径；API 资源可由协议网关选择上游。

跨 Harness 协作由**协调逻辑＋原生会话接入**完成：发现有能力且获准的目标、传递输入、在目标的安全边界注入、委派工作、回传结果，并处理并发／取消／失败。任务树、消息、权限和恢复状态的持久化为这些行为提供可靠性，保存它们本身不构成协作。MCP 暴露协作工具、ACP 作为可替换接入方式已获用户认可；ACPHub 不设为前置依赖。具体工作链见第 7 节。

技术基线更新为 **Rust 主核心＋SQLite＋原生客户端**：Apple 用 Swift／UIKit 方向，Android 用 Kotlin／Jetpack Compose。此前 TypeScript／Node.js 主核心与 Web 前端建议已被用户替换；不再作为备选默认。用户接受多套原生 UI 的实验成本，设计重点改为共享领域契约、生命周期和可验证的桥接边界。具体模块划分、薄 SDK 桥及 Mac Catalyst 等仍为下述技术推荐，见[复核来源](../sources.md#s10)。

## 2. 必须守住的契约

1. **路由是 LLM 资源选择**。选机器、选 Harness、创建协作子任务由其他控制逻辑完成；不能把“转交给另一个 agent”冒充同一请求的模型切换。
2. **三种 Harness 都纳入路由控制**，但控制粒度必须如实显示：逐请求、逐轮、下一会话／续作。原生订阅未开放的控制点通过隔离运行绑定和安全续作处理，不能悄悄声称逐请求万能切换。
3. **账户不是令牌桶的别名**。身份、计划权益、组织、预算、限流、数据目的地、历史上下文都必须可区分；不合并两个账户额度，不通过换号绕过供应商限制。
4. **兼容性先于便宜和空闲**。接口形似 OpenAI／Anthropic 不代表工具、推理状态、图像、缓存、流和续接兼容。
5. **任务持续，运行可替换**。进程消失不删除任务；逻辑会话身份可保持，底下原生绑定按 Segment 留下 lineage；跨 Harness／账户接续不伪装成原生会话无损恢复。
6. **协作消息不能授予权限**。对方会话的建议、产物和指令是带出处的输入，不继承其身份、预算或工具权限。
7. 当前只实现无凭据模拟核心；真实登录、凭据配置、模型调用、原生 Harness 与 UI 设备安装、部署未执行。

## 3. 组件与控制／数据路径

### 控制面

原生 UI → Rust Client 接入层 → 本地 IPC → Rust Host Service → Session Supervisor → Harness Adapter → 原生进程。后续远程客户端使用同一命令／事件契约的认证网络传输。

- **Task Service**：目标、父子关系、依赖、验收、状态、工作区和产物归属
- **Session Supervisor**：启动、连接、恢复、单写控制租约、取消、权限转交、断线核对
- **Routing Policy**：根据模型能力、可用账户、预算、状态和明确偏好产生 RouteDecision
- **Collaboration Coordinator**：目标发现与能力／权限匹配、输入与结果路由、委派关系、交付期限及失败／冲突协调
- **Collaboration Broker**：消息接纳、队列、回执和 inbox／outbox；与 Supervisor 协作完成实际会话注入，不代替 Coordinator 的行为决策
- **Policy Service**：项目范围、数据出站、工具、账户、花费与委派许可；只允许缩小授权的子任务
- **Adapter Registry**：记录具体 Harness／协议／版本的能力，不把最低公分母当作完整能力

### Rust 与原生客户端的具体边界

以下是收敛推荐，不是已经实现的库／进程：

- **Rust Domain**：Task／Session／Segment、路由与权限策略、协作状态转换及契约，不依赖 UIKit／Compose／进程启动
- **Rust Host Service**：桌面／服务器上的权威服务，组合 Domain、SQLite、Supervisor、Gateway、Coordinator、Broker 与进程适配；关闭 UI 不等于结束任务。首版本机一个服务，不先拆微服务
- **Rust Client 层**：给原生 UI 提供版本化命令、快照／游标事件、取消、断线重连与本地只读投影／待发草稿；不复制第二个权威 Supervisor 或路由账本到每个 UI
- **Swift／Kotlin UI**：导航、任务树、流式展示、系统交互与权限呈现；不直接改 Host 的 SQLite、不复制路由规则。审批响应携带原始 action／epoch，离线或过期审批不能重连后盲发

Host 是权威 SQLite 的单一服务写入者；每台客户端的缓存是独立数据库，不能用文件同步把它变成共享主库。WAL 不是跨机器同步协议，网络只传命令／事件／产物，见[平台与存储证据](../../tasks/harness-routing-feasibility/evidence.md#原生客户端与-rust-边界)。

**FFI 与 IPC 分开**：Swift／Kotlin 调用 Rust Client 库才需要 FFI；Client 与独立 Host 通信用 IPC，不把整个服务嵌在会被 UI 关闭或系统回收的进程里。建议以 UniFFI 做首个绑定原型，验证 Swift／Kotlin 的异步、取消、错误、句柄释放、主线程回调与事件背压；它是候选工具，用户尚未指定。失败可用窄 C ABI／平台桥替换，不改变产品契约，不借机改回 Web UI。

**Rust 主核心不要求重写上游 Harness SDK**。Codex 直接接 app-server；Pi 以 RPC 子进程接入，逐请求 virtual-model hook 由最薄的 Pi 扩展调用 Rust 路由服务；Claude 优先保留官方 SDK 的薄桥进程以正确处理其生命周期。桥只做编解码、关联 ID、流和原生权限回调，领域状态、路由、预算、协作与持久化留在 Rust。Node／TypeScript 若存在仅为原生 Harness／SDK 依赖与桥，不重新成为产品主核心；不得因改 Rust 而把 Claude 私有控制 envelope 当稳定公开协议重写。

### Apple 与 Android 宿主

- **Apple 推荐落点**：iOS／iPadOS 用 Swift＋UIKit；macOS 先评估显式 Mac Catalyst target，共享 UIKit UI，同时做 Mac 的菜单、窗口、键盘和文件交互适配。Mac Catalyst 不是把 UIKit 直接链接成 AppKit 应用；“Apple-universal”也不推定一个 UIKit binary 覆盖所有 Apple OS
- **尚需收敛的 Apple 选择**：Mac Catalyst 为本轮推荐，未视作用户已批准。若本地服务启动、文件访问或桌面交互受其边界阻碍，用独立 macOS helper／服务接口解决；仍不满足时再提出 Swift＋AppKit 的 Mac 专用 UI，不能悄悄切框架。watchOS／tvOS／visionOS 不在本轮默认承诺内，确需覆盖时按具体平台设计
- **Android**：Kotlin＋Jetpack Compose 原生 UI，通过同一 Rust Client 契约接入；系统权限、Keystore／通知和应用生命周期由平台层承担
- **移动端角色**：首期作为连接既有 Host 的原生客户端，缓存、查看、输入和审批；不把 iOS／Android 后台能力当常驻任意 coding Harness 的保证。系统允许的长任务机制有条件和配额，不能自动等同于桌面守护进程。手机 UI 被挂起，远端任务继续；离线输入是待发送，收到 Host admission 才显示已启动

本地 Host 安装、启动／重启、IPC 认证、签名／公证／商店分发和 sandbox 权限仍需原型。移动端不因此提前变成首要目标：先完成桌面本地三 Harness 核心，再接移动客户端所需的认证远程传输。具体 UI 与 FFI 用独立小原型尽早验证，不用 Web 过渡实现替代用户选择。

### 模型数据面

- Codex → Responses Gateway → 获准的 Responses 兼容上游；原生订阅或正式 ChatGPT 计划接入由对应认证路径持有身份
- Claude Code → Messages Gateway → 获准的 Claude／兼容实验上游；原生订阅默认由原版 Claude Code 直接持有登录
- Pi → virtual-model 路由回调 → Routing Policy → Pi provider；只有上游协议或统一计量需要时经过 Gateway

Gateway 负责协议保持、流传递、资源预留和尝试记录，不执行 shell／文件工具，不创建任务，也不决定切换 Harness。工具仍在 Harness 及其受限 Runner 中执行。

默认原生订阅模式下，控制面不接收订阅令牌。API key 由用户后续授权配置到对应 Runner 的密钥设施，数据库只存 opaque credential reference。正式 OAuth 集成若要求本产品持有令牌，必须有独立的注册、授权、隔离和生命周期设计；本轮不配置。

**“接管”不等于拦截进程全部网络**：只承诺覆盖经验证的模型请求通道，启动探测、遥测、搜索、安全检查、插件网络另列清单。上线前用无真实内容的流量测试验证主请求、子 agent、压缩、标题等是否全部归属正确；未知出站不会被计成已受控。

## 4. 三个 Harness 的落点

| Harness | 原生控制接口与路由落点 | 订阅、兼容与限制 | 推荐执行 |
| --- | --- | --- | --- |
| Codex | `app-server` 管线程、轮次、事件、审批；自定义 provider／Responses gateway 接数据面 | ChatGPT 原生登录与 API key 分开；当前官方 SIWC 面向符合条件的开源／本地应用；付费／远程托管走合作申请。原生 app-server 旧认证不能作为商业／托管授权；不能直接拿 CLI 缓存 token 当任意 API key | 先用官方 app-server，逐轮选模型；API lane 用 Responses gateway。本地／开源订阅 lane 用原生隔离绑定；正式 SIWC 用公开 Responses endpoint，按其限制实现逐请求路由，托管 eligibility 独立验证 |
| Claude Code | 原版 CLI／Agent SDK 管会话；`ANTHROPIC_BASE_URL` 接 Messages gateway | 当前文档允许平台运行原版 binary，用户走官方登录；禁止第三方代收或中转用户 Claude 凭据。仅 BASE_URL 的 OAuth pass-through 有技术文档，但不等于允许本产品聚合／改派订阅身份。非 Claude 模型不受 Anthropic 官方支持 | API lane 做网关，先覆盖 Claude 多供应商；订阅 lane 保留原生身份。非 Claude 协议桥作为明确标记的工程实验，不修改 binary、伪造身份或默默剥离功能 |
| Pi | RPC／SDK 管会话，当前 virtual models 提供逐请求 `route`，可见 continuation／retry 及分支状态 | provider／OAuth 的代码支持不代表供应商授权；当前有原生 MCP。没有内建沙箱或逐工具审批，须补执行隔离和工具策略 | 把路由策略接入 virtual model，而非多套互相覆盖的代理；provider 适配由 Pi 处理，产品控制资源 eligibility、预算、粘性和审计 |

正式 SIWC 不是通用 Responses 全功能别名：当前要求公开 `/v1/responses`、流式、`store:false`、完整所需历史，不使用 `previous_response_id`；可用模型按当前账户查询，不能把 app-server 缓存列表当 entitlement。注册／workspace／host ID 分开，refresh 串行；正式 app-server 路径需 token 更新后重启并恢复，不能假定环境变量热更新。见下列证据。

具体证据：[Codex](../../tasks/harness-routing-feasibility/evidence.md#codex)、[Claude Code](../../tasks/harness-routing-feasibility/evidence.md#claude-code)、[Pi](../../tasks/harness-routing-feasibility/evidence.md#pi)。

### 不把最难部分藏在“兼容”二字里

每个 `CompatibilityProfile` 绑定 Harness 版本、客户端协议、上游 API／模型版本、工具 schema／并行调用、图像、推理／签名块、上下文大小、压缩、结构化输出、缓存、流事件与错误格式。状态为 `untested / verified / degraded / blocked`，附测试日期。未知字段在同协议直通时保持，跨协议桥接遇到未知语义拒绝或显式降级，不能猜着删。

- Codex Responses 与 Chat Completions 不作为可任意互换的同一协议；服务器侧 response/session ID 不跨账户或供应商复用
- Claude 网关保留完整流序列、必要 headers／body 和原始错误语义；不能仅把文本 delta 拼起来。未知 model alias 的能力假设需校准，不能把所有上游伪装成同一种 Claude
- Pi 路由回调可逐请求选模，不代表带签名的推理、工具结果与缓存可随意跨模型搬移；同样执行 CompatibilityProfile
- 初版“保真直通”优先，跨协议转换逐项开通；失败关闭的是具体资源组合，不删除对应 Harness

## 5. 多订阅、多供应商与自动路由

### 资源模型

`Account`：用户／组织与供应商身份；`Entitlement`：订阅、API 或供应商批准的其他权益；`CredentialBinding`：原生登录或密钥引用及其作用域；`ResourceSlot`：某身份下某模型／endpoint／认证方式的可调用资源。

每个 Slot 记录：允许 Harness／协议、能力档案、数据目的地、并发上限、额度信号及其时间和可信度、reset/cooldown、费用档案、可用状态。订阅“剩余精确 token”不可假定可见；缺数据就展示未知，不能伪造余额。相同账户多个 worker 共享该账户的限额账本，避免本地并发超卖。

需要区分两种身份绑定：原生订阅 Segment 固定 NativeAuthBinding（账户／workspace）；网关 Segment 固定的是 GatewayBinding 与获准资源集合，物理上游账户按每个 Attempt 记账。后者只有在无账户绑定状态、上下文流转获准且兼容性通过时，才可在同一 Segment 逐请求换上游账户；这不是修改原生登录。Pi 的 provider／auth store 也按资源实例隔离，不能靠覆盖一个运行中 provider 的全局凭据切号；无法独立绑定的组合走新 Segment。

原生订阅 worker 按 `owner + account + harness + config-root` 隔离；不共享 HOME、原生历史目录或凭据缓存。例如 Claude 的 CLAUDE_CONFIG_DIR 已有目录级隔离文档，但无 API key 的 Console 登录存在例外；真实隔离仍必须做合成账户夹具与后续获准测试。正式产品登录流程仍由供应商拥有，不能仿造登录框收密码。

### 用户的两个订阅怎样落到日常工作

两个 ChatGPT 账户建成两个独立 Account／workspace registration，显示各自允许模型、观测用量、更新时间和恢复时间；即使邮箱一样也不合并身份。用户为项目选择可用账户范围、资源偏好和是否允许 API 花费。正常开始一轮时，Router 在获准集合里选择资源，不用用户每次手动改环境变量。

遇到局部限流先遵守其 scope 和等待时间；若存在规则允许、已授权且状态兼容的替代资源，才自动切换。需要换原生订阅绑定时，将当前已完成产物和确认状态落 checkpoint，在同一逻辑 Session 创建下一 Segment，并把切换原因显示出来。源 turn 状态未知或工具可能仍运行时先核对，不抢跑。全部资源不能调用时保留任务、排队或请求决定，不丢工作，也不默认付费。

### 决策算法

每次进入可控模型边界时执行如下固定顺序，首版不用另一个 LLM 猜路由：

1. 读取 `RouteIntent`：任务／会话／请求类别、质量档位（fast／balanced／deep）、已承诺语义、上下文估计、最大输出、allowed resources、预算及数据出站及跨账户／workspace 上下文流转许可
2. 硬过滤：同用户和授权范围、认证可用、供应商规则允许、协议与能力通过、context 可容纳、未禁用／未熔断、在预算和并发内。未知余额并非无限；未知资格不调用
3. 固定需要粘性的 lineage：未完成工具往返、provider 状态引用、签名推理、会话绑定身份。适配器声明安全切换点；不满足就只在原 slot 排队或启动显式接续
4. 在合格集合按确定性次序排序：明确用户偏好 → 满足质量 → 订阅／API 的获准优先级 → 成本估计与剩余预算 → 限流／排队 → 延迟与近期可靠性。平分时保持原模型，避免振荡；设置切换收益阈值和冷却
5. 每个 Attempt 在事务内独立预留并发与保守费用，写入 decision＋attempt，再发送。耗尽时排队并给原因；付费 fallback 未授权时默认禁止，不悄悄消耗 API 余额
6. 结束时结算已报告 usage；价格版本、cache 成本、未知费用单独记。订阅账单与推定资源消耗分开。旧 Attempt 费用未知时继续占用保守额度，fallback 另做 admission；无法逐次观测的 SDK 重试按声明上限预留。更新成功率／延迟／限额，崩溃后先核对再释放不确定费用

执行结果是 `dispatch / wait / handoff_required / needs_decision`：dispatch 只进入当前 Segment 的兼容资源；wait 有等待时间或唤醒信号；已获准的 handoff 由 Supervisor 自动完成 checkpoint、旧 Run 停止／失去执行权确认、Segment 建立与续作，不要求用户逐次手工操作。只有新增费用、数据目的地、授权或无法核对副作用时才 needs_decision。源端状态未知时不能让两个 Segment 同时执行工具。

RouteDecision 至少有 `requestId, segmentId, runEpoch, policyRevision, capabilityRevision, accountId, slotId, model, reason, effectiveBoundary`；各 Attempt 另带 reservation ID，界面能回答“为什么用了这个账户／模型”。网关不能信任 agent 自填的 accountId：身份绑定由本地受控连接或短期 session-bound ticket 决定。

评估用固定的合成／获准任务集，按任务成功率、测试通过率、修订次数、首 token／总延迟、真实 API 花费、订阅限流等待和恢复成功率比较；保留固定模型基线。质量档位先由实测规则映射，未经用户许可不复制真实代码到额外模型做在线 shadow。反馈只调整后续策略版本，不改变正在执行的决策。

### 失败处理必须分层

| 失败／边界 | 动作 |
| --- | --- |
| 确认未发送，或只做模型生成且无上游变更型工具，或上游有可核验幂等／恢复的故障 | 在统一 deadline 和次数上限内退避重试；仅在许可、能力和上下文兼容、尚未向 Harness 交付有效事件时透明换资源。首版网关禁用上游变更型工具；已接收但结果未知可能计费，每次 attempt 单独预留 |
| 429 临时限流 | 尊重 retry-after 与 scope；该账户／模型冷却，选择获准替代或排队。不按“额度没了就换号”规避计划限制 |
| 401、权限／条款拒绝、用户禁用、预算耗尽 | 停止该路径，显式重新认证／决策；不把政策拒绝当短暂故障无限绕路 |
| 已输出文本、推理或工具调用后断流 | 不把另一模型续流伪装成同一回答；标记 partial／unknown，保留可见进度，交原生恢复或在安全点发起新 attempt／续作 |
| 工具可能已产生副作用 | 先核对工具状态与产物，再决定继续；不重跑整个 turn。外部系统不支持幂等时不能保证 exactly-once |
| 必须换原生绑定身份、不兼容协议／上下文或 Harness | 保留原会话，在原逻辑 Session 下创建有 lineage 的新 SessionSegment（切换 Harness 另由任务策略授权），移交目标、已确认事实、变更／测试产物和待办；不移交登录、隐藏思考、未经允许的上下文 |

“Harness 没收到事件”不证明上游没做动作。有上游工具副作用而无法核对的请求进入 unknown／reconciling，不透明重试。

Gateway、Harness 和底层 SDK 可能各自重试。适配器必须声明 retry owner／预算；能关闭重复重试的关闭，不能关闭的保留原生错误与 retry headers，由上层约束总时长和次数。绝不堆出“3×3×3”隐形尝试。暂停／取消不宣称撤销已发给上游的请求或已经发生的工具副作用。

## 6. 持久化任务、会话与生命周期

### 最小领域对象

- `Project`：repo／工作区根、默认策略、数据范围
- `Task`：目标、唯一 parent（用于树）、依赖边（DAG，拒绝环）、状态、验收、owner、产物引用。一个任务可以有多个会话
- `Session`：稳定的产品 UUID、Task、用途、协作身份、权限、当前 Segment；用户界面和消息地址在获准续作后不变。另开分支／独立协作者才创建新逻辑 Session
- `SessionSegment`：不可变的 Harness、nativeSessionId、执行身份绑定（NativeAuthBinding 或 GatewayBinding）、能力版本、工作区与来源 checkpoint。换原生身份或不能兼容恢复时新建 Segment；合格的无状态网关上游切换不必新建 Segment，实际账户／Slot 归属 Attempt。旧 Segment 保留，不跨账户复用原生状态标识
- `Run`：某 Segment 一次附着到特定 Runner 的执行生命周期；包含进程 epoch、控制租约、连接状态、active turn、最后确认事件。进程重启创建新 Run
- `Turn / ModelRequest / Attempt`：用户或协作消息触发的一轮、其中的模型请求、重试／fallback；避免把一次用户消息计成一次模型调用
- `PermissionRequest`：请求内容 hash、目标、范围、turn／toolCall、有效期与决定；默认不继承到不同动作
- `Message / Artifact / RouteDecision / BudgetReservation`：分别负责可靠通信、可核对产物、路由解释和预算约束

### 状态与所有权

Task：`planned → active ↔ blocked → done / failed / cancelled`。`done` 需要验收证据，Harness 的 stop reason 不是任务完成证明。

Session：`created → ready → running → waiting_permission / waiting_message / idle → closed`；断连进入 `unknown/reconciling`，不直接判 failed。Segment 另有 active／superseded／unrecoverable，Run 的 completed／failed 与 Session 可恢复性分别记录。

同一 Session 同时只有一个 active Segment 和写控制者：带 fencing epoch 的 lease。旁观者可订阅，不能并发发 prompt／取消／审批。接管先让旧控制端失效，再核对原生 active turn；原生接口无法防双写时禁止强抢，要求原控制端释放。lease 过期不是杀掉旧进程的证据。

### 恢复与存储

SQLite 同一事务写 command、状态变更和 outbox；事件追加记录，列表／树用可重建投影。原生会话存档由 Harness 保持，产品保存定位与已确认事件游标；两者不是一套可无损互换的 transcript。完整原始请求默认不记录，token／authorization headers 永不进日志。

提交原生 prompt 前持久化 command／injection ID 和 injecting；只有可核验的原生接受证据才能写 delivered。DB 与进程接口没有共同事务：接受后未落库崩溃记 delivery_unknown，先按原生 turn／事件核对；不能核对且上游无幂等提交能力时禁止自动重投。Broker 去重不等于 Harness exactly-once。

服务重启：恢复待投递消息和未决审批 → 将旧 Run 标记 reconciling → 查询适配器原生状态／存档 → 对齐 active turn、产物与最后游标 → 恢复或提示人工核对。不完整事件保留 gap 标记；不根据缺日志推断“没执行”。任何无法去重的 prompt 都不能因重启而自动重发。

Artifact 保存内容 hash、生成者、基线 commit、验证结果与权限。不同写任务默认独立 worktree；合并产物通过集成步骤完成，冲突暴露给任务负责人。共享目录“约定不要同时写”不作为并发控制。worktree 隔离不等同沙箱；文件访问和 shell 仍需要 OS 权限边界。

## 7. 跨 Harness 会话协作协议

### 协作行为与支撑设施分责

这里的“产品负责”有具体操作含义，不能从“数据存在本产品”推导出来：

1. A（例如 Codex）通过 MCP 请求查找某任务范围的审阅者；Coordinator 按 session registry、能力、任务范围和授权返回目标 B（例如 Claude Code）。agent 可以提出分工，产品校验而不假装替代所有任务推理
2. A 发出带 message／delegation ID、目标、预期产物和期限的请求；Coordinator 建立关联与目标归属，Broker 持久接纳。此时只有 accepted，不能报告 B 已看见
3. Supervisor 检查 B 的 active Segment／Run 和控制租约，经 Claude 原生 adapter 在 idle 时启动输入、busy 时排队或在能力及授权允许时 steer；消息明确标记为会话来源，不冒充用户授权。注入与确认缺口沿用 delivery_unknown 规则
4. B 的问题／进度／结果经协作工具或可关联的原生事件返回；Coordinator 用 replyTo／delegation ID 路由到 A，Supervisor 同样负责把结果实际送入 A 的会话。只写数据库、不唤醒或供下次安全轮次消费，不算交流闭环
5. 委派任务由目标明确接纳／拒绝；Coordinator 管依赖、owner、期限、取消传播和结果收集，区分 claimed completion 与验收通过。副作用未知时不自动改派另一会话重做；并发改动依工作区／产物契约集成
6. Policy 在发现、出站、注入、委派与产物访问逐步限制权限；恢复设施重建未决操作／消息／租约。它们保障过程可审计、可恢复，不能替代前五步

因此分三层：**Coordinator 决定可执行的协作动作与业务关系；Broker＋Supervisor＋Adapter 让消息和控制真正抵达；SQLite／事件／权限记录支撑可靠性**。MCP／ACP 是接入契约，单有它们或单有存储都不等于协作已实现。

### 用户可以区分的四种能力

1. **发现**：在授权任务／项目内列出会话用途、Harness、状态、公开摘要；不泄露别的项目存在性
2. **观察**：读取获准的事件、进度、产物与最终结论；不默认开放完整 transcript、凭据或隐藏思考
3. **交流**：给明确会话发送问题／建议／结果，支持关联回复与撤回尚未消费的投递；已消费内容不可假装收回
4. **协作**：创建或领取子任务、声明交付物与依赖、返回证据；需要单独 delegate／write 权限，不能因为可以聊天就任意启动高权限工作

### Broker 契约

对 Harness 暴露 `sessions.list, sessions.read, messages.send, messages.receive/ack, tasks.delegate, tasks.report, artifacts.read`。Codex／Claude Code／当前 Pi 可通过 MCP 工具接入；Pi 也可用扩展自定义工具直连相同 broker。控制端取消和审批不作为普通 peer 工具默认暴露。

消息 envelope：`messageId, taskId, fromSessionId, toSessionId, kind, replyTo, causationId, scope, artifactRefs, createdAt, expiresAt, idempotencyKey`。`fromSessionId` 来自 broker 的已认证进程绑定，忽略 agent 声称的身份。内容带来源与信任等级，作为协作输入送入，不升级为 system 指令。

持久 outbox/inbox 提供 **Broker 层 at-least-once 投递＋消费去重**；跨原生 prompt 注入的 unknown 边界按上一节处理，不保证端到端 exactly-once。消息地址是稳定 Session，由 Supervisor 解析当前 Segment；Attempt、事件、审批和审计记实际 Segment＋Run。回执区分 `accepted / delivered / consumed / expired / rejected`；“对方回答”是单独的业务结果。支持游标 replay、单目标顺序、超时与死信；不承诺全局全序或任意工具 exactly-once。

忙碌会话默认排入下一安全消息边界。仅当适配器经验证支持 steer 且用户允许时，才 mid-turn 注入；否则显示 queued，不能把排队当作已读。避免互等：委派必须有父任务、期望结果和 deadline，禁止依赖环；消息回合／深度与 token 预算有上限，同一内容不会自动广播到所有会话。

### 权限执行

读取／发现／发消息／委派／读产物分别授权，默认仅同一任务树并可进一步缩小。目标会话的写文件、网络、付费和外部发送仍由其自身策略决定。审批带 action hash 和租约 epoch，过期／重启后不复用到另一个动作。

Codex 原生审批事件、Claude 的 hooks／permission 机制、Pi 的扩展 tool_call 钩子分别接 Policy Service；不存在一个可以假定覆盖全部工具的统一回调。特别是 Pi 缺少内建沙箱，Claude 的 canUseTool 可能不经过已自动批准的工具，必须配 OS 隔离、可信 hook 与 fail-closed 策略。MCP／插件也在隔离内，不能因为工具来自插件就获得额外权限。

可信边界包括 Supervisor、原生 Harness、获准 adapters／hooks／extensions 和凭据设施；agent 可操作的工作区与工具子进程不属于它。数据库、策略、hook 代码、凭据目录不在 agent 工具的可读写范围；子进程清理环境并收窄 OS 权限，不能继承控制 socket／完整凭据。Gateway／Broker 票据绑定 Segment＋Run epoch＋操作范围；推理票据无会话控制权。Pi 内置文件工具与同进程扩展还须做路径／配置策略，单改 HOME 不够；项目自带扩展／MCP 不能自动进入可信边界。若某平台无法保证这些条件，该组合不能标为安全受管，只能明确以用户信任的执行模式提供，且不承诺防恶意代码。

### ACP、MCP、ACPHub 的位置

- **ACP**：客户端与 agent 的会话、prompt、流事件、权限、取消，以及按能力声明的 list／load／resume 等。适合接编辑器或已有 ACP adapter；不是供应商 LLM API，也不自动定义产品任务树、跨服务可靠投递和单写接管
- **MCP**：把 broker 的协作操作暴露为工具／资源；不直接承担 Harness 生命周期和 agent 到 agent 的可靠业务协议
- **原生 adapters**：Codex app-server、Claude CLI／SDK、Pi RPC／SDK 保留各自真实控制能力；统一一个接口并保留扩展，而非强行把三者削成相同文本输入输出
- **ACPHub**：本轮未取得 `lexoliu/acphub` 可核验源码／commit，不能断言其已有全局发现、可靠恢复、安全控制或具体依赖。获得正确可读来源后只评估复用 transport／registry／adapter 的价值；替换它不改变 Task／Session／Message 契约

## 8. Adapter 必须声明的能力

最小接口：`probe, start, attach, list, read, prompt, cancel, resume, close, respondPermission, getUsage, setModel, getCheckpoint`。不是每个方法每版本都可用；返回 typed unsupported，不假装成功。

能力包括：路由粒度、模型／provider 切换点、原生恢复／fork、运行中 steer、历史 replay、外部存量会话发现、无损 usage、审批覆盖范围、工具幂等信息、能否查 active turn。服务启动时 probe 并记录版本，升级前跑兼容测试。

终止事件按上游语义归一：Pi 的 `agent_end` 不是队列／重试已经耗尽，等待 `agent_settled`；其 Stop 要先 `clear_queue` 再 `abort` 并确认稳定。Claude 的一次 result 不保证后台 agent／workflow 都结束，要继续对齐会话状态。Codex turn 完成也不自动完成产品 Task。关闭 UI、取消 turn、清空队列、终止进程是不同操作。

首版完整管理**本产品启动或明确接入的会话**。外部 CLI 会话只有在官方接口确实允许发现／读／控制且得到授权时才纳入；扫描日志只能标为 observed，不能声称可接管任意正在运行的终端。不能通过杀进程重启来伪造 attach。

## 9. 首版闭环、验证与后续分布式

### 实施顺序建议

- **V0 契约原型**：Rust Host／薄桥／Swift 与 Kotlin 接口及 Apple 桌面落点先做边界验证；三个 Harness 并行验证原生控制、请求入口、权限／流／恢复；先用假上游和合成状态做故障注入。任何失败都落在具体适配问题，不重开 Harness 选择
- **V1 本地骨架**：Rust Host、SQLite、单控制租约、三个 adapter、账户隔离绑定、手动模型／资源选择；原生桌面任务树能管理三者，不建立 Web UI
- **V2 核心闭环**：统一 Routing Policy＋各自数据路径、预算／使用记录、broker 协作工具、子任务／产物、允许范围内自动资源选择与明确的续作 handoff
- **V3 产品验收**：断流、重启、重复投递、账户限流、审批过期、worktree 冲突、错误兼容、计费未知等故障不丢任务、不越权、不静默重放工具

V1 是基础检查点，**V2＋V3 才构成首个可用版本**。不能只接一个 Harness，或只做任务树 UI 就宣布核心目标完成。订阅路径和 API 路径分别验收；尚未获准的资源组合显示 unavailable／experimental，不影响其他已验证路径。

推荐端到端验收：一个父任务内 Codex 执行、Claude Code 审阅、Pi 完成独立检查，各自有隔离工作区和预算；能发现、问答、交付产物。模拟主资源 429、半途断流、控制服务重启和重复消息，证明不乱换身份、不重复副作用、任务树和路由账目能恢复。这里 Harness 分工由任务计划固定，模型资源仍由 Router 选择。

### 待验证而不假定已解决

具体输入、观察证据和通过标准见 [Task Packet](../../tasks/harness-routing-feasibility/packet.md#原型验证队列)。真实账户／费用测试须后续授权；本地模拟结果见 [开发与复验](../development.md)，不作为原生接入证据。

### 分布式路线

核心通过后再拆出 Runner Agent：控制服务持有全局任务、策略和消息；设备只保留本地执行、原生凭据、受控工作区与可恢复 spool。使用显式设备注册、认证加密连接、重连游标、lease epoch 和版本能力协商。控制指令由 session owner 执行，离线期间不双写；断网既不立刻判任务失败，也不自动在别处重跑。

模型网关优先随凭据／Runner 部署，按允许的数据驻留位置路由；不为远控便利集中复制所有订阅凭据。产物按 hash 同步，运行中工作区不当普通网盘双向同步。迁移先停到 checkpoint，确认源端失去控制，再在目标建立新 Run；无法迁移的原生上下文用可见续作。设备调度独立于 LLM 路由。

## 10. 复核结果与剩余决策

[11:28 用户复核](../sources.md#s10)已认可核心产品设计，以下不再作为未决方向：

1. 三 Harness＋统一路由控制、分协议执行；账户边界与安全续作、稳定逻辑会话保持
2. MCP 暴露协作工具、ACP 可替换接入；明确以实际发现、注入、委派、回传与失败协调解释跨 Harness 协作，持久化只是支撑
3. Rust 主核心、SQLite、Apple Swift／UIKit 方向与 Android Kotlin／Jetpack Compose 原生 UI；替换 TypeScript 主核心和 Web UI，接受实验成本
4. 先核心闭环，后分布式扩展；底层细节仍由设计方收敛，不让用户重做方案选择

具体 Mac Catalyst／AppKit 落点、Apple 平台范围、UniFFI／IPC、官方 SDK 薄桥、服务安装与安全边界仍为推荐／原型项；已有方向认可不自动批准所有细节。官方集成资格、具体账户权益、ACPHub 来源、真实路由／恢复效果仍需验证。已有无凭据核心原型；不把模拟成功当作登录、付费或部署授权。

## 11. 当前核心切片与原生体验接线

实现入口：[src/lib.rs](../../src/lib.rs)、[schema.sql](../../src/schema.sql)、[adapter.rs](../../src/adapter.rs)、[routing.rs](../../src/routing.rs)。核心只覆盖固定深度委派、稳定 Session、单 Segment 绑定、每次重启的新 Run、独立 Attempt、事务 outbox 与合成结果验收；不是前述完整架构的实现。

单宿主通过 SQLite exclusive connection 拒绝第二个 Host；不是跨设备 lease／native fencing。未知提交阻塞该 Segment 后续投递，无自动 handoff 或人工强制解锁接口。路由只允许明确获准的模拟资源，无 API／订阅 lane。MCP 工具服务、ACP transport、真实 gateways／SDK 桥、费用预留与完整安全隔离尚未实现。

Mac mini 首个体验切片已获准使用 Swift＋AppKit 薄壳。已编写原生 UI 与独立 Rust Host 接线源码；UI 通过随包 `rpc` 子进程访问同用户 Unix socket v1，操作同一核心及 DB，显示模拟模式、任务、会话、attempt 与协作事件。UI 退出不终止 Host；显式停止取消待投递消息、保留不确定状态。无 TCP／LAN、系统常驻安装或 UI 自有任务模拟。Cloud 已验证核心与 IPC；Mac 编译／安装／用户体验待父会话设备验收。
交付版本与反馈循环见 [开发说明](../development.md#mac-mini-原生体验交付契约)。
