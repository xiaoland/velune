# Mac 工作区原生外壳

## 当前状态

2026-11-23 用户确认诊断与方向（[S92](../../docs/sources/workspace-keyboard.md#s92)）：先只处理 Mac；工作区窗口外壳改用 AppKit；工具模式收敛为停靠、独立窗口、隐藏，取消吸附浮动面板；最低系统 macOS 14+（现行构建已是 15.0，无需改动）。共享布局模型暂留 Mac 的 Swift 内，Windows 暂不接入，跨平台共享待 Mac 稳定后再议。

2026-11-24 用户授权自主推进与提交，要求在独立分支与工作树中进行；标签过多时改为可滚动（接近 JetBrains、VS Code，而非浏览器的等分压缩）。

- 分支 `feat/workspace-native-shell`，工作树 `tmp/worktrees/workspace-native-shell`（在主检出内、被 `.git/info/exclude` 忽略：开发用 Linux 虚拟机只挂载了主检出目录，惯用的 `Development/.worktrees/` 不可见；在 Mac 上可用 `git worktree move` 移到惯用位置）。基于已提交的 `develop`，主检出中未提交的改动不在其中，合回前需重新对齐。
- 步骤 1 的纯模型部分已完成：见下方[进展](#进展)。
- 不重装日常应用，不读取真实会话；验收用开发实例与合成数据。

## 问题根源

现有症状（分屏／标签动画跳动、停靠区开关回弹、重启后工作区恢复缺失或串窗）来自以下结构，逐个修补无法收敛：

1. **一个窗口有三套布局引擎。** 左区用 `NavigationSplitView`，右区用 `.inspector`，底区和编辑区分屏用 `GeometryReader`＋`HStack`＋`SashOverlay` 自建（`Views.swift` 的 `VeluneRootView`、`Panes.swift` 的 `PaneSplitView`）。分隔条手感、折叠动画、最小尺寸和尺寸归属各不相同；左区 `columnVisibility` 靠 get/set 双向映射回 `ToolWindowLayout`。
2. **视图身份随结构变化。** `PaneSplitView` 的 `ForEach` 以 `\.element.paneIDs` 为身份，子树面板集合一变（拆分、合并、移入标签）就整棵重建；`PaneView` 再以 `.id(tab.id)` 重建内容。标签滑动用等分宽度加 `offset` 模拟，拖动一半走 AppKit `TabHandle`、一半走 SwiftUI `DropDelegate`。
3. **恢复没有单一来源。** `@SceneStorage("windowLayout")`、SceneStorage 中的随机 `windowIdentity`、`WindowWorkspaceJournal` 文件、`RememberedToolLayout`（`mac-tool-layout.json`）与“最近工作区”回退并存；写入分散在 `onChange`、去抖、`willClose`、`willTerminate`、`onDisappear`＋async，并靠 `savedLayoutWritable`／`acceptsWorkspacePersistence`／`workspaceRestored` 防误写（`main.swift` 的 `ConversationWindowContent`）。窗口生命周期和恢复身份由 SwiftUI `WindowGroup` 决定，SceneStorage 何时提交由系统决定，而 Velune 需要确定的身份与布局。
4. **布局状态与活对象混在一起。** `ConversationWindowModel`（`Workspace.swift`）同时持有分屏树、`Pane` 活对象、工具状态、composer 草稿、sheet 与恢复逻辑，操作没有集中的不变量校验。

## 目标架构

### 1. 布局模型：纯值、有限操作、集中校验

新增 `WorkspaceLayout`（值类型，Codable）：

- `grid`：分屏树，节点为 `split(axis, children, fractions)` 或 `group(id)`。
- `groups[id]`：标签 id 的有序列表与选中项。
- `focusedGroup`、`tools`（沿用 `ToolWindowLayout`，去掉 `.floating` 与 `panelFrame`）。
- 标签内容身份（会话 id／未发送新会话／Artifact）单独成表，不放进树。

所有修改只经少数操作：`split`、`insertTab`、`moveTab`（同组、跨组、跨窗口两端各一步）、`closeTab`、`closeGroup`、`resize`、`focus`、`dock`／`undock`／`hide`。每次操作后统一规整：组非空、树中无单子 split、fractions 归一且满足最小尺寸、焦点组存在、选中标签属于该组。

活对象（`ConversationTab`、`ArtifactTab`、转录阅读状态、composer）由窗口的标签注册表按 id 持有，布局只引用 id。视图层只读布局、发出操作，不自持布局状态。操作与规整全部有单元级检查，覆盖今日 `WorkspaceNavigationChecks`／`WorkspaceContinuityChecks` 中的布局断言。

### 2. 持久化与恢复：一个文件、一个身份

- 新文件 `VELUNE_HOME/mac-workspace.json`（schema 1）：窗口清单（窗口 id、种类、设备、frame、全屏、置顶、`WorkspaceLayout`、恢复所需的标签身份与未发送目标），以及“最近工作区”的工具安排。取代 SceneStorage、`mac-window-workspaces/*.json` 与 `mac-tool-layout.json`。
- 窗口 id 由 Velune 生成并写入文件，不再依赖 SceneStorage。
- 系统窗口恢复只承担“重开哪些窗口”：工作区窗口设 `isRestorable` 与 `restorationClass`，`encodeRestorableState` 只编码窗口 id；系统回调时按 id 从文件取布局。这样“退出时关闭窗口”设置、注销时的“重新打开窗口”等系统语义原样保留（PRD：系统决定重开哪些原生窗口）。
- 系统未重开任何工作区窗口时，打开一个默认工作区，按文件中的最近工作区恢复编辑区和工具；会话独立窗口快照不参与这一回退（沿用现行 PRD）。
- 写入由布局模型的变更统一触发：去抖写入，关闭窗口和 `applicationWillTerminate` 同步补写；一个写入入口，不再按视图生命周期猜测。
- 读取失败或未知 schema：显示默认布局、禁用写回并记录诊断，不覆盖原文件（仓库数据约束）。

### 3. 窗口外壳：AppKit 承载、SwiftUI 只做内容

每个工作区窗口一个 `WorkspaceWindowController`（`NSWindowController`），内容为 `NSSplitViewController`：

| 位置 | 实现 |
| --- | --- |
| 左区 | `NSSplitViewItem(sidebarWithViewController:)`：系统侧栏材质、标题栏下全高、系统折叠动画与 ⌃⌘S |
| 右区 | `NSSplitViewItem(inspectorWithViewController:)` |
| 中间 | 竖向 `NSSplitViewController`：编辑区＋底区（底区为可折叠 item，高度即 `bottomHeight`） |
| 区内多个工具 | 区内一个 `NSSplitView`，工具视图按工具 id 复用 |

工具栏改用 `NSToolbar`，侧栏与内容间用 `NSTrackingSeparatorToolbarItem`。会话列表、大纲、composer、转录等内容继续是 SwiftUI，经 `NSHostingController` 嵌入，各自持有稳定实例，区内增删工具不重建它们。

问题、定时任务、已归档、设置等辅助窗口保留 SwiftUI scene，不在本任务范围。

### 4. 编辑区网格

嵌套 `NSSplitView`，由布局树增量同步：节点控制器按组 id 复用，拆分或合并时只插入或移除相邻视图，已有组的视图与转录原样移动，不重建。拖动分隔条时视图直接调整，松开写回 fractions；双击恢复均分。最小组宽高由 `NSSplitView` 的 holding priority 与约束表达，不再手算。

### 5. 组内标签栏

AppKit 自绘 `TabBarView`，行为对齐系统 Tab Bar（Xcode 分屏标签也是自绘）：

- 标签为 `NSView`，滑动与让位用 Core Animation；按下即选中，双击重命名，中键关闭，悬停关闭钮，运行／待答／未读状态沿用现有规则。
- 拖动统一走 `NSDraggingSource`／`NSDraggingDestination`：栏内重排、跨组、跨窗口、拖到所有窗口外成为会话独立窗口；组内容区边缘四分之一拆分，中间并入。会话列表拖入沿用私有类型 `local.velune.conversation-link`。
- 标签过多时横向滚动，不等分压缩：标签有最小与最大宽度，标题在其中截断；滚轮与触控板横向滚动标签栏，选中、新开或拖入的标签自动滚入可见区；栏尾“＋”固定不随滚动。具体宽度在步骤 6 对照系统 Tab Bar 与 Xcode 确定。

### 6. 工具窗口

模式只剩停靠（左／右／底）、独立窗口与隐藏，另有各工具的内建位置（大纲弹出／就地折叠、composer 在各面板中、关联 Agent 与后台任务弹出）。删除 `ToolPanel`、吸附、子窗口跟随、拖到窗口边缘停靠的逻辑与相应检查；独立窗口保留标题栏附件菜单。

旧布局中的 `.floating` 读入时迁移为 `.window`（保留“脱离主窗口”的意图），尺寸取原面板尺寸、位置重新放在主窗口旁。迁移前备份原文件。

## 实施步骤

每步可独立构建与验收，按 [Mac 指引](../../app/mac/AGENTS.md)使用开发实例和私有合成目录。

0. **技术验证。** 在开发实例中验证：SwiftUI `Commands` 的 `FocusedValue` 能否从 AppKit 窗口内的 `NSHostingView` 取得焦点目标，不行则改用响应者链动作；AppKit 窗口内打开设置窗口与 SwiftUI 辅助 scene 的入口；`NSSplitViewController` inspector 项与 SwiftUI 内容的工具栏协作。结论写回本包再进入步骤 4。
1. **布局模型。** 纯值 `WorkspaceLayout` 与操作，独立模块，可在任何平台检查。已完成，见[进展](#进展)。
2. **窗口模型改用布局模型。** `ConversationWindowModel` 以 `WorkspaceLayout` 为唯一布局状态，`Pane` 改为按组 id 呈现的视图模型；外观不变。验收：既有 `--check-*` 布局与连续性检查。
3. **去掉浮动模式。** 删除浮动面板与吸附，迁移 `.floating`。验收：工具位置菜单、独立窗口往返、旧布局迁移。
4. **AppKit 窗口外壳、单文件持久化与恢复。** `WorkspaceWindowController`、`NSSplitViewController` 三区、`NSToolbar`；`mac-workspace.json` 与迁移（从 journal 与 `mac-tool-layout.json` 导入并备份；SceneStorage 中的内容不能脱离 scene 读取，以 journal 为准）；系统窗口恢复只编码 id；移除工作区 `WindowGroup` 与 SceneStorage。持久化与外壳同步进行，因为窗口身份正是由外壳决定。验收：迁移检查（合成旧文件、未知 schema 不覆盖）、退出重开、系统“退出时关闭窗口”两种设置、多窗口、会话独立窗口、全屏中新开窗口。
5. **编辑区网格。** 嵌套 `NSSplitView`。验收：拆分、合并、移动标签时转录不重建（滚动位置保留），分隔条拖动与双击。
6. **标签栏与拖放。** AppKit `TabBarView`、横向滚动与统一拖放。验收：栏内重排动画、溢出滚动、跨组、跨窗口、拖出成窗口、列表拖入、减少动态效果。
7. **清理与文档。** 删除旧 SwiftUI 布局视图、`SplitSash`、`WindowWorkspaceJournal`、`RememberedToolLayout` 及失效检查；改写 [Mac README](../../app/mac/README.md) 的窗口段落。

视觉验收按 [DESIGN.md](../../DESIGN.md) 观察完整区域与状态转换，取得截图证据；不以构建通过代替。

## 进展

### 步骤 1：布局模型

- `VeluneWorkspaceModel`（`app/mac/WorkspaceModel/`）：`PaneLayout`／`PaneSplit`／`PaneEdge`／`PaneDropZone` 从 `Workspace.swift` 移入，编码格式不变；新增 `TabGroup` 与 `WorkspaceLayout`。操作：插入、新组、移除（组随最后一个标签关闭，窗口最后一个标签留给窗口决定）、关闭组、栏内重排、跨组移动、拆分移动、调整与均分比例、保存时排除临时标签。构造、解码与每次操作后布局都是完整的，网格已规整：没有单子 split，同轴嵌套合并并保留各自份额，比例为正且和为一。
- 相对旧行为的差异：`removing` 关闭组后会合并同轴嵌套（旧实现可能留下 `split(h, [split(h, …)])`），`resizing` 拒绝路径或数量不符的比例。
- `app/mac/WorkspaceModelImport.swift` 以 `@_exported import` 让 App 各文件照旧使用这些类型。
- 检查：`swift run velune-workspace-checks`（`app/mac/WorkspaceModelChecks/`）覆盖选择历史、最后一个标签、重排、拆分与关闭、跨组移动、比例、编解码与拒绝不完整数据，另有 200 组各 60 步随机操作的完整性检查。在 Linux 虚拟机用 Swift 6.1.2（`-swift-version 5 -warnings-as-errors`）编译通过，36068 项全部通过，连续三次。
- **尚未在 Mac 编译**：`Package.swift` 新增两个目标，`VeluneMac` 依赖新模块并排除其目录；Linux 上 `swift package dump-package` 解析通过。移除 `Workspace.swift` 中旧类型后的 App 整体编译需在 Mac 上确认。

## Mac 验证途径

开发 Agent 运行在 Mac 宿主的 Linux 虚拟机中：仓库主检出经 sshfs 挂载在同一路径，虚拟机内没有 Xcode、无法运行 AppKit，宿主（网关 `192.168.5.2`）的 SSH 端口当前拒绝连接，`gh` 未登录。纯 Foundation 代码可在虚拟机内用 Linux 版 Swift 编译与检查；凡涉及 AppKit／SwiftUI 的步骤须在 Mac 上验证。

建议途径：用户在 Mac 开启“远程登录”（仅允许本用户），并把虚拟机生成的专用公钥加入 `~/.ssh/authorized_keys`。之后 Agent 经 SSH 在 Mac 上执行：

- `swift run velune-workspace-checks` 与 `svc run build-macos`（在本工作树）。
- 既有 `--check-*` 程序化入口：私有合成目录、本次子进程句柄，遵守 [Mac 指引](../../app/mac/AGENTS.md)的焦点约束。
- 视觉证据：`svc run start-macos-dev` 后用 `screencapture -l <窗口号>` 截取开发实例窗口，不激活、不抢焦点；需要为 SSH 会话授予屏幕录制权限，由用户在系统设置中决定。

## 风险与待定

- 步骤 0 若显示 SwiftUI `Commands` 无法可靠跟随 AppKit 窗口焦点，菜单需改为 AppKit 动作与 `validateMenuItem`，工作量增加，届时向用户说明。
- 现有大量 `--check-*` 启动参数挂在 `ConversationWindowContent.onAppear`，需随外壳迁移改挂到窗口控制器。
- 共享布局模型是否下沉到 Rust 供 Windows 共用，Mac 完成后再决定。
