import SwiftUI
import VeluneBindings

/// Identifies a template in the window's selection; unsaved ones carry their kind.
enum TemplateSelection {
    static let newPrefix = "new:"
    static func new(_ kind: BindingRuntimeResourceKind) -> String { newPrefix + String(describing: kind) }
    static func newKind(_ id: String) -> BindingRuntimeResourceKind? {
        guard id.hasPrefix(newPrefix) else { return nil }
        return BindingRuntimeResourceKind.templateKinds.first { String(describing: $0) == id.dropFirst(newPrefix.count) }
    }
}

struct FailureBanner: View {
    let message: String
    var body: some View {
        HStack(alignment: .firstTextBaseline, spacing: 8) {
            Image(systemName: "exclamationmark.triangle.fill").foregroundStyle(.yellow)
            Text(message).font(.callout).textSelection(.enabled)
            Spacer()
        }.padding(8).background(.quaternary, in: RoundedRectangle(cornerRadius: 6))
    }
}

/// Edits one template and shows where it is applied.
struct TemplateEditorView: View {
    @ObservedObject var library: TemplateLibrary
    @ObservedObject var store: AppStore
    let selection: String
    let canModify: Bool
    let select: (String?) -> Void
    @State private var draft: TemplateDraft?
    @State private var saved: TemplateDraft?
    @State private var confirmingDelete = false
    @State private var applying = false

    private var isNew: Bool { TemplateSelection.newKind(selection) != nil }
    private var isDirty: Bool { draft != saved }

    var body: some View {
        Group {
            if draft != nil {
                VStack(spacing: 0) {
                    ScrollView {
                        VStack(alignment: .leading, spacing: 16) {
                            TemplateForm(draft: Binding(get: { draft ?? TemplateDraft(kind: .userInstructions) }, set: { draft = $0 }), canEdit: canModify)
                            if !isNew { appliedSection }
                        }.padding(16)
                    }
                    Divider()
                    footer.padding(12)
                }
            } else if let message = library.failure {
                ContentUnavailableView { Label("无法读取模板", systemImage: "exclamationmark.triangle") } description: { Text(message) }
            } else {
                ProgressView().frame(maxWidth: .infinity, maxHeight: .infinity)
            }
        }
        .task(id: selection) { load() }
        .sheet(isPresented: $applying) {
            if let draft, !draft.id.isEmpty { ApplyTemplateSheet(store: store, library: library, templateID: draft.id, templateName: draft.name, kind: draft.kind, initial: []) }
        }
        .alert("删除模板“\(draft?.name ?? "")”？", isPresented: $confirmingDelete) {
            Button("取消", role: .cancel) {}
            Button("删除", role: .destructive) { delete() }
        } message: { Text("已应用此模板的运行时保留各自的原生条目，但不再受 Velune 管理。") }
    }

    private func load() {
        draft = nil; saved = nil
        if let kind = TemplateSelection.newKind(selection) {
            let value = TemplateDraft(kind: kind)
            draft = value; saved = value
            return
        }
        let id = selection
        library.run({ try $0.readResourceTemplate(id) }, done: { template in
            let value = TemplateDraft(template)
            draft = value; saved = value
        })
    }

    @ViewBuilder private var appliedSection: some View {
        let applied = library.status(of: draft?.id ?? "")
        VStack(alignment: .leading, spacing: 8) {
            HStack {
                Text("已应用的运行时").font(.headline)
                Spacer()
                if library.hasUpdates(draft?.id ?? "") {
                    Button("同步全部") { syncAll() }.disabled(!canModify || library.isWorking)
                }
            }
            if applied.isEmpty {
                Text("尚未应用到任何运行时。").foregroundStyle(.secondary)
            } else {
                ForEach(applied, id: \.runtimeID) { item in
                    HStack {
                        Text(store.runtimeInstances.first { $0.id == item.runtimeID }?.name ?? item.runtimeID)
                        Spacer()
                        Text(item.status.state.title).foregroundStyle(item.status.state.needsAttention ? .orange : .secondary).lineLimit(1)
                    }
                }
            }
        }
    }

