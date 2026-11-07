# Velune Agent 指引

本文件是仓库内代码 Agent 的入口，只保留知识导航与跨仓库约束。产品事实、平台细节和任务状态由各自的权威文件维护。

## 按需查阅

- **产品行为与范围**：[`docs/prd/index.md`](docs/prd/index.md)，按其中“能力”索引进入对应分册。
- **核心与平台分工、跨 unit 契约**：[`docs/product-tdd/index.md`](docs/product-tdd/index.md)，索引与主题正文统一放在该目录。
- **界面原则与平台约束**：[`DESIGN.md`](DESIGN.md)；平台实现细节见对应子树的 `AGENTS.md`、README 或 TDD。
- **开发构建复用与缓存**：[`docs/development/builds.md`](docs/development/builds.md)。同工作树用具名SVC入口共享在途构建，按影响层验收并复用最终证据。
- **构建、运行与验证**：[`docs/development.md`](docs/development.md) 和对应 unit 的说明。
- **当前任务、临时证据与未决事项**：任务对应的 `tasks/<name>/packet.md`。
- **需要追溯用户决定时**：[`docs/sources.md`](docs/sources.md)。

## 仓库约束

- 默认从 `develop` 开发；`main` 只通过 PR 合入，不直接推送、强推或覆盖。
- 不得通过 hard-reset／hard-cutoff 丢弃已有用户数据；首次启动自动备份并迁移已知旧格式，格式变化须完成可恢复备份与核验，未知／损坏格式不得被空状态覆盖。现行迁移入口见[数据迁移与恢复](docs/development.md#数据迁移与恢复)。
- Mac 开发实例使用 `svc run start-macos-dev`／`status-macos-dev`／`stop-macos-dev`；不得按名称批量终止 Velune、用裸执行启动普通 GUI 或使用日常状态目录；既有程序化检查入口须用私有合成目录和本次启动的准确子进程句柄。用途、目录与进程控制见[开发实例隔离](docs/development/environment.md#mac-开发实例)。
- 开发验证使用合成数据；真实凭据、账户状态、支付信息和会话记录的处理边界与授权例外见 [`docs/development.md`](docs/development.md)。
- 项目文档和任务包直接用 Markdown 维护。开发资源和构建／重装命令归 `svc.json`，使用 `svc dev` 与 `svc run`；见 [开发环境](docs/development.md#开发环境)。重装日常应用会退出当前 Velune，由用户在应用外执行。
- 修改文档后检查相对链接与标题锚点，并运行 `git diff --check`。

<!-- svc:begin -->
## SVC

Use `svc --help` or `svc <command> --help`.

- `svc status`: inspect project state
- `svc dev`: manage declared development targets

If `AGENTS.local.md` exists, read it after this file. It is ignored local guidance; shared rules belong here.
<!-- svc:end -->
