# 跨语言接入

`velune-bindings` 是本地 application 与远程客户端共用的 UniFFI 接入层。Swift、Kotlin 和 Python 使用同一份 Rust 元数据生成接口。领域 package 不依赖 UniFFI；此处的 `Binding*` records 与 enums 将领域契约转换为语言绑定能够表达的类型。Mac 在应用进程内加载此 unit 的动态库，不通过 Host 或 socket 调用 Rust。

Kotlin 生成目标为 Android，`uniffi.toml` 固定 `bindings.kotlin.android=true`，生成物依赖 Android API 与 AndroidX 注解。UniFFI 在 API 34 及以上使用 Android `SystemCleaner`，较低 API 使用 JNA Cleaner，以支持当前 Android 端的 API 28 下限。

原生资源通过 `runtime_resource_catalog`／`read_runtime_resource`／`write_runtime_resource`／`delete_runtime_resource` 管理。`BindingRuntimeResourceKind`、scope、key、state、access 和 revision 均为具名类型；资源内容用 bytes 保留原生文件或单条 MCP JSON，Rust Debug 隐去正文。写入传 Missing 或此前的 Match revision，冲突保留为 `BindingFailureKind::RuntimeManagement`、`phase=runtime_management`、`code=conflict`，不退化为普通字符串错误。原生 Agent 定义不是活动会话。**待实现**：平台资源管理与冲突处理界面；资源保存不自动重启会话。

Agent 间协作没有专门的 bindings API：参与方是普通会话，平台用现有会话、消息与交互接口呈现；MCP 工具、票据与收件箱归 application（见其 README）。

`VeluneApplication` 的同一组具名接口用于配置维护、会话控制、提供商导入和认证交互。本地构造持有 `ApplicationService`，远程构造只持有连接；命令与结果的权威类型归 `application-api`，此处只转换 UniFFI 表示。提供商拥有模型条目；`BindingProviderDraft` 与 `BindingAuthenticationEdit` 共同保存普通字段和认证编辑，后者明确区分保留、更新 API key 和清除，认证来源由 `BindingProviderDraft.authentication_source` 选择。列表与 snapshot 只返回非秘密认证描述，`read_provider_api_key` 显式读取一个提供商的已保存 key。认证检查和登录以网关／提供商 ID 定位，不存在独立认证资源 CRUD。提供商模型的公开 DTO 不含 adapter 私有 Pi metadata，平台不读取或维护该内部字段。

Codex 订阅的前端接入复用 `authentication_start`／`authentication_poll`／`authentication_cancel`／`authentication_inspect`，无需配置运行时。先以 `authentication_source=CodexOAuth`、`execution_backend=Http`、`request_profile=CodexResponses` 保存提供商（Responses 协议、`https://chatgpt.com/backend-api/codex`），再开始登录；再次保存原认证使用 `Keep`，更换认证来源或选择 `Clear` 会重置所选来源认证。`BindingProviderAuthentication.disclosure` 的 `title`／`detail` 应在选择认证和登录入口呈现；它如实说明 Codex 客户端身份及规则冲突。选择前没有已保存的认证描述，平台用静态的 `provider_templates()` 取得有序模板目录（`BindingProviderTemplate`）：每项给出 ID、订阅／API 种类、名称与简介、协议、地址、四项组合字段、需在本机定位的命令（`executable_command`）及同一披露，平台以此呈现添加提供商的选择，不自行罗列订阅或硬编码组合。目录直接来自 `application-api`，因此 remote-only 产物同样可用，不装配本地执行栈。`AuthenticationProgress.events` 中 `type_id="notify"` 的 notification 提供 `kind="url"`、`url` 和说明，平台打开系统浏览器并保留手动打开入口；URL 与回调参数不进入日志或分析。持续 poll 到 `running=false`，最终 `result` 表示成功、失败或取消；成功使用返回的 gateways 更新视图。前端不读写秘密文件，也不自行兑换、刷新或缓存 bearer。`account_label` 只显示已知绑定账户 ID 的尾 8 位，失效后仍可保留，用于区分多个提供商登录；它不代表有效认证。`configured=false` 表示尚未登录或需重新登录；`configured=true` 仍须实际推理验收。


