import Combine
import Foundation
import AppKit
import VeluneBindings

@MainActor
final class AppStore: ObservableObject {
    @Published private(set) var conversationBrowserPreferences: BindingConversationBrowserPreferences?
    private var savedBrowserPreferences: BindingConversationBrowserPreferences?
    private var browserPreferencesGeneration: UInt64 = 0
    @Published private(set) var outlinePresentation: BindingOutlinePresentation = .popover
    @Published private(set) var conversationBrowserGroupLimit = 20
    @Published private(set) var conversations: [Conversation] = []
    @Published private(set) var selectedConversationIDs: Set<String> = []
    @Published private(set) var selectedConversationID: String?
    @Published private(set) var pendingConversationID: String?
    @Published private(set) var conversationManagementStatus: String?
    @Published private(set) var projectionRuntimeID: String?
    @Published private(set) var nextTurnRuntimeID: String?
    @Published private(set) var nextTurnModelRecordKey: String?
    let transcript = TranscriptModel()
    @Published private(set) var gateway = GatewayConfig()
    @Published private(set) var runtimeInstances: [RuntimeInstance] = []
    @Published private(set) var runtimeTypes: [RuntimeTypeDescriptor] = []
    @Published private(set) var protocols: [ProtocolDescriptor] = []
    @Published private(set) var modelTemplates: [ModelTemplate] = []
    @Published private(set) var providerImportTypes: [RuntimeTypeDescriptor] = []
    @Published private(set) var authenticationRunning = false
    @Published private(set) var authenticationPrompt: AuthenticationPrompt?
    @Published private(set) var authenticationNotifications: [AuthenticationNotification] = []
    @Published private(set) var authenticationResult: String?
    @Published var showsAuthentication = false
    @Published private(set) var isLoading = false
    @Published private(set) var isShuttingDown = false
    @Published private(set) var activity: String?
    @Published private(set) var problems: [AppProblem] = []
    @Published private var snapshot: ConversationSnapshot?

    private let transport: Transport?
    /// Native runtime resources are edited from settings with their own loading state.
    var resourceTransport: Transport? { transport }
    // Workspaces share application services; their navigation and transcript stay local.
    private let sharedApplication: AppStore?
    private var subscriptions: Set<AnyCancellable> = []
    private var sharesApplication = false
    private var sharedUpdateScheduled = false
    @Published private var executionStarting = false
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
    /// Next-turn runtime and model chosen per conversation in this workspace.
    private var nextTurnIntents: [String: NextTurnIntent] = [:]
    private var intentConversationID: String?
    private var lastChosenModelRecordKey: String?
    private struct NextTurnIntent { var runtimeID: String?; var modelRecordKey: String? }

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
        if conversationBrowserPreferences != application.conversationBrowserPreferences { conversationBrowserPreferences = application.conversationBrowserPreferences }
        savedBrowserPreferences = application.savedBrowserPreferences
        if conversationBrowserGroupLimit != application.conversationBrowserGroupLimit { conversationBrowserGroupLimit = application.conversationBrowserGroupLimit }
        if outlinePresentation != application.outlinePresentation { outlinePresentation = application.outlinePresentation }
        var sharedConversations = application.conversations
        if let current = snapshot?.conversation, !isDraft, !sharedConversations.contains(where: { $0.id == current.id }) { sharedConversations.insert(current, at: 0) }
        if conversations != sharedConversations { conversations = sharedConversations }
        if authenticationRunning != application.authenticationRunning { authenticationRunning = application.authenticationRunning }
        if authenticationPrompt != application.authenticationPrompt { authenticationPrompt = application.authenticationPrompt }
        if authenticationNotifications != application.authenticationNotifications { authenticationNotifications = application.authenticationNotifications }
        if authenticationResult != application.authenticationResult { authenticationResult = application.authenticationResult }
        if problems != application.problems { problems = application.problems }
        reconcileNextTurnIntent()
        if !isDraft, pendingConversationID == nil, let active = application.snapshot,
           active.conversation.id == loadedConversationID, active.conversation.runtimeID == snapshot?.conversation.runtimeID {
            apply(active, preserveSelection: true)
        }
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

