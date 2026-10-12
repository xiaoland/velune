# 跨语言接入

`velune-bindings` 是共享 application unit 的 UniFFI 接入层。Swift、Kotlin 和 Python 使用同一份 Rust 元数据生成接口。领域 package 不依赖 UniFFI；此处的 `Binding*` records 与 enums 将领域契约转换为语言绑定能够表达的类型。Mac 在应用进程内加载此 unit 的动态库，不通过 Host 或 socket 调用 Rust。

原生资源通过 `runtime_resource_catalog`／`read_runtime_resource`／`write_runtime_resource`／`delete_runtime_resource` 管理。`BindingRuntimeResourceKind`、scope、key、state、access 和 revision 均为具名类型；资源内容用 bytes 保留原生文件或单条 MCP JSON，Rust Debug 隐去正文。写入传 Missing 或此前的 Match revision，冲突保留为 `BindingFailureKind::RuntimeManagement`、`phase=runtime_management`、`code=conflict`，不退化为普通字符串错误。原生 Agent 定义不是活动子任务，bindings 不据此生成协作授权。**待实现**：平台资源管理与冲突处理界面；资源保存不自动重启会话。

`create_collaborative_conversation` 接受 typed target grant 与 limits，子任务通过 `collaboration_children`／`child_snapshot` 观察，权限答复必须同时匹配 child ID、turn 和 interaction ID。`close_collaboration` 关闭范围及其子任务。父 Agent 的 MCP 请求直接访问独立 Coordinator，不获取 `VeluneApplication` 操作锁；子任务 worker 在当前应用进程内运行。子任务最终正文仅表示其报告，不能当作独立验证的成功。**待实现**：平台授权、状态与审批界面。

`VeluneApplication` 公开配置维护、会话控制、提供商导入、认证交互与关闭操作，参数和结果具有具名类型。提供商拥有模型条目；`BindingProviderDraft` 与 `BindingAuthenticationEdit` 共同保存普通字段和认证编辑，后者明确区分保留、更新 API key 和清除。列表与 snapshot 只返回非秘密认证描述，`read_provider_api_key` 显式读取一个提供商的已保存 key。认证检查和登录以网关／提供商 ID 定位，不存在独立认证资源 CRUD。`BindingProviderModel.adapter_metadata_json` 由运行时 adapter 拥有，平台只保留它，不解释其结构。

一个对象串行执行应用操作；多个会话可同时执行，`cancel`／`reply_runtime_interaction` 以运行时与会话 ID 寻址，`working_changes`／`file_diff` 返回会话工作目录的 git 变更（`BindingWorkingChanges`、`BindingChangeKind`）与按行分类的差异（`BindingFileDiff`）。`conversation_activity(since)` 返回 `BindingConversationActivity`：执行中、待处理交互或 best-effort 判断在 Velune 之外运行（`BindingRunState::Elsewhere`）的会话，以及游标之后结束的 turn（`BindingTurnOutcome`）和下次使用的游标。`shutdown` 在应用忙碌时返回错误并保留对象，调用方可继续读取 snapshot 或明确取消 Agent 执行；关闭成功后除再次关闭外的操作返回 `Closed` 类型的诊断。释放语言包装对象负责释放其 Rust 引用，不代表用户发出了取消 Agent 的命令。会话状态仍通过 `snapshot` 或游标式 `snapshot_update` 观察，本 unit 不新增事件订阅或后台宿主。`BindingMessagePayload` 区分内联内容与只带字节数的延迟内容，`message_payloads` 读取后者。

