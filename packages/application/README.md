# 应用层

`velune-application` 是平台 app 共用的应用用例 unit。公开入口 `Application` 提供类型化的配置、连接、会话、模型选择、提供商导入与认证操作。平台不自行协调文件提交、子进程状态与投影失败清理。

多个会话可以同时执行。每个打开的会话有自己的执行、模型选择、投影与协作范围；以会话 ID 寻址的操作（打开、发送、取消、回复交互、重命名、删除、snapshot）作用于该会话，每次操作都推进所有会话。每个会话同一时刻最多一个 turn；`cancel` 与 `reply_runtime_interaction` 需要会话 ID。重开正在执行的会话返回其运行中的视图，结束后再打开才重读原生历史。执行中、有待处理交互或有协作范围的会话保持打开，空闲会话按最近使用保留 8 个。同一运行时的并发执行各用一个 lane 目录（`runtime-projections/<运行时>/lane-<n>`）存放网关模型与选择文件。`active_conversations` 列出执行中或待处理交互的会话，可频繁轮询。运行时、提供商、导入与登录配置在任一会话执行或存在协作范围时拒绝变更。[并行会话手动验收](../../scripts/manual-parallel-sessions.py) 用隔离 Pi 1.0.2 与本机 loopback 上游检查同时执行、独立结束、取消与配置门禁。

`runtime_resource_catalog`、`read_runtime_resource`、`write_runtime_resource` 和 `delete_runtime_resource` 管理指定实例的原生 user instructions、MCP server 条目、skills 与原生 Agent 定义。调用方明确给出 User／Project 范围；Project 的 cwd 属于执行机器。目录是 adapter 已知的原生来源，不是 Harness 合并后的最终配置，也不扫描任意命令猜测其资源格式。catalog 将功能不支持与资源 Missing／Invalid 分开，并单独说明可写或只读；不支持原因可能是原生功能缺失、协议没有入口或适配器尚未实现。

资源查询不会启动 Harness、创建资源根目录或安装辅助 bundle；停用实例仍可管理。本机／SSH 与 managed HOME 复用同一位置解析，但不要求与资源读写无关的执行路径。写入在父 turn 忙碌或协作 scope 存在时拒绝，不隐式关闭空闲会话；保存只表示原生文件已变更，生效时间由 Harness 决定，不宣称进行了 live reload。

资源 key 是类别根目录内的相对路径，MCP key 则是 opaque server name。读取返回单文件 bytes 或单个 MCP 条目的原生 JSON；内容可能含私有设置，不进入 Debug 或日志。写入必须提供 `RuntimeResourceRevision::Missing` 或此前读取的 `Match` revision，外部修改返回可识别的 conflict；混合原生配置由 adapter 只修改目标条目并保留其余内容。Skills 按文件和资产管理，catalog 包含目录及资产；先创建 SKILL.md 再添加资产，不提供整包替换，目录只允许在为空时删除。原生 Agent 定义与协作 scope 的 target grants、运行中的 child 和权限彼此独立，编辑定义不会启动或授权子任务。

**待实现**：平台调用原生资源入口的浏览、编辑、冲突处理和敏感内容展示；本轮没有 App 改动，也没有新增资源副本配置库。

[资源 API 手动验收](../../scripts/manual-runtime-management-api.py) 接收本次构建的动态库与 Python 绑定目录，在隔离 HOME 中检查 typed CRUD、revision 冲突和会话生命周期门禁；默认只需空资源目录。它内置合成 ACP peer，不代替原生 Harness 的实际资源加载验收。

`create_collaborative_conversation` 接纳父运行时、模型选择、已授权目标和子任务数量／轮次期限。每个目标固定运行时实例、cwd 和模型选择，建立 scope 时捕获执行与提供商配置；Agent 只能引用 opaque target ID，不能通过协作工具修改 binary、环境、cwd 或模型。scope 存在时拒绝运行时、提供商和登录配置变更，调用方先关闭协作范围，再编辑配置。

父 Agent 通过临时 MCP 使用 `agents`、`spawn`、`send`、`status`、`wait`、`cancel` 和 `close`。每个子任务 worker 独占 adapter、轮询、网关和执行覆盖目录，共享配置实例原有 HOME 与原生历史；父 turn 等待工具时子任务仍可运行。首版允许一层并行委派和空闲子任务的多轮发送，不排队、不 steer、不 adopt。委派文字带有可见来源说明，作为普通消息提交，不冒充人类授权。返回的最终助手正文是子 Agent 的报告，不是独立验收结论。没有 load 能力的 ACP 也可作活动子任务，关闭后不承诺恢复。

