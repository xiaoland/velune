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
    var id: String { candidateKey }
    var candidateKey: String
    var providerModelID: String
    var name: String
    var contextWindow: UInt32?
    var maxOutputTokens: UInt32?
    var reasoningLevels: [String]
    var canImport: Bool
    var issues: [String]
}
struct ProviderImportSelection: Encodable, Sendable {
    var providerId: String
    var candidateKeys: [String]
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
    @State private var providerID: String?
    @State private var selectedCandidateKey: String?
    @State private var query = ""
    @State private var onlyImportable = false
    @State private var selectedModels: Set<String> = []
    @State private var replaceExisting = false
    @State private var attemptedOperation = false

    private var supportedInstances: [RuntimeInstance] { store.runtimeInstances.filter { instance in store.providerImportTypes.contains { $0.id == instance.typeID } } }
    private var selectedInstance: RuntimeInstance? { supportedInstances.first { $0.id == sourceInstanceID } }
    private var source: ProviderImportSource? { guard let selectedInstance else { return nil }; return ProviderImportSource(harnessTypeId: selectedInstance.typeID, sourceInstanceId: selectedInstance.id, settings: [:]) }
    private var provider: ProviderImportCandidate? { preview?.providers.first { $0.id == providerID } }
    private var model: ProviderImportModel? { provider?.models.first { $0.id == selectedCandidateKey } }
    private var visibleModels: [ProviderImportModel] {
        guard let provider else { return [] }
        return provider.models.filter { model in
            (!onlyImportable || (enabled(provider) && model.canImport)) &&
            (query.isEmpty || model.name.localizedCaseInsensitiveContains(query) || model.providerModelID.localizedCaseInsensitiveContains(query))
        }
    }
    private var selections: [ProviderImportSelection] {
        (preview?.providers ?? []).filter { enabled($0) }.compactMap { provider in
            let ids = provider.models.filter { $0.canImport && selectedModels.contains($0.id) }.map(\.id)
            guard !ids.isEmpty else { return nil }
            return ProviderImportSelection(providerId: provider.id, candidateKeys: ids)
        }
    }
    private var selectedCount: Int { selections.reduce(0) { $0 + $1.candidateKeys.count } }

    var body: some View {
        VStack(alignment: .leading, spacing: 0) {
            HStack(spacing: 12) {
                Text("导入提供商").font(.headline)
                Spacer()
                Picker("来源", selection: Binding(get: { sourceInstanceID ?? "" }, set: { sourceInstanceID = $0 })) {
                    ForEach(supportedInstances) { Text($0.name).tag($0.id) }
                }
                .frame(maxWidth: 330)
                .disabled(store.isLoading || supportedInstances.isEmpty)
                Button(preview == nil ? "读取配置" : "重新读取") { readSource() }
                    .disabled(source == nil || store.isLoading)
                if store.isLoading { ProgressView().controlSize(.small) }
            }
            .padding(16)
            if let preview {
                if preview.providers.isEmpty {
                    ContentUnavailableView("没有发现提供商", systemImage: "shippingbox", description: Text("此运行时没有可读取的提供商配置。"))
                        .frame(maxWidth: .infinity, maxHeight: .infinity)
                } else {
                    browser(preview)
                }
            } else {
                ContentUnavailableView(supportedInstances.isEmpty ? "没有可导入的运行时" : "读取运行时配置", systemImage: "square.and.arrow.down", description: Text(supportedInstances.isEmpty ? "请先在运行时设置中配置支持提供商导入的 Agent 运行时。" : "选择已配置的运行时，然后读取提供商与模型预览。"))
                    .frame(maxWidth: .infinity, maxHeight: .infinity)
            }
            Divider()
            footer
        }
        .frame(width: 1020, height: 680)
        .onAppear { selectSourceInstance(sourceInstanceID ?? supportedInstances.first?.id) }
        .onChange(of: sourceInstanceID) { _, value in selectSourceInstance(value) }
        .onChange(of: providerID) { _, _ in selectedCandidateKey = nil; query = "" }
        .onChange(of: replaceExisting) { _, _ in
            selectedModels = Set(selections.flatMap(\.candidateKeys))
        }
    }

