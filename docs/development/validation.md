# 验证方式与实现约束

[返回开发导航](../development.md)

以类型安全、静态检查和构建为先。行为验收使用手动操作或临时端到端脚本，不建立成套的单元或集成测试，也不把临时脚本接入 CI。自动化测试只为类型与静态检查无法表达的真实不变量少量保留，以 `cargo test -p <包>` 在相关平台上运行。

内部不变量优先由类型表达：解析后的对象携带所需关联，枚举区分有效状态，穷尽匹配让新增变体触发编译错误。已在配置入口验证的不可变数据不在每次派发时全量重验。lint 用于有明确代码模式和实际维护收益的规则，不以扫描字符串或禁止所有条件分支代替类型设计。文件内容、用户配置、HTTP 与子进程响应仍在接入边界解析；运行时版本、目标存在性和实际执行失败不能预先由编译器证明。

首个 beta 发布前，破坏性的 API、FFI 与数据变更优先于兼容；没有平台调用的导出删除，不为尚未接入的能力保留契约。持久化结构的切换遵守[手动迁移约定](migrations.md)。

## 结构变化的验收

状态归属、持久化或跨端业务规则发生变化时，验收同时回答行为和维护成本两个问题。先沿一个真实调用说明权威写入者、提交点、失败恢复和消费者，再用一个近期可能发生的变化检查哪些地方需要共同修改。例如 composer 导航规则变化应落在关联规则，增加一个客户端应消费既有核心准入投影。

迁入新责任的同时列出并删除失去用途的旧状态、写入通道和调用者协调。确有仍在使用的消费者时，明确过渡入口及退出条件。只有新机制上线、旧机制继续独立写入，不能据功能回归通过认定收敛完成。

能由私有状态、具名类型和穷尽匹配保证的归属优先放进代码。少量不变量检查覆盖身份保持、提交失败以及迟到结果等类型无法证明的关系。原有分层证据继续按影响复用，不因结构审查而重复完整交互矩阵。跨 unit 的新承诺回填对应技术契约，单端内部规则回填该端 README，当前遗留及验证边界留在任务包。

## 验证方式

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

平台代码的检查是各平台构建本身（Swift、Kotlin、C# 均把警告视为错误）。平台中可脱离界面的逻辑放在独立文件，以脚本单独编译检查：`bash scripts/check-token-units.sh` 用同一组样例检查 Mac 与 Windows 的 token 数量格式化与解析；`bash scripts/check-account-quota.sh` 在 `build-macos.sh --build-only` 之后，以实际 Swift 绑定检查 Mac 的额度呈现。

端到端行为用临时脚本验收：先构建动态库，并按 [bindings unit](../../packages/bindings/README.md#生成绑定) 生成匹配的 Python 绑定，再在临时 `VELUNE_HOME` 与临时 Harness HOME 中，以合成凭据和本机回环上游驱动实际 application。需要实际 Harness 时传入已安装 CLI 的绝对路径。脚本随任务使用，不提交到仓库；所用版本、覆盖范围与结果记录在该任务的 Task Packet。验证前先遵守[隔离验证与敏感数据边界](../development.md#隔离验证与敏感数据)。界面检查使用隔离的应用实例或 [Mac 预览模式](platforms.md#mac-构建)。
