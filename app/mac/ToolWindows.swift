import AppKit
import SwiftUI
import VeluneBindings

/// The parts of a conversation window that can leave their place: the list,
/// the outline, the composer, linked agents and background tasks.
enum ToolWindowKind: String, Codable, CodingKeyRepresentable, CaseIterable {
    /// The linked agents keep the name saved layouts know them by, from when they listed sub-agents alone.
    case list, outline, composer, agents = "subAgents"
    case backgroundTasks

    var title: String {
        switch self {
        case .list: "会话列表"
        case .outline: "会话大纲"
        case .composer: "输入区"
        case .agents: "关联 Agent"
        case .backgroundTasks: "后台任务"
        }
    }
}

/// The areas of a window tool windows dock into: the sidebar, the inspector
/// and the strip below the panes.
enum DockSide: String, Codable, CaseIterable {
    case left, right, bottom

    var title: String {
        switch self {
        case .left: "左侧"
        case .right: "右侧"
        case .bottom: "底部"
        }
    }

    /// The tool an area shown with nothing docked there brings along.
    var usualTool: ToolWindowKind {
        switch self {
        case .left: .list
        case .right: .outline
        case .bottom: .composer
        }
    }
}

/// Where a tool window is.
enum ToolPlace: Codable, Hashable {
    /// The tool's own place: the outline and the linked agents from their
    /// toolbar buttons, the composer in each pane. The list has none.
    case builtIn
    case docked(DockSide)
    /// A window of its own.
    case window

    var side: DockSide? {
        if case .docked(let side) = self { side } else { nil }
    }
}

/// One tool window's state; a hidden tool keeps its place.
struct ToolWindowState: Codable, Equatable {
    var place: ToolPlace
    var shown = true
    /// Its share of its dock area beside the other tools there.
    var weight = 1.0
    var windowFrame: ToolFrame?
    /// As a window of its own, kept in front of other apps' windows.
    var pinned = false
}

/// A tool's places as menus offer them. The outline's own place is the
/// toolbar button, which pops it over or collapses the pane in place, as an
/// application setting says.
enum ToolChoice: Hashable {
    case place(ToolPlace)
    case outline(OutlinePresentation)

    static func all(for kind: ToolWindowKind) -> [(choice: ToolChoice, title: String)] {
        let docked: [(ToolChoice, String)] = (kind == .composer ? [DockSide.bottom, .left, .right] : DockSide.allCases)
            .map { (.place(.docked($0)), "停靠在\($0.title)") }
        let detached: [(ToolChoice, String)] = [(.place(.window), "独立窗口")]
        return switch kind {
        case .list: docked + detached
        case .outline: [(.outline(.popover), "弹出"), (.outline(.inPlace), "就地折叠")] + docked + detached
        case .composer: [(.place(.builtIn), "在每个面板中")] + docked + detached
        case .agents, .backgroundTasks: [(.place(.builtIn), "弹出")] + docked + detached
        }
    }
}

/// Where a window's tool windows are, restored with the window. Visibility
/// is two things: whether an area is open, and which area a tool is in. An
/// area is open while it shows a tool: hiding the last one there closes the
/// area, which keeps the tool for when it opens again, and one with nothing
/// left in it is closed. A tool hidden beside others has left the area and
/// keeps it to come back to.
struct ToolWindowLayout: Codable, Equatable {
    var tools: [ToolWindowKind: ToolWindowState] = [.list: ToolWindowState(place: .docked(.left)), .outline: ToolWindowState(place: .builtIn), .composer: ToolWindowState(place: .builtIn)]
    /// The areas closed by hand; one with nothing to show is closed as well.
    var collapsed: Set<DockSide> = []
    /// Nil in older saved layouts means the default split presentation.
    var sideChatPresentation: SideChatPresentation?
    /// Total drawer height, independent of composer state or draft length.
    var bottomHeight = 220.0
    /// The sidebar's and the inspector's widths as last dragged; nil until then.
    var leftWidth: Double?
    var rightWidth: Double?
    /// The areas "editor only" collapsed, to open again.
    var editorOnly: Set<DockSide>?

