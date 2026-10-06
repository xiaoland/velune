# 运行时与会话配置体验复核

2026-10-06 用户完成上轮验收，质疑运行时“初始模型”和“连接运行时”的必要性，反馈提供商列表更多菜单图标重叠，并建议从公开目录拉取模型模板。

primary 负责生命周期追踪、产品决定、文档与最终安装；advisor 只读分析模型归属、准备阶段及历史读取边界；公开模板 owner 贯通 application／bindings／Mac 的目录模板与图标修复。源码 owner 不修改运行时生命周期，避免在产品语义明确前只隐藏字段或改变名称。当前参考 AGENTS.md 指向的 guides/delegation.md 在仓库中不存在，按其已给出的所有权和授权规则安排，没有以缺失文档阻塞有界工作。

## 已观察行为

初始模型实际用于 connect 的网关注入与运行时准备，并用于新建 Pi 会话的默认值。即便编辑器允许“稍后选择”，connect 仍要求 model_record_key 非空。connect 对 Pi 主要准备网关与投影，在新建／打开会话时才启动 Pi；Codex／DSH connect 已启动原生控制子进程。application 已按配置实例读取历史，会话不应依赖运行时设置里的模型才可被发现。以上是当前代码事实，不证明这些字段属于正确产品职责。

“更多”使用 24pt borderless Menu，默认菜单箭头与 ellipsis 同时绘制。将由 owner 使用原生菜单语义修复。候选公开模板源为 models.dev；只读取公开模型规格，不作为实际提供商协议、模型可用性或 reasoning wire 参数的权威。需要保留来源提供商和实际模型 ID，复制成独立可编辑模板，不自动更新已保存模型。

## 当前工作

模型／运行时准备的产品方向等待 advisor 证据及用户选择；图标与公开模板明确改进已授权实施。无真实配置、凭据或会话读取；只允许公开目录 GET、静态检查与隔离合成端到端脚本，不新增或运行自动化测试。未扩大为自动模型发现、供应商请求或路由功能。

## 生命周期判断与待选行为

advisor 已完成只读追踪：模型选择属于会话，“连接”宜成为按需准备，不能作为浏览历史前置步骤。现有 PRD 明确认可过运行时默认模型，因此本轮属于用户重新质疑后的产品复核，尚未修改该决定。候选新建流程为选择运行时、目录与模型后自动准备；恢复优先使用 Harness 保存的可识别模型，失效时明确要求选择，不猜提供商、不默默回退。用户已收到流程偏好问题，依赖该决定的生命周期重构暂未开始。

只读详情仍有一个具体缺口：Codex／DSH 的 huihua 已能投影 messages；Pi helper 的 inspect 目前只给 cwd／model／virtualState，需要通过固定 SDK buildSessionContext 返回历史消息。只取消按钮或在 app 启动时自动 connect 不能解决这些前置耦合。后续判别验收应覆盖无模型浏览历史、会话选择后发送、失效模型恢复提示、实例切换不串模型与忙时切换保护。

## 明确改进的开发方验收

目录 owner 已完成并冻结：原生菜单隐藏 indicator 并补可访问名称；既有模板管理提供公开目录入口，搜索、来源和能力预览后进入原模板编辑器保存。获取使用固定公开 HTTPS GET、30 秒／16 MiB、禁重试／重定向，不使用认证。目录原始 API 实际存在 effort values 中的 null，解析忽略未声明值，不猜 off／none。来源同名模型仍分属不同提供商。模板保存与运行时生命周期无关，schema 6 未改变。

当前 debug 动态库公开 GET 返回 8389 条 provider-scoped 条目，拉取没有写配置；模板编辑后重开恢复、未自动建立提供商。合成 parser 覆盖 map key 不替代模型 ID、跨来源同 ID、0／null 规格、reasoning bool／toggle 不猜等级、effort null、无效 JSON 与超大输入。application／bindings 严格 clippy（含 no-default）、全 Mac Swift warnings-as-errors 类型检查通过。primary 正在进行 workspace 静态检查与 clean 安装包收口。仅访问公开目录，没有真实上游推理或用户配置访问；没有自动化测试。

收口时将公开 GET 置于既有 bindings 操作边界，保留对象关闭检查与诊断编号；关闭后调用返回 `application_closed` 诊断而不继续访问网络。未额外建立日志层或目录服务进程。

