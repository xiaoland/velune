# 输入区状态、轮次展示与恢复

2026-10-30 用户要求一并修复：composer 状态归属随布局变化（独立停靠／浮动时不随会话切换）；压缩与错误状态不要一直挂在 turn 底部；粘贴图片及 Finder 文件为附件；仅正常完成 turn 后自动折叠工作过程；原生运行时未重试时由 Velune 指数退避，显示状态，考虑统一 overloaded 类型与组件。另探索在缓存即将过期前自动压缩。

授权：前五项实现；自动压缩按“考虑”先核对事实与提出方案。未授权提交、重装日常应用或真实模型请求。当前工作区还包含之前已完成的侧聊与 SVC 改动，保留。

当前阶段：已按用户“现在落地方案”的授权完成修订后的[技术方案](design.md)及[实施计划](plan.md)，源码和验证结果可供验收。未提交、未重装或退出日常 Velune；其他任务的未提交工作保留。

实施进展：已统一内部 Executions／活动终态、迁移适配器和历史／续接装饰／转录投影，删除 Message 结果补记；Codex 恢复使用异步起轮及中断，单一阶段派生展示，预算仅显式继续重置。Mac 已改用 ComposerContext，移除弱引用模式覆盖，语义修订保存并保护迟到失败回包。隔离应用组合验证、实际进程恢复和最新源码 Mac 构建均已通过。新增应答屏障、steer／独立维护、Pi 确认／命名空间测试；最新 45 项 Rust 测试通过（含当前工作区新增的 Pi 终态检查）。

本轮组合证据：合成 Codex CLI 的应答屏障验证了恢复待答不阻塞另一已加载会话的快照／停止，待答停止只发一次中断，提前 item 与完成通知不丢失，正常完成可赢得取消竞态；另覆盖退避／起轮中断线、拒绝、迟到回复、原生 willRetry 不叠加及耗尽后同配置显式继续的新预算。Pi stop／error／aborted／仅 settled 与身份确认、逻辑会话命名空间、steer 共享结果、独立维护分组均有 Rust 夹具。

Mac 隔离应用最新 `--check-composer-state` 通过隐藏底部区域→换目标→命令面板改选，会话选择保持原值；原生图片／Finder 文件粘贴、内置／停靠／浮动切换、不同窗口模型的缓冲隔离、迟到失败不覆盖新输入或另一会话草稿均通过。实际退出该副本后重启，`--check-composer-recovery-read` 确认正文 `RESTORED DRAFT v1 + edited`、两个附件引用与明确选择恢复，并经原生 AX 界面核对。日志位于 `/tmp/velune-composer-Hlu1HJ/`；没有操作日常 Velune。首次恢复实验遗留状态暴露的 layout detach 粘贴回归已修正；开发退出检查已移到 RunLoop，避免在 main DispatchQueue 内嵌套 AppKit 退出等待。

最终 45 项 Rust 测试及 workspace 全 features／targets 严格 clippy 已通过；`svc run build-macos` 已成功构建当前源码（run `c9a72760-9235-4200-86cb-cf0f83d7f466`）。日志 `/tmp/velune-context-tests-final-check.log`、`/tmp/velune-context-clippy-release.log`、`/tmp/velune-context-build-source-current.log`；最终隔离应用写入／重启证据为 `/tmp/velune-composer-Hlu1HJ/final-write-err.log` 与 `final-read-err.log`。真实提供商、跨设备 GUI 与其他平台真机构建未纳入本轮验收；不将合成夹具或模型隔离检查称为这类证据。

独立评审：用户随后明确要求送 advisor 获取独立评审建议。独立上下文的 advisor 已完成只读核对，建议保留总体路线，先澄清显式继续的活动周期语义，再进入验收设计／实施准备；不将评审请求解释为用户接受方案或进入实现。

采纳结果：已对照 `NativeSession::start_turn` 每次显式开始时重置恢复配置的路径、Pi 的 `stopReason`／`agent_settled` 处理及核心发送队列暂停规则，采纳 advisor 的两项建议并修订 [design](design.md)：展示执行关联保持稳定，每次显式发送／继续建立新的活动周期，重新捕获配置、预算和取消意图；自动重试与 steer 不重置周期。application 每周期处理一次终态，不能按展示执行 ID 永久去重。另将各适配器明确终态来源清单列为实施准备义务，特别保留 Pi `aborted` 的取消含义，不能以闲置或非 error 推断成功。没有未采纳的实质分歧；该采纳是技术方案修订，不是用户验收。

