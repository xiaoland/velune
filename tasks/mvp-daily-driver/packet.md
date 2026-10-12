# 日常主力 MVP

## 当前任务

2026-10-11 用户目标：继续推进 Velune 到 MVP，逐步解除对 Codex App 的 vendor-lock，并提供更强的软件开发能力；请开发方提出需要尽快落地的功能／改进。同时反馈细节问题：composer 模型无默认值；Markdown 表格缺少下边距；会话读取是静态的、缺少“正在进行”信息（会话 SSoT 在运行时侧，best-effort，可加手动刷新）；删除会话后侧栏仍显示但无法打开；Pi 运行时编辑表单难看难用；“回到底部”按钮应为矩形并注意与滚动条的间距。用户决定已写入 [PRD](../../docs/prd/index.md)。

授权沿用持续迭代：Mac／core 改动自主本地提交并重建安装；不 push、不新增自动化测试、不读取真实会话或凭据、不调用真实提供商。

## 观察（main `649c863`）

- 每个窗口是共享应用 store 的 workspace；删除／重命名返回的会话列表只写入 workspace，之后应用级旧列表被复制回来，已删除会话重新出现（HEAD 的 Store 在临时脚本中复现）。
- 核心 `CoreRuntime` 只有一个 `active_state`：同一时刻只有一个会话在执行，执行时侧栏整体禁用、不能切换会话。打开的历史会话是 `ActiveState::History` 静态快照，轮询不会重读运行时文件。
- composer 模型只来自本会话意图、快照模型或本次启动内最近选择，新启动或原生模型会话为空。
- 表格：TextKit 不可靠地处理 `NSTextTable` 自身的 margin（下方被忽略，上方使表格变窄）。
- Pi 是唯一有四个路径字段、两个自动发现按钮的类型；分组表单把占位文字显示成字段后的“必填／可选”标签，路径右对齐挤在窄列，还显示原始版本正则。

## 本轮修改

- 删除：workspace 的列表结果同时交给应用 store。
- 运行指示：应用正在执行的会话在侧栏标题旁显示小型进度指示（所有窗口）。
- 手动“重新读取”（详情工具栏，⌘R）：从运行时重新读取会话列表与当前会话；执行中不可用。
- composer 默认模型：本会话意图／快照模型 → 最近选择 → 第一个兼容模型；切换运行时后不兼容时同样回退。
- 表格前后的间距放在相邻段落，表格本身不设 margin。
- 运行时编辑：路径字段改为标签在上、整行输入框加按钮、说明在下；去掉版本正则行与 Swift 端未用的 `versionRegex`；标题补空格。
- “回到底部”改为标准矩形 bordered 按钮，保留滚动条间距。

## 验证

- Swift 严格构建通过；核心未改。
- 临时 Swift 脚本（未提交）编译生产 `Store`，临时 HOME 下用 Pi 1.0.2 SDK 合成三个会话、本机慢速 loopback 上游：默认模型为第一个兼容模型；删除后侧栏不再出现且应用级更新后仍不出现；外部向当前会话追加消息后“重新读取”显示新消息；发送后两个窗口的运行指示都指向该会话，结束后清除。同一脚本对 HEAD 的 Store 复现无默认模型与删除残留。
- 截图：preview 中段落／表格／引用的间距一致；临时 HOME 中 Pi 编辑表单新布局。
- 未验证：长会话中“回到底部”按钮外观（未构造长会话）；侧栏右键菜单无法经 AX 打开，删除仅由脚本验证。

## 路线（2026-10-11 用户确认 1–5 的顺序；6 暂缓，待决定 Velune 是否管理 git worktree）

按“能否每天替代 Codex App”排序：

