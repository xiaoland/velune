import AppKit
import Combine
import VeluneBindings

/// A restored window keeps identities and associations, including displaced
/// unfinished input. Layout changes move these identities; they never copy text.
struct SavedComposerWorkspace: Codable, Equatable {
    struct Draft: Codable, Equatable {
        var id: UUID
        var name: String
        var state: SavedComposerDraft
        var conversationID: String?
        var editedAt: Date? = nil
        var pending: Bool? = nil
        var revision: UInt64? = nil
    }
    var schemaVersion: Int? = 3
    var drafts: [Draft]
    var conversations: [String: UUID]
    var shared: UUID
    var retained: [UUID]
    var activeContext: String? = nil
    var targetContext: String? = nil
    var synchronizedRevision: UInt64? = nil
    var pendingBindings: [String: SavedComposerBinding]? = nil
    var pendingPromotions: [String: SavedComposerPromotion]? = nil
    var pendingTargets: [String: SavedComposerTarget]? = nil
}

@MainActor final class ComposerDrafts {
    enum Owner: Equatable { case conversation(String), shared }
    private var models: [UUID: ComposerModel] = [:]
    private var names: [UUID: String] = [:]
    private var editedAt: [UUID: Date] = [:]
    private var conversations: [String: UUID] = [:]
    private var retained: [UUID] = []
    private var watches: [UUID: AnyCancellable] = [:]
    private var resolving: Set<String> = []
    private(set) var shared: ComposerModel
    private var sharedInitialized = false
    private var activeContext: String?
    private var targetContext: String?
    private weak var targetTab: ConversationTab?
    let changes = PassthroughSubject<Void, Never>()
    let cursorChanges = PassthroughSubject<Void, Never>()
    let creations = PassthroughSubject<ComposerCreation, Never>()
    private var synchronizedRevision: UInt64?
    private weak var device: DeviceStore?
    private var deviceWatch: AnyCancellable?
    private var restoredBindings: [String: SavedComposerBinding] = [:]
    private var restoredTargets: [String: SavedComposerTarget] = [:]

    init(device: DeviceStore) {
        self.device = device
        shared = ComposerModel()
        register(shared, name: "共享输入")
        deviceWatch = device.composerInputs.changes.sink { [weak self] in self?.synchronize() }
    }

    private func register(_ model: ComposerModel, name: String) {
        guard models[model.id] == nil else { return }
        models[model.id] = model; names[model.id] = name
        watches[model.id] = model.draftChanges.sink { [weak self, weak model] in
            guard let self, let model else { return }
            editedAt[model.id] = Date()
            device?.composerInputs.save(model.id, draft: model.savedDraft)
            changes.send()
        }
    }

    private func keep(_ id: UUID) {
        if let model = models[id], model.hasUnfinishedInput, !retained.contains(id) { retained.append(id) }
    }

    func focus(_ tab: ConversationTab) {
        targetTab = tab
        targetContext = tab.store.selectedConversationID ?? tab.store.loadedConversationID ?? "tab:\(tab.id)"
    }

    /// Navigation is the only ordinary action that changes the shared input.
    /// Merely focusing a selected tab does not call this method.
    func checkpoint() {
        if let previous = targetContext, !previous.hasPrefix("tab:"), !previous.hasPrefix(ConversationStore.draftConversationPrefix) {
            conversations[previous] = shared.id
            device?.composerInputs.bind(previous, input: shared)
        }
    }
    func navigate(to id: String, tab: ConversationTab, sharedInput: Bool = true, checkpointPrevious: Bool = true) {
        if sharedInput, checkpointPrevious { checkpoint() }
        device?.composerInputs.beginNavigation(id)
        if let old = conversations[id], let current = device?.composerInputs.inputID(for: id), old != current {
            keep(old); conversations[id] = nil
        }
        let model = conversation(id, tab: tab)
        if sharedInput, shared.id != model.id, !conversations.values.contains(shared.id) { keep(shared.id) }
        if sharedInput { shared = model; activeContext = id; sharedInitialized = true }
        focus(tab)
        targetContext = id
        model.attach(tab.store)
        changes.send()
    }

