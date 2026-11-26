import AppKit
import Combine
import SwiftUI
import VeluneBindings

// MARK: - Menu item

struct ScheduledTasksMenuItem: View {
    @Environment(\.openWindow) private var openWindow
    var body: some View { Button("定时任务…") { openWindow(id: "scheduled-tasks") } }
}

// MARK: - Per-device model

@MainActor final class ScheduledTasksModel: ObservableObject {
    let store: DeviceStore
    @Published var state: ScheduledTasksState?
    @Published var busy = false
    @Published var refreshing = false
    @Published var error: String?
    @Published var refreshError: String?
    private var queuedRefresh = false
    init(store: DeviceStore) { self.store = store }
    func refresh(manual: Bool = false) {
        if busy || refreshing {
            if manual { queuedRefresh = true }
            return
        }
        guard !store.isPreview else { return }
        guard let transport = store.transport else {
            refreshError = "暂时无法连接设备。"
            return
        }
        refreshing = true
        store.performOnQueue { [weak self] in
            let result = Result { try transport.scheduledTasks(.list) }
            DispatchQueue.main.async {
                guard let self else { return }
                self.refreshing = false
                switch result {
                case .success(let state): self.state = state; self.refreshError = nil
                case .failure(let error): self.refreshError = AppProblem.failure(error, source: "定时任务").detail
                }
                self.runQueuedRefresh()
            }
        }
    }
    func request(_ operation: ScheduledTasksOperation, completed: ((ScheduledTasksState) -> Void)? = nil) {
        guard !busy, let transport = store.transport else { return }
        busy = true
        store.performOnQueue { [weak self] in
            let result = Result { try transport.scheduledTasks(operation) }
            DispatchQueue.main.async {
                guard let self else { return }
                self.busy = false
                switch result {
                case .success(let state):
                    if case .previewImport = operation {} else { self.state = state }
                    self.error = nil; completed?(state)
                case .failure(let error): self.error = AppProblem.failure(error, source: "定时任务").detail
                }
                self.runQueuedRefresh()
            }
        }
    }
    private func runQueuedRefresh() {
        guard queuedRefresh, !busy, !refreshing else { return }
        queuedRefresh = false
        refresh(manual: true)
    }
}

// MARK: - Multi-device wrapper

struct ScheduledTasksView: View {
    let local: DeviceStore
    let hub: DeviceHub?
    @StateObject private var hubModel: DeviceWindowSelectionModel
    init(local: DeviceStore, hub: DeviceHub?) {
        self.local = local
        self.hub = hub
        _hubModel = StateObject(wrappedValue: DeviceWindowSelectionModel(local: local, hub: hub))
    }
    var body: some View {
        ScheduledTasksContent(store: hubModel.activeStore)
            .id(hubModel.selectedPeerID ?? "local")
            .toolbar {
                if !hubModel.peerDevices.isEmpty {
                    ToolbarItem(placement: .navigation) {
                        Picker("设备", selection: $hubModel.selectedPeerID) {
                            Text(hubModel.localDeviceName).tag(nil as String?)
                            ForEach(hubModel.peerDevices, id: \.id) { device in
                                Text(device.name).tag(device.id as String?)
                            }
                        }
                        .pickerStyle(.menu)
                        .labelsHidden()
                        .help("切换设备")
                    }
                }
            }
    }
}

// MARK: - Per-device content

