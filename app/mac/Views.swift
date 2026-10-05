import SwiftUI
import AppKit

struct VeluneRootView: View {
    @ObservedObject var store: AppStore
    let previewEmpty: Bool
    let previewSettings: Bool
    @Environment(\.openSettings) private var openSettings
    @State private var search = ""
    @State private var draft = ""
    private var messages: [Message] { previewEmpty ? [] : store.messages }
    private var conversations: [Conversation] { store.conversations.filter { search.isEmpty || $0.title.localizedCaseInsensitiveContains(search) } }
    private var selectedConnection: Connection? { store.connections.first { $0.id == store.selectedConnectionID } }

    var body: some View {
        NavigationSplitView {
            List(selection: Binding(get: { store.selectedConversationID }, set: { if let id = $0 { store.selectConversation(id: id) } })) {
                Section("会话") {
                    ForEach(conversations) { conversation in
                        VStack(alignment: .leading, spacing: 3) {
                            Text(conversation.title).lineLimit(1)
                            if let date = conversation.updatedAt { Text(shortDate(date)).font(.caption).foregroundStyle(.secondary) }
                        }.padding(.vertical, 3).tag(conversation.id)
                    }
                }
            }
            .listStyle(.sidebar)
            .searchable(text: $search, placement: .sidebar, prompt: "搜索会话")
            .navigationTitle("会话")
            .navigationSplitViewColumnWidth(min: 190, ideal: 240, max: 340)
            .disabled(store.isGenerating)
            .toolbar {
                ToolbarItem { Button(action: { store.createConversation() }) { Label("新建会话", systemImage: "square.and.pencil") }.help("新建会话（⌘ N）").disabled(store.isGenerating || store.isLoading) }
            }
        } detail: {
            VStack(spacing: 0) {
                if messages.isEmpty { emptyState.frame(maxWidth: .infinity, maxHeight: .infinity) }
                else { transcript }
                Divider()
                composer
            }
            .navigationTitle(previewEmpty ? "新会话" : store.selectedConversationTitle ?? "Velune")
            .toolbar { conversationToolbar }
        }
        .navigationSplitViewStyle(.balanced)
        .onReceive(NotificationCenter.default.publisher(for: .veluneSend)) { _ in send() }
        .task {
            store.start()
            if previewSettings { openSettings() }
        }
    }

    @ToolbarContentBuilder private var conversationToolbar: some ToolbarContent {
        ToolbarItemGroup(placement: .primaryAction) {
            Menu {
                ForEach(store.models.filter { $0.contextWindow != nil }) { model in Button(model.nickname.isEmpty ? model.id : model.nickname) { store.selectModel(modelID: model.id) } }
                Divider()
                SettingsLink { Text("管理AI提供商与模型…") }
            } label: { Text(store.selectedModelName ?? "选择模型") }
            .accessibilityLabel("模型：\(store.selectedModelName ?? "未选择")")
            .help("选择模型（当前：\(store.selectedModelName ?? "未选择")）")
            .disabled(!store.canSwitchModel)
            Menu {
                ForEach(store.connections) { connection in Button(connection.name) { store.selectConnection(id: connection.id) } }
                Divider()
                SettingsLink { Text("Agent 运行时设置…") }
            } label: { Label(selectedConnection?.name ?? "Agent 运行时", systemImage: "desktopcomputer").labelStyle(.titleAndIcon) }
            .accessibilityLabel("Agent 运行时：\(selectedConnection?.name ?? "未连接")")
            .help("选择Agent 运行时（当前：\(selectedConnection?.name ?? "未连接")）")
            .disabled(store.isGenerating || store.isLoading)
        }
    }

    private var emptyState: some View {
        ContentUnavailableView {
            Label { Text("新会话") } icon: { VeluneLogo(size: 48) }
        } description: {
            Text(store.canSend ? "输入消息，开始工作。" : "先在设置中配置AI提供商、模型与Agent 运行时。")
        } actions: {
            if !store.canSend { SettingsLink { Text("打开设置") } }
        }
    }

    private var transcript: some View {
        ScrollViewReader { proxy in
            ScrollView {
                LazyVStack(alignment: .leading, spacing: 24) {
                    ForEach(messages) { message in ConversationMessageView(message: message).id(message.id) }
                    if let activity = store.activity { HStack { ProgressView().controlSize(.small); Text(activity).foregroundStyle(.secondary) }.font(.callout).frame(maxWidth: .infinity) }
                    Color.clear.frame(height: 1).id("bottom")
                }.frame(maxWidth: 760).padding(24).frame(maxWidth: .infinity)
            }
            .onChange(of: store.messages) { _, _ in if store.isGenerating { proxy.scrollTo("bottom", anchor: .bottom) } }
        }
    }

    private var composer: some View {
        VStack(alignment: .leading, spacing: 8) {
            if store.needsModelSelection {
                Text("此会话的模型当前不可用，请选择已配置的模型。").font(.callout).foregroundStyle(.secondary)
            }
            if let error = store.error {
                HStack(alignment: .top) { Label(error, systemImage: "exclamationmark.triangle").textSelection(.enabled); Spacer(); SettingsLink { Text("检查运行时设置") } }.font(.callout).foregroundStyle(.secondary)
            }
            ComposerView(text: $draft, enabled: store.canSend, generating: store.isGenerating, canCancel: store.canCancel, send: send, cancel: store.cancel)
        }.frame(maxWidth: 760).padding(16).frame(maxWidth: .infinity)
    }

