import SwiftUI

/// A provider model as a route lists it.
struct RouteMember: Identifiable {
    let provider: AIProvider
    let model: ProviderModel
    var id: String { model.recordKey }
    var title: String { "\(model.displayName) · \(provider.name)" }
}

extension AppStore {
    func routeMember(_ recordKey: String) -> RouteMember? {
        for provider in providers { if let model = provider.models.first(where: { $0.recordKey == recordKey }) { return RouteMember(provider: provider, model: model) } }
        return nil
    }
    /// The subscription terms that apply to the route's members, once each.
    func disclosures(of members: [String]) -> [SubscriptionDisclosure] {
        var values: [SubscriptionDisclosure] = []
        for key in members { if let value = routeMember(key)?.provider.authentication.disclosure, !values.contains(value) { values.append(value) } }
        return values
    }
}

private let routingRules = "无论哪种策略，都先跳过需要登录、冷却中或达到使用上限的模型。所选模型在开始回复前因限流、额度、服务端错误或连接失败而失败时，改用下一个；认证被拒绝或请求本身无效时不切换。"

private func stickyExplanation(_ strategy: RoutingStrategy) -> String {
    switch strategy {
    case .rotate, .weight: return "会话保持时，每个新会话分到一个模型，之后继续使用它，直到它不再可用。"
    case .smart, .order: return "会话保持时，会话继续使用上次成功应答的模型，直到它不再可用。"
    }
}

struct ModelRouteSettingsView: View {
    @ObservedObject var store: AppStore
    @State private var selectedID: String?
    @State private var editor: ModelRoute?
    @State private var deleting: ModelRoute?
    @State private var providerEditor: AIProvider?
    private var routes: [ModelRoute] { store.gateway.routes }
    private var selected: ModelRoute? { route(selectedID) }
    var body: some View {
        HStack(alignment: .top, spacing: 0) {
            VStack(spacing: 8) {
                List(selection: $selectedID) {
                    ForEach(routes) { route in
                        VStack(alignment: .leading, spacing: 3) {
                            Text(route.name).lineLimit(1)
                            Text("\(route.strategy.label) · \(route.members.count) 个模型").font(.caption).foregroundStyle(.secondary)
                        }.tag(route.recordKey)
                    }
                }.contextMenu(forSelectionType: String.self) { ids in
                    if let route = route(ids.first) { Button("编辑…") { editor = route }; Button("删除…") { deleting = route } }
                } primaryAction: { ids in editor = route(ids.first) }
                .listStyle(.bordered)
                .overlay { if routes.isEmpty { Text("尚无模型路由").foregroundStyle(.secondary) } }
                HStack {
                    Button { editor = ModelRoute(recordKey: "", name: "", members: []) } label: { Image(systemName: "plus") }.help("添加模型路由").disabled(store.providers.allSatisfy { $0.models.isEmpty })
                    Button { deleting = selected } label: { Image(systemName: "minus") }.help("删除所选模型路由").disabled(selected == nil)
                    Spacer()
                }
            }.frame(width: 190).padding([.leading, .vertical], 16)
            Group {
                if let route = selected {
                    ModelRouteOverview(store: store, route: route, edit: { editor = route }, editProvider: { providerEditor = $0 })
                } else {
                    VStack(spacing: 8) {
                        Text(routes.isEmpty ? "模型路由把多个提供商模型合成一个模型选择，例如聚合几个订阅，由网关按策略挑选并在失败时切换。" : "选择一个模型路由以查看其模型与额度。")
                            .foregroundStyle(.secondary).multilineTextAlignment(.center).frame(maxWidth: 320)
                    }.frame(maxWidth: .infinity, maxHeight: .infinity)
                }
            }.frame(maxWidth: .infinity, maxHeight: .infinity)
        }
        .onAppear { if selected == nil { selectedID = routes.first?.recordKey } }
        .onChange(of: routes.map(\.recordKey)) { old, keys in
            // Select a route just added, or a neighbour of one deleted.
            if let added = keys.first(where: { !old.contains($0) }) { selectedID = added } else if selected == nil { selectedID = keys.first }
        }
        .sheet(item: $editor) { ModelRouteEditor(store: store, route: $0) }
        .sheet(item: $providerEditor) { ProviderEditor(store: store, provider: $0) }
        // Sign-in started here; editors present their own.
        .sheet(isPresented: Binding(get: { store.showsAuthentication && editor == nil && providerEditor == nil }, set: { store.showsAuthentication = $0 })) { AuthenticationView(store: store) }
        .alert("删除模型路由？", isPresented: Binding(get: { deleting != nil }, set: { if !$0 { deleting = nil } }), presenting: deleting) { route in
            Button("删除", role: .destructive) { store.deleteModelRoute(route.recordKey); deleting = nil }; Button("取消", role: .cancel) { deleting = nil }
        } message: { route in Text("删除「\(route.name)」。其中的提供商模型保留；选择了此路由的会话需要重新选择模型。") }
    }
    private func route(_ id: String?) -> ModelRoute? { routes.first { $0.recordKey == id } }
}

