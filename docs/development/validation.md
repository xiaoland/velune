# 验证方式与实现约束

[返回开发导航](../development.md)

先做类型检查和构建。行为验收复用已有检查入口，必要时用临时端到端脚本或手动操作。只为静态检查无法证明的真实不变量保留少量自动化检查。Rust 回归用 `cargo test -p <包>` 在相关平台运行。

内部不变量优先由类型表达：解析后的对象携带所需关联，枚举区分有效状态，穷尽匹配让新增变体触发编译错误。已在配置入口验证的不可变数据不在每次派发时全量重验。lint 用于有明确代码模式和实际维护收益的规则，不以扫描字符串或禁止所有条件分支代替类型设计。文件内容、用户配置、HTTP 与子进程响应仍在接入边界解析；运行时版本、目标存在性和实际执行失败不能预先由编译器证明。

首个 beta 发布前，破坏性的 API、FFI 与数据变更优先于兼容；没有平台调用的导出删除，不为尚未接入的能力保留契约。持久化结构的切换遵守[手动迁移约定](migrations.md)。

## 结构变化的验收

状态归属、持久化或跨端业务规则发生变化时，验收同时回答行为和维护成本两个问题。先沿一个真实调用说明权威写入者、提交点、失败恢复和消费者，再用一个近期可能发生的变化检查哪些地方需要共同修改。例如 composer 导航规则变化应落在关联规则，增加一个客户端应消费既有核心准入投影。

迁入新责任的同时列出并删除失去用途的旧状态、写入通道和调用者协调。确有仍在使用的消费者时，明确过渡入口及退出条件。只有新机制上线、旧机制继续独立写入，不能据功能回归通过认定收敛完成。

能由私有状态、具名类型和穷尽匹配保证的归属优先放进代码。少量不变量检查覆盖身份保持、提交失败以及迟到结果等类型无法证明的关系。原有分层证据继续按影响复用，不因结构审查而重复完整交互矩阵。跨 unit 的新承诺回填对应技术契约，单端内部规则回填该端 README，当前遗留及验证边界留在任务包。

## 验证方式

资源同步验证使用隔离 HOME 和实际安装的 Harness，确认 Claude Code、Codex、Pi 能识别写入的内容，不调用模型或读取真实配置。定时任务接管使用合成任务，不启用或运行真实任务；真实联调另按数据授权边界执行。

Rust 工作区的静态检查：

```sh
cargo fmt --all --check
cargo check --locked --workspace --all-targets --all-features
cargo clippy --locked --workspace --all-targets --all-features -- -D warnings
cargo clippy --locked -p velune-bindings --no-default-features --all-targets -- -D warnings
cargo clippy --locked -p velune-bindings --no-default-features --features remote --all-targets -- -D warnings
cargo tree --locked -p velune-bindings --no-default-features --features remote
```

`velune-bindings --no-default-features --features remote` 是纯控制端，依赖树不含 application、agent-runtime、gateway 或 ai-provider；完全禁用 feature 时没有本地或远程构造入口。application 自身禁用 `local-runtime` 仅裁掉 Harness，配置校验仍链接 gateway，不是纯远端客户端的装配路径。

根目录 [`clippy.toml`](../../clippy.toml) 禁止直接调用 `Command::new`。子进程经 `velune_agent_runtime::process::command` 创建（ai-provider 不依赖 agent-runtime，使用自己的 `hidden_command`），长期运行的进程树经 agent-runtime 的 `process_tree` 创建：Unix 使用独立进程组，Windows 使用 kill-on-close 的 Job Object 并加 `CREATE_NO_WINDOW`，避免 GUI 宿主为每个子进程弹出控制台。

平台代码的检查是各平台构建本身（Swift、Kotlin、C# 均把警告视为错误）。Apple 原生控件的系统明暗适配不另设完整验收矩阵，自绘 logo、固定色位图、阴影与边框则按所在背景核对可见性。平台中可脱离界面的逻辑放在独立文件，以脚本单独编译检查：`bash scripts/check-token-units.sh` 用同一组样例检查 Mac 与 Windows 的 token 数量格式化与解析；`bash scripts/check-account-quota.sh` 在 `build-macos.sh --build-only` 之后，以实际 Swift 绑定检查 Mac 的额度呈现。

临时端到端脚本使用匹配的动态库和 Python 绑定，在独立 `VELUNE_HOME` 与 Harness HOME 中通过回环上游驱动 application。需要实际 Harness 时提供已安装 CLI 的绝对路径。任务包记录版本、范围、结果及数据授权。生成步骤和授权边界见[绑定说明](../../packages/bindings/README.md#生成绑定)与[隔离验证](../development.md#隔离验证与敏感数据)。

### Mac 读取恢复复验

用开发包运行仓库内的检查脚本，`--help` 列出入口，`--app` 可指定优化开发包。`background --dark` 检查后台任务的深色状态，`aggregated-list`、`pagination`、`archived` 分别核对搜索、分页和归档。冷打开用 `experience --workspace-only`，提供商用量呈现用 `provider-analytics`。脚本用隔离副本与合成数据复验，结果可通过 `--output` 留存。视觉检查根据生成的截图判断。 [检查脚本](../../scripts/check-macos-composer.py) [Mac 进程与焦点约束](../../app/mac/AGENTS.md)
