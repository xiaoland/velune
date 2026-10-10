# 应用层

`velune-application` 是平台 app 共用的应用用例 unit。公开入口 `Application` 提供类型化的配置、连接、会话、模型选择、提供商导入与认证操作。平台不自行协调文件提交、子进程状态与投影失败清理。

`config` 定义 schema 7 的应用配置；`repository` 是内部文件设施，负责锁、加载和原子提交。提供商拥有模型条目和私有认证，参数模板仅用于复制填写字段，不参与调用解析。API key 与普通配置在同一文件原子保存，文件创建权限为 0600；公开配置摘要不含 key，只有显式单提供商读取接口返回已保存的值。认证编辑区分保留、更新和清除，读取失败不清空原值。OAuth 保留原来源登录和刷新，来源定位不进入公开摘要。根目录与资源目录由平台通过 `Options` 显式传入。

提供商导入选择已保存的 Agent 运行时实例，应用从其配置确定目录及 Node 路径；调用方不能另填来源配置。预览不解析秘密，应用导入时将静态 API key 保存到提供商私有配置，OAuth 仍委托来源刷新。实例、源目录元数据或目标配置变化使预览失效。读取不要求执行准备或当前会话选择模型。来源文件与 Harness 会话保留，Velune 不新建会话数据库。保存首个使用默认网关的运行时会建立空的默认网关，便于先配置运行时再导入。

默认 `local-runtime` feature 组合本机运行时和 LLM 网关。`local` 按配置、连接、会话、认证协调与 Pi／原生运行时装配分模块。应用捕获提供商私有认证快照，装配异步解析器；解析器校验捕获的协议与服务地址，未知引用不回退环境认证。Unix 来源 helper 的期限、输出上限与进程组取消由应用负责，网关不解释来源或调用 helper。Pi 元数据在装配时剥离为 `velune_gateway::GatewayConfig`，AI package 不依赖运行时。

关闭 `local-runtime` 后，提供商、API key、模型模板和运行时配置的加载与修改仍可使用；本机连接、会话执行、来源导入以及需要 adapter 的认证操作明确返回不支持错误。本构建不链接 Agent runtime；当前配置校验仍依赖 gateway 的配置类型与 AI-provider，不能将其称为完全排除本机 AI 依赖的产物。它不启动网关，也不携带 SSH 子进程执行能力；本机与 SSH 运行时都需要 `local-runtime`。

公开 API 和结果记录定义在 `api.rs`，没有平台 JSON dispatcher。内部用例仍有 JSON 投影转换，不构成跨语言契约。隐藏稳定 `recordKey` 用于模型选择引用；`providerModelId` 是可编辑的精确上游标识，修改它不改变内部身份。能力可以未知，只有具体运行时准备边界检查必需字段。模板不复制内部键、认证、服务地址或 Pi 执行元数据。

只接受 schema 7。更低版本的普通配置原子覆盖为空配置，不备份、不迁移、不操作原 Harness 会话、认证或既有平台秘密；未来版本、损坏 JSON 和当前版本中的未知字段明确报错。

运行时描述包含 familyId、versioned variant ID 与 versionRegex，配置实例引用精确 variant。当前支持 `pi-1.0.2`、`codex-0.159.3`、`claude-code-2.1.286` 与 `dsh-acp-0.2.0-rc.2`；family 不参与控制协议派发。应用协调 Codex app-server／DSH ACP 与 huihua 历史读取，原生 ID 与 runtimeId 共同形成公开会话身份；读取成功不等于原生恢复成功。运行时添加后直接可读历史，不要求模型或执行准备。新建显式给出会话模型并自动准备；历史视图选择模型后，在发送时按需恢复执行。Pi 历史中的 Velune 模型标记只恢复 managed 引用；原生目录提供的模型通过显式 native 选择发送，不另存会话状态。

