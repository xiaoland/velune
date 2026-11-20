import SwiftUI
#if canImport(VeluneBindings)
import VeluneBindings
#endif

/// A page-owned draft, retained by the presenting Apple view across navigation.
@MainActor final class RemoteNetworkSettingsModel: ObservableObject {
    enum Mode: String, CaseIterable {
        case publicService, custom, disabled
        var title: String {
            switch self {
            case .publicService: return "公共中继"
            case .custom: return "自定义服务器"
            case .disabled: return "不使用中继服务器"
            }
        }
        var explanation: String {
            switch self {
            case .publicService: return "自动使用公共中继；能够直连时仍优先直连。"
            case .custom: return "两端都需能访问所选服务器。每台服务器可单独设置访问令牌。"
            case .disabled: return "本机只建立直接连接，仍可经可信设备中转。"
            }
        }
    }
    struct Server: Identifiable {
        let id = UUID()
        var url: String
        var token = ""
        var savedTokenURL: String?
        var removeToken = false
        var editingToken = false
        var hasSavedToken: Bool { savedTokenURL.map { Self.key(url) == Self.key($0) } ?? false }
        var hasCredential: Bool { savedTokenURL != nil || !token.isEmpty }
        static func key(_ value: String) -> String { RemoteNetworkSettingsModel.urlKey(value) }
    }
    @Published var mode = Mode.publicService
    @Published var servers: [Server] = []
    @Published var otherServers: [Server] = []
    @Published private(set) var network: NetworkState?
    @Published private(set) var failure: String?
    @Published private(set) var busy = false
    private struct SavedButUnapplied: Error { let state: NetworkState }
    private var baseline: Draft?
    private var revision: UInt64?
    private var operation = 0

    private struct Credential: Equatable {
        let url: String
        let token: String
        let savedURL: String?
        let removed: Bool
    }
    private struct Draft: Equatable {
        let mode: Mode
        let urls: [String]
        let credentials: [Credential]
    }
    private var credentialServers: [Server] { servers.filter(\.hasCredential) + otherServers }
    private var draft: Draft {
        Draft(mode: mode, urls: mode == .custom ? servers.map { Self.urlKey($0.url) } : [],
            credentials: credentialServers.map {
                Credential(url: Self.urlKey($0.url), token: $0.token, savedURL: $0.savedTokenURL, removed: $0.removeToken)
            }.sorted { $0.url < $1.url })
    }
    var hasChanges: Bool { baseline.map { $0 != draft } ?? false }
    var hasConflict: Bool { revision != nil && network?.saved?.revision != revision }
    var canSave: Bool { revision != nil && hasChanges && validation == nil && !hasConflict && !busy }
    var validation: String? {
        if mode == .custom {
            if servers.isEmpty { return "请添加至少一台服务器。" }
            if servers.count > 16 { return "最多可添加 16 台中继服务器。" }
            if servers.contains(where: { !Self.validURL($0.url) }) { return "请填写有效的 HTTP 或 HTTPS 服务器根地址，不含账号、路径或查询参数。" }
            if Set(servers.map { Self.urlKey($0.url) }).count != servers.count { return "服务器地址重复，请保留一条。" }
        }
        if mode == .disabled, network?.saved?.pathPolicy == .relayOnly { return "当前本机连接策略要求使用中继服务器，请选择公共中继或自定义服务器。" }
        let active = credentialServers.filter { !$0.removeToken }
        if active.count > 64 { return "最多可保存 64 台服务器的访问资格。" }
        if active.contains(where: { !Self.validURL($0.url) }) { return "请补全访问令牌对应的服务器地址。" }
        if Set(active.map { Self.urlKey($0.url) }).count != active.count { return "同一服务器只需一份访问令牌。" }
        for server in active {
            if server.token.isEmpty && !server.hasSavedToken { return "请填写访问令牌，或移除不需要的认证条目。" }
            if !server.token.isEmpty {
                if server.token.utf8.count > 8192 || !server.token.utf8.allSatisfy({ $0 >= 33 && $0 <= 126 }) {
                    return "令牌不能包含空格或非 ASCII 字符，且不能超过 8192 字节。"
                }
                let url = URLComponents(string: Self.urlKey(server.url))
                let host = url?.host?.trimmingCharacters(in: CharacterSet(charactersIn: "[]")) ?? ""
                let octets = host.split(separator: ".", omittingEmptySubsequences: false).compactMap { Int($0) }
                let loopback = host == "::1" || (octets.count == 4 && octets[0] == 127 && octets.allSatisfy { (0...255).contains($0) })
                if url?.scheme != "https" && !loopback {
                    return "带访问令牌的服务器需使用 HTTPS。"
                }
            }
        }
        return nil
    }

