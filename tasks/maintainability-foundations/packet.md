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

## 构建空间审计（2026-11-26）

用户发现主检出 `target` 约 208 GiB，要求找出损坏、不一致、低效与不再需要的部分，优先删除，其次简化、优化，最后才自动化。物理占用用文件 extent（`F_LOG2PHYS_EXT`）去重测得，不用 `du`：APFS 克隆会让 `du` 严重高估，构建镜像中统计 83 GiB 的对象删除后卷可用空间只增加约 2.5 GiB。主检出 `target` 实测物理 197.6 GiB，克隆共享仅 2.4 GiB。

| 部分 | 物理 | 结论 |
| --- | --- | --- |
| 用途域以外、无任何跟踪文件引用的 122 个目录（23 日后无改动） | 85.5 GiB | 任务自建 target-dir（`manual-*` 等 18 个 Cargo 目录，逻辑约 72 GiB）与旧证据；待删除 |
| `host-rust` | 66.5 GiB | 11-06 回收后约 4.6 GiB；约 26 GiB 为无引用调试对象，其余为旧变体及增量缓存。按预算整体回收 |
| `cross-rust` | 18.3 GiB | 同上，按预算回收 |
| `checks/archive-loading` 中复制的两份源码树（含 target） | 约 11 GiB | 违反“不复制编译树作证据”；待删除 |
| `release` | 3.9 GiB | 旧 Cargo release 输出，内含已不存在的 `velune_core`；待删除 |
| 主检出 `tmp/runtime-workspace-reliability` | 约 30 GiB | 对应任务包已不存在；待删除 |
| `.dev-cache/velune-*-alpha.*` | 约 1.3 GiB | 11-21 发布时遗留的临时目录；待删除 |

调试对象增长的原因已用最小工程复现：rustc 增量编译加 macOS unpacked split-debuginfo，每次重编以新文件名写出整组 `.rcgu.o`，旧组不再被读取也不被删除；非增量、`debug = 0` 或 packed 均不积累，与 mbx 无关。`debug = 0` 只去掉回溯每帧的行号（panic 位置仍在），单次构建体积不变（velune-application 测试均为 2.0 GiB），因此不改 profile、不增加清理工具，沿用超预算整体回收。packed 的代价另行实测（velune-application 测试，改一行后 `cargo test --no-run`）：unpacked 预热后重编 5–8 秒、每次留下约 130 个调试对象；packed 不留对象，但每次链接都要 dsymutil 汇总调试信息，重编 35–54 秒，同样不采用。曾实现按调试映射删除无引用对象的 `sweep-objects`，按上述优先级撤回。

已改：

- `build-cache.py` 删去历史目录登记与 `--retired`：已退役的目录在本机都已不存在，剩下的 `release` 已失效，`android-native` 是 Gradle 正在读取的现行输出却列为历史目录。`report` 增加用途域以外目录的合计，此前报告只显示 87 GiB 的用途域，另外 120 GiB 不可见。
- `Cargo.toml` 删去与 dev 重复的 `[profile.test]` 和默认的 `incremental = true`；test profile 继承 dev，产物 hash 不变。

用户确认后已删除（卷可用空间 323.2 → 504.9 GiB，增加 181.7 GiB）：

- 上表“待删除”各项：`target` 遗留目录、`release` 与源码副本增加 101.2 GiB，`tmp` 证据与 `.dev-cache` 临时目录增加 26.2 GiB。
- `host-rust` 与 `cross-rust` 整体回收增加 51.4 GiB，低于 extent 统计的 84.8 GiB；extent 去重只覆盖 target 内部，差额应是与其他检出共享的克隆块，未逐一核实；主检出下次 Mac／iOS／Android 构建冷启动。
- 旧分支工作树 `velune-maint`（分支保留）增加 2.9 GiB；另删去 9 个已无内容的旧工作树父目录。
- 主检出 `tmp/`、`target/checks`、`target/verification` 下 11-24 以来无改动、路径与目录名都未被 develop 跟踪文件引用的 55 项，增加 5 GiB。

未改、需用户决定：