struct ScheduledTasksContent: View {
    @ObservedObject var store: DeviceStore
    @StateObject private var model: ScheduledTasksModel
    @State private var selected: String?
    @State private var editGeneration = UUID()
    @State private var draft: ScheduledTask?
    @State private var original: ScheduledTask?
    @State private var pendingSelection: String?
    @State private var pendingNew = false
    @State private var discarding = false
    @State private var importing = false
    @State private var confirming = false
    @State private var deleting = false
    @State private var scheduleReady = false
    @State private var notice: String?
    @Environment(\.openWindow) private var openWindow
    private var saved: ScheduledTask? { model.state?.tasks.first { $0.id == selected } }
    private var dirty: Bool { draft != nil && draft != original }
    init(store: DeviceStore) {
        self.store = store
        _model = StateObject(wrappedValue: ScheduledTasksModel(store: store))
    }
    var body: some View {
        HSplitView {
            VStack(spacing: 0) {
                if let error = model.refreshError {
                    ReadFailureNotice(title: "无法读取定时任务", detail: error) { model.refresh(manual: true) }
                    Divider()
                }
                sidebar.disabled(model.busy)
            }.frame(minWidth: 210, idealWidth: 240, maxWidth: 300)
            detail.disabled(model.busy)
        }
        .onAppear {
            store.openConversationWindow = { seed in WorkspaceWindows.shared.open(.workspace, seed: seed) }
        }
        .task { store.start(); while !Task.isCancelled { model.refresh(); try? await Task.sleep(for: .seconds(5)) } }
        .toolbar {
            ToolbarItem(placement: .primaryAction) {
                Button("新建", systemImage: "plus") { requestNew() }.help("新建定时任务")
            }
            ToolbarItem {
                Button("导入", systemImage: "square.and.arrow.down") { importing = true }
                    .help("从 ChatGPT / Codex 导入")
            }
            ToolbarItem {
                HStack(spacing: 6) {
                    if model.refreshing { ProgressView().controlSize(.small).accessibilityLabel("正在刷新定时任务") }
                    ReadRefreshButton(label: "刷新任务与运行记录") { model.refresh(manual: true) }
                }
            }
        }
        .sheet(isPresented: $importing) {
            ScheduledTaskImport(model: model, store: store) { id in requestSelection(id) }
        }
        .confirmationDialog("丢弃尚未保存的修改？", isPresented: $discarding) {
            Button("丢弃修改并继续", role: .destructive) {
                if pendingNew { create() } else { select(pendingSelection) }; pendingNew = false
            }
            Button("取消", role: .cancel) { pendingNew = false }
        }
        .confirmationDialog("确认原宿主停止派发", isPresented: $confirming) {
            Button("已在 ChatGPT 中暂停，并已处理在途运行") {
                guard let saved else { return }
                model.request(.confirmHandover(id: saved.id, revision: saved.revision)) { _ in
                    notice = "已确认交接；任务仍暂停，核对配置后可启用。"
                }
            }
        } message: {
            Text("只修改 automation.toml 不能证明宿主已停止派发。请在原应用中暂停任务，并确认旧运行已结束或由你处理。确认后仍保持暂停。来源重新启用时，Velune 会再次阻止派发。")
        }
        .confirmationDialog("删除此任务？", isPresented: $deleting) {
            Button("删除任务", role: .destructive) {
                guard let saved else { return }
                model.request(.delete(id: saved.id, revision: saved.revision)) { _ in select(nil) }
            }
        } message: { Text("保留运行记录和已经创建的会话，不取消正在执行的运行。") }
    }

    // MARK: Sidebar

    private var sidebar: some View {
        List(selection: Binding(get: { selected }, set: requestSelection)) {
            ForEach(model.state?.tasks ?? [], id: \.id) { task in
                HStack(spacing: 10) {
                    Image(systemName: task.status.iconName)
                        .foregroundStyle(task.status.tintColor)
                        .frame(width: 14, alignment: .center)
                    VStack(alignment: .leading, spacing: 2) {
                        Text(task.name).lineLimit(1)
                        Text(task.listDetail).uiFont(.caption).foregroundStyle(.secondary).lineLimit(1)
                    }
                }
                .padding(.vertical, 1)
                .tag(task.id)
                .contextMenu {
                    Button("编辑…") { requestSelection(task.id) }
                    if !dirty {
                        Button(task.status == .active ? "暂停" : "启用") { setEnabled(task, task.status != .active) }
                    }
                    Divider()
                    Button("删除…", role: .destructive) { requestSelection(task.id); if !dirty { deleting = true } }
                }
            }
        }
        .listStyle(.sidebar)
    }

    // MARK: Detail container

    private var detail: some View {
        VStack(spacing: 0) {
            if let error = model.error {
                HStack(alignment: .top, spacing: 10) {
                    Label(error, systemImage: "exclamationmark.circle")
                        .foregroundStyle(.red).textSelection(.enabled).font(.callout)
                    Spacer()
                    Button { model.error = nil } label: {
                        Image(systemName: "xmark")
                    }.buttonStyle(.borderless)
                }
                .padding(14)
                .background(Color.red.opacity(0.07))
                Divider()
            }
            if let draft {
                editor(draftBinding(draft))
            } else if let saved {
                overview(saved)
            } else {
                ContentUnavailableView("定时任务", systemImage: "clock",
                    description: Text("安排 Agent 定期处理工作，或接管已有的 ChatGPT 本地任务。\nVelune 运行期间调度；退出期间错过的触发不补跑。"))
                    .frame(maxWidth: .infinity, maxHeight: .infinity)
            }
        }
        .frame(minWidth: 560)
    }

    // MARK: Overview