    nonisolated static func urlKey(_ value: String) -> String {
        let value = value.trimmingCharacters(in: .whitespacesAndNewlines)
        guard var url = URLComponents(string: value) else { return value }
        url.scheme = url.scheme?.lowercased()
        url.host = url.host?.lowercased()
        if (url.scheme == "https" && url.port == 443) || (url.scheme == "http" && url.port == 80) { url.port = nil }
        if url.path.isEmpty { url.path = "/" }
        return url.string ?? value
    }
    nonisolated static func validURL(_ value: String) -> Bool {
        let value = value.trimmingCharacters(in: .whitespacesAndNewlines)
        guard !value.contains(where: \.isWhitespace), let url = URLComponents(string: value),
              ["http", "https"].contains(url.scheme?.lowercased() ?? ""), let host = url.host, !host.isEmpty,
              url.user == nil, url.password == nil, url.query == nil, url.fragment == nil,
              url.path.isEmpty || url.path == "/" else { return false }
        return url.url != nil
    }

    /// Move an existing qualification next to its server, without changing the saved token.
    func attachCredential(to id: UUID) {
        guard let index = servers.firstIndex(where: { $0.id == id }), !servers[index].hasCredential,
              let other = otherServers.firstIndex(where: { Self.urlKey($0.url) == Self.urlKey(servers[index].url) }) else { return }
        let credential = otherServers.remove(at: other)
        servers[index].savedTokenURL = credential.savedTokenURL
        servers[index].token = credential.token
        servers[index].removeToken = credential.removeToken
        servers[index].editingToken = credential.editingToken
    }

    func discard() {
        guard let network else { return }
        install(network)
        failure = nil
    }
    private func install(_ state: NetworkState) {
        network = state
        guard let saved = state.saved else { return }
        revision = saved.revision
        var remaining = saved.credentials.filter(\.configured).map { Server(url: $0.url, savedTokenURL: $0.url) }
        switch saved.relay {
        case .developmentPublic: mode = .publicService; servers = []
        case .disabled: mode = .disabled; servers = []
        case .custom(let urls):
            mode = .custom
            servers = urls.map { url in
                if let index = remaining.firstIndex(where: { Self.urlKey($0.url) == Self.urlKey(url) }) { return remaining.remove(at: index) }
                return Server(url: url)
            }
        }
        otherServers = remaining
        baseline = draft
    }

