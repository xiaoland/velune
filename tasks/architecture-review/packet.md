# 任务链实现复核与纠正

## 目标与当前状态

用户要求整理已修正／改进的清单，核查治标不治本与架构腐坏，并纠正。随后明确要求继续收敛已识别的能力投影与缓存依赖。源码实现及关键验收已完成。2026-11-05 已按用户授权提交 073cbee。用户随后更正重装失败，当前继续处理退出死锁并完成重装。

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
