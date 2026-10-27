# 本机 Codex／Node 进程归属排查

2026-10-25。用户发现“Codex CLI 约 925 MB、208 个 Node 子进程”，要求排查是否由 Velune 引起。本次授权为调查；不修改源码、不提交、不终止进程。已有未提交改动属于其他工作，保持不动。

完成标准：核实当前进程的可执行文件、父子关系、数量与内存口径；核对 Velune 的启动和清理实现；给出有证据支持的归属结论与仍无法确定的事项。调查不读取真实凭据或会话正文。

## 结论与证据

排查完成。2026-10-25 13:25–13:27（Asia/Shanghai）使用 `ps` 获取进程树、`lsof` 核对工具进程工作目录、`vmmap -summary` 核对 physical footprint。PID 与内存均为当时快照，不作为持续状态。

| 进程 | 父进程 | 可执行文件 | Physical footprint | Node 类后代 |
| --- | --- | --- | --- | --- |
| Codex app-server，PID 27119 | ChatGPT，PID 26385 | `/Applications/ChatGPT.app/Contents/Resources/codex-cli/CodexCLI.app/Contents/MacOS/codex` | 998.6M，峰值 1.1G | 139 个 node＋134 个 node_repl，共 273 个 |
| Codex app-server，PID 63818 | Velune，PID 84642 | `/opt/homebrew/bin/codex` | 96.7M，峰值 195.0M | 1 个 node＋2 个 node_repl，共 3 个 |
| Velune，PID 84642 | launchd | `/Applications/Velune.app/Contents/MacOS/Velune` | 552.7M，峰值 596.5M | 同上 |

大批进程属于 Codex 桌面应用内置后台，进程归属不支持“这批进程由 Velune 启动”的猜测。桌面应用的 bundle 标识为 `com.openai.codex`，版本为 26.1013.31256（20586），虽然安装路径和主进程名是 ChatGPT。用户没有手动启动 CLI 与存在这个后台并不矛盾。

桌面 app-server 从 10 月 21 日 10:30:14 起运行。直接子进程包括 45 个代码审查插件 Node 服务（工作目录 `.codex/code-review-plugin`）、45 个 `codex-app-tools/0.1.6` Node 服务、45 个 `cua-repl.mjs` Node 服务和 89 个 node_repl。其余 Node 类后代是这些服务的子进程。273 个 Node 类后代分布在 10 月 21–25 日启动；这说明存在多批长期保留的工具实例，不足以证明每一批均已失去用途或发生泄漏，也不代表有 273 个推理中的 Agent。

Velune 从 10 月 23 日 21:47:28 起运行；其当前 Codex 从 10 月 25 日 13:24:38 起运行。采样中还出现过第二个 Codex（PID 67165），后续已消失；没有观察到 Velune 下同规模的进程积累，也未发现 PPID 为 1 的 Codex／cua_node 残留。当前数量和用户此前的 208 不同，不能在缺少此前 PID、时间和统计口径的情况下声称精确复现原快照。`ps` RSS 明显低于 footprint，报告内存以 `vmmap` 的 footprint 为准。

## Velune 代码核对

当前仓库的 [执行启动](../../packages/agent-runtime/src/native/mod.rs) 与 [历史客户端](../../packages/agent-runtime/src/native/codex.rs) 都显式启动 `codex app-server`；[历史读取](../../packages/agent-runtime/src/native/codex_history.rs) 因此也会短暂启动 CLI，无需用户打开终端。[RpcClient](../../packages/agent-runtime/src/native/rpc.rs) 在 Drop 时调用 shutdown；[ProcessTree](../../packages/agent-runtime/src/process_tree.rs) 在 Unix 上建立独立进程组并向该组发送 SIGKILL。

清理实现的边界仍需保留：当前 Velune 的 CUA／node_repl 子进程已建立与 Codex 不同的进程组，组信号本身不覆盖它们，是否退出还依赖管道关闭后的自身处理。此次未终止应用、未做退出回收实验，不能据此证明 Velune 的所有退出路径都无残留；这也不能解释目前明确挂在桌面 app-server 下的数百进程。

## 后续建议与验证边界

若目标是释放本次识别的大批进程，最小操作是待工作结束后完整退出并重新打开 Codex 桌面应用，并比较退出前后的 PID 与数量。该操作会中断当前聊天及桌面应用下的后台任务，本次未执行。要进一步认定泄漏，应比较同一聊天结束或关闭前后对应工具实例是否退出，并排除仍载入的聊天继续持有它们。

只新增本任务包，未改源码、未提交、未终止任何已有进程，未读取真实凭据文件或会话正文。没有运行构建、模型调用、退出实验或泄漏复现；结论限于当前进程归属、快照内存与源码清理边界。
