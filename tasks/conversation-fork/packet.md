# 会话 fork：turn 末尾消息底部的 fork 按钮

用户授权实现「会话 fork」：在转录区每个 turn 的收尾回复消息底部、copy 按钮旁边显示 fork 按钮；点击后以该 turn 结束处为边界分叉出新会话（包含该 turn 及之前的全部内容），原会话不变，新会话在新标签页打开，等待用户输入下一条消息。追加授权：可编辑的用户消息在 copy 旁提供编辑按钮（铅笔），打开现有编辑表单（可选原地或新会话继续）。

## 已实现（已交付，见[工作区收尾](../workspace-delivery-and-network/packet.md)）

- 核心：`FollowUp.can_fork`（conversation）；`forkable`／`fork_message`（application `local/editing.rs`，守卫同消息编辑）；`ForkConversation` 命令（application-api／application commands／remote 白名单）；`fork_conversation`（bindings）。
- 运行时：`NativeSession::fork_through`——Codex `codex_fork`（`thread/fork`，steer 消息按所属原生轮次）、Claude `claude_fork`（下一用户消息前一条记录，`ClaudeStart::Fork.at` 改 `Option`，末轮整分支复制）；`PiSession::fork_at`（下一条目前 `fork`，末轮 `clone`）。
- Mac：`TranscriptRow.fork`（turn userMessageId）／`.edit`（editable 集合）→ 布局 `.fork`／`.edit` 元素（copy 旁，操作行按钮 hug 边缘）→ `MessageForkButton`／`MessageEditButton`（悬停淡入）→ Panes 接线 → `ConversationStore.forkConversation`（新标签页 `openInNewTab(adopting:)`）与既有 `editMessage` 编辑表单。
- 收尾任务在原实现上进一步加固（归其任务记录）：Codex 源会话停放为历史再分叉、Claude 延迟 fork 的 PendingFork 跨重配保留、`import.rs` 编译修复；均已提交。

## 验收状态

实际 CLI＋合成 HOME／回环上游：Pi 原生分叉边界与源保留、Codex 0.166 分叉边界／源继续／继承分支续发重开／编辑后分叉发送、Claude 2.1 live 分叉／源继续／首发换模型／重开／末轮边界封闭全部通过；workspace／remote-only 严格 Clippy、release bindings、Mac SVC build2、Windows／iOS／Android 构建通过；原生离屏视觉核对（宽 1100／窄 560，复制／编辑／fork 按钮）通过。证据在 `target/verification/workspace-delivery-20261104`（忽略、不入 git）。

已知限制：Claude 尚未首发的分支是进程内原生准备状态，不保证跨重启恢复；Codex 0.159 未补真实 fork CLI 验收（共用代码经构建检查）；Windows／iOS／Android 未加按钮入口。

## 运行中可用（2026-11-04 用户修订）

用户指出：fork／编辑按钮不应因会话运行而整会话隐藏。已实现：

- 核心：`TranscriptTurn.can_fork` 按 turn 判定（已完成 turn 在可分叉运行时上为真；未完成 turn 为假），application `stamp_forkable_turns` 打戳（live 快照与只读历史两处）；`fork_message` 守卫改为按 turn（`forkable_turn_users`），不再要求空闲；`FollowUp.can_fork` = 任一 turn 可分叉。
- 编辑：`editing_choices` 去掉空闲／队列／审批等守卫，仅保留 sub-agent 父会话、简单来源与运行时能力；`edit_message` 对 in-place 在 `turn_running()` 时先 `self.cancel(runtime_id)` 停止当前 turn 再继续（用户确认的选择）。
- Mac：`TranscriptModel` 直接用 `turn.canFork`；`editability` 去掉 `canSend` 守卫；`ConversationStore` 新增 hold 语义——`forkConversation` 与编辑的 fork 放置（`editMessage` placement == .fork）在运行中点击时挂起（`heldFork`／`heldEditFork`），快照回到空闲（`forkReady`）时自动执行，底部提示条显示“已安排：当前 turn 结束后自动分叉”（`heldForkNotice`，TranscriptView 新增 `forkNotice` 参数）；切会话清空。编辑表单：运行中显示说明文案，发送不再要求 `canSend`（只看 editability），确认对话框标注“先停止当前 turn”。
- 守卫保留：sub-agent（父会话）、跨运行时组合视图、只读／异地视图（control.rs 同步清除 per-turn can_fork）、DeepSeek／ACP。

## 关联会话的翻译分叉（2026-11-04 用户修订）

用户指出参考文本注入不可取（重蹈覆辙）：已删除 `edit_from_reference`（reference_context 机制不再有生产者，字段与消费端暂保留），改为翻译分叉 `translated_branch`：逐段导出原生有效上下文（`export_pi_context`／`export_context`，Pi／Codex166／ClaudeCode 段），按边界消息正文在导出上下文中定位截断（压缩／改写则拒绝；合并的 steer 消息整体保留，符合原生轮次粒度），拼接后整体翻译写入当前上下文运行时的新原生会话（ClaudeCode／Codex166／Pi 可导入；复用 handoff 的导入逻辑但不链接进源会话），随后发送编辑消息或（fork）等待输入。守卫：`translatable`（关联会话全部段可导出＋上下文运行时支持导入）放开 `editing_choices` 与 `forkable_turn_users`；in-place 不适用（只能新会话继续）。`RuntimeKind::Codex`（0.159）已退役，codex-0.159 段的旧链接在新代码下不翻译（守卫自然为假，安全降级）。application README 已同步。

