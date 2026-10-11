# Agent 运行时

本 unit 实现通用 ACP v1 接入，以及 Pi RPC／SDK、Codex app-server、Claude Code stream-json 和 DeepSeek Harness ACP 的专用适配器。它负责原生控制、会话读取与消息 projection；真实会话由各 Harness 持久化，Velune 不复制为另一套会话历史。Pi 适配器另提供显式来源的认证交互和只读提供商发现。

在工作区内，它只依赖 [会话契约](../conversation/README.md)，不依赖 AI 服务、网关、application 或 UniFFI。执行配置与网关注入的临时 token 由 application 显式传入；它不选择上游模型路由、保存应用配置，也不拥有原生界面。

`authentication` 与 `provider_source` 通过本包资源中的薄 Pi helper 执行来源操作，返回非秘密的类型化状态和发现结果。application 负责将结果导入网关及原子提交；application 的提供商私有认证解析器通过固定 `pi_auth.mjs` 委托 OAuth 来源解析与刷新，不读取环境里的默认认证。

执行侧通过 `pi_rpc.mjs` 装配固定 SDK。所选 CLI 必须对应 Pi 1.0.2 安装；原运行时目录继续拥有设置、认证、扩展和会话，Velune 将本次网关模型目录作为额外 provider 配置追加到同一 `ModelRuntime`。因此原生 provider 与 Velune gateway 可以共存，Pi SDK 自己负责 provider 注册、模型选择和派发；普通来源扩展注册不会因为不是 Velune provider 而被拦截。来源读取失败保留错误类别、阶段、退出状态和原始原因；SDK 错误输出供本地诊断使用，不以固定说明替换。常规诊断不采集配置文件或会话正文。unit 只发出 tracing 事件，由消费方装配日志订阅器，RPC 读取线程显式继承其 dispatcher 与 span。`velune/auto` 与注入的 `velune-gateway` 由适配器装配；Velune hook 仅处理其 virtual model，不修改原生 provider 的请求。

Pi 来源适配器按固定 SDK 1.0.2 的协议与有效兼容要求判断可导入性。Chat Completions 的已知设置只在当前网关请求能保持其语义时放行；原提供商和 URL 推断的输出字段、推理格式等默认要求也参与判断。`max_tokens` 来源设置通过可选 Pi 投影交给 application 转换为提供商模型的类型化 wire 选项，adapter 不依赖 AI-provider。未知字段或需要未实现 wire 行为的设置逐项说明原因，不将所有非空 compat 对象一概拒绝，也不把旧 Codex 后端或 Messages API 冒充受支持的 OpenAI 协议。

Pi 固定版本及协议边界见 [开发说明](../../docs/development.md)。SDK 与 Node 资源只随需要本地执行的 app 装配，不进入禁用 local-runtime 的绑定产物。

## 版本与原生控制

运行时类型是 versioned variant，family 仅用于展示分组，不作为派发键。四个专用 variant 为 `pi-1.0.2`（family `pi`）、`codex-0.159.3`（family `codex`）、`dsh-acp-0.2.0-rc.2`（family `deepseek-harness`）、`claude-code-2.1.286`（family `claude-code`）。各自版本 regex 是 `^1\.0\.2$`、`^0\.159\.3$`、`^0\.2\.0-rc\.2$`、`^2\.1\.286$`。专用适配器启动前探测配置的 CLI 版本；协议握手继续检查所需能力。不同 breaking-change 版本应增加独立 variant 与 adapter，不能放宽 regex 假定兼容；多个 variant 与同 variant 的多个实例可并存。

通用 `acp-v1`（family `acp`）的 `version_regex` 为 `^1$`，表示协议主版本。它通过 `acp::Client::connect` 的 `initialize` 协商，既不执行未知程序的 `--version`，也不根据 `agentInfo` 名称或版本筛选 Harness。公开 CLI 版本探测 API 对此 family 明确返回 `unsupported`，调用方应使用协议握手。

## 通用 ACP v1