执行 owner 串行接纳业务操作；远控对象使用短状态锁与独立 QUIC 流，等待一个业务操作或上传附件不会锁住同对象的查询、取消和审批入口。多个会话可同时执行，`cancel`／`reply_runtime_interaction` 以运行时与会话 ID 寻址，`working_changes`／`file_diff` 返回会话工作目录的 git 变更（`BindingWorkingChanges`、`BindingChangeKind`）与按行分类的差异（`BindingFileDiff`）。`conversation_activity(since)` 返回 `BindingConversationActivity`：执行中、待处理交互或按 Harness 存活记录由其他进程运行（`BindingRunState::Elsewhere`，展示为运行中）的会话，以及游标之后结束的 turn（`BindingTurnOutcome`）和下次使用的游标。`shutdown` 在应用忙碌时返回错误并保留对象，调用方可继续读取 snapshot 或明确取消 Agent 执行；关闭成功后除再次关闭外的操作返回 `Closed` 类型的诊断。释放语言包装对象负责释放其 Rust 引用，不代表用户发出了取消 Agent 的命令。会话状态仍通过 `snapshot` 或游标式 `snapshot_update` 观察；`ApplicationService` 在执行应用内主动推进状态，远程控制连接退出后工作继续。`BindingMessagePayload` 区分内联内容与只带字节数的延迟内容，`message_payloads` 读取后者。`BindingMessageBlock` 的 `Reasoning` 带摘要与其下正文，`Image` 带 `BindingImageSource`（本机路径或可延迟的 base64 内容），`File` 是用户消息附带的文件路径。

