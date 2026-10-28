# 遥测语义与可维护性

2026-10-28。用户已确认总目标、范围和技术方案，并明确要求跳过独立验收设计，理由是本次关注语义合理性。用户随后以“开始”授权按计划实现和交付，当前已进入实现。保留与修改相称的构建、实际输出核对和语义审查，不以自动检查通过代替语义判断，也不再建立独立验收方案。

## 已确认总目标

建立并落实一套适用于 Velune 分布式架构的遥测约定，使 agent 能通过稳定的查询理解跨设备操作的因果、进度和结果，并使后来的人和 agent 能依照官方 SDK、项目文档和真实范例正确新增、验证及演进埋点。

完成后的改变应同时体现在三个场景中：

1. 排障 agent 面对一次真实操作，能够区分请求受理、后台执行、单次尝试和最终结果，定位可见的失败边界；采样、断连或数据缺失不足以支持结论时，明确保持未知。
2. 维护者增加一个业务操作时，能够确定由谁观察、何时开始和结束、应发 span／事件／指标中的哪些数据，以及如何命名和关联；同一业务动作在不同客户端具有一致含义。
3. 维护者调整埋点或升级 SDK 时，能够发现输出与约定的偏差，并识别对查询、历史数据和其他客户端版本的影响。普通重构不应无意改变事件身份。

## 已确认范围与边界

范围覆盖四端原生埋点及 Rust 共用链路：原生操作、FFI、远控受理与结果查询、执行轮次、Harness 的可见边界、Gateway 请求及尝试。先盘点现有埋点和消费者，再修正已发现的语义问题；不是只修改示例而保留相同问题的其他调用处。

需要收敛的契约包括 service／scope 归属，span 和事件的命名、类型、起止与触发条件，字段类型和单位，错误及取消／未知语义，分布式关联，计数归属，采样与数据缺失的解释，以及约定的变更方式。交付包括符合约定的实现、现有权威文档、可复用查询和覆盖关键边界的真实范例；最终不只交付一份规范。

保留前任务已经确定的产品边界：默认不配置、不启用；各客户端独立控制；遥测后端不影响业务可用性；本地完整诊断与向外发送范围分开；沿用成熟 SDK 和 OTLP；继续由 SVC dev 管理开发服务。对不同版本数据的处理需要有明确方案，但不预设永久双写或通用兼容层。

本任务不以增加采集数量为目标，不要求覆盖每个函数、外部 Harness 内部或全部业务功能。新的托管观测平台、完整仪表盘／告警体系、自制通用遥测框架、全量自动埋点和长期生产压力治理不在当前提案范围。工具、scope 细分、事件出口与 API 调整均待技术阶段根据证据选择；不把前期推荐当作既定实现。

## 已确认预期效果

实际导出的数据与约定一致，四端的同类操作能够使用相同语义查询。验证至少包含同机调用、跨设备受理后异步执行、网关失败后重试成功，并覆盖结果未知与缺失数据的解释。名称、kind、字段和时间边界需要直接检查，不能以“后台收到三信号”替代。

原先拟通过专门的埋点扩展练习和 agent 盲诊来观察可维护性。按用户本轮调整，不再为这些活动另设验收设计或独立流程；实现过程中结合真实调用、输出样例和查询审查其可理解性。语义是否合适由业务边界和维护成本判断，字段是否按约定输出则可直接核对，两者不能互相替代。

## 已有证据与后续工作

