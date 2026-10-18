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

## 第二批（2026-10-13，[S21](../../docs/sources.md#s21)）

已确认：

1. 模型、推理强度、权限模式按“会话 × 运行时”保存（应用运行期间）。切换运行时未发送时，离开再回来只把运行时恢复为会话当前的运行时；尚未首发的新会话保留所选运行时。
2. 某运行时在此会话中还没有选择时，模型默认取该运行时在此会话中最近一轮的模型。
3. 推理强度显示原值，不本地化。推理强度改为会话状态：不选时沿用最近一轮在同一运行时上的级别，菜单首项“沿用当前（…）”。
4. 轮次配置悬停只显示运行时与模型；详情加该轮 token 用量（含推理 token）。
5. 资源库：原生被改时可“用原生内容更新资源库”。实施与验证记在 [运行时资源管理任务](../runtime-management/packet.md#原生改动回填资源库2026-10-13s21)。

观察：

- “默认”此前不发送任何值。不发送时：Claude Code 每轮回到自身默认；Pi 沿用会话上次的思考级别；Codex 视路径而定（托管模型每轮重启 app-server 回到配置默认，原生模型沿用上一轮覆盖）。
- 轮次用量的原生来源：
  - Codex 0.159.3 rollout 的 `token_usage_record`：按 `turn_id`，`turn_token_usage` 是该轮累计，含 `reasoning_output_tokens`。实时通知 `thread/tokenUsage/updated` 带 `turnId`，以及每次响应的 `last`。
  - Claude Code 2.1.286：assistant 记录的 `message.usage`，同一 message id 会重复出现；不单列思考 token。
  - Pi 1.0.2：assistant 消息的 `usage`（`input` 不含缓存，`reasoning` 仅在提供商报告时出现，是 `output` 的子集）。Pi 实时快照也来自 `get_messages`。
- 网关的分析记录没有会话或轮次归属，不用于轮次用量。

实施（切片 4，composer 与轮次用量）：

- Mac：`NextTurnIntent` 改为每会话一份 `choices[运行时]`（模型、强度、模式）加只对未首发会话保留的运行时。打开会话时运行时取 `contextRuntimeID`，切换运行时时恢复该运行时的选择。只有用户的选择被记住；`reconcileModelSelection` 依次取：所选、该运行时最近一轮的模型、会话记录的 managed 模型、该运行时最近选择、目录第一项。最近一轮的模型按 Velune 名称、原生模型 ID、唯一的提供商 model ID 对应到选项。强度显示原值，菜单首项“沿用当前（…）”。
- application：`requested_effort` 与 `requested_permission_mode` 一样，未指定时取最近一轮在同一运行时上的值，所选模型仍接受才发送。
- 核心：`conversation::TurnUsage`（输入含缓存读写、缓存读取、缓存写入、输出含推理、推理，均可空）挂在用户消息 `turn_usage` 上，`TranscriptTurn.usage` 原样取用。来源：
  - Codex 历史读 rollout 的 `token_usage_record`；实时累加 `thread/tokenUsage/updated`。
  - Codex 每轮重启 app-server 后 `thread/resume` 会重建消息、丢掉已挂的用量，因此重建时也从本机 rollout 补回配置与用量。
  - Claude 历史按 message id 去重累加；实时取 `result.usage`。
  - Pi 在 `replace_history` 中累加。
- Mac：悬停只显示“运行时 · 模型”；popover 增“Token 用量”一节，推理未单独记录时注明。

验证（切片 4）：

- fmt／check／clippy／no-default-features、Mac `--build-only` 通过。
- `manual-turn-settings.py` 四项 PASS：
  - 上游每次请求报告固定用量（Responses 含缓存与推理，Messages 含缓存读写），实时与重启后读历史，三种 Harness 的轮次用量都等于请求数乘以单次用量；Claude 推理为空。
  - 不选强度的第二轮沿用第一轮的强度，Codex 与 Pi 的上游请求实际带该强度。
- `manual-reasoning-effort.py` 改为“不选时沿用当前”后 PASS（Codex、Claude、Pi 第三轮都带上一轮所选强度）。`manual-permission-modes.py` PASS。
- 回归 PASS：acp-adapter、runtime-selection、codex-history-identity、continuation-boundaries、native-subagents、live-native、attachments、collaboration、parallel-sessions。

## 验收反馈修订（2026-10-13）

- 下拉的三角由系统绘制、紧贴标题。给标题加 padding 不生效；隐藏系统三角、在 label 里自绘也不行，因为菜单按钮只取 label 的文字和图标，自绘的图标被放到了标题前。最后在标题末尾加一个不换行空格，三角仍由系统绘制。开发版截图确认间距变宽。
- 第一个下拉的菜单只保留三个子菜单（运行时、模型、推理强度），标题带当前选择。资源库与运行时设置放在“运行时”子菜单，重新读取模型与提供商设置放在“模型”子菜单。为避免开发版窗口被带到前台，菜单展开后的样子未截图，需用户验收。

## 会话当前、持久化与新会话默认（2026-10-18，[S42](../../docs/sources.md#s42)）

- 起因：Claude Code 会话的模型显示为目录第一项（“default”），推理强度为 medium。核对后：
  - `claude_history.py` 早已读出模型、推理强度与权限模式。之前对用户说的“完全不读 Claude 的原生设置”是错的，已更正。
  - 实际原因：原生记录里是完整模型 ID（如带日期的 ID），目录里是别名，两者对不上，Mac 于是回落到目录第一项。
  - 另一个原因：下一轮的选择只存在内存里，而且按标签页各存一份。
- 用户决定：
  - composer 的选择持久化，重启后保留。
  - 新会话默认用上次的选择，可在运行时实例中改为运行时默认或指定值。
  - “默认值”在 Velune 内部就是 null。
  - 其余按开发方建议：每项分会话当前与下一轮的选择两层，未知时不发送；模型同样如此；对不上目录的原生 ID 作为“会话当前”项原样发送；Velune 的发送记录持久化；选择在核心按会话保存、各标签页共享；推理强度保持单一维度。
- 核心：
  - `ModelSelection::RuntimeDefault` 删除，各处的选择改为 `Option<ModelSelection>`（bindings 与远控协议同步）。
  - `TurnSettings.model_selection` 是 Velune 请求的模型选择。
  - `composer_choices`／`set_next_turn_intent`／`set_runtime_composer_choices` 持久化到 `composer-choices.json`：各会话的下一轮运行时与各运行时的 `TurnChoice`，以及各运行时的上次选择与 `NewConversationChoice`。全空的记录不保存；未知运行时拒绝；删除会话或运行时实例时一并删除。
  - 发送请求按（用户消息序号，正文 sha256 前 16 位）存于 `turn-requests.json`：实时 ID 是临时的，原生 ID 要重读后才有，这两种 ID 都不能跨重读使用。打开与只读读取（`read_conversation`）都会找回这些记录；读历史时以历史中仍有的轮次为准重建。
  - Claude 历史另读 `permission-mode` 记录，补齐用户记录里没有 `permissionMode` 的轮次。
- Mac：
  - 未选的模型沿用当前；当前未知时为“运行时默认”（nil，不发送），不取目录第一项。
  - 发送不再要求先选模型。
  - 选择存于应用级 store，各标签页共享，经核心持久化。
  - 运行时实例设置新增“新会话”一节：上次的选择／运行时默认／指定，指定时每项也可为运行时默认。
  - 新会话设定只作为种子，不记成会话的选择。否则若持久化的选择在草稿建立后才读到，草稿会停在空选择上（开发版截图发现）。
- 顺带修复：main 合入远控库后 Mac 版无法编译。`adapterMetadataJson` 已从 bindings 删除，`BindingError.RemoteRequest` 与新的 `BindingFailureKind` 缺分支；Mac 已同步。
- 验证：
  - fmt／check（含 `--no-default-features --features remote`、workspace all-targets）与 Mac `--build-only` 通过。
  - `manual-turn-settings.py` 10 项 PASS，新增三项：
    - 重启后各轮仍显示 Velune 请求的模型（`model_selection` 与 Velune 名称）。
    - composer 选择跨重启保留；全空即无记录；未知运行时被拒绝。
    - 删除会话时连同其选择与请求记录一起删除。
  - 删除 `turn-requests.json` 后，原生记录的原有检查照旧通过。
  - 回归 PASS：`manual-reasoning-effort`、`manual-permission-modes`、`manual-edit-message`、`manual-send-queue`、`manual-runtime-selection`。
  - 开发版（临时 VELUNE_HOME、后台启动）：Pi 的新会话设为“指定：推理强度 low，模型运行时默认”后，空 composer 显示“Pi · 运行时默认 · low”。
  - 开发版副本访问外置卷上的 Pi CLI 时历史辅助进程超时（副本没有该卷的访问授权）。改用内置盘上的 Pi 副本后正常。
- 运行时默认的发送（选择为 None）不经网关，会走 Harness 自己的真实提供商，因此未做实测。代码上 None 不调用 `select_native_model`／`set_model`，不改原生配置。
- 留待用户验收：
  - 运行时设置中“新会话”一节的外观。
  - 带日期模型 ID 的 Claude 会话显示“沿用当前（ID）”。
  - 重启后各会话恢复的选择。

## 安装

- 18f38dd 已构建安装至 `/Applications/Velune.app`（签名校验通过，manifest 对应该提交且无未提交改动），重新打开后无崩溃报告。
- 3a6fe45（子菜单与三角间距）已构建安装，签名校验通过，manifest 对应该提交且无未提交改动，重新打开后无崩溃报告。
- e0feb02（第二批：按运行时保存的选择、推理强度沿用当前、轮次用量、资源库回填）已构建安装，签名校验通过，manifest 对应该提交且无未提交改动，重新打开后无崩溃报告。