    private func send() {
        let text = draft.trimmingCharacters(in: .whitespacesAndNewlines)
        guard store.canSend, !text.isEmpty else { return }
        let submittedDraft = draft
        store.send(text: text) { if draft == submittedDraft { draft = "" } }
    }
    private func shortDate(_ text: String) -> String {
        guard let date = ISO8601DateFormatter().date(from: text) else { return text }
        return date.formatted(.dateTime.month(.abbreviated).day())
    }
}

struct ComposerView: View {
    @Binding var text: String
    let enabled: Bool
    let generating: Bool
    let canCancel: Bool
    let send: () -> Void
    let cancel: () -> Void
    var body: some View {
        VStack(alignment: .leading, spacing: 8) {
            ZStack(alignment: .topLeading) {
                if text.isEmpty { Text("输入消息…").foregroundStyle(.tertiary).padding(.horizontal, 5).padding(.top, 8).allowsHitTesting(false) }
                TextEditor(text: $text).font(.body).scrollContentBackground(.hidden).scrollIndicators(.hidden).frame(height: min(140, max(60, CGFloat(text.split(separator: "\n", omittingEmptySubsequences: false).count) * 20 + 20))).accessibilityLabel("消息内容")
            }.padding(5).background(.background, in: RoundedRectangle(cornerRadius: 6)).overlay(RoundedRectangle(cornerRadius: 6).stroke(.separator, lineWidth: 1))
            HStack {
                Text("⌘ Return 发送，Return 换行").font(.caption).foregroundStyle(.secondary)
                Spacer()
                if generating { Button("停止生成", systemImage: "stop.fill", action: cancel).disabled(!canCancel) }
                else { Button("发送", systemImage: "arrow.up", action: send).buttonStyle(.borderedProminent).disabled(!enabled || text.trimmingCharacters(in: .whitespacesAndNewlines).isEmpty) }
            }
        }
    }
}

struct ConversationMessageView: View {
    let message: Message
    var body: some View {
        VStack(alignment: .leading, spacing: 9) {
            ForEach(Array(message.blocks.enumerated()), id: \.offset) { _, block in
                if block.kind == "tool" {
                    ToolDisclosure(title: block.title ?? "工具执行", detail: block.text ?? "", state: block.state ?? "done")
                } else if let text = block.text, !text.isEmpty {
                    if block.kind == "notice" || (message.role != "assistant" && message.role != "user") {
                        Text(text).font(.callout).foregroundStyle(.secondary).textSelection(.enabled).multilineTextAlignment(.center).frame(maxWidth: .infinity)
                    } else if message.role == "user" {
                        HStack { Spacer(minLength: 60); MessageText(text: text, alignment: .trailing).frame(maxWidth: 600) }
                    } else {
                        MessageText(text: text)
                    }
                }
            }
        }
    }
}

struct MessageText: View {
    let text: String
    var alignment: HorizontalAlignment = .leading
    var body: some View {
        VStack(alignment: alignment, spacing: 12) {
            ForEach(Array(text.components(separatedBy: "```").enumerated()), id: \.offset) { index, segment in
                if index % 2 == 1 {
                    let lines = segment.split(separator: "\n", omittingEmptySubsequences: false)
                    CodeBlockView(language: String(lines.first ?? "").trimmingCharacters(in: .whitespaces), code: lines.dropFirst().joined(separator: "\n").trimmingCharacters(in: .newlines))
                } else if !segment.trimmingCharacters(in: .whitespacesAndNewlines).isEmpty {
                    Text((try? AttributedString(markdown: segment.trimmingCharacters(in: .newlines), options: .init(interpretedSyntax: .inlineOnlyPreservingWhitespace))) ?? AttributedString(segment)).font(.body).lineSpacing(4).textSelection(.enabled).multilineTextAlignment(alignment == .trailing ? .trailing : .leading).frame(maxWidth: .infinity, alignment: alignment == .trailing ? .trailing : .leading)
                }
            }
        }
    }
}

struct CodeBlockView: View {
    let language: String
    let code: String
    @State private var copied = false
    var body: some View {
        VStack(alignment: .leading, spacing: 8) {
            HStack { Text(language.isEmpty ? "代码" : language).font(.caption).foregroundStyle(.secondary); Spacer(); Button(copied ? "已复制" : "复制", systemImage: copied ? "checkmark" : "doc.on.doc") { NSPasteboard.general.clearContents(); NSPasteboard.general.setString(code, forType: .string); copied = true; DispatchQueue.main.asyncAfter(deadline: .now() + 2) { copied = false } }.controlSize(.small) }
            ScrollView(.horizontal) { Text(code).font(.system(.body, design: .monospaced)).textSelection(.enabled).frame(maxWidth: .infinity, alignment: .leading) }
        }.padding(12).background(.quaternary.opacity(0.45), in: RoundedRectangle(cornerRadius: 6))
    }
}

