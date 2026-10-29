import AppKit
import SwiftUI
import VeluneBindings

/// The conversation a side chat opens on and the next-turn choices it asks with.
struct SideChatTarget: Sendable {
    let runtimeID: String
    let conversationID: String
    let selection: ModelSelection?
    let effort: String?
    let permissionMode: String?
}

/// A tab's side chat (侧聊 in the PRD): questions on the shown conversation's
/// context, answered in a copy its runtime keeps and never saves. The core
/// knows it by its own identity; closing it, or the tab going away, ends it.
@MainActor final class SideChatModel: ObservableObject {
    let runtimeID: String
    /// The conversation it was opened on.
    let conversationID: String
    @Published private(set) var snapshot: ConversationSnapshot?
    @Published private(set) var failure: String?
    /// A request to the core is under way.
    @Published private(set) var isLoading = false
    @Published var draft = ""
    /// Asks the panel's field to take the keyboard.
    @Published private(set) var focusRequest = 0
    private let transport: Transport
    private let lease: SideChatLease
    private let queue = DispatchQueue(label: "local.velune.side-chat")
    private var id: String?
    private var polling = false
    private var closed = false

    init(transport: Transport, target: SideChatTarget) {
        self.transport = transport
        runtimeID = target.runtimeID
        conversationID = target.conversationID
        lease = SideChatLease(transport: transport)
        open(target)
    }

    var isRunning: Bool { snapshot.map { $0.runState == .running || $0.runState == .stopping } ?? false }
    var pendingInteraction: RuntimeInteraction? { snapshot?.pendingInteractions.first }
    var canSend: Bool {
        id != nil && !isLoading && !isRunning && pendingInteraction == nil && !draft.trimmingCharacters(in: .whitespacesAndNewlines).isEmpty
    }
    var canStop: Bool { id != nil && isRunning && snapshot?.actions.canCancel == true }

    func focus() { focusRequest &+= 1 }

    func send() {
        guard canSend, let id else { return }
        let text = draft
        draft = ""
        run({ [transport] in try transport.sendSideChat(sideChatID: id, text: text) }, onFailure: { [weak self] in
            if self?.draft.isEmpty == true { self?.draft = text }
        }) { [weak self] in self?.apply($0) }
    }

    func stop() {
        guard canStop, let id else { return }
        run({ [transport, runtimeID] in try transport.cancel(runtimeID: runtimeID, conversationID: id) }) { [weak self] in self?.apply($0) }
    }

    func reply(_ interaction: RuntimeInteraction, _ reply: RuntimeInteractionReply) {
        guard let id, !isLoading else { return }
        run({ [transport, runtimeID] in
            try transport.replyRuntimeInteraction(runtimeID: runtimeID, conversationID: id, interactionID: interaction.id, reply: reply)
        }) { [weak self] in self?.apply($0) }
    }

    /// Ends the side chat; nothing of it stays.
    func close() {
        guard !closed else { return }
        closed = true
        lease.release()
    }

    private func open(_ target: SideChatTarget) {
        run({ [transport] in
            try transport.openSideChat(runtimeID: target.runtimeID, conversationID: target.conversationID, selection: target.selection,
                                       effort: target.effort, permissionMode: target.permissionMode)
        }) { [weak self] result in
            guard let self else { return }
            guard let snapshot = result.snapshot else { failure = "侧聊未能打开"; return }
            id = snapshot.conversation.id
            lease.hold(snapshot.conversation.id)
            apply(result)
            focus()
        }
    }

    private func apply(_ result: SnapshotResult) {
        guard let value = result.snapshot, value.conversation.id == id else { return }
        snapshot = BindingMapping.snapshot(value)
        failure = nil
        if isRunning || pendingInteraction != nil { poll() }
    }

    /// Follows the answer while one is under way or a request waits.
    private func poll() {
        guard !polling, let id else { return }
        polling = true
        queue.asyncAfter(deadline: .now() + 0.4) { [weak self, transport, runtimeID] in
            let result = Result { try transport.snapshot(runtimeID: runtimeID, conversationID: id) }
            DispatchQueue.main.async {
                guard let self, !self.closed else { return }
                self.polling = false
                switch result {
                case .success(let value): self.apply(value)
                case .failure(let error): self.failure = error.localizedDescription
                }
            }
        }
    }

