# Artifact 与运行时后续改进

## 目标与授权

用户 2026-11-03 要求实现：Mermaid code block 查看及统一 Artifact（sheet／Tab／窗口）；Claude 工作过程折叠；Pi 新建、子代理容量与 steering 故障；Tab 会话菜单和状态；指定 Codex 会话继续失败；大纲跳到用户消息底部；可开关和选模型的自动命名兜底；队列移入会话详情、删除重复说明；全局操作状态移到窗口全局位置；核实原生队列和暂停语义。

已授权诊断与范围内实现，未授权 commit、重装日常应用。保留所有已有修改。不使用 LiteLLM，不操作鼠标、键盘、前台或 Space；原生离屏验收。真实记录只读用户点名的四份会话及必要错误结构，不将其用作开发测试或输出正文／凭据。

## 当前实现与诊断

- Artifact：新增 `ArtifactDocument`（来源＋内容摘要＋源码快照）、`ArtifactTab`、本地 WKWebView renderer；既有 `ConversationTab` 提取共同 `EditorTab` 契约，Pane／跨窗交接接受二者；sheet 拖动、移入 Tab／分屏／窗口、布局持久化已接入，未最终验收。Mermaid 11.12.0 本地固定分发，许可证在 `app/mac/Licenses/Mermaid`；没有网络渲染或图内导航。源码视图的短内容居中已在视觉检查中发现并改为左上对齐。
- Tab 会话菜单：抽取 `ConversationManagementMenu`，列表与 Tab 复用；确认由窗口承载。通用 `EditorTabActions` 复用布局菜单。Tab 增加未读／失败指示，保持已有运行／待答指示；当前聚焦并已阅读时遵循列表消点规则。
- Claude：历史 helper 先前完全不输出 executions。已按原生 `stop_reason`、API error／interruption 补充完成证据；指定真实记录只读诊断得到 4 completed、4 failed，平台继续按核心结果折叠，不将 idle 猜成成功。
- Pi 无工作区：指定记录头 cwd 为 `/`，扩展使用项目 cwd 创建 `.pi`。新建时纳入必须真实 cwd 的运行时，回退到执行 HOME 的 Documents/Velune；原记录未修改。
- Pi 子代理：指定记录确切错误 `bg_delegate route velune/auto reports no usable context-window capacity`／`candidate 1 has no positive context window`。虚拟模型默认容量为 0；现暴露选中物理模型的 contextWindow／maxTokens，未知保持未知。
- Pi steering：`PiSession.busy` 原先只在应用发送时设 true，扩展自行开始／缓冲事件交接后不恢复；现跟随 agent_start，并恢复启动时已经运行的状态。
- 指定 Codex 会话：只读原记录的 2026-11-03T07:51:02.518Z `task_complete.error` 为上游 HTTP 400 `role 'developer' is not allowed`；并非子代理容量。诊断已明确，不篡改会话或重发真实请求；仍需为该会话选择接受 Codex 指令角色的模型服务。
- 队列：移入所属会话详情，删除重复顺序说明。真实日志证明一次“暂停”来自控制连接交接时 `正在检查会话控制权` 被锁存为 SendFailed；现建模为 `QueueWait::Control` 并自动继续。停止／失败仍需用户确认继续，UI 显示实际原因。
- 原生队列：实际 Codex 0.166 `--experimental` schema 有 thread/queue add/list/update/delete/reorder/start；队列只存 input，启动仍由 client 调用，不能保存 Velune 的逐项运行时／模型选择。此次保留统一调度，没有接入原生队列存储；须在交付中明确回答。
- 全局管理进度移到窗口 toolbar 的 `ConversationOperationStatus`。
- 大纲：jump 定位用户气泡底部（排除复制控件），预量目标之前的可见带，避免懒高度随后漂移；尚待原生几何验收。
- 命名：Pi 1.4 SDK 没有自动命名开关；新增按实例可开关、选模型的 Velune 兜底（默认关闭）。后台直接消费 AI 服务，不创建会话 turn／网关／工具；30 秒超时及取消。生成名作为既有组织记录的 generatedTitles 持久化，ConversationTitle 新增 Generated，native 优先。关闭或模型变更丢弃迟到结果。Mac 设置用明确 Toggle／模型 Picker；live title 立即更新列表及 Tab。

## 已有证据与验证边界

