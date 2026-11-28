# Agent 运行时升级 SOP

外部 Harness 发新版，或自动更新后被 Velune 拒绝时，用这份流程判断哪些版本可以交付。兼容性以 Velune 实际使用的协议、能力和会话数据为准。先实测，再决定是否扩大区间或新增适配器。[开发导航](../development.md#原生运行时与历史读取) · [决策来源 S44](../sources/runtime-collaboration.md#s44)

## 1. 确认升级范围

1. 核对当前实例的运行时类型、执行位置、HOME 和启动入口。在实际执行机器上记录旧版及目标版的绝对路径、完整版本和平台架构。包含桌面应用内置 CLI、SSH 或 Node 入口时，以实例实际启动的程序为准。验收使用可并存的目标二进制和隔离配置。[运行时依赖](runtimes.md)
2. 对照上游发布说明和协议定义，检查 Velune 已经消费的请求、响应、事件及历史格式。区间跨过多个次版本时，核对中间版本的变更和实测证据。专用适配器按 `version.rs` 的区间准入，alpha 和 beta 按对应正式版计入。通用 ACP 以 `initialize` 协商的协议主版本和能力为准。[版本注册表](../../packages/agent-runtime/src/version.rs) · [版本与原生控制](../../packages/agent-runtime/README.md#版本与原生控制)
3. 如果旧区间挡住目标版，在候选代码中临时调整准入，用目标二进制和它实际报告的版本复验。版本越界的报错只能说明准入规则生效。是否兼容由下一步的行为结果判断。

## 2. 用真实运行时复验

兼容回归用独立的 `VELUNE_HOME` 和 Harness HOME、合成凭据及本机回环上游。通过 Velune 的正常入口执行，用匹配的核心库和绑定核对应用收到的结果。运行时端到端验收使用经授权的真实场景。涉及改动历史时，在私有副本中操作并关停重开核验。[验收套件](validation.md#验收套件) · [生成绑定](../../packages/bindings/README.md#生成绑定) · [隔离验证与敏感数据](../development.md#隔离验证与敏感数据)

按适配器原有能力选择下表中的检查。通用 ACP 还要核对界面提供的操作与本次连接协商的能力一致。

经 Velune 的回归先跑[验收用例](../../tests/acceptance/README.md)中声明了该 CLI 的用例。以下脚本不经 Velune，只用合成上游核对已安装 CLI 本身仍符合适配器依赖的行为，适合先判断差异是否来自上游：

| 脚本 | 核对的上游行为 |
| --- | --- |
| `scripts/check-codex-sampling-route.py --codex <路径>` | Codex 持久分叉与采样请求的路由 |
| `scripts/check-pi-child-host.mjs --pi <路径>` | Pi 与 pi-subagents 的前台、嵌套、并行与后台子任务托管 |
| `scripts/check-pi-child-history.py --pi <路径>` | Pi 子任务历史的写出格式 |
| `scripts/check-pi-projection-snapshot.mjs <路径>` | Pi 会话投影快照 |

Pi 历史改写和取消通知另有三个直接调用 Velune 运行资源的检查。在仓库根目录用目标 Pi 和其支持的 Node 执行。删除与编辑会创建临时会话、操作后用官方 SDK 重新打开，取消检查用本地合成对端观察协议终态。

```sh
node packages/agent-runtime/tests/scripts/pi_message_deletion.mjs /absolute/path/to/pi
node packages/agent-runtime/tests/scripts/pi_message_editing.mjs /absolute/path/to/pi
node packages/agent-runtime/tests/scripts/pi_notification_cancellation.mjs /absolute/path/to/pi
```

Rust 中需要已安装 CLI 的导入／读取检查使用 `#[ignore]`，默认测试不执行它们。显式启用后必须提供对应的绝对路径；缺条件会失败，不能作为通过记录。例如：

```sh
VELUNE_TEST_CODEX_BINARY=/absolute/path/to/codex cargo test --locked -p velune-agent-runtime --test native_import_ownership -- --ignored
VELUNE_TEST_CODEX_BINARY=/absolute/path/to/codex cargo test --locked -p velune-agent-runtime --test native_context_deletion installed_codex -- --ignored
VELUNE_TEST_CLAUDE_BINARY=/absolute/path/to/claude cargo test --locked -p velune-agent-runtime --test native_context_deletion installed_claude -- --ignored
```

`pi_background_cold` 和应用层 `real_pi_sdk_cold_core_deletion_preserves_replies_without_model_turns` 的显式检查还需要 `VELUNE_TEST_PI_SYNTHETIC_FIXTURE`、`VELUNE_TEST_PI_BINARY` 和 `VELUNE_TEST_NODE_BINARY`；fixture 必须是经授权的合成会话目录，不能指向日常 Harness HOME。它们克隆 fixture 后分别观察实际 SDK 的冷读取／后台任务和删除历史，不调用模型。默认层继续保留原生历史格式和导入内容的合成检查。提供这些变量后分别执行：

```sh
cargo test --locked -p velune-agent-runtime --test pi_background_cold -- --ignored
cargo test --locked -p velune-application real_pi_sdk_cold_core_deletion_preserves_replies_without_model_turns -- --ignored
```

| 检查项 | 通过依据 |
| --- | --- |
| 发现与启动 | 专用适配器用实际 CLI 版本匹配对应类型。握手后可读取运行时公开的能力。扩大区间时，验证目标版本通过准入，区间外版本得到含实际版本的诊断。 |
| 正常发送 | 新建会话，按运行时支持的方式选择模型，发送一轮并收到完成结果。改动涉及托管路由时，另核对网关实际收到的模型和请求。 |
| 历史与恢复 | 用旧版在隔离目录创建会话，退出后切换到目标版。验证会话仍可定位、历史按正确顺序读出、继续发送后重新打开仍保留上下文和同一会话身份。支持改写历史的适配器按受影响路径加测分叉或回退。 |
| 受影响的其他能力 | 按上游变化和适配器实际声明补测权限、取消、后台任务、会话管理或导入。核对可见结果和协议终态。 |

失败时，用原支持版本在同一场景复验，定位差异发生在版本探测、协议交互、历史读取还是 Velune 的应用行为。隔离环境或上游服务异常时，修正条件后再判定。

## 3. 决定并交付更新

- **实测兼容。** 目标版超出原区间时，扩大现有适配器的兼容区间，同步更新版本说明，保留运行时类型和已有实例标识。Codex 的 `CODEX_LATEST_SUPPORTED_VERSION` 还影响订阅的自动客户端版本，调整时同时核对这一默认值和用户显式设定的版本。[订阅版本取值](../../packages/application/src/config.rs)
- **确认存在破坏性变化。** 新旧版本需要不同协议实现时，新增独立运行时类型与适配器。若涉及已保存实例、会话关联或持久格式的转换，按现行规则交付可恢复备份、迁移和恢复路径，并用旧数据验证。Codex 0.166 的实例与会话关联曾一起迁移，可作为参考。[数据迁移规则](migrations.md#破坏性变更与启动迁移) · [Codex 0.166 实例迁移](migrations.md#codex-0166-实例迁移)
- **证据不足。** 保留现有准入范围，记下缺少的版本、平台或关键场景，以及哪一项结果会改变兼容判断。

用当前源码构建并复验受影响的入口。交付时记录二进制的版本与来源、平台及执行方式、适配器区间或 ACP 握手结果、实测项目和结果，以及会影响使用的验证缺口。涉及迁移时同时记录备份与恢复结果。[开发构建与缓存](builds.md) · Codex 0.167 验证范例（git 历史：tasks/codex-167-range）
