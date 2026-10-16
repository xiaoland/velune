# 运行时工作的生命周期：后台子 Agent 不随回合结束被终止

2026-10-16 开始。来源：[Agent 间协作任务](../agent-collaboration/packet.md#实施2026-10-16) 的验收发现经网关的 Codex 轮次结束时子 Agent 被终止；用户确认需要修复，并判断问题普遍存在、会较大改变 Agent Runtime lib 的架构。沿用 AGENTS.md 的验证、安装与提交规则；不 push。

## 已确认（用户）

- 需要修复：父轮次结束后仍在运行的原生子 Agent 不应被 Velune 终止（2026-10-16）。
- 既有产品决定（[PRD](../../docs/prd/index.md#ai-网关与-agent-运行时配置)，2026-10-10）：只支持单提供商的运行时按 turn 临时使用 Velune 网关，成功、失败、取消都要结束临时覆盖，不改原 Harness 持久提供商配置。
- 2026-10-16 重新界定（[S32](../../docs/sources.md#s32)）：临时覆盖与运行时进程持续到该轮工作全部结束（轮次及其仍在运行的原生子 Agent／后台任务结束、失败或被停止）；工作未结束时，不需重启的下一轮照常发送，需要重启的下一轮先等待或经用户确认停止子 Agent；父会话的停止同时停止该轮及其子 Agent，只剩子 Agent 时父会话提供停止子 Agent。

## 已核实的事实（代码，2026-10-16，main 738a7a1）

- application：经网关的轮次在会话由忙转闲或失败时 `finish_managed_turn` → `shutdown_active`，结束 Harness 进程、网关 runner 与隧道，会话退为只读历史快照。适用于原生 adapter（Codex、Claude Code、DSH）、ACP 与 Pi。用运行时自身提供商的轮次不走这条路径。
- agent-runtime 原生 adapter：网关注入（端点／隧道、`VELUNE_GATEWAY_TOKEN` 环境变量、Codex `config`）只在启动进程时生效；原生与网关之间切换、或更换网关注入都要重启进程（`restart_required`）。`settle` 对经网关的轮次与所有 Claude Code 轮次结束进程；之后 `poll` 不再读取事件。Claude Code 的推理强度与权限模式只能在启动时设定，所以每轮都换子进程。
- 实测（`scripts/manual-subagent-activity.py`，Codex 0.159.3）：Codex 自身提供商下，父轮次不等待即结束后，子 Agent 继续运行、审批到达父会话并完成；经网关时父轮次结束后子 Agent 被终止，上游连接被断开。

## 待验证

- Claude Code 2.1.286：后台 `Agent` 运行时，`session_state_changed: idle` 是否等后台任务结束；若不等，每轮结束子进程会终止后台 Agent（不论是否经网关）。
- Codex：子线程运行状态能否从 app-server 事件得到（线程状态通知、`collabAgentToolCall` 的 `agentsStates`），以便不依赖写入锁扫描。
- ACP、DSH、Pi：是否有可观察的轮次外后台工作。

## 候选方向（产品语义已定，架构待定）

- 运行时工作 = 轮次本身 + 它启动、Harness 报告仍在运行的原生子 Agent／后台任务。进程、网关 runner、临时覆盖持续到工作全部结束、失败或用户停止，再按现行规则释放。
- Agent Runtime lib 为各 adapter 提供统一的“未结束工作”信号，与轮次的运行状态分开；application 以工作结束而非轮次结束决定释放。
- 工作未结束时的下一轮：沿用同一进程与覆盖可直接发送；需要重启进程的选择（换模型来源、Claude 换推理强度等）等待，或由用户明确停止子 Agent 后进行。
