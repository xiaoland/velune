# AI 网关与 AI 服务需求复核及实现审计

## 审计阶段目标与授权

2026-10-05 用户要求重新整理整个 AI 网关的需求、审计实现，必要时覆盖整个 AI 服务。名称为 AI 网关，不将其限定为 Harness 的专属网关。本轮交付需求基线、实际实现证据、偏差清单与重构建议；不直接实施协议迁移或功能扩张。产品源码基线为 f7d4b0d；安装产品仍为 7345ba8 的 clean 0.1 beta.1。

## 已确认边界

AI 网关只承担同协议原生透传、模型路由与 fail-over，当前不做协议转换／翻译。AI 服务为独立 lib，不限于 LLM，sampling 是其中一项操作；Agent 运行时与 AI 服务保持独立。配置由 application 管理和装配，领域包不依赖 UniFFI。读取运行时提供商配置属于导入适配，不能让来源适配能力冒充网关协议保真。

现有导入验收失败见 [来源验收](../pi-mac-first-loop/deepseek-import-acceptance.md)。正常导入被阻断，隔离诊断确认请求、历史与推理流内容不能完整经过 sampling 重建路径；本轮审计不止于这一个模型或协议字段。

## 责任与计划

主执行者整理用户需求、知识归属与最终判断；原网关 owner 审计 gateway、application 装配及配置、运行时来源边界；独立 owner 审计 ai 与 ai-provider 的契约和实际执行；advisor 从已确认约束推导目标职责与关键决策。审计 owner 只读产品源码并返回定位证据，主执行者整合权威文档。

逐项核对协议保真、路由与 fail-over、提供商／模型身份、认证委托、配置快照、流式终态、取消、资源限制和可观测性。区分架构偏差、尚未实现的产品能力、历史有界原型与无证据的假设，不把每个未实现项都判为错误架构。

## 验证与交付

不新增或运行自动化测试，不读取真实配置、凭据、会话，不调用真实模型。引用现有人工端到端证据；必要的新实验使用隔离合成输入。文档检查相对链接、diff 与改动归属；审计不改 Mac 产品源码，不重新安装。重构方案需给出可独立验收的切片，以及确需用户决定的实质范围选择。

## 审计结果与交付

[完整审计](audit.md)覆盖 AI／provider、AI 网关、application 配置与认证装配，并给出需求对照、源码行号、保留／替换建议和五个可独立验收的迁移切片。Chat 的采样桥、MiniMax decoder 耦合与提前提交 HTTP 200 是已确定偏差；Responses HTTP envelope、跨操作观测、认证与关闭有界性也需修正。

最小正常 API 实验进一步确认只选择 A 时实际写入未选 B 的空 provider；正确 shutdown 后退出 0、临时目录清理、原合成来源不变，无真实调用。将该具体事务缺陷与架构重划分分开，不扩大成重做整个导入 UI。Messages 未实现但没有作为受支持协议暴露；Responses 终态后的流完成条件缺少实际异常证据，只列待验证，不制造审计结论。

已更新 PRD、AI 服务权威设计、跨单元架构与导航，标出当前能力与目标的差异，修正 README 的原生 Chat 能力误述。已确认原生透传无需重问；fail-over 的显式候选范围及不确定提交是否可重放仍需具体产品策略。下一步可先实施原生操作／HTTP envelope 与单次执行切片；本轮未授权自动推进全部重构，没有源码或安装变化。

交付检查：9 份改动 Markdown 的 133 个本地链接及标题锚点全部可解析，`git diff --check` 与暂存区检查通过。文档改动不执行产品构建或自动化测试。

后续术语修订：用户指出执行应耦合协议而非具体服务商。核对后明确原 `Decoder` 实际是已解析 JSON 到 sampling 的映射／组装器；修正权威设计与审计，区分 HTTP／SSE／JSON 解析和有损采样投影。服务商作为配置数据与通用协议代码依赖服务商模块是两件事。本次仍不修改产品源码。

后续业务边界修订：用户允许 `SamplingOutput`，要求可观测性与业务数据分开，处理方向为 HTTP → 原生协议 → messages、outputs／stats → 可选业务投影。核对现有代码发现 Usage 位于 observation 模块，SamplingCompletion 又聚合 outcome 与观察对象；后续重构应将协议统计归业务合同，将耗时、尝试与执行阶段独立关联。不由可选投影推导必须保留独立 sampling 执行层。

进一步澄清：usage、finish reason 可以同时是业务数据和观察对象。分离的是业务与观察职责，而非排他划分字段；观察消费协议数据，不接管其定义或改变返回数据。已在权威设计补齐请求、原生返回、可选投影与观察分支，保持现有 units，不引入额外观察 package。

