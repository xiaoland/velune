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

extension BindingRuntimeResourceKind {
    var templateTitle: String {
        switch self {
        case .userInstructions: "指令"
        case .mcpServers: "MCP 服务器"
        case .skills: "技能"
        case .nativeAgentDefinitions: "子 Agent"
        }
    }
    var templateSymbol: String {
        switch self {
        case .userInstructions: "doc.text"
        case .mcpServers: "server.rack"
        case .skills: "wand.and.stars"
        case .nativeAgentDefinitions: "person.2"
        }
    }
    static let templateKinds: [BindingRuntimeResourceKind] = [.userInstructions, .skills, .mcpServers, .nativeAgentDefinitions]
}

/// One file of a skill bundle while it is being edited. Binary files are kept
/// as bytes and are not editable as text.
struct DraftSkillFile: Identifiable, Equatable {
    let id = UUID()
    var path: String
    var text: String?
    var bytes: Data
    var executable: Bool
    init(path: String, text: String, executable: Bool = false) { self.path = path; self.text = text; self.bytes = Data(); self.executable = executable }
    init(path: String, bytes: Data, executable: Bool) {
        self.path = path; self.bytes = bytes; self.executable = executable
        text = String(data: bytes, encoding: .utf8)
    }
    var encoded: Data { text.map { Data($0.utf8) } ?? bytes }
}

struct DraftKeyValue: Identifiable, Equatable {
    let id = UUID()
    var key: String
    var value: String
}

/// A resource as the editor holds it: flat fields for every kind, of which
/// only those of `kind` are used.
struct TemplateDraft: Equatable {
    var id = ""
    var kind: BindingRuntimeResourceKind
    var name = ""
    var body = ""
    var description = ""
    /// Where a sub-agent's model comes from; `model` holds a native ID,
    /// `veluneModel` a Velune model record key.
    enum ModelSource: Hashable { case inherit, native, velune }
    var modelSource = ModelSource.inherit
    var model = ""
    var veluneModel = ""
    var tools = ""
    /// Empty leaves the level to each runtime.
    var effort = ""
    var skills: [String] = []
    var skillFiles: [DraftSkillFile] = []
    var isHTTP = false
    var command = ""
    var arguments = ""
    var url = ""
    var cwd = ""
    /// Seconds, as typed; empty leaves each runtime's default.
    var timeout = ""
    /// Environment values (stdio) or request headers (http); either may be secret.
    var pairs: [DraftKeyValue] = []

    init(kind: BindingRuntimeResourceKind) {
        self.kind = kind
        if kind == .skills { skillFiles = [DraftSkillFile(path: "SKILL.md", text: "---\nname: \ndescription: \n---\n\n")] }
    }

    init(_ template: BindingResourceTemplate) {
        id = template.id; name = template.name
        kind = .userInstructions
        switch template.content {
        case .instructions(let body): kind = .userInstructions; self.body = body
        case .skill(let files):
            kind = .skills
            skillFiles = files.map { DraftSkillFile(path: $0.path, bytes: $0.bytes, executable: $0.executable) }
        case .mcp(let transport, let timeoutSeconds):
            kind = .mcpServers
            timeout = timeoutSeconds.map(String.init) ?? ""
            switch transport {
            case .stdio(let command, let args, let env, let cwd):
                self.command = command; arguments = args.joined(separator: "\n"); self.cwd = cwd ?? ""
                pairs = env.sorted { $0.key < $1.key }.map { DraftKeyValue(key: $0.key, value: $0.value) }
            case .http(let url, let headers):
                isHTTP = true; self.url = url
                pairs = headers.sorted { $0.key < $1.key }.map { DraftKeyValue(key: $0.key, value: $0.value) }
            }
        case .subagent(let description, let instructions, let model, let veluneModel, let tools, let effort, let skills):
            kind = .nativeAgentDefinitions
            self.description = description; body = instructions; self.tools = tools.joined(separator: ", ")
            self.model = model ?? ""; self.veluneModel = veluneModel ?? ""
            modelSource = veluneModel != nil ? .velune : model != nil ? .native : .inherit
            self.effort = effort ?? ""; self.skills = skills
        }
    }

