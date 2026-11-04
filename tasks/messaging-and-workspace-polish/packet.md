# 消息提交、窗口恢复与外观改进

本包记录当时的实现与证据。2026-11-04 后续的[速度模型变体](../speed-as-model-variants/packet.md)已取代下文按轮 service tier 设计及相关使用边界；[工作区收尾](../workspace-delivery-and-network/packet.md)接管最终组合验收与提交，当前 managed 自定义参数也已接通。

## 目标与授权

2026-11-04 用户要求处理八项：排队／插队即时呈现；Pi 继续的文本兜底；模型速度选项与 composer 空间；消息时间；Pi 背景通知按 client side 呈现；独立会话窗口默认内置 composer；重启恢复 editor groups／Tabs；外观字号与密度。

“需要你处理”授权本范围必要源码修复及实现；沿用不提交、不重装／退出日常应用、不读真实会话或凭据、不使用 computer-use 或抢占鼠标／前台的约束。不委派。分层、仅受影响关键路径，复用适用证据。

## 起点与修复判断

- 普通发送独有 Mac PendingSend 与 transcript.showPending；排队／steer 未走这条路径，用户意图与运行时回显的接续没有统一。
- 独立会话窗口由主窗口 rememberedToolLayout 派生，仅藏列表／调关联 Agent，composer 的停靠和收起状态仍继承。需要窗口类型自己的初始布局，与显式恢复分开。
- WindowWorkspaceJournal 已保存完整 editor，但主动写入主要监听输入变化；需追踪结构改变、正常退出与新进程恢复，不能只加序列化字段。
- 速度不能从模型名称猜测。已核对 Codex 官方 schema 与 Kimi 官方接口；仅对可真实传递的路径提供选项，未选择时由运行时决定。

## 产品与实施方向

消息的派发方式与可见提交状态分开：点击后同一提交立即出现，受理后接续到队列或运行时回显；失败保留可恢复输入，不以另一条消息替代。继续优先原生无输入恢复，无此能力时由核心明确发送继续文本。元信息统一放消息操作行；时间用原生创建时间，缺失不编造。client／server 是消息来源，不从文本猜角色。

按依赖推进：事实与最小行为方案 → 共享消息／选项契约 → Mac 呈现和窗口生命周期 → 分层关键检查／原生静态视觉证据 → 文档与交付。外观控制首先沿用 Mac 设置结构，明确 UI／消息／代码与密度的作用范围，避免把业务输入选项挤进同一行。

## 验收安排

局部检查覆盖提交接续不重复、Pi 兜底与准入、速度参数透传与能力过滤、时间格式及来源。组合检查使用合成运行时／服务，检查实际请求与延迟回包。窗口用真实模型动作与新进程恢复验证 groups／Tabs／选择及独立默认布局。视觉仅检查本轮改变的消息操作行、client 通知、composer 与外观缩放；使用原生离屏／后台脚本，不激活用户窗口。构建只按受影响端补齐，不反复完整验收。

## 当前实现与证据

主要实现与已获得证据如下：