    private func overview(_ task: ScheduledTask) -> some View {
        ScrollView {
            VStack(alignment: .leading, spacing: 0) {
                // Header: name, status badge, actions
                VStack(alignment: .leading, spacing: 12) {
                    HStack(alignment: .firstTextBaseline) {
                        Text(task.name).uiFont(.title2).fontWeight(.semibold)
                        Spacer()
                        statusPill(task.status)
                        Menu { Button("删除任务…", role: .destructive) { deleting = true } }
                            label: { Image(systemName: "ellipsis") }
                            .menuStyle(.borderlessButton).fixedSize().help("更多操作")
                    }
                    HStack(spacing: 8) {
                        if task.status == .active {
                            Button("暂停") { setEnabled(task, false) }.disabled(model.busy)
                        } else {
                            Button("启用") { setEnabled(task, true) }
                                .disabled(model.busy || availability(task)?.enableIssue != nil)
                        }
                        Button("运行一次") {
                            model.request(.runNow(id: task.id, revision: task.revision)) { _ in
                                notice = "已加入执行队列；可在下方查看运行进度。"
                            }
                        }.disabled(model.busy || availability(task)?.runIssue != nil)
                        Button("编辑…") { editGeneration = UUID(); draft = task; original = task; notice = nil }
                            .disabled(model.busy)
                        if model.busy { ProgressView().controlSize(.small) }
                    }
                    if let issue = availability(task)?.enableIssue ?? task.issue {
                        Label(issue, systemImage: "exclamationmark.circle").foregroundStyle(.secondary).textSelection(.enabled)
                    }
                    if let issue = availability(task)?.runIssue, issue != availability(task)?.enableIssue {
                        Text(issue).foregroundStyle(.secondary)
                    }
                    if let notice { Label(notice, systemImage: "checkmark.circle").foregroundStyle(.secondary) }
                }
                .padding(20)
                Divider()
                // Schedule
                VStack(alignment: .leading, spacing: 10) {
                    sectionHeader("日程")
                    ScheduledScheduleSummary(task: task, store: store).id("\(task.id):\(task.revision)")
                }
                .padding(20)
                Divider()
                VStack(alignment: .leading, spacing: 8) {
                    sectionHeader("任务指令")
                    Text(task.prompt)
                        .multilineTextAlignment(.leading)
                        .textSelection(.enabled)
                        .frame(maxWidth: .infinity, alignment: .leading)
                }
                .padding(20)
                Divider()
                // Execution config
                VStack(alignment: .leading, spacing: 8) {
                    sectionHeader("执行配置")
                    Text(store.runtimeInstances.first { $0.id == task.target.runtimeID }?.name ?? "运行时不可用")
                    switch task.target {
                    case .newConversation(_, let cwds, let worktree):
                        Text(cwds.isEmpty ? "每次新建会话，使用运行时默认目录"
                            : "每个目录分别创建会话：\n" + cwds.joined(separator: "\n"))
                            .textSelection(.enabled)
                        if worktree { Text("每次使用独立 Git 工作树；产物保留。").foregroundStyle(.secondary) }
                    case .existingConversation(_, let id):
                        HStack {
                            Text("继续会话：" + (store.conversations.first { $0.id == id }?.title ?? "已选择的会话"))
                            Button("打开") { openConversation(id) }.buttonStyle(.borderless)
                        }
                    }
                    Text("模型：\(task.model.map { scheduleModelName($0, store: store) } ?? "执行运行时的默认模型")")
                        .textSelection(.enabled)
                    Text("推理：\(task.effort ?? "默认")；权限：\(permissionLabel(task))；\(task.notifyOnlyFailures ? "仅失败时通知" : "完成与失败时通知")")
                        .uiFont(.callout).foregroundStyle(.secondary)
                }
                .padding(20)
                // Handover progress
                if let source = task.source {
                    Divider()
                    GroupBox("接管进度") {
                        VStack(alignment: .leading, spacing: 10) {
                            Text(source.handoverConfirmed
                                ? "已确认原宿主停用。启用后仍持续检查来源。"
                                : "1. 编辑并核对 Velune 的执行配置\n2. 在 ChatGPT 中暂停原任务，处理已经启动的运行\n3. 确认交接，再启用 Velune 任务")
                            if !source.handoverConfirmed {
                                Button("确认原宿主已暂停…") { confirming = true }.disabled(model.busy)
                            }
                            Text("来源模型：\(source.originalModel ?? "原宿主默认") → Velune：\(task.model.map { scheduleModelName($0, store: store) } ?? "执行运行时的默认模型（未沿用来源模型）")").uiFont(.callout)
                            Text("来源环境脚本与自动归档不会自动执行，请另行配置。").uiFont(.callout).foregroundStyle(.secondary)
                            DisclosureGroup("来源与诊断详情") {
                                VStack(alignment: .leading) {
                                    Text(source.path)
                                    Text(source.originalMetadata).uiFont(.caption.monospaced())
                                }.textSelection(.enabled)
                            }
                        }
                        .frame(maxWidth: .infinity, alignment: .leading).padding(6)
                    }
                    .padding(20)
                }
                Divider()
                history(task).padding(20)
                Text("Velune 退出后暂停调度，重开后从未来的触发继续。暂停计划不会停止已启动的运行；运行中的审批请在会话中处理。")
                    .uiFont(.caption).foregroundStyle(.secondary)
                    .padding(.horizontal, 20).padding(.bottom, 20)
            }
            .frame(maxWidth: .infinity, alignment: .leading)
        }
    }

