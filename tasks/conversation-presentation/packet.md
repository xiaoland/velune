# 会话模型与消息展示

2026-10-06 用户验收后要求修复标题文件名、时间显示、默认底部与消息长列表性能、用户气泡、工具及 LLM 左对齐和完整 Markdown，并要求内部权威会话模型与 UI 展示分离。已授权实现；不新增会话数据库、标题生成模型调用或真实资源验收。

当前事实：conversation package 已提供可替换投影，但 message.role 为 string；Pi helper 已提供 name／firstMessage／标准 ISO 时间，application 标题却回退文件名。Mac 已有 LazyVStack，但仅生成时滚底，时间解析未处理小数秒；手写 fence 分割和 inline-only Markdown 不满足完整语法。

root 负责权威文档、独立验收与安装；既有源码 owner 贯通领域／adapter／bindings／Mac／构建与必要手动脚本；advisor 只读判断权威模型和 UI 展示层边界。guides/delegation.md 仍不存在，沿用现有明确所有权，不阻塞有界实施。

候选方向：Rust conversation 是内存投影的类型权威，Harness 是持久化权威；Mac 可将消息／块拆成稳定展示行，不将 UI events 持久化或送回领域。采用原生懒布局与跟随底部状态，已有历史浏览不得被流式强行拉走。Markdown 使用实际兼容 macOS 14 的 Swift 发行包，锁版本并遵守许可，不复制解析器。具体契约和依赖待当前证据收敛。

验证只使用类型检查、fmt/check/clippy、构建和隔离临时脚本；不运行或新增自动化测试，不读取真实配置、凭据、会话，不调用真实模型。完成后重建安装 /Applications/Velune.app，GUI与真实服务由用户验收。

## 已采用的契约

advisor 复核确认 conversation unit 已是适当权威，不新增数据库、总线或事件持久化。采纳角色枚举与类型内容、可选 Unix 毫秒和共享标题规则；Pi 历史／实时工具与 huihua 工具需统一关联和状态，不能仅把 UI 居中改左侧。源码 owner 负责这些归一与 Swift 稳定展示行。

官方包核对：MarkdownUI 2.4.1 支持 macOS 12／Swift tools 5.6，依赖 cmark-gfm 与 NetworkImage；Textual 0.5.0 要求 macOS 15／tools 6.0，不符合当前 macOS 14 底线。选择固定 MarkdownUI 发行包，锁传递依赖，构建产品不包含或执行依赖测试 target。MIT 及传递许可必须随应用保留。官方来源：[MarkdownUI manifest](https://github.com/gonzalezreal/swift-markdown-ui/blob/2.4.1/Package.swift)、[Textual manifest](https://github.com/gonzalezreal/textual/blob/0.5.0/Package.swift)。检查日期 2026-10-06。

## 当前隔离证据

root 独立浏览脚本在 fresh typed debug dylib／Python 绑定和当前 helper 资源下通过：SDK 原生名称优先、首用户内容 fallback、只读打开不改变标题、类型 User 及 Unix 毫秒、Reasoning 保留，以及同一 native call ID 只有一次已完成工具并带原始输出。同时无提供商／消失工作目录浏览、选模型不执行、外来会话拒绝、准备失败保留视图与选择、单实例读取隔离仍通过。来源记录字节保持，配置 schema 7 未变。当前是 debug 证据，尚非新 Mac 安装或视觉验收。

标题复核发现新建占位字符串还可能永久保留，且根据“未命名会话”等字符串判断来源会误改同名原生标题。经 advisor 采用 Rust `ConversationTitle::{Native,FirstMessage,Untitled}`，bindings 只导出 display text，Swift 不增加 source 标志。新建为 Untitled，首正文派生标题，准备克隆已有摘要，缺失 native name 不清空已知来源；root 加入原生名称恰好为“未命名会话”的合成验收，避免表面字符串修复。

## 后端轮询的性能边界

owner 发现 Pi snapshot 每 0.6 秒调用完整 state/get_messages 并无条件重投影、增加 revision。advisor 核对固定 SDK 1.0.2：session_info_changed 提供名称，compaction 完成会整体刷新 finalized context，扩展 navigateTree/newSession/fork/switchSession 的 session_tree 不进入 RPC session.subscribe。因此采纳普通轮询只 drain，首次装载／handled 命令／settled／取消完成／显式切换一次权威同步，不采纳永久只追加消息事件。空闲同步应先处理缓冲，handled 命令不无条件追加 user，重载保留已有消息身份；分支与压缩验证仍待 owner 提供证据。

owner Mac 编译通过，MarkdownUI 2.4.1 由根目录 SwiftPM 产品接入并锁传递依赖。5000 展示行临时脚本证明首次 2500 次 Markdown 解析约 0.074 秒，100 次同内容 apply 约 0.421 秒且不解析，变化末条只解析一次、全部 row 对象身份保留；这些是模型缓存测量，不是屏幕滚动帧率。

root typed-title fresh debug 复验通过，包括显式原生标题恰好为“未命名会话”仍保留。owner actual Pi 五次 loopback 工具续轮和下一轮、首用户标题、Reasoning、唯一 Completed 工具输出与三次 idle poll 不改变 revision 均通过。权威同步 metadata 使用固定 SDK 公开只读扩展命令查询当前 SessionManager，不只靠最初 factory，从而观察原地 tree/new/fork 后的目录／身份／名称；普通 poll 不触发该同步。扩展分支与最终静态／安装仍待收口。

Codex actual loopback发现 huihua 将 response_item 初始化模型上下文映为 user，直接取第一条会产生环境 XML 标题。最初候选是偏好 event_msg.user_message，但实际 0.159.3 不发这种记录；来源序列为 session_meta、developer response、初始化 user context、world_state、turn_context、真实 user response。采用公开 sourceType=turn_context 的原生边界识别当前版本首次用户交互，不以 XML 文本启发式删消息；真实用户引用相同文本不受影响。scan 的派生 title 也须核对出处，不能将 SDK 从注入内容推导的标题冒充原生明确名称。此修复归固定 Codex variant 的历史语义边界，不外推其它版本，AI 服务与 Mac 不解析这些来源记录。

## 冻结源码与开发方验收

源码 owner 已冻结：全 workspace fmt／check／全 targets 与 features strict clippy、no-default bindings strict clippy、SwiftPM 固定解析锁与 warnings-as-errors，以及 Python／JS／shell 静态与 diff 检查通过。Mac 绑定静态编入 Swift 产品，Rust dylib 保持同进程嵌入；旧 Swift wrapper dylib 不再打包。

`manual-pi-session-resync.py` 以固定 SDK 验证 same-ID／same-count 分支、显式名称恰好为占位文字、新 session 与 cwd、handled slash 不伪造用户消息、idle 不重载，零 upstream。metadata 已改由公开 ctx.ui.setStatus 的专属键传输，直接 stdout 会被 SDK 捕获；adapter 消费该状态，不将它暴露为业务通知或持久化记录。`manual-pi-native-loop.py` 五合成请求与 `manual-multi-runtime.py` Codex／DSH 五合成请求通过新建、打开、准备、继续的标题一致性及工具／推理；审批／取消／终态失败恢复仍通过。

长列表手工脚本为 `scripts/manual-transcript-presentation.py`，需先构建 SwiftPM 产品，或显式给 `--swift-build`；它测量稳定展示对象与解析缓存，未测屏幕帧率。既有 --preview 合成样例覆盖标题、列表、任务列表、引用、表格、代码、推理与已完成工具，用于无 Transport 的原生预览。当前正在 clean 构建和安装，尚不将 debug 证据当作安装交付。
