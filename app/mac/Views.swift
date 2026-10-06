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
        .sheet(item: Binding(get: { store.pendingInteractions.first }, set: { _ in })) { RuntimeInteractionView(store: store, interaction: $0) }
        .onReceive(NotificationCenter.default.publisher(for: .veluneSend)) { _ in send() }
        .task {
            store.start()
            if previewSettings { openSettings() }
        }
    }

    @ToolbarContentBuilder private var conversationToolbar: some ToolbarContent {
        ToolbarItemGroup(placement: .primaryAction) {
            Menu {
                ForEach(store.runtimeCompatibleModels) { model in Button(model.displayName) { store.selectModel(modelRecordKey: model.recordKey) } }
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

// Limit the workaround to expansion writes; content controls keep their own transactions.
struct ImmediateDisclosureGroup<Label: View, Content: View>: View {
    @State private var expanded = false
    private var externalExpansion: Binding<Bool>?
    private let content: Content
    private let label: Label

    init(isExpanded: Binding<Bool>? = nil, @ViewBuilder content: () -> Content, @ViewBuilder label: () -> Label) {
        externalExpansion = isExpanded
        self.content = content()
        self.label = label()
    }

    private var expansion: Binding<Bool> {
        Binding(
            get: { externalExpansion?.wrappedValue ?? expanded },
            set: { value in
                var transaction = Transaction(animation: nil)
                transaction.disablesAnimations = true
                let target = externalExpansion ?? $expanded
                target.transaction(transaction).wrappedValue = value
            }
        )
    }

    var body: some View {
        DisclosureGroup(isExpanded: expansion) { content } label: { label }
    }
}

extension ImmediateDisclosureGroup where Label == Text {
    init(_ title: String, @ViewBuilder content: () -> Content) {
        self.init(content: content, label: { Text(title) })
    }
}

struct ToolDisclosure: View {
    let title: String
    let detail: String
    let state: String
    @State private var expanded = false
    var body: some View {
        ImmediateDisclosureGroup(isExpanded: $expanded) {
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
    @State private var templates = false
    @State private var deleting: AIProvider?
    var body: some View {
        VStack(spacing: 0) {
            List(selection: $selectedID) {
                ForEach(store.providers) { provider in
                    VStack(alignment: .leading, spacing: 3) {
                        Text(provider.name)
                        Text("\(provider.models.count) 个模型 · \(provider.endpoint)").font(.caption).foregroundStyle(.secondary).lineLimit(1)
                    }.tag(provider.id).contextMenu { Button("编辑…") { editor = provider }; Button("删除…") { deleting = provider } }
                }
            }.listStyle(.bordered).padding(.horizontal, 20).padding(.top, 16)
            HStack {
                Button { creating = true } label: { Image(systemName: "plus") }.help("添加 AI 提供商")
                Button { deleting = store.providers.first { $0.id == selectedID } } label: { Image(systemName: "minus") }.disabled(selectedID == nil)
                Button("从运行时导入…") { importing = true }.disabled(store.providerImportTypes.isEmpty || store.isBusy)
                Menu { Button("模型模板…") { templates = true } } label: { Image(systemName: "ellipsis") }.menuStyle(.borderlessButton).frame(width: 24)
                Spacer()
                Button("编辑…") { editor = store.providers.first { $0.id == selectedID } }.disabled(selectedID == nil)
            }.padding(.horizontal, 20).padding(.vertical, 12)
            SettingsError(message: store.error)
        }
        .sheet(isPresented: $importing) { ProviderImportView(store: store) }
        .sheet(item: $editor) { ProviderEditor(store: store, provider: $0) }
        .sheet(isPresented: $creating) { ProviderEditor(store: store, provider: nil) }
        .sheet(isPresented: $templates) { ModelTemplatesView(store: store) }
        .alert("删除 AI 提供商？", isPresented: Binding(get: { deleting != nil }, set: { if !$0 { deleting = nil } }), presenting: deleting) { provider in
            Button("删除", role: .destructive) { store.deleteProvider(id: provider.id); deleting = nil }; Button("取消", role: .cancel) { deleting = nil }
        } message: { provider in Text("删除「\(provider.name)」及其模型配置。使用这些模型的运行时需要重新选择模型；原运行时文件与会话保留。") }
    }
}

private struct ProviderModelDraft: Identifiable {
    let id = UUID()
    var recordKey = ""
    var providerModelID = ""
    var nickname = ""
    var icon = ""
    var contextWindow = ""
    var maxOutputTokens = ""
    var declaresReasoning = false
    var reasoningLevels = ""
    var adapterMetadataJSON: String?
    init() {}
    init(_ model: ProviderModel) {
        recordKey = model.recordKey; providerModelID = model.providerModelID; nickname = model.nickname; icon = model.icon ?? ""
        contextWindow = model.contextWindow.map(String.init) ?? ""; maxOutputTokens = model.maxOutputTokens.map(String.init) ?? ""
        declaresReasoning = model.reasoningLevels != nil; reasoningLevels = model.reasoningLevels?.joined(separator: ", ") ?? ""
        adapterMetadataJSON = model.adapterMetadataJSON
    }
    var displayName: String { nickname.isEmpty ? (providerModelID.isEmpty ? "新模型" : providerModelID) : nickname }
    var valid: Bool { !providerModelID.isEmpty && validOptionalTokenCount(contextWindow) && validOptionalTokenCount(maxOutputTokens) && !(UInt32(maxOutputTokens).map { output in UInt32(contextWindow).map { output > $0 } ?? false } ?? false) }
    var efforts: [String]? { declaresReasoning ? reasoningLevels.split(separator: ",").map { $0.trimmingCharacters(in: .whitespacesAndNewlines) }.filter { !$0.isEmpty } : nil }
    var capabilitySummary: String {
        let effortsSummary = efforts.map { $0.isEmpty ? "不支持推理 effort" : "支持 effort：" + $0.joined(separator: "、") } ?? "推理 effort 未知"
        return effortsSummary + (contextWindow.isEmpty ? "" : " · 上下文 \(contextWindow)") + (maxOutputTokens.isEmpty ? "" : " · 输出 \(maxOutputTokens)")
    }
    var definition: ProviderModel { ProviderModel(recordKey: recordKey, providerModelID: providerModelID, nickname: nickname, icon: icon.isEmpty ? nil : icon, contextWindow: UInt32(contextWindow), maxOutputTokens: UInt32(maxOutputTokens), reasoningLevels: efforts, adapterMetadataJSON: adapterMetadataJSON) }
}
private func validOptionalTokenCount(_ value: String) -> Bool { value.isEmpty || (UInt32(value).map { $0 > 0 } ?? false) }

private struct ProviderModelFields: View {
    @Binding var draft: ProviderModelDraft
    var body: some View {
        Section("模型") {
            TextField("模型 ID", text: $draft.providerModelID, prompt: Text("提供商 API 规定的标识"))
            TextField("显示名称", text: $draft.nickname, prompt: Text("可选"))
        }
        Section {
            ImmediateDisclosureGroup {
                VStack(alignment: .leading, spacing: 12) {
                    TextField("上下文窗口", text: $draft.contextWindow, prompt: Text("未知"))
                    TextField("最大输出 Token", text: $draft.maxOutputTokens, prompt: Text("未知"))
                    Toggle("声明支持的 reasoning effort", isOn: $draft.declaresReasoning)
                    if draft.declaresReasoning {
                        TextField("支持的 effort", text: $draft.reasoningLevels, prompt: Text("逗号分隔，空白表示不支持"))
                    }
                    Text("以提供商协议为准；此处声明可用能力，不改变会话当前选择的 effort。").font(.caption).foregroundStyle(.secondary)
                    TextField("图标", text: $draft.icon, prompt: Text("可选的 SF Symbol 名称"))
                }.frame(maxWidth: .infinity, alignment: .leading).padding(.top, 8)
            } label: {
                VStack(alignment: .leading, spacing: 3) { Text("能力与 reasoning effort"); Text(draft.capabilitySummary).font(.caption).foregroundStyle(.secondary).fixedSize(horizontal: false, vertical: true) }
            }
        }
    }
}

private enum ProviderEditorSelection: Hashable { case connection, model(UUID) }

struct ProviderEditor: View {
    @ObservedObject var store: AppStore
    let provider: AIProvider?
    @Environment(\.dismiss) private var dismiss
    @State private var name = ""
    @State private var protocolID: ProviderProtocol = .chatCompletionsV1
    @State private var endpoint = ""
    @State private var models: [ProviderModelDraft] = []
    @State private var selection: ProviderEditorSelection? = .connection
    @State private var apiKey = ""
    @State private var originalAPIKey: String?
    @State private var attemptedKeyRead = false
    @State private var showKey = false
    @State private var editingAPIKey = false
    @State private var clearAuthentication = false
    @State private var templateDraft: ModelTemplate?
    @State private var confirmsProjectionRemoval = false
    @State private var oldProtocol: ProviderProtocol = .chatCompletionsV1
    private var authentication: ProviderAuthentication { store.providers.first { $0.id == provider?.id }?.authentication ?? provider?.authentication ?? ProviderAuthentication() }
    private var modelIndex: Int? { guard case .model(let id) = selection else { return nil }; return models.firstIndex { $0.id == id } }
    private var valid: Bool { !name.trimmingCharacters(in: .whitespacesAndNewlines).isEmpty && URL(string: endpoint)?.host != nil && models.allSatisfy(\.valid) && (!editingAPIKey || (!apiKey.isEmpty && (originalAPIKey != nil || authentication.method != .apiKey || clearAuthentication))) }
    var body: some View {
        VStack(spacing: 0) {
            HStack { Text(provider == nil ? "添加 AI 提供商" : "编辑 AI 提供商").font(.headline); Spacer() }.padding(20)
            Divider()
            HSplitView {
                VStack(spacing: 0) {
                    List(selection: $selection) {
                        Label("连接与认证", systemImage: "network").tag(ProviderEditorSelection.connection)
                        Section("模型") {
                            ForEach(models) { model in VStack(alignment: .leading, spacing: 3) { Text(model.displayName).lineLimit(1); if !model.providerModelID.isEmpty { Text(model.providerModelID).font(.caption).foregroundStyle(.secondary).lineLimit(1) } }.tag(ProviderEditorSelection.model(model.id)) }
                        }
                    }.listStyle(.sidebar)
                    HStack {
                        Menu {
                            Button("添加模型") { addModel(ProviderModelDraft()) }
                            if !store.modelTemplates.isEmpty { Divider(); ForEach(store.modelTemplates) { template in Button("从「\(template.name)」填写") { addModel(ProviderModelDraft(template.model)) } } }
                        } label: { Image(systemName: "plus") }.menuStyle(.borderlessButton)
                        Button { if let index = modelIndex { models.remove(at: index); selection = .connection } } label: { Image(systemName: "minus") }.disabled(modelIndex == nil)
                        Spacer()
                    }.padding(12)
                }.frame(minWidth: 170, idealWidth: 190, maxWidth: 230)
                Group {
                    if let index = modelIndex {
                        Form {
                            ProviderModelFields(draft: $models[index])
                            Section { Button("保存为模型模板…") { let model = models[index].definition; templateDraft = ModelTemplate(name: model.displayName, suggestedProviderModelID: model.providerModelID, nickname: model.nickname, icon: model.icon, contextWindow: model.contextWindow, maxOutputTokens: model.maxOutputTokens, reasoningLevels: model.reasoningLevels) } }
                        }
                    } else { connectionForm }
                }.formStyle(.grouped).frame(minWidth: 360, maxWidth: .infinity, maxHeight: .infinity)
            }
            Divider()
            SettingsError(message: store.error).padding(.top, 8)
            HStack { Spacer(); Button("取消") { dismiss() }.keyboardShortcut(.cancelAction); Button("保存") { commit() }.keyboardShortcut(.defaultAction).disabled(!valid || store.isBusy) }.padding(16)
        }.frame(width: 650, height: 510)
        .onAppear { name = provider?.name ?? ""; protocolID = provider?.protocolID ?? .chatCompletionsV1; oldProtocol = protocolID; endpoint = provider?.endpoint ?? ""; models = (provider?.models ?? []).map(ProviderModelDraft.init); editingAPIKey = authentication.method == .unconfigured; readAPIKeyIfNeeded() }
        .onChange(of: selection) { _, value in if value == .connection { readAPIKeyIfNeeded() } }
        .onChange(of: protocolID) { old, _ in if protocolID != provider?.protocolID && models.contains(where: { $0.adapterMetadataJSON != nil }) { oldProtocol = old; confirmsProjectionRemoval = true } }
        .alert("移除来源适配参数？", isPresented: $confirmsProjectionRemoval) {
            Button("移除并继续") { for index in models.indices { models[index].adapterMetadataJSON = nil } }
            Button("取消", role: .cancel) { protocolID = oldProtocol }
        } message: { Text("新协议不能直接沿用原运行时的协议适配参数。模型 ID 与已填写的能力仍保留。") }
        .sheet(item: $templateDraft) { value in ModelTemplateEditor(store: store, template: value) }
        .sheet(isPresented: $store.showsAuthentication) { AuthenticationView(store: store) }
    }
    private var connectionForm: some View {
        Form {
            Section("连接") {
                TextField("名称", text: $name)
                Picker("协议", selection: $protocolID) { ForEach(store.protocols.filter(\.supported)) { Text($0.name).tag($0.id) } }
                TextField("服务地址", text: $endpoint, prompt: Text("https://…/v1"))
            }
            Section("认证") {
                LabeledContent("方式", value: editingAPIKey ? "API key · Bearer" : clearAuthentication ? "尚未配置" : authentication.method == .oauth ? "OAuth · Bearer" : authentication.method.label)
                if editingAPIKey {
                    HStack {
                        if showKey { TextField("API key", text: $apiKey) } else { SecureField("API key", text: $apiKey) }
                        Button { showKey.toggle() } label: { Image(systemName: showKey ? "eye.slash" : "eye") }.buttonStyle(.borderless).help(showKey ? "隐藏 API key" : "显示 API key")
                    }.privacySensitive()
                    if authentication.method == .apiKey && originalAPIKey == nil && !clearAuthentication {
                        Button("读取 API key") { attemptedKeyRead = false; readAPIKeyIfNeeded() }.disabled(store.isLoading)
                    }
                } else {
                    if authentication.method == .oauth { Text(authentication.configured ? "已登录" : "需要登录").foregroundStyle(.secondary) }
                    if let provider, !clearAuthentication, !authentication.actions.isEmpty { Button("登录…") { store.startAuthentication(provider.id) }.disabled(store.isBusy) }
                    Button("使用 API key…") { editingAPIKey = true; clearAuthentication = true; apiKey = "" }
                }
                if authentication.configured || editingAPIKey {
                    Button("清除认证", role: .destructive) { clearAuthentication = true; editingAPIKey = false; apiKey = ""; originalAPIKey = nil }
                }
                if let provenance = authentication.provenance {
                    ImmediateDisclosureGroup("来源详情") { Text(provenance).font(.callout).foregroundStyle(.secondary).frame(maxWidth: .infinity, alignment: .leading).padding(.top, 8) }
                }
            }
        }
    }
    private func addModel(_ draft: ProviderModelDraft) { models.append(draft); selection = .model(draft.id) }
    private func readAPIKeyIfNeeded() {
        guard let provider, authentication.method == .apiKey, !attemptedKeyRead, !clearAuthentication else { return }
        attemptedKeyRead = true
        store.readProviderAPIKey(provider.id) { value in originalAPIKey = value; apiKey = value; editingAPIKey = true }
    }
    private func commit() {
        let edit: AuthenticationEdit = editingAPIKey && apiKey != originalAPIKey ? .setAPIKey(apiKey) : clearAuthentication ? .clear : .keep
        store.saveProvider(AIProvider(id: provider?.id ?? "", name: name.trimmingCharacters(in: .whitespacesAndNewlines), protocolID: protocolID, endpoint: endpoint.trimmingCharacters(in: .whitespacesAndNewlines), authentication: authentication, models: models.map(\.definition)), authenticationEdit: edit) { apiKey = ""; originalAPIKey = nil; dismiss() }
    }
}

struct ModelTemplatesView: View {
    @ObservedObject var store: AppStore
    @Environment(\.dismiss) private var dismiss
    @State private var selectedID: String?
    @State private var editor: ModelTemplate?
    var body: some View {
        VStack(spacing: 0) {
            HStack { Text("模型模板").font(.headline); Spacer() }.padding(20)
            Text("模板用于快速填写模型，修改模板不会改变已配置的模型。").font(.callout).foregroundStyle(.secondary).padding(.horizontal, 20).padding(.bottom, 12)
            List(selection: $selectedID) { ForEach(store.modelTemplates) { template in VStack(alignment: .leading, spacing: 3) { Text(template.name); Text(template.suggestedProviderModelID).font(.caption).foregroundStyle(.secondary) }.tag(template.id) } }.listStyle(.bordered).padding(.horizontal, 20)
            HStack {
                Button { editor = ModelTemplate(name: "", suggestedProviderModelID: "") } label: { Image(systemName: "plus") }
                Button { if let selectedID { store.deleteTemplate(selectedID) } } label: { Image(systemName: "minus") }.disabled(selectedID == nil || store.isLoading)
                Spacer(); Button("编辑…") { editor = store.modelTemplates.first { $0.id == selectedID } }.disabled(selectedID == nil); Button("完成") { dismiss() }.keyboardShortcut(.cancelAction)
            }.padding(16)
            SettingsError(message: store.error)
        }.frame(width: 480, height: 380).sheet(item: $editor) { ModelTemplateEditor(store: store, template: $0) }
    }
}
struct ModelTemplateEditor: View {
    @ObservedObject var store: AppStore
    let template: ModelTemplate
    @Environment(\.dismiss) private var dismiss
    @State private var name = ""
    @State private var draft = ProviderModelDraft()
    var body: some View {
        VStack(spacing: 0) {
            Form { Section { TextField("模板名称", text: $name) }; ProviderModelFields(draft: $draft) }.formStyle(.grouped)
            SettingsError(message: store.error)
            HStack { Spacer(); Button("取消") { dismiss() }.keyboardShortcut(.cancelAction); Button("保存") { let model = draft.definition; store.saveTemplate(ModelTemplate(id: template.id, name: name, suggestedProviderModelID: model.providerModelID, nickname: model.nickname, icon: model.icon, contextWindow: model.contextWindow, maxOutputTokens: model.maxOutputTokens, reasoningLevels: model.reasoningLevels)) { dismiss() } }.keyboardShortcut(.defaultAction).disabled(name.trimmingCharacters(in: .whitespacesAndNewlines).isEmpty || !draft.valid || store.isLoading) }.padding(16)
        }.frame(width: 440, height: 400).onAppear { name = template.name; draft = ProviderModelDraft(template.model) }
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
    let models: [ModelChoice]
    let gatewayID: String
    let isSaving: Bool
    let error: String?
    let save: (RuntimeInstance, (() -> Void)?) -> Void
    @Environment(\.dismiss) private var dismiss
    @State private var name = ""
    @State private var typeID = ""
    @State private var modelRecordKey: String?
    @State private var settings: [String: String] = [:]
    @State private var draftID = UUID().uuidString
    @State private var typeDrafts: [String: [String: String]] = [:]
    @State private var executableDiscoveryRunning = false
    @State private var executableDiscoveryMessage: String?
    private var descriptor: RuntimeTypeDescriptor? { types.first { $0.id == typeID } }
    private var compatibleModels: [ModelChoice] {
        guard let descriptor else { return [] }
        return models.filter { descriptor.supportedProtocols.contains($0.protocolID) }
    }
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
                    Picker("初始模型", selection: $modelRecordKey) { Text("稍后选择").tag(Optional<String>.none); ForEach(compatibleModels) { model in Text(model.displayName).tag(Optional(model.id)) } }
                }
                if let descriptor {
                    Section("实例配置") {
                        LabeledContent("兼容版本", value: descriptor.versionRegex).font(.caption).foregroundStyle(.secondary)
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
            modelRecordKey = instance?.modelRecordKey
            DispatchQueue.main.async { discoverExecutableIfNeeded() }
        }
        .onChange(of: typeID) { old, new in
            if instance == nil { typeDrafts[old] = settings; settings = typeDrafts[new] ?? [:] }
            if let selected = modelRecordKey, !compatibleModels.contains(where: { $0.recordKey == selected }) { modelRecordKey = nil }
            executableDiscoveryMessage = nil
            DispatchQueue.main.async { discoverExecutableIfNeeded() }
        }
    }

    private func discoveryValue(for field: SettingField) -> String { (settings[field.key] ?? field.value).trimmingCharacters(in: .whitespacesAndNewlines) }
    private func discoveryTitle(for field: SettingField) -> String? {
        guard field.executableDiscovery != nil, discoveryValue(for: field).isEmpty else { return nil }
        return executableDiscoveryRunning ? "探测中…" : (executableDiscoveryMessage == nil ? "自动发现" : "重新发现")
    }
    private func discoveryAction(for field: SettingField) -> (() -> Void)? {
        guard field.executableDiscovery != nil, discoveryValue(for: field).isEmpty, !executableDiscoveryRunning else { return nil }
        return { discoverExecutables([field], type: typeID) }
    }
    private func discoverExecutableIfNeeded() {
        discoverExecutables(descriptor?.fields.filter { $0.executableDiscovery != nil && discoveryValue(for: $0).isEmpty } ?? [], type: typeID)
    }
    private func discoverExecutables(_ fields: [SettingField], type: String) {
        guard !executableDiscoveryRunning, type == typeID, let field = fields.first, let discovery = field.executableDiscovery else { return }
        executableDiscoveryRunning = true
        executableDiscoveryMessage = "正在查找\(field.label)…"
        MacExecutableDiscovery.discover(discovery) { result in
            executableDiscoveryRunning = false
            guard type == typeID else { discoverExecutableIfNeeded(); return }
            switch result {
            case .success(let path):
                if discoveryValue(for: field).isEmpty { settings[field.key] = path; executableDiscoveryMessage = "已发现：\(path)" }
            case .failure(let error): executableDiscoveryMessage = error.localizedDescription
            }
            discoverExecutables(Array(fields.dropFirst()), type: type)
        }
    }
    private func commit() {
        guard valid, let descriptor else { return }
        var values: [String: String] = [:]
        for field in descriptor.fields {
            values[field.key] = MacPath.normalized(settings[field.key] ?? field.value, field: field)
        }
        save(RuntimeInstance(id: instance?.id ?? draftID, name: name.trimmingCharacters(in: .whitespacesAndNewlines), typeID: typeID, gatewayID: instance?.gatewayID ?? gatewayID, settings: values, modelRecordKey: modelRecordKey)) { dismiss() }
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
        let path = expanded(value)
        guard !path.isEmpty else { return "" }
        return URL(fileURLWithPath: path).standardizedFileURL.path
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

struct RuntimeInteractionView: View {
    @ObservedObject var store: AppStore
    let interaction: RuntimeInteraction
    @State private var values: [String:String] = [:]
    var body: some View {
        VStack(alignment: .leading, spacing: 16) {
            switch interaction.kind {
            case .approval(let title, let detail, let options):
                Text(title).font(.headline)
                ScrollView { Text(detail).textSelection(.enabled).frame(maxWidth: .infinity, alignment: .leading) }.frame(maxHeight: 220)
                HStack { Button("取消请求", role: .cancel) { reply(.cancel) }; Spacer(); ForEach(options) { option in Button(option.label) { reply(.decision(option.id)) } } }.disabled(store.isLoading)
            case .userInput(let questions):
                Text("运行时需要输入").font(.headline)
                Form {
                    ForEach(questions) { question in
                        Section {
                            Text(question.text).textSelection(.enabled)
                            if !question.options.isEmpty { Picker("选择", selection: value(question.id)) { Text("请选择").tag(""); ForEach(question.options) { Text($0.label).tag($0.id) } } }
                            else if question.secret { SecureField("回答", text: value(question.id)).privacySensitive() }
                            else { TextField("回答", text: value(question.id)) }
                        }
                    }
                }.formStyle(.grouped)
                HStack { Button("取消请求", role: .cancel) { reply(.cancel) }; Spacer(); Button("提交") { reply(.answers(questions.map { InteractionAnswer(questionID: $0.id, values: [values[$0.id] ?? ""]) })) }.disabled(questions.contains { (values[$0.id] ?? "").isEmpty }) }.disabled(store.isLoading)
            }
            SettingsError(message: store.error)
        }.padding(20).frame(width: 500).frame(minHeight: 180, maxHeight: 480)
        .interactiveDismissDisabled()
        .onChange(of: interaction.id) { _, _ in values = [:] }
    }
    private func value(_ id: String) -> Binding<String> { Binding(get: { values[id] ?? "" }, set: { values[id] = $0 }) }
    private func reply(_ value: RuntimeInteractionReply) { store.replyInteraction(interaction, reply: value) }
}
