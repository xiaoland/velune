import SwiftUI
import VeluneBindings

/// The same setting is expressed in the quantity its runtime actually accepts.
enum RuntimeAutoCompactionInput {
    case codex, claude, pi, deepSeek

    init?(_ runtime: RuntimeKind) {
        switch runtime {
        case .codex166: self = .codex
        case .claudeCode: self = .claude
        case .pi: self = .pi
        case .deepSeek: self = .deepSeek
        case .acp: return nil
        }
    }

    var label: String {
        switch self {
        case .codex: "压缩触发阈值（Token）"
        case .claude: "自动压缩窗口（Token）"
        case .pi: "压缩预留量（Token）"
        case .deepSeek: "压缩触发比例（%）"
        }
    }

    var guidance: String {
        switch self {
        case .codex: "Codex 会按实际模型窗口限制生效阈值。"
        case .claude: "接受 100K 到 1M Token。Claude Code 会为输出与摘要预留空间。"
        case .pi: "上下文剩余空间小于预留量时触发。"
        case .deepSeek: "输入 1 到 100 的整数，须高于模型的保留比例。DeepSeek Harness 会为输出和压缩预留空间。"
        }
    }

    var formatError: String {
        self == .deepSeek ? "请输入整数百分比。" : "请输入正整数 Token 数，可用 K、M 或 B。"
    }

    static func text(_ setting: AutoCompactionSetting) -> String {
        switch setting {
        case .codex(let limit), .claude(let limit), .pi(let limit): TokenCount.format(limit)
        case .deepSeek(let percent): String(percent)
        case .acp(let optionID, let value):
            switch value {
            case .select(let selection): "\(optionID) · \(selection)"
            case .boolean(let enabled): "\(optionID) · \(enabled ? "开启" : "关闭")"
            }
        }
    }

    func setting(_ text: String) -> AutoCompactionSetting? {
        let value = text.trimmingCharacters(in: .whitespacesAndNewlines)
        if self == .deepSeek {
            let digits = value.hasSuffix("%") ? String(value.dropLast()).trimmingCharacters(in: .whitespaces) : value
            guard !digits.isEmpty, digits.utf8.allSatisfy({ (48...57).contains($0) }), let percent = UInt8(digits) else { return nil }
            return .deepSeek(thresholdPercent: percent)
        }
        guard let tokens = TokenCount.parse(value) else { return nil }
        switch self {
        case .codex: return .codex(tokenLimit: tokens)
        case .claude: return .claude(windowTokens: tokens)
        case .pi: return .pi(reserveTokens: tokens)
        case .deepSeek: return nil
        }
    }
}

/// A model is fixed for the life of the editor. The user only picks which
/// instance interprets its setting.
struct ModelAutoCompactionTarget: Hashable, Identifiable {
    enum Scope: Hashable {
        case managed(gatewayID: String)
        case native(runtimeID: String, type: RuntimeKind)
    }

    let model: ModelSelection
    let title: String
    let scope: Scope
    var id: Self { self }

    @MainActor static func managed(_ modelKey: String, title: String, in store: DeviceStore) -> Self? {
        guard !modelKey.isEmpty, store.gateway.providers.contains(where: { $0.models.contains { $0.recordKey == modelKey } })
            || store.gateway.routes.contains(where: { $0.recordKey == modelKey }) else { return nil }
        return Self(model: .managed(modelRecordKey: modelKey), title: title, scope: .managed(gatewayID: store.gateway.id))
    }

    /// The catalog can omit an old model, but the policy still carries its exact identity.
    static func native(_ model: ModelSelection, title: String?, runtime: RuntimeInstance) -> Self? {
        guard case .native(let provider, let modelID) = model, !modelID.isEmpty else { return nil }
        switch runtime.typeId {
        case .claudeCode, .acp: guard provider == nil else { return nil }
        case .pi, .deepSeek: guard let provider, !provider.isEmpty else { return nil }
        default: break
        }
        let name = title ?? provider.map { "\(modelID)（\($0)）" } ?? modelID
        return Self(model: model, title: name, scope: .native(runtimeID: runtime.id, type: runtime.typeId))
    }

