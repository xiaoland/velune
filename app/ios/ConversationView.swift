import SwiftUI

struct ConversationView: View {
    @ObservedObject var model: ConversationModel
    let active: [BindingActiveConversation]
    let openLinked: (String, Bool) -> Void
    let readChanged: () -> Void
    var runtimes: [BindingRuntimeInstance] = []
    var projects: [BindingConversationProject] = []
    var deviceName = ""
    @Environment(\.scenePhase) private var phase
    var body: some View {
        ConversationContent(model: model, active: active, openLinked: openLinked, runtimes: runtimes, projects: projects, deviceName: deviceName)
        .task(id: phase) {
            guard phase == .active else { return }
            model.resumeReading()
            if model.loading { await model.open() }
            while !Task.isCancelled {
                if !Task.isCancelled, await model.markSeen() { readChanged() }
                do { try await Task.sleep(for: .milliseconds(model.snapshot?.runState == .running ? 800 : 3000)) }
                catch { return }
                await model.poll()
            }
        }
        .task(id: model.runtimeID) { await model.loadModels() }
        .onDisappear { model.onComposerChange?(model) }
    }
}

struct ConversationContent: View {
    @ObservedObject var model: ConversationModel
    var active: [BindingActiveConversation] = []
    var openLinked: (String, Bool) -> Void = { _, _ in }
    let runtimes: [BindingRuntimeInstance]
    let projects: [BindingConversationProject]
    let deviceName: String
    @State private var following: Bool
    @State private var restored = false
    @State private var position = ScrollPosition(edge: .bottom)
    @SceneStorage("conversationInspector") private var inspectorVisible = false
    @SceneStorage("conversationPanel") private var panelName = ConversationPanel.outline.rawValue
    private var panel: Binding<ConversationPanel> { Binding(get: { ConversationPanel(rawValue: panelName) ?? .outline }, set: { panelName = $0.rawValue }) }
    init(model: ConversationModel, active: [BindingActiveConversation] = [], openLinked: @escaping (String, Bool) -> Void = { _, _ in }, runtimes: [BindingRuntimeInstance] = [], projects: [BindingConversationProject] = [], deviceName: String = "") {
        self.model = model; self.active = active; self.openLinked = openLinked
        self.runtimes = runtimes; self.projects = projects; self.deviceName = deviceName
        _following = State(initialValue: model.following)
    }
    @State private var isNearBottom = true
    @State private var scrollPhase = ScrollPhase.idle
    @FocusState private var composing: Bool
    @Environment(\.horizontalSizeClass) private var sizeClass
    @Environment(\.openWindow) private var openWindow
    @Environment(\.supportsMultipleWindows) private var supportsMultipleWindows
    var body: some View {
        ScrollView {
                VStack(spacing: 20) {
                    if model.loading { ProgressView("读取会话…") }
                    TranscriptView(model: model, openLinked: openLinked)
                    Color.clear.frame(height: 1).id("bottom")
                }.padding(20).frame(maxWidth: 820).frame(maxWidth: .infinity)
            }
            .overlay {
                if model.isNew && !model.loading && model.draft.isEmpty {
                    ContentUnavailableView("新会话", systemImage: "square.and.pencil")
                        .allowsHitTesting(false)
                } else if !model.isNew && model.snapshot == nil && !model.loading {
                    ContentUnavailableView {
                        Label("未能读取会话", systemImage: "wifi.exclamationmark")
                    } actions: {
                        Button("重新读取") { Task { await model.open() } }.buttonStyle(.bordered)
                        ProblemsButton()
                    }
                }
            }
            .scrollPosition($position)
            .scrollDismissesKeyboard(.interactively)
            .defaultScrollAnchor(.bottom, for: .initialOffset)
            .defaultScrollAnchor(.top, for: .alignment)
            .onScrollGeometryChange(for: TranscriptPosition.self) { TranscriptPosition($0) } action: { old, new in
                isNearBottom = new.isNearBottom
                if !restored && !model.loading && new.contentHeight > 0 {
                    restored = true
                    if !following { position.scrollTo(y: model.readingOffset); return }
                }
                if restored { model.readingOffset = new.offset }
                // Content growth changes position, but does not mean the user stopped following.
                if following && scrollPhase == .idle && (old.contentHeight != new.contentHeight || old.visibleHeight != new.visibleHeight) {
                    position.scrollTo(edge: .bottom)
                }
            }
            .onScrollPhaseChange { old, new, context in
                scrollPhase = new
                if new == .interacting { following = false }
                if new == .idle {
                    if old == .interacting || old == .decelerating {
                        following = TranscriptPosition(context.geometry).isNearBottom
                    }
                    if following { position.scrollTo(edge: .bottom) }
                }
            }
            .overlay(alignment: .bottomTrailing) {
                if !isNearBottom {
                    Button {
                        following = true
                        withAnimation { position.scrollTo(edge: .bottom) }
                    } label: {
                        Image(systemName: "arrow.down").padding(12).background(.regularMaterial, in: Circle())
                    }.padding().accessibilityLabel("回到最新")
                }
            }
            .onChange(of: model.sending) { _, value in
                if value != nil { following = true; position.scrollTo(edge: .bottom) }
            }
        .focusedSceneValue(\.conversationActions, ConversationKeyboardActions(
            send: model.canSend ? sendFromKeyboard : nil,
            stop: model.snapshot?.actions.canCancel == true && !model.busy ? { model.stop() } : nil,
            focus: { composing = true }, latest: { following = true; position.scrollTo(edge: .bottom) },
            outline: { show(.outline) }, agents: { show(.agents) }))
        .onAppear { if model.isNew { composing = true } }
        .onChange(of: following) { _, value in model.following = value }
        .toolbar {
            ToolbarItemGroup(placement: .topBarTrailing) {
                Button { inspectorVisible.toggle() } label: { Label("会话面板", systemImage: "sidebar.right") }
                    .accessibilityValue(inspectorVisible ? panelName : "已关闭")
                    .help("大纲与关联 Agent")
                Menu {
                    Button("大纲", systemImage: "list.bullet") { show(.outline) }
                    Button("关联 Agent", systemImage: "person.2") { show(.agents) }
                    Divider()
                    Button("聚焦输入", systemImage: "text.cursor") { composing = true }
                    Button("发送消息", systemImage: "arrow.up") { model.send() }.disabled(!model.canSend)
                    Button("压缩上下文", systemImage: "arrow.down.right.and.arrow.up.left") { model.compact() }.disabled(model.snapshot?.followUp.canCompact != true || model.busy)
                    Button("停止生成", systemImage: "stop") { model.stop() }.disabled(model.snapshot?.actions.canCancel != true || model.busy)
                    Button("回到最新", systemImage: "arrow.down") { following = true; position.scrollTo(edge: .bottom) }
                    if supportsMultipleWindows, model.snapshot != nil {
                        Button("在新窗口中打开", systemImage: "macwindow.badge.plus") {
                            openWindow(id: "workspace", value: WindowRoute(peer: model.peer, target: model.currentTarget))
                        }
                    }
                    ProblemsButton()
                } label: { Label("会话操作", systemImage: "ellipsis.circle") }
            }
        }
        .navigationTitle(model.title)
        .toolbar {
            ToolbarItem(placement: .principal) {
                VStack(spacing: 2) {
                    Text(model.title).font(.headline).lineLimit(1)
                    if !deviceName.isEmpty { Text(deviceName).font(.caption).foregroundStyle(.secondary).lineLimit(1) }
                }
                .accessibilityElement(children: .combine)
            }
        }
        .navigationBarTitleDisplayMode(.inline)
        .safeAreaInset(edge: .bottom, spacing: 0) {
            VStack(spacing: 10) {
                if let interactions = model.snapshot?.pendingInteractions, !interactions.isEmpty {
                    ScrollView {
                        VStack(spacing: 8) {
                            ForEach(interactions, id: \.id) { interaction in
                                InteractionView(interaction: interaction, busy: model.busy) { model.reply(interaction.id, $0) }
                            }
                        }
                    }.frame(maxHeight: 240)
                }
                if model.snapshot?.conversation.parentConversationId != nil {
                    Label("子 Agent 会话 · 只读", systemImage: "lock").font(.callout).foregroundStyle(.secondary)
                } else {
                    ComposerView(model: model, runtimes: runtimes, projects: projects, composing: $composing, sendFromKeyboard: sendFromKeyboard)
                }
            }.padding(.horizontal, 16).padding(.vertical, 8).frame(maxWidth: 852).frame(maxWidth: .infinity).background(.bar)
        }
        .inspector(isPresented: $inspectorVisible) {
            ConversationInspector(model: model, active: active, panel: panel, jump: { id in
                following = false
                position.scrollTo(id: id, anchor: .top)
                if sizeClass == .compact { inspectorVisible = false }
            }, open: { id, newWindow in
                if sizeClass == .compact { inspectorVisible = false }
                openLinked(id, newWindow)
            }, close: { inspectorVisible = false })
            .inspectorColumnWidth(min: 260, ideal: 290, max: 340)
            .presentationDetents([.medium, .large])
            .presentationDragIndicator(.visible)
        }
    }
    private func sendFromKeyboard() {
        guard model.canSend else { return }
        model.send()
        // Reapply input focus after the keyboard command is dispatched.
        DispatchQueue.main.async { composing = true }
    }
    private func show(_ requested: ConversationPanel) {
        if inspectorVisible && panelName == requested.rawValue { inspectorVisible = false }
        else { panelName = requested.rawValue; inspectorVisible = true }
    }
}

