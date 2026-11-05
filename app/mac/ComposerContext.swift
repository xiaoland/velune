import Foundation
import VeluneBindings

/// A window pairs a draft owner with a target. Visibility never changes ownership.
/// Values resolved here are captured again at the start of a submission.
@MainActor struct ComposerContext {
    let tab: ConversationTab
    let model: ComposerModel
    var store: ConversationStore { tab.store }
    let nextTurn: NextTurn?
    private var targetID: String? { store.selectedConversationID ?? store.loadedConversationID }
    private var targetChoices: ComposerTargetChoices? { store.isSideChat ? nil : targetID.flatMap { store.device.composerInputs.targetChoices(for: $0) } }
    private var targetReady: Bool { store.loadedConversationID != nil && (store.selectedConversationID == nil || store.selectedConversationID == store.loadedConversationID) && !store.isLoading }
    init(tab: ConversationTab, model: ComposerModel) {
        self.tab = tab; self.model = model
        let store = tab.store
        let key = store.selectedConversationID ?? store.loadedConversationID
        let ready = store.loadedConversationID != nil && (store.selectedConversationID == nil || store.selectedConversationID == store.loadedConversationID) && !store.isLoading
        guard ready else { nextTurn = nil; return }
        if store.isSideChat { nextTurn = store.nextTurn; return }
        let client = store.device.composerInputs
        if let key, let choices = client.targetChoices(for: key), choices.runtimeId != nil || !choices.choices.isEmpty {
            nextTurn = store.resolve(kept: ConversationChoices(runtimeID: choices.runtimeId, choices: Dictionary(uniqueKeysWithValues: choices.choices.filter { $0.ownerId == client.ownerID }.map { ($0.runtimeId, $0.choice) })))
        } else { nextTurn = store.nextTurn }
        if let key, let nextTurn, let owner = client.ownerID {
            client.ensureTarget(key, seed: ComposerTargetChoices(runtimeId: nextTurn.runtimeId, runtimeOwnerId: owner, choices: [.init(ownerId: owner, runtimeId: nextTurn.runtimeId, choice: nextTurn.choice)], choiceEdited: false))
        }
    }
    private func capturedChoices() -> ComposerTargetChoices? {
        guard let nextTurn, let owner = store.device.composerInputs.ownerID else { return nil }
        var value = targetChoices ?? ComposerTargetChoices(runtimeId: nextTurn.runtimeId, runtimeOwnerId: owner, choices: [], choiceEdited: false)
        value.runtimeId = nextTurn.runtimeId; value.runtimeOwnerId = value.runtimeOwnerId ?? owner
        value.choices.removeAll { $0.ownerId == owner && $0.runtimeId == nextTurn.runtimeId }
        value.choices.append(.init(ownerId: owner, runtimeId: nextTurn.runtimeId, choice: nextTurn.choice))
        return value
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
        guard canSwitchModel, store.device.enabledRuntimeInstances.contains(where: { $0.id == id }) else { return }
        guard !store.isSideChat || store.device.hasSideChats(id) else { return }
        if store.isSideChat { store.selectSideChatRuntime(id); return }
        guard let key = targetID, let owner = store.device.composerInputs.ownerID else { return }
        var choices = targetChoices ?? ComposerTargetChoices(runtimeId: nil, runtimeOwnerId: nil, choices: [], choiceEdited: false)
        choices.runtimeId = id; choices.runtimeOwnerId = owner; choices.choiceEdited = true
        store.device.composerInputs.saveTarget(key, choices: choices, userEdit: true)
        store.loadModelCatalog(runtimeID: id)
    }
    func selectModel(_ selection: ModelSelection?) { change(.model(selection: selection)) }
    func selectEffort(_ effort: String?) { change(.effort(level: effort)) }
    func selectPermissionMode(_ mode: String?) { change(.permissionMode(mode: mode)) }
    private func change(_ change: TurnChoiceChange) {
        guard canSwitchModel else { return }
        if store.isSideChat { store.changeSideChatChoice(change); return }
        guard let nextTurn, let key = targetID, let owner = store.device.composerInputs.ownerID else { return }
        guard targetChoices?.runtimeOwnerId == nil || targetChoices?.runtimeOwnerId == owner else { return }
        guard let update = VeluneBindings.changeNextTurn(next: nextTurn, runtimeChoices: store.device.runtimeComposerChoices[nextTurn.runtimeId], change: change), var choices = capturedChoices() else { return }
        if let runtimeChoices = update.runtimeChoices { store.device.setRuntimeComposerChoices(runtimeChoices) }
        choices.choices.removeAll { $0.ownerId == owner && $0.runtimeId == nextTurn.runtimeId }
        choices.choices.append(.init(ownerId: owner, runtimeId: nextTurn.runtimeId, choice: update.choice))
        choices.choiceEdited = true
        store.device.composerInputs.saveTarget(key, choices: choices, userEdit: true)
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
        if store.isUnsentConversation, let choices = capturedChoices() { store.device.composerInputs.captureTarget(model.id, choices: choices) }
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
