import SwiftUI
import AppKit
import MarkdownUI
import VeluneBindings

struct VeluneRootView: View {
    @ObservedObject var store: AppStore
    let previewEmpty: Bool
    let previewSettings: Bool
    @Environment(\.openSettings) private var openSettings
    @State private var browser = ConversationBrowser()
    @State private var draft = ""
    @State private var scrollRequest: UInt64 = 0
    @State private var sentAfterUserID: String?
    @State private var showsOutline = false
    @State private var renameTarget: Conversation?
    @State private var deleteTargets: [Conversation] = []
    private var conversationSections: [ConversationBrowser.Section] { browser.sections(conversations: store.conversations, runtimes: store.runtimeInstances) }
    private var projects: [String] { Array(Set(store.conversations.filter { conversation in store.enabledRuntimeInstances.contains { $0.id == conversation.runtimeID } }.compactMap(\.cwd))).sorted { $0.localizedStandardCompare($1) == .orderedAscending } }
    private var nextTurnRuntime: RuntimeInstance? { store.runtimeInstances.first { $0.id == store.nextTurnRuntimeID } }

    var body: some View {
        NavigationSplitView {
            List(selection: Binding(get: { store.selectedConversationIDs }, set: { store.selectConversations(ids: $0) })) {
                if conversationSections.isEmpty && !store.isLoading && !store.conversations.isEmpty {
                    Text("没有匹配的会话").foregroundStyle(.secondary)
                    Button("清除筛选") { browser.runtimeID = nil; browser.project = .all; browser.search = "" }
                }
                ForEach(conversationSections) { section in
                  Section {
                    ForEach(section.conversations) { conversation in
                        VStack(alignment: .leading, spacing: 3) {
                            Text(conversation.title).lineLimit(1)
                            HStack(spacing: 8) {
                                Text(rowContext(conversation)).lineLimit(1).truncationMode(.middle).help(conversation.cwd ?? "未指定项目")
                                Spacer(minLength: 0)
                                if let date = browser.sort.timestamp(conversation) { Text(shortDate(date)).help("\(browser.sort.title)：\(Date(timeIntervalSince1970: Double(date) / 1000).formatted())").accessibilityLabel("\(browser.sort.title)：\(shortDate(date))") }
                            }.font(.caption).foregroundStyle(.secondary)
                        }.padding(.vertical, 3).tag(conversation.id)
                        .selectionDisabled(store.isLoading)
                        .contextMenu { conversationManagementMenu(store.selectedConversationIDs.contains(conversation.id) ? store.selectedConversationIDs : [conversation.id]) }
                    }
                    if section.hasMore {
                        Button("加载更多（\(section.conversations.count) / \(section.totalCount)）") { browser.loadMore(section.id) }
                            .buttonStyle(.plain).foregroundStyle(.secondary).selectionDisabled(true)
                    }
                  } header: { Text(section.title).lineLimit(1).truncationMode(.middle).help(section.title) }
                }
            }
            .contextMenu(forSelectionType: String.self) { ids in conversationManagementMenu(ids) } primaryAction: { ids in
                guard ids.count == 1, let id = ids.first, let target = store.conversations.first(where: { $0.id == id }), store.canManageConversations, store.canRenameConversation(target) else { return }
                renameTarget = target
            }
            .listStyle(.sidebar)
            .searchable(text: $browser.search, placement: .sidebar, prompt: "搜索会话")
            .navigationTitle("会话")
            .navigationSplitViewColumnWidth(min: 190, ideal: 240, max: 340)
            .disabled(store.isGenerating || store.authenticationRunning || store.isShuttingDown)
            .toolbar {
                ToolbarItem(placement: .automatic) { browserMenu }
            }
        } detail: {
            VStack(spacing: 0) {
                if store.pendingConversationID != nil { ProgressView(store.conversationManagementStatus ?? "正在加载会话…").frame(maxWidth: .infinity, maxHeight: .infinity) }
                else if previewEmpty || store.transcript.rows.isEmpty { emptyState.frame(maxWidth: .infinity, maxHeight: .infinity) }
                else { transcript }
                Divider()
                composer
            }
            .navigationTitle(previewEmpty ? "新会话" : store.selectedConversationTitle ?? "Velune")
            .toolbar {
                ToolbarItemGroup(placement: .primaryAction) {
                    Button { showsOutline = true } label: { Label("会话大纲", systemImage: "list.bullet") }
                        .help("会话大纲")
                        .disabled(store.pendingConversationID != nil || store.transcript.userRows.isEmpty)
                    Button { store.createConversation() } label: { Label("新建会话", systemImage: "square.and.pencil") }
                        .help("新建会话（⌘ N）").disabled(store.applicationIsGenerating || store.isLoading)
                }
            }
        }
        .navigationSplitViewStyle(.balanced)
        .onReceive(store.$conversationBrowserGroupLimit) { browser.initialLimit = $0 }
        .onReceive(store.$conversationBrowserPreferences) { value in
            if let value, value != browser.preferences { browser.restore(value) }
        }
        .onChange(of: browser.preferences) { _, value in
            if store.conversationBrowserPreferences != nil { store.setConversationBrowserPreferences(value) }
        }
        .onChange(of: browser.query) { _, _ in browser.resetPagination() }
        .onChange(of: store.pendingConversationID) { _, value in if value != nil { showsOutline = false } }
        .onChange(of: store.loadedConversationID) { _, _ in showsOutline = false }
        .toolbar { if !store.problems.isEmpty { ToolbarItem(placement: .primaryAction) { ProblemsButton(store: store) } } }
        .sheet(item: $renameTarget) { ConversationRenameView(store: store, conversation: $0) }
        .alert(deleteTargets.count == 1 ? "删除会话？" : "删除 \(deleteTargets.count) 个会话？", isPresented: Binding(get: { !deleteTargets.isEmpty }, set: { if !$0 { deleteTargets = [] } })) {
            Button("取消", role: .cancel) { deleteTargets = [] }
            Button("删除", role: .destructive) { store.deleteConversations(deleteTargets); deleteTargets = [] }
        } message: {
            Text("将永久删除所选 \(deleteTargets.count) 个会话，以及它们在各 Agent 运行时中的关联会话数据。此操作无法撤销。")
        }
        .sheet(isPresented: $store.showsNewConversation) { NewConversationView(store: store) }
        .sheet(item: Binding(get: { store.pendingInteractions.first }, set: { _ in })) { RuntimeInteractionView(store: store, interaction: $0) }
        .focusedSceneValue(\.conversationCommands, ConversationCommandTarget(store: store, send: send))
        .task {
            store.start()
            if previewSettings { openSettings() }
        }
    }

