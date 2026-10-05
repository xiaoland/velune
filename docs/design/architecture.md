# 多 Harness 路由与协作架构

状态：**核心产品设计及 Rust／SQLite／原生 UI 方向已认可；细化方案待验证**。更新于 2026-10-02 UTC。首批 Codex、Claude Code、Pi 已由用户固定；本页负责收敛实现方案，不重新评选 Harness，也不表示产品已经实现。需求权威见 [PRD](../prd/index.md)，来源和版本见[研究证据](../../tasks/harness-routing-feasibility/evidence.md)。

2026-10-03 范围修订：独立 AI service 的契约、provider 和配置归属以 [AI service 设计](ai-service.md) 为准。本页旧 Account／Slot 和 mock Router 方案不自动成为新 AI lib 的结构；旧 UI／Host 原型保留，本步不迁移。

2026-10-04 首循环边界：用户明确要求实际拆分 `core`（AI 服务、Harness 适配器）与 `app`（Mac 等平台）。当前 Pi 切片由 Pi 拥有会话历史和持久化，Velune 只投影会话列表、消息和运行状态，不另建真实会话数据库。Mac 提供 Chatbot 界面、会话列表与通用资源配置，不能硬编码资源示例；真实验收由用户执行。此切片不将模拟核心的逻辑 Session 契约强加给 Pi 历史，也不声称完整统一自动路由已完成。执行状态见 [首循环任务](../../tasks/pi-mac-first-loop/packet.md)。

首循环的当前装配边界（2026-10-04 用户修订，2026-10-05 全面采用 UniFFI）：共享能力是 Rust libs，由 Mac／Windows 等平台应用通过生成的类型接口嵌入，不是独立 Host 应用。CoreRuntime 拥有通用动作、配置校验与原子持久化、Harness 适配与 AI 网关生命周期；平台应用显式注入配置根目录、资源目录和平台秘密服务，并管理应用级唯一 handle。独立 AI service lib 仍不读取全局配置或环境。全局模型独立于 AI 提供商，提供商关联多个模型；网关配置还包含显式路由及策略归属。当前先实现 OpenAI ChatCompletions v1 和显式路由，未定义的自动 fail-over 不默认启用。旧 App↔Host IPC 接入是待移除的实现偏差，不是认可的产品边界。

