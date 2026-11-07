# 多 Harness 路由与协作架构

Velune 是外部 Agent 运行时（Harness）的 control surface：UI、AI 网关与协调层服务于这些运行时，不生成代表 Velune 自身的 Agent 人格。共享能力是一组 Rust libs，由各平台 app 通过 UniFFI 生成的类型接口在同一进程内嵌入，不另启常驻 Host。Harness 是会话历史的权威，Velune 只投影，不建立第二份会话数据库。

需求见 [PRD](../prd/index.md)，跨 unit 契约入口见 [Product TDD](index.md)。本页只维护 unit 拆分、装配与执行 owner；运行时及会话细节按下文链接读取。AI 服务与网关见 [AI 服务设计](ai-service.md)，设备与远控见 [远程访问](remote-access.md)，遥测见 [遥测装配](telemetry.md)。

## 设计原则

- **路由只选择 LLM 资源**。选机器、选 Harness、联系另一个会话由其他控制逻辑完成，不能把“转交给另一个 Agent”冒充同一请求的模型切换。
- **账户不是令牌桶的别名**。身份、计划权益、限流与数据目的地都要可区分；不合并账户额度，不通过换号绕过供应商限制。
- **协议形似不等于兼容**。同协议原生透传；异协议 best-effort 转换，无法映射的语义降级并记录诊断，不伪造历史、工具执行或成功终态。
- **协作消息不授予权限**。另一会话的消息是带出处的输入，不继承其身份或工具权限，也不冒充用户授权。
- **结果不明的操作不自动重发**。已交给 Harness 或上游的请求，在结果不明时不重放，也不改投其他账户或会话重做。
- **网关只接管模型请求通道**。网关不执行工具、不创建会话；Harness 的启动探测、遥测、搜索、插件等其他网络不算受控。
- **不同操作分开表达**。关闭窗口、取消一轮、清空队列、结束进程与退出应用是不同事件；取消观察不等于取消执行。

## 独立 package 与平台装配

每个 package 和每个平台 app 都是一个 unit，有自己的契约、依赖与 README。代码目录、unit 与 ABI 产物不要求一一对应；内部模块可以形成清楚的边界而不成为独立 package。

| unit | 职责与边界 |
| --- | --- |
| [`ai`](../../packages/ai/README.md) | 中立的 AI 操作契约，不限 LLM，不依赖 Harness、全局配置或存储 |
| [`ai-provider`](../../packages/ai-provider/README.md) | 具体协议执行与 RelayKit 协议转换；只消费本次解析出的短生命周期认证 |
| [`gateway`](../../packages/gateway/README.md) | LLM 网关：loopback HTTP 入口、路由策略、额度与冷却、派发与用量分析；是 AI 服务的一种应用模式 |
| [`conversation`](../../packages/conversation/README.md) | 运行时中立的会话契约：摘要、消息块、快照、交互与能力；不依赖 Harness 原生格式或文件访问 |
| [`agent-runtime`](../../packages/agent-runtime/README.md) | Harness 子进程、原生协议与事件转换、历史读取、原生资源管理、SSH 执行与进程树；不选择 AI 提供商 |
| [`application-api`](../../packages/application-api/README.md) | 公开业务输入、结果、错误与消息投影，只依赖中立类型；本地执行与远控共用 |
| [`application`](../../packages/application/README.md) | 应用用例：组合各领域能力、协调完整操作与失败清理，持有配置持久化与私有认证 |
| [`remote`](../../packages/remote/README.md) | 设备身份、配对、iroh 连接与远程调用；消费 `application-api`，不依赖执行实现 |
| [`telemetry`](telemetry.md) | 客户端遥测 owner：本地日志与 OTLP SDK 装配 |
| [`bindings`](../../packages/bindings/README.md) | UniFFI 导出与装配：把 application、remote 与 telemetry 组合成平台可用的对象 |

`application-api` 中的 `ExecutionBackend`、`RequestProfile`、`RoutingStrategy`、`QuotaLimit` 与 gateway／ai-provider 的同名配置有意各保留一份：契约与它们唯一共同的轻量依赖是只承载协议契约的 `velune-ai`，让契约依赖 gateway 会把 HTTP 栈带进控制端构建。application 中的转换是穷尽 `match`，任一侧增减变体都会编译失败。

共享用例不亲自解析 SDK、编码协议或实现文件存储；UI 不按底层步骤自行启动网关、注入配置、启动 Harness 或补偿跨包事务。配置持久化是 application 的内部模块：领域拥有自己的配置类型，application 负责聚合、跨领域引用校验与原子提交。只有出现独立消费者或编译边界无法用内部模块解决时，才复核是否拆出 package。异步执行资源由装配后的运行实例管理，领域包不隐式创建自己的 executor。

两种装配：

- **完整端**（Mac、Windows）：本机 `ApplicationService` 加远控服务，本机运行 Harness，也可控制其他设备。
- **控制端**（iPad／iPhone、Android）：`velune-bindings --no-default-features --features remote`，依赖树不含 application、agent-runtime、gateway 与 ai-provider。

