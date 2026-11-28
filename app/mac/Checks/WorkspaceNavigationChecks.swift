#if VELUNE_DEV_CHECKS
import AppKit
import Combine
import SwiftUI

/// Real window and pane models, using only synthetic preview conversations.
@MainActor func workspaceNavigationChecks(home: URL) {
    func note(_ text: String) { FileHandle.standardError.write(Data((text + "\n").utf8)) }
    workspaceKeyboardPaneFocusChecks()
    let device = DeviceStore(preview: true)
    // Per-group activation history is checked with the layout model: `swift run velune-workspace-checks`.
    do {
        // The grid's panes are native views kept per group: never ordered on screen.
        let model = ConversationWindowModel(application: device, kind: .workspace)
        model.start(restoring: nil, seed: WindowSeed(conversationID: "sample-0"))
        let grid = PaneGridView(window: model, previewEmpty: false)
        let host = NSWindow(contentRect: NSRect(x: 0, y: 0, width: 1200, height: 800), styleMask: [.titled], backing: .buffered, defer: true)
        host.contentView = grid
        func apply() { grid.apply(model.layout, groups: Set(model.panes.keys)); grid.layoutSubtreeIfNeeded() }
        apply()
        let first = model.focusedPaneID
        guard let firstView = grid.paneView(first) else { fatalError("缺少第一组的面板视图") }
        model.split(.horizontal)
        apply()
        let second = model.focusedPaneID
        precondition(grid.paneView(first) === firstView && firstView.window === host, "拆分不重建已有组的面板视图")
        model.split(.vertical)
        apply()
        guard let secondView = grid.paneView(second) else { fatalError("缺少第二组的面板视图") }
        model.closePane(first)
        apply()
        precondition(grid.paneView(second) === secondView && secondView.window === host && grid.paneView(first) == nil && firstView.superview == nil,
                     "关闭一组只移走它自己的视图")
        model.resize(at: [], fractions: [3, 1])
        apply()
        guard let root = grid.subviews.first as? PaneSplitView, root.subviews.count == 2 else { fatalError("网格根应为两项的分栏") }
        // The first group closed: the two left are stacked.
        let heights = root.subviews.map(\.frame.height)
        precondition(!root.isVertical && abs(heights[0] / (heights[0] + heights[1]) - 0.75) < 0.01, "分栏按布局比例排布：\(heights)")
        precondition(grid.paneView(second) === secondView, "调整比例不重建面板视图")
        host.contentView = nil
        for tab in model.allTabs { tab.close() }
        model.list.close()
        note("PASS pane grid: group views survive splits and closes, closed groups leave, shares follow the layout")
    }
    do {
        // Visibility is whether an area is open and which area a tool is in: an open area always shows a tool.
        var tools = ToolWindowLayout()
        precondition(tools.isOpen(.left) && !tools.isOpen(.right) && !tools.isOpen(.bottom), "默认只有左区打开")
        tools.hide(.list)
        precondition(!tools.isOpen(.left) && tools[.list].shown && tools[.list].place == .docked(.left), "隐藏区内最后一个工具即收起该区，工具留在区内")
        tools.toggleArea(.left)
        precondition(tools.tools(in: .left) == [.list], "重开该区带回它的工具")
        tools.move(.outline, to: .docked(.left))
        tools.hide(.outline)
        precondition(tools.tools(in: .left) == [.list] && tools.isOpen(.left), "区内还有其他工具时只移走被隐藏的那个")
        tools.move(.list, to: .docked(.right))
        precondition(!tools.isOpen(.left) && tools.tools(in: .right) == [.list], "工具移走后空出的区收起")
        tools.toggleArea(.left)
        precondition(tools.tools(in: .left) == [.outline], "重开空出的区带回留在那里的工具")
        note("PASS dock visibility: an open area always shows a tool, hiding its last tool or moving it away closes the area")
    }
    do {
        // The bottom area opens and closes beside the same panes, keeps its height as the window resizes, and its tools keep their views.
        let model = ConversationWindowModel(application: device, kind: .workspace)
        model.start(restoring: nil, seed: WindowSeed(conversationID: "sample-0"))
        let column = EditorColumnView(window: model, previewEmpty: false)
        let host = NSWindow(contentRect: NSRect(x: 0, y: 0, width: 1200, height: 800), styleMask: [.titled], backing: .buffered, defer: true)
        host.contentView = column
        func apply() {
            column.grid.apply(model.layout, groups: Set(model.panes.keys))
            column.apply(bottom: !model.tools.tools(in: .bottom).isEmpty, height: model.tools.bottomHeight, minimum: 100)
            column.layoutSubtreeIfNeeded()
        }
        apply()
        guard let pane = column.grid.paneView(model.focusedPaneID) else { fatalError("缺少面板视图") }
        precondition(column.bottomArea == nil, "默认没有底区")
        model.tools.bottomHeight = 240
        model.tools.move(.outline, to: .docked(.bottom))
        apply()
        guard let bottom = column.bottomArea else { fatalError("底区应打开") }
        precondition(abs(bottom.frame.height - 240) < 1 && column.grid.paneView(model.focusedPaneID) === pane && pane.window === host,
                     "底区按保存的高度打开，面板视图不变：\(bottom.frame.height)")
        host.setContentSize(NSSize(width: 1200, height: 600))
        column.layoutSubtreeIfNeeded()
        precondition(abs(bottom.frame.height - 240) < 1, "窗口变矮时底区保持高度，面板让出空间：\(bottom.frame.height)")
        model.tools.move(.backgroundTasks, to: .docked(.bottom))
        apply()
        @MainActor func toolSplit(_ view: NSView) -> ToolSplitView? {
            if let split = view as? ToolSplitView { return split }
            for child in view.subviews { if let found = toolSplit(child) { return found } }
            return nil
        }
        RunLoop.main.run(until: Date().addingTimeInterval(0.1))
        column.layoutSubtreeIfNeeded()
        guard let tools = toolSplit(bottom), let split = tools.splitView, split.subviews.count == 2, split.isVertical else { fatalError("底区两个工具应并排于原生分栏") }
        let outline = split.subviews[0]
        model.tools.move(.agents, to: .docked(.bottom))
        apply()
        RunLoop.main.run(until: Date().addingTimeInterval(0.1))
        precondition(tools.splitView?.subviews.count == 3 && tools.splitView?.subviews.first === outline, "加入工具不重建已在的工具视图")
        model.tools.hide(.outline); model.tools.hide(.agents); model.tools.hide(.backgroundTasks)
        apply()
        precondition(column.bottomArea == nil && column.grid.paneView(model.focusedPaneID) === pane, "底区收起后面板视图不变")
        host.contentView = nil
        for tab in model.allTabs { tab.close() }
        model.list.close()
        note("PASS editor column: bottom area beside the same panes, keeps its height, tools keep their views")
    }

    let window = ConversationWindowModel(application: device, kind: .workspace)
    window.start(restoring: nil, seed: WindowSeed(conversationID: "sample-0"))
    let first = window.focused.id
    let originalTab = window.focused
    let ordinaryDraft = window.composer(for: originalTab)
    ordinaryDraft.restore("保留普通输入", attachments: [])
    guard let quickOpen = window.paletteItems(.conversations).first(where: { $0.id == "sample-1" }) else { fatalError("缺少合成会话候选") }
    quickOpen.run(false)
    precondition(window.tabCount == 2 && originalTab.store.loadedConversationID == "sample-0"
                 && ordinaryDraft.submission == "保留普通输入" && window.focused.store.device === device,
                 "快速打开默认新Tab并保留来源及输入和执行设备")
    quickOpen.run(true)
    precondition(window.tabCount == 2, "修饰键确认已有目标也复用同设备Tab")
    let middle = window.focused.id
    window.openInNewTab("sample-2")
    let last = window.focused.id
    window.selectTab(at: 1)
    precondition(window.focused.id == first, "序号从当前组左侧开始")
    window.selectTab(at: 9)
    precondition(window.focused.id == first, "不存在的序号不改选择")
    window.perform(.selectTab3)
    precondition(window.focused.id == last, "命令定位组内第3个标签")
    precondition(!window.canPerform(.selectTab9))
    window.openInNewTab("sample-0", device: device)
    precondition(window.tabCount == 3 && window.focused.id == first, "已打开目标复用标签")
    note("PASS indexed tab commands: left-to-right, missing slot no-op, open target reused")
    window.selectTab(first)
    window.closeTab(middle)
    window.selectTab(last)
    window.reopenClosedTab()
    precondition(window.focusedPane.tabs.map(\.id) == [first, middle, last], "关闭恢复应放回原索引并保留标签身份")
    precondition(window.focused.id == middle && window.focused.store.loadedConversationID == "sample-1")
    window.split(.vertical)
    let splitPane = window.focusedPaneID
    let blank = window.focused.id
    let splitLayout = window.layout
    window.selectTab(at: 1)
    precondition(window.focused.id == blank, "序号只作用于当前组")
    window.focusNeighbour(forward: false)
    precondition(window.focusedPaneID != splitPane, "上一个组切换聚焦")
    window.focusNeighbour(forward: true)
    precondition(window.focusedPaneID == splitPane, "下一个组恢复聚焦")
    window.closeTab(blank)
    precondition(!window.isSplit)
    window.reopenClosedTab()
    precondition(window.layout == splitLayout && window.focusedPaneID == splitPane && window.focused.id == blank,
                 "恢复最后一个组内标签时恢复组的位置和空标签身份")
    note("PASS closed tab recovery: original index, stable identity, removed split group")

    let lone = ConversationWindowModel(application: device, kind: .workspace)
    lone.start(restoring: nil, seed: WindowSeed(conversationID: "sample-0"))
    final class CloseWindow: NSWindow {
        var closeCount = 0
        override func close() { closeCount += 1 }
    }
    let native = CloseWindow(contentRect: NSRect(x: 0, y: 0, width: 600, height: 400), styleMask: [.titled], backing: .buffered, defer: false)
    native.isReleasedWhenClosed = false
    let anchor = NSView()
    native.contentView = anchor
    lone.register(lone.focusedPaneID, anchor: { anchor })
    lone.closeTab(lone.focused.id)
    precondition(native.closeCount == 0 && lone.tabCount == 1 && lone.focused.isBlank,
                 "关闭最后文档应留一个空白标签")
    lone.closeTab(lone.focused.id)
    precondition(native.closeCount == 1, "关闭最后空白标签才关闭窗口")
    note("PASS final tab: document → blank → window close")

    let handle = TabItemView(tab: lone.focused, bar: nil, model: lone)
    handle.frame = NSRect(x: 0, y: 0, width: 110, height: 28)
    anchor.addSubview(handle)
    handle.toolTip = "一个超出标签宽度的完整会话标题"
    let external = NSTrackingArea(rect: handle.bounds, options: [.mouseEnteredAndExited, .activeAlways], owner: native)
    handle.addTrackingArea(external)
    handle.updateTrackingAreas()
    handle.updateTrackingAreas()
    precondition(handle.trackingAreas.contains { $0 === external }, "更新 hover 不删除系统 tooltip 等其他 owner 的 tracking area")
    precondition(handle.trackingAreas.contains { $0 !== external && ($0.owner as AnyObject?) !== handle },
                 "原生 tooltip manager 的 tracking area 必须保留")
    precondition(handle.toolTip == "一个超出标签宽度的完整会话标题")
    note("PASS tab tooltip: full title and foreign tracking survive hover updates")

    do {
        // Tabs past the bar's width scroll without a scrollbar; the shown one is brought into view; titles that fit are whole.
        let model = ConversationWindowModel(application: device, kind: .workspace)
        model.start(restoring: nil, seed: WindowSeed(conversationID: "sample-0"))
        model.openInNewTab("sample-1")
        model.openInNewTab("sample-2")
        model.insertTab(ArtifactTab(document: .mermaid(source: "flowchart LR; A-->B", conversationID: nil, sourceID: "tabbar-check")))
        for _ in 0..<8 { model.openInNewTab() }
        let bar = TabBarView(window: model, pane: model.focusedPane)
        let host = NSWindow(contentRect: NSRect(x: 0, y: 0, width: 420, height: 28), styleMask: [.titled], backing: .buffered, defer: true)
        host.isReleasedWhenClosed = false
        host.contentView = bar
        @MainActor func settleBar() {
            bar.update()
            bar.layoutSubtreeIfNeeded()
            RunLoop.main.run(until: Date().addingTimeInterval(0.08))
            bar.layoutSubtreeIfNeeded()
        }
        settleBar()
        func descendants(_ view: NSView) -> [NSView] { [view] + view.subviews.flatMap(descendants) }
        guard let scroll = bar.subviews.compactMap({ $0 as? TabScrollView }).first, let strip = scroll.documentView else { fatalError("标签栏缺少滚动视图") }
        precondition(!scroll.hasHorizontalScroller && !scroll.hasVerticalScroller, "标签栏不得启用系统滚动条")
        precondition(descendants(bar).compactMap { $0 as? NSScroller }.allSatisfy { $0.isHidden || $0.frame.isEmpty },
                     "标签栏不得绘制可见 NSScroller（避免常驻滚动条遮挡标签标题）")
        precondition(abs(scroll.contentSize.height - bar.bounds.height) < 0.5 && abs(strip.frame.height - bar.bounds.height) < 0.5,
                     "滚动区与标签带高度必须占满整个标签栏")
        let items = strip.subviews.compactMap { $0 as? TabItemView }
        precondition(items.count == 12 && strip.frame.width > scroll.contentSize.width, "超出栏宽的标签可滚动：\(strip.frame.width) / \(scroll.contentSize.width)")
        precondition(items.allSatisfy { $0.frame.width >= TabBarView.minimumTabWidth && $0.frame.width <= TabBarView.maximumTabWidth }, "标签宽度在上下限之内")
        guard let shown = items.first(where: { $0.tab.id == model.focusedPane.selectedID }) else { fatalError("缺少显示中的标签") }
        precondition(scroll.contentView.bounds.contains(NSRect(x: shown.frame.minX, y: 0, width: shown.frame.width, height: 1)), "显示中的标签滚入可见范围")
        for item in items where item.frame.width < TabBarView.maximumTabWidth {
            let title = (item.tab.title as NSString).size(withAttributes: [.font: NSFont.systemFont(ofSize: 11 * AppearancePreferences.shared.values.uiScale)]).width
            precondition(item.frame.width - 2 * TabItemView.titleInset >= title, "未达上限的标签标题完整：\(item.tab.title)")
        }

        // Edge fades track scroll position at start, middle and end.
        guard let leadingFade = bar.subviews.first(where: { $0.identifier?.rawValue == "tab-bar-leading-fade" }),
              let trailingFade = bar.subviews.first(where: { $0.identifier?.rawValue == "tab-bar-trailing-fade" }) else {
            fatalError("标签栏缺少溢出渐变提示视图")
        }
        scroll.scrollHorizontally(by: -10_000)
        settleBar()
        precondition(scroll.contentView.bounds.origin.x <= 0.5 && leadingFade.isHidden && !trailingFade.isHidden, "滚到最左端时只显示右侧溢出渐变")
        scroll.scrollHorizontally(by: 120)
        settleBar()
        precondition(!leadingFade.isHidden && !trailingFade.isHidden, "滚到中间时两侧均显示溢出渐变")
        scroll.scrollHorizontally(by: 10_000)
        settleBar()
        precondition(!leadingFade.isHidden && trailingFade.isHidden, "滚到最右端时只显示左侧溢出渐变")

        @MainActor func captureBar(_ name: String) {
            settleBar()
            bar.displayIfNeeded()
            guard let bitmap = bar.bitmapImageRepForCachingDisplay(in: bar.bounds) else { fatalError("无法绘制标签栏：\(name)") }
            bar.cacheDisplay(in: bar.bounds, to: bitmap)
            try! bitmap.representation(using: .png, properties: [:])!.write(to: home.appendingPathComponent(name + ".png"))
        }

        // Visual renders: mid-scroll in Light, Dark, and 150% Zoom.
        let savedAppearance = NSApp.appearance
        let savedPreferences = AppearancePreferences.shared.values
        scroll.scrollHorizontally(by: -120)
        NSApp.appearance = NSAppearance(named: .aqua)
        host.appearance = NSApp.appearance
        captureBar("tabbar-overflow-light")
        NSApp.appearance = NSAppearance(named: .darkAqua)
        host.appearance = NSApp.appearance
        captureBar("tabbar-overflow-dark")
        NSApp.appearance = NSAppearance(named: .aqua)
        host.appearance = NSApp.appearance
        var zoomed = savedPreferences
        zoomed.zoomPercent = 150
        AppearancePreferences.shared.values = zoomed
        host.setContentSize(NSSize(width: 420, height: PaneChrome.barHeight(for: zoomed)))
        RunLoop.main.run(until: Date().addingTimeInterval(0.08))
        captureBar("tabbar-overflow-zoom150")
        AppearancePreferences.shared.values = savedPreferences
        NSApp.appearance = savedAppearance
        host.appearance = nil
        host.setContentSize(NSSize(width: 420, height: 28))
        RunLoop.main.run(until: Date().addingTimeInterval(0.08))
        settleBar()

        // Closing trailing tabs clamps clip origin so no blank trailing gap remains.
        scroll.scrollHorizontally(by: 10_000)
        settleBar()
        for tab in model.focusedPane.tabs.suffix(8) { model.closeTab(tab.id) }
        settleBar()
        precondition(scroll.contentView.bounds.maxX <= strip.frame.width + 0.5,
                     "关闭尾部标签后滚动位置应贴合末尾，不得留出空白：\(scroll.contentView.bounds.maxX) vs \(strip.frame.width)")

        // Narrowing the bar keeps the selected tab visible.
        for _ in 0..<8 { model.openInNewTab() }
        host.setContentSize(NSSize(width: 560, height: 28))
        settleBar()
        let lastTab = model.focusedPane.tabs.last!
        model.selectTab(lastTab.id)
        settleBar()
        host.setContentSize(NSSize(width: 300, height: 28))
        settleBar()
        guard let shownNarrow = strip.subviews.compactMap({ $0 as? TabItemView }).first(where: { $0.tab.id == lastTab.id }) else {
            fatalError("变窄后缺少选中标签")
        }
        precondition(scroll.contentView.bounds.intersects(shownNarrow.frame),
                     "窗口变窄后仍应保持当前选中标签可见")

        host.contentView = nil
        for tab in model.allTabs { tab.close() }
        model.list.close()
        note("PASS tab bar: no system scroller, edge fades track scroll, overflow scrolls & clamps on close/narrow, light/dark/150% renders captured")
    }

    precondition(ConversationListClicks.openDelay(pressedAt: 10, now: 10.1, interval: 0.5) > 0.39)
    precondition(ConversationListClicks.openDelay(pressedAt: 10, now: 10.6, interval: 0.5) == 0,
                 "原生 tracking 已等待完整双击间隔时不再追加等待")
    let start = ProcessInfo.processInfo.systemUptime
    for _ in 0..<20 {
        window.open("sample-0", device: device)
        window.open("sample-1", device: device)
    }
    let elapsed = ProcessInfo.processInfo.systemUptime - start
    precondition(window.focused.store.loadedConversationID == "sample-1")
    note(String(format: "PASS existing-tab navigation: 40 synthetic switches in %.3f s; double-click delay counted once from native event", elapsed))

    let aggregated = AggregatedListModel(local: device, hub: nil)
    var successes: [(String, Set<String>)] = []
    let watch = aggregated.managedConversations.sink { successes.append(($0.deviceID, $0.conversations)) }
    device.manageConversations(source: "合成成功", ids: ["sample-0"], operation: { true }, apply: { _ in })
    let deadline = Date().addingTimeInterval(3)
    while successes.isEmpty && Date() < deadline { RunLoop.main.run(until: Date().addingTimeInterval(0.01)) }
    precondition(successes.count == 1 && successes[0].0 == "local" && successes[0].1 == ["sample-0"])
    let selected: Set<AggregatedRowID> = [
        .init(section: "local‖all", conversation: "sample-0"),
        .init(section: "local‖all", conversation: "sample-1"),
        .init(section: "peer:synthetic‖all", conversation: "sample-0"),
    ]
    let remaining = selected.filter { !$0.belongs(to: successes[0].0, conversations: successes[0].1) }
    precondition(remaining.count == 2, "完成整理只清除同设备受处理项，保留其他选择")
    withExtendedLifetime(watch) {}
    note("PASS organization completion: successful, device-qualified items clear independently")
    conversationManagementCancellationChecks()
    conversationManagementVisualChecks(home: home)
}