    var template: BindingResourceTemplate {
        let content: BindingResourceTemplateContent
        switch kind {
        case .userInstructions: content = .instructions(body: body)
        case .skills: content = .skill(files: skillFiles.map { BindingTemplateSkillFile(path: $0.path, bytes: $0.encoded, executable: $0.executable) })
        case .mcpServers:
            var dictionary: [String: String] = [:]
            for pair in pairs where !pair.key.isEmpty { dictionary[pair.key] = pair.value }
            let cwd = cwd.trimmingCharacters(in: .whitespaces)
            content = .mcp(transport: isHTTP
                ? .http(url: url, headers: dictionary)
                : .stdio(command: command, args: arguments.split(separator: "\n").map(String.init), env: dictionary, cwd: cwd.isEmpty ? nil : cwd),
                timeoutSeconds: UInt32(timeout.trimmingCharacters(in: .whitespaces)))
        case .nativeAgentDefinitions:
            let list = tools.split(whereSeparator: { $0 == "," || $0 == "\n" }).map { $0.trimmingCharacters(in: .whitespaces) }.filter { !$0.isEmpty }
            let model = model.trimmingCharacters(in: .whitespaces)
            content = .subagent(description: description, instructions: body,
                                model: modelSource == .native && !model.isEmpty ? model : nil,
                                veluneModel: modelSource == .velune && !veluneModel.isEmpty ? veluneModel : nil,
                                tools: list, effort: effort.isEmpty ? nil : effort, skills: skills)
        }
        return BindingResourceTemplate(id: id, name: name.trimmingCharacters(in: .whitespaces), content: content)
    }
}

extension TemplateDraft {
    /// The MCP declaration as editable JSON, in the common `command`/`url` shape.
    var mcpJSON: String {
        var object: [String: Any] = [:]
        let pairs = Dictionary(self.pairs.filter { !$0.key.isEmpty }.map { ($0.key, $0.value) }, uniquingKeysWith: { _, last in last })
        if isHTTP {
            object["url"] = url
            if !pairs.isEmpty { object["headers"] = pairs }
        } else {
            object["command"] = command
            let args = arguments.split(separator: "\n").map(String.init)
            if !args.isEmpty { object["args"] = args }
            if !pairs.isEmpty { object["env"] = pairs }
            if !cwd.isEmpty { object["cwd"] = cwd }
        }
        if let seconds = UInt32(timeout) { object["timeoutSeconds"] = seconds }
        let data = (try? JSONSerialization.data(withJSONObject: object, options: [.prettyPrinted, .sortedKeys, .withoutEscapingSlashes])) ?? Data()
        return String(data: data, encoding: .utf8) ?? ""
    }

    /// Reads a pasted declaration: one server object, or `{"mcpServers": {name: {...}}}` with one
    /// server, whose name fills an empty name. Returns the fields it ignored.
    mutating func applyMCPJSON(_ text: String) throws -> [String] {
        guard let root = try JSONSerialization.jsonObject(with: Data(text.utf8)) as? [String: Any] else { throw MCPJSONError.notObject }
        var object = root
        if let servers = root["mcpServers"] as? [String: Any] {
            guard servers.count == 1, let (serverName, value) = servers.first, let server = value as? [String: Any] else { throw MCPJSONError.serverCount }
            if name.trimmingCharacters(in: .whitespaces).isEmpty { name = serverName }
            object = server
        }
        let strings = { (value: Any?) -> [DraftKeyValue] in
            ((value as? [String: Any]) ?? [:]).compactMap { key, value in (value as? String).map { DraftKeyValue(key: key, value: $0) } }.sorted { $0.key < $1.key }
        }
        if let url = object["url"] as? String {
            isHTTP = true; self.url = url
            pairs = strings(object["headers"] ?? object["http_headers"])
        } else if let command = object["command"] as? String {
            isHTTP = false; self.command = command
            arguments = ((object["args"] as? [Any]) ?? []).compactMap { $0 as? String }.joined(separator: "\n")
            pairs = strings(object["env"]); cwd = object["cwd"] as? String ?? ""
        } else {
            throw MCPJSONError.noTransport
        }
        timeout = (object["timeoutSeconds"] as? NSNumber).map { String($0.uint32Value) } ?? ""
        let known: Set<String> = ["type", "url", "headers", "http_headers", "command", "args", "env", "cwd", "timeoutSeconds"]
        return object.keys.filter { !known.contains($0) }.sorted()
    }
}