前任务完成初步接入，独立保留其验收结论。已有 [语义评审](../otlp-observability/packet.md#后续埋点语义评审) 和 [维护性调研](../otlp-observability/maintainability-research.md) 可复用。已观察的问题包括 Rust span 同名、跨设备操作均标为 INTERNAL、原生写操作落入读取类别、Rust eventName 依赖文件行号、原生与 Rust 事件字段不同，以及事件双路输出。它们是新任务的输入，不代表问题清单已经覆盖所有现有埋点。

产品约束归 [诊断 PRD](../../docs/prd/diagnostics.md#客户端遥测)，当前实现契约归 [遥测装配](../../docs/design/telemetry.md)，开发操作归 [开发文档](../../docs/development.md)。后续确认的长期规则应更新这些现有所有者，不在任务包中建立第二套规范。

用户已批准[技术方案](technical-design.md)：保留 service／scope 归属，明确每种操作的 name、kind 与生命周期，统一标准事件名和受控字段，并使用 v2 标记区分新旧数据。已核对四端固定 SDK 支持所需事件输出，不需要先升级。

远控设计修正了前期建议：SERVER 表示鉴权后的受理／查询 handler，CLIENT 表示 RPC 等待，后台执行使用 INTERNAL。当前没有消息消费边界，不采用 CONSUMER；也不为完整网络往返计时新增跨线程状态管理。Gateway 则区分 HTTP 处理结果、单次上游尝试和业务轮次，避免完整返回错误响应仍被当作成功。

用户已跳过验收设计并授权实现，已按以下顺序完成实现与检查。没有新增依赖或更换 SDK。工作区的其他任务源码修改不属于本任务交付。

## 实施顺序

按共用输出、领域调用、原生调用、消费者的顺序推进，使后面的改动使用同一套已经明确的语义。各步同时维护受影响的已有检查，不另写验收设计。

1. **修正共用输出与生命周期。** 修改 `packages/telemetry/src/lib.rs` 和 `sdk.rs`：显式操作名、kind、父上下文来源，固定 eventName、字段和 v2 resource 标记；分开 span 与事件的出口；完成日志直接携带已知关联字段。Operation 明确单次终态、执行专用事件和补报不计量的策略，并在 owner 完成时释放 span。更新现有 `wire_tests.rs` 中受影响的实际 OTLP 观察，特别关注过滤后的日志关联、未采样结果和重复终态，而非测试一张名称映射表。
2. **迁移 Rust 调用边界。** 在 bindings 的静态入口加领域前缀，同步调整成功读取降噪和缺父判断，保留本地错误详情；涉及 `src/lib.rs`、`diagnostics.rs`、`remote.rs` 及直接 diagnostics 调用。remote 的 `client.rs`／`service.rs` 分别记录 RPC 与鉴权后的 handler，首次受理把 server carrier 交给本地 execute；查询先取得真实结果再完成。application 的 `local/telemetry.rs` 保留 Session 终态所有权。Gateway 的 `runtime.rs`／`observation.rs` 将响应状态、body 生命周期和 HTTP／CLI 类别交给观察对象，分别判断总体请求与单次尝试；补报保留原耗时和身份，不重复计量。不调整业务重试、账本或网络协议。
3. **统一四端原生调用。** 先调整 Mac／iOS 共用的 `ClientTelemetry.swift` 及 `Transport.swift`、`RemoteClient.swift` 调用入口，再调整 Android 的 coordinator、DeviceHub、ConversationModel 等调用者，以及 Windows 的 coordinator、Core、Conversation、Listings 等调用者。枚举类别为必填参数，删除默认 read 与 CallerMemberName 推断；保留本地诊断的方法名称。Mac 的 OperationEffect 还控制关闭时读写准入和配置变更通知，不能用遥测类别替换或改变它。原生 SDK 同步输出 v2 字段和标准事件名。按已有脚本完成受影响平台构建，Windows 使用 win-ws 上的 `scripts/build-windows.ps1`；本次不改界面，因而不另做浅深色视觉验收。
4. **同步查询、现有样例与权威文档。** 更新 `scripts/dev-telemetry.py` 的版本选择与返回范围说明，新查询默认 v2，历史或混合版本必须显式选择；继续保留原始返回，不归一化猜测旧语义。维护 `scripts/manual-telemetry.py` 及其直接引用的旧任务脚本中的当前输出解析，历史已采集证据保持原样。将已实现契约写入 `docs/design/telemetry.md`，在 `docs/development.md` 写清查询方法、本地诊断与外传字段的区别，按涉及内容同步平台说明和 Windows PARITY。开发服务沿用现有 `svc dev` 入口，无需新增服务。
5. **检查实际改动并交付。** 按调用链审查受理、等待、执行、尝试与终态是否各有明确含义，结合已有合成数据脚本查看实际输出，修复遗漏或不一致。检查默认关闭、隐私白名单和配置代际未被本次语义调整破坏；复用已有相关检查，不重跑安装、分享配置和设备网络恢复的整套流程。完成文档链接检查和 `git diff --check`，报告已观察到的结果及未检查部分，不把构建成功或字段一致写成语义合理性的证明。

## 实施准备中核实的细节

- Rust 固定版本的 trace layer 在进入 span 时激活 OpenTelemetry context，日志 SDK 在 emit 时读取该 context。因此将两条出口分开不需要自制 context 桥接；实现时仍用现有未采样日志与实际 OTLP 检查确认没有遗漏。
- 日志 bridge 默认不复制 span 的属性。RequestId 等完成记录所需字段必须保留在 Operation 上并显式写入事件，不能只更新 span 字段。
- Mac 的操作名称、OperationEffect 和遥测类别承担不同职责。实现保留前两者的现有行为，显式增加语义类别，避免以重分类为由改动关闭或通知行为。
- 共享工作区中，Mac Transport、bindings 入口和平台文档等文件存在其他任务改动。实施前重读相关片段，在现状上做局部修改，不重置文件或把其他任务的改动计入本任务。

实施准备没有发现需要升级 SDK、改变协议或新增服务才能继续的前提。若实现中出现这类新事实，应先修正受影响的技术决定；普通编译、过滤或查询问题在既定范围内直接修复。


## 实现与检查进展

已实现具体 span 名称与 kind、固定标准事件名、v2 字段与资源标记、原生显式类别、远控 server→本地 execute 关联、Gateway HTTP／CLI 结果，以及结果补报不重复计量。Operation 在终态直接结束 SDK span，避免后台诊断句柄延长其持续时间。查询、现有解析脚本和权威文档同步更新，没有新增依赖。

2026-10-28 已检查：Apple 共用 coordinator 类型检查通过；iOS 全部当前源码使用从本轮 Rust metadata 生成的新绑定进行类型检查通过。Android Kotlin 编译与 APK 打包通过，独立重跑 lintDebug 通过，复用了既有配套 Rust so／绑定产物。Windows 相关 Rust release 构建、C#／WinUI 发布通过，产物在 `C:\Users\yyh\velune-observability\target\windows\Velune`。Windows 构建副本中由 Mac tar 带入的 AppleDouble 文件已清理；它们曾被 XAML 编译器误当源码。没有安装 App 或改动真实业务数据。

现有 LGTM 上的 traces／logs／metrics 查询分别以 v2、legacy、all 执行，九种查询均被后端接受；使用无业务记录的合成 run ID，证明查询语法可执行，不证明本轮新记录已入库。文档相对链接与 git diff --check 通过。实际 OTLP 回归已在本机以单 Cargo job 执行通过：18 passed，0 failed，日志为 `target/telemetry-semantics-tests.log`。因机器同时有其他构建且出现大量交换空间，本任务已停止早期并发构建，不启动设备验收或安装流程。


实现已完成。Mac 全部当前源码类型检查通过，使用本轮 Rust metadata 生成的绑定；检查输出包含已有的 imported-type conformance 警告，没有类型错误。Rust 实际 OTLP 检查确认具体 span 名／kind、父子关系、仅日志事件出口、轮次单条终态，以及持有诊断句柄时 SDK span 仍按终态结束；未采样日志和配置代际相关检查也通过。Android 采用已有 Rust 构建产物，Apple 做源码类型检查，本轮没有重新构建移动端 Rust 包、安装或运行真机；不将这些检查声称为设备验收。Windows release／原生发布与 LGTM 查询语法检查见上文。

长期契约已更新到 `docs/design/telemetry.md`，查询与版本使用方式归 `docs/development.md`。源码、脚本和文档保留在工作区；本轮没有提交，也没有改动默认关闭、配置分享或后端部署行为。独立验收设计、专用盲诊与扩展练习按用户指示跳过，当前没有等待用户决定的实施事项。


## 提交与全端安装

2026-10-28 用户明确授权提交，并允许全部设备覆盖安装。当前开始提交本任务改动，随后重建完整 Rust／原生包并更新 Mac、Windows、Android 和已连接 iOS 真机，保留应用数据。


安装结果（2026-10-28）：语义提交为 `0b27ff1`。共享工作区随后出现其他任务的未提交源码变动，因此除 Windows 外的最终包使用该提交的独立 worktree 构建，避免混入未提交代码。构建中收窄了 Mac 生成器的依赖，只启用 cli；未修改应用语义或升级依赖。

| 平台 | 完整构建与覆盖位置 | 启动观察 |
| --- | --- | --- |
| Mac | release Rust、UniFFI、Swift 与签名完成，覆盖 `/Applications/Velune.app`；主程序和 Rust 库摘要与构建产物一致，manifest 来源为 0b27ff1。 | 已启动并确认进程存活。 |
| Windows | 从提交归档完整构建，覆盖既有 `C:\Users\yyh\velune-mygo\target\windows\Velune`；四个程序／DLL 摘要一致。 | 已在原交互桌面 SessionId=2 启动。临时启动任务已清理。 |
| Android | Rust arm64 库、Kotlin 绑定、APK 与 lint 完成，使用 `adb install -r` 更新日常版和既有遥测版。 | 两个应用分别启动成功，包更新时间均为本轮。 |
| iPad | Rust 设备库、Swift 绑定、Xcode 构建与签名完成，覆盖既有 app.velune.ios.telemetry，系统回执安装成功。 | 首次启动曾被系统信任检查阻止；用户随后完成开发者信任，并明确确认能正常打开。这是用户的人工启动确认。 |

整个安装过程未卸载或清空应用数据，未安装测试 runner。Android 打包的 Java 堆上限调整为 2 GiB、单 worker，重跑后打包与 lint 通过。Cargo registry 软链接导致 UniFFI 模板宏引用错误路径，使用实际缓存根目录重建生成器后完成；说明已补入开发文档。

原始构建日志、安装回执与 Android APK 保存在原工作区被 Git 忽略的 `target/telemetry-installed-builds/`。安装相关代码／文档只提交本任务文件，其他任务的修改保留。用户已确认 iPad 信任与启动成功，四端覆盖安装及启动确认均已完成，当前没有待处理的安装事项。
