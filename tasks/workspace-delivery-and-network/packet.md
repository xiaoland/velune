# 工作区收尾与整机网络故障

2026-11-04 用户授权：整理整个当前工作区，继续未完成实现，对完成项做关键验收并提交；优先诊断当日两次整机无法打开网页、只能重启的问题。用户补充报错为 BUFF NO SPACE 一类。

## 边界

允许提交当前工作区已有各任务修改。沿用不抢鼠标／前台、不重装日常应用、合成验证、不运行 Windows UI 的约束。用户本次明确要求诊断整机网络，读取相关系统网络状态与故障日志用于取证；不读取会话正文或凭据。默认不改系统网络设置，不做可能再次耗尽整机资源的压力实验。

## 当前事实

- 重启时间为 11:25、19:24（本地时区）。19:01 起内核出现大量 `mbuf_class_under_pressure`，总数72500、剩余约260、max86016。与用户报错一致；尚未确认资源所有者。
- 重启后的 Velune 没有运行；当前系统代理设置关闭，但默认 DNS 经 sing-box TUN utun4。当前资源已恢复，不能用于排除重启前泄漏。
- 工作树包含 209 个 tracked 改动和多批新增文件。今日后续的速度模型变体、原生交接、fork、open退避与迁移重装尚缺最终组合验证；旧任务包的 service tier 方向已被 speed-as-model-variants 取代。

## 推进

1. 定位网络缓冲区耗尽来源，验证可证实的修复；不能证明原因时保留证据和最小捕获手段，不把推测宣称根因。
2. 收拢今日在途接口，完成共享核心编译与必要运行时关键路径；复用此前有效验证。
3. 按可独立审阅的主题提交；跨文件交织无法独立编译的改动以一致的功能集提交，记录剩余部署与用户决定。

## 网络证据补充

