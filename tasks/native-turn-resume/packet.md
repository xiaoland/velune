# 停止后原生继续

2026-10-26 用户要求核对 Codex app-server 在 interrupt／stop 后无需新用户消息继续生成的能力；若存在则接入 Velune，并检查其他运行时。已授权必要实现、验证与文档；随后明确授权提交并安装。primary 负责实现与交付，advisor 只读判断协议语义、能力边界与接口方案。

目标是“继续”不再必须发送一条“继续”用户消息。必须区分重新载入会话与真正启动生成；不重放已执行工具、不自动重新批准交互、不跨 Harness 假装恢复原轮次。现有队列暂停语义保持。

实施前事实：Mac 与 Windows 的“继续”均发送普通用户文本；core 的 can_continue 只表明停止／失败后可发送，不代表原生无输入能力。Codex adapter 支持 0.159.3 至 <0.164.0，NativeSession 与 application 的普通发送边界都拒绝空消息。旧任务关于“Codex 无原生继续”的结论仅依据 schema，不能证明运行时语义。

当前状态：Codex／Claude Code 的原生恢复已贯通共享核心与桌面端。2026-10-27 获批交互修正已实施：一个发送／排队／继续主按钮、停止独立，继续仅表示原生恢复且与正文／附件互斥；Pi／DSH／ACP 不显示继续，用户可自行输入文本发送。Mac 与 Windows 构建、真实 CLI 隔离回归和合成 UI 检查通过，已按原授权单独提交并安装，安装清单与验证见下文。其他任务的工作保留，不进入本次提交与安装。

完成依据：停止后的继续实际发起模型请求，转录与原生历史不新增用户消息；普通空消息仍拒绝；运行中／待交互／不支持的操作明确拒绝；失败不会隐式改发文本或重放；现有发送与队列回归通过。所有验证使用隔离合成资料，不读取真实配置、凭据、会话或调用真实模型。

## 已确认的协议与方案

