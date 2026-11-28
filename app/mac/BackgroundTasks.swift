import SwiftUI
import VeluneBindings

extension ConversationStore {
    /// The tool is available even before a runtime has reported its first task.
    var canShowBackgroundTasks: Bool {
        guard pendingConversationID == nil else { return false }
        let kind = device.runtimeInstances.first { $0.id == projectionRuntimeID }?.typeId
        return !isUnsentConversation && (kind == .pi || kind == .claudeCode || kind == .codex166)
    }
}

extension BackgroundTaskStatus {
    var isTerminal: Bool {
        switch self {
        case .completed, .failed, .cancelled: true
        case .running, .stopping, .unknown: false
        }
    }

    var label: String {
        switch self {
        case .running: "运行中"
        case .stopping: "正在停止"
        case .completed: "已完成"
        case .failed: "失败"
        case .cancelled: "已停止"
        case .unknown: "状态未知"
        }
    }
}
extension BackgroundTaskInfo {
    var kindLabel: String {
        switch kind {
        case "shell", "local_bash", "run": "Shell"
        case "delegate": "委派任务"
        case "fusion": "汇总任务"
        case "local_agent", "agent-shell", "managed", "attested": "Agent 任务"
        case "local_workflow": "工作流"
        default: "后台任务"
        }
    }
}

private struct BackgroundTaskIdentity: Hashable {
    let id: String
    let kind: String
    let title: String
    let child: String?

    init(_ task: BackgroundTaskInfo) {
        id = task.id
        kind = task.kind
        title = task.title
        child = task.childSessionId
    }
}

/// Clearing a row affects the Mac tool across windows for this device while
/// Velune is open. Native task metadata and output remain available to reads.
@MainActor final class BackgroundTaskDismissals: ObservableObject {
    private struct Entry: Equatable {
        let identity: BackgroundTaskIdentity
        let status: BackgroundTaskStatus
    }
    @Published private var byConversation: [String: [String: Entry]] = [:]

    func contains(_ task: BackgroundTaskInfo, in scope: String) -> Bool {
        task.status.isTerminal && clearedStatus(of: task, in: scope) != nil
    }

    func clearedStatus(of task: BackgroundTaskInfo, in scope: String) -> BackgroundTaskStatus? {
        guard let entry = byConversation[scope]?[task.id], entry.identity == BackgroundTaskIdentity(task) else { return nil }
        return entry.status
    }

    func clearFinished(_ tasks: [BackgroundTaskInfo], in scope: String) {
        var entries = byConversation[scope] ?? [:]
        for task in tasks where task.status.isTerminal {
            entries[task.id] = Entry(identity: BackgroundTaskIdentity(task), status: task.status)
        }
        if !entries.isEmpty { byConversation[scope] = entries }
    }

    func restore(in scope: String) { byConversation.removeValue(forKey: scope) }

    func observe(_ tasks: [BackgroundTaskInfo], in scope: String) {
        guard let previous = byConversation[scope] else { return }
        var entries = previous
        for task in tasks {
            guard let known = entries[task.id] else { continue }
            if task.status == .running || task.status == .stopping || known.identity != BackgroundTaskIdentity(task) {
                entries.removeValue(forKey: task.id)
            }
        }
        if entries != previous { byConversation[scope] = entries.isEmpty ? nil : entries }
    }
}

/// Owned by the visible tool, never by the transcript or composer polling loop.
@MainActor final class BackgroundTasksModel: ObservableObject {
    let dismissals: BackgroundTaskDismissals
    @Published var tasks: [BackgroundTaskInfo] = []
    @Published var issue: String?
    @Published private(set) var live: Bool?
    @Published var loading = false
    @Published private(set) var refreshing = false
    @Published var output: BackgroundTaskOutput?
    @Published var outputIssue: String?
    @Published private(set) var readingOutput = false
    @Published var stopping: Set<String> = []
    @Published var finishedExpanded = false
    private var key = ""
    private var generation: UInt64 = 0
    private var outputGeneration: UInt64 = 0
    private var pendingList = false
    private var pendingManualRefresh = false
    private var pendingOutput = false
    private var outputTask: String?
    private var outputRequestID: String?
    private weak var source: ConversationStore?

