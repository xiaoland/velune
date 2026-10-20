# Kimi Coding Plan：接入收敛

2026-10-20，用户认可模板、额度适配和服务错误语义适配方向，要求验证普通 Coding Key 的查询权限并继续收敛。用户随后明确“好的，开始实现”，本文件转为当前实施与验收依据；完成状态见下文。产品决定归 [PRD](../../docs/prd/index.md#ai-网关与-agent-运行时配置)，跨单元边界归 [AI service 设计](../../docs/design/ai-service.md#订阅能力组合与执行后端)。

## 已完成的只读验证

使用用户指定的相邻项目私有文件中 `KIMI_CODING_PLAN_API_KEY` 与 `KIMI_CODING_PLAN_BASE_URL`。只提取这两个变量；Key 具有 Coding API Key 前缀，不是 JWT 形状。主机严格限定官方 Coding 服务，TLS 正常校验、禁止重定向，使用 Velune 真实 User-Agent。密钥仅存在于进程内存及 curl 标准输入，不出现在命令行参数、日志或文件中；响应正文未保存，账户标识未输出。

共两次请求，无模型调用、OAuth 登录或重试：

| 请求 | 实测 | 可得结论 |
| --- | --- | --- |
| GET `https://api.kimi.com/coding/v1/usages` | HTTP 200，根级 `usages` 存在；`limit_5h`、`limit_7d` 有数值比例及 reset 字段 | 本 Key 在本次检查可读取两个窗口 |
| GET `https://api.kimi.com/coding/v1/me` | HTTP 200，`user_id`、`global_id` 存在 | 本 Key 可取得账户归属信息 |

本次未返回 `limit_month_total`、`limit_month_code` 或 `boosterWallet`。缺失不等于零、无限额度、未开通钱包或没有余额。本次只确认字段存在及基本类型，RFC3339、比例区间等完整边界校验留在实现与合成验收；不能将两个 200 推导成所有 Key/套餐和海外域均兼容。

第一方实现固定为 [kimi-code 5710c549](https://github.com/MoonshotAI/kimi-code/tree/5710c5496a213742ca0687f69dd8eeb5bde52dbf)：[managed-usage.ts](https://github.com/MoonshotAI/kimi-code/blob/5710c5496a213742ca0687f69dd8eeb5bde52dbf/packages/oauth/src/managed-usage.ts)、[managed-userinfo.ts](https://github.com/MoonshotAI/kimi-code/blob/5710c5496a213742ca0687f69dd8eeb5bde52dbf/packages/oauth/src/managed-userinfo.ts)。这些云接口有官方实现和本次实测依据，仍不是供应商承诺稳定的外部额度 API。

## 已授权的首版契约

**组合与请求。** 新增 Kimi Coding Plan 模板，组合为 HTTP、Messages、API Key、KimiCoding profile、KimiUsage source；上游基址为 `https://api.kimi.com/coding/`，最终 Messages 路径为 `/coding/v1/messages`。不创建供应商类别、CLI 后端、OAuth 流程或内部账户池。模型 ID 继续由提供商模型配置拥有。上层 Chat/Responses 入口使用现有 best-effort 转换，不能宣称思考与工具历史无损互换。

**服务语义。** 在现有 request profile 增加 KimiCoding，拥有该服务的目标和错误约定，不另增 ErrorPolicy 配置字段。三种 HTTP adapter 可以共用此服务分类器，输出中立事实；网关不读取 Kimi 文案或判断品牌。

| 已识别事实 | 调度含义 |
| --- | --- |
| Key 无效 | 认证失败，不伪造刷新能力 |
| 套餐或模型权限不足，即使 HTTP 401 | 权限/请求问题，不把 Key 标为过期 |
| 明确的 5 小时、周或月额度耗尽，即使 HTTP 403 | 提交前可尝试下一有资格成员，触发额度重取 |
| 风控/账户限制或未知 403 | 停止，不当成额度或普通频控 |
| HTTP 429 的频控/引擎过载 | 短暂不可用，按既有有界冷却处理 |
| 明确的账户禁用，即使 HTTP 500 | 停止，不被通用瞬态 5xx 规则覆盖 |

错误结构在边界有界解析，只采用第一方支持的精确 code/type 或固定文案模式；不使用全局“含 quota 即切换”的猜测。原始状态和响应保留透传；已交付响应后继续禁止切换或拼流。尚无本 Key 的真实模型错误 JSON 样本，不能以官方人类可读表格证明具体机器错误码稳定。

**认证与额度。** application 从提供商私有快照捕获当前 Key、区域和 credential generation，经固定目标解析 `/me`、`/usages`；不扩大模型 `CredentialTarget` 的授权含义。复用按需采集、single-flight、有界并发、超时和取消。窗口比例转百分比、RFC3339 reset 转时间戳；只映射返回的有效窗口。月窗口用具名 ID，不把自然月伪装成固定秒数；钱包数据没有可信值就不生成金额。

**账户归属。** 使用服务区域与 `user_id` 作为已验证 owner；`global_id` 暂不用于跨区域合并。在已有 provider 元数据中检查重复 owner，不建立中央账户注册表。同一会员创建的多个 Key 不能启用为多个独立订阅候选。新 Key 未验证时不得借旧 owner 通过订阅身份检查。

**换 Key。** generation 仍负责取消旧采集、阻止旧结果回写。新 Key 重新 `/me` 后，同 owner 保留阈值、账户冷却与既有绑定，不按换 Key 清零；不同 owner 才失效旧身份事实和会话黏性。无需为保留短 TTL 新增账户共享缓存：可暂时返回 Unknown，重新获取真实窗口。配置了使用上限而相应窗口未知时，按既有契约暂停；未配置上限仍沿用既有未知额度策略，不显示满额或保证不会越线。

**缺失与费用。** `/me` 成功但额度不可读时，可报告已验证账户及额度 Unknown；不能清除 Key。账户无法验证时，明确显示未验证，不启用声称去重的订阅聚合。Velune 不修改 Kimi 侧的加油包开关；开启后服务可能在额度用尽时继续扣充值余额。窗口准入不等于硬性账单保证，也不能以 wallet 缺失证明不会扣费。

## 验收判据与实现顺序

1. 先贯通公共组合类型、模板和保存校验，核对 Messages 最终 URL。保留原 Native/Codex/Claude 的可用组合；错误域、错误路径和不匹配组合在边界拒绝。
2. 实现账户/额度适配及 generation 生命周期。合成验证比例非法、无 reset、未知窗口、月/钱包缺失、401/403 读取失败、取消及旧任务不回写。两个 Key 指向同 owner 时不能形成独立可用候选；换 Key 不解除阈值或冷却。
3. 实现 profile 内错误分类及 gateway 中立消费。合成证明 quota 403 可提交前切换，权限/风控 403、套餐 401、禁用类 500 停止，未知错误不被过度分类；取消与已提交响应不重放。未触发真实限额来制造故障。
4. 同步 application-api、bindings、平台模板消费者及必要说明。最小接入沿模板目录展示，保留平台原有编辑结构；需要平台源码时在实施范围中明确，不默认扩张为独立界面重做。
5. 运行规定的 fmt/check/clippy/build 与有界合成人工脚本。真实文本、思考工具往返和跨协议请求尚未获本轮授权；实施后须明确调用范围再进行。当前两次只读 GET 只证明账户与额度接口权限，不充当推理验收。

## 当前实施

2026-10-20 从 main `ea7362b` 干净工作区开始。transport owner负责 ai/ai-provider/gateway 的profile、语义分类和派发；source owner负责 application-api/application/bindings 的模板、身份、额度与绑定；platform owner负责Mac既有编辑器与Windows既有API表单的最小模板接通，Android没有提供商配置入口不扩展；manual owner负责合成人工脚本，主执行者负责整合、权威文档、最终静态/构建与Mac安装。

本轮实现和合成HTTP/应用验收已授权，不读取真实秘密或调用真实模型；此前只读接口成功结果作为外部权限证据复用。未更改配置结构前不因新增模板名机械提升schema，不为短期身份cache引入账户注册表。平台已有模板消费者需要的最小改动属于接入，不重做界面。

实施中确认的必要收口：现 ResponsesClient 只保护 Responses，Chat/Messages 的独立消费者可传入允许重定向的裸 Client。将现有包装提升为三协议共享 HttpClient，合并 gateway 双池，构造末尾强制禁用 redirect/retry，不保留旧类型 alias；目标校验仍归 profile。该变化保护独立 lib 的实际凭据目标，不增加另一套 Kimi 构造。

Android 虽无提供商配置页面，完整业务快照携带 profile/quota 枚举，旧 JNI 会在 Kotlin 接收前反序列化失败。因此需重建常规 Android native/生成绑定与 APK，不新增 Android 界面。

## 实现与验证结果

模板、公共枚举、固定目标校验、身份与额度采集、错误分类和平台消费者已实现。身份声明、去重与发布在同一状态锁内完成；credential generation 阻止换 Key 前的请求和结果继续生效，运行期实例标识另阻止删除后以相同 ID、Key 和 generation 重建时接收旧采样。公开账户标签使用区域与 user_id 的散列短尾，不暴露账户标识。上述身份约束属于显式 Kimi 订阅组合；通用 Native API 提供商仍允许用户配置服务地址，不按 URL 推断品牌。

2026-10-20 合成人工验收结果：

| 范围 | 结果与证据边界 |
| --- | --- |
| Kimi HTTP 与 application 主矩阵 | 38 组通过：三协议目标、原状态透传与服务错误分类、冷却/切换、重定向第二跳零命中、隐式重试零重放、取消与提交闸门、跨网关账户去重、换 Key 后 cap/cooldown/sticky、换账户失效、旧任务与删除重建、窗口解析、保存重开 |
| Kimi 读取失败补验 | 6 组通过：`/me` 401/403 时保留 Key、未验证且零模型请求；`/me` 成功而 `/usages` 401/403 时账户已验证、额度 Unknown，无 cap 可用、有 cap 暂停。复用主矩阵已编译的隔离库，仅重编临时 runner；相同场景已纳入主脚本 |
| 原生 Messages 回归 | 8 次回环 HTTP 请求通过，覆盖 JSON、SSE、工具、思考、错误与 headers，直接消费方已改用共享 HttpClient |
| Claude 工具生命周期回归 | `manual-subscription-composition.py --case cancel` 3 组通过，覆盖挂起工具的取消/结束、新轮恢复、迟到结果拒绝及已完成会话跨 Runner 复用 |
| Codex 订阅回归 | `manual-codex-subscriptions.py` 24 组通过，覆盖认证、续期、额度、限额/路由和 bindings 门面的保存重开；修正人工脚本对公共 API 拆包后类型及临时源码依赖的引用 |

最终 Rust fmt、全工作区 all-features check、全 targets/all-features clippy（`-D warnings`）、bindings debug/release build 与 application/bindings 无默认 feature 检查通过。Mac 已通过完整 Rust/生成 Swift 绑定/SwiftPM 构建及严格签名校验。Android 已执行完整 native/Kotlin 绑定重建、`assembleDebug` 和 `lintDebug`，并覆盖安装到已连接手机，未清除数据；APK SHA-256 为 `e5ea84aeda514347b4c5a690c37c40c387c281ffbbe7116787b9d900272d43a9`。Windows 表单源码在 Mac 宿主 check/clippy 通过，尚无 Windows 原生构建和运行证据。

本轮真实模型请求、登录与秘密读取均为零。实现前的两个只读 GET 不替代真实文本、思考工具往返、跨协议或海外区域验收；本轮也未制造真实额度耗尽。

交付：实现提交为本地 main `1444d56c871038ffaf0f4ba0d2e67baaae8ad4b3`。从该干净提交重新构建 Mac 并安装至 `/Applications/Velune.app`（`0.1 beta.1`）；构建清单 source_commit 与该提交一致、dirty=false、配置 schema 仍为 10。严格签名校验通过，安装后的可执行文件和 Rust 动态库 SHA-256 与构建产物逐一匹配。安装时无运行中的 Velune，不读取应用配置或真实会话；不推送或发布。额度 403 的原始 body bytes、status 和合成错误 header 另以已编译隔离库复验通过，主脚本保留相同断言。
