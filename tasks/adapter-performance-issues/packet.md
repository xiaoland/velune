# Adapter 控制观察与会话地址重构

## 2026-11-18 从 dogfood 重新定位

本轮在 `perf/adapter-reexamine` 工作树，以已提交的 `3221b31` 为基线。该基线的 application 库通过编译。原工作树继续承载并行 dogfood 修复。两个工作树的未提交改动归各自工作树所有。

### 观测

经用户授权，聚合了 11 月 18 日北京时间 20:28 至 22:15 的本机滚动诊断日志，共 98,419 条。以下为一次读取的窗口，运行中的安装版本与本分支的对应关系未知。

| 操作 | 窗口中的事实 |
| --- | --- |
| 发送 | 6 次 `bindings.send_turn` 的持续时间为 0.24–5.70 秒，owner 排队 0–14 毫秒。3 次在同一操作编号下记录到原生历史读取。 |
| 列表与打开 | 13 次 `bindings.list` 中位数 5.66 秒，最长 19.82 秒。一次归档列表调用用时 26.46 秒，与一个等待 owner 的请求重叠 26.13 秒。时间重叠提示应检查列表占用 owner 的区段。 |
| 网关 | 358 次请求结束，359 条上游响应头记录中有 357 条 HTTP 200、2 条 HTTP 400。日志中 73,012 条是省略 Codex 请求字段的 INFO。原报告的两条诊断编号均未命中。原始 503 的来源仍未知。 |

### 本分支的修复与验证

- Mac 恢复的列表由设备读取一次，指定会话直接走现有来源解析与打开入口。新建草稿和默认选择等首份配置就绪后再决定。
- Core 的发送准入与视图共用来源、交接、控制权和准入规则。发送准备只刷新所需的消息身份与设置，按需复制恢复执行所需的历史。进入新轮前从同一份 transcript 取缺省设置并关联新请求。
- Codex 网关按一次转换汇总被省略的字段，输出固定字段名与计数。合成请求里 256 个历史项加 3 个顶层字段从 259 条日志降为 1 条，并保持原 wire JSON。
- 新的慢操作日志以 100 毫秒为门槛。`application_command.owner_execute` 与 `owner_queue` 标出命令执行和排队，`application_advance` 记录后台推进，`conversation_list` 给出刷新索引及构造返回页的分段时间。

在 macOS arm64 上，用同一份合成历史和 Cargo 默认 test profile 对比 `3221b31` 与本分支。

| 测量 | 基线 | 本分支 |
| --- | ---: | ---: |
| 成功发送，从公开的 `ApplicationService::execute(SendTurn)` 到返回完整结果，历史含 4 MiB 助手正文 | 465.9 ms（461.4–468.6） | 297.6 ms（297.0–299.3） |
| 同一实验中，发送前首次 `ReadSnapshot` | 343.3 ms（336.6–561.9） | 337.5 ms（332.4–349.2） |
| 同一实验的全进程峰值 RSS | 85.6 MiB（85.5–89.7） | 85.9 MiB（82.0–98.0） |

表中为中位数与范围。两版各预热一次，再交错测量各三次，每次独立进程、私有 HOME、合成 Codex helper。每次都检查只发一次 `turn/start`、历史轮保留原设置、新轮绑定原生消息身份与所选模型、推理级别及权限。成功发送缩短约 168 毫秒，成功路径的 RSS 样本范围重叠。逐次结果保存在本工作树的 `tmp/adapter-performance-issues/send-turn-4m.KPiLQ3TB/results.json`。

另一个实验只经过派发前路径。合成 192 条消息 × 192 KiB，共 36 MiB，因 ACP 不支持所选 managed 模型而拒绝。首次 2,737 → 4 毫秒，随后四次 560–672 → 2 毫秒，全进程峰值 RSS 223,068,160 → 100,040,704 字节。集成后重跑五次为 4–9 毫秒。可用 `synthetic_long_send_measurement --ignored --nocapture` 复跑，峰值取自 macOS `/usr/bin/time -l`。

同一工作树通过 application 库 127 项、公开发送集成检查 1 项、agent-runtime 库 89 项、ai-provider 库 2 项和网关回环检查。上述 36 MiB 测量从默认套件中跳过，已单独运行。