`acp::Config` 接受目标机器上的绝对 `binary`、完整 `args`、显式 `environment`、`ExecutionTarget` 与可选 `os_home`。用户安装的原生 ACP 程序和社区 bridge 均可直接作为启动命令；本包不安装 Agent、不维护程序名单，也不要求通用 ACP 使用 Node 或本包 helper。SSH 与 HOME 复用下文执行边界。设置 `os_home` 时禁止环境表重复设置 `HOME`，避免家目录归属不明确；环境值可能包含秘密，因此该配置不实现 Debug。

wire 类型采用官方 `agent-client-protocol-schema = 1.11.0` 的稳定 v1 模块，关闭默认与不稳定特性。握手只接受协议 1，保留 Agent 信息、认证方法描述和实际协商能力。`create`、`send`、`poll`、`cancel` 与 `reply` 提供会话、流式更新和权限控制；`list_sessions`、`load`、`resume`、`close_session`、`delete_session` 分别检查 Agent 公布的能力。注册表里的静态删除标记不能替代连接上的动态能力。连接会跨 turn 保留，未支持 load 的 Agent 仍可在当前连接持续对话。

显式 `shutdown` 对空闲且支持关闭的会话先发送 `session/close`，让 Agent 完成资源释放与自身持久化，再关闭输入并有界等待进程退出；关闭请求失败仍清理进程并向调用方返回错误。不支持 close 时只关闭连接；忙碌或已失败的连接直接终止。Drop 仅执行进程清理，不阻塞等待新的协议请求。

模型与其他会话配置统一通过 `config_options` 和 `set_config_option(config_id, value)` 暴露。`ConfigOption` 保留 id、名称、说明、可选 category、当前值及带分组的选项；`ConfigValue` 区分 Select 与 Boolean，选项值是不透明字符串，不能拆解或推测 provider。`models`／`select_model` 是首个 `category=model` 配置的便利入口，不替代完整配置 API。没有模型配置、缺少 category 或出现未知配置类型时，仍可使用 Agent 默认值运行；官方 schema 会忽略不支持的可选配置类型。配置响应与 `config_option_update` 都完整替换当前配置。Agent 明确拒绝配置请求时保留原状态；传输中断、超时或无效响应使结果未知，会终止连接并禁止继续发送，不能展示旧配置后猜测成功或自动重试。

权限请求保留 Agent 公布的 opaque option ID，只有调用方的明确选择才会回复；不会默认批准。取消先将待答权限回复为 cancelled，再发送 `session/cancel`，继续消费更新直到原 prompt 响应确认结束。输入输出目前限于文本消息、文本资源、工具与计划 projection；图片、音频和二进制资源显示明确的内容占位。客户端不公布文件系统、终端或终端认证能力，不在 SSH 场景误把目标文件操作执行到本机。`auth_methods` 仅用于描述外部认证入口，本轮没有 `authenticate` API；认证不足的原生错误保留来源码与说明，由用户在外部完成登录或配置。

会话列表来自 `session/list`，保留原生 ID、cwd、标题与 RFC 3339 更新时间，支持分页，不伪造创建时间。`load` 消费 Agent 实际回放的历史；`resume` 只恢复原生上下文并明确提示没有回放，不能当作历史读取。原生消息 ID 可直接确认本地投影身份；没有原生 ID 时使用会话 ID 与消息位置生成确定性投影 ID，同样的回放结构可重复得到同样的身份，但它不是原生持久 UUID。live 用户消息的首个原生回声按本轮位置替换待确认消息，不按正文匹配；若 Agent 改变历史顺序或消息分段，位置身份不能保证跨版本稳定。Velune 不为这些缺口建立另一套历史存储。

