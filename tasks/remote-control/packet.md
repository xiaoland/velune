# 远程控制需求与 Rust packages

2026-10-17 开始。用户要求参考 [ds-harness-remote](https://github.com/liguobao/ds-harness-remote)，共同梳理远程控制需求，厘清架构后实现相应 Rust packages。

## 当前阶段与权限

本轮已完成：Rust packages、隔离端到端验收、必要 Mac 调用方迁移及构建安装。已确认产品承诺归 [PRD](../../docs/prd/remote-access.md)，跨单元契约归 [架构](../../docs/design/architecture.md#远控契约与唯一执行-owner2026-10-17)。新增 `application-api`、`remote`，调整 application／bindings，不新增远控界面、不部署或发布。新契约删除公开 Pi 元数据后，现有 Mac 绑定映射和人工脚本会直接失效，必要调用方迁移由 root 贯通，并按仓库规范重建安装；初始“不构建 Mac”是工作切片设定，不是用户禁止修复必要调用方。

沿用同进程 Rust lib、平台原生 app、UniFFI 和 Harness 持有原生会话的边界。现有 SSH runtime transport 是执行位置，不等同于远程 Velune 控制客户端。不读取真实配置、凭据或会话，不部署、不发布、不调用真实模型；不新增或运行自动化测试。实现阶段采用静态检查、构建和隔离人工端到端验收。

## 目标与完成判据

- 形成可供用户决定的远程使用场景、控制范围、设备与状态归属、连接和授权生命周期。
- 以参考项目的固定提交和本仓库实际实现为依据，确定 Rust package 责任、依赖和平台接入契约。
- 需求厘清后实现已确认切片，完成 fmt/check/clippy、构建及合成数据的跨连接验收；将稳定结论归入 PRD 和架构，不在任务中另建长期权威。

## 当前事实与待决问题

- 工作区开始时干净，分支为 main；本任务不发布远端。
- 现有 `agent-runtime` 和 `application` 支持通过 OpenSSH 执行远端 Harness，远端传输中断会清理它创建的进程组；这不能满足控制设备离线后仍持续执行的承诺。
- 用户已确认：同一个人的多台设备；公网使用，免预配 VPN／端口转发；覆盖完整 Velune 业务能力；GUI 与无 GUI 宿主都需要，远控架构共用核心，不以第一批界面形态缩窄设计。
- 用户纠正：GUI 是宿主装配区别，不是核心远控需求分叉。headless 应用同样直接持有核心，不必恢复桌面 App↔Host 双进程。独立终端／通用文件管理不从完整业务能力自动推出。
- 用户随后确认控制端断线／退出／后台后继续执行、保留审批，其他已配对设备可接续操作；公网最终开箱即用且允许自部署。用户没有承诺退出执行应用后仍运行，机器睡眠／断电／不可达也不是单靠常驻进程能解决的问题。
- 用户已同意 iroh；新增要求是具体参考、对比 ds-harness-remote 的 WebRTC／TURN 稳定性机制，尽可能在无持续数据中继时保持稳定。[固定提交比较](reference.md) 已完成，iroh 默认保留 relay 协助建连后提升直连、IPv4／IPv6 与原生路径观测，不另建应用选路器。loopback 已观察 Direct，显式公共开发 RelayOnly 探针已观察 Relay；均不作为跨公网直连率证明。

### 实施前边界证据（2026-10-17）

- [Application](../../packages/application/src/lib.rs) 直接持有 CoreRuntime；[bindings](../../packages/bindings/src/lib.rs) 的单个 Mutex 串行操作同一实例；[Repository](../../packages/application/src/repository.rs) 用配置根独占锁。远程入口应复用已有执行 owner，不能为每个连接再打开同一配置根。
- [CoreRuntime 的请求入口](../../packages/application/src/local/mod.rs) 在 `route` 中推进活动及 parked 会话、协作，在关闭时检查仍在执行的工作。若承诺无客户端在线仍推进执行，须增加执行端主动推进机制；仅保留进程不构成该承诺的验证。
- [TranscriptCursor 与 SnapshotUpdate](../../packages/application/src/transcript_delivery.rs)、[delivery 实现](../../packages/application/src/local/delivery.rs) 已提供 generation／sequence、增量消息和不匹配时的全量投影；这不是持久事件日志或命令去重机制。
- [公开应用 API](../../packages/application/src/api.rs) 包含配置、认证、资源文件和执行操作，不能整体当作远程授权范围；其中 `read_provider_api_key`、资源内容与显式环境可能包含秘密。网络入口需要具体操作与授权边界，不暴露内部 JSON dispatcher。
- [浏览窗口](../../packages/application/src/local/organization.rs) 的归档视图和分页，以及 CoreRuntime 的默认下一轮运行时等，是当前应用共享状态；多端独立浏览与写入冲突语义尚须厘清，串行 Mutex 本身不解决陈旧审批或重复发送。

## 完成证据与剩余工作

实现分支为 `feat/remote-control`，开始基线 `6b1a543`。主执行者负责集成、workspace manifest／lock、必要 Mac／脚本调用方、权威文档与本地提交；不发布远端。reference owner 完成 [固定上游证据](reference.md)，advisor 完成应用、传输与依赖边界判断。实施前的 [方案推导](design.md) 保留讨论依据，当前契约由各 unit README 与架构拥有。

- application owner 已完成契约提取、唯一业务 owner、独立推进、视图隔离、修订冲突与受控附件；修复人工验收发现的 generic ACP 工作区查询错误依赖原生 agentDir 问题。
- transport owner 已完成 iroh、设备信任、一次性配对／撤销、接纳账本、多流和路径观测；独立合成探针另覆盖乱序请求及慢流不阻塞第二流。
- bindings owner 已完成具名本地／远程接入、原子配置及 revision、显式编辑基线、连接恢复与锁边界。remote-only 依赖树只有 bindings／API／conversation／remote，没有 application／agent-runtime／gateway／ai-provider。
- verification owner 已用冻结动态库及 Python bindings 完成真实 iroh→UniFFI→Application 闭环，[22 项观察全部通过](verification.md)，没有跳过；包含全控制端离线续跑、审批竞争及原生 ID 复用隔离、结果查询、同对象并发、17 MiB 内容传输及超限拒绝。主执行者另完成提供商配置隔离脚本。

已通过 `cargo fmt --all --check`、workspace 全 target／feature 的 locked check 和严格 clippy，以及 bindings 的 remote-only／无 feature 严格 clippy。已构建 debug bindings、release remote 并生成 Swift／Kotlin／Python；Kotlin 仅生成，未编译。所有人工脚本通过 Python 语法解析。

源码已本地提交为 `15399d1`，从干净工作区构建 Mac arm64 release，原子安装至 `/Applications/Velune.app`。正常退出请求未完成后按既有授权结束经 bundle 身份确认的旧 Velune；安装器及安装后签名校验均通过，清单 source_commit 对应该提交且 dirty=false，版本 `0.1 beta.1`。已安装二进制与构建产物 SHA-256 一致，没有启动应用或读取真实资料。

没有读取真实配置、秘密或会话，没有调用真实模型，没有新增或运行自动化测试。未 push、创建 PR 或部署。跨 NAT、真实切网、严格代理、移动后台和正式公网设施仍需现场或运营验收，本次不部署这些服务；平台远控 UI 与这些现场条件不是本轮 SDK 完成证据。
