import AppKit
import OSLog
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
    /// One independent edit, regardless of its configured surface.
    @Published var editing: MessageEditingSession?
    /// Preparation belongs to this captured submission, never a global blank tab.
    @Published var continuationPreparation: ContinuationPreparationSession? {
        didSet { store.objectWillChange.send() }
    }
    var messageEditingPanel: MessageEditingPanel?
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
    /// Blank is a navigation identity even before a runtime is available to
    /// create an unsent projection (and in synthetic previews).
    var isBlank: Bool {
        guard !store.isSideChat else { return false }
        let destination = store.reachingConversationID ?? store.selectedConversationID ?? store.loadedConversationID
        return store.isUnsentConversation || destination == nil || destination?.hasPrefix(ConversationStore.draftConversationPrefix) == true
    }

    override func close() {
        continuationPreparation?.close()
        continuationPreparation = nil
        finishMessageEditing(); store.close()
    }
    /// Reading starts when the tab is first shown, so restored background tabs cost nothing.
    private var pendingStart: (() -> Void)?

    init(id: UUID = UUID(), store: ConversationStore, start: @escaping () -> Void) {
        self.store = store
        store.unsentTabID = id
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
        if continuationPreparation != nil {
            return ComposerSubmission(phase: .sending, hasInput: hasInput, isComposing: model.isComposing,
                                      canSend: false, canQueue: false, canResume: false)
        }
        let configured = (next ?? store.nextTurn)?.chosenModelUnavailable != true && ComposerContext(tab: self, model: model).inputIssue == nil
        let phase: ComposerSubmission.Phase
        if store.isStopping { phase = .stopping }
        else if let pending = store.composerRequest { phase = pending }
        else if store.followUp.contextPreparation != nil || store.isGenerating || store.followUp.isCompacting { phase = .running }
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
        if !(store.followUp.contextPreparation != nil && store.canQueue),
           let target = next ?? store.nextTurn, prepareContinuationIfNeeded(using: composer, target: target) { return }
        // A queue request keeps its original input until the core admits it.
        // Edits made while waiting belong to the next input, even if identical.
        if store.canQueue {
            let submittedRevision = composer.revision
            _ = store.queue(text: text, attachments: attachments.map(\.path), next: next, onAccepted: { [composer] in
                guard composer.revision == submittedRevision else { return }
                composer.clear(submitted: submitted, attachments: attachments)
            })
            return
        }
        guard store.canSend else { return }
        // The message moves to the transcript at once; a turn that is not
        // sent hands its text and attachments back to the composer.
        let createsConversation = store.isUnsentConversation
        let requestedTarget = next ?? store.nextTurn
        let sourceID = store.loadedConversationID
        var restoredForPreparation = false
        let sent = store.send(text: text, attachments: attachments.map(\.path), next: next, input: composer, onFailure: { [composer, store] createdID in
            restoredForPreparation = clearedRevision == composer.revision
            composer.restoreFailed(submitted, attachments: attachments, revision: clearedRevision)
            store.device.composerInputs.save(composer.id, draft: composer.savedDraft)
            // The global new entry takes the consumed input back, unless its
            // fresh input has gained newer content meanwhile.
            if createsConversation { store.device.composerInputs.restoreConsumedNewConversationInput(composer, whileCreating: store, createdConversationID: createdID) }
        }, onDetailedFailure: { [weak self, composer, store] error in
            guard let self, restoredForPreparation, !createsConversation,
                  AppProblem.failure(error).code == "portable_summary_required",
                  store.loadedConversationID == sourceID, let requestedTarget, let current = store.nextTurn,
                  current.runtimeId == requestedTarget.runtimeId, current.model == requestedTarget.model,
                  current.effort == requestedTarget.effort, current.permissionMode == requestedTarget.permissionMode
            else { return }
            _ = self.prepareContinuationIfNeeded(using: composer, target: requestedTarget, afterProviderRejection: true)
        })
        guard sent else { return }
        composer.clear(submitted: submitted, attachments: attachments); clearedRevision = composer.revision; scrollRequest &+= 1
        if createsConversation { store.device.composerInputs.consumeNewConversationInput(composer) }
    }

    /// Insert using the running turn's native channel, retaining the draft
    /// until admission. Without native steering, use ordinary submission.
    func steer() { steer(using: composer) }
    func steer(using composer: ComposerModel, next: NextTurn? = nil) {
        guard store.isGenerating, store.followUp.canSteer else { return send(using: composer, next: next) }
        let submitted = composer.submission
        let attachments = composer.attachments
        let text = submitted.trimmingCharacters(in: .whitespacesAndNewlines)
        guard submission(using: composer, next: next).canSubmitDraft, store.canSteer else { return }
        let submittedRevision = composer.revision
        store.steer(text: text, attachments: attachments.map(\.path), next: next, onAccepted: { [composer] in
            guard composer.revision == submittedRevision else { return }
            composer.clear(submitted: submitted, attachments: attachments)
        })
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
        guard editing == nil else { messageEditingPanel?.showFromUserAction(); return }
        guard let editable = store.editability(of: messageID), let message = store.message(id: messageID) else { return }
        let request = MessageEditRequest(message: message, conversationID: store.loadedConversationID, inPlace: editable.inPlace, isLast: store.isLastUserMessage(messageID), revision: editable.revision)
        let session = MessageEditingSession(source: store, request: request)
        let identity = session.id
        session.onClose = { [weak self] in self?.finishMessageEditing(identity) }
        editing = session
        session.start()
    }

    func finishMessageEditing(_ identity: UUID? = nil) {
        guard let session = editing, identity == nil || session.id == identity else { return }
        editing = nil
        session.cancel()
        let panel = messageEditingPanel
        messageEditingPanel = nil
        panel?.closeEditor()
    }

    func changeMessageEditorPresentation(in window: ConversationWindowModel) {
        guard let editing else { return }
        objectWillChange.send()
        let destination: MessageEditPresentation = editing.presentation == .sheet ? .floating : .sheet
        if destination == .sheet {
            window.selectTab(id)
            let panel = messageEditingPanel
            messageEditingPanel = nil
            panel?.closeForTransfer()
        }
        editing.setPresentation(destination)
        // Sheet -> panel is ordered by the sheet's onDismiss callback, after
        // AppKit has finished tearing down its modal session.
    }

    func showFloatingMessageEditor(in window: ConversationWindowModel) {
        guard let editing, editing.presentation == .floating else { return }
        if messageEditingPanel == nil { messageEditingPanel = MessageEditingPanel(session: editing, window: window, source: self) }
        messageEditingPanel?.showFromUserAction()
    }

    /// The link a drag carries to other apps; an unsent conversation has none yet.
    override var link: URL? {
        guard let id = store.loadedConversationID, !store.isSideChat, !store.isUnsentConversation,
              let runtime = store.conversation(id: id)?.runtimeID ?? store.projectionRuntimeID else { return nil }
        return ConversationLink.url(runtimeID: runtime, conversationID: id)
    }

    override var title: String {
        if let context = store.sideChatContext { return store.selectedConversationTitle ?? "侧聊：" + context.title }
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
/// A group of the window's layout as its views observe it: the group's
/// tabs and the shown one. Only the window changes it, following its layout.
@MainActor final class Pane: ObservableObject, Identifiable {
    let id: UUID
    @Published fileprivate(set) var tabs: [EditorTab]
    @Published fileprivate(set) var selectedID: UUID

    fileprivate init(id: UUID, tabs: [EditorTab], selected: UUID) {
        self.id = id
        self.tabs = tabs
        selectedID = selected
    }

    var selected: EditorTab { tabs.first { $0.id == selectedID } ?? tabs[0] }

    fileprivate func show(_ tabs: [EditorTab], selected: UUID) {
        if !self.tabs.elementsEqual(tabs, by: { $0 === $1 }) { self.tabs = tabs }
        if selectedID != selected { selectedID = selected }
    }
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

/// What a window restores: its arrangement, each pane's tabs, each tab's
/// conversation, the focused pane and the tool windows.
struct SavedWindowLayout: Codable, Equatable {
    /// Public beta format baseline. Zero is reserved for the known unversioned pre-beta shape.
    var schemaVersion = 1
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
    /// Each tab's paired device (its peer id) when not this Mac; tabs absent here are local.
    var conversationDevices: [UUID: String]? = nil
    /// Kept input state of tabs on paired devices, by peer id; the local
    /// device keeps the legacy single fields below.
    var composerByDevice: [String: SavedComposerDraft]? = nil
    var composerWorkspaceByDevice: [String: SavedComposerWorkspace]? = nil
    var windowKind: ConversationWindowKind? = nil
    /// Absent only in legacy snapshots whose execution device is unknown.
    var windowDevice: ExecutionDevice? = nil

    private enum CodingKeys: String, CodingKey {
        case schemaVersion, layout, panes, conversations, focused, tools, pinned, composer, composerWorkspace, artifacts, unsentConversations, creationTargets, conversationDevices, composerByDevice, composerWorkspaceByDevice, windowKind, windowDevice
    }

    init(schemaVersion: Int = 1, layout: PaneLayout, panes: [UUID: SavedPane], conversations: [UUID: String], focused: UUID, tools: ToolWindowLayout, pinned: Bool, composer: SavedComposerDraft? = nil, composerWorkspace: SavedComposerWorkspace? = nil, artifacts: [UUID: ArtifactDocument]? = nil, unsentConversations: [String: Conversation]? = nil, creationTargets: [UUID: Conversation]? = nil, conversationDevices: [UUID: String]? = nil, composerByDevice: [String: SavedComposerDraft]? = nil, composerWorkspaceByDevice: [String: SavedComposerWorkspace]? = nil, windowKind: ConversationWindowKind? = nil, windowDevice: ExecutionDevice? = nil) {
        self.schemaVersion = schemaVersion
        self.layout = layout
        self.panes = panes
        self.conversations = conversations
        self.focused = focused
        self.tools = tools
        self.pinned = pinned
        self.composer = composer
        self.composerWorkspace = composerWorkspace
        self.artifacts = artifacts
        self.unsentConversations = unsentConversations
        self.creationTargets = creationTargets
        self.conversationDevices = conversationDevices
        self.composerByDevice = composerByDevice
        self.composerWorkspaceByDevice = composerWorkspaceByDevice
        self.windowKind = windowKind
        self.windowDevice = windowDevice
    }

    init(from decoder: Decoder) throws {
        let values = try decoder.container(keyedBy: CodingKeys.self)
        schemaVersion = values.contains(.schemaVersion) ? try values.decode(Int.self, forKey: .schemaVersion) : 0
        layout = try values.decode(PaneLayout.self, forKey: .layout)
        panes = try values.decode([UUID: SavedPane].self, forKey: .panes)
        conversations = try values.decode([UUID: String].self, forKey: .conversations)
        focused = try values.decode(UUID.self, forKey: .focused)
        tools = try values.decode(ToolWindowLayout.self, forKey: .tools)
        pinned = try values.decode(Bool.self, forKey: .pinned)
        composer = try values.decodeIfPresent(SavedComposerDraft.self, forKey: .composer)
        composerWorkspace = try values.decodeIfPresent(SavedComposerWorkspace.self, forKey: .composerWorkspace)
        artifacts = try values.decodeIfPresent([UUID: ArtifactDocument].self, forKey: .artifacts)
        unsentConversations = try values.decodeIfPresent([String: Conversation].self, forKey: .unsentConversations)
        creationTargets = try values.decodeIfPresent([UUID: Conversation].self, forKey: .creationTargets)
        conversationDevices = try values.decodeIfPresent([UUID: String].self, forKey: .conversationDevices)
        composerByDevice = try values.decodeIfPresent([String: SavedComposerDraft].self, forKey: .composerByDevice)
        composerWorkspaceByDevice = try values.decodeIfPresent([String: SavedComposerWorkspace].self, forKey: .composerWorkspaceByDevice)
        windowKind = try values.decodeIfPresent(ConversationWindowKind.self, forKey: .windowKind)
        windowDevice = try values.decodeIfPresent(ExecutionDevice.self, forKey: .windowDevice)
    }

    /// A pre-aggregation workspace window of one paired device becomes an
    /// aggregated window: its tabs' conversations belong to that device, and
    /// the window itself no longer names one. Idempotent.
    func migratingToAggregatedWorkspace() -> SavedWindowLayout {
        var layout = self
        if layout.windowKind != .single, case .remote(let peer)? = layout.windowDevice {
            var devices = layout.conversationDevices ?? [:]
            for tab in layout.panes.values.flatMap(\.tabs) where layout.artifacts?[tab] == nil && devices[tab] == nil {
                devices[tab] = peer
            }
            layout.conversationDevices = devices
            if let composer = layout.composer {
                var composers = layout.composerByDevice ?? [:]
                if composers[peer] == nil { composers[peer] = composer }
                layout.composerByDevice = composers
                layout.composer = nil
            }
            if let workspace = layout.composerWorkspace {
                var workspaces = layout.composerWorkspaceByDevice ?? [:]
                if workspaces[peer] == nil { workspaces[peer] = workspace }
                layout.composerWorkspaceByDevice = workspaces
                layout.composerWorkspace = nil
            }
            layout.windowDevice = .local
        }
        return layout
    }

    static func decodeKnown(_ data: Data) throws -> SavedWindowLayout {
        let raw = try JSONSerialization.jsonObject(with: data) as? [String: Any]
        let layout = try JSONDecoder().decode(SavedWindowLayout.self, from: data)
        guard layout.schemaVersion == 0 || layout.schemaVersion == 1 else {
            throw NSError(domain: "VelunePersistence", code: 20, userInfo: [NSLocalizedDescriptionKey: "窗口恢复点 schema 不受支持"])
        }
        let current = try JSONSerialization.jsonObject(with: JSONEncoder().encode(layout))
        guard let raw, raw.keys.allSatisfy({ CodingKeys(rawValue: $0) != nil }),
              hasOnlyEncodedKeys(raw, encoded: current) else {
            throw NSError(domain: "VelunePersistence", code: 21, userInfo: [NSLocalizedDescriptionKey: "窗口恢复点含未知字段"])
        }
        guard layout.isWhole else {
            throw NSError(domain: "VelunePersistence", code: 23, userInfo: [NSLocalizedDescriptionKey: "恢复点布局不完整"])
        }
        return layout
    }

    static func hasOnlyEncodedKeys(_ source: Any, encoded: Any) -> Bool {
        hasOnlyEncodedKeys(source, encoded: encoded, field: nil)
    }

    private static func hasOnlyEncodedKeys(_ source: Any, encoded: Any, field: String?) -> Bool {
        if let source = source as? [String: Any], let encoded = encoded as? [String: Any] {
            return source.keys.allSatisfy { key in
                guard let current = encoded[key] else {
                    guard source[key] is NSNull else { return false }
                    // Older migration tools encode absent values as null. Keep only
                    // declared optional turn-choice fields, including in remote drafts.
                    return field != "choice" || ["model", "effort", "permissionMode"].contains(key)
                }
                return hasOnlyEncodedKeys(source[key]!, encoded: current, field: key)
            }
        }
        if let source = source as? [Any], let encoded = encoded as? [Any] {
            if ["panes", "artifacts", "creationTargets", "conversationDevices"].contains(field ?? "") {
                guard let sourceMap = uuidDictionaryEntries(source),
                      let encodedMap = uuidDictionaryEntries(encoded),
                      Set(sourceMap.keys) == Set(encodedMap.keys) else { return false }
                return sourceMap.allSatisfy { entry in
                    let (key, value) = entry
                    guard let other = encodedMap[key] else { return false }
                    return hasOnlyEncodedKeys(value, encoded: other, field: nil)
                }
            }
            return source.count == encoded.count && zip(source, encoded).allSatisfy { hasOnlyEncodedKeys($0.0, encoded: $0.1) }
        }
        return true
    }

    private static func uuidDictionaryEntries(_ values: [Any]) -> [String: Any]? {
        guard values.count.isMultiple(of: 2) else { return nil }
        var result: [String: Any] = [:]
        for pair in stride(from: 0, to: values.count, by: 2) {
            guard let key = values[pair] as? String, UUID(uuidString: key) != nil,
                  result[key] == nil else { return nil }
            result[key] = values[pair + 1]
        }
        return result
    }

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
    /// A conversation and the device it belongs to, for renames and deletes asked outside its tab.
    struct ConversationActionTarget: Equatable, Identifiable {
        var conversation: Conversation
        var device: DeviceStore
        var id: String { conversation.id }
        static func == (lhs: Self, rhs: Self) -> Bool { lhs.conversation == rhs.conversation && lhs.device === rhs.device }
    }
    @Published var renameTarget: ConversationActionTarget?
    @Published var hapiShareTarget: HapiShareTarget?
    @Published var deleteTargets: [ConversationActionTarget] = []
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
                for failure in value.failures { list.recordProblem(failure.detail, source: "打开会话：" + failure.runtimeId) }
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
    /// The paired device the window shows; non-nil only on a window opened
    /// for one conversation of a paired device. Workspace windows aggregate
    /// every device and keep this nil.
    let device: String?
    /// The paired-device hub; nil in previews.
    let hub: DeviceHub?
    /// This window's aggregated conversation list: this Mac and every paired
    /// device it controls, in one sidebar.
    private(set) lazy var aggregated = AggregatedListModel(local: application, hub: hub)
    /// The list's workspace: list, search and selection; it opens no conversation.
    let list: ConversationStore
    /// The grid, groups and focus. Every change goes through `changeLayout`.
    @Published private(set) var workspace: WorkspaceLayout
    /// The editors the layout's tabs name.
    private var editors: [UUID: EditorTab]
    /// Each group as its views observe it, following `workspace`.
    @Published private(set) var panes: [UUID: Pane]
    var layout: PaneLayout { workspace.grid }
    var focusedPaneID: UUID { workspace.focused }
    /// Each tab's conversation as last selected, for the layout the window restores.
    @Published private(set) var tabConversations: [UUID: String] = [:]
    /// Each tab's paired device (peer id) as last shown; tabs absent here are local.
    @Published private(set) var tabDevices: [UUID: String] = [:]
    private func noteTabDevice(_ tab: ConversationTab) {
        if let peer = tab.store.device.peerID { tabDevices[tab.id] = peer }
        else { tabDevices[tab.id] = nil }
    }
    /// Closed conversation tabs' targets (nil for an unsent new
    /// conversation) and the device each belongs to, latest last, for
    /// 重新打开关闭的标签页.
    struct ClosedTarget: Equatable {
        var device: String?
        var conversation: String?
        var tabID: UUID
        var paneID: UUID
        var index: Int
        var layout: PaneLayout
        var creation: Conversation?
        var artifact: ArtifactDocument?
    }
    @Published private(set) var recentlyClosed: [ClosedTarget] = []
    /// Bumped when a pane's tabs change, for the layout the window restores.
    @Published private(set) var tabsRevision = 0
    private var subscriptions: [UUID: Set<AnyCancellable>] = [:]
    private var preservingComposerLayout = false
    private var started = false
    private var closed = false
    private var workspaceRestored = false
    let restorationLifecycleID = UUID()
    /// SwiftUI can retain a closed scene while other windows keep changing.
    /// Its last recovery point must stop following those later publications.
    var acceptsWorkspacePersistence: Bool { started && workspaceRestored && !closed }
    func noteWindowClosed() {
        closed = true
        for tab in conversationTabs { tab.finishMessageEditing() }
        refreshConversationObservations()
    }
    private var persistenceID: String?
    private var workspacePersistence: AnyCancellable?
    private var conversationObservation: AnyCancellable?
    private var restoredCreationTargets: [UUID: Conversation] = [:]
    func persistWorkspace() {
        guard acceptsWorkspacePersistence, let persistenceID else { return }
        let layout = savedLayout
        application.windowWorkspaceJournal.save(layout, id: persistenceID)
    }
    /// Every device's draft directory drives the window the same way:
    /// changes persist the workspace, cursor moves refresh the UI, and a
    /// native creation re-points the unsent tabs that hold its input.
    private func attachDraftHandlers(_ drafts: ComposerDrafts) {
        drafts.changes.sink { [weak self] _ in self?.persistWorkspace() }.store(in: &draftHandlers)
        drafts.cursorChanges.sink { [weak self] _ in self?.objectWillChange.send() }.store(in: &draftHandlers)
        drafts.creations.sink { [weak self] creation in
            guard let self, let input = drafts.existing(creation.inputId) else { return }
            let tabs = self.conversationTabs.filter { tab in
                tab.store.isUnsentConversation && drafts.owns(input, for: tab)
            }
            guard !tabs.isEmpty else { return }
            for tab in tabs {
                let previous = tab.store.selectedConversationID ?? tab.store.loadedConversationID ?? ConversationStore.draftConversationPrefix + "new-conversation"
                drafts.reassociate(input, from: previous, to: creation.conversationId, targetTab: tab, publish: false)
            }
            tabs.forEach { $0.store.followComposerCreation(creation) }
            self.objectWillChange.send()
        }.store(in: &draftHandlers)
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
    private var draftHandlers: [AnyCancellable] = []
    private(set) lazy var composerDrafts = ComposerDrafts(device: application)
    private var remoteDrafts: [String: ComposerDrafts] = [:]
    /// The kept-input directory of the device a tab's conversations live on.
    /// Each device owns its own composer input directory; a window with tabs
    /// on several devices keeps one directory client per device.
    func drafts(for store: DeviceStore) -> ComposerDrafts {
        if store === application { return composerDrafts }
        guard let peer = hub?.peerID(of: store) else { return composerDrafts }
        if let drafts = remoteDrafts[peer] { return drafts }
        let drafts = ComposerDrafts(device: store)
        remoteDrafts[peer] = drafts
        attachDraftHandlers(drafts)
        return drafts
    }
    /// A list-type store over any device this window shows.
    func listStore(for store: DeviceStore) -> ConversationStore {
        if store === application { return list }
        if let device = aggregated.devices.first(where: { $0.store === store }) { return aggregated.listStore(for: device) }
        return list
    }
    @Published var composerMinimumHeight = ComposerTextView.minimumHeight + 54
    private var restoredComposerDraft: ComposerModel?
    var sharedComposer: ComposerModel { composerDrafts.shared }
    /// The shared input of the focused tab's device.
    func sharedComposer(for tab: ConversationTab) -> ComposerModel { drafts(for: tab.store.device).shared }
    /// Temporary side-chat inputs stay with their tab and never enter the registry.
    func composerOwner(for tab: ConversationTab, tools placement: ToolWindowLayout? = nil) -> ComposerDrafts.Owner? {
        if tab.store.isSideChat || tab.store.loadedParentConversationID != nil { return nil }
        let placement = placement ?? tools
        if tab === sheetTab || tab === composerSheetTab { return .conversation(tab.store.selectedConversationID ?? tab.store.loadedConversationID ?? "tab:\(tab.id)") }
        switch placement[.composer].place {
        case .floating, .window, .docked: return .shared
        default: return .conversation(tab.store.selectedConversationID ?? tab.store.loadedConversationID ?? "tab:\(tab.id)")
        }
    }
    /// A shared panel is only a presentation host. Looking up another visible
    /// tab's input must never return the currently focused tab's draft.
    private func composerInputOwner(for tab: ConversationTab) -> ComposerDrafts.Owner? {
        guard composerOwner(for: tab) != nil else { return nil }
        return .conversation(tab.store.selectedConversationID ?? tab.store.loadedConversationID ?? "tab:\(tab.id)")
    }
    func composer(for tab: ConversationTab) -> ComposerModel {
        guard let owner = composerInputOwner(for: tab) else { return tab.composer }
        let model = drafts(for: tab.store.device).model(for: owner, tab: tab)
        tab.composer = model
        return model
    }
    func existingComposer(for tab: ConversationTab) -> ComposerModel? {
        guard let owner = composerInputOwner(for: tab) else { return tab.composer }
        return drafts(for: tab.store.device).existingModel(for: owner)
    }
    func selectComposerDraft(_ model: ComposerModel, for tab: ConversationTab) {
        guard let owner = composerInputOwner(for: tab) else { return }
        drafts(for: tab.store.device).select(model, replacing: composer(for: tab), owner: owner, tab: tab)
        objectWillChange.send()
        composer(for: tab).focusWhenShown()
    }
    func newComposerDraft(for tab: ConversationTab) {
        guard let owner = composerInputOwner(for: tab) else { return }
        drafts(for: tab.store.device).newDraft(replacing: composer(for: tab), owner: owner, tab: tab)
        objectWillChange.send()
        composer(for: tab).focusWhenShown()
    }
    private func navigateComposer(to id: String, tab: ConversationTab, checkpointPrevious: Bool = true) {
        guard composerOwner(for: tab) != nil else { return }
        drafts(for: tab.store.device).navigate(to: id, tab: tab, checkpointPrevious: checkpointPrevious)
    }
    /// A new conversation opens in a new tab after the focused pane's shown
    /// one; a focused tab already holding an unsent conversation is restored
    /// in place instead, not stacked under another empty tab. Every blank tab
    /// restores its own content and execution choices when selected.
    @discardableResult
    func createConversation() -> ConversationTab? {
        if let shown = focusedEditor as? ConversationTab, shown.store.device === application, shown.store.isUnsentConversation {
            return shown.store.createConversation() ? shown : nil
        }
        return openNewConversationTab()
    }
    /// Group creation is an explicit target choice on the device the group
    /// belongs to, including for a shared composer whose unfinished input
    /// otherwise keeps its own intent. The conversation opens as
    /// `createConversation()` does: in a new tab.
    func createConversation(in group: ConversationGroupKey, device target: DeviceStore? = nil) {
        let device = target ?? application
        let project: ConversationProject? = if case .project(let path?) = group {
            device.conversationProjects.first { $0.path == path }
        } else { nil }
        let runtimeID: String? = if case .runtime(let id) = group { id } else { project?.nativeRuntimeId }
        let targetRuntimeID = runtimeID ?? (focused.store.device === device ? composerContext(for: focused).nextTurnRuntimeID : nil)
        let selectedProject: NewConversationProject? = project.flatMap { project in
            guard let runtimeID = project.nativeRuntimeId,
                  let choice = device.workingDirectoryProjects[runtimeID]?.first(where: { $0.project == project.path && $0.canSelectProject }) else { return nil }
            return NewConversationProject(key: choice.project, name: choice.name, runtimeID: runtimeID, directories: choice.directories)
        }
        // A native project's group means that whole project; do not quietly
        // reduce an unavailable project to its first directory.
        if project?.nativeRuntimeId != nil && selectedProject == nil {
            list.recordProblem("此项目当前没有可用的完整工作区，请刷新项目或选择工作目录", source: "新建会话")
            return
        }
        let tab: ConversationTab?
        if let shown = focusedEditor as? ConversationTab, shown.store.device === device, shown.store.isUnsentConversation {
            tab = shown.store.createConversation(runtimeID: targetRuntimeID, cwd: project?.primaryFolder, project: selectedProject) ? shown : nil
        } else {
            tab = openNewConversationTab(device: device, runtimeID: targetRuntimeID, cwd: project?.primaryFolder, project: selectedProject)
        }
        guard let tab else { return }
        if let runtimeID { composerContext(for: tab).selectNextTurnRuntime(runtimeID) }
    }
    /// The draft is created before the tab starts: an unavailable target
    /// leaves the window unchanged. Its navigation event fired before the tab
    /// was followed, so the shared input is navigated to as selecting the tab
    /// would. `start` only reads; the snapshot already exists (the preview
    /// branch would otherwise recreate it with the default target).
    func openNewConversationTab(device: DeviceStore? = nil, runtimeID: String? = nil, cwd: String? = nil, project: NewConversationProject? = nil) -> ConversationTab? {
        let device = device ?? application
        let store = ConversationStore(device: device)
        guard store.createConversation(runtimeID: runtimeID, cwd: cwd, project: project) else { return nil }
        let tab = ConversationTab(store: store, start: {
            store.start(loadFirstConversation: false, newConversation: false)
        })
        insertTab(tab, at: afterShownTab)
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
                let previous = drafts(for: focused.store.device).model(for: previousOwner, tab: focused)
                drafts(for: focused.store.device).move(previous, to: owner, tab: focused)
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
    @Published var backgroundTasksPopover = false
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

    /// A transcript is polled only while its selected pane or conversation
    /// sheet is actually on screen. Split panes are independently visible;
    /// an unselected tab in either pane is not.
    static func isConversationStoreVisible(_ store: ConversationStore) -> Bool {
        // Programmatic checks use synthetic homes and often create window
        // models without a native NSWindow; keep their data-driven polling
        // paths active while ordinary app windows remain visibility-gated.
        if MacExecutionContext.admitted.isDevelopment,
           CommandLine.arguments.contains(where: { $0.hasPrefix("--check-") }) {
            return true
        }
        return isConversationStoreSelectedAndVisible(store)
    }

    /// Pure app-window visibility predicate, kept separate from the explicit
    /// development-check polling override above.
    static func isConversationStoreSelectedAndVisible(_ store: ConversationStore) -> Bool {
        guard !NSApp.isHidden else { return false }
        return openWindows.contains { model in
            if model.detached.hasVisibleComposerWindow(for: store) { return true }
            guard let native = model.window, native.isVisible, native.isOnActiveSpace, !native.isMiniaturized else { return false }
            return isConversationStoreSelected(store, in: model)
        }
    }

    /// Model-only selection predicate for synthetic window checks; it does
    /// not require the model to own a native NSWindow.
    static func isConversationStoreSelected(_ store: ConversationStore, in model: ConversationWindowModel) -> Bool {
        guard !model.closed else { return false }
        let paneIsSelected = model.orderedPanes.contains { pane in
            (pane.selected as? ConversationTab)?.store === store
        }
        let sheetIsSelected = model.sheetTab?.store === store || model.composerSheetTab?.store === store
        return paneIsSelected || sheetIsSelected
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
        return all.lazy.compactMap(\.model).first { $0.pane(holding: tab) != nil || $0.artifactSheet?.id == tab || $0.sheetTab?.id == tab || $0.composerSheetTab?.id == tab }
    }

    init(application: DeviceStore, kind: ConversationWindowKind, device: String? = nil, hub: DeviceHub? = nil) {
        self.application = application
        self.kind = kind
        self.device = device
        self.hub = hub
        list = ConversationStore(device: application)
        let store = ConversationStore(device: application)
        let first = ConversationTab(store: store, start: {
            store.start(loadFirstConversation: kind == .workspace, newConversation: kind == .single)
        })
        workspace = WorkspaceLayout(tab: first.id)
        editors = [first.id: first]
        panes = [:]
        syncPanes()
        follow(first)
        Self.all.append(WeakWindow(model: self))
        conversationObservation = objectWillChange.sink { [weak self] in
            // Published properties announce before committing selection/layout.
            DispatchQueue.main.async { self?.refreshConversationObservations() }
        }
        attachDraftHandlers(composerDrafts)
        // Window recovery belongs to the model, including changes made before
        // SwiftUI observes a scene. Read after @Published has committed its value.
        workspacePersistence = Publishers.MergeMany([
            $workspace.dropFirst().map { _ in () }.eraseToAnyPublisher(),
            $tabConversations.dropFirst().map { _ in () }.eraseToAnyPublisher(),
            $tabsRevision.dropFirst().map { _ in () }.eraseToAnyPublisher(),
            $tools.dropFirst().map { _ in () }.eraseToAnyPublisher(),
            $pinned.dropFirst().map { _ in () }.eraseToAnyPublisher(),
        ]).debounce(for: .milliseconds(150), scheduler: RunLoop.main)
            .sink { [weak self] in self?.persistWorkspace() }
    }

    private func refreshConversationObservations() {
        var seen: Set<ObjectIdentifier> = []
        for device in [application] + conversationTabs.map({ $0.store.device }) {
            if seen.insert(ObjectIdentifier(device)).inserted { device.refreshConversationObservation() }
        }
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
        if composerOwner(for: tab) != nil { drafts(for: tab.store.device).focus(tab) }
        tab.store.noteFocused()
        tab.store.noteVisible()
    }
    var isSplit: Bool { workspace.isSplit }
    var orderedPanes: [Pane] { workspace.groupIDs.compactMap { panes[$0] } }
    var allTabs: [EditorTab] { workspace.tabIDs.compactMap { editors[$0] } }
    var tabCount: Int { workspace.tabCount }

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
        let remoteComposers = Dictionary(uniqueKeysWithValues: remoteDrafts
            .filter { !$0.value.saved.drafts.isEmpty || !$0.value.saved.retained.isEmpty || $0.value.shared.hasUnfinishedInput }
            .map { ($0.key, $0.value) })
        return SavedWindowLayout(layout: keptLayout, panes: kept,
                                 conversations: tabConversations.filter { ids.contains($0.key) },
                                 focused: kept[focusedPaneID] == nil ? keptLayout.paneIDs[0] : focusedPaneID,
                                 tools: tools, pinned: pinned, composer: sharedComposer.savedDraft,
                                 composerWorkspace: composerDrafts.saved,
                                 artifacts: Dictionary(uniqueKeysWithValues: allTabs.compactMap { editor in (editor as? ArtifactTab).map { ($0.id, $0.document) } }),
                                 creationTargets: restoredCreationTargets.filter { ids.contains($0.key) }.merging(Dictionary(uniqueKeysWithValues: conversationTabs.compactMap { tab in tab.store.unsentConversation.map { (tab.id, $0) } }), uniquingKeysWith: { _, current in current }),
                                 conversationDevices: tabDevices.filter { ids.contains($0.key) },
                                 composerByDevice: remoteComposers.isEmpty ? nil : remoteComposers.mapValues { $0.shared.savedDraft },
                                 composerWorkspaceByDevice: remoteComposers.isEmpty ? nil : remoteComposers.mapValues { $0.saved },
                                 windowKind: kind, windowDevice: SavedWindowLayout.ExecutionDevice(device))
    }

    /// Restore a saved arrangement, take a handed-over tab, or open `seed`'s
    /// conversation, then start reading. A lone new tab opens the list's
    /// first conversation.
    func start(restoring saved: SavedWindowLayout?, seed: WindowSeed?, persistenceID: String? = nil) {
        closed = false
        guard !started else { return }
        started = true
        // Restoring the composer synchronously publishes draft changes. They
        // must not save the initial blank panes over the snapshot being read.
        defer {
            workspaceRestored = true
            persistWorkspace()
        }
        // The archived WindowGroup value survives native window renumbering.
        // SceneStorage may be absent on restoration, so its fresh UUID cannot
        // be the only address of a seeded window's recovery point.
        let recoveryID = seed?.id.uuidString ?? persistenceID
        self.persistenceID = recoveryID
        let aggregatingDevices = kind == .workspace && device == nil && hub != nil
        let sceneSaved: SavedWindowLayout? = saved.flatMap {
            let candidate = aggregatingDevices ? $0.migratingToAggregatedWorkspace() : $0
            if let tab = seed?.tab, Self.handoff[tab] == nil,
               !candidate.panes.values.contains(where: { $0.tabs.contains(tab) }) { return nil }
            return candidate.isWhole && (candidate.windowKind == nil || candidate.windowKind == kind)
                && (candidate.windowDevice == nil || candidate.windowDevice == SavedWindowLayout.ExecutionDevice(device)) ? candidate : nil
        }
        let journalSaved = recoveryID.flatMap { application.windowWorkspaceJournal.restore(id: $0, recoverLatest: seed == nil && sceneSaved == nil && kind == .workspace, kind: kind, device: device, aggregatingDevices: aggregatingDevices, legacyTabID: seed?.tab) }
        let saved = journalSaved ?? sceneSaved
        let recoveryBranch = seed?.tab.flatMap { Self.handoff[$0] } != nil ? "handoff" : saved != nil ? "snapshot" : seed?.conversationID != nil ? "conversation" : seed?.tab != nil ? "missingHandoff" : "default"
        let recoveryKind = kind.sceneID
        let modelID = restorationLifecycleID.uuidString
        let journalTabs = journalSaved?.panes.values.flatMap(\.tabs).map(\.uuidString).sorted().joined(separator: ",") ?? "none"
        windowRecoveryLogger.debug("phase=modelStart model=\(modelID, privacy: .public) kind=\(recoveryKind, privacy: .public) recovery=\(recoveryID ?? "none", privacy: .public) branch=\(recoveryBranch, privacy: .public) journal=\(journalSaved != nil) journalTabs=\(journalTabs, privacy: .public) scene=\(sceneSaved != nil)")
        list.start(loadFirstConversation: false)
        // A layout the system restores below replaces this one.
        switch kind {
        case .workspace: tools = application.rememberedToolLayout.layout
        case .single:
            tools = ToolWindowLayout.forSingleConversation()
            tools.toggleEditorOnly()
        }
        if let id = seed?.tab, let tab = Self.handoff.removeValue(forKey: id) {
            if let tab = tab as? ConversationTab, !tab.store.isSideChat, let conversationID = tab.store.loadedConversationID { drafts(for: tab.store.device).adopt(tab.composer, conversationID: conversationID, tab: tab) }
            replaceEditors([tab], layout: WorkspaceLayout(tab: tab.id))
            tab.activate()
            noteSelected(tab)
        } else if let saved, saved.isWhole {
            if let workspace = saved.composerWorkspace { composerDrafts.restore(workspace) }
            else { composerDrafts.restoreLegacy(saved.composer) }
            for (peer, draft) in saved.composerByDevice ?? [:] {
                guard saved.composerWorkspaceByDevice?[peer] == nil, let store = hub?.store(for: peer) else { continue }
                drafts(for: store).restoreLegacy(draft)
            }
            for (peer, workspace) in saved.composerWorkspaceByDevice ?? [:] {
                guard let store = hub?.store(for: peer) else { continue }
                drafts(for: store).restore(workspace)
            }
            if saved.composerWorkspace == nil, sharedComposer.hasUnfinishedInput { restoredComposerDraft = sharedComposer }
            let lone = saved.layout.paneIDs.count == 1 && saved.panes.values.first?.tabs.count == 1
            let restored: [EditorTab] = saved.layout.paneIDs.flatMap { paneID in
                saved.panes[paneID]!.tabs.map { tabID -> EditorTab in
                    if let document = saved.artifacts?[tabID] { return ArtifactTab(id: tabID, document: document) }
                    let deviceStore = saved.conversationDevices?[tabID].flatMap { hub?.store(for: $0) } ?? application
                    let store = ConversationStore(device: deviceStore)
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
            }
            let groups = saved.panes.compactMapValues { TabGroup(tabs: $0.tabs, selected: $0.selected) }
            replaceEditors(restored, layout: WorkspaceLayout(grid: saved.layout, groups: groups, focused: saved.focused) ?? WorkspaceLayout(tab: restored[0].id))
            tabConversations = saved.conversations
            tabDevices = saved.conversationDevices ?? [:]
            restoredCreationTargets = saved.creationTargets ?? [:]
            tools = saved.tools.normalized()
            pinned = saved.pinned
            for group in workspace.groupIDs { editors[workspace.groups[group]!.selected]?.activate() }
        } else if let id = seed?.conversationID {
            // A seed may name the paired device the conversation lives on;
            // a workspace window then binds the tab to that device's store.
            let deviceStore = seed?.device.flatMap { hub?.store(for: $0) } ?? application
            let tab = makeTab(device: deviceStore, opening: id)
            replaceEditors([tab], layout: WorkspaceLayout(tab: tab.id))
            focused.activate()
        } else if seed?.tab != nil {
            // An in-process handoff is not a cold-start recovery source.
            // A missing snapshot must not impersonate the list's first session.
            let tab = makeTab(opening: nil)
            replaceEditors([tab], layout: WorkspaceLayout(tab: tab.id))
            focused.activate()
            application.recordProblem("此窗口的内容未能恢复，已打开空白标签页。", source: "窗口恢复")
        } else {
            focused.activate()
        }
        installMouseMonitor()
        detached.sync()
        if let seed, WindowPresentation.notificationTargets.remove(seed.id) != nil, let id = seed.conversationID {
            // A restored layout retains the user's editors. The notification
            // is a new navigation intent applied after that restoration.
            let target = seed.device.flatMap { hub?.store(for: $0) } ?? application
            openInNewTab(id, device: target)
            focusNotificationTarget(id)
        }
    }

    /// The editors before reading starts: the first group's untouched tab gives way.
    private func replaceEditors(_ replacement: [EditorTab], layout: WorkspaceLayout) {
        for tab in allTabs { tab.close() }
        subscriptions = [:]
        editors = Dictionary(uniqueKeysWithValues: replacement.map { ($0.id, $0) })
        workspace = layout
        editors = editors.filter { layout.contains(tab: $0.key) }
        syncPanes()
        replacement.forEach(follow)
        tabsRevision &+= 1
    }

    // MARK: Layout

    /// Every change to the grid, groups and focus; the groups' views follow,
    /// editors the layout no longer names are let go, and new ones followed.
    private func changeLayout(_ change: (inout WorkspaceLayout) -> Void) {
        var next = workspace
        change(&next)
        guard next != workspace else { return }
        let tabsChanged = next.groups != workspace.groups
        workspace = next
        let named = Set(next.tabIDs)
        editors = editors.filter { named.contains($0.key) }
        syncPanes()
        for id in next.tabIDs where subscriptions[id] == nil { editors[id].map(follow) }
        if tabsChanged { tabsRevision &+= 1 }
    }

    private func syncPanes() {
        var next: [UUID: Pane] = [:]
        for id in workspace.groupIDs {
            let group = workspace.groups[id]!
            let tabs = group.tabs.compactMap { editors[$0] }
            guard !tabs.isEmpty else { continue }
            let selected = editors[group.selected] == nil ? tabs[0].id : group.selected
            if let pane = panes[id] { pane.show(tabs, selected: selected); next[id] = pane }
            else { next[id] = Pane(id: id, tabs: tabs, selected: selected) }
        }
        for id in panes.keys where next[id] == nil { anchors[id] = nil }
        if Set(next.keys) != Set(panes.keys) { panes = next }
    }

    /// Typing that went to the tab put behind goes to the one shown, as
    /// clicking a tab does; the shown tab starts reading.
    private func noteShown(_ tab: EditorTab, replacing previous: EditorTab?) {
        if let previous = previous as? ConversationTab, previous !== tab, let next = tab as? ConversationTab, previous.composer.isFocused {
            next.composer.focusWhenShown()
        }
        tab.activate()
    }

    /// A layout change that shows `tab` in its group, with what showing it does.
    private func show(_ tab: UUID, by change: (inout WorkspaceLayout) -> Void) {
        let before = workspace.groups.mapValues(\.selected)
        changeLayout(change)
        guard let group = workspace.group(holding: tab), let editor = editors[tab] else { return }
        noteShown(editor, replacing: before[group].flatMap { editors[$0] })
    }

    /// Show `tab` in its group and focus that group.
    private func show(_ tab: UUID) { show(tab) { $0.select(tab) } }

    /// `tab` in `group` (the focused one when nil) at `index`, else at the
    /// end; shown, and its group focused.
    func insertTab(_ tab: EditorTab, in group: UUID? = nil, at index: Int? = nil) {
        guard !workspace.contains(tab: tab.id) else { return }
        editors[tab.id] = tab
        show(tab.id) { $0.insert(tab.id, in: group, at: index) }
    }

    /// Where a new tab goes: just after the focused group's shown tab.
    private var afterShownTab: Int? {
        let group = workspace.focusedGroup
        return group.tabs.firstIndex(of: group.selected).map { $0 + 1 }
    }

    /// Take `tab` from a group that keeps other tabs; the tab shown in its
    /// place starts as a clicked one would.
    private func take(_ tab: UUID) {
        guard let group = workspace.group(holding: tab), workspace.groups[group]!.tabs.count > 1 else { return }
        let taken = editors[tab], shown = workspace.groups[group]!.selected
        changeLayout { $0.remove(tab) }
        guard let now = workspace.groups[group]?.selected, now != shown, let editor = editors[now] else { return }
        noteShown(editor, replacing: taken)
    }

    /// Notes the newly focused group's shown tab when focus moved from `before`.
    private func noteFocus(from before: UUID) {
        if focusedPaneID != before { noteSelected(focusedEditor) }
    }

    // MARK: Opening

    /// Open a conversation in the focused tab; one a tab of this window
    /// already shows is selected there instead. The device the conversation
    /// lives on comes from the aggregated list; unknown conversations open
    /// on this Mac and resolve there.
    func open(_ conversationID: String, device target: DeviceStore? = nil) {
        let target = target ?? aggregated.device(of: conversationID)?.store ?? application
        guard hasConversationFocus else { return openInNewTab(conversationID, device: target) }
        if select(showing: conversationID, device: target) { return }
        if focused.store.isSideChat { openInNewTab(conversationID, device: target) }
        else if focused.store.device === target { focused.store.openLinkedConversation(id: conversationID) }
        else { openInNewTab(conversationID, device: target) }
    }

    /// Notifications preserve the current editor. Reuse a conversation tab
    /// in a window that can show the device, otherwise add one in its last
    /// focused window.
    static func openNotificationConversation(_ conversationID: String, application: DeviceStore) -> ConversationWindowModel? {
        let windows = openWindows.filter { $0.canShow(application) }
        let existing = windows.first { $0.tab(showing: conversationID, device: application) != nil }
        guard let target = existing ?? windows.first(where: { $0.focused.store === application.focusedConversation }) ?? windows.first else { return nil }
        target.openInNewTab(conversationID, device: application)
        return target
    }

    /// Whether this window can present the given device's conversations:
    /// its own, or any paired device the aggregated list reaches.
    func canShow(_ store: DeviceStore) -> Bool {
        if application === store { return true }
        return kind == .workspace && (hub?.peerID(of: store) != nil || aggregated.devices.contains { $0.store === store })
    }

    /// A workspace window that can present any device, for the devices list to focus.
    static func openWorkspaceWindow() -> ConversationWindowModel? {
        openWindows.first { $0.kind == .workspace && !$0.closed }
    }

    /// Show one device's conversations in the aggregated list.
    func showDevice(_ peerID: String) {
        aggregated.setFilter(.peer(peerID))
    }

    /// The waiting conversation's turn wants the user: focus its answer
    /// field; otherwise the composer takes the keyboard.
    func focusNotificationTarget(_ conversationID: String) {
        if focused.store.device.waitingConversationIDs.contains(conversationID) { focused.focusInteraction() }
        else { composer(for: focused).focusWhenShown() }
    }

    /// Ordinary conversation commands use the chosen editor presentation.
    func openSideChat(from source: ConversationTab? = nil) {
        let source = source ?? commandTab
        guard let target = composerContext(for: source).sideChatTarget else { return }
        openSideChat(target, presentation: sideChatPresentation, parent: source.id)
    }

    /// A list action opens over the current editor, without selecting its target.
    func openSideChat(on conversationID: String, device target: DeviceStore? = nil) {
        endSideChatOpening()
        if let source = conversationTabs.first(where: { $0.store.loadedConversationID == conversationID }),
           let target = source.store.sideChatTarget {
            return openSideChat(target, presentation: .sheet, parent: nil)
        }
        let request = BackgroundSideChat(tab: makeTab(device: target, opening: conversationID))
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
            if tabCount == 1 { insertTab(makeTab(opening: nil), in: pane.id) }
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
    func openInNewTab(_ conversationID: String? = nil, device target: DeviceStore? = nil) {
        if let conversationID, select(showing: conversationID, device: target) { return }
        insertTab(makeTab(device: target, opening: conversationID), at: afterShownTab)
        focused.store.noteFocused()
    }

    /// Open a conversation linked from `tab`'s (the one that started it, a
    /// sub-agent's, a collaborator's) where `opening` says, else where the
    /// setting does. From the sheet, a sheet is the sheet itself, and a tab
    /// or window takes its place.
    func openLinked(_ conversationID: String, from tab: ConversationTab, opening: LinkedConversationOpening? = nil) {
        if tab.store.isSideChat { openInNewTab(conversationID, device: aggregated.device(of: conversationID)?.store); return }
        let opening = opening ?? application.linkedConversationOpening
        // Native child and parent links are on the device that owns their history.
        let nativeRelation = tab.store.loadedParentConversationID == conversationID
            || tab.store.subAgents.contains { $0.id == conversationID }
        let target = nativeRelation ? tab.store.device : nil
        if tab === sheetTab {
            if opening == .currentTab || opening == .sheet { return tab.store.openLinkedConversation(id: conversationID) }
            closeSheet()
        }
        switch opening {
        case .currentTab: tab.store.openLinkedConversation(id: conversationID)
        case .newTab: openInNewTab(conversationID, device: target)
        case .sheet: openInSheet(conversationID, device: tab.store.device)
        case .newWindow: openInNewWindow(conversationID, device: target)
        }
    }

    /// Open a conversation in a sheet over the window; one a tab of this
    /// window shows is selected there instead.
    func openInSheet(_ conversationID: String, device target: DeviceStore? = nil) {
        if select(showing: conversationID, device: target) { return }
        presentInSheet(makeTab(device: target, opening: conversationID))
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
    func openComposerSheet(_ conversationID: String, device target: DeviceStore? = nil) {
        closeComposerSheet()
        let tab = makeTab(device: target, opening: conversationID)
        tab.activate()
        tab.composer.focusWhenShown()
        composerSheetTab = tab
    }

    func closeComposerSheet() {
        guard let tab = composerSheetTab else { return }
        composerSheetTab = nil
        tab.close()
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
    func resumeFromList(_ conversationID: String, device target: DeviceStore? = nil) {
        let device = target ?? application
        if let shown = Self.openWindows.lazy.filter({ $0.canShow(device) }).compactMap({ $0.tab(showing: conversationID, device: device)?.1 }).first {
            guard shown.store.canResume else { return list.recordProblem("此会话当前不能继续", source: "继续会话") }
            return shown.store.continueTurn()
        }
        let tab = makeTab(device: device, opening: conversationID)
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
        insertTab(tab)
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
        insertTab(tab, at: afterShownTab)
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
    func openInNewTab(adopting result: SnapshotResult, device: DeviceStore) {
        guard let id = result.snapshot?.conversation.id else { return }
        if select(showing: id, device: device) { return }
        let store = ConversationStore(device: device)
        insertTab(ConversationTab(store: store, start: {
            store.start(adopting: result)
        }), at: afterShownTab)
        navigateComposer(to: id, tab: focused)
        focused.store.noteFocused()
    }

    /// Divide the focused pane; the new pane takes focus and opens
    /// `conversationID`, or starts a new conversation.
    func split(_ axis: PaneSplit.Axis, opening conversationID: String? = nil, device target: DeviceStore? = nil) {
        if let conversationID, select(showing: conversationID, device: target) { return }
        addPane(with: makeTab(device: target, opening: conversationID), beside: focusedPaneID, edge: axis == .horizontal ? .trailing : .bottom)
    }

    private func makeTab(device target: DeviceStore? = nil, opening conversationID: String?) -> ConversationTab {
        let store = ConversationStore(device: target ?? application)
        // A tab owns its destination before reading finishes. Otherwise a
        // close/restart during a remote lookup restores an empty tab.
        if let conversationID { store.noteRestoredConversation(conversationID) }
        return ConversationTab(store: store, start: {
            store.start(loadFirstConversation: false, newConversation: conversationID == nil, opening: conversationID)
        })
    }

    /// Select the tab showing a conversation, if one does.
    private func select(showing conversationID: String, device: DeviceStore? = nil) -> Bool {
        guard let (_, tab) = tab(showing: conversationID, device: device) else { return false }
        selectTab(tab.id)
        return true
    }

    func tab(showing conversationID: String, device: DeviceStore? = nil) -> (Pane, ConversationTab)? {
        for pane in orderedPanes {
            if let tab = pane.tabs.compactMap({ $0 as? ConversationTab }).first(where: { tab in
                (device == nil || tab.store.device === device)
                    && (tab.store.selectedConversationID == conversationID || tab.store.reachingConversationID == conversationID)
            }) { return (pane, tab) }
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
        show(id)
        noteSelected(focusedEditor)
    }

    /// One-based visible order in the focused group; an absent slot is a no-op.
    func selectTab(at position: Int) {
        guard position > 0, position <= focusedPane.tabs.count else { return }
        selectTab(focusedPane.tabs[position - 1].id)
    }

    /// The tab after (or before) the focused pane's shown one.
    func selectNeighbourTab(forward: Bool) {
        let pane = focusedPane
        guard pane.tabs.count > 1, let index = pane.tabs.firstIndex(where: { $0.id == pane.selectedID }) else { return }
        selectTab(pane.tabs[(index + (forward ? 1 : pane.tabs.count - 1)) % pane.tabs.count].id)
    }

    /// The final document leaves a blank tab; closing that blank closes the window.
    func closeTab(_ id: UUID) {
        guard let pane = pane(holding: id), let closing = pane.tabs.first(where: { $0.id == id }) else { return }
        noteRecentlyClosed(closing, in: pane)
        let navigating = id == focusedEditor.id && tabCount > 1 && focusedEditor is ConversationTab
        if navigating, !tools.panesHaveComposer { drafts(for: focused.store.device).checkpoint() }
        closeSideChats(from: id)
        guard let tab = detach(id, emptying: .close) else { return }
        tab.close()
        if navigating, let next = focusedEditor as? ConversationTab {
            navigateComposer(to: next.store.loadedConversationID ?? "tab:\(next.id)", tab: next, checkpointPrevious: false)
        }
    }

    /// Side chats end with their source and are not reopenable.
    private func noteRecentlyClosed(_ tab: EditorTab, in pane: Pane) {
        let conversation = tab as? ConversationTab
        guard conversation?.store.isSideChat != true else { return }
        var target = conversation?.store.isUnsentConversation == true ? nil : conversation?.store.selectedConversationID ?? tabConversations[tab.id]
        if target?.hasPrefix(ConversationStore.draftConversationPrefix) == true { target = nil }
        recentlyClosed.append(ClosedTarget(device: conversation?.store.device.peerID, conversation: target,
                                          tabID: tab.id, paneID: pane.id, index: pane.tabs.firstIndex { $0.id == tab.id }!,
                                          layout: layout, creation: conversation?.store.unsentConversation,
                                          artifact: (tab as? ArtifactTab)?.document))
        if recentlyClosed.count > 25 { recentlyClosed.removeFirst() }
    }
    /// Restore identity and position, including a group removed with its last tab.
    func reopenClosedTab() {
        while let target = recentlyClosed.popLast() {
            guard let device = closedTargetDevice(target),
                  target.conversation.map({ !device.deletedConversationIDs.contains($0) }) ?? true else { continue }
            if let id = target.conversation, select(showing: id, device: device) { return }
            let tab: EditorTab
            if let artifact = target.artifact { tab = ArtifactTab(id: target.tabID, document: artifact) }
            else {
                let store = ConversationStore(device: device)
                if let id = target.conversation { store.noteRestoredConversation(id) }
                tab = ConversationTab(id: target.tabID, store: store, start: {
                    if let creation = target.creation { store.restoreUnsentConversation(creation) }
                    else { store.start(loadFirstConversation: false, newConversation: target.conversation == nil, opening: target.conversation) }
                })
            }
            if workspace.groups[target.paneID] != nil { insertTab(tab, in: target.paneID, at: target.index) }
            else {
                // Back where it was unless the workspace was rearranged since;
                // a rearranged one keeps its new groups.
                let currentIDs = Set(layout.paneIDs)
                let original = target.layout.paneIDs.filter { !currentIDs.contains($0) && $0 != target.paneID }
                    .reduce(target.layout) { $0.removing($1) ?? $0 }
                editors[tab.id] = tab
                show(tab.id) { layout in
                    if !layout.restoreGroup(target.paneID, with: tab.id, grid: original) {
                        layout.addGroup(target.paneID, with: tab.id, edge: .trailing)
                    }
                }
            }
            selectTab(tab.id)
            return
        }
    }
    private func closedTargetDevice(_ target: ClosedTarget) -> DeviceStore? {
        guard let peer = target.device else { return application }
        return hub?.store(for: peer)
    }
    var canReopenClosedTab: Bool {
        recentlyClosed.contains { entry in
            guard let device = closedTargetDevice(entry) else { return false }
            return entry.conversation.map { !device.deletedConversationIDs.contains($0) } ?? true
        }
    }
    func closeOtherTabs(_ id: UUID) {
        guard let pane = pane(holding: id) else { return }
        for tab in pane.tabs where tab.id != id { closeTab(tab.id) }
    }

    /// Close a pane with all its tabs.
    func closePane(_ id: UUID) {
        guard isSplit, let group = workspace.groups[id] else { return }
        for tab in group.tabs { closeSideChats(from: tab) }
        // Side chats closing may have changed the group, or closed it.
        guard isSplit, let closing = workspace.groups[id]?.tabs.compactMap({ editors[$0] }) else { return }
        removePane(id)
        for tab in closing {
            forgetTab(tab.id)
            tab.close()
        }
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
        guard let group = workspace.group(holding: id), let tab = editors[id] else { return nil }
        if let tab = tab as? ConversationTab, !tab.store.isSideChat, let conversationID = tab.store.loadedConversationID {
            tab.composer = drafts(for: tab.store.device).model(for: .conversation(conversationID), tab: tab)
        }
        if workspace.groups[group]!.tabs.count > 1 {
            take(id)
            if group == focusedPaneID { noteSelected(focusedEditor) }
        } else if isSplit {
            removePane(group)
        } else {
            switch emptying {
            case .keep: return nil
            case .close where (tab as? ConversationTab)?.isBlank != true,
                 .leave where hasToolWindows:
                insertTab(makeTab(opening: nil), in: group)
                take(id)
                noteSelected(focusedEditor)
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
        guard isSplit, workspace.groups[id] != nil else { return }
        let previous = composer(for: focused)
        let previousOwner = composerOwner(for: focused)
        let before = focusedPaneID
        // Focus moves to the group before it, or the next when it was first.
        changeLayout { $0.closeGroup(id) }
        if before == id { noteSelected(focusedEditor) }
        if !tools.panesHaveComposer, let previousOwner, let owner = composerOwner(for: focused), previousOwner != owner {
            drafts(for: focused.store.device).move(previous, to: owner, tab: focused)
        }
    }

    private func addPane(with tab: EditorTab, beside target: UUID, edge: PaneEdge) {
        preservingComposerLayout = true
        defer { preservingComposerLayout = false }
        let previous = composer(for: focused)
        let previousOwner = composerOwner(for: focused)
        editors[tab.id] = tab
        changeLayout { $0.addGroup(with: tab.id, beside: target, edge: edge) }
        tab.activate()
        noteSelected(tab)
        let inputTarget = (tab as? ConversationTab) ?? focused
        if !tools.panesHaveComposer, let previousOwner, let owner = composerOwner(for: inputTarget), previousOwner != owner {
            drafts(for: inputTarget.store.device).move(previous, to: owner, tab: inputTarget)
        }
    }

    // MARK: Moving

    /// A tab, or a conversation from the list, dropped on a pane: among its
    /// tabs (at `index`, else at the end) or beside an edge. A tab comes
    /// from wherever it is, this window or another; a conversation a tab of
    /// this window shows moves that tab; any other opens in a new tab.
    func drop(_ link: URL, on target: UUID, zone: PaneDropZone, at index: Int? = nil, from sourceWindow: ConversationWindowModel? = nil) {
        guard workspace.groups[target] != nil else { return }
        ListDragWatch.shared.accepted()
        if let sourceWindow, !acceptsEditor(from: sourceWindow) { return }
        let tabID = TabDragSource.shared.tab(dropping: link)
            ?? ConversationLink.conversation(in: link).flatMap { tab(showing: $0.conversationID)?.1.id }
        // A tab closed since its drag began leaves its conversation to open anew.
        guard let tabID, let source = Self.owning(tab: tabID) else {
            guard let conversationID = ConversationLink.conversation(in: link)?.conversationID else { return }
            if let (_, existing) = tab(showing: conversationID) {
                let before = focusedPaneID
                show(existing.id)
                return noteFocus(from: before)
            }
            let tab = makeTab(device: aggregated.device(of: conversationID)?.store, opening: conversationID)
            switch zone {
            case .center: insertTab(tab, in: target, at: index); noteSelected(tab)
            case .edge(let edge): addPane(with: tab, beside: target, edge: edge)
            }
            return
        }
        guard acceptsEditor(from: source) else { return }
        let sourceGroup = source === self ? workspace.group(holding: tabID) : nil
        switch zone {
        case .center where sourceGroup == target:
            let before = focusedPaneID
            show(tabID) { $0.move(tabID, to: target, at: index) }
            noteFocus(from: before)
        case .edge where sourceGroup == target && workspace.groups[target]!.tabs.count == 1:
            // A pane's only tab beside its own pane has nowhere new to go.
            break
        default:
            guard let tab = source.detach(tabID, emptying: .leave) else { return }
            if source !== self { source.releaseComposerForHandoff(tab) }
            adopt(tab)
            switch zone {
            case .center:
                // The source group may have been this one's last neighbour and closed with the tab.
                guard workspace.groups[target] != nil else { return insertTab(tab) }
                insertTab(tab, in: target, at: index); noteSelected(tab)
            case .edge(let edge):
                guard workspace.groups[target] != nil else { return }
                addPane(with: tab, beside: target, edge: edge)
            }
        }
    }

    /// Move a tab to `place` among its pane's tabs, counted as they will be.
    func moveTab(_ id: UUID, toPlace place: Int) {
        guard workspace.contains(tab: id) else { return }
        let before = focusedPaneID
        show(id) { $0.moveTab(id, toPlace: place) }
        noteFocus(from: before)
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
        let peer = (tab as? ConversationTab).flatMap { hub?.peerID(of: $0.store.device) } ?? device
        openWindow(.single, WindowSeed(tab: tab.id, device: peer))
        if let frame { Self.place(windowNotIn: existing, frame: frame, attempts: 20) }
    }

    /// Only crossing a window boundary separates local editor state. Closing
    /// a tab or rearranging panes in this window keeps the existing model.
    private func releaseComposerForHandoff(_ editor: EditorTab) {
        guard let tab = editor as? ConversationTab, !tab.store.isSideChat,
              let replacement = drafts(for: tab.store.device).releaseEditingModel(tab.composer) else { return }
        for remaining in conversationTabs where remaining !== tab && remaining.composer === tab.composer {
            remaining.composer = replacement
        }
        if restoredComposerDraft === tab.composer { restoredComposerDraft = replacement }
    }

    /// Open a conversation in a conversation window of its own; a tab of this
    /// window showing it moves there. At `point`, the window's top left is there.
    func openInNewWindow(_ conversationID: String, device target: DeviceStore? = nil, at point: NSPoint? = nil) {
        if let (_, tab) = tab(showing: conversationID, device: target) { return moveToNewWindow(tab.id, at: point) }
        guard let openWindow else { return }
        let frame = newWindowFrame(size: nil, at: point)
        let existing = Set(NSApp.windows.map(\.windowNumber))
        openWindow(.single, WindowSeed(conversationID: conversationID, device: target == nil ? device : target?.peerID))
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

    /// Only crossing a window boundary separates local editor state. Closing
    /// a tab or rearranging panes in this window keeps the existing model.
    /// Any two workspace windows show the same devices, so a tab crosses
    /// between them freely; single-conversation windows still match devices.
    private func acceptsEditor(from source: ConversationWindowModel) -> Bool {
        if source === self { return true }
        if kind == .workspace && source.kind == .workspace { return true }
        guard source.application !== application else { return true }
        list.recordProblem("标签页只能移到查看同一设备的窗口。", source: "移动标签页")
        return false
    }

    /// A tab arriving from another window.
    private func adopt(_ tab: EditorTab) {
        if let tab = tab as? ConversationTab, !tab.store.isSideChat, let conversationID = tab.store.loadedConversationID { drafts(for: tab.store.device).adopt(tab.composer, conversationID: conversationID, tab: tab) }
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
        guard workspace.groups[id] != nil, focusedPaneID != id else { return }
        changeLayout { $0.focus(id) }
        noteSelected(focusedEditor)
    }

    /// Focus the pane after (or before) the focused one, in layout order.
    func focusNeighbour(forward: Bool) {
        let order = layout.paneIDs
        guard order.count > 1, let index = order.firstIndex(of: focusedPaneID) else { return }
        focus(order[(index + (forward ? 1 : order.count - 1)) % order.count])
        // Pointer focus must leave the clicked control alone; this keyboard
        // command also moves the responder. Otherwise typing still edits the
        // previous group's composer despite the newly selected command target.
        window?.makeFirstResponder(nil)
        guard let tab = focusedEditor as? ConversationTab else { return }
        switch tools[.composer].place {
        case .builtIn:
            composer(for: tab).focusWhenShown()
        case .docked where tools.isOpen(.composer):
            composer(for: tab).focusWhenShown()
        default:
            // Changing groups does not reopen a hidden panel or bring a
            // detached tool forward. The selected editor takes the keyboard.
            tab.focusTranscript()
        }
    }

    func resize(at path: [Int], fractions: [Double]) {
        changeLayout { $0.resize(at: path, fractions: fractions) }
    }

    func register(_ pane: UUID, anchor: @escaping () -> NSView?) { anchors[pane] = anchor }
    func view(of pane: UUID) -> NSView? { anchors[pane]?() }

    private func follow(_ editor: EditorTab) {
        guard let tab = editor as? ConversationTab, !tab.store.isSideChat else { return }
        // Unsent tabs may never select a conversation, but their execution
        // device already determines where their creation target will run.
        noteTabDevice(tab)
        var watches = Set<AnyCancellable>()
        tab.store.composerEvents.sink { [weak self, weak tab] navigation in
            guard let self, let tab else { return }
            if case let .created(previous, id, runtimeID, input) = navigation {
                self.drafts(for: tab.store.device).reassociate(input, from: previous, to: id, runtimeID: runtimeID, targetTab: tab, publish: false)
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
                self.drafts(for: tab.store.device).removeConversations(tab.store.device.deletedConversationIDs)
                if tab === self.focused, tab.store.loadedConversationID != nil, let restored = self.restoredComposerDraft {
                    self.restoredComposerDraft = nil
                    if let owner = self.composerOwner(for: tab) { self.drafts(for: tab.store.device).move(restored, to: owner, tab: tab) }
                }
            }
            if self.tabConversations[tab.id] != id {
                self.closeSideChats(from: tab.id)
                self.tabConversations[tab.id] = id
                self.noteTabDevice(tab)
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
    private var refreshToolbarAnchor: (() -> NSView?)?

    func registerToolbarButton(_ kind: ToolWindowKind, anchor: @escaping () -> NSView?) { toolbarAnchors[kind] = anchor }
    func registerRefreshToolbarButton(anchor: @escaping () -> NSView?) { refreshToolbarAnchor = anchor }

    /// NSToolbar consumes SwiftUI context menus before they reach the button.
    /// Reuse the window's native event bridge for both native context gestures.
    private func showsRefreshMenu(for event: NSEvent) -> Bool {
        guard let view = refreshToolbarAnchor?(), view.window === event.window,
              view.bounds.contains(view.convert(event.locationInWindow, from: nil)) else { return false }
        let store = focused.store
        let menu = NSMenu()
        menu.autoenablesItems = false
        let conversation = ClosureMenuItem("刷新当前会话") { [weak store] in store?.reloadFromRuntimes() }
        let device = ClosureMenuItem("刷新设备状态") { [weak store] in store?.reloadDeviceState() }
        conversation.isEnabled = store.canReloadFromRuntimes
        device.isEnabled = store.canReloadFromRuntimes
        menu.addItem(conversation)
        menu.addItem(device)
        NSMenu.popUpContextMenu(menu, with: event, for: view)
        return true
    }

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

    /// A primary click in a pane focuses it, wherever in the pane it lands.
    /// Context menus and middle-button close act on their target without
    /// changing the active pane or the shared composer.
    private func installMouseMonitor() {
        guard mouseMonitor == nil else { return }
        mouseMonitor = NSEvent.addLocalMonitorForEvents(matching: [.leftMouseDown, .rightMouseDown, .otherMouseDown]) { [weak self] event in
            guard let self else { return event }
            return self.handleMouseDown(event)
        }
    }

    func handleMouseDown(_ event: NSEvent) -> NSEvent? {
        if event.type == .rightMouseDown || (event.type == .leftMouseDown && event.modifierFlags.contains(.control)),
           showsRefreshMenu(for: event) { return nil }
        if event.type == .rightMouseDown, showsToolbarPlaceMenu(for: event) { return nil }
        guard event.type == .leftMouseDown, !event.modifierFlags.contains(.control) else { return event }
        noteClick(event)
        for tab in conversationTabs {
            guard let view = conversationAnchors[tab.id]?(), view.window === event.window,
                  view.bounds.contains(view.convert(event.locationInWindow, from: nil)) else { continue }
            focusConversation(tab)
            break
        }
        guard isSplit else { return event }
        for (id, anchor) in anchors {
            guard let view = anchor(), let window = view.window, window === event.window else { continue }
            if view.bounds.contains(view.convert(event.locationInWindow, from: nil)) {
                focus(id)
                break
            }
        }
        return event
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
