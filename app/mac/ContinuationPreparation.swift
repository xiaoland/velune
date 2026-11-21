import Foundation
import Combine
import SwiftUI
import VeluneBindings

/// UI projections of the core's preparation contract. No summary text crosses
/// this surface: the core owns its temporary context and opaque prepared token.
enum ContinuationRequirement: Sendable {
    case checking(String)
    case ready
    case summary(NextTurnInput, String?)
    case failed(String)
}
struct ContinuationQueuedWork: Sendable {
    let workID: String
    let queuedMessageID: String
    let result: SnapshotResult?
}
struct ContinuationPreparationOperations: Sendable {
    var requirements: @Sendable (String?) throws -> ContinuationRequirement
    var queue: @Sendable (String?, ModelSelection?, String?) throws -> ContinuationQueuedWork
}

extension ContinuationPreparationOperations {
    static func live(transport: Transport, sourceRuntimeID: String, conversationID: String, target: NextTurn, turn: ComposedTurn?) -> Self {
        let submissionId = "submission:" + UUID().uuidString.lowercased()
        return Self(requirements: { requestID in
            switch try transport.handoffRequirements(sourceRuntimeID: sourceRuntimeID, conversationID: conversationID,
                                                     targetRuntimeID: target.runtimeId, requestID: requestID) {
            case .checking(let id): return .checking(id)
            case .ready: return .ready
            case .portableSummaryRequired(let basis, let issue): return .summary(basis, issue)
            case .failed(_, let detail): return .failed(detail)
            }
        }, queue: { queuedMessageID, model, effort in
            let accepted: QueuedHandoffSummaryStart
            if let queuedMessageID {
                accepted = try transport.retryQueuedHandoffSummary(sourceRuntimeID: sourceRuntimeID, conversationID: conversationID,
                    queuedMessageID: queuedMessageID, model: model, effort: effort)
            } else {
                guard let turn else { throw TransportError.rejected("没有可受理的待发送消息") }
                accepted = try transport.queueHandoffSummary(sourceRuntimeID: sourceRuntimeID, conversationID: conversationID,
                    targetRuntimeID: target.runtimeId, selection: target.model, turn: turn, model: model, effort: effort, submissionId: submissionId)
            }
            return ContinuationQueuedWork(workID: accepted.workId, queuedMessageID: accepted.queuedMessageId, result: accepted.result)
        })
    }
}

/// One confirmation/acceptance transaction. Runtime work belongs to the Core
/// queue after acknowledgement, never to this sheet or tab's lifetime.
@MainActor final class ContinuationPreparationSession: ObservableObject, Identifiable {
    enum Phase: Equatable { case checking, ready, queueing, sending, failed, closed }
    let id = UUID()
    let input: ComposerModel
    private let initialDraft: SavedComposerDraft
    private let initialRevision: UInt64
    var text: String { initialDraft.text }
    var attachments: [URL] { initialDraft.attachments }
    let target: NextTurn
    let sourceRuntimeID: String
    let sourceContextRuntimeID: String
    let conversationID: String
    private weak var source: ConversationStore?
    private let operations: ContinuationPreparationOperations
    private let send: (ContinuationPreparationSession) -> Void
    private let finish: (UUID) -> Void
    private let retainInput: (ComposerModel) -> Void
    private let receivedQueue: (SnapshotResult) -> Void
    private struct RecoveryInput { let model: ComposerModel; let revision: UInt64; let draft: SavedComposerDraft }
    private var recoveryInput: RecoveryInput?
    private var submissionAccepted: Bool
    private let existingQueuedMessageID: String?
    private var requirementsRequestID: String?
    private var queueAfterRequirements = false
    private var hasConfirmed = false
    private var summaryChoiceEdited = false
    private var closed = false
    @Published private(set) var phase: Phase = .checking
    @Published private(set) var failure: String?
    @Published private(set) var isPresented = false
    @Published private(set) var summaryChoice = TurnChoice()
    @Published private(set) var summaryBasis: NextTurnInput?
    @Published private(set) var defaultModelIssue: String?