## 安装与当前交付状态

源码 `5f54a9d798b4b3afea3631da855c4af2297c22c0` 已 clean 构建并安装 `/Applications/Velune.app`，保持 0.1 beta.1、schema 6；manifest、签名与第三方许可资源核对通过。最终 workspace fmt/check/全 targets／features 严格 clippy 和 no-default bindings 严格 clippy 通过。按安装包动态库生成 Python 绑定后，公开 GET 与隔离模板保存／编辑／重开、拉取不写配置、关闭对象拒绝均通过，当前公开源返回 8389 条模型。没有调用模型服务、读取真实配置或运行自动化测试。

只启动过无 Transport 的合成 --preview-settings 预览以尝试检查菜单；电脑 UI 服务启动失败，未取得视觉证据，已退出预览。菜单的原生控件接线与 Swift 类型检查已通过，实际视觉仍由用户复核。

明确的两项改进已交付；初始模型与连接流程保持既有行为，产品偏好问题仍待用户回答。后续生命周期重构必须同时移除历史浏览与模型／准备的前置依赖，不以隐藏按钮代替契约修改。

## 已确认的会话驱动重构

用户已确认会话模型、按需准备与独立历史浏览，允许推进真实契约重构。添加实例即纳入浏览／选择范围；模型与凭据缺失不阻止列表或详情。初始模型字段和手动连接产品流程删除，执行准备失败仍明确显示且不得派发真实上游请求。启用／禁用不是本切片的必要门槛，不据此扩张管理框架。沿用一个活跃 runner、Harness 权威历史与 hard-cutoff；公开接口、配置 schema、Mac、人工脚本需同步，不能仅隐藏控件。

advisor 正在收敛单活跃 runner 下的浏览／执行状态与模型恢复边界。primary 维护文档、最终静态与安装；唯一源码 owner 将贯通 agent-runtime／application／bindings／Mac 和必要人工脚本，不读取真实配置、凭据或会话。

源码 owner 已确认具体接口：`select_runtime` 只切换浏览来源；configuration snapshot 使用 selectedRuntimeInstanceID 与 historyFailures，删除 connections；新建显式携带 modelRecordKey，删除 connect_runtime 和运行时默认模型。requiresReconnect 改为 executionInvalidated，闲置失效保留历史而在发送时重新准备。primary 独立维护新增人工浏览脚本，源码 owner 同步其它已存在的手工流程。只读模型选择先留在内存，实际执行后才由 Harness 保存，不引入另一份会话持久化。

新增默认目录浏览验收最初失败，定位为把 Pi 默认分层 sessions 根错误当作自定义直接目录，以及有界 helper 的环境清理使 agentDir 环境未生效。已改由 Rust 显式传 agentDir，JS 在 dynamic import 固定 SDK 前设置实例目录；默认 sessionDir 保持空，让 SDK 执行原生分层扫描，自定义目录保持 SDK 直接目录语义。复验通过：无提供商／消失 cwd 正常读取，选模型不启动执行，任意来源路径拒绝，准备失败保留历史和选择，单实例历史失败隔离，源历史逐字节保留，schema 7 没有 runtime.modelRecordKey。均为临时 HOME／固定 SDK，无真实模型调用。

源码已完成会话驱动契约：一个 Empty／History／Pi／Native 状态；新建显式模型、已有会话只读打开，首次发送准备，配置修改保留历史并使执行失效。Codex／DSH 原生记录不证明提供商身份，因此历史模型留空；Pi 只恢复有效自定义 recordKey。会话投影 revision 仅在本次投影生命周期内递增，Mac 不用它比较重新准备前后的顺序，而以串行操作、generation 与当前会话阻止过期应用。

隔离实际 SDK 验收通过 Pi 工具及次轮续接、Codex／DSH 同 ID 不同提供商选择、DSH Chat 到 Responses 选模型零请求且下一次发送自动续接、错误版本准备保留旧详情、审批／秘密回答／取消／终态失败后的新建恢复。公开目录拉取和模板重开通过。全 workspace fmt／check／全 targets 与 features strict clippy、no-default bindings strict clippy、全 Mac Swift warnings-as-errors 通过。正在从冻结源码构建并核对最终安装包。