    subscript(kind: ToolWindowKind) -> ToolWindowState {
        get { tools[kind] ?? ToolWindowState(place: kind == .list ? .docked(.left) : .builtIn) }
        set { tools[kind] = newValue }
    }

    /// As a tool window, anywhere.
    func isOpen(_ kind: ToolWindowKind) -> Bool {
        let state = self[kind]
        guard state.shown else { return false }
        return switch state.place {
        case .builtIn: false
        case .docked(let side): !collapsed.contains(side)
        case .window: true
        }
    }

    /// The tools an area shows, in a fixed order.
    func tools(in side: DockSide) -> [ToolWindowKind] {
        collapsed.contains(side) ? [] : docked(in: side).filter { self[$0].shown }
    }

    /// What an area's view holds: its tools, and while it is collapsed the
    /// ones it showed, so closing and opening it move the same views rather
    /// than tear them down and build them again. A composer is put away with
    /// its area.
    func kept(in side: DockSide) -> [ToolWindowKind] {
        collapsed.contains(side) ? docked(in: side).filter { self[$0].shown && $0 != .composer } : tools(in: side)
    }

    func docked(in side: DockSide) -> [ToolWindowKind] {
        ToolWindowKind.allCases.filter { self[$0].place == .docked(side) }
    }

    /// The window's one composer is showing.
    var sharesComposer: Bool { isOpen(.composer) }
    /// Each pane has its own composer while the window's has no place of its
    /// own; docked in a collapsed area it is put away with the area.
    var panesHaveComposer: Bool { self[.composer].place == .builtIn }
    var isEditorOnly: Bool { editorOnly != nil }

    mutating func move(_ kind: ToolWindowKind, to place: ToolPlace) {
        self[kind].place = place
        show(kind)
    }

    mutating func show(_ kind: ToolWindowKind) {
        self[kind].shown = true
        if let side = self[kind].place.side { open(side) }
    }

    /// The composer, which a pane cannot do without, goes back to the panes.
    /// The last tool an area shows closes the area instead, and comes back with it.
    mutating func hide(_ kind: ToolWindowKind) {
        if let side = self[kind].place.side, tools(in: side) == [kind], kind != .composer {
            collapse(side)
        } else if kind == .composer {
            self[kind].place = .builtIn
        } else {
            self[kind].shown = false
        }
    }

    /// Whether an area is open; an open one shows at least one tool.
    func isOpen(_ side: DockSide) -> Bool { !tools(in: side).isEmpty }

    mutating func collapse(_ side: DockSide) {
        collapsed.insert(side)
    }

    /// An area with nothing docked there brings its usual tool; one whose
    /// tools are all hidden shows them again.
    mutating func showArea(_ side: DockSide) {
        open(side)
        if docked(in: side).isEmpty { move(side.usualTool, to: .docked(side)) }
        else if tools(in: side).isEmpty { for kind in docked(in: side) { self[kind].shown = true } }
    }

    mutating func toggleArea(_ side: DockSide) {
        if isOpen(side) { collapse(side) } else { showArea(side) }
    }

    /// Collapse every area showing something, or open them again.
    mutating func toggleEditorOnly() {
        if let editorOnly {
            collapsed.subtract(editorOnly)
            self.editorOnly = nil
            return
        }
        let open = Set(DockSide.allCases.filter { !tools(in: $0).isEmpty })
        guard !open.isEmpty else { return }
        collapsed.formUnion(open)
        editorOnly = open
    }

    /// The tools of a window opened for one conversation: its linked agents
    /// on the left in the list's place; the list stays one command away.
    static func forSingleConversation() -> ToolWindowLayout {
        var layout = ToolWindowLayout()
        layout[.list].shown = false
        layout[.agents].place = .docked(.left)
        layout[.agents].shown = true
        layout.collapsed.remove(.left)
        return layout
    }

