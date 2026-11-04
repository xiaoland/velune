# Windows 与 Mac 的对齐

新功能与行为改变尽可能先在 Mac 落地，Windows 按 [PRD](../../docs/prd/index.md) 跟进（规则见 [本地指引](AGENTS.md)）。本表按 PRD 分册记录 Windows 的现状，只写与 Mac 的差距与原因；Windows 的具体呈现见 [README](README.md)。平台改动后更新对应行和下方的核对基线。

核对基线：main `2b89eac`（2026-10-21）；Windows 截至 2026-10-22；xURL 深链于 2026-10-23 对齐，拷贝子菜单与 xURL 导出于 2026-10-26 对齐；token 单位显示与输入于 2026-10-26 对齐；额度节奏、金额、历史、提醒与账户额度开关于 2026-10-27 对齐。

| PRD 分册 | Windows 现状 | 与 Mac 的差距 |
| --- | --- | --- |
| [会话](../../docs/prd/conversations.md) | 已实现 | Windows 未注册 `velune://`；xURL 支持 `agents://` 系统深链与会话正文点击，唯一匹配自动打开 |
| [会话视图](../../docs/prd/conversation-view.md) | 已实现 | 长转录的性能未在真机验收 |
| [输入区](../../docs/prd/composer.md) | 已实现；消费共享控制权锁定与原因 | 模型目录读取中或失败时只说明状态，不像 Mac 先列出可达的 managed 模型；中文输入法未在真机验收；状态弹层尚未接入所选模型或路由的提供商额度，也未接入核心的原生 Claude 登录额度查询，只显示原生轮次的限额 |
| [工作区布局](../../docs/prd/workspace-layout.md) | 已实现（[Unit TDD](TDD.md)） | 未在真机逐项操作验收 |
| [键盘操作](../../docs/prd/keyboard.md) | 已实现 | win-ws 构建通过；浅色与深色界面检查及真机验收待做 |
| [协作](../../docs/prd/collaboration.md) | 关联 Agent、xURL MCP 工具与随包官方 CLI 已实现 | — |
| [Agent 运行时](../../docs/prd/agent-runtimes.md) | 本机运行时已实现 | 经 SSH 的运行时只能在 Mac 上设置：Windows 宿主没有 OpenSSH ControlMaster 所需的 Unix socket |
| [AI 提供商](../../docs/prd/ai-providers.md) | 已实现；官方 API 模板与订阅识别复用共享目录；已接入独立组织账单面板 | 本次官方 API 编辑器与账单入口已通过构建、浅／深色原生 UIAutomation 和正常退出检查；额度窗口名称由 Windows 按核心的窗口 ID 拼出，核心尚未提供显示名；额度历史为对话框中的自绘柱条，悬停提示代替 Mac 的选中读数 |
| [模型路由](../../docs/prd/model-routing.md) | 已实现 | 成员顺序用上移、下移，不用拖动（WinUI 拖动重排的单一父元素问题） |
| [资源库](../../docs/prd/resource-library.md) | 已实现，本机核心 | 技能只编辑 SKILL.md；Windows 没有执行位，技能文件的执行属性不写入 |
| [定时任务](../../docs/prd/scheduled-tasks.md) | 共享调度业务 API 与执行 owner 已接入 | 原生任务管理与来源交接窗口尚未实现；Mac 已有入口 |
| [分析](../../docs/prd/analytics.md) | 已实现，本机核心 | — |
| [问题与诊断](../../docs/prd/diagnostics.md) | 已实现 | 系统通知横幅与点按打开未目视验收 |
| [远程访问](../../docs/prd/remote-access.md) | 完整端：配对、控制请求、切换目标设备 | — |
| [平台](../../docs/prd/platforms.md) | 未打包的 WinUI 3 应用 | xURL 深链主路径已在 Windows 与 Mac 验收；其他界面不据此扩大验收结论 |

补充说明：

- 2026-11-02 控制权只限制提交，各端保留本地草稿编辑；Windows 的文本、粘贴与附件准入与提交资格分离。Mac 本轮进一步实现输入状态随导航／共享工作空间转换、冲突草稿恢复、侧聊下一轮改选和一行 drawer，Windows 尚未跟进这些界面能力；频率限制恢复由共享核心处理。本轮 Windows 构建与原生视觉结果以任务验证记录为准。

