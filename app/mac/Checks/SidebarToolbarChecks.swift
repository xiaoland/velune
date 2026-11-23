#if VELUNE_DEV_CHECKS
import AppKit
import SwiftUI
import ObjectiveC

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
    // This predicate deliberately has no x-only fallback. A control in the
    // detail toolbar can overlap the sidebar at a particular window width.
    func sidebarOwns(action: Int, separator: Int, matchesOuterSplit: Bool) -> Bool {
        matchesOuterSplit && action < separator
    }
    require(!sidebarOwns(action: 4, separator: 3, matchesOuterSplit: true), "negative: detail item before an inner divider must fail")
    require(!sidebarOwns(action: 2, separator: 7, matchesOuterSplit: false), "negative: unrelated inner split must fail")
    require(sidebarOwns(action: 2, separator: 3, matchesOuterSplit: true), "positive sidebar partition")
    let preferences = UserDefaults.standard.volatileDomain(forName: UserDefaults.argumentDomain)
    var synthetic = preferences
    synthetic["toolbarButtonsHidden"] = [String]()
    synthetic["toolbarButtonOrder"] = ["listFilter", "newConversation"]
    UserDefaults.standard.setVolatileDomain(synthetic, forName: UserDefaults.argumentDomain)
    var evidence: [[String: Any]] = []
    for dark in [false, true] {
        NSApp.appearance = NSAppearance(named: dark ? .darkAqua : .aqua)
        let device = DeviceStore(preview: true)
        let model = ConversationWindowModel(application: device, kind: .workspace)
        model.focused.store.selectConversation(id: "sample-0")
        let root = VeluneRootView(window: model, store: model.list, tab: model.focused,
                                  focused: model.focused.store, previewEmpty: false, previewSettings: false)
        let host = NSHostingController(rootView: root)
        let window = SidebarToolbarCheckWindow(contentRect: NSRect(x: 0, y: 0, width: 1060, height: 760),
                                              styleMask: [.titled, .closable, .resizable, .fullSizeContentView], backing: .buffered, defer: true)
        windows.append(window)
        window.isReleasedWhenClosed = false; window.isRestorable = false
        window.contentViewController = host
        func capture(_ name: String, width: CGFloat, mode: String, requestedSidebarWidth: CGFloat? = nil,
                     restoreMisorderedToolbar: Bool = false) {
            window.setContentSize(NSSize(width: width, height: 760))
            for _ in 0..<5 { host.view.layoutSubtreeIfNeeded(); settle() }
            host.view.layoutSubtreeIfNeeded()
            let views = descendants(host.view)
            guard let marker = views.first(where: { $0.identifier?.rawValue == "conversation-sidebar-column" }) else { fail("missing production sidebar anchor: \(name)") }
            var ancestor = marker.superview
            var outer: NSSplitView?
            while let view = ancestor {
                if let split = view as? NSSplitView, split.isVertical { outer = split; break }
                ancestor = view.superview
            }
            guard let outer, let toolbar = window.toolbar else { fail("complete native sidebar split/toolbar unavailable: \(name)") }
            if let requestedSidebarWidth {
                outer.setPosition(requestedSidebarWidth, ofDividerAt: 0)
                outer.adjustSubviews()
                for _ in 0..<5 { host.view.layoutSubtreeIfNeeded(); settle() }
            }
            // Save the complete native drawing before any semantic assertion,
            // so unsupported hidden-host behavior never becomes a blind retry.
            let nativeFrame = window.contentView!.superview!
            func saveDrawing(_ artifact: String) {
                guard let drawing = nativeFrame.bitmapImageRepForCachingDisplay(in: nativeFrame.bounds) else { fail("native window capture unavailable") }
                nativeFrame.cacheDisplay(in: nativeFrame.bounds, to: drawing)
                try! drawing.representation(using: .png, properties: [:])!.write(to: directory.appendingPathComponent(artifact + ".png"))
            }
            if restoreMisorderedToolbar && !toolbar.allowsUserCustomization {
                let original = toolbar.itemIdentifiers
                let items = toolbar.items
                guard let partition = marker as? ConversationToolbarPartition.Anchor else { fail("missing production partition observer") }
                let misplaced = items.filter { item in
                    if let separator = item as? NSTrackingSeparatorToolbarItem { return separator.splitView === outer }
                    guard let view = item.view else { return false }
                    return !identifiers(view).isDisjoint(with: ["conversation-list-filter", "conversation-new-conversation"])
                }.map(\.itemIdentifier)
                let navigation = items.first { $0.view.map { identifiers($0).contains("conversation-forward") } ?? false }
                require(misplaced.count == 3 && navigation != nil, "need two list actions, sidebar divider and forward button to recreate displacement")
                var incorrect = original.filter { !misplaced.contains($0) }
                let afterNavigation = incorrect.firstIndex(of: navigation!.itemIdentifier)! + 1
                incorrect.insert(contentsOf: misplaced, at: afterNavigation)

                // First let the bad native order settle without its observer.
                partition.hasList = false
                toolbar.itemIdentifiers = incorrect
                for _ in 0..<5 { host.view.layoutSubtreeIfNeeded(); settle() }
                require(toolbar.itemIdentifiers == incorrect, "native toolbar must retain the incorrect order without recovery")
                saveDrawing(name + "-misordered")
                partition.hasList = true
                partition.scheduleCheck()
                for _ in 0..<5 { host.view.layoutSubtreeIfNeeded(); settle() }
                require(toolbar.itemIdentifiers == original, "initial check did not restore the sidebar partition")

                // Also force a late native reorder with the observer installed.
                // In this order the divider is in place and only the buttons drift.
                guard let separatorID = misplaced.last else { fail("need outer split divider") }
                var separated = original.filter { !misplaced.dropLast().contains($0) }
                let contentIndex = separated.firstIndex(of: navigation!.itemIdentifier)! + 1
                separated.insert(contentsOf: misplaced.dropLast(), at: contentIndex)
                require(separated.contains(separatorID), "late reorder must retain sidebar divider")
                toolbar.itemIdentifiers = separated
                for _ in 0..<5 { host.view.layoutSubtreeIfNeeded(); settle() }
                require(toolbar.itemIdentifiers == original, "observed reorder did not restore the sidebar partition")
            }
            saveDrawing(name)
            let hierarchy = descendants(nativeFrame).map { view in
                "\(String(describing: type(of: view))) id=\(view.identifier?.rawValue ?? "") frame=\(NSStringFromRect(view.convert(view.bounds, to: nativeFrame)))"
            }.joined(separator: "\n")
            try! hierarchy.write(to: directory.appendingPathComponent(name + "-native.txt"), atomically: true, encoding: .utf8)
            let items = toolbar.items
            let separators = items.enumerated().compactMap { index, item -> (Int, NSTrackingSeparatorToolbarItem)? in
                guard let separator = item as? NSTrackingSeparatorToolbarItem, separator.splitView === outer else { return nil }
                return (index, separator)
            }
            require(separators.count == 1, "must bind exactly one separator to outer sidebar split: \(name)")
            let separator = separators[0]
            require(separator.1.dividerIndex == 0, "sidebar must be first outer split column")
            let itemIDs = items.map { $0.view.map(identifiers) ?? [] }
            func indices(_ id: String) -> [Int] { itemIDs.indices.filter { itemIDs[$0].contains(id) } }
            let filter = indices("conversation-list-filter"), create = indices("conversation-new-conversation")
            let back = indices("conversation-back"), forward = indices("conversation-forward")
            if mode == "solo" {
                require(filter.count == 1 && create.count == 1, "sidebar action identities must resolve uniquely: \(name), \(itemIDs)")
                for index in filter + create {
                    require(sidebarOwns(action: index, separator: separator.0, matchesOuterSplit: true), "action belongs to Content toolbar: \(name)")
                }
            } else if mode == "shared" {
                require(filter.isEmpty && create.count == 1, "shared block owns filter; sidebar toolbar owns only new: \(name)")
                require(sidebarOwns(action: create[0], separator: separator.0, matchesOuterSplit: true), "shared new belongs to Content toolbar")
                require(identifiers(host.view).contains("conversation-list-filter"), "shared block filter remains rendered")
            } else {
                require(filter.isEmpty && create.isEmpty, "hidden/right list must not leak actions into titlebar: \(name)")
            }
            require(back.count == 1 && forward.count == 1, "navigation identities must resolve uniquely: \(name)")
            require(back[0] > separator.0 && forward[0] > separator.0, "navigation belongs to detail partition")
            var splitChild: NSView = marker
            while let parent = splitChild.superview, parent !== outer { splitChild = parent }
            require(outer.arrangedSubviews.first === splitChild, "anchor must identify outer sidebar column")
            let frame = window.contentView!.superview!
            let sidebarRect = splitChild.convert(splitChild.bounds, to: frame)
            if mode == "solo" {
                for index in filter + create {
                    require(toolbar.visibleItems?.contains(where: { $0 === items[index] }) == true,
                            "sidebar action must remain directly visible, not in global overflow: \(name)")
                    guard let view = items[index].view else { fail("missing visible sidebar action view") }
                    let rect = view.convert(view.bounds, to: frame)
                    require(rect.width > 0 && rect.minX >= sidebarRect.minX && rect.maxX <= sidebarRect.maxX,
                            "visible action must fit the actual sidebar: \(name), \(rect), \(sidebarRect)")
                }
                if requestedSidebarWidth != nil { require(sidebarRect.width >= 220, "sole-list sidebar must honor toolbar minimum") }
            }
            let rows = items.enumerated().map { index, item -> [String: Any] in
                let rect = item.view.map { $0.convert($0.bounds, to: frame) } ?? .zero
                return ["index": index, "itemID": item.itemIdentifier.rawValue, "actionIDs": itemIDs[index].sorted(),
                        "frame": NSStringFromRect(rect), "outerSidebarSeparator": index == separator.0,
                        "visibleItem": toolbar.visibleItems?.contains(where: { $0 === item }) == true]
            }
            evidence.append(["case": name, "hostOS": ProcessInfo.processInfo.operatingSystemVersionString,
                             "width": width, "mode": mode, "items": rows,
                             "sidebarFrame": NSStringFromRect(splitChild.convert(splitChild.bounds, to: frame)),
                             "outerSplit": String(describing: ObjectIdentifier(outer)),
                             "separatorSplit": String(describing: ObjectIdentifier(separator.1.splitView)),
                             "splitClass": String(describing: type(of: outer)), "dividerIndex": separator.1.dividerIndex,
                             "visible": window.isVisible, "key": window.isKeyWindow, "main": window.isMainWindow])
            try! JSONSerialization.data(withJSONObject: evidence, options: [.prettyPrinted, .sortedKeys]).write(to: directory.appendingPathComponent("sidebar-toolbar-check.json"))
            safe()
        }
        let tone = dark ? "dark" : "light"
        capture("sidebar-solo-\(tone)-1060", width: 1060, mode: "solo")
        capture("sidebar-recovered-\(tone)", width: 1060, mode: "solo", restoreMisorderedToolbar: true)
        capture("sidebar-solo-\(tone)-720", width: 720, mode: "solo")
        capture("sidebar-solo-\(tone)-1440", width: 1440, mode: "solo")
        capture("sidebar-minimum-\(tone)-requested190", width: 1060, mode: "solo", requestedSidebarWidth: 190)
        var shared = ToolWindowLayout(); shared[.outline] = ToolWindowState(place: .docked(.left))
        model.tools = shared
        capture("sidebar-shared-\(tone)", width: 1060, mode: "shared")
        model.tools = ToolWindowLayout()
        capture("sidebar-restored-\(tone)", width: 1060, mode: "solo")
        model.tools.collapse(.left)
        capture("sidebar-hidden-\(tone)", width: 1060, mode: "hidden")
        model.tools.showArea(.left)
        capture("sidebar-shown-\(tone)", width: 1060, mode: "solo")
        var right = ToolWindowLayout(); right[.list].place = .docked(.right)
        model.tools = right
        capture("sidebar-right-\(tone)", width: 1060, mode: "right")
        model.tools = ToolWindowLayout()
        capture("sidebar-return-left-\(tone)", width: 1060, mode: "solo")
        var inspector = ToolWindowLayout()
        inspector[.outline] = ToolWindowState(place: .docked(.right))
        model.tools = inspector
        capture("sidebar-with-inspector-\(tone)", width: 1060, mode: "solo")
        model.split(.horizontal)
        capture("sidebar-with-inspector-and-split-\(tone)", width: 1200, mode: "solo", restoreMisorderedToolbar: true)
        window.contentViewController = nil
        model.list.close(); model.focused.store.close()
    }
    UserDefaults.standard.setVolatileDomain(preferences, forName: UserDefaults.argumentDomain)
    safe()
    print("PASS sidebar toolbar: \(evidence.count) production layouts, native partition recovery, unique action IDs, branch transitions; no presentation or activation")
    exit(0)
}
#endif
