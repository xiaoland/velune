# 新建会话改为新开 Tab + 恢复 Tab 标题

用户要求（2026-11-04）：新建会话要新建一个 Tab，而不是替换当前 Tab。已授权实现；无提交、无日常重装、不碰真实数据（验证用合成／预览数据）。

同日追加：①会话列表 ↔ Tab 的关系对齐 VSCode File Explorer ↔ Editor（核对结论见下）；②重启恢复的 Tab 在首次点开前显示占位名称——内容可延迟读取，标题不能懒加载（已修：恢复时向未启动的 store 预置会话身份 `noteRestoredConversation`，标题经设备列表取名，已有 device→store 的 objectWillChange 中继负责刷新；顺带修复恢复的后台标签不被“已打开”检查识别、列表单击会重复打开的问题）。

## VSCode 对齐核对结论

已对齐：单击列表就地打开（≈preview 复用）；已打开会话激活已有标签；切换标签列表选中跟随；新建开新标签；删除会话关闭对应标签。差距：切换标签时列表不滚动 reveal 到该会话（VSCode Auto Reveal 行为，未实施，待用户决定是否要）；无 preview tab（斜体、双击固定）概念，属设计差异。

## 方案

Mac 端所有“新建会话”入口（菜单 ⌘N、侧栏工具栏按钮、会话列表组菜单、命令面板及全局快捷键）统一走窗口级 `ConversationWindowModel.createConversation()`：

- 聚焦标签已是未发送的新会话时原地恢复（重定目标），不叠加空标签；全局新建输入只有一份，这是既有设计。
- 否则在当前标签之后新开一个标签承载新会话；草稿在 Tab 启动前创建（`ConversationStore.createConversation` 先验证运行时可用），目标不可用时窗口不变。
- 组菜单 `createConversation(in:)` 同样新开 Tab，运行时／项目主目录作用于新 Tab；组显式运行时同步到新 Tab 的 composer。

## 改动点

- `app/mac/Workspace.swift`：新增 `createConversation()` 与私有 `openNewConversationTab(runtimeID:cwd:)`；改写 `createConversation(in:)`。
- `app/mac/main.swift`：⌘N 菜单改调窗口动作。
- `app/mac/ConversationList.swift`：`NewConversationButton` 改为持有窗口模型并调用窗口动作（5 处使用点）。
- `app/mac/CommandPalette.swift`：`canPerform`/`.newConversation` 适配新语义（不再因当前 store 加载中禁用）。
- `app/mac/ComposerChecks.swift`：`--check-new-tab` 预览断言（新开 Tab、全局输入恢复、组目标、草稿复用、无效运行时无操作）。
- `app/mac/ConversationChecks.swift`：`checkGroupCreation` 按新语义改写（空列表环境走原地复用路径）。
- 文档：`app/mac/README.md`、`docs/prd/composer.md`、`docs/sources.md`。

## 验证

- `scripts/build-macos.sh --build-only`（Swift 警告即错误）。多会话并行 cargo 构建造成锁等待与一次中途死亡；最终只跑 Swift 编译步（未动 Rust，复用现有 dylib/绑定）。
- 运行内置 `--check-new-tab`（预览数据，无需核心配置）。
- 手工合成 VELUNE_HOME（generic-config.json 两个启用运行时 + conversation-organization.json 一个项目）运行 `--check-group-creation`。
- 文档相对链接与 `git diff --check`。

## 进展（已完成）

三项改动均实现并通过验证：

1. **新建会话新开 Tab**：窗口级 `createConversation()`；当前标签已是未发送新会话时原地恢复。入口全覆盖：⌘N、侧栏工具栏、组菜单、命令面板、全局快捷键、`/new`。
2. **恢复 Tab 标题**：`ConversationStore.noteRestoredConversation` 在恢复时预置会话身份；标题经既有 `selectedConversationTitle` 从设备列表取名（device→store 的 objectWillChange 中继保证列表到达后刷新）；顺带修复恢复的后台标签不被“已打开”检查识别。
3. **竞态**：首发消费全局新建输入时立即换绑下一份空输入（`ComposerSync.consumeNewConversationInput`，保留选择）；发送失败换回原输入（其间对新输入的编辑由 dirty 保护优先）。修复在发送中新建空白 Tab 被创建落地夺取的问题。

### 交付证据

- Swift release 构建通过（警告即错误），最终二进制 15:5x 含全部改动。
- `--check-new-tab` PASS：新开标签、原标签不变、全局输入恢复、组目标、无效运行时无操作、草稿原地复用、恢复后台标签身份、发送中新建标签不被夺取。日志 `/tmp/velune-new-tab/check-new-tab.log`（最后一次运行于终端直接输出）。
- `--check-group-creation`（隔离副本 + 合成 VELUNE_HOME `/tmp/velune-new-tab/home`，`open -n -g --env/--stderr`）PASS：“in-place retarget of unsent tab, project primary folder, shared input and unavailable runtime; no turn sent”。该环境会话列表为空，从既有会话新开标签的路径按设计跳过（消息如实反映）；该路径由 `--check-new-tab` 预览断言覆盖。
- `git diff --check` 通过；文档锚点核对通过。

