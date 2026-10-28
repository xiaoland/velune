# 账户额度展示修复

2026-10-27 用户要求直接改进现有显示：重置时间具体到分钟，逐项显示多个额度窗口，并修复 Claude Code 提供商额度未出现在 composer 状态弹层的问题。初次授权覆盖必要实现、验证与文档；2026-10-28 用户进一步授权提交并重新安装。

本次修复 Mac 的现有入口。提供商编辑器和 composer 共用本地日期、时分格式。managed 模型或路由按提供商账户显示额度；原生 Claude Code 与默认选择从所选运行时读取登录额度，无需先导入提供商。Codex 继续使用原生轮次记录的限额。切换来源时不混入旧账户记录，额度来源不报告重置时间时明确未知，不推算。

已确认：Mac 的相对日期格式导致“下周重置”；composer 仅读取原生轮次的 Codex rateLimits。Claude CLI 额度适配报告 5 小时、7 天窗口，但没有稳定时间戳，核心保留未知。Windows 提供商页已有分钟格式；composer 的提供商额度接入作为平台差异记录。

执行顺序：补全展示和账户选择 → 合成回归覆盖多个窗口、路由去重、模型范围及跨账户切换 → Mac 构建与原生渲染检查 → 同步 PRD 和平台差异、检查链接及 diff。

## 首次实现的证据（后续发现原生路径遗漏）

- `bash scripts/check-account-quota.sh` 已用最终生成的 Swift 绑定通过：直连多窗口、路由账户去重、成员模型范围并集、切换账户、原生／未知模型不猜账户、首次读取前保留账户、未登录及无额度来源过滤、本地日期与分钟、未知时间。日志为 `/tmp/velune-account-quota-final-check.log`。
- 原生 `NSHostingView` 离屏渲染已确认同一账户的 5 小时／7 天窗口、各自余量和完整重置时刻，以及原生记录的两个限额。内容使用真实 `StatusDetails`／`QuotaWindowRow`，设置链接在临时探针中为替身，因此不是端到端点击验收。四账户详情的视口为 600pt、内容为 748pt，实际滚至 148pt，最后账户和设置入口完整可见；短详情为 398pt／352pt。发现并修复了仅限制外框高度导致滚动视口仍超高的问题，改为内容能放下时自然收起、否则使用受限滚动区。截图和探针日志位于 `/tmp/velune-account-quota-ui/`。
- 修改文档的 61 个相对链接与标题锚点通过；`git diff --check` 通过。
- Rust Release 核心构建通过，日志为 `/tmp/velune-account-quota-build.log`。随后脚本在构建绑定工具时遇到开发文档已记录的 Cargo registry 软链接／Askama 路径错误。确认已有 `target/debug/velune-bindgen` 使用相同 UniFFI 0.32.2，且入口仅调用 `uniffi_bindgen_main`，因此复用它从本次 Release 库生成绑定，继续原构建脚本的 Swift 编译与打包步骤。Swift warnings-as-errors 构建及签名打包通过，日志为 `/tmp/velune-account-quota-swift-build.log`；产物为 `target/macos/Velune.app`。该替代路径的临时脚本为 `/tmp/velune-account-quota-finish-build.sh`，没有修改仓库构建脚本。
- 上述验证阶段未替换已安装应用、未提交，也未读取真实账户或运行模型。Windows 的状态弹层尚未接入提供商额度，已在对齐表明确记录。

## 提交与安装（2026-10-28）

用户授权后，复用上述验证产物，通过 `scripts/install-macos.py` 安装至 `/Applications/Velune.app`（0.1 beta.1）。旧应用正常退出，安装后的签名严格验证通过；程序、核心库及 build manifest 的 SHA-256 均与源 bundle 一致。程序哈希为 `8481f7194faadfe1d22950df8109a4b2e5f942a819e899d2016232aca54a5518`，核心库为 `b721963a78dd1b7924f5c6d16844533ac924a5f3a701f0b6602d82aec6a7cc16`。后台重新启动后，已确认进程运行于安装路径，未检查真实账户或会话内容。

该产物在提交前构建，manifest 保留实际构建基线 `06aea5a` 与 `dirty: true`，没有改写成提交后构建。相关源码从通过验证后未再变化。本次实现、验证和安装已完成，代码与本记录一并提交；独立遥测任务的并发文档改动不包含在本次提交中。用户体验仍待反馈。

