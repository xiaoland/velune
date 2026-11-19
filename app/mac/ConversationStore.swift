import Combine
import Foundation
import AppKit
import VeluneBindings

/// One tab's execution target and its observed projection. This owns target
/// preparation, refresh and request lifetime; windows pair it with independent
/// input content. Device configuration and shared input state come from `device`.
@MainActor
final class ConversationStore: ObservableObject {
    enum ComposerEvent { case conversation(String), newConversation, creationTargetChanged, created(previous: String, id: String, runtimeID: String, input: ComposerModel) }
    let composerEvents = PassthroughSubject<ComposerEvent, Never>()
    let device: DeviceStore
    let sideChatContext: SideChatTarget?
    var isSideChat: Bool { sideChatContext != nil }
    var allowsAttachments: Bool { !isSideChat && draftEditable }
    private let sideChatLease: SideChatLease?
    /// A temporary side chat owns its next-turn choices for its lifetime;
    /// neither its source conversation nor the shared composer directory does.
    private var temporaryChoices = ConversationChoices()
    @Published private(set) var openingFailure: AppProblem?
    @Published private(set) var creationFailure: String?
    /// This window's search: its text, and the matches once read.
    @Published private(set) var searchText = ""
    @Published private(set) var searchResults: (conversations: [Conversation], groups: [ConversationGroup])?
    private var searchRequest = 0
    /// Conversations reached outside the list (a link, a search result), so they can be opened.
    private var offListConversations: [Conversation] = []
    @Published private(set) var selectedConversationIDs: Set<String> = []
    @Published private(set) var selectedConversationID: String?
    @Published private(set) var pendingConversationID: String?
    /// A destination whose source is being resolved on this device.
    @Published private(set) var reachingConversationID: String?
    private var linkedConversationRequest: UUID?
    /// A tab's first destination (restored, listed or linked) may open while
    /// its history/device is not ready. Keep it until success or navigation.
    private var unopenedConversationID: String?
    private var openingNeedsRetry = false
    private var openingRetry = PollBackoff()
    /// The conversation is being opened, as loading shows.
    var isOpeningConversation: Bool { pendingConversationID != nil || reachingConversationID != nil || openingNeedsRetry || (isSideChat && snapshot == nil && openingFailure == nil) }
    @Published private(set) var projectionRuntimeID: String?
    @Published private(set) var nextTurnRuntimeID: String? { didSet { if isSideChat, oldValue != nextTurnRuntimeID { loadModelCatalog() } } }
    /// The shared target facade's effective choice, resolved again with local
    /// pending intent and catalog arrivals through the same shared rules.
    @Published private(set) var nextTurn: NextTurn?
    @Published private(set) var composerTargetFailure: String?
    private var preparedComposerTarget: PreparedComposerTarget?
    private var composerTargetRequestID: UUID?
    private var composerTargetNeedsRefresh = false
    /// A failed preparation is retried with backoff; changed facts retry at once.
    private var composerTargetRetry = PollBackoff()
    private var composerTargetClosed = false
    /// An explicit runtime action may arrive before the first authority read.
    private var pendingComposerRuntime: (target: ComposerTargetRequest, runtimeID: String)?
    /// Model catalogs by runtime instance, read from the runtime and Velune's
    /// gateway; configuration changes discard them.
    @Published private(set) var modelCatalogs: [String: RuntimeModelCatalog] = [:]
    private var modelCatalogRequests: [String: UUID] = [:]
    /// The open conversation's live-session options, for runtimes that have
    /// them; `nil` until its session is live on the device.
    @Published private(set) var sessionOptions: [AcpConfigOption]?
    /// Conversation and run state the options were last read for.
    private var sessionOptionsKey: String?
    let transcript = TranscriptModel()
    private let outgoing = OutgoingSubmissions()
    private struct QuotaResult {
        let request: ComposerQuotaRequest
        let entries: [ComposerQuotaEntry]
        let failure: String?
    }
    @Published private var composerQuotaResults: [QuotaResult] = []
    @Published private(set) var isLoading = false {
        didSet { if oldValue && !isLoading { refreshNextTurn() } }
    }
    @Published private(set) var activity: TranscriptActivity?
    @Published private var snapshot: ConversationSnapshot?
    @Published private var executionStarting = false
    @Published private(set) var composerRequest: ComposerSubmission.Phase?
    /// Counts sends, queued messages, insertions and resumes the core took.
    @Published private(set) var acceptedSubmissions: UInt64 = 0
    @Published private var stopRequested = false
    /// A conversation chosen while an operation ran, opened when it ends.
    private var deferredSelection: (id: String, move: NavigationMove)?
    private enum InitialSelection { case firstOrDraft, draft }
    private var initialSelection: InitialSelection?
    private var started = false
    private var isDraft = false
    private var subscriptions: Set<AnyCancellable> = []
    private var deviceUpdateScheduled = false
    /// What of the device's this tab last followed.
    private var seenListRevision = -1
    private var seenConfigurationRevision: UInt64 = 0
    private var seenComposerChoicesRevision = 0
    private let queue = DispatchQueue(label: "local.velune.requests", qos: .userInitiated)
    private var generation = 0
    private var timer: Timer?
    private var pollPending = false
    private var payloadRequests: Set<PayloadKey> = []
    /// Complete messages read on demand, by message identifier.
    private var loadedPayloads: [String: DeliveredMessage] = [:]
    static let draftConversationPrefix = "draft:"
    var unsentTabID = UUID()
    var unsentContextID: String { Self.draftConversationPrefix + unsentTabID.uuidString.lowercased() }
    private(set) var restoresLegacyUnsentInput = false

    init(device: DeviceStore, sideChat: SideChatTarget? = nil) {
        self.device = device
        sideChatContext = sideChat
        if let sideChat {
            temporaryChoices = ConversationChoices(runtimeID: sideChat.runtimeID, choices: [sideChat.runtimeID:
                TurnChoice(model: sideChat.selection, effort: sideChat.effort, permissionMode: sideChat.permissionMode)])
        }
        sideChatLease = sideChat.flatMap { _ in device.transport.map(SideChatLease.init) }
        seenConfigurationRevision = device.configurationRevision
        seenComposerChoicesRevision = device.composerChoicesRevision
        outgoing.changes.sink { [weak self] in
            guard let self else { return }
            transcript.showSubmissions(outgoing.displayed)
        }.store(in: &subscriptions)
        device.objectWillChange.sink { [weak self] in
            guard let self else { return }
            self.objectWillChange.send()
            guard !self.deviceUpdateScheduled else { return }
            self.deviceUpdateScheduled = true
            Task { @MainActor in
                self.deviceUpdateScheduled = false
                self.deviceChanged()
            }
        }.store(in: &subscriptions)
        device.composerInputs.changes.sink { [weak self] in
            self?.composerInputStateChanged()
        }.store(in: &subscriptions)
        device.conversationRenamed.sink { [weak self] change in
            guard let self, snapshot?.conversation.id == change.id else { return }
            snapshot?.conversation.title = change.title
        }.store(in: &subscriptions)
        deviceChanged()
        if isPreview {
            if let sideChat { nextTurnRuntimeID = sideChat.runtimeID; nextTurn = sideChat.nextTurn }
            apply(device.previewSnapshots["sample-0"])
        }
    }

    var isPreview: Bool { device.isPreview }
    private var transport: Transport? { device.transport }
    var isShuttingDown: Bool { device.isShuttingDown }

    /// Follows what changed on the device: a list read, configuration the
    /// catalogs depend on, kept choices once read, a deleted conversation.
    private func deviceChanged() {
        if isSideChat {
            if seenConfigurationRevision != device.configurationRevision {
                seenConfigurationRevision = device.configurationRevision
                invalidateModelCatalogs()
            }
            return
        }
        if let loadedConversationID, device.deletedConversationIDs.contains(loadedConversationID) { resetProjection() }
        if device.listRevision != seenListRevision {
            seenListRevision = device.listRevision
            // Organization changes the list's own rows; reached ones follow them.
            let listed = Dictionary(device.conversations.map { ($0.id, $0) }, uniquingKeysWith: { first, _ in first })
            offListConversations = offListConversations.map { listed[$0.id] ?? $0 }
            if searchResults != nil { refreshSearch() }
            let selected = selectedConversationIDs.filter { conversation(id: $0) != nil }
            if selected != selectedConversationIDs { selectedConversationIDs = selected }
            applyInitialSelectionIfReady()
            retryOpeningIfNeeded()
        }
        let configurationChanged = device.configurationRevision != seenConfigurationRevision
        let defaultsChanged = device.composerChoicesRevision != seenComposerChoicesRevision
        seenConfigurationRevision = device.configurationRevision
        seenComposerChoicesRevision = device.composerChoicesRevision
        if configurationChanged { modelCatalogs.removeAll(); modelCatalogRequests.removeAll() }
        noteSeen()
        if configurationChanged || defaultsChanged { requestComposerTarget(refreshFacts: true) }
        else { refreshNextTurn() }
    }

    /// Some conversation executes, in any window. Configuration its execution
    /// depends on cannot change meanwhile; other conversations stay usable.
    var applicationIsGenerating: Bool { isGenerating || executionStarting || device.hasRunningWork }
    var loadedConversationID: String? { snapshot?.conversation.id }
    var loadedConversationWorkingDirectory: String? { snapshot?.conversation.cwd }
    var loadedConversationProject: NewConversationProject? { isDraft ? snapshot?.conversation.creationProject : nil }
    /// Set while a native sub-agent's conversation is shown: the conversation that started it.
    var loadedParentConversationID: String? { snapshot?.conversation.parentConversationID }
    /// The list's conversations, the open one with its runtime's fields, and
    /// first the open one when the list does not show it.
    var conversations: [Conversation] {
        var listed = device.conversations
        guard var current = snapshot?.conversation, !isDraft, !isSideChat else { return listed }
        if let remembered = device.rememberedConversations[current.id] { current = current.organized(like: remembered) }
        if !listed.contains(where: { $0.id == current.id }) { var unlisted = current; unlisted.group = nil; listed.insert(unlisted, at: 0) }
        return listed
    }
    /// The conversations the sidebar lists; native sub-agents open from their parent's transcript.
    var sidebarConversations: [Conversation] { device.conversations.filter { $0.group != nil } }
    /// A conversation by identity: listed, or reached outside the list.
    func conversation(id: String) -> Conversation? {
        if let current = snapshot?.conversation, current.id == id, !isDraft { return isSideChat ? current : conversations.first { $0.id == id } }
        return device.conversations.first { $0.id == id } ?? searchResults?.conversations.first { $0.id == id } ?? offListConversations.first { $0.id == id }
    }
    var selectedConversationTitle: String? { selectedConversationID.flatMap { conversation(id: $0) }?.title ?? (snapshot?.conversation.id == selectedConversationID ? snapshot?.conversation.title : nil) }
    var isGenerating: Bool { snapshot?.runState == .running || snapshot?.runState == .stopping }
    var isBusy: Bool { isLoading || applicationIsGenerating || isShuttingDown || device.authenticationRunning }
    var canManageConversations: Bool { !isSideChat && !isShuttingDown }
    /// A running conversation keeps its native session until the turn ends.
    func isRunning(_ conversation: Conversation) -> Bool { device.runningConversationIDs.contains(conversation.id) || (loadedConversationID == conversation.id && (isGenerating || executionStarting)) }
    /// The core validates the selection again; an unread catalog does not block sending.
    var isStopping: Bool { stopRequested || snapshot?.runState == .stopping }
    private var activityBlocksSending: Bool {
        loadedConversationID.map(device.isConversationRunning) == true
    }
    var inputAccess: InputAccess { snapshot?.actions.inputAccess ?? .editable }
    var inputSubmissionAllowed: Bool { inputAccess == .editable }
    /// Runtime ownership restricts submission, not local drafting.
    var draftEditable: Bool {
        if case .readOnly = inputAccess { return false }
        return !isShuttingDown
    }
    var inputLockMessage: String? { inputAccess.message }
    var canSend: Bool { !executionStarting && !device.authenticationRunning && !isLoading && !isStopping && !activityBlocksSending && !isShuttingDown && (snapshot?.actions.canSend ?? false) }
    var canCancel: Bool { !isStopping && snapshot?.runState == .running && (snapshot?.actions.canCancel ?? false) }
    var pendingQueuedMessages: [QueuedMessage] { outgoing.queued }
    var followUp: FollowUp {
        var value = snapshot?.followUp ?? FollowUp()
        let ids = Set(value.queue.map(\.id))
        let tokens = Set(value.queue.compactMap(\.submissionId))
        value.queue += outgoing.queued.filter { !ids.contains($0.id) && !($0.submissionId.map(tokens.contains) ?? false) }
        return value
    }
    var controlHandoff: ControlHandoff? { isOpeningConversation || isSideChat ? nil : followUp.controlHandoff }
    /// The loaded conversation's native sub-agents.
    var subAgents: [SubAgent] { pendingConversationID == nil ? snapshot?.subAgents ?? [] : [] }
    /// The conversations the loaded one took part with in agent-to-agent collaboration.
    var collaborators: [Collaborator] { pendingConversationID == nil ? snapshot?.collaborators ?? [] : [] }
    /// The loaded conversation has a parent, sub-agents or collaborators to list.
    var hasLinkedAgents: Bool { pendingConversationID == nil && (loadedParentConversationID != nil || !subAgents.isEmpty || !collaborators.isEmpty) }
    /// While a turn runs, a sent message waits in the conversation's queue.
    var canQueue: Bool { followUp.canQueue && !isPreview && !isStopping && !executionStarting && !isLoading && !isShuttingDown && !device.authenticationRunning }
    var canResume: Bool { canResume(next: nextTurn) }
    func canResume(next: NextTurn?) -> Bool { canSend && followUp.canResume && next?.runtimeId == snapshot?.contextRuntimeID }
    var canCompact: Bool { canCompact(next: nextTurn) }
    func canCompact(next: NextTurn?) -> Bool { canSend && followUp.canCompact && next?.runtimeId == snapshot?.contextRuntimeID }
    var continueHelp: String { "继续当前会话；不支持原生恢复的运行时会发送“continue”" }
    /// The next turn's choices are final: its runtime's catalog was read, or could not be.
    var nextTurnSettled: Bool {
        guard nextTurn != nil, let runtimeID = nextTurnRuntimeID else { return false }
        if case .loading = modelCatalogs[runtimeID] ?? .loading { return false }
        return true
    }

    // MARK: - Next turn

