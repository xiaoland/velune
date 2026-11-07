# Codex 0.167 区间扩展

## 结果：已完成，实测通过，区间已抬高

ChatGPT.app 自带的 Codex app-server 升级到 0.167.0-alpha.3.2，超出 `codex-0.166` variant 的兼容区间（≥0.166.0，<0.167.0），导致项目读取被拒。按 S34／S44 既定决策处理：**扩展既有 variant，不新增 adapter**。

## 决定依据

- S44（2026-10-21）确立流程：新版本先以真实 CLI 实测，无破坏性变化则抬高既有 variant 上限；确有破坏才另建 adapter。
- 0.159→0.166 当时新建 adapter 是因为有真实破坏（gateway provider 启动参数注入、load_for_resume 三方核对、权限预设映射、readState receipt）。
- 本次协议面静态核查：0.167.0-alpha.3.2 二进制中 adapter 依赖的全部 RPC 方法、事件、参数取值仍在。

## 变更

- `packages/agent-runtime/src/version.rs`：`codex-0.166` 的 `below` 从 `0.167.0` 提到 `0.168.0`。variant ID 与名称不变（先例：0.159 名字沿用到 0.163）。
- `packages/agent-runtime/README.md`：区间文字改为 ≥0.166.0 <0.168.0，补记 0.167.0-alpha.3.2 已验证。
- `docs/sources/runtime-collaboration.md` + `docs/sources.md`：新增 S83 决定记录。

## 验证（2026-11-06，全部通过）

环境：临时 `VELUNE_HOME`（/tmp/velune-codex-167）、合成凭据、本机回环 SSE 上游、真实 bundled CLI（`/Applications/ChatGPT.app/Contents/Resources/codex-cli/CodexCLI.app/Contents/MacOS/codex`，0.167.0-alpha.3.2）、临时 bindings（`target/verification/codex-167-range/bindings/`）。脚本：`target/verification/codex-167-range/verify.py`。未调用真实服务。

- `DESCRIPTOR_RANGE_PASS`：codex-0.166 descriptor 区间文字为 ≥0.166.0，<0.168.0。
- `VERSION_REJECTION_PASS`：伪造 99.0.0 二进制被拒绝，错误含实际版本与新区间文字。
- `NATIVE_CATALOG_PASS`：原生目录 18 个模型、9 个速度档位变体（`model/list` + serviceTiers）。
- `FIRST_TURN_AND_TITLE_PASS`：首轮 settle、答案到达、FIRST_MESSAGE 标题。
- `PERMISSION_MODE_TURN_PASS`：read-only 权限预设轮（approvalPolicy/sandbox 映射被 0.167 接受）。
- `NATIVE_RESUME_HISTORY_PASS`：原生恢复 + 分页历史重放完整。
- `IN_PLACE_EDIT_REVERT_PASS`：就地编辑（`thread/revert`）后 wire 与显示正确截断。
- `FORK_EDIT_PASS`：分叉编辑（`thread/fork`）正确继承上文、源会话不受影响。
- `NATIVE_LISTING_PASS`：原生 thread/list 可见新会话。

静态检查：`cargo build -p velune-bindings` 通过；`cargo clippy -p velune-agent-runtime --all-targets` 无警告；`cargo test -p velune-agent-runtime` 37 通过；`rustfmt --check` 本任务文件通过；`git diff --check` 通过。

## 备注

- 首条用户消息的分叉编辑按 application 既有设计新建空会话（`editing.rs` `first && Fork` 分支），非 0.167 行为变化；实测脚本已按此调整取最后一条用户消息验证 fork 继承。
- 工作树中 `app/mac/*`、`packages/application/src/local/{editing,submission}.rs` 等改动来自其它在途任务，与本任务无关。
- `packages/application/README.md:61` 仍提及已删除的 `codex-0.159`，属既有文档漂移，未在本次范围内修改。
- 0.167.0 正式版及其后续预发布按其正式版计入 <0.168.0；出现 0.168 或实测发现破坏时按 S44 流程另建 adapter。