## 原生路径遗漏（2026-10-28）

用户安装后仍看不到 Claude Code 额度，并确认选择的是原生模型或“沿用当前／运行时默认”。复查发现 `ComposerAccountQuota.accounts` 只接受 managed 模型；native 与 nil 会直接返回空，因此不会触发额度读取。上轮回归还把原生返回空当成正确断言，组件渲染没有覆盖实际入口，不能证明原始需求完成。此前“已接入 Claude Code”的结论仅对 managed 路径成立。

继续在原授权内修复。原生运行时的登录额度来源现已确认：核心已拥有登录目录、执行位置与 HOME 的规则，前端不能按模型同名或路径自行猜账户。原生运行时可以保留自己的 API 配置，因此关联登录的额度也不能冒称本轮请求的实际计费账户。已补齐 native／default 的来源、读取与状态入口，并用合成多账户场景检查 AppStore、真实额度服务与 composer 展示模型；只构造详情组件不再作为完成证据。

用户补充：提供商的账户来源是“本机 Claude Code 登录”。顾问独立复查后建议原生运行时直接读取自身 CLI 登录额度，避免把导入提供商变成原生能力的前提；该建议已采用。现有提供商额度服务还负责调度、上限、提醒与历史，因此只共享 Claude 采集器，原生只读缓存单独拥有生命周期，不创建伪提供商。

修复顺序已确定：共享 Claude 用量采集与解析 → 核心增加原生登录额度查询、按需刷新与取消，并声明运行时能力 → Mac 按原生／managed 来源读取中性额度展示 → 用无提供商及双账户的实际 Store／核心／CLI／展示模型链路验证，再验证来源变更、迟到结果和不支持的 HOME／SSH → 构建、提交并重新安装。当前完整范围为本机继承 HOME 的 Claude Code；其它执行位置明确返回不支持，不能回退为本机账户。

验证方式已按用户反馈调整：一次临时原生 UI 试验启动了可见的 Probe，因测试断言失败出现崩溃提示；该试验不构成功能证据，已清理临时应用与进程。用户要求优先使用 Velune CLI 和日志，不再启动可见测试 App。已通过现有 CLI 读取服务状态，并仅统计近期日志的额度操作名与结果，未输出日志正文；所检片段只有额度提醒轮询。后续使用正常退出码报告的后台检查，贯通真实配置映射、Store 观察循环、核心 CLI 采样与 composer 展示模型，窗口交互留作用户体验，不再宣称已经点击验收。

## 原生修复的交付检查

原生登录额度使用独立缓存，和提供商共享 CLI 用量采集器。缓存合并同一来源的并发读取，限制最多四个采集，成功保留一分钟、失败退避三十秒；修改来源、删除运行时与关闭应用会取消采集，旧结果不能覆盖新来源。Mac 使用核心声明的能力，从实际配置映射启动观察循环，首次读取、失败和不支持状态均保留可见入口。登录额度不冒称本轮请求的实际计费账户。

- Rust application／bindings 的 check、Clippy（拒绝警告）、application 无默认特性检查，以及 Release 库构建均通过。日志分别为 `/tmp/velune-native-quota-check-final.log`、`/tmp/velune-native-quota-clippy.log`、`/tmp/velune-native-quota-portable.log`、`/tmp/velune-native-quota-release.log`。
- Mac 使用新核心生成绑定，warnings-as-errors 编译与签名打包通过，日志 `/tmp/velune-native-quota-mac.log`。生成器复用方式与上次相同。
- `bash scripts/check-account-quota.sh` 使用本次绑定通过。移除了“原生必须返回空”的错误断言，补上原生来源保留所有窗口且不继承提供商上限的约束。日志 `/tmp/velune-native-quota-pure.log`。
- 后台贯通脚本为 [manual-native-quota.py](../../scripts/manual-native-quota.py)，入口使用实际 Store 的观察循环和核心配置，只有 CLI 与用户选择是合成输入。核心来源检查已通过，随后复跑发现 helper 的 EOF／退出竞态，已用修复后的核心通过完整链路检查；不打开窗口，不验证真实用户账户或 popover 点击。
- 修改文档的 83 个相对链接和锚点通过。全仓格式检查遇到并行遥测任务尚未格式化的文件，因此当前任务另行限定检查，不修改对方源码。

