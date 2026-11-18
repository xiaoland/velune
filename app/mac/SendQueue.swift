import SwiftUI
import VeluneBindings

/// The conversation's send queue, in its detail: messages that go out
/// one by one after the running turn. A system list: click selects, double
/// click edits, drag reorders, Delete removes; the context menu has the rest.
struct SendQueueView: View {
    @ObservedObject var store: ConversationStore
    @ObservedObject var composer: ComposerModel
    @Binding var editing: QueuedMessageEditingSession?
    @State private var selection = Set<String>()

    private var queue: [QueuedMessage] { store.followUp.queue }
    /// Rows shown before the list scrolls.
    private static let visibleRows = 4
    private static let rowHeight: CGFloat = 24

    var body: some View {
        VStack(alignment: .leading, spacing: 4) {
            header
            if !queue.isEmpty { list }
        }
        .onChange(of: queue.map(\.id)) { _, ids in selection.formIntersection(ids) }
        .onChange(of: composer.queueEditRequest) { _, request in
            guard let request else { return }
            composer.queueEditRequest = nil
            openEditor(request.id)
        }
    }

    private func openEditor(_ id: String) {
        guard editing == nil, let conversationID = store.loadedConversationID,
              let message = queue.first(where: { $0.id == id }),
              let context = message.editContext else { return }
        editing = QueuedMessageEditingSession(store: store, conversationID: conversationID, message: message, context: context)
    }

    private var header: some View {
        HStack(spacing: 6) {
            if let pause = store.followUp.paused {
                Label(pause.explanation, systemImage: "pause.circle")
                Spacer(minLength: 8)
                if !queue.isEmpty { Button("继续发送", action: store.resumeQueue).controlSize(.small).disabled(store.isLoading || !store.inputSubmissionAllowed) }
            } else if store.followUp.waiting == .control {
                Label("等待会话控制权", systemImage: "lock")
                    .help(store.inputLockMessage ?? "正在确认会话控制权")
                Spacer(minLength: 0)
            } else if store.followUp.waitingForSubAgents {
                Label("子 Agent 仍在运行；结束后发送", systemImage: "hourglass")
                    .help("下一条需要重新启动运行时，这会结束仍在运行的子 Agent")
                Spacer(minLength: 8)
                Button("停止子 Agent 并发送", action: store.stopSubAgents).controlSize(.small)
                    .disabled(!store.followUp.canStopSubAgents || store.isLoading)
            } else {
                Text("排队 \(queue.count) 条")
                Spacer(minLength: 0)
            }
        }
        .uiFont(.callout).foregroundStyle(.secondary)
    }

    private var list: some View {
        List(selection: $selection) {
            ForEach(queue) { message in
                HStack(spacing: 6) {
                    Text(message.text.isEmpty ? "（仅附件）" : message.text).lineLimit(1).truncationMode(.tail)
                    Spacer(minLength: 4)
                    if !message.attachments.isEmpty {
                        Label("\(message.attachments.count)", systemImage: "paperclip").labelStyle(.titleAndIcon).foregroundStyle(.secondary)
                    }
                }
                .help(message.text)
                .tag(message.id)
            }
            .onMove { from, to in
                guard let index = from.first, from.count == 1 else { return }
                store.moveQueued(queue[index].id, to: to > index ? to - 1 : to)
            }
        }
        .listStyle(.bordered(alternatesRowBackgrounds: false))
        .environment(\.defaultMinListRowHeight, Self.rowHeight)
        .frame(height: CGFloat(min(queue.count, Self.visibleRows)) * Self.rowHeight + 2)
        .accessibilityLabel("发送队列")
        .onDeleteCommand { remove(selection) }
        .contextMenu(forSelectionType: String.self) { ids in
            if ids.count == 1, let message = queue.first(where: { ids.contains($0.id) }), let index = queue.firstIndex(of: message) {
                Button("编辑…") { openEditor(message.id) }.disabled(message.editContext == nil)
                Button("插入本轮") { store.steerQueued(message.id) }.disabled(!store.followUp.canSteer || store.isLoading)
                Divider()
                Button("移到最前") { store.moveQueued(message.id, to: 0) }.disabled(index == 0)
                Button("上移") { store.moveQueued(message.id, to: index - 1) }.disabled(index == 0)
                Button("下移") { store.moveQueued(message.id, to: index + 1) }.disabled(index == queue.count - 1)
                Divider()
            }
            if !ids.isEmpty { Button(ids.count > 1 ? "删除 \(ids.count) 条" : "删除") { remove(ids) } }
        } primaryAction: { ids in
            if ids.count == 1, let id = ids.first { openEditor(id) }
        }
    }

