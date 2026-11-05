# 任务链实现复核与纠正

## 目标与当前状态

用户要求整理已修正／改进的清单，核查治标不治本与架构腐坏，并纠正。随后明确要求继续收敛已识别的能力投影与缓存依赖。源码实现及关键验收已完成。2026-11-05 用户报告已完成重装，授权提交本任务改动。

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
