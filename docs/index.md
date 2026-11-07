# Velune 文档导航

按要解决的问题进入对应主题，不必从头加载全部文档。PRD 维护当前产品规则，设计文档维护实现契约，来源记录保留历史决定；任务计划、临时证据和未决项不混入这三类文档。

## 从哪里开始

| 需要了解什么 | 入口 |
| --- | --- |
| 产品目的、范围与术语 | [产品定义](prd/index.md)、[范围与术语](prd/scope.md) |
| 某项功能应该怎样工作 | [能力索引](prd/index.md#能力)，直接进入行为分册 |
| 核心、平台与远控怎样协作 | [技术契约索引](product-tdd/index.md)，按问题进入设计 |
| 界面原则及平台惯例 | [DESIGN](../DESIGN.md)，具体平台见下表 |
| 构建、运行、验证、迁移与排障 | [开发与复验](development.md) |
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
| 当前任务 | [tasks/](../tasks/) 中相应任务的 `packet.md`；完成后删除，历史由 git 保留 |

产品目标、设计契约与交付证据分别阅读。来源中的“本轮”“当前切片”和操作授权只属于当时上下文，不能推定今天仍未实现或已获新任务授权。各平台的实现和验证边界由其 README 与任务记录说明。

## 维护约定

每条规则只在一个主题维护，其他页链接引用。`prd/` 收录产品行为，`product-tdd/` 将跨 unit 契约的索引和正文放在一起；单个 unit 的内部设计归其 README／TDD。按独立用途拆分，不把同一主题机械切成编号文件；入口只提供定位和必要边界。修改规则时合并已被明确取代的表述，保留来源链接；无法确定是否取代的差异留在任务中核实，不让读者自行按日期猜测。

[仓库 README](../README.md)面向产品使用者，[Agent 指引](../AGENTS.md)维护仓库工作约束。Beluna 是独立姐妹项目，不与本目录混用。

<!-- svc:begin navigation sha256=55c5c21e0087021bc7f81047e2ffe91f889b81829473655d86d78fbe2029d4f2 -->
## SVC CLI

Use `svc --help` and the installed SVC Skills for framework guidance. Project documentation outside this marked block remains Consumer-owned.
<!-- svc:end navigation -->
