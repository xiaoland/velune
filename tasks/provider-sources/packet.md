# 提供商来源：既有登录与运行时导入

状态：已实施（2026-10-18），待用户以真实 Claude Code／Codex 配置验收。

## 用户要求（2026-10-17）

1. Codex 订阅可手动指定 auth.json 文件，或输入 auth.json 内容。
2. Claude Code 订阅若能本地导入，也应支持。
3. “从 Agent 运行时导入”应支持 Codex、Claude Code 等；ACP 若可能也支持。需考虑 AI 提供商与 Agent 运行时两个 Rust lib 的协同。

## 约束

- AGENTS.md：不读取、复制登录令牌；委托原 Harness 认证来源时“不复制 refresh credential”；不借 CLI token 冒充正式接入，不规避供应商限制。
- 2026-10-16 已确认的边界：订阅聚合只走供应商自身登录，或由官方客户端自己执行请求；不读取其他工具保存的 OAuth 令牌（明确包括 `~/.codex/auth.json`）再由 Velune 以官方客户端身份发出请求。用户拥有账户或另行授权不改变这一点。
- 一个订阅对应一个提供商；模板只决定创建时的组合（见 PRD 提供商模板段落）。

## 已核实事实（代码 main `3f01f9b`）

- 运行时导入目前只支持 Pi：`application/provider_import.rs` 通过 `agent-runtime::provider_source` 调 Pi SDK adapter 读取非秘密快照；API key 等来源认证以 RuntimeProvider 委托解析，不复制。
- Velune 的 Claude CLI 提供商在 `VELUNE_HOME/claude-cli/<id>` 建独立 `CLAUDE_CONFIG_DIR`（`application/claude_cli.rs`）；ai-provider 的 Claude Code 执行器同样设置该变量。bindings README 规定平台不能传入真实既有登录目录。
- `agent-runtime/native/claude.rs`：在 macOS 上设置 `CLAUDE_CONFIG_DIR`（即使等于默认目录）会隐藏用户本人的登录；使用本人登录须不设置该变量。运行时实例因此已能沿用用户本人的 Claude Code 登录。
- 包依赖：ai-provider 与 agent-runtime 互不依赖，application 同时装配二者。

## 待验证

- Codex refresh token 轮换：同一 refresh token 在两处使用时，先刷新的一方可能使另一方失效（Codex 客户端对此有“refresh token 已被使用”的错误提示）。需以当前 Codex 源码复核。
- ACP 规范只有 `authMethods`／`authenticate` 等认证协商，没有导出提供商、端点或凭据的接口；需以当前规范复核。

## 方案（建议）

### Claude Code：沿用本机登录（引用，不复制）

Claude CLI 提供商的认证增加“沿用既有登录”：本人默认登录（不设 `CLAUDE_CONFIG_DIR`）或某个 Claude Code 运行时实例的配置目录。仍由官方 CLI 执行、保存并续期，Velune 不读取或复制凭据。此类提供商不提供“退出登录”（以免退出用户本人或运行时的登录），额度按同一目录读取；用量与本人 Claude Code 共享同一账户。

### Codex：auth.json

不实现读取或粘贴 auth.json 并由 Velune 发出请求：这正是上述边界排除的路径，且共享 refresh token 会使原登录随时失效。可选替代：

- A. 维持 Velune 内“登录…”（现状）：每个账户在 Velune 中单独登录一次。
- B. Codex 执行后端（类比 Claude CLI）：由官方 Codex 以指定 `CODEX_HOME`（含用户选择的 auth.json）执行请求，Codex 自己续期。需先研究 Codex app-server 能否作为原始模型执行器（工具桥接、流式、用量），工作量大。
- C. 运行时层聚合：多个 Codex 运行时实例各自登录，在运行时层切换（2026-10-16 记录的另一条合规路径），与网关路由是不同能力。

### 从运行时导入：两个 lib 的协同