评审边界：advisor 检查了方案、任务包、相关产品／架构契约与 Mac、Codex、Pi、应用结束和转录路径，未运行新实验，未完整核验 Claude／DSH／ACP 的全部终态出口。其返回是设计建议，不是修复完成证据；主 agent 在该次评审采纳阶段只修订任务文档，未修改源码。

示例澄清：用户质疑 M1／M2 是否意味着 provider fail-over。它们原指用户主动改选的两个模型，例子表述不够明确；已改为保持原有模型／路由配置、用户主动点击继续的例子，并明确本轮兜底不引入 provider fail-over、不改变已有网关路由策略。活动周期只决定本次明确操作的配置捕获、重试预算、取消及终态消费，不承担自动更换提供商的职责。

独立 composer 的方案前提沿用上一轮已说明的处理：“草稿、附件、撤销与明确的下一轮选择均归独立输入区”，未选项按目标会话解析；这是本设计接受评审的行为前提，此前的可选询问不构成额外等待事项。

完成依据：合成状态验证两类 composer 的切换与发送归属；原生粘贴附件；失败／停止／成功的工作过程展示；合成 overloaded 与原生 willRetry 对照，退避、取消和恢复不重发原用户消息、不重放已完成的工具调用，不承诺运行时后续不会自行决定再次调用同一工具。自动压缩依据以官方缓存与运行时契约为准，未知到期时间不伪造。

修正前的基线：上一轮曾报告前五项的 Mac 实现与验证完成；随后架构审查修订了该结论，形成以下六项缺陷，本轮已据方案修正。第六项的调查与建议已可供用户决定，尚未启用后台自动压缩。未提交、未重装日常应用，早先侧聊与 SVC 的工作区改动保留。

2026-10-30 架构可维护性审查：用户要求审查本轮实现，授权范围为只读审查与任务记录。补充组合路径核对后，上述“完成”结论需修订：以下问题当时尚待修复；该次审查未修改源码。

- 高优先级：[自动重试派发](../../packages/agent-runtime/src/native/codex_retry.rs)在 `poll` 中同步等待 `turn/start`；应用服务线程串行推进全部会话，慢应答会阻塞其他会话更新与停止命令。应复用异步请求及待答状态，让轮询保持非阻塞。
- 高优先级：[composer 绑定](../../app/mac/Composer.swift)依赖视图挂载回调来给 [ConversationStore](../../app/mac/ConversationStore.swift)安装选择归属。独立底部输入区收起、切到另一标签再从命令面板选模型时，会写入会话的选择；重新展开后被独立输入区原选择覆盖。布局、聚焦目标与命令应共用窗口管理的输入上下文，不依赖视图是否存在。
- [轮次结束记录](../../packages/application/src/local/session.rs)按原生用户消息 ID 写入结果，但续接投影的消息 ID 经 [continuation](../../packages/application/src/local/continuation.rs)加上 `segment-{index}:` 前缀；Pi 等依赖应用记录结果的适配器因此丢失结束结果。结果与消息应经过同一身份转换边界。
- [原生结束处理](../../packages/agent-runtime/src/native/mod.rs)和 Codex 都仅给最后一条用户消息标记结果，而 [转录投影](../../packages/agent-runtime/src/transcript.rs)按每条用户消息建立工作段。同一原生 turn 中 steer 加入第二条用户消息后，第一段工作即使正常结束也没有完成结果。应按执行区间记录结果，再映射到该区间全部展示段。
- [连接失败处理](../../packages/agent-runtime/src/native/mod.rs)直接清掉重试定时器，未终止对应消息块的 Waiting 状态；进程在退避中退出后，会话已 Failed，界面仍承诺将重试。重试执行与展示应由同一状态转换收尾。
- [转录分组](../../packages/agent-runtime/src/transcript.rs)仅用消息是否 completed 区分压缩状态；主动维护没有新用户轮次，完成或失败后会进入上一已完成轮次的折叠工作组。应在压缩来源处保留“轮次内／独立维护”的归属，分组按归属处理。