    var canSwitchModel: Bool { nextTurnRuntimeID != nil && !isShuttingDown }
    var modelCatalog: RuntimeModelCatalog? { nextTurnRuntimeID.flatMap { modelCatalogs[$0] } }
    var chosenModelUnavailable: Bool { nextTurn?.chosenModelUnavailable ?? false }
    /// Whether the conversation has a turn on the next turn's runtime, whose
    /// choices an unchosen part follows; otherwise it is the runtime's default.
    var nextTurnHasCurrent: Bool { nextTurn?.hasCurrent ?? false }
    var modelChoices: [RuntimeModel] { nextTurn?.models ?? [] }
    /// The model chosen for the next turn; `nil` follows the conversation's current one.
    var chosenModel: ModelSelection? { nextTurn?.choice.model }
    /// The model the next turn is sent with; `nil` is the runtime's own default.
    var nextTurnModel: ModelSelection? { nextTurn?.model }
    var selectedModelName: String? {
        guard let nextTurn else { return nil }
        return nextTurn.model == nil ? "运行时默认" : nextTurn.modelName
    }
    /// The chosen level while the model accepts it; `nil` follows the conversation's current one.
    var nextTurnEffort: String? { nextTurn?.effort }
    var reasoningEffortOptions: [String] { nextTurn?.efforts ?? [] }
    var permissionModeOptions: [PermissionMode] { nextTurn?.permissionModes ?? [] }
    var effectivePermissionMode: PermissionMode? { nextTurn?.permissionMode }
    var currentPermissionMode: PermissionMode? { nextTurn?.currentPermissionMode }
    var currentEffort: String? { nextTurn?.currentEffort }
    /// Side chats keep their target choices for the lifetime of this temporary store.
    /// Ordinary target commands below persist raw intent through the shared client.
    func changeSideChatChoice(_ change: TurnChoiceChange) {
        guard isSideChat, let nextTurn,
              let update = VeluneBindings.changeNextTurn(next: nextTurn, runtimeChoices: device.runtimeComposerChoices[nextTurn.runtimeId], change: change) else { return }
        if let choices = update.runtimeChoices { device.setRuntimeComposerChoices(choices) }
        temporaryChoices.choices[nextTurn.runtimeId] = update.choice
        refreshNextTurn()
    }
    func selectSideChatRuntime(_ id: String) {
        guard isSideChat, !isShuttingDown, device.enabledRuntimeInstances.contains(where: { $0.id == id }), device.hasSideChats(id) else { return }
        temporaryChoices.runtimeID = id
        if nextTurnRuntimeID == id { refreshNextTurn() } else { nextTurnRuntimeID = id }
    }
    /// Reads the next-turn runtime's catalog again, e.g. after its native
    /// configuration or session changed outside Velune's settings.
    func reloadModelCatalog(runtimeID explicitRuntimeID: String? = nil) {
        guard let runtimeID = explicitRuntimeID ?? nextTurnRuntimeID else { return }
        modelCatalogs[runtimeID] = nil
        loadModelCatalog(runtimeID: runtimeID)
    }
    private func invalidateModelCatalogs() {
        modelCatalogs.removeAll(); modelCatalogRequests.removeAll()
        loadModelCatalog()
    }
    /// Listing native models starts the runtime, so a catalog is read only
    /// for an open conversation's next-turn runtime and kept until invalidated.
    func loadModelCatalog(runtimeID explicitRuntimeID: String? = nil) {
        // Navigation retains the previous snapshot until the destination opens.
        // Its composer event must not start a runtime catalog ahead of that open.
        guard !isOpeningConversation, snapshot != nil, let runtimeID = explicitRuntimeID ?? nextTurnRuntimeID, device.enabledRuntimeInstances.contains(where: { $0.id == runtimeID }), modelCatalogs[runtimeID] == nil else { refreshNextTurn(); return }
        if isPreview {
            let managed = device.gateway.providers.flatMap { provider in provider.models.map { RuntimeModel(selection: .managed(modelRecordKey: $0.recordKey), name: "\($0.displayName) · \(provider.name)", reasoningEfforts: $0.reasoningLevels ?? []) } }
            modelCatalogs[runtimeID] = .ready([RuntimeModel(selection: .native(providerId: "sample", modelId: "sample-native"), name: "运行时示例模型", reasoningEfforts: ["low", "medium", "high"])] + managed)
            refreshNextTurn(); return
        }
        guard let transport, !isShuttingDown else { refreshNextTurn(); return }
        let request = UUID()
        modelCatalogRequests[runtimeID] = request
        modelCatalogs[runtimeID] = .loading
        refreshNextTurn()
        performOnQueue { [weak self] in
            let result = Result { try transport.runtimeModels(runtimeID: runtimeID) }
            DispatchQueue.main.async {
                guard let self, self.modelCatalogRequests[runtimeID] == request else { return }
                self.modelCatalogRequests[runtimeID] = nil
                switch result {
                case .success(let models): self.modelCatalogs[runtimeID] = .ready(models)
                case .failure(let error): self.modelCatalogs[runtimeID] = .failed(AppProblem.failure(error, source: "模型目录").detail)
                }
                self.refreshNextTurn()
            }
        }
    }
    /// A target's execution projection is owned by this Store, independently
    /// of which window input or editor currently addresses it.
    private var composerTargetRequest: ComposerTargetRequest? {
        guard !isSideChat, !composerTargetClosed, !isShuttingDown, !isLoading,
              !isOpeningConversation, let snapshot,
              selectedConversationID == nil || selectedConversationID == snapshot.conversation.id else { return nil }
        let unsent = isDraft || snapshot.conversation.id.hasPrefix(Self.draftConversationPrefix)
        return ComposerTargetRequest(runtimeId: unsent ? snapshot.contextRuntimeID : snapshot.conversation.runtimeID,
                                     conversationId: snapshot.conversation.id)
    }
    private var composerTargetID: String? { selectedConversationID ?? loadedConversationID }
    var composerTargetChoices: ComposerTargetChoices? {
        guard !isSideChat else { return nil }
        return composerTargetID.flatMap { device.composerInputs.targetChoices(for: $0) }
    }
    /// A copied composer state fixes the values currently shown, including
    /// inherited execution settings that raw intent deliberately leaves nil.
    var composerNavigationChoices: ComposerTargetChoices? {
        guard let next = nextTurn, let owner = device.composerInputs.ownerID else { return composerTargetChoices }
        var choices = composerTargetChoices ?? ComposerTargetChoices(runtimeId: nil, runtimeOwnerId: nil, choices: [], choiceEdited: false)
        choices.runtimeId = next.runtimeId; choices.runtimeOwnerId = owner; choices.choiceEdited = true
        choices.choices.removeAll { $0.ownerId == owner && $0.runtimeId == next.runtimeId }
        let effective = TurnChoice(model: next.model, effort: next.effort ?? next.currentEffort,
                                   permissionMode: (next.permissionMode ?? next.currentPermissionMode)?.id)
        choices.choices.append(.init(ownerId: owner, runtimeId: next.runtimeId, choice: effective))
        return choices
    }
    var composerTargetIssue: String? {
        guard !isSideChat else { return nil }
        guard let request = composerTargetRequest else { return "正在打开目标会话" }
        if composerTargetRequestID != nil { return "正在刷新目标执行配置" }
        if let failure = composerTargetFailure { return failure }
        guard preparedComposerTarget?.request == request, nextTurn != nil else { return "正在准备目标执行配置" }
        return nil
    }
    private func composerInputStateChanged() {
        if let pending = pendingComposerRuntime {
            if pending.target != composerTargetRequest { pendingComposerRuntime = nil }
            else if device.composerInputs.ownerID != nil {
                pendingComposerRuntime = nil
                selectComposerRuntime(pending.runtimeID)
            }
        }
        refreshNextTurn()
    }
    func selectComposerRuntime(_ id: String) {
        guard !isSideChat, let request = composerTargetRequest,
              device.enabledRuntimeInstances.contains(where: { $0.id == id }) else { return }
        guard let owner = device.composerInputs.ownerID, let targetID = composerTargetID else {
            pendingComposerRuntime = (request, id)
            device.composerInputs.poll(force: true)
            return
        }
        var choices = normalizeComposerTargetChoices(choices: composerTargetChoices ?? ComposerTargetChoices(runtimeId: nil, runtimeOwnerId: nil, choices: [], choiceEdited: false))
        choices.runtimeId = id; choices.runtimeOwnerId = owner; choices.choiceEdited = true
        device.composerInputs.saveTarget(targetID, choices: choices, userEdit: true)
        refreshNextTurn()
    }
    func changeComposerChoice(_ change: TurnChoiceChange) {
        if isSideChat { changeSideChatChoice(change); return }
        guard let nextTurn, let targetID = composerTargetID, let owner = device.composerInputs.ownerID,
              composerTargetRequest != nil else { return }
        let raw = composerTargetChoices ?? ComposerTargetChoices(runtimeId: nil, runtimeOwnerId: nil, choices: [], choiceEdited: false)
        guard raw.runtimeOwnerId == nil || raw.runtimeOwnerId == owner,
              let update = changeComposerTarget(next: nextTurn, targetChoices: raw, ownerId: owner,
                                                runtimeChoices: device.runtimeComposerChoices[nextTurn.runtimeId], change: change) else { return }
        if let defaults = update.runtimeChoices { device.setRuntimeComposerChoices(defaults) }
        device.composerInputs.saveTarget(targetID, choices: update.choices, userEdit: true)
        refreshNextTurn()
    }
    func captureComposerTarget(_ inputID: UUID) {
        guard isUnsentConversation else { return }
        let raw = normalizeComposerTargetChoices(choices: composerTargetChoices ?? ComposerTargetChoices(runtimeId: nil, runtimeOwnerId: nil, choices: [], choiceEdited: false))
        device.composerInputs.captureTarget(inputID, choices: raw)
    }
    private func refreshNextTurn() {
        if isSideChat { refreshSideChatNextTurn(); return }
        guard let request = composerTargetRequest else {
            if nextTurn != nil { nextTurn = nil }
            return
        }
        guard let prepared = preparedComposerTarget, prepared.request == request,
              let resolved = resolveComposerTarget(prepared: prepared, choices: composerTargetChoices,
                                                   catalog: modelCatalogs[prepared.basis.runtimeId]?.models) else {
            if nextTurn != nil { nextTurn = nil }
            requestComposerTarget()
            return
        }
        if nextTurn != resolved { nextTurn = resolved }
        if nextTurnRuntimeID != resolved.runtimeId { nextTurnRuntimeID = resolved.runtimeId }
        if modelCatalogs[resolved.runtimeId] == nil, device.enabledRuntimeInstances.contains(where: { $0.id == resolved.runtimeId }) {
            loadModelCatalog(runtimeID: resolved.runtimeId)
        }
    }
    private func requestComposerTarget(refreshFacts: Bool = false) {
        guard !isSideChat, let request = composerTargetRequest else { return }
        if !refreshFacts, composerTargetFailure != nil, !composerTargetRetry.ready { return }
        if composerTargetRequestID != nil {
            if refreshFacts { composerTargetNeedsRefresh = true }
            return
        }
        if isPreview { preparePreviewComposerTarget(request); return }
        let client = device.composerInputs
        guard client.state != nil, client.ownerID != nil else { client.poll(); return }
        guard let transport, let targetID = composerTargetID else { return }
        let requestID = UUID(), currentGeneration = generation
        let configurationRevision = device.configurationRevision
        let defaultsRevision = device.composerChoicesRevision
        let overlay = client.pendingTarget(for: targetID)?.choices
        let requestedChoices = normalizeComposerTargetChoices(choices: client.targetChoices(for: targetID)
            ?? ComposerTargetChoices(runtimeId: nil, runtimeOwnerId: nil, choices: [], choiceEdited: false))
        composerTargetRequestID = requestID
        composerTargetNeedsRefresh = false
        objectWillChange.send()
        performOnQueue { [weak self] in
            let result = Result { try transport.prepareComposerTarget(request, overlayChoices: overlay) }
            DispatchQueue.main.async {
                guard let self, self.composerTargetRequestID == requestID else { return }
                self.composerTargetRequestID = nil
                let refreshAgain = self.composerTargetNeedsRefresh
                self.composerTargetNeedsRefresh = false
                guard self.generation == currentGeneration, self.composerTargetRequest == request,
                      self.device.configurationRevision == configurationRevision,
                      self.device.composerChoicesRevision == defaultsRevision else {
                    self.requestComposerTarget(refreshFacts: true)
                    return
                }
                let choicesChanged = requestedChoices != normalizeComposerTargetChoices(choices: client.targetChoices(for: targetID)
                    ?? ComposerTargetChoices(runtimeId: nil, runtimeOwnerId: nil, choices: [], choiceEdited: false))
                switch result {
                case .success(let prepared):
                    guard prepared.request == request, prepared.ownerId == client.ownerID,
                          prepared.context == client.context(targetID) else {
                        self.composerTargetRetry.failed()
                        self.composerTargetFailure = "执行端身份已变化，请重新连接设备"
                        self.objectWillChange.send()
                        return
                    }
                    self.preparedComposerTarget = prepared
                    self.composerTargetRetry.succeeded()
                    self.composerTargetFailure = nil
                    client.observePreparedTarget(prepared)
                    self.refreshNextTurn()
                case .failure(let error):
                    self.composerTargetRetry.failed()
                    let detail = AppProblem.failure(error, source: "准备输入目标").detail
                    if self.composerTargetFailure != detail {
                        self.composerTargetFailure = detail
                        self.device.recordProblem(error, source: "准备输入目标")
                    }
                }
                self.objectWillChange.send()
                if refreshAgain || choicesChanged, self.composerTargetRequestID == nil { self.requestComposerTarget(refreshFacts: true) }
            }
        }
    }
    /// Preview supplies synthetic execution facts to the same shared resolver;
    /// it does not turn observed defaults into explicit user choices.
    private func preparePreviewComposerTarget(_ request: ComposerTargetRequest) {
        guard let owner = device.composerInputs.ownerID, let targetID = composerTargetID,
              let context = device.composerInputs.context(targetID), let snapshot else { return }
        let raw = normalizeComposerTargetChoices(choices: composerTargetChoices ?? ComposerTargetChoices(runtimeId: nil, runtimeOwnerId: nil, choices: [], choiceEdited: false))
        guard raw.runtimeOwnerId == nil || raw.runtimeOwnerId == owner else {
            composerTargetFailure = "所选运行时位于另一设备，输入已保留"
            nextTurn = nil
            return
        }
        let runtimeID = raw.runtimeId ?? snapshot.contextRuntimeID
        guard device.enabledRuntimeInstances.contains(where: { $0.id == runtimeID }) else {
            composerTargetFailure = "所选运行时在此设备不可用"
            nextTurn = nil
            return
        }
        guard let policy = try? device.newConversationChoice(for: runtimeID) else { nextTurn = nil; return }
        let typeID = device.runtimeInstances.first { $0.id == runtimeID }?.typeId
        let basis = NextTurnInput(runtimeId: runtimeID, runtimeType: device.runtimeTypes.first { $0.id == typeID },
                                 providers: device.gateway.providers, catalog: nil,
                                 turnSettings: request.conversationId == nil ? [] : snapshot.transcriptTurns.compactMap(\.settings),
                                 conversationRuntimeId: snapshot.conversation.runtimeID, modelRecordKey: snapshot.modelRecordKey,
                                 kept: nil, runtimeChoices: device.runtimeComposerChoices[runtimeID], newConversation: policy)
        let target = device.composerInputs.targetRecord(for: targetID)
            ?? ComposerTarget(context: context, revision: 1, choices: ComposerTargetChoices(runtimeId: nil, runtimeOwnerId: nil, choices: [], choiceEdited: false))
        let prepared = PreparedComposerTarget(request: request, ownerId: owner, context: context, target: target,
                                              defaultRuntimeId: snapshot.contextRuntimeID, basis: basis, nextTurn: resolveNextTurn(input: basis))
        preparedComposerTarget = prepared
        device.composerInputs.observePreparedTarget(prepared)
        composerTargetFailure = nil
        refreshNextTurn()
    }
    private func refreshSideChatNextTurn() {
        guard let runtimeID = nextTurnRuntimeID else { if nextTurn != nil { nextTurn = nil }; return }
        guard let newConversation = try? device.newConversationChoice(for: runtimeID) else { nextTurn = nil; return }
        let typeID = device.runtimeInstances.first { $0.id == runtimeID }?.typeId
        let resolved = resolveNextTurn(input: NextTurnInput(
            runtimeId: runtimeID, runtimeType: device.runtimeTypes.first { $0.id == typeID }, providers: device.gateway.providers,
            catalog: modelCatalogs[runtimeID]?.models, turnSettings: snapshot?.transcriptTurns.compactMap(\.settings) ?? [],
            conversationRuntimeId: snapshot?.conversation.runtimeID, modelRecordKey: snapshot?.modelRecordKey,
            kept: temporaryChoices.choices[runtimeID], runtimeChoices: device.runtimeComposerChoices[runtimeID], newConversation: newConversation))
        if nextTurn != resolved { nextTurn = resolved }
    }
    /// Message editing addresses the current native context rather than the
    /// composer's selected execution target. Its existing caller owns that path.
    func resolve(kept: ConversationChoices, editingMessageID: String? = nil) -> NextTurn? {
        guard let runtimeID = kept.runtimeID ?? nextTurnRuntimeID,
              let policy = try? device.newConversationChoice(for: runtimeID) else { return nil }
        let typeID = device.runtimeInstances.first { $0.id == runtimeID }?.typeId
        let settings = isDraft ? [] : snapshot?.transcriptTurns
            .filter { editingMessageID == nil || $0.userMessageId == editingMessageID }.compactMap(\.settings) ?? []
        var resolved = resolveNextTurn(input: NextTurnInput(
            runtimeId: runtimeID, runtimeType: device.runtimeTypes.first { $0.id == typeID }, providers: device.gateway.providers,
            catalog: modelCatalogs[runtimeID]?.models, turnSettings: settings,
            conversationRuntimeId: snapshot?.conversation.runtimeID,
            modelRecordKey: editingMessageID == nil ? snapshot?.modelRecordKey : nil,
            kept: kept.choices[runtimeID], runtimeChoices: device.runtimeComposerChoices[runtimeID], newConversation: policy))
        // A historical display name is resolved by the shared native/model-route
        // rules, never fabricated into a native ID. An unresolvable recorded
        // model must not silently become a default-model edit after loading.
        if editingMessageID != nil, kept.choices[runtimeID]?.model == nil,
           settings.contains(where: { $0.runtimeId == runtimeID && ($0.model != nil || $0.modelSelection != nil) }),
           modelCatalogs[runtimeID]?.models != nil, resolved.model == nil {
            resolved.chosenModelUnavailable = true
        }
        return resolved
    }
    var keptComposerChoices: ConversationChoices {
        if isSideChat { return temporaryChoices }
        guard let raw = composerTargetChoices, let owner = device.composerInputs.ownerID else { return ConversationChoices() }
        return ConversationChoices(runtimeID: raw.runtimeId, choices: Dictionary(uniqueKeysWithValues: raw.choices.filter { $0.ownerId == owner }.map { ($0.runtimeId, $0.choice) }))
    }
    /// Where the open conversation's files are: its runtime's machine.
    var loadedConversationLocation: RuntimeTransport { device.location(of: snapshot?.conversation.runtimeID) }
    // MARK: - Status

