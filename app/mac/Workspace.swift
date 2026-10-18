import AppKit
import Combine
import SwiftUI
import VeluneBindings

/// One conversation tab: its own workspace, composer and reading state. A
/// tab keeps them while it is in the background and when it moves to another
/// pane or window.
@MainActor final class ConversationTab: ObservableObject, Identifiable {
    let id: UUID
    let store: AppStore
    /// Not observed by the tab: only the composer re-renders while typing.
    let composer = ComposerModel()
    @Published var scrollRequest: UInt64 = 0
    /// The list shows only user messages (in-place outline).
    @Published var outlineCollapsed = false
    @Published var outlineNavigation: TranscriptJump?
    /// Searching the conversation's messages (see `TranscriptFindBar`).
    let find = TranscriptFind()
    /// The user message being edited, in a sheet over the tab.
    @Published var editing: MessageEditRequest?
    /// Kept while the tab is behind another, with the scroll position.
    let reading = TranscriptReading()
    /// Reading starts when the tab is first shown, so restored background tabs cost nothing.
    private var pendingStart: (() -> Void)?

    init(id: UUID = UUID(), store: AppStore, start: @escaping () -> Void) {
        self.id = id
        self.store = store
        pendingStart = start
    }

    func activate() {
        guard let start = pendingStart else { return }
        pendingStart = nil
        start()
    }

    func send() {
        let submitted = composer.submission
        let attachments = composer.attachments
        let text = submitted.trimmingCharacters(in: .whitespacesAndNewlines)
        guard !text.isEmpty || !attachments.isEmpty, !composer.isComposing else { return }
        // While a turn runs the message waits in the queue above the composer.
        if store.canQueue {
            let conversationID = store.loadedConversationID
            guard store.queue(text: text, attachments: attachments.map(\.path), onFailure: { [composer] in
                composer.restore(submitted, attachments: attachments)
            }) else { return }
            composer.clear(submitted: submitted, attachments: attachments, conversationID: conversationID)
            return
        }
        guard store.canSend else { return }
        // The message moves to the transcript at once; a turn that is not
        // sent hands its text and attachments back to the composer.
        let conversationID = store.loadedConversationID
        let sent = store.send(text: text, attachments: attachments.map(\.path), onFailure: { [composer] in
            composer.restore(submitted, attachments: attachments)
        })
        guard sent else { return }
        composer.clear(submitted: submitted, attachments: attachments, conversationID: conversationID); scrollRequest &+= 1
    }

    /// Development check of the send queue without driving the interface:
    /// send a turn, then queue messages while it runs.
    func checkSendQueue(step: Int = 0, attempts: Int = 0) {
        guard attempts < 120 else { return }
        DispatchQueue.main.asyncAfter(deadline: .now() + 0.5) { [self] in
            let texts = ["第一条排队消息", "第二条排队消息：比较长的一段文字，用来看两行截断在窄面板里是否得体，以及悬停时删除按钮的位置。", "第三条"]
            let ready = step == 0 ? store.canSend : store.canQueue
            guard ready, step == 0 ? store.send(text: "TURN_HOLD") : store.queue(text: texts[step - 1]) else {
                return checkSendQueue(step: step, attempts: attempts + 1)
            }
            if step < texts.count { checkSendQueue(step: step + 1) }
        }
    }

    /// Development check of message editing without driving the interface:
    /// open the editor on the conversation's second user message.
    func checkEditMessage(attempts: Int = 0) {
        guard attempts < 120 else { return }
        DispatchQueue.main.asyncAfter(deadline: .now() + 0.5) { [self] in
            let users = transcript(of: store).filter { $0.role == .user }
            guard users.count > 1, store.editability(of: users[1].id) != nil else { return checkEditMessage(attempts: attempts + 1) }
            edit(messageID: users[1].id)
        }
    }
    private func transcript(of store: AppStore) -> [Message] { store.transcript.rows.map(\.message) }

    /// Go back to the conversation shown before, where reading stood in it.
    func goBack() {
        reading.table.restoring = store.backHistory.last
        store.goBack()
    }
    func goForward() {
        reading.table.restoring = store.forwardHistory.last
        store.goForward()
    }

    /// Opens the find bar on the shown conversation, or brings its field forward.
    func beginFind() {
        find.isShown = true
        find.focusRequest &+= 1
    }

    func navigate(to id: String) {
        outlineNavigation = TranscriptJump(id: id, token: (outlineNavigation?.token ?? 0) &+ 1)
    }

    /// Edit a user message of the transcript, when it may be edited now.
    func edit(messageID: String) {
        guard let editable = store.editability(of: messageID), let message = store.message(id: messageID) else { return }
        editing = MessageEditRequest(message: message, inPlace: editable.inPlace, isLast: store.isLastUserMessage(messageID))
    }