    private func conversation(_ id: String, tab: ConversationTab) -> ComposerModel {
        let synced = device?.composerInputs.inputID(for: id)
        let local = conversations[id]
        let key = local.flatMap { device?.composerInputs.isDirty($0) == true ? $0 : nil } ?? synced ?? local
        let model: ComposerModel
        if let key, let kept = models[key] { model = kept }
        else {
            model = ComposerModel(id: key ?? UUID())
            model.attach(tab.store)
            let state = key.flatMap { device?.composerInputs.draft($0) }
                ?? SavedComposerDraft(text: "", attachments: [])
            model.restoreSavedDraft(state)
            register(model, name: tab.title + " 的输入")
        }
        conversations[id] = model.id
        model.attach(tab.store)
        if !id.hasPrefix("tab:"), !id.hasPrefix("side-chat:"), synced == nil, !resolving.contains(id), let device {
            resolving.insert(id)
            device.composerInputs.ensure(id, model: model) { [weak self, weak model] resolved in
                guard let self, let model else { return }
                self.resolving.remove(id)
                if resolved != model.id, self.device?.composerInputs.isDirty(model.id) == true {
                    // Editing before loading completes remains the same visible work.
                    self.device?.composerInputs.bind(id, input: model)
                    return
                }
                self.conversations[id] = resolved
                self.synchronize()
            }
        }
        return model
    }

    func model(for owner: Owner, tab: ConversationTab) -> ComposerModel {
        if targetTab == nil { focus(tab) }
        let model: ComposerModel
        switch owner {
        case .shared:
            if !sharedInitialized {
                let id = tab.store.selectedConversationID ?? tab.store.loadedConversationID ?? "tab:\(tab.id)"
                shared = conversation(id, tab: tab); activeContext = id; sharedInitialized = true
            }
            model = shared
        case .conversation(let id): model = conversation(id, tab: tab)
        }
        model.attach(tab.store)
        return model
    }

    private func synchronize() {
        guard let device else { return }
        if device.composerInputs.ownerID != nil, !restoredBindings.isEmpty {
            let bindings = restoredBindings; restoredBindings.removeAll()
            for (context, id) in bindings { device.composerInputs.restoreBinding(context, value: id) }
        }
        if device.composerInputs.ownerID != nil, !restoredTargets.isEmpty {
            let targets = restoredTargets; restoredTargets.removeAll()
            for (context, target) in targets { device.composerInputs.restoreTarget(context, value: target) }
        }
        let baseline = synchronizedRevision ?? 0
        synchronizedRevision = device.composerInputs.state?.revision
        if let state = device.composerInputs.state {
            for creation in state.creations where creation.revision > baseline && creation.ownerId == device.composerInputs.ownerID {
                let context = ComposerContextKey.conversation(ownerId: creation.ownerId, conversationId: creation.conversationId)
                if state.bindings.contains(where: { $0.context == context && $0.inputId == creation.inputId }) { creations.send(creation) }
            }
        }
        var changedCursor = false
        for (id, model) in models {
            if let state = device.composerInputs.draft(id), state != model.savedDraft { model.receiveDraft(state) }
            // Availability and downloaded bytes can change without changing
            // the content value. The editor must reevaluate submission access.
            model.objectWillChange.send()
        }
        for context in Array(conversations.keys) {
            if let id = device.composerInputs.inputID(for: context) {
                // A not-yet-saved editor must not disappear behind an incoming baseline.
                if let old = conversations[context], device.composerInputs.isDirty(old), old != id { continue }
                if let old = conversations[context], old != id { keep(old) }
                if conversations[context] != id { changedCursor = true }
                conversations[context] = id
            }
        }
        if let context = activeContext, let tab = targetTab, let id = conversations[context], id != shared.id {
            keep(shared.id)
            shared = conversation(context, tab: tab)
        }
        if changedCursor { cursorChanges.send() }
        changes.send()
    }

