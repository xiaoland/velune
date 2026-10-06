import Combine
import Foundation
import AppKit
import VeluneBindings

@MainActor
final class AppStore: ObservableObject {
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
    @Published private(set) var historyFailures: [HistoryFailure] = []
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
    @Published private(set) var error: String?
    @Published private var snapshot: ConversationSnapshot?

    private let transport: Transport?
    let isPreview: Bool
    private let queue = DispatchQueue(label: "local.velune.requests", qos: .userInitiated)
    private var generation = 0
    private var timer: Timer?
    private var pollPending = false
    private var authenticationPollPending = false
    private var hasGateway = false
    private var hasInitializedNextTurnIntent = false
    private var queuedConversationManagement: (() -> Void)?
    private var previewSnapshots: [String: ConversationSnapshot] = [:]

    init(transport: Transport? = nil, preview: Bool = false) {
        isPreview = preview
        if preview { self.transport = nil }
        else if let transport { self.transport = transport }
        else {
            do { self.transport = try Transport.applicationDefault() }
            catch { self.transport = nil; self.error = error.localizedDescription }
        }
        if preview { seedPreview() }
    }
    var models: [ModelChoice] { gateway.providers.flatMap { provider in provider.models.map { ModelChoice(recordKey: $0.recordKey, displayName: "\($0.displayName) · \(provider.name)", protocolID: provider.protocolID) } } }
    var enabledRuntimeInstances: [RuntimeInstance] { runtimeInstances.filter(\.enabled) }
    var providers: [AIProvider] { gateway.providers }
    var loadedConversationID: String? { snapshot?.conversation.id }
    var selectedConversationTitle: String? { conversations.first { $0.id == selectedConversationID }?.title }
    var isGenerating: Bool { snapshot?.runState == .running || snapshot?.runState == .stopping }
    var isBusy: Bool { isLoading || isGenerating || isShuttingDown || authenticationRunning }
    var canManageConversations: Bool { !isGenerating && !authenticationRunning && !isShuttingDown && conversationManagementStatus == nil && (!isLoading || pendingConversationID != nil) }
    var canSend: Bool { !authenticationRunning && !isLoading && !isGenerating && !isShuttingDown && (snapshot?.actions.canSend ?? false) && executionBoundaryMessage == nil && nextTurnModelRecordKey != nil && runtimeCompatibleModels.contains { $0.recordKey == nextTurnModelRecordKey } }
    var canCancel: Bool { snapshot?.actions.canCancel ?? false }
    var canSwitchModel: Bool { nextTurnRuntimeID != nil && !isShuttingDown }
    var needsModelSelection: Bool { snapshot != nil && nextTurnModelRecordKey == nil }
    var selectedModelName: String? { models.first { $0.recordKey == nextTurnModelRecordKey }?.displayName }
    var runtimeCompatibleModels: [ModelChoice] {
        let typeID = runtimeInstances.first { $0.id == nextTurnRuntimeID }?.typeID
        guard let descriptor = runtimeTypes.first(where: { $0.id == typeID }) else { return [] }
        return models.filter { descriptor.supportedProtocols.contains($0.protocolID) }
    }
    var executionBoundaryMessage: String? {
        guard let source = snapshot?.conversation.runtimeID, let target = nextTurnRuntimeID, source != target else { return nil }
        return "历史来自另一运行时；跨运行时继续尚未接入，请选择来源运行时以继续此会话。"
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
                    completion(true)
                case .failure(let failure):
                    self.error = failure.localizedDescription
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
    func start() {
        guard !isPreview, let transport else { return }
        enqueue({ try transport.list() }) { [weak self] data in
            guard let self else { return }
            applyList(data)
            schedulePolling()
            if let id = conversations.first?.id { selectConversation(id: id) }
        }
    }
    func setConversationBrowserGroupLimit(_ limit: Int) {
        guard let value = UInt32(exactly: limit), value > 0 else { error = "每组展示数量须为正整数"; return }
        if isPreview { conversationBrowserGroupLimit = limit; return }
        guard let transport else { error = "本地核心未配置"; return }
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
        guard let transport else { error = "本地核心未配置"; return }
        // The sidebar reflects user intent immediately. The old loaded transcript
        // is retained until success, but hidden while this destination is loading.
        let previousSelection = snapshot?.conversation.id
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
        runtimeTypes.first { $0.id == runtimeInstances.first(where: { $0.id == conversation.runtimeID })?.typeID }?.canRenameConversations == true
    }
    func canDeleteConversation(_ conversation: Conversation) -> Bool {
        runtimeTypes.first { $0.id == runtimeInstances.first(where: { $0.id == conversation.runtimeID })?.typeID }?.canDeleteConversations == true
    }
    func renameConversation(_ conversation: Conversation, title: String, onRenamed: @escaping () -> Void) {
        guard canManageConversations else { error = "请等待正在进行的操作结束，再修改会话"; return }
        guard canRenameConversation(conversation) else { error = "此运行时适配器尚未接入原生会话重命名"; return }
        let name = title.trimmingCharacters(in: .whitespacesAndNewlines)
        guard !name.isEmpty else { error = "会话名称不能为空"; return }
        if isPreview {
            if let index = conversations.firstIndex(where: { $0.id == conversation.id }) { conversations[index].title = name }
            previewSnapshots[conversation.id]?.conversation.title = name
            if snapshot?.conversation.id == conversation.id { snapshot?.conversation.title = name }
            onRenamed(); return
        }
        guard let transport else { error = "本地核心未配置"; return }
        scheduleConversationManagement("重命名“\(conversation.title)”") { [weak self] in
            guard let self else { return }
            generation += 1
            conversationManagementStatus = "正在重命名“\(conversation.title)”…"
            enqueue({ try transport.renameConversation(runtimeID: conversation.runtimeID, conversationID: conversation.id, title: name) }, onFailure: { [weak self] _ in self?.finishConversationManagement() }, preserveError: pendingConversationID != nil) { [weak self] data in
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
        guard canManageConversations else { error = "请等待正在进行的操作结束，再修改会话"; return }
        guard !targets.isEmpty, targets.allSatisfy(canDeleteConversation) else { error = "所选会话中有运行时适配器尚未接入原生删除"; return }
        if isPreview {
            let ids = Set(targets.map(\.id))
            conversations.removeAll { ids.contains($0.id) }; selectedConversationIDs.subtract(ids)
            for id in ids { previewSnapshots.removeValue(forKey: id) }
            if let loadedConversationID, ids.contains(loadedConversationID) { resetProjection() }
            return
        }
        guard let transport else { error = "本地核心未配置"; return }
        scheduleConversationManagement("删除 \(targets.count) 个会话") { [weak self] in
            guard let self else { return }
            generation += 1
            conversationManagementStatus = "正在删除 \(targets.count) 个会话…"
            // Runtime-owned deletions are independent operations. Preserve each
            // success and report failures rather than promising a transaction.
            enqueue({
                var latest: BindingConfigurationSnapshot?
                var deleted: Set<String> = []
                var failures: [String] = []
                for target in targets {
                    do { latest = try transport.deleteConversation(runtimeID: target.runtimeID, conversationID: target.id); deleted.insert(target.id) }
                    catch { failures.append("\(target.title)：\(error.localizedDescription)") }
                }
                return (latest, deleted, failures)
            }, preserveError: pendingConversationID != nil) { [weak self] result in
                guard let self else { return }
                if let data = result.0 { applyList(data) }
                selectedConversationIDs.subtract(result.1)
                if let loadedConversationID, result.1.contains(loadedConversationID) { resetProjection() }
                finishConversationManagement()
                if !result.2.isEmpty { error = "\(result.1.count) 个已删除，\(result.2.count) 个未删除：\n" + result.2.joined(separator: "\n") }
            }
        }
    }
    func createConversation() { showsNewConversation = true }
    func createConversation(runtimeID: String, cwd: String, modelRecordKey: String, onCreated: @escaping () -> Void) {
        hasInitializedNextTurnIntent = true
        nextTurnRuntimeID = runtimeID; nextTurnModelRecordKey = modelRecordKey
        guard !isLoading, !isGenerating else { return }
        if isPreview {
            let conversation = Conversation(id: UUID().uuidString, title: "新会话", updatedAtUnixMs: Int64(Date().timeIntervalSince1970 * 1000), runtimeID: runtimeID, cwd: cwd)
            projectionRuntimeID = runtimeID
            apply(ConversationSnapshot(revision: 1, conversation: conversation, modelRecordKey: modelRecordKey, runState: .idle, messages: [], actions: ConversationActions(canSend: true, canCancel: false, canSwitch: true)))
            onCreated(); return
        }
        guard let transport else { error = "本地核心未配置"; return }
        enqueue({ try transport.createConversation(runtimeID: runtimeID, cwd: cwd, modelRecordKey: modelRecordKey) }) { [weak self] in
            self?.generation += 1; self?.projectionRuntimeID = runtimeID; self?.applySnapshotResult($0); onCreated()
        }
    }
    func send(text: String, onAccepted: (() -> Void)? = nil) {
        guard !text.trimmingCharacters(in: .whitespacesAndNewlines).isEmpty else { return }
        if let boundary = executionBoundaryMessage { error = boundary; return }
        guard canSend, let target = nextTurnRuntimeID, let model = nextTurnModelRecordKey else { return }
        if isPreview { snapshot?.modelRecordKey = model; snapshot?.messages.append(Message(id: UUID().uuidString, role: .user, blocks: [.text(text)])); apply(snapshot); onAccepted?(); return }
        guard let transport else { error = "本地核心未配置"; return }
        enqueue({ try transport.sendTurn(runtimeID: target, modelRecordKey: model, text: text) }, onAccepted: onAccepted) { [weak self] in self?.applySnapshotResult($0) }
    }
    func cancel() {
        guard canCancel, !isLoading else { return }
        if isPreview { snapshot?.runState = .idle; snapshot?.actions.canCancel = false; apply(snapshot); return }
        guard let runtimeID = projectionRuntimeID else { error = "请选择运行时实例"; return }
        guard let transport else { error = "本地核心未配置"; return }
        enqueue({ try transport.cancel(runtimeID: runtimeID) }) { [weak self] in self?.applySnapshotResult($0) }
    }
    func saveProvider(_ value: AIProvider, authenticationEdit: AuthenticationEdit, onSaved: (() -> Void)? = nil) {
        guard !authenticationRunning, !isGenerating else { error = "请先停止当前任务或完成登录"; return }
        if isPreview { var saved = value; if saved.id.isEmpty { saved.id = UUID().uuidString }; for index in saved.models.indices where saved.models[index].recordKey.isEmpty { saved.models[index].recordKey = UUID().uuidString }; gateway.providers.removeAll { $0.id == saved.id }; gateway.providers.append(saved); hasGateway = true; onSaved?(); return }
        guard let transport else { error = "本地核心未配置"; return }
        enqueue({ try transport.saveProvider(gatewayID: self.gateway.id, provider: value, authenticationEdit: authenticationEdit) }) { [weak self] data in
            self?.applyGatewayUpdate(data); onSaved?()
        }
    }
    func readProviderAPIKey(_ providerID: String, completion: @escaping (String) -> Void) {
        guard !isPreview, let transport else { error = "预览不会读取 API key"; return }
        enqueue({ try transport.readProviderAPIKey(gatewayID: self.gateway.id, providerID: providerID) }, apply: completion)
    }
    func saveTemplate(_ value: ModelTemplate, onSaved: (() -> Void)? = nil) {
        if isPreview { var saved = value; if saved.id.isEmpty { saved.id = UUID().uuidString }; modelTemplates.removeAll { $0.id == saved.id }; modelTemplates.append(saved); onSaved?(); return }
        guard let transport else { error = "本地核心未配置"; return }
        enqueue({ try transport.saveTemplate(value) }) { [weak self] result in self?.modelTemplates = result.map(BindingMapping.template); onSaved?() }
    }
    func saveTemplates(_ values: [ModelTemplate], onSaved: @escaping (Int) -> Void) {
        guard !values.isEmpty else { return }
        if isPreview { for value in values { saveTemplate(value) }; onSaved(values.count); return }
        guard let transport else { error = "本地核心未配置"; return }
        enqueue({
            var templates: [BindingModelTemplate]?
            var count = 0
            var failures: [String] = []
            for value in values {
                do { templates = try transport.saveTemplate(value); count += 1 }
                catch { failures.append("\(value.name)：\(error.localizedDescription)") }
            }
            return (templates, count, failures)
        }) { [weak self] result in
            if let templates = result.0 { self?.modelTemplates = templates.map(BindingMapping.template) }
            if !result.2.isEmpty { self?.error = "\(result.1) 个已添加，\(result.2.count) 个未添加：\n" + result.2.joined(separator: "\n") }
            onSaved(result.1)
        }
    }
    func fetchPublicModelCatalog(completion: @escaping ([CatalogModel]) -> Void) {
        guard !isPreview, let transport else { error = "预览不会访问公开目录"; return }
        enqueue({ try transport.publicModelCatalog() }) { values in
            completion(values.map { CatalogModel(sourceProviderID: $0.sourceProviderId, sourceProviderName: $0.sourceProviderName, modelID: $0.modelId, name: $0.name, contextWindow: $0.contextWindow, maxOutputTokens: $0.maxOutputTokens, reasoningLevels: $0.reasoningLevels) })
        }
    }
    func deleteTemplate(_ id: String) {
        if isPreview { modelTemplates.removeAll { $0.id == id }; return }
        guard let transport else { error = "本地核心未配置"; return }
        enqueue({ try transport.deleteTemplate(id: id) }) { [weak self] result in self?.modelTemplates = result.map(BindingMapping.template) }
    }
    func previewProviderImport(_ source: ProviderImportSource, completion: @escaping (ProviderImportPreview) -> Void) {
        guard !isPreview, !isGenerating, !authenticationRunning else { error = "请先停止任务或完成登录，再读取提供商配置"; return }
        guard let transport else { error = "本地核心未配置"; return }
        enqueue({ try transport.providerImportPreview(gatewayID: self.gateway.id, source: BindingMapping.bindingImportSource(source)) }) { completion(BindingMapping.importPreview($0).preview) }
    }
    func applyProviderImport(_ source: ProviderImportSource, preview: ProviderImportPreview, selections: [ProviderImportSelection], replaceExisting: Bool, completion: @escaping () -> Void) {
        guard !isPreview, !isGenerating, !authenticationRunning else { error = "请先停止任务或完成登录，再导入提供商配置"; return }
        guard let transport else { error = "本地核心未配置"; return }
        enqueue({ try transport.providerImportApply(gatewayID: self.gateway.id, source: BindingMapping.bindingImportSource(source), previewToken: preview.token, selections: selections.map(BindingMapping.bindingSelection), replaceExisting: replaceExisting) }) { [weak self] data in
                guard let self else { return }
                if let saved = data.gateways.first(where: { $0.id == self.gateway.id }) { self.gateway = BindingMapping.gateway(saved); self.hasGateway = true }
                if data.executionInvalidated { refreshAfterExecutionInvalidation() }
                if !data.executionInvalidated { enqueue({ try transport.list() }) { [weak self] in self?.applyList($0) } }
                completion()
            }
    }
    func startAuthentication(_ providerID: String) {
        guard !isGenerating else { error = "请先停止当前任务，再开始登录"; return }
        guard !isPreview else { error = "预览不会启动认证"; return }
        guard let transport else { error = "本地核心未配置"; return }
        enqueue({ try transport.authenticationStart(gatewayID: self.gateway.id, providerID: providerID) }) { [weak self] data in
                guard let self else { return }
                authenticationPrompt = nil; authenticationNotifications = []; authenticationResult = nil
                showsAuthentication = true; applyAuthentication(BindingMapping.authenticationProgress(data))
            }
    }
    func answerAuthentication(id: String, value: String) {
        guard let transport else { error = "本地核心未配置"; return }
        enqueue({ try transport.authenticationReply(promptID: id, value: value) }) { [weak self] in self?.authenticationPrompt = nil; self?.applyAuthentication(BindingMapping.authenticationProgress($0)) }
    }
    func cancelAuthentication() {
        guard let transport else { error = "本地核心未配置"; return }
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
                case .success(let data): self.applyAuthentication(BindingMapping.authenticationProgress(data))
                case .failure(let failure): self.error = failure.localizedDescription
                }
            }
        }
    }
    private func applyAuthentication(_ data: AuthenticationData) {
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
                authenticationResult = event.ok == true ? "登录完成，认证已保存在原来源。" : event.cancelled == true ? "登录已取消。" : event.error ?? "登录未完成。"
            }
        }
    }
    func deleteProvider(id: String) {
        if isPreview { gateway.providers.removeAll { $0.id == id }; return }
        guard let transport else { error = "本地核心未配置"; return }
        enqueue({ try transport.deleteProvider(gatewayID: self.gateway.id, providerID: id) }) { [weak self] in self?.applyGatewayUpdate($0) }
    }
    private func applyGatewayUpdate(_ data: BindingGatewayUpdate) {
        if let saved = data.gateways.first(where: { $0.id == gateway.id }) { gateway = BindingMapping.gateway(saved); hasGateway = true }
        reconcileNextTurnIntent()
        if data.executionInvalidated { refreshAfterExecutionInvalidation() }
    }
    func runtimeDiscoveryHints(userHome: String, overrides: [String: String], onFailure: (() -> Void)? = nil, completion: @escaping ([BindingRuntimeDiscoveryHint]) -> Void) {
        guard let transport else { error = "本地核心未配置"; return }
        enqueue({ try transport.runtimeDiscoveryHints(userHome: userHome, overrides: overrides) }, onFailure: { _ in onFailure?() }, apply: completion)
    }
    func discoverRuntimes(_ probes: [BindingRuntimeDiscoveryProbe], onFailure: (() -> Void)? = nil, completion: @escaping ([BindingRuntimeDiscoveryCandidate]) -> Void) {
        guard let transport else { error = "本地核心未配置"; return }
        enqueue({ try transport.discoverRuntimes(probes) }, onFailure: { _ in onFailure?() }, apply: completion)
    }
    func importRuntimes(_ candidates: [BindingRuntimeDiscoveryCandidate], completion: @escaping (Set<String>) -> Void) {
        guard !isBusy, !candidates.isEmpty, candidates.allSatisfy({ $0.supported && !$0.alreadyConfigured }) else { error = "请仅选择可导入且尚未配置的运行时"; return }
        guard let transport else { error = "本地核心未配置"; return }
        let gatewayID = gateway.id
        enqueue({
            var imported: Set<String> = []
            var failures: [String] = []
            for candidate in candidates {
                var runtime = BindingMapping.runtime(candidate.runtime); runtime.gatewayID = gatewayID
                do { _ = try transport.upsertRuntime(BindingMapping.bindingRuntime(runtime)); imported.insert(runtime.id) }
                catch { failures.append("\(runtime.name)：\(error.localizedDescription)") }
            }
            return (try transport.list(), imported, failures)
        }) { [weak self] result in
            self?.applyList(result.0)
            if !result.2.isEmpty { self?.error = "\(result.1.count) 个已导入，\(result.2.count) 个未导入：\n" + result.2.joined(separator: "\n") }
            completion(result.1)
        }
    }
    func saveRuntimeInstance(_ instance: RuntimeInstance, onSaved: (() -> Void)? = nil) {
        guard !isGenerating else { error = "请先停止当前任务，再修改运行时实例"; return }
        if isPreview { runtimeInstances.removeAll { $0.id == instance.id }; runtimeInstances.append(instance); if !instance.enabled && projectionRuntimeID == instance.id { resetProjection(); projectionRuntimeID = nil }; onSaved?(); return }
        guard let transport else { error = "本地核心未配置"; return }
        enqueue({ try transport.upsertRuntime(BindingMapping.bindingRuntime(instance)) }) { [weak self] data in
            guard let self else { return }
            runtimeInstances = data.runtimeInstances.map(BindingMapping.runtime); runtimeTypes = data.runtimeTypes.map(BindingMapping.runtimeType)
            if data.executionInvalidated { refreshAfterExecutionInvalidation() }
            else { enqueue({ try transport.list() }) { [weak self] in self?.applyList($0) } }
            if data.runtimeInstances.contains(where: { $0.id == instance.id }) { onSaved?() }
            else { error = "核心未返回保存后的运行时实例" }
        }
    }
    func deleteRuntimeInstance(id: String) {
        if isPreview {
            runtimeInstances.removeAll { $0.id == id }
            if projectionRuntimeID == id { resetProjection(); projectionRuntimeID = nil }
            return
        }
        guard let transport else { error = "本地核心未配置"; return }
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
        guard let transport else { error = "本地核心未配置"; return }
        enqueue({ try transport.list() }) { [weak self] data in
            guard let self else { return }; applyList(data)
            if let runtimeID = snapshot?.conversation.runtimeID {
                enqueue({ try transport.snapshot(runtimeID: runtimeID) }) { [weak self] in self?.applySnapshotResult($0) }
            } else { resetProjection() }
        }
    }
    private func resetProjection() { generation += 1; snapshot = nil; projectionRuntimeID = nil; selectedConversationID = nil; pendingConversationID = nil; transcript.reset(); activity = nil }
    private func applyList(_ data: BindingConfigurationSnapshot) {
        let mapped = BindingMapping.configuration(data)
        conversationBrowserGroupLimit = Int(data.conversationBrowserGroupLimit)
        conversations = mapped.conversations; selectedConversationIDs.formIntersection(conversations.map(\.id)); historyFailures = mapped.historyFailures
        if let saved = mapped.gateways.first(where: { $0.id == gateway.id }) ?? mapped.gateways.first { gateway = saved; hasGateway = true }
        runtimeInstances = mapped.runtimes; runtimeTypes = mapped.runtimeTypes
        protocols = mapped.protocols
        modelTemplates = mapped.modelTemplates
        providerImportTypes = mapped.importTypes
        if !hasInitializedNextTurnIntent { nextTurnRuntimeID = mapped.selectedRuntimeID }
        reconcileNextTurnIntent()
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
        snapshot = value; projectionRuntimeID = value.conversation.runtimeID; selectedConversationID = value.conversation.id; if !preserveSelection && selectedConversationIDs.count <= 1 { selectedConversationIDs = [value.conversation.id] }; transcript.apply(value.messages)
        if let index = conversations.firstIndex(where: { $0.id == value.conversation.id }) { if conversations[index] != value.conversation { conversations[index] = value.conversation } }
        else { conversations.insert(value.conversation, at: 0) }
        activity = value.runState == .running ? "正在思考与执行" : value.runState == .stopping ? "正在停止" : nil
        if isPreview { previewSnapshots[value.conversation.id] = value }
    }
    private func applySnapshotResult(_ value: BindingSnapshotResult, unchangedPoll: Bool = false) {
        guard let incoming = value.snapshot else { return }
        // A revision is only compared inside the currently selected projection.
        // Preparation/open reset that projection; repeated idle polling does no UI work.
        if unchangedPoll, let current = snapshot, incoming.conversation.id == current.conversation.id,
           incoming.revision == current.revision { return }
        apply(BindingMapping.snapshot(incoming), preserveSelection: unchangedPoll)
    }
    private func poll() {
        guard !isPreview, !isShuttingDown, snapshot != nil, !isLoading, !pollPending, let transport, let runtimeID = projectionRuntimeID else { return }
        let revision = generation
        let selected = selectedConversationID
        pollPending = true
        queue.async { [weak self] in
            let result = Result { try transport.snapshot(runtimeID: runtimeID) }
            DispatchQueue.main.async {
                guard let self else { return }
                self.pollPending = false
                guard self.generation == revision, self.selectedConversationID == selected else { return }
                switch result {
                case .success(let data): self.applySnapshotResult(data, unchangedPoll: true)
                case .failure(let failure): self.error = failure.localizedDescription
                }
            }
        }
    }
    private func enqueue<T: Sendable>(_ operation: @escaping () throws -> T, onAccepted: (() -> Void)? = nil, onFailure: ((Error) -> Void)? = nil, preserveError: Bool = false, apply: @escaping (T) -> Void) {
        guard !isLoading, !isShuttingDown else { error = "请等待当前操作完成后重试"; return }
        guard transport != nil else { error = "本地核心未配置"; return }
        let revision = generation
        isLoading = true
        if !preserveError { error = nil }
        queue.async { [weak self] in
            let result = Result { try operation() }
            DispatchQueue.main.async {
                guard let self else { return }
                self.isLoading = false
                guard revision == self.generation else { return }
                switch result {
                case .success(let value): onAccepted?(); apply(value)
                case .failure(let failure): self.error = failure.localizedDescription; onFailure?(failure)
                }
            }
        }
    }
    private func seedPreview() {
        gateway = GatewayConfig(providers: [AIProvider(id: "sample-provider", name: "示例 AI 服务", protocolID: .chatCompletionsV1, endpoint: "https://example.invalid/v1", models: [ProviderModel(recordKey: "sample-model", providerModelID: "external-example", nickname: "通用模型", contextWindow: 8192, maxOutputTokens: 4096)])])
        hasGateway = true
        protocols = [ProtocolDescriptor(id: .chatCompletionsV1, name: "OpenAI Chat Completions v1", supported: true), ProtocolDescriptor(id: .responsesV1, name: "OpenAI Responses v1", supported: true), ProtocolDescriptor(id: .messagesV1, name: "Anthropic Messages v1", supported: true)]
        runtimeTypes = [RuntimeTypeDescriptor(id: "sample-type", familyID: "sample", versionRegex: ".*", canRenameConversations: true, canDeleteConversations: true, supportedProtocols: [.chatCompletionsV1, .responsesV1, .messagesV1], name: "示例运行时", fields: [])]
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
            previewSnapshots[conversation.id] = ConversationSnapshot(revision: 1, conversation: conversation, modelRecordKey: models[0].id, runState: .idle, messages: history, actions: ConversationActions(canSend: true, canCancel: false, canSwitch: true))
            conversations.append(conversation)
        }
        apply(previewSnapshots["sample-0"])
    }
}
