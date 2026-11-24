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
- 第二批夹具迁移：agent-runtime 的 Pi 端口、Claude 端口、后台任务、问题对账夹具，application 的 Codex 问题夹具与 send_turn 集成测试；`TempRoot` 可序列化为路径。
- 架构：`HistoryConfig` 删去与 `kind` 重复的 `provider` 字符串，原生历史读取的分支改为匹配 `RuntimeKind`，消除 `provider != "codex" || kind != Codex166` 一类双重判定；`agents://` 地址解析为 `UriFamily` 枚举，application 中按 `"codex"`／`"claude"`／`"pi"` 字符串的分支改为穷尽匹配；可执行发现的命令与最低版本改由运行时变体声明，消除与版本表的重复（DeepSeek 下限此前已分叉）。

## 验收现状（2026-11-25）

在主机构建镜像中对分支最终源码执行：

- Rust 静态门禁：fmt、`clippy --workspace --all-targets --all-features -D warnings`、bindings 的 `--no-default-features` 与 `--features remote` 两个特性集 Clippy 均通过；remote-only 依赖树不含本地运行时 crate（981 项）。
- `cargo test --workspace`：628 项通过、1 项失败。失败项 `archive_reads_native_pages_and_only_locally_archived_titles` 已由 9f8a4ddf3 修正后单独复跑通过（见下）。
- Mac：`svc run build-macos`（dev，含 `VELUNE_DEV_CHECKS`）通过，Rust 库、Swift、资源与签名用时 807 秒。旧分支 T2 记录的“Mac 桌面端调用签名未验证”由此关闭。程序化检查 `late-send-echo`（develop 新增、经 CheckAccess 移植）与 `outgoing-submissions` 通过；两者不创建可见窗口、不激活应用。后者产生的 18 张截图未作视觉判断。
- 验收套件：`tests/acceptance/run.py --build --codex <ChatGPT.app 内置 codex-cli 0.167.0-alpha.4.1>` 的 `codex_conversation_lifecycle` 通过，8 项检查、3.8 秒，每阶段恰一次上游调用。清单中的 `source.head` 是构建镜像的分离 HEAD（develop），不代表本分支提交；镜像源码由同步脚本从本工作树复制。Homebrew 的 codex 0.159.3 不在适配器范围内。

修正的问题：

- develop 回归（4f7746ce6）：3e939ac37 把父任务完成判定改为 `work_status() == Settled`，无活动连接的父会话变成 `Untracked`，子会话加载态不清除。恢复“无活动连接即无剩余工作”。
- 时序依赖的测试（9f8a4ddf3）：develop 新增的启动期 Codex 活动索引预热与归档视图共用假 Codex 的扫描日志，整份日志断言随交错而变（实测 8～9 行）。改为只核对归档分页。曾验证非规范化临时目录同样失败，排除了 TempRoot 的影响。
- `pi_port_pending_and_unconsumed_evidence_reserve_replies_and_count_encoded_bytes` 在第一次全量运行中失败一次，此后两次运行通过，原因未查，记为偶发。

未验证：Windows 与 Android 构建；Mac 的可见界面、系统键盘与 AX 验收。

构建环境：主机的 `mbx` 编译缓存会清理它认为已脱离检出的 target 内容，本任务的几轮测试因此反复从头编译依赖。

## 待处理

- `discovery.rs` 与 `runtime_discovery.rs` 仍按 `family_id` 字符串判断 Pi／Codex（`local/mod.rs` 的可执行发现已改由 `RuntimeVariant::executable()` 提供），应由运行时描述携带所需能力。
- 验收 runner 的 `--build` 不复用平台构建已有的绑定库，且清单只记录所在检出的 HEAD；应增加内容指纹并允许直接指定已构建的库。
- 其余约 110 处手写临时目录、176 处手动 `remove_dir_all` 与 15 段内联假进程脚本按文件迁移。
- `scripts/check-*` 其余端到端脚本迁入验收套件。
- 超大模块：`local/editing.rs` 的内联测试外移后再按用例拆分；`service.rs`、`native/codex.rs`、`ConversationStore.swift` 的拆分需先确认边界测试。

## 工作树与构建

源码在 `tmp/worktrees/maintainability`（编辑环境只能访问主仓库目录）。嵌套在主仓库内会让 Cargo 将被排除的 third-party crate 归入外层 workspace，因此主机构建在 `~/Development/.worktrees/velune/maintainability-foundations` 进行，构建前由同目录的 `sync-maintainability.sh` 同步源码。该镜像只用于构建和验收，不在其中编辑或提交。
