import SwiftUI
import AppKit
import VeluneBindings

/// Why a resource operation failed, as far as the editor reacts to it.
struct ResourceFailure: Equatable {
    enum Kind { case conflict, other }
    var kind: Kind
    var message: String

    init(_ error: Error) {
        var cause = error
        while let transport = cause as? TransportError, case .operation(_, let inner) = transport { cause = inner }
        if let transport = cause as? TransportError, case .diagnostic(_, let detail, let code, _, _) = transport {
            kind = code == "conflict" ? .conflict : .other
            message = detail
        } else {
            kind = .other
            message = cause.localizedDescription
        }
    }
}

/// One runtime's user instruction files. Edits are kept per file until they
/// are saved or discarded, so switching files or runtimes loses nothing.
@MainActor final class InstructionsModel: ObservableObject {
    struct Saved {
        var text: String
        var revision: BindingRuntimeResourceRevision
        /// Why Velune does not edit this entry, when it does not.
        var readOnly: String?
    }
    @Published private(set) var catalog: BindingRuntimeResourceCatalog?
    @Published private(set) var loadFailure: String?
    @Published private(set) var isLoading = false
    @Published private(set) var isWorking = false
    @Published private(set) var saved: [String: Saved] = [:]
    @Published private(set) var edits: [String: String] = [:]
    @Published private(set) var failure: ResourceFailure?
    @Published var selectedPath: String?
    private let runtimeID: String
    private let transport: Transport?
    private let queue = DispatchQueue(label: "local.velune.runtime-resources", qos: .userInitiated)
    private nonisolated static let kind = BindingRuntimeResourceKind.userInstructions
    private nonisolated static let scope = BindingRuntimeResourceScope.user

    init(runtimeID: String, transport: Transport?) { self.runtimeID = runtimeID; self.transport = transport }

    var entries: [BindingRuntimeResourceEntry] { catalog?.entries ?? [] }
    var selectedEntry: BindingRuntimeResourceEntry? { entries.first { $0.key.path == selectedPath } }

    func text(_ path: String) -> String { edits[path] ?? saved[path]?.text ?? "" }
    func isDirty(_ path: String) -> Bool { edits[path] != nil }
    func isPresent(_ entry: BindingRuntimeResourceEntry) -> Bool { if case .present = entry.state { true } else { false } }
    func isWritable(_ entry: BindingRuntimeResourceEntry) -> Bool { if case .writable = entry.access { saved[entry.key.path]?.readOnly == nil } else { false } }
    func readOnlyReason(_ entry: BindingRuntimeResourceEntry) -> String? {
        if case .readOnly(let reason) = entry.access { return reason }
        if let reason = saved[entry.key.path]?.readOnly { return reason }
        if case .invalid(let reason) = entry.state { return reason }
        return nil
    }

    func setText(_ value: String, for path: String) {
        failure = nil
        if value == (saved[path]?.text ?? "") { edits[path] = nil } else { edits[path] = value }
    }
    func revert(_ path: String) { edits[path] = nil; failure = nil }

    func load() {
        guard !isLoading else { return }
        guard let transport else { loadFailure = "本地核心未配置"; return }
        isLoading = true; loadFailure = nil
        let runtimeID = runtimeID
        queue.async { [weak self] in
            let result = Result { () -> (BindingRuntimeResourceCatalog, [String: Saved]) in
                let catalog = try transport.runtimeResourceCatalog(runtimeID: runtimeID, kind: Self.kind, scope: Self.scope)
                var saved: [String: Saved] = [:]
                for entry in catalog.entries { saved[entry.key.path] = try Self.read(entry, runtimeID: runtimeID, transport: transport) }
                return (catalog, saved)
            }
            DispatchQueue.main.async {
                guard let self else { return }
                self.isLoading = false
                switch result {
                case .success(let value):
                    self.catalog = value.0; self.saved = value.1
                    // An edit equal to what is now on disk is no longer an edit.
                    self.edits = self.edits.filter { value.1[$0.key]?.text != $0.value }
                    if self.selectedEntry == nil { self.selectedPath = value.0.entries.first(where: self.isPresent)?.key.path ?? value.0.entries.first?.key.path }
                case .failure(let error): self.loadFailure = ResourceFailure(error).message
                }
            }
        }
    }

