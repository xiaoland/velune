import SwiftUI
import VeluneBindings

/// What the content column of the library selects.
enum LibrarySelection: Hashable {
    case resource(String)
    /// A resource being created, not yet saved.
    case newResource(BindingRuntimeResourceKind)
    /// A runtime's native entry that no resource manages.
    case native(runtimeID: String, path: String)
}

extension BindingResourceSyncState {
    var isEnabled: Bool {
        switch self {
        case .notEnabled, .unsupported: false
        default: true
        }
    }
    var needsAttention: Bool {
        switch self {
        case .drifted, .missing: true
        default: false
        }
    }
    var title: String {
        switch self {
        case .notEnabled: "未启用"
        case .unsupported(let reason): reason
        case .synced: "已同步"
        case .pending: "待同步"
        case .drifted: "原生内容已被修改"
        case .missing: "原生内容已被删除"
        }
    }
}

extension BindingResourceTemplateSummary: Identifiable {}

/// The library, the state of every resource on every runtime, and the native
/// entries no resource manages. Edits are kept per selection until saved or
/// reverted; the model lives as long as the app, so closing the window loses
/// nothing either, and quitting asks first.
@MainActor final class ResourceLibraryModel: ObservableObject {
    private(set) static var shared: ResourceLibraryModel?
    static func shared(transport: Transport?) -> ResourceLibraryModel {
        if let shared { return shared }
        let model = ResourceLibraryModel(transport: transport)
        shared = model
        return model
    }

    @Published private(set) var summaries: [BindingResourceTemplateSummary] = []
    /// Resource id → runtime id → state.
    @Published private(set) var states: [String: [String: BindingResourceSyncState]] = [:]
    @Published private(set) var unmanaged: [BindingRuntimeResourceKind: [BindingUnmanagedRuntimeResource]] = [:]
    @Published private(set) var isLoaded = false
    @Published private(set) var isWorking = false
    @Published var failure: String?
    @Published var drafts: [LibrarySelection: TemplateDraft] = [:]
    @Published private(set) var saved: [LibrarySelection: TemplateDraft] = [:]
    private let transport: Transport?
    private let queue = DispatchQueue(label: "local.velune.resource-library", qos: .userInitiated)

    init(transport: Transport?) { self.transport = transport }

    /// Runs `work` off the main thread, then reloads; failures become the visible message.
    func run<T: Sendable>(_ work: @escaping @Sendable (Transport) throws -> T, done: @escaping (T) -> Void = { _ in }) {
        guard let transport else { failure = "本地核心未配置"; return }
        isWorking = true; failure = nil
        queue.async { [weak self] in
            let result = Result { try work(transport) }
            DispatchQueue.main.async {
                guard let self else { return }
                self.isWorking = false
                switch result {
                case .success(let value): done(value); self.reload()
                case .failure(let error): self.failure = ResourceFailure(error).message; self.reload()
                }
            }
        }
    }

    func reload() {
        guard let transport else { return }
        queue.async { [weak self] in
            let result = Result { () -> ([BindingResourceTemplateSummary], [BindingResourceSyncStatus], [BindingRuntimeResourceKind: [BindingUnmanagedRuntimeResource]]) in
                var unmanaged: [BindingRuntimeResourceKind: [BindingUnmanagedRuntimeResource]] = [:]
                for kind in BindingRuntimeResourceKind.templateKinds { unmanaged[kind] = try transport.unmanagedRuntimeResources(kind) }
                return (try transport.listResourceTemplates(), try transport.resourceSyncStatus(), unmanaged)
            }
            DispatchQueue.main.async {
                guard let self else { return }
                switch result {
                case .success(let (summaries, statuses, unmanaged)):
                    self.summaries = summaries.sorted { $0.name.localizedStandardCompare($1.name) == .orderedAscending }
                    var states: [String: [String: BindingResourceSyncState]] = [:]
                    for status in statuses { states[status.templateId, default: [:]][status.runtimeId] = status.state }
                    self.states = states
                    self.unmanaged = unmanaged
                    self.isLoaded = true
                case .failure(let error): self.failure = ResourceFailure(error).message
                }
            }
        }
    }

    func resources(_ kind: BindingRuntimeResourceKind) -> [BindingResourceTemplateSummary] { summaries.filter { $0.kind == kind } }
    func state(_ resourceID: String, _ runtimeID: String) -> BindingResourceSyncState { states[resourceID]?[runtimeID] ?? .notEnabled }
    func attentionCount(_ kind: BindingRuntimeResourceKind) -> Int {
        resources(kind).filter { item in states[item.id]?.values.contains { $0.needsAttention } ?? false }.count
    }
    func summary(named name: String, kind: BindingRuntimeResourceKind) -> BindingResourceTemplateSummary? { summaries.first { $0.kind == kind && $0.name == name } }

    var unsavedNames: [String] { drafts.keys.filter(isDirty).compactMap { drafts[$0]?.name }.map { $0.isEmpty ? "未命名" : $0 } }

    /// A new, unsaved copy of a resource, opened in the editor.
    func duplicate(_ item: BindingResourceTemplateSummary, then: @escaping (LibrarySelection) -> Void) {
        let id = item.id
        run({ try $0.readResourceTemplate(id) }, done: { [weak self] template in
            guard let self else { return }
            var copy = TemplateDraft(template)
            copy.id = ""
            copy.name = item.kind == .userInstructions ? item.name + " 副本" : item.name + "-copy"
            let selection = LibrarySelection.newResource(item.kind)
            self.drafts[selection] = copy; self.saved[selection] = TemplateDraft(kind: item.kind)
            then(selection)
        })
    }

