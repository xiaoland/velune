# 实施与验收

用户已授权落地，范围与边界见 [packet](packet.md)，职责与组合判据见 [design](design.md)。

1. 统一完整快照的执行关联。Codex completed／failed／interrupted 分别确认成功／失败／取消；ACP、DSH 的 EndTurn 确认成功、Cancelled 确认取消、错误确认失败，其余限制／拒绝保持非成功；Claude 以 result 加 idle 确认，取消请求本身不能覆盖正常完成；Pi 保留 stop／error／aborted，settled 仅结束忙碌。连接失败仅终止当前活动。同步迁移历史、续接、身份确认及 transcript，删除 last-user 补记；主动压缩独立分组。
2. Codex 将计时、起轮待答、停止和终态集中在当前活动恢复状态；异步起轮／中断，固定配置和预算。用合成应答屏障检验轮询、停止和消息不重放，以及乱序、断线、耗尽后显式继续。
3. Mac 按窗口布局显式配对输入 owner 与目标，所有命令及发送消费同一上下文；以语义修订通知保存独立草稿。隔离应用检查隐藏输入区改选、聚焦切换、sheet、异步回包及恢复；保留现有附件粘贴。
4. 更新权威文档；运行相关 Rust 测试、严格 clippy、bindings 与 Mac 构建、文档链接和 diff 检查。开发应用使用临时 HOME／独立 bundle，在 /tmp 后台运行；不重装当前应用。

进度：四项已完成；45 项当前 Rust 测试、workspace 严格 clippy、当前源码的 SVC Mac 构建、隔离应用组合／实际进程恢复及文档检查均通过。具体证据与限制见 packet；不以修复前的绿色检查代替本轮组合路径证据。
