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
    /// A device's last usable result for the current sidebar search.
    struct SearchRead {
        var results: (conversations: [Conversation], groups: [ConversationGroup])?
        var failure: String?
        var isLoading = false
    }
    @Published private(set) var searchText = ""
    @Published private(set) var searchRead = SearchRead()
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
    @Published private var snapshot: ConversationSnapshot?
    @Published private var executionStarting = false
    private var pendingSendCancellation: VeluneSendCancellation?
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
    /// Same-conversation refresh invalidates requests, not viewport demand.
    private var navigationID = UUID()
    private var timer: Timer?
    private var pollPending = false
    private var nativeUserMessageWakePending = false
    private var needsResync = false
    private var projectionEpoch: UInt64 = 0
    private let observationQueue = DispatchQueue(label: "local.velune.observation", qos: .userInitiated)
    private let payloadQueue = DispatchQueue(label: "local.velune.payloads", qos: .userInitiated)
    private let replyQueue = DispatchQueue(label: "local.velune.interaction", qos: .userInitiated)
    private let controlQueue = DispatchQueue(label: "local.velune.control", qos: .userInitiated, attributes: .concurrent)
    private var controlRequests: Set<String> = []
    private var replyAttempt: UUID?
    private var payloadPending = false
    /// Coalesced IDs are bounded by the current projection, not by request count.
    private var payloadDemand: Set<String> = []
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
            transcript.showSubmission(outgoing.message)
            objectWillChange.send()
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
        device.nativeUserMessageWakes.sink { [weak self] conversationID in
            self?.readNativeUserMessage(for: conversationID)
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
        for id in Set(interactionDrafts.keys.map(\.conversationID)).intersection(device.deletedConversationIDs) { discardInteractionDrafts(conversationID: id) }
        if let loadedConversationID, device.deletedConversationIDs.contains(loadedConversationID) { resetProjection() }
        if device.listRevision != seenListRevision {
            seenListRevision = device.listRevision
            // Organization changes the list's own rows; reached ones follow them.
            let listed = Dictionary(device.conversations.map { ($0.id, $0) }, uniquingKeysWith: { first, _ in first })
            offListConversations = offListConversations.map { listed[$0.id] ?? $0 }
            if !searchText.isEmpty { refreshSearch() }
            let selected = selectedConversationIDs.filter { conversation(id: $0) != nil }
            if selected != selectedConversationIDs { selectedConversationIDs = selected }
            // A later successful list can finish startup after the first read failed.
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
    var loadedConversationRuntimeID: String? { snapshot?.conversation.runtimeID }
    var loadedConversationWorkingDirectory: String? { snapshot?.conversation.cwd }
    var loadedConversationProject: NewConversationProject? { isDraft ? snapshot?.conversation.creationProject : nil }
    /// Set while a native sub-agent's conversation is shown: the conversation that started it.
    var loadedParentConversationID: String? { snapshot?.conversation.parentConversationID }
    var recordedChildModelName: String? {
        guard let snapshot, snapshot.conversation.parentConversationID != nil else { return nil }
        return snapshot.transcriptTurns.compactMap(\.settings).last { $0.runtimeId == snapshot.contextRuntimeID }?.model
            ?? nextTurn?.modelName ?? snapshot.modelRecordKey
    }
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
        return device.conversations.first { $0.id == id } ?? searchRead.results?.conversations.first { $0.id == id } ?? offListConversations.first { $0.id == id }
    }
    var selectedConversationTitle: String? { selectedConversationID.flatMap { conversation(id: $0) }?.title ?? (snapshot?.conversation.id == selectedConversationID ? snapshot?.conversation.title : nil) }
    var isGenerating: Bool { snapshot?.runState == .running || snapshot?.runState == .stopping }
    var activity: TranscriptActivity? {
        guard let snapshot else { return nil }
        switch snapshot.runState {
        case .running, .elsewhere:
            return TranscriptActivity(text: Self.currentWork(snapshot.messages.last), isLive: true)
        case .stopping:
            return TranscriptActivity(text: "正在停止", isLive: false)
        case .idle, .failed:
            return nil
        }
    }
    var isBusy: Bool { isLoading || applicationIsGenerating || isShuttingDown || device.authenticationRunning }
    var canManageConversations: Bool { !isSideChat && !isShuttingDown }
    /// A running conversation keeps its native session until the turn ends.
    func isRunning(_ conversation: Conversation) -> Bool {
        device.isConversationRunning(conversation.id) || (loadedConversationID == conversation.id && executionStarting)
    }
    /// Tab chrome follows the device's live state even when this transcript is paused.
    var tabIsRunning: Bool {
        if isSideChat { return executionStarting || isGenerating }
        return executionStarting || selectedConversationID.map(device.isConversationRunning) == true
    }
    var tabIsWaiting: Bool {
        if isSideChat { return !pendingInteractions.isEmpty }
        return selectedConversationID.map(device.waitingConversationIDs.contains) == true
    }
    /// The core validates the selection again; an unread catalog does not block sending.
    var isStopping: Bool { stopRequested || pendingSendCancellation?.isCancelled() == true || snapshot?.runState == .stopping }
    private var activityBlocksSending: Bool {
        loadedConversationID.map(device.isIdleSubmissionBusy) == true
    }
    var inputAccess: InputAccess { snapshot?.actions.inputAccess ?? .editable }
    var inputSubmissionAllowed: Bool { inputAccess == .editable }
    /// Native child views have no draft of their own.
    var draftEditable: Bool { loadedParentConversationID == nil && !isShuttingDown }
    var inputLockMessage: String? {
        if inputAccess == .readOnly && loadedParentConversationID != nil {
            return "子 Agent 会话由父会话继续。"
        }
        return inputAccess.message
    }
    /// Early native turn activity revokes idle admission; background-only activity does not.
    var canSend: Bool { !executionStarting && !device.authenticationRunning && !isLoading && !isStopping && !activityBlocksSending && !isShuttingDown && (snapshot?.actions.canSend ?? false) }
    var canCancel: Bool { !isStopping && (pendingSendCancellation != nil || (snapshot?.runState == .running && (snapshot?.actions.canCancel ?? false))) }
    var pendingQueuedMessages: [QueuedMessage] { outgoing.queued }
    var hasQueuedMessages: Bool { !followUp.queue.isEmpty || !pendingQueuedMessages.isEmpty }
    var followUp: FollowUp { snapshot?.followUp ?? FollowUp() }
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
        guard loadedParentConversationID == nil else { return }
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
            let managed = device.gateway.providers.flatMap { provider in provider.models.map { RuntimeModel(selection: .managed(modelRecordKey: $0.recordKey), name: "\($0.displayName)（\(provider.name)）", reasoningEfforts: $0.reasoningLevels ?? []) } }
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
        guard loadedParentConversationID == nil, !isSideChat, let request = composerTargetRequest,
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
        guard loadedParentConversationID == nil else { return }
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
        if let value = snapshot, value.conversation.parentConversationID != nil {
            // A child has no next submission. Project only its recorded facts;
            // preparing a composer target would create persistent input state.
            let runtimeID = value.contextRuntimeID
            let typeID = device.runtimeInstances.first { $0.id == runtimeID }?.typeId
            let settings = value.transcriptTurns.compactMap(\.settings)
            var observed = resolveNextTurn(input: NextTurnInput(runtimeId: runtimeID,
                runtimeType: device.runtimeTypes.first { $0.id == typeID }, providers: device.gateway.providers,
                catalog: nil, turnSettings: settings, conversationRuntimeId: runtimeID,
                modelRecordKey: value.modelRecordKey, kept: nil, runtimeChoices: nil,
                newConversation: .chosen(choice: TurnChoice())))
            // The catalog constrains editable choices, not a recorded effort.
            observed.currentEffort = settings.last { $0.runtimeId == runtimeID }?.effort
            if nextTurn != observed { nextTurn = observed }
            if nextTurnRuntimeID != runtimeID { nextTurnRuntimeID = runtimeID }
            return
        }
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
        if loadedParentConversationID != nil { refreshNextTurn(); return }
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
        guard loadedParentConversationID == nil, !isPreview, !isShuttingDown, !request.runtimeID.isEmpty, let transport else { return }
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
        let brief = [runtime?.name ?? settings.runtimeId, settings.model].compactMap { $0 }.joined(separator: "，模型 ")
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

    /// Restored identities open independently of the sidebar's first scan.
    /// The default conversation and new drafts wait for the device's configuration.
    func start(loadFirstConversation: Bool = true, newConversation: Bool = false, opening conversationID: String? = nil) {
        guard !started else { return }; started = true; composerTargetClosed = false
        if let target = sideChatContext, let transport, let lease = sideChatLease {
            nextTurnRuntimeID = target.runtimeID
            nextTurn = target.nextTurn
            let scope = projectionScope()
            enqueue({
                let result = try transport.openSideChat(runtimeID: target.runtimeID, conversationID: target.conversationID,
                    selection: target.selection, effort: target.effort, permissionMode: target.permissionMode)
                guard let id = result.snapshot?.conversation.id else { throw TransportError.rejected("运行时未返回侧聊") }
                lease.hold(id)
                return result
            }, onFailure: { [weak self] error in self?.openingFailure = AppProblem.failure(error, source: "打开侧聊") }) { [weak self] result in
                if let self {
                    var opening = scope
                    opening.destination = result.snapshot?.conversation.id
                    self.applySnapshotResult(result, scope: opening)
                }
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
            return
        }
        guard newConversation || loadFirstConversation else { return }
        initialSelection = newConversation ? .draft : .firstOrDraft
        let openingGeneration = generation
        device.whenInitialConfigurationReady { [weak self] result in
            guard let self, self.started, !self.isShuttingDown, self.generation == openingGeneration,
                  self.initialSelection != nil else { return }
            switch result {
            case .success: self.applyInitialSelectionIfReady()
            case .failure(let error): self.recordProblem(error, source: "读取初始会话列表")
            }
        }
    }

    private func applyInitialSelectionIfReady() {
        guard let selection = initialSelection, device.listRevision > 0 else { return }
        initialSelection = nil
        guard started, !isShuttingDown, snapshot == nil, pendingConversationID == nil,
              linkedConversationRequest == nil else { return }
        if selection == .firstOrDraft, let id = sidebarConversations.first?.id {
            selectConversation(id: id)
        } else {
            createConversation()
        }
    }
    /// The tab closes: it stops following its conversation.
    func close() {
        isDeletingMessage = false
        started = false
        composerTargetClosed = true
        composerTargetRequestID = nil
        pendingComposerRuntime = nil
        deferredSelection = nil
        clearInitialOpening()
        abandonLinkedConversationLookup()
        generation += 1
        navigationID = UUID()
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
                guard let self, ConversationWindowModel.isConversationStoreVisible(self) else { return }
                if self.composerTargetFailure != nil { self.requestComposerTarget() }
                self.poll()
            }
        }
    }
    func noteFocused() {
        // Device-level navigation and notifications address regular conversations.
        guard !isSideChat else { return }
        device.focusedConversation = self; noteSeen()
    }
    /// Selecting a pane or sheet should refresh a transcript that was paused
    /// while hidden without waiting for the next timer tick.
    /// Queue acceptance is independent of the tab's current editor. Apply its
    /// projection only while this view still presents the same source, and do
    /// not roll a newer delivery back to the acknowledgement's older cursor.
    func acceptContextPreparationQueue(_ result: SnapshotResult, conversationID: String, sourceContext: String) {
        guard loadedConversationID == conversationID, projectionRuntimeID == sourceContext else { return }
        // This external receipt has no captured request base. Only a comparable
        // delivery is installable; otherwise an independent authority read wins.
        guard let incoming = result.snapshot.map(BindingMapping.snapshot),
              let current = snapshot?.cursor, let cursor = incoming.cursor,
              incoming.contextRuntimeID == sourceContext, current.generation == cursor.generation else { requestProjectionResync(); return }
        guard cursor.sequence > current.sequence else {
            if cursor.sequence == current.sequence { requestProjectionResync() }
            return
        }
        installProjection(incoming, scope: projectionScope(), preserveSelection: true)
    }
    func cancelContextPreparation() {
        guard let context = followUp.contextPreparation, context.isPreparing || context.isDispatching, let transport else { return }
        requestControl("preparation", operation: { _ = try transport.cancelHandoffSummary(workID: context.workId); return nil })
    }
    func noteVisible() {
        guard started, ConversationWindowModel.isConversationStoreVisible(self) else { return }
        poll()
    }
    /// The open conversation is read once it shows in the focused tab of the
    /// active application.
    private func noteSeen() {
        guard !isSideChat, !isPreview, device.focusedConversation === self, NSApp?.isActive == true,
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
    var canReloadFromRuntimes: Bool { !isSideChat && !isPreview && !isLoading && !isShuttingDown && !executionStarting && pendingConversationID == nil }
    /// Runtimes own their sessions and can change them outside Velune (another
    /// client, a turn still running there). The core follows local native files
    /// best-effort; this rereads the open conversation for whatever that misses.
    /// Refresh also resamples control and
    /// composer facts without stopping execution or replacing local drafts.
    func reloadFromRuntimes() {
        guard canReloadFromRuntimes else { return }
        if canRetryFailedOpening { retryFailedOpening(); return }
        guard let transport else { return }
        let open = isDraft ? nil : snapshot?.conversation
        reloadProjection { try open.map { try transport.refreshConversation(runtimeID: $0.runtimeID, conversationID: $0.id) } }
    }
    /// Refresh invalidates older RPC results without abandoning this viewport.
    private func reloadProjection(_ read: @escaping () throws -> SnapshotResult?) {
        generation += 1
        isDeletingMessage = false
        device.composerInputs.resumePolling()
        composerTargetRetry.succeeded()
        let scope = projectionScope()
        enqueue(read, onFailure: { [weak self] _ in
            self?.requestComposerTarget(refreshFacts: true)
        }) { [weak self] result in
            guard let self else { return }
            if let result { applySnapshotResult(result, scope: scope) }
            requestComposerTarget(refreshFacts: true)
        }
    }
    func reloadDeviceState() {
        guard canReloadFromRuntimes else { return }
        device.refreshRuntimeState()
        modelCatalogs.removeAll()
        modelCatalogRequests.removeAll()
        reloadFromRuntimes()
    }

    // MARK: - Working directory

    /// Git changes in the loaded conversation's working directory.
    func readWorkingChanges(_ apply: @escaping (Result<WorkingChanges, Error>) -> Void) {
        guard !isShuttingDown else { apply(.failure(TransportError.quitting)); return }
        guard let transport else { apply(.failure(TransportError.unavailable)); return }
        guard let conversation = snapshot?.conversation, !isDraft else {
            apply(.failure(TransportError.rejected("请先打开具有工作目录的会话。"))); return
        }
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
        guard !isShuttingDown else { apply(.failure(TransportError.quitting)); return }
        guard let transport else { apply(.failure(TransportError.unavailable)); return }
        guard let conversation = snapshot?.conversation, !isDraft else {
            apply(.failure(TransportError.rejected("请先打开具有工作目录的会话。"))); return
        }
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
        let query = text.trimmingCharacters(in: .whitespacesAndNewlines)
        guard query != searchText else { return }
        searchText = query
        searchRead = SearchRead()
        refreshSearch()
    }
    func retrySearch() {
        guard !searchRead.isLoading else { return }
        refreshSearch(retryConnection: true)
    }
    private func refreshSearch(retryConnection: Bool = false) {
        searchRequest += 1
        let request = searchRequest
        let text = searchText
        guard !text.isEmpty, !isPreview, !isShuttingDown else { return }
        guard let transport else {
            searchRead.failure = "暂时无法连接此设备。"
            return
        }
        searchRead.isLoading = true
        performOnQueue { [weak self] in
            if retryConnection { transport.retryOpen() }
            let result = Result { try transport.searchConversations(text) }
            DispatchQueue.main.async {
                guard let self, request == self.searchRequest else { return }
                switch result {
                case .success(let found):
                    self.searchRead = SearchRead(results: (found.conversations.map(BindingMapping.listed), found.groups))
                case .failure(let error):
                    let detail = AppProblem.failure(error, source: "搜索会话").detail
                    self.searchRead = SearchRead(results: self.searchRead.results, failure: detail)
                    self.recordProblem(error, source: "搜索会话")
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
        if retryable { openingRetry.failed() }
        openingFailure = AppProblem.failure(error, source: "打开会话")
    }
    var canRetryFailedOpening: Bool {
        !isSideChat && !isLoading && !isShuttingDown && !executionStarting && pendingConversationID == nil
            && reachingConversationID == nil && deferredSelection == nil && coldOpeningProblem != nil && unopenedConversationID != nil
    }
    /// A temporary failure is retried on its own (the history being read as
    /// the app starts); about the first 18 s of automatic retries show loading,
    /// not an error that clears itself. Later ones, and failures that wait for
    /// the user, show it.
    var retriesOpeningQuietly: Bool { coldOpeningProblem != nil && openingNeedsRetry && openingRetry.failures < 5 }
    var coldOpeningProblem: AppProblem? {
        loadedConversationID == nil && pendingConversationID == nil && reachingConversationID == nil ? openingFailure : nil
    }
    func retryFailedOpening() {
        guard canRetryFailedOpening, let id = unopenedConversationID else { return }
        openingNeedsRetry = false
        if let transport { performOnQueue { transport.retryOpen() } }
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
        guard !isPreview, !isShuttingDown else { abandonLinkedConversationLookup(); return }
        if snapshot == nil { unopenedConversationID = id }
        guard let transport else {
            abandonLinkedConversationLookup(); noteOpeningFailure(TransportError.unavailable); return
        }
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
    func rememberOffListConversation(_ conversation: Conversation) {
        offListConversations.removeAll { $0.id == conversation.id }
        offListConversations.append(conversation)
    }
    private func finishLinkedConversationLookup(_ result: Result<ListedConversation?, Error>, id: String, move: NavigationMove, request: UUID) {
        guard linkedConversationRequest == request else { return }
        abandonLinkedConversationLookup()
        switch result {
        case .success(let found?):
            rememberOffListConversation(BindingMapping.listed(found))
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
        if snapshot == nil { unopenedConversationID = id }
        guard let transport else {
            noteOpeningFailure(TransportError.unavailable); recordProblem("本地核心未配置"); return
        }
        // The sidebar reflects user intent immediately. The old loaded transcript
        // is retained until success, but hidden while this destination is loading.
        let previousSelection = snapshot?.conversation.id
        let previousWasDraft = isDraft
        // An unsent conversation is not one to go back to.
        let shown = isDraft ? nil : previousSelection
        isDraft = false
        generation += 1 // Invalidate any poll already queued for the old conversation.
        navigationID = UUID()
        selectedConversationID = id
        pendingConversationID = id
        composerEvents.send(.conversation(id))
        openingFailure = nil
        enqueueSnapshot({ try transport.openConversation(runtimeID: runtimeID, conversationID: id) }, scope: projectionScope(destination: id), onFailure: { [weak self] error in
            guard let self else { return }
            self.noteOpeningFailure(error)
            self.isDraft = previousWasDraft
            self.selectedConversationID = self.unopenedConversationID ?? previousSelection
            self.selectedConversationIDs = Set([self.selectedConversationID].compactMap { $0 })
            if let previousSelection { self.composerEvents.send(previousWasDraft ? .newConversation : .conversation(previousSelection)) }
            self.finishConversationLoading()
        }) { [weak self] _ in
            if let shown, shown != id { self?.recordNavigation(from: shown, move) }
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
        let contextID = snapshot?.conversation.runtimeID
        let scope = projectionScope()
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
                catch { failures.append(AppProblem.failure(error, source: "删除会话「\(target.title)」")) }
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
        }, completedIDs: { $0.1 }) { [weak self, weak device] result in
            device?.noteDeleted(result.1)
            if let data = result.0 { device?.applyList(data) }
            device?.appendProblems(result.2)
            guard let self else { return }
            selectedConversationIDs.subtract(result.1)
            if let loadedConversationID, result.1.contains(loadedConversationID) { resetProjection() }
            else if loadedConversationID == loadedID {
                if result.4 { requestProjectionResync() }
                else if let refreshed = result.3 { applySnapshotResult(refreshed, scope: scope) }
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
    /// Begins submission and immediately shows it in the message list.
    /// `onAccepted` receives the
    /// conversation that took the turn: sending in a new conversation replaces
    /// its unsent identity with the created one. `onFailure` runs when it was not
    /// sent and receives any identity already created by this operation.
    @discardableResult
    func send(text: String, attachments: [String] = [], next: NextTurn? = nil, input: ComposerModel? = nil,
              preparedHandoff: String? = nil, onAccepted: ((String?) -> Void)? = nil,
              onFailure: ((String?) -> Void)? = nil, onDetailedFailure: ((Error) -> Void)? = nil) -> Bool {
        guard !text.trimmingCharacters(in: .whitespacesAndNewlines).isEmpty || !attachments.isEmpty else { return false }
        guard allowsAttachments || attachments.isEmpty else { return false }
        guard canSend, let next = next ?? nextTurn, case let target = next.runtimeId, let source = snapshot?.conversation.runtimeID, let conversationID = loadedConversationID else { return false }
        if let project = snapshot?.conversation.creationProject, project.runtimeID != target {
            recordProblem("所选项目属于另一运行时，请重新选择项目或工作目录")
            return false
        }
        guard preparedHandoff == nil || (!isDraft && !isSideChat) else { return false }
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
        let cancellation = VeluneSendCancellation()
        pendingSendCancellation = cancellation
        let submission = outgoing.begin(text: text, attachments: attachments, sendingFrom: snapshot)
        let creating = isDraft
        let ticket = SendProjectionTicket(scope: projectionScope(), input: input, device: device)
        let temporary = isSideChat
        let cwd = snapshot?.conversation.cwd
        let project = snapshot?.conversation.creationProject
        // Without a level or mode the core keeps the conversation's current one.
        let turn = ComposedTurn(text: text, effort: next.effort, permissionMode: next.permissionMode?.id, attachments: attachments)
        enqueue({
            if cancellation.isCancelled() { throw TransportError.submissionCancelled }
            if temporary { return try transport.sendSideChat(sideChatID: conversationID, runtimeID: target, selection: model, turn: turn, cancellation: cancellation) }
            if creating {
                return try transport.createAndSend(runtimeID: target, cwd: cwd, project: project?.key, selection: model, turn: turn, cancellation: cancellation, submissionId: submission) { [weak self] created in
                    DispatchQueue.main.async {
                        self?.publishCreatedIdentity(created, ticket: ticket, previous: conversationID)
                    }
                }
            }
            if let preparedHandoff {
                return try transport.sendTurnWithPreparedHandoff(sourceRuntimeID: source, conversationID: conversationID,
                                                                 targetRuntimeID: target, selection: model, turn: turn,
                                                                 preparedToken: preparedHandoff, cancellation: cancellation)
            }
            return try transport.sendTurn(sourceRuntimeID: source, conversationID: conversationID, targetRuntimeID: target, selection: model, turn: turn, cancellation: cancellation, submissionId: submission)
        // Retain this store through acknowledgement: closing its tab after a
        // cancellation request must not lose the eventual native interrupt.
        }, consumeReceipt: { [self] result in
            let acceptedID = ticket.createdID ?? conversationID
            pendingSendCancellation = nil
            executionStarting = false
            composerRequest = nil
            switch result {
            case .success(let value):
                acceptSubmission(submission, receipt: value)
                if cancellation.isCancelled(), let operation = OutgoingSubmissions.queuedCancellation(value.submissionReceipt) {
                    // No runtime received this message. Remove only its queue
                    // identity, then recover the original revision-bound input.
                    stopRequested = false
                    let removing = requestControl("execution", operation: {
                        try transport.sendQueue(runtimeID: source, conversationID: acceptedID, operation: operation)
                    }, consumeReceipt: { [weak self] result in
                        switch result {
                        case .success: onFailure?(ticket.createdID)
                        case .failure:
                            self?.acceptedSubmissions &+= 1
                            onAccepted?(acceptedID)
                        }
                    })
                    if !removing {
                        acceptedSubmissions &+= 1; onAccepted?(acceptedID)
                        recordProblem("消息已排队；取消操作暂未执行，请在发送队列中删除该消息。")
                    }
                } else {
                    acceptedSubmissions &+= 1
                    onAccepted?(acceptedID)
                    if cancellation.isCancelled() {
                        stopRequested = true
                        requestControl("execution", operation: { try transport.cancel(runtimeID: target, conversationID: acceptedID) }, onFailure: { [weak self] in self?.stopRequested = false })
                    }
                }
            case .failure(let error):
            if AppProblem.failure(error).code == "message_delivery_accepted" {
                // Delivery was accepted even though the projection failed.
                // Honour a pending stop before a running snapshot arrives.
                self.outgoing.accept(submission, receipt: nil, hasSnapshot: false, submittedMessageID: nil)
                self.acceptedSubmissions &+= 1
                onAccepted?(acceptedID)
                if generation == ticket.scope.storeGeneration { requestProjectionResync() }
                if cancellation.isCancelled() {
                    self.stopRequested = true
                    self.requestControl("execution", operation: { try transport.cancel(runtimeID: target, conversationID: acceptedID) },
                                        onFailure: { [weak self] in self?.stopRequested = false })
                }
                return
            }
            let cancelled = (error as? TransportError)?.isSubmissionCancelled == true
            if creating, ticket.createdID == nil, !cancelled, self.generation == ticket.scope.storeGeneration { self.creationFailure = AppProblem.failure(error).detail }
            self.stopRequested = false
            if self.outgoing.fail(submission) {
                onFailure?(ticket.createdID)
                onDetailedFailure?(error)
            } else {
                // A poll has already confirmed admission with this token.
                self.acceptedSubmissions &+= 1
                onAccepted?(acceptedID)
            }
            }
        }) { [weak self] value in
            self?.applySnapshotResult(value, scope: ticket.scope)
        }
        return true
    }
    private func acceptSubmission(_ id: String, receipt: SnapshotResult) {
        outgoing.accept(id, receipt: receipt.submissionReceipt, hasSnapshot: receipt.snapshot != nil, submittedMessageID: receipt.submittedMessageId)
        // A newer poll may already have installed the echo before this receipt.
        if let snapshot { outgoing.reconcile(snapshot, afterReceipt: true) }
    }
    /// Stop the sub-agents the loaded conversation started; they may run after its turn.
    func stopSubAgents() {
        guard followUp.canStopSubAgents, !isPreview else { return }
        guard let runtimeID = projectionRuntimeID, let conversationID = loadedConversationID else { recordProblem("请选择会话"); return }
        guard let transport else { recordProblem("本地核心未配置"); return }
        requestControl("execution", operation: { try transport.stopSubAgents(runtimeID: runtimeID, conversationID: conversationID) })
    }
    func setConversationHandoff(release: Bool) {
        let handoff = controlHandoff
        guard (release ? handoff == .canRelease : handoff == .releasing || handoff == .released),
              !isLoading, !isPreview,
              let transport, let runtimeID = projectionRuntimeID, let conversationID = loadedConversationID else { return }
        enqueueSnapshot({ try transport.setConversationHandoff(runtimeID: runtimeID, conversationID: conversationID, release: release) }, scope: projectionScope())
    }
    func cancel() {
        guard canCancel else { return }
        if let pendingSendCancellation {
            pendingSendCancellation.cancel()
            stopRequested = true
            return
        }
        guard !isLoading else { return }
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
        requestControl("execution", operation: { try transport.cancel(runtimeID: runtimeID, conversationID: conversationID) }, onFailure: { [weak self] in self?.stopRequested = false })
    }
    /// Queue a message after the running turn, with the next-turn choices as
    /// they are now. `onFailure` runs when it was not queued.
    @discardableResult
    func queue(text: String, attachments: [String] = [], next: NextTurn? = nil, onAccepted: (() -> Void)? = nil, onFailure: (() -> Void)? = nil) -> Bool {
        guard !text.trimmingCharacters(in: .whitespacesAndNewlines).isEmpty || !attachments.isEmpty else { return false }
        guard canQueue, let next = next ?? nextTurn, case let target = next.runtimeId, let source = snapshot?.conversation.runtimeID else { return false }
        let turn = ComposedTurn(text: text, effort: next.effort, permissionMode: next.permissionMode?.id, attachments: attachments)
        let submission = outgoing.begin(text: text, attachments: attachments)
        composerRequest = .queueing
        changeQueue(.add(sourceRuntimeId: source, targetRuntimeId: target, selection: next.model, turn: turn), submissionId: submission, onFailure: { [self] in
            composerRequest = nil
            if outgoing.fail(submission) { onFailure?() }
            else { acceptedSubmissions &+= 1; onAccepted?() }
        }) { [self] receipt in
            composerRequest = nil
            acceptSubmission(submission, receipt: receipt); acceptedSubmissions &+= 1
            onAccepted?()
        }
        return true
    }
    func compactContext(next: NextTurn? = nil) {
        guard canCompact(next: next ?? nextTurn), let runtimeID = snapshot?.contextRuntimeID, let conversationID = loadedConversationID, let transport else { return }
        let selection = (next ?? nextTurn)?.model
        executionStarting = true
        enqueueSnapshot({ try transport.compactConversation(runtimeID: runtimeID, conversationID: conversationID, selection: selection) }, scope: projectionScope(), onFailure: { [weak self] _ in self?.executionStarting = false }) { [weak self] _ in
            self?.executionStarting = false
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
        enqueueSnapshot({ try transport.resumeTurn(runtimeID: runtimeID, conversationID: conversationID, selection: selection, effort: effort, permissionMode: mode) }, scope: projectionScope(),
                consumeReceipt: { [weak self] result in if case .success = result { self?.acceptedSubmissions &+= 1 } },
                onFailure: { [weak self] _ in self?.executionStarting = false; self?.composerRequest = nil }) { [weak self] _ in
            self?.executionStarting = false
            self?.composerRequest = nil
        }
    }
    var canSteer: Bool { canQueue && followUp.canSteer }
    /// Into the running turn, with an optional explicit permission change.
    @discardableResult
    func steer(text: String, attachments: [String] = [], next: NextTurn? = nil, onAccepted: (() -> Void)? = nil) -> Bool {
        guard canSteer, let runtimeID = projectionRuntimeID,
              let conversationID = loadedConversationID, let transport else { return false }
        let next = next ?? nextTurn
        let turn = ComposedTurn(text: text, effort: nil, permissionMode: next?.choice.permissionMode, attachments: attachments)
        let submission = outgoing.begin(text: text, attachments: attachments, sendingFrom: snapshot)
        let scope = projectionScope()
        composerRequest = .sending
        enqueueSnapshot({
            try transport.steerTurn(runtimeID: runtimeID, conversationID: conversationID, turn: turn,
                                    targetRuntimeID: next?.runtimeId)
        }, scope: scope, consumeReceipt: { [self] result in
            if generation == scope.storeGeneration { composerRequest = nil }
            switch result {
            case .success(let value): acceptSubmission(submission, receipt: value)
            case .failure(let error):
                if AppProblem.failure(error).code == "message_delivery_accepted" {
                    outgoing.accept(submission, receipt: nil, hasSnapshot: false, submittedMessageID: nil)
                } else if outgoing.fail(submission) {
                    return
                }
            }
            acceptedSubmissions &+= 1
            onAccepted?()
        })
        return true
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
        let scope = projectionScope()
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
            self?.applySnapshotResult(value, scope: scope)
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
    @Published private(set) var isDeletingMessage = false

    func canDeleteMessage(_ id: String) -> Bool {
        guard !composerTargetClosed, !isShuttingDown, !isLoading, !isForking, !isDeletingMessage, !isGenerating, let choice = editability(of: id) else { return false }
        return choice.deleteOnly || choice.deleteFollowing
    }

    func confirmDeleteMessage(_ id: String) {
        guard canDeleteMessage(id), let choice = editability(of: id), let revision = choice.revision,
              let runtimeID = snapshot?.conversation.runtimeID, let conversationID = loadedConversationID,
              let transport else { return }
        let currentGeneration = generation
        let alert = NSAlert()
        alert.alertStyle = .warning
        alert.messageText = "删除这条用户消息？"
        alert.informativeText = "仅删除此条会保留后续回复和工具记录。删除此条及之后会截断后续历史。原始会话保留可恢复副本；此操作不会生成新回复。"
        var modes: [Bool] = []
        if choice.deleteOnly { alert.addButton(withTitle: "仅删除此条"); modes.append(false) }
        if choice.deleteFollowing { alert.addButton(withTitle: "删除此条及之后"); modes.append(true) }
        alert.addButton(withTitle: "取消")
        let index = alert.runModal().rawValue - NSApplication.ModalResponse.alertFirstButtonReturn.rawValue
        guard modes.indices.contains(index), generation == currentGeneration,
              loadedConversationID == conversationID, snapshot?.conversation.runtimeID == runtimeID,
              canDeleteMessage(id), editability(of: id)?.revision == revision else { return }
        isDeletingMessage = true
        enqueueSnapshot({ try transport.deleteUserMessage(runtimeID: runtimeID, conversationID: conversationID, messageID: id, truncate: modes[index], expectedRevision: revision) }, scope: projectionScope(),
                onFailure: { [weak self] _ in self?.isDeletingMessage = false }) { [weak self] value in
            guard let self, generation == currentGeneration, !composerTargetClosed else { return }
            isDeletingMessage = false
        }
    }

    /// A fork runs independently; only an in-place replacement stops the
    /// source turn, as the user confirmed in the editor.
    func editMessage(_ messageID: String, placement: EditPlacement, text: String, attachments: [String], choices: ConversationChoices? = nil, expectedRevision: String? = nil, forked: @escaping (SnapshotResult) -> Void, accepted: @escaping () -> Void, failed: ((String, String?) -> Void)? = nil) {
        guard editability(of: messageID) != nil, let runtimeID = snapshot?.conversation.runtimeID, let conversationID = loadedConversationID else {
            failed?("原消息暂时不能编辑，修改仍保留。", nil)
            return
        }
        let kept = choices ?? messageEditingChoices(messageID)
        let editNext = resolve(kept: kept, editingMessageID: messageID)
        let model = editNext?.model
        guard let transport else { failed?("执行端未连接，修改仍保留。", nil); return }
        guard !isLoading, !isForking, !isShuttingDown else { failed?("请等待当前操作完成后重试。", nil); return }
        let inputs = device.composerInputs
        guard let targetChoices = messageEditingTargetChoices(kept, base: placement == .inPlace ? composerTargetChoices : nil) else {
            failed?("执行配置尚未就绪，请稍后重试。", nil); return
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
        let scope = projectionScope()
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
            case .success(let value):
                if placement == .fork {
                    self?.device.noteForkCompleted()
                    forked(value)
                }
                accepted()
            case .failure(let error):
                let problem = AppProblem.failure(error)
                failed?(problem.detail, problem.code)
            }
        }) { [weak self] value in
            if placement == .inPlace { self?.applySnapshotResult(value, scope: scope) }
        }
    }
    /// Copy the completed prefix immediately, even while the source runs.
    /// The core creates a separate native execution and leaves this one alone.
    func forkConversation(through messageID: String, forked: @escaping (SnapshotResult) -> Void) {
        guard canFork, let runtimeID = snapshot?.conversation.runtimeID, let conversationID = loadedConversationID, let transport else { return }
        isForking = true
        enqueue({ try transport.forkConversation(runtimeID: runtimeID, conversationID: conversationID, messageID: messageID) },
                consumeReceipt: { [weak self] _ in self?.isForking = false }) { [weak self] value in
            guard let self else { return }
            self.device.noteForkCompleted()
            forked(value)
        }
    }
    /// Show a conversation the core has already opened, as `result` gives it.
    func adopt(_ result: SnapshotResult) { applySnapshotResult(result, scope: projectionScope(destination: result.snapshot?.conversation.id)) }
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
        enqueueSnapshot({ try transport.openConversation(runtimeID: creation.runtimeId, conversationID: creation.conversationId) }, scope: projectionScope(destination: creation.conversationId), completion: { [weak self] _ in
            self?.finishConversationLoading()
        })
    }
    func steerQueued(_ id: String) {
        guard followUp.queue.contains(where: { $0.id == id }) else { return }
        changeQueue(.steer(id: id))
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
        }, scope: projectionScope(), consumeReceipt: { result in
            switch result {
            case .success: completion(nil)
            case .failure(let error):
                let problem = AppProblem.failure(error, source: "编辑排队消息")
                completion([problem.detail, problem.operationID.map { "操作编号 \($0)" }].compactMap { $0 }.joined(separator: "\n"))
            }
        })
    }
    func removeQueued(_ id: String) { changeQueue(.remove(id: id)) }
    func withdrawQueued(_ message: QueuedMessage) {
        guard let revision = message.editContext?.revision,
              followUp.queue.contains(where: { $0.id == message.id }) else { return }
        let inputs = device.composerInputs
        changeQueue(.withdraw(id: message.id, expectedRevision: revision),
                    onFailure: { inputs.poll(force: true) },
                    onDone: { _ in inputs.poll(force: true) })
    }
    func moveQueued(_ id: String, to index: Int) { changeQueue(.move(id: id, index: UInt32(max(index, 0)))) }
    func resumeQueue() { guard inputSubmissionAllowed else { return }; changeQueue(.resume) }
    private func changeQueue(_ operation: SendQueueOperation, submissionId: String? = nil, onFailure: (() -> Void)? = nil, onDone: ((SnapshotResult) -> Void)? = nil) {
        guard !isPreview else { return }
        guard let runtimeID = projectionRuntimeID, let conversationID = loadedConversationID else { recordProblem("请选择会话"); onFailure?(); return }
        guard let transport else { recordProblem("本地核心未配置"); onFailure?(); return }
        enqueueSnapshot({ try transport.sendQueue(runtimeID: runtimeID, conversationID: conversationID, operation: operation, submissionId: submissionId) }, scope: projectionScope(), consumeReceipt: { result in
            switch result { case .success(let receipt): onDone?(receipt); case .failure: onFailure?() }
        })
    }
    struct InteractionDraftKey: Hashable { let conversationID: String; let interactionID: String }
    private var interactionDrafts: [InteractionDraftKey: RuntimeInteractionDraft] = [:]
    func interactionDraft(for interaction: RuntimeInteraction) -> RuntimeInteractionDraft {
        // SwiftUI may evaluate the outgoing form once more during navigation
        // or removal. It must not recreate a resolved draft or attach that old
        // form's answers to the newly selected conversation.
        guard pendingInteractions.contains(interaction) else { return RuntimeInteractionDraft() }
        let key = InteractionDraftKey(conversationID: loadedConversationID ?? "", interactionID: interaction.id)
        if let draft = interactionDrafts[key] { return draft }
        let draft = RuntimeInteractionDraft(interaction: interaction)
        interactionDrafts[key] = draft
        return draft
    }
    @Published private var interactionReplyFailures: [InteractionDraftKey: String] = [:]
    @Published private var interactionReplyInFlight: (key: InteractionDraftKey, title: String, attempt: UUID, interaction: RuntimeInteraction)?
    func interactionReplyStatus(_ id: String) -> String? {
        guard let conversationID = loadedConversationID,
              let flight = interactionReplyInFlight, pendingInteractions.contains(flight.interaction),
              flight.key == InteractionDraftKey(conversationID: conversationID, interactionID: id) else { return nil }
        return interactionReplyInFlight?.title
    }
    func interactionReplyFailure(_ id: String) -> String? {
        guard let conversationID = loadedConversationID else { return nil }
        return interactionReplyFailures[InteractionDraftKey(conversationID: conversationID, interactionID: id)]
    }
    private func discardInteractionDrafts(conversationID: String) {
        if interactionReplyInFlight?.key.conversationID == conversationID { interactionReplyInFlight = nil }
        for (key, draft) in interactionDrafts where key.conversationID == conversationID { draft.clear() }
        interactionDrafts = interactionDrafts.filter { $0.key.conversationID != conversationID }
        interactionReplyFailures = interactionReplyFailures.filter { $0.key.conversationID != conversationID }
    }
    var pendingInteractions: [RuntimeInteraction] { snapshot?.pendingInteractions ?? [] }
    func canReply(to interaction: RuntimeInteraction) -> Bool {
        !isShuttingDown && !composerTargetClosed && pendingConversationID == nil && replyAttempt == nil
            && pendingInteractions.contains(interaction)
    }
    func replyInteraction(_ interaction: RuntimeInteraction, reply: RuntimeInteractionReply) {
        guard canReply(to: interaction),
              let runtimeID = projectionRuntimeID, let conversationID = loadedConversationID, let transport else { return }
        let scope = projectionScope(), attempt = UUID()
        let key = InteractionDraftKey(conversationID: conversationID, interactionID: interaction.id)
        interactionReplyFailures[key] = nil
        let title: String
        switch reply {
        case .answers: title = "正在提交回答"
        case .decision: title = "正在提交决定"
        case .cancel: title = "正在取消请求"
        }
        replyAttempt = attempt
        interactionReplyInFlight = (key, title, attempt, interaction)
        replyQueue.async { [weak self] in
            let result = Result { try transport.replyRuntimeInteraction(runtimeID: runtimeID, conversationID: conversationID, interactionID: interaction.id, reply: reply) }
            DispatchQueue.main.async {
                self?.finishInteractionReply(result, interaction: interaction, key: key, scope: scope, attempt: attempt)
            }
        }
    }
    private func finishInteractionReply(_ result: Result<SnapshotResult, Error>, interaction: RuntimeInteraction,
                                        key: InteractionDraftKey, scope: ProjectionScope, attempt: UUID) {
        if replyAttempt == attempt { replyAttempt = nil }
        guard interactionReplyInFlight?.attempt == attempt else { return }
        interactionReplyInFlight = nil
        guard generation == scope.storeGeneration, !composerTargetClosed,
              loadedConversationID == key.conversationID else { return }
        switch result {
        case .success(let value): applySnapshotResult(value, scope: scope)
        case .failure(let error):
            guard pendingInteractions.contains(interaction) else { return }
            interactionReplyFailures[key] = error.localizedDescription
            recordProblem(error)
        }
    }

    // MARK: - Session options

    /// An isolated UI check can inspect the next-turn menu without opening a Harness.
    static func modelAutoCompactionCheckTab(device: DeviceStore, next: NextTurn, options: [AcpConfigOption]? = nil) -> ConversationTab {
        let store = ConversationStore(device: device)
        store.snapshot = ConversationSnapshot(revision: 1, conversation: Conversation(id: "synthetic-compaction-menu",
            title: "合成模型菜单", updatedAtUnixMs: nil, runtimeID: next.runtimeId, cwd: nil, canRename: false, canDelete: false),
            contextRuntimeID: next.runtimeId, modelRecordKey: nil, runState: .idle, messages: [],
            actions: ConversationActions(inputAccess: .editable, canSend: true, canCancel: false, canSwitch: true))
        store.nextTurn = next
        store.sessionOptions = options
        return ConversationTab(store: store, start: {})
    }

    /// The option the composer's model menu already offers: the session's
    /// first model-category option, which the runtime's catalog is read from.
    var editableSessionOptions: [AcpConfigOption] {
        guard var options = sessionOptions else { return [] }
        if let model = options.firstIndex(where: { $0.category == "model" }) { options.remove(at: model) }
        return options
    }
    var canChangeSessionOptions: Bool { inputSubmissionAllowed && !isGenerating && !executionStarting && !isLoading && !isShuttingDown }
    private var acpCompactionTarget: (runtime: RuntimeInstance, model: ModelSelection)? {
        guard let runtimeID = snapshot?.contextRuntimeID,
              let runtime = device.runtimeInstances.first(where: { $0.id == runtimeID && $0.typeId == .acp }),
              let sessionOptions, let model = acpSessionModel(options: sessionOptions) else { return nil }
        return (runtime, model)
    }
    var acpAutoCompactionOptions: [AcpConfigOption] {
        guard acpCompactionTarget != nil else { return [] }
        return editableSessionOptions.filter { canBindAcpAutoCompactionOption(option: $0) }
    }
    var acpAutoCompactionSetting: AutoCompactionSetting? {
        guard let target = acpCompactionTarget,
              let entries = try? runtimeModelAutoCompactions(runtime: target.runtime) else { return nil }
        return entries.first { $0.model == target.model }?.setting
    }
    func acpAutoCompactionModel(runtimeID: String, selectedModel: ModelSelection?) -> ModelSelection? {
        guard let target = acpCompactionTarget, target.runtime.id == runtimeID,
              selectedModel == nil || target.model == selectedModel else { return nil }
        return target.model
    }
    func saveAcpAutoCompaction(_ setting: AutoCompactionSetting?, runtimeID: String, model: ModelSelection) {
        guard canChangeSessionOptions, let target = acpCompactionTarget,
              target.runtime.id == runtimeID, target.model == model else { return }
        if let setting {
            guard case let .acp(optionID, value) = setting,
                  acpAutoCompactionOptions.contains(where: {
                      $0.id == optionID && acpAutoCompactionAccepts(option: $0, value: value)
                  }) else { return }
        }
        device.saveRuntimeModelAutoCompaction(runtimeID: target.runtime.id, model: target.model, setting: setting) {
            [weak self] in self?.objectWillChange.send()
        }
    }
    func setSessionOption(_ option: AcpConfigOption, value: AcpConfigValue) {
        guard canChangeSessionOptions, value != option.value, let runtimeID = snapshot?.contextRuntimeID, let conversationID = loadedConversationID, let transport else { return }
        enqueue({ try transport.setSessionOption(runtimeID: runtimeID, conversationID: conversationID, optionID: option.id, value: value) }) { [weak self] options in
            guard let self, self.loadedConversationID == conversationID, self.snapshot?.contextRuntimeID == runtimeID else { return }
            self.sessionOptions = options
        }
    }
    /// Options exist only while the conversation's session is live here, so
    /// they are read again when the conversation or its run state changes;
    /// not being live yet is not a problem to report.
    private func refreshSessionOptions() {
        guard let value = snapshot, !isDraft, !isPreview, let transport,
              let typeID = device.runtimeInstances.first(where: { $0.id == value.contextRuntimeID })?.typeId,
              device.runtimeTypes.first(where: { $0.id == typeID })?.sessionOptions == true else {
            if sessionOptions != nil { sessionOptions = nil }; sessionOptionsKey = nil; return
        }
        let prefix = "\(value.conversation.id)|\(value.contextRuntimeID)|"
        let key = prefix + "\(value.runState)"
        guard key != sessionOptionsKey else { return }
        if sessionOptionsKey?.hasPrefix(prefix) != true { sessionOptions = nil }
        sessionOptionsKey = key
        let runtimeID = value.contextRuntimeID, conversationID = value.conversation.id
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

    private func resetProjection() {
        nativeUserMessageWakePending = false
        if let loadedConversationID { discardInteractionDrafts(conversationID: loadedConversationID) }
        projectionEpoch &+= 1
        payloadDemand.removeAll()
        initialSelection = nil
        composerTargetRetry = PollBackoff(); composerRequest = nil; stopRequested = false; executionStarting = false; preparedComposerTarget = nil; composerTargetRequestID = nil; nextTurn = nil; nextTurnRuntimeID = nil; loadedPayloads.removeAll(); isDraft = false; generation += 1; snapshot = nil; projectionRuntimeID = nil; selectedConversationID = nil; pendingConversationID = nil; abandonLinkedConversationLookup(); transcript.reset(); outgoing.reset(); sessionOptions = nil; sessionOptionsKey = nil
    }
    private func apply(_ value: ConversationSnapshot?, preserveSelection: Bool = false, coreDelivery: Bool = true) {
        guard var value else { return }
        initialSelection = nil
        if coreDelivery { projectionEpoch &+= 1 }
        if value.conversation.id != snapshot?.conversation.id || value.contextRuntimeID != snapshot?.contextRuntimeID || value.cursor?.generation != snapshot?.cursor?.generation {
            loadedPayloads.removeAll()
        }
        if value.conversation.id != snapshot?.conversation.id { payloadDemand.removeAll(); navigationID = UUID() }
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
        payloadDemand.formIntersection(value.messages.filter(\.hasDeferredPayload).map(\.id))
        if coreDelivery { outgoing.reconcile(value) }
        guard snapshot != value else { return }
        for (key, draft) in interactionDrafts where key.conversationID == value.conversation.id && !value.pendingInteractions.contains(where: { $0.id == key.interactionID }) {
            draft.clear()
        }
        interactionDrafts = interactionDrafts.filter { key, _ in
            key.conversationID != value.conversation.id || value.pendingInteractions.contains { $0.id == key.interactionID }
        }
        interactionReplyFailures = interactionReplyFailures.filter { key, _ in
            key.conversationID != value.conversation.id || value.pendingInteractions.contains { $0.id == key.interactionID }
        }
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
        if isPreview { device.keepPreviewSnapshot(value) }
        refreshSessionOptions()
    }
    /// What a running turn is doing, from the newest block of its last
    /// message: a reasoning summary, the running tool, or replying.
    private static func currentWork(_ message: DeliveredMessage?) -> String {
        guard let message, let block = message.blocks.last else { return "正在思考" }
        if case .runtimeFailure(let failure, _, let retry?) = block {
            return failure.title + "，" + retry.label
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
    /// Cursors order only one delivery key within one opaque Core generation.
    /// Epoch makes nil cursors and equal-cursor header changes safe CAS bases.
    struct ProjectionScope {
        let storeGeneration: Int
        let navigationID: UUID
        let source: String?
        let conversationID: String?
        let context: String?
        let cursor: TranscriptCursor?
        let epoch: UInt64
        var destination: String?
    }
    /// Only onCreated advances this operation's base; final Send never samples
    /// a fresh base after an intervening poll.
    @MainActor final class SendProjectionTicket {
        var scope: ProjectionScope
        private(set) var createdID: String?
        let input: ComposerModel?
        private let device: DeviceStore?
        init(scope: ProjectionScope, input: ComposerModel? = nil, device: DeviceStore? = nil) {
            self.scope = scope; self.input = input; self.device = device
        }
        func consumeCreated(_ identity: ConversationSummary) {
            guard createdID == nil else { return }
            createdID = identity.id
            if let input { device?.composerInputs.promote(input, to: identity.id, runtimeID: identity.runtimeId) }
        }
    }
    private func publishCreatedIdentity(_ created: SnapshotResult, ticket: SendProjectionTicket, previous: String) {
        guard let identity = created.snapshot?.conversation else { return }
        // Creation is a business fact, even after the tab/subscriber disappears.
        ticket.consumeCreated(identity)
        guard generation == ticket.scope.storeGeneration, !composerTargetClosed,
              baseIsCurrent(ticket.scope), isDraft else { return }
        var creation = ticket.scope
        creation.destination = identity.id
        isDraft = false
        if let input = ticket.input { composerEvents.send(.created(previous: previous, id: identity.id, runtimeID: identity.runtimeId, input: input)) }
        applySnapshotResult(created, scope: creation)
        ticket.scope = projectionScope()
    }
    private func projectionScope(destination: String? = nil) -> ProjectionScope {
        ProjectionScope(storeGeneration: generation, navigationID: navigationID, source: snapshot?.conversation.runtimeID,
            conversationID: loadedConversationID, context: projectionRuntimeID, cursor: snapshot?.cursor,
            epoch: projectionEpoch, destination: destination)
    }
    private func baseIsCurrent(_ scope: ProjectionScope) -> Bool {
        scope.epoch == projectionEpoch && scope.conversationID == loadedConversationID
            && scope.source == snapshot?.conversation.runtimeID && scope.context == projectionRuntimeID && scope.cursor == snapshot?.cursor
    }
    private func deliveryPosition(_ snapshot: ConversationSnapshot) -> TranscriptDeliveryPosition? {
        snapshot.cursor.map {
            TranscriptDeliveryPosition(contextRuntimeId: snapshot.contextRuntimeID,
                conversationId: snapshot.conversation.id, cursor: $0)
        }
    }
    @discardableResult
    private func installProjection(_ incoming: ConversationSnapshot?, scope: ProjectionScope, preserveSelection: Bool = false) -> Bool {
        guard !composerTargetClosed, generation == scope.storeGeneration else { return false }
        let baseCurrent = baseIsCurrent(scope)
        guard let incoming else {
            guard baseCurrent else { requestProjectionResync(); return false }
            resetProjection(); return true
        }
        let sameIdentity = incoming.conversation.id == scope.conversationID && incoming.conversation.runtimeID == scope.source
        guard sameIdentity || (scope.destination == incoming.conversation.id && baseCurrent) else { return false }
        guard loadedConversationID == incoming.conversation.id || baseCurrent else { return false }
        if let position = deliveryPosition(incoming) {
            switch classifyTranscriptDelivery(installed: snapshot.flatMap(deliveryPosition), incoming: position,
                deltaBase: nil, requestIsCurrent: baseCurrent) {
            case .replace: break
            case .ignore: return false
            case .applyDelta, .readFull: requestProjectionResync(); return false
            }
        } else if !baseCurrent {
            requestProjectionResync(); return false
        }
        apply(incoming, preserveSelection: preserveSelection)
        return true
    }
    private func applySnapshotResult(_ value: SnapshotResult, scope: ProjectionScope) {
        if value.snapshot == nil, value.submissionReceipt != nil {
            // Admission succeeded; unavailable projection data is not absence
            // of the conversation and must not clear its transcript or input.
            guard generation == scope.storeGeneration, !composerTargetClosed else { return }
            requestProjectionResync()
            return
        }
        installProjection(value.snapshot.map(BindingMapping.snapshot), scope: scope)
    }
    private func requestProjectionResync() {
        needsResync = true
        poll()
    }
    enum PolledSnapshot: Sendable {
        case absent, unchanged
        case complete(ConversationSnapshot)
        /// Changed messages only; merged on the main actor into the snapshot at `base`.
        case changes(base: TranscriptCursor, header: ConversationSnapshot, order: [String], changed: [DeliveredMessage])
    }
    private func poll() {
        device.refreshConversationObservation()
        // First-open failures have no snapshot or projection runtime yet.
        // Their retry must run before the ordinary transcript polling guard.
        guard started, ConversationWindowModel.isConversationStoreVisible(self) else { return }
        retryOpeningIfNeeded()
        pollProjection()
    }
    /// A native wake carries only an address. Keep one pending read if a
    /// delivery that began before the native event is already in flight.
    private func readNativeUserMessage(for conversationID: String?) {
        guard let loadedConversationID, conversationID == nil || conversationID == loadedConversationID,
              ConversationWindowModel.isConversationStoreVisible(self) else { return }
        nativeUserMessageWakePending = true
        pollProjection()
    }
    /// Admission is separate from visibility so headless checks can exercise
    /// the exact Store/Transport lane without constructing a window or scene.
    private func pollProjection() {
        guard started, !composerTargetClosed, !isDraft, !isPreview, !isShuttingDown, pendingConversationID == nil, !pollPending, let transport, let current = snapshot else { return }
        let scope = projectionScope()
        let runtimeID = current.conversation.runtimeID, selected = current.conversation.id
        let since = needsResync ? nil : current.cursor
        needsResync = false
        nativeUserMessageWakePending = false
        pollPending = true
        observationQueue.async { [weak self] in
            // Only changed messages cross the boundary; mapping stays off the main thread.
            let result = Result { () throws -> PolledSnapshot in
                guard let since else {
                    return try transport.snapshot(runtimeID: runtimeID, conversationID: selected).snapshot.map { .complete(BindingMapping.snapshot($0)) } ?? .absent
                }
                switch try transport.snapshotUpdate(runtimeID: runtimeID, conversationID: selected, since: since) {
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
                defer {
                    if self.nativeUserMessageWakePending
                        || (self.generation == scope.storeGeneration && self.needsResync) { self.poll() }
                }
                guard self.generation == scope.storeGeneration, !self.composerTargetClosed else { return }
                switch result {
                case .success(let polled):
                    self.device.clearActivityProblem("poll:" + runtimeID)
                    self.consumePolledSnapshot(polled, scope: scope)
                case .failure(let failure): self.recordProblem(failure, source: "读取会话", activityKey: "poll:" + runtimeID)
                }
            }
        }
    }
    private func consumePolledSnapshot(_ polled: PolledSnapshot, scope: ProjectionScope) {
        guard generation == scope.storeGeneration, !composerTargetClosed else { return }
        switch polled {
        case .unchanged: break
        case .absent: installProjection(nil, scope: scope, preserveSelection: true)
        case .complete(let value): installProjection(value, scope: scope, preserveSelection: true)
        case .changes(let base, var header, let order, let changed):
            guard let current = snapshot, let position = deliveryPosition(header) else { needsResync = true; return }
            switch classifyTranscriptDelivery(installed: deliveryPosition(current), incoming: position,
                deltaBase: base, requestIsCurrent: baseIsCurrent(scope)) {
            case .applyDelta: break
            case .ignore: return
            case .replace, .readFull: needsResync = true; return
            }
            let ids = Set(order), changedIDs = Set(changed.map(\.id))
            guard ids.count == order.count, changedIDs.count == changed.count, ids.isSuperset(of: changedIDs) else {
                needsResync = true; return
            }
            var known = Dictionary(current.messages.map { ($0.id, $0) }, uniquingKeysWith: { first, _ in first })
            for message in changed { known[message.id] = message }
            let messages = order.compactMap { known[$0] }
            guard messages.count == order.count else { needsResync = true; return }
            header.messages = messages
            installProjection(header, scope: scope, preserveSelection: true)
        }
    }
    /// Read deferred reasoning and tool output of shown messages. Loaded content
    /// replaces a delivered message only while its stamp is current.
    func loadPayloads(messageIDs: [String]) {
        guard !isPreview, !composerTargetClosed, pendingConversationID == nil, let snapshot else { return }
        let deferred = Set(snapshot.messages.filter(\.hasDeferredPayload).map(\.id))
        payloadDemand.formIntersection(deferred)
        payloadDemand.formUnion(Set(messageIDs).intersection(deferred))
        drainPayloadDemand()
    }
    private func drainPayloadDemand() {
        guard !payloadPending, !composerTargetClosed, !isShuttingDown, pendingConversationID == nil, let transport, let snapshot else { return }
        let deferred = Dictionary(snapshot.messages.filter(\.hasDeferredPayload).map { ($0.id, $0.stamp) }, uniquingKeysWith: { first, _ in first })
        payloadDemand.formIntersection(deferred.keys)
        let wanted = Array(payloadDemand.prefix(32))
        guard !wanted.isEmpty else { return }
        payloadDemand.subtract(wanted)
        let stamps = Dictionary(uniqueKeysWithValues: wanted.map { ($0, deferred[$0]!) })
        let scope = projectionScope()
        let runtimeID = snapshot.contextRuntimeID, conversationID = snapshot.conversation.id
        payloadPending = true
        payloadQueue.async { [weak self] in
            let result = Result { try transport.messagePayloads(runtimeID: runtimeID, conversationID: conversationID, messageIDs: wanted) }
            DispatchQueue.main.async {
                guard let self else { return }
                self.payloadPending = false
                defer { self.drainPayloadDemand() }
                self.consumePayloads(result, scope: scope, stamps: stamps)
            }
        }
    }
    private func consumePayloads(_ result: Result<[DeliveredMessage], Error>, scope: ProjectionScope, stamps: [String: UInt64]) {
        guard !composerTargetClosed, pendingConversationID == nil, navigationID == scope.navigationID,
              var current = snapshot, current.conversation.id == scope.conversationID,
              current.conversation.runtimeID == scope.source else { return }
        let deferred = current.messages.filter(\.hasDeferredPayload)
        guard generation == scope.storeGeneration,
              current.contextRuntimeID == scope.context, current.cursor?.generation == scope.cursor?.generation,
              current.cursor != nil || projectionEpoch == scope.epoch else {
            // A sole viewport request must survive delivery replacement while
            // its batch was in flight. Re-resolve stamps against the new view.
            payloadDemand.formUnion(deferred.filter { stamps[$0.id] != nil }.map(\.id))
            return
        }
        switch result {
        case .success(let loaded):
            for message in loaded where stamps[message.id] == message.stamp {
                loadedPayloads[message.id] = message
            }
            payloadDemand.formUnion(deferred.filter { stamps[$0.id] != nil && stamps[$0.id] != $0.stamp }.map(\.id))
            current.messages = withLoadedPayloads(current.messages)
            apply(current, preserveSelection: true, coreDelivery: false)
        case .failure(let failure): recordProblem(failure, source: "读取消息内容")
        }
    }
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
    /// Snapshot-bearing operations name their base; business receipts settle
    /// before projection arbitration, including after navigation or closure.
    private func enqueueSnapshot(_ operation: @escaping () throws -> SnapshotResult, scope: ProjectionScope,
                                 consumeReceipt: ((Result<SnapshotResult, Error>) -> Void)? = nil,
                                 onFailure: ((Error) -> Void)? = nil, completion: @escaping (SnapshotResult) -> Void = { _ in }) {
        enqueue(operation, consumeReceipt: consumeReceipt, onFailure: onFailure) { [weak self] value in
            self?.applySnapshotResult(value, scope: scope)
            completion(value)
        }
    }
    /// Control calls have two bounded keys (turn/children and preparation),
    /// independent of ordinary ACKs, polling and interaction replies.
    @discardableResult
    private func requestControl(_ key: String, operation: @escaping @Sendable () throws -> SnapshotResult?,
                                consumeReceipt: ((Result<SnapshotResult?, Error>) -> Void)? = nil,
                                onFailure: (() -> Void)? = nil) -> Bool {
        guard !isShuttingDown, !controlRequests.contains(key), controlRequests.count < 2 else { return false }
        let scope = projectionScope()
        controlRequests.insert(key)
        controlQueue.async { [weak self] in
            let result = Result { try operation() }
            DispatchQueue.main.async {
                // Business ownership survives switching or closing the view.
                consumeReceipt?(result)
                guard let self else { return }
                self.controlRequests.remove(key)
                guard self.generation == scope.storeGeneration, !self.composerTargetClosed else { return }
                switch result {
                case .success(let value):
                    if let value { self.applySnapshotResult(value, scope: scope) }
                    else { self.requestProjectionResync() }
                case .failure(let error): self.recordProblem(error); onFailure?()
                }
            }
        }
        return true
    }
    /// One conversation operation at a time; one that finishes after the tab
    /// moved on is dropped, and a conversation chosen meanwhile then opens.
    private func enqueue<T: Sendable>(_ operation: @escaping () throws -> T, consumeReceipt: ((Result<T, Error>) -> Void)? = nil, onAccepted: (() -> Void)? = nil, onFailure: ((Error) -> Void)? = nil, apply: @escaping (T) -> Void) {
        guard !isLoading, !isShuttingDown else { recordProblem("请等待当前操作完成后重试"); return }
        guard transport != nil else { recordProblem("本地核心未配置"); return }
        enqueuePreparedOperation(operation, consumeReceipt: consumeReceipt, onAccepted: onAccepted, onFailure: onFailure, apply: apply)
    }
    /// The serial operation/receipt runner has no backend dependency of its
    /// own. Production validates its captured transport before entering it.
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
                guard revision == self.generation, !self.composerTargetClosed else { return }
                switch result {
                case .success(let value): onAccepted?(); apply(value)
                case .failure(let failure):
                    if (failure as? TransportError)?.isSubmissionCancelled != true { self.recordProblem(failure) }
                    onFailure?(failure)
                }
            }
        }
    }
}

#if VELUNE_DEV_CHECKS
extension ConversationStore {
    var checkAccess: CheckAccess { CheckAccess(store: self) }

    /// State and operations exercised by the development checks.
    @MainActor struct CheckAccess {
        fileprivate let store: ConversationStore

        var offListConversations: [Conversation] {
            get { store.offListConversations }
            nonmutating set { store.offListConversations = newValue }
        }
        var selectedConversationID: String? {
            get { store.selectedConversationID }
            nonmutating set { store.selectedConversationID = newValue }
        }
        var pendingConversationID: String? {
            get { store.pendingConversationID }
            nonmutating set { store.pendingConversationID = newValue }
        }
        var linkedConversationRequest: UUID? {
            get { store.linkedConversationRequest }
            nonmutating set { store.linkedConversationRequest = newValue }
        }
        var unopenedConversationID: String? {
            get { store.unopenedConversationID }
            nonmutating set { store.unopenedConversationID = newValue }
        }
        var openingNeedsRetry: Bool { store.openingNeedsRetry }
        var openingRetry: PollBackoff { store.openingRetry }
        var deferredSelection: (id: String, move: NavigationMove)? {
            get { store.deferredSelection }
            nonmutating set { store.deferredSelection = newValue }
        }
        var started: Bool {
            get { store.started }
            nonmutating set { store.started = newValue }
        }
        var isDraft: Bool {
            get { store.isDraft }
            nonmutating set { store.isDraft = newValue }
        }
        var snapshot: ConversationSnapshot? { store.snapshot }
        var generation: Int {
            get { store.generation }
            nonmutating set { store.generation = newValue }
        }
        var navigationID: UUID {
            get { store.navigationID }
            nonmutating set { store.navigationID = newValue }
        }
        var pollPending: Bool { store.pollPending }
        var needsResync: Bool {
            get { store.needsResync }
            nonmutating set { store.needsResync = newValue }
        }
        var projectionEpoch: UInt64 { store.projectionEpoch }
        var payloadQueue: DispatchQueue { store.payloadQueue }
        var payloadPending: Bool {
            get { store.payloadPending }
            nonmutating set { store.payloadPending = newValue }
        }
        var payloadDemand: Set<String> {
            get { store.payloadDemand }
            nonmutating set { store.payloadDemand = newValue }
        }
        var loadedPayloads: [String: DeliveredMessage] {
            get { store.loadedPayloads }
            nonmutating set { store.loadedPayloads = newValue }
        }
        var composerTargetRequest: ComposerTargetRequest? { store.composerTargetRequest }
        var composerTargetRequestID: UUID? { store.composerTargetRequestID }
        var composerTargetClosed: Bool {
            get { store.composerTargetClosed }
            nonmutating set { store.composerTargetClosed = newValue }
        }
        var modelCatalogs: [String: RuntimeModelCatalog] {
            get { store.modelCatalogs }
            nonmutating set { store.modelCatalogs = newValue }
        }
        var modelCatalogRequests: [String: UUID] { store.modelCatalogRequests }
        var outgoing: OutgoingSubmissions { store.outgoing }
        var isLoading: Bool {
            get { store.isLoading }
            nonmutating set { store.isLoading = newValue }
        }
        var executionStarting: Bool {
            get { store.executionStarting }
            nonmutating set { store.executionStarting = newValue }
        }
        var pendingSendCancellation: VeluneSendCancellation? {
            get { store.pendingSendCancellation }
            nonmutating set { store.pendingSendCancellation = newValue }
        }
        var composerRequest: ComposerSubmission.Phase? {
            get { store.composerRequest }
            nonmutating set { store.composerRequest = newValue }
        }
        var acceptedSubmissions: UInt64 {
            get { store.acceptedSubmissions }
            nonmutating set { store.acceptedSubmissions = newValue }
        }
        var stopRequested: Bool {
            get { store.stopRequested }
            nonmutating set { store.stopRequested = newValue }
        }
        var interactionDrafts: [InteractionDraftKey: RuntimeInteractionDraft] { store.interactionDrafts }
        var interactionReplyFailures: [InteractionDraftKey: String] {
            get { store.interactionReplyFailures }
            nonmutating set { store.interactionReplyFailures = newValue }
        }
        var interactionReplyInFlight: (key: InteractionDraftKey, title: String, attempt: UUID, interaction: RuntimeInteraction)? {
            get { store.interactionReplyInFlight }
            nonmutating set { store.interactionReplyInFlight = newValue }
        }
        var replyAttempt: UUID? {
            get { store.replyAttempt }
            nonmutating set { store.replyAttempt = newValue }
        }

        func requestComposerTarget() {
            store.requestComposerTarget()
        }

        func refreshNextTurn() {
            store.refreshNextTurn()
        }

        func preparePreviewComposerTarget(_ request: ComposerTargetRequest) {
            store.preparePreviewComposerTarget(request)
        }

        func reloadProjection(_ read: @escaping () throws -> SnapshotResult?) {
            store.reloadProjection(read)
        }

        func noteOpeningFailure(_ error: Error) {
            store.noteOpeningFailure(error)
        }

        func takeOpeningRetry() -> String? {
            store.takeOpeningRetry()
        }

        func abandonLinkedConversationLookup() {
            store.abandonLinkedConversationLookup()
        }

        func finishLinkedConversationLookup(_ result: Result<ListedConversation?, Error>, id: String, move: NavigationMove, request: UUID) {
            store.finishLinkedConversationLookup(result, id: id, move: move, request: request)
        }

        func finishConversationLoading() {
            store.finishConversationLoading()
        }

        func acceptSubmission(_ id: String, receipt: SnapshotResult) {
            store.acceptSubmission(id, receipt: receipt)
        }

        func finishInteractionReply(_ result: Result<SnapshotResult, Error>, interaction: RuntimeInteraction,
            key: InteractionDraftKey, scope: ProjectionScope, attempt: UUID) {
            store.finishInteractionReply(result, interaction: interaction, key: key, scope: scope, attempt: attempt)
        }

        func resetProjection() {
            store.resetProjection()
        }

        func apply(_ value: ConversationSnapshot?, preserveSelection: Bool = false, coreDelivery: Bool = true) {
            store.apply(value, preserveSelection: preserveSelection, coreDelivery: coreDelivery)
        }

        func publishCreatedIdentity(_ created: SnapshotResult, ticket: SendProjectionTicket, previous: String) {
            store.publishCreatedIdentity(created, ticket: ticket, previous: previous)
        }

        func projectionScope(destination: String? = nil) -> ProjectionScope {
            store.projectionScope(destination: destination)
        }

        func baseIsCurrent(_ scope: ProjectionScope) -> Bool {
            store.baseIsCurrent(scope)
        }

        @discardableResult
        func installProjection(_ incoming: ConversationSnapshot?, scope: ProjectionScope, preserveSelection: Bool = false) -> Bool {
            store.installProjection(incoming, scope: scope, preserveSelection: preserveSelection)
        }

        func applySnapshotResult(_ value: SnapshotResult, scope: ProjectionScope) {
            store.applySnapshotResult(value, scope: scope)
        }

        func pollProjection() {
            store.pollProjection()
        }

        func consumePolledSnapshot(_ polled: PolledSnapshot, scope: ProjectionScope) {
            store.consumePolledSnapshot(polled, scope: scope)
        }

        func consumePayloads(_ result: Result<[DeliveredMessage], Error>, scope: ProjectionScope, stamps: [String: UInt64]) {
            store.consumePayloads(result, scope: scope, stamps: stamps)
        }

        func enqueueSnapshot(_ operation: @escaping () throws -> SnapshotResult, scope: ProjectionScope,
            consumeReceipt: ((Result<SnapshotResult, Error>) -> Void)? = nil,
            onFailure: ((Error) -> Void)? = nil, completion: @escaping (SnapshotResult) -> Void = { _ in }) {
            store.enqueueSnapshot(operation, scope: scope, consumeReceipt: consumeReceipt, onFailure: onFailure, completion: completion)
        }

        func enqueue<T: Sendable>(_ operation: @escaping () throws -> T, consumeReceipt: ((Result<T, Error>) -> Void)? = nil, onAccepted: (() -> Void)? = nil, onFailure: ((Error) -> Void)? = nil, apply: @escaping (T) -> Void) {
            store.enqueue(operation, consumeReceipt: consumeReceipt, onAccepted: onAccepted, onFailure: onFailure, apply: apply)
        }

        func enqueuePreparedOperation<T: Sendable>(_ operation: @escaping () throws -> T, consumeReceipt: ((Result<T, Error>) -> Void)? = nil, onAccepted: (() -> Void)? = nil, onFailure: ((Error) -> Void)? = nil, apply: @escaping (T) -> Void) {
            store.enqueuePreparedOperation(operation, consumeReceipt: consumeReceipt, onAccepted: onAccepted, onFailure: onFailure, apply: apply)
        }
    }
}
#endif