    /// Where the loaded conversation stands, as the core reads its turns' native records.
    var conversationStatus: ConversationStatus {
        ConversationStatus(conversationID: isDraft ? nil : snapshot?.conversation.id,
                           directory: isDraft ? nil : snapshot?.conversation.cwd,
                           usage: snapshot?.usage ?? ConversationUsage())
    }
    var composerQuotaRequest: ComposerQuotaRequest { quotaRequest(next: nextTurn) }
    func quotaRequest(next: NextTurn?) -> ComposerQuotaRequest {
        ComposerQuotaRequest(runtimeID: next?.runtimeId ?? "", selection: next?.model,
            conversationID: isDraft ? nil : snapshot?.conversation.id,
            conversationRevision: snapshot?.revision, configurationRevision: device.configurationRevision,
            providerReportRevision: device.providerReportRevision)
    }
    var composerAccountQuotas: [ComposerQuotaEntry] { quotaEntries(for: composerQuotaRequest) }
    func quotaEntries(for request: ComposerQuotaRequest) -> [ComposerQuotaEntry] { composerQuotaResults.first { $0.request == request }?.entries ?? [] }
    var composerQuotaError: String? { quotaError(for: composerQuotaRequest) }
    func quotaError(for request: ComposerQuotaRequest) -> String? { composerQuotaResults.first { $0.request == request }?.failure }
    private func quotaTargetMatches(_ request: ComposerQuotaRequest) -> Bool {
        request.conversationID == (isDraft ? nil : snapshot?.conversation.id)
            && request.conversationRevision == snapshot?.revision
            && request.configurationRevision == device.configurationRevision
            && request.providerReportRevision == device.providerReportRevision
    }
    func observeComposerQuota(_ request: ComposerQuotaRequest) async {
        guard !isPreview, !isShuttingDown, !request.runtimeID.isEmpty, let transport else { return }
        while !Task.isCancelled && quotaTargetMatches(request) {
            let result: Result<[ComposerQuotaEntry], Error> = await withCheckedContinuation { continuation in
                performOnQueue { continuation.resume(returning: Result { try transport.composerQuota(request) }) }
            }
            guard !Task.isCancelled, !isShuttingDown, quotaTargetMatches(request) else { return }
            let observation: QuotaResult
            switch result {
            case .success(let entries): observation = QuotaResult(request: request, entries: entries, failure: nil)
            case .failure(let error): observation = QuotaResult(request: request, entries: [], failure: error.localizedDescription)
            }
            composerQuotaResults.removeAll { !quotaTargetMatches($0.request) || $0.request == request }
            composerQuotaResults.append(observation)
            let waiting = observation.entries.contains {
                $0.credits?.refreshing == true || $0.quota.refreshing || ($0.quota.freshness == .unknown && $0.quota.problem == nil)
            }
            try? await Task.sleep(for: .seconds(waiting ? 2 : 30))
        }
    }
    /// A turn's settings in the names the composer uses (an unknown runtime
    /// or a mode its type does not declare shows as recorded), and its usage.
    func describe(_ turn: TranscriptTurn) -> TurnSettingsSummary? {
        let settings = turn.settings ?? TurnSettings()
        let runtime = settings.runtimeId.flatMap { id in device.runtimeInstances.first { $0.id == id } }
        let type = runtime.flatMap { instance in device.runtimeTypes.first { $0.id == instance.typeId } }
        let parts: [TurnSettingsSummary.Part] = [
            settings.runtimeId.map { .init(label: "运行时", value: runtime?.name ?? $0) },
            settings.model.map { .init(label: "模型", value: $0) },
            settings.effort.map { .init(label: "推理强度", value: $0) },
            settings.permissionMode.map { mode in .init(label: "权限", value: type?.permissionModes.first { $0.id == mode }?.label ?? mode) },
        ].compactMap { $0 }
        let brief = [runtime?.name ?? settings.runtimeId, settings.model].compactMap { $0 }.joined(separator: " · ")
        let usage = turn.usage.map { usage -> [TurnSettingsSummary.Part] in
            let count = { (value: UInt64?) in value.map { TokenCount.format($0) } }
            return [
                count(usage.inputTokens).map { .init(label: "输入", value: $0) },
                count(usage.cacheReadTokens).map { .init(label: "缓存读取", value: $0) },
                count(usage.cacheWriteTokens).map { .init(label: "缓存写入", value: $0) },
                count(usage.outputTokens).map { .init(label: "输出", value: $0) },
                // Shown even when unknown: a harness may count it only within output.
                .init(label: "推理", value: count(usage.reasoningTokens) ?? "未单独记录"),
                count(usage.contextTokens).map { tokens in .init(label: "结束时上下文", value: count(usage.contextWindow).map { "\(tokens) / \($0)" } ?? tokens) },
            ].compactMap { $0 }
        } ?? []
        return parts.isEmpty && usage.isEmpty ? nil : TurnSettingsSummary(parts: parts, brief: brief, usage: usage)
    }

    // MARK: - Lifecycle

    /// An explicit destination can resolve on this tab's device immediately.
    /// Selecting a first row or creating a draft needs that device's configuration.
    func start(loadFirstConversation: Bool = true, newConversation: Bool = false, opening conversationID: String? = nil) {
        guard !started else { return }; started = true; composerTargetClosed = false
        if let target = sideChatContext, let transport, let lease = sideChatLease {
            nextTurnRuntimeID = target.runtimeID
            nextTurn = target.nextTurn
            enqueue({
                let result = try transport.openSideChat(runtimeID: target.runtimeID, conversationID: target.conversationID,
                    selection: target.selection, effort: target.effort, permissionMode: target.permissionMode)
                guard let id = result.snapshot?.conversation.id else { throw TransportError.rejected("运行时未返回侧聊") }
                lease.hold(id)
                return result
            }, onFailure: { [weak self] error in self?.openingFailure = AppProblem.failure(error, source: "打开侧聊") }) { [weak self] result in
                self?.applySnapshotResult(result)
                self?.schedulePolling()
            }
            return
        }
        device.start()
        if isPreview {
            if let conversationID { selectConversation(id: conversationID) }
            else if newConversation { createConversation() }
            return
        }
        guard transport != nil else { return }
        schedulePolling()
        if let conversationID {
            unopenedConversationID = conversationID
            if snapshot == nil { selectedConversationID = conversationID }
            openLinkedConversation(id: conversationID)
        } else if loadFirstConversation || newConversation {
            initialSelection = loadFirstConversation ? .firstOrDraft : .draft
            applyInitialSelectionIfReady()
        }
    }

    private func applyInitialSelectionIfReady() {
        guard let selection = initialSelection, device.listRevision > 0 else { return }
        initialSelection = nil
        guard started, !isShuttingDown, snapshot == nil else { return }
        if selection == .firstOrDraft, let id = sidebarConversations.first?.id {
            selectConversation(id: id)
        } else {
            createConversation()
        }
    }
    /// The tab closes: it stops following its conversation.
    func close() {
        started = false
        composerTargetClosed = true
        composerTargetRequestID = nil
        pendingComposerRuntime = nil
        deferredSelection = nil
        clearInitialOpening()
        abandonLinkedConversationLookup()
        generation += 1
        subscriptions.removeAll()
        timer?.invalidate()
        timer = nil
        sideChatLease?.release()
    }
    /// Each tab follows the conversation it shows; the device follows the rest.
    private func schedulePolling() {
        timer?.invalidate()
        timer = Timer.scheduledTimer(withTimeInterval: 0.6, repeats: true) { [weak self] _ in
            Task { @MainActor in
                if self?.composerTargetFailure != nil { self?.requestComposerTarget() }
                self?.poll()
            }
        }
    }
    func noteFocused() {
        // Device-level navigation and notifications address regular conversations.
        guard !isSideChat else { return }
        device.focusedConversation = self; noteSeen()
    }
    /// The open conversation is read once it shows in the focused tab of the
    /// active application.
    private func noteSeen() {
        guard !isSideChat, !isPreview, NSApp.isActive, device.focusedConversation === self,
              let id = loadedConversationID, pendingConversationID == nil,
              var conversation = conversation(id: id) else { return }
        if let shown = snapshot?.conversation.nativeReadState {
            guard shown.unread else { return }
            conversation.nativeReadState = shown
        } else {
            guard conversation.nativeReadState == nil, conversation.unread else { return }
        }
        device.markRead(conversation)
    }
    private func applyList(_ data: ConfigurationSnapshot) {
        device.applyList(data)
        deviceChanged()
    }
    var canReloadFromRuntimes: Bool { !isSideChat && !isPreview && !isLoading && !isShuttingDown && !isGenerating && !executionStarting && pendingConversationID == nil }
    /// Runtimes own their sessions and can change them outside Velune (another
    /// client, a turn still running there). The core follows local native files
    /// best-effort; this reads the conversation list and the open conversation
    /// again for whatever that misses.
    func reloadFromRuntimes() {
        guard canReloadFromRuntimes, let transport else { return }
        let open = isDraft ? nil : snapshot?.conversation
        generation += 1
        enqueue({ () -> (ConfigurationSnapshot, SnapshotResult?) in
            (try transport.list(), try open.map { try transport.openConversation(runtimeID: $0.runtimeID, conversationID: $0.id) })
        }) { [weak self] result in
            guard let self else { return }
            applyList(result.0)
            if let conversation = result.1 { applySnapshotResult(conversation) }
        }
    }

    // MARK: - Working directory

    /// Git changes in the loaded conversation's working directory.
    func readWorkingChanges(_ apply: @escaping (Result<WorkingChanges, Error>) -> Void) {
        guard !isPreview, !isShuttingDown, let transport, let conversation = snapshot?.conversation, !isDraft else { return }
        performOnQueue {
            let result = Result { try transport.workingChanges(runtimeID: conversation.runtimeID, conversationID: conversation.id) }
            DispatchQueue.main.async { apply(result) }
        }
    }
    /// The branch checked out in the loaded conversation's working directory;
    /// `nil` when there is none or it cannot be read (a remote runtime).
    func readWorkingBranch(_ apply: @escaping (String?) -> Void) {
        guard !isPreview, !isShuttingDown, let transport, let conversation = snapshot?.conversation, !isDraft else { apply(nil); return }
        performOnQueue {
            let branch = (try? transport.workingBranch(runtimeID: conversation.runtimeID, conversationID: conversation.id)) ?? nil
            DispatchQueue.main.async { apply(branch) }
        }
    }
    func readFileDiff(path: String, _ apply: @escaping (Result<FileDiff, Error>) -> Void) {
        guard !isPreview, !isShuttingDown, let transport, let conversation = snapshot?.conversation, !isDraft else { return }
        performOnQueue {
            let result = Result { try transport.fileDiff(runtimeID: conversation.runtimeID, conversationID: conversation.id, path: path) }
            DispatchQueue.main.async { apply(result) }
        }
    }

    // MARK: - List

    /// Organization requests belong to the device queue, independently of
    /// loading and the tab that accepted them.
    func organize(_ targets: [Conversation], _ change: ConversationOrganizationChange) {
        guard canManageConversations, !targets.isEmpty, let transport else { return }
        let references = targets.map { ConversationReference(runtimeId: $0.runtimeID, conversationId: $0.id, nativeReadState: $0.nativeReadState) }
        device.manageConversations(source: "整理会话", ids: Set(targets.map(\.id)), operation: {
            try transport.organizeConversations(references, change)
        }) { [weak device] data in
            device?.applyList(data)
            device?.rememberOrganization(targets, change)
        }
    }
    func setSearchText(_ text: String) {
        guard text != searchText else { return }
        searchText = text
        refreshSearch()
    }
    private func refreshSearch() {
        searchRequest += 1
        let request = searchRequest
        let text = searchText.trimmingCharacters(in: .whitespacesAndNewlines)
        guard !text.isEmpty, !isPreview, let transport else { searchResults = nil; return }
        performOnQueue { [weak self] in
            let result = Result { try transport.searchConversations(text) }
            DispatchQueue.main.async {
                guard let self, request == self.searchRequest else { return }
                switch result {
                case .success(let found): self.searchResults = (found.conversations.map(BindingMapping.listed), found.groups)
                case .failure(let failure): self.recordProblem(failure, source: "搜索会话")
                }
            }
        }
    }
    /// A window's list selection, for managing several conversations; the
    /// window opens a single one in a pane.
    func setListSelection(_ ids: Set<String>) {
        guard !isShuttingDown else { return }
        let selected = ids.filter { conversation(id: $0) != nil }
        if selected != selectedConversationIDs { selectedConversationIDs = selected }
    }
    func selectConversations(ids: Set<String>) {
        guard !isLoading, !isShuttingDown, !device.authenticationRunning else { return }
        selectedConversationIDs = ids.filter { conversation(id: $0) != nil }
        if selectedConversationIDs.count == 1, let id = selectedConversationIDs.first, id != loadedConversationID { selectConversation(id: id) }
    }

    // MARK: - Navigation

    private func clearInitialOpening() {
        initialSelection = nil
        unopenedConversationID = nil
        openingNeedsRetry = false
        openingRetry.succeeded()
        openingFailure = nil
    }
    /// Only temporary availability failures can recover without user action.
    /// History/invalid/closed errors keep their diagnostic and do not spin.
    private func noteOpeningFailure(_ error: Error) {
        guard unopenedConversationID != nil else { return }
        var cause = error
        while let transport = cause as? TransportError, case .operation(_, let inner) = transport { cause = inner }
        let retryable: Bool
        switch cause as? TransportError {
        case .unavailable?: retryable = true
        case .diagnostic(let kind, _, _, _, _)?: retryable = kind == "unavailable" || kind == "connection"
        default: retryable = false
        }
        openingNeedsRetry = retryable
        if retryable { openingRetry.failed(); openingFailure = nil }
        else { openingFailure = AppProblem.failure(error, source: "打开会话") }
    }
    var canRetryFailedOpening: Bool {
        !isSideChat && !isLoading && !isShuttingDown && openingFailure != nil && unopenedConversationID != nil
    }
    var coldOpeningProblem: AppProblem? {
        loadedConversationID == nil && !isOpeningConversation ? openingFailure : nil
    }
    func retryFailedOpening() {
        guard canRetryFailedOpening, let id = unopenedConversationID else { return }
        openLinkedConversation(id: id)
    }
    private func takeOpeningRetry() -> String? {
        guard started, !isShuttingDown, openingNeedsRetry, openingRetry.ready,
              snapshot == nil, !isLoading, !executionStarting,
              linkedConversationRequest == nil, deferredSelection == nil,
              let id = unopenedConversationID else { return nil }
        openingNeedsRetry = false
        return id
    }
    private func retryOpeningIfNeeded() {
        guard let id = takeOpeningRetry() else { return }
        openLinkedConversation(id: id)
    }

    /// How opening a conversation moves through the conversations shown before.
    enum NavigationMove { case new, back, forward }
    /// Conversations shown before the current one, oldest first, and those
    /// gone back from, nearest last.
    @Published private(set) var backHistory: [String] = []
    @Published private(set) var forwardHistory: [String] = []
    private static let historyLimit = 50
    func goBack() { if let id = backHistory.last { openLinkedConversation(id: id, move: .back) } }
    func goForward() { if let id = forwardHistory.last { openLinkedConversation(id: id, move: .forward) } }
    private func recordNavigation(from previous: String, _ move: NavigationMove) {
        switch move {
        case .new: backHistory.append(previous); forwardHistory.removeAll()
        case .back: if !backHistory.isEmpty { backHistory.removeLast() }; forwardHistory.append(previous)
        case .forward: if !forwardHistory.isEmpty { forwardHistory.removeLast() }; backHistory.append(previous)
        }
        if backHistory.count > Self.historyLimit { backHistory.removeFirst(backHistory.count - Self.historyLimit) }
    }
    /// Open a conversation a link names, such as a sub-agent's, or one gone
    /// back to; an agent may have started it after the list was last read.
    func openLinkedConversation(id: String, move: NavigationMove = .new) {
        if unopenedConversationID != id { clearInitialOpening() }
        if conversation(id: id) != nil { selectConversation(id: id, move: move); return }
        guard !isPreview, !isShuttingDown, let transport else { abandonLinkedConversationLookup(); return }
        // Native links already carry the source instance. The history reader
        // validates membership, without rebuilding that instance's list.
        if let separator = id.firstIndex(of: ":"), id.index(after: separator) != id.endIndex {
            let runtimeID = String(id[..<separator])
            if device.enabledRuntimeInstances.contains(where: {
                $0.id == runtimeID && [.claudeCode, .codex166, .pi].contains($0.typeId)
            }) {
                openConversation(id: id, runtimeID: runtimeID, move: move)
                return
            }
        }
        if snapshot == nil { unopenedConversationID = id }
        let request = UUID()
        linkedConversationRequest = request
        deferredSelection = nil
        reachingConversationID = id
        performOnQueue { [weak self] in
            let result = Result { try transport.findConversation(id) }
            DispatchQueue.main.async {
                self?.finishLinkedConversationLookup(result, id: id, move: move, request: request)
            }
        }
    }
    private func abandonLinkedConversationLookup() { linkedConversationRequest = nil; reachingConversationID = nil }
    private func finishLinkedConversationLookup(_ result: Result<ListedConversation?, Error>, id: String, move: NavigationMove, request: UUID) {
        guard linkedConversationRequest == request else { return }
        abandonLinkedConversationLookup()
        switch result {
        case .success(let found?):
            let reached = BindingMapping.listed(found)
            offListConversations.removeAll { $0.id == reached.id }
            offListConversations.append(reached)
            selectConversation(id: id, move: move)
        case .success(nil):
            clearInitialOpening()
            let error = TransportError.rejected("找不到这个会话，它可能已被删除或其运行时已停用")
            openingFailure = AppProblem.failure(error, source: "打开会话")
            recordProblem(error, source: "打开会话")
        case .failure(let failure):
            noteOpeningFailure(failure)
            recordProblem(failure, source: "打开会话")
        }
    }
    func selectConversation(id: String, move: NavigationMove = .new) {
        guard let conversation = conversation(id: id) else {
            if unopenedConversationID != id { clearInitialOpening() }
            abandonLinkedConversationLookup()
            return
        }
        openConversation(id: id, runtimeID: conversation.runtimeID, move: move)
    }
    private func openConversation(id: String, runtimeID: String, move: NavigationMove) {
        if unopenedConversationID != id { clearInitialOpening() }
        abandonLinkedConversationLookup()
        // Chosen while another operation runs, it opens once that is done; the latest choice wins.
        guard !isLoading, !executionStarting else { deferredSelection = (id, move); return }
        deferredSelection = nil
        selectedConversationIDs = [id]
        if isPreview { projectionRuntimeID = runtimeID; apply(device.previewSnapshots[id]); composerEvents.send(.conversation(id)); return }
        guard let transport else { recordProblem("本地核心未配置"); return }
        if snapshot == nil { unopenedConversationID = id }
        // The sidebar reflects user intent immediately. The old loaded transcript
        // is retained until success, but hidden while this destination is loading.
        let previousSelection = snapshot?.conversation.id
        let previousWasDraft = isDraft
        // An unsent conversation is not one to go back to.
        let shown = isDraft ? nil : previousSelection
        isDraft = false
        generation += 1 // Invalidate any poll already queued for the old conversation.
        selectedConversationID = id
        pendingConversationID = id
        composerEvents.send(.conversation(id))
        openingFailure = nil
        enqueue({ try transport.openConversation(runtimeID: runtimeID, conversationID: id) }, onFailure: { [weak self] error in
            guard let self else { return }
            self.noteOpeningFailure(error)
            self.isDraft = previousWasDraft
            self.selectedConversationID = self.unopenedConversationID ?? previousSelection
            self.selectedConversationIDs = Set([self.selectedConversationID].compactMap { $0 })
            if let previousSelection { self.composerEvents.send(previousWasDraft ? .newConversation : .conversation(previousSelection)) }
            self.finishConversationLoading()
        }) { [weak self] in
            if let shown, shown != id { self?.recordNavigation(from: shown, move) }
            self?.applySnapshotResult($0)
            self?.finishConversationLoading()
        }
    }
    private func selectDeferredConversation() {
        guard let (id, move) = deferredSelection, !isLoading, !executionStarting else { return }
        deferredSelection = nil
        openLinkedConversation(id: id, move: move)
    }
    private func finishConversationLoading() {
        pendingConversationID = nil
        if !isShuttingDown, deferredSelection == nil { noteSeen(); requestComposerTarget(refreshFacts: true) }
    }