    /// The link a drag carries to other apps; an unsent conversation has none yet.
    var link: URL? {
        guard let id = store.loadedConversationID, !store.isUnsentConversation,
              let runtime = store.conversation(id: id)?.runtimeID ?? store.projectionRuntimeID else { return nil }
        return ConversationLink.url(runtimeID: runtime, conversationID: id)
    }

    var title: String {
        store.selectedConversationTitle ?? store.reachingConversationID.map { store.conversation(id: $0)?.title ?? "正在打开…" } ?? "新会话"
    }
}

/// A leaf of a window's split: its tabs and the one it shows.
@MainActor final class Pane: ObservableObject, Identifiable {
    let id: UUID
    @Published private(set) var tabs: [ConversationTab]
    @Published private(set) var selectedID: UUID

    init(id: UUID = UUID(), tabs: [ConversationTab], selected: UUID? = nil) {
        self.id = id
        self.tabs = tabs
        selectedID = selected.flatMap { id in tabs.contains { $0.id == id } ? id : nil } ?? tabs[0].id
    }

    var selected: ConversationTab { tabs.first { $0.id == selectedID } ?? tabs[0] }

    func select(_ id: UUID) {
        guard let tab = tabs.first(where: { $0.id == id }) else { return }
        if selectedID != id {
            // Typing that went to the tab put behind goes to the one shown.
            if selected.composer.isFocused { tab.composer.focusWhenShown() }
            selectedID = id
        }
        tab.activate()
    }

    /// `tab` at `index` (the end when nil), shown.
    func insert(_ tab: ConversationTab, at index: Int? = nil) {
        tabs.insert(tab, at: min(index ?? tabs.count, tabs.count))
        select(tab.id)
    }

    /// Without `id`; the tab after it shows, or the one before when it was
    /// last. False when it was the only tab, which stays.
    @discardableResult func remove(_ id: UUID) -> Bool {
        guard tabs.count > 1, let index = tabs.firstIndex(where: { $0.id == id }) else { return false }
        tabs.remove(at: index)
        if selectedID == id { select(tabs[min(index, tabs.count - 1)].id) }
        return true
    }

    func move(_ id: UUID, to index: Int) {
        guard let from = tabs.firstIndex(where: { $0.id == id }) else { return }
        let tab = tabs.remove(at: from)
        tabs.insert(tab, at: min(from < index ? index - 1 : index, tabs.count))
        select(id)
    }
}

/// Which side of a pane something dropped on it goes to.
enum PaneEdge: Equatable {
    case leading, trailing, top, bottom
    var axis: PaneSplit.Axis { self == .leading || self == .trailing ? .horizontal : .vertical }
    var after: Bool { self == .trailing || self == .bottom }
}

/// Where a tab dropped on a pane goes: among its tabs, or beside an edge.
enum PaneDropZone: Equatable {
    case center
    case edge(PaneEdge)
}

/// The tab a drag carries, so a drop moves that tab rather than opening its
/// conversation again; a drag from the list has none. A tab with no
/// conversation yet drags a link naming the tab itself. Written where the
/// drag begins, possibly off the main thread.
final class TabDragSource: @unchecked Sendable {
    static let shared = TabDragSource()
    private static let scheme = "x-velune-tab"
    private let lock = NSLock()
    private var current: (tab: UUID, link: URL)?

    /// The link a tab's drag carries: its conversation's, or one naming the tab.
    static func link(for tab: UUID, conversation: URL?) -> URL {
        conversation ?? URL(string: "\(scheme):\(tab.uuidString)")!
    }

    func begin(tab: UUID, link: URL) {
        lock.withLock { current = (tab, link) }
    }

    func end() {
        lock.withLock { current = nil }
    }

    /// The tab a dropped link came from; nil for a conversation dragged from the list.
    func tab(dropping link: URL) -> UUID? {
        if link.scheme == Self.scheme { return UUID(uuidString: link.absoluteString.dropFirst(Self.scheme.count + 1).description) }
        return lock.withLock { current?.link == link ? current?.tab : nil }
    }
}

/// A conversation dragged from the list and let go outside every window
/// opens in a window of its own there, as a tab dragged off its bar does.
/// The list's drag is the table's and tells nothing of where it ended, so
/// from its start the mouse button is watched until it is let go.
@MainActor final class ListDragWatch {
    static let shared = ListDragWatch()
    private var timer: Timer?
    /// The first conversation of the drag, and the window it left.
    private var dragged: (window: ConversationWindowModel, conversationID: String)?

    func begin(_ conversationID: String, from window: ConversationWindowModel) {
        guard NSEvent.pressedMouseButtons & 1 != 0 else { return }
        if dragged == nil { dragged = (window, conversationID) }
        guard timer == nil else { return }
        // In the common modes, so it fires while the drag tracks the mouse.
        let timer = Timer(timeInterval: 0.05, repeats: true) { _ in MainActor.assumeIsolated { ListDragWatch.shared.poll() } }
        RunLoop.main.add(timer, forMode: .common)
        self.timer = timer
    }

