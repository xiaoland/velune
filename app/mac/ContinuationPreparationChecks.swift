import AppKit
import SwiftUI
import VeluneBindings

/// The production preparation Session and generated records, with only the
/// asynchronous operations replaced. No model, account or native runtime starts.
private final class ContinuationCheckBackend: @unchecked Sendable {
    struct State {
        var required = true
        var defaultIssue: String?
        var preflightFailure: String?
        var checking = 0
        var requirementsGate: DispatchSemaphore?
        var basisOverride: NextTurnInput?
        var requestIDs: [String?] = []
        var queueGate: DispatchSemaphore?
        var queueFailure = false
        var queueCalls = 0
        var acknowledgements = 0
        var queuedIDs: [String?] = []
        var selectedModel: ModelSelection?
        var effort: String?
    }
    struct Failure: LocalizedError { let detail: String; var errorDescription: String? { detail } }
    private let lock = NSLock()
    private var state = State()
    let basis: NextTurnInput
    init(basis: NextTurnInput) { self.basis = basis }
    func withState<T>(_ body: (inout State) throws -> T) rethrows -> T { try lock.withLock { try body(&state) } }
    func operations() -> ContinuationPreparationOperations {
        ContinuationPreparationOperations(requirements: { [self] requestID in
            let (result, gate) = withState { state -> (ContinuationRequirement, DispatchSemaphore?) in
                state.requestIDs.append(requestID)
                let result: ContinuationRequirement
                if state.checking > 0 { state.checking -= 1; result = .checking("synthetic-requirements") }
                else if let detail = state.preflightFailure { result = .failed(detail) }
                else { result = state.required ? .summary(state.basisOverride ?? basis, state.defaultIssue) : .ready }
                let gate = state.requirementsGate; state.requirementsGate = nil
                return (result, gate)
            }
            if let gate, gate.wait(timeout: .now() + 5) != .success { throw Failure(detail: "合成预检回执超时") }
            return result
        }, queue: { [self] existingID, model, effort in
            let gate = withState { state in
                state.queueCalls += 1; state.queuedIDs.append(existingID)
                state.selectedModel = model; state.effort = effort
                return state.queueGate
            }
            if let gate, gate.wait(timeout: .now() + 5) != .success { throw Failure(detail: "合成队列ACK超时") }
            return try withState { state in
                if state.queueFailure { throw Failure(detail: "合成队列未受理") }
                state.acknowledgements += 1
                return ContinuationQueuedWork(workID: "core-owned-work", queuedMessageID: existingID ?? "accepted-head", result: .init(snapshot: nil))
            }
        })
    }
}
@MainActor private final class ContinuationCheckObserver {
    var sends = 0
    var finishes = 0
    var token: String?
    var capturedText: String?
    var capturedAttachmentPaths: [String] = []
    var holdSend = false
    var retainedInputs: [ComposerModel] = []
}
@MainActor private final class ContinuationCheckFixture {
    let device: DeviceStore
    let store: ConversationStore
    let input = ComposerModel()
    let backend: ContinuationCheckBackend
    let observer = ContinuationCheckObserver()
    let target: NextTurn
    let attachment: URL
    let alpha = ModelSelection.native(providerId: "synthetic-source", modelId: "alpha")
    let beta = ModelSelection.native(providerId: "synthetic-source", modelId: "beta")
    init(directory: URL, long: Bool = false) {
        let device = DeviceStore(preview: true)
        self.device = device
        let alpha = ModelSelection.native(providerId: "synthetic-source", modelId: "alpha")
        let beta = ModelSelection.native(providerId: "synthetic-source", modelId: "beta")
        var codex = device.runtimeInstances.first { $0.id == "sample-instance" }!
        codex.name = "Codex"; device.saveRuntimeInstance(codex)
        var pi = device.runtimeInstances.first { $0.id == "sample-review" }!
        pi.name = "Pi"; device.saveRuntimeInstance(pi)
        store = ConversationStore(device: device)
        attachment = directory.appendingPathComponent("synthetic-attachment.txt")
        try! Data("Only synthetic attachment bytes".utf8).write(to: attachment)
        let models = [RuntimeModel(selection: alpha, name: "Alpha", reasoningEfforts: ["low", "high"]),
                      RuntimeModel(selection: beta, name: long ? "Beta " + String(repeating: "Long source-compatible model name ", count: 8) : "Beta", reasoningEfforts: ["low", "high"])]
        let basis = NextTurnInput(runtimeId: "sample-instance", runtimeType: device.runtimeTypes.first,
            providers: [], catalog: models,
            turnSettings: [.init(runtimeId: "sample-instance", model: "alpha", effort: "low", modelSelection: alpha)],
            conversationRuntimeId: "sample-instance", modelRecordKey: nil, kept: nil, runtimeChoices: nil,
            newConversation: .chosen(choice: TurnChoice()))
        backend = ContinuationCheckBackend(basis: basis)
        let piModel = ModelSelection.native(providerId: "synthetic-target", modelId: "pi-target-model")
        target = resolveNextTurn(input: NextTurnInput(runtimeId: "sample-review", runtimeType: device.runtimeTypes.first,
            providers: [], catalog: [.init(selection: piModel, name: "Pi 接续模型", reasoningEfforts: ["low", "high"])],
            turnSettings: [], conversationRuntimeId: "sample-instance", modelRecordKey: nil,
            kept: TurnChoice(model: piModel, effort: "high"), runtimeChoices: nil,
            newConversation: .chosen(choice: TurnChoice())))
        input.attach(store)
        input.restore(long ? String(repeating: "保留这条原输入、附件和 Pi 的执行配置，摘要模型独立选择。\n", count: 12) : "保留现有约束，继续检查这个合成项目。", attachments: [attachment])
        store.selectComposerRuntime("sample-review")
    }
    func session() -> ContinuationPreparationSession {
        let observer = observer
        return ContinuationPreparationSession(source: store, input: input, target: target,
            sourceRuntimeID: store.loadedConversationRuntimeID!, sourceContextRuntimeID: store.projectionRuntimeID!,
            conversationID: store.loadedConversationID!, operations: backend.operations(),
            send: { session in
                observer.sends += 1; observer.capturedText = session.text; observer.capturedAttachmentPaths = session.attachments.map(\.path)
                if !observer.holdSend { session.accepted() }
            }, finish: { _ in observer.finishes += 1 },
            retainInput: { model in
                if !observer.retainedInputs.contains(where: { $0 === model }) { observer.retainedInputs.append(model) }
            })
    }
    func close() { store.close() }
}
@MainActor private final class ContinuationCheckWindow: NSWindow {
    override var canBecomeKey: Bool { false }
    override var canBecomeMain: Bool { false }
    private func refused() -> Never {
        FileHandle.standardError.write(Data("FAIL continuation native check attempted to show or activate a window\n".utf8)); exit(1)
    }
    override func makeKey() { refused() }
    override func makeMain() { refused() }
    override func makeKeyAndOrderFront(_ sender: Any?) { refused() }
    override func orderFront(_ sender: Any?) { refused() }
    override func orderFrontRegardless() { refused() }
    override func order(_ place: NSWindow.OrderingMode, relativeTo otherWin: Int) { if place != .out { refused() }; super.order(place, relativeTo: otherWin) }
}