    // MARK: Helper views

    private func sectionHeader(_ title: String) -> some View {
        Text(title).uiFont(.headline)
    }
    private func statusPill(_ status: ScheduledTaskStatus) -> some View {
        Text(status.title)
            .uiFont(.caption).fontWeight(.medium)
            .padding(.horizontal, 8).padding(.vertical, 3)
            .background(status.tintColor.opacity(0.12), in: Capsule())
            .foregroundStyle(status.tintColor)
    }

    // MARK: Editor

    private func draftBinding(_ fallback: ScheduledTask) -> Binding<ScheduledTask> {
        let generation = editGeneration
        return Binding(
            get: { editGeneration == generation ? (draft ?? fallback) : fallback },
            set: { value in guard editGeneration == generation, draft != nil else { return }; draft = value }
        )
    }
    private func editor(_ binding: Binding<ScheduledTask>) -> some View {
        VStack(spacing: 0) {
            HStack {
                Text(binding.wrappedValue.id.isEmpty ? "新建任务" : "编辑任务").uiFont(.title2)
                Spacer()
                if let saved, saved.status == .active {
                    Button("暂停当前计划") { setEnabled(saved, false) }.disabled(model.busy)
                }
            }.padding(20)
            if let saved, saved.revision != original?.revision {
                HStack {
                    Text("任务已更新；草稿已保留。请载入最新定义后再编辑。")
                    Button("载入最新定义…") { requestSelection(saved.id) }
                }.uiFont(.callout).padding(.horizontal, 20)
            }
            if saved?.status == .active {
                Text("保存前仍按原计划运行。").foregroundStyle(.secondary).padding(.horizontal, 20)
            }
            ScheduledTaskEditor(task: binding, store: store, scheduleReady: $scheduleReady)
                .id("\(binding.wrappedValue.id):\(binding.wrappedValue.revision)")
            Divider()
            HStack {
                Button("取消") { draft = nil; original = nil; model.error = nil }.keyboardShortcut(.cancelAction)
                Spacer()
                if model.busy { ProgressView().controlSize(.small) }
                Button(binding.wrappedValue.id.isEmpty ? "创建任务" : "保存修改") { save() }
                    .keyboardShortcut("s").disabled(model.busy || !scheduleReady)
            }.padding(16)
        }
    }

    // MARK: Run history

    private func history(_ task: ScheduledTask) -> some View {
        VStack(alignment: .leading, spacing: 14) {
            sectionHeader("最近运行")
            let runs = (model.state?.runs ?? []).filter { $0.taskId == task.id }
            if runs.isEmpty { Text("尚未运行").foregroundStyle(.secondary) }
            ForEach(runs, id: \.id) { run in
                VStack(alignment: .leading, spacing: 5) {
                    HStack {
                        Text(scheduleDate(run.dueAtUnixMs, zone: task.timeZone))
                        Text(run.phase.title).foregroundStyle(.secondary)
                        Spacer()
                        if let id = run.conversationId {
                            Button(run.phase == .outcomeUnknown ? "查看会话核查" : "打开结果") { openConversation(id) }
                        }
                    }
                    if let detail = run.detail { Text(detail).uiFont(.callout).foregroundStyle(.secondary).textSelection(.enabled) }
                    if let cwd = run.cwd { Text(cwd).uiFont(.caption).foregroundStyle(.secondary).textSelection(.enabled) }
                }
            }
            if !runs.isEmpty { Text("显示本设备最近 200 次运行中的记录。").uiFont(.caption).foregroundStyle(.secondary) }
        }
    }