    func isDirty(_ selection: LibrarySelection) -> Bool { drafts[selection] != nil && drafts[selection] != saved[selection] }

    /// Loads a resource, or starts a new one, unless an edit is already held.
    func open(_ selection: LibrarySelection) {
        guard drafts[selection] == nil else { return }
        switch selection {
        case .newResource(let kind):
            let draft = TemplateDraft(kind: kind)
            drafts[selection] = draft; saved[selection] = draft
        case .resource(let id):
            run({ try $0.readResourceTemplate(id) }, done: { [weak self] template in
                let draft = TemplateDraft(template)
                self?.drafts[selection] = draft; self?.saved[selection] = draft
            })
        case .native: break
        }
    }

    func revert(_ selection: LibrarySelection) { drafts[selection] = saved[selection]; failure = nil }

    func forget(_ selection: LibrarySelection) { drafts[selection] = nil; saved[selection] = nil }

    /// Saves and lets the core write it to its enabled runtimes; reports failures per runtime.
    func save(_ selection: LibrarySelection, runtimeName: @escaping (String) -> String, then: @escaping (String) -> Void) {
        guard let draft = drafts[selection] else { return }
        let template = draft.template
        run({ try $0.saveResourceTemplate(template) }, done: { [weak self] result in
            guard let self else { return }
            var stored = draft; stored.id = result.summary.id
            let key = LibrarySelection.resource(result.summary.id)
            self.forget(selection)
            self.drafts[key] = stored; self.saved[key] = stored
            let failed = result.reports.compactMap { report -> String? in
                if case .failed(let reason) = report.outcome { return "\(runtimeName(report.runtimeId))：\(reason)" }
                return nil
            }
            if !failed.isEmpty { self.failure = "已保存，但以下运行时未能同步：" + failed.joined(separator: "；") }
            then(result.summary.id)
        })
    }
}

/// Which category, item and runtime the library window shows; entry points
/// elsewhere set it before opening the window.
@MainActor final class ResourceLibraryNavigator: ObservableObject {
    static let shared = ResourceLibraryNavigator()
    @Published var kind: BindingRuntimeResourceKind = .userInstructions
    @Published var selection: LibrarySelection?
    /// Focuses the lists on one runtime; nil shows all.
    @Published var runtimeID: String?
    func show(_ runtimeID: String, _ openWindow: OpenWindowAction) {
        self.runtimeID = runtimeID
        openWindow(id: "resource-library")
    }
}

struct ResourceLibraryMenuItem: View {
    @Environment(\.openWindow) private var openWindow
    var body: some View { Button("资源库…") { openWindow(id: "resource-library") } }
}

/// Instructions, skills, MCP servers and sub-agents defined once in Velune and
/// enabled on any number of runtimes.
struct ResourceLibraryView: View {
    @ObservedObject var store: AppStore
    @ObservedObject private var navigator = ResourceLibraryNavigator.shared
    @StateObject private var library: ResourceLibraryModel
    @State private var importing = false

    init(store: AppStore) {
        self.store = store
        _library = StateObject(wrappedValue: ResourceLibraryModel.shared(transport: store.resourceTransport))
    }

    var body: some View {
        NavigationSplitView {
            List(selection: Binding(get: { navigator.kind }, set: { if let kind = $0, kind != navigator.kind { navigator.kind = kind; navigator.selection = nil } })) {
                ForEach(BindingRuntimeResourceKind.templateKinds, id: \.self) { kind in
                    Label(kind.templateTitle, systemImage: kind.templateSymbol)
                        .badge(library.attentionCount(kind))
                        .tag(kind)
                }
            }
            .navigationSplitViewColumnWidth(min: 150, ideal: 170, max: 220)
        } content: {
            ResourceListView(store: store, library: library, navigator: navigator)
                .navigationSplitViewColumnWidth(min: 200, ideal: 240, max: 320)
        } detail: {
            detail
        }
        .navigationTitle("资源库")
        .toolbar(id: "resource-library") {
            ToolbarItem(id: "runtime") {
                Picker("运行时", selection: $navigator.runtimeID) {
                    Text("全部运行时").tag(String?.none)
                    ForEach(store.runtimeInstances) { Text($0.name).tag(Optional($0.id)) }
                }.fixedSize().help("只看某个运行时的状态与其他内容")
            }
            ToolbarItem(id: "import") {
                Button { importing = true } label: { Label("从运行时导入…", systemImage: "square.and.arrow.down") }
                    .help("从运行时导入：把某个运行时中不在资源库的内容一次加入资源库")
                    .disabled(store.runtimeInstances.isEmpty)
            }
            ToolbarItem(id: "new") {
                Button { navigator.selection = .newResource(navigator.kind) } label: { Label("新建\(navigator.kind.templateTitle)", systemImage: "plus") }
                    .help("新建\(navigator.kind.templateTitle)")
            }
        }
        .sheet(isPresented: $importing) { RuntimeImportSheet(store: store, library: library, navigator: navigator) }
        .onAppear { library.reload() }
        // A finished turn lets the core write what waited; show it.
        .onChange(of: store.applicationIsGenerating) { _, generating in if !generating { library.reload() } }
        .onChange(of: store.runtimeInstances) { _, instances in
            if let id = navigator.runtimeID, !instances.contains(where: { $0.id == id }) { navigator.runtimeID = nil }
            library.reload()
        }
    }