/// Actual AppKit responders and production composer coordinators: model-only
/// group selection cannot prove where the next typed character will go.
@MainActor private func workspaceKeyboardPaneFocusChecks() {
    func wait(_ condition: () -> Bool) {
        let deadline = Date().addingTimeInterval(3)
        while !condition(), Date() < deadline { RunLoop.main.run(until: Date().addingTimeInterval(0.01)) }
        precondition(condition(), "等待原生组输入焦点超时")
    }
    let local = DeviceStore(preview: true)
    let peer = DeviceStore(preview: true, peerID: "synthetic-keyboard-peer")
    let workspace = ConversationWindowModel(application: local, kind: .workspace)
    workspace.tools.move(.composer, to: .builtIn)
    workspace.start(restoring: nil, seed: WindowSeed(conversationID: "sample-0"))
    let leftID = workspace.focusedPaneID, leftTab = workspace.focused
    workspace.split(.vertical)
    workspace.openInNewTab("sample-1", device: peer)
    let rightID = workspace.focusedPaneID, rightTab = workspace.focused
    let left = workspace.composer(for: leftTab), right = workspace.composer(for: rightTab)
    left.restoreSavedDraft(.init(text: "left draft", attachments: []))
    right.restoreSavedDraft(.init(text: "right draft", attachments: []))
    let native = NSWindow(contentRect: NSRect(x: 0, y: 0, width: 900, height: 180), styleMask: [.borderless], backing: .buffered, defer: false)
    native.isReleasedWhenClosed = false
    let host = NSStackView()
    host.orientation = .horizontal
    host.distribution = .fillEqually
    native.contentView = host
    let leftHost = NSHostingView(rootView: ComposerEditor(model: left, placeholder: "", submit: {}, steer: {}, focused: { workspace.focusConversation(leftTab) }))
    let rightHost = NSHostingView(rootView: ComposerEditor(model: right, placeholder: "", submit: {}, steer: {}, focused: { workspace.focusConversation(rightTab) }))
    host.addArrangedSubview(leftHost); host.addArrangedSubview(rightHost)
    workspace.register(leftID, anchor: { leftHost })
    workspace.register(rightID, anchor: { rightHost })
    host.layoutSubtreeIfNeeded(); wait { left.textView != nil && right.textView != nil }
    guard let leftView = left.textView, let rightView = right.textView else { fatalError("合成双组原生输入框未挂载") }
    precondition(native.makeFirstResponder(rightView))
    workspace.focusNeighbour(forward: false)
    wait { native.firstResponder === leftView }
    precondition(workspace.focusedPaneID == leftID && native.firstResponder === leftView,
                 "上一个组必须同时转移AppKit输入焦点，不能只改变命令目标")
    leftView.setSelectedRange(NSRange(location: (leftView.string as NSString).length, length: 0))
    (native.firstResponder as! NSTextView).insertText(" typed-left", replacementRange: leftView.selectedRange())
    precondition(left.submission == "left draft typed-left" && right.submission == "right draft",
                 "切组后真实输入只改变目的设备会话的composer")
    workspace.focusNeighbour(forward: true)
    wait { native.firstResponder === rightView }
    precondition(workspace.focusedPaneID == rightID && native.firstResponder === rightView)
    rightView.setSelectedRange(NSRange(location: (rightView.string as NSString).length, length: 0))
    (native.firstResponder as! NSTextView).insertText(" typed-right", replacementRange: rightView.selectedRange())
    precondition(right.submission == "right draft typed-right" && left.submission == "left draft typed-left")
    workspace.focus(leftID)
    precondition(native.firstResponder === rightView, "鼠标驱动的组选择不能抢走所点控件的first responder")
    let target = rightHost.convert(NSPoint(x: rightHost.bounds.midX, y: rightHost.bounds.midY), to: nil)
    func press(_ type: NSEvent.EventType, modifiers: NSEvent.ModifierFlags = []) -> NSEvent {
        NSEvent.mouseEvent(with: type, location: target, modifierFlags: modifiers,
                           timestamp: ProcessInfo.processInfo.systemUptime, windowNumber: native.windowNumber,
                           context: nil, eventNumber: 1, clickCount: 1, pressure: 1)!
    }
    for event in [press(.rightMouseDown), press(.leftMouseDown, modifiers: .control), press(.otherMouseDown)] {
        precondition(workspace.handleMouseDown(event) === event)
        precondition(workspace.focusedPaneID == leftID && workspace.focused === leftTab,
                     "打开后台组的上下文菜单或中键关闭不能激活该组")
        precondition(workspace.composer(for: leftTab) === left && workspace.composer(for: rightTab) === right
                     && left.submission == "left draft typed-left" && right.submission == "right draft typed-right",
                     "非激活操作不能切换或覆盖输入状态")
    }
    _ = workspace.handleMouseDown(press(.leftMouseDown))
    precondition(workspace.focusedPaneID == rightID, "同位置的普通左键仍聚焦目标组，避免检查仅因未命中而通过")
    workspace.focus(leftID)
    FileHandle.standardError.write(Data("PASS contextual pane press: right/control/middle preserve active tab and draft ownership; primary click still focuses target\n".utf8))
    let artifact = ArtifactTab(document: .mermaid(source: "flowchart LR; A-->B", conversationID: nil, sourceID: "keyboard-artifact"))
    workspace.insertTab(artifact)
    workspace.focus(rightID)
    workspace.focusNeighbour(forward: false)
    precondition(workspace.focusedEditor === artifact && native.firstResponder !== rightView,
                 "只读Artifact组不能继续把文字写到上一组的composer")
    workspace.register(leftID, anchor: { nil })
    workspace.register(rightID, anchor: { nil })
    native.contentView = nil
    FileHandle.standardError.write(Data("PASS keyboard pane focus: native first responder and inserted text follow destination; peer input remains independent; pointer focus unchanged; Artifact clears stale editor\n".utf8))
}