/// A route as it stands: how it picks, and each member's login and quota.
private struct ModelRouteOverview: View {
    @ObservedObject var store: AppStore
    let route: ModelRoute
    let edit: () -> Void
    let editProvider: (AIProvider) -> Void
    /// Providers whose quota can be read: those of the members, signed in.
    private var providerIDs: [String] {
        var ids: [String] = []
        for key in route.members { if let provider = store.routeMember(key)?.provider, !provider.authentication.awaitsSignIn, !ids.contains(provider.id) { ids.append(provider.id) } }
        return ids
    }
    private var reportsQuota: Bool { providerIDs.contains { store.providerQuotas[$0].map { $0.freshness != .unsupported } ?? false } }
    var body: some View {
        Form {
            Section {
                LabeledContent("策略", value: route.strategy.label)
                LabeledContent("会话保持", value: route.sticky ? "开启" : "关闭")
            } header: {
                HStack { Text(route.name).font(.headline); Spacer(); Button("编辑…", action: edit).controlSize(.small) }
            } footer: {
                Text(route.strategy.explanation + (route.sticky ? stickyExplanation(route.strategy) : "每个请求重新选择模型。"))
                    .formFooterStyle()
            }
            Section {
                ForEach(Array(route.members.enumerated()), id: \.element) { index, key in
                    if let member = store.routeMember(key) {
                        RouteMemberStatus(store: store, member: member, position: index + 1, weight: route.strategy == .weight ? route.weights[key] : nil, totalWeight: route.weights.values.reduce(0, +))
                            .contentShape(Rectangle())
                            .onTapGesture(count: 2) { editProvider(member.provider) }
                            .contextMenu { Button("编辑提供商…") { editProvider(member.provider) } }
                    }
                }
            } header: {
                HStack {
                    Text("模型")
                    Spacer()
                    if reportsQuota {
                        Button("刷新额度") { store.readProviderQuotas(providerIDs, refresh: true) }.controlSize(.small)
                            .disabled(providerIDs.allSatisfy { store.providerQuotas[$0]?.refreshing == true })
                    }
                }
            } footer: {
                Text(routingRules + " 双击模型以编辑其提供商的登录与使用上限。").formFooterStyle()
            }
            ForEach(store.disclosures(of: route.members), id: \.title) { disclosure in
                Section { SubscriptionDisclosureNote(disclosure: disclosure) }
            }
        }
        .formStyle(.grouped)
        .readsQuota(of: providerIDs, from: store)
    }
}

