# SDK 与 FFI 隔离实验

2026-10-26，为收敛[实现计划](plan.md)进行。实验代码、锁文件、生成绑定和完整合成接收记录在仓库 `target/otlp-sdk-probes/`，不修改产品源码、不读取应用数据。接收器使用官方 OTLP protobuf 和 gRPC，监听开发机随机回环端口；与 sfp7 后台实验分开。

## 实际版本与结果

| 生态 | 实际组合 | 观察 |
| --- | --- | --- |
| Rust | OTel／SDK／OTLP／appender 0.32.0，tracing-opentelemetry 0.33.0，tonic 0.14.6，Rust 1.99 | 三信号通过；原生 tracing bridge 形成正确父子 span 和日志关联；counter=7；撤销 SDK 队列和连接等待期间的出口后无 discard／pending 数据抵达；新代际可发送 |
| Swift | core 与 exporter 2.6.0，grpc-swift 1.27.6，Swift 6.2.3／Swift 5 language mode | 三信号通过；channel 关闭后 flush 不发送旧队列，新 channel 正常；必须显式注册 metric views，不能用 flush 成功证明有指标 |
| Java／Kotlin | OTel BOM、SDK、OTLP、Kotlin extension 1.62.0；项目相同的 Kotlin 2.2.21；coroutines 1.10.2；JDK 17 | 官方默认 OkHttp sender 三信号通过；先关闭 exporter 再清理 providers 时旧队列不发送；两个协程交错调度仍保留各自 context，退出后恢复原 context |
| .NET | OTel 1.20.1；本机 .NET SDK 8.0.204，net8 控制台 | 三信号通过；使用可持有的专属 HttpClient，取消／释放后旧队列不发送，新 client 正常。不是 Windows net10／WinUI 构建证明 |
| UniFFI | 0.32.2；实际生成 Swift bindings，链接 Rust dylib | 两个 Swift detached tasks 交错调用持有不同 context 的不可变视图；Rust child 的 trace ID 和 parent ID 分别匹配各自 Swift parent；共享 owner 显式关闭后旧视图调用被拒绝 |

最终清理了早期实验记录，重新运行四套 SDK 与 FFI 程序，再解码接收器实际 payload 断言：每套 SDK 的 before／after 代际均有 traces、logs、metrics；父子 ID 正确；日志关联 child trace；counter 值为 7；discard／pending 代际没有收到数据。使用冲突的 `OTEL_EXPORTER_OTLP_HEADERS`、`OTEL_RESOURCE_ATTRIBUTES` 和 `OTEL_EXPORTER_OTLP_ENDPOINT`，确认探针仍使用显式地址、资源和 metadata。结构化摘要见[实验结果](sdk-evidence.json)。

## 改变计划的发现

**Android 版本不能照搬“最新稳定”。** OTel Kotlin extension 1.67.0 的 metadata 是 Kotlin 2.4，项目 Kotlin 2.2.21 编译失败。检查 Maven Central 的正式 POM 后，1.62.0 是仍依赖 Kotlin 2.2.21 的组合；统一改用该版本并实际编译、运行三信号和协程实验。没有通过跳过 metadata 检查或升级整个 Android 工具链掩盖问题，也没有把不同版本的 SDK／extension 强行拼装。

**Swift metric views 必须明确注册。** 2.6.0 的 `ViewRegistry.findViews` 只返回已注册 views；首次探针未配置 view 时，flush 成功但接收端没有指标。为测试 counter 注册显式 view 后，实际收到值为 7 的 OTLP metric。产品对批准的 counter／histogram 明确注册 view，并以实际接收值验收；不改第三方库，不依赖未生效的默认行为。

**显式配置仍需隔离 SDK 的环境读取。** Rust 0.32 的 tonic builder 会合并环境 metadata，即使传入了自有 channel。实验在 interceptor 中重建明确允许的 metadata；Swift exporter 显式传 `envVarHeaders: nil`，各 provider 使用显式 resource；.NET 显式设置 Headers／ResourceBuilder；Java 使用直接 builder 与 Resource，而不使用自动配置器。冲突 canary 未出现在最后的 wire 数据中。不得通过修改进程全局环境实现这一隔离。

**只换 exporter 不足以切换已有应用。** 当前 `ApplicationService::open`、Diagnostics 和长期存活的 listener／reader 保留 dispatcher 或 span。实现计划选择稳定 owner 加新工作边界读取当前代际，继续使用成熟 bridge。旧 span 不移植到新 provider；换配置期间的中断需要如实呈现。此处已核实源码与选定接线方式，尚未修改真实 ApplicationService 验证整条切换路径。

## 传输撤销证据的限度

四套探针都先让三信号进入 batch／reader，再撤销出口并 flush，接收端没有旧数据。Rust 额外让 tonic connector 进入等待，再撤销许可、解除等待并恢复新代际；旧 pending 数据没有到达。Rust 采用连接与 I/O 许可检查，既不是自行编码 OTLP，也不是只检查 interceptor。

Swift channel close、.NET 专属 HttpClient 取消／释放、Java exporter shutdown 的入口已在对应正式版本源码与探针中核实。Java 1.62 的默认 sender 使用 `cancelAll`、连接池清理和受管 executor shutdown；实施时仍需把出口准入与关闭排好顺序，防止并发调用在取消以后才 enqueue。没有声称已测完这些 SDK 的全部在途竞态，也没有测试移动网络、TLS 证书和后台冻结。

准备候选 provider 时是否提前捕获数据、真实 ApplicationService 不重建的切换、QUIC 全链路、状态迁移、权限矩阵及真实 UI 都属于产品实现步骤的行为验证。本轮没有先造一套重复产品原型来代替它们。

## 依据

- [Swift ViewRegistry 2.6.0](https://github.com/open-telemetry/opentelemetry-swift-core/blob/2.6.0/Sources/OpenTelemetrySdk/Metrics/View/ViewRegistry.swift)、[Swift gRPC exporter 2.6.0](https://github.com/open-telemetry/opentelemetry-swift/tree/2.6.0/Sources/Exporters/OpenTelemetryProtocolGrpc)。
- [Java Kotlin extension 1.62.0 POM](https://repo.maven.apache.org/maven2/io/opentelemetry/opentelemetry-extension-kotlin/1.62.0/opentelemetry-extension-kotlin-1.62.0.pom)、[1.67.0 POM](https://repo.maven.apache.org/maven2/io/opentelemetry/opentelemetry-extension-kotlin/1.67.0/opentelemetry-extension-kotlin-1.67.0.pom)、[OkHttp sender 1.62.0](https://github.com/open-telemetry/opentelemetry-java/blob/v1.62.0/exporters/sender/okhttp/src/main/java/io/opentelemetry/exporter/sender/okhttp/internal/OkHttpGrpcSender.java)。
- [Rust tonic 配置与 metadata](https://github.com/open-telemetry/opentelemetry-rust/blob/opentelemetry-0.32.0/opentelemetry-otlp/src/exporter/tonic/mod.rs)、[.NET HttpClientFactory 选项](https://github.com/open-telemetry/opentelemetry-dotnet/blob/core-1.20.1/src/OpenTelemetry.Exporter.OpenTelemetryProtocol/OtlpExporterOptions.cs)。
- 本地接线约束：[ApplicationService](../../packages/application/src/service.rs)、[bindings 生命周期](../../packages/bindings/src/lib.rs)、[remote 入站策略](../../packages/remote/src/endpoint.rs)、[设备状态读取](../../packages/remote/src/store.rs)。
