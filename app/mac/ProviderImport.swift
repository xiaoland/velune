import SwiftUI

struct ProviderImportSource: Codable, Sendable, Equatable {
    var kind = "harness"
    var harnessTypeId: String
    var sourceInstanceId: String?
    var settings: [String: String]
}

struct ProviderImportPreviewData: Decodable, Sendable { var preview: ProviderImportPreview }
struct ProviderImportPreview: Decodable, Sendable {
    var token: String
    var sourceLabel: String
    var warnings: [String]
    var providers: [ProviderImportCandidate]
}
struct ProviderImportCandidate: Decodable, Sendable, Identifiable {
    var id: String
    var name: String
    var `protocol`: String
    var endpoint: String
    var canImport: Bool
    var alreadyImported: Bool
    var credentialStatus: String
    var issues: [String]
    var models: [ProviderImportModel]
}
struct ProviderImportModel: Decodable, Sendable, Identifiable {
    var id: String
    var externalModelId: String
    var name: String
    var contextWindow: UInt32?
    var maxOutputTokens: UInt32?
    var reasoningLevels: [String]
    var canImport: Bool
    var issues: [String]
}
struct ProviderImportSelection: Encodable, Sendable {
    var providerId: String
    var modelIds: [String]
    var modelMappings: [String: String]
}
struct ProviderImportResult: Decodable, Sendable {
    var gateways: [GatewayConfig]
    var requiresReconnect: Bool
    var importedProviderIds: [String]
    var skippedProviderIds: [String]
}

struct ProviderImportView: View {
    @ObservedObject var store: AppStore
    @Environment(\.dismiss) private var dismiss
    @State private var sourceInstanceID: String?
    @State private var preview: ProviderImportPreview?
    @State private var selectedModels: Set<String> = []
    @State private var mappings: [String: String] = [:]
    @State private var replaceExisting = false
    @State private var attemptedOperation = false

    private var supportedInstances: [RuntimeInstance] { store.runtimeInstances.filter { instance in store.providerImportTypes.contains { $0.id == instance.typeID } } }
    private var selectedInstance: RuntimeInstance? { supportedInstances.first { $0.id == sourceInstanceID } }
    private var source: ProviderImportSource? { guard let selectedInstance else { return nil }; return ProviderImportSource(harnessTypeId: selectedInstance.typeID, sourceInstanceId: selectedInstance.id, settings: [:]) }
    private var selections: [ProviderImportSelection] {
        (preview?.providers ?? []).filter { enabled($0) }.compactMap { provider in
            let models = provider.models.filter { $0.canImport && selectedModels.contains($0.id) }
            guard !models.isEmpty else { return nil }
            let ids = models.map(\.id)
            return ProviderImportSelection(providerId: provider.id, modelIds: ids, modelMappings: mappings.filter { ids.contains($0.key) && !$0.value.isEmpty })
        }
    }

    var body: some View {
        VStack(alignment: .leading, spacing: 0) {
            Text("从 Agent 运行时导入提供商")
                .font(.headline)
                .padding(.horizontal, 20)
                .padding(.vertical, 16)
            ScrollView {
                VStack(alignment: .leading, spacing: 16) {
                    sourceForm
                    if let preview { previewList(preview) }
                    else if !supportedInstances.isEmpty {
                        ContentUnavailableView("选择配置来源", systemImage: "square.and.arrow.down", description: Text("读取提供商、模型与认证来源。原配置保留。"))
                            .frame(maxWidth: .infinity)
                    }
                    SettingsError(message: attemptedOperation ? store.error : nil)
                }
                .padding(.horizontal, 20)
                .padding(.bottom, 12)
            }
            .frame(maxWidth: .infinity, maxHeight: .infinity)
            footer
        }
        .frame(width: 760, height: 640)
        .onAppear { selectSourceInstance(sourceInstanceID ?? supportedInstances.first?.id) }
        .onChange(of: sourceInstanceID) { _, value in selectSourceInstance(value) }
        .onChange(of: replaceExisting) { _, _ in resetSelection() }
    }

    private var sourceForm: some View {
        GroupBox {
            VStack(alignment: .leading, spacing: 12) {
                if supportedInstances.isEmpty {
                    ContentUnavailableView("没有可导入的运行时", systemImage: "shippingbox", description: Text("请先在运行时设置中配置支持提供商导入的 Agent 运行时。"))
                        .frame(maxWidth: .infinity)
                } else {
                    Picker("Agent 运行时", selection: Binding(get: { sourceInstanceID ?? "" }, set: { sourceInstanceID = $0 })) {
                        ForEach(supportedInstances) { instance in
                            Text(instance.name).tag(instance.id)
                        }
                    }
                    Text("导入不会自动改变模型路由或运行时默认模型。").font(.caption).foregroundStyle(.secondary)
                    HStack {
                        Spacer()
                        Button(preview == nil ? "读取配置" : "重新读取") { readSource() }.disabled(source == nil || store.isLoading)
                    }
                }
            }
        } label: {
            Text("运行时来源")
        }
        .disabled(store.isLoading)
    }

