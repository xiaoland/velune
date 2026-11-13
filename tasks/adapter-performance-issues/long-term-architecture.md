# Adapter 通信架构调整

## 职责与目标

保留 application 的唯一状态 owner。adapter 解释原生协议、锁与持久历史，application 决定观察需求、缓存寿命、准入和逻辑来源。目标是让观察 IO 不占住 owner，且超时后真实资源仍有明确所有者；不是把每个调用机械地换成 `async fn`。

本文件替代早期的示意草案。草案中的无界 channel、超时后重建任务、缩减身份键和未经实测的性能表不作为实现依据。当前证据与未决项见 [任务包](packet.md)。

## 第一切片：共享控制观察

```text
Session / snapshot / admission
             │  cache access + demand lease
             ▼
  ControlObservations (bounded metadata)
             │  single-flight per complete Target
             ▼
  scheduler → bounded queue → four fixed workers
                                   │
                                   ▼
                     versioned native control adapter
                                   │
                    shared ProbeBudget / ProcessTree
```

缓存键使用完整 `Target`：执行机器、HOME、版本类型、原生目录、native ID、Node 与资源路径。相同原生 ID 在不同目录或机器上不合并；逻辑别名不创建第二份物理观察。历史投影与观察寿命分离，驱逐正文不重置探测。

一个 scheduler 管理四个固定 worker、深度 32 的队列和最多 256 个缓存条目。查询只续租 15 秒需求并读取内存状态；scheduler 独立推进。正常结果约 2 秒后刷新，未知结果 5 秒后重试。首次等待最多 5 秒，正常事实在刷新时点后最多再保留 5 秒；过期可写事实改为不可确认，而不是继续允许发送。

本机 native handle 已确认持锁时暂停后台观察，避免每次快照取消后又排入探测；失去句柄后的查询恢复采样，不沿用持锁前的事实。

同一目标最多一个 queued/running flight。失效增加 generation 并请求取消，但不移除 flight；只有真实执行返回才释放名额。原生写拒绝优先于旧探测，generation 和 flight ID 共同拒绝迟到结果。队列满、取消或文件卡顿不能靠创建替代线程绕过资源上限。

调度服务关闭时取消全部预算并 join metadata scheduler。可能停在不可中断 filesystem syscall 的 worker 不同步 join，仍由原请求持有；该边界是有界退化，不是已证明所有线程即时退出。多个执行位置目前共用 worker，尚未提供每机器的隔舱或断路器。

## 原生探测取消

`ProbeBudget` 的截止时间从排入观察队列开始，覆盖 helper 启动及多步操作。DeepSeek 的定位和 SSH 锁查询共享预算，不在每一步重新获得五秒。取消／超时经现有 `ProcessTree` 停止拥有的进程组或 Job Object，并回收直接子进程；进程退出后的 stdout／stderr 等待继续检查同一预算，不另取观察期限。不按名称杀进程。

本地文件打开仍可能阻塞。换成 Tokio 文件接口也不能保证取消底层阻塞操作。未来若需要抵御不可中断挂载，应把该类探测隔离到可回收的进程／设备边界，而非无界扩充 blocking pool。

不以 filesystem watcher 代替锁事实：`flock` 取得或释放未必改变文件内容或 mtime。事件只能提前触发重采样，不能证明排他权。原生写操作仍是最终仲裁。

## 第二切片：统一地址而不混淆发送目标

逻辑会话首段是来源，末段是当前上下文。只读入口归一化这两个合法地址，拒绝中间段和无关 runtime；composer 和写操作继续以来源寻址。下一轮执行选择是独立字段，可以指向第三运行时，但不能用它验证来源，也不能直接用另一个 Harness 的 native ID 恢复。

## 后续通信切片

完整 live transport 还需按实际代码审计拆开：

1. 把请求写入、协议响应等待和异步通知消费的 owner 明确分开；预算必须覆盖写入，不能等 stdin 写完才开始计时。
2. 有界 ingress 不能被等待响应时的旁路缓存抵消。通知必须继续进入投影／工作状态，不能丢弃审批、后台任务或原生身份确认。
3. 背压按队列和字节所有权定义，完整业务历史不通过任意截断“优化”。历史冷读继续由已有 `HistoryWorkers` 负责。
4. 连接在 turn、交互、后台任务与最终元数据均已结算后才可回收；不能只因父 turn 结束就关闭。

这些是后续设计边界，不是本切片已交付的统一异步 live RPC。SSH master 复用和进程表缓存应在耗时证据支持后单独实施，复用键必须包含真实执行位置与 HOME，不使用 family 或 native ID 代替。

## 验收标准

以少量资源不变量检查为主：同来源 single-flight、不同来源隔离、迟到结果拒绝、队列与缓存有界、饱和后不永久 Checking、过期事实不可写、四 worker 并行、取消回收进程树、跨步骤共享截止时间，以及真实 Core 的合成逻辑历史重载／目标选择。CPU／RSS／端到端发送改善需要单独负载测量，不由这些测试推算百分比。