1. 多会话并行：核心从单一 `active_state` 改为按会话的执行，执行中可浏览／发送其他会话，侧栏不再整体禁用。
2. 会话实时状态：打开的会话在运行时文件变化时增量重读；按各 Harness 记录推断外部进行中的 turn（如 Codex rollout 的任务开始／完成事件、Claude Code 与 Pi 末条消息的停止原因），标为“可能仍在运行”，保留手动重新读取。
3. 完成与待审批通知：系统通知与 Dock 标记，配合并行。
4. 变更审阅：按会话工作目录查看 git 改动与逐文件差异，可在编辑器中打开；以后再考虑暂存／提交。（已实现但入口关闭，用户尚未决定纳入。）
5. composer 增强：图片／文件附件、@ 文件引用、推理强度（提供商模型已有推理级别）、技能／命令入口。
6. worktree 隔离：新会话可在独立 git worktree 中运行，便于同一仓库并行任务。
7. 之后：跨 Harness sub-agent（PRD 的核心协作能力）建立在 1–3 之上。

## 安装

`48ccb32` release 构建已安装 `/Applications/Velune.app`：0.1 beta.1、config schema 8，严格签名通过；正常退出旧应用后安装并重新打开，无崩溃报告。manifest dirty=true 仅因工作区未跟踪的 `app/mac/.DS_Store` 与 `app/mac/Assets/`（非本任务文件，未删除）。

## 仓库卫生（2026-10-11）

用户指出 `.DS_Store` 未被忽略说明 `.gitignore` 不完善，且仓库接近 monorepo，各 unit 至少应有自己的 `.gitignore`。已补根目录的系统／编辑器元数据规则，并为 packages 下七个 unit、`app/mac`、`app/host` 各加 `.gitignore`（单独构建输出、npm／Go 手动构建产物、SwiftPM／Xcode 用户状态）；没有已跟踪文件被新规则命中。

## ① 多会话并行：设计（2026-10-11）

现状：执行相关状态（`active_state`、`pi`、网关 runner／隧道、执行运行时、模型选择、未提交草稿、逻辑投影、transcript 投影、协作范围）是 `CoreRuntime` 的单份字段，所有会话用例都围绕“当前会话”编写。

- **会话通道 `Session`**：把上述字段收进一个结构，`CoreRuntime.session` 为当前聚焦的会话，其余在 `parked`。切换是整体 swap，既有用例逻辑不改写。备选“每个用例显式接收 `&mut Session`”改动面大，留待后续需要时再做。
- **身份与路由**：会话以公开会话 ID 标识（逻辑链接用 origin，未提交草稿用其公开 ID）。open／sendTurn／cancel／回复交互／重命名／删除／ACP 会话配置先聚焦目标会话（没有则新建空通道）；getSnapshot／getSnapshotUpdate 只读已有通道，没有则 absent，不新建；create 总是新通道。`cancel` 与 `replyRuntimeInteraction` 增加会话 ID（hard-cutoff）。
- **推进**：每个请求先推进所有通道（swap 进入、drain、swap 回）。每个会话同一时刻最多一个 turn。
- **保留**：执行中、有待处理交互或有协作范围的通道不淘汰；空闲通道按最近使用保留 8 个，淘汰时正常关闭。
- **隔离**：同一运行时的并发执行各用一个 lane 目录（`runtime-projections/<runtime>/lane-<n>`），避免 Pi 的 models.json／选择文件与 DSH 网关补丁互相覆盖；lane 取未被其他存活通道占用的最小编号。
- **全局操作**：配置类检查从“当前会话忙”改为“任一通道忙或有协作范围”；提供商／运行时／认证变更对每个受影响通道执行原有失效逻辑。
- **新接口 `active_conversations`**：列出执行中或有待处理交互的会话（上下文运行时、会话 ID、运行状态、待处理交互数），供侧栏指示与后续通知轮询。
- **Mac**：每个窗口轮询自己打开的会话；应用级 store 轮询 `active_conversations`；执行时不再禁用侧栏、切换与新建会话；重命名／删除拒绝正在执行的会话；配置编辑在任一会话执行时仍被阻止。
- 协作 API（脚本使用，Mac 未接入）仍作用于当前聚焦的会话。

