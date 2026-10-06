# 配置体验、运行时发现与原生 Messages

2026-10-06 用户报告历史详情诊断18dbddc3184363d8-a9ce-3，同时授权八项改进：详情失败修复、提供商／运行时双击编辑、模板多选批量添加、provider模型菜单及template sheet、quick runtime import、AI Anthropic Messages、每组加载20（可设置）、会话多选管理。保留前轮已认可统一浏览与原生管理，不推进未授权标签数据库、自动retry/failover或协议翻译。

所有权：session_management负责Mac源码及派生浏览／加载／模板手工验收；public_model_templates成为AI协议源码owner，负责ai/provider/gateway及必要application协议映射和bindings enum；root负责应用偏好config/API、运行时发现接入、权威docs／独立验收／发布安装。ai_service_audit接任历史错误边界与安全诊断源码 owner，负责合成验收。各owner不回滚他人，必要公共文件按局部字段/API协调；guides/delegation.md仍缺失，使用明确所有权。

本轮仍无自动化测试。静态检查、类型安全、构建及临时HOME／合成HTTP端到端脚本；真实会话／config／credential不由开发工具读取。只读取白名单安全日志元数据定位用户诊断，不能输出raw payload／secret。Messages只对loopback人工验证；实际服务、GUI由用户验收。完成后clean构建安装0.1 beta.1，保持当前有效schema7配置，增量默认字段不造成无谓重置。

已采用：application配置conversationBrowserGroupLimit默认20、正整数；snapshot具名字段和setConversationBrowserGroupLimit返回保存值，不为设置读全部history。每组派生分页不预构建全部视图；来源metadata目前汇总读取的实际性能边界需明确，不能把前端分页冒充上游只读取20条。多选删除非分布式事务，逐项原生结果报告，rename仅单项。快捷发现只检测可执行/版本/非秘密目录并预览，不在检测时读auth；runtime模型通过现provider import显式预览接续。


原生 Messages 源码与隔离证据（2026-10-06）：`packages/ai/messages` 提供具名原生请求、HTTP事件、provider binding和直接消费；`ai-provider/messages` 与 gateway `/v1/messages` 保留JSON业务字段及上游JSON/SSE原始字节，只在gateway替换model、在adapter替换认证。请求的 `anthropic-version`／beta等headers由调用方决定，缺失版本before-dispatch拒绝，不替用户选择版本。API key使用x-api-key，调用方authorization／x-api-key不会透传到上游；loopback入口支持Messages SDK的x-api-key令牌及既有Bearer。Subscription在未核实Messages认证能力时拒绝，不把OAuth来源推定为全协议通用。