@MainActor private func conversationManagementVisualChecks(home: URL) {
    let local = DeviceStore(preview: true)
    let peer = DeviceStore(preview: true, peerID: "synthetic-queue-peer")
    let aggregated = AggregatedListModel(local: local, hub: nil)
    aggregated.updateDevices([
        .init(peerID: nil, store: local, name: "本机合成设备"),
        .init(peerID: "synthetic-queue-peer", store: peer, name: "远端合成设备"),
    ])
    let localStarted = DispatchSemaphore(value: 0), releaseLocal = DispatchSemaphore(value: 0)
    let peerStarted = DispatchSemaphore(value: 0), releasePeer = DispatchSemaphore(value: 0)
    local.manageConversations(source: "重命名会话", ids: ["running"], operation: {
        localStarted.signal(); _ = releaseLocal.wait(timeout: .now() + 8); return true
    }, apply: { _ in })
    peer.performOnQueue { peerStarted.signal(); _ = releasePeer.wait(timeout: .now() + 8) }
    precondition(localStarted.wait(timeout: .now() + 3) == .success)
    precondition(peerStarted.wait(timeout: .now() + 3) == .success)
    peer.manageConversations(source: "归档会话", ids: ["waiting"], operation: { () -> Bool in
        preconditionFailure("通过原生取消按钮取消的排队项不能执行")
    }, apply: { _ in })
    let activeID = local.conversationManagement[0].id, waitingID = peer.conversationManagement[0].id
    let content = ConversationOperationDetails(aggregated: aggregated).applicationAppearance()
        .frame(width: 420, height: 140).background(Color(nsColor: .windowBackgroundColor))
    let view = NSHostingView(rootView: content)
    let native = NSWindow(contentRect: NSRect(x: 0, y: 0, width: 420, height: 140), styleMask: [.borderless], backing: .buffered, defer: false)
    native.isReleasedWhenClosed = false
    native.contentView = view
    view.layoutSubtreeIfNeeded()
    RunLoop.main.run(until: Date().addingTimeInterval(0.2))
    view.layoutSubtreeIfNeeded(); view.displayIfNeeded()
    guard let bitmap = view.bitmapImageRepForCachingDisplay(in: view.bounds) else { fatalError("无法绘制整理队列") }
    view.cacheDisplay(in: view.bounds, to: bitmap)
    try! bitmap.representation(using: .png, properties: [:])!.write(to: home.appendingPathComponent("management-queue.png"))
    func button(_ node: Any, id: String) -> (enabled: Bool, press: () -> Bool)? {
        var visited = Set<ObjectIdentifier>()
        func find(_ node: Any) -> (enabled: Bool, press: () -> Bool)? {
            guard let object = node as? NSObject, visited.insert(ObjectIdentifier(object)).inserted else { return nil }
            if let element = node as? NSAccessibilityProtocol {
                if element.accessibilityIdentifier() == id { return (element.isAccessibilityEnabled(), { element.accessibilityPerformPress() }) }
                for child in element.accessibilityChildren() ?? [] { if let found = find(child) { return found } }
            }
            if let view = node as? NSView {
                for child in view.subviews { if let found = find(child) { return found } }
            }
            return nil
        }
        return find(node)
    }
    if button(view, id: "cancel-management-\(activeID)") != nil { fatalError("执行项不能显示取消按钮") }
    func descendants(_ node: NSView) -> [NSView] { [node] + node.subviews.flatMap(descendants) }
    // SwiftUI draws this label in a child view; NSButton.title is empty.
    // This fixture has exactly one waiting row, hence one native button.
    let nativeButtons = descendants(view).compactMap { $0 as? NSButton }
    let nativeCancel: (enabled: Bool, press: () -> Bool)? = nativeButtons.count == 1 ? nativeButtons.first.map { control in
        (control.isEnabled, { control.performClick(nil); return true })
    } : nil
    guard let cancel = button(view, id: "cancel-management-\(waitingID)") ?? nativeCancel else {
        for node in descendants(view) {
            FileHandle.standardError.write(Data("UI node \(type(of: node)) id=\(node.accessibilityIdentifier()) label=\(String(describing: node.accessibilityLabel()))\n".utf8))
        }
        fatalError("等待项缺少可操作的原生取消按钮")
    }
    precondition(cancel.enabled && cancel.press(), "等待项原生取消按钮必须可操作")
    precondition(peer.conversationManagement.isEmpty, "原生按钮必须取消对应远端设备队列")
    precondition(!native.isVisible && !native.isKeyWindow && !NSApp.isActive)
    releasePeer.signal(); releaseLocal.signal()
    RunLoop.main.run(until: Date().addingTimeInterval(0.1))
    native.contentView = nil
    FileHandle.standardError.write(Data("PASS management native UI: waiting cancel enabled and actionable, running cancel absent, devices explicit, hidden rendered image\n".utf8))
}

