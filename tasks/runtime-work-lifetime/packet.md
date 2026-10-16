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

## 探测（临时 HOME＋回环合成上游，探测脚本留在忽略的 `target/probes/`，2026-10-16）

- Codex 0.159.3 app-server：同一连接内子线程有自己的 `thread/status/changed`（active→idle）与 `turn/started`／`turn/completed`，可据此判断子 Agent 运行，不必依赖写入锁。`turn/interrupt` 父轮次不停止子线程，子线程照常完成；停止子 Agent 需对其当前轮次单独中断。本版未观察到 `subAgentActivity` 条目。协议定义由 `codex app-server generate-json-schema` 生成核对：`ThreadStatus` 为 notLoaded／idle／systemError／active（activeFlags：waitingOnApproval、waitingOnUserInput）。
- Claude Code 2.1.292（本机已自动更新，Velune 固定的 2.1.286 不在本机；结论需在 2.1.286 复核）：后台 Agent 运行时，父轮次 `result` 后会话状态保持 running，任务完成后 Claude 自动再跑一轮回应任务通知，之后才 `session_state_changed: idle`。因此 Velune 的 Claude 轮次在后台工作结束前一直忙，进程不会被提前结束。
- 待查：ACP、DSH、Pi 是否有可观察的轮次外工作（目前无已知例子；Pi 的子 Agent 由扩展提供，尚未支持）。另：本机 Claude Code 已为 2.1.292，需确认 Velune 版本检查是否拒绝它。

## 设计（2026-10-16，经 advisor 复核后采纳）

根因比“轮次结束即释放”更宽：进程寿命在各处都以 `busy()`（轮次）为准。会终止子 Agent 的路径有：`NativeSession::settle` 对经网关轮次 `rpc.finish`；application `drain_runtime` → `finish_managed_turn` → `shutdown_active`；轮次结束时 `finish_turn` 用原生配置 `thread/resume` 恢复 Codex 提供商（之后同一进程不能再承接经网关的轮次）；`send_turn` 在历史状态或选择变化时 `prepare_with_selection` → `shutdown_active`；`invalidate_execution`（重新读取、编辑、接续、配置变更）；`live.rs` 跟随来源文件时，非忙会话的 rollout 变化被当作外部修改并关闭进程（子 Agent 写入即会触发）。因此判断须落在“此会话的进程是否还有未结束的工作”，与是否经网关无关。

- **检测归 adapter，寿命策略归 application。** agent-runtime 库级 `work` 模块定义未结束工作（后台任务：会话 ID 与状态 运行／等待审批／等待输入／停止中）。Codex 依 app-server 的 `thread/status/changed` 与子线程 `turn/started`／`turn/completed` 跟踪本连接内除父线程外的线程（一会话一个 app-server，不依赖 `receiverThreadIds` 的到达顺序；`sub_agents` 只用于归属），记录子线程当前轮次以便中断；Claude 取未结束任务（其轮次本就保持忙）；ACP、DSH、Pi 暂无。写入锁扫描只保留给“在别处运行”（它不能用于 SSH、间隔 5 秒且没有轮次 ID）。
- **轮次与工作分开，hard cut。** 删除 `Session::busy()`／`NativeSession::busy()`，换成 `turn_running()` 与 `work()`，由编译器逼出所有调用点（执行中判定、pinned、退出、回收、跟随来源、队列、发送守卫）。
- **临时覆盖的显式状态。** `gateway_runner`／`gateway_tunnel` 换成 `TurnOverride { runner, tunnel, phase: Prepared | InUse }`，只从 `InUse` 且轮次结束、工作为空时释放；避免准备与发送之间的 drain 误释放。Codex 提供商恢复从轮次结束移到释放时（`NativeSession::release_override`：先恢复父线程及子线程的原生提供商，再结束进程）。
- **是否需要重启** 由 application 计算 `TurnExecution { Reuse, Restart(cause) }`（历史状态、选择、会话、运行时变化），adapter 提供纯函数判断自身原因（网关↔原生、Claude 启动参数），`prepare_turn` 共用同一规则。工作未结束且需要重启时：先显式报错（切片 2），切片 3 改为自动等待的队列状态（与“等待用户恢复”的 `QueuePause` 分开）及“停止子 Agent 并发送”。
- **停止**：中断每个运行中的子线程轮次，等其到 idle／systemError（带超时）后才可重启；父会话停止同时停止子 Agent，只剩子 Agent 时提供停止子 Agent。退出应用在有未结束工作时需要确认。

### 切片

1. 检测（agent-runtime）：`work` 类型、Codex 线程状态与子线程轮次跟踪、`stop_background`；活动改用它。验证：Codex 自身提供商下脚本显示子 Agent 运行与等待审批。
2. 寿命 hard cut：`settle`／`finish_turn` 去掉轮次结束释放，`release_override`，`TurnOverride`，`busy()` 拆分，跟随来源把未结束工作视为自身写入；需要重启时显式报错。验证：经网关时子 Agent 完成，之后进程、runner、隧道结束，Harness 配置未改，父会话原生恢复可用；子 Agent 运行中同模型的下一轮复用进程。
3. 队列与停止：自动等待、停止子 Agent 并发送、停止范围、退出确认、Mac 界面。验证：扩展队列脚本。
4. 复核：Claude Code 2.1.286 的后台任务与中断（本机仅 2.1.292）、SSH 隧道长连接、子线程以 velune 提供商持久化后的原生恢复。
