import SwiftUI

struct PublicModelCatalogView: View {
    @ObservedObject var store: AppStore
    @Environment(\.dismiss) private var dismiss
    @State private var models: [CatalogModel] = []
    @State private var search = ""
    @State private var selectedID: String?
    @State private var editor: ModelTemplate?
    private var candidates: [CatalogModel] {
        guard !search.isEmpty else { return models }
        return models.filter { "\($0.name) \($0.modelID) \($0.sourceProviderName)".localizedCaseInsensitiveContains(search) }
    }
    private var selected: CatalogModel? { models.first { $0.id == selectedID } }
    var body: some View {
        VStack(spacing: 0) {
            HStack {
                Text("公开模型目录").font(.headline)
                Spacer()
                Link("models.dev", destination: URL(string: "https://models.dev")!)
            }.padding(20)
            Text("选择来源提供商的模型，复制为可编辑模板。目录信息仅供参考；协议、服务地址与能力参数仍需核对提供商文档。")
                .font(.callout).foregroundStyle(.secondary).padding(.horizontal, 20).padding(.bottom, 12)
            TextField("搜索模型或来源提供商", text: $search).textFieldStyle(.roundedBorder).padding(.horizontal, 20).padding(.bottom, 8)
            List(selection: $selectedID) {
                ForEach(candidates) { model in
                    VStack(alignment: .leading, spacing: 3) {
                        Text(model.name)
                        Text("\(model.sourceProviderName) (\(model.sourceProviderID)) · \(model.modelID)").font(.caption).foregroundStyle(.secondary)
                    }.tag(model.id)
                }
            }.listStyle(.bordered).padding(.horizontal, 20)
            .overlay {
                if !store.isLoading && candidates.isEmpty {
                    Text(models.isEmpty ? "尚无目录内容，可重新拉取。" : "没有匹配的模型").foregroundStyle(.secondary)
                }
            }
            .onChange(of: search) { _, _ in selectedID = nil }
            if let selected {
                HStack {
                    Text("上下文：\(selected.contextWindow.map(String.init) ?? "未知")")
                    Text("输出上限：\(selected.maxOutputTokens.map(String.init) ?? "未知")")
                    Spacer()
                }.font(.caption).foregroundStyle(.secondary).padding(.horizontal, 20).padding(.top, 8)
            }
            if let selected, let levels = selected.reasoningLevels {
                Text("来源声明的推理等级：\(levels.joined(separator: "、"))").font(.caption).foregroundStyle(.secondary).padding(.horizontal, 20).padding(.top, 4)
            }
            SettingsError(message: store.error)
            HStack {
                Button(models.isEmpty ? "拉取目录" : "重新拉取") { fetch() }.disabled(store.isLoading)
                if store.isLoading { ProgressView().controlSize(.small) }
                Spacer()
                Button("完成") { dismiss() }.keyboardShortcut(.cancelAction)
                Button("添加模板…") { editor = selected?.template }.disabled(selected == nil || store.isLoading).keyboardShortcut(.defaultAction)
            }.padding(16)
        }.frame(width: 600, height: 480)
        .sheet(item: $editor) { ModelTemplateEditor(store: store, template: $0) }
        .onAppear { fetch() }
    }
    private func fetch() {
        store.fetchPublicModelCatalog { values in models = values; selectedID = nil }
    }
}