    init(source: ConversationStore, input: ComposerModel, target: NextTurn,
         sourceRuntimeID: String, sourceContextRuntimeID: String, conversationID: String,
         operations: ContinuationPreparationOperations,
         send: @escaping (ContinuationPreparationSession) -> Void,
         finish: @escaping (UUID) -> Void,
         retainInput: @escaping (ComposerModel) -> Void = { _ in },
         queuedMessageID: String? = nil,
         receivedQueue: @escaping (SnapshotResult) -> Void = { _ in }) {
        self.source = source; self.input = input; self.target = target
        self.sourceRuntimeID = sourceRuntimeID; self.sourceContextRuntimeID = sourceContextRuntimeID
        self.conversationID = conversationID; self.operations = operations; self.send = send; self.finish = finish
        self.retainInput = retainInput; self.receivedQueue = receivedQueue
        initialDraft = input.savedDraft; initialRevision = input.revision
        existingQueuedMessageID = queuedMessageID; submissionAccepted = queuedMessageID != nil
    }
    var next: NextTurn? {
        guard var basis = summaryBasis else { return nil }
        basis.kept = summaryChoice
        return resolveNextTurn(input: basis)
    }
    var targetName: String { source?.device.runtimeInstances.first { $0.id == target.runtimeId }?.name ?? "目标运行时" }
    var modelTitle: String {
        if summaryChoice.model == nil, defaultModelIssue != nil { return "请选择摘要模型" }
        if summaryChoice.model == nil { return next?.modelName.map { "\($0)（默认速度）" } ?? "源模型（默认速度）" }
        if next?.chosenModelUnavailable == true { return "所选摘要模型不可用" }
        return next?.modelName ?? "所选摘要模型不可用"
    }
    var summaryEffortTitle: String {
        if let effort = summaryChoice.effort, next?.efforts.contains(effort) != true { return "\(effort)（不可用）" }
        return next?.effort ?? next?.currentEffort ?? "默认"
    }
    var modelIssue: String? {
        if summaryChoice.model == nil, let defaultModelIssue { return defaultModelIssue }
        if next?.chosenModelUnavailable == true { return "所选摘要模型不可用，请重新选择。" }
        if let effort = summaryChoice.effort, next?.efforts.contains(effort) != true { return "所选推理强度不可用，请重新选择。" }
        return nil
    }
    var isWorking: Bool { [.checking, .sending, .queueing].contains(phase) }
    var canChoose: Bool { !closed && !isWorking && summaryBasis != nil }
    var canProceed: Bool {
        let valid = existingQueuedMessageID != nil || !text.trimmingCharacters(in: .whitespacesAndNewlines).isEmpty || !attachments.isEmpty
        return !closed && !isWorking && valid && (summaryBasis == nil || modelIssue == nil)
    }
    var primaryTitle: String {
        switch phase {
        case .checking: "正在确认…"
        case .queueing: "正在加入队列…"
        case .sending: "正在发送…"
        case .failed where summaryBasis == nil: "重新检查"
        default: "压缩并继续"
        }
    }
    var feeNotice: String { "压缩上下文会新增一次来源模型调用。" }
    var status: String? {
        switch phase {
        case .checking: "正在确认接续方式…"
        case .queueing: "正在接收排队消息，受理后将在会话中继续压缩。"
        case .sending: "正在发送消息…"
        default: nil
        }
    }
    func show() { if !closed { isPresented = true } }
    func hide() { isPresented = false }
    func choose(_ change: TurnChoiceChange) {
        guard canChoose, let next, let update = changeNextTurn(next: next, runtimeChoices: nil, change: change) else { return }
        summaryChoiceEdited = true; summaryChoice = update.choice
    }
    private var sourceStillMatches: Bool {
        guard let source, !source.isShuttingDown else { return false }
        return source.loadedConversationID == conversationID && source.loadedConversationRuntimeID == sourceRuntimeID
            && source.projectionRuntimeID == sourceContextRuntimeID
    }
    private func perform<T: Sendable>(_ operation: @escaping @Sendable () throws -> T,
                                     then: @escaping @MainActor (Result<T, Error>) -> Void) {
        DispatchQueue.global(qos: .userInitiated).async {
            let result = Result { try operation() }
            DispatchQueue.main.async { then(result) }
        }
    }
    func start() {
        guard !closed, phase == .checking else { return }
        let requestID = requirementsRequestID
        perform({ [operations] in try operations.requirements(requestID) }) { [self] result in
            guard !closed else { return }
            guard sourceStillMatches else { close(); return }
            switch result {
            case .success(.checking(let id)):
                requirementsRequestID = id
                DispatchQueue.main.asyncAfter(deadline: .now() + 0.25) { [self] in start() }
            case .success(.ready):
                queueAfterRequirements = false; requirementsRequestID = nil
                isPresented = false; phase = .sending; send(self)
            case .success(.summary(let basis, let issue)):
                requirementsRequestID = nil; summaryBasis = basis; defaultModelIssue = issue
                // Native IDs are scoped to a runtime/account. Only a global
                // managed key explicitly offered by the source can be reused.
                if !summaryChoiceEdited, let selected = target.model, case .managed = selected,
                   let candidate = basis.catalog?.first(where: { $0.selection == selected }),
                   target.effort.map({ candidate.reasoningEfforts.contains($0) }) ?? true {
                    summaryChoice.model = selected; summaryChoice.effort = target.effort
                }
                let confirmed = queueAfterRequirements; queueAfterRequirements = false
                failure = nil; phase = .ready
                if !hasConfirmed { isPresented = true }
                if confirmed, modelIssue == nil { acceptQueue() }
            case .success(.failed(let detail)): fail(TransportError.rejected(detail))
            case .failure(let error): fail(error)
            }
        }
    }
    func proceed() {
        guard canProceed else { return }
        guard sourceStillMatches else { close(); return }
        failure = nil; isPresented = false; requirementsRequestID = nil
        queueAfterRequirements = summaryBasis != nil
        if queueAfterRequirements { hasConfirmed = true }
        phase = .checking; start()
    }
    private func acceptQueue() {
        guard modelIssue == nil else { return }
        phase = .queueing
        let queueID = existingQueuedMessageID, model = summaryChoice.model, effort = summaryChoice.effort
        perform({ [operations] in try operations.queue(queueID, model, effort) }) { [self] result in
            switch result {
            case .success(let accepted):
                consumeAcceptedInput()
                if let result = accepted.result { receivedQueue(result) }
                source?.noteVisible()
                // Core owns summary progress, cancellation and dispatch through
                // typed FollowUp. No frontend polling or SendPrepared path.
                close()
            case .failure(let error): fail(error)
            }
        }
    }
    private func preserveCapturedInput() {
        guard !submissionAccepted, recoveryInput == nil,
              input.revision != initialRevision || input.savedDraft != initialDraft else { return }
        let model = ComposerModel(); model.restoreSavedDraft(initialDraft)
        if let source { model.attach(source) }
        recoveryInput = RecoveryInput(model: model, revision: model.revision, draft: model.savedDraft)
        retainInput(model)
    }
    private func consumeAcceptedInput() {
        guard !submissionAccepted else { return }
        if input.revision == initialRevision, input.savedDraft == initialDraft {
            input.clear(submitted: initialDraft.text, attachments: initialDraft.attachments)
        }
        if let recoveryInput, recoveryInput.model.revision == recoveryInput.revision, recoveryInput.model.savedDraft == recoveryInput.draft {
            recoveryInput.model.clear(submitted: recoveryInput.draft.text, attachments: recoveryInput.draft.attachments)
        }
        submissionAccepted = true
    }
    func beganQueueing() { phase = .queueing }
    func accepted() { consumeAcceptedInput(); close() }
    func sendFailed(_ error: Error) { fail(error) }
    private func fail(_ error: Error) {
        queueAfterRequirements = false; preserveCapturedInput()
        guard !closed else { return }
        failure = AppProblem.failure(error).detail; phase = .failed
        if !hasConfirmed { isPresented = true }
    }
    func cancel() {
        guard !closed, phase != .queueing else { return }
        if phase == .sending { source?.cancel(); return }
        close()
    }
    func close() {
        guard !closed else { return }
        preserveCapturedInput(); closed = true; queueAfterRequirements = false; isPresented = false
        phase = .closed; finish(id)
    }
}

