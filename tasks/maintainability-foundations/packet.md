# 可维护性地基

## 目标

`refactor/maintainability` 完成了编译告警、fmt／Clippy 清零、PR 门禁与部分测试外移，属于卫生层。本任务处理决定长期变化成本的地基：验证方针、测试基建、验收基建、架构边界与文档归属。用户 2026-11-24 判断旧分支“仅局限在形式上的 DX”，确认 alpha 0.1 后“不建立成套测试”的方针已经过时，授权自主推进与自动提交，并允许用主机命令执行验收（[S93](../../docs/sources/development.md#s93)）。

分支 `refactor/maintainability-foundations` 基于 `develop` 并合入旧分支，旧分支的卫生成果全部保留。

## 起点诊断

以下数字取自合并后的源码，供后续阶段对照，不是验收结论。

| 维度 | 起点 |
| --- | --- |
| 方针 | `validation.md` 规定不建立成套单元／集成测试、临时脚本不入库不进 CI，与仓库已有 25 个 `scripts/check-*`、约 770 个 Rust 测试及 CI 全量测试相矛盾。 |
| 测试基建 | 没有共享 test-support；81 个文件中有 160 处各自建临时目录，15 段手写假运行时脚本内联在测试中；部分测试依赖真实 Pi SDK。application-api、conversation、ai 几乎无测试，Windows／Android 无测试。 |
| 验收基建 | 端到端脚本随任务丢弃，证据散在 `target/verification/<task>`；Mac 有 17 个隐藏的 `--experience-*-only` 入口，组合规则写在启动分派中。 |
| 架构 | `local/editing.rs` 4565 行（其中约 3500 行为内联测试）、`service.rs` 3902 行、`native/codex.rs` 4355 行、`ConversationStore.swift` 3467 行；单函数最长 300 行以上。application 中 7 个文件按 `"codex"`／`"claude"`／`"pi"` 字符串分支，适配器知识泄漏到核心层。`bindings/src/lib.rs` 平铺 163 个公开项。 |
| 文档 | `tasks/` 与 `docs/` 体量相当，任务包中混有 PID、现场恢复等过程记录，部分决定未回填 TDD。 |

## 阶段与验收标准

阶段按依赖排序，后一阶段以前一阶段为安全网。

1. **方针**：重写 [验证方式](../../docs/development/validation.md)，确立分层测试与可重放验收，取代“不建成套测试”。验收：文档链接与锚点有效，来源记录可追溯。
2. **测试基建**：新增共享 test-support（临时 HOME／VELUNE_HOME、合成运行时实例、假 Harness 进程、事件序列 fixture），先迁移重复最多的调用点；真实运行时测试归可选层，默认不跑。验收：`cargo test --workspace` 通过，默认测试不依赖网络或已安装的 CLI。
3. **验收基建**：把以 Python 绑定驱动实际 application 的端到端脚本沉淀为仓库内可重放的验收套件，带统一的隔离目录、结果清单和入口。验收：一个命令在合成数据下复现，结果可机读。
4. **架构收敛**：适配器知识回到 agent-runtime 的类型或描述中，清除 application 的字符串分支；以领域用例为界拆分超大模块，内联测试外移；bindings 按领域分组。每一步先有覆盖该边界的测试再动结构。验收：各步骤的边界测试、Clippy 与受影响平台的构建通过。
5. **收尾**：逐模块清点错误路径上的 `unwrap`／`expect`，回填任务包中的架构决定到 TDD，清理过程叙事。

## 进度

- 合并：`develop` 与 `refactor/maintainability` 有 14 个冲突文件，已按“develop 行为＋旧分支结构”解决。旧分支的 `CommandRoute` 新增 `ListRefresh`，承接 develop 的共享列表刷新；Mac 检查分派、TranscriptTable 与 ConversationStore 的检查代码按旧分支的 `Checks/`、`CheckAccess` 结构移植了 develop 的改动。ConversationStore 检查代码用机械改写移植：先用同一改写从合并基复现旧分支版本（仅一处隐式 self 不同，另行补丁），再作用于 develop 版本。
- 方针：[验证方式](../../docs/development/validation.md)已重写，来源为 S93。
- 测试基建：新增 `packages/test-support`（`velune-test-support`，仅作 dev-dependency）。首批迁移 application、bindings、agent-runtime 中 10 个手写夹具，删去各自的随机目录命名与 `Drop` 清理。
- 验收基建：新增 `tests/acceptance/`，入口 `python3 tests/acceptance/run.py`（`svc run acceptance`）。首个用例 `codex_conversation_lifecycle` 由 `scripts/check-conversation-parity.py` 迁入。
- 架构：`HistoryConfig` 删去与 `kind` 重复的 `provider` 字符串，原生历史读取的分支改为匹配 `RuntimeKind`，消除 `provider != "codex" || kind != Codex166` 一类双重判定；`agents://` 地址解析为 `UriFamily` 枚举，application 中按 `"codex"`／`"claude"`／`"pi"` 字符串的分支改为穷尽匹配。

## 验收现状（2026-11-24）

主机镜像构建：fmt 通过；`clippy --workspace --all-targets --all-features -D warnings` 通过；remote-only bindings 依赖边界通过（981 项，不含本地运行时 crate）。bindings 两个窄特性集的 Clippy 尚未取得明确退出码。`cargo test --workspace`：agent-runtime 290/291、application 269/271，其余全部通过。三项失败：

- `local::submission::tests::finished_parent_work_clears_the_viewed_childs_loading_state`：develop 的 3e939ac37 把 `parent.work_settled()`（无活动连接视为已结算）改为 `work_status() == Settled`，无活动连接的父会话返回 `Untracked`，子会话加载态不再清除。判定为 develop 已有回归，待修正 `parent_finished_child` 对 `Untracked` 的处理并复验。
- `local::organization::pagination_tests::archive_reads_native_pages_and_only_locally_archived_titles`：Codex 归档扫描次数与期望的 4 不符，实际值未取得。测试体与 develop 仅差 `TempRoot` 一行，需对照 develop 基线确认是否为 develop 新增的后台索引预热造成的竞争。
- `client::port::tests::pi_port_pending_and_unconsumed_evidence_reserve_replies_and_count_encoded_bytes`（tests.rs:288）：运行时尚未迁移该文件，原因未查。

Mac、Windows 构建与验收套件实跑均未进行；旧分支 T2 记录的“桌面端调用签名未验证”仍然开放。

工作树中另有未提交、未验证的改动：第二批夹具迁移（agent-runtime 5 个端口／会话测试、question_tests、send_turn）、`RuntimeVariant::command`／`executable()` 取代 `local/mod.rs` 中按 `family_id` 硬编码的可执行发现（DeepSeek 下限统一为变体声明的 0.2.0-rc.2，Mac 只取数字部分，行为不变）、`run.py` 版本行识别修正。

## 待处理

- `discovery.rs`、`local/mod.rs`、`runtime_discovery.rs` 仍按 `family_id` 字符串判断 Pi／Codex，应由运行时描述携带所需能力。
- 其余约 110 处手写临时目录、176 处手动 `remove_dir_all` 与 15 段内联假进程脚本按文件迁移。
- `scripts/check-*` 其余端到端脚本迁入验收套件。
- 超大模块：`local/editing.rs` 的内联测试外移后再按用例拆分；`service.rs`、`native/codex.rs`、`ConversationStore.swift` 的拆分需先确认边界测试。

## 工作树与构建

源码在 `tmp/worktrees/maintainability`（编辑环境只能访问主仓库目录）。嵌套在主仓库内会让 Cargo 将被排除的 third-party crate 归入外层 workspace，因此主机构建在 `~/Development/.worktrees/velune/maintainability-foundations` 进行，构建前由同目录的 `sync-maintainability.sh` 同步源码。该镜像只用于构建和验收，不在其中编辑或提交。