    // MARK: - Managing conversations

    func canRenameConversation(_ conversation: Conversation) -> Bool {
        conversation.canRename
    }
    func canDeleteConversation(_ conversation: Conversation) -> Bool {
        conversation.canDelete && !isRunning(conversation)
    }
    func renameConversation(_ conversation: Conversation, title: String, onRenamed: @escaping () -> Void) {
        guard canManageConversations else { recordProblem("请等待正在进行的操作结束，再修改会话"); return }
        guard canRenameConversation(conversation) else { recordProblem("此会话的原生标题当前无法修改"); return }
        let name = title.trimmingCharacters(in: .whitespacesAndNewlines)
        guard !name.isEmpty else { recordProblem("会话名称不能为空"); return }
        if isPreview {
            device.renameInPreview(conversation.id, title: name)
            if snapshot?.conversation.id == conversation.id { snapshot?.conversation.title = name }
            onRenamed(); return
        }
        guard let transport else { recordProblem("本地核心未配置"); return }
        device.manageConversations(source: "重命名“\(conversation.title)”", ids: [conversation.id], operation: {
            try transport.renameConversation(runtimeID: conversation.runtimeID, conversationID: conversation.id, title: name)
        }) { [weak device] data in
            device?.applyList(data)
            device?.noteRenamed(conversation.id, title: name)
        }
        onRenamed()
    }
    func deleteConversation(_ conversation: Conversation) { deleteConversations([conversation]) }
    func deleteConversations(_ targets: [Conversation]) {
        guard canManageConversations else { recordProblem("请等待正在进行的操作结束，再修改会话"); return }
        guard !targets.isEmpty, targets.allSatisfy(canDeleteConversation) else { recordProblem("所选会话中有原生会话当前无法删除"); return }
        if isPreview {
            let ids = Set(targets.map(\.id))
            selectedConversationIDs.subtract(ids)
            device.deleteInPreview(ids)
            if let loadedConversationID, ids.contains(loadedConversationID) { resetProjection() }
            return
        }
        guard let transport else { recordProblem("本地核心未配置"); return }
        let loadedID = loadedConversationID
        let contextID = snapshot?.contextRuntimeID
        // Runtime-owned deletions are independent operations. Preserve each
        // success and report failures rather than promising a transaction.
        device.manageConversations(source: "删除会话", ids: Set(targets.map(\.id)), deleting: true, operation: {
            var latest: ConfigurationSnapshot?
            var deleted: Set<String> = []
            var failures: [AppProblem] = []
            var refreshed: SnapshotResult?
            var lostContext = false
            for target in targets {
                do { latest = try transport.deleteConversation(runtimeID: target.runtimeID, conversationID: target.id); deleted.insert(target.id) }
                catch { failures.append(AppProblem.failure(error, source: "删除会话 · \(target.title)")) }
            }
            if !failures.isEmpty {
                do { latest = try transport.list() }
                catch { failures.append(AppProblem.failure(error, source: "刷新会话列表")) }
                if let contextID, !deleted.contains(loadedID ?? "") {
                    do { refreshed = try transport.snapshot(runtimeID: contextID, conversationID: loadedID ?? "") }
                    catch { lostContext = true; failures.append(AppProblem.failure(error, source: "读取当前会话")) }
                }
            }
            return (latest, deleted, failures, refreshed, lostContext)
        }) { [weak self, weak device] result in
            device?.noteDeleted(result.1)
            if let data = result.0 { device?.applyList(data) }
            device?.appendProblems(result.2)
            guard let self else { return }
            selectedConversationIDs.subtract(result.1)
            if let loadedConversationID, result.1.contains(loadedConversationID) { resetProjection() }
            else if loadedConversationID == loadedID {
                if result.4 { resetProjection() }
                else if let refreshed = result.3 { applySnapshotResult(refreshed) }
            }
        }
    }

    // MARK: - Composing