/// A scoped native preparation sheet. Its model choice shares the ordinary
/// composer's menu content; it does not manufacture another text composer.
struct ContinuationPreparationView: View {
    @ObservedObject var session: ContinuationPreparationSession
    @ObservedObject private var appearance = AppearancePreferences.shared
    @State private var settingsExpanded: Bool
    init(session: ContinuationPreparationSession, settingsExpanded: Bool = false) {
        self.session = session
        _settingsExpanded = State(initialValue: settingsExpanded || session.modelIssue != nil)
    }
    private var modelMenuTitle: String {
        let title = session.modelTitle
        let font = NSFont.systemFont(ofSize: 13 * appearance.values.uiScale)
        func width(_ value: String) -> CGFloat { (value as NSString).size(withAttributes: [.font: font]).width }
        guard width(title) > 250 else { return title }
        var prefix = title
        while !prefix.isEmpty, width(prefix + "…") > 250 { prefix.removeLast() }
        return prefix + "…"
    }
    var body: some View {
        VStack(alignment: .leading, spacing: 18) {
            VStack(alignment: .leading, spacing: 8) {
                Text("切换到 \(session.targetName) 前需要压缩上下文")
                    .uiFont(.headline).fixedSize(horizontal: false, vertical: true)
                Text("消息先在队列中等待，压缩完成后自动发送。")
                    .uiFont(.callout).foregroundStyle(.secondary)
                    .fixedSize(horizontal: false, vertical: true)
            }
            if let next = session.next {
                DisclosureGroup("摘要设置", isExpanded: $settingsExpanded) {
                    Grid(alignment: .leading, horizontalSpacing: 12, verticalSpacing: 10) {
                        GridRow {
                            Text("模型").foregroundStyle(.secondary)
                            Menu {
                                ComposerModelChoiceContent(next: next, catalog: .ready(next.models),
                                    selection: Binding(get: { session.summaryChoice.model }, set: { session.choose(.model(selection: $0)) }),
                                    enabled: session.canChoose, defaultTitle: "源会话模型（默认速度）",
                                    defaultAvailable: session.defaultModelIssue == nil,
                                    defaultHelp: "使用源会话模型的默认速度",
                                    unavailableMessage: "所选摘要模型不可用，请重新选择。")
                            } label: { Text(modelMenuTitle).frame(maxWidth: .infinity, alignment: .leading) }
                            .menuStyle(.button).buttonStyle(.bordered).disabled(!session.canChoose)
                            .help(session.modelTitle).accessibilityIdentifier("continuation-summary-model")
                        }
                        if !next.efforts.isEmpty {
                            GridRow {
                                Text("推理强度").foregroundStyle(.secondary)
                                Menu {
                                    ComposerEffortChoiceContent(next: next,
                                        selection: Binding(get: { session.summaryChoice.effort }, set: { session.choose(.effort(level: $0)) }),
                                        enabled: session.canChoose, defaultTitle: "默认")
                                } label: { Text(session.summaryEffortTitle).frame(maxWidth: .infinity, alignment: .leading) }
                                .menuStyle(.button).buttonStyle(.bordered).disabled(!session.canChoose)
                                .accessibilityIdentifier("continuation-summary-effort")
                            }
                        }
                    }.padding(.top, 8)
                    Text("摘要使用来源会话的账户，与新消息的模型设置独立。")
                        .uiFont(.caption).foregroundStyle(.secondary)
                        .fixedSize(horizontal: false, vertical: true)
                }.uiFont(.callout)
            }
            if let issue = session.modelIssue {
                Text(issue).uiFont(.callout).foregroundStyle(.secondary)
                    .fixedSize(horizontal: false, vertical: true)
            }
            if let failure = session.failure {
                Text(failure).uiFont(.callout).foregroundStyle(.red).lineLimit(4).help(failure)
            } else if let status = session.status {
                Text(status).uiFont(.callout).foregroundStyle(.secondary)
            }
            if session.summaryBasis != nil, !session.isWorking {
                Text(session.feeNotice).uiFont(.callout).foregroundStyle(.secondary)
                    .fixedSize(horizontal: false, vertical: true)
            }
            HStack(spacing: 8) {
                Spacer(minLength: 12)
                Button("取消", action: session.cancel)
                    .disabled(session.phase == .queueing).keyboardShortcut(.cancelAction)
                Button(session.primaryTitle, action: session.proceed)
                    .buttonStyle(.borderedProminent).disabled(!session.canProceed)
                    .keyboardShortcut(.defaultAction)
            }
        }
        .padding(24).frame(width: 440)
        .interactiveDismissDisabled(session.isWorking)
        .onChange(of: session.modelIssue) { _, issue in if issue != nil { settingsExpanded = true } }
    }
}