## ① 实施与验证（2026-10-11）

- core：`local/session.rs` 的 `Session` 收拢执行字段，`CoreRuntime.session`／`parked`；请求先按会话 ID 聚焦再推进全部会话，非聚焦会话的推进失败只记日志；配置门禁改为 `execution_active`，提供商／导入／登录／运行时变更对每个会话执行原失效逻辑；lane 目录用于 Pi models／selection 与原生 projection；`active_conversations`；`cancel`／`reply_runtime_interaction` 增加会话 ID；重开执行中的会话返回运行视图。
- Mac：应用级 store 轮询 `activeConversations`，各窗口轮询自己的会话；执行中不再禁用侧栏、切换与新建；重命名／删除排除执行中的会话；配置编辑在任一会话执行时仍禁用。
- 静态检查：fmt、clippy（workspace all-targets all-features）、`--no-default-features`、Swift 严格构建。
- `scripts/manual-parallel-sessions.py`（新增）：同一 Pi 运行时两个会话同时执行、各自 lane、重开运行中的会话、配置门禁、先后独立结束、取消一个不影响另一个——通过。
- 临时 Swift 脚本（未提交）编译生产 Store：一个窗口发送 A 后切到 B 再发送，两个都在运行；另一窗口打开 A 显示运行中且可停止；两者分别结束——通过。可见窗口截图确认侧栏两个会话都有运行指示、A 实时显示。
- 既有脚本：`manual-runtime-interactions`、`manual-acp-adapter`、`manual-resource-library`、`manual-continuation-boundaries` 通过（前两者已改用新签名）。`manual-session-loading.py` 在本改动前已因 `createConversation` 签名过时无法编译，未修；`manual-cross-harness`／`manual-harness-interoperation` 需要 DSH／OpenCode，本机没有，协作路径未复验。
- 未验证：Codex／Claude Code 原生会话的并发执行（逻辑与 Pi 同路径，未用真实 CLI 并发跑）；SSH 运行时的并发。
- 安装：`994b7c4` release 构建已安装 `/Applications/Velune.app`（0.1 beta.1、config schema 8、manifest dirty=false），严格签名通过；正常退出旧应用后安装并重新打开，无崩溃报告。

## ② 会话实时状态（2026-10-11）

- 观察：Mac 打开会话使用只读 `readConversation`，核心不保留会话通道，窗口轮询一直返回 absent；因此打开的会话从不实时更新，此前只能靠“重新读取”。
- 核心：被 snapshot 轮询而没有通道的会话由首次轮询读入并打开通道（读失败仍返回 absent）；`live.rs` 对本机 Codex／Claude Code／Pi 运行时每秒比对原生文件 mtime／长度，未在 Velune 执行且有改动时重读（空闲执行先释放），按读取耗时放宽间隔；自身执行中及结束 5 秒内的写入视为自己的。
- 外部进行中：`agent-runtime::activity` 读文件末尾 256 KiB 判断 turn 是否未结束；10 分钟内写入的未结束 turn 为新增 `RunState::Elsewhere`，不能取消、仍可发送；`active_conversations` 每 5 秒扫描会话目录附加 Elsewhere 会话，排除 Velune 正在或刚刚执行的会话。Codex 0.159.3 二进制同时含 `task_*` 与 `turn_*` 两套事件名，两者都接受。
- Mac：侧栏对 Elsewhere 会话显示同样的小型进度指示（提示“可能正在 Velune 之外运行”）；对话底部状态行说明内容随原生记录更新；出现列表中没有的外部会话时自动重新读取列表；Elsewhere 不锁定配置，不阻止发送。
- 不跟随：SSH、ACP、DeepSeek Harness、跨 Harness 续接的逻辑会话；这些仍靠手动重新读取。

验证：