    /// Opens an unsent conversation: an empty transcript and composer. The
    /// runtime and model start from the current next-turn choice and can be
    /// changed in the composer; the conversation is created at the first send.
    @discardableResult
    func createConversation(runtimeID requestedRuntimeID: String? = nil, cwd: String? = nil, project: NewConversationProject? = nil) -> Bool {
        guard !isSideChat, !isLoading, !executionStarting else { return false }
        let enabled = device.enabledRuntimeInstances
        guard let runtimeID = requestedRuntimeID ?? enabled.first(where: { $0.id == nextTurnRuntimeID })?.id ?? enabled.first?.id,
              enabled.contains(where: { $0.id == runtimeID }) else { return false }
        clearInitialOpening()
        abandonLinkedConversationLookup()
        let id = unsentContextID
        let conversation = Conversation(id: id, title: "新会话", updatedAtUnixMs: isPreview ? Int64(Date().timeIntervalSince1970 * 1000) : nil, runtimeID: runtimeID, cwd: project?.directories.first ?? cwd, creationProject: project, canRename: isPreview, canDelete: isPreview)
        creationFailure = nil
        isDraft = !isPreview
        projectionRuntimeID = runtimeID
        apply(ConversationSnapshot(revision: 1, conversation: conversation, contextRuntimeID: runtimeID, modelRecordKey: nil, runState: .idle, messages: [], actions: ConversationActions(inputAccess: .editable, canSend: true, canCancel: false, canSwitch: true)))
        composerEvents.send(.newConversation)
        return true
    }
    /// A restored tab names its conversation before reading starts on first
    /// show: the tab's title and the window's already-open checks work
    /// meanwhile, following the device's list once it is read.
    func noteRestoredConversation(_ id: String) {
        // Preview initialization selects its sample before restoration runs.
        // A restored background tab still owns the saved destination.
        guard !started, selectedConversationID == nil || isPreview else { return }
        selectedConversationID = id
    }
    /// An unsent conversation can still choose its working directory.
    var isUnsentConversation: Bool { isDraft || loadedConversationID?.hasPrefix(Self.draftConversationPrefix) == true }
    var unsentConversation: Conversation? { isUnsentConversation ? snapshot?.conversation : nil }
    func restoreUnsentConversation(_ saved: Conversation) {
        guard !started else { return }
        restoresLegacyUnsentInput = saved.id == Self.draftConversationPrefix + "new-conversation"
        if restoresLegacyUnsentInput { device.composerInputs.restoreLegacyTarget(to: unsentContextID) }
        let conversation = Conversation(id: unsentContextID, title: saved.title, updatedAtUnixMs: saved.updatedAtUnixMs, createdAtUnixMs: saved.createdAtUnixMs, runtimeID: saved.runtimeID, cwd: saved.cwd, creationProject: saved.creationProject, canRename: saved.canRename, canDelete: saved.canDelete)
        started = true
        device.start()
        isDraft = true
        projectionRuntimeID = conversation.runtimeID
        nextTurnRuntimeID = conversation.runtimeID
        apply(ConversationSnapshot(revision: 1, conversation: conversation, contextRuntimeID: conversation.runtimeID, modelRecordKey: nil, runState: .idle, messages: [], actions: ConversationActions(inputAccess: .editable, canSend: true, canCancel: false, canSwitch: true)))
        if !isPreview { schedulePolling() }
    }
    /// The conversation a side chat opens on, with the next turn's choices it
    /// asks with: one sent, not a sub-agent's, of a runtime that has side
    /// chats and that the next turn stays on (see 侧聊 in the PRD).
    var sideChatTarget: SideChatTarget? { sideChatTarget(next: nextTurn) }
    func sideChatTarget(next: NextTurn?) -> SideChatTarget? {
        guard !isSideChat, !isPreview, !isDraft, let nextTurn = next, modelCatalogs[nextTurn.runtimeId] != nil, modelCatalogs[nextTurn.runtimeId] != .loading, let snapshot, snapshot.conversation.parentConversationID == nil,
              snapshot.messages.contains(where: { $0.role == .user }),
              nextTurn.runtimeId == snapshot.contextRuntimeID, device.hasSideChats(snapshot.contextRuntimeID) else { return nil }
        return SideChatTarget(runtimeID: snapshot.contextRuntimeID, conversationID: snapshot.conversation.id,
                              title: selectedConversationTitle ?? "会话", nextTurn: nextTurn)
    }
    func setUnsentWorkingDirectory(_ path: String?) {
        guard isDraft, snapshot != nil else { return }
        creationFailure = nil
        snapshot?.conversation.cwd = path
        snapshot?.conversation.creationProject = nil
        composerEvents.send(.creationTargetChanged)
    }
    func setUnsentProject(_ project: NewConversationProject) {
        guard isDraft, snapshot != nil else { return }
        creationFailure = nil
        snapshot?.conversation.cwd = project.directories.first
        snapshot?.conversation.creationProject = project
        composerEvents.send(.creationTargetChanged)
    }
    /// Where pasted images are kept so they can be attached by path.
    var attachmentsDirectory: URL {
        (transport?.stateDirectory ?? FileManager.default.temporaryDirectory).appendingPathComponent("attachments", isDirectory: true)
    }
    /// Returns whether the turn was sent; the message shows at once, marked as
    /// sending until the runtime accepts it. `onAccepted` receives the
    /// conversation that took the turn: sending in a new conversation replaces
    /// its unsent identity with the created one. `onFailure` runs when it was not sent.
    @discardableResult
    func send(text: String, attachments: [String] = [], next: NextTurn? = nil, input: ComposerModel? = nil, onAccepted: ((String?) -> Void)? = nil, onFailure: (() -> Void)? = nil) -> Bool {
        guard !text.trimmingCharacters(in: .whitespacesAndNewlines).isEmpty || !attachments.isEmpty else { return false }
        guard allowsAttachments || attachments.isEmpty else { return false }
        guard canSend, let next = next ?? nextTurn, case let target = next.runtimeId, let source = snapshot?.conversation.runtimeID, let conversationID = loadedConversationID else { return false }
        if let project = snapshot?.conversation.creationProject, project.runtimeID != target {
            recordProblem("所选项目属于另一运行时，请重新选择项目或工作目录")
            return false
        }
        let model = next.model
        if isPreview {
            guard var value = snapshot else { return false }
            if case .managed(let key)? = model { value.modelRecordKey = key }
            let message = DeliveredMessage(id: UUID().uuidString, role: .user, blocks: [.text(text: text)])
            value.messages.append(message)
            value.transcriptItems.append(.message(id: message.id, messageId: message.id))
            value.transcriptOutline.append(TranscriptOutlineEntry(id: message.id, messageId: message.id))
            value.transcriptMessageIdentities.append(TranscriptMessageIdentity(id: message.id, messageId: message.id))
            apply(value); onAccepted?(loadedConversationID); return true
        }
        guard let transport else { recordProblem("本地核心未配置"); return false }
        creationFailure = nil
        executionStarting = true
        composerRequest = .sending
        let submission = outgoing.begin(.send, text: text, attachments: attachments, snapshot: snapshot)
        let requestGeneration = generation
        let creating = isDraft
        let temporary = isSideChat
        let cwd = snapshot?.conversation.cwd
        let project = snapshot?.conversation.creationProject
        // Without a level or mode the core keeps the conversation's current one.
        let turn = ComposedTurn(text: text, effort: next.effort, permissionMode: next.permissionMode?.id, attachments: attachments)
        enqueue({
            if temporary { return try transport.sendSideChat(sideChatID: conversationID, runtimeID: target, selection: model, turn: turn) }
            if creating {
                return try transport.createAndSend(runtimeID: target, cwd: cwd, project: project?.key, selection: model, turn: turn, submissionId: submission) { [weak self] created in
                    DispatchQueue.main.async {
                        guard let self, self.generation == requestGeneration, !self.composerTargetClosed else { return }
                        self.isDraft = false
                        // Text typed while sending and the next-turn choice follow the created conversation.
                        if let id = created.snapshot?.conversation.id {
                            if let input { self.composerEvents.send(.created(previous: conversationID, id: id, runtimeID: created.snapshot!.conversation.runtimeId, input: input)) }
                        }
                        self.applySnapshotResult(created)
                    }
                }
            }
            return try transport.sendTurn(sourceRuntimeID: source, conversationID: conversationID, targetRuntimeID: target, selection: model, turn: turn, submissionId: submission)
        }, consumeReceipt: { [self] result in
            executionStarting = false
            composerRequest = nil
            switch result {
            case .success(let value):
                acceptSubmission(submission, receipt: value)
                acceptedSubmissions &+= 1
                onAccepted?(value.snapshot?.conversation.id ?? conversationID)
            case .failure(let error):
                if AppProblem.failure(error).code == "message_delivery_accepted" {
                    outgoing.accept(submission, snapshot: nil)
                    acceptedSubmissions &+= 1
                    onAccepted?(conversationID)
                } else if outgoing.fail(submission) {
                    if creating && isDraft { creationFailure = AppProblem.failure(error).detail }
                    onFailure?()
                } else {
                    acceptedSubmissions &+= 1; onAccepted?(conversationID)
                }
            }
        }) { [weak self] value in self?.applySnapshotResult(value) }
        return true
    }
    private func acceptSubmission(_ id: String, receipt: SnapshotResult) {
        outgoing.accept(id, receipt: receipt.submissionReceipt, snapshot: receipt.snapshot.map(BindingMapping.snapshot))
        if case .queued = receipt.submissionReceipt {} else { outgoing.reconcile(snapshot) }
    }
    /// Stop the sub-agents the loaded conversation started; they may run after its turn.
    func stopSubAgents() {
        guard followUp.canStopSubAgents, !isPreview else { return }
        guard let runtimeID = projectionRuntimeID, let conversationID = loadedConversationID else { recordProblem("请选择会话"); return }
        guard let transport else { recordProblem("本地核心未配置"); return }
        enqueue({ try transport.stopSubAgents(runtimeID: runtimeID, conversationID: conversationID) }) { [weak self] in self?.applySnapshotResult($0) }
    }
    func setConversationHandoff(release: Bool) {
        let handoff = controlHandoff
        guard (release ? handoff == .canRelease : handoff == .releasing || handoff == .released),
              !isLoading, !isPreview,
              let transport, let runtimeID = projectionRuntimeID, let conversationID = loadedConversationID else { return }
        enqueue({ try transport.setConversationHandoff(runtimeID: runtimeID, conversationID: conversationID, release: release) }) { [weak self] in
            self?.applySnapshotResult($0)
        }
    }
    func cancel() {
        guard canCancel, !isLoading else { return }
        if isPreview {
            snapshot?.runState = .idle; snapshot?.actions.canCancel = false
            if var value = snapshot {
                for index in value.messages.indices {
                    value.messages[index].blocks = value.messages[index].blocks.map { block in
                        if case let .runtimeFailure(failure, detail, retry) = block, retry != nil {
                            return .runtimeFailure(failure: failure, detail: detail, retry: .cancelled)
                        }
                        return block
                    }
                }
                apply(value)
            }
            return
        }
        guard let runtimeID = projectionRuntimeID, let conversationID = loadedConversationID else { recordProblem("请选择运行时实例"); return }
        guard let transport else { recordProblem("本地核心未配置"); return }
        stopRequested = true
        enqueue({ try transport.cancel(runtimeID: runtimeID, conversationID: conversationID) }, onFailure: { [weak self] _ in self?.stopRequested = false }) { [weak self] in self?.applySnapshotResult($0) }
    }
    /// Queue a message after the running turn, with the next-turn choices as
    /// they are now. `onFailure` runs when it was not queued.
    @discardableResult
    func queue(text: String, attachments: [String] = [], next: NextTurn? = nil, onAccepted: (() -> Void)? = nil, onFailure: (() -> Void)? = nil) -> Bool {
        guard !text.trimmingCharacters(in: .whitespacesAndNewlines).isEmpty || !attachments.isEmpty else { return false }
        guard canQueue, let next = next ?? nextTurn, case let target = next.runtimeId, let source = snapshot?.conversation.runtimeID else { return false }
        let turn = ComposedTurn(text: text, effort: next.effort, permissionMode: next.permissionMode?.id, attachments: attachments)
        let submission = outgoing.begin(.queue, text: text, attachments: attachments, snapshot: snapshot)
        composerRequest = .queueing
        changeQueue(.add(sourceRuntimeId: source, targetRuntimeId: target, selection: next.model, turn: turn), submissionId: submission, onFailure: { [self] in
            composerRequest = nil
            if outgoing.fail(submission) { onFailure?() }
            else { acceptedSubmissions &+= 1; onAccepted?() }
        }) { [self] receipt in
            composerRequest = nil
            acceptSubmission(submission, receipt: receipt)
            acceptedSubmissions &+= 1; onAccepted?()
        }
        return true
    }
    func compactContext(next: NextTurn? = nil) {
        guard canCompact(next: next ?? nextTurn), let runtimeID = snapshot?.contextRuntimeID, let conversationID = loadedConversationID, let transport else { return }
        let selection = (next ?? nextTurn)?.model
        executionStarting = true
        enqueue({ try transport.compactConversation(runtimeID: runtimeID, conversationID: conversationID, selection: selection) }, onFailure: { [weak self] _ in self?.executionStarting = false }) { [weak self] value in
            self?.executionStarting = false
            self?.applySnapshotResult(value)
        }
    }
    func continueTurn(next: NextTurn? = nil) {
        guard canResume(next: next ?? nextTurn) else { return }
        guard let runtimeID = snapshot?.contextRuntimeID, let conversationID = loadedConversationID, let transport else { return }
        let selection = (next ?? nextTurn)?.model
        let effort = (next ?? nextTurn)?.effort
        let mode = (next ?? nextTurn)?.permissionMode?.id
        executionStarting = true
        composerRequest = .resuming
        enqueue({ try transport.resumeTurn(runtimeID: runtimeID, conversationID: conversationID, selection: selection, effort: effort, permissionMode: mode) },
                onFailure: { [weak self] _ in self?.executionStarting = false; self?.composerRequest = nil }) { [weak self] value in
            self?.executionStarting = false
            self?.composerRequest = nil
            self?.acceptedSubmissions &+= 1
            self?.applySnapshotResult(value)
        }
    }
    var canSteer: Bool { canQueue && followUp.canSteer }
    /// Into the running turn; the turn keeps its own choices.
    func steer(text: String, attachments: [String] = [], onFailure: (() -> Void)? = nil) {
        guard canSteer, let runtimeID = projectionRuntimeID, let conversationID = loadedConversationID, let transport else { onFailure?(); return }
        let turn = ComposedTurn(text: text, effort: nil, permissionMode: nil, attachments: attachments)
        let submission = outgoing.begin(.steer, text: text, attachments: attachments, snapshot: snapshot)
        enqueue({ try transport.steerTurn(runtimeID: runtimeID, conversationID: conversationID, turn: turn) }, onFailure: { [weak self] _ in self?.outgoing.fail(submission); onFailure?() }) { [weak self] in
            self?.outgoing.accept(submission, snapshot: $0.snapshot.map(BindingMapping.snapshot))
            self?.acceptedSubmissions &+= 1
            self?.applySnapshotResult($0)
        }
    }
    /// Whether the user message may be edited now, and how. The edit goes to
    /// the conversation's own runtime with the next turn's choices.
    func editability(of messageID: String) -> EditableMessage? {
        guard !isPreview, !isDraft else { return nil }
        return followUp.editable.first { $0.messageId == messageID }
    }
    func message(id: String) -> DeliveredMessage? { snapshot?.messages.first { $0.id == id } }
    var runtimeRecovery: RuntimeRecovery? {
        guard let snapshot, let recovery = RuntimeRecovery.latest(in: snapshot.messages) else { return nil }
        // A stored earlier failure is history, not a current recovery notice,
        // once this user turn has completed. Live typed results remain visible.
        if recovery.retry == nil, let user = snapshot.messages.last(where: { $0.role == .user }),
           snapshot.transcriptTurns.last(where: { $0.userMessageId == user.id })?.outcome == .completed { return nil }
        return recovery
    }
    /// Whether a turn's closing reply may fork the conversation now.
    var canFork: Bool { !isPreview && !isDraft && !isForking && !isLoading && !isShuttingDown }
    /// Copying is independent of whether the source has a running turn.
    @Published private(set) var isForking = false
    func messageEditingChoices(_ messageID: String) -> ConversationChoices {
        var choices = ConversationChoices()
        let settings = snapshot?.transcriptTurns.first { $0.userMessageId == messageID }?.settings
        let runtime = snapshot?.contextRuntimeID ?? projectionRuntimeID
        choices.runtimeID = runtime
        if let runtime, let settings, settings.runtimeId == runtime {
            // Legacy `model` is a recorded label, not necessarily a native ID.
            // The resolver maps it using this message's settings and catalog.
            choices.choices[runtime] = TurnChoice(model: settings.modelSelection, effort: settings.effort, permissionMode: settings.permissionMode)
        }
        return choices
    }
    func messageEditingRecordedModel(_ messageID: String) -> String? {
        snapshot?.transcriptTurns.first { $0.userMessageId == messageID }?.settings?.model
    }
    /// Core may discover a native writer only when checking the edit CAS.
    /// Re-read that source once after rejection; never advance the editor's
    /// confirmed baseline or retry its mutation as part of this read.
    func refreshMessageEditingState(_ messageID: String, reloaded: @escaping (String?, String?) -> Void) {
        guard !isPreview, !isLoading, !isShuttingDown, let runtime = snapshot?.conversation.runtimeID,
              let conversation = loadedConversationID, let transport else {
            reloaded("原会话暂时无法重新读取，编辑副本仍保留。", nil)
            return
        }
        enqueue({ try transport.refreshConversation(runtimeID: runtime, conversationID: conversation) },
                consumeReceipt: { result in
            switch result {
            case .success(let value):
                let revision = value.snapshot?.followUp.editable.first { $0.messageId == messageID }?.revision
                // A poll may supersede this projection. Its read receipt still
                // belongs to the edit; process it after the apply attempt.
                DispatchQueue.main.async { reloaded(nil, revision) }
            case .failure(let error): reloaded(AppProblem.failure(error).detail, nil)
            }
        }) { [weak self] value in
            self?.applySnapshotResult(value)
        }
    }
    func restoreMessageEditingChoices(_ kept: ConversationChoices) -> Bool {
        guard let runtimeID = kept.runtimeID, let targetID = loadedConversationID,
              composerTargetIssue == nil,
              let owner = device.composerInputs.ownerID,
              device.enabledRuntimeInstances.contains(where: { $0.id == runtimeID }) else { return false }
        var raw = composerTargetChoices ?? ComposerTargetChoices(runtimeId: nil, runtimeOwnerId: nil, choices: [], choiceEdited: false)
        raw.runtimeId = runtimeID; raw.runtimeOwnerId = owner; raw.choiceEdited = true
        raw.choices.removeAll { $0.ownerId == owner && $0.runtimeId == runtimeID }
        raw.choices.append(.init(ownerId: owner, runtimeId: runtimeID, choice: kept.choices[runtimeID] ?? TurnChoice()))
        device.composerInputs.saveTarget(targetID, choices: raw, userEdit: true)
        refreshNextTurn()
        return true
    }
    /// Build only a target update. Ordinary input content and attachments never
    /// participate in accepting a historical edit, including a fork.
    func messageEditingTargetChoices(_ kept: ConversationChoices, base: ComposerTargetChoices?) -> ComposerTargetChoices? {
        guard let runtime = kept.runtimeID, let owner = device.composerInputs.ownerID else { return nil }
        var raw = base ?? ComposerTargetChoices(runtimeId: nil, runtimeOwnerId: nil, choices: [], choiceEdited: false)
        raw.runtimeId = runtime; raw.runtimeOwnerId = owner; raw.choiceEdited = true
        raw.choices.removeAll { $0.ownerId == owner && $0.runtimeId == runtime }
        raw.choices.append(.init(ownerId: owner, runtimeId: runtime, choice: kept.choices[runtime] ?? TurnChoice()))
        return raw
    }
    var messageEditingRuntimeID: String? { snapshot?.contextRuntimeID }
    var messageEditingRuntimeName: String? { snapshot.flatMap { value in device.runtimeInstances.first { $0.id == value.contextRuntimeID }?.name } }
    func isLastUserMessage(_ messageID: String) -> Bool { snapshot?.messages.last { $0.role == .user }?.id == messageID }
    /// Send `text` in place of a user message. In place the conversation
    /// shows the new turn here; a fork's new conversation goes to `forked`.
    /// A fork runs independently; only an in-place replacement stops the
    /// source turn, as the user confirmed in the editor.
    func editMessage(_ messageID: String, placement: EditPlacement, text: String, attachments: [String], choices: ConversationChoices? = nil, expectedRevision: String? = nil, forked: @escaping (SnapshotResult) -> Void, accepted: @escaping () -> Void, failed: ((String) -> Void)? = nil) {
        guard editability(of: messageID) != nil, let runtimeID = snapshot?.conversation.runtimeID, let conversationID = loadedConversationID else { return }
        let kept = choices ?? messageEditingChoices(messageID)
        let editNext = resolve(kept: kept, editingMessageID: messageID)
        let model = editNext?.model
        guard let transport else { recordProblem("本地核心未配置"); return }
        guard !isLoading, !isForking, !isShuttingDown else { failed?("请等待当前操作完成后重试"); return }
        let inputs = device.composerInputs
        guard let targetChoices = messageEditingTargetChoices(kept, base: placement == .inPlace ? composerTargetChoices : nil) else {
            failed?("执行配置尚未就绪，请稍后重试"); return
        }
        executionStarting = true
        isForking = placement == .fork
        // The editor's private choices become the continuing conversation's
        // next-turn intent only after its replacement has actually been sent.
        let targetRevision: UInt64?
        if placement != .inPlace { targetRevision = nil }
        else if let pending = inputs.pendingTarget(for: conversationID) { targetRevision = pending.revision }
        else { targetRevision = inputs.targetRecord(for: conversationID)?.revision }
        let turn = ComposedTurn(text: text, effort: editNext?.effort, permissionMode: editNext?.permissionMode?.id, attachments: attachments)
        enqueue({
            let result = try transport.editMessage(runtimeID: runtimeID, conversationID: conversationID, messageID: messageID, placement: placement, targetRuntimeID: editNext?.runtimeId, selection: model, turn: turn, expectedRevision: expectedRevision)
            if let snapshot = result.snapshot {
                // The accepted target survives closing the editing surface or
                // source tab before its UI completion callback is delivered.
                let resultID = snapshot.conversation.id
                DispatchQueue.main.async { inputs.saveEditedTarget(resultID, choices: targetChoices, expectedRevision: targetRevision) }
            }
            return result
        }, consumeReceipt: { [weak self] result in
            self?.executionStarting = false
            self?.isForking = false
            switch result {
            case .success:
                if placement == .fork { self?.device.noteForkCompleted() }
                accepted()
            case .failure(let error): failed?(AppProblem.failure(error).detail)
            }
        }) { [weak self] value in
            guard let self else { return }
            if placement == .fork { forked(value) } else { self.applySnapshotResult(value) }
        }
    }
    /// Copy the completed prefix immediately, even while the source runs.
    /// The core creates a separate native execution and leaves this one alone.
    func forkConversation(through messageID: String, forked: @escaping (SnapshotResult) -> Void) {
        guard canFork, let runtimeID = snapshot?.conversation.runtimeID, let conversationID = loadedConversationID, let transport else { return }
        isForking = true
        enqueue({ try transport.forkConversation(runtimeID: runtimeID, conversationID: conversationID, messageID: messageID) },
                onFailure: { [weak self] _ in self?.isForking = false }) { [weak self] value in
            guard let self else { return }
            self.isForking = false
            self.device.noteForkCompleted()
            forked(value)
        }
    }
    /// Show a conversation the core has already opened, as `result` gives it.
    func adopt(_ result: SnapshotResult) { applySnapshotResult(result) }
    /// A fork is already open on the execution owner. Do not put a full
    /// runtime-list scan ahead of preparing this conversation's composer.
    func start(adopting result: SnapshotResult) {
        guard !started else { adopt(result); return }
        started = true
        composerTargetClosed = false
        device.start()
        adopt(result)
        schedulePolling()
    }
    func followComposerCreation(_ creation: ComposerCreation) {
        guard isDraft, let transport else { return }
        isDraft = false; snapshot = nil
        projectionRuntimeID = creation.runtimeId
        selectedConversationID = creation.conversationId
        pendingConversationID = creation.conversationId
        enqueue({ try transport.openConversation(runtimeID: creation.runtimeId, conversationID: creation.conversationId) }) { [weak self] result in
            self?.applySnapshotResult(result); self?.finishConversationLoading()
        }
    }
    func steerQueued(_ id: String) {
        guard let message = followUp.queue.first(where: { $0.id == id }) else { return }
        let submission = outgoing.begin(.steer, text: message.text, attachments: message.attachments, snapshot: snapshot)
        changeQueue(.steer(id: id), onFailure: { [weak self] in self?.outgoing.fail(submission) }) { [weak self] receipt in
            self?.acceptSubmission(submission, receipt: receipt)
        }
    }
    func replaceQueued(_ operation: SendQueueOperation, conversationID: String, completion: @escaping (String?) -> Void) {
        guard selectedConversationID == nil || selectedConversationID == conversationID,
              loadedConversationID == conversationID, let runtimeID = projectionRuntimeID else {
            completion("已切换会话，当前修改仍保留。"); return
        }
        guard !isLoading, !isShuttingDown else { completion("请等待当前操作完成后重试。"); return }
        guard !isPreview, let transport else { completion("执行端未连接，当前修改仍保留。"); return }
        enqueueSnapshot({
            try transport.sendQueue(runtimeID: runtimeID, conversationID: conversationID, operation: operation)
        }, consumeReceipt: { result in
            switch result {
            case .success: completion(nil)
            case .failure(let error):
                let problem = AppProblem.failure(error, source: "编辑排队消息")
                completion([problem.detail, problem.operationID.map { "操作编号 \($0)" }].compactMap { $0 }.joined(separator: "\n"))
            }
        })
    }
    func removeQueued(_ id: String) { changeQueue(.remove(id: id)) }
    func moveQueued(_ id: String, to index: Int) { changeQueue(.move(id: id, index: UInt32(max(index, 0)))) }
    func resumeQueue() { guard inputSubmissionAllowed else { return }; changeQueue(.resume) }
    private func changeQueue(_ operation: SendQueueOperation, submissionId: String? = nil,
                             onFailure: (() -> Void)? = nil, onDone: ((SnapshotResult) -> Void)? = nil) {
        guard !isPreview else { return }
        guard let runtimeID = projectionRuntimeID, let conversationID = loadedConversationID else { recordProblem("请选择会话"); onFailure?(); return }
        guard let transport else { recordProblem("本地核心未配置"); onFailure?(); return }
        enqueueSnapshot({ try transport.sendQueue(runtimeID: runtimeID, conversationID: conversationID, operation: operation, submissionId: submissionId) }, consumeReceipt: { result in
            switch result { case .success(let receipt): onDone?(receipt); case .failure: onFailure?() }
        })
    }
    var pendingInteractions: [RuntimeInteraction] { snapshot?.pendingInteractions ?? [] }
    func replyInteraction(_ interaction: RuntimeInteraction, reply: RuntimeInteractionReply) {
        guard pendingInteractions.contains(where: { $0.id == interaction.id }), let runtimeID = projectionRuntimeID, let conversationID = loadedConversationID, let transport else { return }
        let currentGeneration = generation
        enqueue({ try transport.replyRuntimeInteraction(runtimeID: runtimeID, conversationID: conversationID, interactionID: interaction.id, reply: reply) }) { [weak self] value in
            guard let self, generation == currentGeneration else { return }; applySnapshotResult(value)
        }
    }

