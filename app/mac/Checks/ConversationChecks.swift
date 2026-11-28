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
    let nativeID = "native:multi-folder"
    var nativeProject = project
    nativeProject.path = nativeID
    nativeProject.primaryFolder = "/tmp/native-primary"
    nativeProject.nativeRuntimeId = "sample-instance"
    let nativeFolders = ["/tmp/native-primary", "/tmp/native-extra"]
    let workspace = WorkingDirectoryProject(project: nativeID, name: "Native workspace", canSelectProject: true, directories: nativeFolders)
    var configuration = ConfigurationSnapshot(conversations: [], conversationGroups: [], conversationProjects: [project, nativeProject],
        workingDirectoryProjects: ["sample-instance": [workspace]], showsArchived: false, historyFailures: [], gateways: [],
        runtimeInstances: device.runtimeInstances, runtimeTypes: device.runtimeTypes, modelTemplates: [],
        conversationBrowserLimits: device.conversationBrowserLimits, outlinePresentation: .popover, linkedConversationOpening: .currentTab,
        usageDisplay: .used, contextUsageDisplay: .used, conversationBrowserPreferences: ConversationBrowser().preferences,
        statusLine: [], providerImportTypes: [], protocols: [], selectedRuntimeInstanceId: "sample-instance")
    device.applyList(configuration)
    window.tools.move(.composer, to: .docked(.bottom))
    let originalCount = window.tabCount
    window.createConversation(in: .runtime(runtimeId: "missing-synthetic-runtime"))
    precondition(window.focused === source && window.tabCount == originalCount, "不可用运行时不能覆盖当前会话或打开新标签")
    window.createConversation(in: .runtime(runtimeId: "sample-instance"))
    let draft = window.focused
    precondition(draft !== source && source.store.loadedConversationID == "sample-0" && draft.store.isUnsentConversation
                 && window.tabCount == originalCount + 1 && window.composerContext().nextTurnRuntimeID == "sample-instance")
    precondition(draft.store.loadedConversationID == draft.store.unsentContextID, "新建草稿与标签必须从第一次打开就使用同一个身份")
    let input = window.composer(for: draft)
    let attachment = MacExecutionContext.admitted.stateDirectory.appendingPathComponent("group-draft.txt")
    try! FileManager.default.createDirectory(at: attachment.deletingLastPathComponent(), withIntermediateDirectories: true)
    let bytes = Data("synthetic group attachment".utf8)
    try! bytes.write(to: attachment)
    defer { try? FileManager.default.removeItem(at: attachment) }
    input.restore("GROUP SHARED DRAFT", attachments: [attachment])
    window.createConversation(in: .runtime(runtimeId: "sample-review"))
    precondition(window.focused === draft && window.composerContext().nextTurnRuntimeID == "sample-review")
    window.createConversation(in: .project(path: project.path))
    precondition(draft.store.loadedConversationWorkingDirectory == "/tmp/group-workspace"
                 && window.composerContext().nextTurnRuntimeID == "sample-review", "项目应使用主目录并保留所选运行时")
    let id = draft.store.loadedConversationID
    window.createConversation(in: .runtime(runtimeId: "missing-synthetic-runtime"))
    precondition(window.focused === draft && window.tabCount == originalCount + 1 && draft.store.loadedConversationID == id
                 && window.composer(for: draft) === input && input.submission == "GROUP SHARED DRAFT"
                 && input.attachments == [attachment] && (try? Data(contentsOf: attachment)) == bytes)
    window.createConversation(in: .project(path: nativeID))
    let expected = NewConversationProject(key: nativeID, name: "Native workspace", runtimeID: "sample-instance", directories: nativeFolders)
    precondition(draft.store.unsentConversation?.creationProject == expected && draft.store.loadedConversationWorkingDirectory == "/tmp/native-primary"
                 && window.composerContext().nextTurnRuntimeID == "sample-instance" && window.composer(for: draft) === input)
    window.selectTab(source.id)
    configuration.workingDirectoryProjects = [:]
    device.applyList(configuration)
    window.createConversation(in: .project(path: nativeID))
    precondition(window.focused === source && window.tabCount == originalCount + 1
                 && device.problems.contains { $0.detail.contains("没有可用的完整工作区") },
                 "缺少原生项目的完整工作区应说明原因并保留原标签，不能降级到主目录")
    print("PASS group creation: group actions open or retarget the selected tab, preserve its input and use the project’s primary folder")
    exit(0)
}
#endif