- 2026-11-02 运行时保存、提供商操作与资源同步的忙时判断已在共享核心改为按依赖检查，Mac 去掉全局运行中禁用；Windows 设置与资源库没有同类全局界面禁用。本轮行为与构建验证在 Mac 完成，Windows 未重新构建／真机复验。
- 同日共享核心将重命名与删除拆开：支持的原生运行时可在运行中改标题并保留执行。Mac 移除额外的运行中禁用；Windows 原有菜单按核心 `can_rename` 消费，没有额外运行状态限制，本轮未重新构建／真机复验。
- 通用设置分别选择额度与上下文按占用或剩余显示，状态栏消费两个核心偏好。原生 Codex 项目按独立身份列出，可在编辑对话框修改原生名称与有序文件夹（含空列表）；目录建议使用 `PrimaryFolder`，不使用项目身份键。
- 发送／排队／继续是一个主按钮，停止独立，与 Mac 相同：实时正文与附件使继续和发送互斥，发送键不触发空输入恢复，组词、停止或请求在途时禁止提交。
- 主动上下文压缩有输入区按钮、更多菜单与可配置快捷键命令，支持 Codex、Claude Code 与 Pi；Mac 另有会话菜单、命令面板与 `/compact`。
- 额度：智能策略说明与核心的重置节奏排序一致；金额以文字显示；有预计用尽提示（注意色）、路由成员的金额、编辑器的“读取 … 账户额度”开关、“历史…”对话框与额度提醒通知。
- 与 Mac 的已知差别：状态行的账户限额来自轮次记录，Mac 来自输入区额度查询；“跟随当前运行时”取最后一轮的运行时，Mac 取上下文运行时；最近项目与运行中状态文字各自计算。
- 遥测：本机设置、双 SDK 代际、显式 OTLP 测试、独立分享邀请／授权／撤销与一次性复制已接入，使用官方 .NET SDK。调用使用显式 NativeOperation 类别，与 Apple／Android 的同类操作一致，不依赖 CallerMemberName；标准事件名、字段与结果含义归[遥测契约](../../docs/design/telemetry.md)。

组织用量与账单与 Mac 使用同一核心契约：独立查询认证、组织／Project／Workspace／Key 范围、本月 UTC 报表、部分失败／过期状态；费用、支出限额与速率配置分别展示，不参与准入。源码构建与浅／深色原生入口、必填校验、轮询和正常退出检查通过；验证范围与证据由 [任务记录](../../tasks/api-provider-templates/packet.md#组织账单改造实施2026-11-02)保留，尚未以真实账户联调。


2026-11-03：共享契约已接入 Codex 0.166 原生已读 receipt，列表动作回传所见版本，自动确认按正文 receipt 去重并避免冲突重试。Mac 新增的问题历史读回、完整输入的窗口恢复点及管理请求队列尚未移植到 Windows；此轮未做 Windows 原生运行验收。


### 2026-11-03 后续 Mac 呈现差异

Mac 新增统一 Artifact（Mermaid 查看、sheet／Tab／独立窗口）及通用编辑文稿承载，Windows 尚未接入。Mac 会话 Tab 复用列表菜单并显示未读／失败，发送队列移入各会话详情，跨会话管理进度放窗口 toolbar；Windows 仍需同步这些呈现调整。核心的 Pi cwd／虚拟模型能力／steering、控制权等待和自动命名兜底规则共享；Windows 自动命名尚无 Mac 的专门模型 Picker。本批未进行 Windows 原生 UI 验收。


### Composer 状态同步（2026-11-03）

Windows 接入共享 InputID、全局新建输入、完整正文／持久附件／执行选择、输入 CAS 与本机未同步基线；运行时选择包含执行来源。已知原生身份先保存待完成交接，再派发首条消息；读取共享创建记录后，正在编辑同一新建输入的视图接续原生会话。`Conversation.ComposerSync.cs` 拥有目录与缓存，view 只同步原生文本控件。Windows 仍按自身现有每 Tab 输入呈现；Mac 的共享 drawer 导航／聚焦体验不因此宣称已整体移植。验证证据与限制见[任务记录](../../tasks/composer-state-ownership/packet.md)。


### 消息与工作区改进（2026-11-04）

共享核心已支持 Pi 的 `continue` 文本兜底及独立 Client 通知；Windows 将 client 内容右对齐、运行时状态左对齐。模型速度表达为目录中的独立模型条目：managed 模型记录在编辑器中携带额外请求参数，Codex 原生目录的档位由 adapter 摊平为变体模型；Windows 模型编辑器支持编辑额外请求参数。Windows 隔离源码构建通过，本轮未启动 UI。Mac 的即时提交占位、消息操作行时间、独立窗口默认 composer、editor journal 恢复及独立字号／密度设置尚未移植到 Windows。


### Composer 正文与目标配置分离（2026-11-04）

Windows 按共享契约分别同步正文／附件的 InputID 与目标上下文的运行时／模型／推理强度／权限选择，配置采用独立 revision、CAS 与离线待同步缓存。正文恢复不再改写发送配置，配置恢复也不替换正文。既有会话读取实际 snapshot 后才初始化目标；新建目标全局唯一，原生创建前捕获配置，交接时为实际会话单独初始化。旧缓存 Schema 1 保留并要求显式迁移；待同步冲突保留本地编辑并限制提交。Windows 仍沿用每 Tab 输入呈现。本轮 Windows 隔离源码构建通过，未进行 Windows UI 验收。