各端界面只写一份，经同一 `VeluneApplication` 业务接口操作本机或 `application_for_device` 返回的远端代理，由设备选择决定目标。application 的 `local-runtime` feature 只裁掉 Harness 执行，配置校验仍链接 gateway，不是控制端的装配路径。

**跨语言接入**：UniFFI 0.32.2 生成 Swift、Kotlin 与 Python 接口，C# 由固定提交的 `uniffi-bindgen-cs` 生成。绑定对象拥有应用实例，提供具名方法、record／enum 与结构化错误，由生成代码管理跨语言内存；平台在后台队列调用同步操作，按快照轮询投影。关闭忙时拒绝并保留对象；执行取消是独立操作，释放平台包装不等于发出取消。

**应用根目录**：平台解析 `VELUNE_HOME`（未设置为 `~/.velune`）并显式传给 Application，库不读取环境。普通配置与提供商私有认证在该目录原子保存，公开描述不含秘密；来源型 OAuth 保留原来源的登录与刷新，不复制 refresh credential。

### 远控契约与唯一执行 owner

`ApplicationService` 用单个 owner 线程串行接纳业务，并独立推进所有会话、发送队列、协作与终态清理。远控操作进入执行端这一个 owner，不另开同一配置根，也不绕过用例直接控制 Harness。bindings 在装配层把网络 Handler 接到已有的 `ApplicationService`，remote 不反向依赖执行实现。网络连接只拥有观察与请求等待；断开、后台或关闭远控入口不调用 application shutdown。宿主退出和机器电源状态决定执行是否继续。

可信设备拥有完整业务能力，但 API key、认证交互、资源正文与附件只经显式操作传递，普通摘要与日志不携带秘密；运行时摘要省略 ACP 启动参数与环境。多端按已认证设备与视图 ID 隔离浏览、导航位置与当前发送目标；composer 正文、附件及明确运行选择的同步边界按[输入区](../prd/composer-input.md#内容与目标的归属)与[跨设备接续](../prd/remote-access.md#composer-状态接续)约定，不能把呈现隔离扩展为输入内容隔离。修改配置或替换内容以明确的修订为依据，轮询更新不能替换用户打开表单时的基线。审批沿用适配器的执行世代与单调 ID 映射。网络接纳账本按 incarnation、客户端 namespace、序号窗口与退休水位去重；结果有界缓存，重连可查询，结果过期或执行端重启后不承诺可以安全重发。设备、配对、寻址与连接恢复的契约见[远程访问](remote-access.md)。

### 本地可观测性装配

Rust 使用 tracing。subscriber 与文件输出由 `telemetry` 装配，领域 unit 只发事件，不初始化进程全局 subscriber。每个 `VeluneApplication` 拥有独立 Dispatch，同步调用建立作用域，跨线程显式传播上下文，避免不同 `VELUNE_HOME` 的对象混写。失败边界输出明确的 code 与 phase，并保留实际底层错误、子进程 stderr 与失败上下文；默认不采集请求、配置或会话正文。Mac 使用 OSLog.Logger 并消费 typed 失败的诊断编号。日志位置与保留见[开发说明](../development.md#本地诊断)，OTLP 见[遥测装配](telemetry.md)。

问题面板消费本机持久诊断日志形成跨重启投影；恢复与清理由同一日志 owner 记录，不另建问题数据库。详见[本地问题历史](telemetry.md#本地问题历史)。

## 运行时

接入方式、执行位置／HOME、工作寿命和原生资源管理见[运行时接入与执行](runtime-execution.md)。

## 模型数据面

原生／managed 路径、各 Harness 的网关注入与模型身份见[模型数据面](runtime-execution.md#模型数据面)。

## 会话

权威投影、浏览、跨 Harness 接续、管理与增量交付见[会话契约](conversations.md#会话)。

## 协作

A2A 与原生子 Agent 的身份、权限和生命周期见[协作契约](conversations.md#协作)。

## 平台宿主

- **Mac**：SwiftUI／AppKit 原生窗口、菜单与设置，嵌入 Rust dylib。每台设备一个 `DeviceStore`，每个会话列表与标签页各一个 `ConversationStore`；平台命令作用于聚焦的窗口。见 [Mac README](../../app/mac/README.md)。
- **Windows**：C#／.NET 10＋WinUI 3（Windows App SDK，非打包、自包含）完整端（[S45](../sources/remote-platforms.md#s45)）。C# 绑定以 P/Invoke 调用 `velune_bindings.dll`，与核心同进程；与同级变体同名的变体字段由 `uniffi.toml` 改名，不改 Rust API。见 [Windows README](../../app/windows/README.md)。
- **iPad 与 iPhone**：SwiftUI 通用控制端，只链接 remote-only 核心。见 [iOS README](../../app/ios/README.md)。
- **Android**：Kotlin＋Jetpack Compose 控制端，经 remote-only UniFFI／JNA 嵌入，不携带 Harness 或 AI 执行栈。见 [Android README](../../app/android/README.md)。

各平台遵循自己的原生视觉与交互习惯，跨平台共享领域契约而不共享视觉布局；原则见 [DESIGN.md](../../DESIGN.md)。产品显示版本、ABI、配置 schema 与源码提交分开记录；签名、公证与商店分发不在当前范围。