/// The visible composer owns presentation. A result for a background tab stays
/// with that tab and cannot move focus or show a sheet over another conversation.
struct ContinuationPreparationPresentation: ViewModifier {
    @ObservedObject var tab: ConversationTab
    @ObservedObject var window: ConversationWindowModel
    @Environment(\.controlActiveState) private var activeState
    let enabled: Bool
    func body(content: Content) -> some View {
        content.background {
            if enabled, let session = tab.continuationPreparation {
                ContinuationPreparationSheetAnchor(session: session,
                    canPresent: window.canTargetConversation && window.commandTab === tab && activeState == .key)
            }
        }
    }
}
private struct ContinuationPreparationSheetAnchor: View {
    @ObservedObject var session: ContinuationPreparationSession
    let canPresent: Bool
    @State private var wasPresented = false
    var body: some View {
        Color.clear.frame(width: 0, height: 0)
            .sheet(isPresented: Binding(get: { session.isPresented && (canPresent || wasPresented) },
                                        set: { if !$0 { session.hide() } }), onDismiss: { wasPresented = false }) {
                ContinuationPreparationView(session: session)
                    .onAppear { wasPresented = true }
            }
    }
}
struct ContinuationPreparationControl: View {
    @ObservedObject var session: ContinuationPreparationSession
    var show: (() -> Void)? = nil
    private var title: String {
        switch session.phase {
        case .checking: "取消准备"
        case .sending: "取消发送"
        case .queueing: "正在加入队列…"
        case .failed: "未能继续发送…"
        default: session.modelIssue == nil ? "需要压缩上下文…" : "需要更新压缩设置…"
        }
    }
    var body: some View {
        Button {
            if [.checking, .sending].contains(session.phase) { session.cancel() }
            else if let show { show() }
            else { session.show() }
        } label: {
            HStack(spacing: 6) {
                if session.isWorking { ProgressView().controlSize(.small) }
                Text(title)
            }
        }
        .disabled(session.phase == .queueing)
        .help(session.status ?? "查看接续准备")
        .accessibilityIdentifier("composer-continuation-preparation")
    }
}


