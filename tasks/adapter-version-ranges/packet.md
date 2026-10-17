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

- agent-runtime `RuntimeVariant` 以 `versions: VersionRange { minimum, below }`（语义化版本比较，含预发布）取代 `version_regex`；`check_version_at`、`matching_variant` 与公开描述随之改；错误信息给出兼容区间。ACP 保持协议主版本 `1`。
- 新 ID：`pi-1.0`、`codex-0.159`、`dsh-acp-0.2`、`claude-code-2.1`；名称不含补丁号。代码与脚本中的旧 ID 一次性改名，不保留旧 ID 映射。
- 公开描述的 `version_regex` 换为区间文字（bindings／Mac 同步）。

## 验证计划

fmt／check／clippy；Mac 构建（待 8616bd0 的 Mac 映射修复）；以本机 Claude Code 2.1.292、Codex 0.159.3、Pi 1.0.2 运行受影响的人工脚本。安装前先就 schema 9 的配置初始化告知用户。