    private func remove(_ ids: Set<String>) {
        for message in queue where ids.contains(message.id) { store.removeQueued(message.id) }
    }
}

@MainActor final class QueuedMessageEditingSession: ObservableObject, Identifiable {
    let id = UUID()
    let store: ConversationStore
    let editor = ComposerModel()
    private let conversationID: String
    private let message: QueuedMessage
    private let captured: QueuedMessageEditContext
    private let originalChoice: TurnChoice
    @Published private(set) var retained: [ComposerRetainedAttachment]
    @Published private var choices: ConversationChoices
    @Published private(set) var saving = false
    @Published private(set) var failure: String?

    init(store: ConversationStore, conversationID: String, message: QueuedMessage, context: QueuedMessageEditContext) {
        self.store = store
        self.conversationID = conversationID
        self.message = message
        self.captured = context
        let original = TurnChoice(model: context.selection, effort: context.effort, permissionMode: context.permissionMode)
        self.originalChoice = original
        self.choices = ConversationChoices(runtimeID: context.runtimeId, choices: [context.runtimeId: original])
        self.retained = message.attachments.enumerated().map { index, path in
            let name = URL(fileURLWithPath: path.replacingOccurrences(of: "\\", with: "/")).lastPathComponent
            return ComposerRetainedAttachment(id: UInt32(index), name: name)
        }
        editor.attach(store, policy: .messageEdit)
        editor.receiveDraft(.init(text: message.text, attachments: []))
    }

    private var runtimeID: String { choices.runtimeID ?? captured.runtimeId }
    private var choice: TurnChoice { choices.choices[runtimeID] ?? TurnChoice() }
    private var executionChanged: Bool { runtimeID != captured.runtimeId || choice != originalChoice }
    private var next: NextTurn {
        let runtime = store.device.runtimeInstances.first { $0.id == runtimeID }
        let catalog = store.modelCatalogs[runtimeID]?.models
        var target = resolveNextTurn(input: NextTurnInput(
            runtimeId: runtimeID, runtimeType: store.device.runtimeTypes.first { $0.id == runtime?.typeId },
            providers: store.device.gateway.providers, catalog: catalog, kept: choice,
            newConversation: .chosen(choice: TurnChoice())))
        // Show captured values while the runtime's catalog is loading. An
        // unmodified request retains these values exactly, even if the menu changes.
        if choice == originalChoice && runtimeID == captured.runtimeId { target.effort = choice.effort }
        return target
    }

    var issue: String? {
        guard store.loadedConversationID == conversationID,
              store.selectedConversationID == nil || store.selectedConversationID == conversationID,
              let current = store.followUp.queue.first(where: { $0.id == message.id }) else {
            return "这条消息已开始发送或已离开队列。你仍可复制修改后的内容。"
        }
        guard current.editContext?.revision == captured.revision else {
            return "这条消息已在另一处修改。请重新打开编辑，当前内容仍可复制。"
        }
        if executionChanged {
            if !store.device.enabledRuntimeInstances.contains(where: { $0.id == runtimeID }) { return "所选运行时不可用。" }
            if next.chosenModelUnavailable { return "所选模型不可用，请重新选择。" }
        }
        if editor.attachments.contains(where: { !FileManager.default.isReadableFile(atPath: $0.path) }) {
            return "有新增附件无法读取，请重新添加或移除该附件。"
        }
        return nil
    }

