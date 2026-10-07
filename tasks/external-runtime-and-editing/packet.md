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

## 2026-10-07 Pi 列表缺失诊断

用户授权只读诊断现有 Pi 会话读取失败，可查看与此失败有关的本地 observability。没有授权本轮修改运行时安装／用户配置或修复源码；不读取真实认证及会话正文。复用本 packet，不另开竞争任务。

最近本地错误 operation_id `18dbfbf87b683440-13b7d-2`（2026-10-06 15:58:41 UTC）为 list/history_list，runtime_slot 1。真实 helper stderr 为 pi_sdk 的 `resolvePiInstallation` 首次 `realpathSync(binary)` 失败，code `sdk_entrypoint_missing`、子进程退出 1；发生于 SDK 装载之前，不涉及模型或会话内容。安装 manifest 源码为 `86b4778`、dirty=false。调用链直接把配置 binary 传入 --cli，未发现用模型选择覆盖入口。

公开系统 pi launcher 独立复核：其 pnpm 链接指向 0.85.1，链接目标不存在；该入口的独立解析在 launcher target 的 realpath 失败，区别于 Velune 日志的首次 realpath 失败。不能把两次失败混同，也不能在未取得应用配置的入口字段前断言用户实例使用它。此前移除 bundled Pi 的旧实例路径残留也是候选原因，未证实。

可观测性残余：pi_sdk.mjs 两处 realpath catch 仍抹掉原始 filesystem cause 和实际路径；本轮未修改源码。已向用户请求唯一非秘密字段“Pi 可执行文件”的配置路径，以区分残留内置路径、损坏外部入口及访问失败。终点是核对该入口的公开文件系统存在性／SDK manifest，不打开原生会话或凭据。

用户后续指出“不能确认配置入口”本身就是可观测性缺陷。本轮继续既有完整原因链修复，落实这一上下文要求，不再依赖此前请求用户手工提供路径。PRD 已补齐运行时失败应定位实例和实际非秘密入口的行为；ai_service_audit 继续作为 runtime SDK／application 错误边界与人工脚本的源码 owner，root 负责文档、整合与安装。实现与隔离验证不访问用户真实配置或会话；不修复用户全局 Pi、不替换实例路径、不引入 fallback。

完成依据：两个合成 Pi 实例（入口缺失与 launcher 目标失效）的实际 UniFFI list/read 失败能明确区分实例和入口，错误含原始 filesystem code／cause；日志操作编号可关联，公开解析器保留版本/安装分类与详情；正常外部入口仍工作。类型／静态检查及构建通过，Mac 重建安装。真实现存故障的旧日志无法事后补回已抹掉的路径，新构建的重试应无需用户抄配置。

归因增强的首轮返回未满足验收：JS resolver 源码未改；版本桥虽输出 detail 却未在 Rust 消费；问题详情缺入口，人工脚本未扩展。已将这些源码与终端观察的具体反证回送同 owner 修复，不能据静态检查将其采用为完成。新版安装后重试将只读取相关 Pi 失败日志的运行时定位字段，确认实际配置入口，不要求用户抄路径。

最终归因修复由 root 接管，原 owner 已中断并冻结；此前返回不足不作为完成证据。人工验收首次失败证明 open_conversation 先经 summaries_for 查询来源，入口失效发生于 history_lookup，不能仅在 Pi 内容读取分支加上下文。已补该实际边界，同时保留非 Pi 既有 typed history code。原 SDK 分类与原始 filesystem cause 分开保留，未知错误不泛化；discovery 使用 Node 标准 inspect 展开 Error.cause。

2026-10-07 合成人工验收通过：实际 UniFFI 的两个实例分别触发缺失 CLI 与 dangling launcher 目标，列表和打开会话失败可关联到各自 id/name/type、binary/nodeBinary/agentDir；日志带原始 ENOENT、操作编号和实际失败阶段。版本发现保留 entrypoint_missing 分类与原 cause；正常外部 Pi 1.0.2 的空临时来源与版本发现正常。没有用假 helper 的固定错误字符串代替这段 SDK 验证。既有合成 provider／history parser cause 验证也仍通过。

交付与生产定位：源码 `11042ab53abcb90f281d5f6b12a1770d814f88df` 干净 release 构建安装完成，显示 `0.1 beta.1`，严格签名及 Swift warnings-as-errors 构建通过。正常重启应用后，新本地 Pi list 失败日志 operation_id `18dbfdcdfc3ca520-18277-2` 已直接记录实际实例及非秘密路径；确认原“Pi”实例仍配置已移除的 bundle `Contents/Resources/node_modules/@earendil-works/pi-coding-agent/dist/bundle/cli.js`，原始错误为该包目录的 ENOENT。该入口不是 shell 当前系统 launcher，两者此前的关联假设现已排除。仅查看相关失败日志定位字段，没有读取原生会话内容或真实秘密，也未改用户运行时配置／全局安装。

要恢复此实例的会话列表，需要用户配置有效外部 Pi 安装入口；系统公开 launcher 的 0.85.1 包链接另有损坏。修复或升级用户全局安装尚未授权，不自动使用开发目录下的验证 runtime 替代。诊断改进已完成，原会话列表缺失尚待外部安装／实例配置恢复。

安装实际 release 核心库再次运行 `manual-error-diagnostics.py` 通过，包含真实 SDK 两实例列表／打开失败、日志关联、discovery 分类和正常入口。Rust fmt/check/workspace clippy（all-targets/all-features、-D warnings）、bindings 无默认 features 检查、Node 语法与人工脚本 py_compile 均通过。

## 系统 Pi 修复授权

2026-10-07 用户明确要求修复系统 Pi 安装。pi_install_repair 拥有系统公开入口的安装修复与隔离验证，优先现有 package manager，固定当前 Velune 支持的1.0.2，不任意安装latest；root负责采用结果。此授权替代此前“未授权修改系统安装”的任务限制，不触碰原生auth／会话、其它全局包或自动替换应用实例。并行的Mac界面工作归[体验整顿任务](../mac-navigation-refinement/packet.md)。

系统安装owner返回已采用：原公开pnpm入口指向0.85.1缺失store目录；通过原pnpm渠道安装固定@earendil-works/pi-coding-agent@1.0.2。隔离HOME公开pi --version返回1.0.2，以当前release dylib和fresh Python绑定执行manual-runtime-discovery，版本识别／拒绝、外部入口解析、去重与临时配置持久化通过。未读取或修改~/.pi，未修改真实Velune运行时配置。原Velune实例仍指向已移除bundle CLI属于已确认的独立配置残余；系统公开入口修复不自动替换实例，不把安装成功等同于真实会话已恢复。