    /// Reads one entry; absent or non-regular entries have nothing to read.
    private nonisolated static func read(_ entry: BindingRuntimeResourceEntry, runtimeID: String, transport: Transport) throws -> Saved {
        let revision = entry.revision.map { BindingRuntimeResourceRevision.match($0) } ?? .missing
        guard case .present = entry.state else { return Saved(text: "", revision: revision, readOnly: nil) }
        guard entry.node == .file else { return Saved(text: "", revision: revision, readOnly: "此条目不是普通文件，Velune 不编辑它。") }
        let document = try transport.readRuntimeResource(runtimeID: runtimeID, key: entry.key)
        let current = document.entry.revision.map { BindingRuntimeResourceRevision.match($0) } ?? .missing
        guard let text = String(data: document.bytes, encoding: .utf8) else { return Saved(text: "", revision: current, readOnly: "此文件不是有效的 UTF-8 文本，Velune 不编辑它。") }
        return Saved(text: text, revision: current, readOnly: nil)
    }

    /// Writes the edited text. `overwrite` re-reads the file first and replaces
    /// whatever external change made the earlier attempt conflict.
    func save(_ entry: BindingRuntimeResourceEntry, overwrite: Bool = false) {
        let path = entry.key.path
        guard let transport, !isWorking, let text = edits[path] ?? saved[path]?.text else { return }
        let expected = saved[path]?.revision ?? .missing
        isWorking = true; failure = nil
        let runtimeID = runtimeID
        queue.async { [weak self] in
            let result = Result { () -> Void in
                var expected = expected
                if overwrite {
                    let current = try transport.runtimeResourceCatalog(runtimeID: runtimeID, kind: Self.kind, scope: Self.scope).entries.first { $0.key.path == path }
                    expected = current?.revision.map { .match($0) } ?? .missing
                }
                _ = try transport.writeRuntimeResource(runtimeID: runtimeID, key: entry.key, expected: expected, bytes: Data(text.utf8))
            }
            DispatchQueue.main.async {
                guard let self else { return }
                self.isWorking = false
                switch result {
                case .success: self.edits[path] = nil; self.load()
                case .failure(let error): self.failure = ResourceFailure(error)
                }
            }
        }
    }

    func delete(_ entry: BindingRuntimeResourceEntry) {
        let path = entry.key.path
        guard let transport, !isWorking, let revision = saved[path]?.revision, revision != .missing else { return }
        isWorking = true; failure = nil
        let runtimeID = runtimeID
        queue.async { [weak self] in
            let result = Result { try transport.deleteRuntimeResource(runtimeID: runtimeID, key: entry.key, expected: revision) }
            DispatchQueue.main.async {
                guard let self else { return }
                self.isWorking = false
                switch result {
                case .success: self.edits[path] = nil; self.load()
                case .failure(let error): self.failure = ResourceFailure(error)
                }
            }
        }
    }
}

/// Keeps one editing model per runtime for as long as the settings window lives.
@MainActor final class RuntimeResourceModels: ObservableObject {
    private var instructions: [String: InstructionsModel] = [:]
    func instructions(for runtimeID: String, transport: Transport?) -> InstructionsModel {
        if let model = instructions[runtimeID] { return model }
        let model = InstructionsModel(runtimeID: runtimeID, transport: transport)
        instructions[runtimeID] = model
        return model
    }
}

struct InstructionsView: View {
    @ObservedObject var model: InstructionsModel
    let canModify: Bool
    @State private var confirmingDelete = false

    var body: some View {
        Group {
            if let catalog = model.catalog {
                switch catalog.support {
                case .unsupported(let reason):
                    ContentUnavailableView("此运行时不支持管理用户指令", systemImage: "doc.text", description: Text(reason))
                case .supported: editor
                }
            } else if let failure = model.loadFailure {
                ContentUnavailableView { Label("无法读取用户指令", systemImage: "exclamationmark.triangle") } description: { Text(failure) } actions: { Button("重试") { model.load() } }
            } else {
                ProgressView().frame(maxWidth: .infinity, maxHeight: .infinity)
            }
        }
        .frame(maxWidth: .infinity, maxHeight: .infinity)
        .task { if model.catalog == nil { model.load() } }
    }

