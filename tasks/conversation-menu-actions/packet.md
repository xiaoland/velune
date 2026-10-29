# 会话菜单的停止、继续与 sheet 发送

2026-10-29 用户解除功能冻结后要求（[S56](../../docs/sources.md#s56)）：会话的右键菜单能停止与继续会话，并能在 sheet 中打开输入区发送一次消息；首个 beta 前只做 Mac。产品规则见 [会话管理](../../docs/prd/conversations.md#会话管理) 与 [在 sheet 中发送](../../docs/prd/composer.md#在-sheet-中发送)。

完成依据：Mac 的单会话菜单按状态提供停止、停止子 Agent、继续与“发送消息…”，多选提供停止；这些操作不切换列表选中与显示的会话；sheet 中的输入区与会话详情中的同一份草稿与选择，提交被受理后关闭。使用临时 `VELUNE_HOME` 与合成上游验证，不读取真实配置、凭据或会话。

## 实施前事实

- 核心已有 `Cancel`、`StopSubAgents`、`ResumeTurn`；继续的资格是快照里的 `FollowUp.can_resume`，Mac 另要求下一轮运行时等于上下文运行时（`ConversationStore.canResume`）。
- 列表侧只知道运行状态：`DeviceStore` 由 `ReadConversationActivity` 得到运行中、异地运行、子 Agent 的父会话与待处理请求的会话；不知道哪些空闲会话可继续。
- Mac 每个标签页有自己的 `ConversationStore` 与 `ComposerModel`；草稿按会话存在核心（`SetComposerDraft`），下一轮选择按会话与运行时存在核心。
- `FollowUp.can_continue` 注释为 `can_resume` 的旧名，属于待删除的重复字段。

## 方案

1. 核心：会话活动增加“可继续”的会话（本进程中会话的 live 状态 `can_resume` 为真），Mac 列表据此决定菜单是否出现“继续”；执行时 `ResumeTurn` 仍重新判定。没有 live 状态的会话（例如应用重启后）在打开前不出现“继续”。
2. Mac 菜单：运行中“停止”（`Cancel`），只剩子 Agent 时“停止子 Agent”（`StopSubAgents`），可继续时“继续”，未发送的新会话以外均有“发送消息…”；多选时“停止”作用于其中运行中的会话。
3. sheet：为目标会话建一个不显示的 `ConversationTab`（自己的 store 与 composer），sheet 只承载其 `ComposerView`；提交受理后关闭，草稿通过核心与会话详情共享。
4. 顺带删除 `FollowUp.can_continue`。

## 验证

待实施后记录。