    // MARK: Actions

    private func availability(_ task: ScheduledTask) -> ScheduledTaskAvailability? {
        model.state?.availability.first { $0.taskId == task.id }
    }
    private func permissionLabel(_ task: ScheduledTask) -> String {
        guard let mode = task.permissionMode else { return "沿用会话设置，新会话使用默认" }
        let type = store.runtimeInstances.first { $0.id == task.target.runtimeID }?.typeId
        return store.runtimeTypes.first { $0.id == type }?.permissionModes.first { $0.id == mode }?.label ?? mode
    }
    private func setEnabled(_ task: ScheduledTask, _ enabled: Bool) {
        model.request(.setEnabled(id: task.id, revision: task.revision, enabled: enabled)) { state in
            if draft != nil, let updated = state.tasks.first(where: { $0.id == task.id }) {
                draft?.revision = updated.revision; draft?.status = updated.status; original = updated
            }
            notice = enabled ? "计划已启用。" : "计划已暂停；已启动的运行继续。"
        }
    }
    private func save() {
        guard let draft else { return }
        let existing = Set(model.state?.tasks.map(\.id) ?? [])
        model.request(.save(task: draft)) { state in
            select(draft.id.isEmpty ? state.tasks.first { !existing.contains($0.id) }?.id : draft.id)
            notice = draft.id.isEmpty ? "已创建，尚未启用。" : "修改已保存。"
        }
    }
    private func requestSelection(_ id: String?) {
        guard selected != id || draft != nil else { return }
        if dirty { pendingSelection = id; pendingNew = false; discarding = true } else { select(id) }
    }
    private func select(_ id: String?) { selected = id; draft = nil; original = nil; notice = nil; model.error = nil }
    private func requestNew() { if dirty { pendingNew = true; discarding = true } else { create() } }
    private func create() {
        selected = nil; notice = nil; model.error = nil; editGeneration = UUID()
        draft = ScheduledTask(id: "", revision: 0, name: "", prompt: "",
            rrule: "FREQ=DAILY;BYHOUR=9;BYMINUTE=0;BYSECOND=0", timeZone: TimeZone.current.identifier,
            status: .paused, target: .newConversation(runtimeId: store.enabledRuntimeInstances.first?.id ?? "",
            workingDirectories: [], worktree: false),
            model: nil, effort: nil, permissionMode: nil, notifyOnlyFailures: false,
            nextRunAtUnixMs: nil, source: nil, issue: nil)
        original = draft
    }
    private func openConversation(_ id: String) { store.showConversation(id) }
}

// MARK: - Extensions

extension ScheduledTarget {
    var runtimeID: String { switch self { case .newConversation(let id, _, _), .existingConversation(let id, _): id } }
}
extension ModelSelection {
    var scheduleName: String { switch self { case .managed(let key): key; case .native(_, let model): model } }
}
extension ScheduledTaskStatus {
    var title: String { switch self { case .paused: "已暂停"; case .active: "已启用"; case .needsConfiguration: "需要处理" } }
    var iconName: String { switch self { case .active: "circle.fill"; case .paused: "circle"; case .needsConfiguration: "exclamationmark.triangle.fill" } }
    var tintColor: Color { switch self { case .active: .green; case .paused: .secondary; case .needsConfiguration: .orange } }
}
extension ScheduledTask {
    var listDetail: String { if status == .active, let next = nextRunAtUnixMs { return "下次 " + scheduleDate(next, zone: timeZone) }; return status.title }
}
extension ScheduledRunPhase {
    var title: String { switch self { case .waiting: "等待执行"; case .dispatching: "正在派发"; case .running: "运行中"; case .completed: "已完成"; case .failed: "失败"; case .outcomeUnknown: "结果待核查"; case .skipped: "已跳过" } }
}

func scheduleDate(_ ms: Int64, zone: String) -> String {
    let formatter = DateFormatter()
    formatter.dateStyle = .medium; formatter.timeStyle = .short; formatter.timeZone = TimeZone(identifier: zone)
    return formatter.string(from: Date(timeIntervalSince1970: Double(ms) / 1000))
}

@MainActor func scheduleModelName(_ selection: ModelSelection, store: DeviceStore) -> String {
    if case .managed(let key) = selection {
        for provider in store.gateway.providers {
            if let model = provider.models.first(where: { $0.recordKey == key }) {
                return "\(model.displayName)（\(provider.name)）"
            }
        }
    }
    return selection.scheduleName
}
