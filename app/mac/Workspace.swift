import AppKit
import Combine
import SwiftUI

/// One conversation pane of a window: its own workspace, composer and
/// reading state. A window with one pane looks as it always did.
@MainActor final class ConversationPane: ObservableObject, Identifiable {
    let id: UUID
    let store: AppStore
    /// Not observed by the pane: only the composer re-renders while typing.
    let composer = ComposerModel()
    @Published var scrollRequest: UInt64 = 0
    /// The list shows only user messages (in-place outline).
    @Published var outlineCollapsed = false
    @Published var outlineNavigation: TranscriptJump?

    init(id: UUID = UUID(), store: AppStore) {
        self.id = id
        self.store = store
    }

    func send() {
        let submitted = composer.submission
        let attachments = composer.attachments
        let text = submitted.trimmingCharacters(in: .whitespacesAndNewlines)
        guard store.canSend, !text.isEmpty || !attachments.isEmpty, !composer.isComposing else { return }
        // The message moves to the transcript at once; a turn that is not
        // sent hands its text and attachments back to the composer.
        let conversationID = store.loadedConversationID
        let sent = store.send(text: text, attachments: attachments.map(\.path), onFailure: { [composer] in
            composer.restore(submitted, attachments: attachments)
        })
        guard sent else { return }
        composer.clear(submitted: submitted, attachments: attachments, conversationID: conversationID); scrollRequest &+= 1
    }

    func navigate(to id: String) {
        outlineNavigation = TranscriptJump(id: id, token: (outlineNavigation?.token ?? 0) &+ 1)
    }
}

/// How a window's panes are arranged: a pane, or panes side by side
/// (horizontal) or stacked (vertical), each taking a fraction of the room.
indirect enum PaneLayout: Codable, Equatable {
    case pane(UUID)
    case split(PaneSplit)

    var paneIDs: [UUID] {
        switch self {
        case .pane(let id): [id]
        case .split(let split): split.children.flatMap(\.paneIDs)
        }
    }

    /// `pane` divided along `axis`, with `added` after it.
    func splitting(_ pane: UUID, axis: PaneSplit.Axis, adding added: UUID) -> PaneLayout {
        switch self {
        case .pane(let id):
            guard id == pane else { return self }
            return .split(PaneSplit(axis: axis, children: [.pane(id), .pane(added)], fractions: [0.5, 0.5]))
        case .split(var split):
            // A pane in a split along the same axis gives half its room to the new one.
            if split.axis == axis, let index = split.children.firstIndex(of: .pane(pane)) {
                let half = split.fractions[index] / 2
                split.fractions[index] = half
                split.children.insert(.pane(added), at: index + 1)
                split.fractions.insert(half, at: index + 1)
                return .split(split)
            }
            split.children = split.children.map { $0.splitting(pane, axis: axis, adding: added) }
            return .split(split)
        }
    }

    /// Without `pane`; a split left with one child becomes that child. Nil
    /// when `pane` was all there was.
    func removing(_ pane: UUID) -> PaneLayout? {
        switch self {
        case .pane(let id): return id == pane ? nil : self
        case .split(var split):
            var children: [PaneLayout] = []
            var fractions: [Double] = []
            for (child, fraction) in zip(split.children, split.fractions) {
                if let kept = child.removing(pane) { children.append(kept); fractions.append(fraction) }
            }
            guard let first = children.first else { return nil }
            if children.count == 1 { return first }
            let total = fractions.reduce(0, +)
            split.children = children
            split.fractions = fractions.map { $0 / total }
            return .split(split)
        }
    }

    /// The split at `path` (child indices from the root) with new fractions.
    func resizing(at path: [Int], fractions: [Double]) -> PaneLayout {
        guard case .split(var split) = self else { return self }
        guard let index = path.first else {
            split.fractions = fractions
            return .split(split)
        }
        split.children[index] = split.children[index].resizing(at: Array(path.dropFirst()), fractions: fractions)
        return .split(split)
    }
}

struct PaneSplit: Codable, Equatable {
    enum Axis: String, Codable { case horizontal, vertical }
    var axis: Axis
    var children: [PaneLayout]
    var fractions: [Double]
}

/// What a window restores: its arrangement, each pane's conversation and
/// which pane had focus.
struct SavedWindowLayout: Codable, Equatable {
    var layout: PaneLayout
    var conversations: [UUID: String]
    var focused: UUID
}

/// What a new window starts with.
struct WindowSeed: Codable, Hashable {
    static let sceneID = "conversation"
    /// The conversation to open in its pane.
    var conversationID: String?
}

/// A conversation window: the list with its own workspace, and the panes
/// that show conversations. The list opens conversations in the focused
/// pane; the window's toolbar and commands act on it.
@MainActor final class ConversationWindowModel: ObservableObject {
    let application: AppStore
    /// The list's workspace: list, search and selection; it opens no conversation.
    let list: AppStore
    @Published private(set) var layout: PaneLayout
    @Published private(set) var panes: [UUID: ConversationPane]
    @Published private(set) var focusedPaneID: UUID
    /// Each pane's conversation as last selected, for the layout the window restores.
    @Published private(set) var paneConversations: [UUID: String] = [:]
    private var subscriptions: [UUID: AnyCancellable] = [:]
    private var started = false
    /// The pane views by pane, for telling which one a click lands in.
    private var anchors: [UUID: () -> NSView?] = [:]
    private var mouseMonitor: Any?

    init(application: AppStore) {
        self.application = application
        list = application.makeWorkspace()
        let first = ConversationPane(store: application.makeWorkspace())
        layout = .pane(first.id)
        panes = [first.id: first]
        focusedPaneID = first.id
        follow(first)
    }

