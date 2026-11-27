# 验证方式与实现约束

[返回开发导航](../development.md)

类型和静态检查最先拦住错误，自动化测试保护类型证明不了的行为契约，可重放的验收套件在真实装配上复现用户路径。三者都提交到仓库，并随源码一起维护；任务包只记录这次运行了什么、结果如何，不承载验证手段本身。本方针取代 alpha 0.1 前“不建立成套测试、端到端脚本随任务丢弃”的约定（[S93](../sources/development.md#s93)）。

## 类型优先

内部不变量优先由类型表达：解析后的对象携带所需关联，枚举区分有效状态，穷尽匹配让新增变体触发编译错误。已在配置入口验证的不可变数据不在每次派发时全量重验。lint 用于有明确代码模式和实际维护收益的规则，不以扫描字符串或禁止所有条件分支代替类型设计。文件内容、用户配置、HTTP 与子进程响应仍在接入边界解析；运行时版本、目标存在性和实际执行失败不能预先由编译器证明，这些由测试和验收覆盖。

首个 beta 发布前，破坏性的 API、FFI 与数据变更优先于兼容；没有平台调用的导出删除，不为尚未接入的能力保留契约。持久化结构的切换遵守[数据迁移约定](migrations.md)。

## 测试分层

| 层 | 范围 | 依赖 | 默认运行 |
| --- | --- | --- | --- |
| 单元 | 一个模块的纯逻辑：解析、投影、状态机、规则表 | 无 I/O 或只用临时目录 | 是 |
| 组件 | 一个 crate 的公开入口，经命令／事件驱动 owner，覆盖提交点、失败恢复、迟到结果与重开 | 临时 HOME、合成配置、假 Harness 进程、本机回环上游 | 是 |
| 契约 | 跨 unit 的承诺：命令特性表、持久化格式与迁移、FFI 记录、网关配置快照 | 已提交的 fixture 与期望表 | 是 |
| 真实运行时 | 与已安装 Claude Code／Codex／Pi 等实际 CLI 或 SDK 的协议对齐 | 本机 CLI、SDK 或授权的真实数据 | 否，显式启用 |

默认层必须确定、离线，不读取真实 HOME、凭据或已安装的 Harness，在 Linux CI 与开发机上结果一致。依赖外部程序的测试归真实运行时层，用 `#[ignore = "<原因与启用条件>"]` 标出，由验收入口或开发者显式运行；它们不能是某个行为唯一的保护。

测试守护的是 owner 边界上的契约，而不是复刻一次缺陷的调用路径。修复缺陷时，先判断它违反了哪条契约：契约由类型能表达就改类型；不能表达的，在该 owner 的组件测试中补上能捕获这一类违反的用例。不为单个现象新增专用入口、深度计数或只服务本次修复的检查，也不用局部测试代替结构收敛（[S81](../sources/development.md#s81)）。

### 布局与共享基建

- 单元测试放在被测模块旁的 `<module>/tests.rs` 或 `<module>_tests.rs`，由 `#[cfg(test)] mod tests;` 引入。仅有几行的检查可以内联，生产文件中不保留大段测试代码。
- 组件与契约测试放在 crate 的 `tests/`，只经公开入口驱动。需要私有状态才能观察的契约，说明 owner 缺少可观察的投影，应先补投影，而不是放宽可见性。
- 临时目录、合成运行时实例、假 Harness 进程、事件序列等夹具归共享的测试支持 crate，作为 dev-dependency 引入；不在每个测试文件中重新编写，也不内联假进程脚本。
- 期望值较多的规则用已提交的表或快照表达，例如命令特性的 TSV 表与网关配置快照；变更期望时在同一提交中更新表并说明原因。

## 验收套件

验收套件在实际装配上复现用户路径：构建动态库，按 [bindings unit](../../packages/bindings/README.md#生成绑定) 生成匹配的 Python 绑定，在临时 `VELUNE_HOME` 与临时 Harness HOME 中以合成凭据和本机回环上游驱动实际 application。用例、夹具和入口提交到仓库，可以重复运行；默认使用合成数据，需要实际 Harness 时显式传入已安装 CLI 的绝对路径。真实场景数据的使用遵守[隔离验证与敏感数据](../development.md#隔离验证与敏感数据)。

每次运行把结果清单写在被忽略的 `target/verification/` 下，记录源码身份、启用的用例、所用外部程序版本和逐项结果。任务包引用这份清单与结论，不复制日志，也不在任务目录另写一次性脚本。新增用户路径或修复跨 unit 的行为时，扩充已有用例或新增用例，使下一次变化可以直接复用。

## 结构变化的验收

状态归属、持久化或跨端业务规则发生变化时，验收同时回答行为和维护成本两个问题。先沿一个真实调用说明权威写入者、提交点、失败恢复和消费者，再用一个近期可能发生的变化检查哪些地方需要共同修改。例如 composer 导航规则变化应落在关联规则，增加一个客户端应消费既有核心准入投影。

迁入新责任的同时列出并删除失去用途的旧状态、写入通道和调用者协调。确有仍在使用的消费者时，明确过渡入口及退出条件。只有新机制上线、旧机制继续独立写入，不能据功能回归通过认定收敛完成。

结构调整前，先确认被移动的边界有组件或契约测试覆盖；缺少时先补测试，再改结构，用同一组测试证明行为未变。跨 unit 的新承诺回填对应技术契约，单端内部规则回填该端 README，当前遗留及验证边界留在任务包。

## 静态检查与 CI

Rust 工作区的静态检查与默认测试：

```sh
cargo fmt --all --check
cargo check --locked --workspace --all-targets --all-features
cargo clippy --locked --workspace --all-targets --all-features -- -D warnings
cargo clippy --locked -p velune-bindings --no-default-features --all-targets -- -D warnings
cargo clippy --locked -p velune-bindings --no-default-features --features remote --all-targets -- -D warnings
cargo tree --locked -p velune-bindings --no-default-features --features remote
cargo test --locked --workspace
```

改动 Rust 的 PR 由 [`PR checks`](../../.github/workflows/pr-checks.yml) 在 Linux 上执行上述格式、Clippy 与依赖树检查，并运行默认层测试；`cargo check` 由 Clippy 覆盖。真实运行时层不进入 CI。

`scripts/check-*` 中只用合成数据、不需要外部 CLI 的检查覆盖迁移脚本、构建支持、开发实例和 Pi bridge 等随仓库发布的工具，每个 PR 都在同一 workflow 中运行，清单以 workflow 为准。需要已安装 Codex 或 Pi CLI 的检查属于真实运行时层，清单与用法见[运行时升级](runtime-upgrades.md#2-用真实运行时复验)，升级运行时或改动对应 helper 时手动运行。新增的端到端检查写成[验收用例](../../tests/acceptance/README.md)，不再新增独立的端到端脚本。

`velune-bindings --no-default-features --features remote` 是纯控制端，依赖树不含 application、agent-runtime、gateway 或 ai-provider；完全禁用 feature 时没有本地或远程构造入口。application 自身禁用 `local-runtime` 仅裁掉 Harness，配置校验仍链接 gateway，不是纯远端客户端的装配路径。

根目录 [`clippy.toml`](../../clippy.toml) 禁止直接调用 `Command::new`。子进程经 `velune_agent_runtime::process::command` 创建（ai-provider 不依赖 agent-runtime，使用自己的 `hidden_command`），长期运行的进程树经 agent-runtime 的 `process_tree` 创建：Unix 使用独立进程组，Windows 使用 kill-on-close 的 Job Object 并加 `CREATE_NO_WINDOW`，避免 GUI 宿主为每个子进程弹出控制台。

## 平台与界面

平台代码的第一道检查是各平台构建本身（Swift、Kotlin、C# 均把警告视为错误）。平台中可脱离界面的逻辑放在独立文件，以脚本单独编译检查：`bash scripts/check-token-units.sh` 用同一组样例检查 Mac 与 Windows 的 token 数量格式化与解析；`bash scripts/check-account-quota.sh` 在 `build-macos.sh --build-only` 之后，以实际 Swift 绑定检查 Mac 的额度呈现。

Mac 的程序化检查只编入定义 `VELUNE_DEV_CHECKS` 的开发构建，由 `app/mac/Checks/` 中的分派表按生命周期阶段启动，日常构建不含这些入口。界面证据分为四种覆盖范围，记录时写明属于哪一种：离屏渲染只证明布局与绘制；本地控件调用证明模型与控件连接，不经过事件分发；系统键盘输入经过真实的事件与焦点链；AX 查询证明辅助功能树的结构与顺序。前一种不能代替后一种的结论。Apple 原生控件的系统明暗适配不另设完整验收矩阵，自绘 logo、固定色位图、阴影与边框则按所在背景核对可见性。

资源同步验证使用隔离 HOME 和实际安装的 Harness，确认 Claude Code、Codex、Pi 能识别写入的内容，不调用模型或读取真实配置。定时任务接管使用合成任务，不启用或运行真实任务；真实联调另按数据授权边界执行。界面检查使用隔离的应用实例或 [Mac 预览模式](platforms.md#mac-构建)；Mac 自动化的进程、焦点和前台操作约束见[平台 Agent 指引](../../app/mac/AGENTS.md)。

### Mac 读取恢复复验

用开发包运行仓库内的检查脚本，`--help` 列出入口，`--app` 可指定优化开发包。`background --dark` 检查后台任务的深色状态，`aggregated-list`、`pagination`、`archived` 分别核对搜索、分页和归档。冷打开用 `experience --workspace-only`，提供商用量呈现用 `provider-analytics`。脚本用隔离副本与合成数据复验，结果可通过 `--output` 留存。视觉检查根据生成的截图判断。 [检查脚本](../../scripts/check-macos-composer.py) [Mac 进程与焦点约束](../../app/mac/AGENTS.md)
