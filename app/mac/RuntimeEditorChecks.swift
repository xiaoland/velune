import AppKit
import SwiftUI
import VeluneBindings

/// The real editor's bindings and save action, captured after its execution settings load.
/// The hidden native harness drives these locally without posting system input.
struct RuntimeEditorCheckAccess {
    let type: Binding<RuntimeKind?>
    let settings: Binding<[String: String]>
    let newConversation: Binding<NewConversationChoice>
    let execution: () -> RuntimeExecution?
    let launch: () -> AcpLaunch?
    let save: () -> Void
}

@MainActor func runtimeEditorChecks(home: URL) -> Never {
    func note(_ value: String) { FileHandle.standardError.write(Data((value + "\n").utf8)) }
    func wait(_ condition: () -> Bool) {
        let deadline = Date().addingTimeInterval(15)
        while !condition(), Date() < deadline { RunLoop.main.run(until: Date().addingTimeInterval(0.02)) }
        precondition(condition(), "等待运行时编辑器检查超时")
    }
    let transport = Transport(stateDirectory: home.appendingPathComponent("runtime-editor"), resourcesDirectory: Bundle.main.resourceURL!)
    let source = RuntimeInstance(enabled: false, id: "synthetic-existing-runtime", name: "保留实例名称", typeId: .acp, gatewayId: "default", settings: ["binary": "/synthetic/source-agent", "source-only": "保留原类型字段"])
    let execution = RuntimeExecution(transport: .ssh(binary: "/usr/bin/ssh", destination: "synthetic.invalid", port: 2222), home: .managed(remoteRoot: "/synthetic/runtime-homes"), remoteResourcesDirectory: "/synthetic/resources")
    let launch = AcpLaunch(args: ["--synthetic", "source argument"], environment: ["SYNTHETIC_ONLY": "source value"])
    _ = try! transport.upsertRuntime(source, execution: execution, launch: launch)
    try! transport.setRuntimeComposerChoices(RuntimeComposerChoices(runtimeId: source.id, runtimeType: source.typeId, lastChosen: TurnChoice(model: .native(providerId: "source-provider", modelId: "source-model"))))
    let device = DeviceStore(transport: transport)
    device.start()
    wait { device.runtimeInstances.contains { $0.id == source.id } && device.runtimeComposerChoices[source.id] != nil }
    var actions: RuntimeEditorCheckAccess?
    let editor = NSHostingView(rootView: RuntimeEditor(store: device, instance: source, observeForCheck: { actions = $0 }).applicationAppearance())
    editor.frame = NSRect(x: 0, y: 0, width: 590, height: 600)
    let window = NSWindow(contentRect: editor.frame, styleMask: [.borderless], backing: .buffered, defer: false)
    window.contentView = editor
    editor.layoutSubtreeIfNeeded()
    wait { actions != nil }
    func settle() { RunLoop.main.run(until: Date().addingTimeInterval(0.1)); editor.layoutSubtreeIfNeeded() }
    func popups(_ view: NSView) -> [NSPopUpButton] {
        (view as? NSPopUpButton).map { [$0] } ?? view.subviews.flatMap(popups)
    }
    settle()
    let sourceName = device.runtimeTypes.first { $0.id == .acp }!.name
    guard let picker = popups(editor).first(where: { $0.title == sourceName }) else { fatalError("原生类型选择器缺失") }
    precondition(picker.isEnabled, "现有实例的类型选择器必须可用")
    let form = actions!
    precondition(form.type.wrappedValue == .acp && form.execution() == execution && form.launch() == launch)
    form.settings.wrappedValue["binary"] = "/synthetic/edited-source-agent"
    let originalChoice = NewConversationChoice.chosen(choice: TurnChoice(permissionMode: "source-only-choice"))
    form.newConversation.wrappedValue = originalChoice
    form.type.wrappedValue = .claudeCode
    settle()
    precondition(form.settings.wrappedValue.isEmpty, "新类型不能继承同名 binary 或原类型隐藏字段")
    precondition(form.newConversation.wrappedValue == .lastChosen && form.launch() == AcpLaunch(args: [], environment: [:]))
    let targetSettings = ["binary": "/synthetic/target-agent", "agentDir": "/synthetic/target-state"]
    form.settings.wrappedValue = targetSettings
    let permission = device.runtimeTypes.first { $0.id == .claudeCode }!.permissionModes.first!.id
    let targetChoice = NewConversationChoice.chosen(choice: TurnChoice(permissionMode: permission))
    form.newConversation.wrappedValue = targetChoice
    form.type.wrappedValue = .acp
    settle()
    precondition(form.settings.wrappedValue["binary"] == "/synthetic/edited-source-agent" && form.settings.wrappedValue["source-only"] == "保留原类型字段")
    precondition(form.newConversation.wrappedValue == originalChoice && form.launch() == launch)
    form.type.wrappedValue = .codex166
    settle()
    precondition(form.settings.wrappedValue.isEmpty && form.newConversation.wrappedValue == .lastChosen)
    form.type.wrappedValue = .claudeCode
    settle()
    precondition(form.settings.wrappedValue == targetSettings && form.newConversation.wrappedValue == targetChoice && form.execution() == execution)
    let targetName = device.runtimeTypes.first { $0.id == .claudeCode }!.name
    precondition(popups(editor).contains { $0.title == targetName && $0.isEnabled }, "原生选择器必须呈现当前类型")
    note("PASS hidden native editor: existing type picker enabled; ACP → Claude → ACP → Codex166 → Claude keeps separate fields, launch and new-conversation drafts; SSH/HOME unchanged")
    if let bitmap = editor.bitmapImageRepForCachingDisplay(in: editor.bounds) {
        editor.cacheDisplay(in: editor.bounds, to: bitmap)
        try! bitmap.representation(using: .png, properties: [:])!.write(to: home.appendingPathComponent("runtime-editor.png"))
    }
    form.save()
    // Change the active form before the asynchronous save completion can run.
    // The saved instance and its new-conversation choice must use the click's snapshot.
    form.type.wrappedValue = .acp
    form.newConversation.wrappedValue = .lastChosen
    precondition(form.type.wrappedValue == .acp && form.newConversation.wrappedValue == .lastChosen)
    wait { device.runtimeInstances.first { $0.id == source.id }?.typeId == .claudeCode && (try? device.newConversationChoice(for: source.id)) == targetChoice }
    let saved = try! transport.list().runtimeInstances.first { $0.id == source.id }!
    precondition(saved.id == source.id && saved.name == source.name && saved.gatewayId == source.gatewayId && saved.enabled == source.enabled && saved.typeId == .claudeCode)
    precondition(saved.settings["binary"] == targetSettings["binary"] && saved.settings["agentDir"] == targetSettings["agentDir"] && saved.settings["source-only"] == nil)
    precondition(try! transport.runtimeExecution(id: saved.id).0 == execution)
    let choices = try! transport.composerChoices().runtimes.first { $0.runtimeId == saved.id }!
    precondition(try! runtimeNewConversationChoice(runtime: saved) == targetChoice)
    precondition(choices.runtimeType == .acp && choices.lastChosen.model != nil)
    var lastPolicy = saved
    lastPolicy = try! runtimeWithNewConversationChoice(runtime: lastPolicy, choice: .lastChosen)
    precondition(try! runtimeStartChoice(runtime: lastPolicy, last: choices) == TurnChoice())
    precondition(!window.isVisible && !window.isKeyWindow && !NSApp.isActive && NSApp.activationPolicy() == .prohibited)
    note("PASS saved through real editor/core: same ID/name/gateway/enabled/SSH/managed HOME; selected type, fields and policy committed together; old last-chosen remains scoped to ACP; no visible window or system input")
    note("PASS save snapshot: switching the editor back to original ACP/default immediately after Save keeps the submitted Claude type, fields and new-conversation choice in the completion and persisted core")
    exit(0)
}
