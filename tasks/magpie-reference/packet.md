# Magpie 架构参考复核

用户于 2026-10-06 建议重点学习 Magpie。初轮只调查与比较；后续用户明确授权开始吸纳，当前实施边界见末节。未授权复制实现、依赖接入或产品范围扩张；提供源码证据，区分可借鉴机制和与 Velune 已确认边界冲突的部分。

目前定位为 [yetone/magpie](https://github.com/yetone/magpie)，产品是多个 Agent 的模型配置控制界面和本机网关，与当前问题吻合。调查时固定实际 checkout 提交，重点核对原生协议／转换边界、提供商及模型／能力归属、配置导入与写入所有权、执行版本适配、失败与流式生命周期。上游 README 的能力声明不作为行为验收。

primary 负责网关与整体比较，既有 huihua/import owner 负责配置 adapter 与模型／导入。仅阅读源码、官方文档和许可证，不运行 Magpie、不访问任何真实配置或认证；不改 Velune 产品源码。结论先归本 packet，用户认可的架构决定再归长期设计。

## 已核对证据

检查日期为 2026-10-06，源码固定 `db55bc8b7a70126fd3a7bede95a56b31c6f20c5f`，许可证为 MIT。使用临时 checkout，只读检查，没有安装依赖、运行产品或执行其测试。下述路径属于上游，不是 Velune 的当前接口。

`internal/gateway/gateway.go:2197` 的 `attempt` 先判断请求协议是否在目标能力中，适合时调用 `passthrough`，否则进入 `translate`。但 passthrough（2568起）除了改 model/auth，还包含 developer→system、reasoning 字段删改、供应商工具改写以及 400 后删除可选参数再请求等兼容行为。不能将该函数名当作原生保真的证明。Velune 目前不翻译、不自动降级请求语义；兼容规则若将来确实需要，应成为显式、可定位的协议能力／来源 adapter 行为。

`internal/gateway/fallback.go` 的 `holdWriter` 显式记录 passing、held、ended 与 context。`mayAskAgain`（1270起）只在该尝试未提交到下游时允许换候选；`gateway.go:1739` 区分调用方取消，停止尝试且不把取消记为供应商故障。可借鉴明确的响应提交边界与取消归因。不过“尚未提交下游”不是上游尚未执行或尚未计费的证明；Velune 的后续 fail-over 决策仍需要另外明确请求重放、预算与副作用边界。本轮不启用 fail-over。

`internal/gateway/trace.go` 在作出决定处记录模型、候选顺序、选择／失败原因、各次耗时和 usage，不从事后日志猜测路由过程。这是可观测性与业务输出分离的有用参考。Magpie 的 Call／OTel 也支持请求与响应正文采集，不能照搬到 Velune 当前不记录消息、配置、认证和原始上游错误的日志合同。

`docs/subsystems/gui-shell.md` 与 `internal/gui` 表明其桌面界面采用 Wails/WebView、JSON HTTP API。可参考任务层级和信息组织，不采用其 UI 技术／控件皮肤；Velune 继续平台原生 app 和同进程 UniFFI。

## 比较中发现的 Velune 缺口（研究时基线）

当前 Mac 的 `RuntimeEditor.compatibleModels`（app/mac/Views.swift:549）按 familyID 筛选协议；application 的 native_injection 则按精确 typeId 检查协议。后端仍拒绝不兼容执行，因此目前没有由此放宽协议，但未来同家族的不同 adapter 能力可能不一致，UI 会错误放行或隐藏。建议让版本化 runtime descriptor 暴露 supportedProtocols，界面直接消费；family 继续只承担分组。这是基于现有多版本决定的候选修正，不借 Magpie 参考扩张产品能力。

## 配置与模型调查结论

来源 owner 已核对以下具体机制，而不是从支持列表推定能力：

- `internal/provider/importapps.go:42` 的 AppImport 用 new／same／taken 表示候选状态，Off 是默认未选，Skip 是不能导入；AppPick 表达具体动作。应用时重新读取来源，UI 提交来源引用。值得借鉴的是状态与操作分开，继续沿用 Velune 的非秘密预览、来源复核与原子提交，不另建导入框架。
- `internal/provider/models.go:27` 的 Available 与 `Exposed:703` 区分可发现目录和用户暴露的条目。可用于区分可导入候选、参数模板与实际已配置模型，避免将目录全部塞进执行菜单；其默认只取前 24 个的策略不是 Velune 需求。
- `internal/agent/native.go:70` 的 DisconnectPlan.Revision 与 CheckFiles 将服务端计划绑定到前置文件状态，应用前拒绝过期确认。Velune 当前已有来源／配置 revision 复核，可以对照其边界完善，而非增加通用计划基础设施。Magpie 的外部配置 ownership／drift 提示可以作为参考，但本轮不改 Velune 已确定的独立执行投影方式。

有两项不应作为实际模型语义：`provider/models.go:1220` 的 resolveIn 在裸 ID 命中多个已暴露提供商时取 hits[0]；不能因唯一匹配的注释就忽略实际实现。`catalog/catalog.go:646` 的 ContextOf／OutputOf／EffortsOf 按规范化裸模型名借用跨提供商规格，然后用于部分配置写入。Velune 继续使用稳定内部记录键和显式 alias，跨提供商目录只作为模板建议，不据此提高实际能力或猜测 reasoning wire 映射。

`internal/agent/agent.go:90` 的 Agent 是配置位置、字段读写与同步描述，DSH writer 同时照顾多种配置形态。`agent/dsh.go:960` 的默认容量注释与我们固定 DSH rc2 上游已核对的值不同。因此这是配置接线经验，不是版本化执行协议的权威；Velune 保留精确 variant、regex 和原生握手。

### 可复核来源

以下均固定同一提交，不使用移动 main 作为永久证据：

- [导入候选与动作](https://github.com/yetone/magpie/blob/db55bc8b7a70126fd3a7bede95a56b31c6f20c5f/internal/provider/importapps.go#L42)
- [发现目录](https://github.com/yetone/magpie/blob/db55bc8b7a70126fd3a7bede95a56b31c6f20c5f/internal/provider/models.go#L27)、[用户暴露模型](https://github.com/yetone/magpie/blob/db55bc8b7a70126fd3a7bede95a56b31c6f20c5f/internal/provider/models.go#L703)
- [确认计划与前置状态](https://github.com/yetone/magpie/blob/db55bc8b7a70126fd3a7bede95a56b31c6f20c5f/internal/agent/native.go#L70)
- [裸 ID 解析](https://github.com/yetone/magpie/blob/db55bc8b7a70126fd3a7bede95a56b31c6f20c5f/internal/provider/models.go#L1220)、[跨提供商能力汇总](https://github.com/yetone/magpie/blob/db55bc8b7a70126fd3a7bede95a56b31c6f20c5f/internal/catalog/catalog.go#L646)
- [原生与转换派发](https://github.com/yetone/magpie/blob/db55bc8b7a70126fd3a7bede95a56b31c6f20c5f/internal/gateway/gateway.go#L2197)、[passthrough 的改写](https://github.com/yetone/magpie/blob/db55bc8b7a70126fd3a7bede95a56b31c6f20c5f/internal/gateway/gateway.go#L2573)
- [流式响应提交边界](https://github.com/yetone/magpie/blob/db55bc8b7a70126fd3a7bede95a56b31c6f20c5f/internal/gateway/fallback.go#L871)、[决策现场 trace](https://github.com/yetone/magpie/blob/db55bc8b7a70126fd3a7bede95a56b31c6f20c5f/internal/gateway/trace.go#L3)

## 研究结论（实施前）

研究已完成，未修改产品源码、引入包、改变协议、启动自动路由或重新安装 app。Magpie 可作为 LLM 网关与配置接入的重点参考，huihua 继续是原生历史读取依赖，Obelisk 是索引／来源参考，各自用途不同。下一候选小切片是版本化 descriptor 的协议能力声明与 UI 消费；下一候选网关改进是无正文的 request／attempt 决策观测。二者均需结合实际使用决定，不把本研究建议自动提升为产品需求。相对链接与 diff 检查通过，没有运行任何自动化测试。

## 已授权吸纳与当前执行

用户已授权实施。primary 负责网关观测、文档和安装验收；运行时 owner 贯通 variant 注册表、application、UniFFI 与 Mac；导入 owner 负责选择边界、替换说明及 Pi 人工脚本。owners 已完成源码与静态检查，未提交或安装，由 primary 统一收口。advisor 调用受到会话 agent thread limit 拒绝，本切片依现有日志装配与可判别的回环实验推进，没有扩大产品范围。

当前变化为版本化 `supportedProtocols` 单一声明、Mac 消费及执行准备检查；导入重复选择拒绝、替换说明纠正与非秘密来源指纹的过期验收；网关 request／attempt 关联、目标快照序号、状态与耗时及取消／关闭归因。导入原有状态和原子提交继续复用，不为参考建立通用计划或目录框架。来源指纹是非秘密规范化快照，不承诺原文件任意字节或秘密刷新都会使预览过期。

schema 6 不变，没有引入新的配置模型或依赖，没有翻译、重试、fail-over 或业务正文日志。已通过 debug bindings 的 Codex／DSH 实际运行时四次合成请求及日志安全关联；原生 HTTP 人工脚本已通过 9 个保真用例、1 个未匹配路由、2 个调用方取消和 1 个在途关闭；13 个请求与单次派发的元数据均正确关联，取消／关闭没有成为提供商失败，idle／active Drop 均为 0ms（本次本机测量，不是时延保证）。fmt、workspace check、workspace 全 targets／features 严格 clippy 和 no-default bindings 严格 clippy 均通过；owner 的全 Mac Swift warnings-as-errors 类型检查通过。没有新增或运行自动化测试，没有真实服务调用。源码提交 `a985bfd42531562afeae7ca198d02fa281f25b5a` 已 clean 构建并安装 `/Applications/Velune.app`，版本 0.1 beta.1、schema 6。安装包签名、manifest、helper 与第三方声明／许可证／锁文件核对通过。按安装动态库重新生成 Python 绑定后，实际 Codex／DSH 四次合成请求与 Pi 五次工具续轮请求均通过；Pi 同时验证重复选择和来源快照变化在写入前拒绝，原来源逐字节保留。开发方有界验收已完成，真实 UI／提供商体验仍由用户验收。