通用 ACP 不假定任何 gateway 环境变量、提供商配置格式或认证来源，因而没有 Velune managed provider 注入能力；专用适配器继续负责已验证的注入路径。协议依据于 2026-10-10 复核：[官方初始化](https://agentclientprotocol.com/protocol/v1/initialization)、[会话配置](https://agentclientprotocol.com/protocol/v1/session-config-options)、[会话建立与加载](https://agentclientprotocol.com/protocol/v1/session-setup)。[acpsub](https://github.com/lexoliu/acpsub) 用于参考任意命令接入与已公布配置选项；其 daemon、自动安装、强制模型选择和自动批准策略不属于本包契约。隔离验证及仍待实现的 app 入口见 [任务记录](../../tasks/acp-adapter/packet.md)。

## 会话级协作 MCP 注入

`mcp::Server { name, endpoint, token }` 描述 application 为当前执行授权的临时 loopback HTTP 服务，Pi、`NativeConfig` 和 `acp::Config` 通过 `mcp_servers` 接收。MCP 服务、父子关系、任务调度与权限范围归 application，本包只把连接交给实际 Harness。application 为父执行生成独立服务名称，子执行传空列表；本包不从工具参数推断父身份，也不自动批准原生权限请求。名称限定为不超过 32 字节的 ASCII 字母、数字、下划线或连字符，列表内不得重复。Debug 隐去 endpoint 和 token；Pi 配置序列化跳过全部临时 MCP 注入，token 不进入命令行。

| 适配器 | 临时注入入口 |
| --- | --- |
| Codex | `thread/start` 与 `thread/resume` 的 `config.mcp_servers`；通过子进程环境传入 bearer token，保留原生配置中的其它服务。 |
| Claude Code | 初始化后读取 `mcp_status`，拒绝与已加载服务重名，再通过 `mcp_set_servers` 追加并确认连接；每轮新进程重新注入，全程经控制 stdin，不写 `.mcp.json`。 |
| Pi | 装配 SDK 的 `createMcpExtension()`，通过 `registerMcpServer` 注册当前会话的 direct 工具；SDK 保留既有 MCP 来源与权限扩展，注册不写回配置。 |
| DSH | `session/new` 与 `session/resume` 的标准 `mcpServers`；核对 HTTP 能力，模型目录发现的临时会话不注入协作服务。 |
| 通用 ACP | `session/new/load/resume` 的标准 `mcpServers`；HTTP 仅在 Agent 公布 `mcpCapabilities.http` 时使用，否则采用标准必需的 stdio 传输。 |

SDK 或协议已有目录入口时只使用其结果处理重名；不扫描认证文件或另行解析用户持久 MCP 配置。通用 ACP 没有枚举 Agent 自有 MCP 服务的标准入口，因此 application 的随机执行名称是避免跨来源名称碰撞的共同约束。Pi 的文件配置优先于扩展注册，重名不会覆写原配置。

MCP 与模型网关复用 `LoopbackTunnel`。远端服务地址由运行时改写为经过核验的目标 loopback 端口，Guard 随运行时执行对象持有；调用方不能再为相同注入包一层转发。通用 ACP 的 stdio 回退在目标机器的私有临时目录投送内嵌 `mcp_stdio_relay.py`，需要当地可用的 Python 3，独立于 Node 与专用 helper 资源包。relay 仅传递 JSON-RPC、初始化的 MCP session/version HTTP 头和显式取消通知；不拥有任务、会话历史、重试或授权决定。每个 HTTP 请求独立等待，避免等待子任务时堵住状态或取消；stdin EOF 与 stdout 断开会结束 relay。执行对象释放后删除临时目录并关闭转发。

relay 只连接显式端口的 `http://127.0.0.1` 服务，忽略代理环境，不跟随重定向。application 的 MCP 服务必须采用 JSON 响应模式和有界等待；relay 的单请求上限为 60 秒，不支持 SSE 或服务端主动请求。正常 HTTP-capable Harness 使用自身原生 MCP 客户端。接入证据见 [运行时注入验收](../../tasks/harness-interoperation/runtime-injection-evidence.md)，父子任务完整闭环归 [当前任务](../../tasks/harness-interoperation/packet.md)。

## 专用适配器行为

Codex 使用 app-server 的 thread／turn 控制；DSH 使用 ACP v1 的 session/new、resume、prompt、cancel、set_config_option。DSH 的 agentInfo.version 是 ACP 插件版本，不用于判断 CLI 版本。Codex 历史由 app-server 的 thread/list、thread/read 与 thread/items/list 分页读取，原生服务负责逻辑身份和历史继承；DSH 历史通过 huihua `0.2.0` 公开 provider API 读取显式 home／roots，未知记录不原样输出。历史浏览不启动 turn 或模型网关，不替代原生 resume；Pi 保留 SDK SessionManager 的分支与模型选择语义。

Codex 0.159.3 的 thread/revert 保留稳定 thread ID，同时创建新的 rollout 文件并更新当前历史；paginated 历史还可以引用旧文件的前缀。因此不能把 thread ID 当成物理文件唯一键，也不能按时间任选重复文件。列表使用原生 thread/list 的完整来源枚举与空提供商过滤；详情用 thread/read 核对身份，再按升序分页读取 thread/items/list。legacy 与 paginated 共用原生入口及实时消息的 item 投影，保留原生 item 时间，不自行解析 history_base 或扫描所有文件。短生命周期 app-server 与既有原生管理共用有界 RPC；原生读取失败明确诊断，不回退为文件选择。

原生权限或回答请求映射为 conversation 的类型化交互，用户显式选择、回答或取消；adapter 不自动批准。执行模型选择区分运行时既有模型与 Velune managed 模型。`NativeConfig.gateway` 为空时保留原生 provider；`models` 返回运行时的模型身份，`select_native_model` 选择既有模型，`select_model` 只为下一轮提供网关注入。运行时配置不含 Velune 默认模型，不能凭 API model ID 猜测网关路由。Codex 从会话 cwd 的 `config/read` 捕获原有 provider，原生 resume 显式传入该 provider；DSH 原生选择必须显式包含 provider ID，严格匹配 ACP 公布的 provider/model 值，不能按同名模型猜测提供商或自行拼接未公布的 opaque option。DSH reasoningEfforts 需要实际 wire 映射，当前仅有能力等级列表，不猜测映射，保留 SDK 默认行为。

DSH 执行 overlay 只为 llm-pi-ai 追加网关目录，保留来源已有 provider、ACP 默认选择和其它设置；不通过禁用原来源 provider 来强制执行路径。CLI 会按上游行为准备其 ACP profile。实例的 CODEX_HOME／DSH_HOME 与会话 cwd 分开；网关注入文件属于 application 的运行时投影目录。DSH 的原生历史以最后一次请求 header 记住 provider/model，ACP 的 set_config_option 只改变当前进程。因此 app 私有 overlay 保留已使用模型的无凭据 metadata，恢复旧会话后再通过显式选择进入 native 或 managed 轮次；不重写真实会话以伪造 provider 恢复。原生轮次不创建网关，native 子进程的历史 overlay 不携带 token，并把派发地址设为不可监听的 loopback 端口 0；它只提供 resume 所需的模型 metadata，必须选择既有 native 模型后才可 prompt。

DSH 的 ACP 模型目录只随会话返回，且 `session/new` 会持久化空会话。未打开会话时，模型发现使用独立进程，把会话与 projection cache 定位到 application 私有临时目录，并关闭持久查询存储；读取原生 `configOptions` 后关闭并清理。成功与失败都不向来源会话根写入空历史，SSH 下的临时目录也在目标机器创建和删除。

Pi 与 Codex 的 stdio JSON-RPC 记录按换行边界读取，不附加固定的 1 MiB 单记录或业务输出上限；历史 helper 的 stdout 也完整读取。stderr 仅保留有界诊断缓存，超出时标记截断，不把诊断量误报成业务输出失败。超时、进程退出、JSON 解析和通道关闭仍按原生错误传播。

Node 与用户配置的 Pi／Codex／DSH 安装均为外部依赖。huihua 固定依赖、完整许可证和安装入口见 [runtime-support 声明](runtime-support/THIRD_PARTY_NOTICES.md) 与 [开发说明](../../docs/development.md#原生运行时与历史读取)。只随 Mac 分发薄 adapter helpers 与 huihua parser 依赖，不分发任何 Agent runtime 或 Pi SDK。所有 Pi 操作从显式 CLI 解析对应外部安装；OAuth 来源固定安装与版本，缺失或变化不隐式回退。

版本注册表同时声明中立 `RuntimeProtocol` 支持集合。application 将其转换成网关协议描述供界面使用，执行准备也检查同一集合；family 不参与协议资格判断。

历史与实时事件归一为同一 conversation 契约，区分正文、推理、工具和系统通知；工具调用／结果按原生关联 ID 聚合，不能在调用出现时标成已完成。来源标题与时间在适配边界保留含义，文件名不充当显示标题。Codex 注入上下文与真实用户记录通过原生来源 metadata 区分，不使用正文字符串启发式。Pi 常规轮询只消费事件，在加载、已处理扩展命令、settled、取消和压缩边界重新同步权威上下文，处理原生分支整体替换；这些行为不新增会话存储。

版本描述声明原生会话重命名／删除能力。Pi 管理通过固定 SDK 和来源范围内的原会话文件完成；Codex 使用独立 app-server 的 thread/name/set 与 thread/delete，不准备 AI 网关。Codex 名称与消息历史都由原生 thread API 提供。DSH 当前 ACP adapter 不开放会话管理，不把归档、本地隐藏或改标题投影代替持久操作。

消息列表的可替换投影由 `transcript` 模块统一处理。它顺序消费权威消息替换、身份确认和执行状态，输出有序展示项、工作范围、大纲锚点及稳定展示身份；不保存正文副本或第二份会话日志。工作范围完整覆盖用户消息到下一用户消息前的响应，不因某条正文完成而截断；只有范围末条纯正文外露，工具、混合内容或系统通知末条也归入工作过程。运行与耗时只依据Harness生命周期。live→native 身份确认更新 canonical 引用，不改变展示身份。forward-only 允许后来的替换修订、删除和重排先前消息。本机观察到的执行起止可提供工作时长，纯历史读取不虚构结束时间。平台消费完整 typed 展示快照，只负责渲染、折叠、outline 可见性与滚动。

## 原生提供商、远端与 HOME

`ExecutionTarget` 明确区分本机和 OpenSSH 目标。运行时可执行文件、Node、会话 cwd、Harness 配置根和历史根均属于目标机器；只把 SSH 客户端路径和待安装的应用资源视为本机路径。版本探测、Pi SDK 验证、历史与执行共用目标传输，不能用本机目录是否存在来判断远端路径。远端需要 Python 3 标准库；OpenSSH 使用既有 host alias、认证与 known-host 配置，启用非交互认证及主机身份检查，不复制 SSH 私钥。

SSH 启动请求通过 stdin 长度帧传入 argv、环境与 cwd，后续 stdin 保留原生协议记录。provider token 不放入 SSH 命令行。远端 supervisor 为 Harness 建立独立进程组，断连后关闭输入并终止该组；脱离该进程组自行 daemonize 的外部工具仍由工具自身负责。一次性版本和历史 helper 使用有界执行，不能将它们的正常输入 EOF 当成会话断连。应用 helper 只由 `install_resources` 从固定白名单及随应用分发的 `node_modules` 安装到显式远端资源目录，不读取 Harness HOME。

本机网关通过该运行时持有的 SSH master 创建动态远端 loopback 反向转发；启动后核对监听实际只在 loopback，释放 guard 时撤销转发。SSH 服务端若强制开放公网监听，或目标平台无法核对监听地址，则明确报错，不把受限隧道降级为开放端口。Pi 的 gateway／隧道由 application 持有，原生 `NativeSession` 持有自身一轮使用的隧道；caller 不得为同一 native injection 再包一层远端转发。

`os_home` 只控制子进程和其工具看到的 `HOME`；`agent_dir` 分别映射 `PI_CODING_AGENT_DIR`、`CODEX_HOME`、`CLAUDE_CONFIG_DIR`、`DSH_HOME`。application 决定采用现有 HOME 还是创建 Velune managed HOME，并显式提供两种路径。本包不读取默认 HOME 推断另一份真实配置，也不复制来源认证。managed HOME 本身不代表已登录。

Claude Code 的每轮执行使用独立 `--print --input-format stream-json --output-format stream-json` 控制进程，首轮 `--session-id`、后续 `--resume` 保留原生会话身份。工具权限通过 `can_use_tool` 转为 conversation 交互，由调用方明确答复。已有 provider 由 Claude 自己解析；managed 轮次设置 `CLAUDE_CODE_PROVIDER_MANAGED_BY_HOST` 与临时 Messages 端点／token，清除该子进程的其它 provider 认证开关，避免来源 `settings.env` 覆盖本轮注入。没有修改、备份或恢复用户 provider 文件。

固定版本启用 `CLAUDE_CODE_SDK_READS_SESSION_STATE`。`result` 只记录轮次结果，原生 `session_state_changed: idle` 确认本轮及其原生后台工作结束后才撤销注入。长期观察任务可能阻止 idle：复用 SDK 的十分钟无后续主线程活动上限，且仅在无待答交互、无 local agent/workflow 任务时关闭进程并将结束状态标为失败，不伪造成功。原生运行时退出、取消或派发失败都释放本轮进程与隧道。Claude 历史只读取显式 projects 根下的原生 JSONL，按 parent UUID 还原当前分支，排除 subagents；不扫描认证文件。

协议依据于 2026-10-10 复核：[Claude Code CLI](https://code.claude.com/docs/en/cli-reference)、[环境变量与 host-managed provider](https://code.claude.com/docs/en/env-vars)、[官方 SDK 控制协议](https://github.com/anthropics/claude-agent-sdk-python/blob/main/src/claude_agent_sdk/_internal/query.py)、[OpenSSH 转发与控制连接](https://man.openbsd.org/ssh.1)。固定版本与隔离实验范围见当前 [任务记录](../../tasks/runtime-provider-ssh/packet.md)。composer 的来源模型选择、SSH/HOME 编辑入口属于 app，明确待实现；本 unit 只提供可消费契约。

## 原生资源管理

`management` 在目标机器上管理固定版本的原生来源文件，不维护第二份资源库，也不计算 Harness 的合并配置。`Config` 只接受版本、`ExecutionTarget`、显式 `agent_dir` 与可选 `cwd`；User 使用配置根，Project 使用项目根。managed HOME 中的资源仍归原生 Harness。读取不启动 Harness、不初始化 HOME；配置目录缺失与能力不支持分别表达。

`catalog` 返回 `Support` 与条目；`EntryState` 区分 Present、Missing、Invalid，`Access` 独立说明 Writable／ReadOnly。`read` 返回选中的字节、revision 和执行位，`write`／`delete` 必须提供 `ExpectedRevision::Missing` 或 `Match`。错误通过 `Error::code()` 保留 conflict、missing、invalid、unsupported、unavailable。声明语法错误可以读取修复；无法定位的共享 MCP section 只能报告源文件问题，不能当成空配置重建。

以下均为本包当前可管理的位置。`A` 是显式 `agent_dir`，`P` 是显式项目 `cwd`；指令括号内为可编辑的其它原生候选，目录顺序不表示实际加载优先级。Claude Agent 目录递归展示源文件，但本轮只验证直属声明加载，不承诺任意嵌套路径的原生发现。

| 固定版本 | User／Project 指令 | User／Project skills 根 | User／Project MCP | User／Project 原生 Agent 定义 |
| --- | --- | --- | --- | --- |
| Codex 0.159.3 | `A/AGENTS.md`、`P/AGENTS.md`（各自的 `AGENTS.override.md`） | `A/skills`／`P/.agents/skills` | `A/config.toml`／`P/.codex/config.toml` 的 `mcp_servers` | `A/agents`／`P/.codex/agents` 下递归 `.toml` |
| Claude Code 2.1.286 | `A/CLAUDE.md`／`P/CLAUDE.md`（项目 `.claude/CLAUDE.md`、`CLAUDE.local.md`） | `A/skills`／`P/.claude/skills` | `A/.claude.json`／`P/.mcp.json` 的 `mcpServers` | `A/agents`／`P/.claude/agents` 下 `.md` |
| Pi 1.0.2 | `A/AGENTS.md`／`P/AGENTS.md`（各自的 `AGENTS.override.md`、`AGENTS.MD`、`CLAUDE.md`、`CLAUDE.MD`） | `A/skills`／`P/.pi/skills` | `A/mcp.json`／`P/.pi/mcp.json` 的 `mcpServers` | 当前没有内置定义格式；扩展自有格式不在范围内 |
| DSH 0.2.0-rc.2 | `A/AGENTS.md`／`P/AGENTS.md`（项目 `CLAUDE.md`、`AGENTS.local.md`、`CLAUDE.local.md`） | `A/skills`／`P/.dsh/skills` | 当前管理适配未支持 Cordis patch／插件声明 | 当前管理适配未支持 Cordis persona／preset 定义 |

通用 ACP v1 未声明这些原生持久资源的管理接口，四类均明确 Unsupported。DSH 的管理限制不表示该 Harness 没有 MCP 或子 Agent 运行能力。其它祖先目录、`~/.agents/skills` 等共享来源、自定义 settings 路径、插件／安装包资源和 Codex 外部 `config_file` 不在此表范围内；同名资源不会从另一个根任意回退。原生 trust、优先级、覆盖、启用开关和重新加载仍由 Harness 决定。

Skill catalog 保留完整目录与附件，普通文件按字节读写；现有执行位默认保留，显式 `executable` 可改变执行位。`SKILL.md` 校验 UTF-8 与 YAML frontmatter 语法，普通附件不强制声明格式。DSH 的 skills 根 `.md` 文件也是声明候选；Pi 只有带有效 description 的根 Markdown 才成为技能，普通 README 仍可作为资产编辑。目录资产并不因此全部成为 Harness 可执行技能。新建 bundle 先写入口，再逐文件添加资产；既有 bundle 修改是单文件操作，不承诺多文件事务或整目录替换。删除仅支持文件与空 skill 目录。

MCP 的 key 是 opaque 服务名，其 `Document.bytes` 仅为该服务的原生 JSON 对象。写入替换该条目，保留未知兄弟服务、provider 字段与其它根配置；Codex TOML 保留非目标注释，Claude／Pi JSON 会规范化排版。两者固定版本的这些配置入口使用严格 JSON，不接受 JSONC／JSON5。调用方应以显式读取的对象编辑，不能将展示用掩码作为真实字段保存；读取可能含该服务自己的 env／header 值，Debug、catalog 和解析诊断不包含正文。管理不连接 MCP、不解析环境引用、不执行脚本，也不改变会话级 `mcp::Server` 注入契约。

资源上限为单文件 16 MiB、每次目录遍历 4096 节点、资源相对路径 64 层。symlink 与特殊节点可见但只读，不跟随读取；超过上限或访问失败有明确错误，不伪装为空目录。本机实现支持 Linux、Android、macOS、iOS、FreeBSD 的目录句柄操作，其它本机目标明确 Unsupported；SSH 使用目标端 Python 3 的 POSIX 文件接口。所有操作在实际目标执行。修改在同次目标操作内复核 revision，单文件替换原子发布；首次创建独占发布，已有文件更新仍是乐观并发控制，不能排除外部编辑器在复核后同时写入。revision 包含内容和权限，MCP revision 覆盖整个共享配置，因此兄弟条目变更也会要求重读。

原生入口和实际加载证据见 [固定版本验收](../../tasks/runtime-management/native-evidence.md)。application 负责 busy／协作 scope 的修改门控；本包不做 UI、自动同步或热重载。App 的资源管理入口明确待实现。