enum MCPJSONError: LocalizedError {
    case notObject, serverCount, noTransport
    var errorDescription: String? {
        switch self {
        case .notObject: "需要一个 JSON 对象。"
        case .serverCount: "mcpServers 中应只有一个服务器。"
        case .noTransport: "需要 command（本地进程）或 url（HTTP）。"
        }
    }
}

/// The fields of one resource kind; read-only when previewing native content.
struct ResourceForm: View {
    @Binding var draft: TemplateDraft
    let canEdit: Bool
    /// Names of the library's skills, which a sub-agent can load.
    var skillNames: [String] = []
    /// Velune's models a sub-agent can run: record key and display name.
    var veluneModels: [(key: String, name: String)] = []
    @State private var revealSecrets = false
    @State private var showsJSON = false
    @State private var jsonText = ""
    @State private var jsonMessage: String?
    /// The draft the JSON text last produced; any other change (revert, a new
    /// selection) re-renders the text.
    @State private var jsonDraft: TemplateDraft?

    var body: some View {
        VStack(alignment: .leading, spacing: 12) {
            HStack(spacing: 8) {
                Image(systemName: draft.kind.templateSymbol).foregroundStyle(.secondary)
                Text(draft.kind.templateTitle).font(.headline)
            }
            field(draft.kind == .userInstructions ? "标题" : "名称", $draft.name)
            switch draft.kind {
            case .userInstructions:
                Text("启用后整体写入运行时的用户指令文件；每个运行时只能启用一份指令。").font(.callout).foregroundStyle(.secondary)
                editor($draft.body, height: 280)
            case .nativeAgentDefinitions:
                Text("描述").font(.subheadline)
                editor($draft.description, height: 64)
                Text("运行时据此决定何时委派给它。").font(.caption).foregroundStyle(.secondary)
                modelField
                effortField
                field("工具", $draft.tools, prompt: "以逗号分隔，可选")
                skillsField
                Text("指令").font(.subheadline)
                editor($draft.body, height: 220)
            case .mcpServers: mcp
            case .skills: SkillFilesEditor(files: $draft.skillFiles, canEdit: canEdit)
            }
        }
        .onChange(of: draft) { _, draft in
            if showsJSON && draft != jsonDraft { jsonText = draft.mcpJSON; jsonDraft = draft; jsonMessage = nil }
        }
    }

    /// Inherit the parent's model, a Velune model supplied to gateway turns
    /// only, or a native ID written into the definition.
    @ViewBuilder private var modelField: some View {
        let veluneName = veluneModels.first { $0.key == draft.veluneModel }?.name
        if canEdit {
            Picker("模型", selection: Binding(get: { draft.modelSource == .velune ? "velune:\(draft.veluneModel)" : draft.modelSource == .native ? "native" : "inherit" }, set: { tag in
                if tag == "inherit" { draft.modelSource = .inherit }
                else if tag == "native" { draft.modelSource = .native }
                else { draft.modelSource = .velune; draft.veluneModel = String(tag.dropFirst("velune:".count)) }
            })) {
                Text("沿用父会话").tag("inherit")
                Text("原生模型 ID").tag("native")
                Section("Velune 提供商") {
                    ForEach(veluneModels, id: \.key) { Text($0.name).tag("velune:\($0.key)") }
                    if draft.modelSource == .velune && veluneName == nil { Text("已删除的模型").tag("velune:\(draft.veluneModel)") }
                }
            }
            if draft.modelSource == .native {
                field("模型 ID", $draft.model, prompt: "运行时提供商的 model ID")
            }
            Text(modelExplanation).font(.caption).foregroundStyle(.secondary).fixedSize(horizontal: false, vertical: true)
        } else {
            LabeledContent("模型", value: draft.modelSource == .native ? draft.model : draft.modelSource == .velune ? (veluneName ?? "已删除的模型") : "沿用父会话")
        }
    }