### 竞态修复的根本性复核（用户要求）

判定：根本性修复。依据：

- 错误跟随的唯一入口是窗口 `creations` 处理器（`followComposerCreation` 全仓库仅此调用方），判据为“标签持有被提升的输入身份”。消费时同步轮换绑定（`consumeNewConversationInput` 与发送同在主线程区段，无交错窗口）使该判据从近似变为精确：发送后遇见新建入口的视图只能解析到下一份输入。
- 逐一排查残留路径：queue/steer 不适用于草稿；编辑 fork 不走草稿创建；多轮并发首发由轮换天然串行；跨窗口／跨设备经同一设备级绑定收敛；借用输入由绑定身份守卫排除。
- 失败回绑加“新输入未被触碰”守卫（`restoreConsumedNewConversationInput`），避免顶替用户在 fresh 输入中的新内容；被退回文本至少保留在“保留的输入”。
- 敏感性实证：将 `consumeNewConversationInput` 临时置为 no-op 后 `--check-new-tab` 在竞态断言处崩溃（exit 133，停在 reuse ok 之后）；恢复后 PASS。证明检查对缺陷敏感。

### 追加：常见 Tab 操作（用户第三轮要求）

⌘W／⌘T 为既有默认；新增 ⇧⌘T“重新打开关闭的标签页”（Mac）：窗口按关闭逆序记录 `recentlyClosed`（上限 25），重开时未发送新会话重开为新会话、已删除会话条目跳过、侧聊不入历史。命令注册（KeyboardShortcuts）、菜单项、命令面板、`canPerform` 全链路接入；`--check-new-tab` 覆盖会话恢复／草稿重开／已删除跳过。Windows 端已在第四轮实施并验证（见下节）。

### 过程中的协作事件

- 另一会话在 TranscriptTable.swift 引入的 `MessageEditButton` 用 `#selector(edit)` 撞上 AppKit 的 `NSControl.edit(withFrame:…)`，全树编译中断约 30 分钟；已做最小修复（改名 `editClicked`），属他人文件，需留意对方后续改动。
- 检查副本直接执行时 `UNUserNotificationCenter` 因 dev bundle 无 Info.plist 抛异常崩溃（另一会话 16:27 把 notifier 创建挪进 `DeviceStore.start()` 后暴露）；检查副本补了最小 Info.plist（`/tmp/velune-new-tab/VeluneCheck.app`），产品代码未动。

### 追加二：Windows 端与开发隐患（用户第四轮授权）

- **Windows 重开标签页**：`Command.ReopenClosedTab`（Ctrl+Shift+T）、`Editor.ReopenClosed`／`recentlyClosed`（上限 25、删除跳过、未发送重开为新会话）、`Run`/`CanRun` 接入；同时把 Windows “新建会话”对齐为新开标签（PRD 已平台中立，原为替换当前标签）。win-ws 验证：worktree（872989b7，app/windows 与本地 HEAD 相同）+ 手工提取仅本任务的 patch（WorkspaceWindow.xaml.cs 含他人 hunk 已剥离），`dotnet publish` 通过无警告；worktree 已按文档清理。Windows 无应用内检查 harness，构建即平台检查。
- **dev bundle Info.plist**：build-macos.sh 原在脚本末尾写 Info.plist，cargo 失败后重建的 bundle 缺失它，直接执行副本在 `UNUserNotificationCenter` 崩溃。已把 plist 写入移到 bundle 目录创建之后（脚本前段），并用同内容手动补入当前 bundle 验证：直接执行 `--check-new-tab` 不再崩溃、PASS。
- **他人 Rust 在途改动当前编译不过**（`TurnSettings.service_tier`，velune-application-api E0609），全量 build-macos.sh 受阻；Swift/C# 验证路径不受影响。未代为修复（属他人文件的责任）。

### 已知边界

- 恢复 Tab 标题的端到端（合成窗口里直接看标题）未单独验证：seeding 在真实 transport 设备上断言，标题解析是既有 `selectedConversationTitle` 机制的组合。预览设备因 store 初始化即加载 sample-0 无法测试 seeding。
- 竞态修复的失败回绑路径（发送失败换回原输入）有代码实现与 dirty 保护，未做运行时级故障注入验证。
- 第一次用过的期二进制试跑检查时用默认 home 启动过一次 GUI 进程，120s 超时被杀；未见对 `~/.velune` 的写入迹象。
- 调试期间一次直接执行副本的进程残留曾持锁导致后续运行读到空配置，已清理；合成 home 保留在 /tmp 供复跑。

## 注意

- 工作树有其他任务的在途改动（composer-state-ownership 等），不要回退或提交不相关文件。
- 旧版 `checkGroupCreation` 的 `loadedConversationID != originalID` 断言在草稿 id 固定为 `draft:new-conversation` 后已不可能成立（composer-state 任务改了 id 语义），本次一并改写。