struct ToolDisclosure: View {
    let title: String
    let detail: String
    let state: String
    @State private var expanded = false
    var body: some View {
        DisclosureGroup(isExpanded: $expanded) {
            Text(detail.isEmpty ? "没有附加输出" : detail).font(.system(.callout, design: .monospaced)).foregroundStyle(.secondary).textSelection(.enabled).padding(.top, 4)
        } label: {
            Label(title, systemImage: state == "running" ? "circle.dotted" : state == "error" ? "exclamationmark.circle" : "checkmark.circle").font(.callout).foregroundStyle(.secondary)
        }.frame(maxWidth: expanded ? 560 : nil).fixedSize(horizontal: !expanded, vertical: false).frame(maxWidth: .infinity)
    }
}


struct SettingsView: View {
    @ObservedObject var store: AppStore
    var body: some View {
        TabView {
            ProviderSettingsView(store: store).tabItem { Label("AI提供商", systemImage: "network") }
            ModelSettingsView(store: store).tabItem { Label("模型", systemImage: "cube") }
            RoutingSettingsView(store: store).tabItem { Label("模型路由", systemImage: "arrow.triangle.branch") }
            RuntimeSettingsView(store: store).tabItem { Label("Agent 运行时", systemImage: "terminal") }
        }.frame(width: 690, height: 560)
    }
}

struct SettingsError: View {
    let message: String?
    var body: some View {
        if let message { Label(message, systemImage: "exclamationmark.triangle").font(.callout).foregroundStyle(.secondary).textSelection(.enabled).padding(.horizontal, 20).padding(.bottom, 12) }
    }
}

struct ProviderSettingsView: View {
    @ObservedObject var store: AppStore
    @State private var selectedID: String?
    @State private var editor: AIProvider?
    @State private var creating = false
    @State private var importing = false
    @State private var deleting: AIProvider?
    var body: some View {
        VStack(spacing: 0) {
            List(selection: $selectedID) {
                ForEach(store.providers) { provider in
                    VStack(alignment: .leading, spacing: 3) { Text(provider.name); Text(provider.endpoint).font(.caption).foregroundStyle(.secondary).lineLimit(1) }.tag(provider.id)
                        .contextMenu { Button("编辑…") { editor = provider }; Button("删除…") { deleting = provider } }
                }
            }.listStyle(.bordered).padding(.horizontal, 20).padding(.top, 16)
            HStack {
                Button { creating = true } label: { Image(systemName: "plus") }.help("添加AI提供商")
                Button { deleting = store.providers.first { $0.id == selectedID } } label: { Image(systemName: "minus") }.disabled(selectedID == nil).help("删除AI提供商")
                Button("从运行时导入…") { importing = true }.disabled(store.providerImportTypes.isEmpty || store.isLoading || store.isGenerating || store.authenticationRunning)
                Spacer()
                Button("编辑…") { editor = store.providers.first { $0.id == selectedID } }.disabled(selectedID == nil)
            }.padding(.horizontal, 20).padding(.vertical, 12)
            SettingsError(message: store.error)
        }
        .sheet(isPresented: $importing) { ProviderImportView(store: store) }
        .sheet(item: $editor) { provider in ProviderEditor(provider: provider, models: store.models, protocols: store.protocols, sourceTypes: store.credentialSourceTypes, authenticate: store.startAuthentication, inspect: store.inspectAuthentication, isSaving: store.isLoading, error: store.error) { value, secret, onSaved in store.saveProvider(value, secret: secret, onSaved: onSaved) }.sheet(isPresented: $store.showsAuthentication) { AuthenticationView(store: store) } }
        .sheet(isPresented: $creating) { ProviderEditor(provider: nil, models: store.models, protocols: store.protocols, sourceTypes: store.credentialSourceTypes, authenticate: store.startAuthentication, inspect: store.inspectAuthentication, isSaving: store.isLoading, error: store.error) { value, secret, onSaved in store.saveProvider(value, secret: secret, onSaved: onSaved) }.sheet(isPresented: $store.showsAuthentication) { AuthenticationView(store: store) } }
        .alert("删除AI提供商？", isPresented: Binding(get: { deleting != nil }, set: { if !$0 { deleting = nil } }), presenting: deleting) { provider in
            Button("删除", role: .destructive) { store.deleteProvider(id: provider.id); deleting = nil }; Button("取消", role: .cancel) { deleting = nil }
        } message: { provider in Text("删除「\(provider.name)」的配置。模型定义与会话不受影响。") }
    }
}