Mac 无窗口检查在私有状态目录中通过 9 项。设备队列在首次列表提交前被人为挂起，两个来源下具有相同原生 ID 的会话均能打开，切换设备后无法借用旧设备的结果。释放队列后只收到一次首列表，默认选择、新草稿、导航与关闭均符合预期。该实验验证客户端请求顺序，真实冷开耗时仍待测。

完整 Mac 开发包仍未构建通过。定稿输入上 `svc run build-macos` 退出 1，构建前后输入摘要相同。`DeviceStore.swift` 中四处既有调用与该基线的接口不匹配，分别涉及项目编辑的 `basis`、窗口的 `canShow`、通知打开的 `openWorkspaceWindow` 和诊断记录的 `record`。无窗口检查使用相同的七个交付 Swift 文件及本次构建的 Rust 库，只在临时副本中适配外围接口，并排除一段无关的编辑器截图检查。具名构建回执、检查输出和临时补丁保存在本工作树的 `target/workspace-cold-evidence-Nw0VIlyl/`。

### 下一次 dogfood 的判断依据

在采用本分支的日常构建中，按操作编号对齐发送、原生历史读取及慢命令时间。列表仍慢时，先看 `refresh_ms` 与 `build_page_ms`。已经在 owner 上执行的慢列表仍可延迟会话打开，冷配置未命中时定向查找也可能读取原生索引。需要验证的是可见工作区恢复、真实发送耗时及长时间运行的 RSS。原始会话的修复状态须待相同操作再次观察。通信端口、完整历史的内存记账及 FFI 所有权改造暂缓，待测量指向对应层再决定。完整历史与原生写入拒绝继续由 Harness 仲裁。

## 11 月 13 日的范围与授权

用户要求从 adapter 与运行时通信的架构层实施长期修复，允许激进调整和本地提交。本切片只修改共享 core、回归检查与设计文档；不推送、不重装日常应用、不访问真实账户、会话正文或故障日志。

用户报告、已确认代码问题和归因边界见 [诊断](diagnosis.md)；职责与后续切片见 [架构](long-term-architecture.md)。未将合成结果解释为原始 incident 已复现或全部性能问题已解决。

## 已实现

- 控制观察从 Session／历史视图移到 `ControlObservations`：完整物理身份去重，四个固定 worker、深度 32 的队列、256 个缓存条目及 15 秒需求租约。
- 同来源最多一个排队／运行探测。失效取消旧预算并增加 generation，但真实工作返回前不释放 flight；迟到结果不能恢复旧可写事实。首次检查和缓存新鲜度有上限，队列饱和或卡顿转为不可确认，不永久 Checking。
- native handle 确认本机持锁时暂停后台观察，避免每次快照取消后再次排队；失去句柄后重新采样。历史投影驱逐不拥有或重建控制观察。
- `ProbeBudget` 覆盖排队、多步 helper、SSH 和进程退出后的 EOF 等待。取消／超时沿 `ProcessTree` 停止本次拥有的进程树并回收直接子进程。不可中断文件 IO 仍占原 worker，不创建替代线程。
- 统一逻辑来源与当前上下文地址：只读接受首段或当前末段，拒绝中间段和无关实例；composer／发送继续校验首段来源，下一轮目标是独立选择。

不改变公开 FFI／DTO 或持久数据格式，不迁移或删除 Harness 历史。

## 验证回执

2026-11-13，在 macOS 主机、当前共享工作树中使用合成数据验证：

| 命令／范围 | 结果 |
| --- | --- |
| `cargo test --locked -p velune-application -p velune-agent-runtime --lib` | 通过：application 136 项、agent-runtime 82 项；加入持锁暂停与 EOF 预算检查后再次通过 |
| `cargo clippy --locked -p velune-application -p velune-agent-runtime --lib -- -D warnings` | 通过 |
| `cargo check --locked -p velune-bindings --no-default-features --features local-runtime` | 通过 |
| `cargo test --locked -p velune-application --lib conversation_address -- --nocapture` | 两项通过，使用稳定序列化合成段引用后再次通过 |
| `cargo clippy --locked -p velune-application -p velune-agent-runtime --all-targets -- -D warnings` | 未通过：既有未提交的 `native_context_deletion.rs` 有九处 `Command::new` 禁用方法及一处 useless vec；未夹带修改该测试 |

