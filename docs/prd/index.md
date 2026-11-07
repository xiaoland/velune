# Velune 产品定义

Velune 是 coding agent 的统一控制界面与协调层，让多个订阅、AI 提供商和 Harness 在稳定、可恢复的体验中使用和协作，降低对单一供应商的依赖。执行 Agent 由 Harness 承载，Velune 自身不充当 Agent。

本目录维护当前产品目标和行为，不是交付状态清单。历史决定见[来源索引](../sources.md)，实现与验证边界见各 unit README 和[待决事项](../../tasks/open-items/packet.md)；早期原型授权不代表当前所有功能已验收。

## 优先级与范围

多订阅、多提供商、自动 LLM 路由、多 Harness 及会话间发现／交流／协作同属核心优先级，彼此未排定实现顺序。远程控制、多设备与多平台是在此基础上的扩展方向，不能替代核心目标。路由只选择模型及提供商，不包含设备或 Harness 调度。依据见 [S2](../sources/foundations.md#s2)。

首批 Harness 目标是 Codex、Claude Code、Pi；后续已增加 DeepSeek Harness 与通用 ACP 接入，实际支持版本归[运行时说明](../../packages/agent-runtime/README.md)。GitHub Copilot 不在范围内。产品坚持原生 UI、可恢复工作与明确的来源／权限边界；完整的目标、限制和术语见[范围与术语](scope.md)。

## 能力

分册是各项行为的权威来源。平台实现以分册为准，不从另一端源码反推产品规则。按当前问题选择一行，无需顺序加载全部文档。

| 主题 | 文档与内容 |
| --- | --- |
| AI 接入 | [AI 服务、提供商与模型](ai-providers.md)：提供商、模板、导入、协议、组织账单 |
| LLM 路由 | [模型路由与订阅聚合](model-routing.md)：fail-over、额度与订阅边界 |
| 执行环境 | [Agent 运行时](agent-runtimes.md)：类型／实例、ACP、HOME、SSH、版本与配置 |
| 资源 | [资源库](resource-library.md)：指令、MCP、技能与子 Agent 定义 |
| 协作 | [Agent 协作与跨 Harness 接续](collaboration.md)：原生子 Agent、A2A 与换 Harness 继续 |
| 会话 | [会话与会话列表](conversations.md)：来源、项目、列表管理、侧聊与命名 |
| 阅读 | [消息展示](conversation-view.md)：Markdown、Artifact、工作过程与大纲 |
| 输入与提交 | [内容与导航](composer-input.md)、[选择与状态](composer-choices.md)、[提交与恢复](composer-actions.md) |
| 工作区 | [多窗口、标签与布局](workspace-layout.md)：分屏、停靠、工具窗口与恢复 |
| 键盘 | [键盘操作](keyboard.md)：命令、发送、审批和默认快捷键 |
| 调度 | [本地定时任务](scheduled-tasks.md)：执行宿主、接管和应用启停边界 |
| 设备 | [远程控制与多设备](remote-access.md)：配对、完整端／控制端与输入接续 |
| 平台 | [平台与界面](platforms.md)：技术栈、原生体验与外观偏好 |
| 诊断 | [问题与诊断](diagnostics.md)：问题面板、本地记录与遥测 |
| 分析 | [分析](analytics.md)：token 消耗与性能 |

## 设计与开发入口

- [核心与平台契约](../product-tdd/index.md)：状态归属、架构和跨 unit 设计。
- [界面与交互原则](../../DESIGN.md)：平台原生体验及通用交互。
- [开发与复验](../development.md)：合成数据、构建、迁移及真实资源授权边界。