    private var footer: some View {
        HStack(spacing: 8) {
            if let message = library.failure { FailureBanner(message: message) } else if !canModify {
                Text("有任务运行时，不能同步到运行时").font(.callout).foregroundStyle(.secondary)
            } else if isDirty {
                Text("有未保存的修改").font(.callout).foregroundStyle(.secondary)
            }
            Spacer()
            if library.isWorking { ProgressView().controlSize(.small) }
            if !isNew { Button("删除…", role: .destructive) { confirmingDelete = true }.disabled(library.isWorking) }
            Button("应用到运行时…") { applying = true }.disabled(isNew || isDirty || !canModify || library.isWorking)
                .help(isDirty ? "请先保存修改" : "")
            Button("还原") { draft = saved; library.failure = nil }.disabled(!isDirty || library.isWorking)
            Button(isNew ? "创建" : "保存") { save() }
                .keyboardShortcut("s", modifiers: .command).buttonStyle(.borderedProminent)
                .disabled(!isDirty && !isNew || library.isWorking || (draft?.name.trimmingCharacters(in: .whitespaces).isEmpty ?? true))
        }
    }

    private func save() {
        guard let value = draft else { return }
        let template = value.template
        library.run({ try $0.saveResourceTemplate(template) }, done: { summary in
            var stored = value; stored.id = summary.id
            draft = stored; saved = stored
            library.reload(runtimeIDs: store.runtimeInstances.map(\.id))
            if isNew { select(summary.id) }
        })
    }

    private func delete() {
        guard let id = draft?.id, !id.isEmpty else { return }
        library.run({ try $0.deleteResourceTemplate(id) }, done: { _ in
            library.reload(runtimeIDs: store.runtimeInstances.map(\.id))
            select(nil)
        })
    }

    private func syncAll() {
        guard let id = draft?.id else { return }
        let targets = library.status(of: id).filter { $0.status.state.isTemplateUpdated }.map { BindingTemplateApplyTarget(runtimeId: $0.runtimeID, overwrite: false) }
        let runtimeIDs = store.runtimeInstances.map(\.id)
        library.run({ _ = try $0.applyResourceTemplate(id, targets: targets) }, done: { _ in library.reload(runtimeIDs: runtimeIDs) })
    }
}

/// The fields of one template kind.
private struct TemplateForm: View {
    @Binding var draft: TemplateDraft
    let canEdit: Bool
    @State private var revealSecrets = false

    var body: some View {
        VStack(alignment: .leading, spacing: 12) {
            HStack(spacing: 8) {
                Image(systemName: draft.kind.templateSymbol).foregroundStyle(.secondary)
                Text(draft.kind.templateTitle).font(.headline)
            }
            LabeledContent(draft.kind == .userInstructions ? "标题" : "名称") {
                TextField("", text: $draft.name).labelsHidden().textFieldStyle(.roundedBorder)
            }
            switch draft.kind {
            case .userInstructions:
                Text("应用时整体替换运行时的用户指令文件。").font(.callout).foregroundStyle(.secondary)
                editor($draft.body, height: 280)
            case .nativeAgentDefinitions:
                LabeledContent("描述") { TextField("", text: $draft.description).labelsHidden().textFieldStyle(.roundedBorder) }
                LabeledContent("模型") { TextField("可选", text: $draft.model).labelsHidden().textFieldStyle(.roundedBorder) }
                LabeledContent("工具") { TextField("以逗号分隔，可选", text: $draft.tools).labelsHidden().textFieldStyle(.roundedBorder) }
                Text("指令").font(.subheadline)
                editor($draft.body, height: 220)
            case .mcpServers: mcp
            case .skills: SkillFilesEditor(files: $draft.skillFiles)
            }
        }.disabled(false)
    }

    private func editor(_ text: Binding<String>, height: CGFloat) -> some View {
        PlainTextEditor(text: text, isEditable: true)
            .frame(height: height)
            .background(Color(nsColor: .textBackgroundColor), in: RoundedRectangle(cornerRadius: 6))
            .overlay(RoundedRectangle(cornerRadius: 6).stroke(.separator))
    }

