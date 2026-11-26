# Mac 工作区原生外壳

## 当前状态

2026-11-23 用户确认诊断与方向（[S92](../../docs/sources/workspace-keyboard.md#s92)）：先只处理 Mac；工作区窗口外壳改用 AppKit；工具模式收敛为停靠、独立窗口、隐藏，取消吸附浮动面板；最低系统 macOS 14+（现行构建已是 15.0，无需改动）。共享布局模型暂留 Mac 的 Swift 内，Windows 暂不接入，跨平台共享待 Mac 稳定后再议。

2026-11-26 用户手动检验后接受，外壳已合入本地 `develop`（未推送）。随后用户指出只在空状态下验收过，任务重开（[S95](../../docs/sources/workspace-keyboard.md#s95)）：以日常数据的只读副本在开发实例中验收真实体验，并顺带找出各功能中损坏、不一致、低效或不再需要的部分，按删除、简化、优化、自动化的优先顺序处理，不破坏既有行为与数据。

2026-11-26 真实数据验收的修正经用户接受：先合入 `develop`（含 Codex 0.171 支持），合并后构建上 18 组 Mac 检查通过，再快进合入本地 `develop`（未推送）。下方“发现但未在此处理”中的 Codex 0.171 已由 `develop` 的 `28a5777d3` 处理；首次加载缓慢另有 `perf/codex-active-index-persistence` 在做。

- 分支 `feat/workspace-native-shell`，工作树 `tmp/worktrees/workspace-native-shell`（在主检出内、被 `.git/info/exclude` 忽略：开发用 Linux 虚拟机只挂载了主检出目录）。
- 真实数据只用副本：见下方[真实数据副本](#真实数据副本)。不读取或复制登录、令牌与 API key，不发送消息，不重装日常应用。

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
- Mac 编译（2026-11-24，Xcode 26.2／Swift 6.2.3，macOS 15.4.1）：`swift run velune-workspace-checks` 36068 项全部通过；`swift build --build-system xcode --product VeluneMac` 中 `app/mac` 全部 Swift 文件在 `-warnings-as-errors` 下编译无错误无警告，仅链接因缺少 Rust `uniffi` 库失败（构建副本未编 Rust）。

### 步骤 2：窗口模型改用布局模型

- `ConversationWindowModel` 以 `workspace: WorkspaceLayout` 为网格、组、焦点的唯一状态，`editors` 按标签 id 持有活对象；所有改动经 `changeLayout`，随后修剪不再被引用的编辑器、跟随新标签并同步视图。`layout`／`focusedPaneID`／`isSplit`／`tabCount` 改为由布局派生。
- `Pane` 只是组的视图投影（`tabs`、`selectedID` 由窗口按布局写入），外部不能再直接改动；现有 SwiftUI 视图不变，留待步骤 4、5 替换。原 `Pane.select` 的副作用（被挡下的 composer 焦点转给新显示标签、`activate`）集中在窗口的 `noteShown`。
- 重开已关闭标签时，被移除的组经新增的 `WorkspaceLayout.restoreGroup` 放回原位；工作区之后被重排则在焦点组右侧新建组（行为不变）。
- 原 `Pane` 激活历史检查并入 `velune-workspace-checks`（36075 项通过），窗口级检查改用 `insertTab`。
- Mac 验证（2026-11-24）：`svc run build-macos` 通过；`scripts/check-macos-composer.py experience --workspace-only`、`tabs`、`ownership`、`targets` 全部通过（含关闭标签恢复已移除的分屏组、跨窗口拖放、侧聊拆分移动）。

### 步骤 3：取消浮动工具面板

- `ToolPlace` 去掉 `.floating`，`ToolWindowState` 去掉 `panelFrame`；删除 `ToolPanel`、吸附、子窗口跟随、拖到窗口边缘停靠与 `dockHint` 高亮。`DetachedTools` 只管理独立窗口（标题栏位置菜单、关闭即隐藏、输入区高度跟随内容）。工具栏无按钮时关联 Agent／后台任务改为独立窗口。
- 迁移在 JSON 层完成，模型类型不保留旧格式：`ToolWindowLayout.migratingFloatingTools` 把浮动工具改为 `window`，`panelFrame` 成为 `windowFrame`（附着的面板保留相对主窗口的位置，首次打开后记录为屏幕位置）。`SavedWindowLayout.decodeKnown` 在严格键校验前迁移；日志恢复点改写前备份为 `<id>.json.before-floating-tools.bak` 并保留修改时间；`mac-tool-layout.json` 改写前备份为 `mac-tool-layout.json.before-floating-tools.bak`。SceneStorage 中的旧布局读入时同样迁移，但无法备份（步骤 4 移除 SceneStorage，以日志为准）。
- 检查：`WorkspaceContinuityChecks` 新增合成旧恢复点与旧工具布局的迁移检查（位置、尺寸、备份、严格读回、修改时间），`experience --workspace-only` 22 组全部通过。`check-macos-composer.py sidebar` 失败（共享左区顶部普通按钮 3 个，期望 2 个）；在本分支基点 `8551f93a9` 上同样失败，属既有问题，与本任务无关。
- 视觉验收（2026-11-24，用户授予 `Cage.app` 屏幕录制后）：非预览开发实例、私有空目录 `/tmp/velune-ws-visual2`，向其两个恢复点与 `mac-tool-layout.json` 写入浮动输入区（附着、相对主窗口 dx 160）。启动后三份文件均生成 `.before-floating-tools.bak` 并改写为 `window`；输入区成为 640pt 宽的独立窗口“输入区 – 新会话”，左缘在主窗口左缘右侧 160pt，标题栏右端有位置按钮，主窗口各面板不再显示输入区；前台应用始终是用户的应用，未抢焦点。窗口截图在工作树 `target/checks/workspace-native-shell/step3-*.png`（不入库）。主窗口显示“无法打开会话”是空目录无运行时所致。位置菜单往返需要点按，未做前台操作，由模型与检查覆盖。

### 步骤 0：技术验证结论（2026-11-24，开发实例实测）

- 命令目标不依赖 SwiftUI `FocusedValue`：`WorkspaceWindows.commandTarget` 跟随 AppKit 键窗口通知，`ConversationCommands` 以 `@ObservedObject` 观察它（Commands 已有同样用法）。应用不活跃时 `NSApp.keyWindow` 为 nil，目标随之为空，不影响菜单（菜单只在应用活跃时使用）。
- AppKit 窗口内 `NSHostingController` 承载的内容里，`@Environment(\.openWindow)`／`openSettings` 照常打开“问题”与设置窗口（临时探针实测，未入库）。
- `NSHostingController.sceneBridgingOptions = [.toolbars, .title]` 让内容声明的工具栏与标题成为窗口的；`NavigationSplitView` 侧栏在标题栏下全高，`inspector` 与各工具栏项照常。
- SwiftUI App 生命周期下，AppKit `restorationClass` 恢复照常被调用；剩余 SwiftUI 场景用 `defaultLaunchBehavior(.suppressed)` 与 `handlesExternalEvents(matching: [])`，链接改由 `applicationWillFinishLaunching` 安装的 Apple Event 处理器接收。

### 步骤 4（一）：AppKit 窗口外壳与恢复

- `WorkspaceWindows.swift`：`WorkspaceWindows` 打开、恢复、追踪窗口并处理链接与 Dock 重开；`WorkspaceWindowController` 持有 `ConversationWindowModel`，内容为原 SwiftUI 根视图；`WorkspaceWindowRestoration` 只解码窗口名、种类与设备。移除工作区与会话窗口的 `WindowGroup`、`SceneStorage`（`windowLayout`／`windowIdentity`）及其写入路径，`ConversationWindowContent` 只在出现时启动模型。
- 持久化决定（偏离原方案）：不合并为单一 `mac-workspace.json`。`mac-window-workspaces/<窗口名>.json` 已按窗口 UUID 存放、有备份、严格解码、未知格式只禁用该窗口写入等保障，去掉 SceneStorage 后它就是唯一来源；合并只会再引入一次全量迁移。`mac-tool-layout.json` 仍记最近工作区的工具安排。
- 系统重开的窗口只读自己的恢复点（`start(..., reopened: true)` 不回退到最近快照，避免两个窗口显示同一份布局）。首次以 AppKit 窗口启动且系统设置保留窗口时，从 journal 重开上次退出时一并写入（30 秒内）的窗口，此后由系统恢复。
- 实测（私有目录 `/tmp/velune-ws-shell`，用户系统 `NSQuitAlwaysKeepsWindows` = 1，`macos-dev.py stop` 为正常退出）：首次启动一个工作区、不打开辅助场景；正常退出再启动，恢复同名窗口、不新建恢复点；构造两个同时写入的恢复点并清除系统恢复状态后，迁移重开两个窗口（含保持在最前的那个，浮动层级）；两窗口正常退出再启动，系统恢复两窗口、无新恢复点；`open -g` 送入的会话链接进入已有工作区窗口（显示该会话无法打开并记录问题），未新开窗口。全程前台应用不变。截图在工作树 `target/checks/workspace-native-shell/step4-*.png`（不入库）。
- 检查：`experience --workspace-only` 22 组、`tabs`、`ownership`、`targets` 全部通过。
- 未实测：全屏中新开窗口、Dock 无窗口时重开、键窗口切换时菜单启用状态（均需前台操作）。

### 编辑区网格（原步骤 4）

- `PaneGrid.swift`：嵌套 `NSSplitView`，每组面板是按组保留的 `NSHostingView`（重设 `applicationAppearance()`，它不继承窗口根视图的环境）。形状不变只更新比例；形状变化时先放入新排布再移走旧根，仍在的面板始终留在窗口内。`PaneSplitView` 按布局比例排布（`applyShares`，`adjustSubviews` 会保留现有比例而不用新比例），拖动由 `NSSplitView` 跟踪，松开写回，双击均分，相邻组至少 320pt。删除 SwiftUI `PaneLayoutView`／`PaneSplitView`。
- 检查：`WorkspaceNavigationChecks` 新增网格检查（拆分与关闭后面板视图身份不变、关闭的组移出、比例跟随布局）；检查曾抓到单面板根被移入新分栏后又被移除的缺陷，已修。

### AppKit 标签栏（原步骤 5）

- `TabBar.swift`：`TabBarView`（`NSScrollView` 横向滚动、纵向滚轮与触控板手势接管、无系统滚动条、左右溢出渐变提示，“＋”固定在外并带左侧分隔线），`TabItemView`（自绘明暗对比配色、悬停关闭、运行／等待／未读指示及随窗口键状态刷新、双击就地重命名、中键关闭、右键菜单、沿栏滑动与 Core Animation 让位、离栏转 `NSDraggingSource`）；标签栏是原生拖放目标（会话链接私有类型与 URL）。标签宽度 96–240pt；显示中的标签在切换及变窄后滚入可见，关闭尾部标签钳制滚动偏移。tracking area 只移除自己的（系统工具提示另有 tracking area，旧检查覆盖）。Artifact sheet 的拖动把手复用 `TabItemView`（无栏，不滑动）。删除 `TabHandle`、SwiftUI 标签栏与标签项、`TabDropDelegate`。面板内容区的边缘拆分投放仍是 SwiftUI `PaneDropDelegate`。
- 检查：新增标签栏溢出检查（可滚动、显示中标签可见、宽度上下限、未达上限的标题完整）；首版宽度未计标签文本框内边距导致截断，检查截图发现后已修。`experience --workspace-only` 24 组、`tabs`、`ownership`、`targets` 通过；开发实例窗口截图确认标签栏与空白处底色。

### 三区停靠与 AppKit 工具栏

- `EditorColumn.swift`：编辑区与底区同处一个 `NSSplitView`（`BottomSplitView`），底区开合不重建面板，窗口缩放时底区保持高度，拖动松开写回、双击恢复默认。区内两个及以上工具放入 `ToolSplit`（复用 `PaneSplitView`，最小 120pt），工具视图按种类保留；只有一个工具时直接是区的内容（列表独占边栏时需要它的搜索框在边栏顶部）。删除 `SplitSash`／`SashOverlay`／`WeightedSplit`／`BottomAreaView`。
- `WindowShell.swift`：`WorkspaceShell` 把窗口内容换成 `NSSplitViewController`（边栏项、编辑列、检查器项），工具栏换成 AppKit `NSToolbar`（`WorkspaceToolbar`）。两区收起与工具布局双向同步：布局变化时（启动恢复期间不动画）设置 `isCollapsed`，KVO 把系统的显示／隐藏边栏与拖动收起写回布局。拖动分隔松开时宽度记入 `ToolWindowLayout.leftWidth/rightWidth`（可选字段，旧文件照常读入），打开时按其恢复。工具栏按钮是 `.toolbar` 样式 `NSButton`，条件项用 `isHidden`（macOS 15+），弹出面板是 `NSPopover`；右键菜单沿用窗口模型的锚点注册。删除 `ConversationToolbarPartition`、`ConversationListSidebarToolbar` 与 SwiftUI 工具栏声明；列表独占边栏时用自己的 `SearchField` 代替 `.searchable(placement: .sidebar)`。⌘R 刷新移入“会话”菜单。macOS 26 工具栏标识改为 `conversation-window`，旧 SwiftUI 工具栏的自定排列不沿用（本机为 macOS 15，没有）。
- `sidebar-toolbar` 检查改写为针对新外壳：边栏跟踪分隔绑定外壳分栏、列表动作在其前且位于边栏内、共享与右区时隐藏、显示／隐藏边栏写回布局、保存的宽度恢复且窗口缩放时保持。原检查在 `develop` 上已失败（`aa42506c2` 改了列表动作归属而未改检查）。

### 边栏与停靠区

已按用户 2026-11-25 的决定收敛（[S93](../../docs/sources/workspace-keyboard.md#s93)）：三区按 Xcode 的导航器／检查器／调试区理解，任一工具仍可放任一区。会话列表在任何位置形态一致，顶部一行是自己的搜索、筛选与新建会话；工具栏的边栏部分只留边栏开关，列表的两个工具栏按钮与“侧栏工具栏”设置一并删去。可见性只剩区是否打开与工具在哪个区：区在有工具可显示时才算打开，隐藏区内最后一个工具即收起该区（工具留在区内，重开带回），工具都移走的区即为收起。程序化检查见 `sidebar-toolbar` 与 `experience --workspace-only` 的 dock visibility。

### 待做与未实测

- 2026-11-25 用户实测：新建标签再关闭即闪退。原因是网格与编辑列的 `NSSplitView` 以自身为代理，响应链询问（`respondsToSelector:`）被转回自身而栈溢出；此前的检查只驱动模型与视图身份，没有经过真实窗口的响应链。已改为独立代理，并加 `workspaceShellChecks`（真实外壳中新建与关闭标签，并对每个视图询问常见动作）；未修复时该检查即崩溃。教训：外壳改动的验收要包含真实窗口中的基本命令路径，不能只看模型与布局。

- 2026-11-25 用户实测：边栏开合卡顿掉帧。离屏量得（调试构建）：展开时重建列表约 20–68ms，收起约 10–19ms，工具状态任何变化都使整个窗口的 SwiftUI 重算约 5–10ms；系统的显示／隐藏边栏先开始动画、再由 KVO 改工具，这些工作正落在动画帧里。已改为收起的区保留视图、开合先改工具并在视图更新提交后才动画（`shell-motion` 检查）。同日授权 agent-desktop 后在开发实例真实窗口中点按边栏按钮六次并以 Time Profiler（`xctrace --attach`）采样：每帧整窗排版中输入区因 `onGeometryChange` 写入宽度状态而每帧重算 body（77 个样本），标签栏每帧重新测量标题；改为布局给宽度、缓存标签宽度后，同样六次开合中超过一帧的主线程连续占用由 8 段降到 3 段，且剩余 3 段都在点按处理本身（动画开始前的状态更新），输入区 body 77→22、宿主视图排版 281→179 样本。Animation Hitches 模板在此宿主上只得 4 个 CPU 样本、hitch 时间戳无效，不作依据。
- 停靠区与区内工具的交互手感（边栏／检查器开合动画、分隔拖动、宽度记忆）需前台验收；见下“三区停靠”。
- 需前台操作、未实测：分隔条拖动与双击、标签滑动重排与拖出成窗口、跨窗口拖放、列表拖入、重命名、键窗口切换时菜单启用、全屏中新开窗口、Dock 无窗口时重开。程序化检查覆盖了模型与视图身份，交互手感需用户或前台验收。
- 2026-11-26 用户手动检验后外壳已合入本地 `develop`（未推送）；真实数据验收的修正在本分支继续，验收后再合。

## 真实数据副本

开发实例目录 `~/.velune-dev/rd6d0f`（`macos-dev.py start --home` 创建，本工作树 scope）。工作树中未入库的 `target/real-data/copy.py` 以 APFS 克隆（`cp -c`）复制日常 `~/.velune` 的布局、草稿、整理、链接、分析与恢复点，以及 Codex、Claude Code、Pi 的原生会话、索引与模型设置，原文件只读：

- 不复制：远控设备身份与配对、订阅登录、运行时登录目录、遥测配置、日志与备份；各 Harness 的 `auth.json`、MCP 配置（含内联 API key）、Codex `config.toml`；Claude `settings.json` 去掉 `env`。
- 配置改写：提供商认证保留种类但清空（API key 为空、订阅为未登录），账单与额度查询关闭；运行时实例的 `agentDir` 指向副本；ACP 实例停用。
- 路径改写：Codex 线程索引 `state_5.sqlite` 的 4161 条 `rollout_path`、Velune 状态与 Pi 会话头中的原生路径改指副本，避免开发实例寻址真实文件。
- 复制后核验：副本中不含任何真实 API key 值，Velune 文件与线程索引不含真实原生路径。
- Codex：ChatGPT.app 自带的 CLI 已是 0.171.0-alpha.13，超出 `codex-0.166` 的兼容区间；副本改用 npm 安装到 `target/real-data/codex-0.170` 的官方 0.170.0。

### 真实数据验收（2026-11-26）

在[真实数据副本](#真实数据副本)上以 agent-desktop 驱动真实窗口、按窗口号截图，并以 Time Profiler 附着采样。采样时用键盘输入：agent-desktop 的快照与按引用点按会遍历辅助功能树，SwiftUI 列表为此物化全部行，单次约 1 秒，会污染测量。

已修正（`ccf50fbee`、`d9be1a410` 及其后）：

- 窗口：新窗口沿用同类窗口上次的帧，首次沿用升级前 SwiftUI 自动保存的帧（日常数据为 1364×834）；从恢复点重开的多个窗口依次层叠。此前升级后所有窗口以 1060×760 叠在屏幕中央。边栏、检查器未拖动过时按 260／280 打开（日常数据此前为 265），不再取 190 的最小宽度。
- 菜单：应用未声明本地化，系统菜单与窗口控件为英文，与应用项混排；改为声明简体中文。系统“关闭”“全部关闭”与“关闭标签页／关闭窗口”重复，去掉后在键窗口切换、设置窗口开关后复核未再出现。辅助窗口在“显示”和“窗口”两处重复，只留“窗口”；后退／前进移到“会话”；九个编号标签项收进子菜单；“关联 Agent位置”补空格。
- 无障碍：工具栏全部按钮都是开关，后退、刷新等被读作复选框；会话行不以标题为名。
- 渲染：紧贴中文标点的 `**` 按 CommonMark 不能开闭，真实会话中原样显示星号或加粗错位；改为解析前在星号与相邻非 ASCII 标点间插入零宽空格。
- 显示：状态栏上下文用量“521.433K”改为三位有效数字；共享输入在单个会话时不再占一栏写“发往”；“问题”窗口以核心操作名（`bindings.runtime_models`）为标题，改为可读名称；读取失败说明先写原因，诊断编号放到括号里；未配置 API key 的提供商在列表中注明。
- 性能（真实窗口采样）：输入绑定判断为二次复杂度，菜单与输入区随窗口变化反复调用，打开会话时占主线程约 1.9／11.1 秒，改为单遍后降到约 0.04 秒；会话列表随窗口的任何变化重建并比对全部行，改为只跟随自身状态；停靠区滑开时消息列表不再按中间宽度重排。边栏开合每次超过一帧的主线程占用由约 5.5 段降到约 2.4 段，最长 74→33ms。

- 窗口命令：菜单“新建窗口”不带种子，按启动规则取最近工作区的恢复点，复制了已打开窗口的分屏与标签（命令面板的同名命令带种子，不复制）；改为同样带种子。SceneStorage 时代按 tab 身份回退的交接恢复在 AppKit 窗口下已不可达，连同检查删去。
- 启动加载重试：启动时历史尚未就绪导致的暂态失败（`unavailable`／`connection`）前 4 次自动重试（约 18 秒）期间显示“正在加载会话…”，不再短暂闪现“无法打开会话”与诊断编号；超时或需用户处理的失败仍显示错误与“重试”按钮（程序化检查见 `experience --workspace-only`；启动截图在 `target/checks/workspace-native-shell/quiet-retry-*.png`，不入库，该次启动首轮即打开，未触发重试分支）。
- 输入区运行时名称：启动后运行时配置尚未到达前，输入区执行配置按钮不再回退显示内部运行时标识（如 `model_record_…`），加载中仅显示模型名并在菜单中注明“正在加载…”，真正缺失时显示“未知运行时”（截图在 `target/checks/workspace-native-shell/runtime-label-*.png`，不入库）。
- 设置窗口焦点：已在 `GeneralSettingsView` 禁用默认焦点并在出现后清除初始选中，不再自动选中第一个文本框（“每次加载”）；已编译通过，未在前台打开设置窗口截图复核。
- 标签栏滚动条与界面边界梳理（`TabBar.swift`、`Panes.swift`、`EditorColumn.swift`、`WindowShell.swift`、`Composer.swift`）：
  - 移除标签栏的 `NSScrollView` 系统横向滚动条（`hasHorizontalScroller = false`），避免系统设置“始终显示滚动条”或外接鼠标触发 `.legacy` 滚动条时以 15pt 不透明白底盖住 28pt 标签标题；在 `TabScrollView` 中同时接管横向手势与纵向滚轮的横向滚动，并在左右溢出端加入不拦截指针与拖放的 `TabEdgeFadeView` 渐变提示；“＋”左侧补分隔线，随界面缩放更新字号。
  - 修复标签栏与外壳的其余边界问题：关闭尾部标签或加宽标签栏时钳制滚动位置不再留出末尾空白；窗口或标签栏变窄后仍保持当前选中标签滚入可见；按住拖动标签随自动滚动时按标签带坐标计算位移避免漂移；窗口激活/失焦时刷新标签未读指示；`TabPaint` 与 `TabShade` 改用语义分隔色及深色模式对比参数；`EditorColumn` 在底区不含输入区时不再误用输入区最小高度；`WindowShell` 在超窄窗口同时展开两侧停靠区时按最小宽度分配保存宽度，并消除 `NSToolbarItem` 初始零尺寸布局警告；`PaneView`／`TabContentView` 加裁剪，输入区在极窄分屏下可横向滚动控制行，防止输入区边框与下拉越过分割线溢入相邻面板。
  - 边界与视觉复核：`WorkspaceNavigationChecks` 与 `SidebarToolbarChecks` 补充系统滚动条禁用、裁剪高度、两端渐变、关闭/变窄钳制、底区多工具拆分及 `640×480` 窄分屏不溢出断言；`svc run build-macos`、`sidebar-toolbar`（30 组布局，明暗无警告）、`experience --workspace-only`（27 项；与前序渲染检查连续运行时首次曾遇冷启动队列 25s 超时，独立复跑通过）、`tabs`、`appearance-zoom` 与 `shell-motion` 全部通过；已通过图片查看全部 9 张离屏渲染截图（`target/checks/macos-experience/tabbar-overflow-{light,dark,zoom150}.png`、`target/checks/macos-sidebar-toolbar/shell-{tabs-overflow,bottom-dock,narrow-edge}-{light,dark}.png`，不入库）。
- 实测补充：配置了运行时后 ⌘N 与 ⇧⌘N 均可用，⇧⌘N 打开 720×760 的独立会话窗口并层叠；新窗口沿用升级前 SwiftUI 的 1364×834 帧。按键自动化曾在副本输入区留下一个“/”，已删除（副本，非日常数据）。
- `experience --workspace-only` 在与完整 `experience` 及运行中的真实数据实例并行时，冷启动打开两个来源超过 25 秒一次；在真实数据开发实例运行期间单独执行也会触发该超时，停止开发实例后复跑通过，属负载下的时限。

发现但未在此处理：

- 首次加载缓慢：真实数据规模下每次启动需约 8–11 秒才显示会话列表，原因是启动时全量读取全部 4161 条 Codex 线程，Pi 与 Claude Code 列表在其后等待；需要在核心运行时跨启动持久化索引新鲜度，属核心运行时工作，另行处理。
- Codex：日常配置的 ChatGPT.app 自带 CLI 为 0.171.0-alpha.13，`develop` 的 `codex-0.166` 区间到 <0.171（`7bad70f60`），日常应用会读不到 Codex 历史；运行时设置页对此没有提示，只在“问题”中出现。属运行时适配，另行处理。
- Pi 模型目录在副本中超时（副本未带 `npm` 与扩展，属副本缺口，非产品问题）。
- 核心错误文本含英文前缀与内部标识（`invalid application operation:`、`id=model_record_…`），属核心的错误呈现。
- 辅助功能树：真实列表有数百行，agent-desktop 按名称或标识查找在到达工具栏前耗尽预算；SwiftUI 列表应答辅助功能时物化全部行。
- 边栏开合剩余的超帧主要是开合处理本身（列表与外壳的布局、SwiftUI 事务），在动画开始前。

## Mac 验证途径

开发 Agent 运行在 Mac 宿主的 Linux 虚拟机中：仓库主检出经 sshfs 挂载在同一路径，虚拟机内没有 Xcode、无法运行 AppKit，`gh` 未登录。纯 Foundation 代码可在虚拟机内用 Linux 版 Swift 编译与检查；凡涉及 AppKit／SwiftUI 的步骤须在 Mac 上验证。

现有通道：

- 宿主命令工具（以用户账户在其登录会话中执行，可直接访问本工作树、`svc` 与 cargo）。构建：`CARGO_HOME=/Volumes/WorkSSD/.dev-cache/cargo svc run build-macos`（不设则 `uniffi_bindgen` 编译失败）；新工作树需先把主检出的 `target/runtime-support` 复制过来或运行 `scripts/install-runtime-support.sh`。程序化检查：`python3 scripts/check-macos-composer.py <check>`，在后台副本与私有临时目录中运行。
- `ssh cage-mac`（独立账户 `cage-runner`）：无外置卷访问、无图形会话，仅适合把源码副本放到 `~/velune-ws` 做纯编译。

视觉证据：`scripts/macos-dev.py start [--preview] --home <私有临时目录>`（即 `svc run start-macos-dev` 的入口，`svc run` 不转发参数；目录须由启动器创建，可先启动再停止后写入合成文件），按开发实例 pid 用 `CGWindowListCopyWindowInfo` 取窗口号，再 `screencapture -x -o -l <窗口号>` 只截该窗口。不要截取屏幕区域，那会带入用户其他应用的内容。预览模式不读写 `mac-tool-layout.json` 与恢复点。

## 风险与待定

- 步骤 0 若显示 SwiftUI `Commands` 无法可靠跟随 AppKit 窗口焦点，菜单需改为 AppKit 动作与 `validateMenuItem`，工作量增加，届时向用户说明。
- 现有大量 `--check-*` 启动参数挂在 `ConversationWindowContent.onAppear`，需随外壳迁移改挂到窗口控制器。
- 共享布局模型是否下沉到 Rust 供 Windows 共用，Mac 完成后再决定。