- agent-runtime 拥有各 Harness 的配置格式知识，按家族给出只读“来源清单”：登录位置（Claude 配置目录／默认登录、Codex `CODEX_HOME`）与已配置的 API 提供商（地址、协议、密钥引用的类型），不读取秘密值。
- application 把清单映射为提供商模板与认证来源：Claude 登录 → 沿用该目录的 Claude CLI 提供商；API 提供商 → 以 RuntimeProvider 委托解析密钥（沿用 Pi 的既有做法）；Codex ChatGPT 登录 → 只有选定 B 后才可导入。
- ai-provider 只接收中立的执行配置（如 Claude 登录位置：默认或指定目录），不依赖 agent-runtime。
- ACP：协议不导出提供商或凭据，按协议无法通用导入；只能对具体 ACP agent 编写其配置文件适配，按需另议。

## 用户决定（2026-10-17）

1. Codex 选 A：维持 Velune 内逐账户登录，不实现 auth.json 导入／粘贴，不做 Codex 执行后端。
2. Claude Code 按“引用本机或运行时登录、不提供退出登录”实施。
3. 运行时导入首批：Claude Code 登录 ＋ Codex／Claude Code 已配置的 API 提供商；ACP 暂不做。

结论已归 PRD（提供商模板之后的段落）。

## 供应商资料复核（2026-10-17）

- Claude Code（code.claude.com/docs settings、env-vars、model-config）：用户设置在配置目录的 `settings.json`，`CLAUDE_CONFIG_DIR` 改变该目录；`env` 中 `ANTHROPIC_BASE_URL` 覆盖端点，`ANTHROPIC_API_KEY` 以 `X-Api-Key` 发送，`ANTHROPIC_AUTH_TOKEN` 以 `Authorization: Bearer` 发送；`apiKeyHelper` 由命令取得密钥；`--model` 接受 `sonnet`／`opus`／`haiku` 等别名。
- Codex（learn.chatgpt.com config-reference）：`config.toml` 位于 `CODEX_HOME`；`model_providers.<id>` 有 `name`、`base_url`、`env_key`、`wire_api`（仅 `"responses"`）、`experimental_bearer_token`、`http_headers`、`env_http_headers`、`query_params`、`requires_openai_auth`；顶层 `model`、`model_provider`。

## 实施要点

- Velune 的 Messages API key 一律以 `x-api-key` 发送；导入 `ANTHROPIC_AUTH_TOKEN` 时如实提示来源使用 Bearer。
- 密钥只有字面值可在应用时复制为 Velune 的 API key（沿用 Pi 导入对 API key 的既有做法）；`env_key`、`apiKeyHelper` 等不解析，导入后提示在 Velune 填写。不打开 Codex `auth.json`。
- 远端与 managed HOME 运行时不参与导入或沿用登录。

## 实施与验证（2026-10-18）

- 设计归 `docs/design/ai-service.md`（订阅组合段落之后），各 unit README 记录接口：agent-runtime `provider_inventory`，application `native_provider_import` 与 `UseClaudeCodeLogin`，bindings `BindingLinkedClaudeLogin`，ai-provider 的可选 `config_dir`，Mac 的“账户来源”。
- 静态检查：`cargo fmt --check`、`cargo check --locked --workspace`、`cargo clippy --locked --workspace --all-targets --all-features -- -D warnings`、application／bindings `--no-default-features` 检查与 Mac `build-macos.sh` 构建均通过。
- `scripts/manual-provider-sources.py`（临时 HOME、合成配置与假 `claude`）通过：Codex 导入（字面 bearer 复制、`env_key` 留空并提示、请求头提示、`wire_api=chat` 只警告）、Claude Code 导入（登录候选无地址、settings API 默认官方地址并复制 key）、预览不含 key、来源变化拒绝旧预览、沿用默认登录时状态命令不带 `CLAUDE_CONFIG_DIR`、同账户在另一登录下被拒、失效登录经检查转为未登录、删除时无 login／logout、不建 `claude-cli` 目录、来源文件不变、日志不含 key。
- 离屏渲染（临时 `VELUNE_HOME`，不激活应用）核对新 Claude 订阅的“账户来源”选择、两种沿用登录的编辑器与提供商列表。渲染发现账户尾号重复“账户”字样（独立登录同样存在）：core 的 `account_label` 改为只返回尾 8 位，由平台加措辞；已保存的旧标签在下一次状态读取时更新。
- 未验证：真实 Claude Code 的默认／运行时登录状态与执行、真实 Codex／Claude Code 配置文件的多样写法；需用户验收。
