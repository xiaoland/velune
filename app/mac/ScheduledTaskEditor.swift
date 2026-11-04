import AppKit
import SwiftUI
import VeluneBindings

struct ScheduledTaskEditor: View {
    @Binding var task: ScheduledTask
    @ObservedObject var store: DeviceStore
    @Binding var scheduleReady: Bool
    @State private var catalog: RuntimeModelCatalog = .loading
    @State private var choosingConversation = false
    @State private var keptDirectories: [String] = []
    @State private var keptWorktree = false
    @State private var keptConversation = ""
    @State private var chosenConversation: Conversation?
    private var runtime: String { task.target.runtimeID }
    private var existing: Bool { if case .existingConversation = task.target { true } else { false } }
    private var permissions: [PermissionMode] { let type = store.runtimeInstances.first { $0.id == runtime }?.typeId; return store.runtimeTypes.first { $0.id == type }?.permissionModes ?? [] }
    private var models: [RuntimeModel] { catalog.models ?? [] }
    private var efforts: [String] {
        if let model = task.model { return models.first { $0.selection == model }?.reasoningEfforts ?? [] }
        var result: [String] = []
        for model in models {
            guard case .native = model.selection else { continue }
            for effort in model.reasoningEfforts where !result.contains(effort) { result.append(effort) }
        }
        return result
    }
    var body: some View {
        Form {
            Section("做什么") {
                TextField("名称", text: $task.name)
                LabeledContent("指令") { TextEditor(text: $task.prompt).uiFont(.body).frame(minHeight: 110).border(.quaternary).accessibilityLabel("任务指令") }
            }
            Section("何时执行") { ScheduledScheduleEditor(task: $task, ready: $scheduleReady, store: store) }
            Section("在哪里执行") {
                Picker("运行时", selection: Binding(get: { runtime }, set: changeRuntime)) {
                    if !store.runtimeInstances.contains(where: { $0.id == runtime }) { Text("请选择运行时").tag(runtime) }
                    ForEach(store.runtimeInstances, id: \.id) { Text($0.name + ($0.enabled ? "" : "（已停用）")).tag($0.id) }
                }
                Picker("上下文", selection: Binding(get: { existing }, set: changeContext)) {
                    Text("每次新建会话").tag(false); Text("继续已有会话").tag(true)
                }
                switch task.target {
                case .newConversation(_, let cwds, let worktree):
                    ForEach(Array(cwds.enumerated()), id: \.offset) { index, path in
                        HStack {
                            TextField("工作目录", text: Binding(get: { path }, set: { value in var paths = cwds; paths[index] = value; setDirectories(paths) }))
                            Button { var paths = cwds; paths.remove(at: index); setDirectories(paths) } label: { Image(systemName: "minus.circle") }.buttonStyle(.borderless).help("移除此目录")
                        }
                    }
                    HStack {
                        Button("添加目录…") { addDirectory() }
                        if case .ssh = store.location(of: runtime) { Text("填写执行设备上的绝对路径").foregroundStyle(.secondary) }
                    }
                    Text(cwds.isEmpty ? "未指定目录时使用运行时默认目录。" : "每个目录会分别创建会话并执行一次任务。").uiFont(.caption).foregroundStyle(.secondary)
                    Toggle("每次创建独立 Git 工作树", isOn: Binding(get: { worktree }, set: { task.target = .newConversation(runtimeId: runtime, workingDirectories: cwds, worktree: $0) }))
                    if worktree { Text("需要 Git 目录。工作树与运行产物会保留。").uiFont(.caption).foregroundStyle(.secondary) }
                case .existingConversation(_, let id):
                    HStack {
                        Text(chosenConversation?.title ?? (store.conversations.first { $0.id == id }?.title ?? (id.isEmpty ? "尚未选择会话" : "已选择的会话")))
                        Spacer(); Button("选择会话…") { choosingConversation = true }
                    }
                    Text("任务等待会话空闲、审批结束和外部控制权释放后再发送。").uiFont(.caption).foregroundStyle(.secondary)
                }
            }
            Section("模型与权限") {
                Picker("模型", selection: Binding(get: { task.model }, set: { task.model = $0; task.effort = nil })) {
                    Text("执行运行时的默认模型").tag(nil as ModelSelection?)
                    ForEach(models, id: \.selection) { Text($0.name).tag(Optional($0.selection)) }
                    if let model = task.model, !models.contains(where: { $0.selection == model }) { Text("\(model.scheduleName)（当前不可用）").tag(Optional(model)) }
                }
                switch catalog {
                case .loading: HStack { ProgressView().controlSize(.small); Text("正在读取模型目录…").foregroundStyle(.secondary) }
                case .failed(let detail): HStack(alignment: .top) { Text("模型目录读取失败：" + detail).foregroundStyle(.red).textSelection(.enabled); Button("重试") { loadModels() } }
                case .ready: EmptyView()
                }
                if !efforts.isEmpty || task.effort != nil {
                    Picker("推理强度", selection: $task.effort) {
                        Text("沿用会话设置 / 新会话默认").tag(nil as String?)
                        ForEach(efforts, id: \.self) { Text($0).tag(Optional($0)) }
                        if let effort = task.effort, !efforts.contains(effort) { Text("\(effort)（请重新选择模型核对）").tag(Optional(effort)) }
                    }
                }
                if !permissions.isEmpty || task.permissionMode != nil {
                    Picker("权限", selection: $task.permissionMode) {
                        Text("沿用会话设置 / 新会话默认").tag(nil as String?)
                        ForEach(permissions, id: \.id) { Text($0.label).tag(Optional($0.id)) }
                        if let mode = task.permissionMode, !permissions.contains(where: { $0.id == mode }) { Text("\(mode)（当前不可用）").tag(Optional(mode)) }
                    }
                    if let mode = permissions.first(where: { $0.id == task.permissionMode }) { Text(mode.detail).uiFont(.caption).foregroundStyle(.secondary) }
                }
                Text("未指定推理或权限时，沿用所选运行时在此会话中的设置；没有可沿用的设置时使用运行时默认。任务不会自动批准请求。").uiFont(.caption).foregroundStyle(.secondary)
                Toggle("仅失败时通知", isOn: $task.notifyOnlyFailures)
            }
        }.formStyle(.grouped)
        .task(id: runtime) { loadModels() }
        .sheet(isPresented: $choosingConversation) {
            ScheduledConversationPicker(store: store, runtime: runtime) { conversation in
                chosenConversation = conversation; keptConversation = conversation.id
                task.target = .existingConversation(runtimeId: runtime, conversationId: conversation.id)
            }
        }
    }
    private func loadModels() {
        let id = runtime; catalog = .loading
        store.readModelCatalog(runtimeID: id) { result in if runtime == id { catalog = result } }
    }
    private func changeRuntime(_ id: String) {
        switch task.target {
        case .newConversation(_, let paths, let worktree): task.target = .newConversation(runtimeId: id, workingDirectories: paths, worktree: worktree)
        case .existingConversation(_, let conversation): task.target = .existingConversation(runtimeId: id, conversationId: conversation)
        }
        task.model = nil; task.effort = nil; task.permissionMode = nil
    }
    private func changeContext(_ value: Bool) {
        switch task.target {
        case .newConversation(_, let paths, let worktree): keptDirectories = paths; keptWorktree = worktree
        case .existingConversation(_, let id): keptConversation = id
        }
        task.target = value ? .existingConversation(runtimeId: runtime, conversationId: keptConversation) : .newConversation(runtimeId: runtime, workingDirectories: keptDirectories, worktree: keptWorktree)
    }
    private func setDirectories(_ paths: [String]) {
        guard case .newConversation(_, _, let worktree) = task.target else { return }
        task.target = .newConversation(runtimeId: runtime, workingDirectories: paths, worktree: worktree)
    }
    private func addDirectory() {
        guard case .newConversation(_, let paths, _) = task.target else { return }
        if case .ssh = store.location(of: runtime) { setDirectories(paths + [""]); return }
        let panel = NSOpenPanel(); panel.canChooseFiles = false; panel.canChooseDirectories = true; panel.allowsMultipleSelection = true
        if panel.runModal() == .OK { setDirectories(paths + panel.urls.map(\.path).filter { !paths.contains($0) }) }
    }
}

