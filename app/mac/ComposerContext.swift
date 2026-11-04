import Foundation
import VeluneBindings

/// A window pairs a draft owner with a target. Visibility never changes ownership.
/// Values resolved here are captured again at the start of a submission.
@MainActor struct ComposerContext {
    let tab: ConversationTab
    let model: ComposerModel
    var store: ConversationStore { tab.store }
    let nextTurn: NextTurn?
    init(tab: ConversationTab, model: ComposerModel) {
        self.tab = tab; self.model = model
        if let intent = model.intent { nextTurn = tab.store.resolve(kept: intent.kept) }
        else { nextTurn = tab.store.nextTurn }
    }
    var nextTurnRuntimeID: String? { nextTurn?.runtimeId }
    var nextTurnModel: ModelSelection? { nextTurn?.model }
    var nextTurnHasCurrent: Bool { nextTurn?.hasCurrent ?? false }
    var chosenModel: ModelSelection? { nextTurn?.choice.model }
    var chosenModelUnavailable: Bool { nextTurn?.chosenModelUnavailable ?? false }
    var inputIssue: String? {
        if let source = model.runtimeOwnerID, source != store.device.composerInputs.ownerID { return "所选运行时位于另一设备，输入已保留" }
        if let runtime = model.intent?.kept.runtimeID, !store.device.enabledRuntimeInstances.contains(where: { $0.id == runtime }) { return "所选运行时在此设备不可用" }
        if !model.followsConversation, let issue = store.device.composerInputs.issue(for: model.id) { return issue }
        if model.attachments.contains(where: { !FileManager.default.isReadableFile(atPath: $0.path) }) { return "附件不可用，点击附件重试或移除" }
        return nil
    }
    var selectedModelName: String? {
        guard let nextTurn else { return nil }
        if nextTurn.chosenModelUnavailable { return "所选模型不可用" }
        return nextTurn.model == nil ? "运行时默认" : nextTurn.modelName
    }
    var nextTurnEffort: String? { nextTurn?.effort }
    var currentEffort: String? { nextTurn?.currentEffort }
    var reasoningEffortOptions: [String] { nextTurn?.efforts ?? [] }
    var permissionModeOptions: [PermissionMode] { nextTurn?.permissionModes ?? [] }
    var effectivePermissionMode: PermissionMode? { nextTurn?.permissionMode }
    var currentPermissionMode: PermissionMode? { nextTurn?.currentPermissionMode }
    var modelChoices: [RuntimeModel] { nextTurn?.models ?? [] }
    var modelCatalog: RuntimeModelCatalog? { nextTurnRuntimeID.flatMap { store.modelCatalogs[$0] } }
    var canSwitchModel: Bool { nextTurn != nil && !store.isShuttingDown }
    var nextTurnSettled: Bool { nextTurn != nil && modelCatalog != .loading }
    var canCompact: Bool { inputIssue == nil && !chosenModelUnavailable && store.canCompact(next: nextTurn) }
    func chooseAttachments() { model.attach(store); model.chooseAttachments() }
    var canResume: Bool { inputIssue == nil && !chosenModelUnavailable && store.canResume(next: nextTurn) }
    var canSteer: Bool { submission().canSubmitDraft && submission().action == .queue && store.canSteer }
    var sideChatTarget: SideChatTarget? { store.sideChatTarget(next: nextTurn) }

    func selectNextTurnRuntime(_ id: String) {
        guard canSwitchModel, store.device.enabledRuntimeInstances.contains(where: { $0.id == id }) else { return }
        guard !store.isSideChat || store.device.hasSideChats(id) else { return }
        if model.intent != nil { model.selectRuntime(id, owner: store.device.composerInputs.ownerID); store.loadModelCatalog(runtimeID: id) }
        else { store.selectNextTurnRuntime(id) }
    }
    func selectModel(_ selection: ModelSelection?) { change(.model(selection: selection)) }
    func selectEffort(_ effort: String?) { change(.effort(level: effort)) }
    func selectPermissionMode(_ mode: String?) { change(.permissionMode(mode: mode)) }
    private func change(_ change: TurnChoiceChange) {
        guard canSwitchModel, let nextTurn else { return }
        guard model.runtimeOwnerID == nil || model.runtimeOwnerID == store.device.composerInputs.ownerID else { return }
        guard let intent = model.intent else { store.changeNextTurn(change); return }
        guard let update = VeluneBindings.changeNextTurn(next: nextTurn, runtimeChoices: store.device.runtimeComposerChoices[nextTurn.runtimeId], change: change) else { return }
        if let choices = update.runtimeChoices { store.device.setRuntimeComposerChoices(choices) }
        intent.kept.choices[nextTurn.runtimeId] = update.choice
    }
    func prepare() { if let runtime = nextTurnRuntimeID { store.loadModelCatalog(runtimeID: runtime) } }
    func reloadModelCatalog() { if let runtime = nextTurnRuntimeID { store.reloadModelCatalog(runtimeID: runtime) } }
    var composerQuotaRequest: ComposerQuotaRequest { store.quotaRequest(next: nextTurn) }
    var composerAccountQuotas: [ComposerQuotaEntry] { store.quotaEntries(for: composerQuotaRequest) }
    var composerQuotaError: String? { store.quotaError(for: composerQuotaRequest) }
    func observeComposerQuota(_ request: ComposerQuotaRequest) async { await store.observeComposerQuota(request) }
    var workingDirectoryProjects: [WorkingDirectoryProject] {
        nextTurnRuntimeID.flatMap { store.device.workingDirectoryProjects[$0] } ?? []
    }
    var canChangeWorkingDirectory: Bool {
        store.isUnsentConversation && !store.isLoading && !store.isShuttingDown && nextTurnRuntimeID != nil
    }
    func selectWorkingDirectory(_ path: String?) {
        guard canChangeWorkingDirectory else { return }
        store.setUnsentWorkingDirectory(path)
    }
    func submission(ignoringText: Bool = false) -> ComposerSubmission { tab.submission(using: model, ignoringText: ignoringText, next: nextTurn) }
    func send() { tab.send(using: model, next: nextTurn) }
    func steer() { tab.steer(using: model, next: nextTurn) }
    func resume() { if canResume { store.continueTurn(next: nextTurn) } }
    func compactContext() { store.compactContext(next: nextTurn) }
}

extension ConversationWindowModel {
    func composerContext(for tab: ConversationTab? = nil) -> ComposerContext {
        let target = tab ?? commandTab
        return ComposerContext(tab: target, model: composer(for: target))
    }
    func send() { guard canTargetConversation else { return }; composerContext().send() }
    func resume() { guard canTargetConversation else { return }; composerContext().resume() }
}