    private var modelExplanation: String {
        switch draft.modelSource {
        case .inherit: "与启动它的会话使用同一个模型。"
        case .native: "写入运行时的定义，由运行时自己的提供商解析；经 Velune 网关的轮次中改由 Velune 中唯一具有此 model ID 的模型提供。"
        case .velune: "不写入运行时的定义，只提供给经 Velune 网关的轮次；运行时使用自己的提供商时不会用到这个子 Agent。"
        }
    }

    /// The levels runtimes accept in an agent definition; a runtime that does
    /// not accept the chosen one skips it and says so when it is written.
    private static let efforts = ["none", "minimal", "low", "medium", "high", "xhigh", "max"]

    @ViewBuilder private var effortField: some View {
        if canEdit {
            Picker("推理强度", selection: $draft.effort) {
                Text("不指定").tag("")
                Divider()
                ForEach(Self.efforts, id: \.self) { Text($0).tag($0) }
                if !draft.effort.isEmpty && !Self.efforts.contains(draft.effort) { Text(draft.effort).tag(draft.effort) }
            }
            Text("Codex 接受 none 至 max，Claude Code 接受 low 至 max；运行时不接受的级别不写入。").font(.caption).foregroundStyle(.secondary)
        } else {
            LabeledContent("推理强度", value: draft.effort.isEmpty ? "—" : draft.effort)
        }
    }

    /// The library's skills, plus names the definition lists that the library does not hold.
    @ViewBuilder private var skillsField: some View {
        let names = skillNames + draft.skills.filter { !skillNames.contains($0) }.sorted()
        LabeledContent("技能") {
            if names.isEmpty {
                Text(canEdit ? "资源库中还没有技能" : "—").foregroundStyle(.secondary)
            } else if canEdit {
                VStack(alignment: .leading, spacing: 4) {
                    ForEach(names, id: \.self) { name in
                        Toggle(isOn: Binding(get: { draft.skills.contains(name) }, set: { on in
                            if on { draft.skills.append(name) } else { draft.skills.removeAll { $0 == name } }
                        })) {
                            Text(name) + Text(skillNames.contains(name) ? "" : "（不在资源库中）").foregroundStyle(.secondary)
                        }.toggleStyle(.checkbox)
                    }
                }
            } else {
                Text(draft.skills.isEmpty ? "—" : draft.skills.joined(separator: "、")).textSelection(.enabled).frame(maxWidth: .infinity, alignment: .leading)
            }
        }
        if canEdit {
            Text("子 Agent 启动时载入所选技能；目前只有 Claude Code 支持，技能也需启用于同一运行时。").font(.caption).foregroundStyle(.secondary)
        }
    }

    /// An editable field, or its value as text when previewing.
    @ViewBuilder private func field(_ label: String, _ text: Binding<String>, prompt: String = "") -> some View {
        if canEdit {
            LabeledContent(label) { TextField(prompt, text: text).labelsHidden().textFieldStyle(.roundedBorder) }
        } else {
            LabeledContent(label) { Text(text.wrappedValue.isEmpty ? "—" : text.wrappedValue).textSelection(.enabled).frame(maxWidth: .infinity, alignment: .leading) }
        }
    }

    private func editor(_ text: Binding<String>, height: CGFloat) -> some View {
        PlainTextEditor(text: text, isEditable: canEdit)
            .frame(height: height)
            .background(Color(nsColor: .textBackgroundColor), in: RoundedRectangle(cornerRadius: 6))
            .overlay(RoundedRectangle(cornerRadius: 6).stroke(.separator))
    }

    @ViewBuilder private var mcp: some View {
        if canEdit {
            Picker("编辑方式", selection: Binding(get: { showsJSON }, set: { json in
                if json { jsonText = draft.mcpJSON; jsonDraft = draft; jsonMessage = nil }
                showsJSON = json
            })) { Text("表单").tag(false); Text("JSON").tag(true) }.pickerStyle(.segmented).fixedSize()
        }
        if showsJSON && canEdit { mcpJSONEditor } else { mcpForm }
    }