    @ViewBuilder private var editor: some View {
        VStack(alignment: .leading, spacing: 10) {
            HStack(spacing: 8) {
                if model.entries.count > 1 {
                    Picker("文件", selection: $model.selectedPath) {
                        ForEach(model.entries, id: \.key.path) { entry in Text(title(entry)).tag(Optional(entry.key.path)) }
                    }.fixedSize()
                } else if let entry = model.entries.first {
                    Text(entry.key.path).font(.headline)
                }
                Spacer()
                if model.isLoading || model.isWorking { ProgressView().controlSize(.small) }
                Button { model.load() } label: { Image(systemName: "arrow.clockwise") }
                    .help("重新读取").accessibilityLabel("重新读取").disabled(model.isLoading || model.isWorking)
            }
            if let entry = model.selectedEntry { file(entry) }
        }.padding(16)
    }

    private func title(_ entry: BindingRuntimeResourceEntry) -> String {
        var value = entry.key.path
        if !model.isPresent(entry) { value += " · 未创建" }
        if model.isDirty(entry.key.path) { value += " · 已修改" }
        return value
    }

    @ViewBuilder private func file(_ entry: BindingRuntimeResourceEntry) -> some View {
        let path = entry.key.path
        let reason = model.readOnlyReason(entry)
        let editable = model.isWritable(entry) && canModify
        Text(entry.absolutePath).font(.caption).foregroundStyle(.secondary).lineLimit(1).truncationMode(.middle).textSelection(.enabled)
        PlainTextEditor(text: Binding(get: { model.text(path) }, set: { model.setText($0, for: path) }), isEditable: editable && !model.isWorking, placeholder: model.isPresent(entry) ? "" : "此文件尚未创建。输入内容并保存即可创建。")
            .id(path)
        if let failure = model.failure { failureBanner(failure, entry: entry) }
        HStack(spacing: 8) {
            status(entry, reason: reason)
            Spacer()
            if model.isPresent(entry), model.isWritable(entry) {
                Button("删除…", role: .destructive) { confirmingDelete = true }.disabled(!canModify || model.isWorking)
            }
            Button("还原") { model.revert(path) }.disabled(!model.isDirty(path) || model.isWorking)
            Button(model.isPresent(entry) ? "保存" : "创建") { model.save(entry) }
                .keyboardShortcut("s", modifiers: .command).buttonStyle(.borderedProminent)
                .disabled(!editable || !model.isDirty(path) || model.isWorking)
        }
        .alert("删除 \(path)？", isPresented: $confirmingDelete) {
            Button("取消", role: .cancel) {}
            Button("删除", role: .destructive) { model.delete(entry) }
        } message: { Text("将从运行时的用户目录永久删除此文件，无法撤销。") }
    }

    @ViewBuilder private func status(_ entry: BindingRuntimeResourceEntry, reason: String?) -> some View {
        let text: String? = if let reason { reason }
            else if !canModify { "有任务运行时不能修改" }
            else if model.isDirty(entry.key.path) { "有未保存的修改" }
            else { "用户级指令，对此运行时的所有项目与会话生效。" }
        if let text { Text(text).font(.callout).foregroundStyle(.secondary).lineLimit(2) }
    }

    @ViewBuilder private func failureBanner(_ failure: ResourceFailure, entry: BindingRuntimeResourceEntry) -> some View {
        HStack(alignment: .firstTextBaseline, spacing: 8) {
            Image(systemName: "exclamationmark.triangle.fill").foregroundStyle(.yellow)
            Text(failure.kind == .conflict ? "此文件在 Velune 之外已被修改，保存被拒绝。" : failure.message).font(.callout)
            Spacer()
            if failure.kind == .conflict {
                Button("重新载入") { model.revert(entry.key.path); model.load() }
                Button("覆盖保存") { model.save(entry, overwrite: true) }
            }
        }
        .padding(8).background(.quaternary, in: RoundedRectangle(cornerRadius: 6))
    }
}

