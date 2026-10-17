# 应用层

`velune-application` 是平台 app 共用的应用用例 unit。公开入口 `Application` 提供类型化的配置、连接、会话、模型选择、提供商导入与认证操作。平台不自行协调文件提交、子进程状态与投影失败清理。

多个会话可以同时执行。每个打开的会话有自己的执行、模型选择、投影与协作票据；以会话 ID 寻址的操作（打开、发送、取消、回复交互、重命名、删除、snapshot）作用于该会话，每次操作都推进所有会话。每个会话同一时刻最多一个 turn；`cancel` 与 `reply_runtime_interaction` 需要会话 ID。重开正在执行的会话返回其运行中的视图，结束后再打开才重读原生历史。执行中或有待处理交互的会话保持打开，空闲会话按最近使用保留 8 个。同一运行时的并发执行各用一个 lane 目录（`runtime-projections/<运行时>/lane-<n>`）存放网关模型与选择文件。`conversation_activity(since)` 列出执行中或待处理交互的会话，并返回游标之后结束的 Velune turn 及其结果（完成、失败、用户停止；Pi 以错误结束模型响应也算失败），最近 64 个可补读；可频繁轮询。运行时、提供商、导入与登录配置在任一会话执行时拒绝变更。[并行会话手动验收](../../scripts/manual-parallel-sessions.py) 用隔离 Pi 1.0.2 与本机 loopback 上游检查同时执行、独立结束、取消与配置门禁。

每个会话另有 Velune 的发送队列（`local/follow_up.rs`），只在应用运行期间保存：`send_queue` 加入（保存发送时的完整选择）、编辑正文、删除、移动与继续发送；turn 正常结束时推进会话即发出队首，各自成轮，停止或失败暂停，未被接受的一条留在队首并暂停、不自动重发；空闲时加入或继续发送立即发出。有排队消息的会话保持打开。快照的 `followUp` 给出队列、暂停原因、能否 steer 与能否继续（本会话上一轮停止或失败）。`steer_turn` 与队列操作 `Steer` 经运行时原生通道把消息插入正在运行的轮次（Pi `steer`、Codex `turn/steer`、Claude Code 轮次中的 stdin 用户消息），不另起一轮、不带下一轮选择；ACP 与 DSH 不提供。[发送队列手动验收](../../scripts/manual-send-queue.py)、[steer 手动验收](../../scripts/manual-steer.py)。

编辑用户消息归 `local/editing.rs`：`followUp.editable` 列出当前可编辑的用户消息及能否原地替换（运行中、待回复请求、跨 Harness 接续、sub-agent 与未发送会话均无；Codex 插入本轮的消息随所在轮次，不单独列出；Claude Code 第一条只能 fork）。`edit_message` 先按发送同样的规则检查附件、模型、推理与权限，再重新读取原生历史，按用户消息序号与正文认出这条消息（不一致时拒绝，提示在别处已改变），之后经原生机制分叉并作为新一轮发出：Codex `thread/revert`／`thread/fork`（复制到前一轮），Claude Code 下一轮子进程 `--resume-session-at`（fork 另加 `--fork-session --session-id`），Pi 扩展命令 `velune-rewind`（`navigateTree`，旧分支留在会话树）／RPC `fork`。第一条消息 fork 时 Codex 与 Claude Code 在同一目录新建会话。fork 在新的会话（session）中进行，原会话保持不变，结果是新会话的快照；分叉前失败时回到原会话。[消息编辑手动验收](../../scripts/manual-edit-message.py)。

会话的权威在 Harness；Velune 对本机 Codex、Claude Code 与 Pi 运行时 best-effort 跟随原生文件。被 snapshot 轮询的会话在没有会话通道时（例如只读打开的视图）由首次轮询读入并打开通道；其后约每秒检查原生文件，未在 Velune 中执行且文件被改动时重新读取，空闲的执行先释放，下一 turn 从改动后的会话继续；读取较慢时按耗时放宽间隔。自身执行期间及结束后 5 秒内的写入视为自己的。Harness 自己的存活记录（见 agent-runtime `activity`）显示另一进程正在运行该会话时，视图的 `run_state` 为 `Elsewhere`（展示为运行中，不可由 Velune 取消，仍可发送）；文件变化时及之后每 5 秒重新判断，进程中途退出后即回到空闲，不依赖时间窗。活动列表另外每 5 秒读取这些运行时的存活记录，附加未被 Velune 执行的 `Elsewhere` 会话。SSH、ACP、DeepSeek Harness 与跨 Harness 续接的会话不跟随，仍靠手动重新读取。[实时会话手动验收](../../scripts/manual-live-sessions.py) 以实际 Pi CLI 作为另一客户端检查上述行为，[实际 Harness 实时脚本](../../scripts/manual-live-native.py) 检查 Codex 与 Claude Code，包括进程中途被杀后不再运行。

