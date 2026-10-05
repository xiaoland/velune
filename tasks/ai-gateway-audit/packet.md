# AI 服务与提供商配置：当前 Task Packet

## 当前状态与目标

2026-10-05：提供商所属模型与私有认证、参数模板以及原生两栏编辑器已实施并重新安装。静态检查、发行构建和已安装动态库的隔离人工端到端验收通过；GUI 与真实提供商验收仍由用户执行。

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

安装源码为 `81f0fec5fa153d7ed38fd728ec67225262374b8e`，clean、schema 5、版本 0.1 beta.1，位于 `/Applications/Velune.app`。本任务后续文档提交不改变已安装源码。发行包深度严格签名验证通过；安装前按已有授权退出确认的 Velune 进程，未启动应用或打开真实配置。

`cargo fmt --all --check`、workspace all-targets all-features check／clippy（`-D warnings`）、bindings no-default-features check／严格 clippy，以及全部十个 Mac Swift 文件的 warnings-as-errors 类型检查均通过。发行 Rust／Swift 构建、Markdown 相对链接和 diff 检查通过。没有自动化测试、真实秘密读取或真实服务调用。

[提供商配置手动脚本](../../scripts/manual-provider-configuration.py)在配置裁剪版和已安装完整库均通过：schema 5 hard-cutoff、0600、key 显式读／修改／重开、公开摘要和日志脱敏、协议／地址／ID 编辑、稳定内部引用、模板快照不传播、坏配置不覆盖及原来源文件保留。先保存运行时再导入可行，默认空网关在运行时保存内部初始化。

[Pi 首循环手动脚本](../../scripts/manual-pi-native-loop.py)在已安装库用固定 Pi SDK 和临时 HOME 完成五次 loopback HTTP 请求：导入→工具续接→下一轮，随后修改 key、API ID 与地址并确认实际派发。旧预览拒绝覆盖新认证，重复跳过保持连接，替换断开旧网关；原来源文件未变。合成原生 HTTP 脚本另通过九个 JSON／SSE／状态与安全头边界、裸内部键拒绝及断连取消。真实订阅登录／刷新和 GUI 尚未验收。

## 当前责任与验收边界

主执行者维护文档、人工脚本、独立端到端验收与安装。稳定源码 owner 完成 packages 与 app/mac；root 接管 portable.rs，共用配置编辑约束。advisor 用于关键契约判断，不作为 reviewer。多代理数量限制时已转交稳定 owner，没有重复创建任务。

[编辑器设计](provider-editor-design.md)已落地：提供商内选择连接或单个模型，主要字段直接可见，能力／effort 摘要可发现，来源详情与模板维护渐进披露。窗口尺寸只辅助，不替代信息层级。OAuth 委托只消费原认证存储，不再依赖原 Pi 模型目录；未真实登录验证，不把合成 API key 路径当成订阅证明。

实现范围已完成，下一步是用户从 Applications 打开应用，验收原生布局、编辑与真实服务。新反馈继续更新本任务，不冻结需求。

## 证据导航

- [最初实现审计](audit.md)：重构前偏差与源码证据。
- [模型业务研究](model-business-research.md)：官方资料与历史误建模；其旧实体推荐已被后续反馈修订。
- [原生协议实施](native-implementation.md)、[认证实施](authentication-implementation.md)、[模型实施](model-implementation.md)：分别对应历史源码和验收边界。
- [AI SDK provider](https://ai-sdk.dev/providers/openai-compatible-providers) 与 [OpenAI reasoning](https://developers.openai.com/api/docs/guides/reasoning)：2026-10-05 核对，支持 provider 地址／key、API model ID 和请求 effort 的区分；不规定 Velune 的 CRUD 页面或模板更新规则。