    @MainActor func runtimes(in store: DeviceStore) -> [RuntimeInstance] {
        let matching: [RuntimeInstance]
        switch scope {
        case .managed(let gatewayID):
            guard gatewayID == store.gateway.id, case .managed(let key) = model,
                  Self.managed(key, title: title, in: store) != nil else { return [] }
            matching = store.runtimeInstances.filter { runtime in
                runtime.gatewayId == gatewayID && RuntimeAutoCompactionInput(runtime.typeId) != nil
                    && (supportsModel(runtime, in: store)
                        || (try? runtimeModelAutoCompactions(runtime: runtime).contains { $0.model == model }) == true)
            }
        case .native(let runtimeID, let type):
            matching = store.runtimeInstances.filter { $0.id == runtimeID && $0.typeId == type }
        }
        return matching.sorted {
            let order = $0.name.localizedStandardCompare($1.name)
            return order == .orderedSame ? $0.id < $1.id : order == .orderedAscending
        }
    }

    @MainActor func supportsModel(_ runtime: RuntimeInstance, in store: DeviceStore) -> Bool {
        guard case .managed(let key) = model else { return true }
        let memberKey = store.gateway.routes.first { $0.recordKey == key }?.members.first ?? key
        guard let protocolID = store.routeMember(memberKey)?.provider.protocol,
              let type = store.runtimeTypes.first(where: { $0.id == runtime.typeId }) else { return false }
        return type.supportedProviderProtocols.contains(protocolID)
    }
}

extension DeviceStore {
    var hasModelAutoCompactionWriter: Bool { transport != nil && !isShuttingDown }
}

/// Keep this action on the saved model's form. Its save is independent of
/// unrelated provider or route fields that the user is still editing.
struct ModelAutoCompactionLink: View {
    @ObservedObject var store: DeviceStore
    let target: ModelAutoCompactionTarget?
    var observeForCheck: ((ModelAutoCompactionTarget?) -> Void)? = nil
    @State private var editing: ModelAutoCompactionTarget?

    var body: some View {
        Group {
            if let target {
                Button("自动压缩…") { editing = target }
                    .disabled(!store.hasModelAutoCompactionWriter || store.isLoading)
                    .accessibilityIdentifier("model-config-auto-compaction")
            } else {
                Text("保存后可设置自动压缩。").foregroundStyle(.secondary)
            }
        }
        .onAppear { observeForCheck?(target) }
        .sheet(item: $editing) { ModelAutoCompactionEditor(store: store, target: $0) }
    }
}

struct ModelAutoCompactionCheckAccess {
    let runtime: Binding<String?>
    let value: Binding<String>
    let save: () -> Void
    let resetAcp: () -> Void
}

struct ModelAutoCompactionEditor: View {
    @ObservedObject var store: DeviceStore
    let target: ModelAutoCompactionTarget
    var observeForCheck: ((ModelAutoCompactionCheckAccess) -> Void)? = nil
    @Environment(\.dismiss) private var dismiss
    @State private var runtimeID: String?
    @State private var editedValue: String?
    @State private var saving = false
    @State private var saveError: String?

    private var runtimes: [RuntimeInstance] { target.runtimes(in: store) }
    private var current: RuntimeInstance? { runtimes.first { $0.id == runtimeID } }
    private var original: Result<AutoCompactionSetting?, Error> {
        Result {
            guard let current else { return nil }
            return try runtimeModelAutoCompactions(runtime: current).first { $0.model == target.model }?.setting
        }
    }
    private var originalText: String { (try? original.get()).map(RuntimeAutoCompactionInput.text) ?? "" }
    private var runtime: Binding<String?> {
        Binding(get: { current?.id }, set: {
            guard editedValue == nil, !saving else { return }
            runtimeID = $0
            saveError = nil
        })
    }
    private var value: Binding<String> {
        Binding(get: { editedValue ?? originalText }, set: {
            editedValue = $0 == originalText ? nil : $0
            saveError = nil
        })
    }
    private var clearingOnly: Bool { current.map { !target.supportsModel($0, in: store) } ?? false }
    private var canWrite: Bool { store.hasModelAutoCompactionWriter && !store.isLoading && !saving }
    private var canResetAcp: Bool {
        guard case .success(.some(.acp)) = original else { return false }
        return canWrite
    }

