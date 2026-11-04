import SwiftUI
import Combine

struct WindowRoute: Hashable, Codable {
    var id = UUID().uuidString
    var peer: String?
    var target: ConversationTarget?
}

struct PendingComposerCreation: Codable { var inputID: String; var ownerID: String; var runtimeID: String; var conversationID: String; var choices: ComposerTargetChoices? = nil }

struct ComposerState: Codable {
    var composerSchema: Int? = 2
    var sharedTarget: ComposerTarget?
    var targetPending: Bool?
    var targetBaseline: UInt64?
    var legacyTarget: ComposerTargetChoices?
    var text = ""
    var selection: ModelSelection?
    var following = true
    var offset: CGFloat = 0
    var destination: DraftDestination?
    var choices: [String: TurnChoice]?
    var sharedInput: ComposerInput?
    var composerPending: Bool?
    var composerBaseline: UInt64?
    var legacyInput: ComposerInputContent?
    var composerUnbound: Bool?
    var composerDirectoryRevision: UInt64?
    var pendingCreation: PendingComposerCreation?
    var composerBindingRevision: UInt64?
    var composerBindingBaseline: UInt64?
}

struct DraftDestination: Codable {
    var runtime: String
    var cwd: String?
}

private struct WindowRecord: Codable {
    var peer: String?
    var selected: ConversationTarget?
    var composers: [String: ComposerState] = [:]
    var drafts: [String: ConversationTarget] = [:]
}

@MainActor
final class DeviceStore: ObservableObject {
    let id: String
    let client: RemoteClient
    let directory: DeviceDirectory
    @Published var peer: String? {
        didSet {
            if oldValue != peer { selected = nil; history.removeAll(); persist() }
        }
    }
    @Published private(set) var selected: ConversationTarget?
    @Published private(set) var history: [ConversationTarget] = []
    private var models: [String: ConversationModel] = [:]
    private var composers: [String: ComposerState] = [:]
    @Published private var drafts: [String: ConversationTarget] = [:]
    var pendingDrafts: [ConversationTarget] {
        drafts.filter { $0.key.hasPrefix((peer ?? "") + "/") && $0.key == key($0.value) }.map(\.value).sorted { $0.key < $1.key }
    }
    func draftTitle(_ target: ConversationTarget) -> String {
        composers[key(target)]?.text.trimmingCharacters(in: .whitespacesAndNewlines).components(separatedBy: .newlines).first ?? "新会话"
    }
    func discardDraft(_ target: ConversationTarget) {
        let storageKey = key(target)
        if selected == target { select(nil) }
        drafts[storageKey] = nil; composers[storageKey] = nil; models[storageKey] = nil
        history.removeAll { $0 == target }; persist()
    }
    private var observation: AnyCancellable?
    var state: RemoteServiceSnapshot? { directory.state }
    var currentModel: ConversationModel? { selected.flatMap { models[key($0)] } }
    var deviceName: String { state?.contacts.first { $0.peerId == peer }?.name ?? "设备" }

    init(route: WindowRoute, directory: DeviceDirectory) {
        id = route.id; client = RemoteClient(viewID: "ios-" + route.id); self.directory = directory
        if let data = UserDefaults.standard.data(forKey: "workspace." + id) {
            do {
                let saved = try JSONDecoder().decode(WindowRecord.self, from: data)
                peer = saved.peer; selected = saved.selected; composers = saved.composers; drafts = saved.drafts
            } catch { ProblemStore.shared.record(error, source: "恢复窗口") }
        } else { peer = route.peer; selected = route.target }
        observation = directory.objectWillChange.sink { [weak self] _ in self?.objectWillChange.send() }
        if let selected, peer != nil { _ = model(for: selected) }
    }

    private func key(_ target: ConversationTarget) -> String { (peer ?? "") + "/" + target.key }

