# 消息编辑、发送队列与继续

2026-10-15 用户要求编辑用户消息、发送队列与 steering、停止后继续。决定见 [S27](../../docs/sources.md#s27)，产品归 [PRD](../../docs/prd/index.md#消息编辑发送队列与继续)。

## 已确认的产品意图

- 运行中发送默认排队：队列显示在 composer 上方，可编辑、删除、调整顺序；turn 结束后逐条发出。
- 队列项可改为 steer，另有快捷键直接 steer；不支持原生 steer 的运行时只能排队。
- 中断沿用停止生成（Esc）；停止或失败后提供“继续”。
- 可编辑任意用户消息；最后一条直接原地继续，更早的询问 fork 还是原地继续。

## 已验证的原生能力（本机固定版本，2026-10-15）

| 能力 | Pi 1.0.2 | Codex 0.159.3 | Claude Code 2.1.286 | ACP v1／DSH |
| --- | --- | --- | --- | --- |
| steer | RPC `steer`（当前工具调用之后、下一次 LLM 调用之前送达） | `turn/steer` {threadId, expectedTurnId, input} | turn 进行中经 stdin 写入用户消息（stream-json 吸收） | 无 |
| 原地编辑 | 扩展命令上下文的 `navigateTree(entryId)`，旧分支留在会话树中 | `thread/revert` {threadId, beforeTurnId}，再 `turn/start` | `--resume <id> --resume-session-at <message uuid>`（＋`--resume-drops-turn`，仅 print 模式） | 无 |
| fork | RPC `fork` {entryId} 新建会话并返回该消息文本 | `thread/fork` {threadId, lastTurnId} | 上述参数加 `--fork-session` | 无 |
| 原生继续 | 内部 `agent.continue()`，最后一条是 assistant（含中止、失败）时只能发出已排队的消息，RPC 未暴露 | 无 | 无 | 无 |

来源：Pi 包内 `docs/rpc.md`、`dist/core/agent-session.js` 与依赖 pi-agent-core 的 `agent.js`；Codex `codex app-server generate-json-schema` 本地生成的 schema；Claude Code `--help` 与本机实测。只适用于上述精确版本。

## 方案

- 队列归 application 的会话（`Session`），各窗口共享；快照携带队列项，Mac 只呈现与发出操作。队列项保存发送时的完整选择（运行时、模型、推理强度、权限、附件），编辑只改正文。
- turn 正常结束后，core 在推进会话时自动发出队首；turn 失败或被停止时队列暂停，由用户“继续发送”或编辑、删除。发出失败（未被运行时接受）时该项留在队首并暂停，不自动重试。
- 队列只在应用运行期间保存，不跨重启；有排队项的会话不会被关闭。
- “继续”统一发送一条真实用户消息“继续”，沿用会话当前的选择；Pi 的原生 continue 在中止或失败的回复之后没有可继续的内容，故不使用。
- steer 由各运行时类型声明能力，经原生通道发送；不支持时界面只提供排队。
- 编辑用户消息：投影消息需对应原生标识（Pi entry ID、Codex turn ID、Claude 消息 uuid）。运行中、ACP／DSH、跨 Harness 接续的较早片段不提供编辑。fork 在新标签页打开。

分步实施并各自提交：1. 队列与继续；2. steer；3. 编辑用户消息。

## 验证

不新增自动化测试。Rust fmt／check／clippy，Mac 构建零警告；临时脚本对 loopback 合成上游与本机固定版本 CLI 走端到端路径；不调用真实模型 API，不读取真实会话。完成后重建安装，产品体验由用户验收。

## 进度

- 2026-10-15：记录决定与能力调研。
