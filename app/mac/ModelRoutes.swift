import SwiftUI

/// A provider model as a route lists it.
private struct RouteMemberChoice: Identifiable {
    let recordKey: String
    let modelName: String
    let providerName: String
    let protocolID: ProviderProtocol
    var id: String { recordKey }
    var title: String { "\(modelName) · \(providerName)" }
}

private extension AppStore {
    var routeMemberChoices: [RouteMemberChoice] {
        providers.flatMap { provider in
            provider.models.map { model in
                RouteMemberChoice(recordKey: model.recordKey, modelName: model.nickname.isEmpty ? model.providerModelID : model.nickname, providerName: provider.name, protocolID: provider.protocolID)
            }
        }
    }
}

struct ModelRouteSettingsView: View {
    @ObservedObject var store: AppStore
    @State private var selectedID: String?
    @State private var editor: ModelRoute?
    @State private var creating = false
    @State private var deleting: ModelRoute?
    private var routes: [ModelRoute] { store.gateway.routes }
    var body: some View {
        VStack(spacing: 0) {
            List(selection: $selectedID) {
                ForEach(routes) { route in
                    VStack(alignment: .leading, spacing: 3) {
                        Text(route.name)
                        Text(memberSummary(route)).font(.caption).foregroundStyle(.secondary).lineLimit(1)
                    }.tag(route.recordKey)
                }
            }.contextMenu(forSelectionType: String.self) { ids in
                if let route = route(ids.first) { Button("编辑…") { editor = route }; Button("删除…") { deleting = route } }
            } primaryAction: { ids in editor = route(ids.first) }
            .listStyle(.bordered).padding(.horizontal, 20).padding(.top, 16)
            .overlay { if routes.isEmpty { Text("尚无模型路由").foregroundStyle(.secondary) } }
            Text("模型路由把若干提供商模型作为一个模型选择。网关按顺序使用其中的模型；当前模型在开始回复前因限流、服务端错误或连接失败而不可用时，自动改用下一个。认证被拒绝或请求本身无效时不切换。")
                .font(.caption).foregroundStyle(.secondary).frame(maxWidth: .infinity, alignment: .leading).fixedSize(horizontal: false, vertical: true)
                .padding(.horizontal, 20).padding(.top, 8)
            HStack {
                Button { creating = true } label: { Image(systemName: "plus") }.help("添加模型路由").disabled(store.providers.allSatisfy { $0.models.isEmpty })
                Button { deleting = route(selectedID) } label: { Image(systemName: "minus") }.disabled(selectedID == nil)
                Spacer()
                Button("编辑…") { editor = route(selectedID) }.disabled(selectedID == nil)
            }.padding(.horizontal, 20).padding(.vertical, 12)
        }
        .sheet(item: $editor) { ModelRouteEditor(store: store, route: $0) }
        .sheet(isPresented: $creating) { ModelRouteEditor(store: store, route: nil) }
        .alert("删除模型路由？", isPresented: Binding(get: { deleting != nil }, set: { if !$0 { deleting = nil } }), presenting: deleting) { route in
            Button("删除", role: .destructive) { store.deleteModelRoute(route.recordKey); deleting = nil }; Button("取消", role: .cancel) { deleting = nil }
        } message: { route in Text("删除「\(route.name)」。其中的提供商模型保留；选择了此路由的会话需要重新选择模型。") }
    }
    private func route(_ id: String?) -> ModelRoute? { routes.first { $0.recordKey == id } }
    private func memberSummary(_ route: ModelRoute) -> String {
        let choices = store.routeMemberChoices
        return route.members.compactMap { key in choices.first { $0.recordKey == key }?.title }.joined(separator: " → ")
    }
}

private struct ModelRouteEditor: View {
    @ObservedObject var store: AppStore
    let route: ModelRoute?
    @Environment(\.dismiss) private var dismiss
    @State private var name = ""
    @State private var members: [String] = []
    @State private var selection: String?
    @FocusState private var nameFocused: Bool
    private var choices: [RouteMemberChoice] { store.routeMemberChoices }
    /// Members share the first member's protocol.
    private var memberProtocol: ProviderProtocol? { members.first.flatMap { key in choices.first { $0.recordKey == key }?.protocolID } }
    private var addable: [RouteMemberChoice] { choices.filter { !members.contains($0.recordKey) && (memberProtocol == nil || $0.protocolID == memberProtocol) } }
    private var valid: Bool { !name.trimmingCharacters(in: .whitespacesAndNewlines).isEmpty && !members.isEmpty }
    var body: some View {
        VStack(spacing: 0) {
            Text(route == nil ? "添加模型路由" : "编辑模型路由").font(.headline).frame(maxWidth: .infinity, alignment: .leading).padding(20)
            Divider()
            Form {
                Section { TextField("名称", text: $name, prompt: Text("在模型选择中显示")).focused($nameFocused) }
                Section {
                    List(selection: $selection) {
                        ForEach(Array(members.enumerated()), id: \.element) { index, key in
                            HStack {
                                Text("\(index + 1)").monospacedDigit().foregroundStyle(.secondary).frame(width: 18, alignment: .trailing)
                                Text(choices.first { $0.recordKey == key }?.title ?? "已删除的模型")
                            }.tag(key)
                        }.onMove { members.move(fromOffsets: $0, toOffset: $1) }
                    }
                    .contextMenu(forSelectionType: String.self) { keys in
                        if let key = keys.first { Button("移除") { remove(key) } }
                    }
                    .frame(minHeight: 140)
                    .overlay { if members.isEmpty { Text("尚未添加模型").foregroundStyle(.secondary) } }
                    HStack {
                        Menu {
                            ForEach(addable) { choice in Button(choice.title) { members.append(choice.recordKey); selection = choice.recordKey } }
                        } label: { Image(systemName: "plus") }.menuIndicator(.hidden).fixedSize().disabled(addable.isEmpty).help("添加模型")
                        Button { if let selection { remove(selection) } } label: { Image(systemName: "minus") }.disabled(selection == nil).help("移除所选模型")
                        Spacer()
                    }.buttonStyle(.borderless)
                } header: {
                    Text("模型")
                } footer: {
                    Text("按顺序使用，拖动以调整顺序。路由中的模型须使用同一协议；会话使用第一个模型的上下文窗口与 reasoning effort。").font(.caption).foregroundStyle(.secondary)
                }
            }.formStyle(.grouped)
            Divider()
            HStack {
                Spacer()
                Button("取消") { dismiss() }.keyboardShortcut(.cancelAction)
                Button("保存") { commit() }.disabled(!valid || store.isBusy).keyboardShortcut(.defaultAction)
            }.padding(16)
        }
        .frame(width: 520, height: 460)
        .onAppear {
            name = route?.name ?? ""; members = route?.members ?? []
            DispatchQueue.main.async { nameFocused = true }
        }
    }
    private func remove(_ key: String) { members.removeAll { $0 == key }; if selection == key { selection = nil } }
    private func commit() {
        // Editing keeps what this sheet does not show: the strategy, its weights and stickiness.
        let edited = (route ?? ModelRoute(recordKey: "", name: "", members: [])).with(name: name.trimmingCharacters(in: .whitespacesAndNewlines), members: members)
        store.saveModelRoute(edited) { dismiss() }
    }
}
