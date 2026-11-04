# LLM 网关路由、协议与计量

本页维护 gateway、ai-provider 与 application 的请求数据面契约。AI 服务与提供商职责见[AI 服务](ai-service.md)，产品规则见[模型路由](../prd/model-routing.md)与[分析](../prd/analytics.md)。

## 路由与 fail-over

一个订阅对应一个提供商实例；application 拥有实例认证、额度采集与唯一快照，gateway 只经中立只读契约取得额度事实，并维护运行期共享的调度状态。提供商的使用上限独立于路由，直接调用同样受约束；路由拥有策略（Smart、Order、Rotate、Weight）、成员权重与粘性。会话身份由装配方显式提供，gateway 不依赖 Agent 运行时，也不根据正文猜测身份。额度窗口可带周期，Smart 按周期内尚余时间的比例衡量余量，只影响排序（[S55](../sources/ai-routing.md#s55)）。

路由捕获一次请求的配置版本、目标与认证引用，热编辑不改变正在执行的尝试。fail-over 由网关单独决策，provider 每次只执行一次尝试，不隐藏重试。候选只来自路由的显式成员；同协议是必要条件，不证明历史、文件、缓存、加密推理或服务端状态可交给另一账户（[S30](../sources/ai-routing.md#s30)）：

- 普通认证 401 停止；Codex 订阅允许同账户有界恢复一次，失效后可在提交前切换。
- 限流、配额耗尽（含结构化的 402 配额错误）、过载、5xx 与未获答复的连接或传输失败，在提交前可以切换下一成员；权限错误与其他请求错误停止。
- 聚合路由明确拒绝 `previous_response_id` 与 `conversation` 等上游状态引用，不静默删字段或改投其他账户。
- 成功响应头交付后不再切换，包括之后的 SSE 业务失败；不拼接两个流，也不缓存完整流来扩大切换窗口。
- Claude Code 子进程启动后，或向已有进程写入本次输入前，即进入不可自动重放状态；之后的退出、超时与解析错误不切换账户重做。CLI 故障不伪装成 HTTP 401。
- 取消结束本次调用，不触发 fail-over，也不声称撤销已发生的上游执行。

分类、冷却与观察的细节见 [gateway](../../packages/gateway/README.md)。

额度观测的附加信息由 application 计算，不进入 gateway：节奏（均匀消耗时此刻应已用的比例、按平均速度的预计用尽时间）与金额。每次成功观测按提供商写入 `$home/quota-history/` 的 JSONL，带认证绑定摘要；绑定改变时清空，未变化的值至多 10 分钟记一次，保留 30 天。观测由不满足变为满足使用上限、窗口用尽或账户不可用时产生提醒，核心保留最近 64 条，平台以序号游标轮询。

## 原生派发与观测

ChatCompletions、Responses 与 Messages 是同一原生派发操作的三种线协议：`velune_ai::NativeProvider` 以 `Protocol` 区分，共用请求、事件、失败与完成类型，每个 adapter 只服务构造时确定的一个协议。ai-provider 的 `NativeHttp` 按协议区分端点、认证头、输出上限字段与终态；Claude Code 本机后端实现同一契约。协议实现按协议组织，服务商名称、端点与模型标识是配置数据，不选择不同的通用执行路径。

gateway 按路由绑定写入精确的 `providerModelId`，其余请求字段保持原样；原生 JSON／SSE、HTTP 状态与安全响应头经同一保真边界。保真以非路由字段与响应事件的内容、顺序及含义为准，不要求 JSON 空白与成员顺序不变；同协议的未知扩展不丢弃。原生事件 sink 可等待，下游以容量为 1 的通道施加背压；取消关闭派发 Future。HTTP 入口使用 Axum。

- **Messages**：baseURL 追加 `/v1/messages`，同协议请求的版本由调用方的 `anthropic-version` 决定，异协议转换为 Messages 时使用 `2023-06-01`；API key 以 `x-api-key` 发送。缺少请求输出上限时使用目标模型配置的上限，不用硬编码常数。
- **注入地址**：Pi 与 DSH 的 Messages 注入使用网关 origin，OpenAI 协议使用 `/v1`。Codex 发给网关的协议是 Responses，上游可经转换使用其他协议。

协议终态与传输结束分别观察：原生 completed、incomplete、failed 与 finish reason 保留原义；HTTP 200、EOF 或客户端没有收到事件都不能单独证明成功或未执行。用量不足时保持未知，不补零。

网关观测记录关联 ID、协议、是否流式、目标在当前配置快照中的序号、HTTP 状态、阶段、耗时与结束归因，不记录正文、实际模型 ID、地址或认证引用。`gateway_response_ready` 只表示响应元数据已准备，不证明客户端已接收，也不是允许重放的提交闸门；request 的 `transport_completed` 表示转发通道结束，不代表模型业务成功。调用方断开与网关关闭分别记为 `downstream_closed` 与 `gateway_stopped`。日志装配见[架构](architecture.md#本地可观测性装配)与[开发说明](../development.md#本地诊断)。

## 协议转换

异协议转换由 ai-provider 内部链接的 New API RelayKit Go 库完成（AGPLv3，见 [ai-provider](../../packages/ai-provider/README.md)），不引入 New API 管理服务。Rust 继续拥有 HTTP/SSE、认证、路由、取消、背压与原始计量；私有桥接封装请求范围的转换状态，不暴露给平台，也不建立常驻 Host 或 IPC。正常结束、失败与释放分别表达，取消与截断不调用成功收尾；同协议绕过转换。

网关在实际候选派发前，以已解析的提供商型号和该模型的输出上限准备转换；排序、fail-over 或认证失败后改选成员时重新准备，响应使用同一次派发的计划。失败且未交付 headers 的尝试不产生转换响应，最终错误仍由调度交付。

转换是 best-effort：无法完整表达的字段尽量保留可用内容、映射参数，并记录不含正文的降级诊断，不整次拒绝；不伪造历史、工具执行或上游成功。

- 文本增量立即交付。转换到 Messages 的工具参数须是合法 JSON object，因此在单个工具完成时交付有界累积的参数；Responses custom 工具在参数完成后解除 `{input:string}` 包装；无法解析的参数保留原始字符串并记录降级。OpenAI 两协议之间的普通 function 参数继续增量交付。
- 上游未知的 token 字段不补零。Messages 必需的数值字段无法获知时使用结构占位并记录 `usage / required_numeric_placeholder`，占位不进入计量。缺少上游 response ID 时生成请求范围的转换 ID，它不能用于上游查询或状态恢复。

## 用量与性能分析契约

计量来源是经过 Velune 网关的真实上游请求。ai-provider 从三种协议的原生 JSON／SSE 提取计量事实；gateway 在协议转换之前观察这些事实，关联请求与实际提供商和模型身份，交给调用方注入的统计接收器。统计与 tracing 独立，转换补造的占位值不进入统计；采集不改写同协议的原始响应。

application 持有分析存储、查询与记录生命周期，文件为配置根目录下的 `analytics.sqlite`。记录保留发生时的提供商身份、名称与实际 provider model ID，之后删除或改名不改变历史含义；它不是会话正文数据库。只统计启用后经过 Velune 的请求，不扫描运行时历史。采集与存储失败不影响模型业务结果，但要在本地诊断与分析读取状态中可辨认。

每项 token 数独立保留“已报告”或“未知”。OpenAI 的缓存读取属于输入子集、推理属于输出子集；Messages 的输入总量包含普通输入、缓存读取与缓存创建，缓存字段缺失时总量未知但保留已报告的普通输入。流式累计 usage 覆盖已知值，不逐事件累加。聚合同时返回报告覆盖数量：总量只合计输入与输出均已报告的请求，输入列与输出列各自合计已报告部分；缓存与推理细项不重复计入总量。依据为 [OpenAI usage 字段](https://developers.openai.com/api/reference/resources/chat/subresources/completions/methods/create)与 [Anthropic 缓存计量](https://platform.claude.com/docs/en/build-with-claude/prompt-caching)。

有效输出速率是已报告输出 token 除以从上游派发到业务终态的耗时，包含等待，不是纯生成速度；跨请求聚合用同一组有效请求的输出总量除以耗时总量。首输出延迟单列，只由流式的实际文本、推理或工具参数触发，headers、心跳与仅含 role 的事件不算；非流式不推测。失败或取消请求的已知用量仍保留。日期查询使用平台给出的本地日边界，存储为 UTC 瞬时。费用、订阅余额与输出质量不由这些指标推断。