    /// Layout transfers the current editor, preserving its native undo state.
    func move(_ model: ComposerModel, to owner: Owner, tab: ConversationTab) {
        guard model.hasUnfinishedInput else { return }
        register(model, name: tab.title + " 的输入")
        switch owner {
        case .shared:
            if shared !== model { keep(shared.id) }
            shared = model
            sharedInitialized = true
            activeContext = conversations.first { $0.value == model.id }?.key ?? activeContext
        case .conversation(let id):
            if let previous = conversations[id], previous != model.id { keep(previous) }
            conversations[id] = model.id
            // A borrowed input never overwrites the global new-conversation slot.
            if !id.hasPrefix(ConversationStore.draftConversationPrefix) { device?.composerInputs.bind(id, input: model) }
        }
        retained.removeAll { $0 == model.id }
        focus(tab); model.attach(tab.store)
        changes.send()
    }

    func alternatives(to current: ComposerModel) -> [(model: ComposerModel, name: String)] {
        if let state = device?.composerInputs.state {
            let referenced = Set(state.bindings.map(\.inputId))
            for input in state.inputs where !referenced.contains(input.id) {
                guard let id = UUID(uuidString: input.id), models[id] == nil, let saved = device?.composerInputs.draft(id), !saved.text.isEmpty || !saved.attachments.isEmpty else { continue }
                let model = ComposerModel(id: id)
                model.restoreSavedDraft(saved); register(model, name: "保留的输入"); keep(id)
            }
        }
        return retained.compactMap { id in
            guard id != current.id, let model = models[id], model.hasUnfinishedInput else { return nil }
            let first = model.submission.split(whereSeparator: \.isNewline).first.map(String.init)
            let summary = first.map { String($0.prefix(36)) } ?? "\(model.attachments.count) 个附件"
            return (model, (names[id] ?? "保留的输入") + " · " + summary)
        }
    }

    func select(_ model: ComposerModel, replacing current: ComposerModel, owner: Owner, tab: ConversationTab) {
        if current !== model { keep(current.id) }
        register(model, name: tab.title + " 的输入")
        switch owner {
        case .shared:
            shared = model
            sharedInitialized = true
            if let context = activeContext { conversations[context] = model.id; device?.composerInputs.bind(context, input: model) }
        case .conversation(let id): conversations[id] = model.id; device?.composerInputs.bind(id, input: model)
        }
        retained.removeAll { $0 == model.id }
        focus(tab); model.attach(tab.store)
        changes.send()
    }
    func newDraft(replacing current: ComposerModel, owner: Owner, tab: ConversationTab) {
        select(ComposerModel(), replacing: current, owner: owner, tab: tab)
    }
    /// A cross-window handoff takes the native editor state with it. Any
    /// source-window references keep the same content identity in a new local
    /// editor, rather than sharing the departing model's text view and undo.
    func releaseEditingModel(_ model: ComposerModel) -> ComposerModel? {
        guard models[model.id] === model else { return nil }
        let replacement = ComposerModel(id: model.id)
        replacement.restoreSavedDraft(device?.composerInputs.draft(model.id) ?? model.savedDraft)
        let name = names[model.id] ?? "会话输入"
        watches[model.id] = nil
        models[model.id] = nil
        register(replacement, name: name)
        if shared === model { shared = replacement }
        cursorChanges.send(); changes.send()
        return replacement
    }
    func adopt(_ model: ComposerModel, conversationID: String, tab: ConversationTab) {
        // The shared client remains the content authority. An incoming editor
        // keeps its undo only while it still represents that current content.
        if let current = device?.composerInputs.draft(model.id), current != model.savedDraft { model.receiveDraft(current) }
        if let previous = models[model.id], previous !== model {
            watches[model.id] = nil
            models[model.id] = nil
            if shared === previous { shared = model }
        }
        register(model, name: tab.title + " 的输入")
        conversations[conversationID] = model.id
        model.attach(tab.store)
        cursorChanges.send(); changes.send()
    }