- 静态检查 fmt／clippy（workspace all-targets all-features）／`--no-default-features`、Swift 严格构建通过。
- `scripts/manual-live-sessions.py`（新增）：另一进程经 Pi SDK 追加消息——打开的会话重读并标 Elsewhere；外部回复后恢复空闲；外部新建会话被报告并列入列表；自身执行及其尾随写入从不报告为 Elsewhere；自身执行后的外部改动被重读，下一 turn 接续改动后的会话——全部通过。
- 临时 Rust 程序（/tmp，未提交）以合成 Codex rollout 与 Claude Code JSONL 检查 turn 边界、中断、本地命令、sidechain、sub-agent 目录、定位、最近扫描与截断尾部——通过。
- Mac：隔离 HOME 的开发构建打开会话 A，外部进程追加提示后数秒内侧栏出现指示、消息列表出现新提示与 Elsewhere 状态行；追加回复后指示与状态行消失（窗口截图）。
- 既有脚本 parallel-sessions、runtime-interactions、acp-adapter、continuation-boundaries、resource-library 通过。
- 2026-10-11 补验（用户提示可用独立 HOME）：`scripts/manual-live-native.py` 以临时 VELUNE_HOME 中 managed HOME 的 Codex 0.159.3 与 Claude Code 2.1.286 实际 CLI、回环合成上游（Responses／Messages，按 turn 闸门挂起）验证：外部 `codex exec`／`claude -p` 新建的会话被报告为 Elsewhere 并列入列表，结束后消失；已打开会话在外部 `exec resume`／`--resume` 期间显示新提示与 Elsewhere，结束后重读出回复——全部通过。实际 rollout 的 turn 事件为 `task_started`／`task_complete`。未读取任何真实 HOME 或会话，未调用真实模型。
- 观察：Mac 运行时设置尚未接入 application 已有的 `RuntimeHome`（继承／指定／managed HOME），只能经绑定配置。
- 未验证：实际外部 Harness 长时间无写入超过 10 分钟时显示回到空闲。
- 安装：`fd2e7cd` release 构建已安装 `/Applications/Velune.app`（0.1 beta.1、config schema 8、manifest dirty=false），严格签名通过；正常退出旧应用后安装并重新打开，无崩溃报告。

## ③ 完成与待确认通知（2026-10-11）

- 核心：会话的 turn 从执行变为结束时记录结果——用户停止为 Cancelled；推进出错、快照 Failed 或 Pi 以 `stopReason=error` 结束模型响应为 Failed（Pi 把模型错误记为 assistant 消息且会话仍可用，投影新增 `turn_failed`）；其余为 Completed。`active_conversations` hard-cutoff 为 `conversation_activity(since)`：活动会话 + 游标后结束的 turn（序号递增，保留最近 64 个）+ 新游标。
- Mac：`Notifications.swift` 的 `ConversationNotifier` 由应用级 store 持有；完成、失败与待确认交互数增加时发系统通知（每会话一条，新状态替换旧的），前台正在查看该会话时不发，用户停止的不发；首次需要时请求通知授权（提示、声音），是否显示由系统设置决定。点击通知激活应用并在最近成为 key 的窗口打开该会话。Dock 标记 = 有待确认交互的会话 ∪ 后台期间结束的会话，回到前台清除后者。
- 验证：静态检查与 Swift 严格构建通过；`manual-parallel-sessions.py` 新增结果检查（完成、停止、上游 400 拒绝为失败，同一游标不重复返回）通过；live-sessions 脚本通过。
- 未验证：Mac 的系统通知、授权提示、点击打开与 Dock 标记未实际运行——开发构建与安装版同为 `local.velune.prototype`，首次通知会弹出系统授权提示，属于用户决定，开发方不在调试中触发。所有窗口都关闭时点击通知只激活应用，不新开窗口。
- 安装：`8ea8655` release 构建已安装 `/Applications/Velune.app`（0.1 beta.1、config schema 8、manifest dirty=false），严格签名通过；正常退出旧应用后安装并重新打开，无崩溃报告；安装后 runtime-interactions、continuation-boundaries、acp-adapter 脚本通过。