    func observe(_ endpoint: @escaping @Sendable () throws -> VeluneRemoteEndpoint) async {
        if revision == nil { await load(endpoint) }
        while !Task.isCancelled {
            do { try await Task.sleep(for: .seconds(2)) } catch { return }
            guard !busy else { continue }
            let generation = operation
            let result = await Task.detached { Result { try endpoint().readNetworkState() } }.value
            guard !Task.isCancelled, generation == operation else { continue }
            if case .success(let state) = result { network = state }
        }
    }
    func load(_ endpoint: @escaping @Sendable () throws -> VeluneRemoteEndpoint) async {
        await perform { try endpoint().readNetworkState() } success: { state in
            guard state.saved != nil else { self.failure = "本机连接设置尚未就绪，请稍后重试。"; return }
            self.install(state)
        }
    }
    func save(_ endpoint: @escaping @Sendable () throws -> VeluneRemoteEndpoint) async {
        guard canSave, let revision else { return }
        let relay: RelayConfig
        switch mode {
        case .publicService: relay = .developmentPublic
        case .disabled: relay = .disabled
        case .custom: relay = .custom(urls: servers.map { Self.urlKey($0.url) })
        }
        let edits = credentialServers.compactMap { server -> RelayCredentialEdit? in
            guard !server.removeToken else { return nil }
            return RelayCredentialEdit(url: Self.urlKey(server.url), credential: server.token.isEmpty ? .keep : .set(value: server.token))
        }
        let expectedURLs = Set(edits.map(\.url))
        await perform {
            do { return try endpoint().applyRelaySettings(expectedRevision: revision, relay: relay, credentials: edits) }
            catch {
                // Applying can fail after the complete target has already been persisted.
                // A conflict or an unchanged revision does not prove this draft was saved.
                switch error {
                case BindingError.Diagnostic(_, _, let code, _, _), BindingError.RemoteRequest(_, _, let code, _, _):
                    if code == "configuration_conflict" { throw error }
                default: break
                }
                if let state = try? endpoint().readNetworkState(), let saved = state.saved,
                   state.phase == .failed, saved.revision > revision, saved.relay == relay,
                   Set(saved.credentials.filter(\.configured).map { Self.urlKey($0.url) }) == expectedURLs {
                    throw SavedButUnapplied(state: state)
                }
                throw error
            }
        } success: { self.install($0) }
    }
    func retry(_ endpoint: @escaping @Sendable () throws -> VeluneRemoteEndpoint) async {
        await perform { try endpoint().retryNetwork() } success: { self.network = $0 }
    }
    private func perform(_ work: @escaping @Sendable () throws -> NetworkState, success: (NetworkState) -> Void) async {
        guard !busy else { return }
        busy = true; failure = nil; operation += 1
        let result = await Task.detached { Result { try work() } }.value
        switch result {
        case .success(let state): success(state)
        case .failure(let saved as SavedButUnapplied):
            install(saved.state)
            failure = "设置已保存，但应用失败。可重试已保存的设置，或继续编辑。"
        case .failure(let error): failure = "操作未完成，编辑内容已保留。\(ClientTelemetry.message(error))"
        }
        busy = false
    }
}

/// Shared native form; saved target, local application and reachability are separate facts.
struct RemoteNetworkSettingsView: View {
    @ObservedObject var model: RemoteNetworkSettingsModel
    let endpoint: @Sendable () throws -> VeluneRemoteEndpoint
    var onClose: (() -> Void)? = nil
    @Environment(\.dismiss) private var dismiss
    @State private var confirmsLeave = false
    @State private var confirmsReload = false
    @State private var removingServer: UUID?
    @State private var advanced = false