    init(dismissals: BackgroundTaskDismissals? = nil) {
        self.dismissals = dismissals ?? BackgroundTaskDismissals()
    }

    func reset(_ store: ConversationStore) {
        generation &+= 1; outputGeneration &+= 1
        source = store; key = identity(store)
        tasks = []; issue = nil; live = nil; loading = false; output = nil; outputIssue = nil; outputTask = nil
        readingOutput = false; outputRequestID = nil
        pendingList = false; pendingManualRefresh = false; refreshing = false; pendingOutput = false; stopping = []
        finishedExpanded = false
    }
    var listedTasks: [BackgroundTaskInfo] { tasks.filter { !dismissals.contains($0, in: key) } }
    var hasClearedTasks: Bool { tasks.contains { dismissals.contains($0, in: key) } }

    func clearFinished() {
        dismissals.clearFinished(tasks, in: key)
        finishedExpanded = false
    }

    func restoreFinished() {
        dismissals.restore(in: key)
        finishedExpanded = true
    }

    func apply(_ value: BackgroundTaskList) {
        dismissals.observe(value.tasks, in: key)
        if value.live {
            tasks = value.tasks
        } else {
            markConnectionUnavailable(reported: value.tasks)
        }
        issue = value.issue
        live = value.live
    }

    private func markConnectionUnavailable(reported: [BackgroundTaskInfo] = []) {
        let terminals = Dictionary(tasks.filter { $0.status.isTerminal }.map { (BackgroundTaskIdentity($0), $0.status) },
                                   uniquingKeysWith: { _, latest in latest })
        tasks = (reported.isEmpty ? tasks : reported).map {
            var task = $0
            if task.status == .unknown {
                task.status = terminals[BackgroundTaskIdentity(task)] ?? dismissals.clearedStatus(of: task, in: key) ?? .unknown
            } else if !task.status.isTerminal {
                task.status = .unknown
            }
            task.canStop = false
            task.canReadOutput = false
            return task
        }
    }

