# 独立 package 与平台能力装配

## 目标与授权

讨论共享能力如何分成独立 package，并使平台按需消费。名称、职责和逐包 ABI 是候选；用户未要求本轮重构。仅进行源码调查、advisor 判断和文档维护，不改产品源码、不替换当前用户待验收安装。产品方向归 [PRD](../../docs/prd/index.md)，候选方案归 [架构](../../docs/design/architecture.md#独立-package-与平台装配候选)，用户出处归 [S17](../../docs/sources.md#s17)。

## 观察与建议

2026-10-05 检查源码提交 d922350：root velune-core 输出 rlib／cdylib，velune-ai 与 velune-ai-provider 已为独立普通 Rust package。core/src/runtime.rs 的 CoreRuntime 同时持有配置、认证、提供商导入、Pi、网关和会话 projection；core/src/conversation.rs 同时定义通用 DTO 与 PiProjection。当前配置以单写锁和 generic-config.json 原子替换保存 gateways／runtimeInstances，Pi 会话不由该配置仓库保存。当前不存在 remote client 或移动 app。

advisor 建议分开领域 package 与 ABI 交付粒度：领域代码独立，按平台能力选择 Rust 依赖，再由薄 FFI 暴露完整操作。逐包 ABI 不是解耦的必要条件；Swift／Kotlin 不应自行承担跨模块失败补偿。remote-only 不依赖本地执行代码，AI 能力是否包含独立选择。配置仓库先服务现有原子配置提交，不预建通用 KV／SQL 存储框架。

此前首循环 packet 仍为已交付产品与用户验收的入口；本任务只是后续架构讨论，不改变其运行状态。guides/delegation.md 当前不存在，已沿用现有设计 advisor 作为稳定 owner，本轮未委派代码实现。

## 尚待收敛

先逐步描述提供商导入／配置提交、本地启动／关闭、远端打开／取消的状态权威和失败处理，再据此确认 package 边界。需特别核对网关与 Harness 集成代码的归属、跨领域配置引用校验、异步资源所有权，以及 remote 不将 cwd 当本地路径的契约。

获准实施后，用无本地执行代码／Pi／Node 的 remote-only 编译组合，以及可独立选入 AI 能力的依赖图作为判别；这只是拟议验证，不声明目前已经构建过该产物。遵守静态检查优先和不新增自动化测试的约束。
