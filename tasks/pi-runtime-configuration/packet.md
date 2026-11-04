# Pi 1.4 升级与运行时配置独立性修复

用户先反馈 Pi 历史读取辅助进程 ENOENT、zsh 中 Pi 正常；随后于 2026-10-30 授权升级 Velune 适配器、移除仓库隔离 Pi 安装，并修复删除 Pi 实例失败（诊断编号 `a521ca2d7a77d36c34f52fc6132e4770`）。本轮实现和隔离验证，不提交、不重装日常应用；保留工作区已有的 composer、转录、侧聊与 SVC 改动。

## 当前结果

- 适配器兼容区间扩至 `≥1.0.2、<1.5.0`，展示为 Pi Agent 1.4。1.4.0 保留了所消费的 SDK／RPC／历史接口，沿用 `pi-1.0` 实例身份；Rust 与 JavaScript 的范围同步。接入 1.1 起 `agent_settled.aborted` 的取消证据，没有助手消息时也可判定取消；没有明确结束证据时不推断成功。
- 删除实例只在使用该执行实例或来源实例的会话／后台任务工作时拒绝；其他实例工作不阻止删除失效配置。2026-11-02 用户进一步授权修复保存的全局门禁及相似误拦截，当前实现与验证见下文。
- 已删除 `target/pi-runtime` 实际目录及 `scripts/install-pi-runtime.sh`，移除现行开发说明中的安装入口。验收使用用户现有的外部 Pi 和合成 HOME。
- 实现与行为探针已完成；SVC Mac 构建及签名验证已通过；严格 clippy 已通过。Pi 与删除场景的行为验收完成；完整 Rust 测试的 3 个 Codex 协议夹具辅助进程超时仍未定位，作为交付限制保留，不能报告完整测试全绿。

## 2026-11-02 运行中重命名

用户继续指出运行中会话不能重命名，询问是否同类人为限制。确认 Mac `canRenameConversation` 额外检查运行状态，核心 `manage_conversation`／`manage_native_conversation` 将重命名与删除一并要求空闲，且当前会话修改前主动关闭执行；均为 Velune 自施的统一约束。沿用上一轮“同类问题一并修复”的授权处理，不提交、不重装、不接触真实会话。

已实现：有执行 owner 时，经该原生进程的元数据接口重命名并保留轮次／子 Agent／审批／队列；历史与跨 Harness 首段继续写原生标题，重读关联投影时保留正在执行的尾段。Pi 从原生 `get_state` 确认名称后更新流式投影，Codex／Claude 仅在原生命令确认后更新。删除仍需空闲；ACP／DSH 的原生能力边界不放宽。

当前证据目录 `/tmp/velune-running-rename`：

- `baseline.log`：旧动态库在 Pi 真实 CLI 的轮次处于回环 HTTP 等待时拒绝改名，原因为“请先停止正在执行的任务与子 Agent，再修改会话”，证明拦截出自 Velune。
- `check-final.log`：最终动态库和从其固定副本生成的匹配 Python 绑定通过全部探针。真实 Pi 1.4.0、Codex 0.159.3、Claude Code 2.1.286 在合成 HOME／实例目录与合成回环上游中，等待模型响应期间原生改名、保持该轮运行、标题持久化并继续正常完成；没有增加用户消息。Pi 未首发草稿改名保留；轮次完成后的原生历史读回和再次改名通过。Pi→合成 ACP 的活动尾段在来源改名后继续运行且可停止；活动删除仍拒绝，ACP 原生标题接口不支持仍明确拒绝。
- `clippy-final-pass.log`：agent-runtime 与 application 在当前工作区的最终 `--all-targets --all-features -- -D warnings` 通过；格式与 `git diff --check` 通过。5 份文档的 124 个相对链接／标题锚点通过。收尾时同一工作区的侧聊实现曾暂时调用不存在的 `ExecutionTarget::remove_file`，随后调用方已改用现有的临时目录清理接口；本轮仅按 clippy 合并其嵌套条件、格式化，未改变清理语义。最终检查已恢复通过，不再是本项交付阻塞。
- 初次 SVC Mac 构建遇到共享工作区的新 `RuntimeFailureKind::RateLimited` 与旧生成绑定不同步；确认 Rust 源码具备变体后，以当前源码重新构建、生成绑定即修复，没有撤销其它任务的接口变更。最终执行 `18311efc-f475-43fb-bace-6c5e5002028a` 于北京时间 20:14:10 以 0 退出，回执 `build-final.json`；Rust release、Swift warnings-as-errors、bundle 签名验证通过。
- 原生界面检查使用最终产物的临时副本，bundle ID 改为 `local.velune.visual-preview`，带 `--preview --preview-composer-access`，无 Transport／真实数据。选中合成运行中的第三个会话，从标签标题双击进入改名并保存“运行中改名验证”，AX 确认窗口、侧栏和标签同步更新，`正在运行` busy indicator 与停止按钮仍存在。预览已退出；没有操作日常应用。

