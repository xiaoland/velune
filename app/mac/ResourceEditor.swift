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
    var model = ""
    var tools = ""
    var skillFiles: [DraftSkillFile] = []
    var isHTTP = false
    var command = ""
    var arguments = ""
    var url = ""
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
        case .mcp(let transport):
            kind = .mcpServers
            switch transport {
            case .stdio(let command, let args, let env):
                self.command = command; arguments = args.joined(separator: "\n")
                pairs = env.sorted { $0.key < $1.key }.map { DraftKeyValue(key: $0.key, value: $0.value) }
            case .http(let url, let headers):
                isHTTP = true; self.url = url
                pairs = headers.sorted { $0.key < $1.key }.map { DraftKeyValue(key: $0.key, value: $0.value) }
            }
        case .subagent(let description, let instructions, let model, let tools):
            kind = .nativeAgentDefinitions
            self.description = description; body = instructions; self.model = model ?? ""; self.tools = tools.joined(separator: ", ")
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
            content = .mcp(transport: isHTTP
                ? .http(url: url, headers: dictionary)
                : .stdio(command: command, args: arguments.split(separator: "\n").map(String.init), env: dictionary))
        case .nativeAgentDefinitions:
            let list = tools.split(whereSeparator: { $0 == "," || $0 == "\n" }).map { $0.trimmingCharacters(in: .whitespaces) }.filter { !$0.isEmpty }
            let model = model.trimmingCharacters(in: .whitespaces)
            content = .subagent(description: description, instructions: body, model: model.isEmpty ? nil : model, tools: list)
        }
        return BindingResourceTemplate(id: id, name: name.trimmingCharacters(in: .whitespaces), content: content)
    }
}

/// The fields of one resource kind; read-only when previewing native content.
struct ResourceForm: View {
    @Binding var draft: TemplateDraft
    let canEdit: Bool
    @State private var revealSecrets = false

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
                field("描述", $draft.description)
                field("模型", $draft.model, prompt: "可选")
                field("工具", $draft.tools, prompt: "以逗号分隔，可选")
                Text("指令").font(.subheadline)
                editor($draft.body, height: 220)
            case .mcpServers: mcp
            case .skills: SkillFilesEditor(files: $draft.skillFiles, canEdit: canEdit)
            }
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
            Picker("传输", selection: $draft.isHTTP) { Text("本地进程（stdio）").tag(false); Text("HTTP").tag(true) }.pickerStyle(.segmented).fixedSize()
        } else {
            LabeledContent("传输", value: draft.isHTTP ? "HTTP" : "本地进程（stdio）")
        }
        if draft.isHTTP { field("地址", $draft.url, prompt: "https://") } else { field("命令", $draft.command) }
        if !draft.isHTTP {
            Text("参数（每行一个）").font(.subheadline)
            editor($draft.arguments, height: 80)
        }
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
