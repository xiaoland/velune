# 开发与复验

本页按操作导航；构建命令从仓库根目录执行。产品与跨单元契约见[知识导航](index.md)，各端内部设计见平台 README。这里的操作说明不代表已执行或验收，具体版本与结果归对应 Task Packet。

## 隔离验证与敏感数据

开发验证默认使用临时 `VELUNE_HOME`、合成凭据、会话与项目。除当前任务明确授权的操作外，不读取、复制、输出或提交真实登录状态、令牌、API key、支付数据或 Harness 会话记录，也不调用真实计费服务或公网联调。获准进行真实联调时，只操作授权范围内的账户、服务与请求；版本、范围和结果记录在对应 Task Packet。合成验证通过不代表真实账户认证、供应商规则或完整产品体验已通过。

## 验证方式

[验证方式与实现约束](development/validation.md)：类型与静态检查优先、少量不变量测试、平台构建及临时端到端验证。

## 破坏性变更与手动迁移

[手动迁移](development/migrations.md)：升级前的迁移义务、一次性脚本接口、锁与失败处理。原有数据和配置不得用重置代替迁移交付。

### Codex 0.166 实例迁移

[实例迁移步骤](development/migrations.md#codex-0166-实例迁移)：保留实例 ID 与 HOME，同时转换实例类型和会话关联。

## 开发环境

[开发环境与重装](development/environment.md)：SVC 资源、构建／重装入口、开发通道与本机覆盖。日常重装由用户在 Velune 外执行，会退出正在使用的应用。

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

Mac 使用 `OSLog.Logger`，subsystem 为 `local.velune`，系统 Console 可按它筛选。Rust 的 `tracing` subscriber 由客户端遥测 owner 装配，JSON Lines 日志位于 `VELUNE_HOME/logs/velune.<日期>.jsonl`，按 UTC 日期轮转并保留最近 7 个文件；只限制文件数量，没有总字节上限。日志目录在 Unix 上为 0700 权限。领域包只发事件，不创建 subscriber；跨线程显式传播 dispatcher 和 span。

失败信息带诊断编号；用它查找日志中的 `operation_id`，可定位操作、错误代码、阶段、耗时与底层原因。Swift 的本地操作编号另行命名，不是 Rust trace ID。常规 snapshot 和认证轮询不产生成功日志。诊断保存实际错误链、helper stderr 与失败上下文，默认不采集请求、配置或会话正文；显式启用的 OTLP 只导出批准的操作元数据。

网关请求另有 `gateway_request` span 的 `request_id`，与建立执行准备时的 `operation_id` 不同；入口、路由、单次派发与结束事件的含义见 [AI 服务](design/ai-service.md)。运行时 stderr 持续排空并在有界缓存中保留用于失败诊断，超出时明确标记截断。

## 开发遥测

[开发遥测](development/telemetry.md)：隔离启动、平台通道、run manifest、状态检查及分版本查询。

## 整机网络缓冲区诊断

[网络缓冲区诊断](development/network-diagnostics.md)：联网故障的有界采样、隐私边界与归因限制。

### 故障前后的独立记录

[独立记录器](development/network-diagnostics.md#故障前后的独立记录)：显式安装、停止和卸载，记录保留及复发处理。