本轮实现与验收已完成。仅使用合成数据与 loopback 模型端点，未读取真实会话或认证、未调用计费服务，未提交或重装日常应用。产品与 Windows 对齐说明已更新；Windows 未复验。用户可在 Velune 外运行 `svc run reinstall-macos` 更新日常应用。

## 2026-11-02 配置独立性修复

用户在确认全局忙时判断后要求“检查有没有其它类似的产品行为问题，一并修复”，授权同类源码修复和隔离验证；不提交、不重装或修改日常配置。

- 运行时保存、新增、启停与删除按活动会话的实例依赖判断，包含跨 Harness 历史来源和后台工作。
- 提供商导入预览不再要求会话空闲；配置写入／登录／退出按正在使用的网关及认证来源判断。网关可向原生子 Agent 提供整个模型目录，因此共享该网关的活动执行仍受保护。
- 提供商变更后的执行失效处理只作用于依赖该网关／认证来源的会话，避免保存一个配置导致无关会话关闭。
- 资源写入按目标实例检查；批量应用返回各目标结果；自动同步只推迟忙碌目标，其他目标立即同步；仅解除托管、保留原生文件的操作不要求运行时空闲。
- Mac 去掉配置与资源操作的全局运行中禁用，核心拥有依赖判断和原因提示。仍保留短请求、关闭与交互登录的串行约束。

资源自动补写的待重试记录按实例保存；目标尚忙时不重新扫描其原生文件。交互登录期间仍串行阻止新发送；队列／协作发送入口补检查，定时任务保持等待，避免登录完成时重置刚启动的执行。

验收目录 `/tmp/velune-runtime-config`，均为临时 HOME、合成 ACP、外部 Pi 1.4.0／Node 24.22.1 与回环合成上游；CLI 登录由合成可执行程序响应，无真实账户或公网模型调用。

- `baseline.log`：旧动态库在另一个 ACP 会话运行期间拒绝保存无关实例，报错与用户诊断一致。探针初版的 Python shebang 直接启动超时，改为显式解释器和 typed ACP argv 后完成复现。
- `check-final.log`：最终动态库与匹配 Python 绑定的全部行为探针通过。覆盖失效路径实例在无关 ACP 运行期间更正并保存、启停／新增／删除、活动实例保护；原生 ACP 运行期间提供商预览／导入／提供商与路由增删、合成 CLI 登录／退出且该 ACP 持续运行；Pi 活动目标资源拒写、批量部分成功、无关实例即时同步和目标停止后自动补写（此时 ACP 仍运行）；managed Pi 活动期间可预览、仅解除托管不改原文件、共享网关配置与认证仍拒绝；相同提供商配置重复保存不重置空闲执行；跨 Harness Pi→ACP 的 Pi 来源保存／删除保护、停止后恢复；关闭后重开确认运行时配置已持久化。
- 最终 `cargo fmt --all --check`、`cargo clippy --locked -p velune-application --all-targets --all-features -- -D warnings` 通过，后者日志 `clippy.log`。debug bindings 已构建并供上述探针使用。
- SVC `build-macos` 最终执行 `e9a0b8c9-8207-4aa4-ae47-3000facbe514` 于北京时间 19:21:11 以 0 退出，回执 `build-final.json`；Rust release、Swift warnings-as-errors 与 bundle 签名验证通过。产物 `target/macos/Velune.app`，未替换日常安装。
- 6 份文档共 116 个相对链接／标题锚点检查通过；`git diff --check` 通过。产品定义、来源与 Windows 对齐说明已更新；Windows 本轮未重新构建／真机复验。

