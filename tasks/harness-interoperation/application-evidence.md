# 应用协作执行验收

2026-10-11 检查 application／UniFFI 的子执行器与生命周期。仅使用临时 HOME、临时项目和合成上游，不读取用户原生认证、会话或项目，不调用真实模型。临时脚本位于 ignored `target/collaboration-acceptance/`，未接入自动化测试或 CI。

## 真实 Pi／Codex managed 子执行器

`manual_children.py` 通过实际 UniFFI 动态库创建协作父会话。父端为合成 ACP peer，在收到 prompt 时通过注入的 MCP HTTP 地址和票据调用真实 `spawn`／`send`／`wait`／`close` 工具；子端为实际 Pi 1.0.2 和 Codex 0.159.3，模型服务为本机合成 ChatCompletions／Responses 上游。

两类子运行时均通过：

- 同一个配置实例启动两个并行 managed 子任务，得到不同原生会话身份。
- 首个子任务第二轮发送后保持原生身份；合成上游实际请求包含首轮上下文。
- 六个实际模型请求均保留协作输入来源说明，明确它不是新的人类授权。
- 正常结束返回 `CHILD_MANAGED_ANSWER`，显式 close 关闭每个子任务，再关闭 scope 与 Application。

这同时覆盖独立执行覆盖目录、各子任务网关、managed Pi 新轮次重建连接但恢复原生会话，以及 Native 网关注入释放后的再次发送。父端使用合成 Agent，因此这份证据不单独证明真实模型会自行选择协作工具；真实父 Harness 的工具调用闭环归 [互操作证据](interoperation-evidence.md)。

## 生命周期与审批边界

补充脚本 `manual_boundaries.py` 使用无 list／load／model 选项的合成 ACP 子 peer，经实际 UniFFI、MCP Coordinator 与 child worker 通过以下边界：

- 未授权 target、数量超限与 busy send 被拒绝；父或子忙碌时 shutdown 和替换 scope 被拒绝，scope 存在时配置修改被拒绝。
- `child_snapshot` 的类型转换成功；所有消息具有非零 stamp，连续读取不变消息的 stamp 保持。实现复用已有 `TranscriptDelivery`，没有另一套消息存储。
- 错 turn、错 interaction ID、已完成轮次、取消后和过期后的权限答复被拒绝；合法宿主答复得到 `ACP_PERMISSION`。审批中的 `wait` 即使携带当前 revision 和 30 秒 timeout，也在两秒断言范围内返回。
- 同一个无 load 子进程完成多轮输入；父取消级联至待审批子任务，返回 Cancelled。取消权限使用 ACP 的 cancelled outcome，未将其伪造为允许。
- 子进程 EOF 得到 Failed，保留 `PARTIAL` 正文、失败投影和不可发送状态；后续 send 被拒绝，实际 prompt 记录没有自动重发。
- 250ms 轮次期限到达后返回 TimedOut，清除 pending interaction；同步 initialize 人工延迟一秒期间取消，初始化返回后未提交任何 prompt。
- 保留活跃子任务直接释放应用对象，同时让另一个线程反复尝试打开同一 HOME。竞争者第一次获得目录锁时，所有已记录子进程 PID 均已退出。

消费审批前的 worker 还重新检查 turn、interaction、取消标记与 deadline，覆盖宿主接纳后、命令出队前的状态变化。手动脚本确认了取消／过期后的外部行为；没有声称穷尽调度器的所有线程交错。未额外运行真实提供商认证、真实模型或 App 验收。

补充 `manual_wait_cancel.py` 以实际 MCP HTTP 请求令当前 revision 的 `wait(timeout=30000)` 保持 pending，再从另一线程请求 cancel／status，两者合计在一秒断言范围内返回；等待被新 revision 唤醒，延迟初始化完成后的最终状态为 Cancelled，peer 没有收到 prompt。

源码保持两条关闭不变量：`Scope::drop` 关闭 Coordinator 并 join 子 worker；`CoreRuntime` 按 Rust 字段声明顺序先释放协作与执行对象，最后释放 `Repository` 的独占 HOME 锁。普通 shutdown 仍拒绝忙碌的父／子任务，不将释放语言包装对象等同于用户批准取消。

轮次期限是 worker 的协作式检查，adapter 同步启动与控制调用仍有自己的有界超时，不宣称硬实时截止。取消／过期发生在启动期间时，启动返回后必须阻止首个 prompt；未知发送结果不自动重发。子结果是最终助手文字报告，不代表任务已被独立验证。