宿主通过 `collaboration_children`、`child_snapshot` 与绑定 child／turn／interaction ID 的 `reply_child_interaction` 处理子任务权限。父 Agent 没有批准子权限的工具；`wait` 遇到待审批立即返回。父 turn 结束保留子任务，父取消会级联取消，显式 `close_collaboration` 撤销范围并清理子任务。普通 shutdown 在父或子任务忙碌时拒绝；成功关闭或直接释放应用时，先清理执行对象，再释放配置目录独占锁。

轮次期限在 worker 中检查；adapter 的同步启动与控制请求仍受各自有界超时约束，不承诺硬实时中断。启动返回后若已经取消或过期，不再提交 prompt；未知结果不自动重试。协作关系和结果只在本次应用生命周期内保存，原生历史仍由 Harness 负责。**待实现**：平台授权目标、子任务状态和子权限审批界面；本轮不修改 App。

`config` 定义 schema 8 的应用配置；`repository` 是内部文件设施，负责锁、加载和原子提交。提供商拥有模型条目和私有认证，参数模板仅用于复制填写字段，不参与调用解析。API key 与普通配置在同一文件原子保存，文件创建权限为 0600；公开配置摘要不含 key，只有显式单提供商读取接口返回已保存的值。认证编辑区分保留、更新和清除，读取失败不清空原值。OAuth 保留原来源登录和刷新，来源定位不进入公开摘要。根目录与资源目录由平台通过 `Options` 显式传入。

提供商导入选择已保存的 Agent 运行时实例，应用从其配置确定目录及 Node 路径；调用方不能另填来源配置。预览不解析秘密，应用导入时将静态 API key 保存到提供商私有配置，OAuth 仍委托来源刷新。实例、源目录元数据或目标配置变化使预览失效。读取不要求执行准备或当前会话选择模型。来源文件与 Harness 会话保留，Velune 不新建会话数据库。保存首个使用默认网关的运行时会建立空的默认网关，便于先配置运行时再导入。

默认 `local-runtime` feature 组合本机运行时和 LLM 网关。`local` 按配置、连接、会话、认证协调与 Pi／原生运行时装配分模块。应用捕获提供商私有认证快照，装配异步解析器；解析器校验捕获的协议与服务地址，未知引用不回退环境认证。Unix 来源 helper 的期限、输出上限与进程组取消由应用负责，网关不解释来源或调用 helper。Pi 元数据在装配时剥离为 `velune_gateway::GatewayConfig`，AI package 不依赖运行时。

关闭 `local-runtime` 后，提供商、API key、模型模板和运行时配置的加载与修改仍可使用；本机连接、会话执行、来源导入以及需要 adapter 的认证操作明确返回不支持错误。本构建不链接 Agent runtime；当前配置校验仍依赖 gateway 的配置类型与 AI-provider，不能将其称为完全排除本机 AI 依赖的产物。它不启动网关，也不携带 SSH 子进程执行能力；本机与 SSH 运行时都需要 `local-runtime`。

公开 API 和结果记录定义在 `api.rs`，没有平台 JSON dispatcher。内部用例仍有 JSON 投影转换，不构成跨语言契约。隐藏稳定 `recordKey` 用于模型选择引用；`providerModelId` 是可编辑的精确上游标识，修改它不改变内部身份。能力可以未知，只有具体运行时准备边界检查必需字段。模板不复制内部键、认证、服务地址或 Pi 执行元数据。

只接受 schema 8。更低版本的普通配置原子覆盖为空配置，不备份、不迁移、不操作原 Harness 会话、认证或既有平台秘密；未来版本、损坏 JSON 和当前版本中的未知字段明确报错。

运行时描述包含 familyId、versioned variant ID 与 versionRegex，配置实例引用精确 variant。专用适配器支持 `pi-1.0.2`、`codex-0.159.3`、`claude-code-2.1.286` 与 `dsh-acp-0.2.0-rc.2`；通用 `acp-v1` 表示协商的协议版本，不限定 Agent 品牌或 CLI 版本。family 不参与控制协议派发。应用协调 Codex app-server／DSH ACP 与 huihua 历史读取，原生 ID 与 runtimeId 共同形成公开会话身份；读取成功不等于原生恢复成功。运行时添加后直接可读历史，不要求模型或执行准备。新建显式给出会话模型并自动准备；历史视图选择模型后，在发送时按需恢复执行。Pi 历史中的 Velune 模型标记只恢复 managed 引用；原生目录提供的模型通过显式 native 选择发送，不另存会话状态。

