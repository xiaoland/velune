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

## 方案（候选，实施中）

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

1. 核心 A2A broker 替换父子 scope；bindings 去掉子任务 API；脚本改为两个普通会话互相联系。
2. Mac：消息列表中 A2A 调用的“打开会话”、收到消息的来源、会话列表刷新。
3. 原生 sub-agent：证据、消息列表显示与单独打开。

## 待用户决定

- 收到的协作消息在消息列表中的位置：当前计划仍在用户侧并标注来源会话（Harness 原生历史把它记为用户消息）；若希望居中作为系统／Harness 状态，请指出。
