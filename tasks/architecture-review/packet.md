# 任务链实现复核与纠正

## 目标与当前状态

用户要求核查整条任务链的治标与结构问题并纠正，已提交首次结构收敛、退出死锁与恢复候选修复。20:30 补全崩溃后，按用户“架构上的上游”重新审查，确认四端缺少统一目标准备用例及未手选语义。这一轮共享用例及四端集成已经完成；用户已确认未手选继续继承默认、手选后只固定该项。源码与验收状态见文末及 [Composer 用例边界](composer-target-use-case.md)。部署和真实两组七 Tab 恢复继续等待用户稍后统一重启。

## 结果导航

- [36 项改进清单](inventory.md)说明各行为当前的职责划分。
- [功能复核与纠正](review.md)记录首次 15 类缺陷修正。
- [结构复核与治理](structure.md)记录五类结构收敛、选择依据及最终证据。
- [输入与执行](composer-execution.md)、[运行时生命周期](runtime-lifecycle.md)、[管理与调度](management-scheduling.md)、[消息与工作区](transcript-workspace.md)保留分组发现。
- [迁移复核](migration.md)记录中断恢复与组合验证。
- [输入结构](structure-input.md)和[提交结构](structure-submission.md)记录关键反例。

## 已完成的结构收敛

Mac 编辑模型只拥有一份 InputID 内容、修订与 undo，删除旧正文保存队列和普通目标配置的第二写入口。侧聊输入归临时 Tab，跨窗移动拆开窗口编辑对象并保留共享内容身份。

application 同源计算发送、排队、插入及继续／压缩的共同准入。新建策略归实例配置一次提交，lastChosen 带 Adapter Type，平台删除第二次保存。显式迁移包含中断恢复。

侧聊策略由 adapter 声明，核心准入与能力描述共用该事实，Mac 消费描述。原生打开与上下文重建穷举策略。历史缓存直接使用实际 reader／ACP launch 配置表达来源，统一覆盖列表、项目、归档、按身份回退和客户端上次列表。执行配置保留独立依赖，Pi sdkHelper 的真实差异由检查约束。

## 验收与交付

证据根为 `target/verification/architecture-review-20261105`。

- 首次复核完成 Composer API、调度、迁移恢复、远端 supervisor 及原生生命周期的定向检查。Mac 隔离 new-tab 与 navigation 检查通过，navigation 共 9 项。
- `structure` 阶段提交政策 8 项、默认策略 API 14 项、真实核心配置提交／失败回滚／重开 1 项、新迁移 3 项与既有组合 6 项通过。四端构建通过。Mac content-target、controls、new-tab、runtime-editor 四个隐藏入口通过，覆盖文本编辑／撤销、跨窗移动、侧聊归属、双 client CAS 和真实表单单提交。
- `closure` 阶段侧聊契约 1 项、缓存与实际客户端切换 3 项、NextTurn API 12 项通过。严格 application release Clippy、最终 Mac 原生库、Swift 绑定、应用编译与严格签名验证通过。复用已有交互证据。

Mac 最终成功日志为 `closure/macos-final-build.log`。SVC build-only 原生阶段成功后，停止本轮重新编译 debug 生成器的进程树，复用既有同版 UniFFI CLI，续跑已删除安装分支的构建尾段。此前复核中一次尾段漏传 build-only 曾触发安装器，被日常运行进程检查拒绝，未发生应用替换，该过程已向用户报告。用户本次自行完成重装。

Windows 临时 worktree 与上传文件已清理，解除 junction 后保留共享构建缓存。验证使用合成数据，平台 UI 与完整交互矩阵按影响复用。模型容量错误后的最终消费者复核及验收由 root 接管完成。

## 实际限制

历史 ENOBUFS 根因仍未确认，资源释放和取证能力的修正已经交付。跨多个独立完整宿主的共同保存端部署，以及各平台原有 UI 验证边界见清单与分组记录。用户报告重装成功，本轮按该事实更新交付状态。

## 重装退出死锁

用户更正重装未完成，SVC 报“控制者拒绝或执行失败”，随后应用无响应。SVC 子执行 `300f1f3d-7beb-40c8-a1b8-89e0500040a3` 实际停在正常退出超时，尚未替换应用或迁移。现场 PID 75887 主线程采样在 `reinstall-hang/daily-sample.txt`，明确显示系统退出兑现 pasteboard promise，经 SwiftUI 同步 NSItemProvider load 等待 semaphore。

