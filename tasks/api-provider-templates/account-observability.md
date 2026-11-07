# OpenAI／Anthropic 账户查询：接口调研与架构建议

核对日期：2026-11-02。本文件保留最初的接口调研。用户随后已授权实施；当前契约和 Mermaid 由 [AI 服务设计](../../docs/product-tdd/ai-service.md#组织用量与账单)维护，进度与验证见 [任务入口](packet.md#组织账单改造实施2026-11-02)。调研和实现验证均未使用真实账户或凭据。

## 能查询什么

| 观测 | OpenAI Platform | Anthropic API |
| --- | --- | --- |
| 充值余额／剩余赠金 | 本轮未发现公开、文档化的官方查询接口；官方账单页提供余额管理 | 本轮未发现公开、文档化的官方查询接口；官方账单页提供余额查看 |
| 组织用量与费用 | 官方 Usage／Costs API，需要 Admin API key | 官方 Usage and Cost Analytics API；当前 beta，需 Admin API 凭据；workspace-scoped 模型 Key 不适用 |
| 支出限额 | 可查询组织的官方月度 hard spend limit；与实际剩余资金分别表达 | 本轮确认的 Rate Limits API 读组织／workspace 的 RPM、输入／输出 TPM 等速率限制；Enterprise 成员 Spend Limits 是另一种产品，不能当作 Console API 账户余额 |

OpenAI 的 Admin Key 与 Anthropic 的 Admin API 凭据均具有组织级权限。Anthropic 当前还允许 org:admin OAuth token 或未绑定 workspace 的个人／service-account key，权限由其身份决定；Admin API 对个人账户不可用。产品不能承诺随便填一个推理 Key 就能查账单。

已核实的接口：

- OpenAI：`GET /v1/organization/usage/completions`、`GET /v1/organization/costs`。Costs 当前按天返回，可按 project／API key 等范围过滤或分组。
- OpenAI：`GET /v1/organization/spend_limit`，返回 USD 分币数额、月份 interval 和 enforcement 状态；状态不能直接当作“当前额度耗尽”。金额阈值也不是当前余额。
- Anthropic：`GET /v1/organizations/analytics/usage` 与 `GET /v1/organizations/analytics/cost`；当前需 `anthropic-beta: analytics-usage-cost-2026-08-24` 与版本头，返回统计时段及数据刷新时间。新接入推荐该 API，旧 `usage_report/messages`／`cost_report` 为 legacy。
- Anthropic：`GET /v1/organizations/rate_limits` 以及 workspace rate-limits 查询，读取配置上限，不等于读取此刻剩余量。

“月上限－月费用”最多是同范围、同周期、同币种下依据报表计算的预算差额，存在统计延迟；不能命名为充值余额，也不能据此声称实时账单硬截断。Credits、退款、赠金、充值与其他客户端消耗均不能仅靠 Velune 的模型 token 计数还原。

## 架构归属

批准后的实现与 Mermaid 已整合到 [AI 服务设计](../../docs/product-tdd/ai-service.md#组织用量与账单)，产品语义归 [组织用量与账单 PRD](../../docs/prd/ai-providers.md#组织用量与账单)。以下保留当时的最小演进建议，当前源码与权威文档优先。

## 建议的最小演进

1. 保持调用配置与观测来源独立。官方模型模板不自动启用 Admin 查询；在提供商下可选配置组织账单来源和查询认证，不新增要求用户另行建档的账户／中央认证实体。
2. 为账单定义独立的结果类型：费用 `BilledCost`、用量 `Usage`、支出限额 `SpendLimit`；与现有余额 `Balance`、订阅窗口分别表达。速率限制另有单位和适用模型组，不能全部压成一个“剩余百分比”。
3. 把现有仅支持 ARK AK/SK 的查询认证结构扩为有类型的查询凭据，显式区分模型凭据引用、独立 Admin Key 与 AK/SK。查询凭据只由 application 的固定官方目标消费，不发给 Harness、模型接口或用户修改后的中转地址。
4. 每条报表带组织／project／workspace／API key 范围、币种或计数单位、统计区间与新鲜度。一个组织由多个提供商引用时，组织总额不能分别计为每个提供商独有的费用，也不能在汇总时重复相加；查询和缓存按实际 scope 复用。该 scope 是事实关联，不要求增加可独立建档的账户实体。
5. 首批只读展示官方用量、费用和可取到的支出限额；报表失败或过期保持未知，不置零，不因缺少报表阻止模型调用。账单报表暂不进入网关准入；余额、窗口耗尽及上游明确拒绝继续沿用现有机制。若以后希望按预算暂停请求，应另行明确周期、范围、延迟与超支容忍。

推荐先做这条官方、可选的只读查询路径。若目标必须是“当前实际剩余充值余额”，现有公开证据不足以承诺接入；控制台会话／Cookie 抓取涉及新的认证生命周期与接入稳定性，不能当作给现有 `ApiAccount` 增加两个域名即可完成。

## 事实依据

- [OpenAI Usage／Costs 官方示例](https://developers.openai.com/cookbook/examples/completions_usage_api)：Admin Key 与两类历史接口。
- [OpenAI Costs API](https://developers.openai.com/api/reference/resources/admin/subresources/organization/subresources/usage/methods/costs)：聚合周期、范围过滤与分组。
- [OpenAI 组织支出限额读取](https://developers.openai.com/api/reference/resources/admin/subresources/organization/subresources/spend_limit/methods/retrieve)：阈值与 enforcement 字段。
- [OpenAI 支出控制语义](https://developers.openai.com/api/docs/guides/spend-limits)：提醒、hard limit、官方 usage limit 与资金余额分别处理，执行存在传播延迟。
- [OpenAI 预付账单说明](https://help.openai.com/en/articles/8264644-setting-up-and-managing-prepaid-api-billing)。
- [Anthropic Usage and Cost Analytics](https://platform.claude.com/docs/en/manage-claude/usage-cost-analytics-api)：新路径、beta、凭据与数据刷新信息。
- [Anthropic Admin API](https://platform.claude.com/docs/en/manage-claude/admin-api)：可用性、组织权限与凭据类型。
- [Anthropic Rate Limits API](https://platform.claude.com/docs/en/manage-claude/rate-limits-api)：组织／workspace 上限与继承。
- [Anthropic 账单说明](https://support.claude.com/en/articles/8977456-how-do-i-pay-for-my-claude-api-usage)。
- [当前采集与缓存](../../packages/application/src/subscription_quota.rs)、[API 余额适配](../../packages/application/src/api_account_quota.rs)、[查询认证](../../packages/application/src/quota_authentication.rs)、[网关只读契约](../../packages/gateway/src/scheduling.rs)。
