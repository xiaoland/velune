# 消除 Velune Dev 空闲轮询高 CPU 占用

## 当前状态

2026-11-26 从本地 `develop` 分出独立任务分支 `perf/idle-poll-cpu`（工作树 `/Volumes/WorkSSD/Development/.worktrees/velune/idle-poll-cpu`）。

## 观测证据

用户在空载情况下发现 `VeluneDev` CPU 消耗偏高，现场采样与日志核查如下：

- 运行中的 `VeluneDev`（`pid 98568`，运行 `2h14m57s`）CPU 占用 `45.1%`（累计 `112:48` CPU 时间），日志已产生约 50 MB 慢命令记录（每 600 ms 轮询一次 `ReadSnapshotUpdate`，单次耗时常超过 100 ms 阈值：`114–368 ms`）。
- 对 `pid 98568` 执行 `sample 98568 3`（961 个采样点，其中 `velune-application` owner 线程占 959 个活跃样本）：
  1. **每次轮询构建并丢弃双份快照（`541 / 959` 样本）**：`Owner::start_history` 为 `HistoryPurpose::Retry(ReadSnapshotUpdate)` 调用 `prepare_conversation_read` → `read_present_conversation` → `snapshot_result()`，拿到 `ConversationRead::Ready(snapshot)` 后直接丢弃 `snapshot`，接着再调用 `self.execute(ReadSnapshotUpdate)` 重建同一份快照（`414 / 959` 样本）。
  2. **`name_turn_models` 全量反复计算 SHA-256 绑定别名（`653 / 959` 样本，含在两次快照构建中）**：`CoreRuntime::name_turn_models` 对历史中的每条 turn 遍历网关内全部可选模型并调用 `gateway.pi_binding_id(...)`（做一次 `serde_json::to_vec` 与 `Sha256::digest`），即使模型名根本不是 `vln_…` / `--vln_…` 路由别名也照算不误。
  3. **`editing_choices` 对消息表做二次方扫描（`77 / 959` 样本）**：`editing_choices` 为每条用户消息调用 `deletion_source` 两次（`delete_only` 与 `delete_following`），而 `deletion_source` 每次对整张 `snapshot.messages` 线性扫描两遍，总复杂度为 $O(U \times M)$。
  - 另有每 5 秒同步执行一次 `ps` 的 `refresh_external_turns`（`44 / 959` 样本，约 `4.5%`），占比较小，暂不改动。

## 修复方案

1. **避免 `HistoryPurpose::Retry` 路径丢弃 `SnapshotResult`**（`conversation_history.rs`、`service.rs`）
   - `ConversationRead` 新增 `Present` 变体；`prepare_conversation_present` 在会话已在内存中时仅执行 `sync_projection()`，跳过 `snapshot_result()`，由后续 `execute(command)` 单次生成快照。
2. **缓存 `CoreRuntime::name_turn_models` 的 Pi 绑定别名映射**（`local/mod.rs`、`model_selection.rs`）
   - 若 `settings.model` 不是 `vln_` 前缀且不含 `--vln_`，直接跳过网关查找；命中 `vln_` 别名格式时，按 `(gateway_id, GatewayConfig)` 在 `CoreRuntime.pi_binding_aliases` 中缓存 `alias -> model_record_key` 哈希表，仅在网关配置变化时重建。
3. **将 `CoreRuntime::editing_choices` 降为线性 $O(M)$**（`editing.rs`）
   - 在遍历用户消息前一次性计算会话级删除条件与 Pi `pi_entry` 出现次数表，单遍为每条用户消息同时算出 `delete_only` 与 `delete_following`，与 `deletion_source` 保持完全一致语义。

## 验证

- `cargo test --locked -p velune-application --lib local::editing`：15 passed, 0 failed（含新增 `editing_choices_matches_deletion_source_across_pi_codex_and_linked_messages`）。
- `cargo test --locked -p velune-application --lib local::model_selection`：2 passed, 0 failed（含新增 `name_turn_models_caches_pi_aliases_skips_non_aliases_and_invalidates_on_gateway_change`）。
- `cargo test --locked -p velune-application --lib local::delivery`：5 passed, 0 failed。
- `cargo test --locked -p velune-application --lib local::conversation_address`：2 passed, 0 failed。
- `cargo test --locked -p velune-application --lib snapshot_poll`：1 passed, 0 failed。
- `cargo test --locked -p velune-application --lib submission_receipt`：4 passed, 0 failed。
- 全量 `cargo test --locked -p velune-application --lib -- --test-threads=4` 中，`claude_automatic_naming_uses_configured_model_and_preserves_native_title` 失败、`pi_delete_waits_for_source_confirmation_without_prompt_or_owner_wait` 挂起、`codex_first_admission_error_still_cleans_failed_owner_when_poll_returns_ok_and_pool_is_full` 失败/挂起；已在未改动的基线工作树（`codex-active-index-persistence`）上复测确认前两者在基线 `develop` 同样失败／挂起，非本次改动引入。
- 尚未重编并重启 `VeluneDev` 实测空闲 CPU 降幅，降耗效果由 `sample` 命中热点消除及上述单测推知。
