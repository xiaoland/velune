# Velune

### 把多个 Coding Agent 放进一个原生工作区。

Velune 是面向 Codex、Claude Code、Pi 等 coding agent 的控制界面与协调层。它帮助你在同一应用中浏览和继续不同运行时的会话、配置 AI 提供商与模型，并让不同 Agent 之间交接上下文、协作处理工作。

Velune 本身不是执行任务的 Agent，也不替代 Codex、Claude Code、Pi 等 Harness。它连接你安装和配置的运行时，由这些运行时执行代码、调用工具并保存各自的会话历史。

## 当前代码包含的能力

- **统一浏览会话**：查看不同运行时的会话、工作过程和状态，并按项目或运行时整理。
- **切换运行时继续工作**：在新运行时会话中交接可携带的文本上下文；旧会话里的工具执行和审批状态不会被重放。
- **配置模型与路由**：管理提供商模型，设置 LLM 路由；符合切换条件时，可在响应交给调用方前尝试下一个路由成员。订阅额度仍由各供应商分别管理。
- **连接 Agent 协作**：查看原生子 Agent 和相关会话，并通过 Velune 的协作能力联系其他会话。
- **管理共享资源**：整理指令、技能、MCP 服务器和子 Agent 定义，并同步到支持的运行时。

## 平台

| 平台 | 定位 | 当前状态 |
| --- | --- | --- |
| macOS | 完整端，可在本机运行 Agent | 原生应用持续开发与验证中 |
| Windows | 完整端，可在本机运行 Agent | WinUI 应用持续开发与验证中 |
| Android | 远程控制端 | 连接 Mac 或 Windows 完整端；不在手机上运行 Agent |

各平台都还在开发验证阶段，功能与操作细节会继续调整。

## 试用

目前 [GitHub Releases](https://github.com/xiaoland/velune/releases) 尚无可下载的安装包。开发者可以按[开发说明](docs/development.md)从源码构建，平台细节见 [macOS](app/mac/README.md)、[Windows](app/windows/README.md) 和 [Android](app/android/README.md) 说明。

运行 Agent 前，需要先自行安装并配置对应的 Harness。Velune 不附带这些运行时；添加模型提供商时，也需要使用者自行配置可用的账户或 API 凭据。多账户路由不会合并额度或绕过供应商规则。

## 文档与反馈

- [产品定义](docs/prd/index.md)：产品目标、范围与能力
- [架构说明](docs/design/architecture.md)：共享核心与平台应用的分工
- [完整文档索引](docs/index.md)
- [提交问题或建议](https://github.com/xiaoland/velune/issues)