    /// The tools' shares of an area, after its dividers moved.
    mutating func setWeights(_ weights: [ToolWindowKind: Double]) {
        for (kind, weight) in weights { self[kind].weight = weight }
    }

    /// As read back from saved state: the list has no place of its own, and
    /// a composer is never merely hidden.
    func normalized() -> ToolWindowLayout {
        var layout = self
        if layout[.list].place == .builtIn { layout[.list].place = .docked(.left) }
        layout[.composer].shown = true
        return layout
    }

    private mutating func open(_ side: DockSide) {
        collapsed.remove(side)
        editorOnly = nil
    }

    /// Saved tool layouts from before floating panels were removed: a
    /// floating tool becomes a window of its own, keeping the panel's size and
    /// its place beside the window (`ToolFrame` keeps an attached panel's
    /// place from the window's edges). `tools` is a `ToolWindowLayout` as
    /// JSON; nil when nothing in it is of that format.
    static func migratingFloatingTools(_ tools: Any) -> [String: Any]? {
        guard var tools = tools as? [String: Any], var states = tools["tools"] as? [String: Any] else { return nil }
        var changed = false
        for (kind, value) in states {
            guard var state = value as? [String: Any] else { continue }
            let panel = state.removeValue(forKey: "panelFrame")
            if (state["place"] as? [String: Any])?["floating"] != nil {
                state["place"] = ["window": [String: Any]()]
                if let panel, !(panel is NSNull) { state["windowFrame"] = panel }
                changed = true
            } else if panel != nil { changed = true }
            states[kind] = state
        }
        guard changed else { return nil }
        tools["tools"] = states
        return tools
    }

    /// `data`, a JSON object whose `tools` is a tool layout, with floating
    /// tools migrated; nil when it has none.
    static func migratingFloatingTools(in data: Data) -> Data? {
        guard var object = try? JSONSerialization.jsonObject(with: data) as? [String: Any],
              let tools = object["tools"].flatMap(migratingFloatingTools) else { return nil }
        object["tools"] = tools
        return try? JSONSerialization.data(withJSONObject: object)
    }
}

/// The tool windows as last arranged in any window, for each window that
/// opens without a layout of its own. The system brings back a window's own
/// layout only when it reopens the window, which by default it does not after
/// quitting, so this one is kept in `VELUNE_HOME`. "Editor only" is a window's
/// passing state and is left out.
@MainActor final class RememberedToolLayout {
    private struct File: Codable {
        enum CodingKeys: String, CodingKey { case schema, tools }
        static let schema = 1
        var schema = File.schema
        var tools: ToolWindowLayout
    }
    private(set) var layout: ToolWindowLayout
    private let url: URL?
    private let failed: (Error) -> Void
    private var pending: DispatchWorkItem?
    private var canWrite = true

    /// Unknown or unreadable formats use display defaults while preserving the original file.
    init(home: URL?, failed: @escaping (Error) -> Void) {
        url = home?.appendingPathComponent("mac-tool-layout.json")
        self.failed = failed
        if let url, FileManager.default.fileExists(atPath: url.path) {
            var data = try? Data(contentsOf: url)
            var migrated = false
            if let original = data, let upgraded = ToolWindowLayout.migratingFloatingTools(in: original) {
                // The original stays beside it before anything is rewritten.
                let backup = url.appendingPathExtension("before-floating-tools.bak")
                do {
                    if !FileManager.default.fileExists(atPath: backup.path) { try original.write(to: backup, options: .atomic) }
                    data = upgraded
                    migrated = true
                } catch { failed(error) }
            }
            let file = data.flatMap { try? JSONDecoder().decode(File.self, from: $0) }
            let strict = data.flatMap { data -> Bool? in
                guard let file else { return nil }
                guard let source = try? JSONSerialization.jsonObject(with: data) as? [String: Any],
                      source.keys.allSatisfy({ File.CodingKeys(rawValue: $0) != nil }),
                      let encodedData = try? JSONEncoder().encode(file),
                      let encoded = try? JSONSerialization.jsonObject(with: encodedData) else { return nil }
                return SavedWindowLayout.hasOnlyEncodedKeys(source, encoded: encoded)
            }
            if let file, file.schema == File.schema, strict == true {
                layout = file.tools.normalized()
                if migrated { write() }
            } else {
                canWrite = false
                layout = ToolWindowLayout()
                failed(NSError(domain: "VelunePersistence", code: 1, userInfo: [NSLocalizedDescriptionKey: "mac-tool-layout.json 格式不受支持；保留原文件，布局更改不会写入。请升级到支持该格式的版本后重试。"]))
            }
        } else {
            layout = ToolWindowLayout()
        }
    }