    var focused: ConversationPane { panes[focusedPaneID]! }
    var isSplit: Bool { panes.count > 1 }
    var orderedPanes: [ConversationPane] { layout.paneIDs.compactMap { panes[$0] } }

    var savedLayout: SavedWindowLayout {
        SavedWindowLayout(layout: layout, conversations: paneConversations, focused: focusedPaneID)
    }

    /// Restore a saved arrangement, or open `seed`'s conversation, then start
    /// reading. The first pane alone opens the list's first conversation.
    func start(restoring saved: SavedWindowLayout?, seed: WindowSeed?) {
        guard !started else { return }
        started = true
        list.start(loadFirstConversation: false)
        if let saved, Set(saved.layout.paneIDs).count == saved.layout.paneIDs.count, saved.layout.paneIDs.contains(saved.focused) {
            for pane in panes.values { pane.store.shutdown { _ in } }
            subscriptions = [:]
            var restored: [UUID: ConversationPane] = [:]
            for id in saved.layout.paneIDs {
                let pane = ConversationPane(id: id, store: application.makeWorkspace())
                restored[id] = pane
                follow(pane)
            }
            panes = restored
            layout = saved.layout
            focusedPaneID = saved.focused
            for pane in orderedPanes {
                let id = saved.conversations[pane.id]
                pane.store.start(loadFirstConversation: restored.count == 1, newConversation: restored.count > 1, opening: id)
            }
        } else if let id = seed?.conversationID {
            focused.store.start(loadFirstConversation: false, opening: id)
        } else {
            focused.store.start()
        }
        installMouseMonitor()
    }

    /// Open a conversation in the focused pane; one another pane of this
    /// window already shows is focused there instead.
    func open(_ conversationID: String) {
        if let pane = orderedPanes.first(where: { $0.store.selectedConversationID == conversationID }) {
            focus(pane.id)
            return
        }
        focused.store.openLinkedConversation(id: conversationID)
    }

    /// Divide the focused pane; the new pane takes focus and opens
    /// `conversationID`, or starts a new conversation.
    func split(_ axis: PaneSplit.Axis, opening conversationID: String? = nil) {
        if let conversationID, let pane = orderedPanes.first(where: { $0.store.selectedConversationID == conversationID }) {
            focus(pane.id)
            return
        }
        let pane = ConversationPane(store: application.makeWorkspace())
        panes[pane.id] = pane
        layout = layout.splitting(focusedPaneID, axis: axis, adding: pane.id)
        follow(pane)
        focusedPaneID = pane.id
        pane.store.start(loadFirstConversation: false, newConversation: conversationID == nil, opening: conversationID)
        pane.store.noteFocused()
    }

    /// Close a pane; the last one stays, since closing it is closing the window.
    func close(_ id: UUID) {
        guard isSplit, let pane = panes[id], let remaining = layout.removing(id) else { return }
        let order = layout.paneIDs
        layout = remaining
        panes[id] = nil
        subscriptions[id] = nil
        paneConversations[id] = nil
        anchors[id] = nil
        pane.store.shutdown { _ in }
        if focusedPaneID == id, let index = order.firstIndex(of: id) {
            // Focus moves to the pane before it, or the next when it was first.
            let neighbours = order.filter { $0 != id }
            focus(neighbours[max(0, min(index - 1, neighbours.count - 1))])
        }
    }

    func focus(_ id: UUID) {
        guard panes[id] != nil, focusedPaneID != id else { return }
        focusedPaneID = id
        panes[id]?.store.noteFocused()
    }

    /// Focus the pane after (or before) the focused one, in layout order.
    func focusNeighbour(forward: Bool) {
        let order = layout.paneIDs
        guard order.count > 1, let index = order.firstIndex(of: focusedPaneID) else { return }
        focus(order[(index + (forward ? 1 : order.count - 1)) % order.count])
    }

    func resize(at path: [Int], fractions: [Double]) {
        layout = layout.resizing(at: path, fractions: fractions)
    }

    func register(_ pane: UUID, anchor: @escaping () -> NSView?) { anchors[pane] = anchor }

    private func follow(_ pane: ConversationPane) {
        subscriptions[pane.id] = pane.store.$selectedConversationID.sink { [weak self] id in
            guard let self else { return }
            if self.paneConversations[pane.id] != id { self.paneConversations[pane.id] = id }
        }
    }

    /// A click in a pane focuses it, wherever in the pane it lands.
    private func installMouseMonitor() {
        guard mouseMonitor == nil else { return }
        mouseMonitor = NSEvent.addLocalMonitorForEvents(matching: [.leftMouseDown, .rightMouseDown, .otherMouseDown]) { [weak self] event in
            guard let self, self.isSplit else { return event }
            for (id, anchor) in self.anchors {
                guard let view = anchor(), let window = view.window, window === event.window else { continue }
                if view.bounds.contains(view.convert(event.locationInWindow, from: nil)) {
                    self.focus(id)
                    break
                }
            }
            return event
        }
    }
}

/// Reports the hosting view of a pane, so a click can be matched to it.
struct PaneAnchor: NSViewRepresentable {
    let register: (@escaping () -> NSView?) -> Void
    func makeNSView(context: Context) -> NSView {
        let view = PassthroughView()
        register { [weak view] in view }
        return view
    }
    func updateNSView(_ view: NSView, context: Context) {}
    private final class PassthroughView: NSView {
        override func hitTest(_ point: NSPoint) -> NSView? { nil }
    }
}
