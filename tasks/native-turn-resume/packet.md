# 停止后原生继续

2026-10-26 用户要求核对 Codex app-server 在 interrupt／stop 后无需新用户消息继续生成的能力；若存在则接入 Velune，并检查其他运行时。已授权必要实现、验证与文档；随后明确授权提交并安装。primary 负责实现与交付，advisor 只读判断协议语义、能力边界与接口方案。

目标是“继续”不再必须发送一条“继续”用户消息。必须区分重新载入会话与真正启动生成；不重放已执行工具、不自动重新批准交互、不跨 Harness 假装恢复原轮次。现有队列暂停语义保持。

实施前事实：Mac 与 Windows 的“继续”均发送普通用户文本；core 的 can_continue 只表明停止／失败后可发送，不代表原生无输入能力。Codex adapter 支持 0.159.3 至 <0.164.0，NativeSession 与 application 的普通发送边界都拒绝空消息。旧任务关于“Codex 无原生继续”的结论仅依据 schema，不能证明运行时语义。

当前状态：Codex／Claude Code 原生恢复已贯通 adapter、共享 API、bindings 与 Mac／Windows 的现有继续入口；本机真实 CLI 的隔离端到端与 Mac 构建通过。Pi／DSH／ACP 保留文本继续。唯一外部验证阻塞是 win-ws 无法连接，Windows 构建与界面运行仍待具备主机条件后完成。仓库有其他任务的未提交修改，均保留；本任务已单独提交，Mac 已从干净提交快照构建并安装；其他未提交工作未进入安装包。

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
