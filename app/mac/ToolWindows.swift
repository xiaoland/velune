import AppKit
import SwiftUI

/// The parts of a conversation window that can leave their place: the list,
/// the outline and the composer. Everything else stays where it is.
enum ToolWindowKind: String, Codable, CodingKeyRepresentable, CaseIterable {
    case list, outline, composer

    var title: String {
        switch self {
        case .list: "会话列表"
        case .outline: "会话大纲"
        case .composer: "输入区"
        }
    }
}

/// Where a window's tool windows are, restored with the window.
struct ToolWindowLayout: Codable, Equatable {
    enum ListPlacement: String, Codable { case docked, floating, hidden }
    /// `toolbar`: the toolbar button shows it as the setting says (popover or in place).
    enum OutlinePlacement: String, Codable { case toolbar, docked, floating }
    enum ComposerPlacement: String, Codable { case docked, floating }

    var list = ListPlacement.docked
    var outline = OutlinePlacement.toolbar
    /// Whether a docked or floating outline is shown; the toolbar button toggles it.
    var outlineShown = true
    var composer = ComposerPlacement.docked
    /// Where each floating panel was last, kept while it is docked.
    var frames: [ToolWindowKind: ToolFrame] = [:]

    func isFloating(_ kind: ToolWindowKind) -> Bool {
        switch kind {
        case .list: list == .floating
        case .outline: outline == .floating && outlineShown
        case .composer: composer == .floating
        }
    }

    mutating func dock(_ kind: ToolWindowKind) {
        switch kind {
        case .list: list = .docked
        case .outline: outline = .docked; outlineShown = true
        case .composer: composer = .docked
        }
    }

    mutating func float(_ kind: ToolWindowKind) {
        switch kind {
        case .list: list = .floating
        case .outline: outline = .floating; outlineShown = true
        case .composer: composer = .floating
        }
    }

    /// Closing a floating panel: the list hides, the outline hides until the
    /// toolbar button shows it again, and the composer, which a pane cannot
    /// do without, goes back to the panes.
    mutating func close(_ kind: ToolWindowKind) {
        switch kind {
        case .list: list = .hidden
        case .outline: outlineShown = false
        case .composer: composer = .docked
        }
    }
}

/// A floating panel's size and place: on the screen, or, attached to its
/// window, from the window's nearer edges so it follows the window.
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
    static func initial(_ kind: ToolWindowKind, beside window: NSRect, screen: NSRect) -> ToolFrame {
        let width: CGFloat = 280
        let frame: NSRect = switch kind {
        // Inside, it leaves the window's title bar uncovered.
        case .list: window.minX - width >= screen.minX ? NSRect(x: window.minX - width, y: window.minY, width: width, height: window.height)
            : NSRect(x: window.minX, y: window.minY, width: width, height: window.height - 52)
        case .outline: window.maxX + width <= screen.maxX ? NSRect(x: window.maxX, y: window.minY, width: width, height: window.height)
            : NSRect(x: window.maxX - width, y: window.minY, width: width, height: window.height - 52)
        case .composer: NSRect(x: window.midX - 320, y: window.minY + 24, width: 640, height: 150)
        }
        return ToolFrame(frame, attachedTo: window)
    }
}

/// A floating tool window: a utility panel above its conversation window.
final class ToolPanel: NSPanel {
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
        minSize = kind == .composer ? NSSize(width: 360, height: 110) : NSSize(width: 200, height: 200)
    }

    override var canBecomeKey: Bool { true }
}

/// The floating panels of one conversation window. Panels dragged close to
/// the window's edges, or to a panel attached to it, snap there and follow
/// the window; dragged into the window's dock zone for them, they dock.
@MainActor final class ToolPanels: NSObject, NSWindowDelegate {
    private unowned let model: ConversationWindowModel
    private var panels: [ToolWindowKind: ToolPanel] = [:]
    private weak var main: NSWindow?
    private var observers: [NSObjectProtocol] = []
    /// Set while a panel moves because the window moved or it was snapped.
    private var placing = false
    private var dragging: Set<ToolWindowKind> = []
    private var syncScheduled = false
    private static let snapDistance: CGFloat = 14
    private static let dockDepth: CGFloat = 110

    init(model: ConversationWindowModel) {
        self.model = model
    }

