# DSH ACP native peer evidence

检查日期：2026-10-10。使用隔离的 `DSH_HOME`、`HOME` 和工作目录启动已安装的 DSH ACP profile；未读取真实配置、凭据或历史。provider 配置为临时 loopback Anthropic Messages fixture，API key 为合成值，未调用真实模型网络。

参考版本：`acp-reference-acpsub` `259db0d`；`acp-reference-protocol` `af9d8f5`。实际 DSH CLI 版本为 `0.2.0-rc.2`；ACP `agentInfo` 单独报告 `deepseek-harness-acp` `0.0.1`，ACP wire 为 v1。

## 可观察协议

真实 peer 对 `initialize` 返回 `protocolVersion: 1`、`agentInfo`、`authMethods: []`，并声明 `sessionCapabilities.close/list/resume`，以及仅 HTTP MCP 和 text-only prompt 能力。`session/new` 返回 session ID 和 `configOptions`；在默认 profile 下模型 option 的值是 opaque JSON pair，例如 `["deepseek-official","deepseek-v4-flash"]`，reasoning option 也会被公布。

给 `llm-pi-ai` 加入临时 `synthetic` provider（`api: anthropic-messages`、loopback `baseURL`、合成 `apiKeyEnv`、显式模型）并将 ACP 初始 provider/model 指向它后，真实 `session/new` 返回：

- model `configOptions` 分组包含 `synthetic/synthetic-model`，值为 `["synthetic","synthetic-model"]`；默认 DeepSeek provider 仍同时存在；
- `session/set_config_option` 接受该 opaque value，并返回完整更新后的 `configOptions`；
- `session/prompt` 触发 loopback `POST /v1/messages?beta=true`，请求使用 `x-api-key: synthetic-key`；
- 合成 SSE 正文 `ACP_SYNTHETIC_ANSWER` 被映射为 `session/update` 的 `agent_message_chunk`，随后发送 `usage_update`，请求响应为 `{ "stopReason": "end_turn" }`。

这证明真实 DSH ACP peer 的 prompt、provider/model opaque config option 和 loopback provider dispatch 可以在隔离环境中闭环；这是第一阶段的 wire-level 结果。完整 Rust 泛型 adapter 的真实 peer 验收见下文。

## DSH 与通用 adapter 的边界

`session/list`、`session/resume`、`session/close` 会在 initialize 能力中公布；`session/load`、fork、delete、额外目录和 elicitation 不在该 DSH profile 的能力中。模型选择必须回放 peer 公布的 opaque pair，不能自行拼接 provider/model 字符串。`session/new` 会返回当前完整 `configOptions`，而不是独立的静态模型表。

## Rust 泛型 Client 端到端验收

同日使用临时 Rust harness（依赖当前 `packages/agent-runtime` 的 `acp::Client`）重新执行真实 peer：`Client::connect` 启动已安装的 DSH ACP profile，随后依次调用 `create`、读取并选择 `configOptions` 中的 opaque 值 `["synthetic","synthetic-model"]`、`send`/`poll`、`close_session`、`list_sessions`、`resume` 和最终 `close_session`。DSH 的模型请求实际到达隔离 loopback `POST /v1/messages?beta=true`，请求包含合成 `x-api-key: synthetic-key` 与 `model: synthetic-model`；SSE 正文 `ACP_RUST_ANSWER` 出现在 Rust snapshot 的 assistant message 中。整个命令返回 `PASS`，退出码为 0。

该验收确认泛型 Rust ACP Client 已经能通过真实 DSH peer 完成创建、配置、prompt、目录发现、恢复和关闭，而不是只通过合成 ACP server。DSH 在 live session 尚未关闭时不会把新记录返回给 `session/list`，所以 harness 先关闭初始上下文再列出，再以列出的 native session ID 调用 `resume`；这是 peer 的持久化时序要求。临时工作目录先 canonicalize，以匹配 macOS `/tmp` 到 `/private/.../T` 的路径解析；没有读取真实 HOME、凭据或历史，也没有调用真实模型网络。

## 正常 shutdown 的持久化验收

在同一隔离 loopback fixture 上，Rust harness 完成一次真实 prompt 后只调用 `Client::shutdown`，没有显式发送 `session/close`。随后用相同隔离 `DSH_HOME` 创建全新 `Client` 并执行 `list_sessions`，成功找到刚才的 native session ID；命令退出码为 0。fixture 仍只收到合成 key 的 loopback `/v1/messages?beta=true` 请求。

这确认应用正常退出路径可以依赖 `Client::shutdown`：当前实现会在空闲且 peer 公布 close 能力时先关闭 session，再有界地结束 ACP 子进程；新 Client 能继续发现会话。所有探针结束后已清理自建 loopback 服务器和临时运行目录。
