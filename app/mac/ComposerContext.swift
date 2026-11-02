import Foundation
import VeluneBindings

/// A directory selection belongs to the conversation and runtime it opened for.
@MainActor struct ComposerContext {
    let tab: ConversationTab
    let model: ComposerModel
    let nextTurnRuntimeID: String?
    var store: ConversationStore { tab.store }
    init(tab: ConversationTab, model: ComposerModel) {
        self.tab = tab; self.model = model
        nextTurnRuntimeID = tab.store.nextTurnRuntimeID
    }
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
}

extension ConversationWindowModel {
    func conversationTab(for store: ConversationStore?) -> ConversationTab {
        let tabs = allTabs + [sheetTab, composerSheetTab].compactMap { $0 }
        return store.flatMap { store in tabs.first { $0.store === store } } ?? focused
    }
    func composerContext(for tab: ConversationTab? = nil) -> ComposerContext {
        let target = tab ?? focused
        return ComposerContext(tab: target, model: target.composer)
    }
}
