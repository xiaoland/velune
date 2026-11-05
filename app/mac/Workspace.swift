import AppKit
import Combine
import SwiftUI
import VeluneBindings

/// One conversation tab: its own workspace, composer and reading state. A
/// tab keeps them while it is in the background and when it moves to another
/// pane or window.
@MainActor class EditorTab: ObservableObject, Identifiable {
    let id: UUID
    init(id: UUID = UUID()) { self.id = id }
    var title: String { fatalError("An editor supplies its title") }
    var link: URL? { nil }
    func activate() {}
    func close() {}
}

@MainActor final class ConversationTab: EditorTab {
    let store: ConversationStore
    /// A side chat owns this input for its entire lifetime. Ordinary tabs
    /// keep a reference selected from the window's persistent input registry.
    /// Not observed by the tab: only the composer re-renders while typing.
    var composer = ComposerModel()
    @Published var scrollRequest: UInt64 = 0
    /// The list shows only user messages (in-place outline).
    @Published var outlineCollapsed = false
    @Published var outlineNavigation: TranscriptJump?
    /// Searching the conversation's messages (see `TranscriptFindBar`).
    let find = TranscriptFind()
    /// The user message being edited, in a sheet over the tab.
    @Published var editing: MessageEditRequest?
    /// When the keyboard was asked to go to the request waiting for an
    /// answer; its card takes it on showing, which may follow loading the
    /// conversation. A request older than a few seconds has lapsed.
    @Published private(set) var interactionFocusRequested: Date?
    func focusTranscript() { reading.table.focusFromKeyboard() }
    func focusInteraction() { interactionFocusRequested = Date() }
    /// Whether the card showing now should take the keyboard; asking clears the request.
    func takeInteractionFocusRequest() -> Bool {
        guard let requested = interactionFocusRequested else { return false }
        interactionFocusRequested = nil
        return Date().timeIntervalSince(requested) < 5
    }
    /// Kept while the tab is behind another, with the scroll position.
    let reading = TranscriptReading()
    /// A side chat opened from this tab ends when its source changes or closes.
    var sideChatParentID: UUID?
    var canOpenSideChat: Bool { store.sideChatTarget != nil }

    override func close() { store.close() }
    /// Reading starts when the tab is first shown, so restored background tabs cost nothing.
    private var pendingStart: (() -> Void)?

    init(id: UUID = UUID(), store: ConversationStore, start: @escaping () -> Void) {
        self.store = store
        pendingStart = start
        super.init(id: id)
        reading.table.observeFind(find)
    }

    override func activate() {
        guard let start = pendingStart else { return }
        pendingStart = nil
        start()
    }

    var submission: ComposerSubmission { submission(using: composer) }
    func submission(using model: ComposerModel, ignoringText: Bool = false, next: NextTurn? = nil) -> ComposerSubmission {
        submission(hasInput: (!ignoringText && !model.isEmpty) || !model.attachments.isEmpty, model: model, next: next)
    }
    /// A slash command consumes its own token before executing; attachments remain input.
    var canResumeAfterCommand: Bool { submission(using: composer, ignoringText: true).canResume }
    private func submission(hasInput: Bool, model: ComposerModel, next: NextTurn?) -> ComposerSubmission {
        let configured = (next ?? store.nextTurn)?.chosenModelUnavailable != true && ComposerContext(tab: self, model: model).inputIssue == nil
        let phase: ComposerSubmission.Phase
        if store.isStopping { phase = .stopping }
        else if let pending = store.composerRequest { phase = pending }
        else if store.isGenerating { phase = .running }
        else { phase = store.canSend ? .idle : .blocked }
        return ComposerSubmission(phase: phase, hasInput: hasInput,
                                  isComposing: model.isComposing, canSend: configured && store.canSend && (store.allowsAttachments || model.attachments.isEmpty),
                                  canQueue: configured && store.canQueue, canResume: configured && store.canResume(next: next ?? store.nextTurn))
    }
    var canSteer: Bool { submission.canSubmitDraft && submission.action == .queue && store.canSteer }
    func resume() {
        guard submission.canResume else { return }
        store.continueTurn()
    }

    func send() { send(using: composer) }
    func send(using composer: ComposerModel, next: NextTurn? = nil) {
        var clearedRevision: UInt64?
        let submitted = composer.submission
        let attachments = composer.attachments
        let text = submitted.trimmingCharacters(in: .whitespacesAndNewlines)
        guard submission(using: composer, next: next).canSubmitDraft, store.allowsAttachments || attachments.isEmpty else { return }
        // While a turn runs the message waits in the queue above the composer.
        if store.canQueue {
            guard store.queue(text: text, attachments: attachments.map(\.path), next: next, onFailure: { [composer] in
                composer.restoreFailed(submitted, attachments: attachments, revision: clearedRevision)
            }) else { return }
            composer.clear(submitted: submitted, attachments: attachments); clearedRevision = composer.revision
            return
        }
        guard store.canSend else { return }
        // The message moves to the transcript at once; a turn that is not
        // sent hands its text and attachments back to the composer.
        let createsConversation = store.isUnsentConversation
        let sent = store.send(text: text, attachments: attachments.map(\.path), next: next, input: composer, onFailure: { [composer, store] in
            composer.restoreFailed(submitted, attachments: attachments, revision: clearedRevision)
            // The global new entry takes the consumed input back, unless its
            // fresh input has gained newer content meanwhile.
            if createsConversation { store.device.composerInputs.restoreConsumedNewConversationInput(composer, whileCreating: store) }
        })
        guard sent else { return }
        composer.clear(submitted: submitted, attachments: attachments); clearedRevision = composer.revision; scrollRequest &+= 1
        if createsConversation { store.device.composerInputs.consumeNewConversationInput(composer) }
    }

    /// Into the running turn through the runtime's own channel; a message it
    /// does not take goes back to the composer. Without a running turn that
    /// takes one, the message is sent or queued instead.
    func steer() { steer(using: composer) }
    func steer(using composer: ComposerModel, next: NextTurn? = nil) {
        guard store.isGenerating, store.followUp.canSteer else { return send(using: composer, next: next) }
        var clearedRevision: UInt64?
        let submitted = composer.submission
        let attachments = composer.attachments
        let text = submitted.trimmingCharacters(in: .whitespacesAndNewlines)
        guard submission(using: composer, next: next).canSubmitDraft, store.canSteer else { return }
        store.steer(text: text, attachments: attachments.map(\.path), onFailure: { [composer] in composer.restoreFailed(submitted, attachments: attachments, revision: clearedRevision) })
        composer.clear(submitted: submitted, attachments: attachments); clearedRevision = composer.revision
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
    private func transcript(of store: ConversationStore) -> [DeliveredMessage] { store.transcript.rows.map(\.message) }

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
        outlineCollapsed = false
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
    override var link: URL? {
        guard let id = store.loadedConversationID, !store.isSideChat, !store.isUnsentConversation,
              let runtime = store.conversation(id: id)?.runtimeID ?? store.projectionRuntimeID else { return nil }
        return ConversationLink.url(runtimeID: runtime, conversationID: id)
    }

    override var title: String {
        if let context = store.sideChatContext { return store.selectedConversationTitle ?? "侧聊 · " + context.title }
        return store.selectedConversationTitle ?? store.reachingConversationID.map { store.conversation(id: $0)?.title ?? "正在打开…" } ?? "新会话"
    }
}

/// A conversation resumed from the list: the tab that does it without
/// showing, until the core took the request or refused it.
@MainActor final class BackgroundResume {
    let tab: ConversationTab
    /// The tab's accepted submissions before the resume.
    let accepted: UInt64
    var requested = false
    var watch: AnyCancellable?
    var timeout: Timer?
    init(tab: ConversationTab, accepted: UInt64) {
        self.tab = tab
        self.accepted = accepted
    }
}

/// A leaf of a window's split: its tabs and the one it shows.
@MainActor final class Pane: ObservableObject, Identifiable {
    let id: UUID
    @Published private(set) var tabs: [EditorTab]
    @Published private(set) var selectedID: UUID

    init(id: UUID = UUID(), tabs: [EditorTab], selected: UUID? = nil) {
        self.id = id
        self.tabs = tabs
        selectedID = selected.flatMap { id in tabs.contains { $0.id == id } ? id : nil } ?? tabs[0].id
    }

    var selected: EditorTab { tabs.first { $0.id == selectedID } ?? tabs[0] }

    func select(_ id: UUID) {
        guard let tab = tabs.first(where: { $0.id == id }) else { return }
        if selectedID != id {
            // Typing that went to the tab put behind goes to the one shown.
            if let previous = selected as? ConversationTab, let next = tab as? ConversationTab, previous.composer.isFocused { next.composer.focusWhenShown() }
            selectedID = id
        }
        tab.activate()
    }