/// One member: which model of which provider, whether it can be used now, and its quota.
private struct RouteMemberStatus: View {
    @ObservedObject var store: AppStore
    let member: RouteMember
    let position: Int
    let weight: UInt32?
    let totalWeight: UInt32
    private var authentication: ProviderAuthentication { member.provider.authentication }
    private var quota: ProviderQuota? { store.providerQuotas[member.provider.id] }
    private var windows: [QuotaWindow] { (quota?.windows ?? []).filter { $0.applies(to: member.model.providerModelID) } }
    var body: some View {
        VStack(alignment: .leading, spacing: 6) {
            HStack(alignment: .firstTextBaseline, spacing: 8) {
                Text("\(position)").monospacedDigit().foregroundStyle(.secondary).frame(width: 16, alignment: .trailing)
                VStack(alignment: .leading, spacing: 2) {
                    Text(member.title).lineLimit(1)
                    Text(account).font(.caption).foregroundStyle(.secondary).lineLimit(1)
                }
                Spacer()
                if let weight, totalWeight > 0 { Text("权重 \(weight) · \(Int((Double(weight) / Double(totalWeight) * 100).rounded()))%").font(.caption).foregroundStyle(.secondary).monospacedDigit() }
                if needsSignIn, let action = authentication.loginAction {
                    Button(action.label) { store.startAuthentication(member.provider.id) }.controlSize(.small).disabled(store.isBusy)
                }
            }
            Group {
                if let notice { Label(notice, systemImage: "pause.circle").font(.caption).foregroundStyle(.orange) }
                ForEach(Array(windows.enumerated()), id: \.offset) { _, window in
                    QuotaWindowRow(window: window, limit: member.provider.quotaLimits.first { $0.windowID == window.id }?.maxUsedPercent, fresh: quota?.freshness == .fresh)
                }
                if !needsSignIn, let quota, quota.freshness != .unsupported, quota.freshness != .fresh || windows.isEmpty { QuotaStatusText(quota: quota) }
            }.padding(.leading, 24)
        }.padding(.vertical, 2)
    }
    private var needsSignIn: Bool { authentication.awaitsSignIn }
    private var account: String {
        var parts: [String] = []
        if let name = member.provider.composition.subscriptionName { parts.append(name) } else if authentication.method != .unconfigured { parts.append(authentication.method.label) }
        if let label = authentication.accountLabel { parts.append("账户 …\(label)") }
        parts.append(needsSignIn ? "需要登录" : authentication.method == .unconfigured ? "尚未配置认证" : authentication.method == .oauth ? "已登录" : "")
        return parts.filter { !$0.isEmpty }.joined(separator: " · ")
    }
    /// Why the gateway will not send this member new requests, as far as login and quota tell.
    private var notice: String? {
        if needsSignIn { return "登录前路由不会使用它" }
        if authentication.method == .unconfigured { return "配置认证前路由不会使用它" }
        if quota?.available == false { return "提供商表示此账户当前不可用" }
        let limits = member.provider.quotaLimits
        guard !limits.isEmpty else { return nil }
        if quota?.freshness != .fresh { return "设有使用上限，取得最新额度前暂停" }
        return nil
    }
}

