# bindings.open 失败风暴修复与重装加固

2026-11-04 起。11-04 02:37–02:55 某 Velune 实例 18 分钟内记录 3224 条 `bindings.open` 失败（kind=invalid, phase=application），11-02 有同款 311 条。

**更正后的诊断**（问题日志 detail 取证后推翻了双实例假设）：11-04 事件是重装/升级后新版检测到旧 composer 数据，按设计拒绝静默初始化（"输入存储需要显式迁移"），Transport 每个操作重试 open 导致风暴；用户按提示退出应用、运行迁移脚本后重启，撞上迁移脚本持锁的几秒窗口（"runtime home is already open" ×9），02:55:36 恢复。风暴窗口内无任何其他实例的成功轮询（已核实为空）。11-02 无问题记录可考，同时点有检查进程崩溃。另发现：多次"Velune 已崩溃"弹窗来自以 `--check-composer-ownership` 等参数启动的隐藏检查进程（bundle ID 与正式版相同），其 composerOwnershipCheck precondition 持续失败（11-03 共 11 次），与锁无关但值得追查是谁在跑检查。

## 根本原因（2026-11-05 定论）

obelisk 全量会话索引取证：风暴（11-04 10:37 本地）前最后一条 Agent 消息是 11-04 00:44 本地，中间近 10 小时无任何 Agent 活动 → 10:37 的重装是用户本人按标准流程执行的（3 秒间隔 = quit→install→open 脚本特征）。根本问题是结构性缺口：仓库政策允许 hard-cutoff + 要求显式手动迁移（S63），而唯一日常升级通道 reinstall-macos.sh 没有迁移环节；composer-state-ownership 按政策交付了迁移脚本，但升级不会执行它 → 应用按设计拒绝启动 → 无退避的重试把一条可操作提示淹没为 3224 条。已修：升级通道内嵌迁移（根本解主体）+ 退避/detail（可观测性）。残余候选（待用户决定）：a) 应用内引导式迁移，覆盖非脚本安装路径，需修订 S63 字面约束；b) reinstall 启动前 headless 预开探针，兜住未登记迁移的 cutoff。11-02 风暴同型但未完全证实。

## 目标与授权

用户已确认的实施范围：

1. `packages/bindings/src/diagnostics.rs`：`operation_failed` 日志补 `detail` 字段（现在只有 code/phase，事故诊断靠排除法）。
2. `app/mac/Transport.swift`：open 失败记入缓存，退避窗口内直接重抛缓存错误、不再调用 open、不重复记日志；窗口过后自动重试。
3. `scripts/reinstall-macos.sh` 与安装器：防治锁占用——安装器拒绝时列出 PID 与路径；启动新版前探测 home 锁占用者并中止启动。
4. 重装处理迁移：新增 `scripts/run-migrations.py` 与 `scripts/migrations/` 约定（`--home` 必填、`--apply` 写入、自检幂等、无事退出 0、自行 flock），两个既有迁移脚本迁入并支持无参数自检；reinstall 在安装后、启动前执行。
5. 顺带：`repository.rs` 锁冲突文案改为可行动的中文指引。

验证：`cargo check/clippy`（application、bindings）；`build-macos.sh --build-only`；迁移 runner 合成验证（空 home、锁占用、composer 旧数据、codex-166 合成配置）；`git diff --check`。文档更新 `docs/development.md`（迁移约定、重装步骤）与 `docs/design/composer-state.md` 的脚本路径。

## 关键事实

- flock 随进程退出（含崩溃、强杀）由 OS 释放，无残留锁；关闭路径 `Job::Close` 成功 → Owner 线程退出 → Repository drop → 锁释放；busy 拒绝时锁按设计保留。无需锁清理机制。
- 问题面板的轮询类问题按 activityKey 去重；非轮询操作每次失败各记一条，是面板刷屏来源。
- 配置 schema 当前为 11；codex-166 迁移仍适用于 schema 11。
- iOS/Android 只开 remote endpoint，无本地 open 重试风暴；Mac 远端 Transport 的 open 闭包模式不同，不在本任务范围。

## 当前进展

- 已完成：diagnostics.rs detail 日志；repository.rs 锁冲突中文文案；Transport.swift open 退避（15s 窗口，缓存错误重抛不重复记日志）；install-macos.py 拒绝时列 PID/路径；`scripts/migrations/`（codex-166.py、composer-state.py）+ `scripts/run-migrations.py`（锁探测 + 顺序执行）；reinstall-macos.sh 接入迁移步骤；docs 更新。
- 迁移机制已合成验证：空 home、锁占用（lsof 列出持有者并中止）、composer 旧数据迁移与幂等、codex-166 预览/执行/幂等/备份均通过。合成验证中新建 stub 首次执行被系统评估随机延迟（最长 70s），属本机安全扫描，与脚本无关。
- 收尾已通过 workspace/all-targets/all-features 严格 Clippy 与 Mac SVC 完整构建；复用上述迁移合成验收，不重复运行，也未迁移真实数据或重装日常应用。最终组合证据与提交由[工作区收尾任务](../workspace-delivery-and-network/packet.md)记录。
- 注意：本机 cargo 需 `CARGO_HOME=/Volumes/WorkSSD/.dev-cache/cargo`，否则 uniffi_bindgen 编译失败。
- 已同步 composer.rs 迁移提示文案到 scripts/migrations/composer-state.py --apply。
- 用户明确：事 A（检查进程崩溃弹窗）不需要处理，不追查。
