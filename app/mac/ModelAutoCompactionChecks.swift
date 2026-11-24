import AppKit
import SwiftUI
import VeluneBindings

/// Mounts the production forms without presenting sheets or starting a Harness.
@MainActor func modelAutoCompactionChecks(home: URL) -> Never {
    func note(_ text: String) { FileHandle.standardError.write(Data((text + "\n").utf8)) }
    func wait(_ operation: String, _ ready: () -> Bool) {
        let until = Date().addingTimeInterval(10)
        while !ready(), Date() < until { RunLoop.main.run(until: Date().addingTimeInterval(0.02)) }
        precondition(ready(), "等待模型自动压缩验收超时：\(operation)")
    }

    let transport = Transport(stateDirectory: home.appendingPathComponent("model-compaction"), resourcesDirectory: Bundle.main.resourceURL!)
    let inventory = try! transport.list()
    let gatewayID = inventory.gateways.first?.id ?? "default"
    let savedProvider = try! transport.saveProvider(gatewayID: gatewayID, provider: ProviderDraft(
        id: "synthetic-compaction-provider", name: "合成提供商", protocol: .messages, endpoint: "http://127.0.0.1:1/v1",
        executionBackend: .http, requestProfile: .native, authenticationSource: .apiKey, quotaSource: .none,
        models: [ProviderModel(recordKey: "", providerModelId: "synthetic-managed", nickname: "合成托管模型",
                               contextWindow: 1_000_000, maxOutputTokens: 32_000)], quotaLimits: []),
        authenticationEdit: .setApiKey(value: "synthetic"))
    let provider = savedProvider.gateways.flatMap(\.providers).first { $0.id == "synthetic-compaction-provider" }!
    let key = provider.models[0].recordKey
    let route = ModelRoute(recordKey: "synthetic-compaction-route", name: "合成聚合路由", members: [key],
                           strategy: .order, weights: [:], sticky: false)
    _ = try! transport.saveModelRoute(gatewayID: gatewayID, route: route)
    let execution = RuntimeExecution(transport: .local, home: .existing(path: home.path), remoteResourcesDirectory: nil)
    let agentDirectory = home.appendingPathComponent("unavailable-agent-home", isDirectory: true)
    try! FileManager.default.createDirectory(at: agentDirectory, withIntermediateDirectories: true)
    let first = RuntimeInstance(enabled: true, id: "compaction-claude", name: "合成 Claude",
                                typeId: .claudeCode, gatewayId: gatewayID,
                                settings: ["binary": home.appendingPathComponent("unavailable-claude").path, "agentDir": agentDirectory.path])
    let second = RuntimeInstance(enabled: false, id: "compaction-claude-disabled", name: "已停用的合成 Claude",
                                 typeId: .claudeCode, gatewayId: gatewayID, settings: [:])
    let acp = RuntimeInstance(enabled: true, id: "compaction-acp", name: "合成 ACP",
                              typeId: .acp, gatewayId: gatewayID, settings: ["binary": home.appendingPathComponent("unavailable-acp").path])
    _ = try! transport.upsertRuntime(first, execution: execution)
    _ = try! transport.upsertRuntime(second, execution: execution)
    _ = try! transport.upsertRuntime(acp, execution: execution, launch: AcpLaunch(args: [], environment: [:]))
    let managed = ModelSelection.managed(modelRecordKey: key)
    let managedRoute = ModelSelection.managed(modelRecordKey: route.recordKey)
    let retired = ModelSelection.native(providerId: nil, modelId: "synthetic-retired-model")
    let current = ModelSelection.native(providerId: nil, modelId: "synthetic-current-model")
    _ = try! transport.setRuntimeModelAutoCompaction(runtimeID: first.id, model: managed, setting: .claude(windowTokens: 700_000))
    _ = try! transport.setRuntimeModelAutoCompaction(runtimeID: first.id, model: managedRoute, setting: .claude(windowTokens: 850_000))
    _ = try! transport.setRuntimeModelAutoCompaction(runtimeID: first.id, model: retired, setting: .claude(windowTokens: 500_000))
    let inactiveAcp = ModelSelection.native(providerId: nil, modelId: "synthetic-acp-retired")
    _ = try! transport.setRuntimeModelAutoCompaction(runtimeID: acp.id, model: inactiveAcp, setting: .acp(optionId: "auto_compaction", value: .boolean(false)))

    let device = DeviceStore(transport: transport)
    device.applyList(try! transport.list())
    var hiddenWindows: [NSWindow] = []
    @discardableResult func mount<V: View>(_ content: V, width: CGFloat, height: CGFloat) -> NSHostingView<some View> {
        let host = NSHostingView(rootView: content.applicationAppearance().background(Color(nsColor: .windowBackgroundColor)))
        let window = NSWindow(contentRect: NSRect(x: 0, y: 0, width: width, height: height),
                              styleMask: [.borderless], backing: .buffered, defer: false)
        window.contentView = host
        host.layoutSubtreeIfNeeded()
        hiddenWindows.append(window)
        return host
    }
    func read(_ model: ModelSelection, runtimeID: String = "compaction-claude") -> AutoCompactionSetting? {
        let instance = try! transport.list().runtimeInstances.first { $0.id == runtimeID }!
        let entries = try! runtimeModelAutoCompactions(runtime: instance)
        return entries.first { $0.model == model }?.setting
    }

    var providerTargets: [ModelAutoCompactionTarget?] = []
    var selectProviderModel: ((String?) -> Void)?
    mount(ProviderEditor(store: device, provider: provider, observeModelCompactionForCheck: { providerTargets.append($0) },
                         observeModelSelectionForCheck: { selectProviderModel = $0 }), width: 650, height: 510)
    wait("读取提供商模型编辑器") { selectProviderModel != nil }
    selectProviderModel?(key)
    wait("定位托管模型入口") { !providerTargets.isEmpty }
    let modelTarget = providerTargets.last!
    precondition(modelTarget?.model == managed && modelTarget?.title == "合成托管模型")
    selectProviderModel?(nil)
    wait("未保存的模型草稿入口") { providerTargets.count > 1 }
    precondition(providerTargets.last! == nil, "草稿不能写入模型设置")

    var routeTargets: [ModelAutoCompactionTarget?] = []
    mount(ModelRouteEditor(store: device, route: route, observeModelCompactionForCheck: { routeTargets.append($0) }), width: 540, height: 540)
    wait("定位模型路由入口") { routeTargets.count == 1 }
    let routeTarget = routeTargets[0]!
    precondition(routeTarget.model == managedRoute && routeTarget.title == route.name)
    let draftRoute = ModelRoute(recordKey: "", name: "", members: [key], strategy: .order, weights: [:], sticky: false)
    mount(ModelRouteEditor(store: device, route: draftRoute, observeModelCompactionForCheck: { routeTargets.append($0) }), width: 540, height: 540)
    wait("未保存的路由入口") { routeTargets.count == 2 }
    precondition(routeTargets[1] == nil)
    precondition(Set(modelTarget!.runtimes(in: device).map(\.id)).isSuperset(of: [first.id, second.id]))
    precondition(!routeTarget.runtimes(in: device).contains { $0.id == acp.id })
    note("PASS saved provider model and route editors expose their own model identity; drafts cannot set policies; generic ACP is not a managed runtime choice")

    func edit(_ target: ModelAutoCompactionTarget, capture: Bool = false) -> ModelAutoCompactionCheckAccess {
        wait("等待配置刷新") { !device.isLoading }
        var actions: ModelAutoCompactionCheckAccess?
        let host = mount(ModelAutoCompactionEditor(store: device, target: target, observeForCheck: { actions = $0 }),
                         width: 450, height: 232)
        wait("加载自动压缩编辑器") { actions != nil }
        if capture {
            RunLoop.main.run(until: Date().addingTimeInterval(0.1))
            host.layoutSubtreeIfNeeded()
            guard let bitmap = host.bitmapImageRepForCachingDisplay(in: host.bounds) else { fatalError("无法绘制自动压缩编辑器") }
            host.cacheDisplay(in: host.bounds, to: bitmap)
            try! bitmap.representation(using: .png, properties: [:])!.write(to: home.appendingPathComponent("model-auto-compaction.png"))
        }
        return actions!
    }
    let form = edit(modelTarget!, capture: true)
    precondition(form.runtime.wrappedValue == first.id && form.value.wrappedValue == "700K")
    form.value.wrappedValue = "720K"
    form.runtime.wrappedValue = second.id
    precondition(form.runtime.wrappedValue == first.id, "编辑时不能把值写到另一个实例")
    form.save()
    wait("覆盖原有托管模型设置") { read(managed) == .claude(windowTokens: 720_000) }
    let another = edit(modelTarget!)
    another.runtime.wrappedValue = second.id
    precondition(another.value.wrappedValue.isEmpty)
    another.value.wrappedValue = "600K"
    another.save()
    wait("独立保存第二个实例") { read(managed, runtimeID: second.id) == .claude(windowTokens: 600_000) }
    let routeForm = edit(routeTarget)
    precondition(routeForm.value.wrappedValue == "850K")
    routeForm.value.wrappedValue = ""
    routeForm.save()
    wait("路由恢复默认") { read(managedRoute) == nil }
    precondition(read(managed) == .claude(windowTokens: 720_000))
    note("PASS existing policies are loaded, provider-model override is scoped to the selected instance, and resetting the route preserves other settings")

    func next(_ runtime: RuntimeInstance, model: ModelSelection?, catalog: [RuntimeModel]) -> NextTurn {
        resolveNextTurn(input: NextTurnInput(runtimeId: runtime.id,
            runtimeType: device.runtimeTypes.first { $0.id == runtime.typeId }, providers: [], catalog: catalog,
            kept: TurnChoice(model: model), newConversation: .chosen(choice: TurnChoice())))
    }
    let nativeCatalog = [RuntimeModel(selection: current, name: "合成原生模型", reasoningEfforts: [])]
    let nativeNext = next(first, model: current, catalog: nativeCatalog)
    let nativeTab = ConversationStore.modelAutoCompactionCheckTab(device: device, next: nativeNext)
    let context = ComposerContext(tab: nativeTab, model: ComposerModel())
    let targets = context.nativeModelAutoCompactionTargets
    precondition(targets.selected?.model == current && targets.selected?.runtimes(in: device).map(\.id) == [first.id])
    precondition(targets.retained.map(\.model) == [retired])
    let retainedForm = edit(targets.retained[0])
    precondition(retainedForm.value.wrappedValue == "500K")
    retainedForm.value.wrappedValue = ""
    retainedForm.save()
    wait("目录外原生模型恢复默认") { read(retired) == nil }
    let defaultTab = ConversationStore.modelAutoCompactionCheckTab(device: device, next: next(first, model: nil, catalog: nativeCatalog))
    precondition(ComposerContext(tab: defaultTab, model: ComposerModel()).nativeModelAutoCompactionTargets.selected == nil)
    let readOnlyDevice = DeviceStore(preview: true)
    readOnlyDevice.applyList(try! transport.list())
    let readOnly = ConversationStore.modelAutoCompactionCheckTab(device: readOnlyDevice, next: nativeNext)
    precondition(ComposerContext(tab: readOnly, model: ComposerModel()).modelAutoCompactionRuntime == nil)

    let agentModel = ModelSelection.native(providerId: nil, modelId: "synthetic-acp-current")
    let otherModel = ModelSelection.native(providerId: nil, modelId: "synthetic-acp-pending")
    let options = [
        AcpConfigOption(id: "model", name: "模型", description: nil, category: "model", value: .select("synthetic-acp-current"), choices: []),
        AcpConfigOption(id: "auto_compaction", name: "自动压缩", description: nil, category: nil, value: .boolean(true), choices: [])
    ]
    let agentNext = next(acp, model: agentModel, catalog: [RuntimeModel(selection: agentModel, name: "合成 ACP 模型", reasoningEfforts: [])])
    let agent = ConversationStore.modelAutoCompactionCheckTab(device: device, next: agentNext, options: options)
    precondition(agent.store.acpAutoCompactionModel(runtimeID: acp.id, selectedModel: agentModel) == agentModel)
    precondition(agent.store.acpAutoCompactionModel(runtimeID: acp.id, selectedModel: otherModel) == nil)
    precondition(agent.store.acpAutoCompactionModel(runtimeID: first.id, selectedModel: agentModel) == nil)
    precondition(agent.store.acpAutoCompactionOptions.map(\.id) == ["auto_compaction"])
    let inactiveAcpTargets = ComposerContext(tab: agent, model: ComposerModel()).nativeModelAutoCompactionTargets.retained
    precondition(inactiveAcpTargets.map(\.model) == [inactiveAcp])
    let inactiveAcpForm = edit(inactiveAcpTargets[0])
    precondition(inactiveAcpForm.value.wrappedValue == "auto_compaction · 关闭")
    inactiveAcpForm.resetAcp()
    wait("未加载 ACP 模型恢复默认") { read(inactiveAcp, runtimeID: acp.id) == nil }
    note("PASS composer model menu resolves the exact next-turn model and runtime, retains unlisted native overrides for clearing, and rejects a mismatched ACP session or unwritable device")

    precondition(hiddenWindows.allSatisfy { !$0.isVisible && !$0.isKeyWindow } && !NSApp.isActive && NSApp.activationPolicy() == .prohibited)
    exit(0)
}
