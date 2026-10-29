# Windows 与 Mac 的对齐

新功能与行为改变尽可能先在 Mac 落地，Windows 按 [PRD](../../docs/prd/index.md) 跟进（规则见 [本地指引](AGENTS.md)）。本表按 PRD 分册记录 Windows 的现状，只写与 Mac 的差距与原因；Windows 的具体呈现见 [README](README.md)。平台改动后更新对应行和下方的核对基线。

核对基线：main `2b89eac`（2026-10-21）；Windows 截至 2026-10-22；xURL 深链于 2026-10-23 对齐，拷贝子菜单与 xURL 导出于 2026-10-26 对齐；token 单位显示与输入于 2026-10-26 对齐；额度节奏、金额、历史、提醒与账户额度开关于 2026-10-27 对齐。

| PRD 分册 | Windows 现状 | 与 Mac 的差距 |
| --- | --- | --- |
| [会话](../../docs/prd/conversations.md) | 已实现 | Windows 未注册 `velune://`；xURL 支持 `agents://` 系统深链与会话正文点击，唯一匹配自动打开 |
| [会话视图](../../docs/prd/conversation-view.md) | 已实现 | 长转录的性能未在真机验收 |
| [输入区](../../docs/prd/composer.md) | 已实现 | 模型目录读取中或失败时只说明状态，不像 Mac 先列出可达的 managed 模型；中文输入法未在真机验收；状态弹层尚未接入所选模型或路由的提供商额度，也未接入核心的原生 Claude 登录额度查询，只显示原生轮次的限额 |
| [工作区布局](../../docs/prd/workspace-layout.md) | 已实现（[Unit TDD](TDD.md)） | 未在真机逐项操作验收 |
| [键盘操作](../../docs/prd/keyboard.md) | 已实现 | win-ws 构建通过；浅色与深色界面检查及真机验收待做 |
| [协作](../../docs/prd/collaboration.md) | 关联 Agent、xURL MCP 工具与随包官方 CLI 已实现 | — |
| [Agent 运行时](../../docs/prd/agent-runtimes.md) | 本机运行时已实现 | 经 SSH 的运行时只能在 Mac 上设置：Windows 宿主没有 OpenSSH ControlMaster 所需的 Unix socket |
| [AI 提供商](../../docs/prd/ai-providers.md) | 已实现 | 额度窗口名称由 Windows 按核心的窗口 ID 拼出，核心尚未提供显示名；额度历史为对话框中的自绘柱条，悬停提示代替 Mac 的选中读数 |
| [模型路由](../../docs/prd/model-routing.md) | 已实现 | 成员顺序用上移、下移，不用拖动（WinUI 拖动重排的单一父元素问题） |
| [资源库](../../docs/prd/resource-library.md) | 已实现，本机核心 | 技能只编辑 SKILL.md；Windows 没有执行位，技能文件的执行属性不写入 |
| [分析](../../docs/prd/analytics.md) | 已实现，本机核心 | — |
| [问题与诊断](../../docs/prd/diagnostics.md) | 已实现 | 系统通知横幅与点按打开未目视验收 |
| [远程访问](../../docs/prd/remote-access.md) | 完整端：配对、控制请求、切换目标设备 | — |
| [平台](../../docs/prd/platforms.md) | 未打包的 WinUI 3 应用 | xURL 深链主路径已在 Windows 与 Mac 验收；其他界面不据此扩大验收结论 |

补充说明：

- 通用设置分别选择额度与上下文按占用或剩余显示，状态栏消费两个核心偏好。原生 Codex 项目按独立身份列出，可在编辑对话框修改原生名称与有序文件夹（含空列表）；目录建议使用 `PrimaryFolder`，不使用项目身份键。
- 发送／排队／继续是一个主按钮，停止独立，与 Mac 相同：实时正文与附件使继续和发送互斥，发送键不触发空输入恢复，组词、停止或请求在途时禁止提交。
- 主动上下文压缩有输入区按钮、更多菜单与可配置快捷键命令，支持 Codex、Claude Code 与 Pi；Mac 另有会话菜单、命令面板与 `/compact`。
- 额度：智能策略说明与核心的重置节奏排序一致；金额以文字显示；有预计用尽提示（注意色）、路由成员的金额、编辑器的“读取 … 账户额度”开关、“历史…”对话框与额度提醒通知。
- 与 Mac 的已知差别：状态行的账户限额来自轮次记录，Mac 来自输入区额度查询；“跟随当前运行时”取最后一轮的运行时，Mac 取上下文运行时；最近项目与运行中状态文字各自计算。
- 遥测：本机设置、双 SDK 代际、显式 OTLP 测试、独立分享邀请／授权／撤销与一次性复制已接入，使用官方 .NET SDK。调用使用显式 NativeOperation 类别，与 Apple／Android 的同类操作一致，不依赖 CallerMemberName；标准事件名、字段与结果含义归[遥测契约](../../docs/design/telemetry.md)。