@MainActor private func conversationManagementCancellationChecks() {
    func wait(_ ready: () -> Bool) {
        let deadline = Date().addingTimeInterval(3)
        while !ready() && Date() < deadline { RunLoop.main.run(until: Date().addingTimeInterval(0.01)) }
        precondition(ready(), "整理操作合成队列超时")
    }
    final class Effects: @unchecked Sendable {
        let lock = NSLock()
        private var performed: [String] = []
        func append(_ value: String) { lock.withLock { performed.append(value) } }
        var values: [String] { lock.withLock { performed } }
    }
    let device = DeviceStore(preview: true)
    let effects = Effects()
    let started = DispatchSemaphore(value: 0)
    let release = DispatchSemaphore(value: 0)
    var successes: [Set<String>] = []
    let watch = device.managedConversations.sink { successes.append($0) }
    device.manageConversations(source: "正在执行", ids: ["active"], operation: {
        started.signal()
        _ = release.wait(timeout: .now() + 3)
        effects.append("active")
        return true
    }, apply: { _ in })
    // Pause main-loop delivery so UI still says waiting while the native
    // operation has already crossed its admission boundary.
    precondition(started.wait(timeout: .now() + 3) == .success)
    let active = device.conversationManagement[0].id
    precondition(!device.cancelConversationManagement(active), "已执行但UI回报尚未到达时不能假取消")
    precondition(device.conversationManagement[0].running)
    device.manageConversations(source: "将取消", ids: ["cancelled"], operation: {
        effects.append("cancelled")
        return true
    }, apply: { _ in preconditionFailure("取消项不得应用结果") })
    let cancelled = device.conversationManagement.last!.id
    precondition(device.cancelConversationManagement(cancelled))
    precondition(!device.cancelConversationManagement(cancelled))
    device.manageConversations(source: "后续项", ids: ["next"], operation: {
        effects.append("next")
        return true
    }, apply: { _ in })
    release.signal()
    wait { device.conversationManagement.isEmpty }
    precondition(effects.values == ["active", "next"], "取消等待项不得执行原生修改，也不得阻止后续操作")
    precondition(successes == [["active"], ["next"]], "取消不发送完成事件，列表保留取消项选择")
    device.manageConversations(source: "部分删除", ids: ["deleted", "failed"], deleting: true,
                               operation: { Set(["deleted"]) }, completedIDs: { $0 }, apply: { _ in })
    wait { device.conversationManagement.isEmpty }
    precondition(successes.last == ["deleted"], "部分删除成功不能清除失败项选择")
    withExtendedLifetime(watch) {}
    FileHandle.standardError.write(Data("PASS management cancellation: waiting effects skipped, active race rejected, later work proceeds, cancelled selection retained\n".utf8))
}

