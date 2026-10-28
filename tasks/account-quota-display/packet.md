# 账户额度展示修复

2026-10-27 用户要求直接改进现有显示：重置时间具体到分钟，逐项显示多个额度窗口，并修复 Claude Code 提供商额度未出现在 composer 状态弹层的问题。初次授权覆盖必要实现、验证与文档；2026-10-28 用户进一步授权提交并重新安装。

本次修复 Mac 的现有入口。提供商编辑器和 composer 共用本地日期、时分格式。composer 按下一轮所选的 managed 模型或路由找到提供商，按账户分组列出适用窗口和余额，复用核心的按需读取。原生模型继续显示运行时记录的限额；切换到 managed 模型时不混入旧轮次的原生账户记录。额度来源不报告重置时间时明确未知，不推算。

已确认：Mac 的相对日期格式导致“下周重置”；composer 仅读取原生轮次的 Codex rateLimits。Claude CLI 额度适配报告 5 小时、7 天窗口，但没有稳定时间戳，核心保留未知。Windows 提供商页已有分钟格式；composer 的提供商额度接入作为平台差异记录。

执行顺序：补全展示和账户选择 → 合成回归覆盖多个窗口、路由去重、模型范围及跨账户切换 → Mac 构建与原生渲染检查 → 同步 PRD 和平台差异、检查链接及 diff。

## 当前证据

- `bash scripts/check-account-quota.sh` 已用最终生成的 Swift 绑定通过：直连多窗口、路由账户去重、成员模型范围并集、切换账户、原生／未知模型不猜账户、首次读取前保留账户、未登录及无额度来源过滤、本地日期与分钟、未知时间。日志为 `/tmp/velune-account-quota-final-check.log`。
- 原生 `NSHostingView` 离屏渲染已确认同一账户的 5 小时／7 天窗口、各自余量和完整重置时刻，以及原生记录的两个限额。内容使用真实 `StatusDetails`／`QuotaWindowRow`，设置链接在临时探针中为替身，因此不是端到端点击验收。四账户详情的视口为 600pt、内容为 748pt，实际滚至 148pt，最后账户和设置入口完整可见；短详情为 398pt／352pt。发现并修复了仅限制外框高度导致滚动视口仍超高的问题，改为内容能放下时自然收起、否则使用受限滚动区。截图和探针日志位于 `/tmp/velune-account-quota-ui/`。
- 修改文档的 61 个相对链接与标题锚点通过；`git diff --check` 通过。
- Rust Release 核心构建通过，日志为 `/tmp/velune-account-quota-build.log`。随后脚本在构建绑定工具时遇到开发文档已记录的 Cargo registry 软链接／Askama 路径错误。确认已有 `target/debug/velune-bindgen` 使用相同 UniFFI 0.32.2，且入口仅调用 `uniffi_bindgen_main`，因此复用它从本次 Release 库生成绑定，继续原构建脚本的 Swift 编译与打包步骤。Swift warnings-as-errors 构建及签名打包通过，日志为 `/tmp/velune-account-quota-swift-build.log`；产物为 `target/macos/Velune.app`。该替代路径的临时脚本为 `/tmp/velune-account-quota-finish-build.sh`，没有修改仓库构建脚本。
- 上述验证阶段未替换已安装应用、未提交，也未读取真实账户或运行模型。Windows 的状态弹层尚未接入提供商额度，已在对齐表明确记录。

## 提交与安装（2026-10-28）

用户授权后，复用上述验证产物，通过 `scripts/install-macos.py` 安装至 `/Applications/Velune.app`（0.1 beta.1）。旧应用正常退出，安装后的签名严格验证通过；程序、核心库及 build manifest 的 SHA-256 均与源 bundle 一致。程序哈希为 `8481f7194faadfe1d22950df8109a4b2e5f942a819e899d2016232aca54a5518`，核心库为 `b721963a78dd1b7924f5c6d16844533ac924a5f3a701f0b6602d82aec6a7cc16`。后台重新启动后，已确认进程运行于安装路径，未检查真实账户或会话内容。

该产物在提交前构建，manifest 保留实际构建基线 `06aea5a` 与 `dirty: true`，没有改写成提交后构建。相关源码从通过验证后未再变化。本次实现、验证和安装已完成，代码与本记录一并提交；独立遥测任务的并发文档改动不包含在本次提交中。用户体验仍待反馈。