重点检查包含：别名 single-flight、不同物理来源隔离、取消保留名额、旧 generation 拒绝、缓存／队列上限、饱和与新鲜度、四 worker 并行、持锁暂停、共享截止时间、Unix 合成 helper 及其 descendant 取消回收，以及 actual Core 的冷读／再次驱逐重载／第三运行时选择。本次提交包含九项新检查；共享工作树另有四项历史缓存检查通过，其历史缓存实现不纳入本提交。不据此推算 CPU、RSS 或延迟改善百分比。

### 最小提交快照

以 `c24043b0d858ec33287d94b41bd5f21e7df201ac` 加本任务暂存改动导出独立源码快照，不使用其它未提交源码。首次编译发现地址 fixture 调用了另一项未提交工作才开放的私有函数；已改为在 fixture 中独立构造 cutoff 摘要，随后编译通过。

| 命令／范围 | 结果 |
| --- | --- |
| 两个包的 `cargo test --locked --lib --no-fail-fast` | agent-runtime 50 通过、1 失败；application 73 通过、1 失败。本次九项新检查全部通过，完整套件未全绿 |
| 不含补丁的相同 HEAD：两个包的库测试 | agent-runtime 48 通过、1 失败；application 67 通过、1 失败，与最小快照相同的两项断言失败 |
| 两个包的 `cargo clippy --locked --lib -- -D warnings` | 通过 |
| `cargo check --locked -p velune-bindings --no-default-features --features local-runtime` | 通过 |

两项在 HEAD 也复现的失败为：`pi_session::tests::notification_receipt_snapshot_observes_preceding_agent_start` 的 `settling old receipts must retain the new run`，以及 `composer::tests::borrowed_input_promotes_without_consuming_new_and_legacy_edits_preserve_content` 的执行主机选择数量断言（实际 0，预期 2）。未夹带这两处的未提交修复。

首次最小 application 整套运行另遇 `runtime_policy_is_one_configuration_commit_and_keeps_old_type_choices` 重开合成状态目录时报锁占用；独立重跑及随后整套重跑均通过，原因未归定，不能据重跑抹去该次失败。

两次独立审查重试仍遇服务端 503，没有独立审查结论。主实现检查了 observer、handoff 与 live RPC 关键路径；这不等于完整 transport 审计通过。

## 剩余通信问题

直接代码检查确认 live RPC 尚有独立于控制观察的边界：

- `native/rpc.rs` 的 `write` 同步写入 stdin；`request_classified`／`claude_control` 在写完后才建立响应期限。application owner 的 `SendTurn` 最终同步进入 `live.send`，Codex 调用 `rpc.request("turn/start", ...)`。对端不读 stdin 可阻塞 owner，不能由本切片的观察预算解除。
- ingress 虽为 64 条有界 channel，等待响应仍把无关记录放入无界 `queued`。持续通知可增加内存，并在随后 poll 中集中投影。下一切片需要协议级 pending request 与异步通知消费分离，不能简单截断或丢弃审批、后台工作和身份确认。
- 不可中断挂载可占住固定观察 worker；当前多执行位置共享 worker，尚无每机器隔舱。关闭只 join metadata scheduler，取消请求不证明所有底层 IO 线程已退出。

未验证 Windows Job Object 实际取消、真实 SSH Harness、GUI 交互、真实账户或负载下的 CPU／RSS／端到端发送延迟。

## 共享工作树与提交边界

任务开始时已有大量未提交修改。使用任务开始时保存的文件基线区分本任务，提交仅收录重构必要文件／hunk；并行产生的 Codex 上下文与后台任务提交保持原样。既有历史缓存、异步刷新、摘要接续、消息删除、四端 UI 与 remote 等工作不因本任务被整体 stage。

最小提交快照的编译、库级 Clippy、本地 bindings 与测试差异已单独核验，结果见上节。未提交的适配 hunk（例如既有 history eviction 测试去除旧 watch 参数）保留在共享工作树，不撤销它们。相关 Markdown 的本地链接／标题锚点及 `git diff --check`、`git diff --cached --check` 均通过。