    @ViewBuilder private var detail: some View {
        switch navigator.selection {
        case .resource, .newResource:
            if let selection = navigator.selection {
                ResourceDetailView(store: store, library: library, navigator: navigator, selection: selection).id(selection)
            }
        case .native(let runtimeID, let path):
            NativeResourceView(store: store, library: library, navigator: navigator, kind: navigator.kind, runtimeID: runtimeID, path: path).id(navigator.selection)
        case nil:
            ContentUnavailableView("选择一项\(navigator.kind.templateTitle)", systemImage: navigator.kind.templateSymbol, description: Text("资源库中的内容可以启用到多个运行时，并由 Velune 保持同步。"))
                .frame(maxWidth: .infinity, maxHeight: .infinity)
        }
    }
}

/// Resources of the selected kind, then native entries that no resource manages.
private struct ResourceListView: View {
    @ObservedObject var store: AppStore
    @ObservedObject var library: ResourceLibraryModel
    @ObservedObject var navigator: ResourceLibraryNavigator
    @State private var deleting: BindingResourceTemplateSummary?
    @State private var query = ""

    private func matches(_ text: String) -> Bool { query.isEmpty || text.localizedCaseInsensitiveContains(query) }
    private var resources: [BindingResourceTemplateSummary] { library.resources(navigator.kind).filter { matches($0.name) } }
    private var others: [BindingUnmanagedRuntimeResource] {
        (library.unmanaged[navigator.kind] ?? []).filter { (navigator.runtimeID == nil || $0.runtimeId == navigator.runtimeID) && matches($0.path) }
    }

    var body: some View {
        List(selection: $navigator.selection) {
            if others.isEmpty { resourceRows } else if !resources.isEmpty { Section("资源库") { resourceRows } }
            if !others.isEmpty {
                Section("运行时中的其他内容") {
                    ForEach(others, id: \.self) { entry in
                        VStack(alignment: .leading, spacing: 2) {
                            Text(entry.path).lineLimit(1)
                            Text(store.runtimeInstances.first { $0.id == entry.runtimeId }?.name ?? entry.runtimeId).font(.caption).foregroundStyle(.secondary).lineLimit(1)
                        }
                        .padding(.vertical, 2)
                        .tag(LibrarySelection.native(runtimeID: entry.runtimeId, path: entry.path))
                    }
                }
            }
        }
        .searchable(text: $query, placement: .toolbar, prompt: "搜索\(navigator.kind.templateTitle)")
        .overlay {
            if library.isLoaded && resources.isEmpty && others.isEmpty && !query.isEmpty {
                ContentUnavailableView.search(text: query)
            } else if library.isLoaded && resources.isEmpty && others.isEmpty {
                ContentUnavailableView {
                    Label("还没有\(navigator.kind.templateTitle)", systemImage: navigator.kind.templateSymbol)
                } actions: {
                    Button("新建\(navigator.kind.templateTitle)") { navigator.selection = .newResource(navigator.kind) }
                }
            }
        }
        .alert("删除“\(deleting?.name ?? "")”？", isPresented: Binding(get: { deleting != nil }, set: { if !$0 { deleting = nil } })) {
            Button("取消", role: .cancel) {}
            Button("删除", role: .destructive) { if let deleting { delete(deleting) } }
        } message: { Text("已启用它的运行时保留各自的原生内容，但不再由 Velune 同步。") }
    }

    private var resourceRows: some View {
        ForEach(resources) { item in
            VStack(alignment: .leading, spacing: 2) {
                HStack(spacing: 6) {
                    Text(item.name).lineLimit(1)
                    if library.isDirty(.resource(item.id)) { Text("已修改").font(.caption).foregroundStyle(.secondary) }
                }
                subtitle(item)
            }
            .padding(.vertical, 2)
            .tag(LibrarySelection.resource(item.id))
            .contextMenu {
                Button("复制") { library.duplicate(item) { navigator.selection = $0 } }
                Divider()
                Button("删除…", role: .destructive) { deleting = item }
            }
        }
    }

    @ViewBuilder private func subtitle(_ item: BindingResourceTemplateSummary) -> some View {
        let states = library.states[item.id] ?? [:]
        if let runtimeID = navigator.runtimeID {
            let state = states[runtimeID] ?? .notEnabled
            Text(state.title).font(.caption).foregroundStyle(state.needsAttention ? .orange : .secondary).lineLimit(1)
        } else {
            let enabled = states.values.filter(\.isEnabled)
            let attention = enabled.contains { $0.needsAttention }
            Text(enabled.isEmpty ? "未启用" : attention ? "有运行时需要处理" : "已启用于 \(enabled.count) 个运行时")
                .font(.caption).foregroundStyle(attention ? .orange : .secondary).lineLimit(1)
        }
    }

    private func delete(_ item: BindingResourceTemplateSummary) {
        let id = item.id
        library.run({ try $0.deleteResourceTemplate(id) }, done: { _ in
            library.forget(.resource(id))
            if navigator.selection == .resource(id) { navigator.selection = nil }
        })
    }
}

