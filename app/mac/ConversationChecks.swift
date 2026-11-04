import AppKit
import VeluneBindings

extension ConversationWindowModel {
    /// Runs the production group action with synthetic configuration. No
    /// global input, activation, AX actions or native turn is dispatched.
    /// Creation opens its own tab: with a conversation shown the tab count
    /// grows; an unsent tab is retargeted in place instead.
    func checkGroupCreation(attempt: Int = 0) {
        guard attempt < 80 else { return groupCreationCheck("FAIL configuration not ready") }
        let runtimes = application.enabledRuntimeInstances
        guard runtimes.count >= 2, !focused.store.isLoading,
              let project = application.conversationProjects.first(where: { $0.primaryFolder != nil }) else {
            if attempt % 20 == 0 { FileHandle.standardError.write(Data("CHECK group-creation: waiting runtimes=\(runtimes.count) loading=\(focused.store.isLoading) projects=\(application.conversationProjects.map { $0.path })\n".utf8)) }
            DispatchQueue.main.asyncAfter(deadline: .now() + 0.1) { [self] in checkGroupCreation(attempt: attempt + 1) }
            return
        }
        tools.move(.composer, to: .docked(.bottom))
        if !isSplit { split(.horizontal) }
        var verifiedNewTab = false
        if let conversation = conversationTabs.first(where: { !$0.store.isUnsentConversation }) {
            selectTab(conversation.id)
            var baseTabs = tabCount
            createConversation(in: .runtime(runtimeId: runtimes[0].id))
            guard tabCount == baseTabs + 1, focused !== conversation, focused.store.isUnsentConversation,
                  !conversation.store.isUnsentConversation, conversation.store.loadedConversationID != nil,
                  composerContext().nextTurnRuntimeID == runtimes[0].id else {
                return groupCreationCheck("FAIL group creation did not open a new tab over the conversation")
            }
            // An unavailable runtime changes nothing even when a new tab would open.
            selectTab(conversation.id)
            baseTabs = tabCount
            createConversation(in: .runtime(runtimeId: "missing-synthetic-runtime"))
            guard tabCount == baseTabs, focused === conversation else { return groupCreationCheck("FAIL unavailable runtime opened a tab") }
            verifiedNewTab = true
            selectTab(conversationTabs.first { $0.store.isUnsentConversation }!.id)
        }
        // The focused tab holds an unsent conversation: retarget it in place.
        let draft = focused
        guard draft.store.isUnsentConversation else { return groupCreationCheck("FAIL no unsent tab to retarget") }
        let input = composer(for: draft)
        input.restore("GROUP SHARED DRAFT", attachments: [])
        composerContext(for: draft).selectNextTurnRuntime(runtimes[0].id)
        let baseTabs = tabCount
        createConversation(in: .runtime(runtimeId: runtimes[1].id))
        guard focused === draft, tabCount == baseTabs,
              composerContext().nextTurnRuntimeID == runtimes[1].id,
              composer(for: focused) === input, input.submission == "GROUP SHARED DRAFT" else {
            return groupCreationCheck("FAIL runtime target or shared draft")
        }
        createConversation(in: .project(path: project.path))
        guard focused === draft, tabCount == baseTabs,
              focused.store.loadedConversationWorkingDirectory == project.primaryFolder,
              composerContext().nextTurnRuntimeID == (project.nativeRuntimeId ?? runtimes[1].id),
              composer(for: focused) === input, input.submission == "GROUP SHARED DRAFT" else {
            return groupCreationCheck("FAIL project primary folder or shared draft")
        }
        let before = focused.store.loadedConversationID
        createConversation(in: .runtime(runtimeId: "missing-synthetic-runtime"))
        guard focused.store.loadedConversationID == before, tabCount == baseTabs else { return groupCreationCheck("FAIL unavailable runtime fell back") }
        groupCreationCheck("PASS \(verifiedNewTab ? "new tab over conversations, " : "")in-place retarget of unsent tab, project primary folder, shared input and unavailable runtime; no turn sent")
    }

    private func groupCreationCheck(_ result: String) {
        FileHandle.standardError.write(Data("CHECK group-creation: \(result)\n".utf8))
        DispatchQueue.main.async { NSApp.terminate(nil) }
    }
}