发送接受 `ComposedTurn`（正文、可选推理强度、可选权限模式与附件）。权限模式须是目标运行时类型在 `RuntimeTypeDescriptor.permission_modes` 中声明的取值，否则在任何状态改变与发送前拒绝；跨 Harness 接续时随目标运行时的首轮一并交付。权限模式是会话状态。未指定时，application 取会话最近一轮在同一运行时上的模式重新发送（Codex 恢复线程时不保留轮次覆盖）；不知道当前模式就不发送，由运行时使用自己的默认或恢复的状态。发送时把本轮的运行时实例、模型名称（managed 模型为“名称 · 提供商”，原生模型为其 ID）、推理强度与权限模式登记到 transcript projection，供 `TranscriptTurn.settings` 显示。轮次用量（`TranscriptTurn.usage`）只取 Harness 原生记录，见 agent-runtime README；网关的分析记录没有会话归属，不参与。唯一例外是上下文窗口：Harness 对它不认识的模型只会给出自己的猜测（Codex 0.159.3 报 258400，Claude Code 2.1.286 报 200000），因此该轮模型能对应到 managed 模型（Velune 的名称，或唯一的提供商模型 ID）且其配置了上下文窗口时，application 用配置值替换或补上；对应不到时保留 Harness 报告的值。`working_branch(runtime, conversation)` 只读地返回会话工作目录当前的 git 分支（`git symbolic-ref --short -q HEAD`），仓库外或 detached HEAD 为空，远端运行时报错。composer 状态栏显示的项（`StatusLineItem`：上下文用量、本会话累计 token、工作目录与分支、账户额度、费用估算）是应用配置，默认只有上下文用量；`set_status_line` 保存时去重并按显示顺序排列。application 给每条用户消息补上所在段的运行时实例，并把 Pi 记录的网关路由 alias 换回 managed 模型的名称。[轮次配置手动验收](../../scripts/manual-turn-settings.py) 检查实时与重启后读历史两种情况。[权限模式手动验收](../../scripts/manual-permission-modes.py) 用实际 Codex 与 Claude Code 检查声明、拒绝，以及 Harness 实际告诉模型的权限。附件是本机绝对路径，在任何状态改变与发送前检查：必须是可读的普通文件；扩展名为 png／jpg／jpeg／gif／webp 的按图片处理，内容须与类型相符且不超过 20 MB，经 Harness 自身的图片输入交付——Codex `turn/start` 的 `localImage{path}`，Claude Code stream-json 用户消息的 base64 `image` 块（置于正文前），Pi RPC `prompt.images`，通用 ACP 的 `image` 块（仅当 Agent 声明 `promptCapabilities.image`）；DeepSeek Harness 不接受图片。其他文件以“附件：”绝对路径列表追加到正文末尾，由 Agent 用自身工具读取，不内联内容。有图片时正文可为空。远端运行时的附件、无法交付的附件都整条拒绝，不静默丢弃。跨 Harness 接续时文件列表随正文进入交接内容，图片随该 turn 交付。Pi 网关模型声明 text＋image 输入（Velune 不知道网关模型的模态，由上游接受或拒绝）。[附件手动验收](../../scripts/manual-attachments.py) 用实际 Codex、Claude Code 与 Pi 经回环上游检查图片字节、文件路径列表、仅图片的 turn 与各类拒绝。`reasoning_efforts(runtime, model)` 给出该运行时与 managed 模型可选的强度：模型声明的级别按 Harness 自身通道的词汇映射——Codex 原样作为 Responses reasoning effort（`turn/start.effort`），Claude Code 取与 `--effort` 接受集合（low／medium／high／xhigh／max）的交集，Pi 用 Pi 级别名（`none` 对应 `off`）并经 `set_thinking_level` 设置；DeepSeek Harness 与通用 ACP 不提供。不在选项内的强度在任何状态改变与发送前拒绝。推理强度与权限模式一样是会话状态：Harness 自身在 Velune 下都不保留它（Codex 不选时不发送，Claude Code 每 turn 启动一次，Pi 的执行以 off 开始），因此未指定时 application 取会话最近一轮在同一运行时上的级别，所选模型仍接受时重新发送；不知道就不发送。按会话、按运行时记住所选强度是平台的职责。[推理强度手动验收](../../scripts/manual-reasoning-effort.py) 用实际 Codex、Claude Code 与 Pi 经回环上游检查选项、拒绝与上游实际收到的推理字段。

`runtime_resource_catalog`、`read_runtime_resource`、`write_runtime_resource` 和 `delete_runtime_resource` 管理指定实例的原生 user instructions、MCP server 条目、skills 与原生 Agent 定义。调用方明确给出 User／Project 范围；Project 的 cwd 属于执行机器。目录是 adapter 已知的原生来源，不是 Harness 合并后的最终配置，也不扫描任意命令猜测其资源格式。catalog 将功能不支持与资源 Missing／Invalid 分开，并单独说明可写或只读；不支持原因可能是原生功能缺失、协议没有入口或适配器尚未实现。

资源查询不会启动 Harness、创建资源根目录或安装辅助 bundle；停用实例仍可管理。本机／SSH 与 managed HOME 复用同一位置解析，但不要求与资源读写无关的执行路径。写入在有 turn 执行时拒绝，不隐式关闭空闲会话；保存只表示原生文件已变更，生效时间由 Harness 决定，不宣称进行了 live reload。

资源 key 是类别根目录内的相对路径，MCP key 则是 opaque server name。读取返回单文件 bytes 或单个 MCP 条目的原生 JSON；内容可能含私有设置，不进入 Debug 或日志。写入必须提供 `RuntimeResourceRevision::Missing` 或此前读取的 `Match` revision，外部修改返回可识别的 conflict；混合原生配置由 adapter 只修改目标条目并保留其余内容。Skills 按文件和资产管理，catalog 包含目录及资产；先创建 SKILL.md 再添加资产，不提供整包替换，目录只允许在为空时删除。原生 Agent 定义与 Agent 间协作无关，编辑定义不会启动会话。

**待实现**：平台调用原生资源入口的浏览、编辑、冲突处理和敏感内容展示；本轮没有 App 改动，也没有新增资源副本配置库。

[资源 API 手动验收](../../scripts/manual-runtime-management-api.py) 接收本次构建的动态库与 Python 绑定目录，在隔离 HOME 中检查 typed CRUD、revision 冲突和会话生命周期门禁；默认只需空资源目录。它内置合成 ACP peer，不代替原生 Harness 的实际资源加载验收。