地址契约按原生SDK：Messages provider配置使用baseURL，adapter追加 `/v1/messages`；Pi／DSH注入gateway origin，OpenAI注入仍使用 `/v1`。Pi1.0.2和DSH0.2rc2固定pi-ai协议注册表及SDK路径已实读并贯通anthropic-messages，Codex0.159.3保持Responses-only。Pi来源Messages compat保留，包括替换来源provider／URL前的SDK session-affinity默认；不引入真实供应商域名分支到AI／gateway。官方依据：[Messages HTTP](https://platform.claude.com/docs/en/api/messages/create)、[版本请求头](https://platform.claude.com/docs/en/api/versioning)、[SSE及原生error event](https://platform.claude.com/docs/en/build-with-claude/streaming)。HTTP传输结束不是模型业务成功，流中原生error由消费者解释，不重建sampling结果。

人工验收入口 `scripts/manual-messages-native.py` 临时Rust消费端+Python loopback已通过：直接AI与Gateway共8请求，JSON text/tool/thinking/unknown字段、SSE tool参数片段／signature／未来事件／error event逐字节保留；429正文、request-id、retry-after保留；缺version、未知subscription before-dispatch拒绝。`scripts/manual-runtime-messages.py --bundle ... --bindings ... --node ... --dsh ...` 用临时HOME合成Pi来源导入Messages，原models文件bytes未变，实际Pi和DSH各两轮共4请求通过gateway，未调用真实服务。Python临时bindings `target/bindings/messages-python` 与debug dylib供本轮独立验收，正式构建由root重新生成。全workspace fmt/check/strict clippy、bindings no-default strict clippy及Python/JS静态通过；源码owner不安装/提交bundle。

诊断现状：白名单日志定位用户 operation 到 open_conversation／history_open、runtime_slot 2、约1535ms，但缺少 family、versioned type 及 helper 失败类别，不能据此证明根因。已向用户明确现有可观测性不足，不再要求额外说明；不得将补日志当作已修复真实故障。新诊断边界由历史 owner 贯通。

快速发现具名 API：平台传入 userHome 和 PI_CODING_AGENT_DIR／CODEX_HOME／DSH_HOME 非秘密覆盖，application 给目录 hints；默认 Pi `.pi/agent`、Codex `.codex`、DSH `.dsh`（huihua 0.2.0 已安装公开源码 roots 实读）。平台只找 executable，application 有界 `--version` 探测并按已注册 variant regex 识别；不启动真实会话。未支持版本显示但不导入，配置不自动保存，source credential／模型仍经显式既有导入预览。

Root 手工 discovery API 验收已通过：临时 HOME 默认目录／存在性、合成 CLI 只允许 --version、Pi1.0.2支持／2.0.0明确拒绝、检测不保存、显式upsert后重复识别、groupLimit20→7保存重开及拒绝0。没有来源认证文件，未读机器实际会话。脚本为 `scripts/manual-runtime-discovery.py`，不是CI入口。

Mac 源码与隔离验收（2026-10-06）：提供商／运行时列表使用系统 List primary action 双击编辑。提供商模型菜单分为“添加新模型”和“从模板中选取…”，后者独立原生 Set 多选 sheet，批量添加后清除选择并保持打开；公开模型目录同样 Set 多选直接批量保存模板，不用 checkbox。会话列表的 UI Set 选择与已加载会话身份分离：单选才打开，多选与后续轮询保留已加载内容和运行时归属；上下文菜单根据点击行是否属于所选集合决定批次，重命名仅单项，删除确认后逐项调用各自运行时 API，成功保留、失败明确列出，不承诺原子批量操作。

每组默认仅生成前20行视图，“加载更多”只增加对应组的展示配额。原生会话设置保存正整数，查询或数量变化重置各组配额；该展示分页不改变既有完整 metadata 来源读取边界。Quick import sheet 从具名 hints 与可执行发现组合候选，先发现 Node，再 canonical binary／目录去重；只允许支持且未配置的候选显式导入，保持当前已加载会话，完成后可另开提供商与模型导入。Messages 表单使用基础 URL 提示，不假定 `/v1`；认证标签不将 API key 固定描述为 Bearer。

SwiftPM `--force-resolved-versions -Xswiftc -warnings-as-errors` 构建通过。`scripts/manual-session-loading.py` 使用实际 AppStore／Transport／UniFFI 与临时 HOME，通过原有慢加载、迟到 poll、加载期间原生管理验收，并新增：两组45项分别首次20、一组加载40而另一组保持20、重置为7；groupLimit 保存后 shutdown／重开仍为7；多选不触发打开且 poll 保持 Set；真实批删一项成功一项失败时保留失败原生文件与选择，再跨运行时删除；批量保存两个模板；公开版本发现 Pi1.0.2 与不支持9.9.9、仅导入支持项且 loaded 身份不变。无凭据读取、无模型请求。新 home 单实例守卫要求先关闭再重开验收，脚本已遵守。Python语法、diff／cached diff／EOF检查通过；GUI手势、sheet外观与体验仍由用户验收。

后续用户线索与产品更正：用户观察当前运行时与目标历史来源不同容易失败，尚非证实根因。真实调用审计与合成 Pi→Codex 历史验证表明 open 传的是会话来源；但现有 selectedRuntimeID／selected_runtime_id 混用了历史来源、活动执行和顶部选择，并在 open／snapshot 时覆盖顶部意图。advisor 明确应拆分 ConversationSource、NextTurnSelection 与固定 execution owner。Mac／application owner 已恢复贯通修复，历史原生管理仍按来源，轮询／取消／审批仍按实际执行。跨 Harness 不可直接 resume 其它来源 ID，也不能把展示投影视为无损完整上下文。

后续范围已确认：2026-10-06 用户接受保留原会话、目标原生会话与可携带上下文交接、只持久化原生引用和切换位置。三状态拆分与既有八项交付的安装证据仍归本文；新的跨 Harness 实施归 [接续任务](../cross-harness-continuation/packet.md)。

后续执行意图澄清（2026-10-06，已实现并隔离验收）：顶部运行时／模型表示下一轮目标，不是历史来源。Mac 旧 selectedRuntimeID 改名 projectionRuntimeID，表示当前快照来源，用于当前 context 的快照查询、取消和审批；真实 execution owner 保留在 core，不由 Swift 的浏览状态推定；历史浏览和管理继续从 Conversation.runtimeID 取得来源。独立 nextTurnRuntimeID／nextTurnModelRecordKey 为可在生成／加载时修改的纯 draft，打开、刷新历史及当前 turn 完成不覆盖它，模型菜单按下一轮目标过滤。选择模型本身不改历史 model、不启动 child／gateway；发送捕获当次目标和模型。已选目标被停用／删除时清除意图，避免自动替换为另一执行者。

当前有界闭环：同来源发送使用具名原子 sendTurn(runtimeID, modelRecordKey, text)，一次接纳当轮意图；跨运行时上下文 transfer 尚待产品范围和公开接口契约，暂显示明确边界并禁止伪 resume／文本拼接。这不是对最终 cross-Harness 能力的拒绝，也不将只读浏览来源与未来执行者重新绑定。既有 AppStore 合成验收已通过新增历史切换保持下一轮 pair、等待 open 时可改 draft、跨来源未接入时明确拒绝而不发送；完整三状态实际 turn 验收已在具名 intent API 接线后通过，证据如下。


下一轮意图端到端验收（2026-10-06）：使用最新 debug dylib、当前 AppStore／Transport／UniFFI 和隔离 Pi1.0.2，`manual-session-loading.py` 全部通过。读取历史保留下一轮 pair、历史没有 Velune 模型 marker 时选择有效模型即可发送；加载期间修改 draft 不影响来源。生成中把下一轮改为另一实例，再读取配置列表与轮询仍保持原会话；第一轮完整 SSE 回复和第二轮取消均保持原 nativeID／来源及下一轮 draft。修改合成 provider endpoint 关闭 execution 后，core 按历史来源恢复只读详情，Mac 不清空消息或下一轮意图，后续快照查询仍可用。仅两次本地 HTTP 请求，模型均为合成 ID，无真实模型 API／认证／会话读取。Swift warnings-as-errors 构建、Python语法和 diff 检查通过。projectionRuntimeID 只表示 Mac 当前快照的查询来源，Core 的实际 execution owner 独立；list 的下一轮意图绝不覆盖它，resetProjection 清空当前来源而保留 draft。

集成收口补充：关闭实际执行后按原生来源恢复只读历史，不伪造 execution owner；修改来源的版本／路径／启用状态后必须重新打开历史，下一轮目标停用或删除后清空该意图。history失败日志记录 source／execution／next-turn 各自配置序号（0表示无实例），以及固定类型与安全类别，无原生ID／路径／内容。原始诊断根因仍不可从旧日志还原。

安装包首轮复验：c009d05 clean release 构建并安装0.1 beta.1、schema7；实际安装库已通过 AppStore 全链（2次loopback）、Pi／DSH Messages（4次loopback）、历史 helper／bindings 分类及跨来源只读、discovery／groupLimit、source browser。discovery脚本最初将协议枚举与字符串比较，已改为具名BindingGatewayProtocol，不涉及产品修改。补充原生管理复验发现删除当前来源后getSnapshot将空状态误报未激活，现明确已知启用实例的空投影返回None，其它实例不能读取已加载内容。Codex实际API秒级时间1791272610000与原生头1791272609954跨秒边界；原生头读取正确，原脚本错误假设差值必非负，现只要求精度差小于一秒，模型marker未知与可提交显式turn的断言也按新契约分开。修正后重新构建／安装／复验，未把脚本错误当产品时间逻辑缺陷。

最终安装与复验（2026-10-06）：`/Applications/Velune.app` 的 build-manifest 确认源码为 clean `6d18c2707224b8f9ca68e0519684dac9e1f20721`，版本0.1 beta.1、schema7、具名UniFFI；签名校验通过。使用安装库重新生成Python bindings后，AppStore全链验收通过（2次loopback），Pi／Codex原生重命名、删除、重开和空投影查询通过（无模型请求）；Codex／DSH原生发送、历史读取及续接、同ID跨provider精确路由、DSH跨协议续接通过（5次loopback）。最后一条旧脚本断言仍假定发送前切换已经修改执行模型，现明确发送前保留上一轮模型、sendTurn后才确认目标模型，没有因此改动产品源码。Rust静态检查及Swift严格构建通过；全部脚本采用合成会话、临时HOME、本地HTTP，不读取用户真实秘密或历史，不调用真实服务。GUI体验和真实提供商调用仍由用户验收；跨Harness实际上下文交接仍待上述范围决定。