    var saved: SavedComposerWorkspace {
        let referenced = Set(conversations.values).union(retained).union([shared.id])
        return SavedComposerWorkspace(drafts: models.values.filter { referenced.contains($0.id) }.map { model in
            .init(id: model.id, name: names[model.id] ?? "输入", state: model.savedDraft, conversationID: conversations.first { $0.value == model.id }?.key, editedAt: editedAt[model.id], pending: device?.composerInputs.isDirty(model.id), revision: device?.composerInputs.revision(model.id))
        }.sorted { $0.id.uuidString < $1.id.uuidString }, conversations: conversations, shared: shared.id, retained: retained, activeContext: activeContext, targetContext: targetContext,
            synchronizedRevision: synchronizedRevision, pendingBindings: conversations.reduce(into: restoredBindings) { if let input = device?.composerInputs.pendingBinding($1.key) { $0[$1.key] = input } }, pendingPromotions: device?.composerInputs.pendingPromotions, pendingTargets: restoredTargets.merging(device?.composerInputs.pendingTargets ?? [:]) { _, pending in pending })
    }
    func restore(_ saved: SavedComposerWorkspace?) {
        guard let saved else { return }
        models.removeAll(); names.removeAll(); watches.removeAll(); editedAt.removeAll()
        for draft in saved.drafts {
            if draft.pending == true { device?.composerInputs.restorePending(draft.id, draft: draft.state, revision: draft.revision) }
            let model = ComposerModel(id: draft.id)
            model.restoreSavedDraft(device?.composerInputs.draft(draft.id) ?? draft.state)
            register(model, name: draft.name); editedAt[model.id] = draft.editedAt
        }
        conversations = saved.conversations.filter { models[$0.value] != nil }
        retained = saved.retained.filter { models[$0] != nil }
        shared = models[saved.shared] ?? ComposerModel()
        sharedInitialized = true
        activeContext = saved.activeContext ?? conversations.first { $0.value == shared.id }?.key
        targetContext = saved.targetContext
        synchronizedRevision = saved.synchronizedRevision
        restoredBindings = saved.pendingBindings ?? [:]
        if device?.composerInputs.ownerID != nil {
            for (conversation, input) in restoredBindings { device?.composerInputs.restoreBinding(conversation, value: input) }
            restoredBindings.removeAll()
        }
        restoredTargets = saved.pendingTargets ?? [:]
        if device?.composerInputs.ownerID != nil {
            for (conversation, target) in restoredTargets { device?.composerInputs.restoreTarget(conversation, value: target) }
            restoredTargets.removeAll()
        }
        for (conversation, input) in saved.pendingPromotions ?? [:] { device?.composerInputs.restorePromotion(conversation, value: input) }
        register(shared, name: "共享输入")
        device?.composerInputs.poll(force: true)
    }
    func existing(_ inputID: String) -> ComposerModel? { UUID(uuidString: inputID).flatMap { models[$0] } }
    func isNewInput(_ model: ComposerModel) -> Bool { shared === model && activeContext?.hasPrefix(ConversationStore.draftConversationPrefix) == true }
    func removeConversations(_ ids: Set<String>) {
        for id in ids { conversations[id] = nil }
    }
    func reassociate(_ model: ComposerModel, from previous: String, to id: String, runtimeID: String? = nil, targetTab: ConversationTab? = nil, publish: Bool = true) {
        register(model, name: "会话输入")
        conversations[id] = model.id
        if conversations[previous] == model.id { conversations[previous] = nil }
        if activeContext == previous, shared === model { activeContext = id }
        if let targetTab, self.targetTab === targetTab, targetContext == previous { targetContext = id }
        if publish, let runtimeID { device?.composerInputs.promote(model, to: id, runtimeID: runtimeID) }
        changes.send()
    }
}
