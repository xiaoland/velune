#if VELUNE_DEV_CHECKS
import AppKit
import SwiftUI
import ObjectiveC
import Vision

@MainActor private final class SidebarToolbarCheckWindow: NSWindow {
    override var canBecomeKey: Bool { false }
    override var canBecomeMain: Bool { false }
    private func refusePresentation() -> Never {
        FileHandle.standardError.write(Data("FAIL sidebar toolbar: presentation requested\n".utf8)); exit(1)
    }
    override func makeKey() { refusePresentation() }
    override func makeMain() { refusePresentation() }
    override func makeKeyAndOrderFront(_ sender: Any?) { refusePresentation() }
    override func orderFront(_ sender: Any?) { refusePresentation() }
    override func orderBack(_ sender: Any?) { refusePresentation() }
    override func orderFrontRegardless() { refusePresentation() }
    override func order(_ place: NSWindow.OrderingMode, relativeTo otherWin: Int) {
        if place != .out { refusePresentation() }
        super.order(place, relativeTo: otherWin)
    }
}

/// Hosts the production split view and native titlebar without presenting a window.
@MainActor func sidebarToolbarChecks() -> Never {
    func fail(_ message: String) -> Never {
        FileHandle.standardError.write(Data("FAIL sidebar toolbar: \(message)\n".utf8)); exit(1)
    }
    func require(_ value: @autoclosure () -> Bool, _ message: String) {
        if !value() { fail(message) }
    }
    guard let path = ProcessInfo.processInfo.environment["VELUNE_CHECK_OUTPUT"] else { fail("missing private output") }
    let directory = URL(fileURLWithPath: path).standardizedFileURL.resolvingSymlinksInPath()
    require(directory.path.hasPrefix("/private/tmp/") || directory.path.hasPrefix("/tmp/"), "output must be private temporary data")
    _ = NSApplication.shared
    if NSApp.activationPolicy() != .prohibited { _ = NSApp.setActivationPolicy(.prohibited) }
    require(NSApp.activationPolicy() == .prohibited, "activation policy must actually prohibit activation")
    var activations = 0, keys = 0, mains = 0, systemActivations = 0
    let frontmost = NSWorkspace.shared.frontmostApplication?.processIdentifier
    let active = NotificationCenter.default.addObserver(forName: NSApplication.didBecomeActiveNotification, object: nil, queue: .main) { _ in activations += 1 }
    let key = NotificationCenter.default.addObserver(forName: NSWindow.didBecomeKeyNotification, object: nil, queue: .main) { _ in keys += 1 }
    let main = NotificationCenter.default.addObserver(forName: NSWindow.didBecomeMainNotification, object: nil, queue: .main) { _ in mains += 1 }
    let system = NSWorkspace.shared.notificationCenter.addObserver(forName: NSWorkspace.didActivateApplicationNotification, object: nil, queue: .main) { notification in
        if (notification.userInfo?[NSWorkspace.applicationUserInfoKey] as? NSRunningApplication)?.processIdentifier == ProcessInfo.processInfo.processIdentifier { systemActivations += 1 }
    }
    defer {
        NotificationCenter.default.removeObserver(active); NotificationCenter.default.removeObserver(key); NotificationCenter.default.removeObserver(main)
        NSWorkspace.shared.notificationCenter.removeObserver(system)
    }
    var windows: [SidebarToolbarCheckWindow] = []
    func safe() {
        require(NSApp.activationPolicy() == .prohibited && !NSApp.isActive && !NSRunningApplication.current.isActive,
                "activation is forbidden throughout the native check")
        require(activations == 0 && keys == 0 && mains == 0 && systemActivations == 0, "unexpected application/key/main activation")
        require(NSWorkspace.shared.frontmostApplication?.processIdentifier == frontmost, "frontmost application changed")
        for window in NSApp.windows {
            require(!window.isVisible && !window.isKeyWindow && !window.isMainWindow, "a window became visible, key or main")
            if !windows.contains(where: { $0 === window }) {
                let origin = class_getImageName(type(of: window)).map { String(cString: $0) } ?? ""
                require(origin.hasPrefix("/System/Library/"), "unexpected non-system window")
            }
        }
    }
    func settle() { RunLoop.main.run(until: Date().addingTimeInterval(0.08)); safe() }
    func descendants(_ view: NSView) -> [NSView] { [view] + view.subviews.flatMap(descendants) }
    func identifiers(_ root: Any) -> Set<String> {
        var visited = Set<ObjectIdentifier>(), result = Set<String>()
        func visit(_ value: Any) {
            if let view = value as? NSView, let id = view.identifier?.rawValue { result.insert(id) }
            guard let ax = value as? NSAccessibilityProtocol else { return }
            let object = value as AnyObject
            guard visited.insert(ObjectIdentifier(object)).inserted else { return }
            if let id = ax.accessibilityIdentifier(), !id.isEmpty { result.insert(id) }
            for child in ax.accessibilityChildren() ?? [] { visit(child) }
            if let view = value as? NSView { for child in view.subviews { visit(child) } }
        }
        visit(root)
        return result
    }
    let preferences = UserDefaults.standard.volatileDomain(forName: UserDefaults.argumentDomain)
    var synthetic = preferences
    synthetic["toolbarButtonsHidden"] = [String]()
    synthetic["toolbarButtonOrder"] = [String]()
    UserDefaults.standard.setVolatileDomain(synthetic, forName: UserDefaults.argumentDomain)
    var evidence: [[String: Any]] = []
    for dark in [false, true] {
        NSApp.appearance = NSAppearance(named: dark ? .darkAqua : .aqua)
        let device = DeviceStore(preview: true)
        let model = ConversationWindowModel(application: device, kind: .workspace)
        model.focused.store.selectConversation(id: "sample-0")
        let root = VeluneRootView(window: model, store: model.list, tab: model.focused,
                                  focused: model.focused.store, previewEmpty: false, previewSettings: false)
        let window = SidebarToolbarCheckWindow(contentRect: NSRect(x: 0, y: 0, width: 1060, height: 760),
                                              styleMask: [.titled, .closable, .resizable, .fullSizeContentView], backing: .buffered, defer: true)
        windows.append(window)
        window.isReleasedWhenClosed = false; window.isRestorable = false
        let shell = WorkspaceShell(window: window, model: model, content: root, previewEmpty: false)
        let host = shell.split
        let outer = host.splitView
        func capture(_ name: String, width: CGFloat, height: CGFloat = 760, mode: String, requestedSidebarWidth: CGFloat? = nil) {
            window.setContentSize(NSSize(width: width, height: height))
            for _ in 0..<5 { host.view.layoutSubtreeIfNeeded(); settle() }
            host.view.layoutSubtreeIfNeeded()
            guard let toolbar = window.toolbar, toolbar === shell.toolbar.toolbar else { fail("window toolbar must be the shell's: \(name)") }
            if let requestedSidebarWidth {
                if mode == "right" {
                    outer.setPosition(outer.bounds.width - requestedSidebarWidth - outer.dividerThickness, ofDividerAt: 1)
                } else {
                    outer.setPosition(requestedSidebarWidth, ofDividerAt: 0)
                }
                for _ in 0..<5 { host.view.layoutSubtreeIfNeeded(); settle() }
            }
            let nativeFrame = window.contentView!.superview!
            for _ in 0..<5 { nativeFrame.layoutSubtreeIfNeeded(); settle() }
            let sidebarView = host.sidebarItem.viewController.view
            require(sidebarView.identifier?.rawValue == "conversation-sidebar-column", "sidebar column must carry its identifier")
            require(host.sidebarItem.isCollapsed == (mode == "hidden" || mode == "right"), "sidebar collapse must follow the tools: \(name)")
            let sidebarRect = host.sidebarItem.isCollapsed ? NSRect.zero : sidebarView.convert(sidebarView.bounds, to: nativeFrame)
            guard let drawing = nativeFrame.bitmapImageRepForCachingDisplay(in: nativeFrame.bounds) else { fail("native window capture unavailable") }
            nativeFrame.cacheDisplay(in: nativeFrame.bounds, to: drawing)
            try! drawing.representation(using: .png, properties: [:])!.write(to: directory.appendingPathComponent(name + ".png"))
            let hierarchy = descendants(nativeFrame).map { view in
                "\(String(describing: type(of: view))) id=\(view.identifier?.rawValue ?? "") frame=\(NSStringFromRect(view.convert(view.bounds, to: nativeFrame)))"
            }.joined(separator: "\n")
            try! hierarchy.write(to: directory.appendingPathComponent(name + "-native.txt"), atomically: true, encoding: .utf8)
            for tabScroll in descendants(nativeFrame).compactMap({ $0 as? TabScrollView }) {
                require(!tabScroll.hasHorizontalScroller && !tabScroll.hasVerticalScroller, "tab bar scroll view must disable system scrollers: \(name)")
                require(descendants(tabScroll).compactMap { $0 as? NSScroller }.allSatisfy { $0.isHidden || $0.frame.isEmpty },
                        "tab bar must not show any NSScroller: \(name)")
                require(abs(tabScroll.contentSize.height - tabScroll.frame.height) < 0.5,
                        "tab bar clip height must match scroll view height: \(name)")
            }
            var checkHideList: (() -> Void)?
            let items = toolbar.items
            let separators = items.enumerated().compactMap { index, item -> (Int, NSTrackingSeparatorToolbarItem)? in
                guard let separator = item as? NSTrackingSeparatorToolbarItem, separator.splitView === outer, separator.dividerIndex == 0 else { return nil }
                return (index, separator)
            }
            require(separators.count == 1, "must bind exactly one separator to the sidebar's divider: \(name)")
            let separator = separators[0]
            func indices(_ id: ConversationToolbar.Item) -> [Int] {
                items.indices.filter { items[$0].itemIdentifier == id.identifier && !items[$0].isHidden }
            }
            let back = indices(.back), forward = indices(.forward)
            // The sidebar's part of the toolbar holds the sidebar button alone;
            // the list keeps its own search, filter and new conversation wherever it is.
            let sidebarPart = items.prefix(separator.0).map(\.itemIdentifier)
            require(sidebarPart == [.toggleSidebar], "the sidebar part holds only the sidebar button: \(name), \(sidebarPart)")
            require(back.count == 1 && forward.count == 1, "navigation identities must resolve uniquely: \(name)")
            require(back[0] > separator.0 && forward[0] > separator.0, "navigation belongs to detail partition")
            if mode != "hidden" {
                let column = mode == "right" ? host.inspectorItem.viewController.view : sidebarView
                let ids = identifiers(column)
                require(ids.contains("conversation-list-filter") && ids.contains("conversation-list-new"),
                        "missing list filter or new action in its own column: \(name)")
                let fields = descendants(column).compactMap { $0 as? NSSearchField }.filter { !$0.isHiddenOrHasHiddenAncestor }
                require(fields.count == 1, "the list has exactly one search field: \(name)")
                let searchRect = fields[0].convert(fields[0].bounds, to: column)
                require(searchRect.width > 0 && searchRect.minX >= -1 && searchRect.maxX <= column.bounds.width + 1,
                        "search field must fit its column: \(name)")
                if let requestedSidebarWidth {
                    require(abs(column.bounds.width - requestedSidebarWidth) <= 2, "actual column width \(column.bounds.width) in \(name)")
                    let buttons = descendants(column).compactMap { $0 as? NSButton }.filter { !$0.isHiddenOrHasHiddenAncestor }
                    // Measure the actual list action row relative to its search field;
                    // a shared tool strip sits above it.
                    let header = buttons.filter { $0.convert($0.bounds, to: column).intersects(
                        NSRect(x: -column.bounds.width, y: searchRect.midY - 1, width: 3 * column.bounds.width, height: 2)) }
                    require(header.contains { $0 is NSPopUpButton } && header.contains { !($0 is NSPopUpButton) },
                            "missing native actions beside the search field: \(name)")
                    if mode == "shared" {
                        let headerRow = buttons.filter { button in
                            let rect = button.convert(button.bounds, to: column)
                            let distanceAboveSearch = column.isFlipped ? searchRect.midY - rect.midY : rect.midY - searchRect.midY
                            return distanceAboveSearch > searchRect.height / 2 && distanceAboveSearch <= PaneChrome.barHeight + searchRect.height / 2
                        }
                        require(headerRow.filter { $0 is NSPopUpButton }.count == 1 && headerRow.filter { !($0 is NSPopUpButton) }.count == 1,
                                "shared list needs its own two actions above search: \(name)")
                        let popup = headerRow.compactMap { $0 as? NSPopUpButton }.first!
                        let hide = headerRow.first { !($0 is NSPopUpButton) }!
                        let menu = popup.menu!
                        menu.delegate?.menuNeedsUpdate?(menu)
                        menu.update()
                        let titles = menu.items.flatMap { item in [item.title] + (item.submenu?.items.map(\.title) ?? []) }
                        require(titles.contains("停靠在右侧") && titles.contains("独立窗口"), "shared tool needs its placement commands: \(name), \(titles)")
                        if !dark && requestedSidebarWidth == 240 {
                            // Offscreen SwiftUI does not vend the static title to AX.
                            // Read the actual text once from the already captured window.
                            let title = VNRecognizeTextRequest()
                            title.recognitionLanguages = ["zh-Hans"]
                            title.regionOfInterest = CGRect(x: 0, y: 0.85, width: sidebarRect.width / nativeFrame.bounds.width, height: 0.15)
                            try! VNImageRequestHandler(cgImage: drawing.cgImage!).perform([title])
                            require(title.results?.contains { $0.topCandidates(1).first?.string == "会话列表" } == true, "shared list title missing: \(name)")
                            checkHideList = {
                                let initial = model.tools
                                hide.performClick(nil)
                                require(!model.tools.isOpen(.list) && model.tools.isOpen(.outline), "hide must remove only the list from its shared dock")
                                model.tools = initial
                            }
                        }
                        for button in headerRow {
                            let rect = button.convert(button.bounds, to: column)
                            require(!rect.isEmpty && column.bounds.contains(rect), "shared tool action outside column: \(name), \(rect)")
                        }
                    }
                    for button in header {
                        let rect = button.convert(button.bounds, to: column)
                        require(rect.width > 0 && rect.minX >= -1 && rect.maxX <= column.bounds.width + 1,
                                "list action extends beyond its column: \(name), \(rect)")
                    }
                }
            }
            if mode == "solo" || mode == "shared" {
                for index in back + forward {
                    guard let view = items[index].view else { fail("missing navigation view: \(name)") }
                    let rect = view.convert(view.bounds, to: nativeFrame)
                    require(rect.minX >= sidebarRect.maxX, "navigation must fit outside the sidebar: \(name)")
                }
            }
            let rows = items.enumerated().map { index, item -> [String: Any] in
                let rect = item.view.map { $0.convert($0.bounds, to: nativeFrame) } ?? .zero
                return ["index": index, "itemID": item.itemIdentifier.rawValue, "hidden": item.isHidden, "actionIDs": item.view.map(identifiers)?.sorted() ?? [],
                        "frame": NSStringFromRect(rect), "outerSidebarSeparator": index == separator.0,
                        "visibleItem": toolbar.visibleItems?.contains(where: { $0 === item }) == true]
            }
            evidence.append(["case": name, "hostOS": ProcessInfo.processInfo.operatingSystemVersionString,
                             "width": width, "height": height, "mode": mode, "items": rows,
                             "sidebarFrame": NSStringFromRect(sidebarRect),
                             "visible": window.isVisible, "key": window.isKeyWindow, "main": window.isMainWindow])
            try! JSONSerialization.data(withJSONObject: evidence, options: [.prettyPrinted, .sortedKeys]).write(to: directory.appendingPathComponent("sidebar-toolbar-check.json"))
            checkHideList?()
            safe()
        }
        let tone = dark ? "dark" : "light"
        capture("sidebar-solo-\(tone)-1060", width: 1060, mode: "solo")
        capture("sidebar-solo-\(tone)-720", width: 720, mode: "solo")
        capture("sidebar-solo-\(tone)-1440", width: 1440, mode: "solo")
        capture("sidebar-minimum-\(tone)-requested190", width: 1060, mode: "solo", requestedSidebarWidth: 190)
        for columnWidth: CGFloat in [240, 420] {
            capture("sidebar-solo-\(tone)-\(Int(columnWidth))", width: 1200, mode: "solo", requestedSidebarWidth: columnWidth)
        }
        var shared = ToolWindowLayout(); shared[.outline] = ToolWindowState(place: .docked(.left))
        model.tools = shared
        for columnWidth: CGFloat in [240, 420] {
            capture("sidebar-shared-\(tone)-\(Int(columnWidth))", width: 1200, mode: "shared", requestedSidebarWidth: columnWidth)
        }
        capture("sidebar-shared-\(tone)", width: 1060, mode: "shared")
        model.tools = ToolWindowLayout()
        capture("sidebar-restored-\(tone)", width: 1060, mode: "solo")
        model.tools.collapse(.left)
        capture("sidebar-hidden-\(tone)", width: 1060, mode: "hidden")
        model.tools.showArea(.left)
        capture("sidebar-shown-\(tone)", width: 1060, mode: "solo")
        // The system's Hide Sidebar collapses the area as the tools see it, and Show Sidebar brings it back.
        host.toggleSidebar(nil)
        for _ in 0..<10 { host.view.layoutSubtreeIfNeeded(); settle() }
        require(model.tools.tools(in: .left).isEmpty, "Hide Sidebar must collapse the left area")
        host.toggleSidebar(nil)
        for _ in 0..<10 { host.view.layoutSubtreeIfNeeded(); settle() }
        require(model.tools.tools(in: .left) == [.list], "Show Sidebar must bring the list back")
        var right = ToolWindowLayout(); right[.list].place = .docked(.right)
        model.tools = right
        for columnWidth: CGFloat in [240, 420] {
            capture("sidebar-right-\(tone)-\(Int(columnWidth))", width: 1200, mode: "right", requestedSidebarWidth: columnWidth)
        }
        capture("sidebar-right-\(tone)", width: 1060, mode: "right")
        require(!host.inspectorItem.isCollapsed, "the right area must open the inspector")
        model.tools = ToolWindowLayout()
        capture("sidebar-return-left-\(tone)", width: 1060, mode: "solo")
        var inspector = ToolWindowLayout()
        inspector[.outline] = ToolWindowState(place: .docked(.right))
        model.tools = inspector
        capture("sidebar-with-inspector-\(tone)", width: 1060, mode: "solo")
        model.split(.horizontal)
        capture("sidebar-with-inspector-and-split-\(tone)", width: 1200, mode: "solo")
        model.openInNewTab("sample-1")
        model.openInNewTab("sample-2")
        for _ in 0..<9 { model.openInNewTab() }
        capture("shell-tabs-overflow-\(tone)", width: 960, mode: "solo")
        let overflowStrips = descendants(window.contentView!.superview!).compactMap { ($0 as? TabScrollView)?.documentView }
        require(overflowStrips.contains { $0.frame.width > ($0.enclosingScrollView?.contentSize.width ?? 0) },
                "overflow shell must have a scrollable tab strip: \(tone)")
        var bottomDock = ToolWindowLayout()
        bottomDock[.outline] = ToolWindowState(place: .docked(.bottom))
        bottomDock[.agents] = ToolWindowState(place: .docked(.bottom))
        model.tools = bottomDock
        capture("shell-bottom-dock-\(tone)", width: 1060, mode: "solo")
        let native = window.contentView!.superview!
        guard let bottomSplit = descendants(native).compactMap({ $0 as? BottomSplitView }).first, bottomSplit.subviews.count == 2 else {
            fail("shell-bottom-dock must mount bottom area: \(tone)")
        }
        require(bottomSplit.subviews[1].frame.height >= 100 && native.bounds.contains(bottomSplit.subviews[1].convert(bottomSplit.subviews[1].bounds, to: native)),
                "bottom area frame must stay inside window: \(tone)")
        var narrowEdge = bottomDock
        narrowEdge[.outline] = ToolWindowState(place: .docked(.right))
        model.tools = narrowEdge
        capture("shell-narrow-edge-\(tone)", width: 640, height: 480, mode: "solo")
        for composerText in descendants(native).compactMap({ $0 as? ComposerTextView }) {
            guard let editorScroll = composerText.enclosingScrollView else { continue }
            var ancestor: NSView? = editorScroll.superview
            while let current = ancestor, !(current.superview is PaneSplitView || current.superview is PaneGridView) {
                ancestor = current.superview
            }
            if let paneHost = ancestor {
                let rect = editorScroll.convert(editorScroll.bounds, to: paneHost)
                require(rect.minX >= -1 && rect.maxX <= paneHost.bounds.width + 1,
                        "narrow pane composer must stay inside its own pane: \(rect) in \(paneHost.bounds) (\(tone))")
            }
        }
        window.contentViewController = nil
        for tab in model.allTabs { tab.close() }
        model.list.close()
    }
    do {
        // Saved widths are what a new window's sidebar and inspector open at.
        let device = DeviceStore(preview: true)
        let model = ConversationWindowModel(application: device, kind: .workspace)
        var tools = ToolWindowLayout()
        tools[.outline] = ToolWindowState(place: .docked(.right))
        tools.leftWidth = 310
        tools.rightWidth = 280
        model.tools = tools
        let root = VeluneRootView(window: model, store: model.list, tab: model.focused,
                                  focused: model.focused.store, previewEmpty: false, previewSettings: false)
        let window = SidebarToolbarCheckWindow(contentRect: NSRect(x: 0, y: 0, width: 1200, height: 760),
                                              styleMask: [.titled, .closable, .resizable, .fullSizeContentView], backing: .buffered, defer: true)
        windows.append(window)
        window.isReleasedWhenClosed = false; window.isRestorable = false
        let shell = WorkspaceShell(window: window, model: model, content: root, previewEmpty: false)
        window.setContentSize(NSSize(width: 1200, height: 760))
        for _ in 0..<8 { shell.split.view.layoutSubtreeIfNeeded(); settle() }
        let left = shell.split.sidebarItem.viewController.view.frame.width
        let right = shell.split.inspectorItem.viewController.view.frame.width
        require(abs(left - 310) < 2 && abs(right - 280) < 2, "saved widths must be restored: \(left), \(right)")
        window.setContentSize(NSSize(width: 1000, height: 760))
        for _ in 0..<5 { shell.split.view.layoutSubtreeIfNeeded(); settle() }
        require(abs(shell.split.sidebarItem.viewController.view.frame.width - 310) < 2, "the sidebar keeps its width as the window narrows")
        window.contentViewController = nil
        model.list.close(); model.focused.store.close()
    }
    UserDefaults.standard.setVolatileDomain(preferences, forName: UserDefaults.argumentDomain)
    safe()
    print("PASS sidebar toolbar: \(evidence.count) production layouts in the AppKit shell, the sidebar part holds only its button and the list keeps its own actions, collapse follows tools and Show/Hide Sidebar, saved widths restored; no presentation or activation")
    exit(0)
}
#endif