private struct ModelRouteEditor: View {
    @ObservedObject var store: AppStore
    let route: ModelRoute
    @Environment(\.dismiss) private var dismiss
    @State private var name = ""
    @State private var strategy: RoutingStrategy = .smart
    @State private var sticky = true
    @State private var members: [String] = []
    @State private var weights: [String: UInt32] = [:]
    @State private var selection: String?
    @FocusState private var nameFocused: Bool
    /// Members share the first member's protocol.
    private var memberProtocol: ProviderProtocol? { members.first.flatMap { store.routeMember($0)?.provider.protocolID } }
    private var totalWeight: UInt32 { members.reduce(0) { $0 + weight($1) } }
    private var valid: Bool { !name.trimmingCharacters(in: .whitespacesAndNewlines).isEmpty && !members.isEmpty }
    var body: some View {
        VStack(spacing: 0) {
            Text(route.recordKey.isEmpty ? "添加模型路由" : "编辑模型路由").font(.headline).frame(maxWidth: .infinity, alignment: .leading).padding(20)
            Divider()
            Form {
                Section { TextField("名称", text: $name, prompt: Text("在模型选择中显示")).focused($nameFocused) }
                Section {
                    Picker("策略", selection: $strategy) { ForEach(RoutingStrategy.allCases) { Text($0.label).tag($0) } }
                    Toggle("会话保持", isOn: $sticky)
                } footer: {
                    Text(strategy.explanation + (sticky ? stickyExplanation(strategy) : "关闭会话保持时，每个请求重新选择模型。"))
                        .formFooterStyle()
                }
                Section {
                    List(selection: $selection) {
                        ForEach(Array(members.enumerated()), id: \.element) { index, key in
                            HStack {
                                Text("\(index + 1)").monospacedDigit().foregroundStyle(.secondary).frame(width: 18, alignment: .trailing)
                                Text(store.routeMember(key)?.title ?? "已删除的模型").lineLimit(1)
                                Spacer()
                                if strategy == .weight {
                                    Text(totalWeight > 0 ? "\(Int((Double(weight(key)) / Double(totalWeight) * 100).rounded()))%" : "").font(.caption).foregroundStyle(.secondary).monospacedDigit()
                                    TextField("权重", value: weightBinding(key), format: .number).labelsHidden().multilineTextAlignment(.trailing).frame(width: 44)
                                    Stepper("权重", value: weightBinding(key), in: 1...999).labelsHidden()
                                }
                            }.tag(key)
                        }.onMove { members.move(fromOffsets: $0, toOffset: $1) }
                    }
                    .contextMenu(forSelectionType: String.self) { keys in
                        if let key = keys.first { Button("移除") { remove(key) } }
                    }
                    .frame(minHeight: 120)
                    .overlay { if members.isEmpty { Text("尚未添加模型").foregroundStyle(.secondary) } }
                    HStack {
                        Menu {
                            ForEach(store.providers.filter { !$0.models.isEmpty }) { provider in
                                Section(provider.name) {
                                    ForEach(provider.models) { model in
                                        Button(model.displayName) { members.append(model.recordKey); selection = model.recordKey }
                                            .disabled(members.contains(model.recordKey) || (memberProtocol.map { $0 != provider.protocolID } ?? false))
                                    }
                                }
                            }
                        } label: { Image(systemName: "plus") }.menuIndicator(.hidden).fixedSize().help("添加模型")
                        Button { if let selection { remove(selection) } } label: { Image(systemName: "minus") }.disabled(selection == nil).help("移除所选模型")
                        Spacer()
                    }.buttonStyle(.borderless)
                } header: {
                    Text("模型")
                } footer: {
                    Text(membersFooter).formFooterStyle()
                }
                ForEach(store.disclosures(of: members), id: \.title) { disclosure in
                    Section { SubscriptionDisclosureNote(disclosure: disclosure) }
                }
            }.formStyle(.grouped)
            Divider()
            HStack {
                Spacer()
                Button("取消") { dismiss() }.keyboardShortcut(.cancelAction)
                Button("保存") { commit() }.disabled(!valid || store.isBusy).keyboardShortcut(.defaultAction)
            }.padding(16)
        }
        .frame(width: 540, height: 540)
        .onAppear {
            name = route.name; strategy = route.strategy; sticky = route.sticky; members = route.members; weights = route.weights
            DispatchQueue.main.async { nameFocused = true }
        }
    }
    private var membersFooter: String {
        let order = strategy == .order ? "按列表顺序使用" : "列表顺序在同等条件下优先"
        return "\(order)，拖动以调整。路由中的模型须使用同一协议，菜单中其他协议的模型不可选；会话使用第一个模型的上下文窗口与 reasoning effort。"
    }
    private func weight(_ key: String) -> UInt32 { weights[key] ?? 1 }
    private func weightBinding(_ key: String) -> Binding<UInt32> {
        Binding(get: { weight(key) }, set: { weights[key] = max(1, $0) })
    }
    private func remove(_ key: String) { members.removeAll { $0 == key }; weights[key] = nil; if selection == key { selection = nil } }
    private func commit() {
        let edited = ModelRoute(recordKey: route.recordKey, name: name.trimmingCharacters(in: .whitespacesAndNewlines), members: members, strategy: strategy,
                                weights: strategy == .weight ? Dictionary(uniqueKeysWithValues: members.map { ($0, weight($0)) }) : [:], sticky: sticky)
        store.saveModelRoute(edited) { dismiss() }
    }
}