extension ConversationTab {
    /// Preflight is read-only and happens before a placeholder or draft
    /// consumption. Ready preserves the existing send/queue path; a summary
    /// requires the separate, explicit model-call confirmation above.
    func prepareContinuationIfNeeded(using input: ComposerModel, target: NextTurn,
                                     afterProviderRejection: Bool = false) -> Bool {
        guard !store.isPreview, !store.isSideChat, !store.isUnsentConversation,
              let sourceRuntime = store.loadedConversationRuntimeID,
              let sourceContext = store.projectionRuntimeID,
              sourceContext != target.runtimeId || afterProviderRejection,
              let conversationID = store.loadedConversationID,
              let transport = store.device.transport else { return false }
        guard continuationPreparation == nil else { return true }
        let drafts = ConversationWindowModel.owning(tab: id)?.drafts(for: store.device)
        let session = ContinuationPreparationSession(source: store, input: input, target: target,
            sourceRuntimeID: sourceRuntime, sourceContextRuntimeID: sourceContext, conversationID: conversationID,
            operations: .live(transport: transport, sourceRuntimeID: sourceRuntime, conversationID: conversationID, target: target,
                              turn: ComposedTurn(text: input.submission.trimmingCharacters(in: .whitespacesAndNewlines), effort: target.effort,
                                                 permissionMode: target.permissionMode?.id, attachments: input.attachments.map(\.path))),
            send: { [weak self] session in
                guard let self else { session.close(); return }
                sendCapturedContinuation(session)
            }, finish: { [weak self] id in
                guard self?.continuationPreparation?.id == id else { return }
                self?.continuationPreparation = nil
            }, retainInput: { model in
                // Keep the existing save/synchronization registry alive through
                // acknowledgement, even if its source window has closed.
                drafts?.retain(model, name: "未发送的消息")
            }, receivedQueue: { [weak store] result in store?.acceptContextPreparationQueue(result, conversationID: conversationID, sourceContext: sourceContext) })
        continuationPreparation = session
        session.start()
        return true
    }
    func showQueuedContextPreparation() {
        if let continuationPreparation { continuationPreparation.show(); return }
        guard let context = store.followUp.contextPreparation, context.canRetrySummary,
              let runtime = store.loadedConversationRuntimeID, let source = store.projectionRuntimeID,
              let conversation = store.loadedConversationID, let transport = store.device.transport else { return }
        let targetInstance = store.device.runtimeInstances.first { $0.id == context.targetRuntimeId }
        let target = resolveNextTurn(input: NextTurnInput(runtimeId: context.targetRuntimeId,
            runtimeType: targetInstance.flatMap { instance in store.device.runtimeTypes.first { $0.id == instance.typeId } },
            providers: [], catalog: [], newConversation: .chosen(choice: TurnChoice())))
        let session = ContinuationPreparationSession(source: store, input: composer, target: target,
            sourceRuntimeID: runtime, sourceContextRuntimeID: source, conversationID: conversation,
            operations: .live(transport: transport, sourceRuntimeID: runtime, conversationID: conversation, target: target, turn: nil),
            send: { [weak store] session in store?.resumeQueue(); session.accepted() },
            finish: { [weak self] id in if self?.continuationPreparation?.id == id { self?.continuationPreparation = nil } },
            queuedMessageID: context.queuedMessageId,
            receivedQueue: { [weak store] result in store?.acceptContextPreparationQueue(result, conversationID: conversation, sourceContext: source) })
        continuationPreparation = session; session.start()
    }
    private func sendCapturedContinuation(_ session: ContinuationPreparationSession) {
        guard continuationPreparation === session,
              store.loadedConversationID == session.conversationID,
              store.loadedConversationRuntimeID == session.sourceRuntimeID,
              store.projectionRuntimeID == session.sourceContextRuntimeID else { session.close(); return }
        let submitted = session.text, attachments = session.attachments
        let text = submitted.trimmingCharacters(in: .whitespacesAndNewlines)
        if store.canQueue {
            session.beganQueueing()
            guard store.queue(text: text, attachments: attachments.map(\.path), next: session.target,
                              onAccepted: { session.accepted() },
                              onFailure: { session.sendFailed(TransportError.rejected("排队未成功，编辑内容仍保留。")) }) else {
                session.sendFailed(TransportError.rejected("当前无法排队，原输入仍保留。")); return
            }
            return
        }
        guard store.canSend else {
            session.sendFailed(TransportError.rejected("当前会话尚不能发送。请等待当前操作结束后重试接续，原输入仍保留。")); return
        }
        let sent = store.send(text: text, attachments: attachments.map(\.path), next: session.target, input: session.input,
                              onAccepted: { _ in session.accepted() },
                              onDetailedFailure: { session.sendFailed($0) })
        guard sent else {
            session.sendFailed(TransportError.rejected("当前无法发送，原输入仍保留。")); return
        }
        scrollRequest &+= 1
    }
}
