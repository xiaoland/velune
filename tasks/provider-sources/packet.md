# 提供商来源：既有登录与运行时导入

状态：方案待用户决定（2026-10-17），未实施。

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

## 待用户决定

1. Codex 方向：A／B／C（可多选）。
2. Claude Code 沿用本机登录（含运行时实例目录）是否按上述“引用、不提供退出”的语义实施。
3. 运行时导入首批范围：Claude Code 登录 ＋ Codex／Claude Code 已配置的 API 提供商，ACP 暂不做。
