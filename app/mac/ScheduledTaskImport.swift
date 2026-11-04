import AppKit
import SwiftUI
import VeluneBindings

struct ScheduledTaskImport: View {
    @ObservedObject var model: ScheduledTasksModel
    @ObservedObject var store: DeviceStore
    let openImported: (String) -> Void
    @Environment(\.dismiss) private var dismiss
    @State private var directory = ""
    @State private var runtime = ""
    @State private var zone = TimeZone.current.identifier
    @State private var preview: [ScheduledTask] = []
    @State private var issues: [String] = []
    @State private var selected: String?
    @State private var imported: [String: String] = [:]
    @State private var updating = false
    @State private var pendingRevision: UInt64?
    private var selection: ScheduledTask? { preview.first { $0.source?.path == selected } }
    private var sourceKey: String { directory + "\n" + runtime + "\n" + zone }
    private var current: ScheduledTask? { model.state?.tasks.first { $0.source?.path == selected } }
    var body: some View {
        VStack(alignment: .leading, spacing: 14) {
            Text("导入 ChatGPT 本地任务").uiFont(.title2)
            Text("先复制任务定义并保持暂停，再在任务详情中核对配置、停止原宿主派发并确认交接。").foregroundStyle(.secondary)
            HStack {
                TextField("Codex HOME 或 automations 目录", text: $directory)
                Button("选择…") { let panel = NSOpenPanel(); panel.canChooseFiles = false; panel.canChooseDirectories = true; if panel.runModal() == .OK { directory = panel.url?.path ?? "" } }
            }
            HStack {
                Picker("对应运行时", selection: $runtime) {
                    Text("请选择").tag("")
                    ForEach(store.runtimeInstances.filter { [RuntimeKind.codex, .codex166].contains($0.typeId) }, id: \.id) { Text($0.name).tag($0.id) }
                }
                ScheduledTimeZonePicker(zone: $zone)
                Button("预览") { load() }.disabled(model.busy || runtime.isEmpty || directory.isEmpty)
            }
            if let error = model.error { Text(error).foregroundStyle(.red).textSelection(.enabled) }
            if !issues.isEmpty { DisclosureGroup("\(issues.count) 项来源问题") { ForEach(issues, id: \.self) { Text($0).uiFont(.caption).foregroundStyle(.orange).textSelection(.enabled) } } }
            Divider()
            HSplitView {
                List(selection: $selected) {
                    ForEach(preview, id: \.source?.path) { task in
                        if let source = task.source {
                            VStack(alignment: .leading, spacing: 4) {
                                Text(task.name)
                                Text(imported[source.path] != nil ? "已导入，保持暂停" : (model.state?.tasks.contains { $0.source?.path == source.path } == true ? "已存在 · 更新需确认" : "来源：\(source.originalStatus)")).uiFont(.caption).foregroundStyle(.secondary)
                            }.tag(source.path)
                        }
                    }
                }.frame(minWidth: 180, idealWidth: 200, maxWidth: 230)
                ScrollView {
                    if let task = selection {
                        VStack(alignment: .leading, spacing: 16) {
                            Text(task.name).uiFont(.headline)
                            Text(task.prompt).textSelection(.enabled)
                            GroupBox("来源配置") {
                                VStack(alignment: .leading, spacing: 6) {
                                    Text("模型：\(task.source?.originalModel ?? "原宿主默认")")
                                    Text("状态：\(task.source?.originalStatus ?? "未知")")
                                    Text("时区：\(task.timeZone)（请核对原宿主时区）")
                                    Text(task.rrule).uiFont(.caption.monospaced()).textSelection(.enabled)
                                }.frame(maxWidth: .infinity, alignment: .leading).padding(4)
                            }
                            GroupBox("Velune 将使用") {
                                VStack(alignment: .leading, spacing: 6) {
                                    Text("运行时：\(store.runtimeInstances.first { $0.id == runtime }?.name ?? "未选择")")
                                    Text("模型：执行运行时的默认模型，未沿用来源模型。导入后可编辑选择。")
                                    Text("推理：\(task.effort ?? "默认") · 权限：运行时默认")
                                    Text(target(task))
                                    Text("导入后保持暂停，交接确认也不会自动启用。")
                                }.frame(maxWidth: .infinity, alignment: .leading).padding(4)
                            }
                            if let current, imported[selected ?? ""] == nil {
                                GroupBox("更新将替换以下配置") {
                                    VStack(alignment: .leading, spacing: 8) {
                                        ForEach(changes(current, task), id: \.self) { Text($0).textSelection(.enabled) }
                                        Text("交接确认将重置，计划将暂停。已经启动的运行继续。").foregroundStyle(.secondary)
                                    }.frame(maxWidth: .infinity, alignment: .leading).padding(4)
                                }
                            }
                            Text("来源环境脚本、自动归档只保留为资料，不会自动执行。").uiFont(.callout).foregroundStyle(.secondary)
                            if let issue = task.issue { Text(issue).uiFont(.callout).foregroundStyle(.secondary) }
                        }.padding(.leading, 16).padding(.vertical, 8).frame(maxWidth: .infinity, alignment: .leading)
                    } else { Text(preview.isEmpty ? "选择来源并预览任务。" : "选择一项任务查看导入配置。").foregroundStyle(.secondary).padding(24) }
                }.frame(minWidth: 390)
            }
            Divider()
            HStack {
                if model.busy { ProgressView().controlSize(.small) }
                Text(imported.isEmpty ? "所有任务先保持暂停。" : "已导入 \(imported.count) 项，均保持暂停。").uiFont(.callout).foregroundStyle(.secondary)
                Spacer()
                Button("关闭") { dismiss() }
                if let path = selected, let id = imported[path] {
                    Button("查看已导入任务") { dismiss(); openImported(id) }
                } else if let task = selection {
                    Button(current == nil ? "导入并保持暂停" : "更新并暂停…") { if current == nil { importTask(task) } else { pendingRevision = current?.revision; updating = true } }.disabled(model.busy)
                }
            }
        }.padding(20).frame(width: 840, height: 650)
        .onChange(of: sourceKey) { _, _ in preview = []; issues = []; selected = nil; imported = [:] }
        .confirmationDialog("更新此任务并暂停？", isPresented: $updating) {
            Button("替换配置、重置交接并暂停", role: .destructive) { if let task = selection { importTask(task, revision: pendingRevision) } }
        } message: { Text("将按预览覆盖此任务的指令、日程、目标与执行配置，并重置原宿主交接确认。请核对上方差异；正在执行的运行不会停止。") }
    }
    private func load() {
        let key = sourceKey
        model.request(.previewImport(sourceDirectory: directory, runtimeId: runtime, timeZone: zone)) { state in
            guard key == sourceKey else { return }
            preview = state.importPreview; issues = state.importIssues; selected = preview.first?.source?.path; imported = [:]
        }
    }
    private func importTask(_ task: ScheduledTask, revision: UInt64? = nil) {
        guard let source = task.source else { return }
        let key = sourceKey
        model.request(.import(sourcePath: source.path, fingerprint: source.fingerprint, runtimeId: runtime, timeZone: zone, expectedRevision: revision)) { state in
            guard key == sourceKey else { return }
            imported[source.path] = state.tasks.first { $0.source?.path == source.path }?.id
        }
    }
    private func target(_ task: ScheduledTask) -> String {
        switch task.target {
        case .newConversation(_, let paths, let worktree): "每个目录新建会话\(worktree ? "，使用独立工作树" : "")：\n" + (paths.isEmpty ? "运行时默认目录" : paths.joined(separator: "\n"))
        case .existingConversation(_, let id): "继续已有会话：" + (store.conversations.first { $0.id == id }?.title ?? id)
        }
    }
    private func changes(_ old: ScheduledTask, _ new: ScheduledTask) -> [String] {
        var result: [String] = []
        if old.name != new.name { result.append("名称：\(old.name) → \(new.name)") }
        if old.prompt != new.prompt { result.append("指令：\(old.prompt)\n→ \(new.prompt)") }
        if old.rrule != new.rrule || old.timeZone != new.timeZone { result.append("日程：\(old.rrule)（\(old.timeZone)）\n→ \(new.rrule)（\(new.timeZone)）") }
        if old.target != new.target { result.append("执行目标：\(target(old))\n→ \(target(new))") }
        result.append("模型：\(old.model.map { scheduleModelName($0, store: store) } ?? "执行运行时默认") → 执行运行时默认（不沿用来源模型）")
        if old.effort != new.effort { result.append("推理强度：\(old.effort ?? "默认") → \(new.effort ?? "默认")") }
        if old.permissionMode != new.permissionMode { result.append("权限：\(old.permissionMode ?? "默认") → 默认") }
        if old.notifyOnlyFailures != new.notifyOnlyFailures { result.append("通知：\(new.notifyOnlyFailures ? "仅失败时" : "完成与失败时")") }
        if result.count == 1 { result.append("指令、日程与执行目标未改变。") }
        return result
    }
}