    private func run<T: Sendable>(_ operation: @escaping @Sendable () throws -> T, onFailure: (() -> Void)? = nil, apply: @escaping (T) -> Void) {
        guard !closed else { return }
        isLoading = true
        queue.async { [weak self, lease] in
            let result = Result { try operation() }
            DispatchQueue.main.async {
                guard let self else { return }
                self.isLoading = false
                // Closed while the side chat was opening: the lease ends it now.
                guard !self.closed else {
                    if let id = (try? result.get() as? SnapshotResult)?.snapshot?.conversation.id { lease.hold(id) }
                    return
                }
                switch result {
                case .success(let value): apply(value)
                case .failure(let error): self.failure = error.localizedDescription; onFailure?()
                }
            }
        }
    }
}

/// Closes the core's side chat once released or no longer held, however its
/// tab goes away.
private final class SideChatLease: @unchecked Sendable {
    private let transport: Transport
    private let lock = NSLock()
    private var id: String?
    private var released = false

    init(transport: Transport) { self.transport = transport }

    func hold(_ id: String) {
        lock.withLock { self.id = id }
        if lock.withLock({ released }) { release() }
    }

    func release() {
        let id: String? = lock.withLock {
            released = true
            defer { self.id = nil }
            return self.id
        }
        guard let id else { return }
        let transport = transport
        DispatchQueue.global(qos: .utility).async { try? transport.closeSideChat(sideChatID: id) }
    }

    deinit { release() }
}

/// The side chat beside the transcript: what was asked and answered, the
/// runtime's requests, and a field for the next question.
struct SideChatPanel: View {
    @ObservedObject var chat: SideChatModel
    let close: () -> Void
    @FocusState private var fieldFocused: Bool

    var body: some View {
        VStack(spacing: 0) {
            HStack(spacing: 6) {
                Text("侧聊").font(.headline)
                Spacer(minLength: 8)
                if chat.isRunning {
                    Button(action: chat.stop) { Image(systemName: "stop.fill") }
                        .buttonStyle(.borderless).disabled(!chat.canStop).help("停止回答")
                }
                Button(action: close) { Image(systemName: "xmark") }
                    .buttonStyle(.borderless).help("关闭侧聊；侧聊不保存")
            }
            .padding(.horizontal, 10)
            .paneBar()
            messages
            if let interaction = chat.pendingInteraction {
                SideChatInteractionView(chat: chat, interaction: interaction).id(interaction.id)
            }
            if let failure = chat.failure {
                Text(failure).font(.callout).foregroundStyle(.red).textSelection(.enabled)
                    .frame(maxWidth: .infinity, alignment: .leading).padding(.horizontal, 12).padding(.vertical, 6)
            }
            Divider()
            HStack(alignment: .bottom, spacing: 6) {
                TextField("就此会话另外提问", text: $chat.draft, axis: .vertical)
                    .textFieldStyle(.plain).lineLimit(1...8)
                    .focused($fieldFocused)
                    .onSubmit(chat.send)
                Button(action: chat.send) { Image(systemName: "arrow.up.circle.fill").font(.title3) }
                    .buttonStyle(.borderless).disabled(!chat.canSend).help("发送（Return）")
            }
            .padding(10)
        }
        .background(Color(nsColor: .textBackgroundColor))
        .onChange(of: chat.focusRequest, initial: true) { _, _ in DispatchQueue.main.async { fieldFocused = true } }
        .accessibilityElement(children: .contain)
        .accessibilityLabel("侧聊")
    }

    private var messages: some View {
        ScrollViewReader { proxy in
            ScrollView {
                LazyVStack(alignment: .leading, spacing: 12) {
                    if chat.snapshot == nil {
                        ProgressView("正在打开侧聊…").controlSize(.small).frame(maxWidth: .infinity)
                    } else if chat.snapshot?.messages.isEmpty == true {
                        Text("在不打断、不改变会话的情况下，就它的上下文另外提问。侧聊关闭即丢弃。")
                            .font(.callout).foregroundStyle(.secondary)
                    }
                    ForEach(chat.snapshot?.messages ?? [], id: \.id) { message in
                        SideChatMessageView(message: message)
                    }
                    if chat.isRunning { ProgressView().controlSize(.small) }
                    Color.clear.frame(height: 1).id("end")
                }
                .padding(12)
            }
            .onChange(of: chat.snapshot?.revision) { _, _ in proxy.scrollTo("end", anchor: .bottom) }
        }
        .frame(maxHeight: .infinity)
    }
}