    /// Kept a moment later, so a divider being dragged writes once.
    func remember(_ tools: ToolWindowLayout) {
        guard canWrite else { return }
        var tools = tools
        if tools.isEditorOnly { tools.toggleEditorOnly() }
        guard tools != layout else { return }
        layout = tools
        pending?.cancel()
        let write = DispatchWorkItem { [weak self] in self?.write() }
        pending = write
        DispatchQueue.main.asyncAfter(deadline: .now() + 0.3, execute: write)
    }

    /// Writes a change still waiting, before the application quits.
    func flush() {
        guard let pending, !pending.isCancelled else { return }
        pending.cancel()
        write()
    }

    private func write() {
        pending = nil
        guard canWrite, let url else { return }
        do {
            try FileManager.default.createDirectory(at: url.deletingLastPathComponent(), withIntermediateDirectories: true)
            try JSONEncoder().encode(File(tools: layout)).write(to: url, options: .atomic)
        } catch { failed(error) }
    }
}

/// A tool window's size and place on the screen. Layouts saved with floating
/// panels may carry an attachment: the place from the window's nearer edges,
/// where the panel was, read once when it first opens as a window.
struct ToolFrame: Codable, Equatable {
    struct Attachment: Codable, Equatable {
        /// Measured from the window's trailing edge, else its leading edge.
        var fromTrailing: Bool
        /// The panel's top from the window's top, else its bottom from the window's bottom.
        var fromTop: Bool
        var dx: Double
        var dy: Double
    }
    var width: Double
    var height: Double
    var x: Double
    var y: Double
    var attachment: Attachment?

    init(_ frame: NSRect, attachedTo window: NSRect?) {
        width = frame.width; height = frame.height; x = frame.minX; y = frame.minY
        attachment = window.map { window in
            let fromTrailing = abs(frame.minX - window.maxX) < abs(frame.minX - window.minX)
            let fromTop = abs(frame.maxY - window.maxY) <= abs(frame.minY - window.minY)
            return Attachment(fromTrailing: fromTrailing, fromTop: fromTop,
                              dx: frame.minX - (fromTrailing ? window.maxX : window.minX),
                              dy: fromTop ? frame.maxY - window.maxY : frame.minY - window.minY)
        }
    }

    func frame(beside window: NSRect) -> NSRect {
        guard let attachment else { return NSRect(x: x, y: y, width: width, height: height) }
        let minX = (attachment.fromTrailing ? window.maxX : window.minX) + attachment.dx
        let minY = attachment.fromTop ? window.maxY + attachment.dy - height : window.minY + attachment.dy
        return NSRect(x: minX, y: minY, width: width, height: height)
    }

    /// Where a tool window first opens: offset from its conversation window, on its screen.
    static func initialWindow(_ kind: ToolWindowKind, beside window: NSRect, screen: NSRect) -> ToolFrame {
        let size = kind == .composer ? NSSize(width: 640, height: 120) : NSSize(width: 320, height: min(640, window.height))
        var frame = NSRect(x: window.minX + 40, y: window.maxY - 60 - size.height, width: size.width, height: size.height)
        frame.origin.x = min(max(frame.minX, screen.minX), screen.maxX - frame.width)
        frame.origin.y = min(max(frame.minY, screen.minY), screen.maxY - frame.height)
        return ToolFrame(frame, attachedTo: nil)
    }
}

