# Windows 与 Mac 的对齐

新功能与行为改变尽可能先在 Mac 落地，Windows 按 [PRD](../../docs/prd/index.md) 跟进（规则见 [本地指引](AGENTS.md)）。本表按 PRD 分册记录 Windows 的现状，只写与 Mac 的差距与原因；Windows 的具体呈现见 [README](README.md)。平台改动后更新对应行和下方的核对基线。

核对基线：main `2b89eac`（2026-10-21）；Windows 截至 2026-10-22。

| PRD 分册 | Windows 现状 | 与 Mac 的差距 |
| --- | --- | --- |
| [会话](../../docs/prd/conversations.md) | 已实现 | Windows 未注册 `velune://`，拷贝的链接要在 Mac 上打开 |
| [会话视图](../../docs/prd/conversation-view.md) | 已实现 | 长转录的性能未在真机验收 |
| [输入区](../../docs/prd/composer.md) | 已实现 | 模型目录读取中或失败时只说明状态，不像 Mac 先列出可达的 managed 模型；中文输入法未在真机验收 |
| [工作区布局](../../docs/prd/workspace-layout.md) | 已实现（[Unit TDD](TDD.md)） | 未在真机逐项操作验收 |
| [协作](../../docs/prd/collaboration.md) | 关联 Agent 已实现 | — |
| [Agent 运行时](../../docs/prd/agent-runtimes.md) | 本机运行时已实现 | 经 SSH 的运行时只能在 Mac 上设置：Windows 宿主没有 OpenSSH ControlMaster 所需的 Unix socket |
| [AI 提供商](../../docs/prd/ai-providers.md) | 已实现 | 额度窗口名称由 Windows 按核心的窗口 ID 拼出，核心尚未提供显示名 |
| [模型路由](../../docs/prd/model-routing.md) | 已实现 | 成员顺序用上移、下移，不用拖动（WinUI 拖动重排的单一父元素问题） |
| [资源库](../../docs/prd/resource-library.md) | 已实现，本机核心 | 技能只编辑 SKILL.md；Windows 没有执行位，技能文件的执行属性不写入 |
| [分析](../../docs/prd/analytics.md) | 已实现，本机核心 | — |
| [问题与诊断](../../docs/prd/diagnostics.md) | 已实现 | 系统通知横幅与点按打开未目视验收 |
| [远程访问](../../docs/prd/remote-access.md) | 完整端：配对、控制请求、切换目标设备 | — |
| [平台](../../docs/prd/platforms.md) | 未打包的 WinUI 3 应用 | 暗色主题未验收（会改动测试机的系统主题） |