    @ViewBuilder private func conversationManagementMenu(_ ids: Set<String>) -> some View {
        let targets = store.conversations.filter { ids.contains($0.id) }
        if targets.count == 1, let target = targets.first {
            Button("重命名…") { renameTarget = target }.disabled(!store.canManageConversations || !store.canRenameConversation(target))
        }
        Button(targets.count > 1 ? "删除所选会话…" : "删除…", role: .destructive) { deleteTargets = targets }
            .disabled(targets.isEmpty || !store.canManageConversations || !targets.allSatisfy(store.canDeleteConversation))
        if targets.contains(where: { !store.canDeleteConversation($0) }) { Text("部分关联原生会话当前无法删除") }
        if targets.count == 1, let target = targets.first, !store.canRenameConversation(target) { Text("原生会话标题当前无法修改") }
    }

    private func rowContext(_ conversation: Conversation) -> String {
        var parts: [String] = []
        if browser.grouping != .runtime { parts.append(store.runtimeInstances.first { $0.id == conversation.runtimeID }?.name ?? "Agent 运行时") }
        if browser.grouping != .project, let path = conversation.cwd { parts.append(URL(fileURLWithPath: path).lastPathComponent) }
        return parts.joined(separator: " · ")
    }

    private var browserMenu: some View {
        Menu {
            Picker("分组", selection: $browser.grouping) { ForEach(ConversationBrowser.Grouping.allCases) { Text($0.title).tag($0) } }
            Picker("排序", selection: $browser.sort) { ForEach(ConversationBrowser.Sort.allCases) { Text($0.title).tag($0) } }
            Toggle("从旧到新", isOn: $browser.oldestFirst)
            Divider()
            Picker("Agent 运行时", selection: $browser.runtimeID) {
                Text("所有运行时").tag(Optional<String>.none)
                ForEach(store.enabledRuntimeInstances) { Text($0.name).tag(Optional($0.id)) }
                if let id = browser.runtimeID, !store.enabledRuntimeInstances.contains(where: { $0.id == id }) {
                    Text("当前筛选的运行时不可用").tag(Optional(id))
                }
            }
            Picker("项目", selection: $browser.project) {
                Text("所有项目").tag(ConversationBrowser.ProjectFilter.all)
                Text("未指定项目").tag(ConversationBrowser.ProjectFilter.unspecified)
                ForEach(projects, id: \.self) { path in Text(path).tag(ConversationBrowser.ProjectFilter.path(path)) }
                if case .path(let path) = browser.project, !projects.contains(path) { Text(path).tag(ConversationBrowser.ProjectFilter.path(path)) }
            }
            if browser.hasFilters { Divider(); Button("清除筛选") { browser.runtimeID = nil; browser.project = .all } }
        } label: { Label("显示", systemImage: browser.hasFilters ? "line.3.horizontal.decrease.circle.fill" : "line.3.horizontal.decrease") }
        .help("会话分组、筛选与排序")
        .disabled(store.conversationBrowserPreferences == nil)
    }

    private var nextTurnControls: some View {
        HStack(spacing: 8) {
            Menu {
                Picker("Agent 运行时", selection: Binding(get: { store.nextTurnRuntimeID }, set: { if let id = $0 { store.selectNextTurnRuntime(id) } })) {
                    ForEach(store.enabledRuntimeInstances) { runtime in Text(runtime.name).tag(Optional(runtime.id)) }
                }.pickerStyle(.inline)
                Divider()
                SettingsLink { Text("设置…") }
            } label: {
                Text(nextTurnRuntime?.name ?? "选择运行时").lineLimit(1).truncationMode(.middle).frame(maxWidth: 120, alignment: .leading)
            }
            .frame(maxWidth: 120, alignment: .leading)
            .accessibilityLabel("下一轮 Agent 运行时：\(nextTurnRuntime?.name ?? "未选择")")
            .help("下一轮运行时：\(nextTurnRuntime?.name ?? "未选择")")
            .disabled(store.isShuttingDown)
            Menu {
                Picker("模型", selection: Binding(get: { store.nextTurnModelRecordKey }, set: { if let key = $0 { store.selectModel(modelRecordKey: key) } })) {
                    ForEach(store.runtimeCompatibleModels) { model in Text(model.displayName).tag(Optional(model.recordKey)) }
                }.pickerStyle(.inline).disabled(!store.canSwitchModel)
                Divider()
                SettingsLink { Text("设置…") }
            } label: {
                Text(store.selectedModelName ?? "选择模型").lineLimit(1).truncationMode(.middle).frame(maxWidth: 160, alignment: .leading)
            }
            .frame(maxWidth: 160, alignment: .leading)
            .accessibilityLabel("下一轮模型：\(store.selectedModelName ?? "未选择")")
            .help("下一轮模型：\(store.selectedModelName ?? "未选择")")
            .disabled(store.isShuttingDown)
        }.menuStyle(.borderlessButton).controlSize(.small).font(.callout)
    }

