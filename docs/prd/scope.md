# 产品范围与术语

[产品入口](index.md)列出能力分册。本页维护跨能力的产品方向、边界和业务语言；具体行为以相应分册为准，技术责任归[Product TDD](../product-tdd/index.md)。

## 产品方向与质量

Velune 已取代 Factory26 的继续推进；Beluna 是独立的非 Coding 通用 Agent／Digital Life 姐妹项目，不共享本仓库范围（[S1](../sources/foundations.md#s1)）。最初动因包括更好使用已有 ChatGPT 订阅，但具体账户身份和凭据不进入仓库，也不据此推定权益可合并。

核心目标包括持久、可恢复的工作上下文、跨 Harness 协作、失败恢复和可观察的结果。早期任务—会话模型与任务树方向见 [S3](../sources/foundations.md#s3)；当前呈现已收敛为普通会话、原生子 Agent 与关联 Agent，不另设 Velune 父子任务树或子任务视图，见[协作规则](collaboration.md)。协作必须包含实际发现、消息接入、委派、结果回传、并发与失败协调，不能用保存消息和状态代替（[S10](../sources/foundations.md#s10)）。

完整体验应稳定、可恢复，不以单一路径演示作为完成。供应商和 Harness 依赖应清楚可辨。各平台遵循自身原生视觉与交互，品牌体现在图标、内容与少量细节，不覆盖系统整体风格（[S15](../sources/first-loop.md#s15)）。

公开 beta 起，升级必须保留用户已有数据，不能以版本或格式变化为理由清空正文、附件、配置、任务、授权及偏好。无法安全迁移时保留原数据并明确停止写入；降级遇到更高版本格式也不能重建为空值。分发版本及对应源码应可追溯到同一发布提交，用户无需自行编译即可从 GitHub Releases 获取已声明平台的安装产物（[S84](../sources/development.md#s84)）。

## Harness 与认证边界

首批固定目标为 Codex、Claude Code、Pi；“Cloud Code”在早期讨论中指 Claude Code。三者都要调查路由与协作接入，不能因兼容性差异删除目标。后续 DeepSeek Harness、通用 ACP 与版本化类型的决定见[运行时](agent-runtimes.md)，GitHub Copilot 明确排除（[S6](../sources/foundations.md#s6)、[S9](../sources/foundations.md#s9)）。

订阅账户、API 账户、模型供应商与 Harness 原生认证分别构成身份和权益边界。“多订阅”不意味着额度合并、无限使用、免 token 计费或可用于任意第三方工具。支持能力须按供应商、账户类型、地区／计划与官方规则验证；具体已知边界归[模型路由](model-routing.md)。接管模型路由不自动授权接管凭据。

应用配置根目录由 `VELUNE_HOME` 指定，默认 `~/.velune`。提供商与 Harness 配置持久保存；API key 可在本机提供商表单查看、修改，订阅认证保留必要的来源与刷新能力。该配置不改变 Harness 对会话历史的权威；开发方不读取、输出或提交真实秘密。操作与授权例外见[开发边界](../development.md)（[S15](../sources/first-loop.md#s15)）。

## 已确认的技术方向

Rust 是共享产品核心，SQLite 是已认可的存储技术；具体配置可用 JSON／TOML 或 SQLite，不意味着所有状态必须进同一个数据库。共享领域能力与平台 app 分离，平台通过 UniFFI 类型绑定在进程内消费共享库；不保留手写 JSON／C ABI 产品入口。MCP 提供协作工具，ACP 是可替换接入方式（[S10](../sources/foundations.md#s10)、[S17](../sources/early-integration.md#s17)）。

UI 使用原生技术，不采用 Web 前端。当前 Apple、Windows 与 Android 的框架及覆盖范围见[平台与界面](platforms.md)，不再用早期“Apple-universal 待定”描述后续已明确的平台。每个 package 和平台 app 有自己的契约与知识归属；内部模块不必独立成 unit，unit、语言 SDK 与二进制产物不要求一一对应。装配细节归[架构](../product-tdd/architecture.md)。

## 参考项目

- HAPI、Lody 用于解释想法，通常不要求复制功能或实现；Android 布局与交互的明确参考要求见[平台与界面](platforms.md)，保留声明、遵守许可，不复制 AGPL 源码（[S46](../sources/remote-platforms.md#s46)）。
- 可以研究参考项目的 Claude Code 订阅认证思路，但仍须用官方限制与实际证据判断。早期“原生 runtime＋官方登录＋隔离配置”是当时的待验证想法，不用它替代[AI 服务设计](../product-tdd/ai-service.md#认证)的现行契约。
- 跨 Harness 通信可参考 `lexoliu/acpsub`（2026-10-10 更正早期 acphub 拼写），允许其他方案，不构成强制技术依赖。

参考边界见 [S4](../sources/foundations.md#s4)、[S5](../sources/foundations.md#s5)、[S9](../sources/foundations.md#s9)。

## 稳定术语

| 术语 | 含义与边界 |
| --- | --- |
| LLM 供应商 | 提供可调用模型服务的一方，不等同于 Harness |
| 订阅／账户 | 供应商下的权益和身份边界，不推定跨工具通用 |
| Harness／Agent 运行时 | 承载 Agent 执行、工具与原生会话的运行载体；类型与配置实例分别表达 |
| LLM 路由 | 为模型请求选择有权使用的模型及提供商，不选择设备或 Harness |
| 任务 | 描述要持续推进的目标和工作；不据此引入独立父子任务树或子任务界面 |
| 会话 | 与 Harness 执行交互的工作上下文；原生历史由 Harness 拥有，跨 Harness 关联由 Velune 维护 |
| 原生 sub-agent | Harness 创建并拥有的子会话，属于父会话体验，不由 Velune 创建 |
| Agent 间协作／A2A | 普通会话之间的对等感知、消息交流与共同工作，与原生子 Agent 分别呈现 |
| 完整端／控制端 | 完整端可本机执行并控制其他设备；控制端只经远控访问执行能力 |