    @ViewBuilder private var mcp: some View {
        Picker("传输", selection: $draft.isHTTP) { Text("本地进程（stdio）").tag(false); Text("HTTP").tag(true) }.pickerStyle(.segmented).fixedSize()
        if draft.isHTTP {
            LabeledContent("地址") { TextField("https://", text: $draft.url).labelsHidden().textFieldStyle(.roundedBorder) }
        } else {
            LabeledContent("命令") { TextField("", text: $draft.command).labelsHidden().textFieldStyle(.roundedBorder) }
            Text("参数（每行一个）").font(.subheadline)
            editor($draft.arguments, height: 80)
        }
        HStack {
            Text(draft.isHTTP ? "请求头" : "环境变量").font(.subheadline)
            Spacer()
            Toggle("显示值", isOn: $revealSecrets).toggleStyle(.checkbox)
            Button { draft.pairs.append(DraftKeyValue(key: "", value: "")) } label: { Image(systemName: "plus") }.help("添加").accessibilityLabel("添加")
        }
        ForEach($draft.pairs) { $pair in
            HStack {
                TextField("名称", text: $pair.key).textFieldStyle(.roundedBorder).frame(maxWidth: 180)
                if revealSecrets { TextField("值", text: $pair.value).textFieldStyle(.roundedBorder) } else { SecureField("值", text: $pair.value).textFieldStyle(.roundedBorder) }
                Button { draft.pairs.removeAll { $0.id == pair.id } } label: { Image(systemName: "minus.circle") }.buttonStyle(.borderless).help("移除").accessibilityLabel("移除")
            }
        }
        Text("值可能含密钥，只保存在 Velune 的私有文件中，默认遮盖。").font(.caption).foregroundStyle(.secondary)
    }
}

private struct SkillFilesEditor: View {
    @Binding var files: [DraftSkillFile]
    @State private var selected: UUID?

    var body: some View {
        HStack(alignment: .top, spacing: 12) {
            VStack(spacing: 6) {
                List(selection: $selected) {
                    ForEach(files) { file in Text(file.path.isEmpty ? "未命名" : file.path).lineLimit(1).truncationMode(.middle).tag(file.id) }
                }
                .listStyle(.bordered)
                .frame(width: 190, height: 260)
                HStack {
                    Button { let file = DraftSkillFile(path: "新文件.md", text: ""); files.append(file); selected = file.id } label: { Image(systemName: "plus") }.help("添加文件").accessibilityLabel("添加文件")
                    Button { files.removeAll { $0.id == selected }; selected = files.first?.id } label: { Image(systemName: "minus") }
                        .help("移除文件").accessibilityLabel("移除文件").disabled(selected == nil || files.first(where: { $0.id == selected })?.path == "SKILL.md")
                    Spacer()
                }
            }
            if let index = files.firstIndex(where: { $0.id == selected }) {
                VStack(alignment: .leading, spacing: 8) {
                    TextField("路径", text: $files[index].path).textFieldStyle(.roundedBorder).disabled(files[index].path == "SKILL.md")
                    if files[index].text != nil {
                        PlainTextEditor(text: Binding(get: { files[index].text ?? "" }, set: { files[index].text = $0 }), isEditable: true)
                            .frame(height: 220)
                            .background(Color(nsColor: .textBackgroundColor), in: RoundedRectangle(cornerRadius: 6))
                            .overlay(RoundedRectangle(cornerRadius: 6).stroke(.separator))
                    } else {
                        Text("二进制文件（\(files[index].bytes.count) 字节），应用时原样写入。").foregroundStyle(.secondary).frame(maxWidth: .infinity, minHeight: 220)
                    }
                    Toggle("可执行", isOn: $files[index].executable)
                }
            } else {
                Text("选择一个文件").foregroundStyle(.secondary).frame(maxWidth: .infinity, minHeight: 260)
            }
        }.onAppear { selected = files.first?.id }
    }
}