    private func previewList(_ preview: ProviderImportPreview) -> some View {
        LazyVStack(alignment: .leading, spacing: 12) {
            GroupBox(preview.sourceLabel) {
                Toggle("替换此次选中的已导入提供商", isOn: $replaceExisting)
                Text("重复项默认跳过；替换保留已有路由和全局模型参数。").font(.caption).foregroundStyle(.secondary)
                ForEach(Array(preview.warnings.enumerated()), id: \.offset) { _, warning in
                    Text(warning).font(.caption).foregroundStyle(.secondary)
                }
            }
            ForEach(preview.providers) { provider in
                GroupBox {
                    ForEach(provider.models) { model in modelRow(model, provider: provider) }
                    ForEach(Array(provider.issues.enumerated()), id: \.offset) { _, issue in Text(issue).font(.caption).foregroundStyle(.secondary) }
                } label: {
                    VStack(alignment: .leading, spacing: 3) {
                        Text(provider.name)
                        Text(store.protocols.first { $0.id.rawValue == provider.protocol }?.name ?? provider.protocol).font(.caption)
                        Text(provider.endpoint).font(.caption)
                        Text(provider.credentialStatus + (provider.alreadyImported ? " · 已导入" : "")).font(.caption)
                    }
                }
            }
            if preview.providers.isEmpty {
                Text("该来源没有配置的提供商。").foregroundStyle(.secondary)
                    .frame(maxWidth: .infinity, alignment: .leading)
            }
        }
        .disabled(store.isLoading)
    }

    private func modelRow(_ model: ProviderImportModel, provider: ProviderImportCandidate) -> some View {
        VStack(alignment: .leading, spacing: 5) {
            Toggle(model.name.isEmpty ? model.externalModelId : model.name, isOn: Binding(get: { selectedModels.contains(model.id) }, set: { if $0 { selectedModels.insert(model.id) } else { selectedModels.remove(model.id) } }))
                .disabled(!enabled(provider) || !model.canImport)
            Text(model.externalModelId).font(.caption).foregroundStyle(.secondary).textSelection(.enabled)
            if let context = model.contextWindow { Text("上下文窗口：\(context.formatted())").font(.caption).foregroundStyle(.secondary) }
            if let output = model.maxOutputTokens { Text("输出预算：\(output.formatted())").font(.caption).foregroundStyle(.secondary) }
            if !model.reasoningLevels.isEmpty { Text("推理级别：" + model.reasoningLevels.joined(separator: "、")).font(.caption).foregroundStyle(.secondary) }
            if selectedModels.contains(model.id) && enabled(provider) && model.canImport {
                Picker("全局模型", selection: Binding(get: { mappings[model.id] ?? "" }, set: { mappings[model.id] = $0 })) {
                    Text("新建模型").tag("")
                    ForEach(store.models) { existing in Text(existing.nickname.isEmpty ? existing.id : existing.nickname).tag(existing.id) }
                }
                if !(mappings[model.id] ?? "").isEmpty { Text("沿用该全局模型的参数，推理级别取共同支持的范围。").font(.caption).foregroundStyle(.secondary) }
            }
            ForEach(Array(model.issues.enumerated()), id: \.offset) { _, issue in Text(issue).font(.caption).foregroundStyle(.secondary) }
        }
        .padding(.vertical, 3)
    }

    private var footer: some View {
        HStack {
            Spacer()
            Button("取消") { dismiss() }.keyboardShortcut(.cancelAction)
            Button("导入所选") {
                guard let preview else { return }
                guard let source else { return }
                attemptedOperation = true
                store.applyProviderImport(source, preview: preview, selections: selections, replaceExisting: replaceExisting) { dismiss() }
            }
            .keyboardShortcut(.defaultAction)
            .disabled(source == nil || preview == nil || selections.isEmpty || store.isLoading)
        }.padding(20)
    }
    private func enabled(_ provider: ProviderImportCandidate) -> Bool { provider.canImport && (!provider.alreadyImported || replaceExisting) }
    private func selectSourceInstance(_ id: String?) {
        sourceInstanceID = id
        preview = nil
        selectedModels = []
        mappings = [:]
        attemptedOperation = false
    }
    private func readSource() {
        guard let source else { return }
        attemptedOperation = true
        store.previewProviderImport(source) { value in preview = value; resetSelection() }
    }
    private func resetSelection() {
        selectedModels = Set((preview?.providers ?? []).filter { enabled($0) }.flatMap { $0.models.filter(\.canImport).map(\.id) })
    }
}
