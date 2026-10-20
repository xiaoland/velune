import SwiftUI

/// Adding a provider starts by choosing what it is, as Mail does for accounts:
/// a subscription template fixes how the provider is reached, the API template
/// stays editable, and importing reuses an agent runtime's provider. The
/// choice then continues in the same sheet.
struct ProviderCreationView: View {
    @ObservedObject var store: AppStore
    @Environment(\.dismiss) private var dismiss
    private enum Choice: Hashable { case template(String), runtimeImport }
    private enum Step { case choosing, editing(ProviderTemplate), importing }
    @State private var choice: Choice?
    @State private var step = Step.choosing
    private var chosenTemplate: ProviderTemplate? {
        guard case .template(let id) = choice else { return nil }
        return store.providerTemplates.first { $0.id == id }
    }
    var body: some View {
        switch step {
        case .choosing: picker
        case .editing(let template): ProviderEditor(store: store, provider: nil, template: template)
        case .importing: ProviderImportView(store: store)
        }
    }
    private var picker: some View {
        VStack(spacing: 0) {
            Text("添加 AI 提供商").font(.headline).frame(maxWidth: .infinity, alignment: .leading).padding(20)
            Divider()
            List(selection: $choice) {
                section("订阅", kind: .subscription, systemImage: "person.crop.circle")
                section("API", kind: .api, systemImage: "key")
                if !store.providerImportTypes.isEmpty {
                    Section("导入") {
                        row("从 Agent 运行时导入", summary: "沿用 Agent 运行时已配置的提供商、模型与认证来源。", systemImage: "square.and.arrow.down").tag(Choice.runtimeImport)
                    }
                }
            }
            .contextMenu(forSelectionType: Choice.self) { _ in } primaryAction: { ids in if ids.count == 1 { choice = ids.first; proceed() } }
            .listStyle(.inset)
            if let template = chosenTemplate, template.kind == .subscription {
                Divider()
                VStack(alignment: .leading, spacing: 8) {
                    if let disclosure = template.disclosure { SubscriptionDisclosureNote(disclosure: disclosure) }
                    Text("一个提供商关联一个订阅账户。聚合多个账户时分别添加，再在“模型路由”中组合。").font(.caption).foregroundStyle(.secondary).fixedSize(horizontal: false, vertical: true)
                }.frame(maxWidth: .infinity, alignment: .leading).padding(16)
            }
            Divider()
            HStack {
                Spacer()
                Button("取消") { dismiss() }.keyboardShortcut(.cancelAction)
                Button("继续") { proceed() }.keyboardShortcut(.defaultAction).disabled(choice == nil)
            }.padding(16)
        }.frame(width: 650, height: 510)
        .onAppear { if choice == nil { choice = store.providerTemplates.first.map { .template($0.id) } } }
    }
    @ViewBuilder private func section(_ title: String, kind: ProviderTemplate.Kind, systemImage: String) -> some View {
        let templates = store.providerTemplates.filter { $0.kind == kind }
        if !templates.isEmpty {
            Section(title) { ForEach(templates) { row($0.name, summary: $0.summary, systemImage: systemImage).tag(Choice.template($0.id)) } }
        }
    }
    private func row(_ name: String, summary: String, systemImage: String) -> some View {
        HStack(spacing: 12) {
            Image(systemName: systemImage).font(.title2).foregroundStyle(.secondary).frame(width: 28)
            VStack(alignment: .leading, spacing: 2) {
                Text(name)
                Text(summary).font(.caption).foregroundStyle(.secondary).fixedSize(horizontal: false, vertical: true)
            }
        }.padding(.vertical, 4)
    }
    private func proceed() {
        switch choice {
        case .template: if let chosenTemplate { step = .editing(chosenTemplate) }
        case .runtimeImport: step = .importing
        case nil: break
        }
    }
}
