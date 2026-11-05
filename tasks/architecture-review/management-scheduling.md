# 管理、问题历史、调度与类型编辑审查

> 后续结构复核已退役 Mac 的 DeviceStore 正文保存队列。下述删除／退出检查记录上一轮局部修复的证据，该旧机制不再作为当前输入持久化方案，现行结果见[结构复核](structure.md)。

本轮确认并修复两项生命周期错误：删除失败前丢弃待写草稿，以及任务执行期间遗漏下一次触发。其余结论来自实际代码路径与既有合成证据复核；没有把产品范围扩大为清理历史会话选项或跨文件原子事务。

## 已落地能力与责任

| 能力 | 实际责任与实现 | 现有边界 |
| --- | --- | --- |
| 会话管理队列 | Mac `DeviceStore` 受理意图并在设备串行队列执行；`ConversationStore` 只提交已捕获的会话引用，完成后更新设备列表／广播名称／通知删除，不依赖发起 tab 的 navigation generation。批量删除逐项保留成功和失败。 | Windows 尚未移植同一管理请求队列；既有 PARITY 已明确。 |
| 问题跨重启 | Mac 把新增、恢复、显式清理交给 telemetry 的既有 rolling JSONL writer；面板读回同一日志，恢复保留历史，显式清理删除投影；损坏单行独立跳过，存储自身失败只在面板呈现。 | 保留沿用 7 个日志文件。Windows 仍为本次运行的内存问题表，并非已完成跨重启。 |
| 本地调度 | application 生命周期承担调度，SQLite owner 保存定义／revision／运行账本；claim 事务先写触发，再派发到既有会话池、控制权准入、网关与审批。重启跳过 Waiting，对 Dispatching／Running 标明结果待核查，不自动重发。 | 退出无独立后台宿主；错过触发不补跑；Windows 尚无任务管理 UI。 |
| 来源交接 | 只读解析选定 automation.toml、以 canonical source path 与指纹复用导入项；始终暂停导入；内容／来源宿主停用／已知数据库 IN_PROGRESS 状态复核，再由用户确认交接。 | 不具备两宿主共享原子租约；来源重新启用的竞争边界已在 PRD 说明；不操作真实来源。 |
| Adapter Type 表单 | Mac 按类型暂存 settings、ACP launch、新建默认；executionReady 前禁用类型切换；异步保存捕获提交时 type 与新建默认。Windows 在 await 前捕获 payload，各类型的 settings／starts 分开，ACP launch 只对 ACP 提交。 | 普通类型编辑不改写历史归属。已有 UI 窄验、实际在途快照与新类型后续执行证据复用；本轮不跑 Windows UI。 |

以上责任与 [调度 PRD](../../docs/prd/scheduled-tasks.md)、[类型编辑 PRD](../../docs/prd/agent-runtimes.md#编辑-adapter-type)、[本地问题历史](../../docs/design/telemetry.md#本地问题历史) 一致。历史任务证据见 [会话控制与调度](../session-control-and-scheduling/packet.md)、[会话管理与恢复](../reliability-and-codex-166/packet.md)、[类型编辑](../adapter-type-editing/packet.md)。

## 修复及反例

### 删除失败必须保留待写草稿

原实现受理删除时立即 `dropUnsavedDraft`，native 删除失败后，屏幕上的文字虽仍在内存，待持久化的最新值已被删除；下一次重开可能只读到旧值。单纯把清理延后也不够：删除进行中定时保存可能排到 native 删除后，重新创建已经删除的输入。

现在复用管理请求本身的身份，标出删除请求；`saveDrafts` 暂留仍在删除中的目标，其余草稿照常落盘。成功后由 `noteDeleted` 清理输入；失败结算后补写保留的最新草稿。相同目标有多个请求时，直到最后一项结算才放行；成功删除后的迟到 view 写入被拒绝。退出先经过既有设备队列到 main 的屏障，让已完成操作的回包及补写草稿入队，再排 `close`，避免删除失败回包晚于核心关闭。Transport 的 closing 状态仅拒绝 `.reads`，草稿的 `.writes` 仍允许收尾。没有额外建立任务队列或第二份草稿存储。

改变位置：`app/mac/DeviceStore.swift`、`app/mac/ConversationStore.swift` 的删除调用；未覆盖其他 Agent 在同文件中的 composer／fork 改动。

验证：从实际 DeviceStore 抽取管理请求、草稿写入和删除确认方法，以合成 Transport 与串行执行器编译 Swift 窄验，三项通过：删除失败后最新草稿重新落盘；成功后不重建且拒绝迟到写入；同目标重叠请求结算前不抢先写入。源码、二进制及日志在 `/tmp/velune-architecture-management/delete-draft-check*`。另以 Swift 解释器执行真实 `shutdown` 方法前后反例，确认延迟 main 回包时，旧方法为 `begin-closing → close → rejected-write`，新方法为 `begin-closing → write → close`，断言通过。日志为同目录 `delete-shutdown-original-interpreter.log`、`delete-shutdown-fixed-interpreter.log`。解释器复用了与已编译窄验相同的源码，绕开本机新 Mach-O 在 `_dyld_start` 的加载迟滞；冗余进程已结束。这是方法边界的行为证据，不是本轮原生窗口验收。

### 执行中的任务应保留一份后续触发

原 sweep 把 Waiting、Dispatching、Running 一并视为“已有待执行”，只前移 next_run_at。若上一轮一直 Running，期间所有到期触发均被跳过，与 PRD 的“一份待执行触发”不同。

现在账本 owner `Store.claim_due` 区分正在执行的触发和后续等待触发。已经执行的触发可留一份未来触发；已有未来等待时，仅前移计划游标。多目录属于同一 due 的多个 slot，剩余 slot 不会误被当成未来触发；派发仍由原 session／任务运行检查串行控制。没有新增执行器或调度状态库。

改变位置：`packages/application/src/scheduled_tasks.rs`、`packages/application/src/local/scheduled_tasks.rs`。实际 release 定向测试 8 项通过，含两项新增临时 SQLite 存储回归：多目录上一轮运行时保留一份未来触发；再到期不累积；上一轮完成后可保留新的下一份；单纯等待会话可用时不积压。已有日历／时区、无解规则、revision、暂停与重启不重放条件同时通过。测试日志：`/tmp/velune-architecture-management/scheduled-tasks-test.log`。新测试进程经历首次加载迟滞后实际运行完成，未将启动等待冒充通过。

## 类型默认的明确边界

现有表单保存运行时后，另写 `RuntimeComposerChoices`，前者负责实例及执行环境，后者负责 LastChosen／新建默认；核心通用 upsert 不承诺同时重写这份独立偏好。UI 已捕获两步所用的同一提交快照，但这不等于跨文件原子事务。未把此差异当成已证实需要清空所有会话显式选择的错误。

本轮撤回了候选的 shared target／多视图历史选择清理，未保留该代码。原交付明确保留正文、附件与历史目标选择，S80 只要求表单按类型隔离及提交所选类型的新建默认。若将来需要强化核心默认归属，应让默认记录表达所属 Adapter Type，并在该记录的读写边界解决；不能借普通 upsert 清掉历史输入或建立类型升级关系。

## 未扩大验收

保留 `app/mac/ScheduledTaskImport.swift` 原有脏改动。本轮没有读取真实会话／凭据、迁移真实配置、退出或安装日常应用、激活前台窗口、注入鼠标键盘、执行真实模型请求、运行 Windows UI 或提交。最终完整 Mac 构建由主 Agent 统一集成；上述方法／SQLite 证据只支持相应边界，不替代未取得的跨平台运行验收。