Agent 运行时区分类型与实例，Pi Agent 为首个类型，同类型可保存多个配置实例。首轮保持一个活跃 runner，空闲时切换实例，运行中禁止切换；这不限制配置只能有一份。会话身份必须包含运行时实例，列表 SDK 使用对应配置目录。适配器注入 Velune 网关地址和模型目录，切换或恢复会话后仍重新绑定网关，不能沿用历史原生 provider 绕过网关。Pi 派生文件不得包含上游端点／凭据，写入应用根目录下的实例专属投影目录。原运行时目录继续作为 Pi home，SDK 将受管模型目录单独注入，不覆盖原模型、认证或设置文件。直接向执行 Harness 注入上游凭据的方案已被网关接入替代。Harness 提供商配置导入包含提供商、有效模型及能力参数与认证来源，由网关后端使用；执行 Harness 仍只访问网关。导入以非秘密预览、原文件保留和来源执行绑定为边界，具体契约见 [AI service 设计](ai-service.md#harness-提供商配置导入)。

Mac 重写的已确认边界：app 只依赖通用会话投影、消息内容、运行状态、能力和配置描述，不解析 Pi 或其他 Harness 的原生事件，不发送 Harness 专用命令。适配器在 core 内执行原生协议与通用契约的转换；平台装配负责 ABI 接入、配置根目录和系统凭据入口。运行时设置使用适配器描述的有限字段与动作，具体名称仅作为数据展示。当前只接已有的 Pi，不为尚未实现的 Harness 增加空适配器或插件系统。

应用配置的已确认归属：平台装配解析 `VELUNE_HOME`，未设置时使用 `~/.velune`，向 CoreRuntime 显式传入根目录。CoreRuntime 在该目录持久化 AI provider 资源与 Harness 设置；独立 AI lib 不读取全局环境。配置文件保存凭据引用，秘密值由平台秘密设施管理。Pi 会话目录由配置指定，Velune 不将会话复制为另一套权威历史。

会话工作目录与运行时实例配置目录分属不同生命周期。本轮调整采用：连接实例时只准备配置、网关和实例范围会话列表，不启动 Pi child；新建时使用用户选择的工作目录，恢复时读取 Pi 保存的 cwd。空闲切换统一按目标 cwd 启动 Pi child，网关继续属于运行时实例，避免会话切换重建路由与认证来源。虽然固定 SDK 的 `switch_session` 能按保存目录重建项目服务，重新启动 child 还保证操作系统 cwd 一致；新建 RPC 本身不能指定 cwd。目录失效时明确失败，不回退到 app 启动目录，旧配置的 `workingDir` 不再决定执行位置。

平台视觉的已确认原则：各平台优先遵循自己的原生视觉与交互习惯，品牌和软件特点只在微小细节中体现。Mac 使用系统侧栏、工具栏、窗口、设置、语义颜色与字体；不能仅因采用 SwiftUI／AppKit 就把自绘的统一皮肤称为原生体验。跨平台共享领域契约，不要求各平台共享同一视觉布局。依据见 [PRD](../prd/index.md#已确认的产品结构与质量方向)。

Apple 外观由原生组件与系统语义样式适配，不建立自有明暗主题配置，不将深色模式单列为验收门槛。自定义绘制或内容出现具体显示问题时，在相应组件边界修复。

Velune 的产品身份是 control surface：UI、AI 服务网关与协调层服务于外部 Agent 运行时，不生成代表 Velune 自身的 Agent 人格。通用会话投影保持 user／assistant／system／tool 语义；平台按左右与居中布局呈现，不给 assistant 注入 Velune 作者标签。

## 独立 package 与平台装配

2026-10-05 用户决定全面采用 UniFFI，并授权按独立 package 拆分。实施采用 ai、ai-provider、conversation、agent-runtime、gateway、application、bindings；配置持久化保留在 application 内部。remote 和移动端尚未实现，不建立空包。

每个 package 和每个平台 app 都是一个 unit，具有明确的契约、依赖和知识维护归属。下表列职责，不要求每一行成为 package。内部模块可以形成清楚的边界而不成为独立 unit；代码目录、unit、ABI 产物不能默认一一对应。

领域 package、语言绑定与交付产物分别决定职责、跨语言使用方式和链接／打包方式，不要求一一对应。用户真正要求 Swift、Kotlin、C# 等可靠消费 Rust lib；此前手写 C ABI 已由生成的 UniFFI 接口取代。ABI 是二进制调用约定，FFI 是跨语言调用机制；生成绑定通常也封装底层 ABI，但能替代手工编写类型转换与调用代码。

建议 Rust unit 先拥有明确的公共 API，平台绑定只暴露需要的完整操作，使用结构化类型、错误与生命周期契约。内部 Rust package 不必全部适应某个绑定工具的限制。remote-only 产物可以完全不链接本地 Harness 执行代码或携带 Pi／Node。跨模块校验和失败协调留在共享用例，平台 SDK 负责语言表达，不重写业务规则。多个库若互传 opaque handle，仍须明确各库的内存释放、版本、回调和关闭关系；不能让 UI 自行补偿。

用户已选定 UniFFI，以生成类型接口取代手写产品 C ABI／JSON dispatcher。UniFFI 官方文档支持 Swift／Kotlin，生成 Swift API、C header 和 modulemap；C# 生态需另验 UniFFI 第三方绑定。工具支持语言不等于已满足 Velune 的流、线程和取消要求；当前文档还提示 Swift 6／Sendable 支持存在边界。具体来源和检查日期留在讨论任务，不把工具介绍当项目运行证据。

建议用配置预览和会话发送两个隔离样例判别：前者覆盖嵌套结构、枚举、可选值与结构化失败，后者覆盖事件流、执行取消、观察取消、忙时关闭和资源释放。取消观察与取消 Agent 执行应有不同语义，不能由绑定工具隐式决定；回调线程和释放后行为也需要明确。本次实施保留快照轮询与独立的执行取消，不引入新的观察订阅机制；完成静态检查和合成验证后更新安装版本。

| 职责归属 | 应拥有的职责与边界 |
| --- | --- |
| AI service 与 provider packages | 独立操作契约及具体协议执行。AI 领域不限 LLM，不依赖 Harness、全局配置或存储。 |
| 本地 Agent 运行时 | Harness 子进程、原生事件解析和本地会话接入。接收装配后的网关接入信息，不选择 AI 提供商或拥有网关策略。 |
| 会话契约 | 列表、消息块、快照和操作／能力。PiProjection 归 Pi adapter；契约不依赖 Pi envelope、具体 AI 协议或本地文件访问。 |
| 未来 Remote client | 尚未实现。将来映射远端控制协议时，状态权威在远端；远端 cwd 不默认是手机可访问的路径。 |
| 应用配置内部模块 | 文件锁、加载、原子提交和存储版本处理。暂留在共享应用用例所在 unit 内，不单独建立 package；领域拥有自己的配置类型，应用层负责聚合及跨领域引用校验。当前不建立通用 persist 框架或第二份 Harness 会话历史。 |
| 应用用例与平台绑定装配 | 组合独立能力、协调完整操作及失败清理。UI 不按底层步骤自行启动网关、注入配置、启动 Harness 或补偿跨包事务。 |

网关的 HTTP ingress、路由策略和提供商派发属于 AI 服务接入的装配能力；Harness 模型投影和提供商配置导入涉及两侧知识，必须留在明确的集成边界，不进入独立 AI lib。共享应用用例不亲自解析 SDK、编码协议或实现文件存储，避免变成换名后的巨大 CoreRuntime。异步执行资源由装配后的运行实例明确管理；各领域包不隐式创建自己的 executor。

当前配置仓库只有聚合应用配置这一项直接消费者；多个平台共同消费应用 SDK，不等于它有多个独立消费者。内部模块已能隔离文件机制，单独建包反而需要额外公开契约和维护责任。只有出现独立应用用例需要直接消费它，或具体依赖／平台编译边界无法通过内部模块解决，再复核提取；增加平台、切换 SQLite 或代码变长都不是单独建包的充分依据。秘密存取保持平台归属。

当前本地装配由 application 的 `local-runtime` feature 选择；禁用后不链接 agent-runtime 或 gateway，保留普通配置用例，本地执行明确返回 Unsupported。此裁剪不代表已实现远端会话。源码证据与验证缺口见 [实施任务](../../tasks/package-boundaries/packet.md)。

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

原生 UI → C ABI → 同进程 CoreRuntime → Harness Adapter → 外部 Harness 进程。AI 服务网关也由库管理，供 Harness 发起模型请求；该 loopback HTTP 数据路径不是 App 与 core 的 IPC。后续远程设备接入需要另验认证传输，不改变每个平台嵌入共享核心的边界。

- **Task Service**：目标、父子关系、依赖、验收、状态、工作区和产物归属
- **Session Supervisor**：启动、连接、恢复、单写控制租约、取消、权限转交、断线核对
- **Routing Policy**：根据模型能力、可用账户、预算、状态和明确偏好产生 RouteDecision
- **Collaboration Coordinator**：目标发现与能力／权限匹配、输入与结果路由、委派关系、交付期限及失败／冲突协调
- **Collaboration Broker**：消息接纳、队列、回执和 inbox／outbox；与 Supervisor 协作完成实际会话注入，不代替 Coordinator 的行为决策
- **Policy Service**：项目范围、数据出站、工具、账户、花费与委派许可；只允许缩小授权的子任务
- **Adapter Registry**：记录具体 Harness／协议／版本的能力，不把最低公分母当作完整能力

### Rust 与原生客户端的具体边界

以下职责以已确认的嵌入式核心边界为准；完整协作机制仍是方案，当前只实施 Pi 首循环：

- **Rust Domain**：Task／Session／Segment、路由与权限策略、协作状态转换及契约，不依赖 UIKit／Compose／进程启动
- **CoreRuntime**：在平台应用进程内组合配置、Supervisor、Gateway、进程适配与投影；跨平台复用同一套校验、状态转换和持久化行为，不另启常驻核心服务
- **UniFFI bindings**：提供具名动作、record／enum、结构化错误与关闭契约，生成接口管理跨语言内存；不恢复手写 JSON dispatcher，不复制路由或持久化事务到 UI
- **Swift／Kotlin 等平台 UI**：导航、流式展示、原生交互、系统权限与秘密设施；显式装配路径和能力。关闭窗口与退出应用是不同事件，退出须在运行时空闲且库完成关闭后进行

首循环由 CoreRuntime 原子保存应用配置，不打开旧模拟 SQLite，也不保存另一套 Pi 会话历史。未来协作数据库如需 SQLite，仍由共享核心管理其事务；不能用文件同步制造跨设备共享主库。历史存储研究见[证据](../../tasks/harness-routing-feasibility/evidence.md#原生客户端与-rust-边界)。

**当前跨语言接入**：bindings unit 以 UniFFI 0.32.2 生成 Swift／Kotlin 接口，Mac 通过具名方法、record／enum 和结构化错误操作 application。单个绑定对象拥有应用实例并串行执行同步操作；Mac 在后台队列调用，按快照轮询投影。关闭忙时拒绝并保留对象，执行取消为独立操作；释放平台包装并不等同于发出用户取消。输入与返回值的内存所有权由生成绑定管理，不再保留手写 `velune_core_*` 产品入口。配置 schema 仍为 2，生成接口由 UniFFI 校验契约。C# 第三方工具需独立验证兼容性，不宣称已有 Windows app。进程内接入保持，不另加本机 App↔core IPC。

**Rust 主核心不要求重写上游 Harness SDK**。Codex 直接接 app-server；Pi 以固定 SDK 与 RPC 子进程管理会话，adapter 注入 AI 网关和私有模型投影；模型请求由 AI 网关路由并经 AI-provider 执行；Claude 优先保留官方 SDK 的薄桥进程以正确处理其生命周期。桥只做编解码、关联 ID、流和原生权限回调，领域状态、路由、预算、协作与持久化留在 Rust。Node／TypeScript 若存在仅为原生 Harness／SDK 依赖与桥，不重新成为产品主核心；不得因改 Rust 而把 Claude 私有控制 envelope 当稳定公开协议重写。

### Apple 与 Android 宿主

- **Apple 当前落点**：Mac 使用 SwiftUI／AppKit 的原生窗口、菜单与设置，嵌入 Rust dylib；iOS／iPadOS 的 UIKit 方向不等于已实现或能运行桌面 Harness。watchOS／tvOS／visionOS 不在本轮承诺内
- **Windows 等桌面平台**：通过同一核心 ABI 复用行为，各自采用平台原生 UI 与秘密设施；当前没有已实现的 Windows app
- **Android**：Kotlin＋Jetpack Compose 原生 UI，后续经平台桥嵌入同一核心；系统权限、Keystore／通知和生命周期由平台层承担
- **移动端能力**：远端 Harness 接入和后台能力仍需单独验证。嵌入核心不代表手机能常驻桌面 Harness，也不把离线输入显示为远端已启动的任务

Mac 当前交付安装到 Applications，产品显示版本、ABI、配置 schema 与源码提交分开记录。签名／公证／商店分发和 sandbox 权限仍有后续边界，不因本地 ad-hoc 构建通过而声称已完成。先完成桌面首循环与共享核心基础，再实施远程客户端，不用 Web 过渡替代原生方向。

### 模型数据面

- Codex → Responses Gateway → 获准的 Responses 兼容上游；原生订阅或正式 ChatGPT 计划接入由对应认证路径持有身份
- Claude Code → Messages Gateway → 获准的 Claude／兼容实验上游；原生订阅默认由原版 Claude Code 直接持有登录
- Pi → 执行侧受管模型／原生 SDK 编码 → AI 网关 → AI-provider → 同协议上游；Pi 原认证来源只用于显式委托解析与刷新，不使执行侧绕过 AI 网关

Gateway 负责协议保持、流传递、资源预留和尝试记录，不执行 shell／文件工具，不创建任务，也不决定切换 Harness。工具仍在 Harness 及其受限 Runner 中执行。

认证委托采用原来源与平台秘密设施，普通配置只存引用，不复制 refresh credential。执行侧 Harness 只持有 AI 网关的本地访问凭据，上游授权由 AI-provider 的装配边界应用。独立订阅与其它 Harness 的资格仍按各自认证规则验证，不把 Pi 的实现提升为通用账号互通。

**“接管”不等于拦截进程全部网络**：只承诺覆盖经验证的模型请求通道，启动探测、遥测、搜索、安全检查、插件网络另列清单。上线前用无真实内容的流量测试验证主请求、子 agent、压缩、标题等是否全部归属正确；未知出站不会被计成已受控。

## 4. 三个 Harness 的落点

| Harness | 原生控制接口与路由落点 | 订阅、兼容与限制 | 推荐执行 |
| --- | --- | --- | --- |
| Codex | `app-server` 管线程、轮次、事件、审批；自定义 provider／Responses gateway 接数据面 | ChatGPT 原生登录与 API key 分开；当前官方 SIWC 面向符合条件的开源／本地应用；付费／远程托管走合作申请。原生 app-server 旧认证不能作为商业／托管授权；不能直接拿 CLI 缓存 token 当任意 API key | 先用官方 app-server，逐轮选模型；API lane 用 Responses gateway。本地／开源订阅 lane 用原生隔离绑定；正式 SIWC 用公开 Responses endpoint，按其限制实现逐请求路由，托管 eligibility 独立验证 |
| Claude Code | 原版 CLI／Agent SDK 管会话；`ANTHROPIC_BASE_URL` 接 Messages gateway | 当前文档允许平台运行原版 binary，用户走官方登录；禁止第三方代收或中转用户 Claude 凭据。仅 BASE_URL 的 OAuth pass-through 有技术文档，但不等于允许本产品聚合／改派订阅身份。非 Claude 模型不受 Anthropic 官方支持 | API lane 做网关，先覆盖 Claude 多供应商；订阅 lane 保留原生身份。非 Claude 协议桥作为明确标记的工程实验，不修改 binary、伪造身份或默默剥离功能 |
| Pi | RPC／SDK 管会话，当前 virtual models 提供逐请求 `route`，可见 continuation／retry 及分支状态 | provider／OAuth 的代码支持不代表供应商授权；当前有原生 MCP。没有内建沙箱或逐工具审批，须补执行隔离和工具策略 | 固定 SDK 管会话和执行侧协议编码；AI 网关负责目标选择与同协议转发，AI-provider 负责上游单次执行。Pi adapter 保留来源能力和历史身份，认证可显式委托原来源 |

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

- **V0 契约原型**：Rust lib／C ABI／平台 UI 验证接口、生命周期与原生交互；当前先用固定 Pi 与假上游证明首循环，不把边界失败变成重新选择 Harness 的理由
- **V1 本地骨架**：嵌入式 CoreRuntime、配置与运行时实例、显式模型路由；后续按已确认范围扩展三个 adapter 和协作存储，不建立 Web UI
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

Mac 当前落点与 C ABI 嵌入已按用户反馈确认，其他 Apple 平台、远程接入、官方 SDK 桥及分发安全边界仍需验证。官方集成资格、具体账户权益、ACPHub 来源、真实路由／恢复效果不由原型成功证明；登录、付费和部署仍需相应授权。

## 11. 历史模拟核心与原生壳接线

本节描述 2026-10-02 模拟原型，已被当前 Pi／ABI 产品接入取代，不是产品的进程、存储或生命周期规范。

实现入口：[simulation.rs](../../app/host/src/simulation.rs)、[schema.sql](../../app/host/src/schema.sql)、[adapter.rs](../../app/host/src/adapter.rs)、[routing.rs](../../app/host/src/routing.rs)。核心只覆盖固定深度委派、稳定 Session、单 Segment 绑定、每次重启的新 Run、独立 Attempt、事务 outbox 与合成结果验收；不是前述完整架构的实现。

单宿主通过 SQLite exclusive connection 拒绝第二个 Host；不是跨设备 lease／native fencing。未知提交阻塞该 Segment 后续投递，无自动 handoff 或人工强制解锁接口。路由只允许明确获准的模拟资源，无 API／订阅 lane。MCP 工具服务、ACP transport、真实 gateways／SDK 桥、费用预留与完整安全隔离尚未实现。

Mac mini 首个体验切片已获准使用 Swift＋AppKit 薄壳。已编写原生 UI 与独立 Rust Host 接线源码；UI 通过随包 `rpc` 子进程访问同用户 Unix socket v1，操作同一核心及 DB，显示模拟模式、任务、会话、attempt 与协作事件。UI 退出不终止 Host；显式停止取消待投递消息、保留不确定状态。无 TCP／LAN、系统常驻安装或 UI 自有任务模拟。Cloud 已验证核心与 IPC；Mac 编译／安装／用户体验待父会话设备验收。
历史模拟壳交付约定见 [开发说明](../development.md#历史-mac-mini-模拟壳)；当前 Pi 与 Mac 切片以本文开头的网关与运行时边界为准。

## 架构参考的使用

Mastra、HAPI、Lody 是用户提出的候选参考。研究围绕待决问题展开，例如运行时边界、事件投影、模型路由注入与生命周期；先核对具体版本的实现和产品前提，再说明对 Velune 的适用边界。列入参考不代表采用其功能、依赖或认证方案。关键取舍通过 advisor 推敲，涉及产品目标与范围时由用户决定。

## Pi 模型身份与网关路由

当前适配向 Pi 注入 Velune 逻辑模型 ID、明确的上下文窗口、输出上限与推理等级，通过 Pi 的模型切换更新会话身份；网关将逻辑 ID 路由到提供商及外部模型 ID。Pi 缓存目录只是该版本的适配约束，不是 Velune 网关必须采用的产品约束。当前切片按用户要求采用稳定 `velune/auto`：Pi virtual model 保留虚拟选择，Core 决定物理路由，适配器同步实际能力与会话分支状态。上游协议差异仍由 provider adapter 处理，不依赖 Pi 根据厂商名称猜测。

上下文窗口可空以保存草稿；缺值模型不能连接或进入 Pi 目录，不猜测默认窗口，也不阻断其它完整模型。运行时默认模型只用于新会话。恢复会话前经 Pi SessionManager 获取所选分支的模型，再校验路由并绑定网关；失效或外部模型要求用户重新选择，不能偷偷套用默认模型。选择归属 Pi transcript，Velune 不增加会话数据库。具体版本证据与验证边界见当前 Task Packet。

## Harness 配置来源与稳定网关入口（候选）

用户希望保留 Harness 原提供商配置，复用它的登录能力，并将相应提供商纳入 Unified AI Gateway。执行侧仍为 `Harness → Velune Gateway → provider adapter`；原配置是可明确接入的来源，不因为接管而删除。只为 Velune 管理的运行实例派生网关配置，独立启动原 Harness 的行为保持可用。普通配置的导入快照和受委托认证引用需分别处理，记录来源并明确后续更新方式，避免形成两个不明的配置权威。

用户已授权推进原生 Responses 与认证来源委托。推荐 provider adapter 复用 SDK 的登录和刷新，Rust 执行原生协议请求，认证保留同一个权威来源，而非复制 OAuth 后各自刷新。统一网关与 app 的契约仍不依赖 Pi；Pi SDK 只属于具体后端实现，不能要求一个活跃 Pi Agent 会话才能发起上游请求。复用 SDK 的实现与复用 Pi 的客户端注册身份是不同决定，现有公开登录参数是否支持 Velune 身份及授权来源仍有缺口。Pi 的受控 Node resolver helper 仅负责认证解析和刷新，不承担推理、不需要活跃 Pi Agent 会话、不恢复常驻 core Host。

`velune/auto` 表达稳定的路由策略选择，不是固定的物理模型能力。Pi 适配器可用 virtual model 向 Core 获取本次决定，返回端点仍为网关的具体模型及能力；网关必须执行同一次决定，不能在请求到达时再次按可变全局选择改投。其他 Harness 的实现需分别调查，不能把 Pi virtual model 作为通用协议。配置入口、实际模型和会话历史各有归属；固定入口不能隐藏上下文、推理级别和历史兼容性的变化。

这仍是候选设计。先通过临时目录、假 provider 与合成 OAuth 流程验证刷新单写、入口恢复、实际模型能力和决策一致性，不需要读取真实凭据或调用模型。用户触发 Pi 登录的产品目标保留，具体注册身份、委托范围和实现成本在证据充分后决定。


Pi 的物理模型身份不能只使用全局逻辑模型 ID。相同逻辑模型改投另一提供商、端点、外部模型或认证来源后，Pi 会按完整 `provider/api/model` 判断历史兼容性；只重连 runner 无法清理旧签名。当前切片使用非秘密绑定配置的稳定 SHA-256 ID，网关将该 ID 映射到同一次不可变路由；昵称、预算和 Node 路径不参与身份。选择与界面仍保存逻辑模型，历史内容转换由 Pi 原生机制完成，不新增 Velune 清签名 hook。

应用内更新 API key 或成功完成来源登录会增加认证绑定 generation，并要求重连。认证刷新不改变 generation，也不哈希 access token。外部直接在同一来源文件或 Keychain 引用背后更换账户，当前尚无可靠非秘密账户身份供 adapter 辨识，不能宣称已覆盖这类改写；应通过应用重新登录或明确更换认证来源后再继续会话。

## AI 网关职责复核

2026-10-05 用户明确 AI 网关只做同协议原生透传、路由与 fail-over，不进行协议转换／翻译。它不限于 Harness 调用方；AI 服务也不限于 LLM，sampling 是独立操作。跨单元职责、原生操作与当前偏差以 [AI 服务设计](ai-service.md) 为准；此前 Pi 原生 provider 直接承担上游派发的候选路径不再是 Velune 管理会话的目标。当前 fail-over 未实现，配置仍 Disabled。源码和独立审计证据归 [AI 网关审计](../../tasks/ai-gateway-audit/packet.md)。

## 本地可观测性装配

2026-10-05 已实施：Rust 使用 tracing，subscriber 和文件输出留在 bindings 内部装配，不建立独立可观测性 package，也不由各领域 unit 初始化进程全局 subscriber。每个 VeluneApplication 拥有独立 Dispatch；同步调用建立作用域，线程显式传播上下文，以免不同 VELUNE_HOME 的对象混写。Mac 使用原生 OSLog.Logger，并消费 typed 失败的诊断编号。日志位置、保留策略与安全字段契约归 [开发说明](../development.md#本地诊断)。

OTLP 保留为 subscriber layer 的扩展方向，尚未接入；不预建 exporter 接口或上传配置。诊断必须由失败边界输出明确 code/phase，不能靠记录原始认证附近的错误文本恢复原因。
