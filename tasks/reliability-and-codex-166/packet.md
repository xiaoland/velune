# 会话管理、恢复与 Codex 0.166

2026-11-03 用户授权实现：[S68](../../docs/sources.md#s68)。当前实现、受影响关键路径验证、静态检查和交付构建均已完成；没有用户待决事项。日常安装与显式配置迁移交由用户在应用外执行。

约束：保留其他任务修改，不提交、不重装日常应用、不迁移正在使用的配置；无 LiteLLM。真实崩溃报告和关联日志仅作本次只读诊断。开发只用合成 HOME／VELUNE_HOME、隔离 CLI 和回环上游；不读真实会话、认证、账户，不调用真实模型。原生后台／离屏检查，无 computer-use、全局鼠标／键盘注入、激活或切换 Space。分层且仅补受影响关键路径。

## 实现与判断

- 真实 15:29:48 崩溃栈为主线程 objc_msgSend → NSConcreteMapTable probeGC → NSOutlineView rowForItem／disclosure tracking；日志表明 Archive 已在崩溃前完成。已把原生 Section 分组改为平铺组头与会话，显式按钮折叠，去掉这条 disclosure 生命周期。栈支持该机制判断，旧崩溃未被确定性重现；不承诺排除所有 AppKit 崩溃。
- DeviceStore 接收管理请求，设备队列顺序执行；表单受理即关闭，按会话显示等待／执行。前项失败仍执行后项，失败后刷新可能部分生效的结果；管理不绑定 tab generation。重命名成功向打开视图广播原生名称。打开但闲置的会话归档前释放 writer，保留可读历史，取消归档可从所属运行时重新定位。
- 问题面板读取既有 rolling JSONL；新增／恢复／显式清理均写同一 writer。恢复保留历史，清理跨重启有效；旧日志缺少 detail 时明确说明。坏 JSON、坏 UTF-8 单行不隐藏其他记录；日志自身失败在面板说明。沿用 7 文件保留，不建平行错误库、不按启动过滤。
- 模型变化直接保存同一 typed SavedWindowLayout 恢复点；含普通、共享、保留、未首发输入身份、附件和选择，退出 flush。多窗口合并按每份输入修改时间，空状态也保存；单个损坏快照不挡住其他恢复。
- composer 两翼等宽，以正文＋配置＋发送整列居中；宽度不足垂直排列，最低一行不退步。
- 0.166 确有 provider admission breaking change，新增 codex-0.166（>=0.166.0,<0.167.0），保留旧类型 >=0.159.3,<0.164.0。0.166 gateway provider 在子进程启动时注入；跨 provider 用既有逻辑上下文接续，保留可见会话身份和旧原生记录，不覆盖原 provider 或其凭据。原生执行、历史、资源与管理保留新类型身份。显式迁移脚本同时更新实例和已有关联段，未在真实配置执行。
- 原生已读 receipt 随列表和正文交付，UI 回传实际所见版本；原生段身份与 opaque revision 一起校验。0.166 CAS 冲突不盲重试，不清掉未见新结果；逻辑会话使用末段 receipt，原生可用时不写第二份 local mark。Mac、Windows 调用点均传递 receipt；其他运行时保持已核实的 fallback。
- Pi 1.4 公开 RPC 没有无消息继续，AgentSession 也没有完整公开入口；底层 Agent.continue 限制末条类型且绕过 session 生命周期。不用空 prompt 或私有 SDK 伪装支持；这是已核实的能力限制。

## 分层证据

临时脚本、日志、原生检查源和截图统一在 `/tmp/velune-reliability/`：

1. `list.log`：实际 SwiftUI／AppKit 列表末项、末组连续移除 8 轮，主动更新 tracking areas，无崩溃。隐藏窗口不占用桌面；旧代码未确定性复现。
2. `queue.log`：立即接受 3 个独立请求，后台顺序执行；中项失败后末项完成，后续导航未被迟到结果覆盖。临时原生 helper 已移出产品源。
3. `write/read/cleared.log`：三个独立进程；模型的实际变化自动保存，普通会话、未首发、共享和保留输入恢复；输入消费后不复活；问题恢复和显式清理跨进程有效。
4. `visual/geometry.json`：无目标／只有目标／只有附件／两翼都有，520/900/1280/1600pt 与最低高度 8 例，编辑列中心误差均 0，输入可编辑、未越界。实际查看 `both.png` 和 `narrow-minimum.png`；所有 windowVisible=false。日志标签中的 editable lock 沿用旧称，本轮只断言可编辑，未重新断言锁定资格；锁定证据复用前轮。此前已验证的锁定可编辑、cwd、导航速度与窗口限制复用，不重跑。
5. `protocol-check.log`：官方 0.159.3 与校验摘要的隔离官方 0.166 CLI，回环 Responses 上游，托管创建／发送／恢复、native↔managed、原生配置不变、已打开会话归档／取消、read CAS 断言均通过。该脚本最初收尾 shutdown 错传参数导致 exit 1；参数已修正，后续独立 reopen／legacy 脚本正常 shutdown，未重跑整套已通过场景。
6. `protocol-reopen.log`：全新 application 读回 166 逻辑历史，新 terminal result 更新 receipt；旧 receipt 的 read 被拒绝，新结果仍 unread，未写本地第二份已读权威。
7. `legacy-archive.log`：159 在最终目录创建真实合成原生历史；迁移预览不写、执行更新实例＋关联段、重复执行无变动；166 读取并继续该 159 历史，再释放已打开 native writer、原生归档、保留正文、原生取消归档。首次复制整个旧 CODEX_HOME 的实验因 SQLite 仍引用原绝对路径而被拒，改为在最终路径创建 fixture；不是产品迁移故障。
8. `problem-check.log`：四进程检查旧 ERROR 回填、坏记录隔离、resolved 保留、显式 clear 不复活。telemetry 永久回归用例另覆盖断裂 UTF-8，最后定向测试通过（`/tmp/velune-reliability-telemetry-test-final.log`，1 项通过、18 项未运行）。
9. SVC 构建已通过 `7b1ed540-1153-439e-bb9c-5dded69be0c1`、`ea3c78f8-94a8-4808-bc32-5b89054fa9c5`、`fcf40f9c-3fd3-4576-9ddc-89a222006569`；最终构建 `37076cb3-0620-4354-b562-dbe51a4cf724` 通过，包含冷启动空快照修正；中间完整源构建 `88fcd3ca-1d50-4900-984f-fe9fdc75ba63` 也通过。未重装 `/Applications/Velune.app`。
10. 本轮文档 183 个相对链接／锚点检查通过，git diff --check 通过；最终 application／telemetry release all-targets Clippy（warnings-as-errors）和格式检查通过，文档链接与 diff 检查通过。

11. `journal-selection.log`：直接编译实际恢复存储 owner（记录形状替身），检查未知窗口身份恢复最新空快照、已知身份不切换、每份输入时间优先于窗口时间、单个损坏快照隔离；这一层只补此前真实三进程检查没覆盖的选择边界。`migration-lock.log` 验证运行中的真实合成 core 持锁时脚本在写入前拒绝迁移。
12. 自动已读以同一原生会话＋revision 去重，避免列表先于正文更新导致成功确认或 CAS 冲突被再次自动提交；网络等其他失败仍可恢复后重试。Windows 同步了写序号与实际正文版本。

## 交付边界

最终产物 `target/macos/Velune.app`；用户需在应用外重装，再退出应用执行显式实例迁移，见[开发说明](../../docs/development.md#codex-0166-实例迁移)。未替用户安装或修改正在运行的配置。Windows 仅更新共享契约消费者，没有以远程桌面或 Windows App 冒充原生验收。早先全屏 Space／真实鼠标拖放限制仍适用，未重跑或扩大声明。

产品与技术事实已维护到 DESIGN、对应 PRD、Mac／application／agent-runtime README、遥测装配和开发文档。此包只保存当前任务进度和证据。
