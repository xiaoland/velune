import Combine
import Foundation
import AppKit
import Security
import VeluneBindings

@MainActor
final class AppStore: ObservableObject {
    @Published private(set) var conversations: [Conversation] = []
    @Published private(set) var selectedConversationID: String?
    @Published private(set) var selectedConnectionID: String?
    @Published private(set) var messages: [Message] = []
    @Published private(set) var connections: [Connection] = []
    @Published private(set) var gateway = GatewayConfig()
    @Published private(set) var runtimeInstances: [RuntimeInstance] = []
    @Published private(set) var runtimeTypes: [RuntimeTypeDescriptor] = []
    @Published private(set) var protocols: [ProtocolDescriptor] = []
    @Published private(set) var authenticationBindings: [AuthenticationBinding] = []
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
    var models: [AIModel] { gateway.models }
    var providers: [AIProvider] { gateway.providers }
    var routes: [ModelRoute] { gateway.routes }
    var selectedConversationTitle: String? { conversations.first { $0.id == selectedConversationID }?.title }
    var isGenerating: Bool { snapshot?.runState == .running || snapshot?.runState == .stopping }
    var isBusy: Bool { isLoading || isGenerating || isShuttingDown || authenticationRunning }
    var canSend: Bool { !authenticationRunning && !isLoading && !isShuttingDown && (snapshot?.actions.canSend ?? false) }
    var canCancel: Bool { snapshot?.actions.canCancel ?? false }
    var canSwitchModel: Bool { snapshot != nil && !isGenerating && !isLoading && !isShuttingDown }
    var needsModelSelection: Bool { snapshot != nil && snapshot?.modelRecordKey == nil }
    var selectedModelName: String? { models.first { $0.recordKey == snapshot?.modelRecordKey }?.displayName }
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
        guard !isLoading, !isGenerating, conversations.contains(where: { $0.id == id }) else { return }
        generation += 1; selectedConversationID = id; snapshot = nil; messages = []
        if isPreview { apply(previewSnapshots[id]); return }
        guard let transport, let runtimeID = selectedConnectionID else { error = "请先连接运行时"; return }
        enqueue({ try transport.openConversation(runtimeID: runtimeID, conversationID: id) }) { [weak self] in self?.applySnapshotResult($0) }
    }
    func createConversation(cwd: String? = nil) {
        guard !isLoading, !isGenerating else { return }
        if !isPreview, cwd == nil {
            let panel = NSOpenPanel()
            panel.title = "选择新会话的工作目录"
            panel.message = "Agent 运行时将在此目录中执行项目工具。"
            panel.canChooseDirectories = true
            panel.canChooseFiles = false
            panel.allowsMultipleSelection = false
            if panel.runModal() == .OK, let url = panel.url {
                createConversation(cwd: url.standardizedFileURL.path)
            }
            return
        }
        generation += 1
        if isPreview {
            let conversation = Conversation(id: UUID().uuidString, title: "新会话", updatedAt: "刚刚", runtimeID: selectedConnectionID ?? runtimeInstances.first?.id ?? "sample-instance", cwd: cwd)
            apply(ConversationSnapshot(revision: 1, conversation: conversation, modelRecordKey: models.first?.id, runState: .idle, messages: [], actions: ConversationActions(canSend: true, canCancel: false, canSwitch: true)))
            return
        }
        guard let cwd, !cwd.isEmpty else { error = "新会话需要选择工作目录"; return }
        guard let runtimeID = selectedConnectionID else { error = "请先连接运行时"; return }
        guard let transport else { error = "本地核心未配置"; return }
        enqueue({ try transport.createConversation(runtimeID: runtimeID, cwd: cwd) }) { [weak self] in self?.applySnapshotResult($0) }
    }
    func send(text: String, onAccepted: (() -> Void)? = nil) {
        guard canSend, !text.trimmingCharacters(in: .whitespacesAndNewlines).isEmpty else { return }
        if isPreview { snapshot?.messages.append(Message(id: UUID().uuidString, role: "user", blocks: [.text(text)])); apply(snapshot); onAccepted?(); return }
        guard let runtimeID = selectedConnectionID else { error = "请先连接运行时"; return }
        guard let transport else { error = "本地核心未配置"; return }
        enqueue({ try transport.send(runtimeID: runtimeID, text: text) }, onAccepted: onAccepted) { [weak self] in self?.applySnapshotResult($0) }
    }
    func cancel() {
        guard canCancel, !isLoading else { return }
        if isPreview { snapshot?.runState = .idle; snapshot?.actions.canCancel = false; apply(snapshot); return }
        guard let runtimeID = selectedConnectionID else { error = "请先连接运行时"; return }
        guard let transport else { error = "本地核心未配置"; return }
        enqueue({ try transport.cancel(runtimeID: runtimeID) }) { [weak self] in self?.applySnapshotResult($0) }
    }
    func saveModel(_ model: AIModel, onSaved: (() -> Void)? = nil) {
        var value = model
        if isPreview && value.recordKey.isEmpty { value.recordKey = UUID().uuidString }
        var config = gateway
        if !value.recordKey.isEmpty { config.models.removeAll { $0.recordKey == value.recordKey } }
        config.models.append(value)
        saveGateway(config, onSaved: onSaved)
    }
    func deleteModel(id: String) {
        var config = gateway; config.models.removeAll { $0.id == id }; config.routes.removeAll { $0.modelRecordKey == id }
        for index in config.providers.indices { config.providers[index].models.removeAll { $0.modelRecordKey == id } }
        saveGateway(config)
    }
    func saveProvider(_ value: AIProvider, onSaved: (() -> Void)? = nil) {
        guard !authenticationRunning else { error = "请先完成或取消登录"; return }
        var config = gateway; config.providers.removeAll { $0.id == value.id }; config.providers.append(value)
        config.routes.removeAll { route in route.providerID == value.id && !value.models.contains { $0.modelRecordKey == route.modelRecordKey } }
        saveGateway(config, onSaved: onSaved)
    }
    func saveAPIKeyBinding(bindingID: String?, expectedGeneration: UInt64?, name: String, protocolID: ProviderProtocol, endpoint: String, secret: String, onSaved: @escaping () -> Void) {
        guard !authenticationRunning, !isGenerating, !isLoading, !isShuttingDown else { error = "请等待当前任务完成后再修改认证"; return }
        guard isPreview || transport != nil else { error = "本地核心未配置"; return }
        let id = bindingID ?? UUID().uuidString
        let reference = "credential-\(UUID().uuidString)"
        do { if !isPreview { try storeCredential(secret, reference: reference) } }
        catch { self.error = error.localizedDescription; return }
        if isPreview {
            authenticationBindings.removeAll { $0.id == id }
            authenticationBindings.append(AuthenticationBinding(id: id, name: name, method: .apiKey, configured: true, protocolID: protocolID, endpoint: endpoint, generation: 1, provenance: nil, actions: []))
            onSaved(); return
        }
        guard let transport else { error = "本地核心未配置"; return }
        enqueue({ try transport.configureAPIKeyBinding(id: id, name: name.trimmingCharacters(in: .whitespacesAndNewlines), keychainRef: reference, protocolID: protocolID, endpoint: endpoint.trimmingCharacters(in: .whitespacesAndNewlines), expectedGeneration: expectedGeneration) }, onFailure: { [weak self] failure in
            if let transportFailure = failure as? TransportError, case .authenticationNotCommitted = transportFailure {
                if let cleanupError = self?.deleteCredential(reference: reference) { self?.error = failure.localizedDescription + "；" + cleanupError }
            } else {
                self?.error = failure.localizedDescription + "；未能确认认证保存结果，新 Keychain 项已保留。"
            }
        }) { [weak self] mutation in
            self?.applyAuthenticationMutation(mutation)
            onSaved()
        }
    }
    func renameAuthenticationBinding(_ id: String, name: String, onSaved: @escaping () -> Void) {
        if isPreview { if let index = authenticationBindings.firstIndex(where: { $0.id == id }) { authenticationBindings[index].name = name }; onSaved(); return }
        guard let transport else { error = "本地核心未配置"; return }
        enqueue({ try transport.renameAuthenticationBinding(id, name: name.trimmingCharacters(in: .whitespacesAndNewlines)) }) { [weak self] mutation in
            self?.applyAuthenticationMutation(mutation); onSaved()
        }
    }
    func deleteAuthenticationBinding(_ id: String) {
        if isPreview { authenticationBindings.removeAll { $0.id == id }; return }
        guard let transport else { error = "本地核心未配置"; return }
        enqueue({ try transport.deleteAuthenticationBinding(id) }) { [weak self] mutation in self?.applyAuthenticationMutation(mutation, deleted: true) }
    }
    private func applyAuthenticationMutation(_ value: BindingAuthenticationMutation, deleted: Bool = false) {
        authenticationBindings.removeAll { $0.id == value.binding.id }
        if !deleted { authenticationBindings.append(BindingMapping.authenticationBinding(value.binding)) }
        var warnings = value.warnings
        for reference in value.obsoleteOwnedKeychainRefs {
            if let warning = deleteCredential(reference: reference) { warnings.append(warning) }
        }
        if value.requiresReconnect { invalidateConnection() }
        if !warnings.isEmpty { error = (deleted ? "认证资源已删除；" : "认证配置已保存；") + warnings.joined(separator: "；") }
    }
    func inspectAuthentication(_ bindingID: String, apply: @escaping (AuthenticationMetadata) -> Void) {
        guard !isPreview else { error = "预览不会读取凭据状态"; return }
        guard let transport else { error = "本地核心未配置"; return }
        enqueue({ try transport.authenticationInspect(bindingID: bindingID) }) { [weak self] value in
            if let index = self?.authenticationBindings.firstIndex(where: { $0.id == bindingID }) { self?.authenticationBindings[index].configured = value.configured }
            apply(BindingMapping.authenticationMetadata(value))
        }
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
                if data.requiresReconnect { invalidateConnection() }
                if !data.requiresReconnect { enqueue({ try transport.list() }) { [weak self] in self?.applyList($0) } }
                completion()
            }
    }
    func startAuthentication(_ bindingID: String) {
        guard !isGenerating else { error = "请先停止当前任务，再开始登录"; return }
        guard !isPreview else { error = "预览不会启动认证"; return }
        guard let transport else { error = "本地核心未配置"; return }
        enqueue({ try transport.authenticationStart(bindingID: bindingID) }) { [weak self] data in
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
        if data.requiresReconnect == true { invalidateConnection() }
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
        var config = gateway; config.providers.removeAll { $0.id == id }; config.routes.removeAll { $0.providerID == id }
        saveGateway(config)
    }
    func saveRoute(modelRecordKey: String, providerID: String) {
        var config = gateway; config.routes.removeAll { $0.modelRecordKey == modelRecordKey }; config.routes.append(ModelRoute(modelRecordKey: modelRecordKey, providerID: providerID))
        saveGateway(config)
    }
    func saveGateway(_ config: GatewayConfig, onSaved: (() -> Void)? = nil) {
        guard !isGenerating else { error = "请先停止当前任务，再修改 AI 服务配置"; return }
        if isPreview { gateway = config; hasGateway = true; onSaved?(); return }
        guard let transport else { error = "本地核心未配置"; return }
        enqueue({ try transport.upsertGateway(BindingMapping.bindingGateway(config)) }) { [weak self] data in
            guard let self else { return }
            if let saved = data.gateways.first(where: { $0.id == config.id }) {
                gateway = BindingMapping.gateway(saved); hasGateway = true
                if data.requiresReconnect { invalidateConnection() }
                onSaved?()
            }
            else { error = "核心未返回保存后的网关配置" }
        }
    }
    func saveRuntimeInstance(_ instance: RuntimeInstance, onSaved: (() -> Void)? = nil) {
        guard !isGenerating else { error = "请先停止当前任务，再修改运行时实例"; return }
        if isPreview { runtimeInstances.removeAll { $0.id == instance.id }; runtimeInstances.append(instance); onSaved?(); return }
        if !hasGateway { saveGateway(gateway) { [weak self] in self?.saveRuntimeInstance(instance, onSaved: onSaved) }; return }
        guard let transport else { error = "本地核心未配置"; return }
        enqueue({ try transport.upsertRuntime(BindingMapping.bindingRuntime(instance)) }) { [weak self] data in
            guard let self else { return }
            runtimeInstances = data.runtimeInstances.map(BindingMapping.runtime); runtimeTypes = data.runtimeTypes.map(BindingMapping.runtimeType)
            if data.requiresReconnect { invalidateConnection() }
            if data.runtimeInstances.contains(where: { $0.id == instance.id }) { onSaved?() }
            else { error = "核心未返回保存后的运行时实例" }
        }
    }
    func deleteRuntimeInstance(id: String) {
        if isPreview {
            runtimeInstances.removeAll { $0.id == id }
            if selectedConnectionID == id { resetProjection(); selectedConnectionID = nil; connections.removeAll { $0.id == id } }
            return
        }
        guard let transport else { error = "本地核心未配置"; return }
        enqueue({ try transport.deleteRuntime(id: id) }) { [weak self] data in
            self?.runtimeInstances = data.runtimeInstances.map(BindingMapping.runtime); self?.runtimeTypes = data.runtimeTypes.map(BindingMapping.runtimeType)
            if data.requiresReconnect { self?.invalidateConnection() }
        }
    }
    func selectModel(modelRecordKey: String) {
        guard !isLoading, !isGenerating, models.contains(where: { $0.id == modelRecordKey }) else { return }
        if isPreview { snapshot?.modelRecordKey = modelRecordKey; apply(snapshot); return }
        guard let runtimeID = selectedConnectionID else { error = "请先选择运行时实例"; return }
        guard let transport else { error = "本地核心未配置"; return }
        enqueue({ try transport.selectModel(runtimeID: runtimeID, modelRecordKey: modelRecordKey) }) { [weak self] in self?.applySnapshotResult($0) }
    }
    func selectConnection(id: String) {
        guard !isLoading, !isGenerating else { return }
        if isPreview { selectedConnectionID = id; return }
        guard let transport else { error = "本地核心未配置"; return }
        enqueue({ try transport.connectRuntime(id: id) }) { [weak self] in
            self?.resetProjection(); self?.selectedConnectionID = $0.runtimeInstanceId ?? id; self?.connections = $0.connections.map { Connection(id: $0.id, name: $0.name, state: $0.state, capabilities: $0.capabilities) }
            self?.loadConversations()
        }
    }
    func performRuntimeAction(instanceID: String, actionID: String) {
        guard !isLoading, !isGenerating else { return }
        if isPreview { selectedConnectionID = instanceID; return }
        guard actionID == "connect", let transport else { error = "此运行时动作暂不支持"; return }
        enqueue({ try transport.connectRuntime(id: instanceID) }) { [weak self] data in
            self?.resetProjection(); self?.selectedConnectionID = data.runtimeInstanceId ?? instanceID
            self?.connections = data.connections.map { Connection(id: $0.id, name: $0.name, state: $0.state, capabilities: $0.capabilities) }
            if !data.connections.isEmpty { self?.loadConversations() }
        }
    }
    private func invalidateConnection() {
        resetProjection(); connections = []; selectedConnectionID = nil
        guard let transport else { error = "本地核心未配置"; return }
        enqueue({ try transport.list() }) { [weak self] in self?.applyList($0) }
    }
    private func resetProjection() { generation += 1; snapshot = nil; selectedConversationID = nil; messages = []; activity = nil }
    private func applyList(_ data: BindingConfigurationSnapshot) {
        let mapped = BindingMapping.configuration(data)
        conversations = mapped.conversations; connections = mapped.connections
        if let saved = mapped.gateways.first(where: { $0.id == gateway.id }) ?? mapped.gateways.first { gateway = saved; hasGateway = true }
        runtimeInstances = mapped.runtimes; runtimeTypes = mapped.runtimeTypes
        protocols = mapped.protocols
        authenticationBindings = mapped.authenticationBindings
        providerImportTypes = mapped.importTypes
        selectedConnectionID = mapped.activeRuntimeID
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
        if snapshot?.conversation.id == value.conversation.id, (snapshot?.revision ?? 0) > value.revision { return }
        snapshot = value; selectedConversationID = value.conversation.id; messages = value.messages
        if let index = conversations.firstIndex(where: { $0.id == value.conversation.id }) { conversations[index] = value.conversation }
        else { conversations.insert(value.conversation, at: 0) }
        activity = value.runState == .running ? "正在思考与执行" : value.runState == .stopping ? "正在停止" : nil
        if isPreview { previewSnapshots[value.conversation.id] = value }
    }
    private func applySnapshotResult(_ value: BindingSnapshotResult) {
        apply(value.snapshot.map(BindingMapping.snapshot))
    }
    private func poll() {
        guard !isPreview, !isShuttingDown, snapshot != nil, !isLoading, !pollPending, let transport, let runtimeID = selectedConnectionID else { return }
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
                case .success(let data): self.apply(data.snapshot.map(BindingMapping.snapshot))
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
    private func deleteCredential(reference: String) -> String? {
        guard reference.hasPrefix("credential-"),
              let id = UUID(uuidString: String(reference.dropFirst("credential-".count))),
              reference.lowercased() == "credential-\(id.uuidString.lowercased())" else {
            return "拒绝清理不属于本应用管理范围的 Keychain 项"
        }
        let query: [String: Any] = [kSecClass as String: kSecClassGenericPassword, kSecAttrService as String: "local.velune.provider", kSecAttrAccount as String: reference]
        let status = SecItemDelete(query as CFDictionary)
        if status != errSecSuccess && status != errSecItemNotFound {
            return "Keychain 清理失败（\(status)）"
        }
        return nil
    }
    private func storeCredential(_ secret: String, reference: String) throws {
        let item: [String: Any] = [kSecClass as String: kSecClassGenericPassword,
                                   kSecAttrService as String: "local.velune.provider", kSecAttrAccount as String: reference,
                                   kSecValueData as String: Data(secret.utf8)]
        let status = SecItemAdd(item as CFDictionary, nil)
        guard status == errSecSuccess else {
            throw NSError(domain: NSOSStatusErrorDomain, code: Int(status), userInfo: [NSLocalizedDescriptionKey: "无法将凭据保存到 Keychain（\(status)）。"])
        }
    }

    private func seedPreview() {
        gateway = GatewayConfig(models: [AIModel(recordKey: "sample-model", nickname: "通用模型", icon: "sparkles", contextWindow: 8192, maxOutputTokens: 4096)], providers: [AIProvider(id: "sample-provider", name: "示例 AI 服务", protocolID: .chatCompletionsV1, endpoint: "https://example.invalid/v1", authenticationID: "sample-reference", models: [ProviderModelBinding(modelRecordKey: "sample-model", providerModelID: "external-example", contextWindow: 8192, maxOutputTokens: 4096)])], routes: [ModelRoute(modelRecordKey: "sample-model", providerID: "sample-provider")])
        hasGateway = true
        protocols = [ProtocolDescriptor(id: .chatCompletionsV1, name: "OpenAI Chat Completions v1", supported: true), ProtocolDescriptor(id: .responsesV1, name: "OpenAI Responses v1", supported: true), ProtocolDescriptor(id: .messagesV1, name: "Anthropic Messages v1", supported: false)]
        runtimeTypes = [RuntimeTypeDescriptor(id: "sample-type", name: "示例运行时", fields: [], actions: [SettingAction(id: "connect", label: "连接")])]
        runtimeInstances = [RuntimeInstance(id: "sample-instance", name: "示例运行时", typeID: "sample-type", gatewayID: gateway.id, settings: [:], modelRecordKey: "sample-model"), RuntimeInstance(id: "sample-review", name: "另一个运行时", typeID: "sample-type", gatewayID: gateway.id, settings: [:], modelRecordKey: "sample-model")]
        connections = [Connection(id: runtimeInstances[0].id, name: runtimeInstances[0].name, state: "ready", capabilities: ["conversation", "streaming", "cancel"])]
        selectedConnectionID = runtimeInstances[0].id
        let topics = ["让设置页更安静", "整理一段代码", "下一步的项目计划"]
        let pairs = [
            ["我想让设置页面更简单，先帮我梳理一下。", "可以。我们先保留四个明确的入口：**AI 提供商、全局模型、模型路由与运行时实例**。每个页面只回答一件事，常用操作直接放在可见位置。", "运行时实例应该放哪里？", "工具栏显示当前运行时实例，详细配置放进系统设置窗口里。会话页面保留对话与模型选择，让注意力留在正在做的工作上。"],
            ["这个读取配置的函数可以再清晰一点吗？", "先把输入校验放在函数边界，主体就能直接表达意图。\n\n```rust\nfn load(path: &Path) -> Result<Config> {\n    let source = fs::read_to_string(path)?;\n    let config: Config = toml::from_str(&source)?;\n    config.validate()?;\n    Ok(config)\n}\n```", "还需要抽一个新的层吗？", "目前不需要。等第二个调用方出现不同的读取需求，再决定是否有值得共享的边界。"],
            ["帮我把接下来的工作排一下。", "先完成可用的会话界面，再检查配置是否真正驱动运行时。最后用一个短任务验证发送、停止和会话切换。", "先从哪个环节开始？", "从会话开始：打开旧会话、发送消息、检查回复，再新建一个会话。这个路径会暴露最直接的体验问题。"]
        ]
        for index in topics.indices {
            let conversation = Conversation(id: "sample-\(index)", title: topics[index], updatedAt: ["今天", "昨天", "周一"][index], runtimeID: runtimeInstances[0].id, cwd: nil)
            var history = pairs[index].enumerated().map { Message(id: "sample-\(index)-\($0.offset)", role: $0.offset.isMultiple(of: 2) ? "user" : "assistant", blocks: [.text($0.element)]) }
            history[1].blocks.append(MessageBlock(kind: "tool", text: index == 1 ? "已读取 3 个文件，未修改项目。" : "已梳理当前任务的上下文。", toolID: "sample-tool-\(index)", title: index == 1 ? "检查项目代码" : "读取工作上下文", state: "done"))
            if index == 2 { history[history.count - 1].blocks.append(MessageBlock(kind: "tool", text: "正在整理计划。", toolID: "sample-progress", title: "整理项目计划", state: "running")) }
            previewSnapshots[conversation.id] = ConversationSnapshot(revision: 1, conversation: conversation, modelRecordKey: models[0].id, runState: .idle, messages: history, actions: ConversationActions(canSend: true, canCancel: false, canSwitch: true))
            conversations.append(conversation)
        }
        apply(previewSnapshots["sample-0"])
    }
}
