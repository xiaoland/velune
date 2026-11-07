# 网关执行快照与配置热编辑

2026-11-06 开始，2026-11-07 纳入首个公开 beta 候选整理。目标是允许编辑提供商、模型、路由与认证配置，已经执行的轮次及其后台工作使用捕获配置，后续执行使用保存后的配置。

## 当前判断与实现

- 首轮实现的全局 epoch 缓存会让同一 epoch 的不同网关、运行时、模型和会话复用首个 Runner。Runner 实际捕获别名、会话身份、额度来源和遥测父级，故本轮恢复每个准备执行独占 Runner 的生命周期。应用继续共享调度状态和实际可复用的后端。
- 配置保存沿用 `ExecutionConfiguration`、`sync_scheduling` 与延期 invalidation。保存失败恢复原配置。使用中的额度服务和后端在新配置旁存活，执行及其后台工作结束时直接释放 Runner。
- 在关闭旧运行时前捕获会话身份，新会话使用独立执行身份。网关、开始轮次、取消和完成使用同一捕获身份，避免原生 ID 尚未创建或投影被清除时失配。
- 删除提供商也可在执行期间保存。订阅重新登录沿用已存在的凭据退休清理，使用者结束后才删除旧凭据。原地写入来源的登录与 CLI 退出仍须等待对应使用者结束。
- 额度服务的各版本共享历史写入锁，当前绑定决定可写版本。旧版本迟到的观测继续供旧网关读取，不能覆盖新绑定历史或重建已删除提供商的文件。API key 提供商更换目标时推进绑定版本。

实现归 `packages/application/src/local/execution.rs`、`provider_management.rs`、`authentication_coordination.rs`、`conversations.rs`，以及 `provider_configuration.rs`、`subscription_quota.rs` 和 `quota_history.rs`。跨 unit 契约归 [LLM 网关](../../docs/product-tdd/ai-gateway.md)。Pi 后台生命周期与并行持久化改动保留，由各自 owner 验证。

## 验证与边界

- `cargo test -p velune-application gateway_execution_tests -- --nocapture` 通过。真实回环 HTTP 请求跨越配置保存，旧执行的后续请求仍发送旧地址及旧 key，新 Pi／Codex 执行独立绑定模型别名和身份。检查多次编辑、保存失败回滚、旧入口关闭、旧额度服务释放及失败后重试。
- `cargo test -p velune-application configuration_tests -- --nocapture` 通过。旧额度版本保持观测且不被新版本取消，迟到写入不能替换新绑定历史，删除后也不能恢复历史文件。
- `cargo clippy -p velune-application --lib -- -D warnings`、两份文档的 10 个相对路径／锚点与 `git diff --check` 通过。
- 候选集成由首个公开 beta 任务复用本轮证据。这里的合成入口使用私有临时目录、模拟 Harness 与回环上游，真实登录和公开分发产物由总体发布任务验收。

## 历史记录修正

旧记录宣称全局 Runner 缓存已完成，该方案已由本轮执行快照实现取代。当前 `editing.rs` 仍有其它任务的未提交改动，候选归属以实际 diff 和上述验证为准。