    private func poll() {
        guard NSEvent.pressedMouseButtons & 1 == 0 else { return }
        timer?.invalidate()
        timer = nil
        guard let (window, conversationID) = dragged else { return }
        dragged = nil
        let point = NSEvent.mouseLocation
        guard !NSApp.windows.contains(where: { $0.isVisible && $0.isOnActiveSpace && $0.styleMask.contains(.titled) && $0.frame.contains(point) }) else { return }
        window.openInNewWindow(conversationID, at: point)
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

    /// `added` beside `pane`'s `edge`, taking half its room.
    func inserting(_ added: UUID, beside pane: UUID, edge: PaneEdge) -> PaneLayout {
        switch self {
        case .pane(let id):
            guard id == pane else { return self }
            let children: [PaneLayout] = edge.after ? [.pane(id), .pane(added)] : [.pane(added), .pane(id)]
            return .split(PaneSplit(axis: edge.axis, children: children, fractions: [0.5, 0.5]))
        case .split(var split):
            // A pane in a split along the same axis shares its own room with the new one.
            if split.axis == edge.axis, let index = split.children.firstIndex(of: .pane(pane)) {
                let half = split.fractions[index] / 2
                split.fractions[index] = half
                split.children.insert(.pane(added), at: edge.after ? index + 1 : index)
                split.fractions.insert(half, at: edge.after ? index + 1 : index)
                return .split(split)
            }
            split.children = split.children.map { $0.inserting(added, beside: pane, edge: edge) }
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

/// What a window restores: its arrangement, each pane's tabs, each tab's
/// conversation, the focused pane and the tool windows.
struct SavedWindowLayout: Codable, Equatable {
    struct SavedPane: Codable, Equatable {
        var tabs: [UUID]
        var selected: UUID
    }
    var layout: PaneLayout
    var panes: [UUID: SavedPane]
    var conversations: [UUID: String]
    var focused: UUID
    var tools: ToolWindowLayout
    /// Kept in front of other apps' windows.
    var pinned: Bool

    /// Every pane and tab named once, the focused pane among them.
    var isWhole: Bool {
        let paneIDs = layout.paneIDs
        let tabIDs = paneIDs.flatMap { panes[$0]?.tabs ?? [] }
        return Set(paneIDs).count == paneIDs.count && paneIDs.contains(focused)
            && paneIDs.allSatisfy { panes[$0].map { !$0.tabs.isEmpty && $0.tabs.contains($0.selected) } ?? false }
            && Set(tabIDs).count == tabIDs.count
    }
}

/// The kinds of conversation window, each its own scene, so the system
/// keeps each kind's window frames apart and restores a window as its kind.
enum ConversationWindowKind: String {
    /// The list with the panes it opens conversations in.
    case workspace = "conversation"
    /// Opened for one conversation, by a tab or a list row dragged out or
    /// "open in new window": the editor area alone at first, its left area
    /// holding that conversation's sub-agents rather than the list, and how
    /// its tools are arranged its own, not what the next window starts with.
    case single = "single-conversation"

    var sceneID: String { rawValue }
}

/// What a new window starts with.
struct WindowSeed: Codable, Hashable {
    /// Each seed opens its own window; an equal value would bring an existing one forward.
    var id = UUID()
    /// The conversation to open in its pane.
    var conversationID: String?
    /// A tab another window hands over, with its state.
    var tab: UUID?
}

/// A conversation window: the list with its own workspace, and the panes
/// whose tabs show conversations. The list opens conversations in the
/// focused pane's tab; the window's toolbar and commands act on it.
@MainActor final class ConversationWindowModel: ObservableObject {
    let application: AppStore
    /// The list's workspace: list, search and selection; it opens no conversation.
    let list: AppStore
    @Published private(set) var layout: PaneLayout
    @Published private(set) var panes: [UUID: Pane]
    @Published private(set) var focusedPaneID: UUID
    /// Each tab's conversation as last selected, for the layout the window restores.
    @Published private(set) var tabConversations: [UUID: String] = [:]
    /// Bumped when a pane's tabs change, for the layout the window restores.
    @Published private(set) var tabsRevision = 0
    private var subscriptions: [UUID: AnyCancellable] = [:]
    private var paneSubscriptions: [UUID: AnyCancellable] = [:]
    private var started = false
    /// The pane views by pane, for telling which one a click lands in.
    private var anchors: [UUID: () -> NSView?] = [:]
    private var mouseMonitor: Any?
    /// Where the list, outline and composer are.
    @Published var tools = ToolWindowLayout() {
        didSet {
            guard tools != oldValue else { return }
            detached.sync()
            if kind == .workspace { application.rememberedToolLayout.remember(tools) }
        }
    }
    /// The area a panel dragged now would dock into, for the window to show where.
    @Published var dockHint: DockSide?
    /// The outline popover from the toolbar button.
    @Published var outlinePopover = false
    @Published var agentsPopover = false
    /// A conversation opened over the window from one linked to it, and the sheet's size.
    @Published private(set) var sheetTab: ConversationTab?
    private(set) var sheetSize = CGSize(width: 760, height: 640)
    /// The list's search text, shared by the sidebar and a floating list.
    @Published var listSearch = ""
    /// Opens a conversation window; set by the window's view, which has the action.
    var openWindow: ((ConversationWindowKind, WindowSeed) -> Void)?
    /// Kept in front of other apps' windows.
    @Published var pinned = false
    let kind: ConversationWindowKind
    private(set) lazy var detached = DetachedTools(model: self)

    /// Windows by the panes they hold, for moving a tab between them.
    private static var all: [WeakWindow] = []
    private struct WeakWindow { weak var model: ConversationWindowModel? }
    /// Tabs on their way to a window being opened for them.
    private static var handoff: [UUID: ConversationTab] = [:]

    static var openWindows: [ConversationWindowModel] {
        all.removeAll { $0.model == nil }
        return all.compactMap(\.model)
    }

    static func isConversationWindow(_ window: NSWindow) -> Bool {
        all.contains { $0.model?.window === window }
    }

    /// The window whose floating panel or tool window is key: commands act on it as on the window itself.
    static func owning(detached window: NSWindow?) -> ConversationWindowModel? {
        (window as? any DetachedTool)?.model
    }

    /// The conversation window an AppKit window shows.
    static func owning(window: NSWindow?) -> ConversationWindowModel? {
        guard let window else { return nil }
        return all.lazy.compactMap(\.model).first { $0.window === window }
    }

    static func owning(tab: UUID) -> ConversationWindowModel? {
        all.removeAll { $0.model == nil }
        return all.lazy.compactMap(\.model).first { $0.pane(holding: tab) != nil }
    }

    init(application: AppStore, kind: ConversationWindowKind) {
        self.application = application
        self.kind = kind
        list = application.makeWorkspace()
        let store = application.makeWorkspace()
        let first = Pane(tabs: [ConversationTab(store: store, start: { store.start() })])
        layout = .pane(first.id)
        panes = [first.id: first]
        focusedPaneID = first.id
        follow(first)
        Self.all.append(WeakWindow(model: self))
    }

    var focusedPane: Pane { panes[focusedPaneID]! }
    /// The focused pane's tab, which the toolbar, commands and tool windows act on.
    var focused: ConversationTab { focusedPane.selected }
    var isSplit: Bool { panes.count > 1 }
    var orderedPanes: [Pane] { layout.paneIDs.compactMap { panes[$0] } }
    var allTabs: [ConversationTab] { orderedPanes.flatMap(\.tabs) }
    var tabCount: Int { panes.values.reduce(0) { $0 + $1.tabs.count } }

    var savedLayout: SavedWindowLayout {
        _ = tabsRevision
        return SavedWindowLayout(layout: layout,
                                 panes: panes.mapValues { SavedWindowLayout.SavedPane(tabs: $0.tabs.map(\.id), selected: $0.selectedID) },
                                 conversations: tabConversations, focused: focusedPaneID, tools: tools, pinned: pinned)
    }

    /// Restore a saved arrangement, take a handed-over tab, or open `seed`'s
    /// conversation, then start reading. A lone new tab opens the list's
    /// first conversation.
    func start(restoring saved: SavedWindowLayout?, seed: WindowSeed?) {
        guard !started else { return }
        started = true
        list.start(loadFirstConversation: false)
        // A layout the system restores below replaces this one.
        switch kind {
        case .workspace: tools = application.rememberedToolLayout.layout
        case .single:
            tools = application.rememberedToolLayout.layout.forSingleConversation()
            tools.toggleEditorOnly()
        }
        if let id = seed?.tab, let tab = Self.handoff.removeValue(forKey: id) {
            replacePanes(with: [Pane(tabs: [tab])], layout: nil)
            tab.activate()
            tab.store.noteFocused()
        } else if let saved, saved.isWhole {
            let lone = saved.layout.paneIDs.count == 1 && saved.panes.values.first?.tabs.count == 1
            let restored = saved.layout.paneIDs.map { paneID in
                let savedPane = saved.panes[paneID]!
                let tabs = savedPane.tabs.map { tabID in
                    let store = application.makeWorkspace()
                    let conversationID = saved.conversations[tabID]
                    return ConversationTab(id: tabID, store: store, start: {
                        store.start(loadFirstConversation: lone, newConversation: !lone && conversationID == nil, opening: conversationID)
                    })
                }
                return Pane(id: paneID, tabs: tabs, selected: savedPane.selected)
            }
            replacePanes(with: restored, layout: saved.layout)
            focusedPaneID = saved.focused
            tools = saved.tools.normalized()
            pinned = saved.pinned
            for pane in restored { pane.selected.activate() }
        } else if let id = seed?.conversationID {
            let store = application.makeWorkspace()
            replacePanes(with: [Pane(tabs: [ConversationTab(store: store, start: { store.start(loadFirstConversation: false, opening: id) })])], layout: nil)
            focused.activate()
        } else {
            focused.activate()
        }
        installMouseMonitor()
        detached.sync()
    }

    /// The panes before reading starts: the first pane's untouched tab gives way.
    private func replacePanes(with replacement: [Pane], layout: PaneLayout?) {
        for tab in allTabs { tab.store.shutdown { _ in } }
        subscriptions = [:]
        paneSubscriptions = [:]
        panes = Dictionary(uniqueKeysWithValues: replacement.map { ($0.id, $0) })
        self.layout = layout ?? .pane(replacement[0].id)
        focusedPaneID = replacement[0].id
        replacement.forEach(follow)
    }

    // MARK: Opening

    /// Open a conversation in the focused tab; one a tab of this window
    /// already shows is selected there instead.
    func open(_ conversationID: String) {
        if select(showing: conversationID) { return }
        focused.store.openLinkedConversation(id: conversationID)
    }

    /// Open a conversation, or a new one, in a new tab after the focused pane's shown tab.
    func openInNewTab(_ conversationID: String? = nil) {
        if let conversationID, select(showing: conversationID) { return }
        let pane = focusedPane
        let index = pane.tabs.firstIndex { $0.id == pane.selectedID }.map { $0 + 1 }
        pane.insert(makeTab(opening: conversationID), at: index)
        focused.store.noteFocused()
    }

    /// Open a conversation linked from `tab`'s (the one that started it, a
    /// sub-agent's, a collaborator's) where `opening` says, else where the
    /// setting does. From the sheet, a sheet is the sheet itself, and a tab
    /// or window takes its place.
    func openLinked(_ conversationID: String, from tab: ConversationTab, opening: BindingLinkedConversationOpening? = nil) {
        let opening = opening ?? application.linkedConversationOpening
        if tab === sheetTab {
            if opening == .currentTab || opening == .sheet { return tab.store.openLinkedConversation(id: conversationID) }
            closeSheet()
        }
        switch opening {
        case .currentTab: tab.store.openLinkedConversation(id: conversationID)
        case .newTab: openInNewTab(conversationID)
        case .sheet: openInSheet(conversationID)
        case .newWindow: openInNewWindow(conversationID)
        }
    }

    /// Open a conversation in a sheet over the window; one a tab of this
    /// window shows is selected there instead.
    func openInSheet(_ conversationID: String) {
        if select(showing: conversationID) { return }
        closeSheet()
        if let content = window?.contentLayoutRect.size {
            sheetSize = CGSize(width: min(max(content.width * 0.8, 560), max(content.width - 40, 560)),
                               height: min(max(content.height * 0.85, 480), max(content.height - 40, 480)))
        }
        let tab = makeTab(opening: conversationID)
        tab.activate()
        sheetTab = tab
    }

    func closeSheet() {
        guard let tab = sheetTab else { return }
        sheetTab = nil
        tab.store.shutdown { _ in }
    }

    /// The sheet's conversation, as it stands, in a new tab after the focused pane's shown tab.
    func moveSheetToTab() {
        guard let tab = sheetTab else { return }
        sheetTab = nil
        let pane = focusedPane
        pane.insert(tab, at: pane.tabs.firstIndex { $0.id == pane.selectedID }.map { $0 + 1 })
        tab.store.noteFocused()
    }

    /// The sheet's conversation, as it stands, in a conversation window of its own.
    func moveSheetToNewWindow() {
        guard let tab = sheetTab, openWindow != nil else { return }
        sheetTab = nil
        hand(tab, toNewWindowIn: newWindowFrame(size: sheetSize, at: nil))
    }

    /// Show in a new tab a conversation the core has just opened, such as an
    /// edit's fork, as `result` gives it.
    func openInNewTab(adopting result: BindingSnapshotResult) {
        guard let id = result.snapshot?.conversation.id else { return }
        if select(showing: id) { return }
        let store = application.makeWorkspace()
        let pane = focusedPane
        let index = pane.tabs.firstIndex { $0.id == pane.selectedID }.map { $0 + 1 }
        pane.insert(ConversationTab(store: store, start: {
            store.start(loadFirstConversation: false)
            store.adopt(result)
        }), at: index)
        focused.store.noteFocused()
    }

    /// Divide the focused pane; the new pane takes focus and opens
    /// `conversationID`, or starts a new conversation.
    func split(_ axis: PaneSplit.Axis, opening conversationID: String? = nil) {
        if let conversationID, select(showing: conversationID) { return }
        addPane(with: makeTab(opening: conversationID), beside: focusedPaneID, edge: axis == .horizontal ? .trailing : .bottom)
    }

    private func makeTab(opening conversationID: String?) -> ConversationTab {
        let store = application.makeWorkspace()
        return ConversationTab(store: store, start: {
            store.start(loadFirstConversation: false, newConversation: conversationID == nil, opening: conversationID)
        })
    }

    /// Select the tab showing a conversation, if one does.
    private func select(showing conversationID: String) -> Bool {
        guard let (pane, tab) = tab(showing: conversationID) else { return false }
        pane.select(tab.id)
        focus(pane.id)
        return true
    }

    func tab(showing conversationID: String) -> (Pane, ConversationTab)? {
        for pane in orderedPanes {
            if let tab = pane.tabs.first(where: { $0.store.selectedConversationID == conversationID }) { return (pane, tab) }
        }
        return nil
    }

    func pane(holding tab: UUID) -> Pane? {
        panes.values.first { $0.tabs.contains { $0.id == tab } }
    }

    // MARK: Tabs

    func selectTab(_ id: UUID) {
        guard let pane = pane(holding: id) else { return }
        pane.select(id)
        focus(pane.id)
        pane.selected.store.noteFocused()
    }

    /// The tab after (or before) the focused pane's shown one.
    func selectNeighbourTab(forward: Bool) {
        let pane = focusedPane
        guard pane.tabs.count > 1, let index = pane.tabs.firstIndex(where: { $0.id == pane.selectedID }) else { return }
        selectTab(pane.tabs[(index + (forward ? 1 : pane.tabs.count - 1)) % pane.tabs.count].id)
    }

    /// Close a tab: its pane's last tab closes the pane, the window's last closes the window.
    func closeTab(_ id: UUID) {
        guard let tab = detach(id, emptying: .close) else { return }
        tab.store.shutdown { _ in }
    }

    func closeOtherTabs(_ id: UUID) {
        guard let pane = pane(holding: id) else { return }
        for tab in pane.tabs where tab.id != id { closeTab(tab.id) }
    }

    /// Close a pane with all its tabs.
    func closePane(_ id: UUID) {
        guard isSplit, let pane = panes[id] else { return }
        for tab in pane.tabs { tab.store.shutdown { _ in } }
        removePane(id)
    }

    /// What becomes of a window whose last tab is taken out.
    private enum Emptying {
        /// The tab stays.
        case keep
        case close
        /// A window with tool windows keeps a new conversation; one of the
        /// editor area alone has nothing left to show and closes.
        case leave
    }

    /// Any tool window shows, docked or outside the window.
    var hasToolWindows: Bool { ToolWindowKind.allCases.contains(where: tools.isOpen) }

    /// A tab can go to a window of its own: one in a window of the editor
    /// area alone needs another tab beside it.
    var canMoveTabToNewWindow: Bool { tabCount > 1 || hasToolWindows }

    /// Take a tab out of this window without ending it. Its pane goes with
    /// its last tab; the window's last tab is kept, leaves a new conversation or closes the window.
    private func detach(_ id: UUID, emptying: Emptying) -> ConversationTab? {
        guard let pane = pane(holding: id), let tab = pane.tabs.first(where: { $0.id == id }) else { return nil }
        if pane.remove(id) {
            if pane.id == focusedPaneID { pane.selected.store.noteFocused() }
        } else if isSplit {
            removePane(pane.id)
        } else {
            switch emptying {
            case .keep: return nil
            case .leave where hasToolWindows:
                pane.insert(makeTab(opening: nil))
                pane.remove(id)
                pane.selected.store.noteFocused()
            case .close, .leave: window?.close()
            }
        }
        subscriptions[id] = nil
        tabConversations[id] = nil
        tabsRevision &+= 1
        return tab
    }

    private func removePane(_ id: UUID) {
        guard let remaining = layout.removing(id) else { return }
        let order = layout.paneIDs
        layout = remaining
        panes[id] = nil
        paneSubscriptions[id] = nil
        anchors[id] = nil
        if focusedPaneID == id, let index = order.firstIndex(of: id) {
            // Focus moves to the pane before it, or the next when it was first.
            let neighbours = order.filter { $0 != id }
            focus(neighbours[max(0, min(index - 1, neighbours.count - 1))])
        }
    }

    private func addPane(with tab: ConversationTab, beside target: UUID, edge: PaneEdge) {
        let pane = Pane(tabs: [tab])
        panes[pane.id] = pane
        layout = layout.inserting(pane.id, beside: target, edge: edge)
        follow(pane)
        focusedPaneID = pane.id
        tab.activate()
        tab.store.noteFocused()
    }

    // MARK: Moving

    /// A tab, or a conversation from the list, dropped on a pane: among its
    /// tabs (at `index`, else at the end) or beside an edge. A tab comes
    /// from wherever it is, this window or another; a conversation a tab of
    /// this window shows moves that tab; any other opens in a new tab.
    func drop(_ link: URL, on target: UUID, zone: PaneDropZone, at index: Int? = nil) {
        guard let targetPane = panes[target] else { return }
        let tabID = TabDragSource.shared.tab(dropping: link)
            ?? ConversationLink.conversation(in: link).flatMap { tab(showing: $0.conversationID)?.1.id }
        // A tab closed since its drag began leaves its conversation to open anew.
        guard let tabID, let source = Self.owning(tab: tabID), let sourcePane = source.pane(holding: tabID) else {
            guard let conversationID = ConversationLink.conversation(in: link)?.conversationID else { return }
            if let (pane, existing) = tab(showing: conversationID) { pane.select(existing.id); focus(pane.id); return }
            let tab = makeTab(opening: conversationID)
            switch zone {
            case .center: targetPane.insert(tab, at: index); focus(target); tab.store.noteFocused()
            case .edge(let edge): addPane(with: tab, beside: target, edge: edge)
            }
            return
        }
        switch zone {
        case .center where sourcePane === targetPane:
            targetPane.move(tabID, to: index ?? targetPane.tabs.count)
            focus(target)
        case .edge where sourcePane === targetPane && targetPane.tabs.count == 1:
            // A pane's only tab beside its own pane has nowhere new to go.
            break
        default:
            guard let tab = source.detach(tabID, emptying: .leave) else { return }
            adopt(tab)
            switch zone {
            case .center: targetPane.insert(tab, at: index); focus(target); tab.store.noteFocused()
            case .edge(let edge):
                guard panes[target] != nil else { return }
                addPane(with: tab, beside: target, edge: edge)
            }
        }
    }

    /// Move a tab to `place` among its pane's tabs, counted as they will be.
    func moveTab(_ id: UUID, toPlace place: Int) {
        guard let pane = pane(holding: id), let from = pane.tabs.firstIndex(where: { $0.id == id }) else { return }
        pane.move(id, to: place > from ? place + 1 : place)
        focus(pane.id)
    }

    /// Move a tab into a new pane beside the pane it is in.
    func splitTab(_ id: UUID, edge: PaneEdge) {
        guard let pane = pane(holding: id), pane.tabs.count > 1, let tab = detach(id, emptying: .keep) else { return }
        addPane(with: tab, beside: pane.id, edge: edge)
    }

    /// Move a tab to a conversation window of its own, as large as its pane,
    /// at `point` (screen coordinates, its top left; nil sets it off from this window).
    func moveToNewWindow(_ id: UUID, at point: NSPoint? = nil) {
        guard canMoveTabToNewWindow, openWindow != nil, let pane = pane(holding: id) else { return }
        let frame = newWindowFrame(size: view(of: pane.id)?.bounds.size, at: point)
        guard let tab = detach(id, emptying: .leave) else { return }
        hand(tab, toNewWindowIn: frame)
    }

    /// A tab no window holds, in a conversation window opened for it.
    private func hand(_ tab: ConversationTab, toNewWindowIn frame: NSRect?) {
        guard let openWindow else { return }
        Self.handoff[tab.id] = tab
        let existing = Set(NSApp.windows.map(\.windowNumber))
        openWindow(.single, WindowSeed(tab: tab.id))
        if let frame { Self.place(windowNotIn: existing, frame: frame, attempts: 20) }
    }

    /// Open a conversation in a conversation window of its own; a tab of this
    /// window showing it moves there. At `point`, the window's top left is there.
    func openInNewWindow(_ conversationID: String, at point: NSPoint? = nil) {
        if let (_, tab) = tab(showing: conversationID) { return moveToNewWindow(tab.id, at: point) }
        guard let openWindow else { return }
        let frame = newWindowFrame(size: nil, at: point)
        let existing = Set(NSApp.windows.map(\.windowNumber))
        openWindow(.single, WindowSeed(conversationID: conversationID))
        if let frame { Self.place(windowNotIn: existing, frame: frame, attempts: 20) }
    }

    /// A conversation window's frame: a pane's size (else a usual one) under
    /// this window's title bar and toolbar, at `point` or set off from this window.
    private func newWindowFrame(size: CGSize?, at point: NSPoint?) -> NSRect? {
        guard let window else { return nil }
        let chrome = window.frame.height - window.contentLayoutRect.height
        let width = max(size?.width ?? 720, 520), height = max((size?.height ?? window.contentLayoutRect.height) + chrome, 420)
        let topLeft = point ?? NSPoint(x: window.frame.minX + 28, y: window.frame.maxY - 28)
        var frame = NSRect(x: topLeft.x, y: topLeft.y - height, width: width, height: height)
        if let screen = (NSScreen.screens.first { $0.frame.contains(topLeft) } ?? window.screen)?.visibleFrame {
            frame.size.height = min(frame.height, screen.height)
            frame.origin.x = min(max(frame.minX, screen.minX), screen.maxX - frame.width)
            frame.origin.y = min(max(frame.minY, screen.minY), screen.maxY - frame.height)
        }
        return frame
    }

    /// A tab arriving from another window.
    private func adopt(_ tab: ConversationTab) {
        follow(tab)
        tabsRevision &+= 1
    }

    /// The new window appears a moment after it is asked for.
    private static func place(windowNotIn existing: Set<Int>, frame: NSRect, attempts: Int) {
        DispatchQueue.main.asyncAfter(deadline: .now() + 0.05) {
            if let window = NSApp.windows.first(where: { !existing.contains($0.windowNumber) && $0.isVisible && isConversationWindow($0) }) { window.setFrame(frame, display: true) }
            else if attempts > 1 { place(windowNotIn: existing, frame: frame, attempts: attempts - 1) }
        }
    }

    // MARK: Focus

    /// The AppKit window showing this one.
    var window: NSWindow? { anchors.values.lazy.compactMap { $0()?.window }.first }

    func focus(_ id: UUID) {
        guard let pane = panes[id], focusedPaneID != id else { return }
        focusedPaneID = id
        pane.selected.store.noteFocused()
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
    func view(of pane: UUID) -> NSView? { anchors[pane]?() }

    private func follow(_ pane: Pane) {
        pane.tabs.forEach(follow)
        paneSubscriptions[pane.id] = pane.$tabs.combineLatest(pane.$selectedID).dropFirst().sink { [weak self] tabs, _ in
            guard let self else { return }
            tabs.filter { self.subscriptions[$0.id] == nil }.forEach(self.follow)
            self.tabsRevision &+= 1
        }
    }

    private func follow(_ tab: ConversationTab) {
        subscriptions[tab.id] = tab.store.$selectedConversationID.sink { [weak self] id in
            guard let self else { return }
            if self.tabConversations[tab.id] != id { self.tabConversations[tab.id] = id }
        }
    }

    /// ⌘F: the find of the tool window that has the focus, else the shown
    /// conversation's messages. Only the list searches so far; the outline
    /// and linked agents would join `findAreas`.
    func find() {
        if focusIsIn(findAreas[.list]) { if listIsBlock { listSearchPopover = true } else { listSearchRequest &+= 1 } } else { focused.beginFind() }
    }

    @Published private(set) var listSearchRequest = 0
    /// The list's grouping, filtering and sorting.
    @Published var listBrowser = ConversationBrowser()
    /// The search popover of the list's title bar.
    @Published var listSearchPopover = false
    /// The list shares its area with other tools, so it is a block with a title bar.
    var listIsBlock: Bool {
        guard let side = tools[.list].place.side else { return false }
        return tools.tools(in: side).count > 1
    }
    private var findAreas: [ToolWindowKind: () -> NSView?] = [:]

    func registerFindArea(_ kind: ToolWindowKind, anchor: @escaping () -> NSView?) { findAreas[kind] = anchor }

    /// The first responder is in the view the anchor marks.
    private func focusIsIn(_ anchor: (() -> NSView?)?) -> Bool {
        guard let area = anchor?(), let window = area.window, let responder = window.firstResponder as? NSView else { return false }
        let middle = responder.convert(NSPoint(x: responder.bounds.midX, y: responder.bounds.midY), to: nil)
        return area.bounds.contains(area.convert(middle, from: nil))
    }

    /// The tools' toolbar buttons by tool, for telling which one a right click lands on.
    private var toolbarAnchors: [ToolWindowKind: () -> NSView?] = [:]

    func registerToolbarButton(_ kind: ToolWindowKind, anchor: @escaping () -> NSView?) { toolbarAnchors[kind] = anchor }

    /// A right click on the outline's or the linked agents' toolbar button
    /// offers where that tool is, which SwiftUI's own context menu does not
    /// on a toolbar button.
    private func showsToolbarPlaceMenu(for event: NSEvent) -> Bool {
        for (kind, anchor) in toolbarAnchors {
            guard let view = anchor(), let window = view.window, window === event.window,
                  view.bounds.contains(view.convert(event.locationInWindow, from: nil)) else { continue }
            let menu = NSMenu()
            let current = choice(for: kind)
            for (choice, title) in ToolChoice.all(for: kind) {
                let entry = ClosureMenuItem(title) { [weak self] in self?.choose(choice, for: kind) }
                entry.state = choice == current ? .on : .off
                menu.addItem(entry)
            }
            NSMenu.popUpContextMenu(menu, with: event, for: view)
            return true
        }
        return false
    }

    /// A click in a pane focuses it, wherever in the pane it lands.
    private func installMouseMonitor() {
        guard mouseMonitor == nil else { return }
        mouseMonitor = NSEvent.addLocalMonitorForEvents(matching: [.leftMouseDown, .rightMouseDown, .otherMouseDown]) { [weak self] event in
            guard let self else { return event }
            if event.type == .rightMouseDown, self.showsToolbarPlaceMenu(for: event) { return nil }
            guard self.isSplit else { return event }
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