**Agent 间协作（A2A）**：application 持有一个 loopback MCP 服务（服务名 `velune`），在 Velune 执行的每个会话启动时注入；每个会话一张随机票据，HTTP 层校验票据、Origin 与 Host，票据决定调用方会话，工具参数不能自报身份。工具为 `list_agents`、`list_conversations`、`start_conversation`、`send_message`、`get_conversation`、`wait_conversation` 与 `cancel_turn`。参与方都是普通会话：Agent 新建的会话发出首条消息后即出现在列表（原生历史写入前也列出），用户可打开、继续并处理其审批。消息以“[Velune 协作消息]”出处包装作为对方的下一轮提交，声明不是人类授权；对方执行中、待审批或在 Velune 外运行时拒绝，不排队。`start_conversation` 的工作目录缺省沿用调用方（同一执行机器时），模型缺省为对方会话已有的选择或运行时目录的第一项。`wait_conversation` 在对方进入调用方尚未读到的非运行状态时返回，单次最长 30 秒，超时不取消。不建立父子关系、不传播取消，也不设联系限制（PRD 决定）。

MCP 线程不访问 application：工具调用进入收件箱，由下一次客户端请求在推进各会话后执行，并在结束后恢复原先寻址的会话；协作进度因此与会话事件一样依赖客户端轮询（Mac 每 0.6 秒读取活动），60 秒内未被处理的调用向 Agent 返回结果未知。服务启动失败、SSH 转发失败或 ACP stdio relay 不可用时，会话照常运行而不带协作工具；会话关闭即撤销其票据。没有专门的 bindings API，平台通过普通会话、消息与交互呈现。[A2A 手动验收](../../scripts/manual-collaboration.py)以两个合成 ACP 运行时检查新建与联系已有会话、审批留在对方会话、忙碌与未知票据被拒。

`config` 定义 schema 10 的应用配置；`repository` 是内部文件设施，负责锁、加载和原子提交。提供商拥有模型条目和私有认证，参数模板仅用于复制填写字段，不参与调用解析。API key 与普通配置在同一文件原子保存，文件创建权限为 0600；公开配置摘要不含 key，只有显式单提供商读取接口返回已保存的值。认证编辑区分保留、更新和清除，读取失败不清空原值。运行时来源 OAuth 保留原来源登录和刷新，来源定位不进入公开摘要。根目录与资源目录由平台通过 `Options` 显式传入。

提供商导入选择已保存的 Agent 运行时实例，应用从其配置确定目录及 Node 路径；调用方不能另填来源配置。预览不解析秘密，应用导入时将静态 API key 保存到提供商私有配置，OAuth 仍委托来源刷新。实例、源目录元数据或目标配置变化使预览失效。读取不要求执行准备或当前会话选择模型。来源文件与 Harness 会话保留，Velune 不新建会话数据库。保存首个使用默认网关的运行时会建立空的默认网关，便于先配置运行时再导入。

Codex 订阅由 application 自身管理，与 Harness 登录独立。保存提供商时使用 `ProviderDraft.authentication_source=CodexOAuth` 与 `AuthenticationEdit::Keep`，协议须为 Responses，地址须为 `https://chatgpt.com/backend-api/codex`；普通字段与模型仍由原有提供商 API 管理。调用 `authentication_start(gateway_id, provider_id)` 开始登录，`authentication_poll` 获取打开浏览器的 URL 通知与最终结果，`authentication_cancel` 请求取消；任何时刻只允许一项交互式认证，取消完成后才释放回调端口。`authentication_inspect.configured` 和公开提供商摘要反映私有会话是否可续期，而非保证上游接受请求。`ProviderAuthenticationDescription.account_label` 与检查结果的同名字段仅含已知账户 ID 尾 8 位，便于区分订阅；失效后仍可显示绑定，但不代表认证有效。`ProviderAuthenticationDescription.disclosure` 是必须向用户呈现的接入身份与规则冲突说明。

浏览器登录使用 Codex 原生 OAuth 客户端、PKCE 和 state，以及固定 `localhost:1455/auth/callback`；端口占用时明确失败，不结束其他程序。每次登录写独立临时会话，配置成功提交新的引用后才清理旧会话，失败或取消保留原认证。秘密保存在应用 HOME 的 `subscription-auth/` 私有目录（0700），文件权限为 0600；提供商配置只保存内部引用。每个会话的锁覆盖重读、刷新和原子替换，等待锁有 35 秒上限；已经发出的刷新由独立工作线程负责保存结果，turn 取消不会撤销它。客户端禁止重定向与 HTTP 隐式重试。请求前临近过期时刷新，Codex 专用 401 恢复由 gateway 有界触发；轮换 token、账户 ID 和有效期一起更新。明确永久拒绝与再次 401 标记需要重新登录，临时网络或服务错误保留原认证；无效状态写入失败只记录固定、无秘密事件。删除提供商或更换认证后清理旧秘密；若清理失败，错误明确说明配置已经保存，不伪造回滚。进程直接退出或机器掉电仍可能打断尚未收到并保存的轮换响应。

本接入借用 Codex 客户端协议，并非正式 SIWC；多订阅轮换与用量池化和 SIWC 禁止规则冲突，不代表供应商认可。正式 SIWC 不在本轮。现有 Switcher、Codex 或 Pi 的认证文件不被读取、复制或修改；用户在 Velune 分别授权每个提供商实例，避免多个工具争用同一 refresh token。

默认 `local-runtime` feature 组合本机运行时和 LLM 网关。`local` 按配置、连接、会话、认证协调与 Pi／原生运行时装配分模块。应用捕获提供商私有认证快照，装配异步解析器；解析器校验捕获的协议与服务地址，未知引用不回退环境认证。Unix 来源 helper 的期限、输出上限与进程组取消由应用负责，网关不解释来源或调用 helper。Pi 元数据在装配时剥离为 `velune_gateway::GatewayConfig`，AI package 不依赖运行时。

关闭 `local-runtime` 后，提供商、API key、模型模板和运行时配置的加载与修改仍可使用；本机连接、会话执行、来源导入以及需要 adapter 的认证操作明确返回不支持错误。本构建不链接 Agent runtime；当前配置校验仍依赖 gateway 的配置类型与 AI-provider，不能将其称为完全排除本机 AI 依赖的产物。它不启动网关，也不携带 SSH 子进程执行能力；本机与 SSH 运行时都需要 `local-runtime`。