/// Chooses the runtimes a template goes to, shows what would happen to each,
/// and applies it. Conflicts need an explicit decision per runtime.
struct ApplyTemplateSheet: View {
    @ObservedObject var store: AppStore
    @ObservedObject var library: TemplateLibrary
    let templateID: String
    let templateName: String
    let kind: BindingRuntimeResourceKind
    let initial: Set<String>
    @Environment(\.dismiss) private var dismiss
    @State private var chosen: Set<String> = []
    @State private var overwrite: Set<String> = []
    @State private var reports: [String: BindingTemplateApplyReport] = [:]
    @State private var applied = false

    private var targets: [BindingTemplateApplyTarget] {
        store.runtimeInstances.filter { chosen.contains($0.id) }.map { BindingTemplateApplyTarget(runtimeId: $0.id, overwrite: overwrite.contains($0.id)) }
    }

    var body: some View {
        VStack(alignment: .leading, spacing: 12) {
            Text("应用“\(templateName)”").font(.title3.bold())
            Text("选中的运行时将写入此\(kind.templateTitle)，之后由 Velune 保持同步。").foregroundStyle(.secondary)
            List {
                ForEach(store.runtimeInstances) { instance in
                    VStack(alignment: .leading, spacing: 3) {
                        Toggle(isOn: Binding(get: { chosen.contains(instance.id) }, set: { on in
                            if on { chosen.insert(instance.id) } else { chosen.remove(instance.id); overwrite.remove(instance.id) }
                            refresh()
                        })) { Text(instance.name) }.disabled(applied)
                        if chosen.contains(instance.id), let report = reports[instance.id] {
                            Text(report.outcome.title(applied: applied)).font(.callout).foregroundStyle(report.outcome.isFailure ? .red : .secondary)
                            ForEach(report.notes, id: \.self) { Text("• \($0)").font(.caption).foregroundStyle(.secondary) }
                            if report.outcome.isConflict && !applied {
                                Toggle("覆盖这些条目", isOn: Binding(get: { overwrite.contains(instance.id) }, set: { on in
                                    if on { overwrite.insert(instance.id) } else { overwrite.remove(instance.id) }
                                    refresh()
                                })).font(.callout)
                            }
                        }
                    }.padding(.vertical, 2)
                }
            }.frame(minHeight: 200)
            if let message = library.failure { FailureBanner(message: message) }
            HStack {
                if library.isWorking { ProgressView().controlSize(.small) }
                Spacer()
                Button(applied ? "完成" : "取消") { dismiss() }.keyboardShortcut(.cancelAction)
                if !applied {
                    Button("应用") { apply() }.buttonStyle(.borderedProminent).keyboardShortcut(.defaultAction)
                        .disabled(chosen.isEmpty || library.isWorking || chosen.contains { id in
                            guard let outcome = reports[id]?.outcome else { return false }
                            return outcome.isConflict || outcome.isFailure
                        })
                }
            }
        }
        .padding(20).frame(width: 520)
        .onAppear { chosen = initial; library.failure = nil; refresh() }
    }

    private func refresh() {
        guard !chosen.isEmpty else { reports = [:]; return }
        let id = templateID, targets = targets
        library.run({ try $0.previewResourceTemplate(id, targets: targets) }, done: { list in
            reports = Dictionary(uniqueKeysWithValues: list.map { ($0.runtimeId, $0) })
        })
    }

    private func apply() {
        let id = templateID, targets = targets, runtimeIDs = store.runtimeInstances.map(\.id)
        library.run({ try $0.applyResourceTemplate(id, targets: targets) }, done: { list in
            reports = Dictionary(uniqueKeysWithValues: list.map { ($0.runtimeId, $0) })
            applied = true
            library.reload(runtimeIDs: runtimeIDs)
        })
    }
}

/// The templates one runtime has applied, with the choices a drifted or
/// missing entry needs.
struct RuntimeTemplatesView: View {
    @ObservedObject var store: AppStore
    @ObservedObject var library: TemplateLibrary
    let instance: RuntimeInstance
    let canModify: Bool
    @State private var applyingTemplate: BindingResourceTemplateSummary?
    @State private var removing: BindingRuntimeTemplateStatus?

    private var statuses: [BindingRuntimeTemplateStatus] { library.statuses[instance.id] ?? [] }