@MainActor func continuationPreparationChecks() -> Never {
    func require(_ value: @autoclosure () -> Bool, _ detail: String) {
        guard value() else { FileHandle.standardError.write(Data("FAIL continuation confirmation: \(detail)\n".utf8)); exit(1) }
    }
    NSApplication.shared.setActivationPolicy(.prohibited)
    require(NSApp.activationPolicy() == .prohibited, "检查禁止激活")
    guard let path = ProcessInfo.processInfo.environment["VELUNE_CONTINUATION_CHECK_HOME"], path.hasPrefix("/tmp/") else { exit(2) }
    let directory = URL(fileURLWithPath: path, isDirectory: true)
    try! FileManager.default.createDirectory(at: directory, withIntermediateDirectories: true)
    let output = ProcessInfo.processInfo.environment["VELUNE_CHECK_OUTPUT"].map { URL(fileURLWithPath: $0, isDirectory: true) } ?? directory
    try! FileManager.default.createDirectory(at: output, withIntermediateDirectories: true)
    let frontmost = NSWorkspace.shared.frontmostApplication?.processIdentifier
    func safe() {
        require(!NSApp.isActive && NSApp.activationPolicy() == .prohibited, "进程不激活")
        require(NSApp.windows.allSatisfy { !$0.isVisible && !$0.isKeyWindow && !$0.isMainWindow }, "检查不显示窗口")
        require(NSWorkspace.shared.frontmostApplication?.processIdentifier == frontmost, "前台不改变")
    }
    func wait(_ condition: () -> Bool) {
        let deadline = Date().addingTimeInterval(8)
        while !condition(), Date() < deadline { RunLoop.main.run(until: Date().addingTimeInterval(0.01)); safe() }
        require(condition(), "异步状态检查超时")
    }
    func fixture(long: Bool = false) -> ContinuationCheckFixture { ContinuationCheckFixture(directory: directory, long: long) }
    var passes: [String] = []
    func passed(_ text: String) { passes.append(text); FileHandle.standardError.write(Data("PASS \(text)\n".utf8)) }
    do {
        let f = fixture(); f.observer.holdSend = true; f.backend.withState { $0.required = false }; let session = f.session()
        session.start(); wait { session.phase == .sending }
        require(f.observer.sends == 1 && f.backend.withState { $0.queueCalls == 0 } && !f.input.isEmpty, "Ready沿用普通发送，接受前不消费")
        session.accepted(); require(f.input.isEmpty, "普通接受消费原baseline")
        f.close(); passed("无需摘要Ready仍走原发送，不新增队列工作或提前清空")
    }
    do {
        let f = fixture(); let session = f.session(); session.start(); wait { session.phase == .ready }
        require(session.isPresented && f.backend.withState { $0.queueCalls == 0 }, "轻确认前不调用模型/队列")
        let gate = DispatchSemaphore(value: 0); f.backend.withState { $0.queueGate = gate }
        session.proceed(); wait { session.phase == .queueing }
        session.hide(); session.cancel()
        require(session.phase == .queueing && !session.isPresented && !f.input.isEmpty, "隐藏不取消已发出的原子受理，也不提前清稿")
        gate.signal(); wait { f.observer.finishes == 1 }
        require(f.input.isEmpty && f.observer.sends == 0 && f.backend.withState { $0.acknowledgements == 1 }, "ACK后清稿，前端不得SendPrepared/第二次派发")
        f.close(); passed("确认后hide，原子ACK才消费；Core拥有压缩与自动派发，前端无第二发送")
    }
    do {
        let f = fixture(); let session = f.session(); session.start(); wait { session.phase == .ready }
        session.choose(.model(selection: f.beta)); session.choose(.effort(level: "high"))
        f.backend.withState { $0.checking = 1 }; session.proceed(); wait { f.observer.finishes == 1 }
        require(f.backend.withState { $0.requestIDs == [nil, nil, "synthetic-requirements"] && $0.selectedModel == f.beta && $0.effort == "high" && $0.queueCalls == 1 }, "每次确认fresh读/精确Checking并保留手选")
        f.close(); passed("确认重新预检，Checking同ID完成后仅一次原子受理")
    }
    for accepted in [false, true] {
        let f = fixture(); let session = f.session(); session.start(); wait { session.phase == .ready }
        f.input.restoreSavedDraft(.init(text: "后来新草稿", attachments: []))
        f.backend.withState { $0.queueFailure = !accepted }; session.proceed()
        if accepted { wait { f.observer.finishes == 1 } } else { wait { session.phase == .failed } }
        require(f.input.submission == "后来新草稿", "ACK或失败不覆盖后来稿")
        if !accepted { require(f.observer.retainedInputs.first?.savedDraft == .init(text: session.text, attachments: session.attachments) && !session.isPresented, "失败经既有保留输入恢复捕获，不自动弹表单"); session.close() }
        f.close(); passed(accepted ? "真实接受只按revision消费，原composer后来输入保留" : "受理失败保留新草稿并找回旧capture，不自动弹表单")
    }
    for edited in [false, true] {
        let f = fixture(); let session = f.session(); session.start(); wait { session.phase == .ready }
        f.input.restoreSavedDraft(.init(text: "后来输入", attachments: []))
        f.backend.withState { $0.queueFailure = true }; session.proceed(); wait { session.phase == .failed }
        let recovered = f.observer.retainedInputs.first!
        if edited { recovered.restoreSavedDraft(.init(text: "恢复副本又修改", attachments: [])) }
        f.backend.withState { $0.queueFailure = false }; session.proceed(); wait { f.observer.finishes == 1 }
        require(edited ? recovered.submission == "恢复副本又修改" : recovered.isEmpty, "重试接受按恢复副本revision消费")
        require(f.input.submission == "后来输入", "恢复副本不抢回新草稿")
        f.close(); passed(edited ? "恢复副本已修改时重试接受保留它" : "恢复副本未修改时重试接受消费它，避免重复输入")
    }
    do {
        let f = fixture(); let gate = DispatchSemaphore(value: 0); let session = f.session()
        session.start(); wait { session.phase == .ready }; f.backend.withState { $0.queueGate = gate }
        session.proceed(); wait { f.backend.withState { $0.queueCalls == 1 } }; session.close(); gate.signal()
        wait { f.input.isEmpty }
        require(f.observer.finishes == 1 && f.backend.withState { $0.acknowledgements == 1 } && f.observer.sends == 0, "关Tab不删除Core已受理队列或吞掉ACK")
        f.close(); passed("关Tab期间迟到ACK仍消费初始稿，不删除Core队列/工作")
    }
    for navigate in [false, true] {
        let f = fixture(); let gate = DispatchSemaphore(value: 0); var session: ContinuationPreparationSession? = f.session()
        session!.start(); wait { session!.phase == .ready }; f.backend.withState { $0.requirementsGate = gate }
        session!.proceed(); wait { f.backend.withState { $0.requestIDs.count == 2 } }
        if navigate { f.store.selectConversation(id: "sample-1") } else { session!.close() }
        weak var retained = session; session = nil; gate.signal(); wait { retained == nil }; retained = nil
        require(f.backend.withState { $0.queueCalls == 0 }, "取消/导航期间晚到fresh读不得受理")
        f.close(); passed(navigate ? "导航后fresh结果不创建队列" : "取消后fresh结果不创建队列")
    }
    do {
        let f = fixture(); let session = f.session(); session.start(); wait { session.phase == .ready }; session.choose(.model(selection: f.beta))
        var basis = f.backend.basis; basis.catalog = basis.catalog?.filter { $0.selection != f.beta }; f.backend.withState { $0.basisOverride = basis }
        session.proceed(); wait { session.phase == .ready }
        require(session.modelIssue != nil && session.modelTitle == "所选摘要模型不可用" && !session.canProceed && f.backend.withState { $0.queueCalls == 0 }, "失效不fallback/计费")
        session.close(); f.close(); passed("fresh模型失效只提示重选，不静默回退或受理")
    }
    do {
        let f = fixture(); var basis = f.backend.basis
        basis.catalog?.append(.init(selection: f.target.model!, name: "同名Native", reasoningEfforts: ["high"]))
        f.backend.withState { $0.basisOverride = basis }; let session = f.session(); session.start(); wait { session.phase == .ready }
        require(session.summaryChoice.model == nil, "Native同ID不证明同账户")
        session.close(); f.close(); passed("跨运行时Native同名仍默认源账户")
    }
    do {
        let f = fixture(); let session = ContinuationPreparationSession(source: f.store, input: f.input, target: f.target,
            sourceRuntimeID: f.store.loadedConversationRuntimeID!, sourceContextRuntimeID: f.store.projectionRuntimeID!, conversationID: f.store.loadedConversationID!,
            operations: f.backend.operations(), send: { _ in }, finish: { _ in }, queuedMessageID: "existing-head")
        session.start(); wait { session.phase == .ready }; f.input.restoreSavedDraft(.init(text: "新Tab的无关草稿", attachments: []))
        session.proceed(); wait { session.phase == .closed }
        require(f.backend.withState { $0.queuedIDs == ["existing-head"] } && f.input.submission == "新Tab的无关草稿", "重开重试复用queueID，不追加/消费新稿")
        f.close(); passed("新Tab重试复用既有queuedMessageID，普通新稿不被消费")
    }
    // Content-only native rendering: work progress belongs to Core's queue.
    var renderings: [[String: Any]] = []
    let dark = CommandLine.arguments.contains("--continuation-check-dark")
    NSApp.appearance = NSAppearance(named: dark ? .darkAqua : .aqua)
    let before = AppearancePreferences.shared.values
    for name in ["ready", "advanced", "long", "default-model", "failure", "checking", "queueing"] {
        let f = fixture(long: name == "long"); let session = f.session()
        if name == "default-model" { f.backend.withState { $0.defaultIssue = "来源模型当前不可用，请选择兼容模型。" } }
        session.start(); wait { session.phase == .ready }
        var gate: DispatchSemaphore?
        if name == "failure" { f.backend.withState { $0.queueFailure = true }; session.proceed(); wait { session.phase == .failed }; session.show() }
        if name == "checking" || name == "queueing" {
            let held = DispatchSemaphore(value: 0); gate = held
            f.backend.withState { if name == "checking" { $0.requirementsGate = held } else { $0.queueGate = held } }
            session.proceed(); wait { name == "checking" ? session.phase == .checking : session.phase == .queueing }
        }
        if name == "long" { session.choose(.model(selection: f.beta)); AppearancePreferences.shared.values.ui = .extraLarge }
        let host = NSHostingView(rootView: ContinuationPreparationView(session: session, settingsExpanded: name == "advanced" || name == "long").applicationAppearance().background(Color(nsColor: .windowBackgroundColor)))
        host.frame = NSRect(x: 0, y: 0, width: 440, height: 500)
        let window = ContinuationCheckWindow(contentRect: host.frame, styleMask: [.borderless], backing: .buffered, defer: true)
        window.isReleasedWhenClosed = false; window.isRestorable = false; window.contentView = host
        host.layoutSubtreeIfNeeded(); RunLoop.main.run(until: Date().addingTimeInterval(0.1)); host.layoutSubtreeIfNeeded(); safe()
        let height = max(160, host.fittingSize.height); require(height <= 600, "轻确认高度有界")
        window.setContentSize(.init(width: 440, height: height)); host.layoutSubtreeIfNeeded(); safe()
        guard let bitmap = host.bitmapImageRepForCachingDisplay(in: host.bounds) else { exit(1) }
        host.cacheDisplay(in: host.bounds, to: bitmap)
        try! bitmap.representation(using: .png, properties: [:])!.write(to: output.appendingPathComponent("\(name)-\(dark ? "dark" : "light").png"))
        renderings.append(["state": name, "width": host.bounds.width, "height": host.bounds.height, "windowVisible": window.isVisible])
        window.contentView = nil; window.close(); session.close(); gate?.signal(); f.close(); AppearancePreferences.shared.values = before
    }
    let result: [String: Any] = ["passed": passes, "nativeStates": renderings, "systemChromeVerified": false, "realModelCalls": 0, "foregroundUnchanged": true,
        "supersedes": "combined editable message sheet and its editor/undo matrix"]
    try! JSONSerialization.data(withJSONObject: result, options: [.prettyPrinted, .sortedKeys]).write(to: output.appendingPathComponent("continuation-check.json"))
    FileHandle.standardError.write(Data("PASS light confirmation: Core owns queued work and dispatch; no frontend model lifecycle\n".utf8)); exit(0)
}
