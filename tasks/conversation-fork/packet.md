# 会话 fork：turn 末尾消息底部的 fork 按钮

用户授权实现「会话 fork」：在转录区每个 turn 的收尾回复消息底部、copy 按钮旁边显示 fork 按钮；点击后以该 turn 结束处为边界分叉出新会话（包含该 turn 及之前的全部内容），原会话不变，新会话在新标签页打开，等待用户输入下一条消息。追加授权：可编辑的用户消息在 copy 旁提供编辑按钮（铅笔），打开现有编辑表单（可选原地或新会话继续）。未授权提交、日常重装或真实数据／模型调用。

## 范围与判断（实现判断，非用户额外批准）

- 平台：仅 Mac。其他平台（Windows／iOS／Android）同样有 copy 按钮，本次不加，沿用此前 Mac 先行的惯例。
- 运行时：仅支持原生可分叉的 Codex／Codex166／Claude Code／Pi；DeepSeek、ACP、跨运行时组合视图、sub-agent 会话不出现按钮（无可靠分叉机制，与消息编辑同边界）。
- 边界语义按 transcript turn：fork 包含该 turn 起始用户消息到下一条用户消息之前的全部内容；最后一条用户消息的 turn 等于分叉整个会话。
- Codex steer 消息开始的 transcript turn 无独立原生边界，fork 按所属原生轮次（最近前有 branch_point 的用户消息）包含整个原生轮次。
- 按钮只在 `followUp.canFork` 时出现（条件同消息编辑：空闲、无待回复请求、无未发送草稿、无父会话、简单来源）。
- Claude fork 的原生会话文件在首轮发送时才实体化（`--fork-session` 在 print 模式生效），与新建会话的草稿行为一致；Codex `thread/fork` 与 Pi fork/clone 立即产生原生会话。

## 技术方案

- conversation crate：`FollowUp` 增加 `can_fork`。
- agent-runtime：`NativeSession::fork_through(message_id)`（Codex：`thread/fork` `lastTurnId`=该消息（或所属轮次）的 branch_point；Claude：下一用户消息的 branch_point 作 `--resume-session-at`，无则整会话 fork，`ClaudeStart::Fork` 的 `at` 改 `Option`）；`PiSession::fork_at(Option<entry>)`（Some→`fork`，None→新增 `clone` RPC）；`client.clone()`。
- application：`local/editing.rs` 增加 `fork_message`（守卫同编辑、新鲜读取校验、focus_new＋回滚），`follow_up.can_fork` 计算；Command/Response/CommandKind 增加 `ForkConversation`；remote controller 白名单同步。
- bindings：`fork_conversation(runtime_id, conversation_id, message_id) -> SnapshotResult`，message_id 为 turn 起始用户消息。
- Mac：TranscriptRow 记录 fork 上下文（turn 的 userMessageId）；TranscriptLayout `messageActions` 在 copy 旁布局 fork 按钮（SF Symbol `arrow.triangle.branch`）；TranscriptCellView 悬停显示、点击经 TranscriptTable 新闭包到 ConversationStore.forkConversation，成功结果走 `openInNewTab(adopting:)`。

## 进展

编辑按钮（追加范围）已实现：TranscriptRow.edit（editable 集合驱动）→ 布局 `.edit` 元素 → MessageEditButton（pencil）→ 现有 `editMessage` 闭包打开编辑表单。

实现已完成（未验证）：conversation `FollowUp.can_fork`；agent-runtime `NativeSession::fork_through`（Codex `codex_fork`／Claude `claude_fork`，`ClaudeStart::Fork.at` 改 `Option`）、`PiSession::fork_at`、`client.clone`；application `editing.rs` 抽出 `branchable`／`simple_source` 共享守卫、新增 `forkable`／`fork_message`，`follow_up` 与 `control.rs` 同步；application-api `ForkConversation` 命令（Effect、changes_shared_state，不要求 revision）；commands 分发与响应映射；remote controller 白名单；bindings `fork_conversation`；Mac 端 TranscriptModel（row.fork=turn userMessageId）→ TranscriptLayout（`.fork` 元素，copy 旁）→ TranscriptTable（MessageForkButton、悬停淡入、闭包）→ TranscriptView/Panes 接线 → ConversationStore.forkConversation → Transport。`cargo check` 六 crate 通过。文档已更新：conversation-view PRD、app/mac README、agent-runtime README、application README。

## 收尾验证与修复

2026-11-04 工作区收尾授权提交，取代本包起始的“不提交”约束。实际 CLI＋合成 HOME／回环上游验收发现并修复三条关键链路：

- Codex 源 runner 停放仍持有 writer：分叉前释放为历史，原生分叉后结束同时拥有源和分支的短寿命 app-server；首发只恢复分支。编辑分叉的原子首发则先取得新分支的 writer。
- Claude live 用户消息临时 ID 与历史 UUID 不同：按用户序号及正文校验后采用 fresh 原生 ID。
- Claude 首发前换模型会丢失延迟 fork：Session 保留 adapter 不透明 PendingFork，重配后恢复、实际首发后清除。末轮分叉也固定原生终点，不带入源会话后来追加。
- Codex 尚无自身轮次的继承分支不出现在列表：复用 reach 权威读取，不能将列表缺席当作不可读取。

Pi 原生分叉边界和源保留已通过。Codex0.166的分叉边界／源继续、继承分支读取／续发／重开、编辑第二条消息后分叉并发送均通过；Claude2.1的live分叉、源继续、首发换managed模型、重开及末轮边界封闭均通过。两库strict Clippy与最终release bindings构建通过；Mac已有完整构建及原生离屏按钮视觉证据，未使用CUA或重跑无关路径。精确日志见[收尾任务](../workspace-delivery-and-network/packet.md)。

尚未首发的 Claude 分支仍为进程内原生准备状态，不保证跨应用重启恢复；本次没有把延迟分支扩展为新的持久化机制。Codex0.159未补真实fork CLI验收，版本共用代码经构建检查。