    // MARK: - Session options

    /// The option the composer's model menu already offers: the session's
    /// first model-category option, which the runtime's catalog is read from.
    var editableSessionOptions: [AcpConfigOption] {
        guard var options = sessionOptions else { return [] }
        if let model = options.firstIndex(where: { $0.category == "model" }) { options.remove(at: model) }
        return options
    }
    var canChangeSessionOptions: Bool { inputSubmissionAllowed && !isGenerating && !executionStarting && !isLoading && !isShuttingDown }
    func setSessionOption(_ option: AcpConfigOption, value: AcpConfigValue) {
        guard canChangeSessionOptions, value != option.value, let runtimeID = snapshot?.conversation.runtimeID, let conversationID = loadedConversationID, let transport else { return }
        enqueue({ try transport.setSessionOption(runtimeID: runtimeID, conversationID: conversationID, optionID: option.id, value: value) }) { [weak self] options in
            guard let self, self.loadedConversationID == conversationID else { return }
            self.sessionOptions = options
        }
    }
    /// Options exist only while the conversation's session is live here, so
    /// they are read again when the conversation or its run state changes;
    /// not being live yet is not a problem to report.
    private func refreshSessionOptions() {
        guard let value = snapshot, !isDraft, !isPreview, let transport,
              let typeID = device.runtimeInstances.first(where: { $0.id == value.conversation.runtimeID })?.typeId,
              device.runtimeTypes.first(where: { $0.id == typeID })?.sessionOptions == true else {
            if sessionOptions != nil { sessionOptions = nil }; sessionOptionsKey = nil; return
        }
        let key = "\(value.conversation.id)|\(value.runState)"
        guard key != sessionOptionsKey else { return }
        if sessionOptionsKey?.hasPrefix(value.conversation.id + "|") != true { sessionOptions = nil }
        sessionOptionsKey = key
        let runtimeID = value.conversation.runtimeID, conversationID = value.conversation.id
        performOnQueue { [weak self] in
            let options = (try? transport.sessionOptions(runtimeID: runtimeID, conversationID: conversationID))
            DispatchQueue.main.async {
                guard let self, self.sessionOptionsKey == key else { return }
                let becameLive = self.sessionOptions == nil && options != nil
                self.sessionOptions = options
                // The runtime's own models come from this session.
                if becameLive, case .ready = self.modelCatalogs[runtimeID] { self.modelCatalogs[runtimeID] = nil; self.loadModelCatalog() }
            }
        }
    }

    // MARK: - Projection

    private func resetProjection() { composerTargetRetry = PollBackoff(); composerRequest = nil; stopRequested = false; executionStarting = false; preparedComposerTarget = nil; composerTargetRequestID = nil; nextTurn = nil; nextTurnRuntimeID = nil; loadedPayloads.removeAll(); isDraft = false; generation += 1; snapshot = nil; projectionRuntimeID = nil; selectedConversationID = nil; pendingConversationID = nil; abandonLinkedConversationLookup(); transcript.reset(); outgoing.reset(); activity = nil; sessionOptions = nil; sessionOptionsKey = nil }
    private func apply(_ value: ConversationSnapshot?, preserveSelection: Bool = false) {
        guard var value else { return }
        if value.conversation.id == unopenedConversationID {
            clearInitialOpening()
        }
        let conversationChanged = value.conversation.id != snapshot?.conversation.id
        let targetFactsChanged = conversationChanged
            || value.contextRuntimeID != snapshot?.contextRuntimeID
            || value.modelRecordKey != snapshot?.modelRecordKey
            || value.transcriptTurns.compactMap(\.settings) != snapshot?.transcriptTurns.compactMap(\.settings)
        if conversationChanged { creationFailure = nil }
        if value.conversation.id != snapshot?.conversation.id || (value.runState != .running && value.runState != .stopping) { stopRequested = false }
        if value.conversation.id != snapshot?.conversation.id || value.contextRuntimeID != snapshot?.contextRuntimeID { loadedPayloads.removeAll() }
        value.messages = withLoadedPayloads(value.messages)
        outgoing.reconcile(value)
        guard snapshot != value else { return }
        snapshot = value; projectionRuntimeID = value.contextRuntimeID; selectedConversationID = value.conversation.id
        if !isSideChat && !isDraft { device.rememberConversationTitle(value.conversation) }
        if isSideChat {
            refreshSideChatNextTurn()
            if let runtime = nextTurnRuntimeID, modelCatalogs[runtime] == nil { loadModelCatalog(runtimeID: runtime) }
        } else if targetFactsChanged { requestComposerTarget(refreshFacts: true) }
        else { refreshNextTurn() }
        if !preserveSelection && conversationChanged && selectedConversationIDs.count <= 1 { selectedConversationIDs = [value.conversation.id] }
        let editable = !isPreview && !isDraft ? Set(value.followUp.editable.map(\.messageId)) : []
        transcript.apply(value.messages, turns: value.transcriptTurns, items: value.transcriptItems, outline: value.transcriptOutline, identities: value.transcriptMessageIdentities, editable: editable)
        activity = switch value.runState {
        // A turn that a Harness process outside this execution runs reads the same.
        case .running, .elsewhere: TranscriptActivity(text: Self.currentWork(value.messages.last), isLive: true)
        case .stopping: TranscriptActivity(text: "正在停止", isLive: false)
        case .idle, .failed: nil
        }
        if isPreview { device.keepPreviewSnapshot(value) }
        refreshSessionOptions()
    }
    /// What a running turn is doing, from the newest block of its last
    /// message: a reasoning summary, the running tool, or replying.
    private static func currentWork(_ message: DeliveredMessage?) -> String {
        guard let message, let block = message.blocks.last else { return "正在思考" }
        if case .runtimeFailure(let failure, _, let retry?) = block {
            return failure.title + " · " + retry.label
        }
        if case .contextCompaction = block, !message.completed { return "正在压缩上下文" }
        guard message.role == .assistant else { return "正在思考" }
        switch block {
        case .reasoning(let summary, _): return summary
        case .tool(_, let title, .running, _, _), .tool(_, let title, .pending, _, _): return title
        case .text: return "正在回复"
        case .tool, .notice, .runtimeFailure, .contextCompaction, .image, .file, .agentSender: return "正在思考"
        }
    }
    /// Preparation and operation results replace the projection; polling asks
    /// the core only for changes after the current cursor (see `poll`).
    private func applySnapshotResult(_ value: SnapshotResult) {
        guard let incoming = value.snapshot else {
            if value.submissionReceipt != nil { return }
            resetProjection(); return
        }
        let mapped = BindingMapping.snapshot(incoming)
        if let current = snapshot, current.conversation.id == mapped.conversation.id,
           current.contextRuntimeID == mapped.contextRuntimeID,
           let previous = current.cursor, let next = mapped.cursor,
           previous.generation == next.generation, next.sequence < previous.sequence { return }
        apply(mapped)
    }
    private enum PolledSnapshot: Sendable {
        case absent, unchanged
        case complete(ConversationSnapshot)
        /// Changed messages only; merged on the main actor into the snapshot at `base`.
        case changes(base: TranscriptCursor, header: ConversationSnapshot, order: [String], changed: [DeliveredMessage])
    }
    private func poll() {
        // First-open recovery precedes the guard requiring an existing projection.
        retryOpeningIfNeeded()
        guard started, !isDraft, !isPreview, !isShuttingDown, snapshot != nil, !isLoading, !pollPending, let transport, let runtimeID = projectionRuntimeID else { return }
        let revision = generation
        let selected = selectedConversationID
        let since = snapshot?.cursor
        pollPending = true
        performOnQueue { [weak self] in
            // Only changed messages cross the boundary; mapping stays off the main thread.
            let result = Result { () throws -> PolledSnapshot in
                guard let since else {
                    return try transport.snapshot(runtimeID: runtimeID, conversationID: selected ?? "").snapshot.map { .complete(BindingMapping.snapshot($0)) } ?? .absent
                }
                switch try transport.snapshotUpdate(runtimeID: runtimeID, conversationID: selected ?? "", since: since) {
                case .absent: return .absent
                case .unchanged: return .unchanged
                case .changed(let incoming):
                    guard let base = incoming.base else { return .complete(BindingMapping.snapshot(incoming)) }
                    return .changes(base: base, header: BindingMapping.snapshotHeader(incoming), order: incoming.messageIds, changed: incoming.messages)
                }
            }
            DispatchQueue.main.async {
                guard let self else { return }
                self.pollPending = false
                guard self.generation == revision, self.selectedConversationID == selected else { return }
                switch result {
                case .success(let polled):
                    self.device.clearActivityProblem("poll:" + runtimeID)
                    switch polled {
                    case .absent, .unchanged: break
                    case .complete(let value): self.apply(value, preserveSelection: true)
                    case .changes(let base, var header, let order, let changed):
                        // A newer complete delivery may have replaced the base; the
                        // next poll then continues from that delivery's cursor.
                        guard let current = self.snapshot, current.cursor == base else { return }
                        var known = Dictionary(current.messages.map { ($0.id, $0) }, uniquingKeysWith: { first, _ in first })
                        for message in changed { known[message.id] = message }
                        let messages = order.compactMap { known[$0] }
                        guard messages.count == order.count else { self.snapshot?.cursor = nil; return }
                        header.messages = messages
                        self.apply(header, preserveSelection: true)
                    }
                case .failure(let failure): self.recordProblem(failure, source: "读取会话", activityKey: "poll:" + runtimeID)
                }
            }
        }
    }
    /// Read deferred reasoning and tool output of shown messages. Loaded content
    /// replaces a delivered message only while its stamp is current.
    func loadPayloads(messageIDs: [String]) {
        guard !isPreview, let transport, let snapshot else { return }
        let deferred = Dictionary(snapshot.messages.filter(\.hasDeferredPayload).map { ($0.id, $0.stamp) }, uniquingKeysWith: { first, _ in first })
        let wanted = Set(messageIDs).filter { id in deferred[id].map { !payloadRequests.contains(PayloadKey(id: id, stamp: $0)) } ?? false }
        guard !wanted.isEmpty else { return }
        let keys = wanted.map { PayloadKey(id: $0, stamp: deferred[$0]!) }
        payloadRequests.formUnion(keys)
        let runtimeID = snapshot.contextRuntimeID, conversationID = snapshot.conversation.id
        performOnQueue { [weak self] in
            let result = Result { try transport.messagePayloads(runtimeID: runtimeID, conversationID: conversationID, messageIDs: Array(wanted)) }
            DispatchQueue.main.async {
                guard let self else { return }
                self.payloadRequests.subtract(keys)
                switch result {
                case .success(let loaded):
                    guard var current = self.snapshot, current.conversation.id == conversationID, current.contextRuntimeID == runtimeID else { return }
                    for message in loaded { self.loadedPayloads[message.id] = message }
                    current.messages = self.withLoadedPayloads(current.messages)
                    self.apply(current, preserveSelection: true)
                case .failure(let failure): self.recordProblem(failure, source: "读取消息内容")
                }
            }
        }
    }
    private struct PayloadKey: Hashable { var id: String; var stamp: UInt64 }
    /// Substitute loaded payloads for deliveries of the same content; a newer
    /// stamp means the content changed and its old payload is dropped.
    private func withLoadedPayloads(_ messages: [DeliveredMessage]) -> [DeliveredMessage] {
        guard !loadedPayloads.isEmpty else { return messages }
        var retained: [String: DeliveredMessage] = [:]
        let merged = messages.map { message -> DeliveredMessage in
            guard let loaded = loadedPayloads[message.id], loaded.stamp == message.stamp else { return message }
            retained[message.id] = loaded
            return loaded
        }
        loadedPayloads = retained
        return merged
    }

    // MARK: - Requests

    func recordProblem(_ error: Error, source: String = "操作", activityKey: String? = nil) { device.recordProblem(error, source: source, activityKey: activityKey) }
    func recordProblem(_ message: String, source: String = "操作") { device.recordProblem(message, source: source) }
    private func performOnQueue(_ work: @escaping @Sendable () -> Void) {
        let owner = try? transport?.telemetryResult.get()
        let queuedAt = ProcessInfo.processInfo.systemUptime
        queue.async {
            owner?.recordQueueWait(ProcessInfo.processInfo.systemUptime - queuedAt)
            work()
        }
    }
    /// One conversation operation at a time; one that finishes after the tab
    /// moved on is dropped, and a conversation chosen meanwhile then opens.
    private func enqueueSnapshot(_ operation: @escaping () throws -> SnapshotResult,
                                 consumeReceipt: ((Result<SnapshotResult, Error>) -> Void)? = nil) {
        enqueue(operation, consumeReceipt: consumeReceipt) { [weak self] value in self?.applySnapshotResult(value) }
    }
    private func enqueue<T: Sendable>(_ operation: @escaping () throws -> T, consumeReceipt: ((Result<T, Error>) -> Void)? = nil, onAccepted: (() -> Void)? = nil, onFailure: ((Error) -> Void)? = nil, apply: @escaping (T) -> Void) {
        guard !isLoading, !isShuttingDown else { recordProblem("请等待当前操作完成后重试"); return }
        guard transport != nil else { recordProblem("本地核心未配置"); return }
        enqueuePreparedOperation(operation, consumeReceipt: consumeReceipt, onAccepted: onAccepted, onFailure: onFailure, apply: apply)
    }
    private func enqueuePreparedOperation<T: Sendable>(_ operation: @escaping () throws -> T, consumeReceipt: ((Result<T, Error>) -> Void)? = nil, onAccepted: (() -> Void)? = nil, onFailure: ((Error) -> Void)? = nil, apply: @escaping (T) -> Void) {
        guard !isLoading, !isShuttingDown else { recordProblem("请等待当前操作完成后重试"); return }
        let revision = generation
        isLoading = true
        performOnQueue { [weak self] in
            let result = Result { try operation() }
            DispatchQueue.main.async {
                guard let self else { return }
                self.isLoading = false
                defer { self.selectDeferredConversation() }
                consumeReceipt?(result)
                guard revision == self.generation else { return }
                switch result {
                case .success(let value): onAccepted?(); apply(value)
                case .failure(let failure): self.recordProblem(failure); onFailure?(failure)
                }
            }
        }
    }
}

