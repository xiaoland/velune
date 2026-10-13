# Agent 间协作（A2A）与原生 sub-agent 展示

2026-10-12 开始。用户检查 Mac 接入任务切片 4（父子委派界面）的开发版后指出：原生 sub-agent 与跨 Harness 协作（A2A）是不同关系，2026-10-11 由另一会话编撰的 PRD 把后者写成了“把另一 runtime 当作 sub-agent”。产品意图已改写到 [PRD](../../docs/prd/index.md#ai-网关与-agent-运行时配置)，来源 [S18](../../docs/sources.md#s18)。沿用 AGENTS.md 的验证与安装规则；本地提交已授权，不 push。

## 已确认（用户，2026-10-12）

1. A2A 参与方是普通会话：出现在会话列表，可单独打开、继续和处理审批；Agent 能新建会话，也能联系已有会话。
2. 联系授权若需要则归运行时设置；用户倾向不做任何限制。本任务不设限制、不加设置项。
3. 原生 sub-agent 属于会话体验；当前只需在消息列表中看到它，并可单独打开其会话。
4. composer 之后需要简化（已有模型、会话选项、工作目录等多个弹出按钮）；本任务不扩大该问题，简化另行设计。

## 被取代的实现

- 2026-10-11 [Harness 协作任务](../harness-interoperation/packet.md)：父会话显式创建协作 scope（目标 profile、并发与期限上限），子会话由独立子执行器运行，不进入会话列表；关闭 scope 结束全部子会话。
- 切片 4 未提交的 Mac 父子界面（composer“子 Agent”菜单、子任务面板、子任务审批）和面向会话寻址的子任务 API 已存入 `git stash`（“slice 4 parent/child collaboration UI”），不再提交。
- 按 hard-cutoff 删除：`create_collaborative_conversation`、子任务状态／快照／审批／关闭 API 及其 binding 类型、Coordinator／ChildRuntime。可复用的是 MCP 入口（rmcp loopback、票据、Origin／Host 校验）、各 adapter 的 MCP 注入与 SSH 转发／relay。

## 方案（核心已实现）

**Broker**：application 持有一个 loopback MCP 服务，向 Velune 执行的每个会话注入，每个会话一张随机票据；票据决定调用方会话，工具自报身份不作为依据。MCP 线程不持有 application；请求进入 broker 收件箱，由 application 在处理客户端请求、推进各会话时执行，等待在推进后按修订号或期限回复。因此会话推进仍依赖客户端轮询（Mac 已按活动轮询）。

**工具**（参数只有会话、运行时、文本与等待条件；不能传程序、环境、认证）：

- `agents`：已启用的运行时（ID、名称、类型、运行位置）
- `conversations`：已有会话（可按工作目录或标题过滤），含状态
- `start`：在某运行时新建会话并发送首条消息；工作目录缺省沿用调用方（同一执行机器时），模型缺省为运行时默认
- `send`：向空闲会话发送消息；忙碌、在 Velune 外运行或待审批时拒绝，不排队
- `status`／`wait`：状态、修订号与最近一轮的回复文本；单次等待不超过 30 秒，超时只结束等待
- `cancel`：取消该会话当前轮次

协作消息以带出处的包装进入接收会话（来源会话标题、运行时与 ID，声明不是新的人类授权）。不传播取消、不建父子关系；接收会话的审批照常出现在该会话。

**Mac**：不加 composer 控件或子任务面板。A2A 工具调用与原生 sub-agent 调用在消息列表中提供“打开会话”；Agent 新建的会话经列表刷新出现；收到的协作消息显示其来源。

**原生 sub-agent**：先按固定版本核对 Claude Code（Task／Agent 工具与 sidechain／`subagents/` 记录）、Codex（`collabAgentToolCall` 与子线程来源）、Pi、DSH、ACP 的可见事件与历史，只用合成 fixture 与官方资料，不读真实会话；再决定消息列表中的关联与打开方式。

## 切片

1. 核心 A2A broker 替换父子 scope；bindings 去掉子任务 API；脚本改为两个普通会话互相联系。（已实现）
2. Mac：消息列表中 A2A 调用的“打开会话”、收到消息的来源、会话列表刷新。（已实现）
3. 原生 sub-agent：证据、消息列表显示与单独打开。（已实现 Codex 与 Claude Code）

## 待用户决定

- 收到的协作消息在消息列表中的位置：当前计划仍在用户侧并标注来源会话（Harness 原生历史把它记为用户消息）；若希望居中作为系统／Harness 状态，请指出。

## 切片 1：核心 broker（2026-10-12）

- 实现：`local/collaboration` 改为 Broker（收件箱、票据、等待者、调用方已读状态）与 MCP 入口；`Session` 增加稳定 key 与票据，`route` 在推进全部会话后执行收件箱并恢复原寻址会话。`list()` 列出所有打开会话的原生草稿，使 Agent 新建、尚未写入历史的会话也出现在列表。设计记录见 [架构](../../docs/design/architecture.md#agent-间协作2026-10-12取代-10-11-子-agent-切片)，操作契约见 [开发说明](../../docs/development.md#agent-间协作)。
- 注入改为每个会话都有，因此失败不能阻断普通会话：MCP 转发失败、ACP stdio relay 不可用、Claude 不接受动态服务时省略服务并记录警告；Codex 已有同名服务时只省略该服务（不移动其余服务的票据环境变量序号）。此前合成 Claude peer 不实现 `mcp_status`，`manual-acp-adapter.py`／`manual-runtime-selection.py` 正是因此失败后修正。
- 删除：`create_collaborative_conversation`、子任务 API 与类型、Coordinator／ChildRuntime、依赖旧 API 且需本机没有的 OpenCode／DSH 的 `manual-harness-interoperation.py`（旧证据仍在 harness-interoperation 任务）。`manual-runtime-management-api.py` 去掉 scope 门禁检查；`manual-resource-library.py` 修正过时的 `BindingComposedTurn` 调用。
- 取舍：协作调用在客户端请求时执行，进度依赖轮询（Mac 每 0.6 秒）；旧子执行器“无前台轮询也能完成”的性质不再保留。
- 验证：fmt／check／clippy／no-default-features、Mac `--build-only` 通过；`manual-collaboration.py`（新建并联系已有会话、审批留在对方会话、忙碌与未知票据被拒）、`manual-acp-adapter.py`、`manual-runtime-selection.py`、`manual-runtime-management-api.py`、`manual-resource-library.py` 通过。真实 Harness 经新路径的 A2A 尚未验证（Codex／Claude 每轮会话都会注入服务，需要用户实际使用或后续合成上游脚本确认）。

## 切片 2：消息列表中的 A2A（2026-10-12）

- 来源行改为追加在正文之后（`velune_conversation::agent_message`），原因：原生首条消息标题会被截断和折叠空白，放在正文前会让被联系会话的标题只剩出处说明。交付把来源行还原为 `AgentSender` 块，标题只取正文。
- 工具块的 `linked_conversation_id` 由交付层从协作工具返回的状态对象（含 `conversation_id`、`runtime_id`、`state`）解析，不按运行时或工具名分支。Codex 的 MCP 调用此前不显示结果（`aggregatedOutput` 只属于命令），现取 `structuredContent` 或文本内容、错误信息。
- Mac：工具行下方“打开对方会话”，用户侧气泡顶部“来自 运行时 会话「标题」”，均为 `velune-conversation:` 链接，在当前窗口打开（不在列表时先重读）；活动轮询发现未列出的运行中会话即重读列表。composer 不再有协作控件。
- 验证：`manual-collaboration.py` 增加来源块、标题与工具链接断言；合成 Alpha 改为可列出与重载，`--seed` 运行一次新建流程后停在 Alpha 草稿“ASK hello”。开发版隔离 HOME 中：Alpha 的工作过程显示 `velune: start_conversation` 与“打开对方会话”，AX 按下后打开 Beta；Beta 气泡顶部显示“来自 Alpha 会话「START Beta PERMIT」”与正文“PERMIT”，列表标题为“PERMIT”，按下来源链接回到 Alpha。检查后已退出开发版。
- 安装：5e96def 已构建安装至 `/Applications/Velune.app`（签名校验通过，manifest 对应该提交且无未提交改动），重新打开后无崩溃报告。

## 切片 3：原生 sub-agent（2026-10-12）

- 证据（合成 loopback 上游、隔离 HOME，不读真实会话）：Claude Code 2.1.286 的 `Agent` 工具在 SDK 非交互下后台启动；工具结果带 `tool_use_result.agentId`（历史为 `toolUseResult.agentId`），子 Agent 消息在实时流中带 `parent_tool_use_id`，历史在 `<session>/subagents/agent-<id>.jsonl`（`isSidechain`）与 `.meta.json`（description、toolUseId）；完成时实时流有 `system task_notification`，历史记一条 `origin.kind = task-notification` 的用户记录。Codex 0.159.3 的 `multi_agent_v1` 工具（spawn_agent、wait_agent 等）在 app-server 中表现为 `collabAgentToolCall`（`receiverThreadIds`、`agentsStates`），子线程是独立 rollout，`thread/list` 的 `source.subAgent.thread_spawn.parent_thread_id` 指向父线程（`parentThreadId` 只在 `thread/read` 中出现）。Pi、DSH、通用 ACP 没有可核对的标准子 Agent 记录，本切片不涉及。
- 实现：见 [架构](../../docs/design/architecture.md#agent-间协作2026-10-12取代-10-11-子-agent-切片) 末段。子会话进入核心列表（复用打开时的列表来源校验），Mac 不在侧栏列出；从父会话工具块的“打开会话”进入，底部以“打开父会话”代替输入区。核心拒绝向子会话发送，A2A 的 `list_conversations` 不提供它们。Claude 历史同时不再显示 `isMeta` 用户记录（Harness 注入的提醒，Claude Code 自身界面也不显示），原先会以用户气泡出现；任务通知不再显示为原始 XML 用户消息，改为“任务通知”工具块。Mac 链接文字由“打开对方会话”统一为“打开会话”。
- 取舍与已知缺口：Claude 父会话中同一子 Agent 有两处链接（工具结果与任务通知），均为真实事件，保留。Claude 的 `Agent` 调用行与结果行仍是两行，调用行保持运行图标（既有行为，未合并工具调用与结果）。Claude 列表现在也解析子 Agent 文件，会话多时列表成本随之增加。Codex 子线程上的审批请求仍按“非活动会话”拒绝（`codex_handle_record` 只接受当前线程），子 Agent 需要审批的命令会被拒绝；是否把子 Agent 审批交到父会话尚未设计。子会话不可在 Velune 中继续，与“由启动它的会话继续”的产品归属一致。
- 验证：fmt／check／clippy／no-default-features、Mac `--build-only` 通过。新增 [manual-native-subagents.py](../../scripts/manual-native-subagents.py)：实际 Codex／Claude Code 经网关与合成上游，实时与历史读取下的链接、父会话无子 Agent 消息、子会话父标记、内容与拒绝发送全部通过（Claude Code 在 Velune 中以 auto 模式运行，其安全分类请求由合成上游答复放行）。回归：`manual-collaboration`、`manual-acp-adapter`、`manual-runtime-selection`、`manual-codex-history-identity`（revert／items）通过；`manual-live-native` 6 次中 1 次在“外部启动的会话已列出”断言失败、其余通过，未能确认失败的运行时及是否与本切片有关。开发版隔离 HOME（`--seed`）中：侧栏只列出两个父会话；Claude 父会话展开工作过程后，工具结果与任务通知下各有“打开会话”，AX 按下后打开标题为子 Agent description 的会话，底部为说明与“打开父会话”，侧栏无选中项。随后隐藏 `isMeta` 的改动只经 bindings 读取确认（开发版窗口位于其他桌面空间，未切换前台）；Codex 父会话的链接未在 App 中逐一点按，只经脚本确认。
- 安装：7419ad4 已构建安装至 `/Applications/Velune.app`（签名校验通过，manifest 对应该提交且无未提交改动），重新打开后无崩溃报告。
