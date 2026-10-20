# Adapter 兼容版本区间

2026-10-17 开始。来源：[运行时工作生命周期任务](../runtime-work-lifetime/packet.md) 复核中，本机 Claude Code 自动更新到 2.1.292 后被版本检查拒绝。沿用 AGENTS.md 的验证、安装与提交规则；不 push。

## 已确认（用户，[S34](../../docs/sources.md#s34)）

- adapter 声明已验证的兼容区间（如 Claude Code ≥2.1.286、<2.2；Codex、Pi、DSH 同理），区间内照常使用，出现破坏性变化才另建 adapter。
- 类型 ID 不含补丁号；按 hard-cutoff 重新添加运行时实例。

## 已核实的事实（2026-10-17）

- 原精确锁定来自架构规则“不靠放宽 regex 声称兼容”与各 variant 的 `version_regex`；没有发现 2.1.292 的协议变化。临时放宽后 9 个实际 CLI 脚本以 2.1.292 全部通过（见生命周期任务）。
- variant ID 字符串在 application／agent-runtime 约 25 处作为分派键，40 个人工脚本写着旧 ID；Mac 不直接使用类型 ID，只消费描述。
- 实例 ID 是随机记录键；会话 ID 以实例 ID 为前缀。
- 配置 schema：8616bd0（订阅调度，另一任务）已把 `CURRENT_SCHEMA_VERSION` 从 8 改为 9；用户当前安装为 8。安装 main 的下一个构建时整个配置按 hard-cutoff 初始化（提供商、API key、路由、模板、运行时实例），重新添加的运行时实例得到新 ID，会话的已读／归档等标记随之失效。类型 ID 改名随同一个未安装的 schema 9 发布，不额外增加重置；但 schema 9 的重置需要先告知用户再安装。
- 8616bd0 只更新了 Rust bindings，Mac `BindingMapping.swift` 缺少新字段（`strategy`、`weights`、`sticky`、`quotaLimits`），Mac 构建当前失败；归该任务修复。

## 方案

## 已实施（2026-10-17）

- agent-runtime `RuntimeVariant.versions: Versions`（`Cli { minimum, below }` 用 semver 比较，`below` 的预发布在区间外；`AcpProtocol(1)`）取代 `version_regex`；`check_version_at`、`matching_variant` 随之改，拒绝信息给出区间。
- 新 ID：`pi-1.0`、`codex-0.159`、`dsh-acp-0.2`、`claude-code-2.1`；代码、脚本、文档中的旧 ID 一次性改名，无映射。
- 公开描述 `version_regex` 改为区间文字 `compatible_versions`（如“≥0.159.3，<0.160.0”）；Mac 不读取该字段，无需改动。
- Pi 的区间 `PI_VERSIONS` 同时用于 SDK helper（`pi_sdk.mjs` 按同一区间核对安装包）与提供商导入快照；OAuth 来源仍保存精确 SDK 版本，安装变化须重新绑定（既有规则）。
- 可执行文件发现只给下限，未改：发现到区间外的新版本时，执行准备给出区间提示。

## 验证（2026-10-17）

fmt／check／clippy 通过。Mac 构建仍因 8616bd0 的映射缺口失败，脚本改用临时组装的 bundle（release 动态库＋当前 helper 资源）：multi-runtime（Codex 0.159.3、DSH 0.2.0-rc.2，99.0.0 被拒且信息含区间）、native-subagents（Codex 0.159.3、Claude Code 2.1.292）、pi-native-loop（Pi 1.0.2，含导入）、provider-configuration、session-browser 全部 PASSED。其中 4 个脚本的 schema 常量随 schema 9 更新（未来 schema 用 10）。

## 未完成

- 安装：等 Mac 映射修复，并先告知用户 schema 9 会初始化整个配置。`build-macos.sh` 的 manifest 仍写 `config_schema_version: 8`，归 8616bd0 的任务。
- Claude Code 2.1.286 本身未在本机复核（已自动更新）。

## Codex 0.163 与预发布（2026-10-21，[S44](../../docs/sources.md#s44)）

用户决定先实测，确有破坏性变化才另建 adapter；预发布版本无所谓。ChatGPT 桌面应用自带的 Codex（macOS `/Applications/ChatGPT.app/Contents/Resources/codex-cli/bin/codex`，Windows `%LOCALAPPDATA%\OpenAI\Codex\bin`）为 0.163.0-alpha.10.2。

- 实测：以该 CLI、临时 HOME 与回环合成上游运行 live-native、codex-segments、permission-modes、native-subagents、subagent-activity、codex-history-identity、turn-settings、reasoning-effort、steer、edit-message、attachments、multi-runtime（含 DSH 0.2.0-rc.2）、cross-harness（含 Pi 1.0.2）全部通过，没有协议变化，所以不增设 adapter。
- 实施：`Versions::contains` 两端只比较 major.minor.patch，预发布按正式版计入；`codex-0.159` 上限提高到 0.164.0，ID 不变（不触发配置重置）；核心发现补上 macOS ChatGPT.app 的 codex-cli。
- 顺带修正：main 的 ARK 改动给 `BindingProviderDraft` 加了必填 `quota_authentication_edit`，28 个人工脚本补 `None`（保持）；55de83f 起 Codex 自报的窗口不再被同 ID 的 managed 模型覆盖，turn-settings 的历史窗口断言随之更新；multi-runtime 的区间文字更新。
- 未单独验证 0.160–0.162；区间按两端已验证版本连续覆盖。
