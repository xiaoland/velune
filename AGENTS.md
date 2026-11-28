# Velune Agent 指引

本文件是仓库内代码 Agent 的入口，只保留知识导航与跨仓库约束。产品事实、平台细节和任务状态由各自的权威文件维护。

## 按需查阅

- **产品行为与范围**：[`docs/prd/index.md`](docs/prd/index.md)，按其中“能力”索引进入对应分册。
- **核心与平台分工、跨 unit 契约**：[`docs/product-tdd/index.md`](docs/product-tdd/index.md)，索引与主题正文统一放在该目录。
- **界面原则与平台约束**：[`DESIGN.md`](DESIGN.md)；平台实现细节见对应子树的 `AGENTS.md`、README 或 TDD。
- **开发构建复用与缓存**：[`docs/development/builds.md`](docs/development/builds.md)。同工作树用具名SVC入口共享在途构建，按影响层验收并复用最终证据。
- **构建、运行与验证**：[`docs/development.md`](docs/development.md) 和对应 unit 的说明。
- **Android ADB 操作**：[`app/android/AGENTS.md`](app/android/AGENTS.md)，面向 Agent 的设备选择、文本粘贴、排障验收与临时资源恢复。
- **Mac 自动验收与焦点**：[`app/mac/AGENTS.md`](app/mac/AGENTS.md)，开发进程范围、前台操作与视觉证据要求。
- **当前任务、临时证据与未决事项**：任务对应的 `tasks/<name>/packet.md`。
- **需要追溯用户决定时**：[`docs/sources.md`](docs/sources.md)。

## 仓库约束

- 主工作树检出 `develop`，用于集成和串行小改。并行改代码或需要隔离源码的任务，各从本地 `develop` 建任务分支，在 `~/Development/.worktrees/velune/<task>` 创建独立工作树，验收后逐项合回。任务结束时，确认改动和验收记录已保留、工作树无人使用，再移除工作树及已合入的本地任务分支，并删除该任务在主检出 `target/`、`tmp/` 下且任务包未引用的输出；任务不自建 Cargo target-dir，也不把源码或编译树复制成证据（[构建空间](docs/development/builds.md#空间报告与维护)）。
- `main` 只通过 PR 合入，不直接推送、强推或覆盖。
- 不得通过 hard-reset／hard-cutoff 丢弃已有用户数据；首次启动自动备份并迁移已知旧格式，格式变化须完成可恢复备份与核验，未知／损坏格式不得被空状态覆盖。现行迁移入口见[数据迁移与恢复](docs/development.md#数据迁移与恢复)。
- Mac 开发实例使用 `svc run start-macos-dev`／`status-macos-dev`／`stop-macos-dev`；不得按名称批量终止 Velune、用裸执行启动普通 GUI 或使用日常状态目录；既有程序化检查入口须用私有合成目录和本次启动的准确子进程句柄。用途、目录与进程控制见[开发实例隔离](docs/development/environment.md#mac-开发实例)。
- 运行时端到端验收使用经授权的真实场景数据。合成回归、UI 验收及真实凭据与会话记录的处理边界见[隔离验证与敏感数据](docs/development.md#隔离验证与敏感数据)。
- 项目文档和任务包直接用 Markdown 维护。开发资源和构建／重装命令归 `svc.json`，使用 `svc dev` 与 `svc run`；见 [开发环境](docs/development.md#开发环境)。重装日常应用会退出当前 Velune，由用户在应用外执行。
- 修改文档后检查相对链接与标题锚点，并运行 `git diff --check`。

## 文档归属

修改文档前区分产品使用者、人类开发者和代码 Agent，按[贡献指南的受众表](CONTRIBUTING.md#documentation-audiences)选择归属。用户步骤不夹带内部验证或协作指令，工程机制不因用于 Agent 验收就搬入 `AGENTS.md`；PRD/TDD 仍分别维护产品规则和技术契约。来源中的“本轮”“当前切片”和操作授权只属于历史上下文，不能推定为当前任务授权或交付状态。

## 构建与验收协作

多 Agent 各验证自己影响的层；稳定集成后由一个负责人取得共同构建，其它 Agent 复用证据。仅改 UI 先验证对应 App，共享 ABI 改变才编译受影响消费者，不把每个 Agent 的完成都变成一次全平台完整构建。构建请求、输入身份与回执机制见[并行构建](docs/development/builds.md#并行构建与结果来源)。

<!-- svc:begin -->
## SVC

Use `svc --help` or `svc <command> --help`.

- `svc status`: inspect project state
- `svc dev`: manage declared development targets

If `AGENTS.local.md` exists, read it after this file. It is ignored local guidance; shared rules belong here.
<!-- svc:end -->