后台贯通、重新安装与启动检查已完成；按已有授权提交当前任务。

后台检查又暴露了共享 CLI helper 的旧缺陷：stdout EOF 后立即杀进程组，再读取退出码，可能把尚在正常退出的 CLI 杀死。合成 CLI 输出有效 JSON、关闭 stdout 后等待 100ms 再正常退出，旧核心稳定返回 Unknown。现已采用顾问复核的修正：Unix 在同一个总超时内用 waitid NOWAIT 观察退出，保持子进程尚未回收，清理进程组后再回收；Windows 使用稳定 Job handle，在正常退出后清理。独立编译实际 helper 的四个后台用例全部通过：延迟正常退出、非零退出、EOF 后超时和输出上限，日志 `/tmp/velune-credential-eof-tests.log`；用例保留在 [credential-helper.rs](acceptance/credential-helper.rs)。主验收的合成 CLI 同样加入了延迟退出，避免再次漏掉此竞态。

为避免并行遥测改动进入安装产物，当前任务使用私有 Git index 冻结源码到 `/tmp/velune-quota-build-source`，以 `target/quota-final-cache` 复用依赖并重建核心。Mac 源码和绑定接口没有再变，复用已通过编译的 Mac 可执行文件，与新核心重新签名为 `target/quota-final/Velune.app`；最终后台检查将使用这个 bundle。

隔离 Release 构建先后在 gateway 和 application 编译进程收到 SIGKILL，未报告源码错误；降低并发后 gateway 通过，application 仍被中止。因此仅为 application 包使用 Cargo 临时配置 `profile.release.package.velune-application.opt-level=1` 降低编译内存，源码与仓库构建配置不变。最终产物必须在此构建通过后重新验收，不能把此前成功的库冒称为含 helper 修正的产物。

最终核心以单任务及 application opt-level=1 构建通过（3m14s），日志 `/tmp/velune-native-quota-final-release-lowmem.log`。生成的新旧 Swift 绑定逐字节一致，确认无需重新编译 Mac 可执行文件；替换核心后完成深度严格签名验证，日志 `/tmp/velune-native-quota-final-package.log`。源码快照树为 `7ede81564cba5687307241a7d033f6f82feb33f9`，manifest 记录了这次核心重建配置。

最终 `manual-native-quota.py` 全部通过，日志 `/tmp/velune-native-quota-final-acceptance.log`。观察包含真实配置映射、Store 的生产观察循环、typed transport、核心缓存与合成 CLI，最终读取 composer 的来源及窗口模型。无提供商时原生与默认选择读取 A；有提供商时 managed B 与原生 B 均读取 B，A／B 使用同一模型名但不同额度。核心额外验证默认目录不设置 CLAUDE_CONFIG_DIR、来源变更后拒绝旧结果、删除与取消、失败状态、不同 HOME／SSH 明确不支持且不读取本机账户。所有合成用量命令先关闭 stdout 再延迟退出，覆盖此次 EOF 竞态。该证据没有包含真实账户数据或 popover 点击。

最终 bundle 已通过安装器替换至 `/Applications/Velune.app`，正常退出后安装并在后台重新启动。安装后的程序、核心库和 manifest 均与验证产物 SHA-256 一致，深度严格签名验证通过，安装路径的 Velune 进程已确认。核心库 SHA-256 为 `5746847ee91852f2dddec568797a6de5337c6dec844d65e665ecd399abe05e68`。安装未读取真实账户或会话正文，真实 popover 体验仍留给用户反馈。当前任务源码、文档和验证文件使用私有 index 提交，未包含并行遥测改动。

## 模块边界重新复核（2026-10-28）

用户质疑 CC 专用区块及耦合，当前结果转为技术边界复核。源码检查与独立顾问复核已完成，结论见 [边界复核](boundary-review.md)。界面复用了通用额度组件，但 Mac 自行解析下一轮的来源并选择查询，通用 RuntimeQuota 又混入 CC 适配细节。建议先由 application 统一来源解析和展示投影，分离 CC 配置捕获与观测生命周期；保留提供商策略、原生只读观测和历史记录的不同含义，暂不合并账户缓存。另记录了跨原生运行时展示历史限额的代码推演风险，尚未运行复现。

