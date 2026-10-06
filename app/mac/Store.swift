import Combine
import Foundation
import AppKit
import VeluneBindings

@MainActor
final class AppStore: ObservableObject {
    @Published private(set) var conversations: [Conversation] = []
    @Published private(set) var selectedConversationID: String?
    @Published private(set) var pendingConversationID: String?
    @Published private(set) var selectedRuntimeID: String?
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
    var providers: [AIProvider] { gateway.providers }
    var loadedConversationID: String? { snapshot?.conversation.id }
    var selectedConversationTitle: String? { conversations.first { $0.id == selectedConversationID }?.title }
    var isGenerating: Bool { snapshot?.runState == .running || snapshot?.runState == .stopping }
    var isBusy: Bool { isLoading || isGenerating || isShuttingDown || authenticationRunning }
    var canSend: Bool { !authenticationRunning && !isLoading && !isShuttingDown && (snapshot?.actions.canSend ?? false) }
    var canCancel: Bool { snapshot?.actions.canCancel ?? false }
    var canSwitchModel: Bool { snapshot != nil && !isGenerating && !isLoading && !isShuttingDown }
    var needsModelSelection: Bool { snapshot != nil && snapshot?.modelRecordKey == nil }
    var selectedModelName: String? { models.first { $0.recordKey == snapshot?.modelRecordKey }?.displayName }
    var runtimeCompatibleModels: [ModelChoice] {
        let typeID = runtimeInstances.first { $0.id == selectedRuntimeID }?.typeID
        guard let descriptor = runtimeTypes.first(where: { $0.id == typeID }) else { return [] }
        return models.filter { descriptor.supportedProtocols.contains($0.protocolID) }
    }
    var pendingInteractions: [RuntimeInteraction] { snapshot?.pendingInteractions ?? [] }
    func replyInteraction(_ interaction: RuntimeInteraction, reply: RuntimeInteractionReply) {
        guard pendingInteractions.contains(where: { $0.id == interaction.id }), let runtimeID = selectedRuntimeID, let transport else { return }
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
    func selectConversation(id: String) {
        guard !isLoading, !isGenerating, let conversation = conversations.first(where: { $0.id == id }) else { return }
        if isPreview { selectedRuntimeID = conversation.runtimeID; apply(previewSnapshots[id]); return }
        guard let transport else { error = "本地核心未配置"; return }
        // The sidebar reflects user intent immediately. The old loaded transcript
        // is retained until success, but hidden while this destination is loading.
        let previousSelection = snapshot?.conversation.id
        generation += 1 // Invalidate any poll already queued for the old conversation.
        selectedConversationID = id
        pendingConversationID = id
        enqueue({ try transport.openConversation(runtimeID: conversation.runtimeID, conversationID: id) }, onFailure: { [weak self] _ in
            self?.pendingConversationID = nil
            self?.selectedConversationID = previousSelection
        }) { [weak self] in
            self?.applySnapshotResult($0)
            self?.pendingConversationID = nil
        }
    }
    func canRenameConversation(_ conversation: Conversation) -> Bool {
        runtimeTypes.first { $0.id == runtimeInstances.first(where: { $0.id == conversation.runtimeID })?.typeID }?.canRenameConversations == true
    }
    func canDeleteConversation(_ conversation: Conversation) -> Bool {
        runtimeTypes.first { $0.id == runtimeInstances.first(where: { $0.id == conversation.runtimeID })?.typeID }?.canDeleteConversations == true
    }
    func renameConversation(_ conversation: Conversation, title: String, onRenamed: @escaping () -> Void) {
        guard !isBusy, canRenameConversation(conversation) else { return }
        let name = title.trimmingCharacters(in: .whitespacesAndNewlines)
        guard !name.isEmpty else { error = "会话名称不能为空"; return }
        if isPreview {
            if let index = conversations.firstIndex(where: { $0.id == conversation.id }) { conversations[index].title = name }
            previewSnapshots[conversation.id]?.conversation.title = name
            if snapshot?.conversation.id == conversation.id { snapshot?.conversation.title = name }
            onRenamed(); return
        }
        guard let transport else { error = "本地核心未配置"; return }
        generation += 1
        enqueue({ try transport.renameConversation(runtimeID: conversation.runtimeID, conversationID: conversation.id, title: name) }) { [weak self] data in
            guard let self else { return }
            applyList(data)
            if snapshot?.conversation.id == conversation.id, let renamed = conversations.first(where: { $0.id == conversation.id }) { snapshot?.conversation = renamed }
            onRenamed()
        }
    }
    func deleteConversation(_ conversation: Conversation) {
        guard !isBusy, canDeleteConversation(conversation) else { return }
        if isPreview {
            conversations.removeAll { $0.id == conversation.id }; previewSnapshots.removeValue(forKey: conversation.id)
            if loadedConversationID == conversation.id { resetProjection() }
            return
        }
        guard let transport else { error = "本地核心未配置"; return }
        generation += 1
        enqueue({ try transport.deleteConversation(runtimeID: conversation.runtimeID, conversationID: conversation.id) }) { [weak self] data in
            guard let self else { return }
            applyList(data)
            if loadedConversationID == conversation.id { resetProjection() }
        }
    }
    func createConversation() { showsNewConversation = true }
    func createConversation(runtimeID: String, cwd: String, modelRecordKey: String, onCreated: @escaping () -> Void) {
        guard !isLoading, !isGenerating else { return }
        if isPreview {
            let conversation = Conversation(id: UUID().uuidString, title: "新会话", updatedAtUnixMs: Int64(Date().timeIntervalSince1970 * 1000), runtimeID: runtimeID, cwd: cwd)
            selectedRuntimeID = runtimeID
            apply(ConversationSnapshot(revision: 1, conversation: conversation, modelRecordKey: modelRecordKey, runState: .idle, messages: [], actions: ConversationActions(canSend: true, canCancel: false, canSwitch: true)))
            onCreated(); return
        }
        guard let transport else { error = "本地核心未配置"; return }
        enqueue({ try transport.createConversation(runtimeID: runtimeID, cwd: cwd, modelRecordKey: modelRecordKey) }) { [weak self] in
            self?.generation += 1; self?.selectedRuntimeID = runtimeID; self?.applySnapshotResult($0); onCreated()
        }
    }
    func send(text: String, onAccepted: (() -> Void)? = nil) {
        guard canSend, !text.trimmingCharacters(in: .whitespacesAndNewlines).isEmpty else { return }
        if isPreview { snapshot?.messages.append(Message(id: UUID().uuidString, role: .user, blocks: [.text(text)])); apply(snapshot); onAccepted?(); return }
        guard let runtimeID = selectedRuntimeID else { error = "请选择运行时实例"; return }
        guard let transport else { error = "本地核心未配置"; return }
        enqueue({ try transport.send(runtimeID: runtimeID, text: text) }, onAccepted: onAccepted) { [weak self] in self?.applySnapshotResult($0) }
    }
    func cancel() {
        guard canCancel, !isLoading else { return }
        if isPreview { snapshot?.runState = .idle; snapshot?.actions.canCancel = false; apply(snapshot); return }
        guard let runtimeID = selectedRuntimeID else { error = "请选择运行时实例"; return }
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
        if data.executionInvalidated { refreshAfterExecutionInvalidation() }
    }
    func saveRuntimeInstance(_ instance: RuntimeInstance, onSaved: (() -> Void)? = nil) {
        guard !isGenerating else { error = "请先停止当前任务，再修改运行时实例"; return }
        if isPreview { runtimeInstances.removeAll { $0.id == instance.id }; runtimeInstances.append(instance); onSaved?(); return }
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
            if selectedRuntimeID == id { resetProjection(); selectedRuntimeID = nil }
            return
        }
        guard let transport else { error = "本地核心未配置"; return }
        enqueue({ try transport.deleteRuntime(id: id) }) { [weak self] data in
            self?.runtimeInstances = data.runtimeInstances.map(BindingMapping.runtime); self?.runtimeTypes = data.runtimeTypes.map(BindingMapping.runtimeType)
            if data.executionInvalidated { self?.refreshAfterExecutionInvalidation() }
        }
    }
    func selectModel(modelRecordKey: String) {
        guard !isLoading, !isGenerating, models.contains(where: { $0.id == modelRecordKey }) else { return }
        if isPreview { snapshot?.modelRecordKey = modelRecordKey; apply(snapshot); return }
        guard let runtimeID = selectedRuntimeID else { error = "请先选择运行时实例"; return }
        guard let transport else { error = "本地核心未配置"; return }
        enqueue({ try transport.selectModel(runtimeID: runtimeID, modelRecordKey: modelRecordKey) }) { [weak self] in self?.applySnapshotResult($0) }
    }
    func selectRuntime(id: String) {
        guard !isLoading, !isGenerating else { return }
        if isPreview { resetProjection(); selectedRuntimeID = id; return }
        guard let transport else { error = "本地核心未配置"; return }
        enqueue({ try transport.selectRuntime(id: id) }) { [weak self] data in
            self?.resetProjection(); self?.applyList(data)
        }
    }
    private func refreshAfterExecutionInvalidation() {
        guard let transport else { error = "本地核心未配置"; return }
        enqueue({ try transport.list() }) { [weak self] data in
            guard let self else { return }; applyList(data)
            if let runtimeID = selectedRuntimeID, snapshot != nil {
                enqueue({ try transport.snapshot(runtimeID: runtimeID) }) { [weak self] in self?.applySnapshotResult($0) }
            } else if selectedRuntimeID == nil { resetProjection() }
        }
    }
    private func resetProjection() { generation += 1; snapshot = nil; selectedConversationID = nil; pendingConversationID = nil; transcript.reset(); activity = nil }
    private func applyList(_ data: BindingConfigurationSnapshot) {
        let mapped = BindingMapping.configuration(data)
        conversations = mapped.conversations; historyFailures = mapped.historyFailures
        if let saved = mapped.gateways.first(where: { $0.id == gateway.id }) ?? mapped.gateways.first { gateway = saved; hasGateway = true }
        runtimeInstances = mapped.runtimes; runtimeTypes = mapped.runtimeTypes
        protocols = mapped.protocols
        modelTemplates = mapped.modelTemplates
        providerImportTypes = mapped.importTypes
        selectedRuntimeID = mapped.selectedRuntimeID
    }
    private func loadConversations() {
        guard let transport else { error = "本地核心未配置"; return }
        enqueue({ try transport.list() }) { [weak self] data in
            guard let self else { return }
            applyList(data)
            if let id = conversations.first?.id { selectConversation(id: id) }
        }
    }
    private func apply(_ value: ConversationSnapshot?) {
        guard let value else { return }
        snapshot = value; selectedRuntimeID = value.conversation.runtimeID; selectedConversationID = value.conversation.id; transcript.apply(value.messages)
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
        apply(BindingMapping.snapshot(incoming))
    }
    private func poll() {
        guard !isPreview, !isShuttingDown, snapshot != nil, !isLoading, !pollPending, let transport, let runtimeID = selectedRuntimeID else { return }
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
    private func enqueue<T: Sendable>(_ operation: @escaping () throws -> T, onAccepted: (() -> Void)? = nil, onFailure: ((Error) -> Void)? = nil, apply: @escaping (T) -> Void) {
        guard !isLoading, !isShuttingDown else { error = "请等待当前操作完成后重试"; return }
        guard transport != nil else { error = "本地核心未配置"; return }
        let revision = generation
        isLoading = true; error = nil
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
        protocols = [ProtocolDescriptor(id: .chatCompletionsV1, name: "OpenAI Chat Completions v1", supported: true), ProtocolDescriptor(id: .responsesV1, name: "OpenAI Responses v1", supported: true), ProtocolDescriptor(id: .messagesV1, name: "Anthropic Messages v1", supported: false)]
        runtimeTypes = [RuntimeTypeDescriptor(id: "sample-type", familyID: "sample", versionRegex: ".*", canRenameConversations: true, canDeleteConversations: true, supportedProtocols: [.chatCompletionsV1, .responsesV1], name: "示例运行时", fields: [])]
        runtimeInstances = [RuntimeInstance(id: "sample-instance", name: "示例运行时", typeID: "sample-type", gatewayID: gateway.id, settings: [:]), RuntimeInstance(id: "sample-review", name: "另一个运行时", typeID: "sample-type", gatewayID: gateway.id, settings: [:])]
        selectedRuntimeID = runtimeInstances[0].id
        let topics = ["让设置页更安静", "整理一段代码", "下一步的项目计划"]
        let pairs = [
            ["我想让设置页面更简单，先帮我梳理一下。", "# 一次清晰的工作循环\n\n先整理任务，再执行。\n\n- [x] 读取上下文\n- [ ] 完成修改\n\n> 每一步都保留可以复核的结果。\n\n| 内容 | 状态 |\n| --- | --- |\n| 会话 | 可继续 |\n| 工具 | 已完成 |\n\n正文支持**强调**、[链接](https://example.invalid)和 `inline code`。", "运行时实例应该放哪里？", "工具栏显示当前运行时实例，详细配置放进系统设置窗口里。会话页面保留对话与模型选择，让注意力留在正在做的工作上。"],
            ["这个读取配置的函数可以再清晰一点吗？", "先把输入校验放在函数边界，主体就能直接表达意图。\n\n```rust\nfn load(path: &Path) -> Result<Config> {\n    let source = fs::read_to_string(path)?;\n    let config: Config = toml::from_str(&source)?;\n    config.validate()?;\n    Ok(config)\n}\n```", "还需要抽一个新的层吗？", "目前不需要。等第二个调用方出现不同的读取需求，再决定是否有值得共享的边界。"],
            ["帮我把接下来的工作排一下。", "先完成可用的会话界面，再检查配置是否真正驱动运行时。最后用一个短任务验证发送、停止和会话切换。", "先从哪个环节开始？", "从会话开始：打开旧会话、发送消息、检查回复，再新建一个会话。这个路径会暴露最直接的体验问题。"]
        ]
        for index in topics.indices {
            let conversation = Conversation(id: "sample-\(index)", title: topics[index], updatedAtUnixMs: Int64(Date().timeIntervalSince1970 * 1000) - Int64(index * 86_400_000), runtimeID: runtimeInstances[0].id, cwd: nil)
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
