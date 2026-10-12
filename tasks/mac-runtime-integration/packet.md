# Mac 接入运行时与 AI 服务更新

2026-10-12 开始。用户指出 Mac App 还没有接入 PRD 中标记为“前端待实现”的 AI 服务与 Agent 运行时更新，因此无法实际发送（见 [PRD](../../docs/prd/index.md#ai-网关与-agent-运行时配置)）。核心契约均已存在（UniFFI：`runtime_models`、`create_conversation_with_selection`／`send_turn_with_selection`、ACP launch／info／session config、`upsert_runtime_with_execution` 的 SSH、协作父会话与子任务）；本任务是平台接入，必要时补核心契约。沿用 AGENTS.md 的验证与安装规则，真实提供商与产品体验由用户验收。

## 切片与顺序

1. composer 模型选择：运行时原生模型、Velune 管理模型、通用 ACP 的 Agent 默认；按所选模型给出推理强度；创建与发送经 selection 接口。（已实现，待用户验收）
2. ACP 运行时：启动命令／参数／环境的设置，Agent 信息与能力展示，会话配置选项。（已实现，待用户验收）
3. SSH：运行时设置中的连接配置，远端工作目录的输入与展示。
4. 跨 Harness 协作：协作会话的创建、子任务状态／投影、审批回复与关闭。

顺序依据：1 直接阻止发送；2–4 依次扩展可用运行时与协作。用户可调整。

## 切片 1：composer 模型选择

- 现状：Mac 只列 Velune 网关模型（按协议过滤），创建／发送只传 managed key；推理强度只查 managed。没有提供商时无法发送，订阅类原生模型不可选。
- 核心事实（2026-10-12 源码）：专用 adapter 接受 Native／Managed，不接受 RuntimeDefault；通用 ACP 只接受 RuntimeDefault 或其会话内的原生选项，模型目录要求已有活动会话。原生强度校验：Codex 任意非空、Claude low…max、Pi 级别名。
- 原生强度证据（临时 HOME 实测）：Codex 0.159.3 `model/list` 每个模型带 `supportedReasoningEfforts[].reasoningEffort`；Claude Code 2.1.286 initialize 的 `models[]` 带 `supportsEffort` 与 `supportedEffortLevels`；Pi 模型的 `reasoning` 标记配合固定级别。
- 方案：`RuntimeModel` 增加该运行时下可选的推理强度（原生取 Harness 目录，managed 沿用核心映射）；Mac 以类型化选择表示下一轮模型，按运行时异步读取并缓存目录，失败时仍可用本地网关模型。默认：此会话记住的选择 → 该运行时最近的选择 → ACP 的 Agent 默认 → 第一个原生模型 → 第一个 managed 模型。

### 实施（2026-10-12）

- 核心：`ModelOption`（agent-runtime）携带原生强度；`RuntimeModel.reasoning_efforts` 贯通 bindings。通用 ACP 的 `runtime_models` 总是先给 `RuntimeDefault`（“Agent 默认模型”），活动会话时再列原生选项，因此 Mac 不需要按 ACP 分支或另设能力标志；managed 名称带提供商名。
- Mac：`ModelSelection`／`ModelChoice`／`RuntimeModelCatalog`；Store 按运行时实例缓存目录（仅在打开会话时读取，网关或运行时配置变化即丢弃），默认取目录顺序；读取中／失败时列出本地可达的 managed 模型，未读到的选择由核心发送时校验。Composer 菜单按选择类别分组，附“重新读取模型”。
- 已知边界：原生选择只在本进程按会话记住；会话快照仅含 managed key，重开应用后回到默认。通用 ACP 打开会话后才有原生目录，需“重新读取模型”刷新缓存。
- 验证：fmt／check／clippy／no-default-features、`build-macos.sh --build-only` 通过。`manual-reasoning-effort.py` 以实际 Codex 0.159.3（8 个原生模型均带强度）、Claude Code 2.1.286（5 个，4 个带强度）、Pi 1.0.2（临时 agentDir 合成提供商：reasoning 模型带七级、普通模型为空）核对目录与 managed 映射，并以 Pi 原生模型经回环上游带 `high` 派发。`manual-acp-adapter.py`、`manual-runtime-selection.py` 通过（顺带修正二者对 `BindingComposedTurn` 与 `cancel` 的过时调用）。开发版隔离 HOME 新建会话默认选中 Pi 原生模型并显示推理菜单。
- 安装：afefe95 已构建安装至 `/Applications/Velune.app`（签名校验通过，manifest 对应该提交且无未提交改动），重新打开后无崩溃报告。

## 切片 2：ACP 运行时配置与会话选项（2026-10-12）

- 核心：运行时类型描述增加 `launch_options`／`agent_info`／`session_options`（当前仅通用 ACP 为真），Mac 据此显示界面，不比较 `acp-v1`。入口字段说明改为指向下方的启动参数与环境变量。
- Mac 编辑器：启动参数逐行（不经 shell）、环境变量名称／值行（空行忽略，名称不能为空、含“=”或重复），读取与运行环境同一次请求，保存经 `upsert_acp_runtime`。“读取 Agent 信息”按需启动一次已保存的 Agent，显示名称、版本、五项会话能力和登录方式，不在打开编辑器时自动启动。
- Mac composer：会话在本应用中活动后（首次发送后）读取 `acp_session_config`，“会话选项”菜单按 Agent 名称与分组列出选择项和开关，生成中禁用；首个 model 类选项由模型菜单提供而不重复。会话变为活动时重新读取该运行时的模型目录，修正切片 1 需手动“重新读取模型”的边界。未活动的读取失败不作为问题上报。
- 未做：按原生 ID 打开无 list 能力的 ACP 会话（`open_acp_conversation`）暂无界面。
- 验证：fmt／clippy／no-default-features、Mac 构建通过；`manual-acp-adapter.py` 通过。开发版隔离 HOME 用该脚本的合成 ACP Agent：打开会话后经 AX 发送，“会话选项”出现，模型菜单变为 Agent 默认／Alpha／Beta 并可选 Beta；切换布尔选项后 Agent 记录到 `config flag-without-category=false`；编辑器载入参数与环境变量、读取到 Agent 名称／版本／能力／登录方式，原样保存后配置中的启动参数不变。检查中 AX 按标题包含匹配误点了 Apple 菜单的“系统设置…”，已立即退出该次启动的系统设置，辅助工具改为精确匹配。