- 两份关机 stall 已解码：11:24和19:23均有 `io.nekohasekai.sfamt.system`（1.14.3）阻塞于内核 `mbuf_allocpacket`；早上另一个线程等待同进程的分配线程。用户确认试过退出 Velune／重启代理，未恢复。
- 晚间日志有8534条 mbuf 压力警告（19:01–19:13）；原始统一日志和 spindump 解码暂在 `/tmp/velune-network-evening.log`、`/tmp/velune-shutdown-{morning,evening}.txt`，原始关机诊断仍在 `/Library/Logs/DiagnosticReports/`。
- 当前只能确认整机网络缓冲区耗尽、关机时网络扩展等待分配，不能确认 Velune 或 sing-box 为资源所有者。源码检查未发现系统路由／代理修改；远控代际切换会关闭旧 endpoint、mDNS并 join任务，重连有1–15秒退避；gateway owner析构关闭监听及执行器。尚无能证明根因的复现。
- 新增只读有界 `scripts/diagnose-network-macos.py`，实际基线采集成功、解析错误0。19:41:47–20:01:44 的120个样本中 mbuf 使用196–256，未持续增长；日常 Velune 未运行，因此不能据此排除它是触发条件。随后补入 nettop 进程累计流量／重传，明确区分 socket 队列长度，单样本实际解析通过。
- 独立复核找到 [sing-box #4607](https://github.com/SagerNet/sing-box/issues/4607)：1.14.2 的 Apple TLS engine 有用户报告连接不随 GUI／配置关闭释放，只有终止系统扩展才能清除；截至11月4日仍 open、无评论或修复关联。当前机器是1.14.3，无法确认同一缺陷。报告复现显式设置 `tls.engine: apple`；[官方 TLS 文档](https://sing-box.sagernet.org/configuration/shared/tls/)规定默认是 go。尚未读取本机活动 profile，不能假定它使用 apple，也未改配置或停止系统扩展。
- GUI／配置重开不等于独立 system extension 进程重启；本机扩展由 launchd 托管。下次复发前后保留进程 PID／计数和统一日志，比仅重启 GUI 更有判别力。尚未得到可复现、可归因的根因，网络问题仍开放。

## 集成进展

- 原生交接任务包早于最后代码，已核对最新意图并补接有效上下文接口；展示仍保留完整来源段，不重复展示导入前缀。
- 速度变体使用 adapter 命名空间编码，修复普通模型 ID 含分隔符时误解析；清除旧 resume 测试签名。
- 未完成的临时 fork example已移出源码树，保留临时证据供补验。
- workspace/all-targets/all-features Clippy `-D warnings`、remote-only绑定严格 Clippy 已通过。Mac SVC build1、Windows隔离 build1/2、iOS build2、Android build2均已通过；Android首次仅因ANDROID_HOME与ANDROID_SDK_ROOT冲突失败，修正调用环境后复用已生成native库完成assemble＋lint。
- 实际 Pi1.4／Claude2.1.292／Codex0.166-alpha6＋回环上游已确认：Pi compact有效上下文→Claude、Claude→Pi、Codex原生compact→Claude、交接重开不重复、Codex速度模型变体与managed额外参数。Pi fork已过；Codex／Claude补验发现的writer与延迟fork生命周期问题已修复，并逐条只补失败路径：源继续、分支续发／重开、Claude首发换模型、末轮边界封闭、Codex编辑分叉原子首发均通过。
- 原生离屏仅补视觉：宽1100／窄560的composer居中、消息两侧／时间／复制／编辑与fork按钮均目视通过。没有重跑已通过的输入所有权、窗口恢复、队列、Pi continue完整检查，也没有激活窗口或注入输入。
- 已补 `scripts/migrations/00-config-schema-11.py`，迁移runner按文件名排序先转换查询秘密的类型，再运行Codex／composer迁移。合成验证覆盖预览不写、AK/SK与认证修订保留、备份0600、幂等、未知结构拒绝、运行中锁拒绝、缺省查询凭据、runner顺序，以及实际bindings读取新配置；没有操作真实配置。

## 交付主题与证据范围

本次收拢多批交织的源码：composer完整输入归属／持久化与同步、会话控制和重试、窗口／Tab／工作区恢复、消息编辑／排队／元信息、Pi与Codex适配、Artifact与计划任务、提供商账单和模板、协议翻译、速度模型变体、原生上下文交接、fork以及升级工具。它们共享契约与平台调用，不能将按文件拆开的中间状态假称可独立编译；既有任务包仍记录各批已验证事实，本包记录组合结果与本轮新增修复。

原始旧任务的 `/tmp` 证据已随本次机器重启清理；复用包中记录的既有结果，不为恢复截图而全量重跑。本轮证据收拢至 `target/verification/workspace-delivery-20261104`（忽略、不入git），包含构建日志、关键运行时日志／脚本、离屏PNG和网络计数／关机堆栈。

最终共享核心证据：`fork-build-4.log`、`fork-clippy-3.log`；实际fork路径：`fork-codex-pending.log`、`fork-claude-fixed.log`、`edit-codex-fork.log`。Mac Swift完整build2（SVC 3f5d1bef-88c5-43d9-a596-32f7dbaf9718）已通过，随后仅修改Rust内部fork生命周期，未改变公开bindings契约；没有因此重跑全部Swift／视觉检查。Windows build3 已对最终12份修复文件冻结快照逐一SHA256核对并构建通过；未启动UI。工作树及缓存junction曾按后续新任务授权保留复用，现已在后续Windows构建通过后安全清理；日常缓存保留，见后续包的windows-cleanup.log。

本批提交：`d8c3db5`（网络诊断脚本）、`9567b02`（全部交织的工作区功能及关键修复）。随后新增dogfood改进独立记录于[后续任务](../post-dogfood-ux/packet.md)，不混同本批完成状态。

## 尚未改变的边界

- 不重装日常应用；真实配置／composer的显式迁移仍在安装前由用户执行，不以合成验收代替真实迁移。
- 不同完整执行宿主的composer共同保存端设置仍待用户选择与接线；同一执行端与配对控制端的同步已实现。原待决事项不因源码提交而视为已部署。
- 原生交接不支持缺少完整replacement的Codex继承／旧compact／未解析rollback；会明确报错，不丢上下文。ACP／DeepSeek沿用引用交接。
- Claude未首发的分支仍为进程内准备状态，不保证跨应用重启恢复；已首发的原生分支重开已验证。
- Mac为本批多数体验的实现端；Windows仅隔离构建，iOS／Android完成受影响契约构建，不宣称已移植所有Mac体验或做过各端UI验收。