/// Plain monospaced text with the editor's native undo; nothing is substituted
/// or corrected because the content is a file, not prose being composed.
struct PlainTextEditor: NSViewRepresentable {
    @Binding var text: String
    let isEditable: Bool
    var placeholder = ""

    func makeCoordinator() -> Coordinator { Coordinator(self) }

    func makeNSView(context: Context) -> NSScrollView {
        let scroll = ComposerFieldView()
        scroll.hasVerticalScroller = true
        let view = PlainTextView()
        view.delegate = context.coordinator
        scroll.documentView = view
        view.string = text
        // The text view fills the field, so a click below the last line still places the caret.
        scroll.contentView.postsFrameChangedNotifications = true
        context.coordinator.observation = NotificationCenter.default.addObserver(forName: NSView.frameDidChangeNotification, object: scroll.contentView, queue: .main) { [weak scroll, weak view] _ in
            MainActor.assumeIsolated { if let scroll, let view { view.fill(scroll.contentSize.height) } }
        }
        DispatchQueue.main.async { [weak scroll, weak view] in if let scroll, let view { view.fill(scroll.contentSize.height) } }
        return scroll
    }

    func updateNSView(_ scroll: NSScrollView, context: Context) {
        context.coordinator.parent = self
        guard let view = scroll.documentView as? PlainTextView else { return }
        if view.string != text { view.string = text }
        view.isEditable = isEditable
        if view.placeholder != placeholder { view.placeholder = placeholder }
    }

    @MainActor final class Coordinator: NSObject, NSTextViewDelegate {
        var parent: PlainTextEditor
        var observation: NSObjectProtocol?
        init(_ parent: PlainTextEditor) { self.parent = parent }
        deinit { if let observation { NotificationCenter.default.removeObserver(observation) } }
        func textDidChange(_ notification: Notification) {
            guard let view = notification.object as? NSTextView, view.string != parent.text else { return }
            parent.text = view.string
        }
    }
}

@MainActor final class PlainTextView: NSTextView {
    var placeholder = "" { didSet { needsDisplay = true } }
    init() {
        let storage = NSTextStorage()
        let layoutManager = NSLayoutManager()
        storage.addLayoutManager(layoutManager)
        let container = NSTextContainer(size: NSSize(width: 100, height: CGFloat.greatestFiniteMagnitude))
        container.widthTracksTextView = true
        layoutManager.addTextContainer(container)
        super.init(frame: .zero, textContainer: container)
        let font = NSFont.monospacedSystemFont(ofSize: NSFont.systemFontSize, weight: .regular)
        self.font = font
        textColor = .labelColor
        typingAttributes = [.font: font, .foregroundColor: NSColor.labelColor]
        drawsBackground = false
        isRichText = false
        importsGraphics = false
        allowsUndo = true
        isAutomaticQuoteSubstitutionEnabled = false
        isAutomaticDashSubstitutionEnabled = false
        isAutomaticTextReplacementEnabled = false
        isAutomaticSpellingCorrectionEnabled = false
        isAutomaticLinkDetectionEnabled = false
        isAutomaticDataDetectionEnabled = false
        smartInsertDeleteEnabled = false
        textContainerInset = NSSize(width: 4, height: 6)
        isVerticallyResizable = true
        isHorizontallyResizable = false
        minSize = .zero
        maxSize = NSSize(width: CGFloat.greatestFiniteMagnitude, height: CGFloat.greatestFiniteMagnitude)
        autoresizingMask = [.width]
    }
    override init(frame frameRect: NSRect, textContainer container: NSTextContainer?) { super.init(frame: frameRect, textContainer: container) }
    required init?(coder: NSCoder) { fatalError("init(coder:) is unavailable") }

    func fill(_ height: CGFloat) {
        minSize = NSSize(width: 0, height: height)
        if frame.height < height { setFrameSize(NSSize(width: frame.width, height: height)) }
    }