struct ProviderEditor: View {
    let provider: AIProvider?
    let models: [AIModel]
    let protocols: [ProtocolDescriptor]
    let sourceTypes: [CredentialSourceType]
    let authenticate: (CredentialSource) -> Void
    let inspect: (CredentialSource, @escaping (AuthenticationMetadata) -> Void) -> Void
    let isSaving: Bool
    let error: String?
    let save: (AIProvider, String, (() -> Void)?) -> Void
    @Environment(\.dismiss) private var dismiss
    @State private var name = ""
    @State private var protocolID: ProviderProtocol?
    @State private var endpoint = ""
    @State private var credentialRef = ""
    @State private var secret = ""
    @State private var sourceTypeID = ""
    @State private var sourceValues: [String: String] = [:]
    @State private var sourceMetadata: AuthenticationMetadata?
    @State private var bindings: [String: String] = [:]
    @State private var draftID = UUID().uuidString
    private var sourceDescriptor: CredentialSourceType? { sourceTypes.first { $0.id == sourceTypeID } }
    private var sourceValid: Bool { sourceTypeID.isEmpty || (sourceDescriptor?.fields.allSatisfy { field in
        let value = sourceValues[field.key] ?? field.value
        return (!field.required || !value.trimmingCharacters(in: .whitespacesAndNewlines).isEmpty) && MacPath.isValid(value, field: field)
    } ?? false) }
    private var draftSource: CredentialSource? {
        sourceDescriptor.map { descriptor in
            var settings = sourceValues
            settings.removeValue(forKey: "providerId")
            for field in descriptor.fields where field.key != "providerId" { settings[field.key] = MacPath.normalized(sourceValues[field.key] ?? field.value, field: field) }
            return CredentialSource(harnessTypeID: descriptor.id, sourceInstanceID: provider?.credentialSource?.sourceInstanceID, providerID: (sourceValues["providerId"] ?? descriptor.fields.first { $0.key == "providerId" }?.value ?? "").trimmingCharacters(in: .whitespacesAndNewlines), settings: settings)
        }
    }
    private var protocolSupported: Bool { protocols.first { $0.id == protocolID }?.supported == true }
    private var valid: Bool { !name.trimmingCharacters(in: .whitespacesAndNewlines).isEmpty && !endpoint.trimmingCharacters(in: .whitespacesAndNewlines).isEmpty && protocolSupported && sourceValid && bindings.values.allSatisfy { !$0.trimmingCharacters(in: .whitespacesAndNewlines).isEmpty } }
    var body: some View {
        VStack(spacing: 0) {
            Text(provider == nil ? "添加AI提供商" : "编辑AI提供商").font(.headline).frame(maxWidth: .infinity, alignment: .leading).padding(20)
            Form {
                Section {
                    TextField("名称", text: $name)
                    Picker("协议", selection: $protocolID) {
                        Text("请选择").tag(Optional<ProviderProtocol>.none)
                        ForEach(protocols) { descriptor in Text(descriptor.name + (descriptor.supported ? "" : "（尚未支持）")).tag(Optional(descriptor.id)).disabled(!descriptor.supported) }
                    }
                    TextField("服务地址", text: $endpoint)
                    Picker("认证方式", selection: $sourceTypeID) {
                        Text("API key（Keychain）").tag("")
                        ForEach(sourceTypes) { type in Text(type.name).tag(type.id) }
                    }
                    if sourceTypeID.isEmpty {
                        TextField("凭据名称", text: $credentialRef, prompt: Text("Keychain中的引用"))
                        SecureField("API key", text: $secret, prompt: Text("留空保留已有凭据"))
                    } else if let descriptor = sourceDescriptor {
                        ForEach(descriptor.fields) { field in
                            SettingFieldView(field: field, value: Binding(get: { sourceValues[field.key] ?? field.value }, set: { sourceValues[field.key] = $0 }))
                        }
                        Text("使用指定来源的认证；原配置保留，凭据不会复制到 Velune 配置文件。").font(.caption).foregroundStyle(.secondary)
                        Button("读取来源信息") { if let source = draftSource { inspect(source) { metadata in guard source == draftSource else { return }; sourceMetadata = metadata; protocolID = metadata.capabilities.protocol; endpoint = metadata.capabilities.endpoint } } }.disabled(!sourceValid || isSaving)
                        if let metadata = sourceMetadata {
                            Text(metadata.configured ? "来源已配置认证" : "来源尚未配置可用认证").font(.caption).foregroundStyle(.secondary)
                            if !metadata.capabilities.explicitOutputCap { Text("此来源不支持服务端输出 token 上限。").font(.caption).foregroundStyle(.secondary) }
                        }
                        ForEach((sourceMetadata?.actions ?? descriptor.actions).filter { $0.id == "login" }) { action in
                            Button(action.label) { if let source = draftSource { authenticate(source) } }.disabled(!sourceValid || isSaving)
                        }
                    }
                }
                Section("关联模型") {
                    ForEach(models) { model in
                        Toggle(model.nickname.isEmpty ? model.id : model.nickname, isOn: Binding(get: { bindings[model.id] != nil }, set: { if $0 { bindings[model.id] = model.id } else { bindings.removeValue(forKey: model.id) } }))
                        if bindings[model.id] != nil { TextField("提供商模型ID", text: Binding(get: { bindings[model.id] ?? "" }, set: { bindings[model.id] = $0 })) }
                    }
                    if models.isEmpty { Text("先在模型页定义模型，再关联到提供商。").foregroundStyle(.secondary) }
                }
            }.formStyle(.grouped)
            HStack { Spacer(); Button("取消") { dismiss() }.keyboardShortcut(.cancelAction); Button("保存") { commit() }.keyboardShortcut(.defaultAction).disabled(!valid || isSaving) }.padding(20)
            SettingsError(message: error)
        }.frame(width: 580, height: 520)
        .onChange(of: sourceValues) { _, _ in sourceMetadata = nil }
        .onChange(of: sourceTypeID) { _, _ in sourceMetadata = nil }
        .onAppear { name = provider?.name ?? ""; protocolID = provider?.protocolID ?? protocols.first { $0.supported }?.id; endpoint = provider?.endpoint ?? ""; credentialRef = provider?.credentialRef ?? ""; sourceTypeID = provider?.credentialSource?.harnessTypeID ?? ""; sourceValues = provider?.credentialSource?.settings ?? [:]; if let source = provider?.credentialSource { sourceValues["providerId"] = source.providerID }; bindings = Dictionary(uniqueKeysWithValues: (provider?.models ?? []).map { ($0.modelID, $0.externalModelID) }) }
    }
    private func commit() {
        guard valid, let protocolID else { return }
        let source = draftSource
        let value = AIProvider(id: provider?.id ?? draftID, name: name.trimmingCharacters(in: .whitespacesAndNewlines), protocolID: protocolID, endpoint: endpoint.trimmingCharacters(in: .whitespacesAndNewlines), credentialRef: source == nil && !credentialRef.isEmpty ? credentialRef : nil, models: bindings.keys.sorted().map { id in let external = bindings[id]!.trimmingCharacters(in: .whitespacesAndNewlines); let existing = provider?.models.first { $0.modelID == id && $0.externalModelID == external }; return ProviderModelBinding(modelID: id, externalModelID: external, adapterMetadataJSON: existing?.adapterMetadataJSON) }, credentialSource: source, credentialGeneration: provider?.credentialGeneration)
        save(value, source == nil ? secret : "") { secret = ""; dismiss() }
    }
}

