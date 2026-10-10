# Claude Code 2.1.286 隔离探针证据

检查日期：2026-10-10。使用已安装的 Claude Code，版本 `2.1.286 (Claude Code)`。所有探针使用全新 `/tmp/velune-claude-*` HOME、`CLAUDE_CONFIG_DIR` 和合成 token；上游只绑定 `127.0.0.1`，没有读取已有配置、凭据或会话，也没有调用模型网络。

## 已验证

- `--print --verbose --output-format stream-json` 是必要组合；缺少 `--verbose` 时 CLI 直接报错：`--output-format=stream-json requires --verbose`。
- `--input-format stream-json` 接受一行控制消息：

  ```json
  {"type":"control_request","request_id":"init-1","request":{"subtype":"initialize"}}
  ```

  输出 `control_response`，`response` 中包含 `commands`、`agents`、`models`、`account`、`pid`、`current_permission_mode`、`pending_permission_requests`、`pending_user_dialog_requests`、`session_state` 和 `capabilities`。本机返回的 `models` 至少包含 `value`、`resolvedModel`、`displayName`、`description`、`supportsEffort`、`supportedEffortLevels`、`supportsAdaptiveThinking`、`supportsFastMode`、`supportsAutoMode`；自定义 `--model synthetic-model` 也出现在列表中。initialize 返回 `session_state: "idle"`。
- 后续会输出 `system`/`init`，其中有 `session_id`、`model`、`permissionMode`、`tools` 和 `claude_code_version`。一次成功合成 turn 的 stream-json 顺序包含 `system status(requesting)`、多个 `stream_event`（`message_start`、`content_block_start`、`content_block_delta`、`content_block_stop`、`message_delta`、`message_stop`）、聚合 `assistant`，最后 `result`；`result` 含 `session_id`、`terminal_reason: "completed"`、`is_error: false`、`result` 文本、`usage` 和 `modelUsage`。
- Anthropic upstream 请求为 `POST /v1/messages?beta=true`。因此 adapter 传给 `ANTHROPIC_BASE_URL` 的值若已经带 `/v1` 会得到错误的 `/v1/v1/messages`；需要在注入环境前去掉末尾 `/v1`。
- managed provider 实验仅设置 `ANTHROPIC_AUTH_TOKEN=synthetic-managed-token`（同时设置 `CLAUDE_CODE_PROVIDER_MANAGED_BY_HOST=1`），请求使用 `Authorization: Bearer synthetic-managed-token`，不会发送 `x-api-key`；`system init` 的 `apiKeySource` 为 `none`。设置 `ANTHROPIC_API_KEY` 时则使用 `x-api-key` 且 `apiKeySource` 为 `ANTHROPIC_API_KEY`。这个实验显式使用 `--no-session-persistence`，只验证单次 provider 注入和 HOME 传入 classifier context，不产生可供 resume 的持久 session。
- 另一个独立的持久化实验没有使用 `--no-session-persistence`。同一个合成 session ID `22222222-2222-4222-8222-222222222222` 完成 native → managed → native 三次 `--resume`：第一、三次请求 native loopback 并返回 `native`，中间一次请求不同的 managed loopback 并返回 `managed`；三次 `system init` 和上游 `x-claude-code-session-id` 保持同一 ID。第三次 native 恢复到 native 模型，`modelUsage` 同时保留该 session 中 native/managed 两种模型统计。managed turn 使用 Bearer token，恢复 native turn 使用 x-api-key。
- settings 字节实验也是独立的合成配置实验。在 `CLAUDE_CONFIG_DIR/settings.json`（包含 env 和 custom 字段）上运行成功 turn 前后，SHA-256 均为 `d32918647740479c4bb5e0f5c7263e37467de812997c9e22ccf4f6fd71e529a5`；本次 Claude CLI 没有改写来源 settings 字节。

## 未完成/限制

- 本轮没有获得可靠的 permission prompt 事件证据；一次工具流探针因临时 SSE 服务被提前终止而只观察到 CLI 重试。不要把权限字段写成已验证事实。
- loopback 服务器也收到一次 `HEAD /api/hello` 探测；这是 CLI 启动时的非模型请求。探针设置 `DISABLE_NON_ESSENTIAL_TRAFFIC=1` 与 `CLAUDE_CODE_DISABLE_NONESSENTIAL_TRAFFIC=1`，但该请求仍出现，adapter 不应假定这两个变量会阻止所有本地健康探测。
- 未验证真实 provider、真实登录、真实模型或 SSH；本证据仅约束 Claude Code SDK CLI 的本机 stream-json/Anthropic transport 形状。

## application/UniFFI 隔离验收

2026-10-10 使用当时已构建的 `libvelune_bindings.dylib`、生成的 Python bindings 和本地 resources，运行临时脚本经 `VeluneApplication` 进行闭环。临时脚本将 runtime binary 指向已安装的 Claude Code，并把 native turn 指向一个 loopback Anthropic Messages SSE fixture；managed provider 则通过 Velune gateway 转发到另一个 loopback fixture。没有修改产品脚本或源码。

验收通过：实际 Claude CLI 返回 native model catalog；application 取得 native selection 后完成 native turn，正文进入 conversation snapshot；同一 conversation 切换到 managed selection，gateway fixture 收到 `/v1/messages` 并返回 managed 正文；结束 managed turn 后再次选择 native，native fixture 再次收到请求并返回正文。application 的历史 projection 保留消息，跨 runtime continuation、managed HOME、SSH typed configuration 和合成来源 settings 不变检查均通过。native fixture 收到 `/v1/messages?beta=true`，managed gateway fixture 收到 `/v1/messages`；这是两层 endpoint 语义，provider endpoint 配置给 gateway 时不应额外带 `/v1`。

这次 application 验收的 provider API key 是合成值，loopback fixture 没有模型网络调用。未覆盖真实登录、真实 provider、真实 SSH 连接和 permission prompt。
