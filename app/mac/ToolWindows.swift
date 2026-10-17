import AppKit
import SwiftUI
import VeluneBindings

/// The parts of a conversation window that can leave their place: the list,
/// the outline, the composer and the sub-agents. Everything else stays where it is.
enum ToolWindowKind: String, Codable, CodingKeyRepresentable, CaseIterable {
    case list, outline, composer, subAgents

    var title: String {
        switch self {
        case .list: "会话列表"
        case .outline: "会话大纲"
        case .composer: "输入区"
        case .subAgents: "子 Agent"
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
    /// The tool's own place: the outline and the sub-agents from their
    /// toolbar buttons, the composer in each pane. The list has none.
    case builtIn
    case docked(DockSide)
    /// A utility panel above its window that snaps to it and follows it.
    case floating
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
    var panelFrame: ToolFrame?
    var windowFrame: ToolFrame?
    /// As a window of its own, kept in front of other apps' windows.
    var pinned = false
}

/// A tool's places as menus offer them. The outline's own place is the
/// toolbar button, which pops it over or collapses the pane in place, as an
/// application setting says.
enum ToolChoice: Hashable {
    case place(ToolPlace)
    case outline(BindingOutlinePresentation)

    static func all(for kind: ToolWindowKind) -> [(choice: ToolChoice, title: String)] {
        let docked: [(ToolChoice, String)] = (kind == .composer ? [DockSide.bottom, .left, .right] : DockSide.allCases)
            .map { (.place(.docked($0)), "停靠在\($0.title)") }
        let detached: [(ToolChoice, String)] = [(.place(.floating), "浮动"), (.place(.window), "独立窗口")]
        return switch kind {
        case .list: docked + detached
        case .outline: [(.outline(.popover), "弹出"), (.outline(.inPlace), "就地折叠")] + docked + detached
        case .composer: [(.place(.builtIn), "在每个面板中")] + docked + detached
        case .subAgents: [(.place(.builtIn), "弹出")] + docked + detached
        }
    }
}

/// Where a window's tool windows are, restored with the window. A docked
/// tool shows while it is shown and its area is not collapsed.
struct ToolWindowLayout: Codable, Equatable {
    var tools: [ToolWindowKind: ToolWindowState] = [.list: ToolWindowState(place: .docked(.left)), .outline: ToolWindowState(place: .builtIn), .composer: ToolWindowState(place: .builtIn)]
    var collapsed: Set<DockSide> = []
    var bottomHeight = 220.0
    /// The composer's text box below the panes, when dragged taller than its text.
    var composerHeight: Double?
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
        case .floating, .window: true
        }
    }

    /// The tools an area shows, in a fixed order.
    func tools(in side: DockSide) -> [ToolWindowKind] {
        collapsed.contains(side) ? [] : docked(in: side).filter { self[$0].shown }
    }

    func docked(in side: DockSide) -> [ToolWindowKind] {
        ToolWindowKind.allCases.filter { self[$0].place == .docked(side) }
    }

    /// The window's one composer is showing; otherwise each pane has its own.
    var sharesComposer: Bool { isOpen(.composer) }
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
    mutating func hide(_ kind: ToolWindowKind) {
        if kind == .composer { self[kind].place = .builtIn } else { self[kind].shown = false }
    }

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
        if tools(in: side).isEmpty { showArea(side) } else { collapse(side) }
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
}

/// The tool windows as last arranged in any window, for each window that
/// opens without a layout of its own. The system brings back a window's own
/// layout only when it reopens the window, which by default it does not after
/// quitting, so this one is kept in `VELUNE_HOME`. "Editor only" is a window's
/// passing state and is left out.
@MainActor final class RememberedToolLayout {
    private struct File: Codable {
        static let schema = 1
        var schema = File.schema
        var tools: ToolWindowLayout
    }
    private(set) var layout: ToolWindowLayout
    private let url: URL?
    private let failed: (Error) -> Void
    private var pending: DispatchWorkItem?

    /// Any other schema or an unreadable file starts from the default and is replaced on the next change.
    init(home: URL?, failed: @escaping (Error) -> Void) {
        url = home?.appendingPathComponent("mac-tool-layout.json")
        self.failed = failed
        let file = url.flatMap { try? Data(contentsOf: $0) }.flatMap { try? JSONDecoder().decode(File.self, from: $0) }
        layout = file.flatMap { $0.schema == File.schema ? $0.tools.normalized() : nil } ?? ToolWindowLayout()
    }

