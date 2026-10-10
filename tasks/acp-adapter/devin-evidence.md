# Devin CLI ACP peer evidence

检查日期：2026-10-10。仅在 `target/devin-probe` 下安装，使用临时 `HOME`、`XDG_DATA_HOME` 和工作目录；清除了探针环境中的 API key、`WINDSURF_API_KEY` 和 Devin token。没有登录、读取现有配置或历史，也没有发送真实模型请求。

官方资料确认 Devin CLI 的安装入口是 [`devin.ai/cli`](https://devin.ai/cli)，ACP 命令是 `devin acp`；[官方命令参考](https://docs.devin.ai/cli/reference/commands)说明本地 ACP 会使用 CLI 自己的认证存储或 `WINDSURF_API_KEY`，并支持 `--model` 与 `--agent-type`。本次使用官方 `https://cli.devin.ai/install.sh`，没有全局安装。安装得到 Devin CLI `3000.11.3 (9c803229faa4)`。安装器随后尝试登录并输出 `Error: Login canceled`，所以后续实验保持未认证状态。

## Rust generic Client 实测

使用当前 `packages/agent-runtime` 的 Rust `acp::Client`，直接启动绝对路径 Devin binary 和参数 `acp`，initialize 成功，返回：

- `agentInfo.name`: `affogato`
- `agentInfo.title`: `Devin Agent`
- `agentInfo.version`: `0.0.0-dev`
- auth method: `devin-browser`（Log in with browser）
- capabilities: `load_session=true`、`list_session=true`、`delete_session=true`；`resume_session=false`、`close_session=false`

`session/new` 在未认证状态也成功，返回 native session ID 和两个 config options：`mode`（`accept-edits`、`ask`、`plan`、`bypass`）以及 `model`（当前值 `swe-1-6-fast`，choices 为空）。同一进程内 `list_sessions` 能看到刚创建的会话。

### 实验一：shutdown 后的 fresh list/load

首次运行只执行 `new`、同进程 `list_sessions` 和 `Client::shutdown`，没有调用 `delete_session`。随后使用相同隔离目录创建第二个 `Client`：`list_sessions` 返回空，尝试 `load` 原 native ID 得到 ACP RPC `-32016: Session not found`。这是未认证、未发送 prompt 的空会话在该实验中的跨进程观察；不能外推到已登录或已执行会话。

### 实验二：config、active load preflight 与显式 delete

另一轮同样未认证且未发送 prompt：将 `mode` 从 `accept-edits` 设置为 `plan`，随后读回值确认为 `plan`；同一 Client 的 `list_sessions` 能看到空会话。调用 `load` 明确返回 `此 ACP Agent 不支持关闭会话，请为新会话创建连接`，这是 adapter 在 peer 未公布 close 能力时拒绝切换上下文，而非伪造 load 成功。随后对该 native ID 显式调用 `delete_session` 并成功，之后 shutdown 和 fresh list 为空；该轮的空列表由显式 delete 解释，不能用来判断 shutdown 的持久化行为。

两轮都没有发送 prompt：即使安装过程被取消登录，ACP `new` 仍能成功，但无法在不触发真实模型请求或登录流程的前提下证明 prompt 的认证错误，所以探针在此停止。安装器的 `Login canceled` 没有留下 Devin/login 进程。也没有继续登录，因为那会需要用户身份/凭据并可能产生费用。

## 结论

Devin CLI 是可启动并能通过 Velune Rust ACP Client 完成 initialize、new、config discovery、config mutation、live list 和空会话 delete 的原生 ACP peer。当前隔离未认证条件下，peer 明确未公布 resume/close；实验一观察到未删除空会话在 shutdown 后 fresh list/load 不可见，实验二则单独证明了 delete 的显式删除行为。`agentInfo` 为 `affogato`/`0.0.0-dev`，没有使用 Windsurf identity 伪装。
