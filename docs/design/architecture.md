# 多 Harness 路由与协作架构

Velune 是外部 Agent 运行时（Harness）的 control surface：UI、AI 网关与协调层服务于这些运行时，不生成代表 Velune 自身的 Agent 人格。共享能力是一组 Rust libs，由各平台 app 通过 UniFFI 生成的类型接口在同一进程内嵌入，不另启常驻 Host。Harness 是会话历史的权威，Velune 只投影，不建立第二份会话数据库。

需求见 [PRD](../prd/index.md)，跨 unit 契约入口见 [Product TDD](../product-tdd/index.md)。本页说明 unit 拆分与装配、执行 owner、运行时接入、模型数据面、会话投影与协作；AI 服务与网关见 [AI 服务设计](ai-service.md)，设备与远控见 [远程访问](remote-access.md)，遥测见 [遥测装配](telemetry.md)。

## 设计原则

- **路由只选择 LLM 资源**。选机器、选 Harness、联系另一个会话由其他控制逻辑完成，不能把“转交给另一个 Agent”冒充同一请求的模型切换。
- **账户不是令牌桶的别名**。身份、计划权益、限流与数据目的地都要可区分；不合并账户额度，不通过换号绕过供应商限制。
- **协议形似不等于兼容**。同协议原生透传；异协议 best-effort 转换，无法映射的语义降级并记录诊断，不伪造历史、工具执行或成功终态。
- **协作消息不授予权限**。另一会话的消息是带出处的输入，不继承其身份或工具权限，也不冒充用户授权。
- **结果不明的操作不自动重发**。已交给 Harness 或上游的请求，在结果不明时不重放，也不改投其他账户或会话重做。
- **网关只接管模型请求通道**。网关不执行工具、不创建会话；Harness 的启动探测、遥测、搜索、插件等其他网络不算受控。
- **不同操作分开表达**。关闭窗口、取消一轮、清空队列、结束进程与退出应用是不同事件；取消观察不等于取消执行。

## 独立 package 与平台装配

每个 package 和每个平台 app 都是一个 unit，有自己的契约、依赖与 README。代码目录、unit 与 ABI 产物不要求一一对应；内部模块可以形成清楚的边界而不成为独立 package。

| unit | 职责与边界 |
| --- | --- |
| `ai` | 中立的 AI 操作契约，不限 LLM，不依赖 Harness、全局配置或存储 |
| `ai-provider` | 具体协议执行与 RelayKit 协议转换；只消费本次解析出的短生命周期认证 |
| `gateway` | LLM 网关：loopback HTTP 入口、路由策略、额度与冷却、派发与用量分析；是 AI 服务的一种应用模式 |
| `conversation` | 运行时中立的会话契约：摘要、消息块、快照、交互与能力；不依赖 Harness 原生格式或文件访问 |
| `agent-runtime` | Harness 子进程、原生协议与事件转换、历史读取、原生资源管理、SSH 执行与进程树；不选择 AI 提供商 |
| `application-api` | 公开业务输入、结果、错误与消息投影，只依赖中立类型；本地执行与远控共用 |
| `application` | 应用用例：组合各领域能力、协调完整操作与失败清理，持有配置持久化与私有认证 |
| `remote` | 设备身份、配对、iroh 连接与远程调用；消费 `application-api`，不依赖执行实现 |
| `telemetry` | 客户端遥测 owner：本地日志与 OTLP SDK 装配 |
| `bindings` | UniFFI 导出与装配：把 application、remote 与 telemetry 组合成平台可用的对象 |

`application-api` 中的 `ExecutionBackend`、`RequestProfile`、`RoutingStrategy`、`QuotaLimit` 与 gateway／ai-provider 的同名配置有意各保留一份：契约与它们唯一共同的轻量依赖是只承载协议契约的 `velune-ai`，让契约依赖 gateway 会把 HTTP 栈带进控制端构建。application 中的转换是穷尽 `match`，任一侧增减变体都会编译失败。

共享用例不亲自解析 SDK、编码协议或实现文件存储；UI 不按底层步骤自行启动网关、注入配置、启动 Harness 或补偿跨包事务。配置持久化是 application 的内部模块：领域拥有自己的配置类型，application 负责聚合、跨领域引用校验与原子提交。只有出现独立消费者或编译边界无法用内部模块解决时，才复核是否拆出 package。异步执行资源由装配后的运行实例管理，领域包不隐式创建自己的 executor。

两种装配：