范围修订：当前 gateway 仅为 LLM Gateway，是不限于 LLM 的 AI 模块的一种应用模式。更新 PRD 与权威设计，明确直接 AI 消费可绕过 gateway；独立 gateway unit 仍可保留，不把应用模式误解为必须合包。早先审计“AI 网关”名称按当前有界 LLM 范围理解，源码审计事实不变。

## 重构实施授权与当前切片

用户在职责复核后明确授权推进，采用持续迭代而非冻结需求。本轮实施原生 ChatCompletions／Responses 执行与 HTTP 响应保真，退出网关 sampling 重建，分离业务数据和运行观察归属，并修复必要的导入选择、来源兼容与有界认证／退出链路。AI／provider owner 负责领域及单次执行，gateway owner 负责 LLM Gateway 与必要 application／Pi 适配，主执行者负责跨包装配验收、文档与 Mac 构建安装。五个迁移切片不是顺序审批关卡，依证据持续调整。

自动 fail-over 的候选和不确定提交重放策略未确认，继续 Disabled；不调用真实服务、不读取真实配置凭据或会话，不新增自动化测试。静态检查优先，使用隔离合成上游、已构建语言绑定及固定 Pi SDK 的临时端到端验收。修改后重建并安装 Mac，用户负责 UI 与真实资源验收。

## 本轮实施交付

源码 `22a15c0` 已完成原生协议切片，类型、全 workspace 静态检查、裁剪检查和 Mac 构建通过；已安装同版 clean bundle，版本为 0.1 beta.1。9 个合成原生 HTTP 边界、断连／helper 进程树清理，以及使用安装库的正常配置导入到 Pi 工具续接、下一轮消息均通过。具体命令、证据与未覆盖边界归 [实施记录](native-implementation.md)。无真实调用、凭据或用户会话读取。

该切片退出了有损 sampling 桥并补齐当前本机原生协议入口；不是整个审计清单已经关闭。后续继续处理配置生效／资源所有权、明确的 fail-over 策略和平台能力，依据真实反馈迭代，不按固定瀑布计划冻结需求。

## 认证边界修复与集中管理

用户实际导入后指出认证方式显示 Pi Agent，进一步要求统一集中管理。检查确认不止文案：application 的 provider 嵌入 Harness CredentialSource、UniFFI／Mac 公开路径和设置，gateway 接收来源 JSON，AI-provider 留有同步来源解析设施。上一轮原生执行重构未守住这条认证边界。

本轮按反馈实施集中认证资源：application 认证表管理来源、允许目标与生命周期，provider 仅选择已登记 ID；gateway 注入异步 resolver，AI-provider 只用短生命周期已解析认证。Settings 提供独立原生认证管理页，提供商只选资源，不填写原路径或任意 Keychain 名称。API key 创建／替换和委托登录、未被引用资源删除经过同一管理用例。Schema 2 非秘密迁移将旧来源及手工 Keychain 引用包装成独立管理资源，不按路径合并、不读取原 auth、不触碰秘密，不建立新 package 或框架。

AI/provider owner 清理领域残留，gateway owner 负责 application／bindings 与 resolver 接入、迁移，独立 Mac owner 负责原生 UI 与平台秘密写入流程；主执行者负责迁移及正常应用端到端验收、文档与重建安装。使用隔离合成配置与 loopback，不新增自动化测试。原存储与 refresh credential 保留，未知资源及目标不匹配不得 fallback。此修复替代当前 provider 内嵌来源的实现说明，完成后同步持久文档。


## 模型业务概念复核

认证实现期间，用户纠正 model-id 为提供商规定的 API 标识，要求先充分核对 Vercel AI SDK、Mastra、Cloudflare AI Gateway 与 OpenAI 官方资料。已修正 PRD 定义；独立研究 owner 对照一手文档和当前 modelId／externalModelId／route alias 用法，证据归 [模型业务研究](model-business-research.md)。本步为研究与审计，模型源码不改；认证集中管理继续完成必要验证和安装。


集中认证源码已完成：schema 3、shared registry mutation、typed UniFFI、独立认证设置页和异步 resolver 均已接入。全 workspace fmt/check/clippy、无 local-runtime 裁剪构建与完整 Swift typecheck 通过。手动隔离验收已确认 portable 迁移／资源生命周期／目标 guard 及 9 个原生 HTTP 边界；安装包的正常 Pi 导入、工具续接与 helper 取消仍待发行构建后验证。模型业务研究已完成并经 advisor 复核，权威设计记录修正建议；未改模型源码，也不宣称已解决现有 model ID 与参数表单误用。


