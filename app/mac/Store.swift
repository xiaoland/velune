import Combine
import Foundation
import Security

private struct ListData: Decodable, Sendable {
    var conversations: [Conversation]
    var connections: [Connection]
    var gateways: [GatewayConfig]
    var runtimeInstances: [RuntimeInstance]
    var runtimeTypes: [RuntimeTypeDescriptor]
    var protocols: [ProtocolDescriptor]?
    var credentialSourceTypes: [CredentialSourceType]?
    var runtimeInstanceID: String?
    enum CodingKeys: String, CodingKey {
        case conversations, connections, gateways, runtimeInstances, runtimeTypes, protocols, credentialSourceTypes
        case runtimeInstanceID = "activeRuntimeInstanceID"
    }
}
private struct SnapshotData: Decodable, Sendable { var snapshot: ConversationSnapshot? }
private struct GatewayData: Decodable, Sendable { var gateways: [GatewayConfig]; var requiresReconnect: Bool? }
private struct RuntimeData: Decodable, Sendable { var runtimeInstances: [RuntimeInstance]; var runtimeTypes: [RuntimeTypeDescriptor]; var requiresReconnect: Bool? }
private struct ConnectData: Decodable, Sendable { var connections: [Connection]; var runtimeInstanceID: String? }

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
    @Published private(set) var credentialSourceTypes: [CredentialSourceType] = []
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
    var needsModelSelection: Bool { snapshot != nil && snapshot?.modelID == nil }
    var selectedModelName: String? { models.first { $0.id == snapshot?.modelID }?.nickname }
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
        guard !isPreview, transport != nil else { return }
        request("list", as: ListData.self) { [weak self] data in
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
        request("open", payload: ["conversationID": id], as: SnapshotData.self) { [weak self] in self?.apply($0.snapshot) }
    }
    func createConversation() {
        guard !isLoading, !isGenerating else { return }
        generation += 1
        if isPreview {
            let conversation = Conversation(id: UUID().uuidString, title: "新会话", updatedAt: "刚刚", runtimeID: selectedConnectionID ?? runtimeInstances.first?.id ?? "sample-instance")
            apply(ConversationSnapshot(revision: 1, conversation: conversation, modelID: models.first?.id, runState: .idle, messages: [], actions: ConversationActions(canSend: true, canCancel: false, canSwitch: true)))
            return
        }
        request("create", as: SnapshotData.self) { [weak self] in self?.apply($0.snapshot) }
    }
    func send(text: String, onAccepted: (() -> Void)? = nil) {
        guard canSend, !text.trimmingCharacters(in: .whitespacesAndNewlines).isEmpty else { return }
        if isPreview { snapshot?.messages.append(Message(id: UUID().uuidString, role: "user", blocks: [.text(text)])); apply(snapshot); onAccepted?(); return }
        request("send", payload: ["text": text], as: SnapshotData.self, onAccepted: onAccepted) { [weak self] in self?.apply($0.snapshot) }
    }
    func cancel() {
        guard canCancel, !isLoading else { return }
        if isPreview { snapshot?.runState = .idle; snapshot?.actions.canCancel = false; apply(snapshot); return }
        request("cancel", as: SnapshotData.self) { [weak self] in self?.apply($0.snapshot) }
    }
    func saveModel(_ model: AIModel, onSaved: (() -> Void)? = nil) {
        var config = gateway; config.models.removeAll { $0.id == model.id }; config.models.append(model)
        saveGateway(config, onSaved: onSaved)
    }
    func deleteModel(id: String) {
        var config = gateway; config.models.removeAll { $0.id == id }; config.routes.removeAll { $0.modelID == id }
        for index in config.providers.indices { config.providers[index].models.removeAll { $0.modelID == id } }
        saveGateway(config)
    }
    func saveProvider(_ provider: AIProvider, secret: String = "", onSaved: (() -> Void)? = nil) {
        guard !authenticationRunning else { error = "请先完成或取消登录"; return }
        guard !isGenerating else { error = "请先停止当前任务，再修改 AI 服务配置"; return }
        guard !isLoading, !isShuttingDown else { error = "请等待当前操作完成后重试"; return }
        var value = provider
        do {
            if !secret.isEmpty {
                guard value.credentialSource == nil else { throw NSError(domain: "Velune", code: 1, userInfo: [NSLocalizedDescriptionKey: "认证来源与 API key 不能同时配置"]) }
                let reference = value.credentialRef?.trimmingCharacters(in: .whitespacesAndNewlines)
                value.credentialRef = reference.flatMap { $0.isEmpty ? nil : $0 } ?? "provider-\(value.id)"
                let generation = value.credentialGeneration ?? 0
                guard generation < UInt64.max else { throw NSError(domain: "Velune", code: 1, userInfo: [NSLocalizedDescriptionKey: "认证配置版本已达到上限"]) }
                if !isPreview { try storeCredential(secret, reference: value.credentialRef!) }
                value.credentialGeneration = generation + 1
            }
            var config = gateway; config.providers.removeAll { $0.id == value.id }; config.providers.append(value)
            config.routes.removeAll { route in route.providerID == value.id && !value.models.contains { $0.modelID == route.modelID } }
            saveGateway(config, onSaved: onSaved)
        } catch { self.error = error.localizedDescription }
    }
    func inspectAuthentication(_ source: CredentialSource, apply: @escaping (AuthenticationMetadata) -> Void) {
        guard !isPreview else { error = "预览不会读取认证来源"; return }
        do { request("authentication", payload: ["operation": "inspect", "source": try json(source)], as: AuthenticationInspection.self) { apply($0.metadata) } }
        catch { self.error = error.localizedDescription }
    }
    func startAuthentication(_ source: CredentialSource) {
        guard !isGenerating else { error = "请先停止当前任务，再开始登录"; return }
        guard !isPreview else { error = "预览不会启动认证"; return }
        do {
            request("authentication", payload: ["operation": "start", "source": try json(source)], as: AuthenticationData.self) { [weak self] data in
                guard let self else { return }
                authenticationPrompt = nil; authenticationNotifications = []; authenticationResult = nil
                showsAuthentication = true; applyAuthentication(data)
            }
        } catch { self.error = error.localizedDescription }
    }
    func answerAuthentication(id: String, value: String) {
        request("authentication", payload: ["operation": "reply", "id": id, "value": value], as: AuthenticationData.self) { [weak self] in self?.authenticationPrompt = nil; self?.applyAuthentication($0) }
    }
    func cancelAuthentication() {
        request("authentication", payload: ["operation": "cancel"], as: AuthenticationData.self) { [weak self] in self?.applyAuthentication($0) }
    }
    private func pollAuthentication() {
        guard authenticationRunning, !authenticationPollPending, !isShuttingDown, let transport else { return }
        authenticationPollPending = true
        queue.async { [weak self] in
            let result = Result { try transport.request("authentication", payload: ["operation": "poll"], as: AuthenticationData.self) }
            DispatchQueue.main.async {
                guard let self else { return }
                self.authenticationPollPending = false
                switch result {
                case .success(let data): self.applyAuthentication(data)
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
    func saveRoute(modelID: String, providerID: String) {
        var config = gateway; config.routes.removeAll { $0.modelID == modelID }; config.routes.append(ModelRoute(modelID: modelID, providerID: providerID))
        saveGateway(config)
    }
    func saveGateway(_ config: GatewayConfig, onSaved: (() -> Void)? = nil) {
        guard !isGenerating else { error = "请先停止当前任务，再修改 AI 服务配置"; return }
        if isPreview { gateway = config; hasGateway = true; onSaved?(); return }
        do {
            request("gateways", payload: ["operation": "upsert", "gateway": try json(config)], as: GatewayData.self) { [weak self] data in
                guard let self else { return }
                if let saved = data.gateways.first(where: { $0.id == config.id }) {
                    gateway = saved; hasGateway = true
                    if data.requiresReconnect == true { invalidateConnection() }
                    onSaved?()
                }
                else { error = "核心未返回保存后的网关配置" }
            }
        } catch { self.error = error.localizedDescription }
    }
    func saveRuntimeInstance(_ instance: RuntimeInstance, onSaved: (() -> Void)? = nil) {
        guard !isGenerating else { error = "请先停止当前任务，再修改运行时实例"; return }
        if isPreview { runtimeInstances.removeAll { $0.id == instance.id }; runtimeInstances.append(instance); onSaved?(); return }
        if !hasGateway { saveGateway(gateway) { [weak self] in self?.saveRuntimeInstance(instance, onSaved: onSaved) }; return }
        do {
            request("runtimeInstances", payload: ["operation": "upsert", "runtimeInstance": try json(instance)], as: RuntimeData.self) { [weak self] data in
                guard let self else { return }
                runtimeInstances = data.runtimeInstances; runtimeTypes = data.runtimeTypes
                if data.requiresReconnect == true { invalidateConnection() }
                if data.runtimeInstances.contains(where: { $0.id == instance.id }) { onSaved?() }
                else { error = "核心未返回保存后的运行时实例" }
            }
        } catch { self.error = error.localizedDescription }
    }
    func deleteRuntimeInstance(id: String) {
        if isPreview {
            runtimeInstances.removeAll { $0.id == id }
            if selectedConnectionID == id { resetProjection(); selectedConnectionID = nil; connections.removeAll { $0.id == id } }
            return
        }
        request("runtimeInstances", payload: ["operation": "delete", "runtimeInstanceID": id], as: RuntimeData.self) { [weak self] data in
            self?.runtimeInstances = data.runtimeInstances; self?.runtimeTypes = data.runtimeTypes
            if data.requiresReconnect == true { self?.invalidateConnection() }
        }
    }
    func selectModel(modelID: String) {
        guard !isLoading, !isGenerating, models.contains(where: { $0.id == modelID }) else { return }
        if isPreview { snapshot?.modelID = modelID; apply(snapshot); return }
        guard let runtimeID = selectedConnectionID else { error = "请先选择运行时实例"; return }
        request("selectModel", payload: ["runtimeInstanceID": runtimeID, "modelID": modelID], as: SnapshotData.self) { [weak self] in self?.apply($0.snapshot) }
    }
    func selectConnection(id: String) {
        guard !isLoading, !isGenerating else { return }
        if isPreview { selectedConnectionID = id; return }
        request("connect", payload: ["runtimeInstanceID": id], as: ConnectData.self) { [weak self] in
            self?.resetProjection(); self?.selectedConnectionID = $0.runtimeInstanceID ?? id; self?.connections = $0.connections
            self?.loadConversations()
        }
    }
    func performRuntimeAction(instanceID: String, actionID: String) {
        guard !isLoading, !isGenerating else { return }
        if isPreview { selectedConnectionID = instanceID; return }
        request("runtimeAction", payload: ["runtimeInstanceID": instanceID, "actionID": actionID], as: ConnectData.self) { [weak self] data in
            self?.resetProjection(); self?.selectedConnectionID = data.runtimeInstanceID ?? instanceID
            self?.connections = data.connections
            if !data.connections.isEmpty { self?.loadConversations() }
        }
    }
    private func invalidateConnection() {
        resetProjection(); connections = []; selectedConnectionID = nil
        request("list", as: ListData.self) { [weak self] in self?.applyList($0) }
    }
    private func resetProjection() { generation += 1; snapshot = nil; selectedConversationID = nil; messages = []; activity = nil }
    private func applyList(_ data: ListData) {
        conversations = data.conversations; connections = data.connections
        if let saved = data.gateways.first(where: { $0.id == gateway.id }) ?? data.gateways.first { gateway = saved; hasGateway = true }
        runtimeInstances = data.runtimeInstances; runtimeTypes = data.runtimeTypes
        protocols = data.protocols ?? []
        credentialSourceTypes = data.credentialSourceTypes ?? []
        selectedConnectionID = data.runtimeInstanceID
    }
    private func loadConversations() {
        request("list", as: ListData.self) { [weak self] data in
            guard let self else { return }
            applyList(data)
            if let id = conversations.first?.id { selectConversation(id: id) } else { createConversation() }
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
    private func poll() {
        guard !isPreview, !isShuttingDown, snapshot != nil, !isLoading, !pollPending, let transport, let runtimeID = selectedConnectionID else { return }
        let revision = generation
        let selected = selectedConversationID
        pollPending = true
        queue.async { [weak self] in
            let result = Result { try transport.request("getSnapshot", payload: ["runtimeInstanceID": runtimeID], as: SnapshotData.self) }
            DispatchQueue.main.async {
                guard let self else { return }
                self.pollPending = false
                guard self.generation == revision, self.selectedConversationID == selected else { return }
                switch result {
                case .success(let data): self.apply(data.snapshot)
                case .failure(let failure): self.error = failure.localizedDescription
                }
            }
        }
    }
    private func request<T: Decodable & Sendable>(_ action: String, payload: [String: String] = [:], as type: T.Type, onAccepted: (() -> Void)? = nil, apply: @escaping (T) -> Void) {
        guard !isLoading, !isShuttingDown else { error = "请等待当前操作完成后重试"; return }
        guard !authenticationRunning || ["authentication", "list", "getSnapshot"].contains(action) else { error = "请先完成或取消登录"; return }
        guard let transport else { error = "本地核心未配置"; return }
        let revision = generation
        var scopedPayload = payload
        if scopedPayload["runtimeInstanceID"] == nil, let runtimeID = selectedConnectionID { scopedPayload["runtimeInstanceID"] = runtimeID }
        let requestPayload = scopedPayload
        isLoading = true; error = nil
        queue.async { [weak self] in
            let result = Result { try transport.request(action, payload: requestPayload, as: type) }
            DispatchQueue.main.async {
                guard let self else { return }
                self.isLoading = false
                guard revision == self.generation else { return }
                switch result {
                case .success(let value): onAccepted?(); apply(value)
                case .failure(let failure): self.error = failure.localizedDescription
                }
            }
        }
    }
    private func json<T: Encodable>(_ value: T) throws -> String {
        String(decoding: try JSONEncoder().encode(value), as: UTF8.self)
    }
    private func storeCredential(_ secret: String, reference: String) throws {
        let query: [String: Any] = [kSecClass as String: kSecClassGenericPassword, kSecAttrService as String: "local.velune.provider", kSecAttrAccount as String: reference]
        let attributes = [kSecValueData as String: Data(secret.utf8)]
        let status = SecItemUpdate(query as CFDictionary, attributes as CFDictionary)
        if status == errSecItemNotFound {
            var item = query; item.merge(attributes) { _, new in new }
            let added = SecItemAdd(item as CFDictionary, nil)
            guard added == errSecSuccess else { throw credentialError(added) }
        } else if status != errSecSuccess { throw credentialError(status) }
    }
    private func credentialError(_ status: OSStatus) -> NSError {
        NSError(domain: "Velune.Keychain", code: Int(status), userInfo: [NSLocalizedDescriptionKey: "无法保存凭据到本机 Keychain（\(status)）"])
    }
    private func seedPreview() {
        gateway = GatewayConfig(models: [AIModel(id: "sample-model", nickname: "通用模型", icon: "sparkles", contextWindow: 8192, maxOutputTokens: 4096, reasoningLevels: ["standard", "deep"])], providers: [AIProvider(id: "sample-provider", name: "示例 AI 服务", protocolID: .chatCompletionsV1, endpoint: "https://example.invalid/v1", credentialRef: "sample-reference", models: [ProviderModelBinding(modelID: "sample-model", externalModelID: "external-example")])], routes: [ModelRoute(modelID: "sample-model", providerID: "sample-provider")])
        hasGateway = true
        protocols = [ProtocolDescriptor(id: .chatCompletionsV1, name: "OpenAI Chat Completions v1", supported: true), ProtocolDescriptor(id: .responsesV1, name: "OpenAI Responses v1", supported: true), ProtocolDescriptor(id: .messagesV1, name: "Anthropic Messages v1", supported: false)]
        runtimeTypes = [RuntimeTypeDescriptor(id: "sample-type", name: "示例运行时", fields: [SettingField(key: "workingDir", label: "工作目录", kind: .directoryPath, required: true, value: "", options: [], help: nil)], actions: [SettingAction(id: "connect", label: "连接")])]
        runtimeInstances = [RuntimeInstance(id: "sample-instance", name: "设计项目", typeID: "sample-type", gatewayID: gateway.id, settings: ["workingDir": "/示例/设计项目"], modelID: "sample-model"), RuntimeInstance(id: "sample-review", name: "代码审查", typeID: "sample-type", gatewayID: gateway.id, settings: ["workingDir": "/示例/代码项目"], modelID: "sample-model")]
        connections = [Connection(id: runtimeInstances[0].id, name: runtimeInstances[0].name, state: "ready", capabilities: ["conversation", "streaming", "cancel"])]
        selectedConnectionID = runtimeInstances[0].id
        let topics = ["让设置页更安静", "整理一段代码", "下一步的项目计划"]
        let pairs = [
            ["我想让设置页面更简单，先帮我梳理一下。", "可以。我们先保留四个明确的入口：**AI 提供商、全局模型、模型路由与运行时实例**。每个页面只回答一件事，常用操作直接放在可见位置。", "运行时实例应该放哪里？", "工具栏显示当前运行时实例，详细配置放进系统设置窗口里。会话页面保留对话与模型选择，让注意力留在正在做的工作上。"],
            ["这个读取配置的函数可以再清晰一点吗？", "先把输入校验放在函数边界，主体就能直接表达意图。\n\n```rust\nfn load(path: &Path) -> Result<Config> {\n    let source = fs::read_to_string(path)?;\n    let config: Config = toml::from_str(&source)?;\n    config.validate()?;\n    Ok(config)\n}\n```", "还需要抽一个新的层吗？", "目前不需要。等第二个调用方出现不同的读取需求，再决定是否有值得共享的边界。"],
            ["帮我把接下来的工作排一下。", "先完成可用的会话界面，再检查配置是否真正驱动运行时。最后用一个短任务验证发送、停止和会话切换。", "先从哪个环节开始？", "从会话开始：打开旧会话、发送消息、检查回复，再新建一个会话。这个路径会暴露最直接的体验问题。"]
        ]
        for index in topics.indices {
            let conversation = Conversation(id: "sample-\(index)", title: topics[index], updatedAt: ["今天", "昨天", "周一"][index], runtimeID: runtimeInstances[0].id)
            var history = pairs[index].enumerated().map { Message(id: "sample-\(index)-\($0.offset)", role: $0.offset.isMultiple(of: 2) ? "user" : "assistant", blocks: [.text($0.element)]) }
            history[1].blocks.append(MessageBlock(kind: "tool", text: index == 1 ? "已读取 3 个文件，未修改项目。" : "已梳理当前任务的上下文。", toolID: "sample-tool-\(index)", title: index == 1 ? "检查项目代码" : "读取工作上下文", state: "done"))
            if index == 2 { history[history.count - 1].blocks.append(MessageBlock(kind: "tool", text: "正在整理计划。", toolID: "sample-progress", title: "整理项目计划", state: "running")) }
            previewSnapshots[conversation.id] = ConversationSnapshot(revision: 1, conversation: conversation, modelID: models[0].id, runState: .idle, messages: history, actions: ConversationActions(canSend: true, canCancel: false, canSwitch: true))
            conversations.append(conversation)
        }
        apply(previewSnapshots["sample-0"])
    }
}