本 unit 在 `home_directory/logs` 装配对象专属的 tracing subscriber，不占用进程全局 subscriber。操作失败的 `BindingError.Diagnostic` 保留分类、白名单 code/phase、说明和 operation ID，平台使用该 ID 关联日志；领域包不依赖这一日志输出或 UniFFI。日志安全字段、轮转和后续 layer 扩展见 [开发说明](../../docs/development.md#本地诊断)。

默认启用 `local-runtime`。使用 `--no-default-features` 构建会排除本地 Agent runtime；这表示不携带本机或 SSH 子进程执行能力；两种执行位置都需要该 feature。

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

运行时描述携带 `family_id`、精确 variant ID 与 `version_regex`；schema 8 配置通过应用用例持久化。当前 variant 为 Pi 1.0.2、Codex 0.159.3、Claude Code 2.1.286 与 DSH 0.2.0-rc.2，绑定不根据 family 自行选择协议。`BindingRuntimeInteraction` 与 `BindingRuntimeInteractionReply` 表达审批选项、问题／答案和取消；回复关联当前交互 ID，不通过字符串命令或隐式批准。原生 snapshot 历史由 huihua／Harness adapter 投影，绑定不暴露来源 JSON 或实现会话存储。

运行时 descriptor 的 `supported_protocols` 为具名协议枚举列表；平台按精确 variant 消费，不按 family 自行维护协议矩阵。协议能力仍由精确 adapter 声明，与会话模型选择分开。

`fetch_public_model_catalog` 返回具名目录候选，而不是配置模型或来源 JSON。它是有界同步调用，平台放在工作队列；保存选中模板仍使用 `save_model_template`。来源提供商与模型 ID 不作为路由绑定，拉取本身不改配置，也不需要运行时连接。

会话消息角色与工具状态通过具名枚举暴露，内容是 Text／Reasoning／Tool／Notice 类型块；工具保留调用 ID、状态和可选输出。摘要更新时间与消息时间使用具名 Unix 毫秒，未知时间为 None。领域标题保留 Native／FirstMessage／Untitled 来源，绑定仅导出其统一 display text；平台不需要维护另一套标题权威。UI 展示行不构成本 unit 的事件通道或持久化契约。

会话管理通过具名 rename_conversation／delete_conversation API 作用于所属运行时，版本描述公开管理能力；不支持的适配器明确拒绝。平台仅提交实例与会话身份，不获得直接修改来源文件的接口。

`BindingConversationSummary.runtime_id` 表示逻辑会话首个实际原生段的来源。未提交草稿首发前改换 Harness 时，公开 ID 保持稳定，但该来源随实际首发目标更新；平台应采用返回的 snapshot。`BindingConversationSnapshot.context_runtime_id` 表示当前原生上下文；平台不得用来源身份代替当前查询、取消或审批归属。摘要 `can_rename`／`can_delete` 为应用装配的关联会话原生管理能力。`read_conversation`只读指定来源会话而不激活执行；`snapshot`同时携带上下文运行时和会话ID，非当前活动身份返回无快照。`send_turn` 原子接纳来源运行时／会话ID及下一轮目标和模型，跨实例接续和关联持久化由 application 处理，bindings 不保存消息副本。

`BindingModelSelection` 明确区分 `Managed`（Velune model record key）、`Native`（运行时 provider／model ID）与 `RuntimeDefault`（通用 ACP 保留 Agent 当前模型选择）。专用适配器的 `runtime_models` 返回 native／managed 两类具名候选，通用 ACP 只返回当前会话的原生选项；新 `create_conversation_with_selection`／`send_turn_with_selection` 接受完整选择，后者以 `BindingComposedTurn`（正文与可选推理强度）承载本轮输入；原有 create／send 入口继续仅接纳 managed key 与纯文本。`reasoning_efforts` 返回某运行时与 managed 模型可选的推理强度，空表示不提供。`next_turn_model_selection` 返回下一轮意图；它不表示本轮已结束后仍存在网关注入。native 模型不会占用 `BindingConversationSnapshot.model_record_key`。

`BindingRuntimeExecution` 通过 `runtime_execution`／`configure_runtime_execution` 提供 typed Local／Ssh 位置、Inherit／Existing／Managed HOME 和远端资源目录。`upsert_runtime_with_execution` 一次保存实例和执行配置，新建 managed HOME 实例不需要假的 agentDir。`runtime_home_directory` 返回实例在执行机器上实际使用的 HOME（继承时为 None），只解析不创建。既有 `BindingRuntimeInstance` 形状不变，平台不得另行解码内部 execution setting。SSH 目标和所有执行路径的校验、helper 投送、gateway 转发、模型临时覆盖及结束清理由 application／agent-runtime 协调。具体语义见 [application README](../application/README.md)。

**待实现**：平台在 composer 呈现 native 模型并编辑 SSH 连接；Mac 已接入 HOME 归属编辑。

通用 ACP 使用同一个 `acp-v1` 类型接纳任意已安装的 ACP 命令或 bridge。`upsert_acp_runtime` 接收 `BindingAcpLaunch` 的 argv／环境和既有 typed execution，不要求 Node、agentDir 或 SSH helper 目录。`acp_launch` 提供配置读取；环境值为用户显式配置，Debug 不包含值。

`acp_runtime_info` 返回 typed Agent 信息、认证方式描述和会话能力，平台不能把无 list／load 解释为零会话或加载成功。`open_acp_conversation` 接纳明确原生 ID 与 cwd，允许有 load、无 list 的 Agent 重开。模型目录属于会话，先以 `RuntimeDefault` 新建或打开后查询；`acp_session_config`／`set_acp_session_config` 保留所有已知 select／boolean 配置项的原 ID、值和分组。普通执行、权限回复、取消复用现有接口；模型切换不重启当前 ACP 进程。外部认证仍由用户完成，bindings 不自动登录。

**待实现**：平台的通用 ACP 命令编辑、能力状态与外部认证提示、默认模型选项、会话配置编辑，以及对无历史恢复能力的清晰展示；本轮只交付 Rust／UniFFI 入口。

资源库：`list/read/save/delete_resource_template`、`resource_sync_status`、`unmanaged_runtime_resources`、`import_runtime_resource`、`adopt_runtime_resource`、`apply_resource_template`、`resolve_resource_binding`、`resource_drift` 以具名类型暴露（`BindingResourceTemplate*`、`BindingResourceSyncState`、`BindingTemplateApply*`、`BindingResolution`、`BindingResourceDriftFile`）；资源内容的 Debug 不含秘密。Mac 的“资源库”窗口是当前唯一消费者。
