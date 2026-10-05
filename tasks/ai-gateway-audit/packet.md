# AI 服务与提供商配置：当前 Task Packet

## 当前状态与目标

2026-10-05：原生协议、认证边界与模型 ID 重构已完成一轮安装验收。用户的新反馈要求先厘清提供商、模型参数和模板的归属，当前处于需求与设计复核，尚未实施最新配置结构。此前未提交的表单／文件凭据实验已撤回，产品源码保持已交付基线。

当前目标是让用户在提供商及其模型配置中完成接入，允许修改协议、服务地址、API model ID 和 API key，并通过模型模板减少重复填写。独立认证建档与全局模型关联流程不作为后续固定前提。

长期产品意图归 [PRD](../../docs/prd/index.md#ai-网关与-agent-运行时配置)，设计及现有实现差距归 [AI 服务设计](../../docs/design/ai-service.md)。本文件只维护当前决定、证据与待办，不继续累积整段历史交付日志。

## 用户已确认

- AI 服务不限于 LLM；当前 gateway 仅为 LLM Gateway，负责同协议透传、路由与 fail-over，暂不翻译协议。AI 服务不依赖 Agent Harness。
- model ID 是提供商 API 标识，表单应显示并允许编辑。内部引用和网关别名必须与它区分。
- 提供商协议和服务地址可修改。API key 可查看、编辑，Keychain 与独立认证管理页不是产品要求。
- 同一真实模型可以跨提供商存在；其产品价值是复用模型参数模板，快速填入，避免每次手填。具体提供商实际能力仍有差异。
- 当前阶段重构 hard-cutoff，不保留旧契约、迁移或旧普通配置备份。原 Harness 文件及会话不删除。
- 不新增或保留自动化测试。使用静态检查、构建和隔离人工端到端验收；真实配置、登录、调用及 GUI 体验由用户验收。

## 候选结构与未决定事项

建议界面采用提供商列表 → 提供商详情 → 模型编辑，认证随提供商编辑，模型可从模板添加或手动配置。提供商条目保存实际 API model ID 与能力，模板提供填写起点；当前请求的 effort 与支持的 effort 集合分别呈现。

“选择模板后生成可编辑快照，模板更新不自动覆盖已有配置”是助手建议，尚未确认。模板来源、维护／编辑入口、模板更新行为仍待讨论；不据此预建模板同步、发现框架或全局模型路由实体。订阅认证保留必要登录与刷新能力，具体存储与新引用结构随配置方案复核。

## 已验证事实与安装基线

安装包源码为 `6efbcc6d248f0673494985ea420fada8acd64b41`，clean、schema 4、版本 0.1 beta.1，位于 `/Applications/Velune.app`。最新文档提交可能晚于安装源码，不表示应用已实现新方向。

当前 ModelEditor 只编辑全局模型名称／规格；实际 model ID 与可用推理等级在提供商绑定表单，等级又藏在 Disclosure。当前还有中央认证资源页与 Keychain 流程；它们是待重构实现，不是最新需求。

已交付源码通过 workspace fmt/check/clippy、bindings 裁剪检查、严格 Swift typecheck、发行构建与签名验证。隔离人工验收覆盖 9 个原生 HTTP JSON／SSE／状态头边界、取消、schema 重置无备份、内部记录键与提供商 ID 分离，以及正常 Pi 导入 → 路由 → 工具续接 → 下一轮与 helper 清理。没有真实服务、秘密或 GUI 验收。

第一次安装验收发现 descriptor 额外字段与严格 DTO 不一致，已改为具名类型构造后重建验证通过。完整证据归 [模型实施记录](model-implementation.md)。

## 当前责任与下一步

主执行者维护产品／设计归属与本 packet，先与用户厘清配置结构，再按明确范围实施。Mac owner 与 advisor 已反馈本轮表单缺口和结构判断，源改动已冻结；不把已启动实现当作需求冻结。

下一步要确定模板与实际提供商模型的最小契约，并将最新方向应用到配置、导入、路由选择与原生表单。实施前更新权威设计；实施后静态检查、隔离端到端验证并重建安装。模板复用价值已经明确，无需再次询问它是否必要。

## 证据导航

- [最初实现审计](audit.md)：重构前偏差与源码证据。
- [模型业务研究](model-business-research.md)：官方资料与历史误建模；其旧实体推荐已被后续反馈修订。
- [原生协议实施](native-implementation.md)、[认证实施](authentication-implementation.md)、[模型实施](model-implementation.md)：分别对应历史源码和验收边界。
- [AI SDK provider](https://ai-sdk.dev/providers/openai-compatible-providers) 与 [OpenAI reasoning](https://developers.openai.com/api/docs/guides/reasoning)：2026-10-05 核对，支持 provider 地址／key、API model ID 和请求 effort 的区分；不规定 Velune 的 CRUD 页面或模板更新规则。