    var body: some View {
        VStack(spacing: 0) {
            if onClose != nil {
                HStack {
                    Text("连接设置").font(.headline)
                    Spacer()
                    Button("完成", action: requestClose).disabled(model.busy)
                }.padding()
            }
            Form {
                summary
                Section {
                    Picker("连接方式", selection: $model.mode) {
                        ForEach(RemoteNetworkSettingsModel.Mode.allCases, id: \.self) { Text($0.title).tag($0) }
                    }.accessibilityIdentifier("remote.relay.mode")
                    Text(model.mode.explanation).font(.caption).foregroundStyle(.secondary)
                } header: { Text("本机如何连接") }
                .disabled(model.busy || model.network?.saved == nil)
                if model.mode == .custom {
                    Section("中继服务器") {
                        ForEach($model.servers) { $server in
                            RemoteRelayServerRow(server: $server, isRelay: true) { removingServer = server.id }
                                .onChange(of: server.url) { _, _ in model.attachCredential(to: server.id) }
                        }
                        Button("添加服务器", systemImage: "plus") { model.servers.append(.init(url: "")) }
                            .disabled(model.servers.count >= 16).accessibilityIdentifier("remote.relay.server.add")
                    }.disabled(model.busy)
                }
                advancedSettings
            }.formStyle(.grouped)
                .safeAreaInset(edge: .bottom, spacing: 0) {
                    VStack(spacing: 0) {
                        Divider()
                        saveActions
                    }.background(.bar)
                }
        }
        .interactiveDismissDisabled(model.hasChanges || model.busy)
        .confirmationDialog("有未保存的更改", isPresented: $confirmsLeave, titleVisibility: .visible) {
            Button("保留编辑并离开") { close() }
            Button("放弃更改并离开", role: .destructive) { model.discard(); close() }
            Button("继续编辑", role: .cancel) {}
        } message: { Text("保留编辑后，可在当前窗口重新打开此页继续。放弃更改会恢复已保存的设置。") }
        .confirmationDialog("重新载入已保存的设置？", isPresented: $confirmsReload, titleVisibility: .visible) {
            Button("放弃编辑并重新载入", role: .destructive) { Task { await model.load(endpoint) } }
            Button("继续编辑", role: .cancel) {}
        }
        .confirmationDialog("移除此服务器条目？", isPresented: Binding(get: { removingServer != nil }, set: { if !$0 { removingServer = nil } }), titleVisibility: .visible) {
            Button("移除", role: .destructive) {
                model.servers.removeAll { $0.id == removingServer }
                model.otherServers.removeAll { $0.id == removingServer }
                removingServer = nil
            }
            Button("取消", role: .cancel) { removingServer = nil }
        } message: { Text("保存后将移除此条目及对应令牌。设备身份和配对不受影响。") }
        .task { await model.observe(endpoint) }
        .onChange(of: model.mode) { _, mode in
            if mode == .custom && model.servers.isEmpty { model.servers.append(.init(url: "")) }
        }
        #if os(iOS)
        .navigationBarBackButtonHidden(model.hasChanges || model.busy)
        .toolbar {
            if model.hasChanges || model.busy {
                ToolbarItem(placement: .cancellationAction) { Button("返回", action: requestClose).disabled(model.busy) }
            }
        }
        #endif
    }

    private var advancedSettings: some View {
        Section {
            DisclosureGroup("高级与诊断", isExpanded: $advanced) {
                VStack(alignment: .leading, spacing: 12) {
                    Text("“已保存”表示本机已记录选择；“已应用”表示本机已采用设置，不代表服务器已经连通。")
                        .font(.caption).foregroundStyle(.secondary)
                    Button(model.hasChanges ? "放弃编辑并重新载入…" : "重新载入") { requestReload() }
                        .accessibilityIdentifier("remote.relay.reload")
                    Divider()
                    Text("其它服务器访问资格").font(.headline)
                    Text("连接使用其它中继的设备时，可能需要对应的访问令牌；邀请不会传递令牌。")
                        .font(.caption).foregroundStyle(.secondary)
                    if model.mode != .custom {
                        ForEach($model.servers) { $server in
                            if server.hasCredential {
                                RemoteRelayServerRow(server: $server, isRelay: false, remove: { removingServer = server.id })
                            }
                        }
                    }
                    ForEach($model.otherServers) { $server in
                        RemoteRelayServerRow(server: $server, isRelay: false) { removingServer = server.id }
                    }
                    Button("添加其它服务器令牌") { model.otherServers.append(.init(url: "", editingToken: true)) }
                        .disabled(model.network?.saved == nil || model.otherServers.count + model.servers.filter(\.hasCredential).count >= 64)
                        .accessibilityIdentifier("remote.relay.credential.add")
                    if let network = model.network, !network.facilities.isEmpty {
                        Divider()
                        Text("服务诊断").font(.headline)
                        ForEach(Array(network.facilities.enumerated()), id: \.offset) { _, facility in
                            VStack(alignment: .leading, spacing: 4) {
                                LabeledContent {
                                    Text(reachabilityText(facility.reachability))
                                } label: {
                                    Text(facility.kind == .homeRelay ? "中继服务器" : "地址查找")
                                    Text(facility.url).font(.caption).foregroundStyle(.secondary).textSelection(.enabled)
                                }
                                if let advice = facilityAdvice(facility) {
                                    Text(advice).font(.caption).foregroundStyle(.secondary)
                                }
                            }
                        }
                    }
                }.frame(maxWidth: .infinity, alignment: .leading)
            }
        }.disabled(model.busy)
    }

