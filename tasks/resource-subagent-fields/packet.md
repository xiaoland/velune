# 资源库子 Agent 字段

2026-10-14 开始。用户要求见 [S23](../../docs/sources.md#s23)，产品行为见 [PRD](../../docs/prd/index.md)（资源库段落后“2026-10-14 用户要求补全子 Agent 资源”）。沿用 AGENTS.md 的验证与安装规则。

## 观察（2026-10-14，本机安装的固定版本）

- Codex 0.159.3：agent role 文件（`agents/*.toml`）在 `name`、`description`、`nickname_candidates` 之外整体按 config.toml 解析（二进制中的 `RawAgentRoleFileToml` 报错文案与 `ConfigToml` 字段表），因此 `model`、`model_reasoning_effort` 有效；`developer_instructions` 必填。推理强度取值 none／minimal／low／medium／high／xhigh／max（二进制中的枚举字符串）。没有“为子 Agent 预载技能”的字段（`skills` 只是启停配置）。
- Claude Code 2.1.286：用户 agent frontmatter 读取 name、description、tools、disallowedTools、skills、initialPrompt、mcpServers、hooks、color、model、effort（low／medium／high／xhigh／max 或整数）、permissionMode、maxTurns、background、memory、isolation 等（二进制中的 agent 解析函数）。
- Pi 1.0.2：没有内置子 Agent；随包的 `examples/extensions/subagent` 从 `~/.pi/agent/agents/*.md` 读 name、description、tools、model，没有技能或单独的思考级别（思考级别仅在沿用父会话模型时继承）。用户提到的“官方 subagents 插件支持技能”与此示例不符，可能指其他版本或第三方包，待用户确认来源。

## 方案

- canonical 子 Agent 增加 `effort: Option<String>` 与 `skills: Vec<String>`（技能名，即资源库技能的原生目录名）。
- Codex：写入 `model`、`model_reasoning_effort`（取值不在其集合内则跳过并说明）；技能跳过并说明。导入时带入这两个字段，其他 config 键照旧列为未导入。
- Claude Code：写入 `effort`（集合外跳过并说明）与 `skills` 列表；导入时带入（整数 effort 列为未导入）。
- Mac：描述改为多行编辑器；推理强度为下拉（不指定＋各级别）；技能从资源库技能多选，并保留已有但不在资源库中的名称。
- 验证：扩展资源库脚本检查写入与导入；用实际 Codex／Claude Code 经回环上游检查子 Agent 请求实际带上模型与推理强度（及 Claude 的技能内容）。

## 实施（2026-10-14）

- canonical／模板／bindings 的子 Agent 增加 `effort` 与 `skills`；Codex 写入并读回 `model`、`model_reasoning_effort`，Claude Code 写入并读回 `effort`、`skills`；集合外的推理强度与 Codex 的技能跳过并说明。
- Mac：描述为多行编辑器；推理强度下拉（不指定＋none 至 max）；技能为资源库技能的勾选列表，保留不在资源库中的名称。

## 验证（2026-10-14）

- [资源库脚本](../../scripts/manual-resource-library.py)：新增子 Agent 字段写入、不支持的说明与 Codex 加入资源库保留模型／强度，全部 PASS。
- [子 Agent 定义脚本](../../scripts/manual-subagent-definitions.py)（实际 Codex 0.159.3、Claude Code 2.1.286，回环合成上游）：父轮次 low，子 Agent 请求为 high 且带其指令；Claude Code 子 Agent 请求含技能正文。
- `eda1f63` 安装至 `/Applications/Velune.app`（0.1 beta.1）：codesign 严格校验通过，manifest 为该提交且 dirty false，后台启动无崩溃报告。资源库窗口未截图核对（打开会使应用置前）。

## 待用户决定：子 Agent 的模型

用户要求（2026-10-14）：既能用 Velune 托管模型也能用原生模型，并且同一运行时可随时在 Velune 托管轮次与原生轮次之间切换。实测事实：

- 托管轮次中，子 Agent 的请求与父轮次一样都到 Velune 网关。网关只认该轮的别名（父轮次的提供商 model ID）和 `velune/model/<记录键>`；其他名称返回 “gateway model route is missing”，子 Agent 失败（Codex 与 Claude Code 均如此）。
- `velune/model/<记录键>` 在托管轮次可用，并按协议转换（Claude Code 的子 Agent 被路由到 Responses 提供商）。
- Codex role 文件中的 `model_provider` 与内联 `[model_providers.*]` 在托管轮次不生效，子 Agent 仍走网关；Claude Code 的端点是进程级环境变量，子 Agent 无法另选端点。
- 原生轮次不经过 Velune，子 Agent 的模型名由运行时自己的提供商解析。

候选方案（建议）：定义文件写入模型的提供商 model ID（所选 Velune 模型取其 `provider_model_id`，或用户填写的原生 ID）；托管轮次中，application 把该运行时已启用的子 Agent 的模型名加入该轮网关别名，指向所选的 Velune 模型（原生 ID 则指向唯一同 ID 的 Velune 模型）。这样同一定义在两种轮次都可用，前提是两侧都认识这个 ID；只有 Velune 有的模型在原生轮次仍会失败，原生提供商没有的 ID 亦然，界面需说明。