## ④ 变更审阅（2026-10-11，入口已关闭）

用户随后澄清尚未打算加入文件变更功能（已写入 PRD）：保留核心 API、Mac 视图与验收脚本，Mac 入口由 `changeReviewEnabled = false` 短路，是否启用另行决定。

- 核心：`working_changes.rs` 调用本机 `git`（`-C`、`core.quotePath=false`、`GIT_OPTIONAL_LOCKS=0`、`LC_ALL=C`）。工作目录取自会话通道或最近列表；`status --porcelain=v2 --branch -z --untracked-files=all` 解析为类型化变更，`diff HEAD --numstat -z --no-renames` 给出行数；单文件差异对已跟踪文件用 `diff HEAD`（无提交时 `--cached`），未跟踪用 `--no-index /dev/null`，解析为 hunk／上下文／增／删／说明行，去掉与列表重复的文件头，超过 1 MiB 截断。拒绝绝对路径与 `..`。远端运行时暂不支持。
- Mac：详情工具栏“变更”开关（⌥⌘D）打开原生 inspector：仓库名与分支、文件列表、选中文件的差异（VSplitView）；双击以默认应用打开，菜单在访达中显示／拷贝路径；打开、切换会话、本会话 turn 结束与手动刷新时读取。“重新读取”的帮助文字改为补上自动跟随遗漏的变化。
- 选择 inspector 而非独立窗口：与会话并排、可调宽度，与 Xcode 等原生检查器一致；如需独立窗口或更宽的差异视图以后再调。
- 验证：静态检查与 Swift 严格构建通过；`scripts/manual-working-changes.py`（新增）以临时仓库检查修改／暂存新增／删除／重命名／未跟踪／二进制／冲突、空格与中文文件名、缺少末尾换行、子目录会话解析到仓库根、非仓库目录、拒绝仓库外路径、读取后不留索引锁——通过。隔离开发构建中打开 inspector 与选择文件的窗口截图确认列表与差异着色。
- 未做：暂存、提交、丢弃、行号栏、并排视图；SSH 运行时。
- 安装：`0f6d0ae`（变更审阅入口关闭）release 构建已安装 `/Applications/Velune.app`（0.1 beta.1、config schema 8、manifest dirty=false），严格签名通过；正常退出旧应用后安装并重新打开，无崩溃报告。

## ⑤ composer 增强：附件与推理强度（2026-10-11 设计）