struct ModelSettingsView: View {
    @ObservedObject var store: AppStore
    @State private var selectedID: String?
    @State private var editor: AIModel?
    @State private var creating = false
    @State private var deleting: AIModel?
    var body: some View {
        VStack(spacing: 0) {
            List(selection: $selectedID) {
                ForEach(store.models) { model in Label { VStack(alignment: .leading, spacing: 3) { Text(model.nickname.isEmpty ? model.id : model.nickname); Text(model.id).font(.caption).foregroundStyle(.secondary) } } icon: { Image(systemName: model.icon ?? "cube") }.tag(model.id).contextMenu { Button("编辑…") { editor = model }; Button("删除…") { deleting = model } } }
            }.listStyle(.bordered).padding(.horizontal, 20).padding(.top, 16)
            HStack { Button { creating = true } label: { Image(systemName: "plus") }.help("添加模型"); Button { deleting = store.models.first { $0.id == selectedID } } label: { Image(systemName: "minus") }.disabled(selectedID == nil).help("删除模型"); Spacer(); Button("编辑…") { editor = store.models.first { $0.id == selectedID } }.disabled(selectedID == nil) }.padding(.horizontal, 20).padding(.vertical, 12)
            SettingsError(message: store.error)
        }
        .sheet(item: $editor) { model in ModelEditor(model: model, isSaving: store.isLoading, error: store.error, save: store.saveModel) }
        .sheet(isPresented: $creating) { ModelEditor(model: nil, isSaving: store.isLoading, error: store.error, save: store.saveModel) }
        .alert("删除模型？", isPresented: Binding(get: { deleting != nil }, set: { if !$0 { deleting = nil } }), presenting: deleting) { model in Button("删除", role: .destructive) { store.deleteModel(id: model.id); deleting = nil }; Button("取消", role: .cancel) { deleting = nil } } message: { model in Text("删除「\(model.nickname.isEmpty ? model.id : model.nickname)」的模型配置，会话不受影响。") }
    }
}

