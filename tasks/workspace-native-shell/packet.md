# Mac 工作区原生外壳

## 当前状态

2026-11-23 用户确认诊断与方向（[S92](../../docs/sources/workspace-keyboard.md#s92)）：先只处理 Mac；工作区窗口外壳改用 AppKit；工具模式收敛为停靠、独立窗口、隐藏，取消吸附浮动面板；最低系统 macOS 14+（现行构建已是 15.0，无需改动）。共享布局模型暂留 Mac 的 Swift 内，Windows 暂不接入，跨平台共享待 Mac 稳定后再议。

`feat/workspace-native-shell` 已抽出独立的布局模型，模型检查在 Linux 和 Mac 上通过。下一步是将窗口模型接入该布局模型。AppKit 外壳及其运行和视觉验收仍待完成。

下文保留设计基线，当前实施顺序和验证结果见该分支的同名任务包。产品规则见 [PRD](../../docs/prd/workspace-layout.md#窗口系统)。验收使用开发实例与合成数据。

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
- 标签溢出时横向滚动。选中、新开或拖入的标签会自动滚入可见区，具体宽度留待实现时核定。

### 6. 工具窗口

模式只剩停靠（左／右／底）、独立窗口与隐藏，另有各工具的内建位置（大纲弹出／就地折叠、composer 在各面板中、关联 Agent 与后台任务弹出）。删除 `ToolPanel`、吸附、子窗口跟随、拖到窗口边缘停靠的逻辑与相应检查；独立窗口保留标题栏附件菜单。

旧布局中的 `.floating` 读入时迁移为 `.window`（保留“脱离主窗口”的意图），尺寸取原面板尺寸、位置重新放在主窗口旁。迁移前备份原文件。

## 实施步骤

每步可独立构建与验收，按 [Mac 指引](../../app/mac/AGENTS.md)使用开发实例和私有合成目录。

0. **技术验证。** 在开发实例中验证：SwiftUI `Commands` 的 `FocusedValue` 能否从 AppKit 窗口内的 `NSHostingView` 取得焦点目标，不行则改用响应者链动作；AppKit 窗口内打开设置窗口与 SwiftUI 辅助 scene 的入口；`NSSplitViewController` inspector 项与 SwiftUI 内容的工具栏协作。结论写回本包再进入步骤 3。
1. **布局模型与单文件持久化。** 抽出 `WorkspaceLayout` 与操作，`ConversationWindowModel` 改为持有它；新文件与迁移（从 journal 与 `mac-tool-layout.json` 导入并备份，SceneStorage 中的内容不可脱离 scene 读取，以 journal 为准）。外观暂不变。验收：布局操作检查、迁移检查（合成旧文件、未知 schema 不覆盖）、新进程恢复。
2. **去掉浮动模式。** 删除浮动面板与吸附，迁移 `.floating`。验收：工具位置菜单、独立窗口往返、旧布局迁移。
3. **AppKit 窗口外壳与恢复。** `WorkspaceWindowController`、`NSSplitViewController` 三区、`NSToolbar`、系统窗口恢复只编码 id；移除工作区 `WindowGroup` 与 SceneStorage。验收：退出重开、系统“退出时关闭窗口”两种设置、多窗口、会话独立窗口、全屏中新开窗口。
4. **编辑区网格。** 嵌套 `NSSplitView`。验收：拆分、合并、移动标签时转录不重建（滚动位置保留），分隔条拖动与双击。
5. **标签栏与拖放。** AppKit `TabBarView` 与统一拖放。验收：栏内重排动画、跨组、跨窗口、拖出成窗口、列表拖入、减少动态效果。
6. **清理与文档。** 删除旧 SwiftUI 布局视图、`SplitSash`、`WindowWorkspaceJournal`、`RememberedToolLayout` 及失效检查；改写 [Mac README](../../app/mac/README.md) 的窗口段落。

视觉验收按 [DESIGN.md](../../DESIGN.md) 观察完整区域与状态转换，取得截图证据；不以构建通过代替。

## 风险与待定

- 步骤 0 若显示 SwiftUI `Commands` 无法可靠跟随 AppKit 窗口焦点，菜单需改为 AppKit 动作与 `validateMenuItem`，工作量增加，届时向用户说明。
- 现有大量 `--check-*` 启动参数挂在 `ConversationWindowContent.onAppear`，需随外壳迁移改挂到窗口控制器。
- 共享布局模型是否下沉到 Rust 供 Windows 共用，Mac 完成后再决定。
