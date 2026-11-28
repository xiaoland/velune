#if VELUNE_DEV_CHECKS
import AppKit
import VeluneBindings

@MainActor func groupCreationCheck() -> Never {
    NSApplication.shared.setActivationPolicy(.prohibited)
    let device = DeviceStore(preview: true)
    let window = ConversationWindowModel(application: device, kind: .workspace)
    window.split(.horizontal)
    let source = window.focused
    source.store.selectConversation(id: "sample-0")
    let project = ConversationProject(path: "project:synthetic", primaryFolder: "/tmp/group-workspace", nativeRuntimeId: nil,
        name: "Synthetic", folders: [], worktrees: [], latestActivityUnixMs: nil)
    device.applyList(ConfigurationSnapshot(conversations: [], conversationGroups: [], conversationProjects: [project],
        workingDirectoryProjects: [:], showsArchived: false, historyFailures: [], gateways: [],
        runtimeInstances: device.runtimeInstances, runtimeTypes: device.runtimeTypes, modelTemplates: [],
        conversationBrowserLimits: device.conversationBrowserLimits, outlinePresentation: .popover, linkedConversationOpening: .currentTab,
        usageDisplay: .used, contextUsageDisplay: .used, conversationBrowserPreferences: ConversationBrowser().preferences,
        statusLine: [], providerImportTypes: [], protocols: [], selectedRuntimeInstanceId: "sample-instance"))
    window.tools.move(.composer, to: .docked(.bottom))
    let originalCount = window.tabCount
    window.createConversation(in: .runtime(runtimeId: "missing-synthetic-runtime"))
    precondition(window.focused === source && window.tabCount == originalCount, "不可用运行时不能覆盖当前会话或打开新标签")
    window.createConversation(in: .runtime(runtimeId: "sample-instance"))
    let draft = window.focused
    precondition(draft !== source && source.store.loadedConversationID == "sample-0" && draft.store.isUnsentConversation
                 && window.tabCount == originalCount + 1 && window.composerContext().nextTurnRuntimeID == "sample-instance")
    let input = window.composer(for: draft)
    input.restore("GROUP SHARED DRAFT", attachments: [])
    window.createConversation(in: .runtime(runtimeId: "sample-review"))
    precondition(window.focused === draft && window.composerContext().nextTurnRuntimeID == "sample-review")
    window.createConversation(in: .project(path: project.path))
    precondition(draft.store.loadedConversationWorkingDirectory == "/tmp/group-workspace"
                 && window.composerContext().nextTurnRuntimeID == "sample-review", "项目应使用主目录并保留所选运行时")
    let id = draft.store.loadedConversationID
    window.createConversation(in: .runtime(runtimeId: "missing-synthetic-runtime"))
    precondition(window.focused === draft && window.tabCount == originalCount + 1 && draft.store.loadedConversationID == id
                 && window.composer(for: draft) === input && input.submission == "GROUP SHARED DRAFT")
    print("PASS group creation: group actions open or retarget the selected tab, preserve its input and use the project’s primary folder")
    exit(0)
}
#endif