严格 clippy（六 crate --all-targets）与测试通过；`svc run build-macos` 通过（Swift 侧运行中可用与提示条改动编译验证）。GUI 与真实三端运行中原生路径补验未做；日常应用未重装。

## 架构自查与纠正（2026-11-04 用户要求）

审查发现五处并已全部纠正：

1. **打戳守卫依赖被寻址会话**（治标不治本）：`simple_source`／`translatable` 原读 `session.logical_projection`，只读路径打开其他关联会话时用错对象（被 control.rs 清零掩盖）。已改为按会话 id 从 `conversation_links` 查（`link_of`）。
2. **核心层无运行中保护**：`fork_message` 去掉空闲守卫后，非 Mac 客户端可在运行中调用，Codex 路径 `invalidate_execution` 会杀死正在运行的 turn。已加核心 typed 拒绝 `Error::Busy`（Failure code `"busy"`，沿用 `control_held_elsewhere` 先例），Mac held fork 触发遇 busy（turn 重启竞争）自动重新挂起。Mac 挂起由此从安全机制降为体验优化。
3. **截断匹配误判风险**：`truncate_context` 原用拼接文本 contains 定位边界，短消息作为子串出现在更早长消息里会截错。已改为 Text 块级精确匹配优先、唯一 contains 兜底、歧义拒绝。
4. **导入逻辑重复**：`import_translated` 复制了 `continue_imported` 约 80 行导入分发。已抽公共 helper `import_context_to`（continuation.rs），handoff 与翻译分叉共用。
5. **held fork 随 tab 关闭丢失**：挂起存在 tab 级 store，关闭即丢。暂不修（修需核心意图队列；意图由发起视图持有是 v1 可接受语义），记录为已知限制。

Pi in-place 中断时序经核实安全（RPC `abort` await 到 turn 真正结束，rewind 有序）。

## 当前修复：fork 交互与配置就绪

用户授权修复 fork 的模态确认、复制加载态、分支名称 ` (n)`、新分支执行配置等待过长，以及原生优先／翻译兜底。不修改日常会话或配置，不重装日常应用，不自动提交。

日常日志只读诊断发现 2026-11-10 的两次 `bindings.fork_conversation` 耗时 40.0 秒和 39.6 秒，源 Codex 历史约 3.26 万 items；同期存在 `composer_target_identity` 的“目标会话尚不可用”，但日志没有足够目标身份，不能断言这些查询属于新分支。源码确认两个会阻塞或错投执行配置的路径：新 tab 的普通启动可先全量扫描列表，fork 后也立即全量刷新；接收新分支固定使用窗口默认设备，而非发起设备。修复改为直接接收已打开的快照、发布新分支摘要，不在 composer 前全量扫描，并保留设备。

已实现：系统模态确认、等待与实际复制的不同状态、复制期间进度指示和重复请求抑制；原生名称使用 ` (n)` 后缀，Claude 未首发名称随原生 fork 和首次启动参数保存。当前运行时实例内的关联段也优先原生 fork；只有原生明确拒绝或创建前不可用才翻译兜底，结果不明或已创建副本后不重复创建。翻译保留所选轮次的回复／工具结果（原实现只截到用户消息）。日常数据诊断之外的验收只用合成目录。已完成实现与本地验收，用户已授权提交；未重装日常应用。

最终核心证据：application 库测试 82 项、agent-runtime 库测试 48 项全部通过；两个 crate 的生产库严格 Clippy 通过。覆盖 Claude 原生待首发名称及边界、Codex 明确拒绝／未知结果／已创建但打开失败的合成 JSON-RPC，以及翻译 fork 的整轮回复／工具结果边界和数字后缀。日志保留在 `target/verification/conversation-fork/core-tests.log`。

提交前另在原始 HEAD 加本轮补丁的隔离工作树验证：两个生产库严格 Clippy 通过，agent-runtime 40 项测试全部通过，application 64 项通过、1 项输入迁移测试失败。移除本轮补丁后，原始 HEAD 的 `composer::tests::borrowed_input_promotes_without_consuming_new_and_legacy_edits_preserve_content` 同样失败（跨执行主机的 runtime ID 区分断言，0 != 2），确认不是本轮回归。提交保留基线消息编辑 API，不夹带工作树中其它编辑器／远控／运行时修改；翻译兜底仅补入所需的 `segment_reference` 共享可见性。隔离证据见同目录的 `commit-isolation.log` 与 `baseline-check.log`。

Mac 最终 `svc run build-macos` 成功，execution `c1b24e29-057b-42ab-8fdb-28790e543fd4`，构建记录 `current=true`、输入前后身份一致、`changed_inputs=[]`。`--check-message-experience --experience-fork-only` 通过：来源设备保留、直接接收快照、新分支立即改运行时／推理配置、名称摘要与加载态。已检查 `target/verification/conversation-fork/fork-loading.png`；构建记录和 UI 日志同目录。共享 bundle 可能被下一次构建改写，因此检查在 `macos.lock` 内复制并验签独立 App，使用私有临时 HOME 与准确子进程句柄；副本和合成目录已自动清理。

验收边界：`--all-targets` 严格 Clippy 曾被本工作树其它任务的测试 lint 阻断（`local/browsing.rs`、`local/configuration.rs` 的 items-after-test-module，以及 `persistence_migrations.rs` 的默认值后赋字段和裸 `Command::new`），不宣称全目标 lint 全绿；本轮未接管这些修复。共享导航合成检查的编译警告已由其修改者补齐。没有在日常会话中创建新分支或重放模型请求。独立诊断子会话因宿主后台能力与虚拟模型注册均未启动成功，没有产生修改。
