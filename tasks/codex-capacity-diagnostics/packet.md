# Agent runtime 原生错误映射

2026-10-26 起。用户最初询问 `Selected model is at capacity. Please try a different model.` 是否由 app-server 暴露，并报告会话 `01a162ed-9659-7672-8410-d4cbdb427bf2` 在 Velune 中缺少提示。初次调查只读检查指定会话的错误、回合状态与关联日志，没有读取凭据文件。

用户先授权 Codex 三个入口的修复、提交与安装，随后指出范围应覆盖已支持运行时的原生错误映射，并明确要求开始、将耦合控制在 adapter 内。当前 root 负责跨运行时实现与交付，不触碰其他任务的未提交改动。原生字段解析、错误分类及生命周期留在 adapter 内，共享层消费既有消息、状态及中立能力。未重现真实容量不足或捕获最初事件，因此不将代码缺口直接认定为该次事件的完整因果解释。

## 首次调查的事实

本机 CLI 为 0.159.3，桌面内置 CLI 为 0.164.0-alpha.11.1。两者的二进制均包含完整容量错误文案。在临时 CODEX_HOME 中分别执行 `app-server generate-json-schema`，两个版本均暴露 ErrorNotification 的 `error.message`、`error.codexErrorInfo`、`willRetry`、`threadId` 和 `turnId`；CodexErrorInfo 包含 `serverOverloaded`。`ThreadTurnsListResponse` 中 Turn 有 `status` 与 `error`。官方 [错误定义](https://github.com/openai/codex/blob/main/codex-rs/protocol/src/error.rs) 和 [通知定义](https://github.com/openai/codex/blob/main/codex-rs/app-server-protocol/src/protocol/v2/notification.rs) 由 Context7 查询交叉核对。

修复前，[Codex 实时适配器](../../packages/agent-runtime/src/native/codex.rs) 的 `error` 分支只生成固定通知“Codex 返回执行诊断；原始错误内容不写入日志。”；`turn/completed` 的 failed 分支只设置 Failed 并生成通用提示，未读取 `turn.error`。[历史读取](../../packages/agent-runtime/src/native/codex_history.rs) 使用 metadata-only `thread/read` 和 `thread/items/list`，未消费 `thread/turns/list` 的回合错误。因此实时错误详情和可用的历史回合错误都存在投影缺口。

指定会话匹配到 10 月 22 日原 rollout 与 10 月 26 日后续 rollout；两者均没有 `event_msg/error`。只读 SQLite 查询 `thread_history_1.sqlite` 中该 thread 的 `thread_turns`：24 个 completed、3 个 interrupted，没有 failed，error_json 均为空。关联桌面日志未找到带该会话 ID 的容量错误。上述缺失不证明当时没有发生容量失败，也不能保证补接历史接口便能恢复该条提示；未直接请求该会话的 app-server 历史响应。

## 已交付的 Codex 修复

实施范围为在现有通知投影中消费实时 `error` 和最终 `turn.error`，按活动 thread/turn 过滤，以稳定消息身份更新同一通知；重试期间标明正在自动重试，成功或中断后移除临时通知，最终失败保留具体原因。最终事件缺少详情时复用本轮最近的错误；原生 failed 回合确实没有详情时明确显示上游未提供。错误的 message 与 additionalDetails 展示在会话中，不新增原始响应日志。

历史同时读取 `thread/items/list` 与 `thread/turns/list`，保留现有 item 时间与配置／用量投影，在对应回合末尾插入错误；没有 item 的失败回合仍保留原生顺序。`thread/resume` 重建消息时也投影回合错误，避免准备执行丢失已有提示。三个公开平台继续消费既有 Notice 契约，不新增绑定、配置或依赖。

实现顺序：共用错误投影与实时状态处理；历史分页与 resume；合成回归验证与 agent-runtime 静态检查；更新 unit README 和本包证据。完成标准覆盖重试后成功、重试后失败、重复事件、只有最终错误、最终缺少详情、相邻及空回合的历史顺序、分页和恢复。

## Codex 修复与首次安装结果

上述修复已完成。修改仅涉及 agent-runtime 的 Codex 适配器、历史投影、活动回合错误缓存、两项回归测试和 unit README；没有修改平台 UI、公开 API、绑定、依赖或配置。README 已同步实际行为。用户指定的旧会话未重放，也未尝试补写其原生历史。

`cargo check --locked -p velune-agent-runtime`、`cargo clippy --locked -p velune-agent-runtime --all-targets -- -D warnings`、`cargo test --locked -p velune-agent-runtime` 全部通过。测试共 6 项，其中新增 2 项覆盖错误详情／重试投影以及首部、中间、尾部空失败回合的顺序，保留已有消息时间与分支身份。`cargo fmt -p velune-agent-runtime`、文档相对链接与标题锚点检查、`git diff --check` 通过。

另在临时目录构建了依赖当前 agent-runtime 的 Rust 验证程序，通过合成 stdio app-server 调用真实公开入口 `NativeSession::connect/open/send/steer/poll` 与 `read_codex_history`。七种实时场景通过：重试后失败、重试后成功、中断、不重试的终止错误、仅最终事件带详情、最终事件缺详情但通知已有详情，以及全程未提供详情。同轮重复错误和完成事件只保留一条通知，其他 thread／旧 turn 的错误被过滤。历史按三页返回，空失败回合、消息顺序和 resume 重建也通过。

真实 Codex 0.159.3 使用隔离 HOME／CODEX_HOME、合成 key 和本地 loopback HTTP 服务完成失败验证。服务返回合成 503；实际 error 通知、turn/completed 与 thread/turns/list 都包含具体错误原因。该情景的原生分类是 httpConnectionFailed(503)，不冒充真实账户的 serverOverloaded。终止该 app-server 后，当前 Velune 历史读取另起进程，仍恢复出唯一一条合成错误通知，证明持久化历史路径贯通。没有真实账户读取或付费请求。

实现、提交与安装均已完成。源码提交为 `0b06c29f2acef3a76ce8cd62270d35b4bac3d238`，只包含本任务的五个文件。主工作区其他任务的未提交改动未纳入提交或安装；release 产物在该提交的干净工作树中，通过现有 `scripts/build-macos.sh --build-only` 构建。首次构建在 UniFFI 模板生成阶段遇到 Cargo registry 符号链接导致的路径计算错误，改用实际缓存目录 `CARGO_HOME=/Volumes/WorkSSD/.dev-cache/cargo` 重跑后，Rust release、绑定生成、Swift release 与签名验证全部通过，没有为该环境问题修改源码。

已退出旧应用，并用 `scripts/install-macos.py` 安装到 `/Applications/Velune.app`。安装后的应用、核心 dylib 和 build-manifest 的 SHA-256 均与签名产物一致，`codesign --verify --deep --strict` 通过。manifest 记录上述源码提交、`dirty=false`、`0.1 beta.1` 和 arm64。应用已重新打开并确认新进程运行。后续安装记录提交只更新本文，不改变已安装代码。

未进行 UI 行为／其他平台实机验证。上游未持久化的旧错误仍无法恢复；本次不增加另一套错误历史存储。

## 范围复核

用户随后指出，本质问题是 agent-runtime 对原生错误的映射不足。只读复核确认，已交付补丁修复的是 Codex 的完整入口，并未完成各受支持运行时的一致性修复。Claude Code 的 result 只保留失败布尔值，等待 idle 后显示固定提示；DeepSeek 的异步 prompt 错误替换为固定失败文本；Pi 的顶层 error／extension_error 等事件也替换为通用提示。与此同时，Pi 的助手 errorMessage、DSH 的历史 error 记录和通用 ACP 的 prompt 错误码／message 已有保留路径，不能把所有错误处理都认定为缺失。

建议将后续任务边界修正为：核对全部已支持运行时的原生错误入口，在实时事件、最终结果、历史与恢复路径中保留上游实际提供的原因、来源和关联身份，并让重试、轮次失败与连接失效的状态处理一致。原生未提供或未保存的信息不推断、不补造；不扩大为自动重试／模型切换、遥测工程或自建错误历史。当前会话展示只有 Notice.text，应用操作失败另有 Failure／诊断契约；先以具体消费需求决定哪些语义需要跨边界类型化，不能仅因多适配器就强制引入统一错误框架。本轮仅调查和修正任务记录，尚未实施上述扩展范围。

独立 advisor 的只读审阅支持上述边界。[问题与诊断要求](../../docs/prd/diagnostics.md) 已明确保留实际可获得的原因与底层错误链，因此这属于既有要求在适配层的落实。实施前还需核实固定支持版本的载荷和终止语义、原生持久化边界，以及真实适配入口到各平台 Notice 的合成证据；Pi client 会将通道错误包装为 velune_error／velune_closed，不能与扩展错误和原生执行错误仅按文案统一处理。

## 跨运行时实现

用户已明确授权“开始”，并要求耦合控制在 adapter 内。root 负责当前实现；advisor 只读核对 Pi 生命周期。协议字段解析、错误类别和身份关联留在 Codex、Claude、DSH、Pi、ACP 各自适配边界，继续输出既有 Message／Notice、RunState 和操作错误，不新增公共错误分类、平台绑定或依赖。保留主工作区 native-turn-resume 等其他任务的未提交改动；只在现有代码上做本任务的窄改动。

执行顺序为：核对固定安装的载荷与错误生命周期；修复 Claude 最终结果／助手错误、DSH 请求错误和 Pi 事件／历史重建；复核 Codex 与通用 ACP 已有路径；通过各适配入口的合成失败、重试成功、取消、重复事件与历史重读验证；更新 unit 文档与本任务记录。只用临时 HOME、合成数据与 loopback 服务。已有提交／安装授权覆盖本任务后续交付，但提交必须仅包含本任务改动；发生共享文件并发变更时不得把其他任务的差异纳入提交或安装。

2026-10-27，跨运行时实现、提交与安装已完成。Claude 的助手 API 错误和 result 合并为同一回合通知，保留原生类别、HTTP 状态、errors 和失败 result 文本，仍等待 idle；已确认原生中断原因不当作失败。Claude 原生历史中的 isApiErrorMessage 同样显示为系统通知。DSH 与 ACP 保留 JSON-RPC code、message、data，DSH 也按协议显示取消、上限及拒绝；DSH 历史保留 huihua 提供的原生 details。Codex 补回 codexErrorInfo。

Pi 的生命周期改为 adapter 私有状态。模型失败沿用原生助手消息，重试开始移除原生省略的失败消息、结束清除临时通知；最终是否失败不从 auto_retry_end.success=false 推断。扩展错误只留诊断，按前一原生消息锚定，身份确认及历史刷新不改变其位置。JSON、读取及 EOF 故障共享首因锁存和终止路径，同步请求前已收到的事件仍可投影；连接故障清除重试提示。application 删除原生 agent_start／agent_settled 匹配逻辑，仅调用 begin_turn、读取 turn_ended 和 can_sync_history，并尊重 adapter 的发送能力。

验证依据：本机隔离 Pi SDK 为 1.0.2，DSH 安装为 0.2.0-rc.2；PATH 上 Pi 1.4.0 不作为协议依据。真实 Claude 2.1.292＋本地 401 响应确认了 assistant.error、is_api_error_message，以及 subtype=success／is_error=true／api_error_status／result 的组合，其持久化历史解析通过。通过实际 NativeSession 与 ACP Client 入口验证了 Claude API／循环上限／仅最终错误／中断／成功，DSH 错误详情、重复响应、取消／输出及请求上限／拒绝，ACP 结构化 data。Pi Client 的无效 JSON、退出码及 stderr、handled 扩展错误、同步失败与后续不可用均通过。

真实 Pi 1.0.2＋本地 HTTP 两轮验证通过：第一轮保留模型错误和非致命扩展诊断，第二轮正常成功，get_messages 与身份确认后的诊断位置保持不变。独立复核发现“结束事件后断连仍请求失效 Client”的贯通缺口，已用中立能力修复；实际 application 入口注入 settled 后的无效 JSON，连续三次 snapshot 都能返回 Failed、首个原因与 stderr，发送及取消保持不可用。另验证了 DSH 历史 helper 的原生结构化详情。上述运行只使用合成数据和 loopback 服务，没有读取真实凭据或会话，没有真实付费请求。

agent-runtime 当前 13 项测试通过；agent-runtime 与 application 的 all-targets 严格 clippy、格式检查、Python／JavaScript 语法及文档相对链接和标题锚点检查通过。平台 Notice 映射已只读核对，未修改平台展示契约；尚未进行本轮 GUI／其他平台实机验证。临时验收探针不接入 CI，未建立新的错误存储或自动恢复策略。

交付源码为 `882d0614c4580d766bbc7e001886ecaf25b1093d`，只提交上述错误映射、必要的中立能力集成、测试和文档，共 13 个文件。构建工作树最初因聊天仍指向已归档的旧工作树，工具将 HEAD 解析为旧 Codex 提交；已在安装前停止该次构建并显式检出上述完整提交，重新完成 release、绑定生成与 Swift 编译。干净提交中的 13 项测试、严格 clippy、全部 adapter 验收、真实 Pi 两轮及 application 故障快照验收均重新通过，未纳入其他任务的未提交遥测、远控或界面改动。

`/Applications/Velune.app` 已替换并重新打开。codesign 深度严格验证通过；应用、核心 dylib、build-manifest 与外置 huihua helper 的 SHA-256 均与签名产物相同。Claude 历史 helper 通过 include_str 嵌入核心库，由已核对的 dylib 交付。manifest 为上述源码提交、dirty=false、0.1 beta.1、arm64。后续交付记录提交不改变已安装代码；临时构建工作树已申请归档清理。