集中认证交付已完成：`31b394c` 的 clean schema 3 bundle 已安装，版本仍为 0.1 beta.1。portable 与已安装库的 registry／迁移验收、原生 HTTP 9 个边界、安装库 Pi 正常工具循环及实际 source adapter、旧预览拒绝、受管 helper 断开／30 秒期限／关闭整组清理均通过。具体边界和脚本修订见 [认证实施记录](authentication-implementation.md)。没有真实调用、秘密或 UI 验收；模型业务误建模只完成研究与设计修订，尚未实施。


## 模型重构实施授权与 hard-cutoff

用户要求立即修正严重的模型业务缺陷，并明确当前阶段所有重构 hard-cutoff。本轮更新领域／原生协议调用、网关目标解析、application 配置、导入、Pi 投影与 UniFFI／Mac 表单，内部记录键隐藏自动生成，提供商模型标识和路由入口分开，参数归实际提供商绑定。schema 4 只支持新契约，删除旧字段及迁移分支；旧普通配置直接原子重置到 schema 4，不保存备份，不转换旧字段；原 Harness 文件、会话与 Keychain 秘密不删除。上游 Pi 协议 compat 的来源语义仍保留，不把它当 Velune 旧版兼容代码删除。

Rust owner 延续共享 packages，Mac owner 延续 UI／绑定接入并修复 Disclosure 内容额外缩进；root 负责文档、合成人工脚本、全静态检查、发行构建安装。此前 advisor 已裁决模型真实业务概念、record key 与 alias 分开、binding 实际能力权威，不新增 Capability／Offering 实体。本轮额外 advisor 调用受 agent thread limit 阻止，沿用已裁决基线处理具体接口，不阻塞已授权工作。行为验收使用隔离临时目录和合成上游，不自动化测试、不读取真实配置／秘密／会话，不调用真实模型。


模型重构源码已贯通：内部 recordKey 自动生成且 UI 隐藏，精确 providerModelId 归绑定，route 自动产生入口 alias；native provider 只校验原生 body.model 与 ProviderModelId 一致，不二次映射。规格与协议推理声明可未知，Pi 准备时独立检查。Schema 4 reset 删除旧配置内容且无备份，未来 schema／损坏 JSON／新 schema 旧字段仍拒绝。导入显式映射必须指向已有记录，无同名跨提供商自动合并。Mac 完整严格 typecheck 通过，Disclosure 内容改原生 Grid 与 leading 对齐。

portable 的合成 UniFFI 验收已通过配置重置、原文件边界、登记认证资源生命周期、自动记录键及跨提供商独立能力；原生 HTTP 9 个保真边界与取消通过，继续核对裸记录键入口拒绝。旧契约重复临时脚本随 hard-cutoff 删除，历史验收记录保留原日期与源码依据。发行安装与安装包正常 Pi 循环待最终确认。


模型重构最终交付：`6efbcc6` clean schema 4 bundle 已安装，版本 0.1 beta.1。第一次安装验收发现 descriptor 额外 capability 字段与严格 DTO 不一致；已用具名类型构造修复生产端，重新构建安装后，安装库配置／认证与正常 Pi 工具循环均通过。9 个原生 HTTP 保真边界、裸内部键拒绝和未知规格仍可调用通过。验证、失败修正及边界归 [模型实施记录](model-implementation.md)。本轮模型业务和 Disclosure 源码修复完成，UI／真实服务体验由用户验收；没有保留旧配置、迁移或备份代码。


## 提供商配置体验复核（最新反馈）

用户在上一轮安装后要求 model ID 显示／编辑、API key 查阅／编辑和恢复 reasoning effort。初查确认模型编辑页只编辑全局记录，ID／协议推理等级在 ProviderModelFields，等级编辑又藏 Disclosure；会话没有当前 effort 选择用例。继续反馈要求先理清归属，指出独立模型／认证管理过度设计，协议与端点应可编辑。已暂停相关源修改并撤回本轮未提交实验补丁，工作区回到已交付源码，不构建或安装未完成方案。

Advisor 复核推荐 Provider 拥有认证和模型条目，不让简单单目标配置要求全局模型、认证资源与显式 route 多处建档。模型条目保存精确 API model ID、名称／图标及该提供商实际能力；可用 efforts 与当前请求 effort 分开，协议负责编码。API key 原地可查看编辑，OAuth 保留必要来源刷新，neutral gateway resolver 不依赖 Harness。这里是候选实施结构，尚未作为已交付契约。

官方资料再次核对（2026-10-05）：[AI SDK OpenAI-compatible provider](https://ai-sdk.dev/providers/openai-compatible-providers) 将 baseURL／apiKey 配置在 provider instance，使用 provider(model-id) 得到模型；[OpenAI reasoning](https://developers.openai.com/api/docs/guides/reasoning) 定义 effort 为每请求推理参数。资料支持归属分层，不规定 Velune 应建立独立 CRUD 页面。待与用户共同理清后再更新具体实施范围。