    private func identity(_ store: ConversationStore) -> String { (store.projectionRuntimeID ?? "") + "\n" + (store.loadedConversationID ?? "") }
    private func target() -> (Transport, String, String)? {
        guard let source, source.canShowBackgroundTasks, identity(source) == key, let transport = source.device.transport,
              let runtime = source.projectionRuntimeID, let conversation = source.loadedConversationID else { return nil }
        return (transport, runtime, conversation)
    }
    private func reportUnavailableConnection() {
        markConnectionUnavailable()
        live = false
        issue = "暂时无法连接执行设备。连接恢复后可重试。"
    }
    func reload(manual: Bool = false) {
        if pendingList {
            if manual && !refreshing {
                pendingManualRefresh = true
                refreshing = true
            }
            return
        }
        guard let (transport, runtime, conversation) = target() else {
            if manual { reportUnavailableConnection() }
            return
        }
        pendingList = true; refreshing = manual; loading = live == nil && issue == nil && !manual
        let captured = key, epoch = generation
        DispatchQueue.global(qos: .utility).async {
            let result = Result { try transport.backgroundTasks(runtimeID: runtime, conversationID: conversation, manual: manual) }
            DispatchQueue.main.async { [weak self] in
                guard let self, key == captured, generation == epoch else { return }
                let retryManually = pendingManualRefresh
                pendingManualRefresh = false
                pendingList = false; loading = false; refreshing = false
                guard let source, identity(source) == captured else { return }
                switch result {
                case .success(let value): apply(value)
                case .failure(let error):
                    markConnectionUnavailable()
                    live = false
                    issue = error.localizedDescription
                }
                if retryManually { reload(manual: true) }
            }
        }
    }
    func stop(_ task: BackgroundTaskInfo) {
        guard task.canStop, !stopping.contains(task.id), let (transport, runtime, conversation) = target() else { return }
        stopping.insert(task.id)
        let captured = key, epoch = generation
        DispatchQueue.global(qos: .userInitiated).async {
            let result = Result { try transport.stopBackgroundTask(runtimeID: runtime, conversationID: conversation, taskID: task.id) }
            DispatchQueue.main.async { [weak self] in
                guard let self, target() != nil, key == captured, generation == epoch else { return }
                stopping.remove(task.id)
                switch result {
                case .success(let value): apply(value)
                case .failure(let error): issue = error.localizedDescription
                }
            }
        }
    }
    func readOutput(_ task: BackgroundTaskInfo, refresh: Bool = false) {
        guard let (transport, runtime, conversation) = target() else {
            if refresh && source != nil { reportUnavailableConnection() }
            return
        }
        guard task.canReadOutput else { return }
        if refresh || outputTask != task.id {
            outputGeneration &+= 1
            if output?.taskId != task.id { output = nil }
            if outputTask != task.id { outputIssue = nil }
            outputTask = task.id; outputRequestID = nil; readingOutput = true; pendingOutput = false
        }
        guard readingOutput, !pendingOutput else { return }
        pendingOutput = true
        let captured = key, epoch = generation, outputEpoch = outputGeneration, request = outputRequestID
        DispatchQueue.global(qos: .utility).async {
            let result = Result { try transport.backgroundTaskOutput(runtimeID: runtime, conversationID: conversation, taskID: task.id, requestID: request) }
            DispatchQueue.main.async { [weak self] in
                guard let self, key == captured, generation == epoch, outputGeneration == outputEpoch, outputTask == task.id else { return }
                pendingOutput = false
                guard let source, identity(source) == captured else { return }
                switch result {
                case .success(let value):
                    outputRequestID = value.requestId
                    readingOutput = value.status == .checking
                    if value.status == .ready { output = value }
                    if !readingOutput { outputIssue = value.status == .failed ? (value.issue ?? "运行时未能返回任务输出。") : nil }
                case .failure(let error):
                    readingOutput = false
                    outputIssue = error.localizedDescription
                }
            }
        }
    }
    func stopReadingOutput() {
        outputGeneration &+= 1
        readingOutput = false
        pendingOutput = false
    }
    func outputIssue(for taskID: String) -> String? {
        outputTask == taskID || output?.taskId == taskID ? outputIssue : nil
    }
}

struct BackgroundTaskRow: View {
    let task: BackgroundTaskInfo
    var body: some View {
        VStack(alignment: .leading, spacing: 3) {
            Text(task.title).lineLimit(2).help(task.title)
            HStack(spacing: 8) {
                Text(task.kindLabel)
                Text(task.status.label)
                    .foregroundStyle(task.status == .failed ? Color.red : Color.secondary)
            }.uiFont(.caption).foregroundStyle(.secondary)
        }.padding(.vertical, 2)
    }
}

/// An independent tool. Opening linked agents never starts task polling.
struct BackgroundTasksView: View {
    @ObservedObject var store: ConversationStore
    var inPopover = true
    var inSidebar = false
    var layout: ConversationWindowModel?
    let open: (String, LinkedConversationOpening?) -> Void
    var body: some View {
        Group {
            if store.canShowBackgroundTasks {
                BackgroundTasksSessionView(store: store, inPopover: inPopover, inSidebar: inSidebar, layout: layout, open: open)
                    .id((store.projectionRuntimeID ?? "") + "\n" + (store.loadedConversationID ?? ""))
            } else if store.pendingConversationID != nil {
                ProgressView("正在打开会话…").controlSize(.small)
                    .frame(maxWidth: .infinity, maxHeight: .infinity)
            } else {
                ContentUnavailableView("请选择支持后台任务的会话", systemImage: "terminal")
                    .frame(maxWidth: .infinity, maxHeight: .infinity)
            }
        }.frame(width: inPopover ? 360 : nil, height: inPopover ? 400 : nil)
    }
}

