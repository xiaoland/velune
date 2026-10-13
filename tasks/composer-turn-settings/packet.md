# composer 选择器、即时发送显示与轮次配置

2026-10-13 开始。用户要求见 [S20](../../docs/sources.md#s20)，产品行为见 [PRD](../../docs/prd/index.md)（“2026-10-13 用户确认 composer 的选择器按语义收为两个原生下拉”一段）。沿用 AGENTS.md 的验证与安装规则：本地提交已授权，不 push；不新增自动化测试，手动脚本放 `scripts/`。

## 已确认（用户，2026-10-13）

1. composer 的选择器收为两个原生下拉。第一个显示“运行时 · 模型 · 推理强度”，菜单分节列出运行时、模型和推理强度。第二个是权限模式；ACP 会话选项也归入这里。去掉自定义 tint，跟随系统外观。
2. 点击发送后，用户消息立即出现在消息列表中，并标为“发送中”，直到运行时确认。
3. 每个 turn 的配置显示在工作过程行右侧，并再收拢：平时只有一个 plain 图标，悬停该行时展开摘要，点击图标显示完整配置。
4. 权限模式是会话状态：不选时沿用会话当前的模式，选择器显示当前模式，不读取 Harness 配置来推断“默认”（见下文“权限模式的会话语义”）。
5. 验收反馈：三角离标题太近；第一个下拉的菜单太长、太杂，可用二级菜单。改为运行时、模型、推理强度三个子菜单（标题带当前选择），设置入口归入对应子菜单。

## 观察

- 深色模式：composer 的下拉统一加了 `.tint(Color(nsColor: .controlColor))`，本意是避免并排的强调色指示器。离屏渲染（非 key 窗口不应用 tint）未能复现；开发版无法在不切前台的情况下改外观，所以没有直接看到问题。改版去掉这个 tint，改用无边框原生下拉，问题不再依赖这一推断。
- 发送：`Store.send` 在串行队列里同步执行 `sendTurn`。核心先准备进程（Claude 每轮启动、Codex 换网关时重启 app-server），再发送，期间不能轮询；`executionStarting` 只让侧栏与消息列表显示运行指示，用户消息要等发送返回、Harness 回显后才出现。
- 原生记录里的轮次配置（固定版本，合成会话）：
  - Codex 0.159.3：rollout 的 `turn_context` 按 `turn_id` 记录 `model`、`effort`、`approval_policy`、`sandbox_policy`、`approvals_reviewer`。app-server 的 `Turn` 不含这些字段；`thread/items/list` 每项带 `turnId`，`thread/read` 给出 rollout 的 `path`（标为 UNSTABLE）。
  - Claude Code 2.1.286：用户记录有 `permissionMode`，assistant 记录有 `message.model`，不记录推理强度。
  - Pi 1.0.2：assistant 消息带 `model`；思考级别只有会话级的 `thinking_level_change` 记录。
  - DSH、通用 ACP：没有可核对的逐轮记录。
- managed 模型交给 Harness 的模型名是提供商的 model ID（`native_composition.rs` 中的 `model_alias`），原生记录里也是这个值，不能反推是哪个提供商。

## 方案

**核心**：

- `conversation::TurnSettings` 包含：运行时实例 ID、模型（显示文本）、推理强度 ID、权限模式 ID，均为可选。
- 用户消息带可选的原生 `turn_settings`，由 adapter 从原生记录填写：
  - Claude：取该轮用户记录的 `permissionMode`，模型取其后第一条 assistant 记录。
  - Codex：按 item 的 `turnId` 对应 rollout 的 `turn_context`；权限按预设反推为 read-only、workspace、auto-review 或 full-access，对不上时保留原值。只读本机 rollout；SSH 远端不读。
  - Pi：模型取该轮 assistant 消息；思考级别取用户消息之前最近的设置。
- application 发送时把实际请求（运行时、所选模型的显示名、推理强度、权限模式）交给 `TranscriptProjection`。projection 把它绑定到发送后新出现的用户消息，原生身份确认时迁移，与轮次耗时同一生命周期，只在内存中、不持久化。
- `TranscriptTurn.settings` 逐项合并：
  - 模型：优先 Velune 请求的显示名，其次原生值。
  - 推理强度、权限：优先原生记录，其次请求。
  - 运行时：取所在段的实例。

**Mac**：

- 两个无边框下拉。
- 发送时先在本地插入待发送的用户消息，标为“发送中”，composer 立即清空。待发送行在核心快照出现新的用户消息、或这一轮结束时移除。发送失败时移除该行，正文和附件放回 composer（composer 已有新内容时不覆盖），并记录问题。
- 工作过程行右侧放一个 plain 图标按钮：悬停该行时在图标左侧展开摘要，点击弹出 popover 显示完整配置。没有工作过程的轮次在用户消息后显示同样的配置行（不可展开）。

## 切片

1. composer 两个下拉与深色模式。
2. 发送即显示。
3. 轮次配置：核心类型与来源（Claude、Codex、Pi、请求）、bindings、Mac 显示。

## 实施（2026-10-13）

- 切片 1、2：86715f1。两个下拉使用 `.menuStyle(.button)` 加 `.buttonStyle(.borderless)`，去掉 tint。附件按钮同样无边框。发送时先在 `TranscriptModel` 末尾插入本地行（`TranscriptRow.status`）；`Store.send` 返回是否已发出，未发出时 composer 不清空。
- 切片 3：
  - 核心：`conversation::TurnSettings`、`Message.turn_settings`（原生）、`TranscriptTurn.settings`（合并结果）。`TranscriptProjection::request`／`withdraw_request` 处理 Velune 的请求，按发送前的用户消息数绑定到之后出现的用户消息，不依赖消息 ID 或 projection 的重置时机。
  - 来源：Claude 在 `claude_history.py`，Codex 读 rollout（`codex_history.rs`），Pi 在 `projection.rs` 的 `replace_history`。application 按段补运行时实例（`stamp_turn_runtime`），并把 Pi 的网关路由 alias 换回 managed 模型名（`name_turn_models`）。bindings 增加 `BindingTurnSettings`。
  - Mac：`TranscriptModel.items(for:turnHeaders:)` 为没有工作过程的 turn 补表头行；`TranscriptTurnSettingsView` 只让图标接收点击，整行用 tracking area 显示摘要；popover 用 SwiftUI `Grid`。

## 权限模式的会话语义（2026-10-13）

- 证据：
  - 写轮次配置脚本时，Claude 历史显示：先 plan、再不选模式的第二轮，记录仍是 `permissionMode: plan`。
  - 对照实验（先 plan 再不选，与两轮都不选）中，第二轮请求的 `safeguards[].classifier_context.permission_mode` 分别是 `plan` 和 `auto`。Claude Code 在 `--resume` 时沿用上次的模式，CLI 没有“回到默认”的取值。
  - plan 提醒是稀疏发送的，因此之前以“新提示后没有 plan 提醒”判定离开 plan 是错的。mvp-daily-driver packet 中 b6cc949 的这条结论作废。
  - Codex 在 Velune 托管路径下每轮重启 app-server 后恢复线程，不选模式时回到 workspace-write，即上一轮的覆盖不保留。
- 用户决定沿用会话当前模式。实现：
  - Codex 不选时不发策略字段。
  - application 在未指定时取会话最近一轮在同一运行时上的模式，重新发送（`requested_permission_mode`）；不知道就不发。
  - Mac 的选择器显示最近一轮记录的模式，菜单首项为“沿用当前（…）”。
- 已知局限：Velune 发出的“未选”轮次在实时阶段只知道请求值。若会话的当前模式来自 Velune 之外，重新读历史后才显示。

## 验证

- fmt／check／clippy／no-default-features、Mac `--build-only` 通过。
- `manual-permission-modes.py` 三项 PASS：
  - Codex：不选时保持 full-access，选 workspace 后回到 workspace-write。
  - Claude：不选时保持 plan，选 auto 后为 auto。
- 新增 `manual-turn-settings.py`，四项 PASS：
  - 实时：三种 Harness 显示请求的运行时、模型名、强度和模式；不选模式的第二轮沿用第一轮的模式。
  - 重启后：Codex 显示 `synthetic-responses`／high／read-only；Claude 显示 `synthetic-messages`／plan；Pi 的模型显示为“responses · responses”。
- 回归通过：`manual-acp-adapter`、`manual-runtime-selection`、`manual-codex-history-identity`（revert／items）、`manual-continuation-boundaries`、`manual-native-subagents`、`manual-live-native`、`manual-reasoning-effort`、`manual-attachments`、`manual-collaboration`、`manual-parallel-sessions`。
- 开发版（临时 HOME、后台启动）：
  - 浅色与深色截图中，两个下拉都随外观变化。深色截图用了临时强制外观的代码，未提交。截图时窗口曾成为前台并弹开菜单，原因未确认，此后不再用这种方式。
  - 工作过程行右端显示信息图标，权限按钮显示会话当前模式“Auto”。
  - 窗口在另一桌面空间，AX 不可达。悬停摘要、popover 与“发送中”流程未在 App 中实际操作，需用户验收。

## 验收反馈修订（2026-10-13）

- 下拉的三角由系统绘制、紧贴标题。给标题加 padding 不生效；隐藏系统三角、在 label 里自绘也不行，因为菜单按钮只取 label 的文字和图标，自绘的图标被放到了标题前。最后在标题末尾加一个不换行空格，三角仍由系统绘制。开发版截图确认间距变宽。
- 第一个下拉的菜单只保留三个子菜单（运行时、模型、推理强度），标题带当前选择。资源库与运行时设置放在“运行时”子菜单，重新读取模型与提供商设置放在“模型”子菜单。为避免开发版窗口被带到前台，菜单展开后的样子未截图，需用户验收。

## 安装

- 18f38dd 已构建安装至 `/Applications/Velune.app`（签名校验通过，manifest 对应该提交且无未提交改动），重新打开后无崩溃报告。
- 3a6fe45（子菜单与三角间距）已构建安装，签名校验通过，manifest 对应该提交且无未提交改动，重新打开后无崩溃报告。
