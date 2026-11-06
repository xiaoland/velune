# Velune Agent 指引

本文件是仓库内代码 Agent 的入口，只保留知识导航与跨仓库约束。产品事实、平台细节和任务状态由各自的权威文件维护。

## 按需查阅

- **产品行为与范围**：[`docs/prd/index.md`](docs/prd/index.md)，按其中“能力”索引进入对应分册。
- **核心与平台分工、跨 unit 契约**：[`docs/product-tdd/index.md`](docs/product-tdd/index.md)；跨单元方案见 [`docs/design/`](docs/design/)。
- **界面原则与平台约束**：[`DESIGN.md`](DESIGN.md)；平台实现细节见对应子树的 `AGENTS.md`、README 或 TDD。
- **开发构建复用与缓存**：[`docs/development/builds.md`](docs/development/builds.md)。同工作树用具名SVC入口共享在途构建，按影响层验收并复用最终证据。
- **构建、运行与验证**：[`docs/development.md`](docs/development.md) 和对应 unit 的说明。
- **当前任务、临时证据与未决事项**：任务对应的 `tasks/<name>/packet.md`。
- **需要追溯用户决定时**：[`docs/sources.md`](docs/sources.md)。

## 仓库约束

- 允许 hard-reset／hard-cutoff，但涉及已有数据或配置时必须完成显式手动迁移。可由 Agent 手工处理或显式执行一次性脚本，不在应用中建立自动迁移机制；见[破坏性变更与手动迁移](docs/development.md#破坏性变更与手动迁移)。
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
