# Windows 与 Mac 的对齐

新功能与行为改变尽可能先在 Mac 落地，Windows 按 [PRD](../../docs/prd/index.md) 跟进（规则见 [本地指引](AGENTS.md)）。本表按 PRD 分册记录 Windows 的现状，只写与 Mac 的差距与原因；Windows 的具体呈现见 [README](README.md)。平台改动后更新对应行和下方的核对基线。

核对基线：main `2b89eac`（2026-10-21）；Windows 截至 2026-10-22；xURL 深链于 2026-10-23 对齐，拷贝子菜单与 xURL 导出于 2026-10-26 对齐；token 单位显示与输入于 2026-10-26 对齐。

| PRD 分册 | Windows 现状 | 与 Mac 的差距 |
| --- | --- | --- |
| [会话](../../docs/prd/conversations.md) | 已实现 | Windows 未注册 `velune://`；xURL 支持 `agents://` 系统深链与会话正文点击，唯一匹配自动打开 |
| [会话视图](../../docs/prd/conversation-view.md) | 已实现 | 长转录的性能未在真机验收 |
| [输入区](../../docs/prd/composer.md) | 已实现 | 模型目录读取中或失败时只说明状态，不像 Mac 先列出可达的 managed 模型；中文输入法未在真机验收 |
| [工作区布局](../../docs/prd/workspace-layout.md) | 已实现（[Unit TDD](TDD.md)） | 未在真机逐项操作验收 |
| [协作](../../docs/prd/collaboration.md) | 关联 Agent、xURL MCP 工具与随包官方 CLI 已实现 | — |
| [Agent 运行时](../../docs/prd/agent-runtimes.md) | 本机运行时已实现 | 经 SSH 的运行时只能在 Mac 上设置：Windows 宿主没有 OpenSSH ControlMaster 所需的 Unix socket |
| [AI 提供商](../../docs/prd/ai-providers.md) | 已实现 | 额度窗口名称由 Windows 按核心的窗口 ID 拼出，核心尚未提供显示名；没有额度历史、额度提醒通知与预计用尽提示；官方账户额度只随模板开启，编辑器没有开关 |
| [模型路由](../../docs/prd/model-routing.md) | 已实现 | 成员顺序用上移、下移，不用拖动（WinUI 拖动重排的单一父元素问题） |
| [资源库](../../docs/prd/resource-library.md) | 已实现，本机核心 | 技能只编辑 SKILL.md；Windows 没有执行位，技能文件的执行属性不写入 |
| [分析](../../docs/prd/analytics.md) | 已实现，本机核心 | — |
| [问题与诊断](../../docs/prd/diagnostics.md) | 已实现 | 系统通知横幅与点按打开未目视验收 |
| [远程访问](../../docs/prd/remote-access.md) | 完整端：配对、控制请求、切换目标设备 | — |
| [平台](../../docs/prd/platforms.md) | 未打包的 WinUI 3 应用 | xURL 深链主路径已在 Windows 与 Mac 验收；其他界面不据此扩大验收结论 |

2026-10-26：通用设置拆分额度与上下文的占用／剩余显示，状态栏分别消费两个核心偏好。原生 Codex 项目按独立身份列出，可在编辑对话框直接修改原生名称、有序文件夹（含空列表）；目录建议使用 `PrimaryFolder`，不使用项目身份键。


2026-10-26：Codex／Claude Code 无新消息恢复的接口与“继续”按钮分派已按 Mac 接入；本次 Windows 构建与浅／深色实际运行的验证状态见 [原生恢复任务](../../tasks/native-turn-resume/packet.md)。


遥测（2026-10-27）：本机设置、双 SDK 代际、显式 OTLP 测试、独立分享邀请/授权/撤销与一次性复制已接入；Windows 使用官方 .NET SDK。完整 WinUI 构建、浅/深色、计时器和正常退出已验收；SDK 构造失败清理、首次写出前撤销及开发临时配置重启也通过，跨端分享矩阵仍有设备阻塞，证据见[遥测任务](../../tasks/otlp-observability/packet.md)。

2026-10-27：Mac 与 Windows 同步为一个发送／排队／继续主按钮，停止独立；实时正文与附件使继续和发送互斥，发送键不触发空输入恢复，组词及停止／请求在途禁止提交。撤去文本继续 fallback。两端判定矩阵与 Windows 完整 WinUI 构建均通过；浅／深色各 11 组实际控件状态检查通过，正常退出且无 crash.log。合成验证的范围与证据见[任务记录](../../tasks/native-turn-resume/packet.md)。

2026-10-27：主动上下文压缩已接入共享核心与 Windows 输入区按钮、更多菜单及可配置快捷键命令，支持 Codex、Claude Code 和 Pi 的原生入口，压缩状态独立展示。Mac 另提供会话菜单、命令面板与 `/compact`。本轮构建与运行证据见[上下文压缩任务](../../tasks/context-compaction/packet.md)。

2026-10-27：额度借鉴 quotio（[任务](../../tasks/quotio-reference/packet.md)）。核心的智能排序考虑重置节奏，Windows 的智能策略说明已同步；金额以文字显示，`openrouter:keyLimit` 有显示名。额度历史、提醒通知、节奏提示与账户额度开关尚未接入。