- `.cargo/config.toml` 的 `jobs = 2` 没有记录理由（主机 10 核、16 GiB 内存）。
- 主机系统盘只剩约 17 GiB，低于 mbx 的 20 GiB 下限，mbx 因此越过预算持续回收；期间其“targets”回收计数增加 2.8 GiB，同时出现相隔几分钟、无源码变化的 `cargo test --no-run` 重编约 110 个 crate，紧接着的第二次为 0。两者相关，未证实因果。
- `tmp/worktrees/workspace-native-shell` 的分支已合入 develop，但今天仍有提交和 3 处未提交改动，可能仍在使用。

## 工作树

分支检出在 `~/Development/.worktrees/velune/maintainability-foundations`。此前编辑环境未找到该目录，曾在主检出的 `tmp/worktrees` 编辑并同步到这里构建；该嵌套工作树与同步脚本已删除。

## 脚本审计（2026-11-26）

`scripts/` 中 18 个脚本不被 svc、CI、文档或其他脚本引用：17 个 `check-*` 与 `mac_telemetry_build.py`。逐个运行后：

- 10 个合成检查在 macOS 与 Linux 上都通过、不需外部 CLI，覆盖随仓库发布的迁移脚本、构建支持、开发实例与 Pi bridge：check-build-cache、check-build-support、check-claude-history、check-composer-migration、check-runtime-start-policy、check-service-tier-migration、check-macos-dev、check-pi-background-work、check-pi-subagent-metadata、check-runtime-background-tasks。它们没有损坏，只是从未被运行；已加入 PR checks，并在[验证方式](../../docs/development/validation.md)写明。
- check-pi-child-history 与 check-pi-projection-snapshot 需要真实 Pi CLI，传入本机 pi 后通过（11 项）；归真实运行时层，手动运行。
- check-claude-background-follow-up 迁为验收用例 `claude_background_follow_up` 及 `_managed` 变体，实际 Claude Code 2.1.327 下两者各 4 项通过；它依赖的 check-conversation-parity 早已迁为 `codex_conversation_lifecycle`。两个原脚本删除。
- `mac_telemetry_build.py` 的调用方（manual-telemetry 与 otlp-observability 任务验收程序）已于此前删除，一并删除。
- `check-codex-sampling-route.py`、`check-pi-child-host.mjs` 只核对已安装 CLI、不经 Velune，与两项 Pi 探测一起列入[运行时升级](../../docs/development/runtime-upgrades.md#2-用真实运行时复验)；合入 develop 后均通过。
- `check-macos-history-cache.py` 已失效：它直接导入生成的 Python 绑定，而 UniFFI 0.32.2 在带默认 `submission_id` 的记录上生成非法签名，只有验收加载器做了修补。已改写为验收用例 `history_cache_pressure`（7 项检查，约 70 秒），删去 vmmap／sample 采样等诊断输出，原脚本删除。
- 迁移中发现验收运行器在用例结束后才删除临时目录，应用的日志线程仍在写入时删除失败，结果却仍记为通过；改为由父进程在 worker 退出后删除。
- `check-portable-handoff.py` 因同一绑定问题失效，按用户决定改写为验收用例 `codex_pi_handoff`（10 项检查；需 `--codex`、`--pi`、`--node`），原脚本删除。删去自建的绑定加载、fixture 根目录与清理校验、manifest 和开发作用域标记，回环上游的文件闸门与两段流程及全部断言保留；目标请求不得携带加密压缩内容由记录改为断言。runner 增加 `--node`，隔离资源同时链接 `xurl`。回环上游已在本地单独测过；**整例尚未在主机上运行**（主机命令工具本轮断开），需用真实 Codex、Pi 与 Node 复验。
- 尚未处理：`check-macos-sidebar-toolbar.py`（需已启动的开发实例窗口）。

`codex_conversation_lifecycle` 曾因 ChatGPT.app 内置 Codex CLI 自动更新到 0.171.0-alpha.13、超出当时适配器范围而失败；合入 develop（支持至 0.171.0）后通过（8 项检查）。`CODEX166` 是适配器族名而非版本，用例无需调整。PR checks 的新步骤尚未在 CI 实际运行。