/// Edits one resource and chooses the runtimes it is enabled on.
private struct ResourceDetailView: View {
    @ObservedObject var store: AppStore
    @ObservedObject var library: ResourceLibraryModel
    @ObservedObject var navigator: ResourceLibraryNavigator
    let selection: LibrarySelection
    @State private var confirmingDelete = false

    private var isNew: Bool { if case .newResource = selection { true } else { false } }
    private var resourceID: String? { if case .resource(let id) = selection { id } else { nil } }
    private var draft: Binding<TemplateDraft>? {
        guard library.drafts[selection] != nil else { return nil }
        return Binding(get: { library.drafts[selection] ?? TemplateDraft(kind: .userInstructions) }, set: { library.drafts[selection] = $0; library.failure = nil })
    }

    var body: some View {
        Group {
            if let draft {
                VStack(spacing: 0) {
                    ScrollView {
                        VStack(alignment: .leading, spacing: 20) {
                            ResourceForm(draft: draft, canEdit: true, skillNames: library.resources(.skills).map(\.name), veluneModels: store.veluneModelNames)
                            RuntimeTargetsView(store: store, library: library, resourceID: resourceID, kind: draft.wrappedValue.kind, name: draft.wrappedValue.name, isDirty: library.isDirty(selection))
                        }.padding(20)
                    }
                    Divider()
                    footer.padding(12)
                }
            } else if let message = library.failure {
                ContentUnavailableView { Label("无法读取资源", systemImage: "exclamationmark.triangle") } description: { Text(message) }
                    .frame(maxWidth: .infinity, maxHeight: .infinity)
            } else {
                ProgressView().frame(maxWidth: .infinity, maxHeight: .infinity)
            }
        }
        .onAppear { library.open(selection) }
        .alert("删除“\(library.drafts[selection]?.name ?? "")”？", isPresented: $confirmingDelete) {
            Button("取消", role: .cancel) {}
            Button("删除", role: .destructive) { delete() }
        } message: { Text("已启用它的运行时保留各自的原生内容，但不再由 Velune 同步。") }
    }

    private var footer: some View {
        HStack(spacing: 8) {
            if let message = library.failure { FailureBanner(message: message) }
            else if library.isDirty(selection) { Text("保存后同步到已启用的运行时").font(.callout).foregroundStyle(.secondary) }
            Spacer()
            if library.isWorking { ProgressView().controlSize(.small) }
            if !isNew { Button("删除…", role: .destructive) { confirmingDelete = true }.disabled(library.isWorking) }
            Button("还原") { library.revert(selection) }.disabled(!library.isDirty(selection) || library.isWorking)
            Button(isNew ? "创建" : "保存") { save() }
                .keyboardShortcut("s", modifiers: .command).buttonStyle(.borderedProminent)
                .disabled((!isNew && !library.isDirty(selection)) || library.isWorking || (library.drafts[selection]?.name.trimmingCharacters(in: .whitespaces).isEmpty ?? true))
        }
    }

    private func save() {
        let names = Dictionary(store.runtimeInstances.map { ($0.id, $0.name) }, uniquingKeysWith: { first, _ in first })
        library.save(selection, runtimeName: { names[$0] ?? $0 }, then: { id in
            if isNew { navigator.selection = .resource(id) }
        })
    }

    private func delete() {
        guard let id = resourceID else { return }
        library.run({ try $0.deleteResourceTemplate(id) }, done: { _ in
            library.forget(selection)
            navigator.selection = nil
        })
    }
}

/// The runtimes a resource is enabled on: a checkbox per runtime with its state
/// and, where native content changed, the user's choices.
private struct RuntimeTargetsView: View {
    @ObservedObject var store: AppStore
    @ObservedObject var library: ResourceLibraryModel
    let resourceID: String?
    let kind: BindingRuntimeResourceKind
    let name: String
    let isDirty: Bool
    @State private var disabling: RuntimeInstance?
    @State private var conflict: (runtime: RuntimeInstance, message: String)?
    @State private var bulkConflicts: [RuntimeInstance] = []
    @State private var reviewing: RuntimeInstance?
    /// Taking a runtime's native content into the resource, once the fields it
    /// cannot carry are known.
    @State private var taking: (runtime: RuntimeInstance, notes: [String])?

    private var canModify: Bool { !store.applicationIsGenerating && !library.isWorking }
    /// Runtimes that support the resource but do not have it yet.
    private var enableable: [RuntimeInstance] {
        guard let id = resourceID else { return [] }
        return store.runtimeInstances.filter { if case .notEnabled = library.state(id, $0.id) { true } else { false } }
    }

