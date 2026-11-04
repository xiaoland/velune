# 来源：运行时、资源库与 Agent 协作

[返回来源索引](../sources.md)。本页保留原始决定与当时的授权边界；它不是当前实现状态或本次任务的授权清单。现行规则从[文档入口](../index.md)按主题查阅。

## S18

2026-10-12 用户检查开发中的协作界面后指出：子 Agent（sub-agent）与跨 Harness 协作（A2A）不同，二者本质是否一样不是 Velune 这个 shell 要考虑的；2026-10-11 的 PRD 表述由另一会话编撰，可能有误。用户确认：A2A 参与方是普通会话（出现在会话列表、可单独打开，Agent 可联系已有会话）；联系授权若需要则归运行时设置，但倾向不做任何限制；原生 sub-agent 属于会话体验，暂时只需在消息列表中看到并可单独打开其会话。用户另提示 composer 之后需要简化（已有三个弹出按钮）。产品归 [PRD](../prd/collaboration.md)，实施归 Agent 间协作任务（git 历史：tasks/agent-collaboration）。

## S19

2026-10-13 用户在原生 sub-agent 切片之后要求继续解决已报告的缺口，并确认收到的协作消息放在用户侧，标出来源会话即可。同条消息指出 composer 还缺少权限模式选择器。产品归 [PRD](../prd/collaboration.md)，实施归 Agent 间协作任务（git 历史：tasks/agent-collaboration）。

## S23

2026-10-14 用户验收 status line 后，对资源库的子 Agent 提出：描述也应是多行文本编辑器；应可配置推理强度；Codex 的子 Agent 支持模型与推理强度，却没有写入，导入时也被丢弃；可以选择技能（用户不确定 Codex／Claude Code 是否支持，认为 Pi 官方的 subagents 插件支持）。用户计划之后让 Pi adapter 支持 subagent extension，并预见版本兼容会使情况复杂不少。产品归 [PRD](../prd/index.md) 资源库段落，实施归 子 Agent 字段任务（git 历史：tasks/resource-subagent-fields）。

同日助手实测托管轮次中子 Agent 的原生模型 ID 被网关拒绝，提出三种模型指定方式后，用户要求第四种：既能使用 Velune 托管模型也能使用原生模型，并允许随时切换使用 Velune 与原生 Agent 运行时；同意“原生 ID 写入定义、托管轮次按 ID 路由到 Velune 模型”的方案后，进一步要求解决 Velune 独有模型在原生轮次失败的问题——不是让原生轮次能用 Velune 模型，而是保证原生轮次时运行时不会用到该子 Agent。用户确认此前所说“Pi 官方 subagents 插件支持技能”是记错了。

## S31

2026-10-16 诊断一个 Codex 会话未读无法消除时（原因是标已读进行中保护未释放；该会话当时在 Codex 桌面端带子 Agent 运行），用户提出：“我们的 sub-agent / a2a 系列功能不完整，我建议优先补齐 UI 上的短板（未来还需要允许 agent runtime 互调用）”。用户从开发方列出的短板中选定第一批：子 Agent 概览、侧栏状态汇总、子会话导航，并一并解决父轮次结束后到达的子 Agent 审批。随后提醒：“虽然我认为一切都是 A2A，但是现在很多 agent runtime 的实践还有一套并行的 sub-agent 机制”。产品归 [PRD](../prd/collaboration.md)，实施归 Agent 间协作任务（git 历史：tasks/agent-collaboration）。

## S32

2026-10-16 子 Agent 界面补齐的验收发现，经网关的 Codex 轮次结束时 Velune 结束运行时进程，后台子 Agent 随之终止。用户：“是的，需要修复这一点，估计这个问题是普遍存在的，不只是 codex 这个 agent runtime 的，而且应该会对 Velune 的 Agent Runtime lib 的架构有较大改变。”随后从开发方的选项中确认：临时覆盖与进程持续到该轮工作（含子 Agent）全部结束；子 Agent 运行时需要重启的下一轮先等待或经确认停止子 Agent；父会话的停止同时停止其子 Agent。产品归 [PRD](../prd/agent-runtimes.md)，实施归 运行时工作生命周期任务（git 历史：tasks/runtime-work-lifetime）。

## S34

2026-10-17 本机 Claude Code 自动更新到 2.1.292 后被 Velune 的版本检查拒绝。用户问：“理论上，我们的 adapter 应该是兼容一段版本才正常的；所以真的是 protocol 变了还是仅仅只是 adapter 自己伪宣称自己只能支持 2.1.286”。开发方核对：精确锁定来自架构规则与 `^2\.1\.286$`，没有协议变化的证据；临时放宽后 2.1.292 通过 9 个实际 CLI 脚本。就开发方的提问，用户选择“按兼容区间”：adapter 声明已验证的兼容区间（如 Claude Code ≥2.1.286、<2.2；Codex、Pi、DSH 同理），区间内照常使用，出现破坏性变化才另建 adapter；类型 ID 不含补丁号，按 hard-cutoff 重新添加运行时实例。实施见 版本区间任务（git 历史：tasks/adapter-version-ranges）。

## S36

2026-10-18 用户要求：“我现在还希望 sub-agent 有更多的打开方式（新 Tab / 当前 tab / Sheet / 新窗口），并且同样可以配置；此外，我们之前说 sub-agent 本质上也就是 A2A，这一点怎么体现，我没想好，现在我想到一点：让‘子 Agent 面板’升级为‘与其它 Agent 面板’（不一定叫这个名字）”。开发方定名“关联 Agent”（沿用 PRD 已有的中立“关联会话”组织），列出父会话、子 Agent 与 A2A 参与方；打开方式为一项应用设置，同样用于消息列表中的会话链接。产品归 [PRD](../prd/collaboration.md)，实施归 Agent 间协作任务（git 历史：tasks/agent-collaboration）。

## S44

2026-10-21 Windows 上 ChatGPT 桌面应用带的 Codex 为 0.163.0-alpha.10.2，超出 `codex-0.159` 的区间。用户问：“codex需要新的适配器？为什么？0.155不必支持了呀？还是说 0.163 又太新了”，随后决定：“先进行实测，如果真的不支持，再增加新的 adapter type；并且可以支持 alpha, beta ，这是无所谓的。”即先以实际 CLI 验证，确有破坏性变化才另建 adapter；预发布版本按其正式版计入区间。实测与区间调整见 版本区间任务（git 历史：tasks/adapter-version-ranges）。