struct ModelEditor: View {
    let model: AIModel?
    let isSaving: Bool
    let error: String?
    let save: (AIModel, (() -> Void)?) -> Void
    @Environment(\.dismiss) private var dismiss
    @State private var id = ""
    @State private var nickname = ""
    @State private var icon = ""
    @State private var maxOutputTokens = ""
    @State private var contextWindow = ""
    @State private var reasoningLevels = ""
    private var valid: Bool { !id.trimmingCharacters(in: .whitespacesAndNewlines).isEmpty && (UInt32(maxOutputTokens).map { $0 > 0 } ?? false) && (contextWindow.isEmpty || (UInt32(contextWindow).map { $0 >= (UInt32(maxOutputTokens) ?? 0) } ?? false)) }
    var body: some View {
        VStack(spacing: 0) {
            Text(model == nil ? "添加模型" : "编辑模型").font(.headline).frame(maxWidth: .infinity, alignment: .leading).padding(20)
            Form {
                TextField("模型ID", text: $id).disabled(model != nil)
                TextField("昵称", text: $nickname, prompt: Text("可选"))
                TextField("图标", text: $icon, prompt: Text("可选，SF Symbols名称"))
                TextField("最大输出 Token 数", text: $maxOutputTokens)
                TextField("上下文窗口 Token 数", text: $contextWindow, prompt: Text("运行前必须填写"))
                TextField("推理等级", text: $reasoningLevels, prompt: Text("以逗号分隔；留空表示不支持"))
            }.formStyle(.grouped)
            HStack { Spacer(); Button("取消") { dismiss() }.keyboardShortcut(.cancelAction); Button("保存") { commit() }.keyboardShortcut(.defaultAction).disabled(!valid || isSaving) }.padding(20)
            SettingsError(message: error)
        }.frame(width: 540, height: 430)
        .onAppear { id = model?.id ?? ""; nickname = model?.nickname ?? ""; icon = model?.icon ?? ""; maxOutputTokens = model.map { String($0.maxOutputTokens) } ?? ""; contextWindow = model?.contextWindow.map(String.init) ?? ""; reasoningLevels = model?.reasoningLevels.joined(separator: ", ") ?? "" }
    }
    private func commit() {
        guard valid, let maximum = UInt32(maxOutputTokens) else { return }
        let window = contextWindow.isEmpty ? nil : UInt32(contextWindow)
        guard contextWindow.isEmpty || (window.map { $0 >= maximum } ?? false) else { return }
        let levels = reasoningLevels.split(separator: ",").map { $0.trimmingCharacters(in: .whitespacesAndNewlines) }.filter { !$0.isEmpty }
        let modelID = id.trimmingCharacters(in: .whitespacesAndNewlines)
        let displayName = nickname.trimmingCharacters(in: .whitespacesAndNewlines)
        save(AIModel(id: modelID, nickname: displayName.isEmpty ? modelID : displayName, icon: icon.isEmpty ? nil : icon, contextWindow: window, maxOutputTokens: maximum, reasoningLevels: levels)) { dismiss() }
    }
}

struct RoutingSettingsView: View {
    @ObservedObject var store: AppStore
    var body: some View {
        Form {
            Section {
                ForEach(store.models) { model in
                    Picker(model.nickname.isEmpty ? model.id : model.nickname, selection: Binding(get: { store.routes.first { $0.modelID == model.id }?.providerID ?? "" }, set: { store.saveRoute(modelID: model.id, providerID: $0) })) {
                        Text("请选择提供商").tag("").disabled(true)
                        ForEach(store.providers.filter { $0.models.contains { $0.modelID == model.id } }) { provider in Text(provider.name).tag(provider.id).disabled(store.protocols.first { $0.id == provider.protocolID }?.supported != true) }
                    }.disabled(store.isLoading || store.isGenerating)
                }
                if store.models.isEmpty { Text("先定义模型并关联AI提供商，再选择模型路由。").foregroundStyle(.secondary) }
            } footer: {
                Text("请求使用所选提供商；失败时不会自动切换。")
            }
            if let error = store.error { Label(error, systemImage: "exclamationmark.triangle").foregroundStyle(.secondary).textSelection(.enabled) }
        }.formStyle(.grouped)
    }
}

struct RuntimeSettingsView: View {
    @ObservedObject var store: AppStore
    @State private var selectedID: String?
    @State private var editor: RuntimeInstance?
    @State private var creating = false
    @State private var deleting: RuntimeInstance?
    private var selected: RuntimeInstance? { store.runtimeInstances.first { $0.id == selectedID } }
    var body: some View {
        VStack(spacing: 0) {
            List(selection: $selectedID) {
                ForEach(store.runtimeInstances) { instance in
                    VStack(alignment: .leading, spacing: 3) { Text(instance.name); Text(store.runtimeTypes.first { $0.id == instance.typeID }?.name ?? instance.typeID).font(.caption).foregroundStyle(.secondary) }.tag(instance.id).contextMenu { Button("编辑…") { editor = instance }; Button("删除…") { deleting = instance } }
                }
            }.listStyle(.bordered).padding(.horizontal, 20).padding(.top, 16)
            HStack {
                Button { creating = true } label: { Image(systemName: "plus") }.help("添加Agent 运行时实例")
                Button { deleting = selected } label: { Image(systemName: "minus") }.disabled(selected == nil).help("删除运行时实例")
                Spacer()
                if let instance = selected, let descriptor = store.runtimeTypes.first(where: { $0.id == instance.typeID }) {
                    ForEach(descriptor.actions) { action in Button(action.label) { store.performRuntimeAction(instanceID: instance.id, actionID: action.id) }.disabled(store.isLoading || store.isGenerating) }
                }
                Button("编辑…") { editor = selected }.disabled(selected == nil)
            }.padding(.horizontal, 20).padding(.vertical, 12)
            SettingsError(message: store.error)
        }
        .sheet(item: $editor) { instance in RuntimeEditor(instance: instance, types: store.runtimeTypes, models: store.models, gatewayID: store.gateway.id, isSaving: store.isLoading, error: store.error, save: store.saveRuntimeInstance) }
        .sheet(isPresented: $creating) { RuntimeEditor(instance: nil, types: store.runtimeTypes, models: store.models, gatewayID: store.gateway.id, isSaving: store.isLoading, error: store.error, save: store.saveRuntimeInstance) }
        .alert("删除运行时实例？", isPresented: Binding(get: { deleting != nil }, set: { if !$0 { deleting = nil } }), presenting: deleting) { instance in Button("删除", role: .destructive) { store.deleteRuntimeInstance(id: instance.id); deleting = nil }; Button("取消", role: .cancel) { deleting = nil } } message: { instance in Text("删除「\(instance.name)」的配置，不删除运行时保存的会话。") }
    }
}