    @ViewBuilder private var mcpJSONEditor: some View {
        Text("可直接粘贴 MCP 文档中的声明（含 mcpServers 的片段亦可）；超时以 timeoutSeconds 表示，单位为秒。").font(.callout).foregroundStyle(.secondary).fixedSize(horizontal: false, vertical: true)
        PlainTextEditor(text: Binding(get: { jsonText }, set: { text in
            jsonText = text
            var updated = draft
            do {
                let ignored = try updated.applyMCPJSON(text)
                jsonDraft = updated
                draft = updated
                jsonMessage = ignored.isEmpty ? nil : "已忽略不支持的字段：\(ignored.joined(separator: "、"))"
            } catch { jsonMessage = error.localizedDescription }
        }), isEditable: true)
        .frame(height: 260)
        .background(Color(nsColor: .textBackgroundColor), in: RoundedRectangle(cornerRadius: 6))
        .overlay(RoundedRectangle(cornerRadius: 6).stroke(.separator))
        if let jsonMessage { Text(jsonMessage).font(.caption).foregroundStyle(.orange) }
        Text("JSON 含环境变量与请求头的值，可能包含密钥。").font(.caption).foregroundStyle(.secondary)
    }

    @ViewBuilder private var mcpForm: some View {
        if canEdit {
            Picker("传输", selection: $draft.isHTTP) { Text("本地进程（stdio）").tag(false); Text("HTTP").tag(true) }.pickerStyle(.segmented).fixedSize()
        } else {
            LabeledContent("传输", value: draft.isHTTP ? "HTTP" : "本地进程（stdio）")
        }
        if draft.isHTTP { field("地址", $draft.url, prompt: "https://") } else { field("命令", $draft.command) }
        if !draft.isHTTP {
            Text("参数（每行一个）").font(.subheadline)
            editor($draft.arguments, height: 80)
            field("工作目录", $draft.cwd, prompt: "可选，绝对路径")
            if !draft.cwd.isEmpty { Text("Claude Code 的 MCP 配置没有工作目录，写入 Claude Code 时会省略。").font(.caption).foregroundStyle(.secondary) }
        }
        field("超时（秒）", $draft.timeout, prompt: "可选，单次工具调用的上限")
        HStack {
            Text(draft.isHTTP ? "请求头" : "环境变量").font(.subheadline)
            Spacer()
            Toggle("显示值", isOn: $revealSecrets).toggleStyle(.checkbox)
            if canEdit { Button { draft.pairs.append(DraftKeyValue(key: "", value: "")) } label: { Image(systemName: "plus") }.help("添加").accessibilityLabel("添加") }
        }
        ForEach($draft.pairs) { $pair in
            HStack {
                TextField("名称", text: $pair.key).textFieldStyle(.roundedBorder).frame(maxWidth: 180).disabled(!canEdit)
                if revealSecrets { TextField("值", text: $pair.value).disabled(!canEdit).textFieldStyle(.roundedBorder) } else { SecureField("值", text: $pair.value).textFieldStyle(.roundedBorder).disabled(!canEdit) }
                if canEdit { Button { draft.pairs.removeAll { $0.id == pair.id } } label: { Image(systemName: "minus.circle") }.buttonStyle(.borderless).help("移除").accessibilityLabel("移除") }
            }
        }
        Text("值可能含密钥，只保存在 Velune 的私有文件中，默认遮盖。").font(.caption).foregroundStyle(.secondary)
    }
}

struct SkillFilesEditor: View {
    @Binding var files: [DraftSkillFile]
    let canEdit: Bool
    @State private var selected: UUID?

