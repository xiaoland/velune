# 会话菜单的停止、继续与 sheet 发送

2026-10-29 用户解除功能冻结后要求（[S56](../../docs/sources.md#s56)）：会话的右键菜单能停止与继续会话，并能在 sheet 中打开输入区发送一次消息；首个 beta 前只做 Mac。产品规则见 [会话管理](../../docs/prd/conversations.md#会话管理) 与 [在 sheet 中发送](../../docs/prd/composer.md#在-sheet-中发送)。

完成依据：Mac 的单会话菜单按状态提供停止、停止子 Agent、继续与“发送消息…”，多选提供停止；这些操作不切换列表选中与显示的会话；sheet 中的输入区与会话详情中的同一份草稿与选择，提交被受理后关闭。使用临时 `VELUNE_HOME` 与合成上游验证，不读取真实配置、凭据或会话。

## 实施前事实

- 核心已有 `Cancel`、`StopSubAgents`、`ResumeTurn`；继续的资格是快照里的 `FollowUp.can_resume`，Mac 另要求下一轮运行时等于上下文运行时（`ConversationStore.canResume`）。
- 列表侧只知道运行状态：`DeviceStore` 由 `ReadConversationActivity` 得到运行中、异地运行、子 Agent 的父会话与待处理请求的会话；不知道哪些空闲会话可继续。
- Mac 每个标签页有自己的 `ConversationStore` 与 `ComposerModel`；草稿按会话存在核心（`SetComposerDraft`），下一轮选择按会话与运行时存在核心。
- `FollowUp.can_continue` 注释为 `can_resume` 的旧名，属于待删除的重复字段。

## 实现

1. 核心：会话活动增加“可继续”的会话（本进程中会话的 live 状态 `can_resume` 为真），Mac 列表据此决定菜单是否出现“继续”；执行时 `ResumeTurn` 仍重新判定。没有 live 状态的会话（例如应用重启后）在打开前不出现“继续”。
2. Mac 菜单（`ConversationList.conversationManagementMenu`）：运行中“停止”（`DeviceStore.stop`，以活动中的上下文运行时发 `Cancel`），只剩子 Agent 时“停止子 Agent”，可继续时“继续”，非子 Agent 会话均有“发送消息…”；多选时“停止所选会话”作用于其中运行中的会话。
3. “继续”：有标签页显示该会话时由它的 store 执行；否则 `ConversationWindowModel.resumeFromList` 建一个不显示的标签页打开会话，等下一轮选择（含模型目录）就绪后执行与输入区相同的继续，受理、拒绝或 30 秒超时后关闭。
4. sheet（`ComposerSheet`）：为目标会话建一个不显示的 `ConversationTab`，sheet 只承载其 `ComposerView`；store 的 `acceptedSubmissions` 在发送、排队、插入本轮或继续被核心受理时递增，sheet 据此关闭；关闭后显示该会话的标签页重新读取草稿。附件属于各自的输入区，不在 sheet 与会话详情之间共享。
5. 删除了重复字段 `FollowUp.can_continue`。

开发检查入口 `--check-list-actions`：对第一个未显示的列表会话依次在 sheet 中发送、从列表停止、从列表继续，进度写到 stderr。

## 验证（2026-10-29）

- 临时脚本以 Python 绑定、临时 `VELUNE_HOME`、受管 Codex HOME、实际 Codex 0.159.3 与回环合成 Responses 上游驱动 application：运行中会话出现在活动中且带上下文运行时；从活动给出的运行时停止后，会话出现在 `resumable`；`ResumeTurn` 未新增用户消息并使其离开 `resumable`。
- 同一合成资料下，以 `open -n -g` 在后台启动隔离的构建副本并带 `--check-list-actions`：sheet 打开、发送被受理后关闭、从列表停止、从列表继续、结束，五步通过；上游依次收到 sheet 中的消息与无新消息的恢复请求。第二次运行时窗口显示另一会话，继续经不显示的标签页完成。窗口截图确认 sheet 的标题、“打开会话”、完整输入区与“取消”。
- 右键菜单本身的弹出未以指针实际点开（需要前台交互），菜单项的条件与动作由上述入口覆盖。
- `cargo fmt --all --check`、application 相关 crate 的严格 clippy、Mac Swift 构建（警告视为错误）通过。
