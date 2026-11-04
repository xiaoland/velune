# Velune 跨 unit 技术契约

<a id="velune-product-tdd"></a>

本入口收录两个以上 unit 必须一致遵守、单靠代码类型不足以说明的权威归属、生命周期与失败语义。产品行为归 [PRD](../prd/index.md)；单个 unit 的内部设计归其 README。技术设计存在不等于相关平台或真实服务已完成验收，已知缺口见[任务记录](../../tasks/open-items/packet.md)。

## 按问题读取

| 要解决的问题 | 契约 |
| --- | --- |
| 哪些产品规则由核心统一，平台能保存什么 | [核心与平台分工](../design/platform-contracts.md) |
| 包依赖、完整端／控制端装配、唯一执行 owner | [架构](../design/architecture.md) |
| Harness 如何接入、HOME／SSH、进程寿命与网关注入 | [运行时接入与执行](../design/runtime-execution.md) |
| 会话投影、跨 Harness 接续、A2A、控制权与定时派发 | [会话与协作](../design/conversations.md) |
| 输入内容、目标配置、跨设备修订与新建交接 | [Composer 输入身份与同步](../design/composer-state.md) |
| AI 操作、认证、路由、额度、协议转换与分析 | [AI 服务与网关](../design/ai-service.md) |
| 网关切换、协议保真与 token 计量 | [LLM 网关](../design/ai-gateway.md) |
| 组织报表、账单缓存与查询凭据 | [组织用量与账单](../design/ai-billing.md) |
| 配对、设备身份、网络恢复与远程请求交付 | [远程访问](../design/remote-access.md) |
| SDK 装配、关联、指标、分享与本地问题历史 | [遥测](../design/telemetry.md) |

## 约束与验证

共享 packages、平台 app 和远控对端必须遵守同一产品状态与业务接口。`bindings` 类型说明“有什么”，但不能阻止平台用本地状态重算规则；评审仍须核对[核心拥有的产品状态](../design/platform-contracts.md#平台-app-与-application-的分工)。类型与绑定的兼容性通过生成和各端构建验证。

<a id="admission"></a>
<a id="平台-app-与-application-的分工"></a>
<a id="验证"></a>
分工及其检查见[核心与平台分工](../design/platform-contracts.md)。

<a id="跨-unit-契约文档"></a>
契约目录见上方[按问题读取](#按问题读取)。

<a id="会话输入准入与调度"></a>
<a id="自动命名兜底"></a>
输入准入、定时派发与自动命名归[会话契约](../design/conversations.md)。

<a id="artifact-与编辑区文稿"></a>
Mac 编辑文稿与 renderer 的专属约定见[Artifact 与编辑区文稿](../design/platform-contracts.md#artifact-与编辑区文稿)。