    func model(for target: ConversationTarget) -> ConversationModel {
        let storageKey = key(target)
        if let cached = models[storageKey] { return cached }
        let model = ConversationModel(client: client, peer: peer ?? "", target: target)
        if let saved = composers[storageKey] { model.restore(saved) }
        if target.conversation == nil {
            model.legacyInputs = drafts.compactMap { oldKey, oldTarget in
                guard oldTarget.conversation == nil, oldKey.hasPrefix((peer ?? "") + "/"), oldKey != storageKey,
                      let saved = composers[oldKey], saved.sharedInput == nil, !saved.text.isEmpty else { return nil }
                return LegacyComposerEntry(id: oldKey, content: .init(text: saved.text, attachments: []), target: saved.legacyTarget ?? .init(runtimeId: saved.destination?.runtime ?? oldTarget.runtime, runtimeOwnerId: nil, choices: (saved.choices ?? [:]).map { .init(ownerId: "", runtimeId: $0.key, choice: $0.value) }, choiceEdited: true))
            }.sorted { $0.id < $1.id }
        }
        model.problemContext = deviceName + " · " + target.title
        model.onComposerChange = { [weak self] model in self?.save(model) }
        model.onIdentityChange = { [weak self] model in self?.adopt(model) }
        models[storageKey] = model
        return model
    }

    func select(_ target: ConversationTarget?, remembering: Bool = false) {
        guard selected != target else { return }
        if remembering, let selected { history.append(selected) }
        else { history.removeAll() }
        show(target)
    }
    private func show(_ target: ConversationTarget?) {
        if let model = currentModel { save(model) }
        selected = target
        if let target { _ = model(for: target) }
        persist()
    }
    func back() {
        guard let target = history.popLast() else { return }
        show(target)
    }
    private func adopt(_ model: ConversationModel) {
        guard model.snapshot != nil else { return }
        let target = model.currentTarget
        let oldKey = model.peer + "/" + model.target.key
        let newKey = model.peer + "/" + target.key
        models[newKey] = model
        if oldKey != newKey { models[oldKey] = nil; composers[oldKey] = nil; drafts[oldKey] = nil }
        if peer == model.peer {
            history = history.map { $0.key == model.target.key ? target : $0 }
            if selected?.key == model.target.key { selected = target }
        }
        save(model)
    }
    private func save(_ model: ConversationModel) {
        let target = model.currentTarget
        let storageKey = model.peer + "/" + target.key
        composers[storageKey] = model.composerState
        if target.conversation == nil { drafts[storageKey] = model.draft.isEmpty ? nil : target }
        persist()
    }
    func persist() {
        do {
            let data = try JSONEncoder().encode(WindowRecord(peer: peer, selected: selected, composers: composers, drafts: drafts))
            UserDefaults.standard.set(data, forKey: "workspace." + id)
        } catch { ProblemStore.shared.record(error, source: "保存窗口", context: deviceName) }
    }
    func refresh() async { await directory.refresh() }
    func activity(foreground: Bool) async {
        guard !Task.isCancelled else { return }
        directory.activity(window: id, active: foreground)
        do {
            try await client.activity(peer: peer, foreground: foreground)
        } catch { ProblemStore.shared.record(error, source: "设备活动", context: deviceName) }
        if !foreground { if let model = currentModel { save(model) }; persist() }
    }
    func perform(_ action: @escaping (RemoteClient) async throws -> Void) {
        Task {
            do { try await action(client); await refresh() }
            catch { ProblemStore.shared.record(error, source: "设备操作", context: deviceName) }
        }
    }
    func status(_ peer: String) -> String {
        guard let controller = state?.controllers.first(where: { $0.peerId == peer }) else { return "未连接" }
        switch controller.phase {
        case .connected: return "已连接"
        case .connecting, .discovering, .recovering: return "连接中"
        case .blocked: return "连接失败"
        case .disconnected, .suspended: return "未连接"
        }
    }
    deinit {
        let directory = directory, id = id
        Task { @MainActor in directory.activity(window: id, active: false) }
    }
}