    var body: some View {
        VStack(alignment: .leading, spacing: 8) {
            HStack {
                Text("启用于").font(.headline)
                Spacer()
                if enableable.count > 1 { Button("全部启用") { enableAll() }.disabled(!canModify) }
            }
            if resourceID == nil {
                Text("创建后可以启用到运行时。").foregroundStyle(.secondary)
            } else if store.runtimeInstances.isEmpty {
                Text("还没有 Agent 运行时。").foregroundStyle(.secondary)
            } else {
                VStack(spacing: 0) {
                    ForEach(Array(store.runtimeInstances.enumerated()), id: \.element.id) { index, runtime in
                        if index > 0 { Divider() }
                        row(runtime).padding(.vertical, 6).padding(.horizontal, 10)
                    }
                }
                .background(.background, in: RoundedRectangle(cornerRadius: 6))
                .overlay(RoundedRectangle(cornerRadius: 6).stroke(.separator))
                if store.applicationIsGenerating { Text("有任务运行时不能更改启用状态；保存的修改会在“待同步”的运行时上稍后重试。").font(.caption).foregroundStyle(.secondary) }
                else if isDirty { Text("启用时写入的是已保存的内容。").font(.caption).foregroundStyle(.secondary) }
                Text("写入后，运行时通常在新会话中加载这些内容；正在进行的会话何时使用新内容由运行时决定，Velune 不会重启会话。").font(.caption).foregroundStyle(.secondary).fixedSize(horizontal: false, vertical: true)
            }
        }
        .confirmationDialog("在 \(disabling?.name ?? "") 上停用“\(name)”？", isPresented: Binding(get: { disabling != nil }, set: { if !$0 { disabling = nil } })) {
            Button("删除原生内容", role: .destructive) { if let runtime = disabling { resolve(runtime, .remove) } }
            Button("保留原生内容") { if let runtime = disabling { resolve(runtime, .unmanage) } }
            Button("取消", role: .cancel) {}
        } message: { Text("保留时，原生内容留在该运行时，但不再由 Velune 同步。") }
        .sheet(item: $reviewing) { runtime in
            if let id = resourceID {
                DriftSheet(library: library, runtime: runtime, resourceID: id, resourceName: name, canTakeNative: !isDirty,
                           resolve: { resolution in resolution == .takeNative ? take(runtime) : resolve(runtime, resolution) })
            }
        }
        .alert("覆盖已有的原生内容？", isPresented: Binding(get: { !bulkConflicts.isEmpty }, set: { if !$0 { bulkConflicts = [] } })) {
            Button("跳过这些运行时", role: .cancel) {}
            Button("覆盖", role: .destructive) { let runtimes = bulkConflicts; bulkConflicts = []; enable(runtimes, overwrite: true) }
        } message: { Text("\(bulkConflicts.map(\.name).joined(separator: "、")) 中已有同名内容\(kind == .userInstructions ? "或已启用另一份指令" : "")，启用将替换它。") }
        .alert("用 \(taking?.runtime.name ?? "") 中的内容更新“\(name)”？", isPresented: Binding(get: { taking != nil }, set: { if !$0 { taking = nil } })) {
            Button("取消", role: .cancel) {}
            Button("更新资源库") { if let runtime = taking?.runtime { resolve(runtime, .takeNative) } }
        } message: {
            let others = store.runtimeInstances.filter { runtime in runtime.id != taking?.runtime.id && resourceID.map { library.state($0, runtime.id).isEnabled } == true }.map(\.name)
            Text(["资源库中的版本将被替换\(others.isEmpty ? "" : "，并写入 \(others.joined(separator: "、"))")；\(taking?.runtime.name ?? "") 中的内容按资源库的格式重写。",
                  (taking?.notes ?? []).isEmpty ? nil : "以下原生内容无法带入，将被丢弃：\((taking?.notes ?? []).joined(separator: "；"))"].compactMap { $0 }.joined(separator: "\n\n"))
        }
        .alert("覆盖 \(conflict?.runtime.name ?? "") 中的内容？", isPresented: Binding(get: { conflict != nil }, set: { if !$0 { conflict = nil } })) {
            Button("取消", role: .cancel) {}
            Button("覆盖", role: .destructive) { if let runtime = conflict?.runtime { enable(runtime, overwrite: true) } }
        } message: { Text(conflict?.message ?? "") }
    }

    @ViewBuilder private func row(_ runtime: RuntimeInstance) -> some View {
        let state = resourceID.map { library.state($0, runtime.id) } ?? .notEnabled
        HStack(spacing: 10) {
            Toggle(isOn: Binding(get: { state.isEnabled }, set: { on in
                if on { enable(runtime, overwrite: false) } else { disabling = runtime }
            })) { Text(runtime.name).lineLimit(1) }
            .toggleStyle(.checkbox)
            .disabled(!canModify || { if case .unsupported = state { true } else { false } }())
            Spacer()
            Text(state.isEnabled || { if case .unsupported = state { true } else { false } }() ? state.title : "")
                .font(.callout).foregroundStyle(state.needsAttention ? .orange : .secondary).lineLimit(1).truncationMode(.tail)
            switch state {
            case .pending:
                Button("同步") { enable(runtime, overwrite: false) }.disabled(!canModify)
            case .drifted:
                Menu("处理…") {
                    Button("查看差异…") { reviewing = runtime }
                    Divider()
                    Button("用资源库的内容覆盖") { resolve(runtime, .overwrite) }
                    Button("用原生内容更新资源库…") { take(runtime) }
                        .disabled(isDirty).help(isDirty ? "先保存或还原未保存的修改" : "以该运行时中的内容替换资源库中的版本")
                    Button("不再同步，保留原生内容") { resolve(runtime, .unmanage) }
                }.fixedSize().disabled(!canModify)
            case .missing:
                Menu("处理…") {
                    Button("按资源库重新创建") { resolve(runtime, .overwrite) }
                    Button("不再同步") { resolve(runtime, .unmanage) }
                }.fixedSize().disabled(!canModify)
            default: EmptyView()
            }
        }
    }

    private func enableAll() { enable(enableable, overwrite: false) }

