# 主动上下文压缩

2026-10-27 用户明确要求接入主动上下文压缩，并展示“正在压缩上下文”这样的消息。已授权必要源码、文档与验证；已于后续请求授权提交；不安装、不读取真实配置／凭据／会话、不调用真实模型。primary 负责实现与交付；advisor 只读复核原生入口、生命周期、历史与转录边界。工作区既有远程配对及遥测等修改保留，不纳入本任务的变更说明。

## 实现

`compact_conversation`／`CompactConversation` 贯通 application、远控命令、UniFFI 与平台。核心按已寻址会话计算 `can_compact`，仅在支持原生压缩、已有历史、空闲、无待答交互／后台子 Agent 且非子会话时提供；操作使用同一运行时界面当前选择的模型，不跨 Harness，不新增普通用户消息。

Codex 0.159.3 使用 `thread/compact/start`，接住压缩独立 turn 的 ID，并在开始通知前保存停止意图；Pi 1.0.2 异步发 `compact`，由开始／结束事件与对应响应收尾；Claude Code 2.1.292 通过其内置非交互 `/compact` 命令执行，以原生 status、结果和 compact_boundary 确认结果。Claude 的保活复用同一进行中消息，命令助手输出不冒充用户轮次的回复。DSH／通用 ACP 不声明主动压缩能力。

`ContextCompaction` 系统块独立于工作过程，保持末条回复位置与复制入口。原生进行中、成功、失败、取消和断连按各自结果收尾，不把内容 completed 当作执行成功。主动压缩占用运行与停止通路，但绕过普通 turn 配置登记、计时、恢复资格和队列推进；压缩期间禁止 steer／修改队列。自动压缩仍属于原用户轮次，不能仅凭一个回合含压缩条目就改变整个回合的错误、恢复资格或用量归属。