    var body: some View {
        VStack(alignment: .leading, spacing: 0) {
            HStack {
                Menu("应用模板…") {
                    ForEach(BindingRuntimeResourceKind.templateKinds, id: \.self) { kind in
                        let items = library.summaries.filter { $0.kind == kind }
                        if !items.isEmpty {
                            Section(kind.templateTitle) { ForEach(items, id: \.id) { item in Button(item.name) { applyingTemplate = item } } }
                        }
                    }
                }.fixedSize().disabled(library.summaries.isEmpty || !canModify)
                if statuses.contains(where: { $0.state.isTemplateUpdated }) {
                    Button("同步全部更新") { syncUpdated() }.disabled(!canModify || library.isWorking)
                }
                Spacer()
                if library.isWorking { ProgressView().controlSize(.small) }
            }.padding(16)
            Divider()
            if statuses.isEmpty {
                ContentUnavailableView("没有受管模板", systemImage: "square.stack.3d.up", description: Text(library.summaries.isEmpty ? "先在侧栏的“模板”中创建模板。" : "用“应用模板…”把模板写入此运行时。"))
            } else {
                List(statuses, id: \.templateId) { status in row(status) }
            }
            if let message = library.failure { FailureBanner(message: message).padding(12) }
        }
        .task { library.reload(runtimeIDs: store.runtimeInstances.map(\.id)) }
        .sheet(item: $applyingTemplate) { item in
            ApplyTemplateSheet(store: store, library: library, templateID: item.id, templateName: item.name, kind: item.kind, initial: [instance.id])
        }
        .alert("移除“\(removing?.templateName ?? "")”？", isPresented: Binding(get: { removing != nil }, set: { if !$0 { removing = nil } })) {
            Button("取消", role: .cancel) {}
            Button("移除", role: .destructive) { if let removing { resolve(removing, .remove) } }
        } message: { Text("将删除此运行时中由该模板写入的原生条目，并停止管理。") }
    }

    private func row(_ status: BindingRuntimeTemplateStatus) -> some View {
        HStack(spacing: 10) {
            Image(systemName: status.kind.templateSymbol).foregroundStyle(.secondary).frame(width: 20)
            VStack(alignment: .leading, spacing: 2) {
                Text(status.templateName).lineLimit(1)
                Text(status.state.title).font(.caption).foregroundStyle(status.state.needsAttention ? .orange : .secondary).lineLimit(2)
            }
            Spacer()
            switch status.state {
            case .templateUpdated:
                Button("同步") { resolve(status, .overwrite) }.disabled(!canModify || library.isWorking)
            case .drifted, .missing:
                Menu("处理…") {
                    Button(status.state.isDrifted ? "以模板覆盖" : "按模板重新创建") { resolve(status, .overwrite) }
                    Button("不再管理，保留原生内容") { resolve(status, .unmanage) }
                }.fixedSize().disabled(!canModify || library.isWorking)
            default: EmptyView()
            }
        }
        .padding(.vertical, 2)
        .contextMenu {
            Button("不再管理，保留原生内容") { resolve(status, .unmanage) }
            Button("移除原生条目并停止管理…", role: .destructive) { removing = status }
        }
    }

    private func resolve(_ status: BindingRuntimeTemplateStatus, _ resolution: BindingResolution) {
        let runtimeID = instance.id, templateID = status.templateId, runtimeIDs = store.runtimeInstances.map(\.id)
        library.run({ try $0.resolveResourceBinding(runtimeID: runtimeID, templateID: templateID, resolution: resolution) }, done: { _ in library.reload(runtimeIDs: runtimeIDs) })
    }

    private func syncUpdated() {
        let runtimeID = instance.id, runtimeIDs = store.runtimeInstances.map(\.id)
        let ids = statuses.filter { $0.state.isTemplateUpdated }.map(\.templateId)
        library.run({ transport in
            for id in ids { _ = try transport.applyResourceTemplate(id, targets: [BindingTemplateApplyTarget(runtimeId: runtimeID, overwrite: false)]) }
        }, done: { _ in library.reload(runtimeIDs: runtimeIDs) })
    }
}

extension BindingTemplateSyncState { var isDrifted: Bool { if case .drifted = self { true } else { false } } }
extension BindingResourceTemplateSummary: Identifiable {}