- `OutgoingSubmissions.swift` 统一普通发送、排队、插队及队列转插队的本地生命周期；`TranscriptModel` 支持多项占位。以 application 稳定消息身份确认回显，并记住已认领回显，避免重复轮询消耗另一个相同文本提交。排队受理后接替到会话详情的权威队列；失败只移除自身占位，原编辑恢复仍按捕获修订。
- 运行时 `ResumeStrategy` 声明 Native／Message／Unsupported，Pi 选择 Message("continue")，核心保留准入与队列暂停边界，并参考最近原生执行的失败／停止记录。Mac 帮助不再承诺所有运行时均无新消息恢复。
- 新 `MessageRole.Client` 用于 Pi custom 通知（display=false 不显示），客户端右对齐，不能冒充 User 触发新轮；已调整四端呈现入口。核心对应投影测试通过：`/tmp/velune-messaging-polish/client-event-tests-1.log`；另已验证 Client 不进入工作过程，不遮掉最终回复，也不增加用户大纲（`client-fold-tests.log`）。
- ~~Codex 官方 0.166 schema 的 `Model.serviceTiers`／`turn/start.serviceTierForTurn` 接通与按轮速度选择~~ **已被 2026-11-04 用户决定取代（S77）**：速度改为目录组合——managed 模型记录携带 `request_parameters` 声明为独立条目，Codex adapter 将目录档位摊平为变体模型；按轮 service tier 维度已删除。见 [速度即模型变体](../speed-as-model-variants/packet.md)。线上验证结论（0.166 `serviceTierForTurn` 到达 Responses 请求）仍然有效，逻辑移入 adapter 内部。
- `MessageMetadata` 统一创建时间格式；用户／client 消息、工作过程底部与最终回复使用消息操作行，缺失原生时间不伪造。Codex 0.166 的 live item/started.startedAtMs 已接，后续完成消息保留原创建时间。
- `Appearance.swift` 加本地 `mac-appearance.json`：独立 UI／消息／代码四档字号、三档密度、composer 最大宽度640–1440，默认960。263 处显式语义 UI 字体改用观察设置的 uiFont；原生消息布局／缓存和 composer 字体更新有专门路径，不重建输入对象。
- 独立窗口从默认工具布局开始；显式保存布局仍恢复自身。窗口结构变动在模型中保存，journal 按 windowKind 筛选恢复；后台懒加载的新建 Tab 目标单独保留，避免下一次保存丢失。
- 共享核心 check5 通过，NextTurn 8 项窄测通过（`core-check-5.log`、`next-turn-tests-2.log`）；严格 all-targets clippy4 通过（含 application、agent-runtime、application-api）。clippy 的迭代只修测试 module 放置与旧 fixture 缺字段，不重复运行时联调。
- 实际 Pi 1.4、合成 HOME 与回环服务：停止→应用重启→snapshot 保留恢复资格→resume 只增加一条 continue 并获得合成回复（`pi-continue-2.log`）。首次脚本错用被动 read_conversation 判断准入，改为 UI 实际 snapshot 路径后通过。
- 实际 Codex 0.166：原生模型目录报告速度；priority 到达实际 Responses 请求，下一默认轮不继承，模型 ID 相同（`codex-speed-verified.log`）。复用原始协议捕获；第一次脚本把原生命名请求算入用户轮总数，是验收 oracle 错误，没有为此重跑运行时。
- 原生隐藏入口已跑：提交反馈和稳定身份确认、时间规则、实际窗口模型两组四 Tab、结构保存、独立 composer 默认及 kind 恢复通过（`native-check-1.log`）；新进程 groups／顺序／选择／后台未激活创建目标／外观恢复通过（`native-reopen-1.log`）。合成根 `native-home`。
- 已实际查看第二组原生离屏 PNG：`visual-home/{message-actions,composer-1100,composer-560,appearance}.png`。消息两侧、工作／最终回复时间与复制、宽窄输入区居中、外观表单正常。首次 PNG 的透明背景／按钮失真修正后只补视觉，没有重跑功能。全程 prohibited activation，无可见窗口、全局输入或 computer-use。
- 最终 Mac SVC build5 通过（execution 4f981a26-f01f-4001-8b6f-368503060336），含菜单默认语义和共享字体修正；Windows build2 隔离构建通过；iOS build2 通过；Android assemble＋lint 通过。iOS 的共享遥测页误用 Mac 字体 modifier 已改为平台条件编译。

## 收尾证据

- `--experience-typography-only` 已通过（`typography-1.log`）：改变消息字号后同一原生 composer 的正文、选区与撤销记录继续可用；没有重跑提交／运行时／布局检查。检查器自己的首轮编译访问了 fileprivate 重载，已修为经真实 delegate 操作原生文本；不涉及产品逻辑回退。
- Windows 精确解除三项共享 cache junction 后清除任务 worktree 与指定临时文件（`windows-cleanup.log`）。未启动 Windows UI，未触碰日常安装。
- 本轮 12 份文档的 227 个本地链接／锚点检查通过（`doc-links.log`）；cargo fmt 与 git diff --check 通过。
- 含新增 service tier 的完整输入跨 view／重开持久化窄检查通过，1 项、其余 25 项未运行（`composer-speed-persistence.log`）。

## 使用边界

本轮 Mac 是体验实施主体。共享核心的继续、Client 与 service tier 契约各端可用；Windows 已接速度选择及 Client 对齐，Mac 的提交占位、时间操作行、外观和新窗口／editor 恢复尚未移植。iOS／Android 保留共享速度选择，尚无专门速度菜单。本轮没有 Windows 原生 UI 验收；只声明隔离源码构建。

原生 Codex 速度已端到端验证；managed gateway 尚不提供 service tier。Kimi Code 的高速版本走官方模型 ID 与现有模型选择，不把不同模型能力合并；未做真实套餐／额度联调。

上轮跨独立执行宿主保存端设置接线及真实旧输入迁移的边界保留在 [composer 状态任务](../composer-state-ownership/packet.md)，不在本轮默默扩范围或假称已完成。未提交、未安装日常构建，未触碰用户鼠标或前台。

本轮授权实现与分层验证已收尾；没有待运行的检查。日常应用仍未重装；上一轮已有输入的显式迁移及跨独立执行宿主保存端接线仍按原任务保留，不因本轮构建通过而视为完成。