    private func browser(_ preview: ProviderImportPreview) -> some View {
        HSplitView {
            List(selection: $providerID) {
                Section("提供商") {
                    ForEach(preview.providers) { candidate in
                        VStack(alignment: .leading, spacing: 3) {
                            Text(candidate.name).lineLimit(1)
                            Text(providerStatus(candidate)).font(.caption).foregroundStyle(.secondary)
                        }
                        .padding(.vertical, 2)
                        .tag(candidate.id)
                    }
                }
            }
            .listStyle(.sidebar)
            .frame(minWidth: 170, idealWidth: 190, maxWidth: 230)
            VStack(alignment: .leading, spacing: 0) {
                if let provider {
                    HStack(spacing: 10) {
                        TextField(onlyImportable ? "搜索可导入模型" : "搜索模型", text: $query).textFieldStyle(.roundedBorder)
                        Menu {
                            Toggle("仅显示可导入模型", isOn: $onlyImportable)
                            Divider()
                            Button("选择筛选结果中可导入的模型") { selectVisibleModels() }
                                .disabled(!visibleModels.contains { enabled(provider) && $0.canImport })
                            Button("清除该提供商的全部选择") {
                                selectedModels.subtract(provider.models.map(\.id))
                            }
                            .disabled(!provider.models.contains { selectedModels.contains($0.id) })
                        } label: { Image(systemName: onlyImportable ? "line.3.horizontal.decrease.circle.fill" : "ellipsis.circle") }
                        .menuStyle(.borderlessButton)
                        .fixedSize()
                        .help(onlyImportable ? "仅显示可导入模型；打开以更改筛选及选择。" : "筛选及批量选择模型")
                    }
                    .padding(10)
                    Table(visibleModels, selection: $selectedCandidateKey) {
                        TableColumn("导入") { model in
                            Toggle("导入 \(model.name)", isOn: selectionBinding(model))
                                .labelsHidden().toggleStyle(.checkbox)
                                .disabled(!enabled(provider) || !model.canImport)
                        }.width(36)
                        TableColumn("模型") { model in
                            VStack(alignment: .leading, spacing: 3) {
                                Text(model.name.isEmpty ? model.providerModelID : model.name).lineLimit(1)
                                Text(model.providerModelID).font(.caption).foregroundStyle(.secondary).lineLimit(1)
                            }.padding(.vertical, 3)
                        }.width(min: 160, ideal: 210)
                        TableColumn("状态") { model in
                            Text(modelStatus(model, provider: provider)).foregroundStyle(.secondary).font(.caption)
                        }.width(65)
                    }
                    .overlay {
                        if visibleModels.isEmpty {
                            ContentUnavailableView("没有匹配的模型", systemImage: "magnifyingglass", description: Text("尝试其他关键词或关闭筛选。"))
                        }
                    }
                } else {
                    ContentUnavailableView("选择提供商", systemImage: "shippingbox")
                }
            }
            .frame(minWidth: 410, maxWidth: .infinity, maxHeight: .infinity)
            ScrollView {
                VStack(alignment: .leading, spacing: 18) {
                    if let provider {
                        if let model { modelDetails(model, provider: provider) }
                        else {
                            Text(provider.name).font(.headline)
                            Text("\(provider.models.count) 个模型").foregroundStyle(.secondary)
                            Text("选择模型以查看详情与关联。").font(.callout).foregroundStyle(.secondary)
                        }
                        if provider.alreadyImported && !replaceExisting {
                            Text("此提供商已导入，默认跳过。可在“导入选项”中允许替换。")
                                .font(.caption).foregroundStyle(.secondary)
                        }
                        ImmediateDisclosureGroup("提供商信息") {
                            providerDetails(provider).padding(.top, 8)
                        }
                    }
                }
                .frame(maxWidth: .infinity, alignment: .leading)
                .padding(16)
            }
            .frame(minWidth: 260, idealWidth: 290, maxWidth: 350)
        }
        .disabled(store.isLoading)
    }

    private func providerDetails(_ provider: ProviderImportCandidate) -> some View {
        VStack(alignment: .leading, spacing: 10) {
            Grid(alignment: .leading, horizontalSpacing: 8, verticalSpacing: 8) {
                GridRow { Text("名称").foregroundStyle(.secondary); Text(provider.name) }
                GridRow { Text("协议").foregroundStyle(.secondary); Text(store.protocols.first { $0.id.rawValue == provider.protocol }?.name ?? provider.protocol) }
            }
            VStack(alignment: .leading, spacing: 4) {
                Text("服务地址").foregroundStyle(.secondary)
                Text(provider.endpoint).textSelection(.enabled).fixedSize(horizontal: false, vertical: true)
            }
            Text(provider.credentialStatus).foregroundStyle(.secondary)
            issueList(provider.issues)
        }
        .font(.callout)
        .frame(maxWidth: .infinity, alignment: .leading)
    }