公开 API 和结果记录定义在 `api.rs`，没有平台 JSON dispatcher。内部用例仍有 JSON 投影转换，不构成跨语言契约。隐藏稳定 `recordKey` 用于模型选择引用；`providerModelId` 是可编辑的精确上游标识，修改它不改变内部身份。能力可以未知，只有具体运行时准备边界检查必需字段。模板不复制内部键、认证、服务地址或 Pi 执行元数据。

只接受 schema 10。更低版本的普通配置原子覆盖为空配置，不备份、不迁移、不操作原 Harness 会话、认证或既有平台秘密；未来版本、损坏 JSON 和当前版本中的未知字段明确报错。

运行时描述包含 familyId、versioned variant ID 与兼容版本区间文字（compatibleVersions），配置实例引用 variant。专用适配器支持 `pi-1.0`、`codex-0.159`、`claude-code-2.1` 与 `dsh-acp-0.2`；通用 `acp-v1` 表示协商的协议版本，不限定 Agent 品牌或 CLI 版本。family 不参与控制协议派发。应用协调 Codex app-server／DSH ACP 与 huihua 历史读取，原生 ID 与 runtimeId 共同形成公开会话身份；读取成功不等于原生恢复成功。运行时添加后直接可读历史，不要求模型或执行准备。新建显式给出会话模型并自动准备；历史视图选择模型后，在发送时按需恢复执行。Pi 历史中的 Velune 模型标记只恢复 managed 引用；原生目录提供的模型通过显式 native 选择发送，不另存会话状态。

原生审批、问题回答及取消通过 RuntimeInteraction／RuntimeInteractionReply 完成，配置变更、执行和失败清理由应用协调。提供商导入和订阅来源认证仍只支持 Pi 适配器，增加 Codex／DSH 执行不自动增加其提供商导入。DSH 模型能力注入只使用已知字段，reasoning wire 映射未实现，不将能力列表解释为请求参数。

公开模型目录的获取和来源解析归本 unit。当前支持用户显式读取 models.dev 的 provider-scoped API，以具名候选记录返回来源提供商、实际模型 ID 和已声明能力，供模板快填；候选不进入配置，用户保存时仍走现有模板用例。同步获取由平台工作队列调用，单次请求的 executor 随调用结束释放，不建立常驻 Host 或全局 subscriber。目录获取不使用提供商凭据，不读取本机运行时配置、不自动刷新或改变协议／地址；未知规格保持未知。

会话列表汇总全部启用运行时，实例 disabled 不读取其历史来源，保留其配置与原生历史；enabled 默认 true 作为当前 schema 的增量配置。当前执行实例与浏览筛选分别管理，不按执行实例隐式限制列表。

列表窗口由 `local/organization.rs` 计算：按浏览偏好筛选、分组（组按最近活动排序，置顶组在前、无项目在后）与排序，总数上限依次分给各组的首页，折叠组不读取，`load_more_conversations` 只为该组追加一页；改变分组、排序或筛选时清空追加页。`list()` 返回窗口内的会话（`ListedConversation`，含原生 ID、置顶、未读、归档、所属项目与组）以及仍打开但不在窗口内的会话（组为空）、组描述与项目清单。会话的项目以主文件夹标识（`local/projects.rs`）：工作目录是某个项目的文件夹时归该项目；否则若它是 Git worktree，归其所属仓库的项目（Claude Code 约定的 `.claude/worktrees/<名称>` 按路径判断，SSH 会话与已删除的 worktree 同样适用；本机会话另读 worktree 的 `.git` 文件与 `commondir`，每个目录读一次并缓存）；再否则工作目录自成项目。`set_project` 设定项目的名称、主文件夹与附加文件夹，一个文件夹只属于一个项目；并入的目录及原主文件夹上的移动记录、项目筛选与折叠分组改指新的主文件夹。运行时在没有项目时安排的位置不算项目、归入无项目组：文件系统根（应用自身不指定目录时的位置）、运行时看到的 HOME（SSH 继承 HOME 时探测一次并缓存），以及版本描述 `projectless_directories` 列出的 HOME 下目录（Codex 为 `Documents/Codex`，即 Codex 应用无项目聊天的默认目录）。Velune 的列表组织记录保存在 `conversation-organization.json`（schema 2）：置顶、Velune 归档、已读位置、手动未读、移到的项目（主文件夹），以及调整过的项目（主文件夹、名称、附加文件夹）；缺少文件或遇到旧 schema 时以当前时间为已读基线重新开始。窗口中未读且在本机的会话另读原生会话文件末尾判断最近结束的轮次是否失败（`failed`，按文件签名缓存；用户停止不算失败）。运行时有原生归档（版本描述 `can_archive_conversations`，当前为 Codex）时归档改用原生 `thread/archive`／`thread/unarchive`，归档视图另读其已归档列表；其余运行时只记录、不改原生会话。归档会取消置顶，向 Velune 归档的会话发送消息会取消归档，删除会话时移除其记录。Claude Code 与 Pi 按文件索引（`local/browsing.rs`）：未读会话以文件时间与会话头的工作目录参与分组、计数和窗口分配，`browse_read` 只读取窗口显示到的会话，读取后以原生时间重排至稳定；搜索读出全部未读会话，按 ID 查找只读该会话，协作工具读取全部。

跨运行时发送通过同一 `send_turn` 接纳下一轮目标，创建目标原生会话并以明确引用文本交接上下文。逻辑会话保持公开身份，并以首个实际原生段的标题为权威，`context_runtime_id` 表示当前原生查询／执行上下文，不能从列表的来源实例推断。`local/continuation.rs` 装配有序投影，封闭段从权威历史计算截止数量和内容摘要；原生前缀变更明确拒绝，截止后的追加不进入旧段。每次回切创建新原生段，不恢复旧执行状态。