    override func draw(_ dirtyRect: NSRect) {
        super.draw(dirtyRect)
        guard string.isEmpty, !placeholder.isEmpty, let textContainer else { return }
        let origin = textContainerOrigin
        let padding = textContainer.lineFragmentPadding
        let rect = NSRect(x: origin.x + padding, y: origin.y, width: bounds.width - origin.x * 2 - padding * 2, height: bounds.height - origin.y)
        NSAttributedString(string: placeholder, attributes: [.font: font ?? .systemFont(ofSize: NSFont.systemFontSize), .foregroundColor: NSColor.placeholderTextColor]).draw(in: rect)
    }
    override func didChangeText() { super.didChangeText(); needsDisplay = true }
}

/// Which runtime the resources window shows; entry points elsewhere set it
/// before opening the window.
@MainActor final class RuntimeResourcesNavigator: ObservableObject {
    static let shared = RuntimeResourcesNavigator()
    @Published var runtimeID: String?
    @Published var templateID: String?
    func show(_ runtimeID: String, _ openWindow: OpenWindowAction) {
        self.runtimeID = runtimeID
        openWindow(id: "runtime-resources")
    }
}

struct RuntimeResourcesMenuItem: View {
    @Environment(\.openWindow) private var openWindow
    var body: some View { Button("运行时资源…") { openWindow(id: "runtime-resources") } }
}

/// Native instructions, skills, MCP servers and sub-agent definitions of each
/// runtime. These are large editing surfaces, so they live in a resizable
/// window of their own rather than in the compact settings.
struct RuntimeResourcesView: View {
    @ObservedObject var store: AppStore
    @ObservedObject private var navigator = RuntimeResourcesNavigator.shared
    @StateObject private var resources = RuntimeResourceModels()
    @StateObject private var library: TemplateLibrary
    @State private var section = RuntimeSection.overview
    @State private var editor: RuntimeInstance?
    init(store: AppStore) { self.store = store; _library = StateObject(wrappedValue: TemplateLibrary(transport: store.resourceTransport)) }
    private var selected: RuntimeInstance? { store.runtimeInstances.first { $0.id == navigator.runtimeID } }

    var body: some View {
        NavigationSplitView {
            List(selection: Binding<String?>(get: { navigator.templateID.map { "t:" + $0 } ?? navigator.runtimeID.map { "r:" + $0 } }, set: { value in
                if let value, value.hasPrefix("t:") { navigator.templateID = String(value.dropFirst(2)); navigator.runtimeID = nil }
                else if let value { navigator.runtimeID = String(value.dropFirst(2)); navigator.templateID = nil }
            })) {
                Section {
                    ForEach(store.runtimeInstances) { instance in
                        VStack(alignment: .leading, spacing: 3) {
                            HStack { Text(instance.name).lineLimit(1); if !instance.enabled { Text("已停用").foregroundStyle(.secondary) } }
                            Text(store.runtimeTypes.first { $0.id == instance.typeID }?.name ?? instance.typeID).font(.caption).foregroundStyle(.secondary).lineLimit(1)
                        }.padding(.vertical, 2).tag("r:" + instance.id)
                    }
                } header: { Text("运行时") }
                Section {
                    ForEach(library.summaries, id: \.id) { item in
                        Label {
                            HStack { Text(item.name).lineLimit(1); if library.hasUpdates(item.id) { Spacer(); Circle().fill(.orange).frame(width: 7, height: 7).help("有运行时尚未同步此模板的更新") } }
                        } icon: { Image(systemName: item.kind.templateSymbol) }.tag("t:" + item.id)
                    }
                } header: {
                    HStack {
                        Text("模板")
                        Spacer()
                        Menu {
                            ForEach(BindingRuntimeResourceKind.templateKinds, id: \.self) { kind in
                                Button { navigator.templateID = TemplateSelection.new(kind); navigator.runtimeID = nil } label: { Label(kind.templateTitle, systemImage: kind.templateSymbol) }
                            }
                        } label: { Image(systemName: "plus") }.menuStyle(.borderlessButton).fixedSize().help("新建模板").accessibilityLabel("新建模板")
                    }
                }
            }
            .listStyle(.sidebar)
            .navigationSplitViewColumnWidth(min: 190, ideal: 230, max: 300)
        } detail: {
            if let template = navigator.templateID {
                TemplateEditorView(library: library, store: store, selection: template, canModify: !store.applicationIsGenerating, select: { navigator.templateID = $0; if $0 == nil { navigator.runtimeID = store.runtimeInstances.first?.id } }).id(template)
            } else if let instance = selected {
                RuntimeDetailView(store: store, instance: instance, section: $section, resources: resources, library: library, edit: { editor = instance }).id(instance.id)
            } else {
                ContentUnavailableView("选择一个 Agent 运行时", systemImage: "terminal", description: Text("在左侧选择运行时，管理它的原生指令与其他资源。"))
            }
        }
        .navigationTitle(navigator.templateID != nil ? "运行时资源 — 模板" : selected.map { "运行时资源 — \($0.name)" } ?? "运行时资源")
        .onAppear {
            if selected == nil && navigator.templateID == nil { navigator.runtimeID = store.runtimeInstances.first?.id }
            library.reload(runtimeIDs: store.runtimeInstances.map(\.id))
        }
        .onChange(of: store.runtimeInstances) { _, instances in
            if navigator.templateID == nil, !instances.contains(where: { $0.id == navigator.runtimeID }) { navigator.runtimeID = instances.first?.id }
            library.reload(runtimeIDs: instances.map(\.id))
        }
        .sheet(item: $editor) { instance in RuntimeEditor(store: store, instance: instance) }
    }
}

