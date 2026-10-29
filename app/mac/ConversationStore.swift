import Combine
import Foundation
import AppKit
import VeluneBindings

/// One conversation tab, or a window's list: the conversation it shows, its
/// composer and next turn, navigation, search and selection. What is the
/// device's (configuration, the list, login, problems, drafts, kept choices)
/// it reads from `device`.
@MainActor
final class ConversationStore: ObservableObject {
    let device: DeviceStore
    /// This window's search: its text, and the matches once read.
    @Published private(set) var searchText = ""
    @Published private(set) var searchResults: (conversations: [Conversation], groups: [ConversationGroup])?
    private var searchRequest = 0
    /// Conversations reached outside the list (a link, a search result), so they can be opened.
    private var offListConversations: [Conversation] = []
    @Published private(set) var selectedConversationIDs: Set<String> = []
    @Published private(set) var selectedConversationID: String?
    @Published private(set) var pendingConversationID: String?
    /// A conversation on its way to opening before its loading starts: while
    /// a new tab reads the list first, or one off the list is looked up.
    @Published private(set) var reachingConversationID: String?
    /// The conversation is being opened, as loading shows.
    var isOpeningConversation: Bool { pendingConversationID != nil || reachingConversationID != nil }
    @Published private(set) var conversationManagementStatus: String?
    @Published private(set) var projectionRuntimeID: String?
    @Published private(set) var nextTurnRuntimeID: String? { didSet { if oldValue != nextTurnRuntimeID { loadModelCatalog() } } }
    /// The next turn's options and effective choice, as the core resolves
    /// them from the kept choices and the runtime's catalog.
    @Published private(set) var nextTurn: NextTurn?
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
    /// The turn just sent, shown in the transcript until the runtime's own copy
    /// of the user message appears or the turn ends.
    private struct PendingSend {
        /// The last user message before this turn; a different one is the turn's own.
        let previousUserMessageID: String?
        var accepted = false
    }
    private var pendingSend: PendingSend?
    @Published private var composerQuotaResult: [ComposerQuotaEntry] = []
    private var composerQuotaResultRequest: ComposerQuotaRequest?
    @Published private var composerQuotaFailure: String?
    @Published private(set) var isLoading = false
    @Published private(set) var activity: TranscriptActivity?
    @Published private var snapshot: ConversationSnapshot?
    @Published private var executionStarting = false
    @Published private(set) var composerRequest: ComposerSubmission.Phase?
    @Published private var stopRequested = false
    /// A conversation chosen while an operation ran, opened when it ends.
    private var deferredSelection: (id: String, move: NavigationMove)?
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
    private var hasInitializedNextTurnIntent = false
    private var queuedConversationManagement: (() -> Void)?
    /// The conversation whose kept choices the next turn follows.
    private var intentConversationID: String?
    static let draftConversationPrefix = "draft:"

    init(device: DeviceStore) {
        self.device = device
        seenConfigurationRevision = device.configurationRevision
        seenComposerChoicesRevision = device.composerChoicesRevision
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
        deviceChanged()
        if isPreview {
            apply(device.previewSnapshots["sample-0"])
            selectModel(.managed(modelRecordKey: "sample-model"))
        }
    }

    var isPreview: Bool { device.isPreview }
    private var transport: Transport? { device.transport }
    var isShuttingDown: Bool { device.isShuttingDown }

    /// Follows what changed on the device: a list read, configuration the
    /// catalogs depend on, kept choices once read, a deleted conversation.
    private func deviceChanged() {
        if let loadedConversationID, device.deletedConversationIDs.contains(loadedConversationID) { resetProjection() }
        if device.listRevision != seenListRevision {
            seenListRevision = device.listRevision
            // Organization changes the list's own rows; reached ones follow them.
            let listed = Dictionary(device.conversations.map { ($0.id, $0) }, uniquingKeysWith: { first, _ in first })
            offListConversations = offListConversations.map { listed[$0.id] ?? $0 }
            if searchResults != nil { refreshSearch() }
            let selected = selectedConversationIDs.filter { conversation(id: $0) != nil }
            if selected != selectedConversationIDs { selectedConversationIDs = selected }
            if !hasInitializedNextTurnIntent, nextTurnRuntimeID != device.selectedRuntimeInstanceID { nextTurnRuntimeID = device.selectedRuntimeInstanceID }
        }
        if device.configurationRevision != seenConfigurationRevision {
            seenConfigurationRevision = device.configurationRevision
            invalidateModelCatalogs()
        }
        takeUpComposerChoices()
        noteSeen()
        reconcileNextTurnIntent()
    }