/// Changing or leaving the observed conversation replaces this owner and its
/// selection immediately, before an async load can act on the previous task.
private struct BackgroundTasksSessionView: View {
    @ObservedObject var store: ConversationStore
    let inPopover: Bool
    let inSidebar: Bool
    let layout: ConversationWindowModel?
    let open: (String, LinkedConversationOpening?) -> Void
    @StateObject private var model: BackgroundTasksModel

    init(store: ConversationStore, inPopover: Bool, inSidebar: Bool, layout: ConversationWindowModel?,
         open: @escaping (String, LinkedConversationOpening?) -> Void) {
        self.store = store
        self.inPopover = inPopover
        self.inSidebar = inSidebar
        self.layout = layout
        self.open = open
        _model = StateObject(wrappedValue: BackgroundTasksModel(dismissals: store.device.backgroundTaskDismissals))
    }

    var body: some View {
        BackgroundTasksContent(model: model, inPopover: inPopover, inSidebar: inSidebar, layout: layout, open: open)
            .task {
                model.reset(store)
                while !Task.isCancelled {
                    model.reload()
                    do { try await Task.sleep(nanoseconds: 2_000_000_000) } catch { return }
                }
            }
    }
}

struct BackgroundTasksContent: View {
    @ObservedObject var model: BackgroundTasksModel
    @ObservedObject private var dismissals: BackgroundTaskDismissals
    var inPopover = true
    var inSidebar = false
    var layout: ConversationWindowModel?
    let open: (String, LinkedConversationOpening?) -> Void
    @State private var selectedID: String?
    @State private var detailTask: BackgroundTaskInfo?

    init(model: BackgroundTasksModel, inPopover: Bool = true, inSidebar: Bool = false,
         layout: ConversationWindowModel? = nil, open: @escaping (String, LinkedConversationOpening?) -> Void) {
        self.model = model
        self.dismissals = model.dismissals
        self.inPopover = inPopover
        self.inSidebar = inSidebar
        self.layout = layout
        self.open = open
    }

    var body: some View {
        let listed = model.listedTasks
        let active = listed.filter { !$0.status.isTerminal }
        let finished = listed.filter { $0.status.isTerminal }
        let visible = model.finishedExpanded ? active + finished : active
        let selected = visible.first { $0.id == selectedID }
        VStack(spacing: 0) {
            if let detailTask {
                BackgroundTaskDetail(initial: detailTask, model: model, back: { self.detailTask = nil }, open: open)
            } else {
                HStack(spacing: 8) {
                    Label("后台任务", systemImage: "terminal").uiFont(.callout.weight(.semibold))
                    Spacer()
                    if model.refreshing { ProgressView().controlSize(.small).accessibilityLabel("正在刷新后台任务") }
                    ReadRefreshButton(label: "刷新后台任务") { model.reload(manual: true) }
                        .disabled(model.refreshing)
                        .accessibilityIdentifier("background-tasks-refresh")
                }.frame(minHeight: 25).padding(.horizontal, 12).padding(.vertical, 8)
                Divider()
                if let issue = model.issue {
                    ReadFailureNotice(title: model.live == true ? "后台任务操作失败" : "无法读取后台任务",
                                      detail: issue, retryDisabled: model.refreshing,
                                      retryLabel: model.live == true ? "刷新状态" : "重试") { model.reload(manual: true) }
                    Divider()
                } else if model.live == false {
                    ReadFailureNotice(title: "暂无实时任务状态",
                                      detail: "历史任务通知仍可在会话消息中查看。", icon: "bolt.slash")
                    Divider()
                }
                if listed.isEmpty && model.hasClearedTasks {
                    Text("已清理已结束的任务").uiFont(.callout).foregroundStyle(.secondary)
                        .frame(maxWidth: .infinity, maxHeight: .infinity)
                } else if listed.isEmpty {
                    if model.loading {
                        ProgressView("正在读取后台任务…").controlSize(.small)
                            .frame(maxWidth: .infinity, maxHeight: .infinity)
                    } else if model.issue == nil && model.live != false {
                        Text("没有后台任务").uiFont(.callout).foregroundStyle(.secondary)
                            .frame(maxWidth: .infinity, maxHeight: .infinity)
                    } else {
                        Spacer(minLength: 0)
                    }
                } else {
                    if inSidebar { list(active, finished).listStyle(.sidebar) }
                    else { list(active, finished).listStyle(.inset) }
                }
                if !listed.isEmpty || model.hasClearedTasks {
                    Divider()
                    HStack {
                        if !listed.isEmpty {
                            Text("\(listed.count) 个任务").uiFont(.caption).foregroundStyle(.secondary)
                        }
                        Spacer()
                        if model.hasClearedTasks { Button("恢复显示", action: model.restoreFinished) }
                        if let selected {
                            Button("查看任务") { detailTask = selected }
                                .keyboardShortcut(inPopover ? .defaultAction : nil)
                        }
                    }.padding(10)
                    .frame(minHeight: 39)
                }
            }
        }
        .frame(width: inPopover ? 360 : nil, height: inPopover ? 400 : nil)
        .onChange(of: visible.map(\.id)) { _, ids in
            if let selectedID, !ids.contains(selectedID) { self.selectedID = nil }
        }
    }