`conversation-links.json` 是 schema 1 的关联文件，独立于 schema 10 普通配置，只保存原生引用、截止位置及交接记录定位，不保存消息或凭据。`composer-drafts.json`（schema 1，0600）按会话公开身份保存未发送的 composer 文字，同样独立于普通配置、不随配置重置；空文字删除条目，删除会话时一并删除，尚未首发的新建会话不写入。公开身份可以是单目标别名；后续交接继续向同一关联追加原生段。目标准备成功后原子保存关联，再发送；未知发送结果不自动重试。重开以各原生历史重建消息，精确校验交接记录后只显示本轮用户输入，不递归复制交接包。首版交接仅携带文本及完成的工具结果引用；工具执行、审批和签名推理状态不重放。

当前进程成功新建的 Codex、Claude Code 和 Pi 草稿，在首次发送前切换模型或实例时可以重建实际目标，并以单目标关联保留公开身份。该资格来自明确的新建生命周期，不能从空消息或历史缺失推断。目标创建及关联原子提交成功后才发送；创建失败保留草稿身份供显式重试，关联提交失败保留已创建的活动目标而不发送。第一次发送调用即消耗草稿资格，失败或断连不自动重建或重发。重启后目标尚无原生历史时显示不可发送状态；确认目标不存在后，显式删除可撤除这条单目标关联。未提交且未关联的内存草稿不保证跨重启保留。DSH 新建时已有原生记录，继续使用普通历史接续。

Pi 草稿的原生重命名可能尚未落盘，因此重命名不消耗未提交资格。同一 Pi 实例重建时转交该明确的原生标题；切到没有原生命名接口的 Harness 时，以新目标的实际标题为准，不建立独立的 Velune 标题存储。单目标别名不显示 Harness 交接通知。

摘要的 `can_rename`／`can_delete` 是 application 装配的原生管理能力。逻辑重命名作用于首段标题；删除逐段执行，确认全部完成后移除关联，部分失败记录已完成项并保留未完成关系。缺少原生删除能力时不以本地隐藏替代。当前尾段不可读取时保留已校验的来源正文并关闭发送，修复原生来源后重新打开；不存在自动重发或伪造 resume。

