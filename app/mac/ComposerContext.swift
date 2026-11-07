import Foundation
import VeluneBindings

/// A window pairs input content with a Store's prepared target projection.
/// Reads are pure; explicit actions forward to the owning Store or editor.
@MainActor struct ComposerContext {
    let tab: ConversationTab
    let model: ComposerModel
    var store: ConversationStore { tab.store }
    let nextTurn: NextTurn?
    private var targetID: String? { store.selectedConversationID ?? store.loadedConversationID }
    private var targetChoices: ComposerTargetChoices? { store.composerTargetChoices }
    private var targetReady: Bool { store.loadedConversationID != nil && (store.selectedConversationID == nil || store.selectedConversationID == store.loadedConversationID) && !store.isLoading }
    init(tab: ConversationTab, model: ComposerModel) {
        self.tab = tab; self.model = model
        nextTurn = tab.store.nextTurn
    }
    var nextTurnRuntimeID: String? { nextTurn?.runtimeId }
    var nextTurnModel: ModelSelection? { nextTurn?.model }
    var nextTurnHasCurrent: Bool { nextTurn?.hasCurrent ?? false }
    var chosenModel: ModelSelection? { nextTurn?.choice.model }
    var chosenModelUnavailable: Bool { nextTurn?.chosenModelUnavailable ?? false }
    var inputIssue: String? {
        if !targetReady { return "正在打开目标会话" }
        if let source = targetChoices?.runtimeOwnerId, source != store.device.composerInputs.ownerID { return "所选运行时位于另一设备，输入已保留" }
        if let runtime = targetChoices?.runtimeId, !store.device.enabledRuntimeInstances.contains(where: { $0.id == runtime }) { return "所选运行时在此设备不可用" }
        if let project = selectedWorkingProject, project.runtimeID != nextTurnRuntimeID { return "所选项目属于另一运行时，请重新选择项目或工作目录" }
        if let project = selectedWorkingProject, !workingDirectoryProjects.contains(where: { $0.project == project.key && $0.canSelectProject }) { return "所选项目已不可用，请刷新或重新选择工作区" }
        if let issue = store.composerTargetIssue { return issue }
        if !store.isSideChat, let key = targetID, let issue = store.device.composerInputs.targetIssue(for: key) { return issue }
        if !store.isSideChat, let issue = store.device.composerInputs.issue(for: model.id) { return issue }
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
        if store.isSideChat { store.selectSideChatRuntime(id) }
        else { store.selectComposerRuntime(id) }
    }
    func selectModel(_ selection: ModelSelection?) { store.changeComposerChoice(.model(selection: selection)) }
    func selectEffort(_ effort: String?) { store.changeComposerChoice(.effort(level: effort)) }
    func selectPermissionMode(_ mode: String?) { store.changeComposerChoice(.permissionMode(mode: mode)) }
    func takeLastQueued() -> Bool {
        model.attach(store)
        return model.takeLastQueued()
    }
    func reloadModelCatalog() { if let runtime = nextTurnRuntimeID { store.reloadModelCatalog(runtimeID: runtime) } }
    var composerQuotaRequest: ComposerQuotaRequest { store.quotaRequest(next: nextTurn) }
    var composerAccountQuotas: [ComposerQuotaEntry] { store.quotaEntries(for: composerQuotaRequest) }
    var composerQuotaError: String? { store.quotaError(for: composerQuotaRequest) }
    func observeComposerQuota(_ request: ComposerQuotaRequest) async { await store.observeComposerQuota(request) }
    var workingDirectoryProjects: [WorkingDirectoryProject] {
        nextTurnRuntimeID.flatMap { store.device.workingDirectoryProjects[$0] } ?? []
    }
    var selectedWorkingProject: NewConversationProject? { store.loadedConversationProject }
    var canChangeWorkingDirectory: Bool {
        store.isUnsentConversation && !store.isLoading && !store.isShuttingDown && nextTurnRuntimeID != nil
    }
    func selectWorkingDirectory(_ path: String?) {
        guard canChangeWorkingDirectory else { return }
        store.setUnsentWorkingDirectory(path)
    }
    func selectWorkingProject(_ project: WorkingDirectoryProject) {
        guard canChangeWorkingDirectory, project.canSelectProject, let runtimeID = nextTurnRuntimeID else { return }
        store.setUnsentProject(NewConversationProject(key: project.project, name: project.name, runtimeID: runtimeID, directories: project.directories))
    }
    func submission(ignoringText: Bool = false) -> ComposerSubmission { tab.submission(using: model, ignoringText: ignoringText, next: nextTurn) }
    func send() {
        store.captureComposerTarget(model.id)
        tab.send(using: model, next: nextTurn)
    }
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