    private var summary: some View {
        Section {
            if let state = model.network, let saved = state.saved {
                VStack(spacing: 6) {
                    LabeledContent("已保存方式", value: modeTitle(saved.relay))
                    LabeledContent("应用状态") {
                        Text(phaseText(state)).foregroundStyle(state.phase == .failed ? Color.red : Color.secondary)
                            .accessibilityIdentifier("remote.relay.state")
                    }
                    LabeledContent("服务可达性", value: reachabilitySummary(state))
                }
                if state.phase == .failed {
                    Button("重试已保存的设置") { Task { await model.retry(endpoint) } }.disabled(model.busy)
                }
                if let advice = relayFailureAdvice(state) {
                    Text(advice).font(.caption).foregroundStyle(.secondary)
                    Button("查看服务诊断") { advanced = true }
                }
            } else {
                Text(model.busy ? "正在读取本机连接设置…" : "本机连接设置尚未就绪").foregroundStyle(.secondary)
            }
        } header: { Text("本机连接状态") }
    }
    private var saveActions: some View {
        VStack(alignment: .leading, spacing: 8) {
            if model.hasConflict {
                Text("设置已在另一处更改，编辑内容已保留。请重新载入后再保存。")
                    .foregroundStyle(.orange).fixedSize(horizontal: false, vertical: true)
                Button("重新载入…") { requestReload() }.disabled(model.busy)
            } else if model.hasChanges, let validation = model.validation {
                Text(validation).foregroundStyle(.red).fixedSize(horizontal: false, vertical: true)
                    .accessibilityIdentifier("remote.relay.validation")
            } else if let failure = model.failure {
                Text(failure).foregroundStyle(.red).textSelection(.enabled).fixedSize(horizontal: false, vertical: true)
                    .accessibilityIdentifier("remote.relay.failure")
                Button("重新载入…") { requestReload() }.disabled(model.busy)
            }
            HStack {
                Button("保存并应用") { Task { await model.save(endpoint) } }
                    .buttonStyle(.borderedProminent)
                    .disabled(!model.canSave).accessibilityIdentifier("remote.relay.save")
                if model.busy { ProgressView().controlSize(.small) }
                else if model.hasChanges { Text("有未保存的更改").foregroundStyle(.secondary).accessibilityIdentifier("remote.relay.draft-state") }
                else if model.network?.saved != nil { Label("已保存", systemImage: "checkmark.circle").foregroundStyle(.secondary) }
                Spacer(minLength: 0)
            }
        }
        .padding(.horizontal, 16).padding(.vertical, 12)
        .frame(maxWidth: .infinity, alignment: .leading)
        .accessibilityIdentifier("remote.relay.actions")
    }
    private func requestReload() { if model.hasChanges { confirmsReload = true } else { Task { await model.load(endpoint) } } }
    private func requestClose() { if model.hasChanges { confirmsLeave = true } else { close() } }
    private func close() { if let onClose { onClose() } else { dismiss() } }
    private func modeTitle(_ relay: RelayConfig) -> String {
        switch relay {
        case .developmentPublic: return "公共中继"
        case .custom(let urls): return "自定义服务器（\(urls.count) 台）"
        case .disabled: return "不使用中继服务器"
        }
    }
    private func phaseText(_ state: NetworkState) -> String {
        switch state.phase {
        case .unapplied: return "尚未应用"
        case .applying: return "正在应用"
        case .applied: return state.appliedRevision == state.saved?.revision ? "已应用" : "等待应用已保存的设置"
        case .failed: return "应用失败，设置仍已保存"
        }
    }
    private func reachabilitySummary(_ state: NetworkState) -> String {
        if case .disabled = state.saved?.relay { return "本机未启用中继服务器" }
        let relays = state.facilities.filter { $0.kind == .homeRelay }
        if relays.contains(where: { $0.reachability == .reachable }) { return "中继可达" }
        if relays.contains(where: { $0.reachability == .authenticationDenied }) { return "中继认证被拒绝" }
        if relays.contains(where: { $0.reachability == .unavailable }) { return "中继暂时不可达" }
        if relays.contains(where: { $0.reachability == .connecting }) { return "正在连接中继" }
        return "尚未取得连接结果"
    }
    private func relayFailureAdvice(_ state: NetworkState) -> String? {
        if case .disabled = state.saved?.relay { return nil }
        let relays = state.facilities.filter { $0.kind == .homeRelay }
        guard !relays.contains(where: { $0.reachability == .reachable }) else { return nil }
        if relays.contains(where: { $0.reachability == .authenticationDenied }) {
            return "中继服务器拒绝了认证，请核对该服务器保存的访问令牌。"
        }
        if relays.contains(where: { $0.reachability == .unavailable }) {
            return "暂时无法连接中继服务器，请检查本机网络和服务器地址。"
        }
        return nil
    }
    private func facilityAdvice(_ facility: FacilityStatus) -> String? {
        switch facility.reachability {
        case .authenticationDenied:
            return facility.kind == .homeRelay ? "请核对这台服务器的访问令牌。" : "请检查此地址查找服务的访问要求。"
        case .unavailable:
            return facility.kind == .homeRelay ? "请检查本机网络及这台服务器的地址。" : "请检查本机网络，或稍后重试地址查找服务。"
        case .unknown, .connecting, .reachable: return nil
        }
    }
    private func reachabilityText(_ value: FacilityReachability) -> String {
        switch value {
        case .unknown: return "尚未取得连接结果"
        case .connecting: return "正在连接"
        case .reachable: return "可达"
        case .unavailable: return "无法连接"
        case .authenticationDenied: return "认证被拒绝"
        }
    }
}