enum RuntimeSection: Hashable {
    case overview, instructions, templates
    var title: String { switch self { case .overview: "概览"; case .instructions: "指令"; case .templates: "模板" } }
    static let all: [RuntimeSection] = [.overview, .instructions, .templates]
}

/// The selected runtime: its configuration summary and the native resources
/// it owns, one section at a time.
private struct RuntimeDetailView: View {
    @ObservedObject var store: AppStore
    let instance: RuntimeInstance
    @Binding var section: RuntimeSection
    @ObservedObject var resources: RuntimeResourceModels
    @ObservedObject var library: TemplateLibrary
    let edit: () -> Void
    private var descriptor: RuntimeTypeDescriptor? { store.runtimeTypes.first { $0.id == instance.typeID } }

    var body: some View {
        VStack(spacing: 0) {
            HStack(alignment: .firstTextBaseline) {
                VStack(alignment: .leading, spacing: 2) {
                    Text(instance.name).font(.title2.bold()).lineLimit(1)
                    Text(descriptor?.name ?? instance.typeID).foregroundStyle(.secondary)
                }
                Spacer()
                Button("编辑配置…", action: edit)
            }.padding([.horizontal, .top], 16)
            Picker("内容", selection: $section) { ForEach(RuntimeSection.all, id: \.self) { Text($0.title).tag($0) } }
                .pickerStyle(.segmented).labelsHidden().fixedSize().padding(.top, 12).padding(.bottom, 8)
            Divider()
            switch section {
            case .overview: overview
            case .instructions:
                InstructionsView(model: resources.instructions(for: instance.id, transport: store.resourceTransport), canModify: !store.applicationIsGenerating)
            case .templates:
                RuntimeTemplatesView(store: store, library: library, instance: instance, canModify: !store.applicationIsGenerating)
            }
        }.frame(maxHeight: .infinity, alignment: .top)
    }

    private var overview: some View {
        Form {
            Section {
                LabeledContent("类型", value: descriptor?.name ?? instance.typeID)
                LabeledContent("状态", value: instance.enabled ? "已启用" : "已停用")
            }
            let fields = (descriptor?.fields ?? []).filter { !(instance.settings[$0.key] ?? "").isEmpty }
            if !fields.isEmpty {
                Section("配置") {
                    ForEach(fields) { field in
                        LabeledContent(field.label) { Text(instance.settings[field.key] ?? "").lineLimit(2).truncationMode(.middle).textSelection(.enabled) }
                    }
                }
            }
        }.formStyle(.grouped)
    }
}