- **完整端**（Mac、Windows）：本机 `ApplicationService` 加远控服务，本机运行 Harness，也可控制其他设备。
- **控制端**（iPad／iPhone、Android）：`velune-bindings --no-default-features --features remote`，依赖树不含 application、agent-runtime、gateway 与 ai-provider。

各端界面只写一份，经同一 `VeluneApplication` 业务接口操作本机或 `application_for_device` 返回的远端代理，由设备选择决定目标。application 的 `local-runtime` feature 只裁掉 Harness 执行，配置校验仍链接 gateway，不是控制端的装配路径。

**跨语言接入**：UniFFI 0.32.2 生成 Swift、Kotlin 与 Python 接口，C# 由固定提交的 `uniffi-bindgen-cs` 生成。绑定对象拥有应用实例，提供具名方法、record／enum 与结构化错误，由生成代码管理跨语言内存；平台在后台队列调用同步操作，按快照轮询投影。关闭忙时拒绝并保留对象；执行取消是独立操作，释放平台包装不等于发出取消。

**应用根目录**：平台解析 `VELUNE_HOME`（未设置为 `~/.velune`）并显式传给 Application，库不读取环境。普通配置与提供商私有认证在该目录原子保存，公开描述不含秘密；来源型 OAuth 保留原来源的登录与刷新，不复制 refresh credential。

### 远控契约与唯一执行 owner

`ApplicationService` 用单个 owner 线程串行接纳业务，并独立推进所有会话、发送队列、协作与终态清理。远控操作进入执行端这一个 owner，不另开同一配置根，也不绕过用例直接控制 Harness。bindings 在装配层把网络 Handler 接到已有的 `ApplicationService`，remote 不反向依赖执行实现。网络连接只拥有观察与请求等待；断开、后台或关闭远控入口不调用 application shutdown。宿主退出和机器电源状态决定执行是否继续。

