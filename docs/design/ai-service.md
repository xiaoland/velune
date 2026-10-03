# AI service 的权威契约与 provider 边界

状态：2026-10-03 用户逐项确认后的有界实现；本步仅模块、类型、构造校验及接口。来源见 [S13](../sources.md#s13)。接口审核与运行验收分开，不以静态检查代替运行证据。

## 职责与依赖

AI service 是独立 lib，领域不限 LLM。本次只定义 sampling 操作，不为未来图像／音频等模态预建框架。sampling 是 service 的方法与契约，不是平级执行服务。

```text
调用方 → velune-ai::AiService::sampling（对外契约）
                    ↓ service 未来实现
         velune-ai::provider::SamplingProvider（对内契约）
                    ↑ provider 实现依赖此契约
         velune-ai-provider（本步只有配置边界）
                    ↓ 后续由具体 adapter 自选并封装
                协议库 / SDK / 网络
```

service 拥有两套权威契约。provider 就是 adapter，负责映射外部协议，不再额外增加一层“provider→adapter”领域模型。service 不引用具体 provider crate，不知道任何供应商 SDK 类型。本次两个 lib 无执行实现、网络依赖或进程入口，旧 `velune-core` 原型不依赖它们。

| 模块 | 归属 | 本步实物 |
| --- | --- | --- |
| [AI lib](../../crates/velune-ai/src/lib.rs) | 调用方契约 | `AiService::sampling`、无供应商错误内容的构造错误、显式可访问但 Debug 脱敏的 Payload |
| [sampling](../../crates/velune-ai/src/sampling.rs) | service | 请求／文本历史／工具定义、工具调用与结果、完成／失败／部分结果；仅表达，不执行工具 |
| [provider](../../crates/velune-ai/src/provider.rs) | service 对内契约 | `SamplingProvider`、`ProviderBinding`、`PreparedSampling`；一个 inward 调用对应一次实际 attempt，禁止隐藏重试 |
| [IDs](../../crates/velune-ai/src/ids.rs) | service | ProviderId／ModelId／CallId／AttemptId／ToolCallId／ToolName 不可互换；非零 ConfigRevision |
| [observation](../../crates/velune-ai/src/observation.rs) | service | 逻辑调用与实际 attempt 分开关联；结构化错误、用量、耗时，没有正文与日志写入器 |
| [provider 配置](../../crates/velune-ai-provider/src/config.rs) | provider | 协议配置＋凭据引用＋模型列表；ID／revision 是定位及快照元数据，无账户／渠道实体 |

根 Cargo workspace 只组织编译，不把新 lib 接入旧 UI／Host。旧架构里的 Account／Entitlement／Slot 和旧 mock Router 不是此 AI service 的权威契约；旧实现作为原型保留，不在本步迁移或重构。

## 配置与装配

统一配置中心拥有持久配置及其外部格式，app main 负责读取、校验、创建 provider 和 service。两个 lib 均不读环境变量、文件或全局配置、不解析秘密、不保存配置。本步不实现配置中心或 app main 接线。

外部未受信配置只能经构造器进入领域类型；没有给已校验类型派生 Deserialize。HttpEndpoint 使用分开的 transport／host／port／base_path，避免把带 userinfo、query、fragment 的任意 URL 混入。校验 DNS 或 IP、非零端口、受限 base path；HTTP 是显式配置，允许目的地及生产 HTTPS 策略仍由装配方决定。此构造器不是完整 URL parser 或网络安全策略。

ProtocolConfig 使用独立 ResponsesConfig／MessagesConfig 的枚举变体，仅声明配置形状，不表示协议已实现／兼容。这里列出的是两个 sampling 协议，不把 Harness Pi 误当成供应商网络协议。未来 adapter 需要新的协议字段时再扩充，不接受万能 JSON 配置袋。

ProviderConfig 校验模型列表非空、对外 ModelId 唯一、外部模型名非空且无控制字符。CredentialRef 仅是不可自动解析的引用；其语法校验不能证明它存在或调用方没有误填秘密。配置中心还需验证跨 provider ID 唯一性和凭据引用可解析性，此 lib 不承担全局注册。

运行配置稳定性：app 对新 revision 创建新的 immutable provider 实例与 ProviderBinding，替换 app 持有的 Arc。`prepare` 校验请求 provider／model 属于该 binding，并捕获 Arc、revision、逻辑 call 与 attempt ID。PreparedSampling 无修改／重选目标方法，消费后返回捕获的 adapter Arc 和请求。不提供“边执行边读最新配置”的入口。

**类型保护的限度**：Arc 保持所选对象身份，不保证 trait 实现没有 interior mutability。实际 adapter 必须捕获自己的协议／模型映射／凭据引用，不在调用途中重新读配置；真实 service 必须在返回异步 Future 之前取得当前 binding。这里有接口和快照载体，没有可执行 service，不能声称已验证并发配置切换。凭据轮换细则和 resolver 的生命周期未实现，也不读取任何当前凭据。

## Sampling 与观测

本步是完整结果型 Future 契约，不是已实现的流式协议。输入含 instructions、用户／assistant／tool-result 历史、工具定义、正整数最大输出 token。构造校验历史非空、工具名唯一、tool-call ID 唯一、tool-result 引用先前且未重复回应的调用。schema 必须是 JSON object；不冒充完整 JSON Schema 验证，也不强行把历史工具名限定在当前可用工具列表。

输出可含文本和多个工具调用。失败保留 typed kind、NotSent／Accepted／Unknown 与可选部分输出。service 和 provider 都只返回工具调用，执行与权限属于调用方。Output／Failure 目前是 provider 要履约的 DTO：完成原因、tool-call 关联、错误是否携带部分结果，仍需 adapter 映射验收，不能说所有语义已由类型穷尽证明。

Usage 的每个值均为 Unknown／Reported／Estimated；默认 Unknown，明确报告零才是 Reported(0)。失败仍带独立 usage，不因为失败就归零。本步只有 sampling token 计量，不做费用计算或跨模态假装通用 token。原始协议出现未支持用量字段时，未来 fixture 映射须显式记录保留／不支持，不能静默伪造。

CallObservation 表示一次逻辑 sampling；AttemptContext／AttemptObservation 表示一次实际 provider 尝试，携带不同 ID、provider／model 和配置 revision。计数与事件完整性由未来执行实现兑现；本步无聚合器、重试循环、路由／fallback 或日志 sink。Payload 的 Debug 输出只显示 redacted；默认观测没有正文、工具参数、原始错误、凭据或 headers。调用方显式访问 Payload 仍可能泄漏，所以不是不可绕过的审计安全沙箱。

## 本步验收与后续执行证据

| 验收项 | 本步载体与证据 | 仍不能声称 |
| --- | --- | --- |
| 权威归属与依赖方向 | 上表公开接口、Cargo manifests、cargo tree；provider crate 依赖 AI lib，反向依赖不存在 | 已有可执行 service 或 SDK 适配 |
| 配置边界 | 私有字段、typed IDs／protocol、显式构造器、无 Deserialize 绕过、无 IO／全局状态源码审核 | 真实配置中心、秘密解析、供应商资格已验证 |
| 稳定调用配置 | ProviderBinding 与 PreparedSampling 的 Arc／revision 所有权审核 | 并发热更新或真实凭据轮换已实测 |
| 输入／输出与工具 | SamplingInput 构造路径及 Outcome／Failure 公开 DTO 审核 | schema 完整有效、工具执行或实际供应商协议无损 |
| 用量与观测 | Unknown／Reported／Estimated、CallId 与 AttemptId、无正文的观测结构审核 | usage／成本准确、每次网络请求均已观测 |
| 编译保护 | fmt／check／clippy，仅编译两个 lib | 测试／运行验收通过 |

验收通过需要审阅者确认上述接口确实对应用户决定；静态命令通过只是基础证据。本次不新增测试、不运行既有集成测试，也不启动 Mac。普通固定响应 provider 示例可作为未来手动验收载体，但在用户进一步确认前不实现执行 service 或示例。

用户已提出真实 AI 接入并固定 fixture 的验收方向；provider／model、凭据既有位置、费用／额度上限仍待确认。此前不发真实请求或读取秘密。具体采集／回放设计和待办见 [本任务](../../tasks/ai-service-contracts/packet.md)。