本轮修复和约定范围的验证已完成，未提交、未重装、未更改日常配置；由用户在 Velune 外运行 `svc run reinstall-macos` 后体验。保留既有 Codex 测试夹具超时事项；本轮未重复整包测试，不扩大上述行为证据的范围，也未宣称 Windows 或原生界面的完整交互已验收。

## 2026-11-02 会话启动诊断

用户要求诊断编号 `fa2e6b4ea846cf8e99b379f413696843`，本轮范围为只读诊断，未获配置修复或源码修改指令。

- 确认发生于北京时间 2026-11-02 18:34:21，操作为 `bindings.create_conversation`，4ms 内以 `invalid` 失败。对应 macOS `local.velune:transport` 日志保留实际原因：`运行时辅助进程无法启动：No such file or directory (os error 2)`；JSONL 只有分类，没有原因正文。
- 当前 Pi 实例仍保存 Node `24.15.0/bin/node` 与 Pi `1.0.2` 的旧绝对路径，两者均已不存在；`agentDir` 仍存在。创建前的版本探测先用配置的 Node 启动安装解析 helper，故直接阻断点是旧 Node 路径；只更正 Node 后还会遇到旧 Pi CLI 路径缺失。
- 当前安装应用 `/Applications/Velune.app` 中的适配器接受本机 Pi 1.4.0。用其 `pi_sdk.mjs` 对 `/Users/lanzhijiang/Library/pnpm/bin/pi` 解析与版本核对通过；使用 Node `/Users/lanzhijiang/.local/share/mise/installs/node/24.22.1/bin/node`、临时合成 HOME 执行解析后的 CLI `--version`，退出 0，输出 `1.4.0`，stderr 为空。
- 恢复建议：编辑现有 Pi 实例，同时将 `binary` 更新为上述 pnpm launcher、`nodeBinary` 更新为上述现存 Node；保留实例身份与来源目录。当前证据不要求再次重装应用。没有修改日常配置、读取真实会话或凭据，也未启动模型请求；未验证更正配置后的完整会话执行。

本轮诊断已完成。配置更正与真实会话重试由后续指令决定；既有实现和 Codex 测试残余保持原状态。

## 事实与原因

### 2026-11-02 保存配置的全局忙时限制诊断

用户反馈保存运行时配置失败，怀疑其他运行时的活动会话也会阻止保存。本轮核对诊断日志与实现，未修改源码或日常配置。

- 诊断编号 `1baebe3d19658b205a84ce39e59b919d` 对应北京时间 18:39:49 的 `bindings.upsert_runtime_with_execution`，`invalid`、耗时小于 1ms。macOS transport 日志确认原因为“会话正在运行，请停止后再修改运行时配置”。
- `change_runtimes` 的 Upsert 分支使用全局 `execution_active()`，没有按待保存实例筛选；用户关于“其他运行时活动也会阻止保存”的怀疑成立。没有读取真实会话，因此不独立断言当时各实例的活动归属。
- 删除分支已使用 `Session::executing() && Session::uses_runtime(id)`，覆盖当前执行实例、跨 Harness 来源与后台子 Agent。保存后的失效处理也已按来源／执行配置变化逐会话判定。
- 当时建议将保存门禁同样缩小为目标实例的活动依赖，保留来源依赖与后台工作保护；同日后续用户已授权实现，完成情况与行为证据见上方“配置独立性修复”。

报错中的 Node `24.15.0/bin/node` 与 pnpm store 下 Pi `1.0.2` 文件都已不存在。zsh 当前入口为 `/Users/lanzhijiang/Library/pnpm/bin/pi`，版本 `1.4.0`；Node 为 `/Users/lanzhijiang/.local/share/mise/installs/node/24.22.1/bin/node`，版本 `v24.22.1`。旧的已安装应用只接受 `<1.1.0`。快速导入保存安装内部 CLI 及当时的 Node 绝对路径，升级清理旧安装后既有配置不会跟随 shell；本轮没有自动重绑日常配置，重装后需编辑路径或重新导入。

所给诊断编号对应 2026-10-30 08:30:55 UTC 的 `bindings.delete_runtime`，错误类别 `invalid`、用时小于 1ms。日志未保留具体 message，不能仅凭编号确认历史操作命中了哪一分支。旧代码的 `change_runtimes` 对任何活动会话一律返回 `runtime is busy`；本轮移除删除操作中的全局门禁，以依赖关系保护活动会话。