    /// Enables on several runtimes at once; those with conflicting native content are asked about together.
    private func enable(_ runtimes: [RuntimeInstance], overwrite: Bool) {
        guard let id = resourceID, !runtimes.isEmpty else { return }
        let targets = runtimes.map { BindingTemplateApplyTarget(runtimeId: $0.id, overwrite: overwrite) }
        library.run({ try $0.applyResourceTemplate(id, targets: targets) }, done: { reports in
            let conflicted = Set(reports.filter { if case .conflict = $0.outcome { true } else { false } }.map(\.runtimeId))
            bulkConflicts = runtimes.filter { conflicted.contains($0.id) }
            let failed = reports.compactMap { report -> String? in
                switch report.outcome {
                case .failed(let reason), .skipped(let reason): "\(runtimes.first { $0.id == report.runtimeId }?.name ?? report.runtimeId)：\(reason)"
                default: nil
                }
            }
            if !failed.isEmpty { library.failure = failed.joined(separator: "；") }
        })
    }

    private func enable(_ runtime: RuntimeInstance, overwrite: Bool) {
        guard let id = resourceID else { return }
        let target = BindingTemplateApplyTarget(runtimeId: runtime.id, overwrite: overwrite)
        library.run({ try $0.applyResourceTemplate(id, targets: [target]) }, done: { reports in
            guard let outcome = reports.first?.outcome else { return }
            switch outcome {
            case .conflict(let unmanaged, let drifted):
                let replaced = library.resources(kind).filter { $0.id != id && library.state($0.id, runtime.id).isEnabled }.map(\.name)
                let message = kind == .userInstructions && !replaced.isEmpty
                    ? "\(runtime.name) 已启用指令“\(replaced.joined(separator: "、"))”。每个运行时只能启用一份指令，继续将替换它。"
                    : "\(runtime.name) 中已有同名的原生内容（\((unmanaged + drifted).joined(separator: "、"))），启用后将被资源库的内容替换。"
                conflict = (runtime, message)
            case .failed(let reason), .skipped(let reason): library.failure = "\(runtime.name)：\(reason)"
            default: break
            }
        })
    }

    private func resolve(_ runtime: RuntimeInstance, _ resolution: BindingResolution) {
        guard let id = resourceID else { return }
        let runtimeID = runtime.id
        library.run({ try $0.resolveResourceBinding(runtimeID: runtimeID, templateID: id, resolution: resolution) }, done: { _ in
            // The resource itself changed; the editor shows the new version.
            if resolution == .takeNative { library.forget(.resource(id)); library.open(.resource(id)) }
        })
    }

    /// Reads what the resource would become first, to list what cannot be carried over.
    private func take(_ runtime: RuntimeInstance) {
        guard let id = resourceID, !isDirty else { return }
        let runtimeID = runtime.id
        library.run({ try $0.resourceNativeImport(runtimeID: runtimeID, templateID: id) }, done: { imported in taking = (runtime, imported.notes) })
    }
}

/// A native entry that no resource manages: shown read-only, with the one
/// thing to do with it, adding it to the library.
private struct NativeResourceView: View {
    @ObservedObject var store: AppStore
    @ObservedObject var library: ResourceLibraryModel
    @ObservedObject var navigator: ResourceLibraryNavigator
    let kind: BindingRuntimeResourceKind
    let runtimeID: String
    let path: String
    @State private var preview: TemplateDraft?
    @State private var notes: [String] = []
    @State private var confirmingAdopt = false
    @State private var confirmingReplace: BindingResourceTemplateSummary?

    private var runtimeName: String { store.runtimeInstances.first { $0.id == runtimeID }?.name ?? runtimeID }
    /// A resource with the same name: adopting would clash, enabling it replaces this entry.
    private var namesake: BindingResourceTemplateSummary? { preview.flatMap { library.summary(named: $0.name, kind: kind) } }

    var body: some View {
        Group {
            if let preview {
                VStack(spacing: 0) {
                    ScrollView {
                        VStack(alignment: .leading, spacing: 16) {
                            Text("\(runtimeName) 中的 \(path) 不在资源库中，Velune 不编辑它。加入资源库后即可编辑，并可启用到其他运行时。")
                                .foregroundStyle(.secondary).fixedSize(horizontal: false, vertical: true)
                            ResourceForm(draft: .constant(preview), canEdit: false, skillNames: library.resources(.skills).map(\.name), veluneModels: store.veluneModelNames)
                        }.padding(20)
                    }
                    Divider()
                    HStack(spacing: 8) {
                        if let message = library.failure { FailureBanner(message: message) }
                        else if let namesake { Text("资源库中已有同名的“\(namesake.name)”。").font(.callout).foregroundStyle(.secondary) }
                        Spacer()
                        if library.isWorking { ProgressView().controlSize(.small) }
                        if let namesake {
                            Button("查看“\(namesake.name)”") { navigator.selection = .resource(namesake.id) }
                            Button("改用资源库的“\(namesake.name)”…") { confirmingReplace = namesake }
                                .buttonStyle(.borderedProminent).disabled(store.applicationIsGenerating || library.isWorking)
                        } else {
                            Button("加入资源库…") { if notes.isEmpty { adopt() } else { confirmingAdopt = true } }
                                .buttonStyle(.borderedProminent).disabled(store.applicationIsGenerating || library.isWorking)
                        }
                    }.padding(12)
                }
            } else if let message = library.failure {
                ContentUnavailableView { Label("无法读取", systemImage: "exclamationmark.triangle") } description: { Text(message) }
                    .frame(maxWidth: .infinity, maxHeight: .infinity)
            } else {
                ProgressView().frame(maxWidth: .infinity, maxHeight: .infinity)
            }
        }
        .onAppear(perform: load)
        .alert("加入资源库", isPresented: $confirmingAdopt) {
            Button("取消", role: .cancel) {}
            Button("加入") { adopt() }
        } message: { Text("以下内容资源库无法表达，不会带入；\(runtimeName) 中的原生文件将改写为资源库的形式：\n" + notes.joined(separator: "\n")) }
        .alert("改用资源库的“\(confirmingReplace?.name ?? "")”？", isPresented: Binding(get: { confirmingReplace != nil }, set: { if !$0 { confirmingReplace = nil } })) {
            Button("取消", role: .cancel) {}
            Button("替换", role: .destructive) { if let namesake = confirmingReplace { replace(with: namesake) } }
        } message: { Text("\(runtimeName) 中的原生内容将被资源库的版本替换，并由 Velune 保持同步。") }
    }