本轮只更新复核记录，尚未实施该重构，也未构建、提交或重新安装；先前安装结果仍是 bb3f86b 的额度修复。重构的边界、迁移顺序与验收条件已经形成可审阅方案。

## 边界重构实施

用户已明确授权按复核方案重构。核心新增 composer_quota 用例，以显式下一轮意图解析全部路由成员、来源及窗口范围；Mac 删除来源解析及 provider/runtime 两套观察分支，只展示核心条目。CC 来源捕获和采集分派移入 native_quota_source，缓存接收捕获结果，不读取运行时字段；提供商策略和原生只读缓存保持分离。公开观测值类型去除 Provider 前缀，历史窗口的未知时间以 None 表达。

Rust 构建、原生／默认及完整路由的 API 回归、历史归属单元检查和 Mac 后台贯通验证均已通过。此前错误的 Mac 来源断言已移除，来源选择现在由核心 API 验证。验证细节和平台范围见下方交付证据。界面验证不创建可见测试 App。

### 重构交付证据

隔离 Release 构建通过，日志 `/tmp/velune-quota-refactor-release.log`；复用上次依赖缓存，application 沿用已验证的 opt-level=1 设置。为避免重复编译，调试 check 已取消，以完整 Release 构建验证最终源码。四项核心单元检查全部通过：提供商窗口范围与路由范围并集、跨原生运行时的历史归属、managed 记录不能充当原生观测、取最近交付的会话上下文；日志 `/tmp/velune-quota-refactor-tests.log`。

新的 typed API 回归通过：无提供商的 native／default 查询、完整路由成员与来源去重、同名模型的账户隔离、来源变更／删除／取消、失败及不同 HOME／SSH 的 Unsupported 状态。Mac 使用新绑定编译和链接通过；临时源码目录没有 Git 元数据，最初的 manifest 步骤因此退出，随后显式指定源码基线和私有 index 完成记录及签名，未跳过编译。日志为 `/tmp/velune-quota-refactor-core.log`、`/tmp/velune-quota-refactor-mac.log`、`/tmp/velune-quota-refactor-package.log`。

最终 bundle 的无窗口 Store → transport → 核心查询 → CLI → 展示条目贯通通过，覆盖无提供商、原生／默认、提供商与运行时切换；日志 `/tmp/velune-quota-refactor-acceptance.log`。Mac 呈现检查通过，日志 `/tmp/velune-quota-refactor-presentation.log`，检查使用正常错误返回码报告失败。开发验证未读取真实账户或会话，也未验证实际 popover 点击。

Windows 在独立临时目录通过完整 build-windows.ps1：Rust Release、固定版本 C# 绑定生成器、dotnet publish 和资源打包均通过。初次包装脚本把 Cargo stderr 进度当作 NativeCommandError，调整为操作系统输出重定向后通过；日志 `/tmp/velune-quota-refactor-windows-retry.log`。本次 Windows 仅适配 DTO 名称和可选有效期，未改变布局，也未做浅色／深色交互验收；composer 消费统一查询仍是既有平台差异。远端隔离位置为 `C:\Users\yyh\Temp\velune-quota-refactor-cff4c533`。

Mac 的正常退出请求超时后，按 development.md 中已有的直接退出授权，终止了核实过的已安装 Velune 进程，再通过安装器替换。安装后的可执行文件、核心库及 manifest 与验证 bundle 的 SHA-256 一致，严格签名验证通过，安装路径进程已重新启动。

本轮实现与可行验证已完成，按已有授权提交当前任务，保留独立构建脚本改动。源码快照树 `71aa7960093b5824d2f8791bcecf0bcdd803dba0` 由 manifest 记录，保留实际提交前构建的基线与 dirty 状态。用户体验仍待反馈。

安装核心库 SHA-256：`befc25bfd600b7731af687b47dd6a346997b2497a3dbd9aeb4ce0e6d84049a6a`。

Windows 的脚本已写出 Built 结束标记；另核验新生成的 composer 额度类型、发布可执行文件、许可证及核心 DLL 哈希，见 `/tmp/velune-quota-refactor-windows-proof.log`。外层 Start-Process -Wait 还在等待编译服务器后代，核实实际构建进程已退出后仅清理了本任务的等待包装进程。