    var canSave: Bool {
        !saving && !store.isLoading && !store.isShuttingDown && issue == nil &&
            !editor.isComposing && (!editor.isEmpty || !retained.isEmpty || !editor.attachments.isEmpty)
    }
    var operation: SendQueueOperation? {
        guard canSave else { return nil }
        return .replace(id: message.id, expectedRevision: captured.revision, targetRuntimeId: runtimeID,
                        selection: choice.model, retainedAttachments: retained.map(\.id),
                        turn: .init(text: editor.submission, effort: choice.effort, permissionMode: choice.permissionMode,
                                    attachments: editor.attachments.map(\.path)))
    }
    func removeRetained(_ id: UInt32) {
        guard !saving else { return }
        retained.removeAll { $0.id == id }
    }
    func loadCatalog() { store.loadModelCatalog(runtimeID: runtimeID) }

    func target(cancel: (() -> Void)? = nil) -> ComposerEditingTarget {
        ComposerEditingTarget(nextTurn: next, issue: issue, enabled: canSave,
                              submitTitle: saving ? "正在保存…" : "保存修改",
                              chooseRuntime: { [weak self] id in self?.chooseRuntime(id) },
                              chooseModel: { [weak self] model in self?.chooseModel(model) },
                              chooseEffort: { [weak self] effort in self?.updateChoice { $0.effort = effort } },
                              choosePermissionMode: { [weak self] mode in self?.updateChoice { $0.permissionMode = mode } },
                              editable: !saving, cancel: cancel, canCancel: !saving,
                              allowsUnavailableModel: !executionChanged,
                              retainedAttachments: retained,
                              removeRetainedAttachment: { [weak self] id in self?.removeRetained(id) })
    }
    var target: ComposerEditingTarget { target(cancel: nil) }

    private func chooseRuntime(_ id: String) {
        guard !saving else { return }
        choices.runtimeID = id
        loadCatalog()
    }
    private func chooseModel(_ model: ModelSelection?) {
        updateChoice {
            if $0.model != model { $0.effort = nil }
            $0.model = model
        }
    }
    private func updateChoice(_ change: (inout TurnChoice) -> Void) {
        guard !saving else { return }
        var updated = choice
        change(&updated)
        choices.choices[runtimeID] = updated
    }
    func save(onSaved: @escaping () -> Void) {
        guard let operation else { return }
        saving = true
        failure = nil
        store.replaceQueued(operation, conversationID: conversationID) { [weak self] error in
            guard let self else { return }
            saving = false
            if let error { failure = error }
            else { onSaved() }
        }
    }
}

struct QueuedMessageEditor: View {
    @ObservedObject var session: QueuedMessageEditingSession
    let window: ConversationWindowModel
    let tab: ConversationTab
    @ObservedObject private var store: ConversationStore
    @ObservedObject private var editor: ComposerModel
    @Environment(\.dismiss) private var dismiss

    init(session: QueuedMessageEditingSession, window: ConversationWindowModel, tab: ConversationTab) {
        self.session = session
        self.window = window
        self.tab = tab
        self.store = session.store
        self.editor = session.editor
    }

    var body: some View {
        let target = session.target(cancel: { dismiss() })
        VStack(alignment: .leading, spacing: 0) {
            Text("编辑排队消息").uiFont(.headline).padding(.horizontal, 16).padding(.top, 16)
            ComposerView(store: store, model: editor, layout: window,
                         send: { session.save { dismiss() } }, steer: {},
                         submission: { ComposerContext(tab: tab, model: editor, editingTarget: target).submission() },
                         resume: {}, editorMinimumHeight: 76, editingTarget: target, inputTab: tab)
            if let notice = session.failure ?? session.issue {
                HStack(alignment: .top, spacing: 8) {
                    Text(notice).uiFont(.callout).foregroundStyle(.secondary)
                        .lineLimit(3).textSelection(.enabled).help(notice)
                    Spacer(minLength: 0)
                }.padding(.horizontal, 16).padding(.bottom, 14)
            }
        }
        .frame(width: 540)
        .interactiveDismissDisabled(session.saving)
        .onAppear { session.loadCatalog() }
    }
}
