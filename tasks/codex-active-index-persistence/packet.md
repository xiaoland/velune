# 跨启动持久化 Codex 活动会话索引校验

## 当前状态

2026-11-26 由用户确认从本地 `develop` 分出独立任务分支 `perf/codex-active-index-persistence`（工作树 `/Volumes/WorkSSD/Development/.worktrees/velune/codex-active-index-persistence`）处理，与 Mac 原生外壳工作区任务分离。

## 问题与根因

真实数据副本（4161 条 Codex 线程）日志显示：

- 每次启动 `codex_active_index_warmup` 耗时约 7.4–14.0 秒（多数为 7.4–10.3 秒），而预热后包含 Codex、Pi、Claude Code 共 3 个运行时的 `conversation_list_index` 只需 273–391 毫秒。
- 原因：`ActiveListIndex`（`packages/agent-runtime/src/native/codex_active_index.rs`）只在进程内存中保存 `VerifiedList`；每次重启后状态为空，应用启动即调度 `WarmCodexIndex` 阻塞 `ListRefresh`，拉起 Codex `app-server` 并做一次全量修复扫描。

## 方案

1. **持久化索引核验证明（`velune-agent-runtime`）**
   - `ActiveListIndex::with_storage(path)` 从磁盘惰性读取、并在成功修复后原子写入 `schemaVersion: 1` 的 JSON 证明，包含三个 SHA-256 摘要：规范编码的 `HistoryConfig` 摘要、完整 `Rollouts` 快照摘要（binary、`config.toml`、`sessions/` 树所有文件与 Unix 文件身份），以及 `thread/list` 响应结果摘要。
   - 若磁盘文件缺失则按未预热处理并在修复后写入；若存在未知 `schemaVersion`、未知字段、损坏 JSON 或读写失败，则只记录诊断并禁用写回（`writable = false`），原文件保持不被覆盖，内存缓存仍正常工作。
   - 新增 `codex_projects::warm_active_index(config, index)`：若加载的持久化证明与当前 `HistoryConfig` 和 `Rollouts` 摘要匹配，则直接跳过拉起 Codex `app-server` 的预热扫描；否则连接 Codex 执行修复并落盘证明。`ListRefresh` 仍照常走 `useStateDbOnly` 查询并在结果摘要或文件变化时自动回退修复，确保外部直接修改 `state_5.sqlite` 时依然安全。
2. **应用层持久化落盘与快速预热（`velune-application`）**
   - `HistoryIndexRead::with_local_codex_index` 为每个启用本机 Codex 索引的运行时绑定 `<VELUNE_HOME>/codex-active-index/<sha256(runtime_id)>.json`。
   - `CodexIndexWarmup::run` 改调 `warm_active_index`，不再重复执行 `project/list` 与摘要构造；首次真正打开列表时，若索引仍与证明一致，只需一次极快的 `useStateDbOnly` 分页读取。

## 验证

- `cargo test --locked -p velune-agent-runtime --lib active_index`：3 passed, 0 failed（覆盖压缩／替换／符号链接、持久化读写与变更失效、未知 schema 与损坏 JSON 保留不覆盖）。
- `cargo test --locked -p velune-agent-runtime --lib native::codex`：47 passed, 0 failed。
- `cargo test --locked -p velune-agent-runtime --lib -- --test-threads=4`：300 passed, 0 failed, 1 ignored（此前全并发下 `acp::tests::acp_explicit_run_and_control_deadlines_preserve_written_and_require_cleanup` 出现一次负载超时，单独复跑与 `--test-threads=4` 全量跑均通过）。
- `cargo test --locked -p velune-application --lib codex_startup_prewarms_the_active_list_without_blocking_the_owner`：1 passed, 0 failed。
- `cargo test --locked -p velune-application --lib codex_active_index`：1 passed, 0 failed（`codex_active_index_persists_across_restarts_and_repairs_on_external_change`，覆盖首次修复落盘、重启跳过预热直接走 `indexed`、外部清空 `list-db` 自动回退修复、新增 rollout 触发预热修复、未知 schema 保留原文件）。
- `cargo test --locked -p velune-application --lib list_refresh`：1 passed, 0 failed。
- 未运行真实数据副本上的 Mac 端重启计时实测；真实数据加速效果由现有结构（重启省去 `codex_active_index_warmup` 的全量扫描）与上述集成测试推知。