    /// Some conversation executes, in any window. Configuration its execution
    /// depends on cannot change meanwhile; other conversations stay usable.
    var applicationIsGenerating: Bool { isGenerating || executionStarting || device.hasRunningWork }
    var loadedConversationID: String? { snapshot?.conversation.id }
    var loadedConversationWorkingDirectory: String? { snapshot?.conversation.cwd }
    /// Set while a native sub-agent's conversation is shown: the conversation that started it.
    var loadedParentConversationID: String? { snapshot?.conversation.parentConversationID }
    /// The list's conversations, the open one with its runtime's fields, and
    /// first the open one when the list does not show it.
    var conversations: [Conversation] {
        var listed = device.conversations
        guard let current = snapshot?.conversation, !isDraft else { return listed }
        if let index = listed.firstIndex(where: { $0.id == current.id }) { listed[index] = current.organized(like: listed[index]) }
        else { var unlisted = current; unlisted.group = nil; listed.insert(unlisted, at: 0) }
        return listed
    }
    /// The conversations the sidebar lists; native sub-agents open from their parent's transcript.
    var sidebarConversations: [Conversation] { device.conversations.filter { $0.group != nil } }
    /// A conversation by identity: listed, or reached outside the list.
    func conversation(id: String) -> Conversation? {
        if let current = snapshot?.conversation, current.id == id, !isDraft { return conversations.first { $0.id == id } }
        return device.conversations.first { $0.id == id } ?? searchResults?.conversations.first { $0.id == id } ?? offListConversations.first { $0.id == id }
    }
    var selectedConversationTitle: String? { selectedConversationID.flatMap { conversation(id: $0) }?.title ?? (snapshot?.conversation.id == selectedConversationID ? snapshot?.conversation.title : nil) }
    var isGenerating: Bool { snapshot?.runState == .running || snapshot?.runState == .stopping }
    var isBusy: Bool { isLoading || applicationIsGenerating || isShuttingDown || device.authenticationRunning }
    var canManageConversations: Bool { !device.authenticationRunning && !isShuttingDown && conversationManagementStatus == nil && (!isLoading || pendingConversationID != nil) }
    /// A running conversation keeps its native session until the turn ends.
    func isRunning(_ conversation: Conversation) -> Bool { device.runningConversationIDs.contains(conversation.id) || (loadedConversationID == conversation.id && (isGenerating || executionStarting)) }
    /// The core validates the selection again; an unread catalog does not block sending.
    var isStopping: Bool { stopRequested || snapshot?.runState == .stopping }
    private var activityBlocksSending: Bool {
        snapshot?.runState == .elsewhere || loadedConversationID.map(device.isConversationRunning) == true
    }
    var canSend: Bool { !executionStarting && !device.authenticationRunning && !isLoading && !isGenerating && !isStopping && !activityBlocksSending && !isShuttingDown && (snapshot?.pendingInteractions.isEmpty ?? true) && (snapshot?.actions.canSend ?? false) }
    var canCancel: Bool { !isStopping && snapshot?.runState == .running && (snapshot?.actions.canCancel ?? false) }
    var followUp: FollowUp { snapshot?.followUp ?? FollowUp() }
    /// The loaded conversation's native sub-agents.
    var subAgents: [SubAgent] { pendingConversationID == nil ? snapshot?.subAgents ?? [] : [] }
    /// The conversations the loaded one took part with in agent-to-agent collaboration.
    var collaborators: [Collaborator] { pendingConversationID == nil ? snapshot?.collaborators ?? [] : [] }
    /// The loaded conversation has a parent, sub-agents or collaborators to list.
    var hasLinkedAgents: Bool { pendingConversationID == nil && (loadedParentConversationID != nil || !subAgents.isEmpty || !collaborators.isEmpty) }
    /// While a turn runs, a sent message waits in the conversation's queue.
    var canQueue: Bool { !followUp.isCompacting && !isPreview && snapshot?.runState == .running && !isStopping && !executionStarting && !isLoading && !isShuttingDown && !device.authenticationRunning && loadedConversationID != nil && snapshot?.conversation.parentConversationID == nil }
    var canResume: Bool { canSend && followUp.canResume && nextTurnRuntimeID == snapshot?.contextRuntimeID }
    var canCompact: Bool { canSend && followUp.canCompact && nextTurnRuntimeID == snapshot?.contextRuntimeID }
    var continueHelp: String { "从现有上下文继续，不新增用户消息" }

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
    func selectPermissionMode(_ mode: String?) { changeNextTurn(.permissionMode(mode: mode)) }
    /// `nil` follows the conversation's current model, and where it has none
    /// is the runtime's own default.
    func selectModel(_ selection: ModelSelection?) {
        guard canSwitchModel else { return }
        changeNextTurn(.model(selection: selection))
    }
    func selectEffort(_ effort: String?) { changeNextTurn(.effort(level: effort)) }
    /// Keeps the changed choice for this conversation on the next turn's
    /// runtime and, as the core decides, as the runtime's last choice.
    private func changeNextTurn(_ change: TurnChoiceChange) {
        guard let nextTurn, let update = VeluneBindings.changeNextTurn(next: nextTurn, runtimeChoices: device.runtimeComposerChoices[nextTurn.runtimeId], change: change) else { return }
        if let choices = update.runtimeChoices { device.setRuntimeComposerChoices(choices) }
        if let intentConversationID {
            var kept = device.choices(of: intentConversationID) ?? ConversationChoices()
            kept.choices[nextTurn.runtimeId] = update.choice
            device.keepChoices(kept, for: intentConversationID)
        }
        refreshNextTurn()
    }
    func selectNextTurnRuntime(_ id: String) {
        guard !isShuttingDown, device.enabledRuntimeInstances.contains(where: { $0.id == id }) else { return }
        hasInitializedNextTurnIntent = true
        if let intentConversationID {
            var kept = device.choices(of: intentConversationID) ?? ConversationChoices()
            kept.runtimeID = id
            device.keepChoices(kept, for: intentConversationID)
        }
        if nextTurnRuntimeID == id { refreshNextTurn() } else { nextTurnRuntimeID = id }
    }
    /// Once the kept choices are read, the open conversation comes back with its own.
    private func takeUpComposerChoices() {
        guard device.composerChoicesRevision != seenComposerChoicesRevision else { return }
        seenComposerChoicesRevision = device.composerChoicesRevision
        guard let snapshot else { return }
        intentConversationID = nil
        restoreNextTurnIntent(for: snapshot)
    }
    /// Reads the next-turn runtime's catalog again, e.g. after its native
    /// configuration or session changed outside Velune's settings.
    func reloadModelCatalog() {
        guard let runtimeID = nextTurnRuntimeID else { return }
        modelCatalogs[runtimeID] = nil
        loadModelCatalog()
    }
    private func invalidateModelCatalogs() {
        modelCatalogs.removeAll(); modelCatalogRequests.removeAll()
        loadModelCatalog()
    }
    /// Listing native models starts the runtime, so a catalog is read only
    /// for an open conversation's next-turn runtime and kept until invalidated.
    private func loadModelCatalog() {
        guard snapshot != nil, let runtimeID = nextTurnRuntimeID, device.enabledRuntimeInstances.contains(where: { $0.id == runtimeID }), modelCatalogs[runtimeID] == nil else { refreshNextTurn(); return }
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
    /// Resolves the next turn again from the kept choices, the open
    /// conversation and the runtime's catalog.
    private func refreshNextTurn() {
        guard let runtimeID = nextTurnRuntimeID else { if nextTurn != nil { nextTurn = nil }; return }
        let typeID = device.runtimeInstances.first { $0.id == runtimeID }?.typeId
        let resolved = resolveNextTurn(input: NextTurnInput(
            runtimeId: runtimeID, runtimeType: device.runtimeTypes.first { $0.id == typeID }, providers: device.gateway.providers,
            catalog: modelCatalogs[runtimeID]?.models, turns: isDraft ? [] : snapshot?.transcriptTurns ?? [],
            conversationRuntimeId: snapshot?.conversation.runtimeID, modelRecordKey: snapshot?.modelRecordKey,
            kept: intentConversationID.flatMap { device.choices(of: $0)?.choices[runtimeID] },
            runtimeChoices: device.runtimeComposerChoices[runtimeID]))
        if nextTurn != resolved { nextTurn = resolved }
    }
    /// A conversation comes back on its current runtime (one not sent yet on
    /// the runtime chosen for it), with that runtime's choices in it.
    private func restoreNextTurnIntent(for value: ConversationSnapshot) {
        guard value.conversation.id != intentConversationID else { return }
        intentConversationID = value.conversation.id
        let kept = isDraft ? device.choices(of: value.conversation.id)?.runtimeID : nil
        let runtimeID = [kept, value.contextRuntimeID].compactMap { $0 }.first { id in device.enabledRuntimeInstances.contains { $0.id == id } }
        if runtimeID != nil { hasInitializedNextTurnIntent = true }
        if let id = runtimeID, id != nextTurnRuntimeID { nextTurnRuntimeID = id } else { loadModelCatalog() }
    }
    private func reconcileNextTurnIntent() {
        let enabled = device.enabledRuntimeInstances
        if !hasInitializedNextTurnIntent {
            let id = enabled.first(where: { $0.id == nextTurnRuntimeID })?.id ?? enabled.first?.id
            if nextTurnRuntimeID != id { nextTurnRuntimeID = id }
            // A tab can see the configuration before it has loaded; it chooses once there is a runtime.
            hasInitializedNextTurnIntent = nextTurnRuntimeID != nil
        } else if let nextTurnRuntimeID, !enabled.contains(where: { $0.id == nextTurnRuntimeID }) {
            self.nextTurnRuntimeID = nil
        }
        refreshNextTurn()
    }
    /// Where the open conversation's files are: its runtime's machine.
    var loadedConversationLocation: RuntimeTransport { device.location(of: snapshot?.conversation.runtimeID) }
    /// Projects of the latest conversations on the machine the next turn
    /// runs on, newest first.
    var recentWorkingDirectories: [String] {
        let location = device.location(of: nextTurnRuntimeID)
        var seen = Set<String>()
        return device.conversations.filter { device.location(of: $0.runtimeID) == location }
            .sorted { ($0.updatedAtUnixMs ?? 0) > ($1.updatedAtUnixMs ?? 0) }
            .compactMap { conversation in
                conversation.project.flatMap { key in device.conversationProjects.first(where: { $0.path == key })?.primaryFolder }
            }.filter { seen.insert($0).inserted }.prefix(5).map { $0 }
    }

    // MARK: - Status

    /// Where the loaded conversation stands, as the core reads its turns' native records.
    var conversationStatus: ConversationStatus {
        ConversationStatus(conversationID: isDraft ? nil : snapshot?.conversation.id,
                           directory: isDraft ? nil : snapshot?.conversation.cwd,
                           usage: snapshot?.usage ?? ConversationUsage())
    }
    var composerQuotaRequest: ComposerQuotaRequest {
        ComposerQuotaRequest(runtimeID: nextTurnRuntimeID ?? "", selection: nextTurnModel,
            conversationID: isDraft ? nil : snapshot?.conversation.id,
            conversationRevision: snapshot?.revision, configurationRevision: device.configurationRevision)
    }
    var composerAccountQuotas: [ComposerQuotaEntry] {
        composerQuotaResultRequest == composerQuotaRequest ? composerQuotaResult : []
    }
    var composerQuotaError: String? { composerQuotaResultRequest == composerQuotaRequest ? composerQuotaFailure : nil }
    func observeComposerQuota(_ request: ComposerQuotaRequest) async {
        guard !isPreview, !isShuttingDown, !request.runtimeID.isEmpty, let transport else { return }
        while !Task.isCancelled && composerQuotaRequest == request {
            let result: Result<[ComposerQuotaEntry], Error> = await withCheckedContinuation { continuation in
                performOnQueue { continuation.resume(returning: Result { try transport.composerQuota(request) }) }
            }
            guard !Task.isCancelled, !isShuttingDown, composerQuotaRequest == request else { return }
            switch result {
            case .success(let entries): composerQuotaResult = entries; composerQuotaFailure = nil
            case .failure(let error): composerQuotaResult = []; composerQuotaFailure = error.localizedDescription
            }
            composerQuotaResultRequest = request
            let waiting = composerQuotaResult.contains { $0.quota.refreshing || ($0.quota.freshness == .unknown && $0.quota.problem == nil) }
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

    /// Begin reading. Once the list is read, a tab opens `conversationID`,
    /// else the list's first conversation, or an unsent one when asked for
    /// (`newConversation`) or when the list is empty.
    func start(loadFirstConversation: Bool = true, newConversation: Bool = false, opening conversationID: String? = nil) {
        guard !started else { return }; started = true
        device.start()
        guard !isPreview, let transport else { return }
        reachingConversationID = conversationID
        enqueue({ try transport.list() }, onFailure: { [weak self] _ in self?.reachingConversationID = nil }) { [weak self] data in
            guard let self else { return }
            applyList(data)
            schedulePolling()
            if let conversationID { openLinkedConversation(id: conversationID) }
            else if loadFirstConversation, let id = sidebarConversations.first?.id { selectConversation(id: id) }
            // With nothing to show yet, a tab starts an unsent conversation.
            else if newConversation || loadFirstConversation, snapshot == nil, pendingConversationID == nil { createConversation() }
        }
    }
    /// The tab closes: it stops following its conversation.
    func close() { subscriptions.removeAll(); timer?.invalidate(); timer = nil }
    /// Each tab follows the conversation it shows; the device follows the rest.
    private func schedulePolling() {
        timer?.invalidate()
        timer = Timer.scheduledTimer(withTimeInterval: 0.6, repeats: true) { [weak self] _ in
            Task { @MainActor in self?.poll() }
        }
    }
    func noteFocused() { device.focusedConversation = self; noteSeen() }
    /// The open conversation is read once it shows in the focused tab of the
    /// active application.
    private func noteSeen() {
        guard !isPreview, NSApp.isActive, device.focusedConversation === self,
              let id = loadedConversationID, pendingConversationID == nil,
              let conversation = conversation(id: id), conversation.unread else { return }
        device.markRead(conversation)
    }
    private func applyList(_ data: ConfigurationSnapshot) {
        device.applyList(data)
        deviceChanged()
    }
    var canReloadFromRuntimes: Bool { !isPreview && !isLoading && !isShuttingDown && !isGenerating && !executionStarting && pendingConversationID == nil }
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

    // MARK: - Drafts

    func draft(for conversationID: String) -> String { device.draft(for: conversationID) }
    func setDraft(_ text: String, for conversationID: String) { device.setDraft(text, for: conversationID) }

    // MARK: - List

    /// Pins, read state, and moves are Velune's records; an archive the
    /// runtime keeps itself changes its sessions, so it waits for loading
    /// like the other native changes.
    func organize(_ targets: [Conversation], _ change: ConversationOrganizationChange) {
        guard !targets.isEmpty, let transport else { return }
        let references = targets.map { ConversationReference(runtimeId: $0.runtimeID, conversationId: $0.id) }
        switch change {
        case .archive, .unarchive:
            guard canManageConversations else { recordProblem("请等待正在进行的操作结束，再修改会话"); return }
            let verb = { if case .archive = change { return "归档" } else { return "取消归档" } }()
            scheduleConversationManagement("\(verb) \(targets.count) 个会话") { [weak self] in
                guard let self else { return }
                conversationManagementStatus = "正在\(verb) \(targets.count) 个会话…"
                enqueue({ try transport.organizeConversations(references, change) }, onFailure: { [weak self] _ in self?.finishConversationManagement() }) { [weak self] data in
                    self?.applyList(data)
                    self?.finishConversationManagement()
                }
            }
        default:
            device.browse({ try transport.organizeConversations(references, change) }, source: "整理会话")
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
        if conversation(id: id) != nil { selectConversation(id: id, move: move); return }
        guard !isPreview, !isShuttingDown, let transport else { reachingConversationID = nil; return }
        reachingConversationID = id
        performOnQueue { [weak self] in
            let result = Result { try transport.findConversation(id) }
            DispatchQueue.main.async {
                guard let self else { return }
                if self.reachingConversationID == id { self.reachingConversationID = nil }
                switch result {
                case .success(let found?):
                    let reached = BindingMapping.listed(found)
                    self.offListConversations.removeAll { $0.id == reached.id }
                    self.offListConversations.append(reached)
                    self.selectConversation(id: id, move: move)
                case .success(nil): self.recordProblem("找不到这个会话；它可能已被删除或其运行时已停用", source: "打开会话")
                case .failure(let failure): self.recordProblem(failure, source: "打开会话")
                }
            }
        }
    }
    func selectConversation(id: String, move: NavigationMove = .new) {
        guard let conversation = conversation(id: id) else { reachingConversationID = nil; return }
        // Chosen while another operation runs, it opens once that is done; the latest choice wins.
        guard !isLoading, !executionStarting else { deferredSelection = (id, move); return }
        deferredSelection = nil
        reachingConversationID = nil
        selectedConversationIDs = [id]
        if isPreview { projectionRuntimeID = conversation.runtimeID; apply(device.previewSnapshots[id]); return }
        guard let transport else { recordProblem("本地核心未配置"); return }
        // The sidebar reflects user intent immediately. The old loaded transcript
        // is retained until success, but hidden while this destination is loading.
        let previousSelection = snapshot?.conversation.id
        // An unsent conversation is not one to go back to.
        let shown = isDraft ? nil : previousSelection
        isDraft = false
        generation += 1 // Invalidate any poll already queued for the old conversation.
        selectedConversationID = id
        pendingConversationID = id
        enqueue({ try transport.openConversation(runtimeID: conversation.runtimeID, conversationID: id) }, onFailure: { [weak self] _ in
            self?.selectedConversationID = previousSelection
            self?.selectedConversationIDs = Set([previousSelection].compactMap { $0 })
            self?.finishConversationLoading()
        }) { [weak self] in
            if let shown, shown != id { self?.recordNavigation(from: shown, move) }
            self?.applySnapshotResult($0)
            self?.finishConversationLoading()
        }
    }
    private func selectDeferredConversation() {
        guard let (id, move) = deferredSelection, !isLoading, !executionStarting else { return }
        deferredSelection = nil
        selectConversation(id: id, move: move)
    }
    private func finishConversationLoading() {
        guard !isShuttingDown else {
            queuedConversationManagement = nil; conversationManagementStatus = nil; pendingConversationID = nil
            return
        }
        if let operation = queuedConversationManagement {
            queuedConversationManagement = nil
            operation()
        } else { pendingConversationID = nil }
        noteSeen()
    }

    // MARK: - Managing conversations

    private func scheduleConversationManagement(_ action: String, operation: @escaping () -> Void) {
        if isLoading && pendingConversationID != nil {
            conversationManagementStatus = "会话加载完成后将\(action)"
            queuedConversationManagement = operation
        } else { operation() }
    }
    private func finishConversationManagement() {
        conversationManagementStatus = nil
        pendingConversationID = nil
    }
    func canRenameConversation(_ conversation: Conversation) -> Bool {
        conversation.canRename && !isRunning(conversation)
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
        scheduleConversationManagement("重命名“\(conversation.title)”") { [weak self] in
            guard let self else { return }
            generation += 1
            conversationManagementStatus = "正在重命名“\(conversation.title)”…"
            enqueue({ try transport.renameConversation(runtimeID: conversation.runtimeID, conversationID: conversation.id, title: name) }, onFailure: { [weak self] _ in self?.finishConversationManagement() }) { [weak self] data in
                guard let self else { return }
                applyList(data)
                if snapshot?.conversation.id == conversation.id, let renamed = device.conversations.first(where: { $0.id == conversation.id }) { snapshot?.conversation = renamed }
                finishConversationManagement()
                onRenamed()
            }
        }
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
        for target in targets { device.dropUnsavedDraft(target.id) }
        scheduleConversationManagement("删除 \(targets.count) 个会话") { [weak self] in
            guard let self else { return }
            generation += 1
            conversationManagementStatus = "正在删除 \(targets.count) 个会话…"
            let loadedID = loadedConversationID
            let contextID = snapshot?.contextRuntimeID
            // Runtime-owned deletions are independent operations. Preserve each
            // success and report failures rather than promising a transaction.
            enqueue({
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
            }) { [weak self] result in
                guard let self else { return }
                selectedConversationIDs.subtract(result.1)
                device.noteDeleted(result.1)
                if let data = result.0 { applyList(data) }
                if let loadedConversationID, result.1.contains(loadedConversationID) { resetProjection() }
                else if result.4 { resetProjection() }
                else if let refreshed = result.3 { applySnapshotResult(refreshed) }
                finishConversationManagement()
                device.appendProblems(result.2)
            }
        }
    }

    // MARK: - Composing

    /// Opens an unsent conversation: an empty transcript and composer. The
    /// runtime and model start from the current next-turn choice and can be
    /// changed in the composer; the conversation is created at the first send.
    func createConversation() {
        guard !isLoading, !executionStarting else { return }
        let enabled = device.enabledRuntimeInstances
        guard let runtimeID = enabled.first(where: { $0.id == nextTurnRuntimeID })?.id ?? enabled.first?.id else { return }
        let id = isPreview ? UUID().uuidString : Self.draftConversationPrefix + UUID().uuidString
        let conversation = Conversation(id: id, title: "新会话", updatedAtUnixMs: isPreview ? Int64(Date().timeIntervalSince1970 * 1000) : nil, runtimeID: runtimeID, cwd: nil, canRename: isPreview, canDelete: isPreview)
        isDraft = !isPreview
        projectionRuntimeID = runtimeID
        apply(ConversationSnapshot(revision: 1, conversation: conversation, contextRuntimeID: runtimeID, modelRecordKey: nil, runState: .idle, messages: [], actions: ConversationActions(canSend: true, canCancel: false, canSwitch: true)))
    }
    /// An unsent conversation can still choose its working directory.
    var isUnsentConversation: Bool { isDraft }
    func setUnsentWorkingDirectory(_ path: String?) {
        guard isDraft, snapshot != nil else { return }
        snapshot?.conversation.cwd = path
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
    func send(text: String, attachments: [String] = [], onAccepted: ((String?) -> Void)? = nil, onFailure: (() -> Void)? = nil) -> Bool {
        guard !text.trimmingCharacters(in: .whitespacesAndNewlines).isEmpty || !attachments.isEmpty else { return false }
        guard canSend, let target = nextTurnRuntimeID, let source = snapshot?.conversation.runtimeID, let conversationID = loadedConversationID else { return false }
        let model = nextTurnModel
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
        executionStarting = true
        composerRequest = .sending
        pendingSend = PendingSend(previousUserMessageID: snapshot?.messages.last { $0.role == .user }?.id)
        let sent: [DeliveredBlock] = (text.isEmpty ? [] : [.text(text: text)]) + attachments.map { path in
            Self.imageExtensions.contains(URL(fileURLWithPath: path).pathExtension.lowercased()) ? .image(mediaType: nil, source: .path(path: path)) : .file(path: path)
        }
        transcript.showPending(DeliveredMessage(id: "pending:" + UUID().uuidString, role: .user, blocks: sent), status: "发送中")
        let creating = isDraft
        let cwd = snapshot?.conversation.cwd
        // Without a level or mode the core keeps the conversation's current one.
        let turn = ComposedTurn(text: text, effort: nextTurn?.effort, permissionMode: effectivePermissionMode?.id, attachments: attachments)
        enqueue({
            if creating {
                return try transport.createAndSend(runtimeID: target, cwd: cwd, selection: model, turn: turn) { [weak self] created in
                    DispatchQueue.main.async {
                        guard let self else { return }
                        self.isDraft = false
                        // Text typed while sending and the next-turn choice follow the created conversation.
                        if let id = created.snapshot?.conversation.id {
                            self.device.moveDraft(from: conversationID, to: id)
                            self.device.moveChoices(from: conversationID, to: id)
                        }
                        self.applySnapshotResult(created)
                    }
                }
            }
            return try transport.sendTurn(sourceRuntimeID: source, conversationID: conversationID, targetRuntimeID: target, selection: model, turn: turn)
        }, onAccepted: { [weak self] in onAccepted?(self?.loadedConversationID) }, onFailure: { [weak self] _ in
            self?.executionStarting = false
            self?.composerRequest = nil
            self?.endPendingSend()
            onFailure?()
        }) { [weak self] value in
            guard let self else { return }
            self.executionStarting = false
            self.composerRequest = nil
            self.pendingSend?.accepted = true
            self.applySnapshotResult(value)
        }
        return true
    }
    /// Images go to the runtime as images, as the core decides from the extension.
    private static let imageExtensions: Set<String> = ["png", "jpg", "jpeg", "gif", "webp"]
    private func endPendingSend() {
        pendingSend = nil
        transcript.showPending(nil, status: nil)
    }
    /// The sent message gives way to the runtime's own copy, or to the end of
    /// a turn that never showed one; once accepted it is no longer "sending".
    private func updatePendingSend(_ value: ConversationSnapshot) {
        guard let pending = pendingSend else { return }
        let ownCopy = value.messages.last { $0.role == .user }?.id != pending.previousUserMessageID
        let ended = pending.accepted && !(value.runState == .running || value.runState == .stopping)
        if ownCopy || ended { endPendingSend() }
        else if pending.accepted, let row = transcript.rows.last, row.status != nil { transcript.showPending(row.message, status: nil) }
    }
    /// Stop the sub-agents the loaded conversation started; they may run after its turn.
    func stopSubAgents() {
        guard followUp.canStopSubAgents, !isPreview else { return }
        guard let runtimeID = projectionRuntimeID, let conversationID = loadedConversationID else { recordProblem("请选择会话"); return }
        guard let transport else { recordProblem("本地核心未配置"); return }
        enqueue({ try transport.stopSubAgents(runtimeID: runtimeID, conversationID: conversationID) }) { [weak self] in self?.applySnapshotResult($0) }
    }
    func cancel() {
        guard canCancel, !isLoading else { return }
        if isPreview { snapshot?.runState = .idle; snapshot?.actions.canCancel = false; apply(snapshot); return }
        guard let runtimeID = projectionRuntimeID, let conversationID = loadedConversationID else { recordProblem("请选择运行时实例"); return }
        guard let transport else { recordProblem("本地核心未配置"); return }
        stopRequested = true
        enqueue({ try transport.cancel(runtimeID: runtimeID, conversationID: conversationID) }, onFailure: { [weak self] _ in self?.stopRequested = false }) { [weak self] in self?.applySnapshotResult($0) }
    }
    /// Queue a message after the running turn, with the next-turn choices as
    /// they are now. `onFailure` runs when it was not queued.
    @discardableResult
    func queue(text: String, attachments: [String] = [], onFailure: (() -> Void)? = nil) -> Bool {
        guard !text.trimmingCharacters(in: .whitespacesAndNewlines).isEmpty || !attachments.isEmpty else { return false }
        guard canQueue, let target = nextTurnRuntimeID, let source = snapshot?.conversation.runtimeID else { return false }
        let turn = ComposedTurn(text: text, effort: nextTurn?.effort, permissionMode: effectivePermissionMode?.id, attachments: attachments)
        changeQueue(.add(sourceRuntimeId: source, targetRuntimeId: target, selection: nextTurnModel, turn: turn), onFailure: onFailure)
        return true
    }
    func compactContext() {
        guard canCompact, let runtimeID = snapshot?.contextRuntimeID, let conversationID = loadedConversationID, let transport else { return }
        let selection = nextTurnModel
        executionStarting = true
        enqueue({ try transport.compactConversation(runtimeID: runtimeID, conversationID: conversationID, selection: selection) }, onFailure: { [weak self] _ in self?.executionStarting = false }) { [weak self] value in
            self?.executionStarting = false
            self?.applySnapshotResult(value)
        }
    }
    func continueTurn() {
        guard canResume else { return }
        guard let runtimeID = snapshot?.contextRuntimeID, let conversationID = loadedConversationID, let transport else { return }
        let selection = nextTurnModel
        let effort = nextTurn?.effort
        let mode = effectivePermissionMode?.id
        executionStarting = true
        composerRequest = .resuming
        enqueue({ try transport.resumeTurn(runtimeID: runtimeID, conversationID: conversationID, selection: selection, effort: effort, permissionMode: mode) },
                onFailure: { [weak self] _ in self?.executionStarting = false; self?.composerRequest = nil }) { [weak self] value in
            self?.executionStarting = false
            self?.composerRequest = nil
            self?.applySnapshotResult(value)
        }
    }
    var canSteer: Bool { canQueue && followUp.canSteer }
    /// Into the running turn; the turn keeps its own choices.
    func steer(text: String, attachments: [String] = [], onFailure: (() -> Void)? = nil) {
        guard canSteer, let runtimeID = projectionRuntimeID, let conversationID = loadedConversationID, let transport else { onFailure?(); return }
        let turn = ComposedTurn(text: text, effort: nil, permissionMode: nil, attachments: attachments)
        enqueue({ try transport.steerTurn(runtimeID: runtimeID, conversationID: conversationID, turn: turn) }, onFailure: { _ in onFailure?() }) { [weak self] in self?.applySnapshotResult($0) }
    }
    /// Whether the user message may be edited now, and how. The edit goes to
    /// the conversation's own runtime with the next turn's choices.
    func editability(of messageID: String) -> EditableMessage? {
        guard !isPreview, !isDraft, canSend, nextTurnRuntimeID == snapshot?.conversation.runtimeID else { return nil }
        return followUp.editable.first { $0.messageId == messageID }
    }
    func message(id: String) -> DeliveredMessage? { snapshot?.messages.first { $0.id == id } }
    func isLastUserMessage(_ messageID: String) -> Bool { snapshot?.messages.last { $0.role == .user }?.id == messageID }
    /// Send `text` in place of a user message. In place the conversation
    /// shows the new turn here; a fork's new conversation goes to `forked`.
    func editMessage(_ messageID: String, placement: EditPlacement, text: String, attachments: [String], forked: @escaping (SnapshotResult) -> Void) {
        guard editability(of: messageID) != nil, let runtimeID = snapshot?.conversation.runtimeID, let conversationID = loadedConversationID else { return }
        let model = nextTurnModel
        guard let transport else { recordProblem("本地核心未配置"); return }
        executionStarting = true
        let turn = ComposedTurn(text: text, effort: nextTurn?.effort, permissionMode: effectivePermissionMode?.id, attachments: attachments)
        enqueue({ try transport.editMessage(runtimeID: runtimeID, conversationID: conversationID, messageID: messageID, placement: placement, selection: model, turn: turn) },
                onFailure: { [weak self] _ in self?.executionStarting = false }) { [weak self] value in
            guard let self else { return }
            self.executionStarting = false
            if placement == .fork { forked(value); self.device.refreshConversations() } else { self.applySnapshotResult(value) }
        }
    }
    /// Show a conversation the core has already opened, as `result` gives it.
    func adopt(_ result: SnapshotResult) { applySnapshotResult(result) }
    func steerQueued(_ id: String) { changeQueue(.steer(id: id)) }
    func editQueued(_ id: String, text: String) { changeQueue(.edit(id: id, text: text)) }
    /// `onRemoved` runs once the core has taken the message out of the queue.
    func removeQueued(_ id: String, onRemoved: (() -> Void)? = nil) { changeQueue(.remove(id: id), onDone: onRemoved) }
    func moveQueued(_ id: String, to index: Int) { changeQueue(.move(id: id, index: UInt32(max(index, 0)))) }
    func resumeQueue() { changeQueue(.resume) }
    private func changeQueue(_ operation: SendQueueOperation, onFailure: (() -> Void)? = nil, onDone: (() -> Void)? = nil) {
        guard !isPreview else { return }
        guard let runtimeID = projectionRuntimeID, let conversationID = loadedConversationID else { recordProblem("请选择会话"); onFailure?(); return }
        guard let transport else { recordProblem("本地核心未配置"); onFailure?(); return }
        enqueue({ try transport.sendQueue(runtimeID: runtimeID, conversationID: conversationID, operation: operation) }, onFailure: { _ in onFailure?() }) { [weak self] in
            self?.applySnapshotResult($0)
            onDone?()
        }
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
    var canChangeSessionOptions: Bool { !isGenerating && !executionStarting && !isLoading && !isShuttingDown }
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

    private func resetProjection() { composerRequest = nil; stopRequested = false; executionStarting = false; intentConversationID = nil; loadedPayloads.removeAll(); isDraft = false; generation += 1; snapshot = nil; projectionRuntimeID = nil; selectedConversationID = nil; pendingConversationID = nil; reachingConversationID = nil; transcript.reset(); endPendingSend(); activity = nil; sessionOptions = nil; sessionOptionsKey = nil }
    private func apply(_ value: ConversationSnapshot?, preserveSelection: Bool = false) {
        guard var value else { return }
        if value.conversation.id != snapshot?.conversation.id || (value.runState != .running && value.runState != .stopping) { stopRequested = false }
        if value.conversation.id != snapshot?.conversation.id || value.contextRuntimeID != snapshot?.contextRuntimeID { loadedPayloads.removeAll() }
        value.messages = withLoadedPayloads(value.messages)
        updatePendingSend(value)
        guard snapshot != value else { return }
        snapshot = value; projectionRuntimeID = value.contextRuntimeID; selectedConversationID = value.conversation.id
        restoreNextTurnIntent(for: value); refreshNextTurn()
        if !preserveSelection && selectedConversationIDs.count <= 1 { selectedConversationIDs = [value.conversation.id] }
        transcript.apply(value.messages, turns: value.transcriptTurns, items: value.transcriptItems, outline: value.transcriptOutline, identities: value.transcriptMessageIdentities)
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
        guard let message, message.role == .assistant, let block = message.blocks.last else { return "正在思考" }
        switch block {
        case .reasoning(let summary, _): return summary
        case .tool(_, let title, .running, _, _), .tool(_, let title, .pending, _, _): return title
        case .text: return "正在回复"
        case .tool, .notice, .contextCompaction, .image, .file, .agentSender: return "正在思考"
        }
    }
    /// Preparation and operation results replace the projection; polling asks
    /// the core only for changes after the current cursor (see `poll`).
    private func applySnapshotResult(_ value: SnapshotResult) {
        guard let incoming = value.snapshot else { resetProjection(); return }
        apply(BindingMapping.snapshot(incoming))
    }
    private enum PolledSnapshot: Sendable {
        case absent, unchanged
        case complete(ConversationSnapshot)
        /// Changed messages only; merged on the main actor into the snapshot at `base`.
        case changes(base: TranscriptCursor, header: ConversationSnapshot, order: [String], changed: [DeliveredMessage])
    }
    private func poll() {
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
    private func enqueue<T: Sendable>(_ operation: @escaping () throws -> T, onAccepted: (() -> Void)? = nil, onFailure: ((Error) -> Void)? = nil, apply: @escaping (T) -> Void) {
        guard !isLoading, !isShuttingDown else { recordProblem("请等待当前操作完成后重试"); return }
        guard transport != nil else { recordProblem("本地核心未配置"); return }
        let revision = generation
        isLoading = true
        performOnQueue { [weak self] in
            let result = Result { try operation() }
            DispatchQueue.main.async {
                guard let self else { return }
                self.isLoading = false
                defer { self.selectDeferredConversation() }
                guard revision == self.generation else { return }
                switch result {
                case .success(let value): onAccepted?(); apply(value)
                case .failure(let failure): self.recordProblem(failure); onFailure?(failure)
                }
            }
        }
    }
}
