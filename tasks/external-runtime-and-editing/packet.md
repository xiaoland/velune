# 外部运行时接入与双击编辑

2026-10-06 用户授权将双击编辑应用于全部可编辑列表、排查系统 Pi 公开版本读取失败、依据真实 breaking-change 增加必要旧版本适配器，并明确禁止随 Velune 捆绑任何 Agent 运行时。产品意图已进入 [PRD](../../docs/prd/index.md)；未来安装运行时功能不在本轮。

系统 Pi 为 0.98 只是用户猜测，不能视作事实。调查先区分无法执行版本命令和已识别但不支持的版本，再核对官方包、RPC 与 SDK 边界。移除自带 runtime 必须贯通执行、历史、提供商导入、认证与资源打包，不能只删 CLI 文件后仍从 bundle 使用完整 SDK。

root 拥有 packages 与构建／安装、权威文档和静态检查；session_management 拥有 Mac 全部可编辑列表交互；ai_service_audit 负责公开安装与版本差异证据、随后必要人工验收脚本；advisor 用于外部 SDK 解析与版本 adapter 的关键判断。owners 不回滚他人改动。guides/delegation.md 仍缺失，按既有共享协作规则协调。

不新增自动化测试、不读取真实配置／凭据／会话。验证采用临时 HOME、实际外部运行时与合成 loopback、类型检查和构建，最后重新安装0.1 beta.1。开发实现与隔离验收已完成，最终安装状态见末尾；GUI体验待用户验收。

用户后续确认：双击进入编辑是项目级 UI/UX 范式；已进入 PRD 与共享规则，未来可编辑列表同样遵循，不限于本轮枚举项。

## 已验证事实与当前实现

公开安装入口指向 Pi 0.85.1，而非猜测的 0.98；目标 CLI 已缺失。隔离 HOME 执行公开版本命令失败，故原因是安装损坏，不能解释为适配器范围拒绝。未修复或升级用户全局安装。npm 的 0.98.0 不存在，不能据此建立 0.9* 适配器。

Pi 0.99.2、1.0.0、1.0.1、1.0.2 官方发行包比对显示，本应用消费的 ModelRuntime、AuthStorage、SessionManager、createAgentSessionRuntime、createAgentSessionServices 与 registerVirtualModel 接口及核心实现保持一致；主要差异为依赖版本与图片 API 增量。这只证明所查版本和当前消费边界，不保证全部未来 1.0.*，也不等于已验收旧版本。当前 adapter 仍精确匹配1.0.2，没有新增无依据的0.9*。

Pi 的 SDK 与 CLI 已从 bundle 完全移除。共享 pi_sdk.mjs 从显式外部 CLI 或已识别的 npm/pnpm launcher 字面目标解析安装；不执行 shell，不继承其 NODE_PATH，不搜索另一安装作为回退。公开 manifest 的包名和 bin.pi 与规范 CLI 一致后才能消费 SDK。入口缺失、包缺失、未知 launcher、版本命令失败与已识别未支持版本分别说明原因；日志仅记录固定阶段／代码。OAuth 来源保存规范 CLI、runtimeTypeId 和精确 sdkVersion，失效时明确失败；独立 API key 不绑定运行时安装。

Mac 使用系统原生 List primaryAction，覆盖全部可编辑列表；提供商内模型聚焦现有详情，模板打开编辑，会话执行真实原生标题重命名。只读候选与公开模型目录不伪装成编辑。单击选择和原生多选保留。

第一轮正式无 Pi SDK 构建已通过 Rust fmt/check/clippy、bindings无默认features检查和Swift warnings-as-errors。外部Pi1.0.2的首循环6个loopback请求、Pi→Codex→DSH→Pi四次接续和实际AppStore加载／重命名路径已通过；后续 launcher 规范化改变了合成发现 fixture，正由原 owners 补齐验收。尚未最终安装；GUI双击手势留用户验收。

官方源复核于2026-10-06：[npm发布元数据](https://registry.npmjs.org/@earendil-works/pi-coding-agent)、[官方CHANGELOG](https://github.com/earendil-works/pi/blob/main/packages/coding-agent/CHANGELOG.md)、[SDK README](https://github.com/earendil-works/pi/blob/main/packages/coding-agent/README.md)。研究 owner 比对四份官方 tarball 的 model-runtime.js、auth-storage.js、agent-session-runtime.js、virtual-models.js，SHA-1一致；相应声明与session-manager.d.ts无差异。launcher模板依据 [pnpm生成逻辑](https://github.com/pnpm/pnpm/blob/main/pnpm11/bins/cmd-shim/src/index.ts)，当前只处理已验证的nativeNode字面目标形式，不声称通用shell解析。

最终源码范围的所有人工脚本已完成外部 --pi 迁移。首循环6请求、原生Messages的Pi导入与Pi/DSH4请求、跨Harness4请求、会话浏览、Pi session resync零请求、continuation失败边界2请求均通过；AppStore新版规范入口fixture复验通过。旧session-management重复入口已删除，现有原生验收承接。DSH临时fixture漏parser依赖曾导致process_exit，已修复为仅链接huihua公开解析依赖并复验。root另以临时文件确认含空格多分支launcher、失效目标、命令替换和歧义目标拒绝，以及冻结SDK版本不匹配／入口失效在凭据文件访问前拒绝。无真实账户、会话或模型服务。

## 最终安装

源码提交2c080bc02bdf6d2a5de7922e1bd7a29b446dd019，干净树构建并安装到标准Applications位置，显示版本0.1 beta.1，签名校验通过。安装包仅包含huihua及其四个parser依赖，不含Pi SDK、CLI或任何Agent runtime。已从安装实际dylib重新生成Python绑定并复验首循环（6个loopback请求）和运行时发现／版本拒绝／去重／配置持久化，两者通过。GUI双击体验由用户验收；若旧实例入口指向应用包内Pi，需显式改为外部安装，不做隐式替代或读取用户配置。

PRD、共享UI/UX原则、架构、各unit说明、开发运行入口及来源索引均已同步；没有远端发布或真实模型调用。