private struct ScheduledConversationPicker: View {
    @ObservedObject var store: DeviceStore
    let runtime: String
    let selected: (Conversation) -> Void
    @Environment(\.dismiss) private var dismiss
    @State private var search = ""
    @State private var found: [Conversation] = []
    @State private var error: String?
    @State private var loading = false
    var body: some View {
        VStack(alignment: .leading, spacing: 12) {
            Text("选择已有会话").uiFont(.title2)
            TextField("搜索标题或目录", text: $search)
            if let error { Text(error).foregroundStyle(.red) }
            if loading { ProgressView().controlSize(.small) }
            List(found.filter { $0.parentConversationID == nil }) { conversation in
                Button { selected(conversation); dismiss() } label: {
                    VStack(alignment: .leading, spacing: 3) {
                        Text(conversation.title)
                        Text((store.runtimeInstances.first { $0.id == conversation.runtimeID }?.name ?? "运行时不可用") + " · " + (conversation.cwd ?? "默认目录")).uiFont(.caption).foregroundStyle(.secondary)
                    }.frame(maxWidth: .infinity, alignment: .leading)
                }.buttonStyle(.plain)
            }
            HStack { Text("所选会话将由任务配置的运行时继续。").uiFont(.caption).foregroundStyle(.secondary); Spacer(); Button("取消") { dismiss() } }
        }.padding(20).frame(width: 600, height: 440)
        .task(id: search) {
            do { try await Task.sleep(for: .milliseconds(250)) } catch { return }
            if search.trimmingCharacters(in: .whitespacesAndNewlines).isEmpty { found = store.conversations; error = nil; loading = false; return }
            guard let transport = store.transport else { return }
            loading = true
            let query = search
            let result = await withCheckedContinuation { continuation in
                store.performOnQueue { continuation.resume(returning: Result { try transport.searchConversations(query) }) }
            }
            guard !Task.isCancelled, query == search else { return }
            loading = false
            switch result {
            case .success(let value): found = value.conversations.map(BindingMapping.listed); error = nil
            case .failure(let failure): error = AppProblem.failure(failure, source: "定时任务").detail
            }
        }
    }
}