    /// `tab` at `index` (the end when nil), shown.
    func insert(_ tab: EditorTab, at index: Int? = nil) {
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
    private var ending: UUID?
    /// The first conversation of the drag, and the window it left.
    private var dragged: (window: ConversationWindowModel, conversationID: String)?
    var sourceWindow: ConversationWindowModel? { dragged?.window }

    func accepted() {
        timer?.invalidate()
        timer = nil
        dragged = nil
        ending = nil
    }

    func begin(_ conversationID: String, from window: ConversationWindowModel) {
        guard NSEvent.pressedMouseButtons & 1 != 0 else { return }
        if ending != nil { accepted() }
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
        let token = UUID()
        ending = token
        let point = NSEvent.mouseLocation
        // SwiftUI accepts the target synchronously, then loads its provider
        // asynchronously. Let that acceptance cancel the drag-out fallback.
        DispatchQueue.main.asyncAfter(deadline: .now() + 0.1) { [weak self] in
            guard let self, ending == token else { return }
            finish(at: point)
        }
    }

    private func finish(at point: NSPoint) {
        guard let (window, conversationID) = dragged else { return }
        accepted()
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
    enum ExecutionDevice: Codable, Equatable {
        case local, remote(String)
        init(_ peerID: String?) { self = peerID.map(Self.remote) ?? .local }
    }
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
    /// Independent composer state follows this window's system restoration.
    var composer: SavedComposerDraft? = nil
    var composerWorkspace: SavedComposerWorkspace? = nil
    var artifacts: [UUID: ArtifactDocument]? = nil
    var unsentConversations: [String: Conversation]? = nil
    /// Creation destinations belong to tabs; their input belongs to the global slot.
    var creationTargets: [UUID: Conversation]? = nil
    var windowKind: ConversationWindowKind? = nil
    /// Absent only in legacy snapshots whose execution device is unknown.
    var windowDevice: ExecutionDevice? = nil

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
enum ConversationWindowKind: String, Codable {
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
    /// The paired device whose core the window shows; nil for this Mac.
    var device: String?
}

/// Presentation intent belongs to this opening, never to restored window data.
@MainActor enum WindowPresentation {
    static var windowed: Set<UUID> = []
    static var notificationTargets: Set<UUID> = []
}

/// A conversation window: the list with its own workspace, and the panes
/// whose tabs show conversations. The list opens conversations in the
/// focused pane's tab; the window's toolbar and commands act on it.
@MainActor final class ConversationWindowModel: ObservableObject {
    @Published var renameTarget: Conversation?
    @Published var deleteTargets: [Conversation] = []
    @Published var showsAgentURI = false
    @Published var agentURIMatches: [Conversation] = []
    @Published var agentURIFailure: String?
    private var agentURIRequest = UUID()

    func openAgentURI(_ uri: String) {
        let request = UUID()
        agentURIRequest = request
        showsAgentURI = false
        list.device.resolveAgentURI(uri) { [weak self] result in
            guard let self, agentURIRequest == request else { return }
            agentURIMatches = []
            agentURIFailure = nil
            switch result {
            case .success(let value):
                agentURIMatches = value.conversations.map(BindingMapping.listed)
                if agentURIMatches.count == 1 && value.failures.isEmpty, let match = agentURIMatches.first {
                    open(match.id)
                    return
                }
                for failure in value.failures { list.recordProblem(failure.detail, source: "打开会话 · " + failure.runtimeId) }
                if !value.failures.isEmpty { agentURIFailure = "部分运行时无法读取，详见问题。" }
                else if agentURIMatches.isEmpty { agentURIFailure = "当前设备的已配置实例中找不到该会话。" }
            case .failure(let error):
                list.recordProblem(error, source: "打开会话")
                agentURIFailure = error.localizedDescription
            }
            showsAgentURI = true
        }
    }

    let application: DeviceStore
    /// The paired device the window shows; nil for this Mac.
    let device: String?
    /// The list's workspace: list, search and selection; it opens no conversation.
    let list: ConversationStore
    @Published private(set) var layout: PaneLayout
    @Published private(set) var panes: [UUID: Pane]
    @Published private(set) var focusedPaneID: UUID
    /// Each tab's conversation as last selected, for the layout the window restores.
    @Published private(set) var tabConversations: [UUID: String] = [:]
    /// Closed conversation tabs' targets (nil for an unsent new
    /// conversation), latest last, for 重新打开关闭的标签页.
    @Published private(set) var recentlyClosed: [String?] = []
    /// Bumped when a pane's tabs change, for the layout the window restores.
    @Published private(set) var tabsRevision = 0
    private var subscriptions: [UUID: Set<AnyCancellable>] = [:]
    private var preservingComposerLayout = false
    private var paneSubscriptions: [UUID: AnyCancellable] = [:]
    private var started = false
    private var closed = false
    func noteWindowClosed() { closed = true }
    private var persistenceID: String?
    private var draftPersistence: AnyCancellable?
    private var composerCursorWatch: AnyCancellable?
    private var composerCreationWatch: AnyCancellable?
    private var workspacePersistence: AnyCancellable?
    private var restoredCreationTargets: [UUID: Conversation] = [:]
    func persistWorkspace() {
        guard started, let persistenceID else { return }
        application.windowWorkspaceJournal.save(savedLayout, id: persistenceID)
    }
    /// The pane views by pane, for telling which one a click lands in.
    private var anchors: [UUID: () -> NSView?] = [:]
    private var conversationAnchors: [UUID: () -> NSView?] = [:]
    @Published private var activeConversationID: UUID?
    /// Conversation commands follow the editor or transcript being used,
    /// including one temporarily presented beside its parent or in a sheet.
    var commandTab: ConversationTab {
        if let composerSheetTab, composerSheetTab.id == activeConversationID { return composerSheetTab }
        if let sheetTab, sheetTab.id == activeConversationID { return sheetTab }
        return focused
    }
    func focusConversation(_ tab: ConversationTab) {
        if activeConversationID != tab.id { activeConversationID = tab.id }
        if let pane = pane(holding: tab.id) { focus(pane.id) }
        noteSelected(tab)
    }
    var conversationTabs: [ConversationTab] {
        allTabs.compactMap { $0 as? ConversationTab } + [sheetTab, composerSheetTab].compactMap { $0 }
    }
    func conversationTab(for store: ConversationStore?) -> ConversationTab {
        store.flatMap { store in conversationTabs.first { $0.store === store } } ?? commandTab
    }
    func registerConversation(_ tab: ConversationTab, anchor: @escaping () -> NSView?) {
        let live = Set(conversationTabs.map(\.id))
        conversationAnchors = conversationAnchors.filter { live.contains($0.key) }
        conversationAnchors[tab.id] = anchor
    }
    private var mouseMonitor: Any?
    private(set) lazy var composerDrafts = ComposerDrafts(device: application)
    @Published var composerMinimumHeight = ComposerTextView.minimumHeight + 54
    private var restoredComposerDraft: ComposerModel?
    var sharedComposer: ComposerModel { composerDrafts.shared }
    /// Temporary side-chat inputs stay with their tab and never enter the registry.
    func composerOwner(for tab: ConversationTab, tools placement: ToolWindowLayout? = nil) -> ComposerDrafts.Owner? {
        if tab.store.isSideChat { return nil }
        let placement = placement ?? tools
        if tab === sheetTab || tab === composerSheetTab { return .conversation(tab.store.selectedConversationID ?? tab.store.loadedConversationID ?? "tab:\(tab.id)") }
        switch placement[.composer].place {
        case .floating, .window, .docked: return .shared
        default: return .conversation(tab.store.selectedConversationID ?? tab.store.loadedConversationID ?? "tab:\(tab.id)")
        }
    }
    func composer(for tab: ConversationTab) -> ComposerModel {
        guard let owner = composerOwner(for: tab) else { return tab.composer }
        let model = composerDrafts.model(for: owner, tab: tab)
        if case .conversation = owner { tab.composer = model }
        return model
    }
    func existingComposer(for tab: ConversationTab) -> ComposerModel? {
        guard let owner = composerOwner(for: tab) else { return tab.composer }
        return composerDrafts.existingModel(for: owner)
    }
    func selectComposerDraft(_ model: ComposerModel, for tab: ConversationTab) {
        guard let owner = composerOwner(for: tab) else { return }
        composerDrafts.select(model, replacing: composer(for: tab), owner: owner, tab: tab)
        objectWillChange.send()
        composer(for: tab).focusWhenShown()
    }
    func newComposerDraft(for tab: ConversationTab) {
        guard let owner = composerOwner(for: tab) else { return }
        composerDrafts.newDraft(replacing: composer(for: tab), owner: owner, tab: tab)
        objectWillChange.send()
        composer(for: tab).focusWhenShown()
    }
    private func navigateComposer(to id: String, tab: ConversationTab, checkpointPrevious: Bool = true) {
        guard let owner = composerOwner(for: tab) else { return }
        composerDrafts.navigate(to: id, tab: tab, sharedInput: owner == .shared, checkpointPrevious: checkpointPrevious)
    }
    /// A new conversation opens in a new tab after the focused pane's shown
    /// one; a focused tab already holding an unsent conversation is restored
    /// in place instead, not stacked under another empty tab. The one global
    /// new-conversation input restores as the tab is selected.
    @discardableResult
    func createConversation() -> ConversationTab? {
        if let shown = focusedEditor as? ConversationTab, shown.store.isUnsentConversation {
            return shown.store.createConversation() ? shown : nil
        }
        return openNewConversationTab()
    }
    /// Group creation is an explicit target choice, including for a shared
    /// composer whose unfinished input otherwise keeps its own intent. The
    /// conversation opens as `createConversation()` does: in a new tab.
    func createConversation(in group: ConversationGroupKey) {
        let project: ConversationProject? = if case .project(let path?) = group {
            application.conversationProjects.first { $0.path == path }
        } else { nil }
        let runtimeID: String? = if case .runtime(let id) = group { id } else { project?.nativeRuntimeId }
        let targetRuntimeID = runtimeID ?? composerContext(for: focused).nextTurnRuntimeID
        let selectedProject: NewConversationProject? = project.flatMap { project in
            guard let runtimeID = project.nativeRuntimeId,
                  let choice = application.workingDirectoryProjects[runtimeID]?.first(where: { $0.project == project.path && $0.canSelectProject }) else { return nil }
            return NewConversationProject(key: choice.project, name: choice.name, runtimeID: runtimeID, directories: choice.directories)
        }
        // A native project's group means that whole project; do not quietly
        // reduce an unavailable project to its first directory.
        if project?.nativeRuntimeId != nil && selectedProject == nil {
            list.recordProblem("此项目当前没有可用的完整工作区，请刷新项目或选择工作目录", source: "新建会话")
            return
        }
        let tab: ConversationTab?
        if let shown = focusedEditor as? ConversationTab, shown.store.isUnsentConversation {
            tab = shown.store.createConversation(runtimeID: targetRuntimeID, cwd: project?.primaryFolder, project: selectedProject) ? shown : nil
        } else {
            tab = openNewConversationTab(runtimeID: targetRuntimeID, cwd: project?.primaryFolder, project: selectedProject)
        }
        guard let tab else { return }
        if let runtimeID { composerContext(for: tab).selectNextTurnRuntime(runtimeID) }
    }
    /// The draft is created before the tab starts: an unavailable target
    /// leaves the window unchanged. Its navigation event fired before the tab
    /// was followed, so the shared input is navigated to as selecting the tab
    /// would. `start` only reads; the snapshot already exists (the preview
    /// branch would otherwise recreate it with the default target).
    private func openNewConversationTab(runtimeID: String? = nil, cwd: String? = nil, project: NewConversationProject? = nil) -> ConversationTab? {
        let store = ConversationStore(device: application)
        guard store.createConversation(runtimeID: runtimeID, cwd: cwd, project: project) else { return nil }
        let tab = ConversationTab(store: store, start: {
            store.start(loadFirstConversation: false, newConversation: false)
        })
        let pane = focusedPane
        pane.insert(tab, at: pane.tabs.firstIndex { $0.id == pane.selectedID }.map { $0 + 1 })
        navigateComposer(to: tab.store.loadedConversationID ?? ConversationStore.draftConversationPrefix + "new-conversation", tab: tab)
        objectWillChange.send()
        noteSelected(tab)
        return tab
    }
    /// Where the list, outline and composer are.
    @Published var tools = ToolWindowLayout() {
        didSet {
            guard tools != oldValue else { return }
            if let previousOwner = composerOwner(for: focused, tools: oldValue), let owner = composerOwner(for: focused), previousOwner != owner {
                let previous = composerDrafts.model(for: previousOwner, tab: focused)
                composerDrafts.move(previous, to: owner, tab: focused)
            }
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
    @Published private(set) var artifactSheet: ArtifactTab?
    @Published private(set) var sheetTab: ConversationTab?
    /// A conversation's composer over the window, to send once without showing the conversation.
    @Published private(set) var composerSheetTab: ConversationTab?
    /// A list target being read without changing a visible editor.
    private var pendingSideChat: BackgroundSideChat?
    /// Conversations resumed from the list through tabs that do not show.
    private var backgroundResumes: [UUID: BackgroundResume] = [:]
    private(set) var sheetSize = CGSize(width: 760, height: 640)
    /// The list's search text, shared by the sidebar and a floating list.
    @Published var listSearch = ""
    /// Opens a conversation window; set by the window's view, which has the action.
    var openWindow: ((ConversationWindowKind, WindowSeed) -> Void)?
    /// Opens a window of another scene by its ID, and the settings; set by the window's view too.
    var openScene: ((String) -> Void)?
    var openSettings: (() -> Void)?
    /// The command palette over the window, and what it lists.
    @Published var palette: PaletteMode?
    /// Where typing went before the palette took it, to go back there.
    weak var paletteReturnResponder: NSResponder?
    @Published var showsShortcutHelp = false
    /// Kept in front of other apps' windows.
    @Published var pinned = false
    let kind: ConversationWindowKind
    private(set) lazy var detached = DetachedTools(model: self)

    /// Windows by the panes they hold, for moving a tab between them.
    private static var all: [WeakWindow] = []
    private struct WeakWindow { weak var model: ConversationWindowModel? }
    /// Tabs on their way to a window being opened for them.
    private static var handoff: [UUID: EditorTab] = [:]

    static var openWindows: [ConversationWindowModel] {
        all.removeAll { $0.model == nil }
        return all.compactMap(\.model).filter { !$0.closed }
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
        return all.lazy.compactMap(\.model).first { $0.pane(holding: tab) != nil || $0.artifactSheet?.id == tab }
    }

    init(application: DeviceStore, kind: ConversationWindowKind, device: String? = nil) {
        self.application = application
        self.kind = kind
        self.device = device
        list = ConversationStore(device: application)
        let store = ConversationStore(device: application)
        let first = Pane(tabs: [ConversationTab(store: store, start: { store.start() })])
        layout = .pane(first.id)
        panes = [first.id: first]
        focusedPaneID = first.id
        follow(first)
        Self.all.append(WeakWindow(model: self))
        draftPersistence = composerDrafts.changes.sink { [weak self] in self?.persistWorkspace() }
        composerCursorWatch = composerDrafts.cursorChanges.sink { [weak self] in self?.objectWillChange.send() }
        composerCreationWatch = composerDrafts.creations.sink { [weak self] creation in
            guard let self, let input = self.composerDrafts.existing(creation.inputId) else { return }
            let tabs = self.conversationTabs.filter { tab in
                tab.store.isUnsentConversation && (self.composerOwner(for: tab) == .shared ? self.composerDrafts.isNewInput(input) : tab.composer === input)
            }
            guard !tabs.isEmpty else { return }
            for tab in tabs {
                let previous = tab.store.selectedConversationID ?? tab.store.loadedConversationID ?? ConversationStore.draftConversationPrefix + "new-conversation"
                self.composerDrafts.reassociate(input, from: previous, to: creation.conversationId, targetTab: tab, publish: false)
            }
            tabs.forEach { $0.store.followComposerCreation(creation) }
            self.objectWillChange.send()
        }
        // Window recovery belongs to the model, including changes made before
        // SwiftUI observes a scene. Read after @Published has committed its value.
        workspacePersistence = Publishers.MergeMany([
            $layout.dropFirst().map { _ in () }.eraseToAnyPublisher(),
            $panes.dropFirst().map { _ in () }.eraseToAnyPublisher(),
            $focusedPaneID.dropFirst().map { _ in () }.eraseToAnyPublisher(),
            $tabConversations.dropFirst().map { _ in () }.eraseToAnyPublisher(),
            $tabsRevision.dropFirst().map { _ in () }.eraseToAnyPublisher(),
            $tools.dropFirst().map { _ in () }.eraseToAnyPublisher(),
            $pinned.dropFirst().map { _ in () }.eraseToAnyPublisher(),
        ]).debounce(for: .milliseconds(150), scheduler: RunLoop.main)
            .sink { [weak self] in self?.persistWorkspace() }
    }

    var focusedPane: Pane { panes[focusedPaneID]! }
    /// The focused pane's tab, which the toolbar, commands and tool windows act on.
    var focusedEditor: EditorTab { focusedPane.selected }
    var hasConversationFocus: Bool { focusedEditor is ConversationTab }
    var visibleConversation: ConversationTab? { artifactSheet == nil ? (composerSheetTab ?? sheetTab ?? (focusedEditor as? ConversationTab)) : nil }
    var canTargetConversation: Bool { artifactSheet == nil && (sheetTab != nil || composerSheetTab != nil || hasConversationFocus) }
    private weak var lastConversation: ConversationTab?
    private lazy var emptyConversation = makeTab(opening: nil)
    var focused: ConversationTab { (focusedEditor as? ConversationTab) ?? lastConversation ?? allTabs.compactMap({ $0 as? ConversationTab }).first ?? emptyConversation }
    private func noteSelected(_ editor: EditorTab) {
        guard let tab = editor as? ConversationTab else { return }
        lastConversation = tab
        if composerOwner(for: tab) != nil { composerDrafts.focus(tab) }
        tab.store.noteFocused()
    }
    var isSplit: Bool { panes.count > 1 }
    var orderedPanes: [Pane] { layout.paneIDs.compactMap { panes[$0] } }
    var allTabs: [EditorTab] { orderedPanes.flatMap(\.tabs) }
    var tabCount: Int { panes.values.reduce(0) { $0 + $1.tabs.count } }

    var savedLayout: SavedWindowLayout {
        _ = tabsRevision
        // Temporary editors do not become restored blank tabs or split panes.
        var keptLayout = layout
        var kept: [UUID: SavedWindowLayout.SavedPane] = [:]
        for pane in orderedPanes {
            let tabs = pane.tabs.filter { ($0 as? ConversationTab)?.store.isSideChat != true }.map(\.id)
            if let first = tabs.first {
                kept[pane.id] = .init(tabs: tabs, selected: tabs.contains(pane.selectedID) ? pane.selectedID : first)
            } else if let remaining = keptLayout.removing(pane.id) { keptLayout = remaining }
        }
        if kept.isEmpty {
            let pane = focusedPane
            keptLayout = .pane(pane.id)
            kept[pane.id] = .init(tabs: [pane.selectedID], selected: pane.selectedID)
        }
        let ids = Set(kept.values.flatMap(\.tabs))
        return SavedWindowLayout(layout: keptLayout, panes: kept,
                                 conversations: tabConversations.filter { ids.contains($0.key) },
                                 focused: kept[focusedPaneID] == nil ? keptLayout.paneIDs[0] : focusedPaneID,
                                 tools: tools, pinned: pinned, composer: sharedComposer.savedDraft,
                                 composerWorkspace: composerDrafts.saved,
                                 artifacts: Dictionary(uniqueKeysWithValues: allTabs.compactMap { editor in (editor as? ArtifactTab).map { ($0.id, $0.document) } }),
                                 creationTargets: restoredCreationTargets.filter { ids.contains($0.key) }.merging(Dictionary(uniqueKeysWithValues: conversationTabs.compactMap { tab in tab.store.unsentConversation.map { (tab.id, $0) } }), uniquingKeysWith: { _, current in current }), windowKind: kind, windowDevice: SavedWindowLayout.ExecutionDevice(device))
    }

    /// Restore a saved arrangement, take a handed-over tab, or open `seed`'s
    /// conversation, then start reading. A lone new tab opens the list's
    /// first conversation.
    func start(restoring saved: SavedWindowLayout?, seed: WindowSeed?, persistenceID: String? = nil) {
        closed = false
        guard !started else { return }
        started = true
        self.persistenceID = persistenceID
        let sceneSaved = saved.flatMap {
            $0.isWhole && ($0.windowKind == nil || $0.windowKind == kind)
                && ($0.windowDevice == nil || $0.windowDevice == SavedWindowLayout.ExecutionDevice(device)) ? $0 : nil
        }
        let saved = persistenceID.flatMap { application.windowWorkspaceJournal.restore(id: $0, recoverLatest: seed == nil && sceneSaved == nil && kind == .workspace, kind: kind, device: device) }
            ?? sceneSaved
        list.start(loadFirstConversation: false)
        // A layout the system restores below replaces this one.
        switch kind {
        case .workspace: tools = application.rememberedToolLayout.layout
        case .single:
            tools = ToolWindowLayout.forSingleConversation()
            tools.toggleEditorOnly()
        }
        if let id = seed?.tab, let tab = Self.handoff.removeValue(forKey: id) {
            if let tab = tab as? ConversationTab, !tab.store.isSideChat, let conversationID = tab.store.loadedConversationID { composerDrafts.adopt(tab.composer, conversationID: conversationID, tab: tab) }
            replacePanes(with: [Pane(tabs: [tab])], layout: nil)
            tab.activate()
            noteSelected(tab)
        } else if let saved, saved.isWhole {
            sharedComposer.restoreSavedDraft(saved.composer)
            composerDrafts.restore(saved.composerWorkspace)
            if saved.composerWorkspace == nil, sharedComposer.hasUnfinishedInput { restoredComposerDraft = sharedComposer }
            let lone = saved.layout.paneIDs.count == 1 && saved.panes.values.first?.tabs.count == 1
            let restored = saved.layout.paneIDs.map { paneID in
                let savedPane = saved.panes[paneID]!
                let tabs: [EditorTab] = savedPane.tabs.map { tabID in
                    if let document = saved.artifacts?[tabID] { return ArtifactTab(id: tabID, document: document) }
                    let store = ConversationStore(device: application)
                    let conversationID = saved.conversations[tabID]
                    let draft = saved.creationTargets?[tabID] ?? conversationID.flatMap({ saved.unsentConversations?[$0] })
                    // A restored tab shows its conversation's title before it
                    // is first shown; only reading waits for activation.
                    if draft == nil, let conversationID, !conversationID.hasPrefix(ConversationStore.draftConversationPrefix) {
                        store.noteRestoredConversation(conversationID)
                    }
                    return ConversationTab(id: tabID, store: store, start: {
                        if let draft { store.restoreUnsentConversation(draft) }
                        else { store.start(loadFirstConversation: lone, newConversation: !lone && conversationID == nil, opening: conversationID) }
                    })
                }
                return Pane(id: paneID, tabs: tabs, selected: savedPane.selected)
            }
            replacePanes(with: restored, layout: saved.layout)
            tabConversations = saved.conversations
            restoredCreationTargets = saved.creationTargets ?? [:]
            focusedPaneID = saved.focused
            tools = saved.tools.normalized()
            pinned = saved.pinned
            for pane in restored { pane.selected.activate() }
        } else if let id = seed?.conversationID {
            let store = ConversationStore(device: application)
            replacePanes(with: [Pane(tabs: [ConversationTab(store: store, start: { store.start(loadFirstConversation: false, opening: id) })])], layout: nil)
            focused.activate()
        } else {
            focused.activate()
        }
        installMouseMonitor()
        detached.sync()
        if let seed, WindowPresentation.notificationTargets.remove(seed.id) != nil, let id = seed.conversationID {
            focusNotificationTarget(id)
        }
    }

    /// The panes before reading starts: the first pane's untouched tab gives way.
    private func replacePanes(with replacement: [Pane], layout: PaneLayout?) {
        for tab in allTabs { tab.close() }
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
        guard hasConversationFocus else { return openInNewTab(conversationID) }
        if select(showing: conversationID) { return }
        if focused.store.isSideChat { openInNewTab(conversationID) }
        else { focused.store.openLinkedConversation(id: conversationID) }
    }

    /// Notifications preserve the current editor. Reuse a conversation tab
    /// in this device's windows, otherwise add one in its last focused window.
    static func openNotificationConversation(_ conversationID: String, application: DeviceStore) -> ConversationWindowModel? {
        let windows = openWindows.filter { $0.application === application }
        let existing = windows.first { $0.tab(showing: conversationID) != nil }
        guard let target = existing ?? windows.first(where: { $0.focused.store === application.focusedConversation }) ?? windows.first else { return nil }
        target.openInNewTab(conversationID)
        return target
    }

    func focusNotificationTarget(_ conversationID: String) {
        if application.waitingConversationIDs.contains(conversationID) { focused.focusInteraction() }
        else { composer(for: focused).focusWhenShown() }
    }

    /// Ordinary conversation commands use the chosen editor presentation.
    func openSideChat(from source: ConversationTab? = nil) {
        let source = source ?? commandTab
        guard let target = composerContext(for: source).sideChatTarget else { return }
        openSideChat(target, presentation: sideChatPresentation, parent: source.id)
    }

    /// A list action opens over the current editor, without selecting its target.
    func openSideChat(on conversationID: String) {
        endSideChatOpening()
        if let source = conversationTabs.first(where: { $0.store.loadedConversationID == conversationID }),
           let target = source.store.sideChatTarget {
            return openSideChat(target, presentation: .sheet, parent: nil)
        }
        let request = BackgroundSideChat(tab: makeTab(opening: conversationID))
        pendingSideChat = request
        request.watch = request.tab.store.objectWillChange.receive(on: RunLoop.main).sink { [weak self] _ in self?.advanceSideChat() }
        request.timeout = Timer.scheduledTimer(withTimeInterval: 30, repeats: false) { [weak self] _ in
            Task { @MainActor in self?.endSideChatOpening(problem: "打开侧聊超时") }
        }
        request.tab.activate()
    }

    private func advanceSideChat() {
        guard let source = pendingSideChat?.tab.store, source.loadedConversationID != nil,
              !source.isLoading, source.nextTurnSettled else { return }
        guard let target = source.sideChatTarget else { return endSideChatOpening(problem: "此会话当前不能侧聊") }
        endSideChatOpening()
        openSideChat(target, presentation: .sheet, parent: nil)
    }

    private func endSideChatOpening(problem: String? = nil) {
        pendingSideChat?.close()
        pendingSideChat = nil
        if let problem { list.recordProblem(problem, source: "打开侧聊") }
    }

    private func openSideChat(_ target: SideChatTarget, presentation: SideChatPresentation, parent: UUID?) {
        let tab: ConversationTab
        if let existing = conversationTabs.first(where: {
            $0.store.sideChatContext?.conversationID == target.conversationID && $0.store.sideChatContext?.runtimeID == target.runtimeID
        }) { tab = existing }
        else {
            let store = ConversationStore(device: application, sideChat: target)
            tab = ConversationTab(store: store) { store.start() }
        }
        tab.sideChatParentID = parent
        presentSideChat(tab, as: presentation)
    }

    /// Move the same tab, composer and reading state through the normal containers.
    func presentSideChat(_ tab: ConversationTab, as presentation: SideChatPresentation) {
        if presentation == .sheet, sheetTab === tab { return tab.composer.focusWhenShown() }
        if presentation == .split, let pane = pane(holding: tab.id) { selectTab(tab.id); focus(pane.id); return }
        if sheetTab === tab { sheetTab = nil }
        if let pane = pane(holding: tab.id) {
            if tabCount == 1 { pane.insert(makeTab(opening: nil)) }
            _ = detach(tab.id, emptying: .keep)
        }
        switch presentation {
        case .sheet: presentInSheet(tab)
        case .split:
            let source = tab.sideChatParentID.flatMap { pane(holding: $0)?.id } ?? focusedPaneID
            addPane(with: tab, beside: source, edge: .trailing)
        }
        focusConversation(tab)
        tab.composer.focusWhenShown()
    }

    private func closeSideChats(from source: UUID) {
        for window in Self.openWindows {
            for tab in window.conversationTabs where tab.sideChatParentID == source {
                if window.sheetTab === tab { window.closeSheet() }
                else { window.closeTab(tab.id) }
            }
        }
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
    func openLinked(_ conversationID: String, from tab: ConversationTab, opening: LinkedConversationOpening? = nil) {
        if tab.store.isSideChat { openInNewTab(conversationID); return }
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
        presentInSheet(makeTab(opening: conversationID))
    }

    private func presentInSheet(_ tab: ConversationTab) {
        closeSheet()
        if let content = window?.contentLayoutRect.size {
            sheetSize = CGSize(width: min(max(content.width * 0.8, 560), max(content.width - 40, 560)),
                               height: min(max(content.height * 0.85, 480), max(content.height - 40, 480)))
        }
        tab.activate()
        sheetTab = tab
        focusConversation(tab)
    }

    func closeSheet() {
        guard let tab = sheetTab else { return }
        sheetTab = nil
        closeSideChats(from: tab.id)
        tab.close()
        focusConversation(focused)
    }

    /// Open `conversationID`'s composer in a sheet over the window; the list
    /// selection and the shown conversations stay.
    func openComposerSheet(_ conversationID: String) {
        closeComposerSheet()
        let tab = makeTab(opening: conversationID)
        tab.activate()
        tab.composer.focusWhenShown()
        composerSheetTab = tab
    }

    func closeComposerSheet() {
        guard let tab = composerSheetTab else { return }
        composerSheetTab = nil
        tab.store.close()
    }

    /// The composer sheet's conversation in a tab of this window, one that shows it or a new one.
    func openComposerSheetConversation() {
        guard let tab = composerSheetTab, let id = tab.store.selectedConversationID else { return }
        closeComposerSheet()
        if !select(showing: id) { openInNewTab(id) }
    }

    /// Resume a conversation's stopped or failed turn from the list, as its
    /// composer's Continue does, with the next-turn choices it keeps. A tab
    /// showing it does it; otherwise one that does not show opens it first.
    func resumeFromList(_ conversationID: String) {
        if let shown = Self.openWindows.lazy.filter({ $0.application === self.application }).compactMap({ $0.tab(showing: conversationID)?.1 }).first {
            guard shown.store.canResume else { return list.recordProblem("此会话当前不能继续", source: "继续会话") }
            return shown.store.continueTurn()
        }
        let tab = makeTab(opening: conversationID)
        tab.activate()
        let id = tab.id
        let resume = BackgroundResume(tab: tab, accepted: tab.store.acceptedSubmissions)
        resume.watch = tab.store.objectWillChange.receive(on: RunLoop.main).sink { [weak self] _ in self?.advanceBackgroundResume(id) }
        resume.timeout = Timer.scheduledTimer(withTimeInterval: 30, repeats: false) { [weak self] _ in
            Task { @MainActor in self?.endBackgroundResume(id, problem: "继续会话超时") }
        }
        backgroundResumes[id] = resume
    }

    private func advanceBackgroundResume(_ id: UUID) {
        guard let resume = backgroundResumes[id] else { return }
        let store = resume.tab.store
        if resume.requested {
            if store.acceptedSubmissions != resume.accepted { return endBackgroundResume(id, problem: nil) }
            // A refused resume records its own problem.
            if store.composerRequest == nil && !store.isLoading { endBackgroundResume(id, problem: nil) }
            return
        }
        guard store.loadedConversationID != nil, !store.isLoading, store.nextTurnSettled else { return }
        guard store.canResume else { return endBackgroundResume(id, problem: "此会话当前不能继续") }
        resume.requested = true
        store.continueTurn()
    }

    private func endBackgroundResume(_ id: UUID, problem: String?) {
        guard let resume = backgroundResumes.removeValue(forKey: id) else { return }
        resume.timeout?.invalidate()
        resume.watch = nil
        resume.tab.store.close()
        if let problem { list.recordProblem(problem, source: "继续会话") }
    }

    /// Development check of the list's run actions without driving the
    /// interface: on the first listed conversation, send from the composer
    /// sheet, stop from the list, then continue from the list.
    func checkListActions(step: Int = 0, attempts: Int = 0) {
        guard attempts < 240 else { return Self.checkLog("CHECK list-actions: step \(step) timed out") }
        DispatchQueue.main.asyncAfter(deadline: .now() + 0.5) { [self] in
            let next = { (step: Int) in self.checkListActions(step: step) }
            let retry = { self.checkListActions(step: step, attempts: attempts + 1) }
            // One no tab shows, so continuing goes through a tab that does not show.
            guard let id = application.conversations.first(where: { tab(showing: $0.id) == nil })?.id ?? application.conversations.first?.id else { return retry() }
            switch step {
            case 0:
                openComposerSheet(id)
                Self.checkLog("CHECK list-actions: sheet opened"); next(1)
            case 1:
                guard let tab = composerSheetTab, tab.store.canSend, tab.store.nextTurnSettled else { return retry() }
                tab.composer.restore("SHEET_MESSAGE HOLD", attachments: [])
                // Long enough to capture the sheet with its text.
                DispatchQueue.main.asyncAfter(deadline: .now() + 4) { tab.send(); next(2) }
            case 2:
                guard composerSheetTab == nil else { return retry() }
                Self.checkLog("CHECK list-actions: sheet closed after the send was accepted"); next(3)
            case 3:
                guard application.isStoppable(id) else { return retry() }
                application.stop([id])
                Self.checkLog("CHECK list-actions: stop requested from the list"); next(4)
            case 4:
                guard application.resumableConversations[id] != nil, !application.isConversationRunning(id) else { return retry() }
                resumeFromList(id)
                Self.checkLog("CHECK list-actions: continue requested from the list"); next(5)
            case 5:
                guard backgroundResumes.isEmpty, application.resumableConversations[id] == nil else { return retry() }
                Self.checkLog("CHECK list-actions: done")
            default: break
            }
        }
    }

    /// Synthetic check of editor-level presentation and the shared conversation flow.
    func checkSideChat(step: Int = 0, attempts: Int = 0, expected: ConversationTab? = nil, original: ConversationTab? = nil) {
        guard attempts < 180 else { return Self.checkLog("CHECK side-chat: step \(step) timed out") }
        DispatchQueue.main.asyncAfter(deadline: .now() + 0.5) { [self] in
            let next = { (step: Int, chat: ConversationTab?, source: ConversationTab?) in
                self.checkSideChat(step: step, expected: chat, original: source)
            }
            let retry = { self.checkSideChat(step: step, attempts: attempts + 1, expected: expected, original: original) }
            guard let id = application.conversations.first?.id else { return retry() }
            switch step {
            case 0:
                sideChatPresentation = .split
                openInNewTab()
                let source = focused
                openSideChat(on: id)
                next(1, nil, source)
            case 1:
                guard let chat = sheetTab, chat.store.canSend, !chat.store.isLoading else { return retry() }
                guard focused === original, application.focusedConversation === original?.store,
                      conversationTabs.allSatisfy({ !$0.store.isSideChat }), original?.store.loadedConversationID != id else {
                    return Self.checkLog("CHECK side-chat: list action changed the current editor")
                }
                let sideID = chat.store.loadedConversationID
                chat.store.createConversation()
                guard chat.store.loadedConversationID == sideID else { return Self.checkLog("CHECK side-chat: new conversation replaced the side session") }
                chat.composer.restore("SIDE_QUESTION_FROM_APP", attachments: [])
                chat.send()
                Self.checkLog("CHECK side-chat: list opened sheet without changing editor; shared send")
                next(2, chat, original)
            case 2:
                guard let chat = expected, chat.store.canSend, !chat.store.isLoading,
                      chat.store.transcript.rows.contains(where: { $0.message.role == .assistant }) else { return retry() }
                focusConversation(chat)
                perform(.find)
                guard commandTab === chat, chat.find.isShown, original?.find.isShown == false else {
                    return Self.checkLog("CHECK side-chat: find targeted another editor")
                }
                chat.find.query = "SIDE_REPLY"
                chat.composer.restore("A FOLLOW-UP STILL BEING TYPED", attachments: [])
                Self.checkLog("CHECK side-chat: sheet answered; shared find")
                DispatchQueue.main.asyncAfter(deadline: .now() + 10) { next(3, chat, original) }
            case 3:
                guard let chat = expected else { return }
                presentSideChat(chat, as: .split)
                guard sheetTab == nil, pane(holding: chat.id) != nil,
                      pane(holding: chat.id)?.id != pane(holding: original!.id)?.id,
                      chat.composer.submission == "A FOLLOW-UP STILL BEING TYPED",
                      savedLayout.isWhole, savedLayout.panes.values.allSatisfy({ !$0.tabs.contains(chat.id) }) else {
                    return Self.checkLog("CHECK side-chat: editor split or restoration is wrong")
                }
                Self.checkLog("CHECK side-chat: peer editor split; draft and reading retained; restoration excludes side")
                DispatchQueue.main.asyncAfter(deadline: .now() + 10) { next(4, chat, original) }
            case 4:
                guard let chat = expected else { return }
                // The retained find field may own focus; two composers may not.
                guard !(chat.composer.isFocused && original?.composer.isFocused == true) else {
                    return Self.checkLog("CHECK side-chat: stale composer focus after moving editor: side=\(chat.composer.isFocused), source=\(original?.composer.isFocused ?? false)")
                }
                sideChatPresentation = .sheet
                guard sheetTab === chat, pane(holding: chat.id) == nil, focused === original,
                      chat.composer.submission == "A FOLLOW-UP STILL BEING TYPED", chat.find.query == "SIDE_REPLY" else {
                    return Self.checkLog("CHECK side-chat: sheet move lost state")
                }
                chat.find.isShown = false
                chat.send()
                next(5, chat, original)
            case 5:
                guard let chat = expected, chat.store.canSend, chat.store.acceptedSubmissions == 2 else { return retry() }
                chat.composer.restore("SIDE_Q_HOLD", attachments: [])
                chat.send()
                next(6, chat, original)
            case 6:
                guard let chat = expected, chat.store.canCancel, !chat.store.isLoading else { return retry() }
                focusConversation(chat)
                guard canPerform(.stop), !canPerform(.attach), !canPerform(.chooseModel) else {
                    return Self.checkLog("CHECK side-chat: wrong command capabilities")
                }
                perform(.stop)
                next(7, chat, original)
            case 7:
                guard let chat = expected, chat.store.canSend else { return retry() }
                chat.composer.restore("SIDE_Q_AFTER_STOP", attachments: [])
                chat.send()
                next(8, chat, original)
            case 8:
                guard let chat = expected, chat.store.canSend, chat.store.acceptedSubmissions == 4 else { return retry() }
                chat.composer.restore("SIDE_Q_CLOSE HOLD", attachments: [])
                chat.send()
                next(9, chat, original)
            case 9:
                guard let chat = expected, chat.store.canCancel, !chat.store.isLoading else { return retry() }
                perform(.closeTab)
                guard commandTab === original, !conversationTabs.contains(where: { $0 === chat }) else {
                    return Self.checkLog("CHECK side-chat: close kept command target")
                }
                original?.store.openLinkedConversation(id: id)
                next(10, nil, original)
            case 10:
                guard let source = original, source.store.sideChatTarget != nil else { return retry() }
                sideChatPresentation = .split
                openSideChat(from: source)
                next(11, focused, original)
            case 11:
                guard let chat = expected, chat.store.canSend else { return retry() }
                guard chat.store.isSideChat, chat.sideChatParentID == original?.id else {
                    return Self.checkLog("CHECK side-chat: menu did not open a peer editor")
                }
                original?.store.createConversation()
                next(12, chat, original)
            case 12:
                guard let chat = expected, !conversationTabs.contains(where: { $0 === chat }) else { return retry() }
                guard savedLayout.isWhole else { return Self.checkLog("CHECK side-chat: invalid saved layout") }
                Self.checkLog("CHECK side-chat: shared stop and follow-up; closed while answering; source lifecycle; done")
            default: break
            }
        }
    }

    /// Unbuffered, so a check launched with `open --stderr` shows its progress.
    private static func checkLog(_ line: String) { FileHandle.standardError.write(Data((line + "\n").utf8)) }

    func openArtifact(_ document: ArtifactDocument) {
        // The document outlives its original sheet and window. Look through
        // the same device's presentations before creating another object.
        for owner in Self.openWindows where owner.application === application {
            if let tab = owner.allTabs.compactMap({ $0 as? ArtifactTab }).first(where: { $0.document.id == document.id }) {
                owner.selectTab(tab.id)
                if owner !== self { owner.window?.makeKeyAndOrderFront(nil) }
                return
            }
            if owner.artifactSheet?.document.id == document.id {
                if owner !== self { owner.window?.makeKeyAndOrderFront(nil) }
                return
            }
        }
        artifactSheet = ArtifactTab(document: document)
    }
    func closeArtifactSheet() { artifactSheet = nil }
    func moveArtifactSheetToTab() {
        guard let tab = artifactSheet else { return }
        artifactSheet = nil
        focusedPane.insert(tab)
        tabsRevision &+= 1
    }
    func moveArtifactSheetToWindow(at point: NSPoint? = nil) {
        guard let tab = artifactSheet, openWindow != nil else { return }
        artifactSheet = nil
        hand(tab, toNewWindowIn: newWindowFrame(size: CGSize(width: 820, height: 600), at: point))
    }

    /// The sheet's conversation, as it stands, in a new tab after the focused pane's shown tab.
    func moveSheetToTab() {
        guard let tab = sheetTab else { return }
        sheetTab = nil
        let pane = focusedPane
        pane.insert(tab, at: pane.tabs.firstIndex { $0.id == pane.selectedID }.map { $0 + 1 })
        noteSelected(tab)
    }

    /// The sheet's conversation, as it stands, in a conversation window of its own.
    func moveSheetToNewWindow() {
        guard let tab = sheetTab, openWindow != nil else { return }
        sheetTab = nil
        hand(tab, toNewWindowIn: newWindowFrame(size: sheetSize, at: nil))
    }

    /// Show in a new tab a conversation the core has just opened, such as an
    /// edit's fork, as `result` gives it.
    func openInNewTab(adopting result: SnapshotResult) {
        guard let id = result.snapshot?.conversation.id else { return }
        if select(showing: id) { return }
        let store = ConversationStore(device: application)
        let pane = focusedPane
        let index = pane.tabs.firstIndex { $0.id == pane.selectedID }.map { $0 + 1 }
        pane.insert(ConversationTab(store: store, start: {
            store.start(loadFirstConversation: false)
            store.adopt(result)
        }), at: index)
        navigateComposer(to: id, tab: focused)
        focused.store.noteFocused()
    }

    /// Divide the focused pane; the new pane takes focus and opens
    /// `conversationID`, or starts a new conversation.
    func split(_ axis: PaneSplit.Axis, opening conversationID: String? = nil) {
        if let conversationID, select(showing: conversationID) { return }
        addPane(with: makeTab(opening: conversationID), beside: focusedPaneID, edge: axis == .horizontal ? .trailing : .bottom)
    }

    private func makeTab(opening conversationID: String?) -> ConversationTab {
        let store = ConversationStore(device: application)
        return ConversationTab(store: store, start: {
            store.start(loadFirstConversation: false, newConversation: conversationID == nil, opening: conversationID)
        })
    }

    /// Select the tab showing a conversation, if one does.
    private func select(showing conversationID: String) -> Bool {
        guard let (_, tab) = tab(showing: conversationID) else { return false }
        selectTab(tab.id)
        return true
    }

    func tab(showing conversationID: String) -> (Pane, ConversationTab)? {
        for pane in orderedPanes {
            if let tab = pane.tabs.compactMap({ $0 as? ConversationTab }).first(where: { $0.store.selectedConversationID == conversationID || $0.store.reachingConversationID == conversationID }) { return (pane, tab) }
        }
        return nil
    }

    func pane(holding tab: UUID) -> Pane? {
        panes.values.first { $0.tabs.contains { $0.id == tab } }
    }

    // MARK: Tabs

    func selectTab(_ id: UUID) {
        guard let pane = pane(holding: id) else { return }
        if pane.selectedID != id, let tab = pane.tabs.first(where: { $0.id == id }) as? ConversationTab,
           !(pane.selected is ArtifactTab && tab === lastConversation) {
            navigateComposer(to: tab.store.loadedConversationID ?? tabConversations[id] ?? "tab:\(id)", tab: tab)
        }
        pane.select(id)
        focus(pane.id)
        noteSelected(pane.selected)
    }

    /// The tab after (or before) the focused pane's shown one.
    func selectNeighbourTab(forward: Bool) {
        let pane = focusedPane
        guard pane.tabs.count > 1, let index = pane.tabs.firstIndex(where: { $0.id == pane.selectedID }) else { return }
        selectTab(pane.tabs[(index + (forward ? 1 : pane.tabs.count - 1)) % pane.tabs.count].id)
    }

    /// Close a tab: its pane's last tab closes the pane, the window's last closes the window.
    func closeTab(_ id: UUID) {
        let navigating = id == focusedEditor.id && tabCount > 1 && focusedEditor is ConversationTab
        if navigating, !tools.panesHaveComposer { composerDrafts.checkpoint() }
        closeSideChats(from: id)
        guard let tab = detach(id, emptying: .close) else { return }
        if let closed = tab as? ConversationTab { noteRecentlyClosed(closed) }
        tab.close()
        if navigating, let next = focusedEditor as? ConversationTab {
            navigateComposer(to: next.store.loadedConversationID ?? "tab:\(next.id)", tab: next, checkpointPrevious: false)
        }
    }

    /// Side chats end with their source and are not reopenable.
    private func noteRecentlyClosed(_ tab: ConversationTab) {
        guard !tab.store.isSideChat else { return }
        var target = tab.store.isUnsentConversation ? nil : tab.store.selectedConversationID ?? tabConversations[tab.id]
        if target?.hasPrefix(ConversationStore.draftConversationPrefix) == true { target = nil }
        recentlyClosed.append(target)
        if recentlyClosed.count > 25 { recentlyClosed.removeFirst() }
    }
    /// Reopen the most recently closed tab: its conversation in a tab after
    /// the shown one; a closed unsent conversation reopens as a fresh new
    /// one. Entries whose conversation was deleted meanwhile are skipped.
    func reopenClosedTab() {
        while let target = recentlyClosed.popLast() {
            guard let id = target else { openInNewTab(); return }
            guard application.conversations.contains(where: { $0.id == id }) else { continue }
            openInNewTab(id); return
        }
    }
    var canReopenClosedTab: Bool {
        recentlyClosed.contains { entry in entry == nil || application.conversations.contains(where: { $0.id == entry }) }
    }
    func closeOtherTabs(_ id: UUID) {
        guard let pane = pane(holding: id) else { return }
        for tab in pane.tabs where tab.id != id { closeTab(tab.id) }
    }

    /// Close a pane with all its tabs.
    func closePane(_ id: UUID) {
        guard isSplit, let pane = panes[id] else { return }
        for tab in pane.tabs {
            closeSideChats(from: tab.id)
            forgetTab(tab.id)
            tab.close()
        }
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
    private func detach(_ id: UUID, emptying: Emptying) -> EditorTab? {
        if let sheet = artifactSheet, sheet.id == id { artifactSheet = nil; return sheet }
        guard let pane = pane(holding: id), let tab = pane.tabs.first(where: { $0.id == id }) else { return nil }
        if let tab = tab as? ConversationTab, !tab.store.isSideChat, let conversationID = tab.store.loadedConversationID {
            tab.composer = composerDrafts.model(for: .conversation(conversationID), tab: tab)
        }
        if pane.remove(id) {
            if pane.id == focusedPaneID { noteSelected(pane.selected) }
        } else if isSplit {
            removePane(pane.id)
        } else {
            switch emptying {
            case .keep: return nil
            case .leave where hasToolWindows:
                pane.insert(makeTab(opening: nil))
                pane.remove(id)
                noteSelected(pane.selected)
            case .close, .leave: window?.close()
            }
        }
        forgetTab(id)
        tabsRevision &+= 1
        return tab
    }

    private func forgetTab(_ id: UUID) {
        subscriptions[id] = nil
        tabConversations[id] = nil
    }

    private func removePane(_ id: UUID) {
        guard let remaining = layout.removing(id) else { return }
        let previous = composer(for: focused)
        let previousOwner = composerOwner(for: focused)
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
        if !tools.panesHaveComposer, let previousOwner, let owner = composerOwner(for: focused), previousOwner != owner {
            composerDrafts.move(previous, to: owner, tab: focused)
        }
    }

    private func addPane(with tab: EditorTab, beside target: UUID, edge: PaneEdge) {
        preservingComposerLayout = true
        defer { preservingComposerLayout = false }
        let previous = composer(for: focused)
        let previousOwner = composerOwner(for: focused)
        let pane = Pane(tabs: [tab])
        panes[pane.id] = pane
        layout = layout.inserting(pane.id, beside: target, edge: edge)
        follow(pane)
        focusedPaneID = pane.id
        tab.activate()
        noteSelected(tab)
        let inputTarget = (tab as? ConversationTab) ?? focused
        if !tools.panesHaveComposer, let previousOwner, let owner = composerOwner(for: inputTarget), previousOwner != owner {
            composerDrafts.move(previous, to: owner, tab: inputTarget)
        }
    }

    // MARK: Moving

    /// A tab, or a conversation from the list, dropped on a pane: among its
    /// tabs (at `index`, else at the end) or beside an edge. A tab comes
    /// from wherever it is, this window or another; a conversation a tab of
    /// this window shows moves that tab; any other opens in a new tab.
    func drop(_ link: URL, on target: UUID, zone: PaneDropZone, at index: Int? = nil, from sourceWindow: ConversationWindowModel? = nil) {
        guard let targetPane = panes[target] else { return }
        ListDragWatch.shared.accepted()
        if let sourceWindow, !acceptsEditor(from: sourceWindow) { return }
        let tabID = TabDragSource.shared.tab(dropping: link)
            ?? ConversationLink.conversation(in: link).flatMap { tab(showing: $0.conversationID)?.1.id }
        // A tab closed since its drag began leaves its conversation to open anew.
        guard let tabID, let source = Self.owning(tab: tabID) else {
            guard let conversationID = ConversationLink.conversation(in: link)?.conversationID else { return }
            if let (pane, existing) = tab(showing: conversationID) { pane.select(existing.id); focus(pane.id); return }
            let tab = makeTab(opening: conversationID)
            switch zone {
            case .center: targetPane.insert(tab, at: index); focus(target); noteSelected(tab)
            case .edge(let edge): addPane(with: tab, beside: target, edge: edge)
            }
            return
        }
        guard acceptsEditor(from: source) else { return }
        let sourcePane = source.pane(holding: tabID)
        switch zone {
        case .center where sourcePane === targetPane:
            targetPane.move(tabID, to: index ?? targetPane.tabs.count)
            focus(target)
        case .edge where sourcePane === targetPane && targetPane.tabs.count == 1:
            // A pane's only tab beside its own pane has nowhere new to go.
            break
        default:
            guard let tab = source.detach(tabID, emptying: .leave) else { return }
            if source !== self { source.releaseComposerForHandoff(tab) }
            adopt(tab)
            switch zone {
            case .center: targetPane.insert(tab, at: index); focus(target); noteSelected(tab)
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
        if artifactSheet?.id == id { return moveArtifactSheetToWindow(at: point) }
        guard canMoveTabToNewWindow, openWindow != nil, let pane = pane(holding: id) else { return }
        let frame = newWindowFrame(size: view(of: pane.id)?.bounds.size, at: point)
        guard let tab = detach(id, emptying: .leave) else { return }
        hand(tab, toNewWindowIn: frame)
    }

    /// A tab no window holds, in a conversation window opened for it.
    private func hand(_ tab: EditorTab, toNewWindowIn frame: NSRect?) {
        guard let openWindow else { return }
        releaseComposerForHandoff(tab)
        Self.handoff[tab.id] = tab
        let existing = Set(NSApp.windows.map(\.windowNumber))
        openWindow(.single, WindowSeed(tab: tab.id, device: device))
        if let frame { Self.place(windowNotIn: existing, frame: frame, attempts: 20) }
    }

    /// Only crossing a window boundary separates local editor state. Closing
    /// a tab or rearranging panes in this window keeps the existing model.
    private func releaseComposerForHandoff(_ editor: EditorTab) {
        guard let tab = editor as? ConversationTab, !tab.store.isSideChat,
              let replacement = composerDrafts.releaseEditingModel(tab.composer) else { return }
        for remaining in conversationTabs where remaining !== tab && remaining.composer === tab.composer {
            remaining.composer = replacement
        }
        if restoredComposerDraft === tab.composer { restoredComposerDraft = replacement }
    }

    /// Open a conversation in a conversation window of its own; a tab of this
    /// window showing it moves there. At `point`, the window's top left is there.
    func openInNewWindow(_ conversationID: String, at point: NSPoint? = nil) {
        if let (_, tab) = tab(showing: conversationID) { return moveToNewWindow(tab.id, at: point) }
        guard let openWindow else { return }
        let frame = newWindowFrame(size: nil, at: point)
        let existing = Set(NSApp.windows.map(\.windowNumber))
        openWindow(.single, WindowSeed(conversationID: conversationID, device: device))
        if let frame { Self.place(windowNotIn: existing, frame: frame, attempts: 20) }
    }

    /// A conversation window's frame: a pane's size (else a usual one) under
    /// this window's title bar and toolbar, at `point` or set off from this window.
    private func newWindowFrame(size: CGSize?, at point: NSPoint?) -> NSRect? {
        guard let window else { return nil }
        let chrome = window.frame.height - window.contentLayoutRect.height
        let fullScreen = window.styleMask.contains(.fullScreen)
        let requestedWidth = size?.width ?? 720
        let requestedHeight = (size?.height ?? window.contentLayoutRect.height) + chrome
        let width = max(fullScreen ? min(requestedWidth, 720) : requestedWidth, 520)
        let height = max(fullScreen ? min(requestedHeight, 760) : requestedHeight, 420)
        let topLeft = point ?? NSPoint(x: window.frame.minX + 28, y: window.frame.maxY - 28)
        var frame = NSRect(x: topLeft.x, y: topLeft.y - height, width: width, height: height)
        if let screen = (NSScreen.screens.first { $0.frame.contains(topLeft) } ?? window.screen)?.visibleFrame {
            frame.size.height = min(frame.height, screen.height)
            frame.origin.x = min(max(frame.minX, screen.minX), screen.maxX - frame.width)
            frame.origin.y = min(max(frame.minY, screen.minY), screen.maxY - frame.height)
        }
        return frame
    }

    /// Window-local navigation and input always refer to one execution device.
    private func acceptsEditor(from source: ConversationWindowModel) -> Bool {
        guard source.application !== application else { return true }
        list.recordProblem("标签页只能移到查看同一设备的窗口。", source: "移动标签页")
        return false
    }

    /// A tab arriving from another window.
    private func adopt(_ tab: EditorTab) {
        if let tab = tab as? ConversationTab, !tab.store.isSideChat, let conversationID = tab.store.loadedConversationID { composerDrafts.adopt(tab.composer, conversationID: conversationID, tab: tab) }
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
        noteSelected(pane.selected)
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

    private func follow(_ editor: EditorTab) {
        guard let tab = editor as? ConversationTab, !tab.store.isSideChat else { return }
        var watches = Set<AnyCancellable>()
        tab.store.composerEvents.sink { [weak self, weak tab] navigation in
            guard let self, let tab else { return }
            if case let .created(previous, id, runtimeID, input) = navigation {
                self.composerDrafts.reassociate(input, from: previous, to: id, runtimeID: runtimeID, targetTab: tab)
                return
            }
            if case .creationTargetChanged = navigation { self.persistWorkspace(); return }
            guard !self.preservingComposerLayout else { return }
            guard tab === self.focused else { return }
            let id: String
            switch navigation {
            case .conversation(let conversation): id = conversation
            case .newConversation: id = tab.store.loadedConversationID ?? ConversationStore.draftConversationPrefix + "new-conversation"
            case .created, .creationTargetChanged: return
            }
            self.navigateComposer(to: id, tab: tab)
            self.objectWillChange.send()
        }.store(in: &watches)
        tab.store.$selectedConversationID.sink { [weak self] id in
            guard let self else { return }
            if id != nil { self.restoredCreationTargets[tab.id] = nil }
            DispatchQueue.main.async { [weak self, weak tab] in
                guard let self, let tab else { return }
                self.composerDrafts.removeConversations(self.application.deletedConversationIDs)
                if tab === self.focused, tab.store.loadedConversationID != nil, let restored = self.restoredComposerDraft {
                    self.restoredComposerDraft = nil
                    if let owner = self.composerOwner(for: tab) { self.composerDrafts.move(restored, to: owner, tab: tab) }
                }
            }
            if self.tabConversations[tab.id] != id {
                self.closeSideChats(from: tab.id)
                self.tabConversations[tab.id] = id
            }
        }.store(in: &watches)
        subscriptions[tab.id] = watches
    }

    private func noteClick(_ event: NSEvent) {
        clickedFindArea = findAreas.first { _, anchor in
            guard let view = anchor(), view.window === event.window else { return false }
            return view.bounds.contains(view.convert(event.locationInWindow, from: nil))
        }?.key
    }

    /// ⌘F: the find of the tool window that has the focus, else the shown
    /// conversation's messages. Only the list searches so far; the outline
    /// and linked agents would join `findAreas`.
    func find() {
        let listHasFocus = findAreas[.list]?()?.window === NSApp.keyWindow && focusIsIn(findAreas[.list])
        if listHasFocus || (!commandTab.store.isSideChat && clickedFindArea == .list) {
            if listIsBlock { listSearchPopover = true } else { listSearchRequest &+= 1 }
        } else { commandTab.beginFind() }
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
    /// The tool window last clicked in, if the last click was in one: clicking a
    /// conversation in the list moves the focus on to the composer, but the
    /// user is still working in the list.
    private var clickedFindArea: ToolWindowKind?

    func registerFindArea(_ kind: ToolWindowKind, anchor: @escaping () -> NSView?) { findAreas[kind] = anchor }

    /// The conversation list's rows have the keyboard (not its search field).
    var listHasKeyboard: Bool {
        guard let responder = (findAreas[.list]?()?.window)?.firstResponder, !(responder is NSText) else { return false }
        return focusIsIn(findAreas[.list])
    }
    /// The list's own keys while its rows have the keyboard; set by the list's view.
    var listKeyHandler: ((NSEvent) -> Bool)?

    /// Typing in the list goes on in its search field.
    func searchListTyping(_ text: String) {
        find()
        // The field takes the keyboard on the next turns of the run loop.
        DispatchQueue.main.asyncAfter(deadline: .now() + 0.05) {
            guard let editor = NSApp.keyWindow?.firstResponder as? NSTextView, editor.isFieldEditor else { return }
            editor.insertText(text, replacementRange: editor.selectedRange())
        }
    }

    /// The first responder is in the view the anchor marks.
    private func focusIsIn(_ anchor: (() -> NSView?)?) -> Bool {
        guard let area = anchor?(), let window = area.window, let responder = window.firstResponder as? NSView else { return false }
        // The middle of what shows: a table's bounds reach over all its rows.
        let shown = responder.visibleRect
        let middle = responder.convert(NSPoint(x: shown.midX, y: shown.midY), to: nil)
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
            if event.type == .leftMouseDown { self.noteClick(event) }
            if event.type == .rightMouseDown, self.showsToolbarPlaceMenu(for: event) { return nil }
            for tab in self.conversationTabs {
                guard let view = self.conversationAnchors[tab.id]?(), view.window === event.window,
                      view.bounds.contains(view.convert(event.locationInWindow, from: nil)) else { continue }
                self.focusConversation(tab)
                break
            }
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

/// Reports the view that marks a place (a pane, a tool, a toolbar button) so
/// a click can be matched to it. Only a view in a window reports itself: SwiftUI
/// may make views it then discards, and the one reported last must be a live one.
struct PaneAnchor: NSViewRepresentable {
    let register: (@escaping () -> NSView?) -> Void
    func makeNSView(context: Context) -> NSView {
        let view = PassthroughView()
        view.register = register
        return view
    }
    func updateNSView(_ view: NSView, context: Context) {}
    private final class PassthroughView: NSView {
        var register: ((@escaping () -> NSView?) -> Void)?
        override func hitTest(_ point: NSPoint) -> NSView? { nil }
        override func viewDidMoveToWindow() {
            super.viewDidMoveToWindow()
            if window != nil { register? { [weak self] in self } }
        }
    }
}
