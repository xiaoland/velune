import Combine
import Foundation
import AppKit
import VeluneBindings

/// What a conversation chose for its next turn: each runtime keeps its own
/// choice. The runtime itself is kept only for a conversation not yet sent;
/// a sent one comes back on its current runtime.
struct ConversationChoices: Equatable {
    var runtimeID: String?
    var choices: [String: TurnChoice] = [:]
}

/// One device's application state, shared by its windows: configuration,
/// the conversation list, login, quotas, analytics, problems, drafts, the
/// composer's kept choices, and which conversations run. Each conversation
/// tab, and each window's list, is a `ConversationStore` over it.
@MainActor
final class DeviceStore: ObservableObject {
    @Published private(set) var conversationBrowserPreferences: ConversationBrowserPreferences?
    private var savedBrowserPreferences: ConversationBrowserPreferences?
    private var browserPreferencesGeneration: UInt64 = 0
    /// Browser choices being saved; until they are, lists read meanwhile do
    /// not replace the list, which would show the choices before them.
    private var browserPreferencesSaving = 0
    @Published private(set) var outlinePresentation: OutlinePresentation = .popover
    /// Where a conversation linked from another opens.
    @Published private(set) var linkedConversationOpening: LinkedConversationOpening = .currentTab
    /// Whether quota and context usage read as used or remaining.
    @Published private(set) var usageDisplay: UsageDisplay = .used
    @Published private(set) var contextUsageDisplay: UsageDisplay = .used
    /// What the composer's status line shows, in display order.
    @Published private(set) var statusLine: [StatusLineItem] = [.contextUsage]
    @Published private(set) var conversationBrowserLimits = ConversationBrowserLimits(page: 20, total: 200)
    /// The conversations the list shows, in display order.
    @Published private(set) var conversations: [Conversation] = []
    /// The list's groups in display order, as the core windows them.
    @Published private(set) var conversationGroups: [ConversationGroup] = []
    @Published private(set) var conversationProjects: [ConversationProject] = []
    @Published private(set) var showsArchived = false
    /// The runtime instance the core last chose, where a new tab's next turn starts.
    private(set) var selectedRuntimeInstanceID: String?
    /// Counts list reads, for tabs to follow what changed with them.
    private(set) var listRevision = 0
    /// Conversations whose read mark is being sent. The mark can leave a
    /// conversation unread when it changed meanwhile, so this follows the
    /// request, not the list.
    private var markingRead: Set<String> = []
    /// Deleted from any tab; a tab showing one closes it.
    @Published private(set) var deletedConversationIDs: Set<String> = []
    /// Conversations executing on this device, across windows.
    @Published private(set) var runningConversationIDs: Set<String> = []
    /// Best-effort: conversations whose native records show a turn running outside Velune.
    @Published private(set) var elsewhereConversationIDs: Set<String> = []
    /// Conversations a native sub-agent of which runs, here or elsewhere.
    @Published private(set) var subAgentParentIDs: Set<String> = []
    /// Conversations with a request waiting for the user, such as an approval.
    @Published private(set) var waitingConversationIDs: Set<String> = []
    /// Conversations running elsewhere that the list did not contain when last read again.
    private var unlistedElsewhereIDs: Set<String> = []
    @Published private(set) var gateway = GatewaySummary(id: "default", name: "默认网关", providers: [], routes: []) { didSet { if oldValue != gateway { configurationRevision &+= 1 } } }
    @Published private(set) var runtimeInstances: [RuntimeInstance] = [] {
        didSet {
            if oldValue != runtimeInstances {
                configurationRevision &+= 1
                refreshRuntimeLocations()
            }
        }
    }
    /// Counts changes to providers and runtime instances; model catalogs and
    /// quota read before one are stale.
    @Published private(set) var configurationRevision: UInt64 = 0
    /// Where each instance runs, read with its saved execution; unknown ones count as local.
    @Published private(set) var runtimeLocations: [String: RuntimeTransport] = [:]
    private var runtimeLocationsRequest = UUID()
    @Published private(set) var runtimeTypes: [RuntimeTypeDescriptor] = []
    @Published private(set) var protocols: [ProtocolDescriptor] = []
    @Published private(set) var modelTemplates: [ModelTemplate] = []
    @Published private(set) var providerImportTypes: [RuntimeTypeDescriptor] = []
    @Published private(set) var authenticationRunning = false
    @Published private(set) var authenticationPrompt: PendingAuthenticationPrompt?
    @Published private(set) var authenticationNotifications: [AuthenticationNotification] = []
    @Published private(set) var authenticationResult: String?
    @Published var showsAuthentication = false
    /// The core's quota observations by provider ID, as last read.
    @Published private(set) var providerQuotas: [String: QuotaObservation] = [:]
    private var quotaReadsPending: Set<String> = []
    /// What a new provider starts from; a static catalog in the core.
    private(set) lazy var providerTemplates: [ProviderTemplate] = (try? VeluneBindings.providerTemplates()) ?? []
    @Published private(set) var isLoading = false
    @Published private(set) var isShuttingDown = false
    @Published private(set) var problems: [AppProblem] = []
    @Published private(set) var analyticsReport: AnalyticsReport?
    @Published private(set) var analyticsLoadedQuery: AnalyticsQuery?
    @Published private(set) var analyticsIsLoading = false
    @Published private(set) var analyticsReadFailed = false
    @Published private(set) var analyticsUpdatedAt: Date?
    private var analyticsGeneration: UInt64 = 0