    private var emptyState: some View {
        ContentUnavailableView {
            Label { Text("新会话") } icon: { VeluneLogo(size: 48) }
        } description: {
            Text(store.executionBusyElsewhere ? "另一个会话正在运行，完成后即可发送。" : store.needsModelSelection ? "选择下一轮模型后即可发送。" : store.canSend ? "输入消息，开始工作。" : "先在设置中配置AI提供商、模型与Agent 运行时。")
        } actions: {
            if !store.canSend && !store.executionBusyElsewhere && !store.needsModelSelection { SettingsLink { Text("打开设置") } }
        }
    }

    private var transcript: some View {
        TranscriptView(model: store.transcript, conversationID: store.loadedConversationID,
                       scrollRequest: scrollRequest, sentAfterUserID: sentAfterUserID, cwd: store.loadedConversationWorkingDirectory, showsOutline: $showsOutline, activity: store.activity, presentation: store.transcriptPresentation)
    }

    private var composer: some View {
        VStack(alignment: .leading, spacing: 8) {
            if store.pendingConversationID == nil, let status = store.conversationManagementStatus {
                HStack { ProgressView().controlSize(.small); Text(status) }.font(.callout).foregroundStyle(.secondary)
            }
            if store.needsModelSelection {
                Text("此会话的模型当前不可用，请选择已配置的模型。").font(.callout).foregroundStyle(.secondary)
            }
            Text("输入消息").font(.caption).foregroundStyle(.secondary)
            TextEditor(text: $draft).font(.body).frame(height: 88)
                .accessibilityLabel("消息内容").help("Return 换行，⌘ Return 发送")
            HStack(spacing: 8) {
                nextTurnControls
                Spacer(minLength: 8)
                if store.executionBusyElsewhere { Text("另一个会话正在运行").font(.caption).foregroundStyle(.secondary) }
                if store.isGenerating {
                    Button("停止生成", systemImage: "stop.fill", action: store.cancel)
                        .fixedSize().disabled(!store.canCancel).help("停止当前生成")
                } else {
                    Button("发送", systemImage: "arrow.up", action: send)
                        .buttonStyle(.borderedProminent).fixedSize()
                        .disabled(!store.canSend || draft.trimmingCharacters(in: .whitespacesAndNewlines).isEmpty)
                        .help("发送消息（⌘ Return）")
                }
            }.controlSize(.small)
        }.frame(maxWidth: 760).padding(12).frame(maxWidth: .infinity)
    }

    private func send() {
        let text = draft.trimmingCharacters(in: .whitespacesAndNewlines)
        guard store.canSend, !text.isEmpty else { return }
        let submittedDraft = draft
        let previousUserID = store.transcript.userRows.last?.id
        store.send(text: text) { if draft == submittedDraft { draft = "" }; sentAfterUserID = previousUserID; scrollRequest &+= 1 }
    }
    private func shortDate(_ unixMs: Int64) -> String {
        Date(timeIntervalSince1970: Double(unixMs) / 1000).formatted(.dateTime.month(.abbreviated).day())
    }
}

struct ConversationRenameView: View {
    @ObservedObject var store: AppStore
    let conversation: Conversation
    @Environment(\.dismiss) private var dismiss
    @State private var title: String
    @FocusState private var titleFocused: Bool
    init(store: AppStore, conversation: Conversation) {
        self.store = store; self.conversation = conversation
        _title = State(initialValue: conversation.title)
    }
    var body: some View {
        VStack(alignment: .leading, spacing: 16) {
            ProblemsSheetHeading(title: "重命名会话", store: store)
            TextField("会话名称", text: $title).focused($titleFocused).onSubmit(rename)
                .disabled(store.conversationManagementStatus != nil)
            if let status = store.conversationManagementStatus { Text(status).font(.callout).foregroundStyle(.secondary) }
            HStack {
                if store.conversationManagementStatus != nil { ProgressView().controlSize(.small) }
                Spacer()
                Button("取消") { dismiss() }.keyboardShortcut(.cancelAction).disabled(store.conversationManagementStatus != nil)
                Button("重命名", action: rename).keyboardShortcut(.defaultAction)
                    .disabled(!store.canManageConversations || title.trimmingCharacters(in: .whitespacesAndNewlines).isEmpty)
            }
        }.padding(20).frame(width: 360).onAppear { titleFocused = true }
    }
    private func rename() { store.renameConversation(conversation, title: title) { dismiss() } }
}