    private func modelDetails(_ model: ProviderImportModel, provider: ProviderImportCandidate) -> some View {
        VStack(alignment: .leading, spacing: 12) {
            Text(model.name.isEmpty ? model.providerModelID : model.name).font(.headline)
            Text(model.providerModelID).font(.caption).foregroundStyle(.secondary).textSelection(.enabled)
                .fixedSize(horizontal: false, vertical: true)
            if !enabled(provider) || !model.canImport {
                Label(modelStatus(model, provider: provider), systemImage: "info.circle").foregroundStyle(.secondary)
            }
            issueList(model.issues)
            ImmediateDisclosureGroup("来源能力") {
                Grid(alignment: .leading, horizontalSpacing: 8, verticalSpacing: 8) {
                    GridRow { Text("上下文窗口").foregroundStyle(.secondary); Text(model.contextWindow?.formatted() ?? "未知") }
                    GridRow { Text("最大输出").foregroundStyle(.secondary); Text(model.maxOutputTokens?.formatted() ?? "未知") }
                    GridRow { Text("运行时推理级别").foregroundStyle(.secondary); Text(model.reasoningLevels.isEmpty ? "未声明" : model.reasoningLevels.joined(separator: "、")) }
                }.font(.caption).frame(maxWidth: .infinity, alignment: .leading).padding(.top, 8)
            }
        }
    }

    private func issueList(_ issues: [String]) -> some View {
        ForEach(Array(issues.enumerated()), id: \.offset) { _, issue in
            Text(issue).font(.callout).foregroundStyle(.secondary).fixedSize(horizontal: false, vertical: true)
        }
    }

    private var footer: some View {
        VStack(alignment: .leading, spacing: 10) {
            if let preview, !preview.warnings.isEmpty {
                ImmediateDisclosureGroup("读取提示 · \(preview.warnings.count)") {
                    ScrollView { issueList(preview.warnings).frame(maxWidth: .infinity, alignment: .leading) }
                        .frame(maxHeight: 80)
                }.font(.callout)
            }
            SettingsError(message: attemptedOperation ? store.error : nil)
            HStack {
                if preview != nil {
                    Text("已选 \(selections.count) 个提供商、\(selectedCount) 个模型").foregroundStyle(.secondary)
                        .help("原运行时配置保留，模型路由不会自动改变。")
                    Spacer()
                } else { Spacer() }
                if preview?.providers.contains(where: \.alreadyImported) == true {
                    Menu("导入选项") {
                        Toggle("允许替换此次选中的已导入提供商", isOn: $replaceExisting)
                    }
                    .fixedSize()
                    .help("替换保留已有路由和关联模型参数。")
                    .disabled(store.isLoading)
                }
                Button("取消") { dismiss() }.keyboardShortcut(.cancelAction)
                Button("导入所选") {
                    guard let preview, let source else { return }
                    attemptedOperation = true
                    store.applyProviderImport(source, preview: preview, selections: selections, replaceExisting: replaceExisting) { dismiss() }
                }
                .keyboardShortcut(.defaultAction)
                .disabled(source == nil || preview == nil || selections.isEmpty || store.isLoading)
            }
        }.padding(16)
    }

    private func selectVisibleModels() {
        guard let provider, enabled(provider) else { return }
        selectedModels.formUnion(visibleModels.filter(\.canImport).map(\.id))
    }
    private func selectionBinding(_ model: ProviderImportModel) -> Binding<Bool> {
        Binding(get: { selectedModels.contains(model.id) }, set: { if $0 { selectedModels.insert(model.id) } else { selectedModels.remove(model.id) } })
    }
    private func providerStatus(_ provider: ProviderImportCandidate) -> String {
        let count = provider.models.filter(\.canImport).count
        if !provider.canImport || count == 0 { return "不可导入 · \(provider.models.count) 个模型" }
        return provider.alreadyImported ? "已导入 · \(count) 个可用模型" : "\(count) 个可导入模型"
    }
    private func modelStatus(_ model: ProviderImportModel, provider: ProviderImportCandidate) -> String {
        if !provider.canImport || !model.canImport { return "不支持" }
        if provider.alreadyImported && !replaceExisting { return "已导入" }
        return "可导入"
    }
    private func enabled(_ provider: ProviderImportCandidate) -> Bool { provider.canImport && (!provider.alreadyImported || replaceExisting) }
    private func selectSourceInstance(_ id: String?) {
        sourceInstanceID = id
        preview = nil
        providerID = nil
        selectedCandidateKey = nil
        selectedModels = []
        query = ""
        attemptedOperation = false
    }
    private func readSource() {
        guard let source else { return }
        attemptedOperation = true
        store.previewProviderImport(source) { value in
            preview = value
            providerID = value.providers.first?.id
            selectedCandidateKey = nil
            selectedModels = []
            }
    }
}