会话行和分组的 registerDataRepresentation 都将 completion 异步投递主线程。主线程正等待该 completion，形成自等。修复直接兑现已捕获的不可变 payload，两个 provider 走同一模式。用户已明确授权在修复验证后强制结束旧进程并继续重装。验证只覆盖 provider 在主线程停止推进时返回数据及修复后的实际退出／重装关键路径。

### 退出修复与实际交付结果

两种 production provider 的完成回调显式声明为 `@Sendable (Foundation.Data?, Swift.Error?) -> Void`，直接返回捕获的数据，整个回调独立于 UI actor。完整构建最终通过，日志 `reinstall-hang/swift-build-3.log`。复用本次用户重装已生成且原生源码一致的 Rust 库，SVC 构建入口触发的重复原生重编仅停止其自有进程树，然后续跑 Swift／资源／签名。

合成隔离入口 `--experience-drag-provider-only` exit 0，共 3 项通过。旧 main.async 模式在主线程限时等待下超时，随后排空负对照。会话行与分组的实际提供器在主线程等待期间返回正确数据。未操作系统剪贴板或输入。

按用户明确批准，验证后强制结束卡在退出阶段的 PID 75887，当时没有子进程。首轮真实迁移发现 3 条已删除运行时的孤立默认选择，已确认无重复 ID。完整原选择与三条孤立记录先私有备份到 `before-orphan-runtime-choice-review-f1jqyu2g`，仅从当前运行时默认集合移出孤立项，历史目标与输入记录保留。随后完整重装脚本 skip-build 成功，所有迁移完成，策略备份为 `before-runtime-start-policy-m8sg0td4`。Codex 退役备份为 `before-codex-166-ir_736h0`。

最终日志 `reinstall-hang/reinstall-final.log` exit 0。新版 PID 35118 已启动，实际安装二进制和验证产物 SHA-256 一致，严格签名通过。现场新采样 `installed-sample.txt` 显示主线程正常处理事件，启动 error 级统一日志查询仅有表头。源码关键检查、实际安装和迁移已完成，继续由用户 dogfood 观察真实拖动后的退出体验。

## 重装后的工作区恢复

用户反馈原先 Tabs 未恢复。只读核对窗口元数据后确认，19:08:37 的退出恢复点仍有两组七个 Tab，七个会话来源均为本机已配置实例。该旧快照缺少 windowDevice，重装后 scene 身份丢失，严格的同设备 latest 筛选拒绝它，新版创建了一组一个 Tab 的新恢复点。此前仅验证了旧无归属快照被排除，遗漏了该真实版本切换的显式数据迁移。

root 负责备份并显式将已确认的旧工作区映射为 local，恢复到当前 scene 的 journal 身份，正常重启核对实际恢复。当前窗口也先完整备份，保留撤回能力。structural_input 负责两个独立入口缺陷及合成反例，无效 SceneStorage 不得抑制有效 journal 回退，最新不完整 journal 不得遮住更早完整候选。继续保留设备隔离及既有 seed 语义。

### 本次恢复修复的验收与部署暂停点

Mac Swift／资源／严格签名构建通过，日志 `reinstall-hang/workspace-build.log`。合成隔离入口 `--experience-workspace-only` exit 0，共三组检查通过，覆盖设备／legacy 边界、无效最新及 exact 快照、实际 window.start 对 SceneStorage 合法性与 seed 的选择。

正常退出脚本已发出请求，但超时返回，日常 PID 35118 继续运行。用户随后明确要求“别退出，我在用，稍后我一起重启”。当前禁止再次请求退出、强制结束或部署，直到用户明确恢复该步骤。shell 使用 set -e，布局迁移与重装均未执行，真实 journal 未改写。

已准备的一次性人工恢复工具在 `target/verification/architecture-review-20261105/reinstall-hang/restore-workspace.py`，尚未执行。它在应用退出后获取 runtime.lock，核验旧两组七 Tab 的结构与本机来源，完整私有备份原恢复点和当前 scene 恢复点，再写入确认的 local／conversation 身份及原布局。源 snapshot 为 EF954EDE-D0DF-4E16-84D3-ECA8A55D229E，当前 scene 为 B29D05CB-9872-4B55-B693-FD0B5A109BA1。届时须重新核对当前 scene 和旧快照，正常退出后应用恢复、安装已验证构建、启动并核对实际重新保存的两组七 Tab。单纯重新打开当前已安装版本尚不会执行这项人工映射。