    let transport: Transport?
    let isPreview: Bool
    /// The tool windows as last arranged, for windows without a layout of their own.
    private(set) lazy var rememberedToolLayout = RememberedToolLayout(home: transport?.stateDirectory) { [weak self] in
        self?.recordProblem($0, source: "保存布局")
    }
    private var subscriptions: Set<AnyCancellable> = []
    private let queue = DispatchQueue(label: "local.velune.device", qos: .userInitiated)
    private var timer: Timer?
    private var started = false
    private var authenticationPollPending = false
    private(set) var hasGateway = false
    /// Unsent composer text by conversation. Deliberately not published: the
    /// composer owns the text while editing and reads a draft when it switches.
    private var composerDrafts: [String: String] = [:]
    private var unsavedDrafts: [String: String] = [:]
    private var draftSave: DispatchWorkItem?
    /// Next-turn choices per conversation; the core keeps them across restarts.
    private var conversationChoices: [String: ConversationChoices] = [:]
    /// Each runtime instance's last choices and what its new conversations
    /// start with, by runtime ID.
    @Published private(set) var runtimeComposerChoices: [String: RuntimeComposerChoices] = [:]
    /// Counts reads of the kept choices, for tabs to take them up.
    @Published private(set) var composerChoicesRevision = 0
    private var activePollPending = false
    private var activityCursor: UInt64 = 0
    private var quotaAlertPollPending = false
    private var quotaAlertCursor: UInt64 = 0
    private var lastQuotaAlertPoll = Date.distantPast
    private var notifier: ConversationNotifier?
    /// The tab whose window was key most recently; notifications open
    /// conversations there.
    weak var focusedConversation: ConversationStore?
    /// Sample conversations shown instead of a core's.
    private(set) var previewSnapshots: [String: ConversationSnapshot] = [:]

    init(transport: Transport? = nil, preview: Bool = false) {
        isPreview = preview
        if preview { self.transport = nil }
        else if let transport { self.transport = transport }
        else {
            do { self.transport = try Transport.applicationDefault() }
            catch { self.transport = nil; self.recordProblem(error, source: "初始化") }
        }
        if preview { seedPreview() }
        transport?.configurationChanged.receive(on: DispatchQueue.main).sink { [weak self] in
            guard let self, self.started else { return }
            self.refreshConfiguration()
        }.store(in: &subscriptions)
    }

    var enabledRuntimeInstances: [RuntimeInstance] { runtimeInstances.filter(\.enabled) }
    var providers: [ProviderSummary] { gateway.providers }
    /// Turns or sub-agents run in Velune's runtime processes; quitting ends them.
    var hasRunningWork: Bool { !runningConversationIDs.isEmpty }
    var isBusy: Bool { isLoading || hasRunningWork || isShuttingDown || authenticationRunning }
    /// A conversation runs here or elsewhere, by identity, listed or not.
    func isConversationRunning(_ id: String) -> Bool { runningConversationIDs.contains(id) || elsewhereConversationIDs.contains(id) }
    func location(of runtimeID: String?) -> RuntimeTransport { runtimeID.flatMap { runtimeLocations[$0] } ?? .local }
    /// Velune's models by record key, named as the composer names them.
    var veluneModelNames: [(key: String, name: String)] {
        gateway.providers.flatMap { provider in provider.models.map { (key: $0.recordKey, name: "\($0.displayName) · \(provider.name)") } }
    }

    /// Begin reading the list and following which conversations run.
    func start() {
        guard !started else { return }; started = true
        guard !isPreview, let transport else { return }
        notifier = ConversationNotifier(application: self)
        loadDrafts(transport)
        loadComposerChoices(transport)
        refreshConfiguration()
        schedulePolling()
    }