struct ConversationMessageView: View {
    @ObservedObject var row: TranscriptRow
    var cwd: String? = nil
    var body: some View {
        HStack(alignment: .top, spacing: 0) {
            if row.message.role == .user { Spacer(minLength: 60) }
            VStack(alignment: .leading, spacing: 9) {
                ForEach(Array(row.message.blocks.enumerated()), id: \.offset) { index, block in
                    switch block {
                    case .tool(_, let title, let state, let output):
                        ToolDisclosure(title: title, detail: output ?? "", state: state)
                    case .reasoning(let text):
                        ImmediateDisclosureGroup { Text(text).textSelection(.enabled).font(.callout).foregroundStyle(.secondary) } label: { Text("思考过程").font(.callout).foregroundStyle(.secondary) }
                    case .notice(let text):
                        Text(text).font(.callout).foregroundStyle(.secondary).textSelection(.enabled)
                    case .text(let text):
                        if row.message.role == .assistant, let content = row.markdown[index] {
                            Markdown(content)
                                .modifier(MessageLinkOpener(cwd: cwd))
                                .markdownTheme(.basic.codeBlock { configuration in CodeBlockView(language: configuration.language ?? "", code: configuration.content) })
                                .textSelection(.enabled)
                        } else { Text(text).textSelection(.enabled).multilineTextAlignment(.leading) }
                    }
                }
            }
            .frame(maxWidth: row.message.role == .user ? 600 : .infinity, alignment: row.message.role == .system ? .center : .leading)
            .padding(row.message.role == .user ? 12 : 0)
            .background { if row.message.role == .user { RoundedRectangle(cornerRadius: 12).fill(.quaternary) } }
            if row.message.role == .user { EmptyView() }
        }
        .frame(maxWidth: .infinity, alignment: row.message.role == .system ? .center : .leading)
        .multilineTextAlignment(row.message.role == .system ? .center : .leading)
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
    let state: ToolState
    @State private var expanded = false
    var body: some View {
        ImmediateDisclosureGroup(isExpanded: $expanded) {
            Text(detail.isEmpty ? "没有附加输出" : detail).font(.system(.callout, design: .monospaced)).foregroundStyle(.secondary).textSelection(.enabled).padding(.top, 4)
        } label: {
            Label(title, systemImage: state == .running ? "circle.dotted" : state == .failed ? "exclamationmark.circle" : state == .pending ? "circle" : "checkmark.circle").font(.callout).foregroundStyle(.secondary)
        }.frame(maxWidth: expanded ? 560 : nil).fixedSize(horizontal: !expanded, vertical: false).frame(maxWidth: .infinity, alignment: .leading)
    }
}


struct SettingsView: View {
    @ObservedObject var store: AppStore
    var body: some View {
        TabView {
            ProviderSettingsView(store: store).tabItem { Label("AI提供商", systemImage: "network") }
            ConversationSettingsView(store: store).tabItem { Label("会话", systemImage: "bubble.left.and.bubble.right") }
            RuntimeSettingsView(store: store).tabItem { Label("Agent 运行时", systemImage: "terminal") }
        }.frame(width: 690, height: 560)
    }
}

private struct ConversationSettingsView: View {
    @ObservedObject var store: AppStore
    @State private var limit = 20
    var body: some View {
        Form {
            Section("消息列表") {
                Picker("默认显示", selection: Binding(get: { store.transcriptPresentation }, set: { store.setTranscriptPresentation($0) })) {
                    Text("完整会话").tag(BindingTranscriptPresentation.conversation)
                    Text("用户消息大纲").tag(BindingTranscriptPresentation.userOutline)
                }.disabled(store.isLoading || store.isShuttingDown)
                Text("大纲模式仅显示用户消息，点击消息可展开从此处开始的后续内容。").font(.caption).foregroundStyle(.secondary)
            }
            Section("会话列表") {
                LabeledContent("每组首次展示") { TextField("数量", value: $limit, format: .number).frame(width: 70); Text("个会话") }
                Text("每组按当前排序显示前若干会话，可在列表中加载更多。").font(.caption).foregroundStyle(.secondary)
                Button("保存") { store.setConversationBrowserGroupLimit(limit) }.disabled(limit < 1 || limit == store.conversationBrowserGroupLimit || store.isBusy)
            }

        }.formStyle(.grouped).onAppear { limit = store.conversationBrowserGroupLimit }
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
                    }.tag(provider.id)
                }
            }.contextMenu(forSelectionType: String.self) { ids in
                if let id = ids.first, let provider = store.providers.first(where: { $0.id == id }) { Button("编辑…") { editor = provider }; Button("删除…") { deleting = provider } }
            } primaryAction: { ids in if let id = ids.first { editor = store.providers.first { $0.id == id } } }
            .listStyle(.bordered).padding(.horizontal, 20).padding(.top, 16)
            HStack {
                Button { creating = true } label: { Image(systemName: "plus") }.help("添加 AI 提供商")
                Button { deleting = store.providers.first { $0.id == selectedID } } label: { Image(systemName: "minus") }.disabled(selectedID == nil)
                Button("从运行时导入…") { importing = true }.disabled(store.providerImportTypes.isEmpty || store.isBusy)
                Menu { Button("模型模板…") { templates = true } } label: { Image(systemName: "ellipsis") }.menuStyle(.borderlessButton).menuIndicator(.hidden).frame(width: 28).accessibilityLabel("更多操作")
                Spacer()
                Button("编辑…") { editor = store.providers.first { $0.id == selectedID } }.disabled(selectedID == nil)
            }.padding(.horizontal, 20).padding(.vertical, 12)

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
    var modelIDFocused: FocusState<Bool>.Binding
    var body: some View {
        Section("模型") {
            TextField("模型 ID", text: $draft.providerModelID, prompt: Text("提供商 API 规定的标识"))
                .focused(modelIDFocused)
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
    @State private var choosingTemplates = false
    @FocusState private var providerNameFocused: Bool
    @FocusState private var modelIDFocused: Bool
    private var authentication: ProviderAuthentication { store.providers.first { $0.id == provider?.id }?.authentication ?? provider?.authentication ?? ProviderAuthentication() }
    private var modelIndex: Int? { guard case .model(let id) = selection else { return nil }; return models.firstIndex { $0.id == id } }
    private var valid: Bool { !name.trimmingCharacters(in: .whitespacesAndNewlines).isEmpty && URL(string: endpoint)?.host != nil && models.allSatisfy(\.valid) && (!editingAPIKey || (!apiKey.isEmpty && (originalAPIKey != nil || authentication.method != .apiKey || clearAuthentication))) }
    var body: some View {
        VStack(spacing: 0) {
            ProblemsSheetHeading(title: provider == nil ? "添加 AI 提供商" : "编辑 AI 提供商", store: store).padding(20)
            Divider()
            HSplitView {
                VStack(spacing: 0) {
                    List(selection: $selection) {
                        Label("连接与认证", systemImage: "network").tag(ProviderEditorSelection.connection)
                        Section("模型") {
                            ForEach(models) { model in VStack(alignment: .leading, spacing: 3) { Text(model.displayName).lineLimit(1); if !model.providerModelID.isEmpty { Text(model.providerModelID).font(.caption).foregroundStyle(.secondary).lineLimit(1) } }.tag(ProviderEditorSelection.model(model.id)) }
                        }
                    }.contextMenu(forSelectionType: ProviderEditorSelection.self) { ids in
                        if ids.count == 1, let target = ids.first { Button("编辑") { editSelection(target) } }
                    } primaryAction: { ids in if ids.count == 1, let target = ids.first { editSelection(target) } }
                    .listStyle(.sidebar)
                    HStack {
                        Menu {
                            Button("添加新模型") { addModel(ProviderModelDraft()) }
                            Button("从模板中选取…") { choosingTemplates = true }
                        } label: { Image(systemName: "plus") }.menuStyle(.borderlessButton)
                        Button { if let index = modelIndex { models.remove(at: index); selection = .connection } } label: { Image(systemName: "minus") }.disabled(modelIndex == nil)
                        Spacer()
                    }.padding(12)
                }.frame(minWidth: 170, idealWidth: 190, maxWidth: 230)
                Group {
                    if let index = modelIndex {
                        Form {
                            ProviderModelFields(draft: $models[index], modelIDFocused: $modelIDFocused)
                            Section { Button("保存为模型模板…") { let model = models[index].definition; templateDraft = ModelTemplate(name: model.displayName, suggestedProviderModelID: model.providerModelID, nickname: model.nickname, icon: model.icon, contextWindow: model.contextWindow, maxOutputTokens: model.maxOutputTokens, reasoningLevels: model.reasoningLevels) } }
                        }
                    } else { connectionForm }
                }.formStyle(.grouped).frame(minWidth: 360, maxWidth: .infinity, maxHeight: .infinity)
            }
            Divider()

            HStack { Spacer(); Button("取消") { dismiss() }.keyboardShortcut(.cancelAction); Button("保存") { commit() }.keyboardShortcut(.defaultAction).disabled(!valid || store.isBusy) }.padding(16)
        }.frame(width: 650, height: 510)
        .onAppear { name = provider?.name ?? ""; protocolID = provider?.protocolID ?? .chatCompletionsV1; endpoint = provider?.endpoint ?? ""; models = (provider?.models ?? []).map(ProviderModelDraft.init); editingAPIKey = authentication.method == .unconfigured; readAPIKeyIfNeeded() }
        .onChange(of: selection) { _, value in if value == .connection { readAPIKeyIfNeeded() } }
        .sheet(isPresented: $choosingTemplates) { ModelTemplatePicker(templates: store.modelTemplates) { values in for template in values { addModel(ProviderModelDraft(template.model)) } } }
        .sheet(item: $templateDraft) { value in ModelTemplateEditor(store: store, template: value) }
        .sheet(isPresented: $store.showsAuthentication) { AuthenticationView(store: store) }
    }
    private var connectionForm: some View {
        Form {
            Section("连接") {
                TextField("名称", text: $name)
                    .focused($providerNameFocused)
                Picker("协议", selection: Binding(get: { protocolID }, set: { value in
                    guard value != protocolID else { return }
                    for index in models.indices { models[index].adapterMetadataJSON = nil }
                    protocolID = value
                })) { ForEach(store.protocols.filter(\.supported)) { Text($0.name).tag($0.id) } }
                Text("切换协议后采用默认适配参数；模型 ID 与已填写的能力保留。")
                    .font(.caption).foregroundStyle(.secondary)
                TextField("服务地址", text: $endpoint, prompt: Text(protocolID == .messagesV1 ? "https://…" : "https://…/v1"))
                    .help(protocolID == .messagesV1 ? "Anthropic Messages 协议基础 URL；请求路径为 /v1/messages。" : "提供商的协议服务基础 URL。")
            }
            Section("认证") {
                LabeledContent("方式", value: editingAPIKey ? "API key" : clearAuthentication ? "尚未配置" : authentication.method.label)
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
    private func editSelection(_ target: ProviderEditorSelection) {
        selection = target
        DispatchQueue.main.async {
            switch target {
            case .connection: modelIDFocused = false; providerNameFocused = true
            case .model: providerNameFocused = false; modelIDFocused = true
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

private struct ModelTemplatePicker: View {
    let templates: [ModelTemplate]
    let add: ([ModelTemplate]) -> Void
    @Environment(\.dismiss) private var dismiss
    @State private var selection: Set<String> = []
    @State private var search = ""
    @State private var addedCount = 0
    private var candidates: [ModelTemplate] { templates.filter { search.isEmpty || "\($0.name) \($0.suggestedProviderModelID)".localizedCaseInsensitiveContains(search) } }
    var body: some View {
        VStack(spacing: 0) {
            HStack { Text("从模板中选取").font(.headline); Spacer() }.padding(20)
            TextField("搜索模板", text: $search).textFieldStyle(.roundedBorder).padding(.horizontal, 20).padding(.bottom, 8)
            List(selection: $selection) { ForEach(candidates) { value in VStack(alignment: .leading, spacing: 3) { Text(value.name); Text(value.suggestedProviderModelID).font(.caption).foregroundStyle(.secondary) }.tag(value.id) } }.listStyle(.bordered).padding(.horizontal, 20)
            .overlay { if candidates.isEmpty { Text(templates.isEmpty ? "尚无模型模板，可在设置中添加。" : "没有匹配的模板").foregroundStyle(.secondary) } }
            HStack {
                Text(addedCount > 0 ? "已添加 \(addedCount) 个模型" : "按住 ⌘ 或 Shift 选择多个模板").font(.caption).foregroundStyle(.secondary)
                Spacer(); Button("完成") { dismiss() }.keyboardShortcut(.cancelAction)
                Button("添加所选") { let values = templates.filter { selection.contains($0.id) }; add(values); addedCount += values.count; selection.removeAll() }.disabled(selection.isEmpty).keyboardShortcut(.defaultAction)
            }.padding(16)
        }.frame(width: 480, height: 380)
        .onChange(of: search) { _, _ in selection.formIntersection(candidates.map(\.id)) }
    }
}

struct ModelTemplatesView: View {
    @ObservedObject var store: AppStore
    @Environment(\.dismiss) private var dismiss
    @State private var selectedID: String?
    @State private var editor: ModelTemplate?
    @State private var catalog = false
    var body: some View {
        VStack(spacing: 0) {
            ProblemsSheetHeading(title: "模型模板", store: store).padding(20)
            Text("模板用于快速填写模型，修改模板不会改变已配置的模型。").font(.callout).foregroundStyle(.secondary).padding(.horizontal, 20).padding(.bottom, 12)
            List(selection: $selectedID) { ForEach(store.modelTemplates) { template in VStack(alignment: .leading, spacing: 3) { Text(template.name); Text(template.suggestedProviderModelID).font(.caption).foregroundStyle(.secondary) }.tag(template.id) } }
            .contextMenu(forSelectionType: String.self) { ids in
                if ids.count == 1, let id = ids.first, let template = store.modelTemplates.first(where: { $0.id == id }) { Button("编辑…") { editor = template } }
            } primaryAction: { ids in if ids.count == 1, let id = ids.first { editor = store.modelTemplates.first { $0.id == id } } }
            .listStyle(.bordered).padding(.horizontal, 20)
            HStack {
                Button { editor = ModelTemplate(name: "", suggestedProviderModelID: "") } label: { Image(systemName: "plus") }
                Button { if let selectedID { store.deleteTemplate(selectedID) } } label: { Image(systemName: "minus") }.disabled(selectedID == nil || store.isLoading)
                Button("从公开目录添加…") { catalog = true }
                Spacer(); Button("编辑…") { editor = store.modelTemplates.first { $0.id == selectedID } }.disabled(selectedID == nil); Button("完成") { dismiss() }.keyboardShortcut(.cancelAction)
            }.padding(16)

        }.frame(width: 480, height: 380).sheet(item: $editor) { ModelTemplateEditor(store: store, template: $0) }
        .sheet(isPresented: $catalog) { PublicModelCatalogView(store: store) }
    }
}
struct ModelTemplateEditor: View {
    @ObservedObject var store: AppStore
    let template: ModelTemplate
    @Environment(\.dismiss) private var dismiss
    @State private var name = ""
    @State private var draft = ProviderModelDraft()
    @FocusState private var modelIDFocused: Bool
    var body: some View {
        VStack(spacing: 0) {
            ProblemsSheetHeading(title: "编辑模型模板", store: store).padding(16)
            Form { Section { TextField("模板名称", text: $name) }; ProviderModelFields(draft: $draft, modelIDFocused: $modelIDFocused) }.formStyle(.grouped)

            HStack { Spacer(); Button("取消") { dismiss() }.keyboardShortcut(.cancelAction); Button("保存") { let model = draft.definition; store.saveTemplate(ModelTemplate(id: template.id, name: name, suggestedProviderModelID: model.providerModelID, nickname: model.nickname, icon: model.icon, contextWindow: model.contextWindow, maxOutputTokens: model.maxOutputTokens, reasoningLevels: model.reasoningLevels)) { dismiss() } }.keyboardShortcut(.defaultAction).disabled(name.trimmingCharacters(in: .whitespacesAndNewlines).isEmpty || !draft.valid || store.isLoading) }.padding(16)
        }.frame(width: 440, height: 400).onAppear { name = template.name; draft = ProviderModelDraft(template.model) }
    }
}

struct RuntimeSettingsView: View {
    @ObservedObject var store: AppStore
    @State private var selectedID: String?
    @State private var editor: RuntimeInstance?
    @State private var creating = false
    @State private var discovering = false
    @State private var deleting: RuntimeInstance?
    private var selected: RuntimeInstance? { store.runtimeInstances.first { $0.id == selectedID } }
    var body: some View {
        VStack(spacing: 0) {
            List(selection: $selectedID) {
                ForEach(store.runtimeInstances) { instance in
                    VStack(alignment: .leading, spacing: 3) { HStack { Text(instance.name); if !instance.enabled { Text("已停用").foregroundStyle(.secondary) } }; Text(store.runtimeTypes.first { $0.id == instance.typeID }?.name ?? instance.typeID).font(.caption).foregroundStyle(.secondary) }.tag(instance.id)
                }
            }.contextMenu(forSelectionType: String.self) { ids in
                if let id = ids.first, let runtime = store.runtimeInstances.first(where: { $0.id == id }) { Button("编辑…") { editor = runtime }; Button(runtime.enabled ? "停用" : "启用") { var updated = runtime; updated.enabled.toggle(); store.saveRuntimeInstance(updated) }.disabled(store.isBusy); Button("删除…") { deleting = runtime } }
            } primaryAction: { ids in if let id = ids.first { editor = store.runtimeInstances.first { $0.id == id } } }
            .listStyle(.bordered).padding(.horizontal, 20).padding(.top, 16)
            HStack {
                Button { creating = true } label: { Image(systemName: "plus") }.help("添加Agent 运行时实例")
                Button("快速导入…") { discovering = true }.disabled(store.isBusy)
                Button { deleting = selected } label: { Image(systemName: "minus") }.disabled(selected == nil).help("删除运行时实例")
                Spacer()
                Button("编辑…") { editor = selected }.disabled(selected == nil)
            }.padding(.horizontal, 20).padding(.vertical, 12)

        }
        .sheet(isPresented: $discovering) { RuntimeDiscoveryView(store: store) }
        .sheet(item: $editor) { instance in RuntimeEditor(store: store, instance: instance) }
        .sheet(isPresented: $creating) { RuntimeEditor(store: store, instance: nil) }
        .alert("删除运行时实例？", isPresented: Binding(get: { deleting != nil }, set: { if !$0 { deleting = nil } }), presenting: deleting) { instance in Button("删除", role: .destructive) { store.deleteRuntimeInstance(id: instance.id); deleting = nil }; Button("取消", role: .cancel) { deleting = nil } } message: { instance in Text("删除「\(instance.name)」的配置，不删除运行时保存的会话。") }
    }
}

struct RuntimeEditor: View {
    @ObservedObject var store: AppStore
    let instance: RuntimeInstance?
    private var types: [RuntimeTypeDescriptor] { store.runtimeTypes }
    private var gatewayID: String { store.gateway.id }
    private var isSaving: Bool { store.isLoading }
    @Environment(\.dismiss) private var dismiss
    @State private var name = ""
    @State private var enabled = true
    @State private var typeID = ""
    @State private var settings: [String: String] = [:]
    @State private var draftID = UUID().uuidString
    @State private var typeDrafts: [String: [String: String]] = [:]
    @State private var executableDiscoveryRunning = false
    @State private var executableDiscoveryMessage: String?
    private var descriptor: RuntimeTypeDescriptor? { types.first { $0.id == typeID } }
    private var valid: Bool { !name.trimmingCharacters(in: .whitespacesAndNewlines).isEmpty && descriptor != nil && (!enabled || (descriptor?.fields.allSatisfy { field in
        let value = settings[field.key] ?? field.value
        return (!field.required || !value.trimmingCharacters(in: .whitespacesAndNewlines).isEmpty) && MacPath.isValid(value, field: field)
    } ?? false)) }
    private var pathValidationMessage: String? {
        guard enabled, let field = descriptor?.fields.first(where: { !MacPath.isValid(settings[$0.key] ?? $0.value, field: $0) }) else { return nil }
        return "「\(field.label)」必须是绝对路径；可使用 ~/ 开头，保存时会展开。"
    }
    var body: some View {
        VStack(spacing: 0) {
            ProblemsSheetHeading(title: instance == nil ? "添加Agent 运行时" : "编辑Agent 运行时", store: store).padding(20)
            Form {
                Section {
                    TextField("实例名称", text: $name)
                    Toggle("启用此运行时", isOn: $enabled).help("停用后不读取其历史，也不创建或执行新任务；原生会话数据保留。")
                    Picker("类型", selection: $typeID) { ForEach(types) { type in Text(type.name).tag(type.id) } }.disabled(instance != nil)
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
                        if let pathValidationMessage { Text(pathValidationMessage).font(.caption).foregroundStyle(.red) }
                        if let executableDiscoveryMessage {
                            Text(executableDiscoveryMessage).font(.caption).foregroundStyle(.secondary)
                        }
                    }
                }
            }.formStyle(.grouped)
            HStack { Spacer(); Button("取消") { dismiss() }.keyboardShortcut(.cancelAction); Button("保存") { commit() }.keyboardShortcut(.defaultAction).disabled(!valid || isSaving) }.padding(20)

        }.frame(width: 590, height: 540)
        .onAppear {
            name = instance?.name ?? ""
            enabled = instance?.enabled ?? true
            typeID = instance?.typeID ?? types.first?.id ?? ""
            settings = instance?.settings ?? [:]
            if let descriptor {
                for field in descriptor.fields where field.kind == .filePath || field.kind == .directoryPath {
                    if let value = settings[field.key] { settings[field.key] = MacPath.expanded(value) }
                }
            }
            DispatchQueue.main.async { discoverExecutableIfNeeded() }
        }
        .onChange(of: typeID) { old, new in
            if instance == nil { typeDrafts[old] = settings; settings = typeDrafts[new] ?? [:] }
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
            case .failure(let error): executableDiscoveryMessage = nil; store.recordProblem(error, source: "自动发现可执行文件")
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
        store.saveRuntimeInstance(RuntimeInstance(enabled: enabled, id: instance?.id ?? draftID, name: name.trimmingCharacters(in: .whitespacesAndNewlines), typeID: typeID, gatewayID: instance?.gatewayID ?? gatewayID, settings: values)) { dismiss() }
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
            ProblemsSheetHeading(title: "认证来源登录", store: store)
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
                ProblemsSheetHeading(title: title, store: store)
                ScrollView { Text(detail).textSelection(.enabled).frame(maxWidth: .infinity, alignment: .leading) }.frame(maxHeight: 220)
                HStack { Button("取消请求", role: .cancel) { reply(.cancel) }; Spacer(); ForEach(options) { option in Button(option.label) { reply(.decision(option.id)) } } }.disabled(store.isLoading)
            case .userInput(let questions):
                ProblemsSheetHeading(title: "运行时需要输入", store: store)
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

        }.padding(20).frame(width: 500).frame(minHeight: 180, maxHeight: 480)
        .interactiveDismissDisabled()
        .onChange(of: interaction.id) { _, _ in values = [:] }
    }
    private func value(_ id: String) -> Binding<String> { Binding(get: { values[id] ?? "" }, set: { values[id] = $0 }) }
    private func reply(_ value: RuntimeInteractionReply) { store.replyInteraction(interaction, reply: value) }
}

struct NewConversationView: View {
    @ObservedObject var store: AppStore
    @Environment(\.dismiss) private var dismiss
    @State private var runtimeID = ""
    @State private var modelRecordKey: String?
    @State private var cwd = ""
    private var compatibleModels: [ModelChoice] {
        guard let runtime = store.enabledRuntimeInstances.first(where: { $0.id == runtimeID }), let type = store.runtimeTypes.first(where: { $0.id == runtime.typeID }) else { return [] }
        return store.models.filter { type.supportedProviderProtocols.contains($0.protocolID) }
    }
    var body: some View {
        VStack(spacing: 0) {
            ProblemsSheetHeading(title: "新建会话", store: store).padding(20)
            Form {
                Picker("Agent 运行时", selection: $runtimeID) { ForEach(store.enabledRuntimeInstances) { Text($0.name).tag($0.id) } }
                Picker("模型", selection: $modelRecordKey) {
                    Text("选择会话模型").tag(Optional<String>.none)
                    ForEach(compatibleModels) { Text($0.displayName).tag(Optional($0.recordKey)) }
                }
                LabeledContent("工作目录") {
                    HStack { TextField("项目目录", text: $cwd); Button("选择…") { chooseDirectory() } }
                }
                Text("可选；留空时使用 Agent 运行时的默认工作目录。")
                    .font(.caption).foregroundStyle(.secondary)
                if store.enabledRuntimeInstances.isEmpty {
                    Text("请先添加 Agent 运行时实例。").foregroundStyle(.secondary)
                    SettingsLink { Text("打开运行时设置") }
                } else if compatibleModels.isEmpty {
                    Text("此运行时尚无兼容模型，请在 AI 提供商设置中添加。").foregroundStyle(.secondary)
                    SettingsLink { Text("打开 AI 提供商设置") }
                }
                Text("首次发送时自动准备运行时并创建会话；浏览已有会话不需要执行准备。").font(.caption).foregroundStyle(.secondary)
            }.formStyle(.grouped)

            HStack {
                Spacer(); Button("取消") { dismiss() }.keyboardShortcut(.cancelAction)
                Button("创建") {
                    if let modelRecordKey {
                        let trimmed = cwd.trimmingCharacters(in: .whitespacesAndNewlines)
                        let selectedCWD = trimmed.isEmpty ? nil : MacPath.expanded(trimmed)
                        store.createConversation(runtimeID: runtimeID, cwd: selectedCWD, modelRecordKey: modelRecordKey) { dismiss() }
                    }
                }.keyboardShortcut(.defaultAction).disabled(runtimeID.isEmpty || modelRecordKey == nil || store.isBusy)
            }.padding(16)
        }.frame(width: 480, height: 310)
        .onAppear { runtimeID = store.enabledRuntimeInstances.first(where: { $0.id == store.nextTurnRuntimeID })?.id ?? store.enabledRuntimeInstances.first?.id ?? "" }
        .onChange(of: runtimeID) { _, _ in modelRecordKey = nil }
    }
    private func chooseDirectory() {
        let panel = NSOpenPanel(); panel.title = "选择新会话的工作目录"; panel.canChooseDirectories = true; panel.canChooseFiles = false; panel.allowsMultipleSelection = false
        if panel.runModal() == .OK, let url = panel.url { cwd = url.standardizedFileURL.path }
    }
}
