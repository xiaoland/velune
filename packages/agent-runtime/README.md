# Agent 运行时

本 unit 实现版本化 Agent 运行时适配器：Pi RPC／SDK、Codex app-server、Claude Code stream-json 和 DeepSeek Harness ACP。它负责原生控制、会话读取与消息 projection；真实会话由各 Harness 持久化，Velune 不复制为另一套会话历史。Pi 适配器另提供显式来源的认证交互和只读提供商发现。

它只依赖 [会话契约](../conversation/README.md)，不依赖 AI 服务、网关、application 或 UniFFI。执行配置与网关注入的临时 token 由 application 显式传入；它不选择上游模型路由、保存应用配置，也不拥有原生界面。

`authentication` 与 `provider_source` 通过本包资源中的薄 Pi helper 执行来源操作，返回非秘密的类型化状态和发现结果。application 负责将结果导入网关及原子提交；application 的提供商私有认证解析器通过固定 `pi_auth.mjs` 委托 OAuth 来源解析与刷新，不读取环境里的默认认证。

执行侧通过 `pi_rpc.mjs` 装配固定 SDK。所选 CLI 必须对应 Pi 1.0.2 安装；原运行时目录继续拥有设置、认证、扩展和会话，Velune 将本次网关模型目录作为额外 provider 配置追加到同一 `ModelRuntime`。因此原生 provider 与 Velune gateway 可以共存，Pi SDK 自己负责 provider 注册、模型选择和派发；普通来源扩展注册不会因为不是 Velune provider 而被拦截。来源读取失败保留错误类别、阶段、退出状态和原始原因；SDK 错误输出供本地诊断使用，不以固定说明替换。常规诊断不采集配置文件或会话正文。unit 只发出 tracing 事件，由消费方装配日志订阅器，RPC 读取线程显式继承其 dispatcher 与 span。`velune/auto` 与注入的 `velune-gateway` 由适配器装配；Velune hook 仅处理其 virtual model，不修改原生 provider 的请求。

Pi 来源适配器按固定 SDK 1.0.2 的协议与有效兼容要求判断可导入性。Chat Completions 的已知设置只在当前网关请求能保持其语义时放行；原提供商和 URL 推断的输出字段、推理格式等默认要求也参与判断。`max_tokens` 来源设置通过可选 Pi 投影交给 application 转换为提供商模型的类型化 wire 选项，adapter 不依赖 AI-provider。未知字段或需要未实现 wire 行为的设置逐项说明原因，不将所有非空 compat 对象一概拒绝，也不把旧 Codex 后端或 Messages API 冒充受支持的 OpenAI 协议。

Pi 固定版本及协议边界见 [开发说明](../../docs/development.md)。SDK 与 Node 资源只随需要本地执行的 app 装配，不进入禁用 local-runtime 的绑定产物。

## 版本与原生控制

运行时类型是 versioned variant，family 仅用于展示分组，不作为派发键。当前四个 variant 为 `pi-1.0.2`（family `pi`）、`codex-0.159.3`（family `codex`）、`dsh-acp-0.2.0-rc.2`（family `deepseek-harness`）、`claude-code-2.1.286`（family `claude-code`）。各自版本 regex 是 `^1\.0\.2$`、`^0\.159\.3$`、`^0\.2\.0-rc\.2$`、`^2\.1\.286$`。启动前探测配置的 CLI 版本；协议握手继续检查所需能力。不同 breaking-change 版本应增加独立 variant 与 adapter，不能放宽 regex 假定兼容；多个 variant 与同 variant 的多个实例可并存。

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
