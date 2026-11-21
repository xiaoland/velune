# 开发与复验

本页面向开发者，按构建、运行与排障操作导航，命令从仓库根目录执行。安装和普通使用见[用户指南](usage.md)，代码 Agent 的任务操作约束见 [AGENTS.md](../AGENTS.md)。产品与跨单元契约见[知识导航](index.md)，各端内部设计见平台 README。这里的操作说明不代表已执行或验收，具体版本与结果归对应 Task Packet。

## 隔离验证与敏感数据

自动回归和 UI 验收默认使用临时 `VELUNE_HOME`、合成凭据、会话与项目。运行时端到端验收以经授权的真实场景数据为准。涉及历史修改时，在私有副本上完成操作、关闭和重开核验，保留原会话。验收记录只保留经过脱敏的结果。

除当前任务明确授权的操作外，不读取、复制、输出或提交真实登录状态、令牌、API key、支付数据或 Harness 会话记录，也不调用真实计费服务或公网联调。获准进行真实联调时，只操作授权范围内的账户、服务与请求。版本、范围和结果记录在对应 Task Packet。

## 验证方式

[验证方式与实现约束](development/validation.md)：类型与静态检查优先、少量不变量测试、平台构建及临时端到端验证。

<a id="破坏性变更与手动迁移"></a>

## 数据迁移与恢复

[数据迁移](development/migrations.md)规定启动前的自动备份、已知格式转换、未来格式拒写及开发期显式过渡步骤。持久格式变化需交付迁移和恢复路径。

### Codex 0.166 实例迁移

[实例迁移步骤](development/migrations.md#codex-0166-实例迁移)：保留实例 ID 与 HOME，同时转换实例类型和会话关联。

## 开发环境

[开发环境与重装](development/environment.md)：SVC 资源、构建／重装入口、开发通道与本机覆盖。日常重装由用户在 Velune 外执行，会退出正在使用的应用。

## 开发构建与缓存

[开发构建与缓存](development/builds.md)：四端dev／release入口、同工作树请求复用、配套生成、来源记录与空闲缓存维护。

## 平台构建

### Mac 构建

[Mac 构建](development/platforms.md#mac-构建)：工具链、依赖、签名、隔离运行、视觉预览与图标。

### iPad 与 iPhone 构建

[iPad 与 iPhone 构建](development/platforms.md#ipad-与-iphone-构建)：模拟器、设备签名与 remote-only 装配。

### Android 构建

[Android 构建](development/platforms.md#android-构建)：Rust／Kotlin 产物、lint 与隔离验收。

### Windows 构建

[Windows 构建](development/platforms.md#windows-构建)：MSVC、RelayKit DLL、绑定生成、开发机同步与安装。

## 原生运行时与历史读取

[运行时依赖](development/runtimes.md)：外部 CLI／Node 版本、runtime-support 安装、历史 parser 与 SSH 前提。

## 本地配对 CLI

[配对 CLI](development/remote-cli.md)：连接当前应用的本地接口、邀请、批准、连接和撤销。

## 本地诊断

Mac 使用 `OSLog.Logger`，subsystem 为 `local.velune`，系统 Console 可按它筛选。Rust 的 `tracing` subscriber 由客户端遥测 owner 装配，JSON Lines 日志位于 `VELUNE_HOME/logs/velune.*.jsonl`。新分片按递增序号排序，按 UTC 日期或单文件 8 MiB 轮转，最多保留 7 个文件、56 MiB 日志数据；保留清理只删除最旧前缀。旧日期命名文件保留原始内容，随正常保留策略淘汰。日志目录在 Unix 上为 0700 权限。领域包只发事件，不创建 subscriber；跨线程显式传播 dispatcher 和 span。

普通 tracing 只将完整记录交给有界磁盘 worker，不在调用线程写文件。每个 writer 的 formatter、队列和在途记录共用 512 条／2 MiB 保留 payload capacity 预算，单记录上限 256 KiB；这不代表瞬时分配或 RSS 有严格上界。问题新增、恢复和清理等待 write＋flush 回执，不保证 fsync。2 秒未确认的回执是不确定结果，不能自动重放。`telemetry/status.json` 的 `localLog` 提供排队、拒绝／未确认记录、写失败及关闭状态；快照仍须按其 run ID 和新鲜度判断。

同目录的协作 writer 在稳定目录锁内选择最新分片、追加和轮转，读取问题历史也在该锁内取得一致边界。慢盘或较长回放会阻塞追加，可能产生回执超时或明确的队列丢弃；不创建替代 worker 绕过上限。该协议不能逆推出旧日志中已有的跨文件交错，也不协调仍运行的旧版 writer。切换版本时应通过既有实例生命周期正常结束旧 writer，不通过删除日志来伪装修复。

失败信息带诊断编号；用它查找日志中的 `operation_id`，可定位操作、错误代码、阶段、耗时与底层原因。Swift 的本地操作编号另行命名，不是 Rust trace ID。常规 snapshot 和认证轮询不产生成功日志。诊断保存实际错误链、helper stderr 与失败上下文，默认不采集请求、配置或会话正文；显式启用的 OTLP 只导出批准的操作元数据。

诊断界面卡顿时，先按时间对齐 `application_command` 的 `owner_queue`（等待应用线程）与 `owner_execute`（该命令占用应用线程）。冷历史命令可能在投递读取任务后就归还线程，正文读取另看 `runtime_history_read`。`application_advance` 记录轮询和维护占用的时间。`conversation_list` 将全量列表拆成刷新来源的 `refresh_ms` 和生成返回页的 `build_page_ms`，后者包含可见条目按需补读。执行、维护和列表分段在耗时达到 100 毫秒时写入本地日志。

历史读取及关联前缀检查同时记录逻辑会话和原生来源身份、运行时与已知段索引。标准 Claude／Codex UUID、Pi JSONL 文件名中的 UUID 可直接检索；其它标识只记录 SHA-256，定位字段不复制原生绝对路径。前缀失败另带 `prefix_kind`、`failure_kind`、预期／实际消息数及摘要，区分长度不足和内容摘要不符。没有明确段上下文时不猜索引为零；一个操作涉及多个来源时按上述身份和段索引关联，不再仅凭运行时类型要求用户补充会话。

网关请求另有 `gateway_request` span 的 `request_id`，与建立执行准备时的 `operation_id` 不同；入口、路由、单次派发与结束事件的含义见 [AI 服务](product-tdd/ai-service.md)。运行时 stderr 持续排空并在有界缓存中保留用于失败诊断，超出时明确标记截断。

## 开发遥测

[开发遥测](development/telemetry.md)：隔离启动、平台通道、run manifest、状态检查及分版本查询。

## 整机网络缓冲区诊断

[网络缓冲区诊断](development/network-diagnostics.md)：联网故障的有界采样、隐私边界与归因限制。

### 故障前后的独立记录

[独立记录器](development/network-diagnostics.md#故障前后的独立记录)：显式安装、停止和卸载，记录保留及复发处理。