extension ConversationStore {
    /// Native state/queue regression for cold-open recovery. All input is
    /// synthetic; the queue case owns only a transport in the supplied home.
    static func checkColdOpeningRetryLifecycle(home: URL) {
        func note(_ value: String) { FileHandle.standardError.write(Data((value + "\n").utf8)) }
        func wait(_ condition: () -> Bool) {
            let deadline = Date().addingTimeInterval(5)
            while !condition(), Date() < deadline { RunLoop.main.run(until: Date().addingTimeInterval(0.02)) }
            precondition(condition(), "冷打开生命周期检查超时")
        }
        let sample = DeviceStore(preview: true).previewSnapshots["sample-2"]!
        let transport = Transport(stateDirectory: home.appendingPathComponent("cold-opening-queue"), resourcesDirectory: Bundle.main.resourceURL!)
        let store = ConversationStore(device: DeviceStore(transport: transport))
        store.started = true
        store.unopenedConversationID = sample.conversation.id
        store.selectedConversationID = sample.conversation.id
        let unavailable = TransportError.operation("openConversation", .diagnostic(kind: "unavailable", detail: "会话历史正在读取，请稍后重试", code: "unavailable", phase: "read", operationID: "synthetic"))
        store.noteOpeningFailure(unavailable)
        precondition(store.snapshot == nil && store.isOpeningConversation && store.openingFailure == nil)
        precondition(store.takeOpeningRetry() == nil && store.openingRetry.failures == 1,
                     "暂态失败不能在下一个通知中立即热重试")
        wait { store.openingRetry.ready }
        store.isLoading = true
        precondition(store.takeOpeningRetry() == nil)
        store.isLoading = false
        store.linkedConversationRequest = UUID()
        precondition(store.takeOpeningRetry() == nil, "在途lookup不能被重试取代")
        store.abandonLinkedConversationLookup()
        precondition(store.takeOpeningRetry() == sample.conversation.id && store.takeOpeningRetry() == nil,
                     "snapshot尚不存在时仍应取出原目标，而且只取一次")
        store.noteOpeningFailure(unavailable)
        precondition(store.openingRetry.failures == 2 && !store.openingRetry.ready)
        store.apply(sample)
        precondition(store.loadedConversationID == sample.conversation.id && !store.openingNeedsRetry && store.unopenedConversationID == nil)
        precondition(store.transcript.rows.map(\.message) == sample.messages)
        note("PASS cold open: typed temporary failure retains destination; backoff, loading/lookup dedup, single retry admission, second failure backoff and successful original transcript")

        for error in [TransportError.closed, .quitting, .rejected("synthetic missing"),
                      .diagnostic(kind: "history", detail: "synthetic invalid history", code: "invalid_history", phase: "read", operationID: "")] {
            store.resetProjection()
            store.unopenedConversationID = sample.conversation.id
            store.noteOpeningFailure(error)
            precondition(!store.openingNeedsRetry && store.openingFailure != nil && store.takeOpeningRetry() == nil)
        }
        store.noteOpeningFailure(unavailable)
        store.offListConversations = [sample.conversation]
        store.selectConversation(id: sample.conversation.id + "-other")
        precondition(store.unopenedConversationID == nil && !store.openingNeedsRetry,
                     "切换目的地必须撤销旧重试，即使新目的地尚未读到列表")
        store.unopenedConversationID = sample.conversation.id
        store.noteOpeningFailure(unavailable)
        let lookup = UUID()
        store.linkedConversationRequest = lookup
        store.finishLinkedConversationLookup(.success(nil), id: sample.conversation.id, move: .new, request: lookup)
        precondition(store.unopenedConversationID == nil && !store.openingNeedsRetry && store.openingFailure != nil)
        note("PASS permanent failures and missing lookup stop retries; navigation cannot return to the abandoned destination")

        store.unopenedConversationID = sample.conversation.id
        store.noteOpeningFailure(TransportError.diagnostic(kind: "history", detail: "synthetic read error",
                                                          code: "provider_read", phase: "native_read", operationID: "synthetic-read"))
        precondition(store.coldOpeningProblem?.detail == "synthetic read error"
                     && store.coldOpeningProblem?.operationID == "synthetic-read" && store.canRetryFailedOpening,
                     "冷打开失败时，正文必须保留诊断和原会话的重试入口")

        let manual = ConversationStore(device: DeviceStore(transport: transport))
        manual.offListConversations = [sample.conversation]
        manual.selectConversation(id: sample.conversation.id)
        precondition(manual.isLoading && manual.unopenedConversationID == sample.conversation.id,
                     "空Tab从列表首次打开也必须登记目标，不能只照顾start(opening:)种子")
        manual.close()
        wait { !manual.isLoading }
        let linked = ConversationStore(device: DeviceStore(transport: transport))
        linked.openLinkedConversation(id: "synthetic-off-list")
        precondition(linked.unopenedConversationID == "synthetic-off-list" && linked.linkedConversationRequest != nil)
        linked.close()
        precondition(linked.unopenedConversationID == nil && linked.linkedConversationRequest == nil)
        note("PASS manual first opens: listed and off-list link destinations are remembered before their asynchronous read; closure cancels both")

        // An operation may already be on the background queue when its tab
        // closes. Its completion must not resurrect that closed tab's state.
        let closing = ConversationStore(device: DeviceStore(transport: transport))
        closing.started = true
        closing.unopenedConversationID = sample.conversation.id
        closing.noteOpeningFailure(unavailable)
        let release = DispatchSemaphore(value: 0)
        var applied = false
        closing.enqueue({
            _ = release.wait(timeout: .now() + 5)
            return true
        }, onFailure: { _ in preconditionFailure("synthetic operation cannot fail") }) { _ in applied = true }
        precondition(closing.isLoading)
        let beforeClose = closing.generation
        closing.close()
        release.signal()
        wait { !closing.isLoading }
        precondition(!applied && closing.generation != beforeClose && !closing.openingNeedsRetry && closing.takeOpeningRetry() == nil)
        note("PASS close: queued successful completion is discarded and no retry survives tab closure")
    }

    /// Isolated fixture for the queue editor's attachment and receipt states.
    static func inspectQueuedEditor(_ inspect: (ConversationWindowModel, QueuedMessage, (QueuedMessage?, Bool) -> Void, (RuntimeModelCatalog?) -> Void) -> Void) {
        let window = ConversationWindowModel(application: DeviceStore(preview: true), kind: .workspace)
        let store = window.focused.store
        var snapshot = store.device.previewSnapshots["sample-0"]!
        let runtimeID = snapshot.contextRuntimeID
        let message = QueuedMessage(id: "synthetic-queue", submissionId: "synthetic-submission", text: "补充验收记录并保留附件。",
                                    attachments: ["/synthetic-host/notes.md"],
                                    editContext: .init(revision: 3, runtimeId: runtimeID,
                                                       selection: .native(providerId: "sample", modelId: "sample-native"),
                                                       effort: "medium", permissionMode: nil))
        store.isDraft = false
        snapshot.followUp = .init()
        snapshot.followUp.queue = [message]
        snapshot.followUp.paused = .turnStopped
        store.apply(snapshot)
        store.loadModelCatalog(runtimeID: runtimeID)
        inspect(window, message, { item, busy in
            snapshot.followUp.queue = item.map { [$0] } ?? []
            store.apply(snapshot)
            store.isLoading = busy
        }, { store.modelCatalogs[runtimeID] = $0 })
    }

    /// The frontend must dispatch while the source runs, not retain a hidden
    /// intent until a later snapshot. The private empty backend rejects this
    /// synthetic identity, which also checks failure restores only loading state.
    static func checkConcurrentForkDispatch(home: URL) {
        let transport = Transport(stateDirectory: home.appendingPathComponent("fork-dispatch"), resourcesDirectory: Bundle.main.resourceURL!)
        let store = ConversationStore(device: DeviceStore(transport: transport))
        store.composerTargetClosed = true
        for state in [RunState.running, .elsewhere] {
            let source = ConversationSnapshot(revision: 1,
                conversation: Conversation(id: "synthetic-running-source", title: "Synthetic", updatedAtUnixMs: nil,
                    runtimeID: "synthetic", cwd: nil, canRename: false, canDelete: false),
                contextRuntimeID: "synthetic", modelRecordKey: nil, runState: state, messages: [],
                actions: ConversationActions(inputAccess: .editable, canSend: false, canCancel: false, canSwitch: true))
            store.apply(source)
            precondition(store.canFork, "源运行态不能禁止独立 fork")
            store.forkConversation(through: "completed-user") { _ in preconditionFailure("合成空后端不应找到会话") }
            precondition(store.isForking && store.isLoading, "必须立即派发，不能等待源结束")
            let deadline = Date().addingTimeInterval(15)
            while store.isLoading, Date() < deadline { RunLoop.main.run(until: Date().addingTimeInterval(0.02)) }
            precondition(!store.isLoading && !store.isForking, "实际失败必须结束复制加载态")
            precondition(store.snapshot?.runState == state && store.loadedConversationID == source.conversation.id,
                         "失败不得替换或停止源会话投影")
        }
        // A completed fork republishes the core list (noteForkCompleted):
        // the sidebar shows the fork only as the core's grouped entry, with
        // no provisional row of its own. The empty backend answers the list
        // read, so the refresh applying is observable through listRevision.
        let revision = store.device.listRevision
        store.device.noteForkCompleted()
        let refreshDeadline = Date().addingTimeInterval(15)
        while store.device.listRevision == revision, Date() < refreshDeadline { RunLoop.main.run(until: Date().addingTimeInterval(0.02)) }
        precondition(store.device.listRevision > revision, "fork 后必须由核心列表刷新提供新会话条目")
        store.close()
        FileHandle.standardError.write(Data("PASS fork dispatch: running and elsewhere sources dispatch immediately; backend failure clears loading without changing source; a completed fork republishes the core list with no provisional entry\n".utf8))
    }

    /// Hidden native check of the production navigation/catalog admission path.
    /// A retained source snapshot must not launch any catalog before open finishes.
    static func checkOpeningCatalogAdmission() {
        let device = DeviceStore(preview: true)
        let store = ConversationStore(device: device)
        let source = device.previewSnapshots["sample-0"]!
        store.apply(source)
        store.modelCatalogs.removeAll()
        let targetRuntime = "sample-review"
        store.pendingConversationID = "synthetic-opening-destination"
        store.loadModelCatalog(runtimeID: targetRuntime)
        precondition(store.modelCatalogs[targetRuntime] == nil, "打开会话前不应启动旧输入触发的模型目录")
        store.nextTurnRuntimeID = targetRuntime
        precondition(store.modelCatalogs[targetRuntime] == nil, "加载中的目标选择不应阻塞历史打开")
        store.deferredSelection = ("synthetic-next-destination", .new)
        store.finishConversationLoading()
        precondition(store.modelCatalogs[targetRuntime] == nil, "已有更新导航时不能先排入旧目标模型目录")
        store.deferredSelection = nil
        store.finishConversationLoading()
        guard case .ready? = store.modelCatalogs[targetRuntime] else {
            preconditionFailure("会话打开完成后应加载实际目标运行时的模型目录")
        }
        FileHandle.standardError.write(Data("PASS conversation opening: retained snapshot cannot start a catalog ahead of open; destination catalog loads after opening\n".utf8))
        checkOpeningFailureRecovery()
    }

    private static func checkOpeningFailureRecovery() {
        let home = FileManager.default.temporaryDirectory.appendingPathComponent("velune-open-failure-\(UUID())", isDirectory: true)
        let transport = Transport(stateDirectory: home, telemetry: .failure(TransportError.unavailable), remote: {
            throw TransportError.rejected("synthetic opening failure")
        })
        let device = DeviceStore(transport: transport)
        let window = ConversationWindowModel(application: device, kind: .workspace)
        window.tools.move(.composer, to: .docked(.bottom))
        let tab = window.focused
        let store = tab.store
        let destination = Conversation(id: "missing:destination", title: "Synthetic destination", updatedAtUnixMs: nil, runtimeID: "missing", cwd: nil, canRename: false, canDelete: false)
        store.offListConversations = [destination]
        func wait() {
            let deadline = Date().addingTimeInterval(5)
            while store.isLoading, Date() < deadline { RunLoop.main.run(until: Date().addingTimeInterval(0.01)) }
            precondition(!store.isLoading, "合成打开失败检查超时")
        }
        func source(_ id: String, isDraft: Bool) {
            store.isDraft = isDraft
            store.apply(ConversationSnapshot(revision: 1, conversation: Conversation(id: id, title: "Synthetic source", updatedAtUnixMs: nil, runtimeID: "missing", cwd: nil, canRename: false, canDelete: false), contextRuntimeID: "missing", modelRecordKey: nil, runState: .idle, messages: [], actions: ConversationActions(inputAccess: .editable, canSend: true, canCancel: false, canSwitch: true)))
            store.composerEvents.send(isDraft ? .newConversation : .conversation(id))
        }
        for isDraft in [false, true] {
            let id = isDraft ? draftConversationPrefix + "new-conversation" : "missing:source"
            source(id, isDraft: isDraft)
            let input = window.composer(for: tab)
            input.restoreSavedDraft(.init(text: isDraft ? "unsent source" : "existing source", attachments: []))
            store.selectConversation(id: destination.id)
            precondition(window.sharedComposer.id != input.id, "发起导航应切换目标输入")
            wait()
            precondition(store.selectedConversationID == id && store.loadedConversationID == id)
            precondition(store.isUnsentConversation == isDraft, "打开失败必须恢复原草稿身份")
            precondition(window.composer(for: tab).id == input.id && window.sharedComposer.submission == input.submission, "打开失败必须恢复原会话输入")
        }
        // A second navigation is deferred until the first settles; a failed
        // opening must not prevent that newer intent from taking its turn.
        let latest = Conversation(id: "missing:latest", title: "Latest destination", updatedAtUnixMs: nil, runtimeID: "missing", cwd: nil, canRename: false, canDelete: false)
        store.offListConversations.append(latest)
        var destinations: [String] = []
        let watch = store.composerEvents.sink { event in if case .conversation(let id) = event { destinations.append(id) } }
        store.selectConversation(id: destination.id)
        store.selectConversation(id: latest.id)
        wait()
        precondition(destinations.contains(latest.id) && store.deferredSelection == nil, "较早失败不得吞掉之后的导航")
        watch.cancel()
        FileHandle.standardError.write(Data("PASS failed opening: source input and unsent identity restored; newer deferred navigation still executes\n".utf8))

        let linkedID = "missing:linked"
        let found = ListedConversation(conversation: ConversationSummary(id: linkedID, title: .native("Synthetic linked"), updatedAtUnixMs: nil, createdAtUnixMs: nil, runtimeId: "missing", cwd: nil, canRename: false, canDelete: false, parentConversationId: nil, nativeReadState: nil), nativeId: "synthetic-native", agentsUri: nil, pinned: false, unread: false, archived: false, project: nil, moved: false, failed: false, group: nil)
        store.openLinkedConversation(id: linkedID)
        let oldRequest = store.linkedConversationRequest!
        store.selectConversation(id: latest.id)
        store.finishLinkedConversationLookup(.success(found), id: linkedID, move: .new, request: oldRequest)
        precondition(store.selectedConversationID == latest.id && store.deferredSelection == nil && store.conversation(id: linkedID) == nil,
                     "已选择新目标后，旧离表查找成功也不能夺回导航")
        wait()

        // Choosing the same link again is a new request too; an older
        // response cannot consume its intent or navigation-history move.
        store.openLinkedConversation(id: linkedID, move: .back)
        let firstRequest = store.linkedConversationRequest!
        store.openLinkedConversation(id: linkedID)
        let currentRequest = store.linkedConversationRequest!
        precondition(firstRequest != currentRequest)
        store.finishLinkedConversationLookup(.success(found), id: linkedID, move: .back, request: firstRequest)
        precondition(store.linkedConversationRequest == currentRequest && store.conversation(id: linkedID) == nil)
        store.finishLinkedConversationLookup(.success(found), id: linkedID, move: .new, request: currentRequest)
        precondition(store.selectedConversationID == linkedID && store.linkedConversationRequest == nil)
        wait()
        FileHandle.standardError.write(Data("PASS linked navigation: stale lookup cannot replace a newer destination or consume a newer request for the same link\n".utf8))
    }
}