原生审批、问题回答及取消通过 RuntimeInteraction／RuntimeInteractionReply 完成，配置变更、执行和失败清理由应用协调。提供商导入和订阅来源认证仍只支持 Pi 适配器，增加 Codex／DSH 执行不自动增加其提供商导入。DSH 模型能力注入只使用已知字段，reasoning wire 映射未实现，不将能力列表解释为请求参数。

公开模型目录的获取和来源解析归本 unit。当前支持用户显式读取 models.dev 的 provider-scoped API，以具名候选记录返回来源提供商、实际模型 ID 和已声明能力，供模板快填；候选不进入配置，用户保存时仍走现有模板用例。同步获取由平台工作队列调用，单次请求的 executor 随调用结束释放，不建立常驻 Host 或全局 subscriber。目录获取不使用提供商凭据，不读取本机运行时配置、不自动刷新或改变协议／地址；未知规格保持未知。

会话列表汇总全部启用运行时，实例 disabled 不读取其历史来源，保留其配置与原生历史；enabled 默认 true 作为当前 schema 的增量配置。当前执行实例与浏览筛选分别管理，不按执行实例隐式限制列表。

跨运行时发送通过同一 `send_turn` 接纳下一轮目标，创建目标原生会话并以明确引用文本交接上下文。逻辑会话保持公开身份，并以首个实际原生段的标题为权威，`context_runtime_id` 表示当前原生查询／执行上下文，不能从列表的来源实例推断。`local/continuation.rs` 装配有序投影，封闭段从权威历史计算截止数量和内容摘要；原生前缀变更明确拒绝，截止后的追加不进入旧段。每次回切创建新原生段，不恢复旧执行状态。

`conversation-links.json` 是 schema 1 的关联文件，独立于 schema 7 普通配置，只保存原生引用、截止位置及交接记录定位，不保存消息或凭据。公开身份可以是单目标别名；后续交接继续向同一关联追加原生段。目标准备成功后原子保存关联，再发送；未知发送结果不自动重试。重开以各原生历史重建消息，精确校验交接记录后只显示本轮用户输入，不递归复制交接包。首版交接仅携带文本及完成的工具结果引用；工具执行、审批和签名推理状态不重放。

当前进程成功新建的 Codex、Claude Code 和 Pi 草稿，在首次发送前切换模型或实例时可以重建实际目标，并以单目标关联保留公开身份。该资格来自明确的新建生命周期，不能从空消息或历史缺失推断。目标创建及关联原子提交成功后才发送；创建失败保留草稿身份供显式重试，关联提交失败保留已创建的活动目标而不发送。第一次发送调用即消耗草稿资格，失败或断连不自动重建或重发。重启后目标尚无原生历史时显示不可发送状态；确认目标不存在后，显式删除可撤除这条单目标关联。未提交且未关联的内存草稿不保证跨重启保留。DSH 新建时已有原生记录，继续使用普通历史接续。

Pi 草稿的原生重命名可能尚未落盘，因此重命名不消耗未提交资格。同一 Pi 实例重建时转交该明确的原生标题；切到没有原生命名接口的 Harness 时，以新目标的实际标题为准，不建立独立的 Velune 标题存储。单目标别名不显示 Harness 交接通知。

摘要的 `can_rename`／`can_delete` 是 application 装配的原生管理能力。逻辑重命名作用于首段标题；删除逐段执行，确认全部完成后移除关联，部分失败记录已完成项并保留未完成关系。缺少原生删除能力时不以本地隐藏替代。当前尾段不可读取时保留已校验的来源正文并关闭发送，修复原生来源后重新打开；不存在自动重发或伪造 resume。