    /// Kept a moment later, so a divider being dragged writes once.
    func remember(_ tools: ToolWindowLayout) {
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
        guard let url else { return }
        do {
            try FileManager.default.createDirectory(at: url.deletingLastPathComponent(), withIntermediateDirectories: true)
            try JSONEncoder().encode(File(tools: layout)).write(to: url, options: .atomic)
        } catch { failed(error) }
    }
}

/// A floating panel's or tool window's size and place: on the screen, or,
/// attached to its window, from the window's nearer edges so it follows the window.
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

    /// Where a panel first floats: the list and outline beside the window's
    /// leading and trailing edges (just inside them when the screen ends
    /// there), the composer over its bottom.
    static func initialPanel(_ kind: ToolWindowKind, beside window: NSRect, screen: NSRect) -> ToolFrame {
        let width: CGFloat = 280
        let frame: NSRect = switch kind {
        // Inside, it leaves the window's title bar uncovered.
        case .list: window.minX - width >= screen.minX ? NSRect(x: window.minX - width, y: window.minY, width: width, height: window.height)
            : NSRect(x: window.minX, y: window.minY, width: width, height: window.height - 52)
        case .outline, .subAgents: window.maxX + width <= screen.maxX ? NSRect(x: window.maxX, y: window.minY, width: width, height: window.height)
            : NSRect(x: window.maxX - width, y: window.minY, width: width, height: window.height - 52)
        case .composer: NSRect(x: window.midX - 320, y: window.minY + 24, width: 640, height: 120)
        }
        return ToolFrame(frame, attachedTo: window)
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

/// A tool window outside its conversation window.
@MainActor protocol DetachedTool: NSWindow {
    var kind: ToolWindowKind { get }
    var model: ConversationWindowModel? { get }
}

/// A floating tool window: a utility panel above its conversation window.
final class ToolPanel: NSPanel, DetachedTool {
    let kind: ToolWindowKind
    weak var model: ConversationWindowModel?

    init(kind: ToolWindowKind, model: ConversationWindowModel) {
        self.kind = kind
        self.model = model
        super.init(contentRect: NSRect(x: 0, y: 0, width: 280, height: 400),
                   styleMask: [.titled, .closable, .resizable, .utilityWindow], backing: .buffered, defer: false)
        title = kind.title
        isFloatingPanel = true
        hidesOnDeactivate = true
        becomesKeyOnlyIfNeeded = false
        isReleasedWhenClosed = false
        collectionBehavior = [.fullScreenAuxiliary]
        minSize = kind == .composer ? NSSize(width: 360, height: 60) : NSSize(width: 200, height: 200)
    }

    override var canBecomeKey: Bool { true }
}

/// A tool window of its own: in Mission Control and the Window menu, able
/// to go full screen.
final class ToolWindow: NSWindow, DetachedTool {
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

/// The tool windows of one conversation window that are outside it. Floating
/// panels dragged close to the window's edges, or to a panel attached to it,
/// snap there and follow the window; with the pointer at its left, right or
/// bottom edge they dock into that area.
@MainActor final class DetachedTools: NSObject, NSWindowDelegate {
    private unowned let model: ConversationWindowModel
    private var hosts: [ToolWindowKind: any DetachedTool] = [:]
    private weak var main: NSWindow?
    private var observers: [NSObjectProtocol] = []
    /// Set while a panel moves because the window moved or it was snapped.
    private var placing = false
    private var dragging: Set<ToolWindowKind> = []
    private var syncScheduled = false
    private static let snapDistance: CGFloat = 14
    private static let dockDepth: CGFloat = 80

    init(model: ConversationWindowModel) {
        self.model = model
    }

    /// Open and close panels and windows to match the layout, once the window exists.
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
            let place = model.tools.isOpen(kind) ? model.tools[kind].place : nil
            let wanted: Bool? = switch place {
            case .floating: true
            case .window: false
            default: nil
            }
            if let host = hosts[kind], wanted != (host is ToolPanel) { close(kind) }
            if let wanted, hosts[kind] == nil { open(kind, floating: wanted, beside: window) }
            if let host = hosts[kind] as? ToolWindow { WindowPinning.apply(model.tools[kind].pinned, to: host) }
        }
    }

    func closeAll() {
        for kind in Array(hosts.keys) { close(kind) }
        observers.forEach(NotificationCenter.default.removeObserver)
        observers = []
    }

    func isHost(_ window: NSWindow) -> Bool { hosts.values.contains { $0 === window } }

    /// A shown tool window comes forward and takes the keyboard.
    func bringForward(_ kind: ToolWindowKind) {
        hosts[kind]?.makeKeyAndOrderFront(nil)
    }

    /// A tool window's subtitle names the conversation its window shows.
    func setSubtitle(_ title: String, for kind: ToolWindowKind) {
        if let window = hosts[kind] as? ToolWindow, window.subtitle != title { window.subtitle = title }
    }

    private var panels: [ToolPanel] { hosts.values.compactMap { $0 as? ToolPanel } }

    private func attach(to window: NSWindow) {
        observers.forEach(NotificationCenter.default.removeObserver)
        main = window
        let center = NotificationCenter.default
        observers = [
            center.addObserver(forName: NSWindow.willCloseNotification, object: window, queue: .main) { [weak self] _ in
                MainActor.assumeIsolated { self?.closeAll() }
            },
            center.addObserver(forName: NSWindow.didResizeNotification, object: window, queue: .main) { [weak self] _ in
                MainActor.assumeIsolated { self?.followWindow() }
            },
            // Another conversation window coming forward takes its own panels; this one's step back.
            center.addObserver(forName: NSWindow.didBecomeMainNotification, object: nil, queue: .main) { [weak self] notification in
                MainActor.assumeIsolated { self?.mainChanged(notification.object as? NSWindow) }
            },
            center.addObserver(forName: NSWindow.didMiniaturizeNotification, object: window, queue: .main) { [weak self] _ in
                MainActor.assumeIsolated { self?.panels.forEach { $0.orderOut(nil) } }
            },
            center.addObserver(forName: NSWindow.didDeminiaturizeNotification, object: window, queue: .main) { [weak self] _ in
                MainActor.assumeIsolated { self?.showPanels() }
            },
        ]
    }

    private func mainChanged(_ window: NSWindow?) {
        guard let window, let main else { return }
        if window === main || isHost(window) { showPanels() }
        else if ConversationWindowModel.isConversationWindow(window) { panels.forEach { $0.orderOut(nil) } }
    }

    private func showPanels() {
        guard let main, main.isVisible, !main.isMiniaturized else { return }
        for panel in panels where !panel.isVisible { panel.orderFront(nil) }
    }

    private func open(_ kind: ToolWindowKind, floating: Bool, beside window: NSWindow) {
        let host: any DetachedTool = floating ? ToolPanel(kind: kind, model: model) : ToolWindow(kind: kind, model: model)
        let hosting = NSHostingView(rootView: DetachedToolContent(kind: kind, window: model))
        // The window's size is the user's, not the content's.
        hosting.sizingOptions = []
        host.contentView = hosting
        host.delegate = self
        host.addTitlebarAccessoryViewController(placeAccessory(for: kind, floating: floating))
        hosts[kind] = host
        let screen = window.screen?.visibleFrame ?? window.frame
        let saved = floating ? model.tools[kind].panelFrame ?? .initialPanel(kind, beside: window.frame, screen: screen)
            : model.tools[kind].windowFrame ?? .initialWindow(kind, beside: window.frame, screen: screen)
        placing = true
        host.setFrame(saved.frame(beside: window.frame), display: false)
        placing = false
        if floating, saved.attachment != nil { window.addChildWindow(host, ordered: .above) }
        if floating, model.tools[kind].panelFrame == nil { model.tools[kind].panelFrame = saved }
        if !floating, model.tools[kind].windowFrame == nil { model.tools[kind].windowFrame = saved }
        if window.isVisible { host.orderFront(nil) }
    }

    /// A borderless symbol at the title bar's trailing end, as title bar
    /// controls are, opening a menu of the tool's other places.
    private func placeAccessory(for kind: ToolWindowKind, floating: Bool) -> NSTitlebarAccessoryViewController {
        let image = NSImage(systemSymbolName: "pip.enter", accessibilityDescription: "位置")?
            .withSymbolConfiguration(NSImage.SymbolConfiguration(pointSize: 11, weight: .regular))
        let button = NSButton(image: image ?? NSImage(), target: self, action: #selector(placePressed(_:)))
        button.isBordered = false
        button.imagePosition = .imageOnly
        button.tag = ToolWindowKind.allCases.firstIndex(of: kind)!
        button.toolTip = "停靠或改为\(floating ? "独立窗口" : "浮动")（也可把浮动面板拖到窗口的左、右或底边）"
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
            if place == .floating || place == .window { menu.addItem(.separator()) }
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

    /// The floating composer is as tall as its content, growing upward as the docked one does.
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
        guard (sender as? any DetachedTool)?.kind == .composer else { return size }
        return NSSize(width: size.width, height: sender.frame.height)
    }

    private func close(_ kind: ToolWindowKind) {
        guard let host = hosts.removeValue(forKey: kind) else { return }
        host.parent?.removeChildWindow(host)
        host.delegate = nil
        host.orderOut(nil)
        host.close()
    }

    /// Attached panels keep their place from the window's edges when it resizes.
    private func followWindow() {
        guard let main else { return }
        placing = true
        for panel in panels {
            guard let frame = model.tools[panel.kind].panelFrame, frame.attachment != nil else { continue }
            panel.setFrame(frame.frame(beside: main.frame), display: true)
        }
        placing = false
    }

    func windowShouldClose(_ sender: NSWindow) -> Bool {
        guard let host = sender as? any DetachedTool else { return true }
        model.tools.hide(host.kind)
        return false
    }

    // Recorded once a resize or a window's move ends: the layout is
    // published, and every publication lays the window's toolbar out again.
    func windowDidEndLiveResize(_ notification: Notification) {
        guard let host = notification.object as? any DetachedTool else { return }
        record(host)
    }

    /// Published only when it changes, for the same reason.
    private func updateDockHint() {
        let hint = dockSide(at: NSEvent.mouseLocation)
        if model.dockHint != hint { model.dockHint = hint }
    }

    func windowDidMove(_ notification: Notification) {
        guard !placing, let host = notification.object as? any DetachedTool, NSEvent.pressedMouseButtons & 1 != 0 else { return }
        let panel = host as? ToolPanel
        if let panel, following(panel) { return }
        // The title bar drag is the window server's; it ends when the button comes up.
        if panel != nil { updateDockHint() }
        guard !dragging.contains(host.kind) else { return }
        dragging.insert(host.kind)
        waitForDragEnd(host)
    }

    /// An attached panel carried along by its window, not dragged itself.
    private func following(_ panel: ToolPanel) -> Bool {
        guard let main, panel.parent === main, let frame = model.tools[panel.kind].panelFrame, frame.attachment != nil else { return false }
        let expected = frame.frame(beside: main.frame)
        return abs(expected.minX - panel.frame.minX) < 1 && abs(expected.minY - panel.frame.minY) < 1
    }

    private func waitForDragEnd(_ host: any DetachedTool) {
        DispatchQueue.main.asyncAfter(deadline: .now() + 0.05) { [weak self, weak host] in
            guard let self, let host else { return }
            let panel = host as? ToolPanel
            if NSEvent.pressedMouseButtons & 1 != 0 {
                if panel != nil { updateDockHint() }
                waitForDragEnd(host)
                return
            }
            dragging.remove(host.kind)
            guard let panel else { record(host); return }
            let side = model.dockHint
            if side != nil { model.dockHint = nil }
            if let side { model.tools.move(panel.kind, to: .docked(side)) } else { snap(panel) }
        }
    }

    /// The edge strip of the window the pointer is in: a panel dropped there docks into that area.
    private func dockSide(at point: NSPoint) -> DockSide? {
        guard let frame = main?.frame, frame.contains(point) else { return nil }
        let depth = Self.dockDepth
        if point.x < frame.minX + depth { return .left }
        if point.x > frame.maxX - depth { return .right }
        if point.y < frame.minY + depth { return .bottom }
        return nil
    }

    /// Close to an edge of the window or of a panel attached to it, the panel
    /// meets that edge (lining up with it where they run side by side) and
    /// follows the window; elsewhere it floats free.
    private func snap(_ panel: ToolPanel) {
        guard let main else { return }
        let attached = panels.filter { $0 !== panel && $0.parent === main }.map(\.frame)
        var frame = panel.frame
        var snapped = false
        for (index, target) in ([main.frame] + attached).enumerated() {
            if let moved = Self.snapped(frame, to: target, inside: index == 0) { frame = moved; snapped = true; break }
        }
        placing = true
        panel.setFrame(frame, display: true, animate: snapped)
        placing = false
        if snapped, panel.parent !== main { main.addChildWindow(panel, ordered: .above) }
        if !snapped, let parent = panel.parent { parent.removeChildWindow(panel) }
        record(panel)
    }

    private func record(_ host: any DetachedTool) {
        guard let main else { return }
        if host is ToolPanel {
            model.tools[host.kind].panelFrame = ToolFrame(host.frame, attachedTo: host.parent === main ? main.frame : nil)
        } else {
            model.tools[host.kind].windowFrame = ToolFrame(host.frame, attachedTo: nil)
        }
    }

    /// `frame` moved onto an edge of `target` it is near: beside it, or, for
    /// the window itself, just inside it.
    static func snapped(_ frame: NSRect, to target: NSRect, inside: Bool) -> NSRect? {
        let distance = snapDistance
        var result = frame
        var meets = false
        let overlapsVertically = frame.maxY > target.minY && frame.minY < target.maxY
        let overlapsHorizontally = frame.maxX > target.minX && frame.minX < target.maxX
        if overlapsVertically {
            var x: CGFloat?
            if abs(frame.minX - target.maxX) < distance { x = target.maxX }
            else if abs(frame.maxX - target.minX) < distance { x = target.minX - frame.width }
            else if inside, abs(frame.minX - target.minX) < distance { x = target.minX }
            else if inside, abs(frame.maxX - target.maxX) < distance { x = target.maxX - frame.width }
            if let x {
                result.origin.x = x
                meets = true
                if abs(frame.maxY - target.maxY) < distance { result.origin.y = target.maxY - frame.height }
                else if abs(frame.minY - target.minY) < distance { result.origin.y = target.minY }
            }
        }
        if !meets, overlapsHorizontally {
            var y: CGFloat?
            if abs(frame.minY - target.maxY) < distance { y = target.maxY }
            else if abs(frame.maxY - target.minY) < distance { y = target.minY - frame.height }
            else if inside, abs(frame.minY - target.minY) < distance { y = target.minY }
            else if inside, abs(frame.maxY - target.maxY) < distance { y = target.maxY - frame.height }
            if let y {
                result.origin.y = y
                meets = true
                if abs(frame.minX - target.minX) < distance { result.origin.x = target.minX }
                else if abs(frame.maxX - target.maxX) < distance { result.origin.x = target.maxX - frame.width }
            }
        }
        return meets ? result : nil
    }
}

extension ConversationWindowModel {
    /// The choice a tool's menus show as made.
    func choice(for kind: ToolWindowKind) -> ToolChoice {
        let place = tools[kind].place
        if kind == .outline, place == .builtIn { return .outline(application.outlinePresentation) }
        return .place(place)
    }

    func choose(_ choice: ToolChoice, for kind: ToolWindowKind) {
        switch choice {
        case .place(let place): tools.move(kind, to: place)
        case .outline(let presentation):
            tools.move(.outline, to: .builtIn)
            if application.outlinePresentation != presentation { application.setOutlinePresentation(presentation) }
        }
    }

    /// Whether showing or hiding a tool can act. The sub-agents in their own
    /// place pop over from a toolbar button offered only while the focused
    /// conversation has some.
    func canToggle(_ kind: ToolWindowKind) -> Bool {
        kind != .subAgents || tools[.subAgents].place != .builtIn || subAgentsPopover || !focused.store.subAgents.isEmpty
    }

    /// Show a tool, or hide it when it shows. The outline in its own place
    /// pops over or collapses the focused pane in place; the sub-agents pop
    /// over, or, with their button customized out of the toolbar or the
    /// toolbar hidden, move to a floating panel, since nothing would pop over.
    func toggle(_ kind: ToolWindowKind) {
        if kind == .subAgents, tools[.subAgents].place == .builtIn {
            if subAgentsPopover { subAgentsPopover = false }
            else if focused.store.subAgents.isEmpty { return }
            else if hasToolbarItem("sub-agents") { subAgentsPopover = true }
            else { tools.move(.subAgents, to: .floating); detached.bringForward(.subAgents) }
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

    /// The window's toolbar shows the item, as customized.
    private func hasToolbarItem(_ id: String) -> Bool {
        guard let toolbar = window?.toolbar, toolbar.isVisible else { return false }
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
        if tools.sharesComposer { detached.bringForward(.composer) }
        focused.composer.focus()
    }
}