分析记录由 application 在配置根目录的 `analytics.sqlite` 独立维护，后台有界写入与应用生命周期共同关闭。`analytics_query` 接收明确 UTC 区间及平台自然日边界，返回具名概览、趋势、提供商／模型拆分与有界请求页。明细页不截断概览人口，未知计量和存储告警明确返回；统计不可用不阻止应用配置与执行。具体口径归 [AI 服务设计](../../docs/design/ai-service.md#用量与性能分析契约)。

会话阅读与执行身份分开：`read_conversation`只读指定来源的原生历史／关联投影，不切换活跃runner；相同活动身份返回当前投影。`snapshot`同时接收上下文运行时和会话ID，非当前活动身份返回None。快照结果是交付形式 `DeliveredSnapshot`：思考块交付为摘要与其下的正文（`DeliveredBlock::Reasoning{summary, text}`），没有摘要的思考块不交付；超过 2KB 的推理与工具输出只携带字节数，由 `message_payloads` 按消息读取；图片内容同样延迟，路径图片只交付路径；用户正文末尾的“附件：”路径列表交付为文件块（`DeliveredBlock::File`），不作为正文；Agent 间消息末尾的来源行交付为正文前的 `DeliveredBlock::AgentSender`；工具块的 `linked_conversation_id` 取 adapter 记录的原生子 Agent 会话，否则取 Velune 协作工具返回的状态对象；带 `parent_conversation_id` 的原生子 Agent 会话打开后不可发送，`send_turn` 明确拒绝；`snapshot_update(since)` 依据上次交付游标返回未变化或只含变化消息的快照，供展示客户端轮询。`send_turn`接收来源运行时／会话ID、目标运行时／模型和文本，在同一个核心操作中激活指定来源再派发，不能用全局最后打开的会话代替发送来源。调用方可为多个工作区保存阅读投影，但共享核心仍只有一个活跃执行。

`ModelSelection` 区分 `Managed { model_record_key }`、`Native { provider_id, model_id }` 和通用 ACP 的 `RuntimeDefault`。专用适配器的 `runtime_models` 从指定实例读取原生模型目录，并附加该实例网关中的 managed 模型（名称带提供商名）；通用 ACP 先提供 `RuntimeDefault`（“Agent 默认模型”），活动会话时再列其原生模型选项。每个 `RuntimeModel` 带 `reasoning_efforts`：原生取 Harness 自身目录（Codex `model/list` 的 `supportedReasoningEfforts`、Claude Code initialize 的 `supportedEffortLevels`、Pi `reasoning` 模型的固定级别），managed 与 `reasoning_efforts` 用例相同；平台据此展示菜单，不另行判断运行时。原生标识不经过网关查找，也不要求已有提供商支持并存。Pi 和 DSH 的 native 选择必须指定 provider ID；Claude Code 使用既有单一 provider，不能另传 provider ID；Codex 可省略 provider ID 以使用现有默认 provider。`create_conversation_with_selection` 和 `send_turn_with_selection` 接受上述选择；原 `create_conversation`／`send_turn` 是合法的 managed 便捷入口。`next_turn_model_selection` 表示当前上下文的下一轮选择意图，managed turn 结束后仍可保持 managed，不声称进程仍在使用网关。snapshot 的 `model_record_key` 仅表示 managed 模型，native 选择返回 None。

模型路由（`save_model_route`／`delete_model_route`，经 `GatewaySummary.routes` 列出）以自己的记录键作为 managed 选择，`runtime_models` 在提供商模型后附加路由（名称带“模型路由”）。解析、Pi 目录与原生注入使用第一个成员的协议与模型参数；只有显式 `CodexResponses` 请求配置才应用 Codex 请求约束，认证来源不决定请求参数。`ModelRoute` 的 `strategy` 明确选择 `Order`（配置顺序）、`Smart`（适用窗口中相对剩余额度的最小值降序）、`Rotate`（均匀轮询）或 `Weight`（按成员正整数权重平滑分配）；仅 `Weight` 填写 `weights`，键为成员模型记录键。`sticky` 适用于四种策略，前端新建默认 true；启用后优先保持合格的成功成员，轮询和权重主要分配新会话，关闭则逐请求分配。资格、额度上限和健康状态优先于黏性。成员删除时同步剪除权重，空路由删除。旧的 `FailoverPolicy/Mode` 与 `failover` 配置字段已删除。

每个提供商拥有 `quota_limits`，条目为稳定窗口 ID 与 `max_used_percent`（0–100 的已用比例阈值），覆盖该提供商的所有路由及直接模型调用；不是 token 计费预算。Codex 的窗口 ID 按来源 `limit_window_seconds` 命名为 `window:<秒数>`，例如 `window:18000`、`window:604800`，不把 primary／secondary 位次当作固定窗口。一个订阅只能绑定一个提供商，登录提交按私有 `(subject, account_id)` 识别重复，不能靠复制提供商绕过上限；相同 workspace 不同主体不被误判为同一订阅。

application 的 `SubscriptionQuota` 持有唯一运行期观测，`provider_quota` 读取状态并在到期时请求采样，`refresh_provider_quota` 显式请求一次采样；两者立即返回，可轮询 `refreshing` 等待结果。状态为 Fresh／Stale／Unknown／Unsupported，保留已过期的窗口供展示但不把它当作新鲜额度。采样仅由查询或使用触发，不是持续定时轮询；按提供商合并并发，全服务最多 4 个任务；槽位满时不会启动或后台排队，返回 `refreshing=false`，后续查询或显式刷新可在槽位释放后重试。正常有效期按提供商分散在 50–70 秒，重置时间更早时使观测过期，由下一次查询或使用触发重取，不推定归零或恢复满额。准备执行时提前请求所选路由的成员额度；有上限但仍无新鲜、适用窗口时暂停该提供商，前端可先刷新并等待 Fresh。无上限的未知实例可作为后备。供应商显式 `allowed=false` 是独立不可用事实，即使没有窗口或百分比低于 100 也暂停；不把它改写成 100%，不实现 credits 自动消费或重置。缺失、负值、非有限百分比、非法时间与错误窗口范围均不能发布为可用观测。

采样只使用精确绑定的 Codex usage 目的 `https://chatgpt.com/backend-api/wham/usage`，复用同一会话的短期认证解析与有界 401 恢复，不扩大推理凭据的可发送地址。采样不保存原始响应；公开状态只有窗口、可用性、时间与固定失败说明。关闭、丢弃应用或更换认证会取消只读采样；已进入的 token 轮换仍完成持久化。观测、采样任务与 Runner 只读视图绑定网关 ID、提供商 ID 和认证 generation，旧结果不能写入新账户；普通 access-token refresh 不改变该绑定。

`SchedulingState` 由 application 跨 Runner 共享，网关 ID 隔离冷却和探测、会话黏性及轮询／权重分配。application 将自己的不透明会话身份以 `SessionIdentity { namespace: "velune.conversation", id }` 显式交给 gateway，不从消息内容猜测；没有身份就不启用黏性。重新登录、换认证和删除提供商同时使旧调度 epoch 失效；旧 Runner 或在途结果不能恢复旧账户绑定。额度和调度状态均不跨应用重启保留。阈值依据有时效的供应商观测暂停新请求，不能保证外部消费和已开始的并发请求精确不越线。

managed 发送只在本轮执行子进程注入网关；正常结束、失败或取消完成后关闭该执行进程和隧道，原生配置文件保持原状。下一轮 native 选择重新以原生 provider 启动。跨实例交接沿用原生历史和关联机制，模型来源不改变交接资格；调用方必须为目标实例显式提供其 native 或 managed 选择，不把来源实例的模型身份自动解释为目标模型。

`RuntimeExecution` 通过 `runtime_execution`／`configure_runtime_execution` 读取与修改；`upsert_runtime_with_execution` 在同一次保存中接纳实例及 typed execution，新建 managed HOME 实例不需要先填假的 agentDir。`RuntimeTransport` 区分 Local 与 Ssh；SSH 使用显式 OpenSSH 入口、目标和可选端口，认证和已知主机仍由用户的 OpenSSH 管理。所有 binary、agentDir、sessionDir、cwd 与 existing HOME 均属于执行机器；远端版本、历史、草稿检查和写入都经 transport 完成，不落回本机同名路径。SSH transport 要求目标具备 Python 3。专用适配器还需独立的 `remote_resources_directory`，application 每次打开期间首次使用时投送已知 helper 与随包解析依赖；网关通过有生命周期的远端 loopback 转发访问。

`RuntimeHome` 区分 Inherit、Existing 和 Managed。本机 Managed HOME 稳定分配到 `VELUNE_HOME/runtime-homes/<实例摘要>`；远端必须提供远端绝对根目录，实例目录在该根下派生。managed HOME 内再建立对应 Harness 的原生配置目录，不复制既有认证或会话，也不随普通配置重置或运行时删除而移除。HOME 不隔离项目设置、系统配置或系统凭据存储，因此不是安全沙箱。新配置经唯一 `runtime_execution` 模块编码在现有 settings 中，不改变 schema 7 的结构或既有绑定 record。

`runtime_home_directory` 只解析实例实际使用的 HOME（Inherit 为 None），不创建目录。Mac 运行时设置已接入 HOME 归属编辑（2026-10-12）；**待实现**：平台 composer 展示并选择 native 模型，以及 SSH 与远端资源目录的原生配置界面；通用 ACP 的命令、参数、环境配置、协商能力、外部登录提示与会话配置编辑同样待平台接入。提供商导入当前仅接受默认本机 execution 配置；SSH、existing HOME 覆盖或 managed HOME 仍可直接执行原生 provider，但不把其目录交给本机来源认证 helper。指定 WSL 主机的真实 SSH 传输已通过合成 Harness 闭环；真实提供商、远端真实 Harness 和前端体验仍待验收，证据见 [本轮任务](../../tasks/runtime-provider-ssh/packet.md)。

通用 ACP 实例通过 `upsert_acp_runtime` 一次保存 `RuntimeInstance`、`RuntimeExecution` 和 `AcpLaunch`。实例只要求 `settings.binary` 为目标机器上的绝对路径；`AcpLaunch.args` 作为独立 argv 传递，不通过 shell 解析，`environment` 由用户显式配置，`acp_launch` 提供 typed 读取。环境值可能含秘密，与普通应用配置一起保存，Debug 仅显示参数数量或配置键，不输出值。应用不复制原 HOME 的认证，不自动执行登录、安装 Agent 或下载 bridge；用户已安装的原生 ACP Agent 或第三方 bridge 均使用同一协议适配器。

ACP 不要求 Node、agentDir、特定历史目录或 helper bundle；SSH 的 `remote_resources_directory` 可省略，`Options.resources_directory` 可以是已有空目录。Managed HOME 只创建该实例稳定的 HOME，不替 Agent 指定私有配置子目录。ACP 初始化直接协商协议，不运行专用 Harness 的版本探测。

`RuntimeTypeDescriptor` 的 `launch_options`、`agent_info`、`session_options` 声明这些 ACP 用例适用于该类型，`remote_resources` 声明 SSH 执行需要远端辅助目录，平台以此决定界面而不比较类型 ID。`acp_runtime_info` 返回 Agent 名称／版本、认证方式描述和实际会话能力。认证信息仅用于说明外部登录途径，不表示 Velune 已认证。`RuntimeDefault` 表示保留 Agent 当前会话的模型选择；无模型选项的 Agent 仍可创建并连续发送。`runtime_models` 总是先返回 `RuntimeDefault`，再读取活动 ACP 会话的首个 model 配置项；未建立会话或没有 model 类别时只有默认项，绝不为探测创建隐藏会话。`acp_session_config`／`set_acp_session_config` 保留原配置 ID、opaque value、类别、分组和 boolean／select 类型，类别可缺省或重复；修改配置和模型继续使用同一活动进程。

ACP 的列表能力缺失与空列表不同；汇总列表通过 `history_failures` 保留不支持说明，当前活动会话仍可显示与使用。`open_acp_conversation(runtime_id, native_session_id, cwd)` 按明确引用加载会话，不依赖列表能力；普通 `open_conversation` 可从 Agent 列表取得真实 cwd。只有 `session/load` 的原生回放可用于读取历史；`resume` 能力不等于历史读取。无 load 的 Agent 在当前进程中支持多轮、权限答复与取消，关闭后的重开明确不支持，不创建新会话冒充恢复。重命名没有通用协议入口，删除按协商能力执行。

同一公开身份的持久跨 Harness 接续要求 ACP 来源和目标都能加载原生历史。能力不足在关闭来源、创建目标或发送之前拒绝，原活动对话保持可用；这不限制普通 ACP 新建或发送。关联段保存真实 cwd 以定位无列表能力的 ACP 会话，仍不保存消息副本。专用 adapter 的 provider overlay 不推广到 ACP，通用 ACP 明确拒绝 managed 模型注入。

资源库（2026-10-11，内部类型仍名 `ResourceTemplate`）：资源是 canonical 定义，保存于 `resource-templates.json`（0600，与普通配置分离、不随配置重置；列表摘要不含内容，`read_resource_template` 显式返回含秘密的内容，Debug 一律遮盖）；`resource-bindings.json` 记录资源在各运行时写入的条目及指纹。`save_resource_template` 保存后立即写入已启用且处于同步状态的运行时，原生被改／被删的运行时等待用户处置，运行中轮次存在时全部留作“待同步”。`resource_sync_status` 返回每个资源在每个运行时的状态（未启用、不支持、已同步、待同步、原生被改、原生被删）；`apply_resource_template` 启用或重试同步，同名非受管条目或另一份指令需显式 `overwrite`；`resolve_resource_binding` 覆盖、停止同步（保留原生）、删除原生，或以原生内容更新资源（`TakeNative`：读回原生条目替换资源内容，保留资源的身份与名称，该运行时按规范形式重写并以此为新基线，其他运行时随之按保存同步）；`resource_native_import` 先返回更新后的资源与无法带入的字段，不保存。`unmanaged_runtime_resources` 列出各运行时未被资源库管理的用户级条目，`import_runtime_resource` 尽力读回为未保存的资源并列出无法带入的字段，`adopt_runtime_resource` 将其加入资源库并启用于来源运行时（原生改写为规范形式）。`resource_drift` 返回原生被改／被删时逐文件的原生与资源库内容（MCP 为排版后的 JSON），供用户覆盖前比较。保存时若运行中轮次或协作 scope 使写入延后，核心记下待补写，在后续请求中发现运行时空闲后自动写入待同步的绑定。MCP 资源含可选工作目录与超时（秒），各运行时按原生字段渲染，无法表达的字段在写入报告中说明。子 Agent 的模型二选一（都不填则沿用父会话；两者都填或 Velune 模型不存在时拒绝保存）：`model` 是运行时提供商的 model ID，写入原生定义，经网关的轮次中，该运行时已启用的这类子 Agent 的 ID 作为该轮网关别名指向 Velune 中唯一具有此提供商 model ID 的模型（与该轮自身模型同名时让位）；`velune_model` 是 Velune 模型的记录键，此时不写入任何原生文件，启用记录没有条目仍算启用，只在经网关的轮次中由 `turn_subagents` 交给适配器（模型名为网关始终可路由的 `velune/model/<记录键>`），运行时用自己的提供商时看不到它。写操作遵守资源写入闸门；删除资源或运行时实例只丢弃启用记录，不触碰原生条目。手动验收见 `scripts/manual-resource-library.py`；真实 Harness 加载见 `scripts/manual-resource-library-native.py`。

`working_changes` 与 `file_diff` 以本机 `git` CLI 只读会话工作目录（取自打开的会话或最近列表）所在仓库：状态解析为新增、修改、删除、重命名、未跟踪与冲突，相对 HEAD 的增删行数（二进制与未跟踪文件无行数），单文件差异解析为 hunk、上下文、增删与说明行，超过 1 MiB 截断；未跟踪文件以新文件显示。文件路径只接受仓库内相对路径；执行时设 `GIT_OPTIONAL_LOCKS=0`，不刷新索引、不与 Harness 的 git 操作争锁。远端运行时的会话暂不支持。[变更审阅手动验收](../../scripts/manual-working-changes.py) 以临时仓库检查各类变更、差异行、仓库根解析与路径拒绝。

## 提供商能力组合与 Claude CLI

`ProviderDraft` 明确保存 `execution_backend`（`Http` 或 `ClaudeCli { binary }`）、`request_profile`（`Native` 或 `CodexResponses`）、`authentication_source` 与 `quota_source`。认证来源为 `None`、`ApiKey`、`CodexOAuth`、已导入的 `RuntimeProvider` 或 `ClaudeCli`；额度来源为 `None`、`CodexUsage` 或 `ClaudeCliUsage`。公开摘要返回同一组字段；私有认证材料持有来源与 generation，不另建账户注册表。新建普通 HTTP 提供商应选择 `Http`、`Native` 和 `None` 额度来源。`AuthenticationEdit` 仅保留 `Keep`、`SetApiKey` 与 `Clear`：切换来源或清除会初始化所选来源的空认证，保存原登录使用 `Keep`。运行时来源只能由导入建立。

组合在保存边界校验。`CodexOAuth` 必须显式配合 HTTP、Responses、`CodexResponses` 及固定 Codex 地址；`CodexUsage` 仅接受该认证来源。`ClaudeCli` 执行必须配合 Messages、`Native`、空 HTTP 地址及 CLI 认证；`ClaudeCliUsage` 读取同一个 CLI 登录目录。HTTP credential 与本机 CLI session 是不同材料类型；gateway 仅依据中立 recovery capability 发起认证恢复。Pi 来源的输出上限限制保留为独立 request capability，不再附带禁用 temperature、cache 或强制 stream。

Claude 登录复用 `authentication_start`、`authentication_poll`、`authentication_cancel` 和 `authentication_inspect`；`authentication_logout` 执行官方 CLI 退出并使绑定失效。应用运行未修改的 `claude auth login`，该命令默认订阅登录；完成后执行默认 JSON 的 `claude auth status`，只接受 `authMethod=claude.ai`。每个提供商通过显式 `CLAUDE_CONFIG_DIR` 使用独立目录。命令从该目录运行，保留正常操作系统 `HOME`、`USER`、`LOGNAME` 与 `PATH`；macOS Keychain 依赖真实用户 HOME，不能用暂存目录替换。其他环境变量不继承；应用不读取 CLI 的凭据文件、Keychain 或 token。CLI 自己负责刷新。登录先暂存到新目录，配置提交成功后才替换旧绑定。取消、重复绑定、配置提交失败或丢弃未提交登录时，原有 worker 在子进程结束后通过有界官方 logout 清理暂存身份，再删除目录；清理错误记录固定诊断，不输出来源内容。状态摘要仅显示散列尾号；优先采用 CLI 给出的稳定主体标识，缺失时以规范化 email 与 orgId 的摘要尽力去重，不声称能严格证明订阅身份，且不保存 email。generation 才是本机绑定生命周期的依据。

应用共享执行池，跨 Runner 保留进程内会话；网关显式传入会话身份。换绑、删除、退出或关闭停止旧绑定，取消停止当前交互。真实新用户轮才调用 `begin_user_turn`，Harness turn 结束调用 `finish_user_turn`，释放仍在执行或等待工具的交互，保留已完成的空闲进程；普通准备或 Runner 替换不清理会话。不承诺应用重启后恢复 CLI 会话。CLI 的工具请求由网关桥接，应用不让它直接执行项目工具。

CLI 额度来源执行内置 `/usage`，关闭工具、外部 MCP、设置来源与会话持久化。仅解析可验证的 `Current session` 与 `Current week (all models)` 百分比，映射为 `window:18000` 和 `window:604800`。未知文本、缺失窗口、进程失败不产生满额快照；保留上次观测并给出未知或过期状态及安全错误。人类可读 reset 文本不被猜测为时间戳，按现有短 TTL 在下次查询或使用时重新观测；不会推断额度恢复。模型特定窗口没有稳定 model ID 映射时不伪装为全模型窗口。额度是按需观测，不是持续轮询。

`/usage` 适配依据是当前官方命令语义与固定 Magpie 实现，CLI 的版本及输出变化可能使观测不可用。Velune 只执行固定命令，绝不在失败后退回普通 prompt 或换参数重试；输出校验只能判断返回是否符合额度格式，不能保证外部 CLI 内部实现或计量永远不变。Claude Code 2.1.292 的官方订阅登录和额度读取已通过真实验收；当前账户 5 小时窗口耗尽，真实文本请求返回限流，正常文本及工具往返尚待额度恢复后验收。证据归 [模型路由任务](../../tasks/model-routing/packet.md)。

Claude 认证失败的公开 `AuthenticationEvent.error` 只包含固定阶段与原因，格式为 `Claude Code 认证未完成 [phase=<阶段>;reason=<固定原因>;exit_code=<整数或none>]`。阶段为 `login`、`status` 或 `worker`，用于区分 CLI 退出、超时、状态 JSON、目录绑定与身份校验失败；不含 CLI 的 stdout、stderr、账户信息或 URL。进程退出码来自操作系统，不按原始错误文案猜测供应商原因。