    private func refreshConfiguration() {
        guard let transport, !isShuttingDown else { return }
        performOnQueue { [weak self] in
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

    func refreshConversations() {
        guard !isPreview, !isLoading, !isShuttingDown, let transport else { return }
        enqueue({ try transport.list() }) { [weak self] in self?.applyList($0) }
    }

    func applyList(_ data: ConfigurationSnapshot) {
        savedBrowserPreferences = data.conversationBrowserPreferences
        if browserPreferencesSaving == 0, conversationBrowserPreferences != data.conversationBrowserPreferences {
            conversationBrowserPreferences = data.conversationBrowserPreferences
        }
        outlinePresentation = data.outlinePresentation
        linkedConversationOpening = data.linkedConversationOpening
        usageDisplay = data.usageDisplay
        contextUsageDisplay = data.contextUsageDisplay
        statusLine = data.statusLine
        conversationBrowserLimits = data.conversationBrowserLimits
        // While browser choices are being saved, the list waits for their
        // result: one read before would show groups folded or unfolded as they were.
        if browserPreferencesSaving == 0 {
            conversations = data.conversations.map(BindingMapping.listed)
            conversationGroups = data.conversationGroups; conversationProjects = data.conversationProjects
        }
        showsArchived = data.showsArchived
        if let saved = data.gateways.first(where: { $0.id == gateway.id }) ?? data.gateways.first { gateway = saved; hasGateway = true }
        runtimeInstances = data.runtimeInstances; runtimeTypes = data.runtimeTypes; reconcileHistoryProblems(data.historyFailures)
        protocols = data.protocols
        modelTemplates = data.modelTemplates
        providerImportTypes = data.providerImportTypes
        selectedRuntimeInstanceID = data.selectedRuntimeInstanceId
        listRevision += 1
    }

    // MARK: - List

    func setConversationBrowserPreferences(_ preferences: ConversationBrowserPreferences) {
        guard preferences != conversationBrowserPreferences, !isShuttingDown else { return }
        if isPreview { conversationBrowserPreferences = preferences; return }
        guard let transport else { recordProblem("本地核心未配置"); return }
        browserPreferencesGeneration &+= 1
        let revision = browserPreferencesGeneration
        conversationBrowserPreferences = preferences
        browserPreferencesSaving += 1
        // Sidebar choices are independent of loading/execution. Serialize their
        // commits with core operations without locking ordinary browsing.
        performOnQueue { [weak self] in
            let result = Result { try transport.setConversationBrowserPreferences(preferences) }
            DispatchQueue.main.async {
                guard let self else { return }
                self.browserPreferencesSaving -= 1
                // A later choice still being saved brings its own list; this
                // one's would show the groups folded or unfolded as before it.
                let latest = revision == self.browserPreferencesGeneration
                switch result {
                case .success(let data):
                    self.savedBrowserPreferences = data.conversationBrowserPreferences
                    guard latest else { return }
                    self.conversationBrowserPreferences = data.conversationBrowserPreferences
                    self.applyList(data)
                case .failure(let failure):
                    self.recordProblem(failure)
                    guard latest else { return }
                    self.conversationBrowserPreferences = self.savedBrowserPreferences
                    // An earlier choice may have been saved, its list left out.
                    self.refreshConversations()
                }
            }
        }
    }
    func setConversationBrowserLimits(_ limits: ConversationBrowserLimits) {
        guard limits.page > 0, limits.total > 0 else { recordProblem("会话加载数量须为正整数"); return }
        if isPreview { conversationBrowserLimits = limits; return }
        guard let transport else { recordProblem("本地核心未配置"); return }
        browse({ _ = try transport.setConversationBrowserLimits(limits); return try transport.list() }, source: "保存会话列表设置")
    }
    /// List operations run besides loading and management: they change only
    /// what the list shows and Velune's organization of it.
    func browse(_ operation: @escaping () throws -> ConfigurationSnapshot, source: String, finished: (() -> Void)? = nil) {
        guard !isPreview, !isShuttingDown else { finished?(); return }
        performOnQueue { [weak self] in
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
    func loadMoreConversations(in group: ConversationGroupKey) {
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
    /// Marks the conversation read once, while no mark is on its way.
    func markRead(_ conversation: Conversation) {
        guard !markingRead.contains(conversation.id), let transport else { return }
        markingRead.insert(conversation.id)
        let reference = ConversationReference(runtimeId: conversation.runtimeID, conversationId: conversation.id)
        browse({ try transport.organizeConversations([reference], .markRead) }, source: "整理会话") { [weak self] in
            self?.markingRead.remove(conversation.id)
        }
    }
    /// Deleted conversations close in the tabs showing them; their drafts
    /// and choices go with them (the core removed the persisted ones).
    func noteDeleted(_ ids: Set<String>) {
        deletedConversationIDs.formUnion(ids)
        for id in ids { conversationChoices[id] = nil; composerDrafts[id] = nil; unsavedDrafts[id] = nil }
    }
    /// A pending write must not recreate the draft of a conversation being deleted.
    func dropUnsavedDraft(_ id: String) { unsavedDrafts[id] = nil }
    func resolveAgentURI(_ uri: String, completion: @escaping (Result<AgentUriResolution, Error>) -> Void) {
        guard let transport, !isShuttingDown else { completion(.failure(NSError(domain: "Velune", code: 1, userInfo: [NSLocalizedDescriptionKey: "核心尚未就绪"]))); return }
        performOnQueue {
            let result = Result { try transport.resolveAgentURI(uri) }
            DispatchQueue.main.async { completion(result) }
        }
    }

    // MARK: - Presentation settings

    func setStatusLine(_ items: [StatusLineItem]) {
        if isPreview { statusLine = items; return }
        guard let transport else { recordProblem("本地核心未配置"); return }
        enqueue({ try transport.setStatusLine(items) }) { [weak self] in self?.statusLine = $0 }
    }
    func setOutlinePresentation(_ presentation: OutlinePresentation) {
        if isPreview { outlinePresentation = presentation; return }
        guard let transport else { recordProblem("本地核心未配置"); return }
        enqueue({ try transport.setOutlinePresentation(presentation) }) { [weak self] in self?.outlinePresentation = $0 }
    }
    func setLinkedConversationOpening(_ opening: LinkedConversationOpening) {
        if isPreview { linkedConversationOpening = opening; return }
        guard let transport else { recordProblem("本地核心未配置"); return }
        enqueue({ try transport.setLinkedConversationOpening(opening) }) { [weak self] in self?.linkedConversationOpening = $0 }
    }
    func setUsageDisplay(_ display: UsageDisplay) {
        if isPreview { usageDisplay = display; return }
        guard let transport else { recordProblem("本地核心未配置"); return }
        enqueue({ try transport.setUsageDisplay(display) }) { [weak self] in self?.usageDisplay = $0 }
    }
    func setContextUsageDisplay(_ display: UsageDisplay) {
        if isPreview { contextUsageDisplay = display; return }
        guard let transport else { recordProblem("本地核心未配置"); return }
        enqueue({ try transport.setContextUsageDisplay(display) }) { [weak self] in self?.contextUsageDisplay = $0 }
    }

    // MARK: - Drafts

    func draft(for conversationID: String) -> String { composerDrafts[conversationID] ?? "" }
    /// Records the composer text; persisted shortly after typing pauses. A new
    /// conversation's draft lives only as long as that unsent conversation.
    func setDraft(_ text: String, for conversationID: String) {
        guard composerDrafts[conversationID, default: ""] != text else { return }
        composerDrafts[conversationID] = text.isEmpty ? nil : text
        guard !isPreview, !conversationID.hasPrefix(ConversationStore.draftConversationPrefix) else { return }
        unsavedDrafts[conversationID] = text
        draftSave?.cancel()
        let work = DispatchWorkItem { [weak self] in self?.saveDrafts() }
        draftSave = work
        DispatchQueue.main.asyncAfter(deadline: .now() + 0.5, execute: work)
    }
    func moveDraft(from old: String, to new: String) {
        guard let text = composerDrafts.removeValue(forKey: old) else { return }
        setDraft(text, for: new)
    }
    private func loadDrafts(_ transport: Transport) {
        performOnQueue { [weak self] in
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
        performOnQueue { [weak self] in
            var failure: Error?
            for (id, text) in pending {
                do { try transport.setComposerDraft(conversationID: id, text: text) } catch { failure = error }
            }
            guard let failure else { return }
            DispatchQueue.main.async { self?.recordProblem(failure, source: "保存草稿") }
        }
    }

    // MARK: - Composer choices

    func choices(of conversationID: String) -> ConversationChoices? { conversationChoices[conversationID] }
    func keepChoices(_ value: ConversationChoices?, for conversationID: String) {
        guard conversationChoices[conversationID] != value else { return }
        conversationChoices[conversationID] = value
        // An unsent conversation's choices live as long as it does.
        guard !isPreview, !conversationID.hasPrefix(ConversationStore.draftConversationPrefix), let transport else { return }
        let intent = NextTurnIntent(conversationId: conversationID, runtimeId: value?.runtimeID,
                                    choices: (value?.choices ?? [:]).sorted { $0.key < $1.key }.map { RuntimeTurnChoice(runtimeId: $0.key, choice: $0.value) })
        performOnQueue { [weak self] in
            do { try transport.setNextTurnIntent(intent) }
            catch { DispatchQueue.main.async { self?.recordProblem(error, source: "保存下一轮选择") } }
        }
    }
    /// The choices of an unsent conversation become the created one's.
    func moveChoices(from old: String, to new: String) {
        let value = conversationChoices.removeValue(forKey: old)
        keepChoices(value, for: new)
    }
    func setRuntimeComposerChoices(_ choices: RuntimeComposerChoices) {
        guard runtimeComposerChoices[choices.runtimeId] != choices else { return }
        runtimeComposerChoices[choices.runtimeId] = choices
        guard !isPreview, let transport else { return }
        performOnQueue { [weak self] in
            do { try transport.setRuntimeComposerChoices(choices) }
            catch { DispatchQueue.main.async { self?.recordProblem(error, source: "保存运行时的选择") } }
        }
    }
    /// What a runtime's new conversations start with.
    func newConversationChoice(for runtimeID: String) -> NewConversationChoice {
        runtimeComposerChoices[runtimeID]?.newConversation ?? .lastChosen
    }
    func setNewConversationChoice(_ value: NewConversationChoice, for runtimeID: String) {
        var choices = runtimeComposerChoices[runtimeID] ?? RuntimeComposerChoices(runtimeId: runtimeID, lastChosen: TurnChoice(), newConversation: .lastChosen)
        choices.newConversation = value
        setRuntimeComposerChoices(choices)
    }
    private func loadComposerChoices(_ transport: Transport) {
        performOnQueue { [weak self] in
            let result = Result { try transport.composerChoices() }
            DispatchQueue.main.async {
                guard let self else { return }
                switch result {
                case .success(let value):
                    // Choices made before the read are newer.
                    for intent in value.conversations where self.conversationChoices[intent.conversationId] == nil {
                        self.conversationChoices[intent.conversationId] = ConversationChoices(runtimeID: intent.runtimeId, choices: Dictionary(intent.choices.map { ($0.runtimeId, $0.choice) }) { $1 })
                    }
                    for runtime in value.runtimes where self.runtimeComposerChoices[runtime.runtimeId] == nil {
                        self.runtimeComposerChoices[runtime.runtimeId] = runtime
                    }
                    self.composerChoicesRevision += 1
                case .failure(let failure): self.recordProblem(failure, source: "读取下一轮选择")
                }
            }
        }
    }

    // MARK: - Activity

    /// The device follows login and which conversations execute; each tab
    /// follows the conversation it shows.
    private func schedulePolling() {
        timer?.invalidate()
        timer = Timer.scheduledTimer(withTimeInterval: 0.6, repeats: true) { [weak self] _ in
            Task { @MainActor in
                guard let self else { return }
                self.pollAuthentication(); self.pollActiveConversations(); self.pollQuotaAlerts()
            }
        }
    }
    /// The user is looking at this conversation now.
    func isShowingConversation(_ id: String) -> Bool { NSApp.isActive && focusedConversation?.loadedConversationID == id }
    func showConversation(_ id: String) {
        guard let tab = focusedConversation else { return }
        if tab.loadedConversationID != id { tab.selectConversation(id: id) }
    }
    /// Quota conditions the core raised since the last poll, announced by the notifier.
    private func pollQuotaAlerts() {
        guard started, !isPreview, !isShuttingDown, !quotaAlertPollPending, Date().timeIntervalSince(lastQuotaAlertPoll) >= 3, let transport else { return }
        quotaAlertPollPending = true
        lastQuotaAlertPoll = Date()
        let since = quotaAlertCursor
        performOnQueue { [weak self] in
            let result = Result { try transport.quotaAlerts(since: since) }
            DispatchQueue.main.async {
                guard let self else { return }
                self.quotaAlertPollPending = false
                switch result {
                case .success(let alerts):
                    self.clearActivityProblem("quota-alerts")
                    self.quotaAlertCursor = alerts.cursor
                    for alert in alerts.alerts { self.notifier?.observe(alert) }
                case .failure(let failure): self.recordProblem(failure, source: "读取额度提醒", activityKey: "quota-alerts")
                }
            }
        }
    }
    private func pollActiveConversations() {
        guard started, !isPreview, !isShuttingDown, !activePollPending, let transport else { return }
        activePollPending = true
        let since = activityCursor
        performOnQueue { [weak self] in
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
                    // So does one started and finished between two polls (a quick
                    // turn sent from a paired device).
                    let finishedUnlisted = activity.finished.contains { turn in
                        !subAgents.contains(turn.conversationId) && !self.conversations.contains { $0.id == turn.conversationId }
                    }
                    if finishedUnlisted || !unlisted.isSubset(of: self.unlistedElsewhereIDs) { self.refreshConversations() }
                    self.unlistedElsewhereIDs = unlisted
                    self.notifier?.observe(activity)
                case .failure(let failure): self.recordProblem(failure, source: "读取运行状态", activityKey: "active-conversations")
                }
            }
        }
    }
    func shutdown(stoppingWork: Bool = false, completion: @escaping (Bool) -> Void) {
        guard !isShuttingDown else { completion(false); return }
        guard !isPreview, let transport else { completion(true); return }
        saveDrafts()
        isShuttingDown = true
        timer?.invalidate(); timer = nil
        // Reads queued in any window give way; the changes queued before this still land.
        transport.beginClosing()
        performOnQueue { [weak self] in
            let result = Result { try transport.close(stoppingWork: stoppingWork) }
            DispatchQueue.main.async {
                guard let self else { completion(false); return }
                self.isShuttingDown = false
                switch result {
                case .success:
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

    // MARK: - Analytics

    func loadAnalytics(_ query: AnalyticsQuery) {
        analyticsGeneration &+= 1
        let requestGeneration = analyticsGeneration
        analyticsReadFailed = false
        guard !isPreview else { analyticsIsLoading = false; return }
        guard !isShuttingDown, let transport else { analyticsIsLoading = false; analyticsReadFailed = true; recordProblem("本地核心不可用", source: "读取分析记录"); return }
        analyticsIsLoading = true
        performOnQueue { [weak self] in
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
    func cancelAnalyticsRead() { analyticsGeneration &+= 1; analyticsIsLoading = false }

    // MARK: - Providers, templates and routes

    func saveProvider(_ value: ProviderDraft, authenticationEdit: AuthenticationEdit, quotaAuthenticationEdit: QuotaAuthenticationEdit? = nil, onSaved: (() -> Void)? = nil) {
        guard !authenticationRunning else { recordProblem("请先完成登录"); return }
        if isPreview { var saved = value; if saved.id.isEmpty { saved.id = UUID().uuidString }; for index in saved.models.indices where saved.models[index].recordKey.isEmpty { saved.models[index].recordKey = UUID().uuidString }; gateway.providers.removeAll { $0.id == saved.id }; gateway.providers.append(ProviderSummary(previewOf: saved)); hasGateway = true; onSaved?(); return }
        guard let transport else { recordProblem("本地核心未配置"); return }
        enqueue({ try transport.saveProvider(gatewayID: self.gateway.id, provider: value, authenticationEdit: authenticationEdit, quotaAuthenticationEdit: quotaAuthenticationEdit) }) { [weak self] data in
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
        performOnQueue { [weak self] in
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
    /// Retained observations since `since`; reading never samples the provider.
    func readQuotaHistory(_ providerID: String, since: Date, completion: @escaping (Result<[QuotaHistorySample], Error>) -> Void) {
        guard !isPreview, let transport else { completion(.success([])); return }
        let gatewayID = gateway.id
        performOnQueue {
            let result = Result { try transport.providerQuotaHistory(gatewayID: gatewayID, providerID: providerID, since: since) }
            DispatchQueue.main.async { completion(result) }
        }
    }
    func clearQuotaHistory(_ providerID: String, completion: @escaping () -> Void) {
        guard !isPreview, let transport else { completion(); return }
        let gatewayID = gateway.id
        enqueue({ try transport.clearProviderQuotaHistory(gatewayID: gatewayID, providerID: providerID) }) { _ in completion() }
    }
    func readProviderAPIKey(_ providerID: String, completion: @escaping (String) -> Void) {
        guard !isPreview, let transport else { recordProblem("预览不会读取 API key"); return }
        enqueue({ try transport.readProviderAPIKey(gatewayID: self.gateway.id, providerID: providerID) }, apply: completion)
    }
    func saveTemplate(_ value: ModelTemplate, onSaved: (() -> Void)? = nil) {
        if isPreview { var saved = value; if saved.id.isEmpty { saved.id = UUID().uuidString }; modelTemplates.removeAll { $0.id == saved.id }; modelTemplates.append(saved); onSaved?(); return }
        guard let transport else { recordProblem("本地核心未配置"); return }
        enqueue({ try transport.saveTemplate(value) }) { [weak self] result in self?.modelTemplates = result; onSaved?() }
    }
    func saveTemplates(_ values: [ModelTemplate], onSaved: @escaping (Int) -> Void) {
        guard !values.isEmpty else { return }
        if isPreview { for value in values { saveTemplate(value) }; onSaved(values.count); return }
        guard let transport else { recordProblem("本地核心未配置"); return }
        enqueue({
            var templates: [VeluneBindings.ModelTemplate]?
            var count = 0
            var failures: [AppProblem] = []
            for value in values {
                do { templates = try transport.saveTemplate(value); count += 1 }
                catch { failures.append(AppProblem.failure(error, source: "添加模型模板 · \(value.name)")) }
            }
            return (templates, count, failures)
        }) { [weak self] result in
            if let templates = result.0 { self?.modelTemplates = templates }
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
        enqueue({ try transport.deleteTemplate(id: id) }) { [weak self] result in self?.modelTemplates = result }
    }
    func previewProviderImport(_ source: ProviderImportSource, completion: @escaping (ProviderImportPreview) -> Void) {
        guard !isPreview, !authenticationRunning else { recordProblem("请先完成登录，再读取提供商配置"); return }
        guard let transport else { recordProblem("本地核心未配置"); return }
        enqueue({ try transport.providerImportPreview(gatewayID: self.gateway.id, source: source) }) { completion($0) }
    }
    func applyProviderImport(_ source: ProviderImportSource, preview: ProviderImportPreview, selections: [ImportSelection], replaceExisting: Bool, completion: @escaping () -> Void) {
        guard !isPreview, !authenticationRunning else { recordProblem("请先完成登录，再导入提供商配置"); return }
        guard let transport else { recordProblem("本地核心未配置"); return }
        enqueue({ try transport.providerImportApply(gatewayID: self.gateway.id, source: source, previewToken: preview.token, selections: selections, replaceExisting: replaceExisting) }) { [weak self] data in
            guard let self else { return }
            if let saved = data.gateways.first(where: { $0.id == self.gateway.id }) { self.gateway = saved; self.hasGateway = true }
            self.refreshAfterConfigurationChange()
            completion()
        }
    }
    func saveModelRoute(_ value: ModelRoute, onSaved: (() -> Void)? = nil) {
        guard !authenticationRunning else { recordProblem("请先完成登录"); return }
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
    private func applyGatewayUpdate(_ data: GatewayUpdate) {
        if let saved = data.gateways.first(where: { $0.id == gateway.id }) { gateway = saved; hasGateway = true }
        if data.executionInvalidated { refreshAfterConfigurationChange() }
    }
    /// The list again after a change the core's execution depends on.
    private func refreshAfterConfigurationChange() {
        guard let transport else { recordProblem("本地核心未配置"); return }
        enqueue({ try transport.list() }) { [weak self] in self?.applyList($0) }
    }

    // MARK: - Authentication

    func startAuthentication(_ providerID: String) {
        guard !isPreview else { recordProblem("预览不会启动认证"); return }
        guard let transport else { recordProblem("本地核心未配置"); return }
        enqueue({ try transport.authenticationStart(gatewayID: self.gateway.id, providerID: providerID) }) { [weak self] data in
            guard let self else { return }
            authenticationPrompt = nil; authenticationNotifications = []; authenticationResult = nil
            showsAuthentication = true; applyAuthentication(data)
        }
    }
    func answerAuthentication(id: String, value: String) {
        guard let transport else { recordProblem("本地核心未配置"); return }
        enqueue({ try transport.authenticationReply(promptID: id, value: value) }) { [weak self] in self?.authenticationPrompt = nil; self?.applyAuthentication($0) }
    }
    func cancelAuthentication() {
        guard let transport else { recordProblem("本地核心未配置"); return }
        enqueue({ try transport.authenticationCancel() }) { [weak self] in self?.applyAuthentication($0) }
    }
    /// Reads again the status of a login the provider references, which its own CLI keeps.
    func inspectAuthentication(_ providerID: String) {
        guard !isPreview else { recordProblem("预览不会检查登录"); return }
        guard let transport else { recordProblem("本地核心未配置"); return }
        enqueue({ _ = try transport.authenticationInspect(gatewayID: self.gateway.id, providerID: providerID); return try transport.list() }) { [weak self] in
            self?.applyList($0); self?.readProviderQuotas([providerID], refresh: true)
        }
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
        performOnQueue { [weak self] in
            let result = Result { try transport.authenticationPoll() }
            DispatchQueue.main.async {
                guard let self else { return }
                self.authenticationPollPending = false
                switch result {
                case .success(let data): self.clearActivityProblem("authentication"); self.applyAuthentication(data)
                case .failure(let failure): self.recordProblem(failure, source: "读取登录进度", activityKey: "authentication")
                }
            }
        }
    }
    private func applyAuthentication(_ data: AuthenticationProgress) {
        if !data.running { clearActivityProblem("authentication") }
        authenticationRunning = data.running
        if let saved = data.gateways?.first(where: { $0.id == gateway.id }) { gateway = saved }
        if data.executionInvalidated == true { refreshAfterConfigurationChange() }
        for event in data.events {
            if event.typeId == "prompt", let id = event.id, let prompt = event.prompt {
                authenticationPrompt = PendingAuthenticationPrompt(id: id, kind: prompt.kind, text: prompt.text, options: prompt.options)
            } else if event.typeId == "notify", let notification = event.notification {
                if notification.kind == "prompt_cancelled" {
                    if authenticationPrompt?.id == notification.id { authenticationPrompt = nil }
                } else {
                    authenticationNotifications.append(notification)
                    // A sign-in page opens in the browser; the sheet keeps a link to open it again.
                    if notification.kind == "url", let address = notification.url, let url = URL(string: address), url.scheme == "https" { NSWorkspace.shared.open(url) }
                }
            } else if event.typeId == "result" {
                authenticationPrompt = nil
                authenticationResult = event.ok == true ? "登录完成。" : event.cancelled == true ? "登录已取消。" : nil
                if event.ok != true && event.cancelled != true { recordProblem(event.error ?? "登录未完成。", source: "登录") }
            }
        }
    }

    // MARK: - Runtimes

    func runtimeDiscoveryHints(userHome: String, overrides: [String: String], onFailure: (() -> Void)? = nil, completion: @escaping ([RuntimeDiscoveryHint]) -> Void) {
        guard let transport else { recordProblem("本地核心未配置"); return }
        enqueue({ try transport.runtimeDiscoveryHints(userHome: userHome, overrides: overrides) }, onFailure: { _ in onFailure?() }, apply: completion)
    }
    func discoverRuntimes(_ probes: [RuntimeDiscoveryProbe], onFailure: (() -> Void)? = nil, completion: @escaping ([RuntimeDiscoveryCandidate]) -> Void) {
        guard let transport else { recordProblem("本地核心未配置"); return }
        enqueue({ try transport.discoverRuntimes(probes) }, onFailure: { _ in onFailure?() }, apply: completion)
    }
    func importRuntimes(_ candidates: [RuntimeDiscoveryCandidate], completion: @escaping (Set<String>) -> Void) {
        guard !isBusy, !candidates.isEmpty, candidates.allSatisfy({ $0.supported && !$0.alreadyConfigured }) else { recordProblem("请仅选择可导入且尚未配置的运行时"); return }
        guard let transport else { recordProblem("本地核心未配置"); return }
        let gatewayID = gateway.id
        enqueue({
            var imported: Set<String> = []
            var failures: [AppProblem] = []
            for candidate in candidates {
                var runtime = candidate.runtime; runtime.gatewayId = gatewayID
                do { _ = try transport.upsertRuntime(runtime); imported.insert(runtime.id) }
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
    func saveRuntimeInstance(_ instance: RuntimeInstance, execution: RuntimeExecution? = nil, launch: AcpLaunch? = nil, onSaved: (() -> Void)? = nil) {
        if isPreview { runtimeInstances.removeAll { $0.id == instance.id }; runtimeInstances.append(instance); onSaved?(); return }
        guard let transport else { recordProblem("本地核心未配置"); return }
        enqueue({
            if let execution, let launch { return try transport.upsertRuntime(instance, execution: execution, launch: launch) }
            return try execution.map { try transport.upsertRuntime(instance, execution: $0) } ?? transport.upsertRuntime(instance)
        }) { [weak self] data in
            guard let self else { return }
            runtimeInstances = data.runtimeInstances; runtimeTypes = data.runtimeTypes
            refreshAfterConfigurationChange()
            if data.runtimeInstances.contains(where: { $0.id == instance.id }) { onSaved?() }
            else { recordProblem("核心未返回保存后的运行时实例") }
        }
    }
    /// The saved location and HOME (and where that HOME is), plus the launch
    /// options when the instance's type takes them.
    func runtimeExecution(id: String, launch: Bool, onFailure: @escaping () -> Void, completion: @escaping (RuntimeExecution, String?, AcpLaunch?) -> Void) {
        guard let transport, !isLoading, !isShuttingDown else { onFailure(); return }
        enqueue({
            let (execution, home) = try transport.runtimeExecution(id: id)
            return (execution, home, launch ? try transport.runtimeLaunch(id: id) : nil)
        }, onFailure: { _ in onFailure() }) { completion($0.0, $0.1, $0.2) }
    }
    /// Starts the saved instance's Agent (unless a session of it is live) to
    /// read what it reports; this does not send anything to a model.
    func agentInfo(runtimeID: String, completion: @escaping (Result<AcpRuntimeInfo, Error>) -> Void) {
        guard let transport, !isShuttingDown else { return }
        performOnQueue {
            let result = Result { try transport.agentInfo(runtimeID: runtimeID) }
            DispatchQueue.main.async { completion(result) }
        }
    }
    func deleteRuntimeInstance(id: String) {
        if isPreview { runtimeInstances.removeAll { $0.id == id }; return }
        guard let transport else { recordProblem("本地核心未配置"); return }
        enqueue({ try transport.deleteRuntime(id: id) }) { [weak self] data in
            self?.runtimeInstances = data.runtimeInstances; self?.runtimeTypes = data.runtimeTypes
            if data.executionInvalidated { self?.refreshAfterConfigurationChange() }
        }
    }
    /// A runtime instance's catalog for its settings, read afresh; listing
    /// native models starts the runtime.
    func readModelCatalog(runtimeID: String, done: @escaping (RuntimeModelCatalog) -> Void) {
        guard !isPreview else { done(.ready([])); return }
        guard let transport else { done(.failed("本地核心未配置")); return }
        performOnQueue {
            let result = Result { try transport.runtimeModels(runtimeID: runtimeID) }
            DispatchQueue.main.async {
                switch result {
                case .success(let models): done(.ready(models))
                case .failure(let error): done(.failed(AppProblem.failure(error, source: "模型目录").detail))
                }
            }
        }
    }
    private func refreshRuntimeLocations() {
        let ids = runtimeInstances.map(\.id)
        guard let transport, !isPreview, !isShuttingDown else { return }
        let request = UUID()
        runtimeLocationsRequest = request
        performOnQueue { [weak self] in
            let result = Result { try transport.runtimeLocations(ids: ids) }
            DispatchQueue.main.async {
                guard let self, self.runtimeLocationsRequest == request, case .success(let locations) = result else { return }
                if self.runtimeLocations != locations { self.runtimeLocations = locations }
            }
        }
    }

    // MARK: - Problems

    func recordProblem(_ error: Error, source: String = "操作", activityKey: String? = nil) {
        if case .operation(_, .quitting) = error as? TransportError { return }
        let value = AppProblem.failure(error, source: source, activityKey: activityKey)
        if let activityKey, problems.contains(where: { $0.activityKey == activityKey && $0.detail == value.detail && $0.code == value.code && $0.phase == value.phase }) { return }
        if let activityKey { problems.removeAll { $0.activityKey == activityKey } }
        appendProblems([value])
    }
    func recordProblem(_ message: String, source: String = "操作") { recordProblem(TransportError.rejected(message), source: source) }
    func appendProblems(_ values: [AppProblem]) {
        problems.append(contentsOf: values)
        if problems.count > 100 { problems.removeFirst(problems.count - 100) }
    }
    /// A background read that failed before works again.
    func clearActivityProblem(_ key: String) {
        guard problems.contains(where: { $0.activityKey == key }) else { return }
        problems.removeAll { $0.activityKey == key }
    }
    func clearProblem(_ id: UUID) { problems.removeAll { $0.id == id } }
    func clearProblems() { problems.removeAll() }
    private func reconcileHistoryProblems(_ failures: [HistoryFailure]) {
        let keys = Set(failures.map { "history:" + $0.runtimeId })
        problems.removeAll { $0.activityKey?.hasPrefix("history:") == true && !keys.contains($0.activityKey ?? "") }
        for failure in failures {
            let key = "history:" + failure.runtimeId
            if problems.contains(where: { $0.activityKey == key && $0.detail == failure.detail }) { continue }
            problems.removeAll { $0.activityKey == key }
            appendProblems([AppProblem(id: UUID(), occurredAt: Date(), source: "会话列表 · " + (runtimeInstances.first { $0.id == failure.runtimeId }?.name ?? failure.runtimeId), detail: failure.detail, kind: "history", code: nil, phase: nil, operationID: nil, activityKey: key)])
        }
    }

    // MARK: - Requests

    /// Runs `work` on the device's serial queue, recording how long it waited.
    func performOnQueue(_ work: @escaping @Sendable () -> Void) {
        let owner = try? transport?.telemetryResult.get()
        let queuedAt = ProcessInfo.processInfo.systemUptime
        queue.async {
            owner?.recordQueueWait(ProcessInfo.processInfo.systemUptime - queuedAt)
            work()
        }
    }
    /// One settings operation at a time; a failure becomes a problem.
    private func enqueue<T: Sendable>(_ operation: @escaping () throws -> T, onFailure: ((Error) -> Void)? = nil, apply: @escaping (T) -> Void) {
        guard !isLoading, !isShuttingDown else { recordProblem("请等待当前操作完成后重试"); return }
        guard transport != nil else { recordProblem("本地核心未配置"); return }
        isLoading = true
        performOnQueue { [weak self] in
            let result = Result { try operation() }
            DispatchQueue.main.async {
                guard let self else { return }
                self.isLoading = false
                switch result {
                case .success(let value): apply(value)
                case .failure(let failure): self.recordProblem(failure); onFailure?(failure)
                }
            }
        }
    }

    // MARK: - Preview

    /// Preview only: renaming and deleting without a core.
    func renameInPreview(_ id: String, title: String) {
        if let index = conversations.firstIndex(where: { $0.id == id }) { conversations[index].title = title }
        previewSnapshots[id]?.conversation.title = title
    }
    func deleteInPreview(_ ids: Set<String>) {
        conversations.removeAll { ids.contains($0.id) }
        for id in ids { previewSnapshots.removeValue(forKey: id) }
        noteDeleted(ids)
    }
    func keepPreviewSnapshot(_ value: ConversationSnapshot) { previewSnapshots[value.conversation.id] = value }

    private func seedPreview() {
        conversationBrowserPreferences = ConversationBrowser().preferences
        gateway = GatewaySummary(id: "default", name: "默认网关", providers: [ProviderSummary(previewOf: ProviderDraft(id: "sample-provider", name: "示例 AI 服务", protocol: .chatCompletions, endpoint: "https://example.invalid/v1", executionBackend: .http, requestProfile: .native, authenticationSource: .apiKey, quotaSource: .none, models: [ProviderModel(recordKey: "sample-model", providerModelId: "external-example", nickname: "通用模型", contextWindow: 8192, maxOutputTokens: 4096)], quotaLimits: []))], routes: [])
        hasGateway = true
        protocols = [ProtocolDescriptor(id: .chatCompletions, name: "OpenAI Chat Completions v1", supported: true), ProtocolDescriptor(id: .responses, name: "OpenAI Responses v1", supported: true), ProtocolDescriptor(id: .messages, name: "Anthropic Messages v1", supported: true)]
        runtimeTypes = [RuntimeTypeDescriptor(id: .pi, familyId: "sample", compatibleVersions: "", canRenameConversations: true, canDeleteConversations: true, launchOptions: false, agentInfo: false, sessionOptions: false, nativeAccountQuota: false, remoteResources: false, supportedProtocols: [.chatCompletions, .responses, .messages], supportedProviderProtocols: [.chatCompletions, .responses, .messages], name: "示例运行时", fields: [], permissionModes: [])]
        runtimeInstances = [RuntimeInstance(id: "sample-instance", name: "示例运行时", typeId: .pi, gatewayId: gateway.id, settings: [:]), RuntimeInstance(id: "sample-review", name: "另一个运行时", typeId: .pi, gatewayId: gateway.id, settings: [:])]
        selectedRuntimeInstanceID = runtimeInstances[0].id
        let topics = ["让设置页更安静", "整理一段代码", "下一步的项目计划"]
        let pairs = [
            ["我想让设置页面更简单，先帮我梳理一下。", "# 一次清晰的工作循环\n\n先整理任务，再执行。\n\n- [x] 读取上下文\n- [ ] 完成修改\n\n> 每一步都保留可以复核的结果。\n\n| 内容 | 状态 |\n| --- | --- |\n| 会话 | 可继续 |\n| 工具 | 已完成 |\n\n正文支持**强调**、[链接](https://example.invalid)和 `inline code`。", "运行时实例应该放哪里？", "工具栏显示当前运行时实例，详细配置放进系统设置窗口里。会话页面保留对话与模型选择，让注意力留在正在做的工作上。"],
            ["这个读取配置的函数可以再清晰一点吗？", "先把输入校验放在函数边界，主体就能直接表达意图。\n\n```rust\nfn load(path: &Path) -> Result<Config> {\n    let source = fs::read_to_string(path)?;\n    let config: Config = toml::from_str(&source)?;\n    config.validate()?;\n    Ok(config)\n}\n```", "还需要抽一个新的层吗？", "目前不需要。等第二个调用方出现不同的读取需求，再决定是否有值得共享的边界。"],
            ["帮我把接下来的工作排一下。", "先完成可用的会话界面，再检查配置是否真正驱动运行时。最后用一个短任务验证发送、停止和会话切换。", "先从哪个环节开始？", "从会话开始：打开旧会话、发送消息、检查回复，再新建一个会话。这个路径会暴露最直接的体验问题。"]
        ]
        for index in topics.indices {
            let conversation = Conversation(id: "sample-\(index)", title: topics[index], updatedAtUnixMs: Int64(Date().timeIntervalSince1970 * 1000) - Int64(index * 86_400_000), createdAtUnixMs: Int64(Date().timeIntervalSince1970 * 1000) - Int64((index + 3) * 86_400_000), runtimeID: runtimeInstances[index == 2 ? 1 : 0].id, cwd: index == 0 ? "/Users/example/Projects/Velune" : index == 1 ? "/Users/example/Projects/Sample" : nil)
            var history = pairs[index].enumerated().map { DeliveredMessage(id: "sample-\(index)-\($0.offset)", role: $0.offset.isMultiple(of: 2) ? .user : .assistant, blocks: [.text(text: $0.element)]) }
            history[1].blocks.insert(.reasoning(summary: "先确认上下文", text: .inline(text: "再选择最小的修改范围。")), at: 0)
            history[1].blocks.append(.tool(toolId: "sample-tool-\(index)", title: index == 1 ? "检查项目代码" : "读取工作上下文", state: .completed, output: .inline(text: index == 1 ? "已读取 3 个文件，未修改项目。" : "已梳理当前任务的上下文。"), linkedConversationId: nil))
            // The last sample is mid-turn: summaries with and without text yet, then a running tool.
            let running = index == 2
            if running { history[history.count - 1].blocks = [.reasoning(summary: "Reading the settings view", text: .inline(text: "The sidebar and the detail pane share one split view.")), .reasoning(summary: "Inspecting the project layout", text: .inline(text: "")), .tool(toolId: "sample-progress", title: "整理项目计划", state: .running, output: .inline(text: "正在整理计划。"), linkedConversationId: nil)] }
            previewSnapshots[conversation.id] = ConversationSnapshot(revision: 1, conversation: conversation, contextRuntimeID: conversation.runtimeID, modelRecordKey: "sample-model", runState: running ? .running : .idle, messages: history,
                transcriptTurns: [
                    TranscriptTurn(id: "preview-work-" + conversation.id, userMessageId: history[0].id, workMessageIds: [history[1].id], lastMessageId: nil, replyStartBlock: 0, durationMs: nil, isRunning: false, settings: nil, usage: nil),
                    TranscriptTurn(id: "preview-answer-" + conversation.id, userMessageId: history[2].id, workMessageIds: running ? [history[3].id] : [], lastMessageId: running ? nil : history[3].id, replyStartBlock: 0, durationMs: nil, isRunning: running, settings: nil, usage: nil)
                ],
                transcriptItems: [.message(id: history[0].id, messageId: history[0].id), .work(turnId: "preview-work-" + conversation.id), .message(id: history[2].id, messageId: history[2].id)]
                    + (running ? [.work(turnId: "preview-answer-" + conversation.id)] : [.message(id: history[3].id, messageId: history[3].id)]) + history.dropFirst(4).map { .message(id: $0.id, messageId: $0.id) },
                transcriptOutline: history.filter { $0.role == .user }.map { TranscriptOutlineEntry(id: $0.id, messageId: $0.id) },
                transcriptMessageIdentities: history.map { TranscriptMessageIdentity(id: $0.id, messageId: $0.id) },
                actions: ConversationActions(canSend: !running, canCancel: running, canSwitch: true))
            conversations.append(conversation)
        }
    }
}