实际 Codex 0.159.3 在隔离 HOME、回环合成 Responses 上游通过：首个模型请求等待时 `turn/interrupt`，收到 `interrupted` 后 `turn/start {input: []}` 完成；两个模型请求、原生历史仅一条 userMessage，恢复获得新 turn ID。公开 [app-server 文档](https://learn.chatgpt.com/docs/app-server) 区分 thread 载入与 turn 启动，schema 本身不能证明空输入的语义。

advisor 判断已采纳：独立 `resume_turn` 与 `can_resume`，保留 `can_continue` 的文本能力；仅同一上下文启用，不允许通过原生恢复跨 Harness 接续，不自动文本回退。普通发送的校验保留。无消息恢复跳过等待下一条用户消息的转录配置登记；无消息原生 turn 的用量累计到前一用户段，保持原发送配置。清除停止资格改到运行时实际受理后，避免拒绝请求导致按钮消失。Codex 原生历史的最后轮次状态用于重开后的恢复资格；未记录任何用户消息时不提供。

| 运行时 | 当前证据与结论 |
| --- | --- |
| Codex 0.159.3 | 原生空输入新轮次已实测，adapter 支持此路径。 |
| Pi 1.0.2 | 固定包 `pi-agent-core/dist/agent.js` 的 `continue()` 要求末条 user／tool-result；中断 assistant 且无队列时抛错；RPC 不提供 continue。全局安装的 Pi 1.4.0 不在本适配器范围内。 |
| DSH 0.2.0-rc.2 | 固定包 `@deepseek-ai/dsh-acp/lib/index.js` 拒绝空 prompt；`session/resume` 只是载入。 |
| ACP v1 | [session setup](https://agentclientprotocol.com/protocol/v1/session-setup) 的 resume 不回放历史，未承诺无输入生成；schema 的空数组不能作为通用行为保证。 |
| Claude Code 2.1.292 | [官方 session 文档](https://code.claude.com/docs/en/agent-sdk/sessions) 的 continue／resume 选择上下文。本轮随后对实际 CLI 实测空 `user.message.content: []`，中断后在同进程及 `--resume` 新进程下都继续请求模型，原生 user 记录没有增加，推翻最初仅根据文档作出的保留判断。adapter 已接入；未据此宣称旧最小版本 2.1.286 或所有未来版本均已实测。 |

当前已改 adapter、共享 command/API、bindings、远控 operation 目标与 Mac／Windows 的现有继续入口；iOS 只同步预览构造所需新字段，没有扩展其交互。开发验证结果见下文；提交与安装已完成，记录见末节。


## 实现与开发验证

Claude Code 原生中断记录保留原 `promptId`，但没有正常输入的 `promptSource`／`turnPosition`；历史读取据这些字段及精确原生标记识别中断，不隐藏用户自己输入的相同文字。回复后的原生中断标记不新建用户段；恢复用量累计到原用户段，流末尾 `message_delta` 的输出用量用于最终上下文。合成自检覆盖真实用户同文保留、原生标记恢复资格与后续回复解除资格。

- `cargo check --locked -p velune-bindings -p velune-remote` 通过。
- `cargo clippy --locked -p velune-agent-runtime -p velune-application -p velune-bindings -p velune-remote --all-targets -- -D warnings` 通过；无默认 features 与 remote-only（`--no-default-features --features remote`）的 bindings 静态检查均通过。
- `cargo test --locked -p velune-agent-runtime input_free_turn_usage -- --nocapture` 通过，验证无用户输入的原生轮次保留原配置、累计计量且不污染之后的用户段。
- 同源码 debug 动态库与 Python／Swift bindings 已生成。首次 CLI-feature 构建遇到本机 Cargo registry 符号链接造成 Askama 模板引用失效；只清理构建缓存，使用既有同版本 bindgen 读取新动态库，没有改依赖或项目构建配置。
- `swift build --build-system xcode --arch arm64 --configuration debug --product VeluneMac --force-resolved-versions`（链接本次 debug 动态库）通过，warnings-as-errors 保持开启。
- `manual-resume-turn.py` 对真实 Codex 0.159.3／Claude Code 2.1.292 的停止、冷重开、换模型与 effort 恢复均通过：无新用户消息、完成工具结果保留、未受理请求保留恢复资格、暂停队列不自动发送、随后普通消息配置正确、重开后的配置／用量一致。Codex 另外通过原生失败轮次的无输入恢复。Claude 的额外 HTTP 400 fixture 被 CLI 自行接续为成功，不能用它证明失败终态恢复；不扩大该部分验收结论。
- `manual-send-queue.py` 对固定 Pi 1.0.2 回归通过：队列编辑／排序／删除、逐条发送、停止／失败暂停、文本继续与空闲入队；明确断言 Pi 不承诺原生无输入恢复。仅使用临时 HOME、合成 key／会话／项目与回环请求。

Windows 已同步源码与提示，但 `ssh -o BatchMode=yes -o ConnectTimeout=5 win-ws pwd` 返回连接被关闭（198.18.1.2:22）；无法完成仓库要求的 Windows 构建与浅／深色实际运行。开发阶段未做 Mac 真人点击验收；后续已按用户授权安装到 `/Applications/Velune.app`。移动端本轮没有增加新按钮，远控契约已具备 `ResumeTurn`。


最后收口：原生运行时仍有子 Agent 工作或没有已记录用户输入时，暂不提供“继续”，避免暂不可恢复时改为发送文本；切换到另一运行时仍使用明确的文本接续。文档检查覆盖本任务 9 份 Markdown 中 207 个相对链接及其标题锚点，全部有效；`git diff --check` 通过。最终源码只补这一可用性判定后重建动态库，并重跑两种实际 CLI 的主要验收与 application 严格 clippy；两种 CLI 验收与 application 严格 clippy 均通过。


## 提交与安装

用户已明确授权提交和安装。原工作目录中仍有其他任务的未提交修改；本任务按文件及文档 hunk 单独暂存，在干净提交快照中构建安装包，不把其他工作带入提交或安装产物。安装使用仓库脚本，应用需正常退出，不强制终止活跃任务。


2026-10-26 提交与安装完成：功能提交为 `a55c06463053618913e8bdaf641d45f67cd85daa`（`feat(runtime): 支持停止后无新消息恢复`）。通过独立干净 worktree 执行 `bash scripts/build-macos.sh --build-only`，Release Rust、同版本生成绑定与 Mac Swift 构建通过；隔离 Release 动态库再次通过 Codex 0.159.3／Claude Code 2.1.292 的 `manual-resume-turn.py`。旧应用已正常退出，仓库安装器安装到 `/Applications/Velune.app`，保持 `0.1 beta.1`、arm64。安装清单 `source_commit` 对应该功能提交，`dirty=false`，锁文件摘要正确；安装前后严格签名校验通过，主程序、核心库、Info.plist 与构建清单哈希一致。动态链接仅指向随包核心和系统库，不依赖临时 worktree。构建与验收日志保存在原仓库 `target/native-turn-resume-install/`，临时 worktree 随后归档清理。未推送，未重新打开真实配置进行会话验收；Windows 验证状态仍如上。

## 2026-10-27 交互复核与获批方案

用户反馈会话进行时“继续”仍可使用、继续与发送没有互斥，要求重新思考。本轮按该请求先复核事实和交互方案；不修改产品源码、不提交、不安装。原生恢复能力已交付不代表 composer 的操作关系已通过验收，当前任务重新进入交互设计复核。

安装版清单为干净提交 `882d0614c4580d766bbc7e001886ecaf25b1093d`，已包含本任务与后续原生错误映射修复。对该安装版动态库使用临时 HOME、回环合成上游创建并停止 Codex 会话，然后用隔离子进程模拟原生 writer lock，并追加合成运行事件。实际快照为 `run_state=ELSEWHERE`，但 `actions.can_send=true`、`follow_up.can_continue=true`、`follow_up.can_resume=true`。没有提交恢复请求；证据是资格错误，不证明请求会穿透后续写入锁。脚本与日志在 `target/native-turn-resume-ui-review/probe-elsewhere.py`、`probe-elsewhere.log`。

原因链已由 primary 实验及 advisor 独立只读复核确认：历史运行检测只把 `run_state` 改为 Elsewhere；History 不算 `Session::turn_running()`；`follow_up` 又使用旧中断标记或历史恢复标记，未明确排除 Elsewhere。Mac 的 `isGenerating` 只识别 Running／Stopping，因此 `canSend` 和继续入口也可放行。普通本地 Running／Stopping 在代码里已有禁用判定；尚不能将用户所见的全部情况都归因于已复现的异地路径，后续还须覆盖活动列表与详情快照不同步的窗口。

其他静态缺口：Mac composer 在可继续且空草稿时添加“继续”，随后仍绘制“发送”；菜单／命令面板的继续判定不读取草稿与附件，发送菜单也只看 Store 的通道资格。Windows 继续入口不看草稿或附件，界面仍并列发送。各入口混用了运行时能力、当前执行状态、请求是否在途和草稿意图，重复布尔条件无法保证它们一致。

推荐方案是一个提交主按钮表达发送／排队／继续，停止保持独立控制。可发送的空闲会话有正文或附件就发送；空草稿只有同一上下文且原生恢复当前可用时提供继续，否则发送禁用。运行中有草稿可排队、原生支持时可插入本轮，独立提供停止；运行中空草稿只提供停止。停止中不接纳新提交或重复停止；发送／恢复请求在途不再次提交，草稿仍可编辑。待审批优先在交互卡处理，排队按核心能力提供；只剩后台工作时不提供恢复，发送沿用既有等待／复用规则并保留停止子 Agent。异地运行、未载入或只读子会话均不得提交恢复。

按钮、菜单、命令面板、快捷键和最终执行入口使用同一份“当前会话状态＋当前 composer 草稿／附件＋本地请求状态”的判定；核心也必须重新核实即时执行资格。Mac 可放在已有同时持有 Store 与 Composer 的 ConversationTab，Windows 使用实时输入状态，不等草稿保存定时器。发送键保留只提交真实草稿的语义，空输入不因发送键而恢复；恢复通过明确按钮、菜单或专用快捷键发出，输入法组合期间不提交。

建议将“继续”的产品语义收敛为无新用户消息的原生恢复，撤去按运行时／下一轮目标自动改发文字“继续”的 fallback。它改变了 2026-10-26 方案中的既有行为，当前仅为建议，未改 PRD 或实现。不支持恢复或切换了目标实例时，用户仍可输入文字继续并正常发送。队列区域使用明确的“继续发送队列”作为独立操作；继续本轮不解除队列暂停，队列操作也不暗中恢复原轮次。

后续验收应覆盖执行状态与草稿／仅附件的组合，以及正文按钮、菜单、命令面板、快捷键与 API 的一致性。关键过程包括停止尚未确认、双击继续、恢复请求在途输入草稿、菜单打开后草稿变化、运行活动先于详情快照到达、由已停止变为异地运行、审批／后台工作、恢复本轮与暂停队列同时存在。队列提交与停止完成并发时，需判别用户意图是否仍然有效，不能只靠按钮置灰掩盖跨客户端时序问题。


2026-10-27 用户已批准上述交互方案并要求更新文档，授权实现。当前贯通核心资格、Mac 单一提交动作与全部命令入口、Windows 对应状态，以及停止后的迟到排队请求；保留其他任务的未提交工作。验收增加实际安装版所复现的 Elsewhere 路径、原生恢复／草稿互斥、输入法组合与请求在途的状态矩阵，文档同步 PRD 与平台说明。


## 2026-10-27 实现与验证

核心仅在 Idle／Failed 且满足原生能力、无待交互／后台工作、已有原输入等条件时提供恢复；Elsewhere 同时撤销普通发送资格，并直接拒绝发送、恢复和队列新增／恢复。队列保留最后一轮停止／失败的原因，空队列的迟到 Add 暂停，已有队列保留用户明确的恢复决定。Mac 的 ConversationTab 和 Windows 的 Conversation.Submission 根据实时正文、附件、组词和请求状态统一提交意图。停止立即锁存；发送／恢复在途保留编辑能力并禁用重复提交。菜单及命令面板订阅实时输入；slash continue 消费自身命令文字后检查剩余附件，发送键始终只提交真实输入。继续的文本 fallback 已删除，队列按钮明确为“继续发送队列”。

- 干净任务快照的 `cargo clippy --locked -p velune-application --all-targets -- -D warnings`、完整 Mac Release 构建通过；当前工作目录的 bindings 检查与 Mac 开发构建也通过。独立 advisor 对状态、入口和时序只读复核，未发现阻止交付的问题。
- `manual-composer-submission.py --csharp` 编译实际 Swift／C# 判定文件，两端各 17 组状态组合通过。Codex 0.159.3、Claude Code 2.1.292 的真实 CLI 停止、冷重开、原生恢复、用量与配置回归在本次 Release 库上通过。Pi 1.0.2 的队列回归通过；新增停止后空队列迟到 Add 与失败后迟到 Add 的暂停断言。
- 异地运行的原安装版反例已消除：新快照的发送、继续、原生恢复资格均为 false；直接 API 的发送、恢复及队列恢复被拒绝，无新模型请求。已将此 Unix writer-lock 场景纳入 `manual-resume-turn.py --external-writer`，连同新增队列直接拒绝一并验证。
- Mac 使用临时源副本挂载实际 ComposerView／ConversationTab／Store，逐状态渲染截图，并核对命令与动作入口。已查看空草稿继续、有草稿发送、运行中空草稿只停止、恢复在途禁用提交的截图；实际 NSTextView 的 setMarkedText／unmarkText 生命周期也正确禁止／恢复提交。独立宿主未提供 AX 子树，因此不以 AX 自动断言作为证据。尚未做真人输入法候选面板的手感验收。
- `win-ws` 别名仍无法连接，但 `win-ws.localhost` 可用。在独立 `C:\Users\yyh\velune-composer-submit` 中完整运行仓库 Windows 构建脚本，Rust Release 与 WinUI publish 通过。随后仅在临时验收构建加入合成状态入口，浅色与深色各检查 11 组实际 SendButton／StopButton 的文字、可见性和启用状态；正文、仅附件、运行／停止、请求在途、组词、Elsewhere 与活动先于详情到达均通过，进程退出码均为 0、无 crash.log，截图外观正确。临时入口不进入产品源码与安装包，启动用隔离 HOME／TEMP，未改系统主题；一次性桌面启动任务已删除。

PRD、键盘规则、设计约束、两端说明与对齐表、application／bindings 契约和来源记录同步更新。10 份文档的 229 个相对链接与锚点及 diff 空白检查通过。日志、临时验收源和截图保存在原仓库 `target/native-turn-resume-ui-review/`；所有上游请求均为本机合成回环，不使用真实配置、凭据、会话或模型。


## 2026-10-27 提交与安装收口

交互修正提交为 `6df0bf14faa8ea36223735cbc1bba8011781cf6b`，Windows 活动先于详情到达的补齐提交为 `6c68cf3746d0356ce66f60a9ebc86c03a6d57ac4`。从后者的独立干净快照执行仓库 Mac Release 构建，随后用标准安装器安装至 `/Applications/Velune.app`（0.1 beta.1，arm64）。旧应用正常退出，没有强制终止进程。安装清单 `source_commit` 为 `6c68cf3…`、`dirty=false`；Cargo.lock 摘要一致，严格签名与资源封装校验通过，安装前后主程序、核心库、Info.plist、build-manifest 的 SHA-256 全部一致。动态链接仅引用随包核心与系统库，不依赖临时工作目录。

最终 Windows 产品构建已移除临时合成入口后再次通过；本轮没有覆盖 Windows 日常安装。工作目录中其他任务的未提交修改保留，未进入本次提交或 Mac 安装。构建及验证产物留在原仓库 target 目录，隔离源码工作区随后清理。提交未推送；没有在真实配置中打开会话验收。