    private func list(_ active: [BackgroundTaskInfo], _ finished: [BackgroundTaskInfo]) -> some View {
        let visible = model.finishedExpanded ? active + finished : active
        return List(selection: $selectedID) {
            taskRows(active)
            if !finished.isEmpty {
                HStack(spacing: 8) {
                    Button { model.finishedExpanded.toggle() } label: {
                        HStack(spacing: 6) {
                            Image(systemName: model.finishedExpanded ? "chevron.down" : "chevron.right")
                                .font(.caption.weight(.semibold)).frame(width: 14)
                            Text("已结束（\(finished.count)）").uiFont(.callout)
                            Spacer(minLength: 0)
                        }
                        .foregroundStyle(.secondary)
                        .contentShape(Rectangle())
                    }
                    .buttonStyle(.plain)
                    .accessibilityLabel("已结束的后台任务，\(finished.count) 个")
                    .accessibilityValue(model.finishedExpanded ? "已展开" : "已折叠")
                    if model.finishedExpanded {
                        Button("清理已结束", action: model.clearFinished)
                            .buttonStyle(.borderless)
                            .help("从此列表移除已结束的任务，可恢复显示")
                    }
                }.selectionDisabled(true)
                if model.finishedExpanded { taskRows(finished) }
            }
        }
        .accessibilityLabel("后台任务")
        .contextMenu(forSelectionType: String.self) { ids in
            if let task = visible.first(where: { ids.contains($0.id) }) {
                Button("查看任务") { detailTask = task }
                if let child = task.conversationId { Button("打开子会话") { open(child, nil) } }
                if task.canStop { Button("停止任务") { model.stop(task) }.disabled(model.stopping.contains(task.id)) }
            } else if let layout {
                ToolPlaceSection(window: layout, kind: .backgroundTasks)
            }
        } primaryAction: { ids in
            detailTask = visible.first { ids.contains($0.id) }
        }
    }

    private func taskRows(_ tasks: [BackgroundTaskInfo]) -> some View {
        ForEach(tasks, id: \.id) { task in BackgroundTaskRow(task: task).tag(task.id) }
    }
}