/// A tool window of its own: in Mission Control and the Window menu, able
/// to go full screen.
final class ToolWindow: NSWindow {
    let kind: ToolWindowKind
    weak var model: ConversationWindowModel?

    init(kind: ToolWindowKind, model: ConversationWindowModel) {
        self.kind = kind
        self.model = model
        super.init(contentRect: NSRect(x: 0, y: 0, width: 320, height: 600),
                   styleMask: [.titled, .closable, .miniaturizable, .resizable], backing: .buffered, defer: false)
        title = kind.title
        isReleasedWhenClosed = false
        // Its conversation window's layout restores it, not the system.
        isRestorable = false
        collectionBehavior = [.fullScreenPrimary, .managed]
        minSize = kind == .composer ? NSSize(width: 360, height: 60) : NSSize(width: 220, height: 240)
    }
}

/// The tool windows of one conversation window that are outside it.
@MainActor final class DetachedTools: NSObject, NSWindowDelegate {
    private unowned let model: ConversationWindowModel
    private var hosts: [ToolWindowKind: ToolWindow] = [:]
    private weak var main: NSWindow?
    private var observer: NSObjectProtocol?
    /// Set while a window is placed or fitted, not moved by the user.
    private var placing = false
    private var syncScheduled = false

    init(model: ConversationWindowModel) {
        self.model = model
    }

    /// Open and close tool windows to match the layout, once the window exists.
    func sync() {
        guard let window = model.window else {
            guard !syncScheduled else { return }
            syncScheduled = true
            DispatchQueue.main.asyncAfter(deadline: .now() + 0.1) { [weak self] in
                self?.syncScheduled = false
                self?.sync()
            }
            return
        }
        if main !== window { attach(to: window) }
        for kind in ToolWindowKind.allCases {
            let wanted = model.tools.isOpen(kind) && model.tools[kind].place == .window
            if !wanted { close(kind) }
            else if hosts[kind] == nil { open(kind, beside: window) }
            if let host = hosts[kind] { WindowPinning.apply(model.tools[kind].pinned, to: host) }
        }
    }

    func closeAll() {
        for kind in Array(hosts.keys) { close(kind) }
        observer.map(NotificationCenter.default.removeObserver)
        observer = nil
    }

    func isHost(_ window: NSWindow) -> Bool { hosts.values.contains { $0 === window } }

    /// The independent composer may remain visible on its own Space after its
    /// conversation window leaves the active Space. Count only the host that
    /// actually presents this window's focused store.
    func hasVisibleComposerWindow(for store: ConversationStore) -> Bool {
        guard model.tools.isOpen(.composer), model.focused.store === store, let host = hosts[.composer] else { return false }
        return host.isVisible && host.isOnActiveSpace && !host.isMiniaturized
    }

    /// A shown tool window comes forward and takes the keyboard.
    func bringForward(_ kind: ToolWindowKind) {
        hosts[kind]?.makeKeyAndOrderFront(nil)
    }

    /// A tool window's subtitle names the conversation its window shows.
    func setSubtitle(_ title: String, for kind: ToolWindowKind) {
        if let window = hosts[kind], window.subtitle != title { window.subtitle = title }
    }

    private func attach(to window: NSWindow) {
        observer.map(NotificationCenter.default.removeObserver)
        main = window
        observer = NotificationCenter.default.addObserver(forName: NSWindow.willCloseNotification, object: window, queue: .main) { [weak self] _ in
            MainActor.assumeIsolated { self?.closeAll() }
        }
    }

