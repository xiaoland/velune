# Adapter 控制观察与会话地址重构

## 范围与授权

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
