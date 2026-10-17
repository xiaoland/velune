import Combine
import Foundation
import AppKit
import VeluneBindings

@MainActor
final class AppStore: ObservableObject {
    @Published private(set) var conversationBrowserPreferences: BindingConversationBrowserPreferences?
    private var savedBrowserPreferences: BindingConversationBrowserPreferences?
    private var browserPreferencesGeneration: UInt64 = 0
    /// Browser choices this store is saving; until they are saved, lists and
    /// the application's copy still carry the choices before them.
    private var browserPreferencesSaving = 0
    @Published private(set) var outlinePresentation: BindingOutlinePresentation = .popover
    /// What the composer's status line shows, in display order.
    @Published private(set) var statusLine: [BindingStatusLineItem] = [.contextUsage]
    @Published private(set) var conversationBrowserLimits = BindingConversationBrowserLimits(page: 20, total: 200)
    /// The conversations the list shows, in display order, then open ones it does not.
    @Published private(set) var conversations: [Conversation] = []
    /// The list's groups in display order, as the core windows them.
    @Published private(set) var conversationGroups: [BindingConversationGroup] = []
    @Published private(set) var conversationProjects: [BindingConversationProject] = []
    @Published private(set) var showsArchived = false
    /// This window's search: its text, and the matches once read.
    @Published private(set) var searchText = ""
    @Published private(set) var searchResults: (conversations: [Conversation], groups: [BindingConversationGroup])?
    private var searchRequest = 0
    /// Conversations reached outside the list (a link, a search result), so they can be opened.
    private var offListConversations: [Conversation] = []
    /// Conversations whose read mark is being sent. The mark can leave a
    /// conversation unread when it changed meanwhile, so this follows the
    /// request, not the list.
    private var markingRead: Set<String> = []
    /// Deleted in any workspace; the application keeps them so the workspace
    /// showing one closes it.
    @Published private var deletedConversationIDs: Set<String> = []
    @Published private(set) var selectedConversationIDs: Set<String> = []
    @Published private(set) var selectedConversationID: String?
    @Published private(set) var pendingConversationID: String?
    @Published private(set) var conversationManagementStatus: String?
    @Published private(set) var projectionRuntimeID: String?
    /// Conversations executing in this application, across windows; polled by
    /// the application store and copied by workspaces.
    @Published private(set) var runningConversationIDs: Set<String> = []
    /// Best-effort: conversations whose native records show a turn running outside Velune.
    @Published private(set) var elsewhereConversationIDs: Set<String> = []
    /// Conversations a native sub-agent of which runs, here or elsewhere.
    @Published private(set) var subAgentParentIDs: Set<String> = []
    /// Conversations with a request waiting for the user, such as an approval.
    @Published private(set) var waitingConversationIDs: Set<String> = []
    /// Conversations running elsewhere that the list did not contain when last read again.
    private var unlistedElsewhereIDs: Set<String> = []
    @Published private(set) var nextTurnRuntimeID: String? { didSet { if oldValue != nextTurnRuntimeID { loadModelCatalog() } } }
    /// `nil` until chosen, or until the runtime's catalog supplies a default.
    @Published private(set) var nextTurnModel: ModelSelection?
    /// The next turn's reasoning level; `nil` leaves it to the runtime's default.
    @Published private(set) var nextTurnEffort: String?
    /// Kept per conversation; offered only while the next turn's runtime declares it.
    @Published private(set) var nextTurnPermissionMode: String?
    /// Model catalogs by runtime instance, read from the runtime and Velune's
    /// gateway; configuration changes discard them.
    @Published private(set) var modelCatalogs: [String: RuntimeModelCatalog] = [:]
    private var modelCatalogRequests: [String: UUID] = [:]
    /// The open conversation's live-session options, for runtimes that have
    /// them; `nil` until its session is live in this application.
    @Published private(set) var sessionOptions: [SessionOption]?
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
    @Published private(set) var gateway = GatewayConfig() { didSet { if oldValue != gateway { invalidateModelCatalogs() } } }
    @Published private(set) var runtimeInstances: [RuntimeInstance] = [] { didSet { if oldValue != runtimeInstances { invalidateModelCatalogs(); refreshRuntimeLocations() } } }
    /// Where each instance runs, read with its saved execution; unknown ones count as local.
    @Published private(set) var runtimeLocations: [String: RuntimeLocation] = [:]
    private var runtimeLocationsRequest = UUID()
    @Published private(set) var runtimeTypes: [RuntimeTypeDescriptor] = []
    @Published private(set) var protocols: [ProtocolDescriptor] = []
    @Published private(set) var modelTemplates: [ModelTemplate] = []
    @Published private(set) var providerImportTypes: [RuntimeTypeDescriptor] = []
    @Published private(set) var authenticationRunning = false
    @Published private(set) var authenticationPrompt: AuthenticationPrompt?
    @Published private(set) var authenticationNotifications: [AuthenticationNotification] = []
    @Published private(set) var authenticationResult: String?
    @Published var showsAuthentication = false
    /// The core's quota observations by provider ID, as last read.
    @Published private(set) var providerQuotas: [String: ProviderQuota] = [:]
    private var quotaReadsPending: Set<String> = []
    /// What a provider is fixed to once it uses Codex subscription authentication; static in the core.
    private lazy var codexSubscriptionTarget: SubscriptionTarget? = try? BindingMapping.subscriptionTarget(VeluneBindings.codexSubscriptionTarget())
    /// The executable is chosen in the editor; until then the path stays empty.
    private lazy var claudeCodeSubscriptionTarget: SubscriptionTarget? = try? BindingMapping.subscriptionTarget(VeluneBindings.claudeCliSubscriptionTarget(binary: ""))
    func subscriptionTarget(_ choice: SubscriptionChoice) -> SubscriptionTarget? {
        switch choice { case .codex: codexSubscriptionTarget; case .claudeCode: claudeCodeSubscriptionTarget }
    }
    @Published private(set) var isLoading = false
    @Published private(set) var isShuttingDown = false
    @Published private(set) var activity: TranscriptActivity?
    @Published private(set) var problems: [AppProblem] = []
    @Published private var snapshot: ConversationSnapshot?

    private let transport: Transport?
    private(set) lazy var remoteAccess = RemoteAccessModel(transport: transport) { [weak self] error in
        self?.recordProblem(error, source: "远程接入", activityKey: "remote-access")
    }
    /// Native runtime resources are edited from settings with their own loading state.
    var resourceTransport: Transport? { transport }
    /// The tool windows as last arranged, for windows without a layout of their own; the application's alone.
    private(set) lazy var rememberedToolLayout = RememberedToolLayout(home: transport?.stateDirectory) { [weak self] in
        self?.recordProblem($0, source: "保存布局")
    }
    // Workspaces share application services; their navigation and transcript stay local.
    private let sharedApplication: AppStore?
    private var subscriptions: Set<AnyCancellable> = []
    private var sharesApplication = false
    private var sharedUpdateScheduled = false
    @Published private var executionStarting = false
    /// A conversation chosen while an operation ran, opened when it ends.
    private var deferredSelection: (id: String, move: NavigationMove)?
    private var started = false
    private var isDraft = false
    let isPreview: Bool
    @Published private(set) var analyticsReport: BindingAnalyticsReport?
    @Published private(set) var analyticsLoadedQuery: BindingAnalyticsQuery?
    @Published private(set) var analyticsIsLoading = false
    @Published private(set) var analyticsReadFailed = false
    @Published private(set) var analyticsUpdatedAt: Date?
    private var analyticsGeneration: UInt64 = 0
    private let queue = DispatchQueue(label: "local.velune.requests", qos: .userInitiated)
    private var generation = 0
    private var timer: Timer?
    private var pollPending = false
    private var payloadRequests: Set<PayloadKey> = []
    /// Complete messages read on demand, by message identifier.
    private var loadedPayloads: [String: Message] = [:]
    private var authenticationPollPending = false
    private var hasGateway = false
    private var hasInitializedNextTurnIntent = false
    private var queuedConversationManagement: (() -> Void)?
    private var previewSnapshots: [String: ConversationSnapshot] = [:]
    /// Unsent composer text by conversation. Deliberately not published: the
    /// composer owns the text while editing and reads a draft when it switches.
    private var composerDrafts: [String: String] = [:]
    private var unsavedDrafts: [String: String] = [:]
    private var draftSave: DispatchWorkItem?
    /// Next-turn choices per conversation in this workspace, while the application runs.
    private var nextTurnIntents: [String: NextTurnIntent] = [:]
    private var intentConversationID: String?
    /// The model last chosen on each runtime instance.
    private var lastChosenModels: [String: ModelSelection] = [:]
    /// A conversation's choices: each runtime keeps its own until sent. The
    /// runtime itself is kept only for a conversation not yet sent; a sent one
    /// comes back on its current runtime.
    private struct NextTurnIntent { var runtimeID: String?; var choices: [String: RuntimeChoice] = [:] }
    /// What the user chose for one runtime; `nil` follows the conversation's
    /// latest turn on it.
    private struct RuntimeChoice { var model: ModelSelection?; var effort: String?; var permissionMode: String? }

    init(transport: Transport? = nil, preview: Bool = false, sharedApplication: AppStore? = nil) {
        self.sharedApplication = sharedApplication
        isPreview = preview
        if preview { self.transport = nil }
        else if let sharedApplication { self.transport = sharedApplication.transport }
        else if let transport { self.transport = transport }
        else {
            do { self.transport = try Transport.applicationDefault() }
            catch { self.transport = nil; self.recordProblem(error, source: "初始化") }
        }
        if preview { seedPreview() }
        if let sharedApplication {
            sharedApplication.sharesApplication = true
            copyApplicationState(sharedApplication)
            sharedApplication.objectWillChange.sink { [weak self, weak sharedApplication] in
                guard let self, !self.sharedUpdateScheduled else { return }
                self.sharedUpdateScheduled = true
                Task { @MainActor in
                    guard let sharedApplication else { return }
                    self.sharedUpdateScheduled = false
                    self.copyApplicationState(sharedApplication)
                }
            }.store(in: &subscriptions)
        } else if let transport = self.transport {
            transport.configurationChanged.receive(on: DispatchQueue.main).sink { [weak self] in
                guard let self, self.sharesApplication else { return }
                self.refreshApplicationConfiguration()
            }.store(in: &subscriptions)
        }
    }

    func makeWorkspace() -> AppStore { AppStore(preview: isPreview, sharedApplication: self) }

    private func copyApplicationState(_ application: AppStore) {
        if gateway != application.gateway { gateway = application.gateway }
        hasGateway = application.hasGateway
        if runtimeInstances != application.runtimeInstances { runtimeInstances = application.runtimeInstances }
        if runtimeTypes != application.runtimeTypes { runtimeTypes = application.runtimeTypes }
        if protocols != application.protocols { protocols = application.protocols }
        if modelTemplates != application.modelTemplates { modelTemplates = application.modelTemplates }
        if providerImportTypes != application.providerImportTypes { providerImportTypes = application.providerImportTypes }
        if browserPreferencesSaving == 0, conversationBrowserPreferences != application.conversationBrowserPreferences { conversationBrowserPreferences = application.conversationBrowserPreferences }
        savedBrowserPreferences = application.savedBrowserPreferences
        if conversationBrowserLimits != application.conversationBrowserLimits { conversationBrowserLimits = application.conversationBrowserLimits }
        if conversationGroups != application.conversationGroups { conversationGroups = application.conversationGroups }
        if conversationProjects != application.conversationProjects { conversationProjects = application.conversationProjects }
        if showsArchived != application.showsArchived { showsArchived = application.showsArchived }
        if outlinePresentation != application.outlinePresentation { outlinePresentation = application.outlinePresentation }
        if statusLine != application.statusLine { statusLine = application.statusLine }
        var sharedConversations = application.conversations
        if let loadedConversationID, application.deletedConversationIDs.contains(loadedConversationID) { resetProjection() }
        if var current = snapshot?.conversation, !isDraft, !sharedConversations.contains(where: { $0.id == current.id }) { current.group = nil; sharedConversations.insert(current, at: 0) }
        if conversations != sharedConversations { conversations = sharedConversations }
        noteSeen()
        if authenticationRunning != application.authenticationRunning { authenticationRunning = application.authenticationRunning }
        if authenticationPrompt != application.authenticationPrompt { authenticationPrompt = application.authenticationPrompt }
        if authenticationNotifications != application.authenticationNotifications { authenticationNotifications = application.authenticationNotifications }
        if authenticationResult != application.authenticationResult { authenticationResult = application.authenticationResult }
        if problems != application.problems { problems = application.problems }
        if runningConversationIDs != application.runningConversationIDs { runningConversationIDs = application.runningConversationIDs }
        if elsewhereConversationIDs != application.elsewhereConversationIDs { elsewhereConversationIDs = application.elsewhereConversationIDs }
        if subAgentParentIDs != application.subAgentParentIDs { subAgentParentIDs = application.subAgentParentIDs }
        if waitingConversationIDs != application.waitingConversationIDs { waitingConversationIDs = application.waitingConversationIDs }
        reconcileNextTurnIntent()
    }

