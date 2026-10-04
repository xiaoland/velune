import SwiftUI

struct ProviderImportSource: Codable, Sendable, Equatable {
    var kind = "harness"
    var harnessTypeId: String
    var settings: [String: String]
}

struct ProviderImportPreviewData: Decodable, Sendable { var preview: ProviderImportPreview }
struct ProviderImportPreview: Decodable, Sendable {
    var token: String
    var sourceLabel: String
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
    @State private var sourceTypeID = ""
    @State private var values: [String: String] = [:]
    @State private var preview: ProviderImportPreview?
    @State private var selectedModels: Set<String> = []
    @State private var mappings: [String: String] = [:]
    @State private var replaceExisting = false

    private var descriptor: RuntimeTypeDescriptor? { store.providerImportTypes.first { $0.id == sourceTypeID } }
    private var source: ProviderImportSource { ProviderImportSource(harnessTypeId: sourceTypeID, settings: values.filter { !$0.value.trimmingCharacters(in: .whitespacesAndNewlines).isEmpty }) }
    private var sourceValid: Bool {
        descriptor?.fields.allSatisfy { !$0.required || !(values[$0.key] ?? $0.value).trimmingCharacters(in: .whitespacesAndNewlines).isEmpty } == true
    }
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
            Text("从 Agent 运行时导入提供商").font(.headline).padding(20)
            sourceForm
            if let preview { previewList(preview) }
            else { ContentUnavailableView("选择配置来源", systemImage: "square.and.arrow.down", description: Text("读取提供商、模型与认证来源。原配置保留。")) }
            SettingsError(message: store.error)
            footer
        }
        .frame(width: 760, height: 640)
        .onAppear { selectSourceType(store.providerImportTypes.first?.id ?? "") }
        .onChange(of: sourceTypeID) { _, value in selectSourceType(value) }
        .onChange(of: values) { _, _ in preview = nil; selectedModels = []; mappings = [:] }
        .onChange(of: replaceExisting) { _, _ in resetSelection() }
    }

    private var sourceForm: some View {
        Form {
            Picker("来源类型", selection: $sourceTypeID) {
                ForEach(store.providerImportTypes) { type in Text(type.name).tag(type.id) }
            }
            if let descriptor {
                ForEach(descriptor.fields) { field in
                    SettingFieldView(field: field, value: Binding(get: { values[field.key] ?? field.value }, set: { values[field.key] = $0 }))
                }
            }
            HStack {
                Text("导入不会自动改变模型路由或运行时默认模型。").font(.caption).foregroundStyle(.secondary)
                Spacer()
                Button(preview == nil ? "读取配置" : "重新读取") { readSource() }.disabled(!sourceValid || store.isLoading)
            }
        }
        .formStyle(.grouped)
        .frame(height: 260)
        .disabled(store.isLoading)
    }

    private func previewList(_ preview: ProviderImportPreview) -> some View {
        List {
            Section(preview.sourceLabel) {
                Toggle("替换此次选中的已导入提供商", isOn: $replaceExisting)
                Text("重复项默认跳过；替换保留已有路由和全局模型参数。").font(.caption).foregroundStyle(.secondary)
            }
            ForEach(preview.providers) { provider in
                Section {
                    ForEach(provider.models) { model in modelRow(model, provider: provider) }
                    ForEach(Array(provider.issues.enumerated()), id: \.offset) { _, issue in Text(issue).font(.caption).foregroundStyle(.secondary) }
                } header: {
                    VStack(alignment: .leading, spacing: 3) {
                        Text(provider.name)
                        Text(store.protocols.first { $0.id.rawValue == provider.protocol }?.name ?? provider.protocol).font(.caption)
                        Text(provider.endpoint).font(.caption)
                        Text(provider.credentialStatus + (provider.alreadyImported ? " · 已导入" : "")).font(.caption)
                    }
                }
            }
            if preview.providers.isEmpty { Text("该来源没有配置的提供商。").foregroundStyle(.secondary) }
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
                store.applyProviderImport(source, preview: preview, selections: selections, replaceExisting: replaceExisting) { dismiss() }
            }
            .keyboardShortcut(.defaultAction)
            .disabled(preview == nil || selections.isEmpty || store.isLoading)
        }.padding(20)
    }
    private func enabled(_ provider: ProviderImportCandidate) -> Bool { provider.canImport && (!provider.alreadyImported || replaceExisting) }
    private func selectSourceType(_ id: String) {
        sourceTypeID = id
        values = Dictionary(uniqueKeysWithValues: (descriptor?.fields ?? []).map { ($0.key, $0.value) })
        preview = nil
    }
    private func readSource() {
        store.previewProviderImport(source) { value in preview = value; resetSelection() }
    }
    private func resetSelection() {
        selectedModels = Set((preview?.providers ?? []).filter { enabled($0) }.flatMap { $0.models.filter(\.canImport).map(\.id) })
    }
}