    var body: some View {
        VStack(alignment: .leading, spacing: 14) {
            Text("自动压缩").uiFont(.headline)
            LabeledContent("模型", value: target.title)
            if runtimes.isEmpty {
                Text("此模型没有可配置自动压缩的运行时实例。").foregroundStyle(.secondary)
                    .fixedSize(horizontal: false, vertical: true)
            } else {
                if case .managed = target.scope {
                    Picker("运行时实例", selection: runtime) {
                        if current == nil { Text("选择运行时实例").tag(String?.none) }
                        ForEach(runtimes) { instance in
                            Text(instance.enabled ? instance.name : "\(instance.name)（已停用）").tag(Optional(instance.id))
                        }
                    }
                    .disabled(editedValue != nil || saving)
                } else if let current {
                    LabeledContent("运行时实例", value: current.name)
                }
                if let current {
                    switch original {
                    case .failure(let error):
                        Text(AppProblem.failure(error, source: "读取自动压缩设置").detail)
                            .uiFont(.caption).foregroundStyle(.red).textSelection(.enabled)
                    case .success:
                        if let input = RuntimeAutoCompactionInput(current.typeId) {
                            LabeledContent(input.label) {
                                TextField(input.label, text: value, prompt: Text("运行时默认"))
                                    .labelsHidden()
                                    .disabled(saving)
                                    .accessibilityIdentifier("model-auto-compaction-value")
                            }
                            Text(clearingOnly ? "此实例当前无法使用此模型。留空可清除已保存的设置。" :
                                    input.guidance + " 留空使用运行时默认值，保存后从下一轮生效。")
                                .formFooterStyle()
                        } else {
                            LabeledContent("已保存的选项", value: originalText.isEmpty ? "跟随 Agent 设置" : originalText)
                            Text("此处可清除已保存的设置。要选择新值，请在 Agent 提供该模型的压缩选项时设置。")
                                .formFooterStyle()
                        }
                    }
                }
            }
            if let saveError {
                Text(saveError).uiFont(.caption).foregroundStyle(.red).textSelection(.enabled)
            }
            HStack {
                if editedValue != nil, case .managed = target.scope {
                    Button("撤销修改") { value.wrappedValue = originalText }.disabled(saving)
                }
                Spacer()
                Button("取消") { dismiss() }.keyboardShortcut(.cancelAction).disabled(saving)
                if current?.typeId == .acp {
                    Button(saving ? "正在保存…" : "恢复 Agent 默认", action: resetAcp)
                        .disabled(!canResetAcp)
                } else {
                    Button(saving ? "正在保存…" : "保存", action: commit)
                        .keyboardShortcut(.defaultAction)
                        .disabled(!canSave)
                }
            }
        }
        .padding(20)
        .frame(width: 450)
        .interactiveDismissDisabled(saving)
        .onAppear {
            runtimeID = runtimes.first { instance in
                (try? runtimeModelAutoCompactions(runtime: instance).contains { $0.model == target.model }) == true
            }?.id ?? runtimes.first?.id
            observeForCheck?(ModelAutoCompactionCheckAccess(runtime: runtime, value: value, save: commit, resetAcp: resetAcp))
        }
    }

    private var canSave: Bool {
        guard let editedValue, current != nil, case .success = original,
              !clearingOnly || editedValue.trimmingCharacters(in: .whitespacesAndNewlines).isEmpty else { return false }
        return canWrite
    }

    private func commit() {
        guard canSave, let current, let input = RuntimeAutoCompactionInput(current.typeId), let editedValue else { return }
        let text = editedValue.trimmingCharacters(in: .whitespacesAndNewlines)
        let setting = text.isEmpty ? nil : input.setting(text)
        guard text.isEmpty || setting != nil else { saveError = input.formatError; return }
        save(setting, to: current.id)
    }

    private func resetAcp() {
        guard canResetAcp, let current else { return }
        save(nil, to: current.id)
    }

    private func save(_ setting: AutoCompactionSetting?, to runtimeID: String) {
        saving = true
        store.saveRuntimeModelAutoCompaction(runtimeID: runtimeID, model: target.model, setting: setting, onFailure: { error in
            saving = false
            saveError = AppProblem.failure(error, source: "保存自动压缩设置").detail
        }) {
            saving = false
            dismiss()
        }
    }
}
