# Windows 与 Mac 的对齐

Windows 按 [PRD](../../docs/prd/index.md) 跟进完整端能力，规则见[本地指引](AGENTS.md)。本表维护现行差距与验证范围；具体呈现与构建见 [README](README.md)，内部布局设计见 [Unit TDD](TDD.md)。平台改动后更新对应行，实施过程和验证证据保留在任务包。

本表于 2026-11-06 汇总既有记录，未重新执行原生界面验收；各行的“已接入”或“已实现”不扩大已有验证结论。

| 能力 | Windows 现状 | 与 Mac 的差距及验证限制 |
| --- | --- | --- |
| [会话](../../docs/prd/conversations.md) | 会话列表、管理、Codex 原生已读 receipt 与 xURL 已接入；菜单消费核心 `can_rename` | 未注册 `velune://`；“跟随当前运行时”仍取最后一轮的运行时，Mac 取上下文运行时；最近项目与运行状态文字仍由平台计算；自动命名没有 Mac 的专门模型 Picker。运行中重命名的共享核心变更未做 Windows 原生复验 |
| [会话视图](../../docs/prd/conversation-view.md) | client 内容右对齐、运行时状态左对齐；工作过程与正文分开呈现 | 未接入统一 Artifact、即时提交占位与消息操作行时间；长转录性能未在真机验收 |
| [输入内容与导航](../../docs/prd/composer-input.md) | 已接入共享 InputID、目标配置、CAS、本机未同步缓存与原生创建交接 | 仍按每 Tab 呈现输入；共享 drawer 的导航／聚焦体验、冲突输入恢复、侧聊下一轮改选与一行 drawer 尚未跟进 Mac。旧 Schema 1 缓存保留并要求显式迁移；中文输入法未在真机验收 |
| [下一轮选择与额度](../../docs/prd/composer-choices.md) | 已接入共享目标准备用例与 resolver；运行时菜单保留准备失败时的修复入口 | 模型目录读取中或失败时未先列出可达的 managed 模型；状态弹层未接入所选模型／路由的提供商额度和原生 Claude 登录额度查询，仍只显示轮次记录限额。共享目标准备尚未做 Windows 原生 UI 验收 |
| [提交与恢复](../../docs/prd/composer-actions.md) | 发送／排队／插入消费核心准入；文本、粘贴与附件编辑资格独立于提交；发送／排队／继续共用主按钮，停止独立 | 主动压缩已有输入区、更多菜单和快捷键入口，未提供 Mac 的会话菜单、命令面板与 `/compact` 入口。最新准入接入仅完成隔离构建与打包，未做 Windows 原生 UI 验收 |
| [工作区布局](../../docs/prd/workspace-layout.md) | 窗口、分屏、标签与工具布局已实现（[Unit TDD](TDD.md)） | Mac 的会话 Tab 菜单与未读／失败呈现、会话详情中的发送队列、窗口工具栏中的管理进度、独立窗口默认 composer、完整输入恢复点与 editor journal 尚未跟进；布局未在真机逐项验收 |
| [键盘操作](../../docs/prd/keyboard.md) | 命令表与可配置快捷键已实现 | 完整键盘路径、浅／深色界面及真机操作验收待做；局部入口检查不代替整体验收 |
| [协作](../../docs/prd/collaboration.md) | 关联 Agent、xURL MCP 工具与随包官方 CLI 已实现 | xURL 深链主路径已在 Windows 与 Mac 验收，其他协作界面不据此扩大验收结论 |
| [Agent 运行时](../../docs/prd/agent-runtimes.md) | 本机运行时及已有实例类型编辑已接入 | 经 SSH 的运行时只能在 Mac 上设置：Windows 宿主没有 OpenSSH ControlMaster 所需的 Unix socket；类型编辑尚未做 Windows 原生 UI 验收 |
| [AI 提供商](../../docs/prd/ai-providers.md) | 官方 API 模板、订阅识别、独立组织账单、额度节奏／金额／历史与提醒已接入；设置没有额外的全局运行中禁用 | 额度窗口名称仍由窗口 ID 拼出；历史使用自绘柱条与悬停提示，Mac 使用选中读数。官方 API 编辑器与账单入口已通过构建、浅／深色原生 UIAutomation 和正常退出检查，尚未用真实账户联调；按依赖判断忙时资格的共享核心变更未做 Windows 原生复验 |
| [模型路由](../../docs/prd/model-routing.md) | 已实现；额外请求参数可编辑，模型速度以目录变体呈现 | 成员顺序用上移／下移，不用拖动；模型编辑器后续改动仅有隔离源码构建证据 |
| [资源库](../../docs/prd/resource-library.md) | 本机核心已接入，界面没有额外的全局运行中禁用 | 技能只编辑 SKILL.md；Windows 不写技能执行属性。按依赖判断同步资格的共享核心变更未做 Windows 原生复验 |
| [定时任务](../../docs/prd/scheduled-tasks.md) | 共享调度业务 API 与执行 owner 已接入 | 原生任务管理与来源交接窗口尚未实现 |
| [分析](../../docs/prd/analytics.md) | 本机核心已接入 | — |
| [问题与诊断](../../docs/prd/diagnostics.md) | 问题面板、本机遥测设置、双 SDK、显式 OTLP 测试与独立配置分享已接入 | Mac 的问题历史读回尚未移植；系统通知横幅与点按打开未目视验收。标准遥测语义归[共享契约](../../docs/product-tdd/telemetry.md) |
| [远程访问](../../docs/prd/remote-access.md) | 配对、控制请求与目标设备切换已接入 | — |
| [平台与外观](../../docs/prd/platforms.md) | 未打包的 WinUI 3 应用，已接入 Velopack 每用户安装与 GitHub Releases beta 更新；用户确认后正常退出再升级 | 更新状态与进程等待仅做所属层逻辑检查，安装、浅／深色界面与实际升级待统一 Windows 验收；独立字号／密度设置尚未跟进 Mac；局部浅／深色检查不代表整个平台已完成原生验收 |

验证范围与证据入口：

- [Composer 状态归属](../../tasks/composer-state-ownership/packet.md)：共享输入、配置分离、修订与交接的实施和验证范围；不代表 Mac 的共享 drawer 体验已移植。
- [组织用量与账单](../../tasks/api-provider-templates/packet.md#组织账单改造实施2026-11-02)：官方 API 编辑器、账单入口与原生检查，尚未进行真实账户联调。
- [消息与工作区改进](../../tasks/messaging-and-workspace-polish/packet.md)和[后续输入／导航改进](../../tasks/post-dogfood-ux/packet.md)：Mac 呈现变化及各端构建范围，平台差距以本表对应行维护。
- [开发构建与缓存](../../tasks/build-performance/packet.md)：隔离开发构建、无变化复用与来源保护检查；未启动 Windows UI。构建行为归[开发构建说明](../../docs/development/builds.md)。
