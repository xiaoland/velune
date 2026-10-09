import Combine
import Foundation
import AppKit
import VeluneBindings

@MainActor
final class AppStore: ObservableObject {
    @Published private(set) var conversationBrowserPreferences: BindingConversationBrowserPreferences?
    private var savedBrowserPreferences: BindingConversationBrowserPreferences?
    private var browserPreferencesGeneration: UInt64 = 0
    @Published private(set) var transcriptPresentation: BindingTranscriptPresentation = .conversation
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
    @Published var showsNewConversation = false
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
    private var authenticationPollPending = false
    private var hasGateway = false
    private var hasInitializedNextTurnIntent = false
    private var queuedConversationManagement: (() -> Void)?
    private var previewSnapshots: [String: ConversationSnapshot] = [:]

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
        gateway = application.gateway; hasGateway = application.hasGateway
        runtimeInstances = application.runtimeInstances; runtimeTypes = application.runtimeTypes
        protocols = application.protocols; modelTemplates = application.modelTemplates
        providerImportTypes = application.providerImportTypes
        conversationBrowserPreferences = application.conversationBrowserPreferences
        savedBrowserPreferences = application.savedBrowserPreferences
        conversationBrowserGroupLimit = application.conversationBrowserGroupLimit
        transcriptPresentation = application.transcriptPresentation
        conversations = application.conversations
        if let current = snapshot?.conversation, !isDraft, !conversations.contains(where: { $0.id == current.id }) { conversations.insert(current, at: 0) }
        authenticationRunning = application.authenticationRunning
        authenticationPrompt = application.authenticationPrompt
        authenticationNotifications = application.authenticationNotifications
        authenticationResult = application.authenticationResult
        problems = application.problems
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
        if !runtimeCompatibleModels.contains(where: { $0.recordKey == nextTurnModelRecordKey }) { nextTurnModelRecordKey = nil }
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
    func setTranscriptPresentation(_ presentation: BindingTranscriptPresentation) {
        if isPreview { transcriptPresentation = presentation; return }
        guard let transport else { recordProblem("本地核心未配置"); return }
        enqueue({ try transport.setTranscriptPresentation(presentation) }) { [weak self] in self?.transcriptPresentation = $0 }
    }
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
    func deleteConversations(_ targets: [Conversation]) {
        guard canManageConversations else { recordProblem("请等待正在进行的操作结束，再修改会话"); return }
        guard !targets.isEmpty, targets.allSatisfy(canDeleteConversation) else { recordProblem("所选会话中有原生会话当前无法删除"); return }
        if isPreview {
            let ids = Set(targets.map(\.id))
            conversations.removeAll { ids.contains($0.id) }; selectedConversationIDs.subtract(ids)
            for id in ids { previewSnapshots.removeValue(forKey: id) }
            if let loadedConversationID, ids.contains(loadedConversationID) { resetProjection() }
            return
        }
        guard let transport else { recordProblem("本地核心未配置"); return }
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
                if let loadedConversationID, result.1.contains(loadedConversationID) { resetProjection() }
                else if result.4 { resetProjection() }
                else if let refreshed = result.3 { applySnapshotResult(refreshed) }
                finishConversationManagement()
                appendProblems(result.2)
            }
        }
    }
    func createConversation() { showsNewConversation = true }
    func createConversation(runtimeID: String, cwd: String?, modelRecordKey: String, onCreated: @escaping () -> Void) {
        hasInitializedNextTurnIntent = true
        nextTurnRuntimeID = runtimeID; nextTurnModelRecordKey = modelRecordKey
        guard !isLoading, !applicationIsGenerating else { return }
        if isPreview {
            let conversation = Conversation(id: UUID().uuidString, title: "新会话", updatedAtUnixMs: Int64(Date().timeIntervalSince1970 * 1000), runtimeID: runtimeID, cwd: cwd)
            projectionRuntimeID = runtimeID
            apply(ConversationSnapshot(revision: 1, conversation: conversation, contextRuntimeID: runtimeID, modelRecordKey: modelRecordKey, runState: .idle, messages: [], actions: ConversationActions(canSend: true, canCancel: false, canSwitch: true)))
            onCreated(); return
        }
        isDraft = true
        let conversation = Conversation(id: "draft:" + UUID().uuidString, title: "新会话", updatedAtUnixMs: nil, runtimeID: runtimeID, cwd: cwd, canRename: false, canDelete: false)
        apply(ConversationSnapshot(revision: 1, conversation: conversation, contextRuntimeID: runtimeID, modelRecordKey: modelRecordKey, runState: .idle, messages: [], actions: ConversationActions(canSend: true, canCancel: false, canSwitch: true)))
        onCreated()
    }
    func send(text: String, onAccepted: (() -> Void)? = nil) {
        guard !text.trimmingCharacters(in: .whitespacesAndNewlines).isEmpty else { return }
        guard canSend, let target = nextTurnRuntimeID, let model = nextTurnModelRecordKey, let source = snapshot?.conversation.runtimeID, let conversationID = loadedConversationID else { return }
        if isPreview { snapshot?.modelRecordKey = model; snapshot?.messages.append(Message(id: UUID().uuidString, role: .user, blocks: [.text(text)])); apply(snapshot); onAccepted?(); return }
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
                        self.sharedApplication?.applySnapshotResult(created)
                        self.applySnapshotResult(created)
                    }
                }
            }
            return try transport.sendTurn(sourceRuntimeID: source, conversationID: conversationID, targetRuntimeID: target, modelRecordKey: model, text: text)
        }, onAccepted: onAccepted, onFailure: { _ in application.executionStarting = false }) { [weak self] value in
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
    private func resetProjection() { isDraft = false; problems.removeAll { $0.activityKey?.hasPrefix("poll:") == true }; generation += 1; snapshot = nil; projectionRuntimeID = nil; selectedConversationID = nil; pendingConversationID = nil; transcript.reset(); activity = nil }
    private func applyList(_ data: BindingConfigurationSnapshot) {
        let mapped = BindingMapping.configuration(data)
        if conversationBrowserPreferences == nil {
            conversationBrowserPreferences = data.conversationBrowserPreferences
            savedBrowserPreferences = data.conversationBrowserPreferences
        }
        transcriptPresentation = data.transcriptPresentation
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
    private func clearActivityProblem(_ key: String) { if let sharedApplication { sharedApplication.clearActivityProblem(key); return }; problems.removeAll { $0.activityKey == key } }
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
        if !runtimeCompatibleModels.contains(where: { $0.recordKey == nextTurnModelRecordKey }) { nextTurnModelRecordKey = nil }
    }
    private func apply(_ value: ConversationSnapshot?, preserveSelection: Bool = false) {
        guard let value else { return }
        problems.removeAll { $0.activityKey?.hasPrefix("poll:") == true && $0.activityKey != "poll:" + value.contextRuntimeID }
        snapshot = value; projectionRuntimeID = value.contextRuntimeID; selectedConversationID = value.conversation.id; if !preserveSelection && selectedConversationIDs.count <= 1 { selectedConversationIDs = [value.conversation.id] }; transcript.apply(value.messages, turns: value.transcriptTurns, confirmations: value.messageIdentityConfirmations)
        if let index = conversations.firstIndex(where: { $0.id == value.conversation.id }) { if conversations[index] != value.conversation { conversations[index] = value.conversation } }
        else if !isDraft { conversations.insert(value.conversation, at: 0) }
        activity = value.runState == .running ? "正在思考与执行" : value.runState == .stopping ? "正在停止" : nil
        if isPreview { previewSnapshots[value.conversation.id] = value }
    }
    private func applySnapshotResult(_ value: BindingSnapshotResult, unchangedPoll: Bool = false) {
        guard let incoming = value.snapshot else { resetProjection(); return }
        // A revision is only compared inside the currently selected projection.
        // Preparation/open reset that projection; repeated idle polling does no UI work.
        if unchangedPoll, let current = snapshot, incoming.conversation.id == current.conversation.id,
           incoming.revision == current.revision { return }
        apply(BindingMapping.snapshot(incoming), preserveSelection: unchangedPoll)
    }
    private func poll() {
        guard sharedApplication == nil, started, !isDraft, !isPreview, !isShuttingDown, snapshot != nil, !isLoading, !pollPending, let transport, let runtimeID = projectionRuntimeID else { return }
        let revision = generation
        let selected = selectedConversationID
        pollPending = true
        queue.async { [weak self] in
            let result = Result { try transport.snapshot(runtimeID: runtimeID, conversationID: selected ?? "") }
            DispatchQueue.main.async {
                guard let self else { return }
                self.pollPending = false
                guard self.generation == revision, self.selectedConversationID == selected else { return }
                switch result {
                case .success(let data): self.clearActivityProblem("poll:" + runtimeID); if data.snapshot != nil { self.applySnapshotResult(data, unchangedPoll: true) }
                case .failure(let failure): self.recordProblem(failure, source: "读取会话", activityKey: "poll:" + runtimeID)
                }
            }
        }
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
            history[1].blocks.insert(.reasoning("先确认上下文，再选择最小的修改范围。"), at: 0)
            history[1].blocks.append(.tool(id: "sample-tool-\(index)", title: index == 1 ? "检查项目代码" : "读取工作上下文", state: .completed, output: index == 1 ? "已读取 3 个文件，未修改项目。" : "已梳理当前任务的上下文。"))
            if index == 2 { history[history.count - 1].blocks.append(.tool(id: "sample-progress", title: "整理项目计划", state: .running, output: "正在整理计划。")) }
            previewSnapshots[conversation.id] = ConversationSnapshot(revision: 1, conversation: conversation, contextRuntimeID: conversation.runtimeID, modelRecordKey: models[0].id, runState: .idle, messages: history, actions: ConversationActions(canSend: true, canCancel: false, canSwitch: true))
            conversations.append(conversation)
        }
        apply(previewSnapshots["sample-0"])
    }
}
