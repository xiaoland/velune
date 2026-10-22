# Velune Agent 指引

本文件是仓库内代码 Agent 的入口，只保留知识导航与跨仓库约束。产品事实、平台细节和任务状态由各自的权威文件维护。

## 按需查阅

- **产品行为与范围**：[`docs/prd/index.md`](docs/prd/index.md)，按其中“能力”索引进入对应分册。
- **核心与平台分工、跨 unit 契约**：[`docs/product-tdd/index.md`](docs/product-tdd/index.md)；跨单元方案见 [`docs/design/`](docs/design/)。
- **界面原则与平台约束**：[`DESIGN.md`](DESIGN.md)；平台实现细节见对应子树的 `AGENTS.md`、README 或 TDD。
- **构建、运行与验证**：[`docs/development.md`](docs/development.md) 和对应 unit 的说明。
- **当前任务、临时证据与未决事项**：任务对应的 `tasks/<name>/packet.md`。
- **需要追溯用户决定时**：[`docs/sources.md`](docs/sources.md)。

## 仓库约束

- 开发验证使用合成数据；真实凭据、账户状态、支付信息和会话记录的处理边界与授权例外见 [`docs/development.md`](docs/development.md)。
- 项目文档和任务包直接用 Markdown 维护；不要引入 SVC CLI、运行时或额外工程。
- 修改文档后检查相对链接与标题锚点，并运行 `git diff --check`。
