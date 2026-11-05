# 提交准入的结构收敛

原来的变化成本来自规则分属多处：核心只给出 `can_send`、`can_resume`、`can_steer`，Mac 与 Windows 各自再组合父会话、待答交互、停止和压缩状态决定提交路线，队列命令另维护受理条件。增加一种占用状态时，命令、投影和每个平台都必须理解它；仅补一个 guard 不能消除这项协调。

本轮将这些提交状态关系收进 [`local/submission.rs`](../../packages/application/src/local/submission.rs) 的纯政策。原生 adapter 仍解释执行、原生重试和 steer 能力；application 合并会话事实与一次控制权观察，派生 follow-up 后统一写公开准入。普通发送直接消费本次最终 `can_send`，包括逻辑会话投影失败；原生 snapshot 只用于准备与拒绝理由，不把交付后的布尔值回灌为原生能力。队列、steer、继续资格与压缩投影消费同一状态政策。`FollowUp.can_queue` 补齐对外契约，Mac 和 Windows 删除对应业务组合，仅保留编辑、加载、在途操作与目标选择的本地限制。

## 用同一个行为检验边界

用户在运行中点击排队，但请求在停止完成后才到达。原实现特意接纳这条消息并暂停；若直接用一个“正在运行才可排队”的布尔值统一命令和界面，会丢掉已提交内容。现在政策区分 `offer_queue`（输入区此刻提供排队）与 `accept_queue`（可接纳已发出的请求）：空闲时前者为假，后者保留，现有队列 owner 继续保存停止／失败的暂停决定。停止后的新“发送”仍直接发送，不被改成暂停队列。

现实变化可用新增一种上下文维护占用检验。此前需要同时修改核心命令、Mac 和 Windows 的条件组合；现在执行 owner 提供事实，application 政策决定各动作资格，各端消费已有字段。代价是核心增加一处显式政策和一个 DTO 字段；没有建立公共大状态机、第二份命令账本或新的提交日志。

## 保留的责任与范围

- 模型、推理、权限、附件与执行位置仍按实际目标校验。继续／压缩保留当前上下文运行时约束，侧聊保留独立发送路径，不取得普通队列、插入或继续资格。
- 原生自动重试仍归 adapter，重试等待期间可排队但是否能 steer 取决于其原生阶段；未把自动重试误作用户重新提交。
- 队列接受之后的暂停、等待、调度和错误恢复仍归 `follow_up.rs`，本轮没有重写队列寿命。编辑／fork 的原生边界、定时领取、创建与首发交接也没有统一成一个生命周期。
- iOS／Android 当前只消费 `actions.can_send` 的既有入口直接获得新判定；没有借此次契约增加尚未实现的移动端队列界面。iOS 的 FollowUp fixture 已同步字段。

跨 unit 语义已回填[会话输入准入契约](../../docs/design/conversations.md#会话输入准入与调度)，应用内部顺序见 [README](../../packages/application/README.md)，Windows 验收边界见[对齐记录](../../app/windows/PARITY.md)。

## 证据

新增八项定向测试，覆盖迟到排队、停止请求、原生重试等待、控制权限制、只读子会话、压缩、侧聊和待答交互。其中一项直接运行合成 `CoreRuntime::send_queue(Add)`，检查停止后迟到请求保留队列与暂停而不再向空闲输入区提供排队。

Rust release 窄测 **8 passed、0 failed**，日志为 `target/verification/architecture-review-20261105/structure/submission-tests.log`。首次尝试在并行修改的 `runtime_start` UniFFI 导出返回类型处编译失败，尚未执行测试；该文件 owner 已纠正导出边界后重跑。测试产物曾在 libtest 输出前等待启动，随后自行运行，八项测试耗时 0.02s，未修改系统安全设置。受影响 Mac／iOS 源码 Swift parse、四份文档的 50 个本地链接／标题锚点与 `git diff --check` 均通过；最终 Mac 绑定生成、构建与隐藏检查由 root 统一执行。本组没有启动 Windows／iOS／Android 构建或 UI，没有访问真实数据、重装日常应用或提交。