- 复用上一批 `reliability-and-codex-166` 无关路径证据，未重复完整验收。
- 定向 cargo check 与 Clippy（application／ai-provider／agent-runtime）通过；后续改动须补最终检查。
- SVC 构建第 3 次 `05d60c6d-da8e-4bc3-a7ba-771bbddc469b`、第 4 次 `e6c2d357-0e99-4638-bfbc-d8c54e83553b` 通过；第 4 次含临时 UI helper，不能交付。第 5 次 `a7f2e932-f8ca-4239-9356-c42ae0a7e5db`、第 6 次 `6581e538-04d2-426e-b3db-96a951463087` 通过；当前最后一次构建正在移除 helper 后交付。
- `/tmp/velune-artifacts/runtime_check.py` 已通过无 cwd Pi 与扩展自行启动后 steering，输出 `/tmp/velune-artifacts-runtime-check.log` 前两条 PASS；后续 fixture 的 RequestProfile 名误用已修正，命名分开验证，不重复已通过的 steering。
- `/tmp/velune-artifacts-naming-behavior.log`：关闭无请求；选中模型一次请求、无新增 turn；跨重启列表保留名称，均 PASS。`name_override.py` 验证原生／用户重命名高于生成名。
- `/tmp/velune-artifacts/pi_capacity.mjs`：用实际 Pi 1.4 createVirtualModel 与已安装 bg_delegate 的纯准入函数，投影 131072／8192 能通过，旧零值拒绝，PASS。
- 合成 Claude helper 生成 outcomes completed／failed／None，PASS；未对真实会话做开发执行。
- `/tmp/velune-artifacts/queue_check.py`：实际隔离 Codex 0.166＋本地假上游，managed 轮次结束后下一条自动发出，未暂停、不重复，PASS（`/tmp/velune-artifacts-queue-check.log`）。
- `/tmp/velune-artifacts/WebkitCheck.swift`：原生隐藏 WKWebView／activation prohibited，中文 flowchart、sequence、语法错误及恶意 directive／外部 img 隔离均 PASS。截图 `/tmp/velune-artifacts/mermaid-native.png` 已目视检查，图表清楚居中。
- 原生整体 helper 只运行隔离 bundle `local.velune.artifact-check`，`VELUNE_HOME`／HOME 也隔离。第一轮 fixture 漏 identities，按真实内部不变量触发 precondition；并非产品运行崩溃。已修复 fixture，以 precondition 代替 release 下无效的 assert，修正透明截屏背景；不得将第一轮结果算作验收通过。

## 交付状态

本批已完成。最终 SVC 构建 `bee6e20a-1645-4c51-9cb1-77226404855a` 成功（日志 `/tmp/velune-artifacts-build-delivery.log`）；产物 `target/macos/Velune.app`。资源与源码逐字节一致，最终二进制不含临时 UI helper flag／成功字符串。未重装日常应用，未 commit。

全部 13 项均有实现或明确诊断／能力核对。两项需在用户汇报中明确：指定 Codex 会话仍需选用接受 developer 指令角色的模型服务；Codex 原生队列存储本次没有接入，保留逐项选择与统一调度。Windows 的新 Artifact／队列与 Tab 呈现未在本批接入或验收，Mac 为本批 UI 交付端。

## 补充验收（2026-11-03）

- `/tmp/velune-artifacts-ui-check.log`：以有效 identities 和 Release 下仍执行的 precondition 验证，所有关键路径 PASS。sheet→Tab 保留对象与源码选择；拖入另一工作区分屏、窗口 handoff、布局 JSON 恢复；会话命令准入；原生代码块点击只为 Mermaid 开放查看，并交付准确源码；大纲历史 u8 与最新 u29 的气泡底部误差均 0。
- 目视检查 `/tmp/velune-artifacts/ui/` 的 sheet 源码、Mermaid 查看按钮、会话队列和窗口操作状态，布局清晰；源码已左上对齐。原生 WKWebView 图表截图单独检查，不把透明缓存的黑背景误认成产品颜色。
- `/tmp/velune-artifacts-input-check.log`：仅补验新增 Artifact 分屏引起的输入所有权转换；添加／移除 Artifact 分屏后 bottom drawer 未发送文本保持连续，PASS。没有重跑上一批完整 composer 验收。
- 文档 12 个知识 owner 相对链接与标题锚点检查通过，git diff --check、cargo fmt 与定向 Clippy 通过。
- Messages 命名协议定向验证暴露了新文本操作缺少 anthropic-version 的实际边界缺陷；已在该操作显式选择 2023-06-01，且拒绝原生非正常终态的部分文本。更新 dylib 后 Messages／Responses 原生请求与结果均 PASS（`/tmp/velune-artifacts-naming-protocols.log`），没有重跑 Chat 的开关／持久化路径。
- 临时 `app/mac/ArtifactChecks.swift` 与 App.init flag 已删除，副本留在 `/tmp/velune-artifacts/`。最终 bundle 已确认不含 helper；日常应用仍未重装。

三种原生协议的命名正常结束结果已通过隔离回环；Messages 显式使用版本 header，Responses 拒绝 incomplete／failed，Chat／Messages 也不把中断输出当成功标题。实际账户／计费请求未执行。工作区其余未提交修改完整保留。