/// The whole window shell, as a new tab opens and closes. Run after the
/// timed checks: a full window's views keep the main thread busy a while.
@MainActor func workspaceShellChecks() {
    func note(_ text: String) { FileHandle.standardError.write(Data((text + "\n").utf8)) }
    let device = DeviceStore(preview: true)
    // The whole window shell, as a new tab opens and closes. Commands and
    // menu validation ask every responder up the chain whether it handles
    // an action; a split view that is its own delegate answers by asking
    // itself again, without end.
    let model = ConversationWindowModel(application: device, kind: .workspace)
    model.start(restoring: nil, seed: WindowSeed(conversationID: "sample-0"))
    let root = VeluneRootView(window: model, store: model.list, tab: model.focused,
                              focused: model.focused.store, previewEmpty: false, previewSettings: false)
    let host = NSWindow(contentRect: NSRect(x: 0, y: 0, width: 1200, height: 800),
                        styleMask: [.titled, .closable, .resizable, .fullSizeContentView], backing: .buffered, defer: true)
    host.isReleasedWhenClosed = false; host.isRestorable = false
    let shell = WorkspaceShell(window: host, model: model, content: root, previewEmpty: false)
    model.tools.move(.outline, to: .docked(.bottom))
    func settle() { for _ in 0..<5 { shell.split.view.layoutSubtreeIfNeeded(); RunLoop.main.run(until: Date().addingTimeInterval(0.05)) } }
    settle()
    func descendants(_ view: NSView) -> [NSView] { [view] + view.subviews.flatMap(descendants) }
    let actions = ["performClose:", "toggleSidebar:", "undo:", "velune_unhandledAction:"].map(NSSelectorFromString)
    func askResponders() -> Int {
        var asked = 0
        for view in descendants(host.contentView!) {
            for action in actions { _ = view.responds(to: action) }
            asked += 1
        }
        return asked
    }
    precondition(askResponders() > 0)
    let tabs = model.tabCount
    model.openInNewTab()
    settle()
    precondition(model.tabCount == tabs + 1, "新建标签")
    _ = askResponders()
    model.closeTab(model.focusedEditor.id)
    settle()
    precondition(model.tabCount == tabs, "关闭新建的标签")
    let asked = askResponders()
    // 在新窗口中新建会话 asks for a conversation window on the focused
    // tab's device, with no conversation: that window starts a new one.
    var requested: [(ConversationWindowKind, WindowSeed)] = []
    model.openWindow = { requested.append(($0, $1)) }
    precondition(device.enabledRuntimeInstances.isEmpty || model.canPerform(.newConversationWindow), "有运行时时可在新窗口中新建会话")
    model.perform(.newConversationWindow)
    if !device.enabledRuntimeInstances.isEmpty {
        precondition(requested.count == 1 && requested[0].0 == .single && requested[0].1.conversationID == nil
                     && requested[0].1.device == nil && requested[0].1.tab == nil, "在新窗口中新建会话：本机的会话窗口，不带会话")
        let single = ConversationWindowModel(application: device, kind: .single)
        single.start(restoring: nil, seed: requested[0].1)
        settle()
        precondition(single.focused.store.isUnsentConversation, "新的会话窗口以未发送的新会话开始")
        for tab in single.allTabs { tab.close() }
        single.list.close()
    }
    precondition(descendants(host.contentView!).allSatisfy { ($0 as? NSSplitView).map { $0.delegate !== $0 } ?? true },
                 "分栏视图不能是自己的代理：AppKit 会把响应链询问转给代理而无限递归")
    // Nothing of this window outlives it into the checks that follow.
    host.toolbar = nil
    host.contentViewController = nil
    host.close()
    for tab in model.allTabs { tab.close() }
    model.list.close()
    RunLoop.main.run(until: Date().addingTimeInterval(0.1))
    withExtendedLifetime(shell) {}
    note("PASS window shell: a new tab opens and closes; \(asked) views answer responder-chain queries without recursion; a new conversation window is asked for (\(requested.count))")
}
#endif