private struct TranscriptPosition: Equatable {
    let contentHeight: CGFloat
    let visibleHeight: CGFloat
    let visibleBottom: CGFloat
    let offset: CGFloat
    var isNearBottom: Bool { contentHeight - visibleBottom <= 60 }
    init(_ geometry: ScrollGeometry) {
        contentHeight = geometry.contentSize.height
        visibleHeight = geometry.visibleRect.height
        visibleBottom = geometry.visibleRect.maxY
        offset = geometry.contentOffset.y + geometry.contentInsets.top
    }
}

private struct InteractionView: View {
    let interaction: BindingRuntimeInteraction
    let busy: Bool
    let reply: (BindingRuntimeInteractionReply) -> Void
    @State private var answers: [String: String] = [:]
    @State private var selected: [String: String] = [:]
    var body: some View {
        VStack(alignment: .leading, spacing: 10) {
            switch interaction.kind {
            case .approval(let title, let detail, let options):
                Text(title).font(.headline)
                if !detail.isEmpty { Text(detail).font(.callout).textSelection(.enabled) }
                ViewThatFits {
                    HStack { approvalButtons(options) }
                    VStack(alignment: .leading) { approvalButtons(options) }
                }
            case .userInput(let questions):
                ForEach(questions, id: \.id) { question in
                    Text(question.text).font(.callout)
                    if question.secret {
                        SecureField("回答", text: answer(question.id)).textFieldStyle(.roundedBorder)
                    } else {
                        TextField("回答", text: answer(question.id), axis: .vertical).textFieldStyle(.roundedBorder)
                    }
                    if !question.options.isEmpty {
                        ScrollView(.horizontal) {
                            HStack { ForEach(question.options, id: \.id) { option in
                                Button {
                                    selected[question.id] = option.id; answers[question.id] = ""
                                } label: {
                                    Label(option.label, systemImage: selected[question.id] == option.id ? "checkmark.circle.fill" : "circle")
                                }.buttonStyle(.bordered)
                            } }
                        }
                    }
                }
                HStack {
                    Button("取消") { reply(.cancel) }
                    Spacer()
                    Button("提交") { reply(.answers(answers: questions.map { .init(questionId: $0.id, values: [value($0.id)]) })) }
                        .disabled(questions.contains { (value($0.id)).trimmingCharacters(in: .whitespacesAndNewlines).isEmpty })
                }
            }
        }.padding(12).background(.quaternary, in: RoundedRectangle(cornerRadius: 12)).disabled(busy)
    }
    private func value(_ id: String) -> String { selected[id] ?? answers[id] ?? "" }
    private func answer(_ id: String) -> Binding<String> { Binding(get: { answers[id] ?? "" }, set: { answers[id] = $0; selected[id] = nil }) }
    private func approvalButtons(_ options: [BindingInteractionOption]) -> some View {
        ForEach(options, id: \.id) { option in Button(option.label) { reply(.decision(optionId: option.id)) }.buttonStyle(.bordered) }
    }
}

struct FailureView: View {
    let text: String
    var body: some View { Label(text, systemImage: "exclamationmark.triangle").font(.callout).foregroundStyle(.red).textSelection(.enabled) }
}