private struct RemoteRelayServerRow: View {
    @Binding var server: RemoteNetworkSettingsModel.Server
    let isRelay: Bool
    let remove: () -> Void
    var body: some View {
        VStack(alignment: .leading, spacing: 8) {
            if isRelay || server.savedTokenURL == nil {
                TextField("服务器地址", text: $server.url, prompt: Text("https://relay.example.com"))
                    .textContentType(.URL).relayURLInput().accessibilityIdentifier("remote.relay.server.url")
            } else { Text(server.url).textSelection(.enabled) }
            if server.removeToken {
                Text("令牌将在保存后移除").font(.caption).foregroundStyle(.secondary)
                Button("撤销移除令牌") { server.removeToken = false }
            } else if server.editingToken {
                SecureField(server.hasSavedToken ? "新访问令牌（留空保留）" : "访问令牌", text: $server.token)
                    .relayURLInput().accessibilityIdentifier("remote.relay.credential.token")
                if server.hasSavedToken {
                    Text(server.token.isEmpty ? "令牌已保存；留空保留原令牌。" : "原令牌已保存；新令牌将在保存时替换。")
                        .font(.caption).foregroundStyle(.secondary)
                }
                Button("取消令牌编辑") { server.token = ""; server.editingToken = false }
            } else if server.hasSavedToken {
                Label("令牌已保存", systemImage: "checkmark.circle").font(.caption).foregroundStyle(.secondary)
                HStack {
                    Button("更换令牌") { server.editingToken = true }
                    Button("移除令牌", role: .destructive) { server.removeToken = true; server.token = "" }
                }
            } else {
                Text(server.token.isEmpty ? "未设置访问令牌" : "令牌将在保存时更新").font(.caption).foregroundStyle(.secondary)
                Button("设置访问令牌（可选）") { server.editingToken = true }
            }
            if server.savedTokenURL != nil && !server.hasSavedToken && !server.removeToken {
                Text("地址已更改，请重新提供令牌；原令牌不会发送给新地址。").font(.caption).foregroundStyle(.secondary)
                Button("移除原令牌") { server.removeToken = true; server.token = ""; server.editingToken = false }
            }
            Button(isRelay ? "移除服务器…" : "移除此访问资格…", role: .destructive, action: remove)
                .accessibilityIdentifier("remote.relay.credential.remove")
        }
    }
}

private extension View {
    @ViewBuilder func relayURLInput() -> some View {
        #if os(iOS)
        self.textInputAutocapitalization(.never).autocorrectionDisabled()
        #else
        self
        #endif
    }
}