    var body: some View {
        HStack(alignment: .top, spacing: 12) {
            VStack(alignment: .leading, spacing: 6) {
                List(selection: $selected) {
                    ForEach(files) { file in Text(file.path.isEmpty ? "未命名" : file.path).lineLimit(1).truncationMode(.middle).tag(file.id) }
                }
                .listStyle(.bordered)
                .frame(height: 260)
                if canEdit { HStack {
                    Button { let file = DraftSkillFile(path: "新文件.md", text: ""); files.append(file); selected = file.id } label: { Image(systemName: "plus") }.help("添加文件").accessibilityLabel("添加文件")
                    Button { files.removeAll { $0.id == selected }; selected = files.first?.id } label: { Image(systemName: "minus") }
                        .help("移除文件").accessibilityLabel("移除文件").disabled(selected == nil || files.first(where: { $0.id == selected })?.path == "SKILL.md")
                } }
            }.frame(width: 190)
            if let index = files.firstIndex(where: { $0.id == selected }) {
                VStack(alignment: .leading, spacing: 8) {
                    TextField("路径", text: $files[index].path).textFieldStyle(.roundedBorder).disabled(!canEdit || files[index].path == "SKILL.md")
                    if files[index].text != nil {
                        PlainTextEditor(text: Binding(get: { files[index].text ?? "" }, set: { files[index].text = $0 }), isEditable: canEdit)
                            .frame(height: 220)
                            .background(Color(nsColor: .textBackgroundColor), in: RoundedRectangle(cornerRadius: 6))
                            .overlay(RoundedRectangle(cornerRadius: 6).stroke(.separator))
                    } else {
                        Text("二进制文件（\(files[index].bytes.count) 字节），原样写入。").foregroundStyle(.secondary).frame(maxWidth: .infinity, minHeight: 220)
                    }
                    Toggle("可执行", isOn: $files[index].executable).disabled(!canEdit)
                }
            } else {
                Text("选择一个文件").foregroundStyle(.secondary).frame(maxWidth: .infinity, minHeight: 260)
            }
        }.onAppear { selected = files.first?.id }
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

/// Line differences between the native text and what the library would write.
struct TextDiffView: View {
    let native: String?
    let library: String?

    private enum Mark { case same, removed, added }
    private var lines: [(mark: Mark, text: String)] {
        let old = (native ?? "").components(separatedBy: "\n"), new = (library ?? "").components(separatedBy: "\n")
        let difference = new.difference(from: old)
        var removed = Set<Int>(), inserted = Set<Int>()
        for change in difference {
            switch change {
            case .remove(let offset, _, _): removed.insert(offset)
            case .insert(let offset, _, _): inserted.insert(offset)
            }
        }
        var result: [(Mark, String)] = []
        var i = 0, j = 0
        while i < old.count || j < new.count {
            if i < old.count, removed.contains(i) { result.append((.removed, old[i])); i += 1 }
            else if j < new.count, inserted.contains(j) { result.append((.added, new[j])); j += 1 }
            else if i < old.count { result.append((.same, old[i])); i += 1; j += 1 }
            else { break }
        }
        return result
    }

    var body: some View {
        ScrollView {
            VStack(alignment: .leading, spacing: 0) {
                ForEach(Array(lines.enumerated()), id: \.offset) { _, line in
                    HStack(alignment: .firstTextBaseline, spacing: 6) {
                        Text(line.mark == .removed ? "−" : line.mark == .added ? "+" : " ")
                            .foregroundStyle(line.mark == .removed ? Color.red : line.mark == .added ? Color.green : Color.secondary)
                        Text(line.text.isEmpty ? " " : line.text).fixedSize(horizontal: false, vertical: true)
                    }
                    .font(.system(.body, design: .monospaced))
                    .frame(maxWidth: .infinity, alignment: .leading)
                    .padding(.horizontal, 6)
                    .background(line.mark == .removed ? Color.red.opacity(0.12) : line.mark == .added ? Color.green.opacity(0.12) : .clear)
                }
            }.textSelection(.enabled).padding(.vertical, 4).frame(maxWidth: .infinity, alignment: .topLeading)
        }
        .background(Color(nsColor: .textBackgroundColor), in: RoundedRectangle(cornerRadius: 6))
        .overlay(RoundedRectangle(cornerRadius: 6).stroke(.separator))
    }
}