Mac 提供会话菜单、命令面板和 `/compact`；iOS 会话操作菜单、Android 顶栏与 Windows 输入区提供按钮。各端只消费同一核心能力与状态。权威行为归 [composer PRD](../../docs/prd/composer.md#主动压缩上下文)、[消息展示 PRD](../../docs/prd/conversation-view.md#上下文压缩状态)；运行时细节见 [agent-runtime README](../../packages/agent-runtime/README.md#主动上下文压缩)。

## 验证

隔离验收脚本 使用实际 Codex 0.159.3、Claude Code 2.1.292 和仓库 `target/pi-runtime` 中的 Pi 1.0.2，经本机合成回环上游运行。PATH 中 Pi 1.4.0 不在适配范围，未拿它替代。三种运行时各通过 5 组检查：进度与完成（包括同 runtime 多会话正确寻址、操作及时返回、可轮询、独立展示且不重新计时）；取消收尾；失败保留原回复／时长／实时恢复资格；重开无命令用户消息或过期进度；已有暂停队列保持暂停。日志在 `/tmp/velune-compaction-{codex,claude,pi}.log`。全部会话和认证均为隔离合成数据。

- agent-runtime 的 16 个小型不变量测试通过，含压缩通知不隐藏回复、Pi 进行中跨刷新与取消、自动压缩后普通回答失败仍保留原生错误。
- bindings／agent-runtime 的 all-targets 严格 clippy 通过；remote-only bindings 检查通过。
- Mac 全源码与本任务新生成绑定的 Swift 类型检查通过，仅有既存 ResourceLibrary 外部类型 Identifiable conformance 警告；未把类型检查当作完整 GUI 验收。
- iOS 模拟器目标的全源码（含共享遥测源）与新生成绑定类型检查通过；隔离检查路径里保留的 Mac Swiftmodule 导致 canImport 警告，不代表产品编译错误。
- Android 使用新生成绑定的 `compileDebugKotlin` 通过，没有替换现有设备上的 native 库或安装应用。
- Windows 独立源码目录 `C:\Users\yyh\velune-context-compaction` 的 Rust Release、生成 C# 与 WinUI publish 已通过；浅／深色各 4 个实际控件状态（空闲、能力不支持、异地运行、压缩中）通过，压缩中仅保留专用状态，不重复通用“正在运行”。截图已检查，两次进程退出码均为 0、无 crash.log；临时验收入口与桌面启动任务均已移除，最终 WinUI publish 再次通过。

## 限制与收尾

Codex 0.159.3 的失败／取消压缩可能只保存普通原生回合错误，未持久化可辨识的压缩条目。重开按原生历史与恢复资格呈现，不能承诺恢复那次主动压缩的身份、原回复归属或恢复资格。没有通过文案猜测，也没有新增 Velune 压缩日志。

未调用真实服务商、未进行真人交互手感或移动设备运行验收，未覆盖日常安装；代码按后续授权提交。实施与开发方验收已完成。7 份相关文档的 166 个相对链接与标题锚点、`git diff --check` 和 workspace fmt 检查通过。最终日志、对应核心库与 Windows 原生截图保存在 `target/context-compaction-verification/`；临时克隆构建缓存已删除，避免保留无关的大量旧构建产物。

## 命令入口与复审（2026-10-27）

用户要求补齐命令入口、复审过度实现与架构，并允许提交。Mac 的命令面板、菜单与 `/compact` 已共用 `ShortcutCommand.compactContext`；本轮补齐 Windows 的 `Command.CompactContext`、可配置快捷键和更多菜单，按聚焦会话的 `CanCompact` 判定。复审发现 Managed Pi 压缩前绑定模型失败会绕过现有网关收尾，将 begin／bind／compact 纳入同一错误处理范围。本轮只提交本任务文件与混合文档中属于本任务的段落，命令入口及失败清理的验证结果见下文。


复审结论：保留现有 application 管会话资格、队列和执行资源、adapter 管原生协议与状态投影的分工，不增加通用维护框架或能力注册层。需要修正的具体问题有两组：

1. Managed Pi 的压缩前模型绑定拒绝可能绕过网关和客户端清理，现已纳入现有错误收尾范围。
2. 主动压缩中的控制连接错误原先使用普通 Notice，会把上一条最终回复重新归入工作过程。现用独立压缩状态保留原因，覆盖准备阶段断连；已经原生确认完成的压缩记录不被后续 EOF 覆盖。新增 Native 三阶段和 Pi 开始通知前后断连回归。

Windows 的统一命令、更多菜单和快捷键保存已在 win-ws 的隔离构建验证，浅／深色各通过原有 4 组控件状态及新增命令资格、快捷键持久化检查，正常退出且无 crash.log，最终无临时验证入口的 publish 通过。截图位于 `target/context-compaction-verification/review-windows-ui/`。9 份相关文档的 182 个相对链接与标题锚点通过。


复审后的本机核心构建、18 项 agent-runtime 测试与 agent-runtime all-targets 严格 clippy 通过。Native 合成控制协议测试覆盖压缩准备阶段断连、进行中断连和成功条目确认后断连；Pi 单元回归覆盖开始通知前后断连。Mac 以实际命令、AppStore 和 NSTextView 代码验证命令面板中英文检索、同一动作分派、`/compact` Return 执行、禁用保留草稿、运行中／异地状态拒绝，全部通过。Codex 与 Claude Code 的实际 CLI 加合成上游各通过原有 5 组及新增进程强制退出检查，确认原回复与时长保留，状态完成。新增日志以 `target/context-compaction-verification/review-` 开头。


Pi 1.0.2 的复审联调同样通过全部 6 组，并通过额外的模型绑定拒绝实验：仅在隔离 helper 中拦截 `set_model` 返回合成错误，实际核心返回错误后子进程已退出、网关监听端口已关闭、会话未进入压缩状态且可重试。实验源为 `target/context-compaction-bind-failure-check.py`，实际结果为 `target/context-compaction-verification/review-pi.log`。三种 CLI 的断连检查均只终止验收进程自己启动的 runtime 子进程，不触及其他会话。最终核心构建日志是 `review-core-build-final.log`，Windows 构建是 `review-windows-build.log`。

复审与修正已完成，提交包含完整主动压缩功能、桌面命令入口、失败收尾修正及对应测试和文档。没有新增依赖、持久化压缩日志或通用维护框架；未覆盖真实提供商与移动真机，未安装或推送。工作区其他任务与用户指引修改不纳入提交。