    private func load() {
        let runtimeID = runtimeID, kind = kind, path = path
        library.run({ try $0.importRuntimeResource(runtimeID: runtimeID, kind: kind, path: path) }, done: { imported in
            preview = TemplateDraft(imported.template)
            notes = imported.notes
        })
    }

    private func adopt() {
        let runtimeID = runtimeID, kind = kind, path = path
        library.run({ try $0.adoptRuntimeResource(runtimeID: runtimeID, kind: kind, path: path) }, done: { summary in
            navigator.selection = .resource(summary.id)
        })
    }

    private func replace(with namesake: BindingResourceTemplateSummary) {
        let target = BindingTemplateApplyTarget(runtimeId: runtimeID, overwrite: true), id = namesake.id
        library.run({ try $0.applyResourceTemplate(id, targets: [target]) }, done: { _ in
            navigator.selection = .resource(id)
        })
    }
}

/// What changed in a runtime's native content, before deciding whether to overwrite it.
private struct DriftSheet: View {
    @ObservedObject var library: ResourceLibraryModel
    let runtime: RuntimeInstance
    let resourceID: String
    let resourceName: String
    /// False while the resource has unsaved edits.
    let canTakeNative: Bool
    let resolve: (BindingResolution) -> Void
    @Environment(\.dismiss) private var dismiss
    @State private var files: [BindingResourceDriftFile]?
    @State private var selected = 0

    var body: some View {
        VStack(alignment: .leading, spacing: 12) {
            Text("“\(resourceName)”在 \(runtime.name) 中的改动").font(.title3.bold())
            HStack(spacing: 16) {
                Label("− 当前原生内容", systemImage: "square.fill").foregroundStyle(.red.opacity(0.7))
                Label("+ 覆盖后（资源库）", systemImage: "square.fill").foregroundStyle(.green.opacity(0.7))
            }.font(.caption).labelStyle(.titleOnly)
            if let files {
                if files.isEmpty {
                    Text("没有差异。").foregroundStyle(.secondary).frame(maxWidth: .infinity, maxHeight: .infinity)
                } else {
                    if files.count > 1 {
                        Picker("文件", selection: $selected) { ForEach(Array(files.enumerated()), id: \.offset) { Text($0.element.path).tag($0.offset) } }.fixedSize()
                    } else {
                        Text(files[0].path).font(.headline)
                    }
                    let file = files[min(selected, files.count - 1)]
                    if file.binary {
                        Text("这是非文本内容，无法逐行比较。").foregroundStyle(.secondary).frame(maxWidth: .infinity, maxHeight: .infinity)
                    } else {
                        if file.native == nil { Text("原生侧没有这个文件；覆盖将创建它。").font(.callout).foregroundStyle(.secondary) }
                        if file.library == nil { Text("资源库不再包含这个文件；覆盖将删除它。").font(.callout).foregroundStyle(.secondary) }
                        TextDiffView(native: file.native, library: file.library)
                    }
                }
            } else if let message = library.failure {
                FailureBanner(message: message)
            } else {
                ProgressView().frame(maxWidth: .infinity, maxHeight: .infinity)
            }
            HStack {
                Button("不再同步，保留原生内容") { dismiss(); resolve(.unmanage) }
                Button("用原生内容更新资源库…") { dismiss(); resolve(.takeNative) }
                    .disabled(!canTakeNative).help(canTakeNative ? "以该运行时中的内容替换资源库中的版本" : "先保存或还原未保存的修改")
                Spacer()
                Button("取消") { dismiss() }.keyboardShortcut(.cancelAction)
                Button("用资源库的内容覆盖") { dismiss(); resolve(.overwrite) }.buttonStyle(.borderedProminent)
            }
        }
        .padding(20).frame(width: 720, height: 520)
        .onAppear {
            let runtimeID = runtime.id, resourceID = resourceID
            library.run({ try $0.resourceDrift(runtimeID: runtimeID, templateID: resourceID) }, done: { files = $0 })
        }
    }
}

/// Adds several native entries of one runtime to the library at once; each
/// stays enabled on that runtime.
private struct RuntimeImportSheet: View {
    @ObservedObject var store: AppStore
    @ObservedObject var library: ResourceLibraryModel
    @ObservedObject var navigator: ResourceLibraryNavigator
    @Environment(\.dismiss) private var dismiss

