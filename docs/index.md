# Velune 文档导航

使用产品从[安装与开始使用](usage.md)进入，参与开发从[开发说明](development.md)进入。PRD 是产品需求规范，TDD 是技术契约，均不作为用户操作手册；代码 Agent 的工作指令归 [AGENTS.md](../AGENTS.md)。

## 从哪里开始

| 需要了解什么 | 入口 |
| --- | --- |
| 安装、添加运行时、手机配对与 iPad 安装 | [安装与开始使用](usage.md) |
| 产品目的、范围与术语 | [产品定义](prd/index.md)、[范围与术语](prd/scope.md) |
| 某项功能应该怎样工作 | [能力索引](prd/index.md#能力)，直接进入行为分册 |
| 核心、平台与远控怎样协作 | [技术契约索引](product-tdd/index.md)，按问题进入设计 |
| 界面原则及平台惯例 | [DESIGN](../DESIGN.md)，具体平台见下表 |
| 构建、运行、验证、迁移与排障 | [开发与复验](development.md) |
| 代码 Agent 的操作范围与平台约束 | [Agent 指引](../AGENTS.md)及对应子树的 `AGENTS.md` |
| 用户为什么作出某个决定 | [来源索引](sources.md)，按主题或 S 编号追溯 |
| 仍有哪些未决或未验证项 | [待决事项与已知缺口](../tasks/open-items/packet.md) |

## 实现与验证位置

| 范围 | 入口 |
| --- | --- |
| 共享 Rust packages | [架构职责表](product-tdd/architecture.md#独立-package-与平台装配)，再进入对应 package README |
| macOS | [Mac README](../app/mac/README.md) |
| Windows | [Windows README](../app/windows/README.md) |
| iPad／iPhone | [iOS README](../app/ios/README.md) |
| Android | [Android README](../app/android/README.md) |
| 当前任务 | [tasks/](../tasks/) 中相应任务的 `packet.md` |

各平台 README 面向开发者，说明实现和验证边界；任务记录保存当次交付证据，来源记录保留历史决定。

文档修改的归属和引用约定见[贡献指南](../CONTRIBUTING.md#documentation-audiences)。

## 开发工具

以下 SVC 接入块由工具维护，供开发者与代码 Agent 查阅。

<!-- svc:begin navigation sha256=55c5c21e0087021bc7f81047e2ffe91f889b81829473655d86d78fbe2029d4f2 -->
## SVC CLI

Use `svc --help` and the installed SVC Skills for framework guidance. Project documentation outside this marked block remains Consumer-owned.
<!-- svc:end navigation -->