核对了本机外部 Pi 1.4.0 的公开声明、所消费的 SDK 类型及实现，并对照已删除前的 1.0.2 安装；SessionManager、AgentSessionServices、RPC 和认证存储的消费接口兼容。OpenAI Completions 的有效兼容默认值未变；新增的未实现导入选项仍遵守现有明确拒绝边界。[上游 1.4.0 发布说明](https://github.com/earendil-works/pi/releases/tag/v1.4.0)作为版本变更索引，实际行为以本机安装验据为准。

## 验证

临时探针与日志位于 `/tmp/velune-pi-upgrade`，不提交。仅使用合成凭据、会话、项目和本机回环上游；没有读取真实登录或会话、没有调用计费模型。真实数据仅按用户提供的诊断编号查询日志。

- `sdk_check.py` 使用真实外部 Pi 1.4.0、Node 24.22.1，驱动实际 `pi_rpc.mjs`／`pi_sessions.mjs`／`pi_provider_import.mjs`：启动、工具读取往返、多轮对话、侧聊不写原文件、原生取消证据、历史列表／读取／重命名／删除、提供商导入通过。日志 `sdk-check.log`。取消探针曾漏掉 abort 应答之前的 settled 事件，修正探针后通过，不是产品缺陷。
- `app_check.py` 经当前 release 动态库及匹配 Python 绑定驱动实际 application：版本描述、原生模型目录、工具往返、多轮、提供商预览／实际导入、managed 网关及虚拟模型、取消投影通过。日志 `app-check.log`。
- `delete_check.py` 以保持运行中的合成 ACP 会话验证：可删除另一个失效 Pi 实例；活动目标被拒绝，提示含原因；停止后可以删除。日志 `delete-check.log`。探针最初引用旧绑定方法名，修正为当前公开入口后通过。
- 文档相对链接／锚点 6 份检查零错误，`git diff --check`、格式检查通过。

跨 Harness 来源保护探针已通过：真实 Pi 源会话接续到合成 ACP，活动期间删除 Pi 来源被拒绝，停止后可删除；同时确认 managed 网关与虚拟模型路径通过。

SVC `build-macos` 已通过（执行 `0f6ada38-86e6-4af9-b094-33f318416c4b`，2026-10-30 08:54:08 UTC 结束），产物为 `target/macos/Velune.app`；Rust release、Swift warnings-as-errors 构建及签名验证成功。已核对 bundle 的 Pi helpers 与工作区一致。

Rust `cargo clippy --locked -p velune-agent-runtime -p velune-application --all-targets --all-features -- -D warnings` 已通过，日志 `clippy.log`。`cargo test --locked -p velune-agent-runtime -p velune-application` 首次运行 agent-runtime 为 23 通过、3 个既有 Codex helper 超时；新增 Pi 取消单例通过。该同一测试程序在工作区另一轮运行中 26 项全过（`/tmp/velune-context-tests-full.log`），单独重跑该 Codex 夹具仍超时；SVC 原生执行器对照也超时（执行 `f291ff1b-a803-4f3a-bb73-472f89322d9e`，退出 101）。因此不能把原因简单归到普通工具启动器。具体原因尚未查明；本轮没有改动这些 Codex 夹具或辅助进程传输。已停止仍在排队的重复整包测试，保留首次失败、独立 Pi 单例及 SVC 对照日志。

交付边界：Pi 的真实外部运行时／合成上游验证、运行时删除及跨 Harness 保护、release／Mac 构建、严格 clippy 与文档检查通过。完整 Rust 测试存在上述 3 个 Codex 夹具超时，未宣称全绿；其根因调查为保留事项。本轮未提交、未重装或更改日常配置。构建通过后，由用户在 Velune 外执行 `svc run reinstall-macos`，再编辑或重新导入日常 Pi 实例。

依据：[版本与入口解析](../../packages/agent-runtime/src/version.rs)、[Pi 事件投影](../../packages/agent-runtime/src/projection.rs)、[删除用例](../../packages/application/src/local/configuration.rs)、[会话依赖](../../packages/application/src/local/session.rs)、[开发与重装](../../docs/development.md)。