    /// Open and close panels to match the layout, once the window exists.
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
            if model.tools.isFloating(kind) { open(kind, beside: window) } else { closePanel(kind) }
        }
    }

    func closeAll() {
        for kind in Array(panels.keys) { closePanel(kind) }
        observers.forEach(NotificationCenter.default.removeObserver)
        observers = []
    }

    func isPanel(_ window: NSWindow) -> Bool { panels.values.contains { $0 === window } }

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
                MainActor.assumeIsolated { self?.panels.values.forEach { $0.orderOut(nil) } }
            },
            center.addObserver(forName: NSWindow.didDeminiaturizeNotification, object: window, queue: .main) { [weak self] _ in
                MainActor.assumeIsolated { self?.showPanels() }
            },
        ]
    }

    private func mainChanged(_ window: NSWindow?) {
        guard let window, let main else { return }
        if window === main || isPanel(window) { showPanels() }
        else if ConversationWindowModel.isConversationWindow(window) { panels.values.forEach { $0.orderOut(nil) } }
    }

    private func showPanels() {
        guard let main, main.isVisible, !main.isMiniaturized else { return }
        for panel in panels.values where !panel.isVisible { panel.orderFront(nil) }
    }

    private func open(_ kind: ToolWindowKind, beside window: NSWindow) {
        guard panels[kind] == nil else { return }
        let panel = ToolPanel(kind: kind, model: model)
        let hosting = NSHostingView(rootView: ToolPanelContent(kind: kind, window: model))
        // The panel's size is the user's, not the content's.
        hosting.sizingOptions = []
        panel.contentView = hosting
        panel.delegate = self
        let accessory = NSTitlebarAccessoryViewController()
        let dock = NSButton(title: "停靠", target: self, action: #selector(dockPressed(_:)))
        dock.bezelStyle = .accessoryBarAction
        dock.controlSize = .small
        dock.font = .systemFont(ofSize: NSFont.smallSystemFontSize)
        dock.tag = ToolWindowKind.allCases.firstIndex(of: kind)!
        dock.toolTip = "放回窗口中（也可把面板拖进窗口\(kind == .list ? "左侧" : kind == .outline ? "右侧" : "底部")）"
        accessory.view = dock
        accessory.layoutAttribute = .trailing
        panel.addTitlebarAccessoryViewController(accessory)
        panels[kind] = panel
        let saved = model.tools.frames[kind] ?? ToolFrame.initial(kind, beside: window.frame, screen: window.screen?.visibleFrame ?? window.frame)
        placing = true
        panel.setFrame(saved.frame(beside: window.frame), display: false)
        placing = false
        if saved.attachment != nil { window.addChildWindow(panel, ordered: .above) }
        if model.tools.frames[kind] == nil { model.tools.frames[kind] = saved }
        if window.isVisible { panel.orderFront(nil) }
    }

    private func closePanel(_ kind: ToolWindowKind) {
        guard let panel = panels.removeValue(forKey: kind) else { return }
        panel.parent?.removeChildWindow(panel)
        panel.delegate = nil
        panel.orderOut(nil)
        panel.close()
    }

    @objc private func dockPressed(_ sender: NSButton) {
        model.tools.dock(ToolWindowKind.allCases[sender.tag])
    }

    /// Attached panels keep their place from the window's edges when it resizes.
    private func followWindow() {
        guard let main else { return }
        placing = true
        for (kind, panel) in panels {
            guard let frame = model.tools.frames[kind], frame.attachment != nil else { continue }
            panel.setFrame(frame.frame(beside: main.frame), display: true)
        }
        placing = false
    }

    func windowShouldClose(_ sender: NSWindow) -> Bool {
        guard let panel = sender as? ToolPanel else { return true }
        model.tools.close(panel.kind)
        return false
    }

    func windowDidResize(_ notification: Notification) {
        guard !placing, let panel = notification.object as? ToolPanel, panel.inLiveResize else { return }
        record(panel)
    }

    func windowDidMove(_ notification: Notification) {
        guard !placing, let panel = notification.object as? ToolPanel, NSEvent.pressedMouseButtons & 1 != 0, !following(panel) else { return }
        // The title bar drag is the window server's; it ends when the button comes up.
        model.dockHint = dockZone(for: panel.kind)?.contains(NSEvent.mouseLocation) == true ? panel.kind : nil
        guard !dragging.contains(panel.kind) else { return }
        dragging.insert(panel.kind)
        waitForDragEnd(panel)
    }

    /// An attached panel carried along by its window, not dragged itself.
    private func following(_ panel: ToolPanel) -> Bool {
        guard let main, panel.parent === main, let frame = model.tools.frames[panel.kind], frame.attachment != nil else { return false }
        let expected = frame.frame(beside: main.frame)
        return abs(expected.minX - panel.frame.minX) < 1 && abs(expected.minY - panel.frame.minY) < 1
    }

    private func waitForDragEnd(_ panel: ToolPanel) {
        DispatchQueue.main.asyncAfter(deadline: .now() + 0.05) { [weak self, weak panel] in
            guard let self, let panel else { return }
            if NSEvent.pressedMouseButtons & 1 != 0 {
                model.dockHint = dockZone(for: panel.kind)?.contains(NSEvent.mouseLocation) == true ? panel.kind : nil
                waitForDragEnd(panel)
                return
            }
            dragging.remove(panel.kind)
            let docking = model.dockHint == panel.kind
            model.dockHint = nil
            if docking { model.tools.dock(panel.kind) } else { snap(panel) }
        }
    }

    /// The strip of the window a panel docks into: the list's leading edge,
    /// the outline's trailing edge, the composer's bottom.
    private func dockZone(for kind: ToolWindowKind) -> NSRect? {
        guard let frame = main?.frame else { return nil }
        let depth = Self.dockDepth
        return switch kind {
        case .list: NSRect(x: frame.minX, y: frame.minY, width: depth, height: frame.height)
        case .outline: NSRect(x: frame.maxX - depth, y: frame.minY, width: depth, height: frame.height)
        case .composer: NSRect(x: frame.minX, y: frame.minY, width: frame.width, height: depth)
        }
    }

    /// Close to an edge of the window or of a panel attached to it, the panel
    /// meets that edge (lining up with it where they run side by side) and
    /// follows the window; elsewhere it floats free.
    private func snap(_ panel: ToolPanel) {
        guard let main else { return }
        let attached = panels.filter { $0.key != panel.kind && $0.value.parent === main }.map(\.value.frame)
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

    private func record(_ panel: ToolPanel) {
        guard let main else { return }
        model.tools.frames[panel.kind] = ToolFrame(panel.frame, attachedTo: panel.parent === main ? main.frame : nil)
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

/// A floating panel's content, following the window's focused pane.
private struct ToolPanelContent: View {
    let kind: ToolWindowKind
    @ObservedObject var window: ConversationWindowModel

    var body: some View {
        let pane = window.focused
        switch kind {
        case .list:
            ConversationListView(window: window, store: window.list, focused: pane.store, placement: .panel)
        case .outline:
            ToolOutline(pane: pane, store: pane.store).id(pane.id)
        case .composer:
            ToolComposer(window: window, pane: pane, store: pane.store).id(pane.id)
        }
    }
}

/// The focused pane's outline, beside the panes or floating.
struct ToolOutline: View {
    @ObservedObject var pane: ConversationPane
    @ObservedObject var store: AppStore

    var body: some View {
        if store.pendingConversationID != nil || store.transcript.userRows.isEmpty {
            ContentUnavailableView("没有用户消息", systemImage: "list.bullet", description: Text("会话有了用户消息后，在这里列出并可跳转。"))
        } else {
            TranscriptOutlineView(model: store.transcript, inPopover: false) { pane.navigate(to: $0) }
        }
    }
}

/// One composer for the window, sending to the focused pane.
private struct ToolComposer: View {
    @ObservedObject var window: ConversationWindowModel
    @ObservedObject var pane: ConversationPane
    @ObservedObject var store: AppStore

    var body: some View {
        VStack(alignment: .leading, spacing: 0) {
            if window.isSplit {
                Text("发往：\(store.selectedConversationTitle ?? "新会话")")
                    .font(.caption).foregroundStyle(.secondary).lineLimit(1).truncationMode(.middle)
                    .padding(.horizontal, 16).padding(.top, 8)
            }
            if store.loadedParentConversationID != nil, store.pendingConversationID == nil {
                Text("这是原生 sub-agent 的会话，由启动它的会话继续。").foregroundStyle(.secondary)
                    .frame(maxWidth: .infinity, maxHeight: .infinity)
            } else {
                Spacer(minLength: 0)
                ComposerView(store: store, model: pane.composer, send: pane.send)
            }
        }
    }
}