本 unit 在 `home_directory/logs` 装配对象专属的 tracing subscriber，不占用进程全局 subscriber。操作失败的 `BindingError.Diagnostic` 保留分类、白名单 code/phase、说明和 operation ID，平台使用该 ID 关联日志；领域包不依赖这一日志输出或 UniFFI。日志安全字段、轮转和后续 layer 扩展见 [开发说明](../../docs/development.md#本地诊断)。

默认启用 `local-runtime` 与 `remote`。`local-runtime` 提供 `VeluneApplication.open(BindingOptions)` 并链接本地 application、Harness、gateway 与提供商执行；本机和 SSH Harness 都需要它。纯控制端使用 `--no-default-features --features remote`，只依赖契约、远控和 UniFFI，不链接 application、agent-runtime、gateway、ai-provider 或 Node 执行资源。领域类型中的执行位置只是业务配置，不代表控制设备执行它。

`VeluneRemoteEndpoint.open(options, application)` 由平台显式指定身份文件、可信设备文件和设备名称；两个文件路径都必须绝对，平台先创建其私有父目录。提供本地 `VeluneApplication` 时入口服务该对象已经持有的唯一执行核心；传 `None` 创建控制端，不打开执行设备配置。`BindingRemoteOptions` 明确选择 relay、地址发现、绑定地址、连接超时和 `path_policy`。正常采用 `Automatic`；`RelayOnly` 用于受控连通诊断，不是产品默认。`DevelopmentPublic` 需显式选择，只代表公共开发设施；`Custom` 可接入自部署 relay，`CustomPkarr` 可接入自部署 HTTPS 地址发现服务。地址发现与建连协调同持续业务 relay 是不同职责：默认推荐 relay 辅助建连后尝试直连，直连成功后业务可走 Direct；`Disabled` 只保留已知地址与现有原生发现选择，不能据单机回环结果声称无需公网协调且能穿透所有 NAT。本轮未提供已运营的 Velune 公网服务，也未完成真实跨公网直连验收。

执行端调用 `create_invitation(lifetime_seconds)` 生成最多 10 分钟的一次性邀请，将返回的 `encoded` 经用户明确选择的渠道交给另一设备。控制端调用 `pair(encoded, view_id)` 完成配对并得到远程 `VeluneApplication`；后续用 `descriptor()` 与 `connect(descriptor, view_id)` 重新连接。公开 descriptor 是拨号信息，不是授权凭据；邀请包含秘密，不写日志或自动同步。`trusted_devices` 列举本设备信任，`revoke(device_id)` 先保存撤销并切断该设备既有连接。设备身份与信任文件重开后保留。每个窗口或独立视图使用稳定且互不相同的 `view_id`，浏览状态按已认证设备与该 ID 隔离；窗口不再需要保留时可显式 `release_view()`，断网不会隐式释放视图或停止执行。

远程对象的 `shutdown` 与 `shutdown_stopping_work` 都只关闭控制连接；停止 Agent 使用明确的 `cancel`／`stop_sub_agents`。`VeluneRemoteEndpoint.shutdown()` 关闭本设备远控入口和连接，仍不关闭执行应用。执行核心由本地对象管理，原有忙时拒绝关闭的语义保留。远程对象的 `reconnect(descriptor)` 显式重连并保留请求编号与编辑基线，不会自动重发业务操作。`remote_descriptor` 限定当前操作属于哪台执行设备；目标 cwd 与变更路径始终属于该设备。`connection_status` 返回原生 Direct／Relay／Other 路径、当前选中状态及安全计数器，允许平台观察连接是否转为直连，不暴露应用正文或传输凭据。平台收到网络切换或前台恢复通知时调用 endpoint 的 `report_network_change(snapshot)`，输入默认网络、可用 Wi-Fi、接口地址与 DNS 等事实；返回序号仅确认通知接收，不确认 socket 或路径已经恢复。平台订阅系统通知，本 unit 校验边界后提示 iroh 重评。`observe_paths().next(timeout_ms)` 返回 `BindingPathObservation`：完整当前快照与该 observer 的 `missed_events`。原生 Lagged 的漏事件数原值保留，不能把重新采样解释为完整事件历史；等待限制为 1–5000 毫秒，返回空表示超时或流结束，调用方检查 `closed`。释放 observer 不关闭业务连接。

配置编辑使用乐观版本：连接构造先读取配置并设置初始编辑基线；表单打开时调用 `list_with_revision()`，保留一起返回的配置与 revision，提交前用 `expect_revision(revision)` 指定该表单的依据，再调用原具名写入 API。`observed_revision()` 只用于连接状态展示；它可能被其它读取更新，不能代替表单对应的版本。其他读取与消息轮询只更新观测版本，不替换编辑基线；成功的自身修改推进基线。冲突保留 `BindingFailureKind::Conflict`，调用方重读配置并让用户合并，不能换版本后自动重发旧编辑。显式 `submit_request`／`query_request` 仅更新观测版本，后续编辑由平台明确采用哪个版本。

显式 `remote-diagnostics` feature 额外暴露 `VeluneTransportProbe`，只用于 debug 合成 marker 验收。它不链接本地执行栈、不加载产品身份，也不提供业务授权入口。`native_events()` 排空最多 512 条原生 QNT／路径事件，记录显式丢弃计数；collector 独占诊断进程的全局 tracing subscriber，只允许固定的 transport 事件，不输出到常规日志。`diagnostics()` 与这些原生事件可能含地址，平台只能保存到应用私有诊断文件。普通 `remote` feature 不包含探针或 collector；诊断绑定和动态库必须匹配生成。

远控接口是有界同步调用，平台放在工作队列执行，避免阻塞界面。普通具名 API 等待有界结果；传输不确定时 `BindingError::RemoteRequest` 保留可选的具名 `BindingRemoteRequestId`、安全分类与说明。`next_request_id()` 允许平台在发送前保存编号，`submit_request(id, BindingApplicationCommand)` 提交封闭的具名命令集合，`Running` 用相同编号 `query_request(id)` 查询；`Completed` 带版本与 `BindingApplicationResponse`，保留创建会话等操作的真实返回值。查询可通过同设备的新授权连接恢复，不会重执行。`Unknown` 表示本 incarnation 未接纳，`ResultExpired` 表示该旧编号不能再执行且结果无可用证据：可能是回执已淘汰，也可能是编号跳过形成的未提交空隙；执行端重启则旧结果未知，这些状态不能被解释为执行失败并盲目重发。结果可能包含用户显式读取的 key 或资源正文，因此命令、回执和邀请不实现 Debug，不写入诊断日志。

远程聊天附件先由平台读取控制设备上用户选择的内容，再调用 `upload_attachment(name, bytes)`；将返回的 `reference` 填入 `BindingComposedTurn.attachments`，不把控制设备路径当执行设备路径。`read_attachment(runtime_id, conversation_id, message_id, block_index)` 只读取该会话消息已有的附件内容，不提供通用任意文件读取。显式的提供商 key 查看／修改与资源内容读写通过同一加密连接返回，配置快照仍只含非秘密认证描述。

## 生成绑定

先构建目标平台动态库，再读取该库中的 UniFFI 元数据。以下命令使用开发构建；平台发行构建由应用构建脚本负责。

```sh
cargo build --locked -p velune-bindings
cargo run --locked -p velune-bindings --features cli --bin velune-bindgen -- \
  generate --library target/debug/libvelune_bindings.dylib \
  --language swift --language kotlin --language python \
  --out-dir target/bindings
```

Swift 生成模块名为 `VeluneBindings`，FFI 模块名为 `VeluneBindingsFFI`；Kotlin 包名为 `app.velune.bindings`。绑定与其动态库必须来自同一次契约构建，UniFFI 在加载时检查接口校验信息。生成目录属于构建产物，不提交生成代码。

当前使用 UniFFI `0.32.2` 官方 Swift、Kotlin、Python 生成器。C# 生成器属于第三方工具，尚未完成兼容性验证；不能据此宣称已有 Windows 应用接入。

模型条目的 `recordKey` 自动生成并隐藏；`providerModelId` 是提供商规定的可编辑 API 标识。运行时选择与 snapshot 引用稳定 `modelRecordKey`，导入选择只包含 `candidateKeys`。能力中的 `None` 表示未知，空推理等级列表表示明确不支持。`BindingModelTemplate` 复制填写参数，不形成调用依赖。旧全局模型、关联映射和中央认证接口已删除，不保留兼容入口。

运行时描述携带 `family_id`、variant ID 与兼容版本区间文字 `compatible_versions`，以及能力标志 `launch_options`（`acp_launch`／`upsert_acp_runtime`）、`agent_info`（`acp_runtime_info`）、`session_options`（`acp_session_config`，原生模型目录属于会话）、`remote_resources`（SSH 执行需要 `remote_resources_directory`）；平台据此展示对应界面，不按类型 ID 分支；schema 10 配置通过应用用例持久化。当前 variant 为 Pi 1.0.2、Codex 0.159.3、Claude Code 2.1.286 与 DSH 0.2.0-rc.2，绑定不根据 family 自行选择协议。`BindingRuntimeInteraction` 与 `BindingRuntimeInteractionReply` 表达审批选项、问题／答案和取消；回复关联当前交互 ID，不通过字符串命令或隐式批准。原生 snapshot 历史由 huihua／Harness adapter 投影，绑定不暴露来源 JSON 或实现会话存储。

运行时 descriptor 的 `supported_protocols` 为具名协议枚举列表；平台按精确 variant 消费，不按 family 自行维护协议矩阵。协议能力仍由精确 adapter 声明，与会话模型选择分开。

`fetch_public_model_catalog` 返回具名目录候选，而不是配置模型或来源 JSON。它是有界同步调用，平台放在工作队列；保存选中模板仍使用 `save_model_template`。来源提供商与模型 ID 不作为路由绑定，拉取本身不改配置，也不需要运行时连接。

会话消息角色与工具状态通过具名枚举暴露，内容是 Text／Reasoning／Tool／Notice 类型块；工具保留调用 ID、状态和可选输出。摘要更新时间与消息时间使用具名 Unix 毫秒，未知时间为 None。领域标题保留 Native／FirstMessage／Untitled 来源，绑定仅导出其统一 display text；平台不需要维护另一套标题权威。UI 展示行不构成本 unit 的事件通道或持久化契约。

会话管理通过具名 rename_conversation／delete_conversation API 作用于所属运行时，版本描述公开管理能力；不支持的适配器明确拒绝。平台仅提交实例与会话身份，不获得直接修改来源文件的接口。

`BindingConversationSummary.runtime_id` 表示逻辑会话首个实际原生段的来源。未提交草稿首发前改换 Harness 时，公开 ID 保持稳定，但该来源随实际首发目标更新；平台应采用返回的 snapshot。`BindingConversationSnapshot.context_runtime_id` 表示当前原生上下文；平台不得用来源身份代替当前查询、取消或审批归属。摘要 `can_rename`／`can_delete` 为应用装配的关联会话原生管理能力；`parent_conversation_id` 标出原生子 Agent 的会话，平台从父会话工具块的 `linked_conversation_id` 打开它，不在会话列表中显示；列表之外的会话用 `find_conversation` 按身份取得。`BindingConfigurationSnapshot.conversations` 是 `BindingListedConversation`：摘要加原生 ID 与 Velune 组织信息，`group` 为空表示仍打开但不在列表窗口内；`conversation_groups` 给出组的顺序、总数、已加载数与折叠状态。`organize_conversations`、`set_project_alias`、`load_more_conversations`、`show_archived_conversations` 与 `set_conversation_browser_preferences` 都返回新的列表快照，`search_conversations` 返回按同样方式分组的匹配。`read_conversation`只读指定来源会话而不激活执行；`snapshot`同时携带上下文运行时和会话ID，非当前活动身份返回无快照。`send_turn` 原子接纳来源运行时／会话ID及下一轮目标和模型，跨实例接续和关联持久化由 application 处理，bindings 不保存消息副本。

`BindingModelSelection` 明确区分 `Managed`（Velune model record key）、`Native`（运行时 provider／model ID）与 `RuntimeDefault`（通用 ACP 保留 Agent 当前模型选择）。专用适配器的 `runtime_models` 返回 native／managed 两类具名候选，通用 ACP 先返回 `RuntimeDefault`，活动会话时再加其原生选项；每项 `reasoning_efforts` 是该运行时对此模型可选的推理强度（空表示不提供）；新 `create_conversation_with_selection`／`send_turn_with_selection` 接受完整选择，后者以 `BindingComposedTurn`（正文、可选推理强度、可选权限模式与附件绝对路径；权限模式缺省为 None，原有构造保持有效）承载本轮输入，`BindingRuntimeTypeDescriptor.permission_modes` 列出该类型可选的权限模式（`BindingPermissionMode` 的 ID、名称与说明），空表示不提供；`BindingTranscriptTurn.settings`（`BindingTurnSettings`）是该轮的运行时实例 ID、模型显示名、推理强度与权限模式 ID，各项未知时为空，平台按配置解析名称；资源库子 Agent（`BindingResourceTemplateContent::Subagent`）的 `model` 是写入原生定义的 model ID，`velune_model` 是只在经网关轮次中提供的 Velune 模型记录键，至多其一；`BindingTranscriptTurn.usage`（`BindingTurnUsage`）是原生记录里该轮的 token 用量：输入（含缓存读写）、缓存读取、缓存写入、输出（含推理）与推理，Harness 不记录的项为空；另有该轮结束时的上下文 token 与模型窗口、Harness 估算的会话累计费用（百万分之一美元）与账户额度（`BindingRateLimit` 列表）；`working_branch` 返回会话工作目录的 git 分支，`BindingConfigurationSnapshot.status_line` 与 `set_status_line` 是 composer 状态栏显示的项（`BindingStatusLineItem`，按显示顺序）；原有 create／send 入口继续仅接纳 managed key 与纯文本。`reasoning_efforts` 返回某运行时与 managed 模型可选的推理强度，空表示不提供。`next_turn_model_selection` 返回下一轮意图；它不表示本轮已结束后仍存在网关注入。native 模型不会占用 `BindingConversationSnapshot.model_record_key`。

`BindingRuntimeExecution` 通过 `runtime_execution`／`configure_runtime_execution` 提供 typed Local／Ssh 位置、Inherit／Existing／Managed HOME 和远端资源目录。`upsert_runtime_with_execution` 一次保存实例和执行配置，新建 managed HOME 实例不需要假的 agentDir。`runtime_home_directory` 返回实例在执行机器上实际使用的 HOME（继承时为 None），只解析不创建。既有 `BindingRuntimeInstance` 形状不变，平台不得另行解码内部 execution setting。SSH 目标和所有执行路径的校验、helper 投送、gateway 转发、模型临时覆盖及结束清理由 application／agent-runtime 协调。具体语义见 [application README](../application/README.md)。

**待实现**：平台在 composer 呈现 native 模型并编辑 SSH 连接；Mac 已接入 HOME 归属编辑。

通用 ACP 使用同一个 `acp-v1` 类型接纳任意已安装的 ACP 命令或 bridge。`upsert_acp_runtime` 接收 `BindingAcpLaunch` 的 argv／环境和既有 typed execution，不要求 Node、agentDir 或 SSH helper 目录。`acp_launch` 提供配置读取；环境值为用户显式配置，Debug 不包含值。

`acp_runtime_info` 返回 typed Agent 信息、认证方式描述和会话能力，平台不能把无 list／load 解释为零会话或加载成功。`open_acp_conversation` 接纳明确原生 ID 与 cwd，允许有 load、无 list 的 Agent 重开。原生模型目录属于会话，未打开时目录只有 `RuntimeDefault`；`acp_session_config`／`set_acp_session_config` 保留所有已知 select／boolean 配置项的原 ID、值和分组。普通执行、权限回复、取消复用现有接口；模型切换不重启当前 ACP 进程。外部认证仍由用户完成，bindings 不自动登录。

**待实现**：平台的通用 ACP 命令编辑、能力状态与外部认证提示、默认模型选项、会话配置编辑，以及对无历史恢复能力的清晰展示；本轮只交付 Rust／UniFFI 入口。

资源库：`list/read/save/delete_resource_template`、`resource_sync_status`、`unmanaged_runtime_resources`、`import_runtime_resource`、`adopt_runtime_resource`、`apply_resource_template`、`resolve_resource_binding`、`resource_native_import`、`resource_drift` 以具名类型暴露（`BindingResourceTemplate*`、`BindingResourceSyncState`、`BindingTemplateApply*`、`BindingResolution`、`BindingResourceDriftFile`）；资源内容的 Debug 不含秘密。Mac 的“资源库”窗口是当前唯一消费者。


智能调度的前端配置：`BindingProviderDraft.quota_limits`／公开提供商的同名字段保存 `BindingQuotaLimit { window_id, max_used_percent }`；窗口 ID 来自额度状态，不让用户维护另一份模型 ID。`BindingModelRoute` 必填 `strategy`（Order／Smart／Rotate／Weight）、`sticky` 和 `weights`；新建前端默认 `sticky=true`，仅 Weight 按所有成员记录键填写正整数权重，其余策略传空 map。资格与额度限制始终优先于策略与黏性。旧 `BindingFailoverMode/Policy` 和 `failover` 字段已删除，配置 schema 为 10；更低普通 schema 按 hard-cutoff 初始化并原子覆盖，不迁移或备份，原 Harness 文件及私有秘密文件不重置。

`provider_quota(gateway_id, provider_id)` 和 `refresh_provider_quota` 返回 `BindingProviderQuotaStatus`，都是立即返回的有界入口；后者请求一次异步采样，重复请求合并；全服务 4 个槽位已满时本次不启动、不排队，返回 `refreshing=false`，调用方需后续 query／refresh 重试，不能把它当成已经获得新额度。平台轮询 `refreshing` 与 `freshness`（Fresh／Stale／Unknown／Unsupported），显示窗口百分比、观测／过期／重置 Unix 毫秒、可选模型范围及 `available`。Stale 仅供展示，不表示仍可用于阈值准入；`available=false` 独立于窗口百分比，可表示供应商明确暂停。首次有阈值但额度仍未知时请求会暂停，前端应允许先刷新并等 Fresh，不能自行把 Unknown 当作零使用量。查询和模型使用才触发到期刷新，应用不为闲置账户持续轮询；关闭或换绑取消采样。认证、额度和调度均由 Rust 协调，前端不调用 usage HTTP、推导刷新结果或保存第二份可写额度缓存。Mac 设置的“模型路由”页与提供商编辑器是当前消费者，见 [Mac README](../../app/mac/README.md)。

提供商组合类型为 `BindingExecutionBackend`、`BindingRequestProfile`、`BindingProviderAuthenticationSource` 与 `BindingProviderQuotaSource`。平台应显式填写这四个字段；普通新提供商默认 `Http`、`Native`、所选认证来源与 `None` 额度来源。CLI 组合使用 `ClaudeCli { binary }`、`MessagesV1`、`Native`、空 endpoint、`ClaudeCli` 认证和可选 `ClaudeCliUsage`；对应模板的 `binary` 为空、`executable_command` 为 `claude`，平台填入定位或选择的绝对路径后保存。路径必须绝对；CLI 目录由 application 创建，平台不能传入真实既有登录目录。

Claude CLI 登录沿用 start/poll/cancel/inspect，CLI 自己打开浏览器并管理秘密，可能没有 URL notification；平台持续 poll 获取最终 result。`authentication_logout(gateway_id, provider_id)` 返回 `BindingGatewayUpdate`，退出后刷新配置视图。公开账号摘要只用于辨别绑定，不保证订阅身份或当前可调用；CLI 状态不可用不能显示为登录有效。quota API 不变，未知、过期与 problem 必须按实际呈现；窗口值不是剩余 token 或费用预算。CLI backend 保留应用运行期间的会话，应用关闭或认证变更会终止旧绑定，不承诺重启恢复。
