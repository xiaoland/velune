# Codex 容量错误信息调查

2026-10-26。用户询问 `Selected model is at capacity. Please try a different model.` 是否由 app-server 暴露，并报告会话 `01a162ed-9659-7672-8410-d4cbdb427bf2` 在 Velune 中缺少提示。本轮授权为调查，不修改源码、不提交、不启动真实模型调用。只读检查指定会话的错误、回合状态与关联日志，不读取凭据文件。其他未提交工作保持不动。

调查已完成；用户随后明确要求“三个缺口都要补上”，授权必要的源码、验证和文档修改，并在本轮明确授权重新安装应用与提交。root 负责实现、提交和安装，不触碰其他未提交工作。未重现容量不足或捕获发生当时的通知，因此不将代码缺口直接认定为该次事件的完整因果解释。

## 已确认的事实

本机 CLI 为 0.159.3，桌面内置 CLI 为 0.164.0-alpha.11.1。两者的二进制均包含完整容量错误文案。在临时 CODEX_HOME 中分别执行 `app-server generate-json-schema`，两个版本均暴露 ErrorNotification 的 `error.message`、`error.codexErrorInfo`、`willRetry`、`threadId` 和 `turnId`；CodexErrorInfo 包含 `serverOverloaded`。`ThreadTurnsListResponse` 中 Turn 有 `status` 与 `error`。官方 [错误定义](https://github.com/openai/codex/blob/main/codex-rs/protocol/src/error.rs) 和 [通知定义](https://github.com/openai/codex/blob/main/codex-rs/app-server-protocol/src/protocol/v2/notification.rs) 由 Context7 查询交叉核对。

[Codex 实时适配器](../../packages/agent-runtime/src/native/codex.rs) 的 `error` 分支只生成固定通知“Codex 返回执行诊断；原始错误内容不写入日志。”；`turn/completed` 的 failed 分支只设置 Failed 并生成通用提示，未读取 `turn.error`。[历史读取](../../packages/agent-runtime/src/native/codex_history.rs) 使用 metadata-only `thread/read` 和 `thread/items/list`，未消费 `thread/turns/list` 的回合错误。因此实时错误详情和可用的历史回合错误都存在投影缺口。

指定会话匹配到 10 月 22 日原 rollout 与 10 月 26 日后续 rollout；两者均没有 `event_msg/error`。只读 SQLite 查询 `thread_history_1.sqlite` 中该 thread 的 `thread_turns`：24 个 completed、3 个 interrupted，没有 failed，error_json 均为空。关联桌面日志未找到带该会话 ID 的容量错误。上述缺失不证明当时没有发生容量失败，也不能保证补接历史接口便能恢复该条提示；未直接请求该会话的 app-server 历史响应。

## 修复方向与边界

实施范围为在现有通知投影中消费实时 `error` 和最终 `turn.error`，按活动 thread/turn 过滤，以稳定消息身份更新同一通知；重试期间标明正在自动重试，成功或中断后移除临时通知，最终失败保留具体原因。最终事件缺少详情时复用本轮最近的错误；原生 failed 回合确实没有详情时明确显示上游未提供。错误的 message 与 additionalDetails 展示在会话中，不新增原始响应日志。

历史同时读取 `thread/items/list` 与 `thread/turns/list`，保留现有 item 时间与配置／用量投影，在对应回合末尾插入错误；没有 item 的失败回合仍保留原生顺序。`thread/resume` 重建消息时也投影回合错误，避免准备执行丢失已有提示。三个公开平台继续消费既有 Notice 契约，不新增绑定、配置或依赖。

实现顺序：共用错误投影与实时状态处理；历史分页与 resume；合成回归验证与 agent-runtime 静态检查；更新 unit README 和本包证据。完成标准覆盖重试后成功、重试后失败、重复事件、只有最终错误、最终缺少详情、相邻及空回合的历史顺序、分页和恢复。

## 实施结果与验证

上述修复已完成。修改仅涉及 agent-runtime 的 Codex 适配器、历史投影、活动回合错误缓存、两项回归测试和 unit README；没有修改平台 UI、公开 API、绑定、依赖或配置。README 已同步实际行为。用户指定的旧会话未重放，也未尝试补写其原生历史。

`cargo check --locked -p velune-agent-runtime`、`cargo clippy --locked -p velune-agent-runtime --all-targets -- -D warnings`、`cargo test --locked -p velune-agent-runtime` 全部通过。测试共 6 项，其中新增 2 项覆盖错误详情／重试投影以及首部、中间、尾部空失败回合的顺序，保留已有消息时间与分支身份。`cargo fmt -p velune-agent-runtime`、文档相对链接与标题锚点检查、`git diff --check` 通过。

另在临时目录构建了依赖当前 agent-runtime 的 Rust 验证程序，通过合成 stdio app-server 调用真实公开入口 `NativeSession::connect/open/send/steer/poll` 与 `read_codex_history`。七种实时场景通过：重试后失败、重试后成功、中断、不重试的终止错误、仅最终事件带详情、最终事件缺详情但通知已有详情，以及全程未提供详情。同轮重复错误和完成事件只保留一条通知，其他 thread／旧 turn 的错误被过滤。历史按三页返回，空失败回合、消息顺序和 resume 重建也通过。

真实 Codex 0.159.3 使用隔离 HOME／CODEX_HOME、合成 key 和本地 loopback HTTP 服务完成失败验证。服务返回合成 503；实际 error 通知、turn/completed 与 thread/turns/list 都包含具体错误原因。该情景的原生分类是 httpConnectionFailed(503)，不冒充真实账户的 serverOverloaded。终止该 app-server 后，当前 Velune 历史读取另起进程，仍恢复出唯一一条合成错误通知，证明持久化历史路径贯通。没有真实账户读取或付费请求。

实现、提交与安装均已完成。源码提交为 `0b06c29f2acef3a76ce8cd62270d35b4bac3d238`，只包含本任务的五个文件。主工作区其他任务的未提交改动未纳入提交或安装；release 产物在该提交的干净工作树中，通过现有 `scripts/build-macos.sh --build-only` 构建。首次构建在 UniFFI 模板生成阶段遇到 Cargo registry 符号链接导致的路径计算错误，改用实际缓存目录 `CARGO_HOME=/Volumes/WorkSSD/.dev-cache/cargo` 重跑后，Rust release、绑定生成、Swift release 与签名验证全部通过，没有为该环境问题修改源码。

已退出旧应用，并用 `scripts/install-macos.py` 安装到 `/Applications/Velune.app`。安装后的应用、核心 dylib 和 build-manifest 的 SHA-256 均与签名产物一致，`codesign --verify --deep --strict` 通过。manifest 记录上述源码提交、`dirty=false`、`0.1 beta.1` 和 arm64。应用已重新打开并确认新进程运行。后续安装记录提交只更新本文，不改变已安装代码。

未进行 UI 行为／其他平台实机验证。上游未持久化的旧错误仍无法恢复；本次不增加另一套错误历史存储。