struct RuntimeEditor: View {
    let instance: RuntimeInstance?
    let types: [RuntimeTypeDescriptor]
    let models: [AIModel]
    let gatewayID: String
    let isSaving: Bool
    let error: String?
    let save: (RuntimeInstance, (() -> Void)?) -> Void
    @Environment(\.dismiss) private var dismiss
    @State private var name = ""
    @State private var typeID = ""
    @State private var modelID: String?
    @State private var settings: [String: String] = [:]
    @State private var draftID = UUID().uuidString
    @State private var executableDiscoveryRunning = false
    @State private var executableDiscoveryMessage: String?
    private var descriptor: RuntimeTypeDescriptor? { types.first { $0.id == typeID } }
    private var valid: Bool { !name.trimmingCharacters(in: .whitespacesAndNewlines).isEmpty && descriptor != nil && (descriptor?.fields.allSatisfy { field in
        let value = settings[field.key] ?? field.value
        return (!field.required || !value.trimmingCharacters(in: .whitespacesAndNewlines).isEmpty) && MacPath.isValid(value, field: field)
    } ?? false) }
    private var pathValidationMessage: String? {
        guard let field = descriptor?.fields.first(where: { !MacPath.isValid(settings[$0.key] ?? $0.value, field: $0) }) else { return nil }
        return "「\(field.label)」必须是绝对路径；可使用 ~/ 开头，保存时会展开。"
    }
    var body: some View {
        VStack(spacing: 0) {
            Text(instance == nil ? "添加Agent 运行时" : "编辑Agent 运行时").font(.headline).frame(maxWidth: .infinity, alignment: .leading).padding(20)
            Form {
                Section {
                    TextField("实例名称", text: $name)
                    Picker("类型", selection: $typeID) { ForEach(types) { type in Text(type.name).tag(type.id) } }.disabled(instance != nil)
                    Picker("初始模型", selection: $modelID) { Text("稍后选择").tag(Optional<String>.none); ForEach(models) { model in Text(model.nickname.isEmpty ? model.id : model.nickname).tag(Optional(model.id)) } }
                }
                if let descriptor {
                    Section("实例配置") {
                        ForEach(descriptor.fields) { field in
                            SettingFieldView(
                                field: field,
                                value: Binding(get: { settings[field.key] ?? field.value }, set: { settings[field.key] = $0 }),
                                discoveryTitle: discoveryTitle(for: field),
                                discoveryAction: discoveryAction(for: field)
                            )
                        }
                        if let executableDiscoveryMessage {
                            Text(executableDiscoveryMessage).font(.caption).foregroundStyle(.secondary)
                        }
                    }
                }
            }.formStyle(.grouped)
            HStack { Spacer(); Button("取消") { dismiss() }.keyboardShortcut(.cancelAction); Button("保存") { commit() }.keyboardShortcut(.defaultAction).disabled(!valid || isSaving) }.padding(20)
            if let pathValidationMessage { Text(pathValidationMessage).font(.caption).foregroundStyle(.red).padding(.horizontal, 20) }
            SettingsError(message: error)
        }.frame(width: 590, height: 540)
        .onAppear {
            name = instance?.name ?? ""
            typeID = instance?.typeID ?? types.first?.id ?? ""
            settings = instance?.settings ?? [:]
            if let descriptor {
                for field in descriptor.fields where field.kind == .filePath || field.kind == .directoryPath {
                    if let value = settings[field.key] { settings[field.key] = MacPath.expanded(value) }
                }
            }
            modelID = instance?.modelID
            DispatchQueue.main.async { discoverExecutableIfNeeded() }
        }
        .onChange(of: typeID) { _, _ in
            executableDiscoveryMessage = nil
            DispatchQueue.main.async { discoverExecutableIfNeeded() }
        }
    }
    private func discoveryField() -> SettingField? { descriptor?.fields.first { $0.executableDiscovery != nil } }
    private func discoveryValue(for field: SettingField) -> String { (settings[field.key] ?? field.value).trimmingCharacters(in: .whitespacesAndNewlines) }
    private func discoveryTitle(for field: SettingField) -> String? {
        guard field.executableDiscovery != nil, discoveryValue(for: field).isEmpty else { return nil }
        return executableDiscoveryRunning ? "探测中…" : (executableDiscoveryMessage == nil ? "自动发现" : "重新发现")
    }
    private func discoveryAction(for field: SettingField) -> (() -> Void)? {
        guard field.executableDiscovery != nil, discoveryValue(for: field).isEmpty, !executableDiscoveryRunning else { return nil }
        return discoverExecutableIfNeeded
    }
    private func discoverExecutableIfNeeded() {
        guard !executableDiscoveryRunning, let field = discoveryField(), let discovery = field.executableDiscovery,
              discoveryValue(for: field).isEmpty else { return }
        executableDiscoveryRunning = true
        executableDiscoveryMessage = "正在查找可执行文件…"
        MacExecutableDiscovery.discover(discovery) { result in
            executableDiscoveryRunning = false
            switch result {
            case .success(let path):
                // A manual edit wins if it happened while discovery was running.
                if discoveryValue(for: field).isEmpty { settings[field.key] = path; executableDiscoveryMessage = "已发现：\(path)" }
            case .failure(let error): executableDiscoveryMessage = error.localizedDescription
            }
        }
    }
    private func commit() {
        guard valid, let descriptor else { return }
        var values = instance?.settings ?? [:]
        for field in descriptor.fields {
            values[field.key] = MacPath.normalized(settings[field.key] ?? field.value, field: field)
        }
        save(RuntimeInstance(id: instance?.id ?? draftID, name: name.trimmingCharacters(in: .whitespacesAndNewlines), typeID: typeID, gatewayID: instance?.gatewayID ?? gatewayID, settings: values, modelID: modelID)) { dismiss() }
    }
}