原生审批、问题回答及取消通过 RuntimeInteraction／RuntimeInteractionReply 完成，配置变更、执行和失败清理由应用协调。提供商导入和订阅来源认证仍只支持 Pi 适配器，增加 Codex／DSH 执行不自动增加其提供商导入。DSH 模型能力注入只使用已知字段，reasoning wire 映射未实现，不将能力列表解释为请求参数。

公开模型目录的获取和来源解析归本 unit。当前支持用户显式读取 models.dev 的 provider-scoped API，以具名候选记录返回来源提供商、实际模型 ID 和已声明能力，供模板快填；候选不进入配置，用户保存时仍走现有模板用例。同步获取由平台工作队列调用，单次请求的 executor 随调用结束释放，不建立常驻 Host 或全局 subscriber。目录获取不使用提供商凭据，不读取本机运行时配置、不自动刷新或改变协议／地址；未知规格保持未知。

会话列表汇总全部启用运行时，实例 disabled 不读取其历史来源，保留其配置与原生历史；enabled 默认 true 作为当前 schema 的增量配置。当前执行实例与浏览筛选分别管理，不按执行实例隐式限制列表。

跨运行时发送通过同一 `send_turn` 接纳下一轮目标，创建目标原生会话并以明确引用文本交接上下文。逻辑会话保持公开身份，并以首个实际原生段的标题为权威，`context_runtime_id` 表示当前原生查询／执行上下文，不能从列表的来源实例推断。`local/continuation.rs` 装配有序投影，封闭段从权威历史计算截止数量和内容摘要；原生前缀变更明确拒绝，截止后的追加不进入旧段。每次回切创建新原生段，不恢复旧执行状态。

`conversation-links.json` 是 schema 1 的关联文件，独立于 schema 8 普通配置，只保存原生引用、截止位置及交接记录定位，不保存消息或凭据。`composer-drafts.json`（schema 1，0600）按会话公开身份保存未发送的 composer 文字，同样独立于普通配置、不随配置重置；空文字删除条目，删除会话时一并删除，尚未首发的新建会话不写入。公开身份可以是单目标别名；后续交接继续向同一关联追加原生段。目标准备成功后原子保存关联，再发送；未知发送结果不自动重试。重开以各原生历史重建消息，精确校验交接记录后只显示本轮用户输入，不递归复制交接包。首版交接仅携带文本及完成的工具结果引用；工具执行、审批和签名推理状态不重放。

当前进程成功新建的 Codex、Claude Code 和 Pi 草稿，在首次发送前切换模型或实例时可以重建实际目标，并以单目标关联保留公开身份。该资格来自明确的新建生命周期，不能从空消息或历史缺失推断。目标创建及关联原子提交成功后才发送；创建失败保留草稿身份供显式重试，关联提交失败保留已创建的活动目标而不发送。第一次发送调用即消耗草稿资格，失败或断连不自动重建或重发。重启后目标尚无原生历史时显示不可发送状态；确认目标不存在后，显式删除可撤除这条单目标关联。未提交且未关联的内存草稿不保证跨重启保留。DSH 新建时已有原生记录，继续使用普通历史接续。

Pi 草稿的原生重命名可能尚未落盘，因此重命名不消耗未提交资格。同一 Pi 实例重建时转交该明确的原生标题；切到没有原生命名接口的 Harness 时，以新目标的实际标题为准，不建立独立的 Velune 标题存储。单目标别名不显示 Harness 交接通知。

摘要的 `can_rename`／`can_delete` 是 application 装配的原生管理能力。逻辑重命名作用于首段标题；删除逐段执行，确认全部完成后移除关联，部分失败记录已完成项并保留未完成关系。缺少原生删除能力时不以本地隐藏替代。当前尾段不可读取时保留已校验的来源正文并关闭发送，修复原生来源后重新打开；不存在自动重发或伪造 resume。