补充证据：`/tmp/velune-architecture-review-20261030` 为临时探针，直接调用当前 Rust 实现；续接 ID、steer 展示段、独立压缩分组、慢应答重试、退避中连接退出五条路径均复现。慢应答夹具延迟两秒，单次 `poll` 实测阻塞 2000 ms。隐藏 composer 的路径通过实际挂载条件、聚焦切换、命令回调与选择写入调用链核对，未另做 GUI 复现。没有读取真实会话、凭据或调用模型服务。草稿持久化初看缺少正文变更通知，但原生编辑回调每次都会发布输入法状态变更，因此没有把“普通编辑丢草稿”列为已确认缺陷；当前保存对该副作用的依赖可随 composer 归属修复一起收敛。

验证：

- `cargo test --offline -p velune-agent-runtime -p velune-application-api -p velune-application`：38 项通过。协议夹具覆盖终止 overloaded 的等待与空输入恢复、只保留一条用户消息及已完成工具、原生 willRetry 不叠加、其他错误不重试、停止与次数上限；投影覆盖完成状态进入工作组、正文后状态不改变回复归属、恢复轮次继承结果、单独压缩不把取消改为成功。
- workspace 全 features／targets 的严格 clippy、release bindings 与 Mac warnings-as-errors Swift 构建通过。Swift／Python 绑定已再生在 target 下。日志 `/tmp/velune-recovery-tests-final.log`、`/tmp/velune-recovery-clippy-final.log`、`/tmp/velune-recovery-release-final.log`、`/tmp/velune-recovery-swift-done.log`。
- `/tmp` 隔离预览应用的 `--check-composer-state` 通过：命名 pasteboard 的 PNG（同时带文字）与 Finder file URL 进入附件；两个不同会话的编辑状态、下一轮选择隔离；底部停靠／浮动／回到会话输入区不覆盖原草稿；独立草稿、附件引用与选择随 SavedWindowLayout 编码、解码与重建。未读或替换用户剪贴板。CUA 实际切换标签后 ⌘Z 撤销原输入区编辑成功，发送进入当前聚焦会话。日志 `/tmp/velune-composer-check-ui.log`。
- 真实 Codex 0.159.3＋合成回环 Responses 上游：一次 `response.failed/server_overloaded` 触发了原生重试，观察到 Native → Recovered、运行中可停止，没有叠加 Velune 兜底。随后侧聊多轮、停止／关闭、原会话继续通过。日志 `/tmp/velune-recovery-real-final.log`。这说明是否重试取决于原生错误路径，不能把“Codex 一概不重试”当作能力事实。早先临时脚本以总行数判断新回复，因重连后原生历史不保存中间错误行而误报；改为验证两条用户消息与两条回答，错误与重试状态另外逐项验证。
- 文档相对链接／锚点检查 `bad 0`，`git diff --check` 通过。

边界：没有真实账户或真实模型联调；Mac 新交互已验，其他平台只补充新增错误类型的基本显示，未进行平台构建验收。没有明确结束证据的旧历史保持工作过程展开，不推断为正常完成。独立草稿的恢复沿用系统窗口恢复设置，本轮验证新状态的序列化与重建；撤销历史仍只在窗口存续期间保留。

自动压缩调查（提案，尚未启用）：[Claude Code 官方缓存文档](https://code.claude.com/docs/en/prompt-caching)说明 `/compact` 会读取旧上下文再替换缓存前缀，下一次请求需缓存摘要新前缀；它已有 idleCompaction（2.1.290+，一小时 TTL 的长空闲会话）。当前 Velune 的 Claude turn 子进程结束后不会常驻，因此不能声称原生空闲触发仍有效。[Anthropic](https://platform.claude.com/docs/en/build-with-claude/prompt-caching)按请求开始与最后命中刷新 TTL；[OpenAI](https://developers.openai.com/api/docs/guides/prompt-caching)的有效期限随模型与缓存策略变化。当前通用运行时没有可靠统一到期时间，不能从 turn 结束或 token 数伪造。建议另做显式可选策略，只在期限有来源、会话空闲且无待答／子 Agent 时触发一次；它会主动发送模型请求并改写上下文，需要用户决定是否采用。