    struct Candidate: Identifiable {
        let kind: BindingRuntimeResourceKind
        let path: String
        let name: String
        let notes: [String]
        /// The library already has a resource of this kind and name.
        let clashes: Bool
        var id: String { "\(kind)/\(path)" }
    }

    @State private var runtimeID = ""
    @State private var candidates: [Candidate]?
    @State private var chosen: Set<String> = []
    @State private var result: String?

    var body: some View {
        VStack(alignment: .leading, spacing: 12) {
            Text("从运行时导入").font(.title3.bold())
            Picker("运行时", selection: $runtimeID) { ForEach(store.runtimeInstances) { Text($0.name).tag($0.id) } }.fixedSize()
            Text("选中的内容将加入资源库，并继续启用于该运行时；原生文件改写为资源库的形式。").font(.callout).foregroundStyle(.secondary).fixedSize(horizontal: false, vertical: true)
            Group {
                if let candidates {
                    if candidates.isEmpty {
                        ContentUnavailableView("没有可导入的内容", systemImage: "tray", description: Text("此运行时的内容都已在资源库中，或还没有内容。"))
                    } else {
                        List {
                            ForEach(BindingRuntimeResourceKind.templateKinds, id: \.self) { kind in
                                let group = candidates.filter { $0.kind == kind }
                                if !group.isEmpty {
                                    Section(kind.templateTitle) { ForEach(group) { row($0) } }
                                }
                            }
                        }
                    }
                } else {
                    ProgressView().frame(maxWidth: .infinity, maxHeight: .infinity)
                }
            }.frame(minHeight: 260, maxHeight: .infinity)
            if let result { Text(result).font(.callout).foregroundStyle(.secondary) }
            if let message = library.failure { FailureBanner(message: message) }
            HStack {
                if let candidates, candidates.contains(where: { !$0.clashes }) {
                    Button(chosen.count == candidates.filter { !$0.clashes }.count ? "全不选" : "全选") {
                        let all = Set(candidates.filter { !$0.clashes }.map(\.id))
                        chosen = chosen == all ? [] : all
                    }
                }
                Spacer()
                if library.isWorking { ProgressView().controlSize(.small) }
                Button(result == nil ? "取消" : "完成") { dismiss() }.keyboardShortcut(.cancelAction)
                Button("导入 \(chosen.count) 项") { importChosen() }.buttonStyle(.borderedProminent)
                    .disabled(chosen.isEmpty || library.isWorking || store.applicationIsGenerating)
            }
        }
        .padding(20).frame(width: 560, height: 560)
        .onAppear { runtimeID = navigator.runtimeID ?? store.runtimeInstances.first?.id ?? ""; load() }
        .onChange(of: runtimeID) { _, _ in load() }
    }

    private func row(_ candidate: Candidate) -> some View {
        Toggle(isOn: Binding(get: { chosen.contains(candidate.id) }, set: { if $0 { chosen.insert(candidate.id) } else { chosen.remove(candidate.id) } })) {
            VStack(alignment: .leading, spacing: 2) {
                Text(candidate.path)
                if candidate.clashes {
                    Text("资源库已有同名的“\(candidate.name)”，请在“运行时中的其他内容”中逐项处理").font(.caption).foregroundStyle(.secondary)
                } else if !candidate.notes.isEmpty {
                    Text("不会带入：" + candidate.notes.joined(separator: "；")).font(.caption).foregroundStyle(.orange)
                }
            }
        }
        .toggleStyle(.checkbox)
        .disabled(candidate.clashes)
    }

    private func load() {
        candidates = nil; chosen = []; result = nil
        guard !runtimeID.isEmpty else { candidates = []; return }
        let runtimeID = runtimeID
        let entries = BindingRuntimeResourceKind.templateKinds.flatMap { library.unmanaged[$0] ?? [] }.filter { $0.runtimeId == runtimeID }
        let existing = Set(library.summaries.map { "\($0.kind)/\($0.name)" })
        library.run({ transport in
            try entries.map { entry in
                let imported = try transport.importRuntimeResource(runtimeID: runtimeID, kind: entry.kind, path: entry.path)
                return (entry, imported)
            }
        }, done: { pairs in
            guard runtimeID == self.runtimeID else { return }
            let list = pairs.map { entry, imported in
                Candidate(kind: entry.kind, path: entry.path, name: imported.template.name, notes: imported.notes, clashes: existing.contains("\(entry.kind)/\(imported.template.name)"))
            }
            candidates = list
            chosen = Set(list.filter { !$0.clashes && $0.notes.isEmpty }.map(\.id))
        })
    }

    private func importChosen() {
        let runtimeID = runtimeID
        let picked = (candidates ?? []).filter { chosen.contains($0.id) }
        library.run({ transport in
            var failures: [String] = []
            for candidate in picked {
                do { _ = try transport.adoptRuntimeResource(runtimeID: runtimeID, kind: candidate.kind, path: candidate.path) }
                catch { failures.append("\(candidate.path)：\(ResourceFailure(error).message)") }
            }
            return failures
        }, done: { failures in
            let added = picked.count - failures.count
            result = failures.isEmpty ? "已导入 \(added) 项。" : "已导入 \(added) 项；未导入：" + failures.joined(separator: "；")
            candidates = (candidates ?? []).filter { candidate in
                !chosen.contains(candidate.id) || failures.contains { $0.hasPrefix(candidate.path + "：") }
            }
            chosen = []
        })
    }
}
