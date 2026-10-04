# 独立 package 与平台能力装配

## 目标与授权

讨论共享能力如何分成独立 package，并使平台按需消费。名称、职责和逐包 ABI 是候选；用户未要求本轮重构。仅进行源码调查、advisor 判断和文档维护，不改产品源码、不替换当前用户待验收安装。产品方向归 [PRD](../../docs/prd/index.md)，候选方案归 [架构](../../docs/design/architecture.md#独立-package-与平台装配候选)，用户出处归 [S17](../../docs/sources.md#s17)。

## 观察与建议

2026-10-05 检查源码提交 d922350：root velune-core 输出 rlib／cdylib，velune-ai 与 velune-ai-provider 已为独立普通 Rust package。core/src/runtime.rs 的 CoreRuntime 同时持有配置、认证、提供商导入、Pi、网关和会话 projection；core/src/conversation.rs 同时定义通用 DTO 与 PiProjection。当前配置以单写锁和 generic-config.json 原子替换保存 gateways／runtimeInstances，Pi 会话不由该配置仓库保存。当前不存在 remote client 或移动 app。

advisor 建议分开领域 package 与 ABI 交付粒度：领域代码独立，按平台能力选择 Rust 依赖，再由薄 FFI 暴露完整操作。逐包 ABI 不是解耦的必要条件；Swift／Kotlin 不应自行承担跨模块失败补偿。remote-only 不依赖本地执行代码，AI 能力是否包含独立选择。配置仓库先服务现有原子配置提交，不预建通用 KV／SQL 存储框架；独立 package 的必要性已按下述反馈复核。

此前首循环 packet 仍为已交付产品与用户验收的入口；本任务只是后续架构讨论，不改变其运行状态。guides/delegation.md 当前不存在，已沿用现有设计 advisor 作为稳定 owner，本轮未委派代码实现。

## 尚待收敛

先逐步描述提供商导入／配置提交、本地启动／关闭、远端打开／取消的状态权威和失败处理，再据此确认 package 边界。需特别核对网关与 Harness 集成代码的归属、跨领域配置引用校验、异步资源所有权，以及 remote 不将 cwd 当本地路径的契约。

获准实施后，用无本地执行代码／Pi／Node 的 remote-only 编译组合，以及可独立选入 AI 能力的依赖图作为判别；这只是拟议验证，不声明目前已经构建过该产物。遵守静态检查优先和不新增自动化测试的约束。

## 配置模块与 unit 修订

用户基本认可职责方向，同意不建立通用存储框架，但质疑配置仓库独立建包，并明确每个 package／app 是一个 unit。沿用设计 advisor 复核后，建议配置持久化先留在共享应用用例所在 unit 的内部模块；多个平台消费同一 SDK，不作为配置仓库有多个直接消费者的证据。

领域负责各自配置类型与规则；应用层负责聚合、跨领域引用校验和提交协调；内部模块负责文件机制；秘密设施仍归平台。不把全部 schema、迁移和事务塞入一个通用 persist 包。若有另一个独立用例需直接消费仓库，或真实编译依赖边界不能用模块解决，再复核建包。package／app 作为 unit 要明确知识、接口和依赖归属，内部模块不必升级为 unit，也不强制 unit 与 ABI 产物一一对应。

已将此修订归位 PRD 和既有架构候选；本轮仅文档变更，不改变已安装的 Mac 版本，不创建空的 unit 规范目录。

## 跨语言接入澄清

用户确认真正目标是 Swift／Kotlin／C# 消费 Rust lib，而非固定使用手写 C ABI。当前 Transport.swift 仍通过 opaque handle 和 JSON 动作调用 Rust；这是当前实现，不是未来接口唯一形态。沿用设计 advisor 后建议先定义 Rust unit 公共 API，再定义平台可用的类型化 SDK／绑定，最后决定链接和打包产物，不让全部内部 package 因工具限制而改写 API。

2026-10-05 复核官方资料：[UniFFI 语言支持](https://mozilla.github.io/uniffi-rs/latest/)、[Swift 生成文件与 Swift 6 边界](https://mozilla.github.io/uniffi-rs/latest/swift/overview.html)、[.NET P/Invoke 源生成](https://learn.microsoft.com/en-us/dotnet/standard/native-interop/pinvoke-source-generation)。UniFFI 提供 Swift／Kotlin 官方绑定；其 Swift 产物仍包含底层 C FFI，文档说明 Swift 6／Sendable 存在支持边界。C# 不能据此声称与官方语言覆盖相同，第三方绑定／.NET 原生接入都需单独确认。没有安装工具、生成绑定或验证这些工具用于 Velune。

拟议隔离判别先用两个实际操作：配置预览的嵌套类型／枚举／可选值／结构化失败，以及会话发送的流／取消／busy close／释放。观察取消与执行取消需分开定义，平台 SDK 不自行补充业务策略。样例不接入 CI、不新增自动化测试；完整重构仍未授权，现有 Mac 安装不变。
