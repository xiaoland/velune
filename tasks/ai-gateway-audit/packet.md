# AI 服务与提供商配置：当前 Task Packet

## 当前状态与目标

2026-10-05：原生协议、认证边界与模型 ID 重构已完成一轮安装验收。用户的新反馈要求先厘清提供商、模型参数和模板的归属，用户已认可参数归属并授权开工，提供商编辑器和配置结构已贯通，正在完成发行安装。此前未提交实验已撤回；新切片以已交付源码为基线，尚未重新安装。

当前目标是让用户在提供商及其模型配置中完成接入，允许修改协议、服务地址、API model ID 和 API key，并通过模型模板减少重复填写。独立认证建档与全局模型关联流程不作为后续固定前提。

长期产品意图归 [PRD](../../docs/prd/index.md#ai-网关与-agent-运行时配置)，设计及现有实现差距归 [AI 服务设计](../../docs/design/ai-service.md)。本文件只维护当前决定、证据与待办，不继续累积整段历史交付日志。

## 用户已确认

- AI 服务不限于 LLM；当前 gateway 仅为 LLM Gateway，负责同协议透传、路由与 fail-over，暂不翻译协议。AI 服务不依赖 Agent Harness。
- model ID 是提供商 API 标识，表单应显示并允许编辑。内部引用和网关别名必须与它区分。
- 提供商协议和服务地址可修改。API key 可查看、编辑，Keychain 与独立认证管理页不是产品要求。
- 同一真实模型可以跨提供商存在；其产品价值是复用模型参数模板，快速填入，避免每次手填。具体提供商实际能力仍有差异。
- 当前阶段重构 hard-cutoff，不保留旧契约、迁移或旧普通配置备份。原 Harness 文件及会话不删除。
- 不新增或保留自动化测试。使用静态检查、构建和隔离人工端到端验收；真实配置、登录、调用及 GUI 体验由用户验收。

## 当前实施结构

界面采用提供商列表 → 提供商详情 → 模型编辑，认证随提供商编辑，模型可从模板添加或手动配置。提供商条目保存实际 API model ID 与能力，模板提供填写起点；当前请求的 effort 与支持的 effort 集合分别呈现。

模板快填复制独立配置快照，不自动传播修改；这是本轮最小实现选择。模板从既有模型保存或手动填写，不预置厂商目录，不复制认证、地址、隐藏引用或 Pi 投影。认证随提供商原子保存，编辑明确区分 Keep／SetApiKey／Clear，单条显式读 key。API key 存 application 私有文件且不进公共描述与日志；OAuth 保留必要来源登录／刷新。

## 已验证事实与安装基线

安装包源码为 `6efbcc6d248f0673494985ea420fada8acd64b41`，clean、schema 4、版本 0.1 beta.1，位于 `/Applications/Velune.app`。最新文档提交可能晚于安装源码，不表示应用已实现新方向。

上述安装基线有全局模型、中央认证与 Keychain 流程；工作树的新契约已删除它们，以提供商及其模型的两栏原生编辑器替代。能力／effort 摘要可见，次要字段展开编辑。

已交付源码通过 workspace fmt/check/clippy、bindings 裁剪检查、严格 Swift typecheck、发行构建与签名验证。隔离人工验收覆盖 9 个原生 HTTP JSON／SSE／状态头边界、取消、schema 重置无备份、内部记录键与提供商 ID 分离，以及正常 Pi 导入 → 路由 → 工具续接 → 下一轮与 helper 清理。没有真实服务、秘密或 GUI 验收。

第一次安装验收发现 descriptor 额外字段与严格 DTO 不一致，已改为具名类型构造后重建验证通过。完整证据归 [模型实施记录](model-implementation.md)。

## 当前责任与下一步

主执行者维护文档、人工脚本、独立端到端验收与安装；原 Mac owner 扩展为 packages 与 app/mac 的源码 owner，负责核心贯通与静态验证；唯一 portable.rs 已交给 root，调用其已建立的共享纯配置编辑逻辑，避免复制约束。advisor 负责关键契约判断，不作为代码 reviewer。旧 Rust owner 及新 worker 均受 agent thread limit 阻止，停止重试，转交稳定 owner；共享 guides/delegation.md 不存在，按现有责任原则执行。

[编辑器功能设计](provider-editor-design.md)与具名 DTO 已贯通。新用户先保存运行时再导入的顺序已实现，首个 default 运行时在同一次保存内部创建空默认网关，不新增网关建档 API。主要字段直接可见，能力与 effort 摘要可发现，来源详情和模板维护按需展开；不以放大窗口代替信息层级。

独立配置手动脚本在 no-default-features 动态库上已通过 schema 5 hard-cutoff、0600、key 显式读／修改／重开、公开摘要和日志脱敏、协议／地址／ID 编辑、稳定内部引用、模板快照不传播、坏配置不覆盖及原来源文件保留。local debug 的合成 Pi 五请求已通过导入→工具续接→下一轮；修改后的 key、API ID 与地址在真实 HTTP 路径生效，旧预览拒绝覆盖新认证，跳过不断连而替换断开旧网关。原生 gateway 的 9 个 HTTP JSON／SSE／状态头边界与断连取消已通过。以上只使用临时目录、合成来源与 loopback 上游，不读取真实配置或调用真实提供商。

workspace fmt/check、bindings 裁剪检查和完整 Swift warnings-as-errors 已通过；workspace 与裁剪版严格 clippy 也已通过，源码已冻结；接下来提交、构建 clean 发行包、安装并在已安装库上复核两条手动路径。GUI 与真实服务验收由用户执行。

## 证据导航

- [最初实现审计](audit.md)：重构前偏差与源码证据。
- [模型业务研究](model-business-research.md)：官方资料与历史误建模；其旧实体推荐已被后续反馈修订。
- [原生协议实施](native-implementation.md)、[认证实施](authentication-implementation.md)、[模型实施](model-implementation.md)：分别对应历史源码和验收边界。
- [AI SDK provider](https://ai-sdk.dev/providers/openai-compatible-providers) 与 [OpenAI reasoning](https://developers.openai.com/api/docs/guides/reasoning)：2026-10-05 核对，支持 provider 地址／key、API model ID 和请求 effort 的区分；不规定 Velune 的 CRUD 页面或模板更新规则。
