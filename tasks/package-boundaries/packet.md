# 独立 package 与平台能力装配

## 目标与授权

讨论共享能力如何分成独立 package，并使平台按需消费。2026-10-05 用户已决定全面采用 UniFFI，并授权实施拆分和 Mac 重建安装。配置持久化为 application 内部模块；不建立空 remote package。产品方向归 [PRD](../../docs/prd/index.md)，候选方案归 [架构](../../docs/design/architecture.md#独立-package-与平台装配)，用户出处归 [S17](../../docs/sources.md#s17)。

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

## 当前实施

采用 ai、ai-provider、conversation、agent-runtime、gateway、application、bindings 七个共享 unit。旧无凭据协作模拟归独立 app/host unit，不混入产品动态库。各 unit 维护自己的 README；公共 Rust API 与生成语言 DTO 分开，领域包不依赖 UniFFI。

application 负责配置聚合、原子提交、Pi 与网关集成和完整用例；gateway 不解释 Harness 认证来源及模型投影；agent-runtime 不依赖 AI／gateway。bindings 暴露具名的类型化操作、明确错误与忙时关闭契约。当前继续使用快照轮询，不为本次迁移新增观察订阅机制。

验收门槛：Rust fmt/check/clippy、无本地运行时构建、Swift 构建及生成 Kotlin 接口检查、生成绑定上的合成会话生命周期。真实登录、提供商调用和体验由用户验收。C# 采用 UniFFI 体系，但第三方绑定兼容性单独记录，不能宣称已有 Windows 产品。

## 跨语言检查发现

生成 Kotlin 后实际编译发现 `close()` 与生成的 AutoCloseable API 重名，错误字段 `message` 与 Throwable 重名。绑定契约改用明确业务操作 `shutdown()` 和错误字段 `detail`，平台对象释放与忙时关闭继续分别表达。

2026-10-05 复核 [NordSecurity C# generator manifest](https://github.com/NordSecurity/uniffi-bindgen-cs/blob/main/bindgen/Cargo.toml)，其当前主分支标注 `0.11.0+v0.31.0`。Velune 使用 UniFFI `0.32.2`，未将第三方工具当作兼容事实；目前没有 C# app，后续接入前须固定兼容工具并生成／编译验证，不恢复手写 C ABI。

## UniFFI 与合成循环检查记录

2026-10-05，UniFFI `0.32.2` 已生成 Swift、Kotlin、Python 接口。Kotlin `2.2.21` 配合 JNA 实际编译生成源码成功；此前发现的 `close`／`message` 命名冲突已通过 `shutdown`／`detail` 修正。`BindingSettingField.kind` 与 `BindingProtocolDescriptor.id` 使用枚举；未知领域字符串在绑定转换边界返回 Contract 错误，平台不以默认控件或协议掩盖错误。bindings 的 `cargo clippy --locked --all-targets --all-features -- -D warnings` 和 `cargo check --locked --no-default-features` 均通过。

手动调用 `scripts/check-pi-uniffi-loop.py`，使用生成 Python 接口、实际固定 Pi SDK、临时 HOME 与回环合成上游，分别跑通 ChatCompletions、Responses 和 Responses 订阅参数约束路径。检查覆盖流式消息、工具工作目录、会话模型恢复、改变模型绑定后的签名清理与工具历史保留、忙时 shutdown 拒绝后继续读取／取消，以及配置重开。脚本没有手写 ctypes ABI 或 JSON 动作派发，不接入 CI。

额外临时提供商导入脚本使用当前 agent-runtime JavaScript 资源、固定 SDK 和实际 Mac 凭据 helper，在隔离目录内创建合成 models.json／auth.json。ChatCompletions 与 Responses 两条 preview→apply→显式配置路由→Pi 发送路径均通过。观察到导入保持空路由与运行时默认模型不变，真正的旧 preview token 在目标配置改变后被拒绝，adapter metadata 在 shutdown／重开后保持，源文件 SHA-256 不变。实际 helper 向回环网关解析合成来源凭据；只比对认证是否正确，不输出凭据值，应用配置不含合成秘密值。

上述行为检查使用开发库及隔离资源装配，尚不等同于最终安装产物检查。真实登录、真实提供商请求与产品体验仍由用户验收；没有读取用户认证或会话，没有访问真实模型服务。

## 最终交付检查

共享 units 与 Mac 源码迁移完成，旧 core 目录、手写 C ABI 及未编译旧 Host/socket 实现已移除。Mac Store／Transport 使用具名 generated records，适配元数据作为不透明字符串原样保留；设置字段及协议描述使用枚举，不以未知值回退到其它控件或协议。

workspace fmt/check/clippy（all targets/all features、warnings denied）、bindings 无默认 feature Clippy，以及 Windows GNU 的 bindings lib 类型检查通过。Mac release 构建和签名校验通过；最终 bundle 的生成 Python 接口经隔离 Pi、合成 loopback 上游完成一次完整循环，包含工具 cwd、路由身份恢复、忙时 shutdown、取消与配置重开。无自动化测试、真实凭据或真实模型调用。Kotlin 生成物实际编译通过；C# 仍按前述第三方工具边界记录，未宣称有 Windows 产品。

本轮源码与构建完成后安装到 Applications；真实产品体验及提供商验收继续由用户执行，当前不将其标记为已通过。

原生 Swift 的手动隔离调用也通过：实际 Transport → 生成 Swift 模块 → Rust application 完成配置保存、昵称编辑及关闭重开，适配器元数据原样保留。该临时脚本与可执行文件留在临时目录，不进入仓库或 CI。