    var executionBusyElsewhere: Bool {
        guard let sharedApplication, sharedApplication.applicationIsGenerating else { return false }
        return sharedApplication.loadedConversationID != loadedConversationID || sharedApplication.snapshot?.conversation.runtimeID != snapshot?.conversation.runtimeID
    }
    var applicationIsGenerating: Bool { isGenerating || executionStarting || sharedApplication?.applicationIsGenerating == true }

    var models: [ModelChoice] { gateway.providers.flatMap { provider in provider.models.map { ModelChoice(recordKey: $0.recordKey, displayName: "\($0.displayName) · \(provider.name)", protocolID: provider.protocolID) } } }
    var enabledRuntimeInstances: [RuntimeInstance] { runtimeInstances.filter(\.enabled) }
    var providers: [AIProvider] { gateway.providers }
    var loadedConversationID: String? { snapshot?.conversation.id }
    var loadedConversationWorkingDirectory: String? { snapshot?.conversation.cwd }
    var selectedConversationTitle: String? { conversations.first { $0.id == selectedConversationID }?.title ?? (snapshot?.conversation.id == selectedConversationID ? snapshot?.conversation.title : nil) }
    var isGenerating: Bool { snapshot?.runState == .running || snapshot?.runState == .stopping }
    var isBusy: Bool { isLoading || isGenerating || isShuttingDown || authenticationRunning }
    var canManageConversations: Bool { !applicationIsGenerating && !authenticationRunning && !isShuttingDown && conversationManagementStatus == nil && (!isLoading || pendingConversationID != nil) }
    var canSend: Bool { !applicationIsGenerating && !authenticationRunning && !isLoading && !isGenerating && !isShuttingDown && (snapshot?.actions.canSend ?? false) && nextTurnModelRecordKey != nil && runtimeCompatibleModels.contains { $0.recordKey == nextTurnModelRecordKey } }
    var canCancel: Bool { snapshot?.actions.canCancel ?? false }
    var canSwitchModel: Bool { nextTurnRuntimeID != nil && !isShuttingDown }
    var needsModelSelection: Bool { snapshot != nil && nextTurnModelRecordKey == nil }
    var selectedModelName: String? { models.first { $0.recordKey == nextTurnModelRecordKey }?.displayName }
    var runtimeCompatibleModels: [ModelChoice] {
        let typeID = runtimeInstances.first { $0.id == nextTurnRuntimeID }?.typeID
        guard let descriptor = runtimeTypes.first(where: { $0.id == typeID }) else { return [] }
        return models.filter { descriptor.supportedProviderProtocols.contains($0.protocolID) }
    }
    func selectNextTurnRuntime(_ id: String) {
        guard !isShuttingDown, enabledRuntimeInstances.contains(where: { $0.id == id }) else { return }
        hasInitializedNextTurnIntent = true
        nextTurnRuntimeID = id
        if nextTurnModelRecordKey != nil, !runtimeCompatibleModels.contains(where: { $0.recordKey == nextTurnModelRecordKey }) { nextTurnModelRecordKey = nil }
        rememberNextTurnIntent()
    }
    var pendingInteractions: [RuntimeInteraction] { snapshot?.pendingInteractions ?? [] }
    func replyInteraction(_ interaction: RuntimeInteraction, reply: RuntimeInteractionReply) {
        guard pendingInteractions.contains(where: { $0.id == interaction.id }), let runtimeID = projectionRuntimeID, let transport else { return }
        let currentGeneration = generation
        enqueue({ try transport.replyRuntimeInteraction(runtimeID: runtimeID, interactionID: interaction.id, reply: reply) }) { [weak self] value in
            guard let self, generation == currentGeneration else { return }; applySnapshotResult(value)
        }
    }
    func shutdown(completion: @escaping (Bool) -> Void) {
        if sharedApplication != nil { subscriptions.removeAll(); completion(true); return }
        guard !isShuttingDown else { completion(false); return }
        guard !isPreview, let transport else { completion(true); return }
        saveDrafts()
        isShuttingDown = true
        timer?.invalidate(); timer = nil
        queue.async { [weak self] in
            let result = Result { try transport.close() }
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
    private func schedulePolling() {
        timer?.invalidate()
        timer = Timer.scheduledTimer(withTimeInterval: 0.6, repeats: true) { [weak self] _ in
            Task { @MainActor in self?.pollAuthentication(); self?.poll() }
        }
    }
    func start(loadFirstConversation: Bool = true) {
        guard !started else { return }; started = true
        sharedApplication?.start(loadFirstConversation: false)
        guard !isPreview, let transport else { return }
        if sharedApplication == nil { loadDrafts(transport) }
        enqueue({ try transport.list() }) { [weak self] data in
            guard let self else { return }
            applyList(data)
            if sharedApplication == nil { schedulePolling() }
            if loadFirstConversation, let id = conversations.first?.id { selectConversation(id: id) }
        }
    }
    func refreshConversations() {
        guard !isPreview, !isBusy, let transport else { return }
        enqueue({ try transport.list() }) { [weak self] in self?.applyList($0) }
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
    func cancelAnalyticsRead() { analyticsGeneration &+= 1; analyticsIsLoading = false }
    func setConversationBrowserPreferences(_ preferences: BindingConversationBrowserPreferences) {
        guard preferences != conversationBrowserPreferences, !isShuttingDown else { return }
        if isPreview { conversationBrowserPreferences = preferences; return }
        guard let transport else { recordProblem("本地核心未配置"); return }
        browserPreferencesGeneration &+= 1
        let revision = browserPreferencesGeneration
        conversationBrowserPreferences = preferences
        // Sidebar choices are independent of loading/execution. Serialize their
        // commits with core operations without locking ordinary browsing.
        queue.async { [weak self] in
            let result = Result { try transport.setConversationBrowserPreferences(preferences) }
            DispatchQueue.main.async {
                guard let self else { return }
                switch result {
                case .success(let saved):
                    self.savedBrowserPreferences = saved
                    if revision == self.browserPreferencesGeneration { self.conversationBrowserPreferences = saved }
                case .failure(let failure):
                    self.recordProblem(failure)
                    if revision == self.browserPreferencesGeneration { self.conversationBrowserPreferences = self.savedBrowserPreferences }
                }
            }
        }
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
    func setConversationBrowserGroupLimit(_ limit: Int) {
        guard let value = UInt32(exactly: limit), value > 0 else { recordProblem("每组展示数量须为正整数"); return }
        if isPreview { conversationBrowserGroupLimit = limit; return }
        guard let transport else { recordProblem("本地核心未配置"); return }
        enqueue({ try transport.setConversationBrowserGroupLimit(value) }) { [weak self] result in self?.conversationBrowserGroupLimit = Int(result) }
    }
    func selectConversations(ids: Set<String>) {
        guard !isLoading, !isGenerating, !isShuttingDown, !authenticationRunning else { return }
        selectedConversationIDs = ids.intersection(conversations.map(\.id))
        if selectedConversationIDs.count == 1, let id = selectedConversationIDs.first, id != loadedConversationID { selectConversation(id: id) }
    }
    func selectConversation(id: String) {
        guard !isLoading, !isGenerating, let conversation = conversations.first(where: { $0.id == id }) else { return }
        selectedConversationIDs = [id]
        if isPreview { projectionRuntimeID = conversation.runtimeID; apply(previewSnapshots[id]); return }
        guard let transport else { recordProblem("本地核心未配置"); return }
        // The sidebar reflects user intent immediately. The old loaded transcript
        // is retained until success, but hidden while this destination is loading.
        let previousSelection = snapshot?.conversation.id
        isDraft = false
        generation += 1 // Invalidate any poll already queued for the old conversation.
        selectedConversationID = id
        pendingConversationID = id
        enqueue({ try transport.openConversation(runtimeID: conversation.runtimeID, conversationID: id) }, onFailure: { [weak self] _ in
            self?.selectedConversationID = previousSelection
            self?.selectedConversationIDs = Set([previousSelection].compactMap { $0 })
            self?.finishConversationLoading()
        }) { [weak self] in
            self?.applySnapshotResult($0)
            self?.finishConversationLoading()
        }
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
        conversation.canRename
    }
    func canDeleteConversation(_ conversation: Conversation) -> Bool {
        conversation.canDelete
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
        guard !isLoading, !applicationIsGenerating else { return }
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
    func send(text: String, onAccepted: ((String?) -> Void)? = nil) {
        guard !text.trimmingCharacters(in: .whitespacesAndNewlines).isEmpty else { return }
        guard canSend, let target = nextTurnRuntimeID, let model = nextTurnModelRecordKey, let source = snapshot?.conversation.runtimeID, let conversationID = loadedConversationID else { return }
        if isPreview {
            guard var value = snapshot else { return }
            value.modelRecordKey = model
            let message = Message(id: UUID().uuidString, role: .user, blocks: [.text(text)])
            value.messages.append(message)
            value.transcriptItems.append(.message(id: message.id, messageID: message.id))
            value.transcriptOutline.append(TranscriptOutlineEntry(id: message.id, messageID: message.id))
            value.transcriptMessageIdentities.append(TranscriptMessageIdentity(id: message.id, messageID: message.id))
            apply(value); onAccepted?(loadedConversationID); return
        }
        guard let transport else { recordProblem("本地核心未配置"); return }
        let application = sharedApplication ?? self
        application.executionStarting = true
        let creating = isDraft
        let cwd = snapshot?.conversation.cwd
        enqueue({
            if creating {
                return try transport.createAndSend(runtimeID: target, cwd: cwd, modelRecordKey: model, text: text) { [weak self] created in
                    DispatchQueue.main.async {
                        guard let self else { return }
                        self.isDraft = false
                        // Text typed while sending and the next-turn choice follow the created conversation.
                        if let id = created.snapshot?.conversation.id {
                            self.moveDraft(from: conversationID, to: id)
                            self.nextTurnIntents[id] = self.nextTurnIntents.removeValue(forKey: conversationID)
                        }
                        self.sharedApplication?.applySnapshotResult(created)
                        self.applySnapshotResult(created)
                    }
                }
            }
            return try transport.sendTurn(sourceRuntimeID: source, conversationID: conversationID, targetRuntimeID: target, modelRecordKey: model, text: text)
        }, onAccepted: { [weak self] in onAccepted?(self?.loadedConversationID) }, onFailure: { _ in application.executionStarting = false }) { [weak self] value in
            application.executionStarting = false
            self?.sharedApplication?.applySnapshotResult(value)
            self?.applySnapshotResult(value)
        }
    }
    func cancel() {
        guard canCancel, !isLoading else { return }
        if isPreview { snapshot?.runState = .idle; snapshot?.actions.canCancel = false; apply(snapshot); return }
        guard let runtimeID = projectionRuntimeID else { recordProblem("请选择运行时实例"); return }
        guard let transport else { recordProblem("本地核心未配置"); return }
        enqueue({ try transport.cancel(runtimeID: runtimeID) }) { [weak self] in self?.applySnapshotResult($0) }
    }
    func saveProvider(_ value: AIProvider, authenticationEdit: AuthenticationEdit, onSaved: (() -> Void)? = nil) {
        guard !authenticationRunning, !isGenerating else { recordProblem("请先停止当前任务或完成登录"); return }
        if isPreview { var saved = value; if saved.id.isEmpty { saved.id = UUID().uuidString }; for index in saved.models.indices where saved.models[index].recordKey.isEmpty { saved.models[index].recordKey = UUID().uuidString }; gateway.providers.removeAll { $0.id == saved.id }; gateway.providers.append(saved); hasGateway = true; onSaved?(); return }
        guard let transport else { recordProblem("本地核心未配置"); return }
        enqueue({ try transport.saveProvider(gatewayID: self.gateway.id, provider: value, authenticationEdit: authenticationEdit) }) { [weak self] data in
            self?.applyGatewayUpdate(data); onSaved?()
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
                } else { authenticationNotifications.append(notification) }
            } else if event.type == "result" {
                authenticationPrompt = nil
                authenticationResult = event.ok == true ? "登录完成，认证已保存在原来源。" : event.cancelled == true ? "登录已取消。" : nil
                if event.ok != true && event.cancelled != true { recordProblem(event.error ?? "登录未完成。", source: "登录") }
            }
        }
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
    func saveRuntimeInstance(_ instance: RuntimeInstance, onSaved: (() -> Void)? = nil) {
        guard !isGenerating else { recordProblem("请先停止当前任务，再修改运行时实例"); return }
        if isPreview { runtimeInstances.removeAll { $0.id == instance.id }; runtimeInstances.append(instance); if !instance.enabled && projectionRuntimeID == instance.id { resetProjection(); projectionRuntimeID = nil }; onSaved?(); return }
        guard let transport else { recordProblem("本地核心未配置"); return }
        enqueue({ try transport.upsertRuntime(BindingMapping.bindingRuntime(instance)) }) { [weak self] data in
            guard let self else { return }
            runtimeInstances = data.runtimeInstances.map(BindingMapping.runtime); runtimeTypes = data.runtimeTypes.map(BindingMapping.runtimeType)
            if data.executionInvalidated { refreshAfterExecutionInvalidation() }
            else { enqueue({ try transport.list() }) { [weak self] in self?.applyList($0) } }
            if data.runtimeInstances.contains(where: { $0.id == instance.id }) { onSaved?() }
            else { recordProblem("核心未返回保存后的运行时实例") }
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
    func selectModel(modelRecordKey: String) {
        guard canSwitchModel, runtimeCompatibleModels.contains(where: { $0.recordKey == modelRecordKey }) else { return }
        nextTurnModelRecordKey = modelRecordKey
        lastChosenModelRecordKey = modelRecordKey
        rememberNextTurnIntent()
    }
    private func rememberNextTurnIntent() {
        guard let intentConversationID else { return }
        nextTurnIntents[intentConversationID] = NextTurnIntent(runtimeID: nextTurnRuntimeID, modelRecordKey: nextTurnModelRecordKey)
    }
    /// Each conversation keeps its own next-turn choice. A conversation opened
    /// for the first time continues on its current runtime and model; without a
    /// recorded model, the most recently chosen compatible one.
    private func restoreNextTurnIntent(for value: ConversationSnapshot) {
        guard value.conversation.id != intentConversationID else { return }
        intentConversationID = value.conversation.id
        let intent = nextTurnIntents[value.conversation.id] ?? NextTurnIntent(runtimeID: value.contextRuntimeID, modelRecordKey: value.modelRecordKey)
        if let runtimeID = intent.runtimeID, enabledRuntimeInstances.contains(where: { $0.id == runtimeID }) {
            hasInitializedNextTurnIntent = true
            nextTurnRuntimeID = runtimeID
        }
        let compatible = Set(runtimeCompatibleModels.map(\.recordKey))
        nextTurnModelRecordKey = [intent.modelRecordKey, lastChosenModelRecordKey].compactMap { $0 }.first { compatible.contains($0) }
        rememberNextTurnIntent()
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
    private func resetProjection() { intentConversationID = nil; loadedPayloads.removeAll(); isDraft = false; problems.removeAll { $0.activityKey?.hasPrefix("poll:") == true }; generation += 1; snapshot = nil; projectionRuntimeID = nil; selectedConversationID = nil; pendingConversationID = nil; transcript.reset(); activity = nil }
    private func applyList(_ data: BindingConfigurationSnapshot) {
        let mapped = BindingMapping.configuration(data)
        if conversationBrowserPreferences == nil {
            conversationBrowserPreferences = data.conversationBrowserPreferences
            savedBrowserPreferences = data.conversationBrowserPreferences
        }
        outlinePresentation = data.outlinePresentation
        conversationBrowserGroupLimit = Int(data.conversationBrowserGroupLimit)
        conversations = mapped.conversations; selectedConversationIDs.formIntersection(conversations.map(\.id))
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
            hasInitializedNextTurnIntent = true
        } else if let nextTurnRuntimeID, !enabledRuntimeInstances.contains(where: { $0.id == nextTurnRuntimeID }) {
            self.nextTurnRuntimeID = nil; nextTurnModelRecordKey = nil
        }
        if nextTurnModelRecordKey != nil, !runtimeCompatibleModels.contains(where: { $0.recordKey == nextTurnModelRecordKey }) { nextTurnModelRecordKey = nil }
    }
    private func apply(_ value: ConversationSnapshot?, preserveSelection: Bool = false) {
        guard var value else { return }
        if value.conversation.id != snapshot?.conversation.id || value.contextRuntimeID != snapshot?.contextRuntimeID { loadedPayloads.removeAll() }
        value.messages = withLoadedPayloads(value.messages)
        let remainingProblems = problems.filter { !($0.activityKey?.hasPrefix("poll:") == true && $0.activityKey != "poll:" + value.contextRuntimeID) }
        if problems != remainingProblems { problems = remainingProblems }
        guard snapshot != value else { return }
        snapshot = value; projectionRuntimeID = value.contextRuntimeID; selectedConversationID = value.conversation.id; restoreNextTurnIntent(for: value); if !preserveSelection && selectedConversationIDs.count <= 1 { selectedConversationIDs = [value.conversation.id] }; transcript.apply(value.messages, turns: value.transcriptTurns, items: value.transcriptItems, outline: value.transcriptOutline, identities: value.transcriptMessageIdentities)
        if let index = conversations.firstIndex(where: { $0.id == value.conversation.id }) { if conversations[index] != value.conversation { conversations[index] = value.conversation } }
        else if !isDraft { conversations.insert(value.conversation, at: 0) }
        activity = value.runState == .running ? "正在思考与执行" : value.runState == .stopping ? "正在停止" : nil
        if isPreview { previewSnapshots[value.conversation.id] = value }
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
        guard sharedApplication == nil, started, !isDraft, !isPreview, !isShuttingDown, snapshot != nil, !isLoading, !pollPending, let transport, let runtimeID = projectionRuntimeID else { return }
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
        runtimeTypes = [RuntimeTypeDescriptor(id: "sample-type", familyID: "sample", versionRegex: ".*", canRenameConversations: true, canDeleteConversations: true, supportedProtocols: [.chatCompletionsV1, .responsesV1, .messagesV1], supportedProviderProtocols: [.chatCompletionsV1, .responsesV1, .messagesV1], name: "示例运行时", fields: [])]
        runtimeInstances = [RuntimeInstance(id: "sample-instance", name: "示例运行时", typeID: "sample-type", gatewayID: gateway.id, settings: [:]), RuntimeInstance(id: "sample-review", name: "另一个运行时", typeID: "sample-type", gatewayID: gateway.id, settings: [:])]
        projectionRuntimeID = runtimeInstances[0].id; hasInitializedNextTurnIntent = true; nextTurnRuntimeID = projectionRuntimeID; nextTurnModelRecordKey = "sample-model"
        let topics = ["让设置页更安静", "整理一段代码", "下一步的项目计划"]
        let pairs = [
            ["我想让设置页面更简单，先帮我梳理一下。", "# 一次清晰的工作循环\n\n先整理任务，再执行。\n\n- [x] 读取上下文\n- [ ] 完成修改\n\n> 每一步都保留可以复核的结果。\n\n| 内容 | 状态 |\n| --- | --- |\n| 会话 | 可继续 |\n| 工具 | 已完成 |\n\n正文支持**强调**、[链接](https://example.invalid)和 `inline code`。", "运行时实例应该放哪里？", "工具栏显示当前运行时实例，详细配置放进系统设置窗口里。会话页面保留对话与模型选择，让注意力留在正在做的工作上。"],
            ["这个读取配置的函数可以再清晰一点吗？", "先把输入校验放在函数边界，主体就能直接表达意图。\n\n```rust\nfn load(path: &Path) -> Result<Config> {\n    let source = fs::read_to_string(path)?;\n    let config: Config = toml::from_str(&source)?;\n    config.validate()?;\n    Ok(config)\n}\n```", "还需要抽一个新的层吗？", "目前不需要。等第二个调用方出现不同的读取需求，再决定是否有值得共享的边界。"],
            ["帮我把接下来的工作排一下。", "先完成可用的会话界面，再检查配置是否真正驱动运行时。最后用一个短任务验证发送、停止和会话切换。", "先从哪个环节开始？", "从会话开始：打开旧会话、发送消息、检查回复，再新建一个会话。这个路径会暴露最直接的体验问题。"]
        ]
        for index in topics.indices {
            let conversation = Conversation(id: "sample-\(index)", title: topics[index], updatedAtUnixMs: Int64(Date().timeIntervalSince1970 * 1000) - Int64(index * 86_400_000), createdAtUnixMs: Int64(Date().timeIntervalSince1970 * 1000) - Int64((index + 3) * 86_400_000), runtimeID: runtimeInstances[index == 2 ? 1 : 0].id, cwd: index == 0 ? "/Users/example/Projects/Velune" : index == 1 ? "/Users/example/Projects/Sample" : nil)
            var history = pairs[index].enumerated().map { Message(id: "sample-\(index)-\($0.offset)", role: $0.offset.isMultiple(of: 2) ? .user : .assistant, blocks: [.text($0.element)]) }
            history[1].blocks.insert(.reasoning(.inline("先确认上下文，再选择最小的修改范围。")), at: 0)
            history[1].blocks.append(.tool(id: "sample-tool-\(index)", title: index == 1 ? "检查项目代码" : "读取工作上下文", state: .completed, output: .inline(index == 1 ? "已读取 3 个文件，未修改项目。" : "已梳理当前任务的上下文。")))
            if index == 2 { history[history.count - 1].blocks.append(.tool(id: "sample-progress", title: "整理项目计划", state: .running, output: .inline("正在整理计划。"))) }
            previewSnapshots[conversation.id] = ConversationSnapshot(revision: 1, conversation: conversation, contextRuntimeID: conversation.runtimeID, modelRecordKey: models[0].id, runState: .idle, messages: history,
                transcriptTurns: [
                    TranscriptTurn(id: "preview-work-" + conversation.id, userMessageID: history[0].id, workMessageIDs: [history[1].id], lastMessageID: nil, durationMs: nil, isRunning: false),
                    TranscriptTurn(id: "preview-answer-" + conversation.id, userMessageID: history[2].id, workMessageIDs: [], lastMessageID: history[3].id, durationMs: nil, isRunning: false)
                ],
                transcriptItems: [.message(id: history[0].id, messageID: history[0].id), .work(turnID: "preview-work-" + conversation.id), .message(id: history[2].id, messageID: history[2].id), .message(id: history[3].id, messageID: history[3].id)] + history.dropFirst(4).map { .message(id: $0.id, messageID: $0.id) },
                transcriptOutline: history.filter { $0.role == .user }.map { TranscriptOutlineEntry(id: $0.id, messageID: $0.id) },
                transcriptMessageIdentities: history.map { TranscriptMessageIdentity(id: $0.id, messageID: $0.id) },
                actions: ConversationActions(canSend: true, canCancel: false, canSwitch: true))
            conversations.append(conversation)
        }
        apply(previewSnapshots["sample-0"])
    }
}