private enum MacPath {
    static func expanded(_ value: String) -> String {
        NSString(string: value.trimmingCharacters(in: .whitespacesAndNewlines)).expandingTildeInPath
    }

    static func normalized(_ value: String, field: SettingField) -> String {
        guard field.kind == .filePath || field.kind == .directoryPath else {
            return value.trimmingCharacters(in: .whitespacesAndNewlines)
        }
        return URL(fileURLWithPath: expanded(value)).standardizedFileURL.path
    }

    static func isValid(_ value: String, field: SettingField) -> Bool {
        guard field.kind == .filePath || field.kind == .directoryPath else { return true }
        let trimmed = value.trimmingCharacters(in: .whitespacesAndNewlines)
        guard !trimmed.isEmpty else { return !field.required }
        let path = expanded(trimmed)
        return path.hasPrefix("/") && !path.contains("\0")
    }
}

struct SettingFieldView: View {
    let field: SettingField
    @Binding var value: String
    var discoveryTitle: String? = nil
    var discoveryAction: (() -> Void)? = nil
    var body: some View {
        if field.kind == .choice {
            Picker(field.label, selection: $value) { ForEach(field.options) { option in Text(option.label).tag(option.id) } }.help(field.help ?? field.label)
        } else {
            LabeledContent(field.label) {
                HStack {
                    TextField(field.required ? "必填" : "可选", text: $value).textFieldStyle(.roundedBorder)
                    if field.kind == .filePath || field.kind == .directoryPath { Button("选择…", action: choosePath) }
                    if let discoveryTitle, let discoveryAction { Button(discoveryTitle, action: discoveryAction).disabled(discoveryTitle == "探测中…") }
                }
            }.help(field.help ?? field.label)
        }
    }
    private func choosePath() {
        let panel = NSOpenPanel(); panel.canChooseDirectories = field.kind == .directoryPath; panel.canChooseFiles = field.kind == .filePath; panel.allowsMultipleSelection = false
        if panel.runModal() == .OK, let url = panel.url { value = url.path }
    }
}

struct AuthenticationView: View {
    @ObservedObject var store: AppStore
    @Environment(\.dismiss) private var dismiss
    @State private var answer = ""
    var body: some View {
        VStack(alignment: .leading, spacing: 16) {
            Text("认证来源登录").font(.headline)
            ScrollView {
                VStack(alignment: .leading, spacing: 12) {
                    ForEach(Array(store.authenticationNotifications.enumerated()), id: \.offset) { _, notice in
                        if let text = notice.text { Text(text).textSelection(.enabled) }
                        if let instructions = notice.instructions { Text(instructions) }
                        if let code = notice.userCode { LabeledContent("验证码", value: code).textSelection(.enabled) }
                        if let address = notice.url ?? notice.verificationUri, let url = URL(string: address), url.scheme == "https" {
                            Link("在浏览器中继续", destination: url)
                        }
                    }
                    if let prompt = store.authenticationPrompt {
                        Text(prompt.text)
                        if let options = prompt.options {
                            Picker("选择", selection: $answer) { Text("请选择").tag(""); ForEach(options) { Text($0.label).tag($0.id) } }
                        } else if prompt.kind == "secret" { SecureField("输入", text: $answer) }
                        else { TextField("输入", text: $answer) }
                        Button("继续") { store.answerAuthentication(id: prompt.id, value: answer); answer = "" }.disabled(answer.isEmpty || store.isLoading)
                    }
                    if let result = store.authenticationResult { Text(result) }
                }.frame(maxWidth: .infinity, alignment: .leading)
            }
            SettingsError(message: store.error)
            HStack {
                if store.authenticationRunning { ProgressView().controlSize(.small) }
                Spacer()
                if store.authenticationRunning { Button("取消登录") { store.cancelAuthentication() }.disabled(store.isLoading) }
                else { Button("完成") { dismiss() }.keyboardShortcut(.defaultAction) }
            }
        }.padding(24).frame(width: 560, height: 450)
        .interactiveDismissDisabled(store.authenticationRunning)
        .onChange(of: store.authenticationPrompt?.id) { _, _ in answer = "" }
    }
}