可信设备拥有完整业务能力，但 API key、认证交互、资源正文与附件只经显式操作传递，普通摘要与日志不携带秘密；运行时摘要省略 ACP 启动参数与环境。多端按已认证设备与视图 ID 隔离浏览、导航位置与当前发送目标；composer 正文、附件及明确运行选择的同步边界按[输入区](../prd/composer.md#输入与布局)与[跨设备接续](../prd/remote-access.md#composer-状态接续)约定，不能把呈现隔离扩展为输入内容隔离。修改配置或替换内容以明确的修订为依据，轮询更新不能替换用户打开表单时的基线。审批沿用适配器的执行世代与单调 ID 映射。网络接纳账本按 incarnation、客户端 namespace、序号窗口与退休水位去重；结果有界缓存，重连可查询，结果过期或执行端重启后不承诺可以安全重发。设备、配对、寻址与连接恢复的契约见[远程访问](remote-access.md)。

### 本地可观测性装配

Rust 使用 tracing。subscriber 与文件输出由 `telemetry` 装配，领域 unit 只发事件，不初始化进程全局 subscriber。每个 `VeluneApplication` 拥有独立 Dispatch，同步调用建立作用域，跨线程显式传播上下文，避免不同 `VELUNE_HOME` 的对象混写。失败边界输出明确的 code 与 phase，并保留实际底层错误、子进程 stderr 与失败上下文；默认不采集请求、配置或会话正文。Mac 使用 OSLog.Logger 并消费 typed 失败的诊断编号。日志位置与保留见[开发说明](../development.md#本地诊断)，OTLP 见[遥测装配](telemetry.md)。

平台的“问题”窗口是交互投影：消费 typed 失败，在当前进程内保留有界记录与原诊断编号，持续读取故障按来源合并、恢复后消解；不读取日志正文，也不反向增加领域包的日志接口。

## 运行时

运行时区分类型与实例，同一类型可保存多个实例。类型是带兼容版本区间的 versioned variant（[S34](../sources.md#s34)、[S44](../sources.md#s44)），family 只用于展示分组，不作为派发键；区间外的版本明确拒绝。各 variant、区间与原生控制方式见 [agent-runtime](../../packages/agent-runtime/README.md#版本与原生控制)。运行时 descriptor 声明协议、启动选项、会话选项与管理能力，平台据此展示，不按类型 ID 分支。

平台 app 不捆绑任何 Agent runtime。适配器从实例配置的外部安装解析 CLI 或 SDK；执行、历史、导入与来源认证不从应用资源或其他实例回退。应用只分发薄 adapter helpers 与只读历史 parser。

### 执行位置、HOME 与模型来源

三个选择相互独立：运行位置决定本机或 SSH，HOME 归属决定实例使用的用户家目录（继承、现有路径或 Velune 管理），模型来源决定本轮由 Harness 原生提供商还是 Velune 网关执行。application 解析实例配置并协调准备与释放，agent-runtime 封装原生协议与进程。

- **模型来源**：Velune 模型条目身份与运行时原生模型身份分开。原生选择附属于一个实例，不按相同 model ID 推断跨提供商等价；使用原生模型不需要 Velune 提供商、复制认证或启动网关。
- **单轮网关覆盖**：managed 轮次的网关接入属于执行环境，不写入持久配置。application 捕获本轮路由并准备 adapter 的原生参数，工作结束或失败时释放临时进程与网关访问。没有运行中的 managed 轮次时，Harness 的持久提供商配置保持原样可用。
- **HOME**：HOME 是 Harness 及其工具子进程看到的家目录，agentDir 是 Harness 的配置与状态目录，cwd 是会话工作目录。Velune 管理的 HOME 按实例稳定分配，配置重置或执行结束不删除其中的历史与认证；它提供环境归属，不承诺阻止工具访问其他文件。
- **SSH**：版本检查、模型发现、启动、恢复与历史操作都在执行机器完成；binary、agentDir、Node 与 cwd 按执行机器的路径解释。远端不启动常驻 Velune Host，也不把原生会话复制到本机。managed 轮次与协作 MCP 经远端 loopback 的反向转发访问本机服务，随执行资源释放；断连不自动重发。跨机器接续沿用来源 cwd，目标必须存在同一路径。SSH 连接认证归 OpenSSH，模型认证归 Harness 或网关。

### 运行时工作的生命周期

运行时进程与一轮的临时网关覆盖持续到该轮工作全部结束；原生子 Agent 与后台任务也算工作（[PRD](../prd/agent-runtimes.md)）。检测归 adapter，寿命策略归 application：

- agent-runtime 的 `NativeSession` 以 `turn_running()` 与 `work()` 区分轮次与轮次外工作。Codex 跟踪同一 app-server 中其他线程的状态与轮次，Claude Code 取未结束的任务；ACP、DSH 与 Pi 暂无轮次外工作。
- application 以“轮次运行或仍有工作”判定会话执行中。经网关的轮次在两者都结束时才结束进程；Codex 随后以短生命周期 app-server 把该进程运行过的线程用原生提供商 `thread/resume` 一次，使线程回到原生提供商，再释放 runner 与隧道。回收、退出、跟随来源、删除、归档与编辑都把这种会话视为执行中。
- 下一轮能复用进程时照常发送。需要另起进程（选择、提供商或会话变化；Claude Code 每轮都是）时，`restart_needed` 与 `ensure_work_ended` 以 `Error::OutstandingWork` 拒绝，不结束子 Agent；消息进入发送队列并标记 `FollowUp.waiting = SubAgents`，工作结束后自动发出。
- 停止父会话同时中断其子 Agent；`stop_sub_agents` 只停止子 Agent。退出时若有轮次或子 Agent 在 Velune 的进程中运行，平台先确认，再以 `shutdown_stopping_work` 结束它们。

### 通用 ACP

`agent-runtime::acp` 是独立的 ACP v1 客户端，连接用户安装的原生 ACP Agent 或社区 bridge；application 只装配执行位置、HOME、可执行文件、参数与显式环境。新增 Agent 不需要新的类型分支、私有历史解析器或版本正则；协议主版本由 `initialize` 协商，程序名称、版本与可选能力只作为数据。官方 Rust schema 定义 wire 类型，Velune 拥有会话投影与进程生命周期。

ACP 与专用 adapter 并存，`acp-v1` 不支持 Velune managed 网关。模型与其他会话选项使用 Agent 提供的 config ID、category 与 opaque value，以 Agent 返回的结果为准；查询模型不创建隐藏会话，模型切换不重启进程。list、load、resume、close、delete 按能力调用：load 回放原生历史，resume 不提供历史；没有 load 的 Agent 仍可连续多轮使用，但关闭后不保证重开，也不能作为跨 Harness 接续的源或目标。客户端只声明实际实现的能力，文件与终端回调不在范围内。

### 原生资源管理

agent-runtime 的 `management` 拥有经过版本验证的原生资源位置、格式与文件操作：用户指令、MCP 配置、skills 与原生子 Agent 定义。application 解析实例、用户或项目范围、执行机器与 HOME，并执行忙碌门禁。原生配置仍是权威，受管 HOME 中的文件同样如此，不形成 Velune 镜像；平台以资源库为唯一编辑入口（[PRD](../prd/resource-library.md)，实现见 [application](../../packages/application/README.md)）。

读取不因目录缺失而创建 HOME 或配置；编辑以预期 revision 避免覆盖已观察到的外部修改。共享 MCP 配置只公开与修改目标服务条目，列表、Debug 与日志不含环境变量或 header 值；各 Harness 的格式不统一转换，坏配置报告无效并拒绝覆盖。skill 是目录资源，按单文件读写并保留执行位，不跟随符号链接或特殊文件。修改在运行中拒绝，不自动重启会话；保存成功只表示原生配置已写入，何时加载由 Harness 决定。管理范围不等于祖先目录、插件或组织策略合并后的有效配置。

## 模型数据面

选择 Velune 模型的轮次走 `Harness → Velune 网关 → ai-provider → 上游`，选择原生模型的轮次由 Harness 沿用其原提供商。执行侧 Harness 只持有网关的本地访问凭据，上游授权在 ai-provider 的装配边界应用；application 私有保存提供商认证，网关只接收中立的异步解析接口。

| Harness | managed 轮次的接入 |
| --- | --- |
| Codex | 线程配置注入名为 `velune` 的 Responses provider，指向网关 |
| Claude Code | `ANTHROPIC_BASE_URL` 指向网关的 Messages 入口；Claude 订阅另经本机 Claude Code 执行后端，不复制 Claude token |
| Pi | SDK 追加独立生成的网关提供商，不覆盖原 `models.json`、认证或设置 |
| DeepSeek Harness | 执行 overlay 追加网关模型目录，不改原生默认选择 |

运行时 descriptor 的 `supportedProtocols` 是它发给网关的协议，`supportedProviderProtocols` 是经网关可到达的提供商协议；协议可到达不等于具体模型支持全部工具、输入模态或推理参数。路由策略、额度、fail-over 与协议转换见 [AI 服务设计](ai-service.md)。

**Pi 的模型身份**：Pi 的 managed 执行使用稳定的 `velune/auto` virtual model，由 Core 决定本轮的具体逻辑模型并返回其能力；网关执行同一次决定，不在请求到达时按可变的全局选择改投。Pi 按完整 `provider/api/model` 判断历史兼容性，因此物理模型 ID 使用非秘密路由绑定的稳定 SHA-256 身份，昵称、预算与 Node 路径不参与；界面与选择状态保留逻辑模型。上下文窗口缺失的模型不能用于 Pi 执行。

应用内更新 API key 或完成来源登录会增加认证绑定 generation，使执行准备失效，下一次发送重新准备；认证刷新不改变 generation。外部在同一来源文件背后更换账户无法可靠辨识，应在应用中重新登录或更换认证来源。

## 会话

### 会话权威与展示投影

`conversation` 是运行时中立的会话投影契约，Harness 是持久化权威。adapter 把原生历史与执行事件映射到同一类型：角色区分用户、assistant、工具与系统，内容区分正文、推理、工具与通知；工具调用与完成分开表达，保留原生关联 ID 与已有输出。原生消息身份优先，缺失时由 adapter 分配投影内稳定的身份。

`agent-runtime::transcript` 统一产生顺序投影：application 输入消息、身份确认与执行状态，输出同一 typed snapshot 中的有序展示项、工作区间、大纲锚点与展示身份。`Message.completed` 只表示内容完整，不代表执行成功。工作区间以用户消息为锚，覆盖到下一条用户消息之前的全部响应；只有区间末条纯 assistant 正文外露，其余折叠，末条不是纯正文时也折叠，不向前寻找旧答复。执行结束与工作时长依据 Harness 生命周期；历史没有完整证据时不虚构耗时。展示身份在当前投影内稳定，live→native 确认只修改 canonical 引用；映射不持久化。原生历史可以替换、分支、压缩与重排，投影不是永久追加日志。

标题由 `ConversationTitle` 区分 Native、FirstMessage、Untitled，bindings 只导出 display text。时间在 adapter 边界规范为可选 Unix 毫秒，缺失保持缺失，平台负责时区与格式。snapshot 的 revision 只在当前投影生命周期内有意义，平台以请求 generation 与会话身份丢弃过期回调。

内部完整快照的 `Executions` 关联原生执行范围与消息成员；每次明确发送／继续／主动压缩建立独立活动身份，steer 和自动恢复留在当前活动。用户显式继续可复用原展示范围，但重置本次配置捕获、恢复预算和终态消费。adapter 解释明确的结束证据，application 按活动一次性处理通知、队列与遥测；闲置、消息完成和取消意图不能替代成功／取消确认。关联只随内存快照保留，不新增会话日志或公开远控协议。

续接装饰同时转换消息 ID、执行范围／成员和身份确认，保留活动身份与原生分支点。转录投影原子消费消息和执行关联，各 steer 展示段共享结果；独立维护始终独立呈现，轮次内压缩完成后进入工作过程。原生历史没有结果证据时保持未知。

### 浏览与执行准备

列表合并全部启用实例，不按当前执行实例过滤；禁用实例保留配置与原生数据，但不读取其历史，也不出现在可执行选择中。每个实例的读取失败独立报告，不解释为其他来源没有会话。历史读取 helper 是只读适配步骤，不是执行进程：读取不要求模型、认证或工作目录仍可执行，详情 ID 必须属于所选实例的列表，不接受任意路径。分组、筛选与排序是展示投影，与会话打开和执行归属分开。

新建会话给出运行时、工作目录与模型，首次发送时自动准备并由 Harness 建立会话。准备失败保留内容与选择，修正后可重试，不自动重发已被接受的消息；配置变化只使执行装配失效。Pi 可从原生记录恢复有效的模型选择；Codex 与 DSH 的历史只有 API model ID，不能证明提供商，继续前要求选择。

### 下一轮选择与跨 Harness 接续

下一轮的运行时与模型是发送意图，不随浏览的历史来源变化。历史的原生来源负责读取、重命名与删除；已开始的轮次保留自己的实例、会话与模型，用于轮询、取消与交互回复。`send_turn` 显式接收来源会话、目标运行时与选择，同来源时恢复原生会话。

不同 Harness 的原生会话 ID 不能互相恢复。跨 Harness 接续为线性逻辑会话：每次切换（包括切回）创建目标原生会话。Claude Code、Codex 与 Pi 之间以**原生上下文翻译**接续：来源 adapter 导出 compact 和分支选择后的有效模型上下文，经过独立的中性上下文类型，目标 adapter 写入自己的原生会话记录，再以普通新消息发送本轮请求。摘要、保留消息、工具参数与结果保持配对；不重新执行历史工具、审批或签名推理，不用固定预算删除旧轮次。无法可靠解释的原生格式在改变会话归属前明确失败。

模型上下文与展示转录分开：界面仍读取各来源段的完整历史；导入目标的上下文前缀只供模型恢复，不在界面重复展示。`VELUNE_HOME/conversation-links.json` 只保存有序原生引用、封闭段截止位置和导入前缀边界及摘要校验，不保存正文。消息 ID 以段命名空间区分。准备目标前核对来源历史与空闲状态；取得目标身份后先保存关联再发送，结果不明时不重发或删除目标。逻辑标题沿用首段原生名称；逻辑删除逐段执行原生删除，全部成功才移除关联。

旧式引用交接仍只读还原；含旧 `<velune-context>` 载荷的来源不能当作可靠上下文再次导入，需先显式处理。ACP、DeepSeek 目标尚无原生写入契约，仍保留既有引用式路径；从它们向上述原生目标的有效上下文导出尚不支持。这些路径不宣称具备完整的 compact 翻译能力。

某些 Harness 的新会话在首条消息前没有持久记录。application 记录本次创建且尚未提交 prompt 的草稿身份；首发前改变模型来源或 Harness 时可以重建目标并保持公开会话身份。一旦尝试发送就撤销重建资格；未发送的草稿不保证跨重启保留。

### 原生会话管理

重命名与删除由 agent-runtime 的原生操作执行，application 负责实例身份校验、执行状态与投影同步；平台不写运行时文件，也不维护标题覆盖或隐藏列表。能力由版本化描述声明，缺失的能力不以本地隐藏代替。执行中的会话不可管理。Pi 首条消息前的未落盘会话仍由活动 SessionManager 拥有，只有身份精确匹配且确认文件不存在时，才经活动 RPC 改名或关闭空草稿。

### 会话数据通路

- **列表**：application 按实例保留最近一次列表（实例配置不变才有效），打开列表中的会话直接复用来源范围校验。Claude Code 与 Pi 的列表先建索引：只取文件状态与文件头的工作目录、开始时间，用于分组、计数与决定先读哪些；只解析列表窗口显示到的会话，未变化的文件复用已读摘要。搜索、协作工具与按 ID 查找按需读取所需会话。索引是进程内缓存。
- **交付**：core 为每个展示视图（上下文运行时＋会话，最近 8 个）记录已交付内容，以 generation／sequence 游标表示交付位置。快照中超过 2 KB 的推理与工具输出只携带字节数，由 `message_payloads` 按消息读取完整内容；`snapshot_update(since)` 只返回变化的消息与完整顺序。
- **平台**：轮询只传游标并按身份合并变化；展开延迟内容时按需读取。滚动条与大纲代表完整会话，不做界面分页。

## 协作

### Agent 间协作

跨 Harness 协作是会话之间的对等交流（A2A），参与方都是普通会话（[PRD](../prd/collaboration.md)）。application 持有一个 loopback MCP 服务，注入 Velune 执行的每个会话，每个会话一张随机票据；票据由 HTTP 层校验并决定调用方，工具参数不能自报身份，也不能传程序、环境或认证。MCP 线程只把调用放进收件箱；下一次客户端请求推进全部会话后，由同一个 owner 执行这些调用。因此被联系的会话与用户打开的会话是同一个执行对象，协作进度依赖客户端持续轮询。工具与参数见 [application](../../packages/application/README.md)。

协作消息带出处包装，作为对方的普通下一轮提交；对方忙碌、待审批或在 Velune 外运行时拒绝，不排队。等待以“调用方尚未读到的非运行状态”判定。取消不跨会话传播，不设联系上限，循环联系由忙碌拒绝自然阻断。SSH 运行时经反向转发访问服务；只支持 stdio MCP 的 ACP Agent 使用内嵌的 Python relay。服务、转发或 relay 不可用时会话照常运行，只是没有协作工具。

### 原生子 Agent

原生子 Agent 是 Harness 自己的子会话，属于会话体验。会话模型以两个通用字段表达，平台不按运行时分支：`ConversationSummary.parent_conversation_id` 标出子会话及其父会话，`MessageBlock::Tool.conversation_id` 记录调用启动或指向的子会话。快照的 `collaborators` 另列 A2A 参与方及方向，平台把子 Agent、父会话与协作方一起呈现为关联 Agent。

- **Codex**：`collabAgentToolCall` 在恰有一个 receiver 线程时链接该线程，`subAgentActivity` 的 `started` 链接其 `agentThreadId`；线程列表按 `parentThreadId` 或 `source.subAgent.thread_spawn.parent_thread_id` 标父会话。Codex 在子 Agent 结束后归档其线程，按身份查找时只用 `thread/read` 读这一个线程，不扫描整个归档。子线程上的审批请求作为父会话的交互出现，标题前缀“子 Agent：”。
- **Claude Code**：以工具结果的 `agentId` 与任务通知链接 `<session>/agent-<id>`，历史读取父会话旁的 `subagents/agent-<id>.jsonl`；`Agent` 调用与其结果合并为一个工具块。

子会话进入 application 列表但平台不在侧栏列出；它们只读，`send_turn` 明确拒绝，A2A 的 `list_conversations` 也不提供。子 Agent 的审批、继续与取消归启动它的会话。

## 平台宿主

- **Mac**：SwiftUI／AppKit 原生窗口、菜单与设置，嵌入 Rust dylib。每台设备一个 `DeviceStore`，每个会话列表与标签页各一个 `ConversationStore`；平台命令作用于聚焦的窗口。见 [Mac README](../../app/mac/README.md)。
- **Windows**：C#／.NET 10＋WinUI 3（Windows App SDK，非打包、自包含）完整端（[S45](../sources.md#s45)）。C# 绑定以 P/Invoke 调用 `velune_bindings.dll`，与核心同进程；与同级变体同名的变体字段由 `uniffi.toml` 改名，不改 Rust API。见 [Windows README](../../app/windows/README.md)。
- **iPad 与 iPhone**：SwiftUI 通用控制端，只链接 remote-only 核心。见 [iOS README](../../app/ios/README.md)。
- **Android**：Kotlin＋Jetpack Compose 控制端，经 remote-only UniFFI／JNA 嵌入，不携带 Harness 或 AI 执行栈。见 [Android README](../../app/android/README.md)。

各平台遵循自己的原生视觉与交互习惯，跨平台共享领域契约而不共享视觉布局；原则见 [DESIGN.md](../../DESIGN.md)。产品显示版本、ABI、配置 schema 与源码提交分开记录；签名、公证与商店分发不在当前范围。
