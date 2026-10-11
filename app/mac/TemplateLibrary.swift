import Foundation
import VeluneBindings

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

extension BindingTemplateSyncState {
    var title: String {
        switch self {
        case .synced: "已同步"
        case .templateUpdated: "模板有更新"
        case .drifted: "原生已被修改"
        case .missing: "原生已被删除"
        case .skipped(let reason): "不适用：\(reason)"
        }
    }
    var needsAttention: Bool {
        switch self {
        case .drifted, .missing: true
        default: false
        }
    }
    var isTemplateUpdated: Bool { if case .templateUpdated = self { true } else { false } }
}

extension BindingTemplateApplyOutcome {
    func title(applied: Bool = false) -> String {
        switch self {
        case .create: applied ? "已创建" : "将创建"
        case .update: applied ? "已更新" : "将更新"
        case .unchanged: "已是最新"
        case .conflict(let unmanaged, let drifted):
            "冲突：" + (unmanaged.isEmpty ? "" : "已有非受管条目 \(unmanaged.joined(separator: "、"))") + (!unmanaged.isEmpty && !drifted.isEmpty ? "；" : "") + (drifted.isEmpty ? "" : "原生已被修改 \(drifted.joined(separator: "、"))")
        case .skipped(let reason): "不适用：\(reason)"
        case .failed(let reason): "失败：\(reason)"
        }
    }
    var isConflict: Bool { if case .conflict = self { true } else { false } }
    var isFailure: Bool { if case .failed = self { true } else { false } }
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

/// A template as the editor holds it: flat fields for every kind, of which
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

/// The template library and what each runtime has applied from it.
@MainActor final class TemplateLibrary: ObservableObject {
    @Published private(set) var summaries: [BindingResourceTemplateSummary] = []
    @Published private(set) var statuses: [String: [BindingRuntimeTemplateStatus]] = [:]
    @Published private(set) var isWorking = false
    @Published var failure: String?
    private let transport: Transport?
    private let queue = DispatchQueue(label: "local.velune.template-library", qos: .userInitiated)

    init(transport: Transport?) { self.transport = transport }

    /// Runs `work` off the main thread; failures become the visible message.
    func run<T: Sendable>(_ work: @escaping @Sendable (Transport) throws -> T, done: @escaping (T) -> Void = { _ in }) {
        guard let transport else { failure = "本地核心未配置"; return }
        isWorking = true; failure = nil
        queue.async { [weak self] in
            let result = Result { try work(transport) }
            DispatchQueue.main.async {
                guard let self else { return }
                self.isWorking = false
                switch result {
                case .success(let value): done(value)
                case .failure(let error): self.failure = ResourceFailure(error).message
                }
            }
        }
    }

    /// Reloads the template list and the applied state of the given runtimes.
    func reload(runtimeIDs: [String]) {
        run({ transport in
            let summaries = try transport.listResourceTemplates()
            var statuses: [String: [BindingRuntimeTemplateStatus]] = [:]
            for id in runtimeIDs { statuses[id] = try transport.runtimeTemplateStatus(runtimeID: id) }
            return (summaries, statuses)
        }, done: { [weak self] value in self?.summaries = value.0; self?.statuses = value.1 })
    }

    func status(of templateID: String) -> [(runtimeID: String, status: BindingRuntimeTemplateStatus)] {
        statuses.flatMap { runtimeID, list in list.filter { $0.templateId == templateID }.map { (runtimeID, $0) } }.sorted { $0.runtimeID < $1.runtimeID }
    }
    func hasUpdates(_ templateID: String) -> Bool { status(of: templateID).contains { $0.status.state.isTemplateUpdated } }
}