    private func refreshApplicationConfiguration() {
        guard let transport, !isShuttingDown else { return }
        queue.async { [weak self] in
            let result = Result { try transport.list() }
            DispatchQueue.main.async {
                guard let self else { return }
                switch result {
                case .success(let value): self.applyList(value)
                case .failure(let error): self.recordProblem(error, source: "刷新配置")
                }
            }
        }
    }

    /// Some conversation executes, in any window. Configuration its execution
    /// depends on cannot change meanwhile; other conversations stay usable.
    var applicationIsGenerating: Bool { isGenerating || executionStarting || !(sharedApplication ?? self).runningConversationIDs.isEmpty }

    var enabledRuntimeInstances: [RuntimeInstance] { runtimeInstances.filter(\.enabled) }
    var providers: [AIProvider] { gateway.providers }
    var loadedConversationID: String? { snapshot?.conversation.id }
    var loadedConversationWorkingDirectory: String? { snapshot?.conversation.cwd }
    /// Set while a native sub-agent's conversation is shown: the conversation that started it.
    var loadedParentConversationID: String? { snapshot?.conversation.parentConversationID }
    /// The conversations the sidebar lists; native sub-agents open from their parent's transcript.
    var sidebarConversations: [Conversation] { conversations.filter { $0.group != nil } }
    /// A conversation by identity: listed, or reached outside the list.
    func conversation(id: String) -> Conversation? {
        conversations.first { $0.id == id } ?? searchResults?.conversations.first { $0.id == id } ?? offListConversations.first { $0.id == id }
    }
    var selectedConversationTitle: String? { selectedConversationID.flatMap { conversation(id: $0) }?.title ?? (snapshot?.conversation.id == selectedConversationID ? snapshot?.conversation.title : nil) }
    var isGenerating: Bool { snapshot?.runState == .running || snapshot?.runState == .stopping }
    var isBusy: Bool { isLoading || applicationIsGenerating || isShuttingDown || authenticationRunning }
    var canManageConversations: Bool { !authenticationRunning && !isShuttingDown && conversationManagementStatus == nil && (!isLoading || pendingConversationID != nil) }
    /// A running conversation keeps its native session until the turn ends.
    func isRunning(_ conversation: Conversation) -> Bool { runningConversationIDs.contains(conversation.id) || (loadedConversationID == conversation.id && (isGenerating || executionStarting)) }
    /// The core validates the selection again; an unread catalog does not block sending.
    var canSend: Bool { !executionStarting && !authenticationRunning && !isLoading && !isGenerating && !isShuttingDown && (snapshot?.actions.canSend ?? false) && nextTurnModel != nil }
    var canCancel: Bool { snapshot?.actions.canCancel ?? false }
    var followUp: FollowUp { snapshot?.followUp ?? FollowUp() }
    /// The loaded conversation's native sub-agents.
    var subAgents: [SubAgent] { pendingConversationID == nil ? snapshot?.subAgents ?? [] : [] }
    func isSubAgentRunning(_ id: String) -> Bool { runningConversationIDs.contains(id) || elsewhereConversationIDs.contains(id) }
    /// While a turn runs, a sent message waits in the conversation's queue.
    var canQueue: Bool { !isPreview && isGenerating && !isLoading && !isShuttingDown && !authenticationRunning && nextTurnModel != nil && loadedConversationID != nil && snapshot?.conversation.parentConversationID == nil }
    /// Asking to continue sends this, as the user's own message.
    static let continueText = "继续"
    var canSwitchModel: Bool { nextTurnRuntimeID != nil && !isShuttingDown }
    var modelCatalog: RuntimeModelCatalog? { nextTurnRuntimeID.flatMap { modelCatalogs[$0] } }
    var needsModelSelection: Bool {
        if case .loading = modelCatalog { return false }
        return snapshot != nil && nextTurnModel == nil
    }
    /// The catalog once read. Until then, and when it cannot be read, the
    /// configured managed models the runtime can reach, without levels.
    var modelChoices: [ModelChoice] {
        if case .ready(let choices) = modelCatalog { return choices }
        let typeID = runtimeInstances.first { $0.id == nextTurnRuntimeID }?.typeID
        guard let descriptor = runtimeTypes.first(where: { $0.id == typeID }) else { return [] }
        return gateway.providers.filter { descriptor.supportedProviderProtocols.contains($0.protocolID) }.flatMap { provider in
            provider.models.map { ModelChoice(selection: .managed(recordKey: $0.recordKey), displayName: "\($0.displayName) · \(provider.name)") }
        }
    }
    var selectedModelName: String? {
        guard let nextTurnModel else { return nil }
        if let choice = modelChoices.first(where: { $0.selection == nextTurnModel }) { return choice.displayName }
        return switch nextTurnModel {
        case .runtimeDefault: "运行时默认模型"
        case .managed: nil
        case .native(_, let modelID): modelID
        }
    }
    /// Reasoning levels the next turn's runtime and model accept, in the runtime's own vocabulary.
    var reasoningEffortOptions: [String] { modelChoices.first { $0.selection == nextTurnModel }?.reasoningEfforts ?? [] }
    /// Permission modes the next turn's runtime type declares.
    var permissionModeOptions: [PermissionMode] {
        let type = runtimeInstances.first { $0.id == nextTurnRuntimeID }?.typeID
        return runtimeTypes.first { $0.id == type }?.permissionModes ?? []
    }
    /// The remembered mode while the next turn's runtime offers it; otherwise
    /// none, and the turn keeps the conversation's current mode.
    var effectivePermissionMode: PermissionMode? { permissionModeOptions.first { $0.id == nextTurnPermissionMode } }
    /// The conversation's current mode, as its last turn on the next turn's
    /// runtime recorded it; a mode is the session's state in the runtime.
    var currentPermissionMode: PermissionMode? {
        guard let settings = currentTurnSettings else { return nil }
        return permissionModeOptions.first { $0.id == settings.permissionMode }
    }
    /// The conversation's current level, as its last turn on the next turn's
    /// runtime recorded it, while the chosen model offers it.
    var currentEffort: String? { currentTurnSettings?.effort.flatMap { reasoningEffortOptions.contains($0) ? $0 : nil } }
    /// Settings of the conversation's last turn when it ran on the next turn's runtime.
    private var currentTurnSettings: TurnSettings? {
        guard let settings = snapshot?.transcriptTurns.last?.settings, settings.runtimeID == nextTurnRuntimeID else { return nil }
        return settings
    }
    /// The model of the conversation's latest turn on a runtime, among the
    /// choices it offers. A turn records the name Velune sent, the native
    /// model ID, or a provider's model ID; the last counts only when one
    /// provider model has it.
    private func latestTurnModel(on runtimeID: String) -> ModelSelection? {
        guard let model = snapshot?.transcriptTurns.last(where: { $0.settings?.runtimeID == runtimeID && $0.settings?.model != nil })?.settings?.model else { return nil }
        let choices = modelChoices
        if let choice = choices.first(where: { $0.displayName == model }) { return choice.selection }
        if let choice = choices.first(where: { if case .native(_, model) = $0.selection { true } else { false } }) { return choice.selection }
        let keys = Set(gateway.providers.flatMap(\.models).filter { $0.providerModelID == model }.map(\.recordKey))
        let managed = choices.filter { if case .managed(let key) = $0.selection { keys.contains(key) } else { false } }
        return managed.count == 1 ? managed[0].selection : nil
    }
    /// A turn's settings in the names the composer uses (an unknown runtime
    /// or a mode its type does not declare shows as recorded), and its usage.
    /// Velune's models by record key, named as the composer names them.
    var veluneModelNames: [(key: String, name: String)] {
        gateway.providers.flatMap { provider in provider.models.map { (key: $0.recordKey, name: "\($0.displayName) · \(provider.name)") } }
    }
    /// Where the loaded conversation stands, from its turns' native records.
    var conversationStatus: ConversationStatus {
        let turns = snapshot?.transcriptTurns ?? []
        let latest = turns.last { $0.usage != nil }?.usage
        func total(_ count: KeyPath<TurnUsage, UInt64?>) -> UInt64? {
            let counts = turns.compactMap { $0.usage?[keyPath: count] }
            return counts.isEmpty ? nil : counts.reduce(0, +)
        }
        return ConversationStatus(
            conversationID: isDraft ? nil : snapshot?.conversation.id,
            endedTurns: turns.filter { !$0.isRunning }.count,
            contextTokens: latest?.contextTokens, contextWindow: latest?.contextWindow,
            inputTokens: total(\.inputTokens), cacheReadTokens: total(\.cacheReadTokens),
            outputTokens: total(\.outputTokens), reasoningTokens: total(\.reasoningTokens),
            unrecordedTurns: turns.filter { !$0.isRunning && $0.usage == nil }.count,
            directory: isDraft ? nil : snapshot?.conversation.cwd,
            rateLimits: turns.last { $0.usage?.rateLimits.isEmpty == false }?.usage?.rateLimits ?? [],
            costMicroUSD: turns.last { $0.usage?.sessionCostMicroUSD != nil }?.usage?.sessionCostMicroUSD)
    }
    func describe(_ turn: TranscriptTurn) -> TurnSettingsSummary? {
        let settings = turn.settings ?? TurnSettings()
        let runtime = settings.runtimeID.flatMap { id in runtimeInstances.first { $0.id == id } }
        let type = runtime.flatMap { instance in runtimeTypes.first { $0.id == instance.typeID } }
        let parts: [TurnSettingsSummary.Part] = [
            settings.runtimeID.map { .init(label: "运行时", value: runtime?.name ?? $0) },
            settings.model.map { .init(label: "模型", value: $0) },
            settings.effort.map { .init(label: "推理强度", value: $0) },
            settings.permissionMode.map { mode in .init(label: "权限", value: type?.permissionModes.first { $0.id == mode }?.label ?? mode) },
        ].compactMap { $0 }
        let brief = [runtime?.name ?? settings.runtimeID, settings.model].compactMap { $0 }.joined(separator: " · ")
        let usage = turn.usage.map { usage -> [TurnSettingsSummary.Part] in
            let count = { (value: UInt64?) in value.map { $0.formatted() } }
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
    func selectPermissionMode(_ mode: String?) {
        guard mode == nil || permissionModeOptions.contains(where: { $0.id == mode }) else { return }
        nextTurnPermissionMode = mode
        rememberNextTurnIntent()
    }
    func selectNextTurnRuntime(_ id: String) {
        guard !isShuttingDown, enabledRuntimeInstances.contains(where: { $0.id == id }) else { return }
        hasInitializedNextTurnIntent = true
        switchNextTurnRuntime(to: id)
        if let intentConversationID { nextTurnIntents[intentConversationID, default: NextTurnIntent()].runtimeID = id }
    }
    /// The runtime's own choices in this conversation come back with it.
    private func switchNextTurnRuntime(to id: String) {
        let choice = intentConversationID.flatMap { nextTurnIntents[$0]?.choices[id] } ?? RuntimeChoice()
        nextTurnModel = choice.model
        nextTurnEffort = choice.effort
        nextTurnPermissionMode = choice.permissionMode
        if nextTurnRuntimeID == id { reconcileModelSelection() } else { nextTurnRuntimeID = id }
    }
    var pendingInteractions: [RuntimeInteraction] { snapshot?.pendingInteractions ?? [] }
    func replyInteraction(_ interaction: RuntimeInteraction, reply: RuntimeInteractionReply) {
        guard pendingInteractions.contains(where: { $0.id == interaction.id }), let runtimeID = projectionRuntimeID, let conversationID = loadedConversationID, let transport else { return }
        let currentGeneration = generation
        enqueue({ try transport.replyRuntimeInteraction(runtimeID: runtimeID, conversationID: conversationID, interactionID: interaction.id, reply: reply) }) { [weak self] value in
            guard let self, generation == currentGeneration else { return }; applySnapshotResult(value)
        }
    }
    /// Turns or sub-agents run in Velune's runtime processes; quitting ends them.
    var hasRunningWork: Bool { applicationIsGenerating }
    func shutdown(stoppingWork: Bool = false, completion: @escaping (Bool) -> Void) {
        if sharedApplication != nil { subscriptions.removeAll(); timer?.invalidate(); timer = nil; completion(true); return }
        guard !isShuttingDown else { completion(false); return }
        guard !isPreview, let transport else { completion(true); return }
        saveDrafts()
        isShuttingDown = true
        timer?.invalidate(); timer = nil
        queue.async { [weak self] in
            let result = Result { try transport.close(stoppingWork: stoppingWork) }
            DispatchQueue.main.async {
                guard let self else { completion(false); return }
                self.isShuttingDown = false
                switch result {
                case .success:
                    self.timer?.invalidate(); self.timer = nil
                    self.generation += 1
                    self.started = false
                    completion(true)
                case .failure(let failure):
                    self.recordProblem(failure)
                    self.schedulePolling()
                    completion(false)
                }
            }
        }
    }
    /// The application store follows login and which conversations execute;
    /// each workspace follows the conversation it shows.
    private func schedulePolling() {
        timer?.invalidate()
        timer = Timer.scheduledTimer(withTimeInterval: 0.6, repeats: true) { [weak self] _ in
            Task { @MainActor in
                guard let self else { return }
                if self.sharedApplication == nil { self.pollAuthentication(); self.pollActiveConversations() } else { self.poll() }
            }
        }
    }
    private var activePollPending = false
    private var activityCursor: UInt64 = 0
    private var notifier: ConversationNotifier?
    /// The workspace whose window was key most recently; notifications open
    /// conversations there.
    private weak var focusedWorkspace: AppStore?
    func noteFocused() { sharedApplication?.focusedWorkspace = self; noteSeen() }
    /// The user is looking at this conversation now.
    func isShowingConversation(_ id: String) -> Bool { NSApp.isActive && focusedWorkspace?.loadedConversationID == id }
    func showConversation(_ id: String) {
        guard let workspace = focusedWorkspace else { return }
        if workspace.loadedConversationID != id { workspace.selectConversation(id: id) }
    }
    private func pollActiveConversations() {
        guard started, !isPreview, !isShuttingDown, !activePollPending, let transport else { return }
        activePollPending = true
        let since = activityCursor
        queue.async { [weak self] in
            let result = Result { try transport.conversationActivity(since: since) }
            DispatchQueue.main.async {
                guard let self else { return }
                self.activePollPending = false
                switch result {
                case .success(let activity):
                    self.clearActivityProblem("active-conversations")
                    self.activityCursor = activity.cursor
                    let active = activity.active
                    let running = Set(active.filter { $0.runState == .running || $0.runState == .stopping }.map(\.conversationId))
                    if self.runningConversationIDs != running { self.runningConversationIDs = running }
                    let elsewhere = Set(active.filter { $0.runState == .elsewhere }.map(\.conversationId))
                    if self.elsewhereConversationIDs != elsewhere { self.elsewhereConversationIDs = elsewhere }
                    let parents = Set(active.compactMap { item in
                        [.running, .stopping, .elsewhere].contains(item.runState) ? item.parentConversationId : nil
                    })
                    if self.subAgentParentIDs != parents { self.subAgentParentIDs = parents }
                    let waiting = Set(active.filter { $0.pendingInteractions > 0 }.map(\.conversationId))
                    if self.waitingConversationIDs != waiting { self.waitingConversationIDs = waiting }
                    // A conversation started outside Velune, or by an agent through
                    // Velune, appears once the list is read again; a sub-agent is
                    // listed under its parent instead.
                    let subAgents = Set(active.filter { $0.parentConversationId != nil }.map(\.conversationId))
                    let unlisted = elsewhere.union(running).subtracting(subAgents).filter { id in !self.conversations.contains { $0.id == id } }
                    if !unlisted.isSubset(of: self.unlistedElsewhereIDs) { self.refreshConversations() }
                    self.unlistedElsewhereIDs = unlisted
                    self.notifier?.observe(activity)
                case .failure(let failure): self.recordProblem(failure, source: "读取运行状态", activityKey: "active-conversations")
                }
            }
        }
    }
    /// Begin reading. Once the list is read, a workspace opens `conversationID`,
    /// else the list's first conversation, or an unsent one when asked for
    /// (`newConversation`) or when the list is empty.
    func start(loadFirstConversation: Bool = true, newConversation: Bool = false, opening conversationID: String? = nil) {
        guard !started else { return }; started = true
        sharedApplication?.start(loadFirstConversation: false)
        guard !isPreview, let transport else { return }
        if sharedApplication == nil { notifier = ConversationNotifier(application: self) }
        if sharedApplication == nil { loadDrafts(transport) }
        enqueue({ try transport.list() }) { [weak self] data in
            guard let self else { return }
            applyList(data)
            schedulePolling()
            if let conversationID { openLinkedConversation(id: conversationID) }
            else if loadFirstConversation, let id = sidebarConversations.first?.id { selectConversation(id: id) }
            // With nothing to show yet, a window starts an unsent conversation.
            else if newConversation || loadFirstConversation, snapshot == nil, pendingConversationID == nil { createConversation() }
        }
    }
    func refreshConversations() {
        guard !isPreview, !isLoading, !isShuttingDown, let transport else { return }
        enqueue({ try transport.list() }) { [weak self] in self?.applyList($0) }
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
        enqueue({ () -> (BindingConfigurationSnapshot, BindingSnapshotResult?) in
            (try transport.list(), try open.map { try transport.openConversation(runtimeID: $0.runtimeID, conversationID: $0.id) })
        }) { [weak self] result in
            guard let self else { return }
            applyList(result.0)
            if let conversation = result.1 { applySnapshotResult(conversation) }
        }
    }
    func loadAnalytics(_ query: BindingAnalyticsQuery) {
        analyticsGeneration &+= 1
        let requestGeneration = analyticsGeneration
        analyticsReadFailed = false
        guard !isPreview else { analyticsIsLoading = false; return }
        guard !isShuttingDown, let transport else { analyticsIsLoading = false; analyticsReadFailed = true; recordProblem("本地核心不可用", source: "读取分析记录"); return }
        analyticsIsLoading = true
        queue.async { [weak self] in
            let result = Result { try transport.analyticsQuery(query) }
            DispatchQueue.main.async {
                guard let self, self.analyticsGeneration == requestGeneration else { return }
                self.analyticsIsLoading = false
                switch result {
                case .success(let report):
                    self.analyticsReport = report; self.analyticsLoadedQuery = query; self.analyticsUpdatedAt = Date()
                    if let warning = report.storageWarning { self.recordProblem(TransportError.rejected(warning), source: "保存分析记录", activityKey: "analytics:storage") }
                    else { self.clearActivityProblem("analytics:storage") }
                case .failure(let failure): self.analyticsReadFailed = true; self.recordProblem(failure)
                }
            }
        }
    }
    /// Git changes in the loaded conversation's working directory.
    func readWorkingChanges(_ apply: @escaping (Result<BindingWorkingChanges, Error>) -> Void) {
        guard !isPreview, !isShuttingDown, let transport, let conversation = snapshot?.conversation, !isDraft else { return }
        queue.async {
            let result = Result { try transport.workingChanges(runtimeID: conversation.runtimeID, conversationID: conversation.id) }
            DispatchQueue.main.async { apply(result) }
        }
    }
    /// The branch checked out in the loaded conversation's working directory;
    /// `nil` when there is none or it cannot be read (a remote runtime).
    func readWorkingBranch(_ apply: @escaping (String?) -> Void) {
        guard !isPreview, !isShuttingDown, let transport, let conversation = snapshot?.conversation, !isDraft else { apply(nil); return }
        queue.async {
            let branch = (try? transport.workingBranch(runtimeID: conversation.runtimeID, conversationID: conversation.id)) ?? nil
            DispatchQueue.main.async { apply(branch) }
        }
    }
    func readFileDiff(path: String, _ apply: @escaping (Result<BindingFileDiff, Error>) -> Void) {
        guard !isPreview, !isShuttingDown, let transport, let conversation = snapshot?.conversation, !isDraft else { return }
        queue.async {
            let result = Result { try transport.fileDiff(runtimeID: conversation.runtimeID, conversationID: conversation.id, path: path) }
            DispatchQueue.main.async { apply(result) }
        }
    }
    func cancelAnalyticsRead() { analyticsGeneration &+= 1; analyticsIsLoading = false }
    func setConversationBrowserPreferences(_ preferences: BindingConversationBrowserPreferences) {
        guard preferences != conversationBrowserPreferences, !isShuttingDown else { return }
        if isPreview { conversationBrowserPreferences = preferences; return }
        guard let transport else { recordProblem("本地核心未配置"); return }
        browserPreferencesGeneration &+= 1
        let revision = browserPreferencesGeneration
        conversationBrowserPreferences = preferences
        browserPreferencesSaving += 1
        // Sidebar choices are independent of loading/execution. Serialize their
        // commits with core operations without locking ordinary browsing.
        queue.async { [weak self] in
            let result = Result { try transport.setConversationBrowserPreferences(preferences) }
            DispatchQueue.main.async {
                guard let self else { return }
                self.browserPreferencesSaving -= 1
                switch result {
                case .success(let data):
                    let saved = data.conversationBrowserPreferences
                    self.savedBrowserPreferences = saved
                    if revision == self.browserPreferencesGeneration { self.conversationBrowserPreferences = saved }
                    self.applyList(data)
                    self.refreshSearch()
                case .failure(let failure):
                    self.recordProblem(failure)
                    if revision == self.browserPreferencesGeneration { self.conversationBrowserPreferences = self.savedBrowserPreferences }
                }
            }
        }
    }
    func setStatusLine(_ items: [BindingStatusLineItem]) {
        if isPreview { statusLine = items; return }
        guard let transport else { recordProblem("本地核心未配置"); return }
        enqueue({ try transport.setStatusLine(items) }) { [weak self] in self?.statusLine = $0 }
    }
    func setOutlinePresentation(_ presentation: BindingOutlinePresentation) {
        if isPreview { outlinePresentation = presentation; return }
        guard let transport else { recordProblem("本地核心未配置"); return }
        enqueue({ try transport.setOutlinePresentation(presentation) }) { [weak self] in self?.outlinePresentation = $0 }
    }
    func draft(for conversationID: String) -> String {
        if let sharedApplication { return sharedApplication.draft(for: conversationID) }
        return composerDrafts[conversationID] ?? ""
    }
    /// Records the composer text; persisted shortly after typing pauses. A new
    /// conversation's draft lives only as long as that unsent conversation.
    func setDraft(_ text: String, for conversationID: String) {
        if let sharedApplication { sharedApplication.setDraft(text, for: conversationID); return }
        guard composerDrafts[conversationID, default: ""] != text else { return }
        composerDrafts[conversationID] = text.isEmpty ? nil : text
        guard !isPreview, !conversationID.hasPrefix(Self.draftConversationPrefix) else { return }
        unsavedDrafts[conversationID] = text
        draftSave?.cancel()
        let work = DispatchWorkItem { [weak self] in self?.saveDrafts() }
        draftSave = work
        DispatchQueue.main.asyncAfter(deadline: .now() + 0.5, execute: work)
    }
    private func moveDraft(from old: String, to new: String) {
        if let sharedApplication { sharedApplication.moveDraft(from: old, to: new); return }
        guard let text = composerDrafts.removeValue(forKey: old) else { return }
        setDraft(text, for: new)
    }
    private func loadDrafts(_ transport: Transport) {
        queue.async { [weak self] in
            let result = Result { try transport.composerDrafts() }
            DispatchQueue.main.async {
                guard let self else { return }
                switch result {
                case .success(let drafts):
                    for draft in drafts where self.composerDrafts[draft.conversationId] == nil { self.composerDrafts[draft.conversationId] = draft.text }
                case .failure(let failure): self.recordProblem(failure, source: "读取草稿")
                }
            }
        }
    }
    /// Draft writes bypass the operation queue's loading state: typing never
    /// waits for, or blocks, a conversation operation.
    private func saveDrafts() {
        draftSave?.cancel(); draftSave = nil
        guard !unsavedDrafts.isEmpty, let transport else { return }
        let pending = unsavedDrafts
        unsavedDrafts = [:]
        queue.async { [weak self] in
            var failure: Error?
            for (id, text) in pending {
                do { try transport.setComposerDraft(conversationID: id, text: text) } catch { failure = error }
            }
            guard let failure else { return }
            DispatchQueue.main.async { self?.recordProblem(failure, source: "保存草稿") }
        }
    }
    static let draftConversationPrefix = "draft:"
    func setConversationBrowserLimits(_ limits: BindingConversationBrowserLimits) {
        guard limits.page > 0, limits.total > 0 else { recordProblem("会话加载数量须为正整数"); return }
        if isPreview { conversationBrowserLimits = limits; return }
        guard let transport else { recordProblem("本地核心未配置"); return }
        browse({ _ = try transport.setConversationBrowserLimits(limits); return try transport.list() }, source: "保存会话列表设置")
    }
    /// List operations run besides loading and management: they change only
    /// what the list shows and Velune's organization of it.
    private func browse(_ operation: @escaping () throws -> BindingConfigurationSnapshot, source: String, finished: (() -> Void)? = nil) {
        guard !isPreview, !isShuttingDown else { finished?(); return }
        queue.async { [weak self] in
            let result = Result { try operation() }
            DispatchQueue.main.async {
                defer { finished?() }
                guard let self else { return }
                switch result {
                case .success(let data): self.applyList(data)
                case .failure(let failure): self.recordProblem(failure, source: source)
                }
            }
        }
    }
    func loadMoreConversations(in group: BindingConversationGroupKey) {
        guard let transport else { return }
        browse({ try transport.loadMoreConversations(group: group) }, source: "加载会话")
    }
    func setShowsArchived(_ archived: Bool) {
        guard let transport, archived != showsArchived else { return }
        showsArchived = archived
        browse({ try transport.showArchivedConversations(archived) }, source: "切换已归档会话")
    }
    /// Shapes the project whose primary folder is `path`.
    func setProject(path: String, primaryFolder: String, name: String?, folders: [String]) {
        guard let transport else { return }
        browse({ try transport.setProject(path: path, primaryFolder: primaryFolder, name: name, folders: folders) }, source: "编辑项目")
    }
    /// Pins, read state, and moves are Velune's records; an archive the
    /// runtime keeps itself changes its sessions, so it waits for loading
    /// like the other native changes.
    func organize(_ targets: [Conversation], _ change: BindingConversationOrganizationChange) {
        guard !targets.isEmpty, let transport else { return }
        let references = targets.map { BindingConversationReference(runtimeId: $0.runtimeID, conversationId: $0.id) }
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
            browse({ try transport.organizeConversations(references, change) }, source: "整理会话")
        }
    }
    /// The open conversation is read once it shows in the focused window of
    /// the active application.
    private func noteSeen() {
        guard !isPreview, NSApp.isActive else { return }
        let application = sharedApplication ?? self
        guard sharedApplication == nil || application.focusedWorkspace === self,
              let id = loadedConversationID, pendingConversationID == nil,
              let conversation = conversation(id: id), conversation.unread, !application.markingRead.contains(id),
              let transport else { return }
        application.markingRead.insert(id)
        let reference = BindingConversationReference(runtimeId: conversation.runtimeID, conversationId: id)
        browse({ try transport.organizeConversations([reference], .markRead) }, source: "整理会话") { [weak application] in
            application?.markingRead.remove(id)
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
        queue.async { [weak self] in
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
        guard !isLoading, !isShuttingDown, !authenticationRunning else { return }
        selectedConversationIDs = ids.filter { conversation(id: $0) != nil }
        if selectedConversationIDs.count == 1, let id = selectedConversationIDs.first, id != loadedConversationID { selectConversation(id: id) }
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
        if conversation(id: id) != nil { selectConversation(id: id, move: move); return }
        guard !isPreview, !isShuttingDown, let transport else { return }
        queue.async { [weak self] in
            let result = Result { try transport.findConversation(id) }
            DispatchQueue.main.async {
                guard let self else { return }
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
        guard let conversation = conversation(id: id) else { return }
        // Chosen while another operation runs, it opens once that is done; the latest choice wins.
        guard !isLoading, !executionStarting else { deferredSelection = (id, move); return }
        deferredSelection = nil
        selectedConversationIDs = [id]
        if isPreview { projectionRuntimeID = conversation.runtimeID; apply(previewSnapshots[id]); return }
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
            if let index = conversations.firstIndex(where: { $0.id == conversation.id }) { conversations[index].title = name }
            previewSnapshots[conversation.id]?.conversation.title = name
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
                if snapshot?.conversation.id == conversation.id, let renamed = conversations.first(where: { $0.id == conversation.id }) { snapshot?.conversation = renamed }
                finishConversationManagement()
                onRenamed()
            }
        }
    }
    func deleteConversation(_ conversation: Conversation) { deleteConversations([conversation]) }
    /// The core already removed the persisted draft with the conversation.
    private func dropDeletedConversationState(_ id: String) {
        nextTurnIntents[id] = nil
        if let sharedApplication { sharedApplication.composerDrafts[id] = nil; sharedApplication.unsavedDrafts[id] = nil }
        else { composerDrafts[id] = nil; unsavedDrafts[id] = nil }
    }
    func deleteConversations(_ targets: [Conversation]) {
        guard canManageConversations else { recordProblem("请等待正在进行的操作结束，再修改会话"); return }
        guard !targets.isEmpty, targets.allSatisfy(canDeleteConversation) else { recordProblem("所选会话中有原生会话当前无法删除"); return }
        if isPreview {
            let ids = Set(targets.map(\.id))
            conversations.removeAll { ids.contains($0.id) }; selectedConversationIDs.subtract(ids)
            for id in ids { dropDeletedConversationState(id) }
            for id in ids { previewSnapshots.removeValue(forKey: id) }
            if let loadedConversationID, ids.contains(loadedConversationID) { resetProjection() }
            return
        }
        guard let transport else { recordProblem("本地核心未配置"); return }
        // A pending write must not recreate the draft of a deleted conversation.
        for target in targets { (sharedApplication ?? self).unsavedDrafts[target.id] = nil }
        scheduleConversationManagement("删除 \(targets.count) 个会话") { [weak self] in
            guard let self else { return }
            generation += 1
            conversationManagementStatus = "正在删除 \(targets.count) 个会话…"
            let loadedID = loadedConversationID
            let contextID = snapshot?.contextRuntimeID
            // Runtime-owned deletions are independent operations. Preserve each
            // success and report failures rather than promising a transaction.
            enqueue({
                var latest: BindingConfigurationSnapshot?
                var deleted: Set<String> = []
                var failures: [AppProblem] = []
                var refreshed: BindingSnapshotResult?
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
                if let data = result.0 { applyList(data) }
                selectedConversationIDs.subtract(result.1)
                for id in result.1 { dropDeletedConversationState(id) }
                sharedApplication?.deletedConversationIDs.formUnion(result.1)
                if let loadedConversationID, result.1.contains(loadedConversationID) { resetProjection() }
                else if result.4 { resetProjection() }
                else if let refreshed = result.3 { applySnapshotResult(refreshed) }
                finishConversationManagement()
                appendProblems(result.2)
            }
        }
    }
    /// Opens an unsent conversation: an empty transcript and composer. The
    /// runtime and model start from the current next-turn choice and can be
    /// changed in the composer; the conversation is created at the first send.
    func createConversation() {
        guard !isLoading, !executionStarting else { return }
        guard let runtimeID = enabledRuntimeInstances.first(where: { $0.id == nextTurnRuntimeID })?.id ?? enabledRuntimeInstances.first?.id else { return }
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
    /// `onAccepted` receives the conversation that took the turn; sending in a
    /// new conversation replaces its unsent identity with the created one.
    /// Where pasted images are kept so they can be attached by path.
    var attachmentsDirectory: URL {
        (transport?.stateDirectory ?? FileManager.default.temporaryDirectory).appendingPathComponent("attachments", isDirectory: true)
    }
    /// Returns whether the turn was sent; the message shows at once, marked as
    /// sending until the runtime accepts it. `onFailure` runs when it was not.
    @discardableResult
    func send(text: String, attachments: [String] = [], onAccepted: ((String?) -> Void)? = nil, onFailure: (() -> Void)? = nil) -> Bool {
        guard !text.trimmingCharacters(in: .whitespacesAndNewlines).isEmpty || !attachments.isEmpty else { return false }
        guard canSend, let target = nextTurnRuntimeID, let model = nextTurnModel, let source = snapshot?.conversation.runtimeID, let conversationID = loadedConversationID else { return false }
        if isPreview {
            guard var value = snapshot else { return false }
            if case .managed(let key) = model { value.modelRecordKey = key }
            let message = Message(id: UUID().uuidString, role: .user, blocks: [.text(text)])
            value.messages.append(message)
            value.transcriptItems.append(.message(id: message.id, messageID: message.id))
            value.transcriptOutline.append(TranscriptOutlineEntry(id: message.id, messageID: message.id))
            value.transcriptMessageIdentities.append(TranscriptMessageIdentity(id: message.id, messageID: message.id))
            apply(value); onAccepted?(loadedConversationID); return true
        }
        guard let transport else { recordProblem("本地核心未配置"); return false }
        executionStarting = true
        pendingSend = PendingSend(previousUserMessageID: snapshot?.messages.last { $0.role == .user }?.id)
        let sent: [MessageBlock] = (text.isEmpty ? [] : [.text(text)]) + attachments.map { path in
            Self.imageExtensions.contains(URL(fileURLWithPath: path).pathExtension.lowercased()) ? .image(mediaType: nil, source: .path(path)) : .file(path: path)
        }
        transcript.showPending(Message(id: "pending:" + UUID().uuidString, role: .user, blocks: sent), status: "发送中")
        let creating = isDraft
        let cwd = snapshot?.conversation.cwd
        // Without a level or mode the core keeps the conversation's current one.
        let turn = BindingComposedTurn(text: text, effort: nextTurnEffort.flatMap { reasoningEffortOptions.contains($0) ? $0 : nil },
                                       permissionMode: effectivePermissionMode?.id, attachments: attachments)
        enqueue({
            if creating {
                return try transport.createAndSend(runtimeID: target, cwd: cwd, selection: model, turn: turn) { [weak self] created in
                    DispatchQueue.main.async {
                        guard let self else { return }
                        self.isDraft = false
                        // Text typed while sending and the next-turn choice follow the created conversation.
                        if let id = created.snapshot?.conversation.id {
                            self.moveDraft(from: conversationID, to: id)
                            self.nextTurnIntents[id] = self.nextTurnIntents.removeValue(forKey: conversationID)
                        }
                        self.applySnapshotResult(created)
                    }
                }
            }
            return try transport.sendTurn(sourceRuntimeID: source, conversationID: conversationID, targetRuntimeID: target, selection: model, turn: turn)
        }, onAccepted: { [weak self] in onAccepted?(self?.loadedConversationID) }, onFailure: { [weak self] _ in
            self?.executionStarting = false
            self?.endPendingSend()
            onFailure?()
        }) { [weak self] value in
            guard let self else { return }
            self.executionStarting = false
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
        enqueue({ try transport.cancel(runtimeID: runtimeID, conversationID: conversationID) }) { [weak self] in self?.applySnapshotResult($0) }
    }
    /// Queue a message after the running turn, with the next-turn choices as
    /// they are now. `onFailure` runs when it was not queued.
    @discardableResult
    func queue(text: String, attachments: [String] = [], onFailure: (() -> Void)? = nil) -> Bool {
        guard !text.trimmingCharacters(in: .whitespacesAndNewlines).isEmpty || !attachments.isEmpty else { return false }
        guard canQueue, let target = nextTurnRuntimeID, let model = nextTurnModel, let source = snapshot?.conversation.runtimeID else { return false }
        let turn = BindingComposedTurn(text: text, effort: nextTurnEffort.flatMap { reasoningEffortOptions.contains($0) ? $0 : nil },
                                       permissionMode: effectivePermissionMode?.id, attachments: attachments)
        changeQueue(.add(sourceRuntimeId: source, targetRuntimeId: target, selection: BindingMapping.bindingSelection(model), turn: turn), onFailure: onFailure)
        return true
    }
    /// Continue after a turn that stopped or failed: a real message, not a replay.
    func continueTurn() { send(text: Self.continueText) }
    var canSteer: Bool { canQueue && followUp.canSteer }
    /// Into the running turn; the turn keeps its own choices.
    func steer(text: String, attachments: [String] = [], onFailure: (() -> Void)? = nil) {
        guard canSteer, let runtimeID = projectionRuntimeID, let conversationID = loadedConversationID, let transport else { onFailure?(); return }
        let turn = BindingComposedTurn(text: text, effort: nil, permissionMode: nil, attachments: attachments)
        enqueue({ try transport.steerTurn(runtimeID: runtimeID, conversationID: conversationID, turn: turn) }, onFailure: { _ in onFailure?() }) { [weak self] in self?.applySnapshotResult($0) }
    }
    /// Whether the user message may be edited now, and how. The edit goes to
    /// the conversation's own runtime with the next turn's choices.
    func editability(of messageID: String) -> EditableMessage? {
        guard !isPreview, !isDraft, canSend, nextTurnRuntimeID == snapshot?.conversation.runtimeID else { return nil }
        return followUp.editable.first { $0.messageID == messageID }
    }
    func message(id: String) -> Message? { snapshot?.messages.first { $0.id == id } }
    func isLastUserMessage(_ messageID: String) -> Bool { snapshot?.messages.last { $0.role == .user }?.id == messageID }
    /// Send `text` in place of a user message. In place the conversation
    /// shows the new turn here; a fork's new conversation goes to `forked`.
    func editMessage(_ messageID: String, placement: BindingEditPlacement, text: String, attachments: [String], forked: @escaping (BindingSnapshotResult) -> Void) {
        guard editability(of: messageID) != nil, let model = nextTurnModel, let runtimeID = snapshot?.conversation.runtimeID, let conversationID = loadedConversationID else { return }
        guard let transport else { recordProblem("本地核心未配置"); return }
        executionStarting = true
        let turn = BindingComposedTurn(text: text, effort: nextTurnEffort.flatMap { reasoningEffortOptions.contains($0) ? $0 : nil },
                                       permissionMode: effectivePermissionMode?.id, attachments: attachments)
        enqueue({ try transport.editMessage(runtimeID: runtimeID, conversationID: conversationID, messageID: messageID, placement: placement, selection: model, turn: turn) },
                onFailure: { [weak self] _ in self?.executionStarting = false }) { [weak self] value in
            guard let self else { return }
            self.executionStarting = false
            if placement == .fork { forked(value); self.refreshConversations() } else { self.applySnapshotResult(value) }
        }
    }
    /// Show a conversation the core has already opened, as `result` gives it.
    func adopt(_ result: BindingSnapshotResult) { applySnapshotResult(result) }
    func steerQueued(_ id: String) { changeQueue(.steer(id: id)) }
    func editQueued(_ id: String, text: String) { changeQueue(.edit(id: id, text: text)) }
    func removeQueued(_ id: String) { changeQueue(.remove(id: id)) }
    func moveQueued(_ id: String, to index: Int) { changeQueue(.move(id: id, index: UInt32(max(index, 0)))) }
    func resumeQueue() { changeQueue(.resume) }
    private func changeQueue(_ operation: BindingSendQueueOperation, onFailure: (() -> Void)? = nil) {
        guard !isPreview else { return }
        guard let runtimeID = projectionRuntimeID, let conversationID = loadedConversationID else { recordProblem("请选择会话"); onFailure?(); return }
        guard let transport else { recordProblem("本地核心未配置"); onFailure?(); return }
        enqueue({ try transport.sendQueue(runtimeID: runtimeID, conversationID: conversationID, operation: operation) }, onFailure: { _ in onFailure?() }) { [weak self] in self?.applySnapshotResult($0) }
    }
    func saveProvider(_ value: AIProvider, authenticationEdit: AuthenticationEdit, onSaved: (() -> Void)? = nil) {
        guard !authenticationRunning, !isGenerating else { recordProblem("请先停止当前任务或完成登录"); return }
        if isPreview { var saved = value; if saved.id.isEmpty { saved.id = UUID().uuidString }; for index in saved.models.indices where saved.models[index].recordKey.isEmpty { saved.models[index].recordKey = UUID().uuidString }; gateway.providers.removeAll { $0.id == saved.id }; gateway.providers.append(saved); hasGateway = true; onSaved?(); return }
        guard let transport else { recordProblem("本地核心未配置"); return }
        enqueue({ try transport.saveProvider(gatewayID: self.gateway.id, provider: value, authenticationEdit: authenticationEdit) }) { [weak self] data in
            self?.applyGatewayUpdate(data); onSaved?()
        }
    }
    /// Reads the providers' quota without holding up other work. Reading
    /// samples a provider whose observation has expired; `refresh` asks for a
    /// sample now. Either returns at once, so callers read again while
    /// `refreshing` holds.
    func readProviderQuotas(_ providerIDs: [String], refresh: Bool = false) {
        let ids = providerIDs.filter { !quotaReadsPending.contains($0) }
        guard !ids.isEmpty, !isPreview, !isShuttingDown, let transport else { return }
        quotaReadsPending.formUnion(ids)
        let gatewayID = gateway.id
        queue.async { [weak self] in
            let results = ids.map { id in (id, Result { try transport.providerQuota(gatewayID: gatewayID, providerID: id, refresh: refresh) }) }
            DispatchQueue.main.async {
                guard let self else { return }
                self.quotaReadsPending.subtract(ids)
                for (id, result) in results {
                    switch result {
                    case .success(let quota): self.providerQuotas[id] = quota; self.clearActivityProblem("quota:\(id)")
                    case .failure(let failure):
                        // A provider deleted meanwhile has no quota to report.
                        guard self.providers.contains(where: { $0.id == id }) else { self.providerQuotas[id] = nil; continue }
                        self.recordProblem(failure, source: "读取额度", activityKey: "quota:\(id)")
                    }
                }
            }
        }
    }
    func readProviderAPIKey(_ providerID: String, completion: @escaping (String) -> Void) {
        guard !isPreview, let transport else { recordProblem("预览不会读取 API key"); return }
        enqueue({ try transport.readProviderAPIKey(gatewayID: self.gateway.id, providerID: providerID) }, apply: completion)
    }
    func saveTemplate(_ value: ModelTemplate, onSaved: (() -> Void)? = nil) {
        if isPreview { var saved = value; if saved.id.isEmpty { saved.id = UUID().uuidString }; modelTemplates.removeAll { $0.id == saved.id }; modelTemplates.append(saved); onSaved?(); return }
        guard let transport else { recordProblem("本地核心未配置"); return }
        enqueue({ try transport.saveTemplate(value) }) { [weak self] result in self?.modelTemplates = result.map(BindingMapping.template); onSaved?() }
    }
    func saveTemplates(_ values: [ModelTemplate], onSaved: @escaping (Int) -> Void) {
        guard !values.isEmpty else { return }
        if isPreview { for value in values { saveTemplate(value) }; onSaved(values.count); return }
        guard let transport else { recordProblem("本地核心未配置"); return }
        enqueue({
            var templates: [BindingModelTemplate]?
            var count = 0
            var failures: [AppProblem] = []
            for value in values {
                do { templates = try transport.saveTemplate(value); count += 1 }
                catch { failures.append(AppProblem.failure(error, source: "添加模型模板 · \(value.name)")) }
            }
            return (templates, count, failures)
        }) { [weak self] result in
            if let templates = result.0 { self?.modelTemplates = templates.map(BindingMapping.template) }
            self?.appendProblems(result.2)
            onSaved(result.1)
        }
    }
    func fetchPublicModelCatalog(completion: @escaping ([CatalogModel]) -> Void) {
        guard !isPreview, let transport else { recordProblem("预览不会访问公开目录"); return }
        enqueue({ try transport.publicModelCatalog() }) { values in
            completion(values.map { CatalogModel(sourceProviderID: $0.sourceProviderId, sourceProviderName: $0.sourceProviderName, modelID: $0.modelId, name: $0.name, contextWindow: $0.contextWindow, maxOutputTokens: $0.maxOutputTokens, reasoningLevels: $0.reasoningLevels) })
        }
    }
    func deleteTemplate(_ id: String) {
        if isPreview { modelTemplates.removeAll { $0.id == id }; return }
        guard let transport else { recordProblem("本地核心未配置"); return }
        enqueue({ try transport.deleteTemplate(id: id) }) { [weak self] result in self?.modelTemplates = result.map(BindingMapping.template) }
    }
    func previewProviderImport(_ source: ProviderImportSource, completion: @escaping (ProviderImportPreview) -> Void) {
        guard !isPreview, !isGenerating, !authenticationRunning else { recordProblem("请先停止任务或完成登录，再读取提供商配置"); return }
        guard let transport else { recordProblem("本地核心未配置"); return }
        enqueue({ try transport.providerImportPreview(gatewayID: self.gateway.id, source: BindingMapping.bindingImportSource(source)) }) { completion(BindingMapping.importPreview($0).preview) }
    }
    func applyProviderImport(_ source: ProviderImportSource, preview: ProviderImportPreview, selections: [ProviderImportSelection], replaceExisting: Bool, completion: @escaping () -> Void) {
        guard !isPreview, !isGenerating, !authenticationRunning else { recordProblem("请先停止任务或完成登录，再导入提供商配置"); return }
        guard let transport else { recordProblem("本地核心未配置"); return }
        enqueue({ try transport.providerImportApply(gatewayID: self.gateway.id, source: BindingMapping.bindingImportSource(source), previewToken: preview.token, selections: selections.map(BindingMapping.bindingSelection), replaceExisting: replaceExisting) }) { [weak self] data in
                guard let self else { return }
                if let saved = data.gateways.first(where: { $0.id == self.gateway.id }) { self.gateway = BindingMapping.gateway(saved); self.hasGateway = true }
                if data.executionInvalidated { refreshAfterExecutionInvalidation() }
                if !data.executionInvalidated { enqueue({ try transport.list() }) { [weak self] in self?.applyList($0) } }
                completion()
            }
    }
    func startAuthentication(_ providerID: String) {
        if let sharedApplication { sharedApplication.startAuthentication(providerID); showsAuthentication = true; return }
        guard !isGenerating else { recordProblem("请先停止当前任务，再开始登录"); return }
        guard !isPreview else { recordProblem("预览不会启动认证"); return }
        guard let transport else { recordProblem("本地核心未配置"); return }
        enqueue({ try transport.authenticationStart(gatewayID: self.gateway.id, providerID: providerID) }) { [weak self] data in
                guard let self else { return }
                authenticationPrompt = nil; authenticationNotifications = []; authenticationResult = nil
                showsAuthentication = true; applyAuthentication(BindingMapping.authenticationProgress(data))
            }
    }
    func answerAuthentication(id: String, value: String) {
        if let sharedApplication { sharedApplication.answerAuthentication(id: id, value: value); return }
        guard let transport else { recordProblem("本地核心未配置"); return }
        enqueue({ try transport.authenticationReply(promptID: id, value: value) }) { [weak self] in self?.authenticationPrompt = nil; self?.applyAuthentication(BindingMapping.authenticationProgress($0)) }
    }
    func cancelAuthentication() {
        if let sharedApplication { sharedApplication.cancelAuthentication(); return }
        guard let transport else { recordProblem("本地核心未配置"); return }
        enqueue({ try transport.authenticationCancel() }) { [weak self] in self?.applyAuthentication(BindingMapping.authenticationProgress($0)) }
    }
    /// Signs the provider's source out, e.g. Claude Code CLI's own login.
    func logoutAuthentication(_ providerID: String) {
        guard !isPreview else { recordProblem("预览不会退出登录"); return }
        guard let transport else { recordProblem("本地核心未配置"); return }
        enqueue({ try transport.authenticationLogout(gatewayID: self.gateway.id, providerID: providerID) }) { [weak self] in
            self?.applyGatewayUpdate($0); self?.providerQuotas[providerID] = nil
        }
    }
    private func pollAuthentication() {
        guard authenticationRunning, !authenticationPollPending, !isShuttingDown, let transport else { return }
        authenticationPollPending = true
        queue.async { [weak self] in
            let result = Result { try transport.authenticationPoll() }
            DispatchQueue.main.async {
                guard let self else { return }
                self.authenticationPollPending = false
                switch result {
                case .success(let data): self.clearActivityProblem("authentication"); self.applyAuthentication(BindingMapping.authenticationProgress(data))
                case .failure(let failure): self.recordProblem(failure, source: "读取登录进度", activityKey: "authentication")
                }
            }
        }
    }
    private func applyAuthentication(_ data: AuthenticationData) {
        if !data.running { clearActivityProblem("authentication") }
        authenticationRunning = data.running
        if let saved = data.gateways?.first(where: { $0.id == gateway.id }) { gateway = saved }
        if data.executionInvalidated == true { refreshAfterExecutionInvalidation() }
        for event in data.events {
            if event.type == "prompt", let id = event.id, let prompt = event.prompt {
                authenticationPrompt = AuthenticationPrompt(id: id, kind: prompt.kind, text: prompt.text, options: prompt.options)
            } else if event.type == "notify", let notification = event.notification {
                if notification.kind == "prompt_cancelled" {
                    if authenticationPrompt?.id == notification.id { authenticationPrompt = nil }
                } else {
                    authenticationNotifications.append(notification)
                    // A sign-in page opens in the browser; the sheet keeps a link to open it again.
                    if notification.kind == "url", let address = notification.url, let url = URL(string: address), url.scheme == "https" { NSWorkspace.shared.open(url) }
                }
            } else if event.type == "result" {
                authenticationPrompt = nil
                authenticationResult = event.ok == true ? "登录完成。" : event.cancelled == true ? "登录已取消。" : nil
                if event.ok != true && event.cancelled != true { recordProblem(event.error ?? "登录未完成。", source: "登录") }
            }
        }
    }
    func saveModelRoute(_ value: ModelRoute, onSaved: (() -> Void)? = nil) {
        guard !authenticationRunning, !isGenerating else { recordProblem("请先停止当前任务或完成登录"); return }
        if isPreview { var saved = value; if saved.recordKey.isEmpty { saved.recordKey = UUID().uuidString }; gateway.routes.removeAll { $0.recordKey == saved.recordKey }; gateway.routes.append(saved); onSaved?(); return }
        guard let transport else { recordProblem("本地核心未配置"); return }
        enqueue({ try transport.saveModelRoute(gatewayID: self.gateway.id, route: value) }) { [weak self] data in
            self?.applyGatewayUpdate(data); onSaved?()
        }
    }
    func deleteModelRoute(_ recordKey: String) {
        if isPreview { gateway.routes.removeAll { $0.recordKey == recordKey }; return }
        guard let transport else { recordProblem("本地核心未配置"); return }
        enqueue({ try transport.deleteModelRoute(gatewayID: self.gateway.id, recordKey: recordKey) }) { [weak self] in self?.applyGatewayUpdate($0) }
    }
    func deleteProvider(id: String) {
        if isPreview { gateway.providers.removeAll { $0.id == id }; return }
        guard let transport else { recordProblem("本地核心未配置"); return }
        enqueue({ try transport.deleteProvider(gatewayID: self.gateway.id, providerID: id) }) { [weak self] in self?.applyGatewayUpdate($0) }
    }
    private func applyGatewayUpdate(_ data: BindingGatewayUpdate) {
        if let saved = data.gateways.first(where: { $0.id == gateway.id }) { gateway = BindingMapping.gateway(saved); hasGateway = true }
        reconcileNextTurnIntent()
        if data.executionInvalidated { refreshAfterExecutionInvalidation() }
    }
    func runtimeDiscoveryHints(userHome: String, overrides: [String: String], onFailure: (() -> Void)? = nil, completion: @escaping ([BindingRuntimeDiscoveryHint]) -> Void) {
        guard let transport else { recordProblem("本地核心未配置"); return }
        enqueue({ try transport.runtimeDiscoveryHints(userHome: userHome, overrides: overrides) }, onFailure: { _ in onFailure?() }, apply: completion)
    }
    func discoverRuntimes(_ probes: [BindingRuntimeDiscoveryProbe], onFailure: (() -> Void)? = nil, completion: @escaping ([BindingRuntimeDiscoveryCandidate]) -> Void) {
        guard let transport else { recordProblem("本地核心未配置"); return }
        enqueue({ try transport.discoverRuntimes(probes) }, onFailure: { _ in onFailure?() }, apply: completion)
    }
    func importRuntimes(_ candidates: [BindingRuntimeDiscoveryCandidate], completion: @escaping (Set<String>) -> Void) {
        guard !isBusy, !candidates.isEmpty, candidates.allSatisfy({ $0.supported && !$0.alreadyConfigured }) else { recordProblem("请仅选择可导入且尚未配置的运行时"); return }
        guard let transport else { recordProblem("本地核心未配置"); return }
        let gatewayID = gateway.id
        enqueue({
            var imported: Set<String> = []
            var failures: [AppProblem] = []
            for candidate in candidates {
                var runtime = BindingMapping.runtime(candidate.runtime); runtime.gatewayID = gatewayID
                do { _ = try transport.upsertRuntime(BindingMapping.bindingRuntime(runtime)); imported.insert(runtime.id) }
                catch { failures.append(AppProblem.failure(error, source: "导入运行时 · \(runtime.name)")) }
            }
            return (try transport.list(), imported, failures)
        }) { [weak self] result in
            self?.applyList(result.0)
            self?.appendProblems(result.2)
            completion(result.1)
        }
    }
    /// Saves an instance; with `execution`, its location and HOME are replaced
    /// too, otherwise the saved ones are kept.
    func saveRuntimeInstance(_ instance: RuntimeInstance, execution: BindingRuntimeExecution? = nil, launch: BindingAcpLaunch? = nil, onSaved: (() -> Void)? = nil) {
        guard !isGenerating else { recordProblem("请先停止当前任务，再修改运行时实例"); return }
        if isPreview { runtimeInstances.removeAll { $0.id == instance.id }; runtimeInstances.append(instance); if !instance.enabled && projectionRuntimeID == instance.id { resetProjection(); projectionRuntimeID = nil }; onSaved?(); return }
        guard let transport else { recordProblem("本地核心未配置"); return }
        enqueue({
            let runtime = BindingMapping.bindingRuntime(instance)
            if let execution, let launch { return try transport.upsertRuntime(runtime, execution: execution, launch: launch) }
            return try execution.map { try transport.upsertRuntime(runtime, execution: $0) } ?? transport.upsertRuntime(runtime)
        }) { [weak self] data in
            guard let self else { return }
            runtimeInstances = data.runtimeInstances.map(BindingMapping.runtime); runtimeTypes = data.runtimeTypes.map(BindingMapping.runtimeType)
            if data.executionInvalidated { refreshAfterExecutionInvalidation() }
            else { enqueue({ try transport.list() }) { [weak self] in self?.applyList($0) } }
            if data.runtimeInstances.contains(where: { $0.id == instance.id }) { onSaved?() }
            else { recordProblem("核心未返回保存后的运行时实例") }
        }
    }
    /// The saved location and HOME (and where that HOME is), plus the launch
    /// options when the instance's type takes them.
    func runtimeExecution(id: String, launch: Bool, onFailure: @escaping () -> Void, completion: @escaping (BindingRuntimeExecution, String?, BindingAcpLaunch?) -> Void) {
        guard let transport, !isLoading, !isShuttingDown else { onFailure(); return }
        enqueue({
            let (execution, home) = try transport.runtimeExecution(id: id)
            return (execution, home, launch ? try transport.runtimeLaunch(id: id) : nil)
        }, onFailure: { _ in onFailure() }) { completion($0.0, $0.1, $0.2) }
    }
    /// Starts the saved instance's Agent (unless a session of it is live) to
    /// read what it reports; this does not send anything to a model.
    func agentInfo(runtimeID: String, completion: @escaping (Result<BindingAcpRuntimeInfo, Error>) -> Void) {
        guard let transport, !isShuttingDown else { return }
        queue.async {
            let result = Result { try transport.agentInfo(runtimeID: runtimeID) }
            DispatchQueue.main.async { completion(result) }
        }
    }
    /// The option the composer's model menu already offers: the session's
    /// first model-category option, which the runtime's catalog is read from.
    var editableSessionOptions: [SessionOption] {
        guard var options = sessionOptions else { return [] }
        if let model = options.firstIndex(where: { $0.category == "model" }) { options.remove(at: model) }
        return options
    }
    var canChangeSessionOptions: Bool { !isGenerating && !executionStarting && !isLoading && !isShuttingDown }
    func setSessionOption(_ option: SessionOption, value: SessionOptionValue) {
        guard canChangeSessionOptions, value != option.value, let runtimeID = snapshot?.conversation.runtimeID, let conversationID = loadedConversationID, let transport else { return }
        enqueue({ try transport.setSessionOption(runtimeID: runtimeID, conversationID: conversationID, optionID: option.id, value: BindingMapping.bindingSessionOptionValue(value)) }) { [weak self] options in
            guard let self, self.loadedConversationID == conversationID else { return }
            self.sessionOptions = options.map(BindingMapping.sessionOption)
        }
    }
    /// Options exist only while the conversation's session is live here, so
    /// they are read again when the conversation or its run state changes;
    /// not being live yet is not a problem to report.
    private func refreshSessionOptions() {
        guard let value = snapshot, !isDraft, !isPreview, let transport,
              let typeID = runtimeInstances.first(where: { $0.id == value.conversation.runtimeID })?.typeID,
              runtimeTypes.first(where: { $0.id == typeID })?.sessionOptions == true else {
            if sessionOptions != nil { sessionOptions = nil }; sessionOptionsKey = nil; return
        }
        let key = "\(value.conversation.id)|\(value.runState)"
        guard key != sessionOptionsKey else { return }
        if sessionOptionsKey?.hasPrefix(value.conversation.id + "|") != true { sessionOptions = nil }
        sessionOptionsKey = key
        let runtimeID = value.conversation.runtimeID, conversationID = value.conversation.id
        queue.async { [weak self] in
            let options = (try? transport.sessionOptions(runtimeID: runtimeID, conversationID: conversationID))?.map(BindingMapping.sessionOption)
            DispatchQueue.main.async {
                guard let self, self.sessionOptionsKey == key else { return }
                let becameLive = self.sessionOptions == nil && options != nil
                self.sessionOptions = options
                // The runtime's own models come from this session.
                if becameLive, case .ready = self.modelCatalogs[runtimeID] { self.modelCatalogs[runtimeID] = nil; self.loadModelCatalog() }
            }
        }
    }
    func deleteRuntimeInstance(id: String) {
        if isPreview {
            runtimeInstances.removeAll { $0.id == id }
            if projectionRuntimeID == id { resetProjection(); projectionRuntimeID = nil }
            return
        }
        guard let transport else { recordProblem("本地核心未配置"); return }
        enqueue({ try transport.deleteRuntime(id: id) }) { [weak self] data in
            self?.runtimeInstances = data.runtimeInstances.map(BindingMapping.runtime); self?.runtimeTypes = data.runtimeTypes.map(BindingMapping.runtimeType)
            if data.executionInvalidated { self?.refreshAfterExecutionInvalidation() }
        }
    }
    func selectModel(_ selection: ModelSelection) {
        guard canSwitchModel, let runtimeID = nextTurnRuntimeID, modelChoices.contains(where: { $0.selection == selection }) else { return }
        nextTurnModel = selection
        lastChosenModels[runtimeID] = selection
        dropUnofferedEffort()
        rememberNextTurnIntent()
    }
    /// Keeps what the user chose for the next turn's runtime in this conversation.
    private func rememberNextTurnIntent() {
        guard let intentConversationID, let runtimeID = nextTurnRuntimeID else { return }
        nextTurnIntents[intentConversationID, default: NextTurnIntent()].choices[runtimeID] = RuntimeChoice(model: nextTurnModel, effort: nextTurnEffort, permissionMode: nextTurnPermissionMode)
    }
    func selectEffort(_ effort: String?) {
        guard effort == nil || reasoningEffortOptions.contains(effort!) else { return }
        nextTurnEffort = effort
        rememberNextTurnIntent()
    }
    func location(of runtimeID: String?) -> RuntimeLocation { runtimeID.flatMap { runtimeLocations[$0] } ?? .local }
    /// Where the open conversation's files are: its runtime's machine.
    var loadedConversationLocation: RuntimeLocation { location(of: snapshot?.conversation.runtimeID) }
    /// Projects of the latest conversations on the machine the next turn
    /// runs on, newest first.
    var recentWorkingDirectories: [String] {
        let location = location(of: nextTurnRuntimeID)
        var seen = Set<String>()
        return conversations.filter { self.location(of: $0.runtimeID) == location }
            .sorted { ($0.updatedAtUnixMs ?? 0) > ($1.updatedAtUnixMs ?? 0) }
            .compactMap(\.project).filter { seen.insert($0).inserted }.prefix(5).map { $0 }
    }
    private func refreshRuntimeLocations() {
        let ids = runtimeInstances.map(\.id)
        guard let transport, !isPreview, !isShuttingDown else { return }
        let request = UUID()
        runtimeLocationsRequest = request
        queue.async { [weak self] in
            let result = Result { try transport.runtimeLocations(ids: ids) }
            DispatchQueue.main.async {
                guard let self, self.runtimeLocationsRequest == request, case .success(let transports) = result else { return }
                let locations = transports.mapValues(BindingMapping.location)
                if self.runtimeLocations != locations { self.runtimeLocations = locations }
            }
        }
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
        guard snapshot != nil, let runtimeID = nextTurnRuntimeID, enabledRuntimeInstances.contains(where: { $0.id == runtimeID }), modelCatalogs[runtimeID] == nil else { reconcileModelSelection(); return }
        if isPreview {
            let managed = gateway.providers.flatMap { provider in provider.models.map { ModelChoice(selection: .managed(recordKey: $0.recordKey), displayName: "\($0.displayName) · \(provider.name)", reasoningEfforts: $0.reasoningLevels ?? []) } }
            modelCatalogs[runtimeID] = .ready([ModelChoice(selection: .native(providerID: "sample", modelID: "sample-native"), displayName: "运行时示例模型", reasoningEfforts: ["low", "medium", "high"])] + managed)
            reconcileModelSelection(); return
        }
        guard let transport, !isShuttingDown else { return }
        let request = UUID()
        modelCatalogRequests[runtimeID] = request
        modelCatalogs[runtimeID] = .loading
        queue.async { [weak self] in
            let result = Result { try transport.runtimeModels(runtimeID: runtimeID) }
            DispatchQueue.main.async {
                guard let self, self.modelCatalogRequests[runtimeID] == request else { return }
                self.modelCatalogRequests[runtimeID] = nil
                switch result {
                case .success(let models): self.modelCatalogs[runtimeID] = .ready(models.map(BindingMapping.modelChoice))
                case .failure(let error): self.modelCatalogs[runtimeID] = .failed(AppProblem.failure(error, source: "模型目录").detail)
                }
                self.reconcileModelSelection()
            }
        }
    }
    /// Without a choice the model follows the conversation's latest turn on
    /// this runtime, then the conversation's recorded model, then the model
    /// last chosen on this runtime. Once the catalog is read, an unavailable
    /// one falls back to the catalog's first entry: the runtime's default, its
    /// own models, then managed models. An unread catalog leaves the choice
    /// for the core to check at sending. Only a user's choice is remembered.
    private func reconcileModelSelection() {
        guard let snapshot, let runtimeID = nextTurnRuntimeID else { return }
        let available = modelChoices.map(\.selection)
        let recorded = snapshot.conversation.runtimeID == runtimeID ? snapshot.modelRecordKey.map { ModelSelection.managed(recordKey: $0) } : nil
        let candidates = [nextTurnModel, latestTurnModel(on: runtimeID), recorded, lastChosenModels[runtimeID]].compactMap { $0 }
        let chosen: ModelSelection?
        if case .ready = modelCatalog {
            chosen = candidates.first { available.contains($0) } ?? available.first
        } else {
            let failed = if case .failed = modelCatalog { true } else { false }
            chosen = candidates.first ?? (failed ? available.first : nil)
        }
        if chosen != nextTurnModel { nextTurnModel = chosen }
        dropUnofferedEffort()
    }
    /// Levels are known only from a read catalog; until then the remembered
    /// level waits and is not sent.
    private func dropUnofferedEffort() {
        guard case .ready = modelCatalog, let effort = nextTurnEffort, !reasoningEffortOptions.contains(effort) else { return }
        nextTurnEffort = nil
    }
    /// A conversation comes back on its current runtime (one not sent yet on
    /// the runtime chosen for it), with that runtime's choices in it; see
    /// `reconcileModelSelection` for the model.
    private func restoreNextTurnIntent(for value: ConversationSnapshot) {
        guard value.conversation.id != intentConversationID else { return }
        intentConversationID = value.conversation.id
        let kept = isDraft ? nextTurnIntents[value.conversation.id]?.runtimeID : nil
        let runtimeID = [kept, value.contextRuntimeID].compactMap { $0 }.first { id in enabledRuntimeInstances.contains { $0.id == id } }
        if runtimeID != nil { hasInitializedNextTurnIntent = true }
        if let id = runtimeID ?? nextTurnRuntimeID { switchNextTurnRuntime(to: id) } else { nextTurnModel = nil; nextTurnEffort = nil; nextTurnPermissionMode = nil }
        loadModelCatalog()
    }
    private func refreshAfterExecutionInvalidation() {
        guard let transport else { recordProblem("本地核心未配置"); return }
        enqueue({ try transport.list() }) { [weak self] data in
            guard let self else { return }; applyList(data)
            if let runtimeID = snapshot?.conversation.runtimeID, let conversationID = loadedConversationID {
                enqueue({ try transport.openConversation(runtimeID: runtimeID, conversationID: conversationID) }) { [weak self] in self?.applySnapshotResult($0) }
            } else { resetProjection() }
        }
    }
    private func resetProjection() { intentConversationID = nil; loadedPayloads.removeAll(); isDraft = false; problems.removeAll { $0.activityKey?.hasPrefix("poll:") == true }; generation += 1; snapshot = nil; projectionRuntimeID = nil; selectedConversationID = nil; pendingConversationID = nil; transcript.reset(); endPendingSend(); activity = nil; sessionOptions = nil; sessionOptionsKey = nil }
    private func applyList(_ data: BindingConfigurationSnapshot) {
        // The application owns the conversation list and configuration that
        // workspaces copy; a workspace's result must reach it, or the stale
        // copy returns (a deleted conversation reappearing in the sidebar).
        sharedApplication?.applyList(data)
        let mapped = BindingMapping.configuration(data)
        savedBrowserPreferences = data.conversationBrowserPreferences
        if browserPreferencesSaving == 0, conversationBrowserPreferences != data.conversationBrowserPreferences {
            conversationBrowserPreferences = data.conversationBrowserPreferences
        }
        outlinePresentation = data.outlinePresentation
        statusLine = data.statusLine
        conversationBrowserLimits = data.conversationBrowserLimits
        conversations = mapped.conversations
        conversationGroups = data.conversationGroups; conversationProjects = data.conversationProjects; showsArchived = data.showsArchived
        // Organization changes the list's own rows; reached ones follow them.
        let listed = Dictionary(conversations.map { ($0.id, $0) }, uniquingKeysWith: { first, _ in first })
        offListConversations = offListConversations.map { listed[$0.id] ?? $0 }
        if searchResults != nil { refreshSearch() }
        selectedConversationIDs = selectedConversationIDs.filter { conversation(id: $0) != nil }
        noteSeen()
        if let saved = mapped.gateways.first(where: { $0.id == gateway.id }) ?? mapped.gateways.first { gateway = saved; hasGateway = true }
        runtimeInstances = mapped.runtimes; runtimeTypes = mapped.runtimeTypes; reconcileHistoryProblems(mapped.historyFailures)
        protocols = mapped.protocols
        modelTemplates = mapped.modelTemplates
        providerImportTypes = mapped.importTypes
        if !hasInitializedNextTurnIntent { nextTurnRuntimeID = mapped.selectedRuntimeID }
        reconcileNextTurnIntent()
    }
    func recordProblem(_ error: Error, source: String = "操作", activityKey: String? = nil) {
        if let sharedApplication { sharedApplication.recordProblem(error, source: source, activityKey: activityKey); return }
        let value = AppProblem.failure(error, source: source, activityKey: activityKey)
        if let activityKey, problems.contains(where: { $0.activityKey == activityKey && $0.detail == value.detail && $0.code == value.code && $0.phase == value.phase }) { return }
        if let activityKey { problems.removeAll { $0.activityKey == activityKey } }
        appendProblems([value])
    }
    func recordProblem(_ message: String, source: String = "操作") { recordProblem(TransportError.rejected(message), source: source) }
    private func appendProblems(_ values: [AppProblem]) {
        if let sharedApplication { sharedApplication.appendProblems(values); return }
        problems.append(contentsOf: values)
        if problems.count > 100 { problems.removeFirst(problems.count - 100) }
    }
    private func clearActivityProblem(_ key: String) { if let sharedApplication { sharedApplication.clearActivityProblem(key); return }; guard problems.contains(where: { $0.activityKey == key }) else { return }; problems.removeAll { $0.activityKey == key } }
    func clearProblem(_ id: UUID) { if let sharedApplication { sharedApplication.clearProblem(id); return }; problems.removeAll { $0.id == id } }
    func clearProblems() { if let sharedApplication { sharedApplication.clearProblems(); return }; problems.removeAll() }
    private func reconcileHistoryProblems(_ failures: [HistoryFailure]) {
        guard sharedApplication == nil else { return }
        let keys = Set(failures.map { "history:" + $0.runtimeID })
        problems.removeAll { $0.activityKey?.hasPrefix("history:") == true && !keys.contains($0.activityKey ?? "") }
        for failure in failures {
            let key = "history:" + failure.runtimeID
            if problems.contains(where: { $0.activityKey == key && $0.detail == failure.detail }) { continue }
            problems.removeAll { $0.activityKey == key }
            appendProblems([AppProblem(id: UUID(), occurredAt: Date(), source: "会话列表 · " + (runtimeInstances.first { $0.id == failure.runtimeID }?.name ?? failure.runtimeID), detail: failure.detail, kind: "history", code: nil, phase: nil, operationID: nil, activityKey: key)])
        }
    }
    private func reconcileNextTurnIntent() {
        if !hasInitializedNextTurnIntent {
            nextTurnRuntimeID = enabledRuntimeInstances.first(where: { $0.id == nextTurnRuntimeID })?.id ?? enabledRuntimeInstances.first?.id
            // A workspace can see the configuration before it has loaded; it chooses once there is a runtime.
            hasInitializedNextTurnIntent = nextTurnRuntimeID != nil
        } else if let nextTurnRuntimeID, !enabledRuntimeInstances.contains(where: { $0.id == nextTurnRuntimeID }) {
            self.nextTurnRuntimeID = nil; nextTurnModel = nil
        }
        reconcileModelSelection()
    }
    private func apply(_ value: ConversationSnapshot?, preserveSelection: Bool = false) {
        guard var value else { return }
        if value.conversation.id != snapshot?.conversation.id || value.contextRuntimeID != snapshot?.contextRuntimeID { loadedPayloads.removeAll() }
        value.messages = withLoadedPayloads(value.messages)
        let remainingProblems = problems.filter { !($0.activityKey?.hasPrefix("poll:") == true && $0.activityKey != "poll:" + value.contextRuntimeID) }
        if problems != remainingProblems { problems = remainingProblems }
        updatePendingSend(value)
        guard snapshot != value else { return }
        snapshot = value; projectionRuntimeID = value.contextRuntimeID; selectedConversationID = value.conversation.id; restoreNextTurnIntent(for: value); if !preserveSelection && selectedConversationIDs.count <= 1 { selectedConversationIDs = [value.conversation.id] }; transcript.apply(value.messages, turns: value.transcriptTurns, items: value.transcriptItems, outline: value.transcriptOutline, identities: value.transcriptMessageIdentities)
        if let index = conversations.firstIndex(where: { $0.id == value.conversation.id }) {
            // The snapshot carries the runtime's fields; the list keeps its organization.
            let updated = value.conversation.organized(like: conversations[index])
            if conversations[index] != updated { conversations[index] = updated }
        }
        else if !isDraft { conversations.insert(value.conversation, at: 0) }
        activity = switch value.runState {
        // A turn that a Harness process outside this execution runs reads the same.
        case .running, .elsewhere: TranscriptActivity(text: Self.currentWork(value.messages.last), isLive: true)
        case .stopping: TranscriptActivity(text: "正在停止", isLive: false)
        case .idle, .failed: nil
        }
        if isPreview { previewSnapshots[value.conversation.id] = value }
        refreshSessionOptions()
    }
    /// What a running turn is doing, from the newest block of its last
    /// message: a reasoning summary, the running tool, or replying.
    private static func currentWork(_ message: Message?) -> String {
        guard let message, message.role == .assistant, let block = message.blocks.last else { return "正在思考" }
        switch block {
        case .reasoning(let summary, _): return summary
        case .tool(_, let title, .running, _, _), .tool(_, let title, .pending, _, _): return title
        case .text: return "正在回复"
        case .tool, .notice, .image, .file, .agentSender: return "正在思考"
        }
    }
    /// Preparation and operation results replace the projection; polling asks
    /// the core only for changes after the current cursor (see `poll`).
    private func applySnapshotResult(_ value: BindingSnapshotResult) {
        guard let incoming = value.snapshot else { resetProjection(); return }
        apply(BindingMapping.snapshot(incoming))
    }
    private enum PolledSnapshot: Sendable {
        case absent, unchanged
        case complete(ConversationSnapshot)
        /// Changed messages only; merged on the main actor into the snapshot at `base`.
        case changes(base: TranscriptCursor, header: ConversationSnapshot, order: [String], changed: [Message])
    }
    private func poll() {
        guard started, !isDraft, !isPreview, !isShuttingDown, sharedApplication?.isShuttingDown != true, snapshot != nil, !isLoading, !pollPending, let transport, let runtimeID = projectionRuntimeID else { return }
        let revision = generation
        let selected = selectedConversationID
        let since = snapshot?.cursor
        pollPending = true
        queue.async { [weak self] in
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
                    return .changes(base: BindingMapping.cursor(base), header: BindingMapping.snapshotHeader(incoming), order: incoming.messageIds, changed: incoming.messages.map(BindingMapping.message))
                }
            }
            DispatchQueue.main.async {
                guard let self else { return }
                self.pollPending = false
                guard self.generation == revision, self.selectedConversationID == selected else { return }
                switch result {
                case .success(let polled):
                    self.clearActivityProblem("poll:" + runtimeID)
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
        queue.async { [weak self] in
            let result = Result { try transport.messagePayloads(runtimeID: runtimeID, conversationID: conversationID, messageIDs: Array(wanted)).map(BindingMapping.message) }
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
    private func withLoadedPayloads(_ messages: [Message]) -> [Message] {
        guard !loadedPayloads.isEmpty else { return messages }
        var retained: [String: Message] = [:]
        let merged = messages.map { message -> Message in
            guard let loaded = loadedPayloads[message.id], loaded.stamp == message.stamp else { return message }
            retained[message.id] = loaded
            return loaded
        }
        loadedPayloads = retained
        return merged
    }
    private func enqueue<T: Sendable>(_ operation: @escaping () throws -> T, onAccepted: (() -> Void)? = nil, onFailure: ((Error) -> Void)? = nil, apply: @escaping (T) -> Void) {
        guard !isLoading, !isShuttingDown else { recordProblem("请等待当前操作完成后重试"); return }
        guard transport != nil else { recordProblem("本地核心未配置"); return }
        let revision = generation
        isLoading = true
        queue.async { [weak self] in
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
    private func seedPreview() {
        conversationBrowserPreferences = ConversationBrowser().preferences
        gateway = GatewayConfig(providers: [AIProvider(id: "sample-provider", name: "示例 AI 服务", protocolID: .chatCompletionsV1, endpoint: "https://example.invalid/v1", models: [ProviderModel(recordKey: "sample-model", providerModelID: "external-example", nickname: "通用模型", contextWindow: 8192, maxOutputTokens: 4096)])])
        hasGateway = true
        protocols = [ProtocolDescriptor(id: .chatCompletionsV1, name: "OpenAI Chat Completions v1", supported: true), ProtocolDescriptor(id: .responsesV1, name: "OpenAI Responses v1", supported: true), ProtocolDescriptor(id: .messagesV1, name: "Anthropic Messages v1", supported: true)]
        runtimeTypes = [RuntimeTypeDescriptor(id: "sample-type", familyID: "sample", canRenameConversations: true, canDeleteConversations: true, supportedProtocols: [.chatCompletionsV1, .responsesV1, .messagesV1], supportedProviderProtocols: [.chatCompletionsV1, .responsesV1, .messagesV1], name: "示例运行时", fields: [])]
        runtimeInstances = [RuntimeInstance(id: "sample-instance", name: "示例运行时", typeID: "sample-type", gatewayID: gateway.id, settings: [:]), RuntimeInstance(id: "sample-review", name: "另一个运行时", typeID: "sample-type", gatewayID: gateway.id, settings: [:])]
        projectionRuntimeID = runtimeInstances[0].id; hasInitializedNextTurnIntent = true; nextTurnRuntimeID = projectionRuntimeID; nextTurnModel = .managed(recordKey: "sample-model")
        let topics = ["让设置页更安静", "整理一段代码", "下一步的项目计划"]
        let pairs = [
            ["我想让设置页面更简单，先帮我梳理一下。", "# 一次清晰的工作循环\n\n先整理任务，再执行。\n\n- [x] 读取上下文\n- [ ] 完成修改\n\n> 每一步都保留可以复核的结果。\n\n| 内容 | 状态 |\n| --- | --- |\n| 会话 | 可继续 |\n| 工具 | 已完成 |\n\n正文支持**强调**、[链接](https://example.invalid)和 `inline code`。", "运行时实例应该放哪里？", "工具栏显示当前运行时实例，详细配置放进系统设置窗口里。会话页面保留对话与模型选择，让注意力留在正在做的工作上。"],
            ["这个读取配置的函数可以再清晰一点吗？", "先把输入校验放在函数边界，主体就能直接表达意图。\n\n```rust\nfn load(path: &Path) -> Result<Config> {\n    let source = fs::read_to_string(path)?;\n    let config: Config = toml::from_str(&source)?;\n    config.validate()?;\n    Ok(config)\n}\n```", "还需要抽一个新的层吗？", "目前不需要。等第二个调用方出现不同的读取需求，再决定是否有值得共享的边界。"],
            ["帮我把接下来的工作排一下。", "先完成可用的会话界面，再检查配置是否真正驱动运行时。最后用一个短任务验证发送、停止和会话切换。", "先从哪个环节开始？", "从会话开始：打开旧会话、发送消息、检查回复，再新建一个会话。这个路径会暴露最直接的体验问题。"]
        ]
        for index in topics.indices {
            let conversation = Conversation(id: "sample-\(index)", title: topics[index], updatedAtUnixMs: Int64(Date().timeIntervalSince1970 * 1000) - Int64(index * 86_400_000), createdAtUnixMs: Int64(Date().timeIntervalSince1970 * 1000) - Int64((index + 3) * 86_400_000), runtimeID: runtimeInstances[index == 2 ? 1 : 0].id, cwd: index == 0 ? "/Users/example/Projects/Velune" : index == 1 ? "/Users/example/Projects/Sample" : nil)
            var history = pairs[index].enumerated().map { Message(id: "sample-\(index)-\($0.offset)", role: $0.offset.isMultiple(of: 2) ? .user : .assistant, blocks: [.text($0.element)]) }
            history[1].blocks.insert(.reasoning(summary: "先确认上下文", text: .inline("再选择最小的修改范围。")), at: 0)
            history[1].blocks.append(.tool(id: "sample-tool-\(index)", title: index == 1 ? "检查项目代码" : "读取工作上下文", state: .completed, output: .inline(index == 1 ? "已读取 3 个文件，未修改项目。" : "已梳理当前任务的上下文。"), conversation: nil))
            // The last sample is mid-turn: summaries with and without text yet, then a running tool.
            let running = index == 2
            if running { history[history.count - 1].blocks = [.reasoning(summary: "Reading the settings view", text: .inline("The sidebar and the detail pane share one split view.")), .reasoning(summary: "Inspecting the project layout", text: .inline("")), .tool(id: "sample-progress", title: "整理项目计划", state: .running, output: .inline("正在整理计划。"), conversation: nil)] }
            previewSnapshots[conversation.id] = ConversationSnapshot(revision: 1, conversation: conversation, contextRuntimeID: conversation.runtimeID, modelRecordKey: "sample-model", runState: running ? .running : .idle, messages: history,
                transcriptTurns: [
                    TranscriptTurn(id: "preview-work-" + conversation.id, userMessageID: history[0].id, workMessageIDs: [history[1].id], lastMessageID: nil, durationMs: nil, isRunning: false),
                    TranscriptTurn(id: "preview-answer-" + conversation.id, userMessageID: history[2].id, workMessageIDs: running ? [history[3].id] : [], lastMessageID: running ? nil : history[3].id, durationMs: nil, isRunning: running)
                ],
                transcriptItems: [.message(id: history[0].id, messageID: history[0].id), .work(turnID: "preview-work-" + conversation.id), .message(id: history[2].id, messageID: history[2].id)]
                    + (running ? [.work(turnID: "preview-answer-" + conversation.id)] : [.message(id: history[3].id, messageID: history[3].id)]) + history.dropFirst(4).map { .message(id: $0.id, messageID: $0.id) },
                transcriptOutline: history.filter { $0.role == .user }.map { TranscriptOutlineEntry(id: $0.id, messageID: $0.id) },
                transcriptMessageIdentities: history.map { TranscriptMessageIdentity(id: $0.id, messageID: $0.id) },
                actions: ConversationActions(canSend: !running, canCancel: running, canSwitch: true))
            conversations.append(conversation)
        }
        apply(previewSnapshots["sample-0"])
    }
}
