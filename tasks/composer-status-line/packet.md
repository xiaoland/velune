# composer status line

2026-10-14 开始。用户要求见 [S22](../../docs/sources.md#s22)，产品行为见 [PRD](../../docs/prd/index.md)（“2026-10-14 用户要求在会话详情的 composer 中增加 status line”一段）。沿用 AGENTS.md 的验证与安装规则。

## 已确认（用户，2026-10-14）

- 位置：操作行右侧紧凑显示。依据：避免信息过载、界面干净，但不损害信息披露；三种候选都不完全满意，重点是位置与呈现方式。
- 默认只放上下文用量；本会话累计 token、工作目录与分支、Codex 账户额度、费用都可以有，默认不放，由用户在会话设置中选择。

## 观察（固定版本，合成会话）

- Codex 0.159.3：
  - 实时通知 `thread/tokenUsage/updated` 带 `modelContextWindow` 与最近一次请求的 `last`；`account/rateLimits/updated` 带 `primary`／`secondary`（`usedPercent`、`windowDurationMins`、`resetsAt` 秒）。
  - rollout 的 `event_msg` `token_count` 带 `info.model_context_window`、`last_token_usage` 与 `rate_limits`（snake_case 同义字段），紧随该请求的 `token_usage_record` 之后，本身不带 `turn_id`。
- Claude Code 2.1.286：
  - `result.modelUsage[模型]` 带 `contextWindow`、`costUSD`、`thinkingTokens`；`result.usage.output_tokens_details.thinking_tokens`。
  - 会话 JSONL：assistant 记录带 `effort`、`gitBranch`，`message.usage.output_tokens_details.thinking_tokens`；另有 `cost-state` 记录（`totalCostUSD`、`hasUnknownModelCost`、按模型的用量与费用），没有上下文窗口。
  - `thinking_tokens` 原样取自上游响应；上游不报告时写 0，所以 0 不可区分“没有思考”和“未报告”。Velune 网关的协议转换不产生该字段。
  - 更正 S21 的两处说法：Claude Code 会逐条记录推理强度（assistant 记录的 `effort`），也会在上游报告时单列思考 token。
- Pi 1.0.2：RPC `get_session_stats` 返回 `contextUsage`（tokens、contextWindow、percent）、累计 token 与 cost；assistant 消息 `usage.cost.total`。

## 方案

- 核心：`TurnUsage` 增加该轮结束时的上下文（最后一次请求的输入含缓存＋输出）、模型上下文窗口、费用估算，以及 Codex 报告的账户额度。status line 取最近一轮有记录的值。application 在能对应到 managed 模型时用其配置的上下文窗口（实施中修订：原计划只在窗口未知时补上，但验证发现 Harness 对不认识的模型报告自己的猜测——Codex 0.159.3 报 258400，Claude Code 2.1.286 报 200000，而合成模型配置为 32768；配置值描述实际上游，因此优先）。
- Claude 的推理 token 与推理强度改为取原生记录：思考 token 大于 0 时记录，0 视为未报告。
- 分支：新增只读 git 查询（仅本机运行时），在打开会话与每轮结束时读取。
- Mac：操作行右侧、发送按钮前是一个圆形容量指示加百分比，用户选择的其他项以紧凑文字跟在后面。悬停显示说明，点按打开 popover，列出全部可用项和“自定义…”入口。设置“会话”页增加状态栏项的选择，持久化在应用配置中。

## 实施（2026-10-14）

- conversation：`TurnUsage` 增加 `context_tokens`、`context_window`、`session_cost_micro_usd`、`rate_limits`（`RateLimit`），`add` 对计数全有才累加，状态取后者。
- agent-runtime：Codex rollout 读 `token_count`、实时处理 `account/rateLimits/updated`；Claude 实时取最后主回复的上下文、`modelUsage` 窗口与价格已知时的 `total_cost_usd`，历史 helper 取 `effort`、`thinking_tokens`（>0）、上下文与 `cost-state`；Pi 取上下文并累加 `usage.cost.total`。字段归属见 agent-runtime README。
- application：managed 模型窗口；只读 `working_branch`；`StatusLineItem` 配置（默认仅上下文，保存去重排序）。bindings 对应 `BindingTurnUsage` 新字段、`BindingRateLimit`、`BindingStatusLineItem`、`working_branch`、`set_status_line`。
- Mac：`StatusLine.swift`（紧凑行、悬停说明、popover、设置入口），设置“会话 → 状态栏”开关，每轮 popover 增加“结束时上下文”。

## 验证（2026-10-14）

- 静态：cargo fmt --check、cargo check --locked、clippy -D warnings、no-default-features check、`scripts/build-macos.sh --build-only` 通过。
- [轮次配置脚本](../../scripts/manual-turn-settings.py)（实际 Codex 0.159.3、Claude Code 2.1.286、Pi 1.0.2，回环合成上游）：7 项 PASS，含实时与重启后的上下文 token／窗口、分支、状态栏配置默认、去重排序与重启保留，以及 Claude 历史推理强度。
- 推理强度、权限模式脚本与 9 项回归脚本（acp、selection、codex-history、continuation、subagents、live、attach、collab、parallel）通过。
- 隔离开发版（临时 VELUNE_HOME，后台启动，未置前）截图核对 Claude 与 Codex 会话的状态栏及 popover，修正了极小比例显示 0%、路径截断与缺少单位。
- 未验证：费用与账户额度需要真实价目或真实账户——合成模型价格未知，Claude 不给费用；网关不转发 Codex 额度头，合成会话没有额度。两者的字段形状来自上面的观察，界面仅以代码路径实现。设置页开关未截图（打开设置会把应用置前）。

## 安装（2026-10-14）

`ede7a32` 经 `scripts/build-macos.sh` 安装至 `/Applications/Velune.app`（0.1 beta.1）：codesign --verify --strict --deep 通过，build-manifest `source_commit` 为该提交、`dirty` false；后台启动后进程运行，无新的崩溃报告。等待用户验收。