/// One message: a question in a bubble, an answer as text.
private struct SideChatMessageView: View {
    let message: DeliveredMessage

    var body: some View {
        if message.role == .user {
            Text(plain)
                .textSelection(.enabled)
                .padding(.horizontal, 10).padding(.vertical, 6)
                .background(Color(nsColor: .quaternaryLabelColor).opacity(0.5), in: RoundedRectangle(cornerRadius: 8))
                .frame(maxWidth: .infinity, alignment: .trailing)
        } else {
            VStack(alignment: .leading, spacing: 6) {
                ForEach(Array(message.blocks.enumerated()), id: \.offset) { _, block in
                    switch block {
                    case .text(let text):
                        Text(Self.markdown(text)).textSelection(.enabled)
                    case .notice(let text), .contextCompaction(let text):
                        Text(text).font(.callout).foregroundStyle(.secondary).textSelection(.enabled)
                    case .tool(_, let title, let state, _, _):
                        Label(title, systemImage: state == .running ? "hammer" : "checkmark.circle")
                            .font(.callout).foregroundStyle(.secondary).lineLimit(2)
                    case .reasoning(let summary, _):
                        if !summary.isEmpty { Text(summary).font(.callout).foregroundStyle(.secondary).italic() }
                    default:
                        EmptyView()
                    }
                }
            }
            .frame(maxWidth: .infinity, alignment: .leading)
        }
    }

    private var plain: String {
        message.blocks.compactMap { if case .text(let text) = $0 { text } else { nil } }.joined(separator: "\n")
    }

    static func markdown(_ text: String) -> AttributedString {
        (try? AttributedString(markdown: text, options: .init(interpretedSyntax: .inlineOnlyPreservingWhitespace))) ?? AttributedString(text)
    }
}

/// A runtime's request in the side chat: an approval's options, or its questions.
private struct SideChatInteractionView: View {
    @ObservedObject var chat: SideChatModel
    let interaction: RuntimeInteraction
    @State private var values: [String: String] = [:]

    var body: some View {
        VStack(alignment: .leading, spacing: 8) {
            switch interaction.kind {
            case .approval(let title, let detail, let options):
                Text(title).font(.headline)
                if !detail.isEmpty {
                    ScrollView { Text(detail).font(.callout).textSelection(.enabled).frame(maxWidth: .infinity, alignment: .leading) }
                        .frame(maxHeight: 120).fixedSize(horizontal: false, vertical: true)
                }
                HStack {
                    Button("取消请求", role: .cancel) { chat.reply(interaction, .cancel) }
                    Spacer(minLength: 0)
                    ForEach(options, id: \.id) { option in
                        Button(option.label) { chat.reply(interaction, .decision(optionId: option.id)) }
                    }
                }
            case .userInput(let questions):
                Text("运行时需要输入").font(.headline)
                ForEach(questions, id: \.id) { question in
                    Text(question.text).textSelection(.enabled)
                    if question.options.isEmpty {
                        TextField("回答", text: value(question.id))
                    } else {
                        Picker("选择", selection: value(question.id)) {
                            Text("请选择").tag("")
                            ForEach(question.options, id: \.id) { Text($0.label).tag($0.id) }
                        }.labelsHidden()
                    }
                }
                HStack {
                    Button("取消请求", role: .cancel) { chat.reply(interaction, .cancel) }
                    Spacer(minLength: 0)
                    Button("提交") {
                        chat.reply(interaction, .answers(answers: questions.map { InteractionAnswer(questionId: $0.id, values: [values[$0.id] ?? ""]) }))
                    }
                    .disabled(questions.contains { (values[$0.id] ?? "").isEmpty })
                }
            }
        }
        .disabled(chat.isLoading)
        .padding(10)
        .background(Color(nsColor: .controlBackgroundColor), in: RoundedRectangle(cornerRadius: 8))
        .overlay { RoundedRectangle(cornerRadius: 8).strokeBorder(Color(nsColor: .separatorColor)) }
        .padding(.horizontal, 10).padding(.bottom, 8)
    }

    private func value(_ id: String) -> Binding<String> { Binding(get: { values[id] ?? "" }, set: { values[id] = $0 }) }
}