用户决定（已写入 PRD）：本轮只做图片／文件附件与推理强度；非图片文件传绝对路径引用；消息列表显示缩略图与文件名。参考 [OAR](https://github.com/botiverse/oar/tree/67d77ddec470dda5add664644c0cfb29c679660c) 的 `InputImage`（本机路径、png/jpeg/gif/webp、交付前读取，不可读或类型不符则整条输入拒绝）与 Claude `--effort`（2.1.284 探测：每次启动生效、会话不保留，`get_settings` 回读实际值）。

各 Harness 通道（2026-10-11 核对）：

| | 图片 | 推理强度 |
| --- | --- | --- |
| Codex 0.159.3 | `turn/start.input` 的 `localImage{path}`（app-server 生成 schema） | `turn/start.effort`（任意非空字符串） |
| Claude Code 2.1.286 | stream-json user 消息 `image` base64 块 | 每轮子进程 `--effort`（low/medium/high/xhigh/max） |
| Pi 1.0.2 | RPC `prompt.images` base64（`docs/rpc-commands.md`） | RPC `set_thinking_level`（off/minimal/low/medium/high/xhigh/max） |
| ACP／DSH | `promptCapabilities.image` 为真时 `image` 块，否则拒绝图片 | 不提供（ACP 会话配置项另有入口） |

- 契约：`send_turn` 增加附件路径列表与可选强度。core 在边界按扩展名分为图片（png/jpeg/gif/webp）与文件，校验存在且可读；不支持图片的运行时、远端运行时的附件直接拒绝整条发送，不静默丢弃。非图片文件追加到消息文本末尾的“附件：”路径列表。
- 强度：选项 = 所选模型声明的 `reasoning_levels` 与 Harness 接受集合的交集（Pi 的 none→off 沿用既有映射）；空则不显示。未选择 = Harness 默认。按会话记住，与下一轮模型意图一起保存。
- 显示：`MessageBlock` 增加附件块（图片／文件、名称、本机路径或原生内嵌数据的引用）；缩略图走既有按需 payload 读取，不随快照传输图片数据。Codex／Claude／Pi 的实时投影与历史读取把原生图片映射为附件块。
- Mac：输入区附件按钮（NSOpenPanel 多选）、拖放、粘贴图片（写入 VELUNE_HOME 下的附件目录）；输入框上方显示可移除的附件标签；操作行增加推理强度弹出按钮。
- 切片：A 推理强度；B 附件发送；C 消息列表附件显示。

### 切片 A：推理强度（2026-10-11 完成）

- 契约：`ComposedTurn{text, effort}` 经 `send_turn_with_selection` 发送；`reasoning_efforts(runtime, model)` 为 managed 模型给出选项（Codex 原样、Claude 交集、Pi 级别名）。范围外强度在 `send_turn` 改变任何状态前拒绝；native 选择按 Harness 接受集合校验。Mac 只用 managed 模型，选项异步查询，所选强度随下一轮意图按会话保存在工作区内存（不跨应用重启）。
- 实测（`scripts/manual-reasoning-effort.py`，实际 CLI＋回环）：Codex `turn/start.effort` 只在该 turn 发出 Responses `reasoning.effort`，下一 turn 不选时不发送；Claude Code `--effort` 成为 `output_config.effort`，不选时为 CLI 默认 `high`（adaptive thinking）；Pi `set_thinking_level` 生效，下一次执行以 off 开始。因此三者“默认”均为单 turn 外的运行时默认。
- 修复：Pi 虚拟模型路由原先总用选择文件里的 `thinkingLevel: "off"`，忽略会话级别；改为跟随会话级别，选择文件不再携带该字段。无 Pi 投影元数据时 thinkingLevelMap 原把 Pi 级别名当作上游值（声明 `none` 时上游收到 `off`），改为映射回声明的 wire 值。
- 顺带修正 `manual-runtime-management-api.py` 中按会话寻址前遗留的 `cancel(runtime)` 调用。
- 安装：e6910c2 已构建安装到 `/Applications/Velune.app`（签名校验通过，manifest commit 一致、dirty=false，重开无崩溃报告）。输入区“推理”按钮的实际提供商体验待用户验收。

### 切片 B：附件发送（2026-10-12 完成）

- 契约：`ComposedTurn.attachments`（本机绝对路径）。application `attachments` 模块在任何状态改变前检查：普通可读文件；按扩展名分出图片（agent-runtime `TurnImage::read` 校验文件头与 20 MB 上限），其余文件追加“附件：”路径列表；远端运行时与 DeepSeek 的图片直接拒绝；有图片时正文可为空。图片路径随 send payload 传到执行处再读出交付（含跨 Harness 接续）。
- 通道：Codex `localImage{path}`；Claude Code base64 `image` 块置于正文前；Pi `prompt.images`；通用 ACP 仅在 `promptCapabilities.image` 时发 `image` 块，否则发送时拒绝。
- 修复：Pi 网关物理模型与虚拟模型原只声明 `input: ["text"]`，Pi 会丢弃图片；改为 text＋image，由上游决定接受与否。
- Mac：操作行最左的回形针（NSOpenPanel 多选）、拖放到输入区、粘贴文件或单独的图片（存为 `VELUNE_HOME/attachments/*.png`）；编辑区上方为 Quick Look 缩略图＋文件名的可移除标签，按会话保存在内存，发送被接受后清除。Claude 等本地回显的用户消息暂只显示正文，附件显示属切片 C。
- 验证：`scripts/manual-attachments.py`（实际 Codex 0.159.3、Claude Code 2.1.286、Pi 1.0.2＋回环上游）：三者上游收到的图片字节与原 PNG 一致，文本含“附件：”路径行；仅图片的 turn 发出图片；缺失、相对路径、目录、内容不符的 .png 均未发送即拒绝——全部通过。回归 `manual-reasoning-effort.py`、`manual-parallel-sessions.py` 通过。开发版界面（临时 VELUNE_HOME）确认回形针按钮位置与打开的“添加附件”面板；AX 无法在面板中选文件或模拟拖放／粘贴（不改用户剪贴板），附件标签与拖放／粘贴的实际体验待用户验收。
- 安装：0337d14 已构建安装（签名校验通过，manifest commit 一致、dirty=false，重开无崩溃报告）；运行时 HOME 设置随 b1fdbc8 先行安装。

## 运行时 HOME 设置（2026-10-12）

用户要求把 application 已有的 `RuntimeHome` 接入 Mac 运行时设置（已写入 PRD）。

- 编辑器增加 HOME 区：继承／指定目录／由 Velune 管理。managed 时隐藏“运行时目录”（core 从 HOME 推出 `.codex`／`.claude`／`.pi/agent`／`.dsh`）。保存走 `upsert_runtime_with_execution`，保留既有 transport 与远端资源目录；SSH 实例在 managed 时需要远端根目录。
- 已保存的 managed 实例显示 core 解析出的 HOME 位置，可在 Finder 中显示。
- 发现：Mac 保存实例时只回写描述字段，会丢掉 settings 中的 `execution` 等非字段键；改为保留其余键。
- 验证：临时 VELUNE_HOME 中经绑定写入 managed 与指定目录两个实例，`runtime_home_directory` 分别返回 `VELUNE_HOME/runtime-homes/<摘要>` 与指定目录；开发版编辑器（AX 打开）显示对应 HOME 选项、managed 位置与指定目录字段，managed 实例经“保存”回写后 `execution` 保持 managed。AX 无法操作 SwiftUI 弹出菜单与文本框，切换 HOME 选项与改路径未经界面实测。

## 零碎改进（2026-10-12）

用户反馈（已写入 PRD）：composer 弹出按钮指示器用中性色；未结束 turn 默认展开工作过程，运行表达参考 Claude Code／Codex（shimmer）但优先原生；外部运行的说明文字去掉。

- 弹出按钮：`.tint(controlColor)`。对比试验窗口中 `.secondary`／`.gray` 在浅色下近黑、过重，controlColor 呈中灰，接近系统石墨色外观；深色下各中性色接近。
- 工作过程：运行中的 turn 除非用户收起，否则展开；结束后按已完成 turn 的规则（默认收起）。
- 状态行：进度指示＋当前工作（最新推理块首行／运行中工具标题／“正在回复”），文字带 CAGradientLayer 遮罩 shimmer，尊重减少动态效果；“正在停止”不闪；Elsewhere 只显示进度指示。
- 验证：临时 VELUNE_HOME＋Pi 1.0.2＋回环 Messages 上游（思考块后挂起 45 秒）：运行中工作过程展开、状态行显示“Inspecting the project layout”并有高光移过，弹出按钮指示器为中性灰。发现：把开发版设为前台会让用户正在进行的键盘输入落入其输入框，改用 `open -n -g --env VELUNE_HOME=…` 后台启动；AX 点击发送仍会把它带到前台，验证后立即退出。turn 结束后收起与 Elsewhere 只显示指示未截图核对。
- 安装：483887e 已构建安装（签名校验通过，manifest commit 一致、dirty=false，重开无崩溃报告）。