分析记录由 application 在配置根目录的 `analytics.sqlite` 独立维护，后台有界写入与应用生命周期共同关闭。`analytics_query` 接收明确 UTC 区间及平台自然日边界，返回具名概览、趋势、提供商／模型拆分与有界请求页。明细页不截断概览人口，未知计量和存储告警明确返回；统计不可用不阻止应用配置与执行。具体口径归 [AI 服务设计](../../docs/design/ai-service.md#用量与性能分析契约)。

会话阅读与执行身份分开：`read_conversation`只读指定来源的原生历史／关联投影，不切换活跃runner；相同活动身份返回当前投影。`snapshot`同时接收上下文运行时和会话ID，非当前活动身份返回None。快照结果是交付形式 `DeliveredSnapshot`：超过 2KB 的推理与工具输出只携带字节数，由 `message_payloads` 按消息读取；`snapshot_update(since)` 依据上次交付游标返回未变化或只含变化消息的快照，供展示客户端轮询。`send_turn`接收来源运行时／会话ID、目标运行时／模型和文本，在同一个核心操作中激活指定来源再派发，不能用全局最后打开的会话代替发送来源。调用方可为多个工作区保存阅读投影，但共享核心仍只有一个活跃执行。

`ModelSelection` 区分 `Managed { model_record_key }`、`Native { provider_id, model_id }` 和通用 ACP 的 `RuntimeDefault`。专用适配器的 `runtime_models` 从指定实例读取原生模型目录，并附加该实例网关中的 managed 模型；通用 ACP 只提供当前会话的原生模型选项。原生标识不经过网关查找，也不要求已有提供商支持并存。Pi 和 DSH 的 native 选择必须指定 provider ID；Claude Code 使用既有单一 provider，不能另传 provider ID；Codex 可省略 provider ID 以使用现有默认 provider。`create_conversation_with_selection` 和 `send_turn_with_selection` 接受上述选择；原 `create_conversation`／`send_turn` 是合法的 managed 便捷入口。`next_turn_model_selection` 表示当前上下文的下一轮选择意图，managed turn 结束后仍可保持 managed，不声称进程仍在使用网关。snapshot 的 `model_record_key` 仅表示 managed 模型，native 选择返回 None。

managed 发送只在本轮执行子进程注入网关；正常结束、失败或取消完成后关闭该执行进程和隧道，原生配置文件保持原状。下一轮 native 选择重新以原生 provider 启动。跨实例交接沿用原生历史和关联机制，模型来源不改变交接资格；调用方必须为目标实例显式提供其 native 或 managed 选择，不把来源实例的模型身份自动解释为目标模型。

`RuntimeExecution` 通过 `runtime_execution`／`configure_runtime_execution` 读取与修改；`upsert_runtime_with_execution` 在同一次保存中接纳实例及 typed execution，新建 managed HOME 实例不需要先填假的 agentDir。`RuntimeTransport` 区分 Local 与 Ssh；SSH 使用显式 OpenSSH 入口、目标和可选端口，认证和已知主机仍由用户的 OpenSSH 管理。所有 binary、agentDir、sessionDir、cwd 与 existing HOME 均属于执行机器；远端版本、历史、草稿检查和写入都经 transport 完成，不落回本机同名路径。SSH transport 要求目标具备 Python 3。专用适配器还需独立的 `remote_resources_directory`，application 每次打开期间首次使用时投送已知 helper 与随包解析依赖；网关通过有生命周期的远端 loopback 转发访问。

`RuntimeHome` 区分 Inherit、Existing 和 Managed。本机 Managed HOME 稳定分配到 `VELUNE_HOME/runtime-homes/<实例摘要>`；远端必须提供远端绝对根目录，实例目录在该根下派生。managed HOME 内再建立对应 Harness 的原生配置目录，不复制既有认证或会话，也不随普通配置重置或运行时删除而移除。HOME 不隔离项目设置、系统配置或系统凭据存储，因此不是安全沙箱。新配置经唯一 `runtime_execution` 模块编码在现有 settings 中，不改变 schema 7 的结构或既有绑定 record。

本轮仅提供 Rust／UniFFI 能力；**待实现**：平台 composer 展示并选择 native／managed 模型，以及 SSH、远端资源目录和 HOME 的原生配置界面；通用 ACP 的命令、参数、环境配置、协商能力、外部登录提示与会话配置编辑同样待平台接入。提供商导入当前仅接受默认本机 execution 配置；SSH、existing HOME 覆盖或 managed HOME 仍可直接执行原生 provider，但不把其目录交给本机来源认证 helper。指定 WSL 主机的真实 SSH 传输已通过合成 Harness 闭环；真实提供商、远端真实 Harness 和前端体验仍待验收，证据见 [本轮任务](../../tasks/runtime-provider-ssh/packet.md)。

通用 ACP 实例通过 `upsert_acp_runtime` 一次保存 `RuntimeInstance`、`RuntimeExecution` 和 `AcpLaunch`。实例只要求 `settings.binary` 为目标机器上的绝对路径；`AcpLaunch.args` 作为独立 argv 传递，不通过 shell 解析，`environment` 由用户显式配置，`acp_launch` 提供 typed 读取。环境值可能含秘密，与普通应用配置一起保存，Debug 仅显示参数数量或配置键，不输出值。应用不复制原 HOME 的认证，不自动执行登录、安装 Agent 或下载 bridge；用户已安装的原生 ACP Agent 或第三方 bridge 均使用同一协议适配器。

ACP 不要求 Node、agentDir、特定历史目录或 helper bundle；SSH 的 `remote_resources_directory` 可省略，`Options.resources_directory` 可以是已有空目录。Managed HOME 只创建该实例稳定的 HOME，不替 Agent 指定私有配置子目录。ACP 初始化直接协商协议，不运行专用 Harness 的版本探测。

`acp_runtime_info` 返回 Agent 名称／版本、认证方式描述和实际会话能力。认证信息仅用于说明外部登录途径，不表示 Velune 已认证。`RuntimeDefault` 表示保留 Agent 当前会话的模型选择；无模型选项的 Agent 仍可创建并连续发送。`runtime_models` 只读取活动 ACP 会话的首个 model 配置项，未建立会话时明确返回不支持，绝不为探测创建隐藏会话。没有 model 类别的活动会话返回空候选。`acp_session_config`／`set_acp_session_config` 保留原配置 ID、opaque value、类别、分组和 boolean／select 类型，类别可缺省或重复；修改配置和模型继续使用同一活动进程。

ACP 的列表能力缺失与空列表不同；汇总列表通过 `history_failures` 保留不支持说明，当前活动会话仍可显示与使用。`open_acp_conversation(runtime_id, native_session_id, cwd)` 按明确引用加载会话，不依赖列表能力；普通 `open_conversation` 可从 Agent 列表取得真实 cwd。只有 `session/load` 的原生回放可用于读取历史；`resume` 能力不等于历史读取。无 load 的 Agent 在当前进程中支持多轮、权限答复与取消，关闭后的重开明确不支持，不创建新会话冒充恢复。重命名没有通用协议入口，删除按协商能力执行。

同一公开身份的持久跨 Harness 接续要求 ACP 来源和目标都能加载原生历史。能力不足在关闭来源、创建目标或发送之前拒绝，原活动对话保持可用；这不限制普通 ACP 新建或发送。关联段保存真实 cwd 以定位无列表能力的 ACP 会话，仍不保存消息副本。专用 adapter 的 provider overlay 不推广到 ACP，通用 ACP 明确拒绝 managed 模型注入。

资源库（2026-10-11，内部类型仍名 `ResourceTemplate`）：资源是 canonical 定义，保存于 `resource-templates.json`（0600，与普通配置分离、不随配置重置；列表摘要不含内容，`read_resource_template` 显式返回含秘密的内容，Debug 一律遮盖）；`resource-bindings.json` 记录资源在各运行时写入的条目及指纹。`save_resource_template` 保存后立即写入已启用且处于同步状态的运行时，原生被改／被删的运行时等待用户处置，运行中轮次或协作 scope 存在时全部留作“待同步”。`resource_sync_status` 返回每个资源在每个运行时的状态（未启用、不支持、已同步、待同步、原生被改、原生被删）；`apply_resource_template` 启用或重试同步，同名非受管条目或另一份指令需显式 `overwrite`；`resolve_resource_binding` 覆盖、停止同步（保留原生）或删除原生。`unmanaged_runtime_resources` 列出各运行时未被资源库管理的用户级条目，`import_runtime_resource` 尽力读回为未保存的资源并列出无法带入的字段，`adopt_runtime_resource` 将其加入资源库并启用于来源运行时（原生改写为规范形式）。`resource_drift` 返回原生被改／被删时逐文件的原生与资源库内容（MCP 为排版后的 JSON），供用户覆盖前比较。保存时若运行中轮次或协作 scope 使写入延后，核心记下待补写，在后续请求中发现运行时空闲后自动写入待同步的绑定。MCP 资源含可选工作目录与超时（秒），各运行时按原生字段渲染，无法表达的字段在写入报告中说明。写操作遵守资源写入闸门；删除资源或运行时实例只丢弃启用记录，不触碰原生条目。手动验收见 `scripts/manual-resource-library.py`；真实 Harness 加载见 `scripts/manual-resource-library-native.py`。