## 20:30 输入补全崩溃的上游复核

实际崩溃报告 PID 35118，主线程反复经过 attach → updateCompletion → canPerform → composerContext → model lookup → attach，最终栈耗尽。历史对照确认，073cbee 前 attach 只绑定 Store，show(false) 在会话目标变化后刷新并先记录目标；结构重构删除 show 后将刷新搬入每次 attach，丢失了原来的时序前提。用户要求继续追上游，并反对新增这类专用回归检查，本轮不新增测试入口或深度计数检查。

局部复核先拆开内容目录、编辑器绑定和补全刷新。随后宏观复核推翻了仍由 Context.prepare、Window 和 View 各推进准备的安排，最终责任见下一节。保留的修正是：资格查询读取已有模型，attach 只绑定 Store，原生焦点与编辑动作维护编辑器归属；Tab 观察统一释放。未新增递归深度检查或专项测试入口。

工作树同时有另一任务的模型表单、消息编辑和 Pi 图片投影改动，保留并单独提交本轮文件。应用当前已崩溃退出。此前用户要求稍后统一重启，当前只实现和验证源码，部署及两组七 Tab 的实际迁移继续等待用户恢复该步骤。

## 当前宏观收敛阶段

用户澄清“上游”指架构宏观层级。已重审共享用例边界并确认跨端初始化语义分叉。用户选择“未手选继续继承默认，手选后才固定”。实施判断、责任及终态验收见[Composer 用例边界](composer-target-use-case.md)。当前局部安全修改保留为一部分，尚未认定共享架构收敛完成。

shared_composer_target 负责共享 API／application／bindings 用例和逐字段规则。structural_input 负责 Mac 会话模型生命周期与纯投影消费。target_client_consumers 负责其余三端。root 负责显式迁移、文档、最终集成与分层验收。其他会话的模型表单和消息编辑 dirty 改动继续保留，不在本任务中回退或自动提交。实际应用已崩溃退出，仍遵守用户“稍后统一重启”的部署暂停决定。

### 宏观实现与已取得的证据

共享 facade 统一规范身份、authority 单目标读取、空意图首次保存及有效配置解析；薄 DTO 只返回身份和轮次配置，不通过目标准备传输会话正文。四端普通打开退出本地 seed 路径，Mac Store 拥有请求及观察寿命，窗口负责内容与目标配对，View／Context／菜单／补全消费投影。配置失败不阻断正文编辑与保存。

`composer-use-case/public-target-probe.log` 7 组公开 API 合成验证通过，包含普通 Claude 历史身份、执行端与保存端分离、动态默认、逐字段明确选择、只读 overlay、foreign owner 拒绝、失败保留、CAS 和 authority 重开。`migration.log` 证明预览无写、明确选择与正文／附件／绑定保留、私有备份、修订推进和幂等。真实 target-intent 迁移未执行。

`native-final-build.log`、`macos-final-build.log` 与仅修正旧夹具后的 `macos-fixture-build.log` 通过，`clippy-final.log` 严格检查通过，`next-turn-final.log` 既有 12 项 resolver 检查通过。`native-controls.log` 现有 6 组原生输入检查通过，`native-content-target-fixture.log` 现有 8 组内容／目标检查通过，涵盖原生撤销、跨窗口移动、迟到失败及实际双 client CAS 恢复。旧 CAS 尾段用不存在的普通会话伪装新建目标，现按真实全局新建身份和实际修订基线修正；未绕过共享准备。此前两次参数／临时目录错误和该旧夹具失败不作为产品通过证据。

`platforms/{ios,android,windows}-build.log` 均通过，覆盖新 native、三种语言绑定及各应用构建；Android 包含 lint，Windows 包含资源打包，未启动 Windows UI。Mac 检查窗口未显示，无前台激活、鼠标键盘注入或真实模型调用。真实已安装应用的完整可视菜单／补全旅程、实际跨网络中断，以及部署后的两组七 Tab 恢复未验收。它们不被此次构建或合成检查替代。源码提交后按用户暂停决定保留部署义务。

构建证据来自当前集成工作树。其它并行会话的模型表单、消息编辑及 Pi 图片投影改动未纳入本轮提交，也未回退；本轮没有声称已单独验收这些功能。