struct BackgroundTaskDetail: View {
    let initial: BackgroundTaskInfo
    @ObservedObject var model: BackgroundTasksModel
    let back: () -> Void
    let open: (String, LinkedConversationOpening?) -> Void
    @State private var outputRefresh: UInt64 = 0
    private var task: BackgroundTaskInfo {
        if let current = model.tasks.first(where: { $0.id == initial.id }) { return current }
        var last = initial
        if !last.status.isTerminal { last.status = .unknown }
        last.canStop = false
        last.canReadOutput = false
        return last
    }
    private var output: BackgroundTaskOutput? { model.output?.taskId == task.id ? model.output : nil }
    var body: some View {
        let outputIssue = model.outputIssue(for: task.id)
        let refreshesOutput = task.canReadOutput && (model.issue == nil || outputIssue != nil)
        VStack(alignment: .leading, spacing: 0) {
            HStack {
                Button("返回列表", systemImage: "chevron.backward", action: back)
                    .labelStyle(.iconOnly).buttonStyle(.plain).help("返回列表")
                Spacer()
                if model.refreshing || model.readingOutput { ProgressView().controlSize(.small).accessibilityLabel("正在刷新任务") }
                ReadRefreshButton(label: refreshesOutput ? "刷新任务输出" : "刷新任务状态") {
                    if refreshesOutput { outputRefresh &+= 1 } else { model.reload(manual: true) }
                }
                .disabled(refreshesOutput ? model.readingOutput : model.refreshing)
                .accessibilityIdentifier("background-task-detail-refresh")
                if task.canStop {
                    Button("停止任务") { model.stop(task) }.controlSize(.small)
                        .disabled(model.stopping.contains(task.id))
                }
            }.frame(minHeight: 22).padding(12)
            VStack(alignment: .leading, spacing: 5) {
                Text(task.title).uiFont(.headline).lineLimit(3).help(task.title)
                HStack(spacing: 8) {
                    Text(task.kindLabel)
                    Text(task.status.label).foregroundStyle(task.status == .failed ? Color.red : Color.secondary)
                }
                    .uiFont(.callout).foregroundStyle(.secondary)
                if let child = task.conversationId { Button("打开子会话") { open(child, nil) } }
            }.padding(.horizontal, 12).padding(.bottom, 12)
            Divider()
            if let issue = outputIssue, refreshesOutput {
                ReadFailureNotice(title: "无法读取输出", detail: issue, retryDisabled: model.readingOutput) { outputRefresh &+= 1 }
                Divider()
            } else if let issue = model.issue {
                ReadFailureNotice(title: "任务状态暂不可用", detail: issue, retryDisabled: model.refreshing) { model.reload(manual: true) }
                Divider()
            }
            ScrollView {
                VStack(alignment: .leading, spacing: 8) {
                    if let output, output.status == .ready {
                        Text(output.text.isEmpty ? "暂时没有输出。" : output.text)
                            .font(.system(.body, design: .monospaced)).multilineTextAlignment(.leading)
                    } else if outputIssue == nil {
                        if task.canReadOutput { ProgressView("正在读取输出…").controlSize(.small) }
                        else if model.issue == nil {
                            Text(task.status == .unknown ? "历史回执不能确认任务仍在运行，当前没有可读取的输出。" : "运行时尚未提供此任务的输出。")
                                .foregroundStyle(.secondary)
                        }
                    }
                }.textSelection(.enabled).frame(maxWidth: .infinity, alignment: .leading).padding(12)
            }.frame(maxWidth: .infinity, maxHeight: .infinity)
            if output?.truncated == true {
                Divider()
                Text("输出已截取").uiFont(.caption).foregroundStyle(.secondary).padding(10)
                    .help("仅显示有界输出，可在原生工具中查看完整内容。")
            }
        }
        .task(id: [task.id, String(task.canReadOutput), String(outputRefresh)]) {
            guard task.canReadOutput else { model.stopReadingOutput(); return }
            model.readOutput(task, refresh: true)
            while !Task.isCancelled {
                do { try await Task.sleep(nanoseconds: 1_000_000_000) } catch { return }
                if !model.readingOutput { break }
                model.readOutput(task)
            }
        }
        .onDisappear { model.stopReadingOutput() }
    }
}