extension ConversationStore {
    /// Runs the production serial receipt-before-projection runner with a
    /// preview store. Synthetic operations never create or open a backend.
    static func checkOutgoingAdmissionLifecycle(home: URL, inspect: (ConversationWindowModel, String) -> Void) {
        precondition(!MacNativePersistenceGate.isEnabled)
        precondition(home.path.hasPrefix("/tmp/") || home.path.hasPrefix("/private/tmp/"))
        let device = DeviceStore(preview: true)
        let window = ConversationWindowModel(application: device, kind: .workspace)
        let store = window.focused.store
        let summary = ConversationSummary(id: "queue-fixture", title: .native("合成提交与队列"), updatedAtUnixMs: nil,
            createdAtUnixMs: nil, runtimeId: "sample-instance", cwd: nil, canRename: false, canDelete: false,
            parentConversationId: nil, nativeReadState: nil)
        let history = DeliveredMessage(id: "history-user", role: .user, blocks: [.text(text: "已有原生用户消息，始终保留。")])
        func snapshot(_ sequence: UInt64, queue: [QueuedMessage] = [], sent: [DeliveredMessage] = []) -> DeliveredSnapshot {
            let messages = [history] + sent
            var followUp = FollowUp()
            followUp.queue = queue
            return DeliveredSnapshot(cursor: .init(generation: 701, sequence: sequence), base: nil,
                messageIds: messages.map(\.id), messages: messages, revision: sequence, conversation: summary,
                contextRuntimeId: "sample-instance", resourceId: nil, modelRecordKey: nil, runState: .running,
                transcriptTurns: [], usage: .init(), transcriptItems: messages.map { .message(id: "message:" + $0.id, messageId: $0.id) },
                transcriptOutline: [], transcriptMessageIdentities: messages.map { .init(id: "message:" + $0.id, messageId: $0.id) },
                messageIdentityConfirmations: [], pendingInteractions: [],
                actions: .init(inputAccess: .editable, canSend: false, canCancel: true, canSwitch: false),
                followUp: followUp, subAgents: [], collaborators: [])
        }
        func apply(_ value: DeliveredSnapshot) { store.apply(BindingMapping.snapshot(value)) }
        func enqueueFixture(_ operation: @escaping () throws -> SnapshotResult, consumeReceipt: @escaping (Result<SnapshotResult, Error>) -> Void) {
            store.enqueuePreparedOperation(operation, consumeReceipt: consumeReceipt) { value in
                store.applySnapshotResult(value)
            }
        }
        func receipt(_ id: String, _ value: SnapshotResult, beforeProjection: (() -> Void)? = nil) {
            var consumed = false
            enqueueFixture({ value }, consumeReceipt: { result in
                guard case .success(let incoming) = result else { preconditionFailure("synthetic receipt failed") }
                store.acceptSubmission(id, receipt: incoming)
                beforeProjection?()
                consumed = true
            })
            let end = Date().addingTimeInterval(8)
            while !consumed, Date() < end { RunLoop.main.run(until: Date().addingTimeInterval(0.01)) }
            precondition(consumed && !store.isLoading, "synthetic business receipt must settle")
        }
        apply(snapshot(1))
        let queueInput = window.composer(for: window.focused)
        queueInput.restore("仅排队：尚未派发，不应进入消息列表。", attachments: [])
        let queueRevision = queueInput.revision
        let queued = store.outgoing.begin(.queue, text: queueInput.submission, attachments: [], snapshot: store.snapshot)
        store.composerRequest = .queueing
        precondition(store.outgoing.displayed.isEmpty && store.pendingQueuedMessages.isEmpty && store.followUp.queue.isEmpty)
        precondition(window.focused.submission(using: queueInput).action == .queueing && !window.focused.submission(using: queueInput).enabled)
        inspect(window, "pending-queue-input-retained")
        let canonical = QueuedMessage(id: "queue-A", submissionId: queued, text: "仅排队：尚未派发，不应进入消息列表。", attachments: [])
        receipt(queued, SnapshotResult(snapshot: snapshot(2, queue: [canonical]), submissionReceipt: .queued(queueId: canonical.id))) {
            store.composerRequest = nil
            if queueInput.revision == queueRevision { queueInput.clear(submitted: canonical.text, attachments: []) }
            precondition(queueInput.isEmpty, "successful queue admission consumes the unchanged captured input")
            precondition(store.followUp.queue.isEmpty, "receipt consumer must precede projection installation")
            precondition(store.outgoing.displayed.isEmpty && store.pendingQueuedMessages.isEmpty,
                         "queue receipt must use its own projection, not the old Store snapshot")
        }
        precondition(store.followUp.queue.map(\.id) == [canonical.id] && store.outgoing.displayed.isEmpty)
        inspect(window, "accepted-queue")

        apply(snapshot(3))
        let racing = store.outgoing.begin(.send, text: "忙闲竞态：核心将这次发送实际接纳为排队。", attachments: [], snapshot: store.snapshot)
        precondition(store.outgoing.displayed.count == 1)
        inspect(window, "optimistic-send")
        let raced = QueuedMessage(id: "queue-race", submissionId: racing, text: "忙闲竞态：核心将这次发送实际接纳为排队。", attachments: [])
        apply(snapshot(4, queue: [raced]))
        precondition(store.outgoing.displayed.isEmpty && store.pendingQueuedMessages.isEmpty && store.followUp.queue.count == 1,
                     "poll-before-ACK must transfer the same submission only to the queue")
        inspect(window, "poll-before-ack")
        let dispatched = DeliveredMessage(id: "native-dispatched", role: .user, blocks: [.text(text: raced.text)])
        apply(snapshot(5, sent: [dispatched]))
        receipt(racing, SnapshotResult(snapshot: snapshot(4, queue: [raced]), submissionReceipt: .queued(queueId: raced.id)))
        precondition(store.snapshot?.cursor?.sequence == 5 && store.followUp.queue.isEmpty && store.pendingQueuedMessages.isEmpty && store.outgoing.displayed.isEmpty,
                     "late queued ACK must neither overwrite a newer dispatch nor resurrect its placeholder")
        precondition(store.transcript.rows.map(\.message.id) == [history.id, dispatched.id])
        inspect(window, "dispatched-late-ack")

        let failure = store.outgoing.begin(.send, text: "已由轮询确认受理，迟到错误不得恢复草稿。", attachments: [], snapshot: store.snapshot)
        let admitted = QueuedMessage(id: "queue-admitted", submissionId: failure, text: "已由轮询确认受理，迟到错误不得恢复草稿。", attachments: [])
        let releaseFailure = DispatchSemaphore(value: 0)
        var failureSettled = false, restoredDrafts = 0
        enqueueFixture({ () throws -> SnapshotResult in
            _ = releaseFailure.wait(timeout: .now() + 8)
            throw TransportError.rejected("合成 RPC 投影失败")
        }, consumeReceipt: { result in
            guard case .failure = result else { preconditionFailure("synthetic RPC must fail") }
            if store.outgoing.fail(failure) { restoredDrafts += 1 }
            failureSettled = true
        })
        apply(snapshot(6, queue: [admitted], sent: [dispatched]))
        releaseFailure.signal()
        let failureDeadline = Date().addingTimeInterval(8)
        while !failureSettled, Date() < failureDeadline { RunLoop.main.run(until: Date().addingTimeInterval(0.01)) }
        precondition(failureSettled && restoredDrafts == 0 && store.pendingQueuedMessages.isEmpty && store.outgoing.displayed.isEmpty,
                     "poll-confirmed admission must forbid draft restoration after a real RPC-lane failure")
        inspect(window, "admitted-before-rpc-failure")

        let unavailable = store.outgoing.begin(.queue, text: "已受理，投影暂时不可用。", attachments: [], snapshot: store.snapshot)
        receipt(unavailable, SnapshotResult(snapshot: nil, submissionReceipt: .queued(queueId: "queue-no-projection")))
        precondition(store.loadedConversationID == "queue-fixture" && store.snapshot?.cursor?.sequence == 6 && store.transcript.rows.count == 2,
                     "successful admission with unavailable projection must retain the current conversation and native transcript")
        let recoveredQueue = QueuedMessage(id: "queue-no-projection", submissionId: unavailable, text: "已受理，投影暂时不可用。", attachments: [])
        apply(snapshot(7, queue: [admitted, recoveredQueue], sent: [dispatched]))
        precondition(store.pendingQueuedMessages.isEmpty && store.followUp.queue.count == 2)

        apply(snapshot(8))
        let immediateInput = "空闲即时派发：原生消息先于受理回执到达。"
        queueInput.restore(immediateInput, attachments: [])
        let immediateRevision = queueInput.revision
        let immediateToken = store.outgoing.begin(.queue, text: immediateInput, attachments: [], snapshot: store.snapshot)
        store.composerRequest = .queueing
        let immediateNative = DeliveredMessage(id: "native-before-ack", role: .user, blocks: [.text(text: immediateInput)])
        apply(snapshot(9, sent: [immediateNative]))
        precondition(store.followUp.queue.isEmpty && store.pendingQueuedMessages.isEmpty && store.outgoing.displayed.isEmpty,
                     "a queue intent dispatched while idle must not manufacture a queue row beside the native echo")
        precondition(queueInput.submission == immediateInput)
        inspect(window, "immediate-native-before-ack")
        receipt(immediateToken, SnapshotResult(snapshot: snapshot(9, sent: [immediateNative]), submissionReceipt: .sending)) {
            store.composerRequest = nil
            if queueInput.revision == immediateRevision { queueInput.clear(submitted: immediateInput, attachments: []) }
        }
        precondition(store.followUp.queue.isEmpty && store.outgoing.displayed.isEmpty && queueInput.isEmpty)
        let ledger = OutgoingSubmissions()
        let base = BindingMapping.snapshot(snapshot(7))
        let failed = ledger.begin(.queue, text: "同文请求", attachments: [], snapshot: base)
        precondition(ledger.fail(failed), "unadmitted failure must restore its original input")
        let retry = ledger.begin(.queue, text: "同文请求", attachments: [], snapshot: base)
        let another = ledger.begin(.queue, text: "同文请求", attachments: [], snapshot: base)
        precondition(failed != retry && retry != another && ledger.queued.isEmpty && ledger.displayed.isEmpty,
                     "unadmitted queue requests are tracked without manufacturing queue rows")
        var unrelated = base
        unrelated.followUp.queue = [.init(id: "unrelated", submissionId: "another-client", text: "同文请求", attachments: [])]
        unrelated.messages.append(.init(id: "real-same-text", role: .user, blocks: [.text(text: "同文请求")]))
        unrelated.transcriptMessageIdentities.append(.init(id: "real-same-text", messageId: "real-same-text"))
        ledger.reconcile(unrelated)
        precondition(ledger.queued.isEmpty && unrelated.messages.count == 2,
                     "same text is neither queue ownership nor permission to filter native users")
        let immediate = OutgoingSubmissions()
        let immediateID = immediate.begin(.queue, text: "空闲时直接派发", attachments: [], snapshot: base)
        immediate.accept(immediateID, receipt: .sending, snapshot: base)
        precondition(immediate.displayed.count == 1 && immediate.queued.isEmpty,
                     "only a typed Sending receipt may promote queue intent to a dispatched placeholder")
        var echoed = base
        echoed.messages.append(.init(id: "native-immediate", role: .user, blocks: [.text(text: "空闲时直接派发")]))
        echoed.transcriptMessageIdentities.append(.init(id: "native-immediate", messageId: "native-immediate"))
        immediate.reconcile(echoed)
        precondition(immediate.displayed.isEmpty && echoed.messages.count == 2)
        let pollFirstImmediate = OutgoingSubmissions()
        let pollFirstID = pollFirstImmediate.begin(.queue, text: "空闲即时派发", attachments: [], snapshot: base)
        var immediateEcho = base
        immediateEcho.messages.append(.init(id: "poll-first-native", role: .user, blocks: [.text(text: "空闲即时派发")]))
        immediateEcho.transcriptMessageIdentities.append(.init(id: "poll-first-native", messageId: "poll-first-native"))
        pollFirstImmediate.reconcile(immediateEcho)
        precondition(pollFirstImmediate.queued.isEmpty && pollFirstImmediate.displayed.isEmpty && immediateEcho.messages.count == 2,
                     "explicit queue dispatched while idle must not duplicate native poll-before-ACK in a local queue")
        pollFirstImmediate.accept(pollFirstID, receipt: .sending, snapshot: immediateEcho)
        precondition(pollFirstImmediate.queued.isEmpty && pollFirstImmediate.displayed.isEmpty)
        let missingProjection = OutgoingSubmissions()
        let missingID = missingProjection.begin(.send, text: "已排队但投影暂缺", attachments: [], snapshot: base)
        missingProjection.accept(missingID, receipt: .queued(queueId: "projection-missing"), snapshot: nil)
        precondition(missingProjection.displayed.isEmpty && missingProjection.queued.first?.id == "projection-missing")
        missingProjection.reconcile(base)
        precondition(missingProjection.queued.isEmpty,
                     "next complete observation may already be after dispatch/removal; never leave a stale local queue copy")
        let closing = OutgoingSubmissions()
        let closingID = closing.begin(.send, text: "关闭后的受理回执", attachments: [], snapshot: base)
        var closingSnapshot = base
        closingSnapshot.followUp.queue = [.init(id: "closing-queue", submissionId: closingID, text: "关闭后的受理回执", attachments: [])]
        closing.reconcile(closingSnapshot); closing.reset()
        precondition(!closing.fail(closingID) && closing.displayed.isEmpty && closing.queued.isEmpty,
                     "navigation must retain in-flight admission proof without leaking into the new view")
        if case .remove(let id) = OutgoingSubmissions.queuedCancellation(.queued(queueId: "cancel-exact")) {
            precondition(id == "cancel-exact")
        } else { preconditionFailure("queued cancellation must target its exact queue identity") }
        precondition(OutgoingSubmissions.queuedCancellation(.sending) == nil && OutgoingSubmissions.queuedCancellation(nil) == nil)
        let composer = ComposerModel()
        composer.restore("取消前的输入", attachments: [])
        let original = composer.submission, attachments = composer.attachments
        composer.clear(submitted: original, attachments: attachments)
        let revision = composer.revision
        composer.restoreFailed(original, attachments: attachments, revision: revision)
        precondition(composer.submission == original, "confirmed queued cancellation recovers the original input")
        composer.clear(submitted: original, attachments: attachments)
        let previous = composer.revision
        composer.restore("较新的编辑", attachments: [])
        composer.restoreFailed(original, attachments: attachments, revision: previous)
        precondition(composer.submission == "较新的编辑", "queued cancellation must not overwrite a newer input revision")
        precondition(!MacNativePersistenceGate.isEnabled && !store.started && !window.acceptsWorkspacePersistence)
        store.close(); window.list.close()
    }
}

@MainActor extension ConversationStore {
    /// Historical execution settings and a synthetic catalog drive the real
    /// message-editor popover without reading or mutating a runtime.
    static func inspectEditorVisualStates(_ inspect: (ConversationWindowModel, MessageEditingSession, String) -> Void) {
        let matrix = CommandLine.arguments.contains("--editor-visual-matrix") || CommandLine.arguments.contains("--experience-message-editor-offscreen")
        let requested = CommandLine.arguments.first(where: { $0.hasPrefix("--editor-visual-state=") }).map { String($0.dropFirst("--editor-visual-state=".count)) }
        for state in ["empty", "long", "failed", "missing-model", "missing-runtime", "fork-only", "legacy-model", "legacy-model-loading", "legacy-model-missing", "catalog-loading", "busy", "failure", "failure-target-changed", "placement", "history-changed", "long-floating", "busy-floating", "failure-floating"] {
            if let requested { if state != requested { continue } }
            else if !matrix, state != "long", state != "failed" { continue }
            let device = DeviceStore(preview: true)
            let workspace = ConversationWindowModel(application: device, kind: .workspace)
            let source = workspace.focused.store
            var value = device.previewSnapshots["sample-0"]!
            let message = value.messages.first(where: { $0.role == .user })!
            let legacy = state.hasPrefix("legacy-model")
            let selection: ModelSelection = legacy ? .managed(modelRecordKey: "synthetic-editor-route")
                : .native(providerId: "sample", modelId: "visual-editor-model")
            let recordedModel = legacy ? "GPT-6.1-Sol（ChatGPT1）" : "visual-editor-model"
            value.transcriptTurns = [.init(id: "editor-turn", userMessageId: message.id, workMessageIds: [], lastMessageId: nil, replyStartBlock: 0, durationMs: nil, isRunning: false, canFork: false, settings: .init(runtimeId: value.contextRuntimeID, model: recordedModel, modelSelection: legacy ? nil : selection), usage: nil)]
            device.keepPreviewSnapshot(value)
            source.isDraft = false
            source.apply(value)
            let session = MessageEditingSession(source: source, request: .init(message: message, conversationID: value.conversation.id, inPlace: state != "fork-only", isLast: state != "placement", revision: ["history-changed", "failure-target-changed"].contains(state) ? "synthetic-old-revision" : nil), presentation: state.hasSuffix("floating") ? .floating : .sheet)
            session.start()
            let store = session.store
            if state == "failed" { store.modelCatalogs[value.contextRuntimeID] = .failed("模型目录读取失败（合成错误）。请重新读取模型后重试。") }
            else { store.modelCatalogs[value.contextRuntimeID] = .ready([.init(selection: selection, name: "视觉验收长模型 " + String(repeating: "Very Long Model Name ", count: 10), reasoningEfforts: ["low", "medium", "high"])]) }
            store.refreshNextTurn()
            if legacy {
                store.modelCatalogs[value.contextRuntimeID] = .loading
                precondition(session.next?.chosenModelUnavailable == false, "目录加载不能被判成模型不可用")
                precondition(session.target.issue?.hasPrefix("正在确认原消息的模型") == true,
                             "尚未恢复历史模型时不可悄悄按默认模型发送")
                store.modelCatalogs[value.contextRuntimeID] = .ready([.init(selection: selection, name: recordedModel, reasoningEfforts: [])])
                precondition(session.next?.model == selection && session.next?.chosenModelUnavailable == false,
                             "历史显示名应恢复为目录中的可靠选择，而非伪造原生ID")
                if state == "legacy-model-loading" {
                    store.modelCatalogs[value.contextRuntimeID] = .loading
                    let context = ComposerContext(tab: session.tab, model: session.editor, editingTarget: session.target)
                    precondition(context.selectedModelName == recordedModel && !context.chosenModelUnavailable,
                                 "确认原消息模型期间应保留记录名称，不能声称默认模型或不可用")
                }
                if state == "legacy-model-missing" {
                    store.modelCatalogs[value.contextRuntimeID] = .ready([])
                    precondition(session.next?.chosenModelUnavailable == true, "无法恢复历史模型时不得默认发送")
                }
            }
            if state == "catalog-loading" {
                let chosen = ModelSelection.native(providerId: "sample", modelId: "user-selected-model")
                session.target.chooseModel(chosen)
                store.modelCatalogs[value.contextRuntimeID] = .loading
                precondition(session.next?.choice.model == chosen && session.next?.chosenModelUnavailable == false,
                             "异步目录准备必须保留用户选择，loading不能判不可用")
            }
            if state == "empty" {
                session.editor.receiveDraft(.init(text: "", attachments: []))
                precondition(session.editor.isEmpty)
            }
            if state.hasPrefix("long") {
                let text = String(repeating: "长正文保持原生字体与内边距，增长到上限后滚动。\n", count: 30)
                session.editor.receiveDraft(.init(text: text, attachments: []))
                precondition(session.editor.submission == text)
            }
            if state == "missing-model" {
                let other = ModelSelection.native(providerId: "sample", modelId: "visual-removed-model")
                store.modelCatalogs[value.contextRuntimeID] = .ready([.init(selection: other, name: "即将移除的模型", reasoningEfforts: [])])
                session.target.chooseModel(other)
                precondition(session.choices.choices[value.contextRuntimeID]?.model == other)
                store.modelCatalogs[value.contextRuntimeID] = .ready([])
                precondition(session.next?.chosenModelUnavailable == true)
            }
            if state == "missing-runtime" {
                precondition(session.next != nil)
                device.deleteRuntimeInstance(id: value.contextRuntimeID)
                precondition(session.next == nil, "Removed runtimes must not resolve an editor execution target")
                precondition(!session.canSubmit)
            }
            session.prepareVisualCheckState(state)
            if state == "failure-target-changed" {
                precondition(session.historyNotice != nil && session.failure != nil)
                precondition(session.noticeText == session.failure && session.noticeHelp == session.failure,
                             "回退后目标变化不能遮住尚未发送的真实失败详情")
            }
            inspect(workspace, session, state)
        }
    }

}