    private func open(_ kind: ToolWindowKind, beside window: NSWindow) {
        let host = ToolWindow(kind: kind, model: model)
        let hosting = NSHostingView(rootView: DetachedToolContent(kind: kind, window: model))
        // The window's size is the user's, not the content's.
        hosting.sizingOptions = []
        host.contentView = hosting
        host.delegate = self
        host.addTitlebarAccessoryViewController(placeAccessory(for: kind))
        hosts[kind] = host
        let screen = window.screen?.visibleFrame ?? window.frame
        let saved = model.tools[kind].windowFrame ?? .initialWindow(kind, beside: window.frame, screen: screen)
        placing = true
        host.setFrame(saved.frame(beside: window.frame), display: false)
        placing = false
        // From here on its place is its own, on the screen.
        if saved.attachment != nil || model.tools[kind].windowFrame == nil { record(host) }
        if window.isVisible { host.orderFront(nil) }
    }

    /// A borderless symbol at the title bar's trailing end, as title bar
    /// controls are, opening a menu of the tool's other places.
    private func placeAccessory(for kind: ToolWindowKind) -> NSTitlebarAccessoryViewController {
        let image = NSImage(systemSymbolName: "pip.enter", accessibilityDescription: "位置")?
            .withSymbolConfiguration(NSImage.SymbolConfiguration(pointSize: 11, weight: .regular))
        let button = NSButton(image: image ?? NSImage(), target: self, action: #selector(placePressed(_:)))
        button.isBordered = false
        button.imagePosition = .imageOnly
        button.tag = ToolWindowKind.allCases.firstIndex(of: kind)!
        button.toolTip = "停靠到窗口或移回原位"
        button.setAccessibilityLabel("位置")
        // In a box that keeps it clear of the window's edge. A tint colour
        // leaves a title bar accessory's symbol undrawn.
        let box = NSView(frame: NSRect(x: 0, y: 0, width: 32, height: 19))
        button.frame = NSRect(x: 4, y: 0, width: 20, height: 19)
        button.autoresizingMask = [.minYMargin, .maxYMargin]
        box.addSubview(button)
        let accessory = NSTitlebarAccessoryViewController()
        accessory.view = box
        accessory.layoutAttribute = .trailing
        return accessory
    }

    @objc private func placePressed(_ sender: NSButton) {
        let kind = ToolWindowKind.allCases[sender.tag]
        let menu = NSMenu()
        let current = model.tools[kind].place
        for (choice, title) in ToolChoice.all(for: kind) {
            guard case .place(let place) = choice, place != current else { continue }
            let item = NSMenuItem(title: title, action: #selector(placeChosen(_:)), keyEquivalent: "")
            item.target = self
            item.representedObject = PlaceChoice(kind: kind, place: place)
            menu.addItem(item)
        }
        menu.popUp(positioning: nil, at: NSPoint(x: 0, y: sender.bounds.maxY + 4), in: sender)
    }

    private final class PlaceChoice: NSObject {
        let kind: ToolWindowKind
        let place: ToolPlace
        init(kind: ToolWindowKind, place: ToolPlace) { self.kind = kind; self.place = place }
    }

    @objc private func placeChosen(_ sender: NSMenuItem) {
        guard let choice = sender.representedObject as? PlaceChoice else { return }
        model.tools.move(choice.kind, to: choice.place)
    }

    /// The composer's window is as tall as its content, growing upward as the docked one does.
    func fitComposer(contentHeight: CGFloat) {
        guard let host = hosts[.composer], contentHeight > 0 else { return }
        let chrome = host.frame.height - host.contentLayoutRect.height
        let height = (contentHeight + chrome).rounded()
        guard abs(host.frame.height - height) >= 1 else { return }
        var frame = host.frame
        frame.size.height = height
        placing = true
        host.setFrame(frame, display: true)
        placing = false
        record(host)
    }

    func windowWillResize(_ sender: NSWindow, to size: NSSize) -> NSSize {
        // Only the composer's width is the user's; its height follows its content.
        guard (sender as? ToolWindow)?.kind == .composer else { return size }
        return NSSize(width: size.width, height: sender.frame.height)
    }

    private func close(_ kind: ToolWindowKind) {
        guard let host = hosts.removeValue(forKey: kind) else { return }
        host.delegate = nil
        host.orderOut(nil)
        host.close()
    }

    func windowShouldClose(_ sender: NSWindow) -> Bool {
        guard let host = sender as? ToolWindow else { return true }
        model.tools.hide(host.kind)
        return false
    }

    // Recorded once a resize or a move ends: the layout is published, and
    // every publication lays the window's toolbar out again.
    func windowDidEndLiveResize(_ notification: Notification) {
        guard let host = notification.object as? ToolWindow else { return }
        record(host)
    }

    func windowDidMove(_ notification: Notification) {
        guard !placing, let host = notification.object as? ToolWindow else { return }
        // The title bar drag is the window server's; record it once the button comes up.
        if NSEvent.pressedMouseButtons & 1 != 0 { waitForDragEnd(host) } else { record(host) }
    }

    private func waitForDragEnd(_ host: ToolWindow) {
        DispatchQueue.main.asyncAfter(deadline: .now() + 0.05) { [weak self, weak host] in
            guard let self, let host else { return }
            if NSEvent.pressedMouseButtons & 1 != 0 { return waitForDragEnd(host) }
            record(host)
        }
    }

    private func record(_ host: ToolWindow) {
        let frame = ToolFrame(host.frame, attachedTo: nil)
        if model.tools[host.kind].windowFrame != frame { model.tools[host.kind].windowFrame = frame }
    }
}

/// How the settings put the outline: from the toolbar button, which pops
/// it over or collapses the messages in place, or listed in a panel of its own.
enum OutlineForm: Hashable {
    case popover, inPlace, listed
}

extension ConversationWindowModel {
    static func outlineForm(application: DeviceStore) -> OutlineForm {
        if application.rememberedToolLayout.layout[.outline].place != .builtIn { return .listed }
        return application.outlinePresentation == .popover ? .popover : .inPlace
    }

    /// Applies the form to every open window; listed, the outline docks at
    /// the right unless a window has it somewhere else than the button.
    static func setOutlineForm(_ form: OutlineForm, application: DeviceStore) {
        let presentation: OutlinePresentation = form == .inPlace ? .inPlace : .popover
        if form != .listed, application.outlinePresentation != presentation { application.setOutlinePresentation(presentation) }
        for window in openWindows {
            switch form {
            case .listed: if window.tools[.outline].place == .builtIn { window.tools.move(.outline, to: .docked(.right)) }
            case .popover, .inPlace: window.tools.move(.outline, to: .builtIn)
            }
        }
    }

    /// The choice a tool's menus show as made.
    func choice(for kind: ToolWindowKind) -> ToolChoice {
        let place = tools[kind].place
        if kind == .outline, place == .builtIn { return .outline(application.outlinePresentation) }
        return .place(place)
    }

    func choose(_ choice: ToolChoice, for kind: ToolWindowKind) {
        if kind == .agents { agentsPopover = false }
        if kind == .backgroundTasks { backgroundTasksPopover = false }
        switch choice {
        case .place(let place): tools.move(kind, to: place)
        case .outline(let presentation):
            tools.move(.outline, to: .builtIn)
            if application.outlinePresentation != presentation { application.setOutlinePresentation(presentation) }
        }
    }

    /// Whether showing or hiding a tool can act. The linked agents in their
    /// own place pop over from a toolbar button offered only while the
    /// focused conversation has some.
    func canToggle(_ kind: ToolWindowKind) -> Bool {
        switch kind {
        case .agents: tools[kind].place != .builtIn || agentsPopover || focused.store.hasLinkedAgents
        case .backgroundTasks: tools[kind].place != .builtIn || backgroundTasksPopover || focused.store.canShowBackgroundTasks
        default: true
        }
    }

    /// Show a tool, or hide it when it shows. The outline in its own place
    /// pops over or collapses the focused pane in place; the linked agents
    /// pop over, or, with their button customized out of the toolbar or the
    /// toolbar hidden, move to a window of their own, since nothing would pop over.
    func toggle(_ kind: ToolWindowKind) {
        if kind == .agents, tools[.agents].place == .builtIn {
            if agentsPopover { agentsPopover = false }
            else if !focused.store.hasLinkedAgents { return }
            else if hasToolbarItem("sub-agents", fallback: .subAgents) { agentsPopover = true }
            else { tools.move(.agents, to: .window); detached.bringForward(.agents) }
            return
        }
        if kind == .backgroundTasks, tools[kind].place == .builtIn {
            if backgroundTasksPopover { backgroundTasksPopover = false }
            else if !focused.store.canShowBackgroundTasks { return }
            else if hasToolbarItem("background-tasks", fallback: .backgroundTasks) { backgroundTasksPopover = true }
            else { tools.move(kind, to: .window); detached.bringForward(kind) }
            return
        }
        if kind == .outline, tools[.outline].place == .builtIn {
            switch application.outlinePresentation {
            case .popover: outlinePopover.toggle()
            case .inPlace: focused.outlineCollapsed.toggle()
            }
            return
        }
        if tools.isOpen(kind) { tools.hide(kind) } else { tools.show(kind); detached.bringForward(kind) }
    }

    /// The window's toolbar shows the item: as the user arranged it where
    /// the toolbar is customizable (macOS 26), else whenever it is visible.
    private func hasToolbarItem(_ id: String, fallback: ToolbarButton) -> Bool {
        guard let toolbar = window?.toolbar, toolbar.isVisible else { return false }
        if !toolbar.allowsUserCustomization { return ToolbarSettings.shared.isShown(fallback) }
        return toolbar.items.contains { $0.itemIdentifier.rawValue == id }
    }

    /// A tool's places as a submenu, for AppKit menus.
    func placeMenuItem(for kind: ToolWindowKind) -> NSMenuItem {
        let places = NSMenu()
        let current = choice(for: kind)
        for (choice, title) in ToolChoice.all(for: kind) {
            let entry = ClosureMenuItem(title) { [weak self] in self?.choose(choice, for: kind) }
            entry.state = choice == current ? .on : .off
            places.addItem(entry)
        }
        let item = NSMenuItem(title: "\(kind.title)位置", action: nil, keyEquivalent: "")
        item.submenu = places
        return item
    }

    /// The window's layout as a menu, for a tab's context menu: each tool's
    /// places, the bottom area, the editor area alone and keeping the window in front.
    func layoutMenuItem() -> NSMenuItem {
        let menu = NSMenu()
        for kind in ToolWindowKind.allCases { menu.addItem(placeMenuItem(for: kind)) }
        menu.addItem(.separator())
        menu.addItem(ClosureMenuItem(tools.tools(in: .bottom).isEmpty ? "显示底部区域" : "隐藏底部区域") { [weak self] in self?.tools.toggleArea(.bottom) })
        menu.addItem(ClosureMenuItem(tools.isEditorOnly ? "恢复停靠区域" : "只显示编辑区") { [weak self] in self?.tools.toggleEditorOnly() })
        let pin = ClosureMenuItem("保持在最前面") { [weak self] in self?.pinned.toggle() }
        pin.state = pinned ? .on : .off
        menu.addItem(pin)
        let item = NSMenuItem(title: "布局", action: nil, keyEquivalent: "")
        item.submenu = menu
        return item
    }

    /// The keyboard goes to the composer the focused pane sends with.
    func focusComposer() {
        // Put away with its area, it comes back with it.
        if !tools.panesHaveComposer, !tools.isOpen(.composer) { tools.show(.composer) }
        if tools.sharesComposer { detached.bringForward(.composer) }
        composer(for: focused).focusWhenShown()
    }
}