分析记录由 application 在配置根目录的 `analytics.sqlite` 独立维护，后台有界写入与应用生命周期共同关闭。`analytics_query` 接收明确 UTC 区间及平台自然日边界，返回具名概览、趋势、提供商／模型拆分与有界请求页。明细页不截断概览人口，未知计量和存储告警明确返回；统计不可用不阻止应用配置与执行。具体口径归 [AI 服务设计](../../docs/design/ai-service.md#用量与性能分析契约)。

会话阅读与执行身份分开：`read_conversation`只读指定来源的原生历史／关联投影，不切换活跃runner；相同活动身份返回当前投影。`snapshot`同时接收上下文运行时和会话ID，非当前活动身份返回None。`send_turn`接收来源运行时／会话ID、目标运行时／模型和文本，在同一个核心操作中激活指定来源再派发，不能用全局最后打开的会话代替发送来源。调用方可为多个工作区保存阅读投影，但共享核心仍只有一个活跃执行。

`ModelSelection` 区分 `Managed { model_record_key }` 与 `Native { provider_id, model_id }`。`runtime_models` 从指定实例读取原生模型目录，并附加该实例网关中的 managed 模型；原生标识不经过网关查找，也不要求已有提供商支持并存。Pi 和 DSH 的 native 选择必须指定 provider ID；Claude Code 使用既有单一 provider，不能另传 provider ID；Codex 可省略 provider ID 以使用现有默认 provider。`create_conversation_with_selection` 和 `send_turn_with_selection` 接受两类选择；原 `create_conversation`／`send_turn` 是合法的 managed 便捷入口。`next_turn_model_selection` 表示当前上下文的下一轮选择意图，managed turn 结束后仍可保持 managed，不声称进程仍在使用网关。snapshot 的 `model_record_key` 仅表示 managed 模型，native 选择返回 None。

managed 发送只在本轮执行子进程注入网关；正常结束、失败或取消完成后关闭该执行进程和隧道，原生配置文件保持原状。下一轮 native 选择重新以原生 provider 启动。跨实例交接沿用原生历史和关联机制，模型来源不改变交接资格；调用方必须为目标实例显式提供其 native 或 managed 选择，不把来源实例的模型身份自动解释为目标模型。

`RuntimeExecution` 通过 `runtime_execution`／`configure_runtime_execution` 读取与修改；`upsert_runtime_with_execution` 在同一次保存中接纳实例及 typed execution，新建 managed HOME 实例不需要先填假的 agentDir。`RuntimeTransport` 区分 Local 与 Ssh；SSH 使用显式 OpenSSH 入口、目标和可选端口，认证和已知主机仍由用户的 OpenSSH 管理。所有 binary、agentDir、sessionDir、cwd 与 existing HOME 均属于执行机器；远端版本、历史、草稿检查和写入都经 transport 完成，不落回本机同名路径。SSH 实例要求目标具备 Python 3，并给出独立的 `remote_resources_directory`，application 每次打开期间首次使用时投送已知 helper 与随包解析依赖；网关通过有生命周期的远端 loopback 转发访问。

`RuntimeHome` 区分 Inherit、Existing 和 Managed。本机 Managed HOME 稳定分配到 `VELUNE_HOME/runtime-homes/<实例摘要>`；远端必须提供远端绝对根目录，实例目录在该根下派生。managed HOME 内再建立对应 Harness 的原生配置目录，不复制既有认证或会话，也不随普通配置重置或运行时删除而移除。HOME 不隔离项目设置、系统配置或系统凭据存储，因此不是安全沙箱。新配置经唯一 `runtime_execution` 模块编码在现有 settings 中，不改变 schema 7 的结构或既有绑定 record。

本轮仅提供 Rust／UniFFI 能力；**待实现**：平台 composer 展示并选择 native／managed 模型，以及 SSH、远端资源目录和 HOME 的原生配置界面。提供商导入当前仅接受默认本机 execution 配置；SSH、existing HOME 覆盖或 managed HOME 仍可直接执行原生 provider，但不把其目录交给本机来源认证 helper。指定 WSL 主机的真实 SSH 传输已通过合成 Harness 闭环；真实提供商、远端真实 Harness 和前端体验仍待验收，证据见 [本轮任务](../../tasks/runtime-provider-ssh/packet.md)。
