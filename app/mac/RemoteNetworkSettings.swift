import SwiftUI
#if canImport(VeluneBindings)
import VeluneBindings
#endif

/// Shared by the Apple apps. Rust owns saved settings, credentials and network replacement.
struct RemoteNetworkSettingsView: View {
    let endpoint: @Sendable () throws -> VeluneRemoteEndpoint
    @State private var network: NetworkState?
    @State private var revision: UInt64?
    @State private var mode = RelayMode.publicService
    @State private var relayURLs = ""
    @State private var credentials: [CredentialDraft] = []
    @State private var failure: String?
    @State private var busy = false
    @State private var visible = false

    private enum RelayMode: String, CaseIterable {
        case publicService, custom, disabled
        var title: String {
            switch self {
            case .publicService: return "公共中继"
            case .custom: return "自定义中继"
            case .disabled: return "关闭中继"
            }
        }
    }
    private struct CredentialDraft: Identifiable {
        let id = UUID()
        var url: String
        var token = ""
        var saved = false
    }

    var body: some View {
        Form {
            Section("本机中继") {
                Picker("中继服务器", selection: $mode) {
                    ForEach(RelayMode.allCases, id: \.self) { Text($0.title).tag($0) }
                }.accessibilityIdentifier("remote.relay.mode")
                if mode == .custom {
                    TextField("HTTPS 中继地址，每行一个", text: $relayURLs, axis: .vertical)
                        .lineLimit(2...5)
                        .textContentType(.URL)
                        .relayURLInput()
                        .accessibilityIdentifier("remote.relay.urls")
                }
                Text("两端都需要能访问所用中继。地址查找沿用当前设置；关闭中继后，两端需要能够直连。")
                    .font(.caption).foregroundStyle(.secondary)
            }.disabled(busy)
            Section {
                ForEach($credentials) { $credential in
                    VStack(alignment: .leading, spacing: 8) {
                        HStack {
                            if credential.saved {
                                Text(credential.url).textSelection(.enabled)
                            } else {
                                TextField("认证对应的 HTTPS 中继地址", text: $credential.url)
                                    .textContentType(.URL).relayURLInput()
                                    .accessibilityIdentifier("remote.relay.credential.url")
                            }
                            Spacer()
                            Button {
                                credentials.removeAll { $0.id == credential.id }
                            } label: { Image(systemName: "minus.circle") }
                                .accessibilityLabel("移除此中继的认证")
                                .accessibilityIdentifier("remote.relay.credential.remove")
                        }
                        SecureField(credential.saved ? "新访问令牌（留空保留）" : "访问令牌", text: $credential.token)
                            .relayURLInput()
                            .accessibilityIdentifier("remote.relay.credential.token")
                    }
                }
                Button("添加中继认证") { credentials.append(CredentialDraft(url: "")) }
                    .disabled(credentials.count >= 64)
                    .accessibilityIdentifier("remote.relay.credential.add")
            } header: { Text("中继认证") } footer: {
                Text("令牌只发送给对应地址。已保存的令牌不会回显；移除该行并保存即可删除认证。")
            }.disabled(busy)
            Section {
                HStack {
                    Button("保存") { save() }.disabled(revision == nil || busy).accessibilityIdentifier("remote.relay.save")
                    Button("重新载入") { load() }.disabled(busy).accessibilityIdentifier("remote.relay.reload")
                    if busy { ProgressView().controlSize(.small) }
                }
                if let network {
                    Text(phaseText(network)).foregroundStyle(network.phase == .failed ? Color.red : Color.secondary)
                        .accessibilityIdentifier("remote.relay.state")
                    if let saved = network.saved, let revision, saved.revision != revision {
                        Text("设置已在另一处更改。请重新载入后再保存。 ")
                            .foregroundStyle(.orange)
                    }
                    if network.phase == .failed {
                        Button("重试已保存的设置") { retry() }.disabled(busy)
                    }
                }
                if let failure { Text(failure).foregroundStyle(.red).textSelection(.enabled).accessibilityIdentifier("remote.relay.failure") }
            }
            if let network, !network.facilities.isEmpty {
                Section("设施状态") {
                    ForEach(Array(network.facilities.enumerated()), id: \.offset) { _, facility in
                        LabeledContent {
                            Text(reachabilityText(facility.reachability))
                        } label: {
                            Text(facility.kind == .homeRelay ? "中继" : "地址查找")
                            Text(facility.url).font(.caption).foregroundStyle(.secondary).textSelection(.enabled)
                        }
                    }
                }
            }
        }
        .formStyle(.grouped)
        .task {
            visible = true
            load()
            let endpoint = endpoint
            while !Task.isCancelled {
                do { try await Task.sleep(for: .seconds(2)) } catch { return }
                guard !busy else { continue }
                let endpoint = endpoint
                let result = await Task.detached { Result { try endpoint().readNetworkState() } }.value
                guard visible else { return }
                if case .success(let value) = result { network = value }
            }
        }
        .onDisappear {
            visible = false
            for index in credentials.indices { credentials[index].token = "" }
        }
    }

    private func load() {
        let endpoint = endpoint
        perform { try endpoint().readNetworkState() } success: { state in
            network = state
            guard let saved = state.saved else {
                revision = nil
                failure = "本机网络设置尚未就绪，请稍后重新载入。"
                return
            }
            revision = saved.revision
            switch saved.relay {
            case .developmentPublic: mode = .publicService; relayURLs = ""
            case .custom(let urls): mode = .custom; relayURLs = urls.joined(separator: "\n")
            case .disabled: mode = .disabled; relayURLs = ""
            }
            credentials = saved.credentials.filter(\.configured).map { CredentialDraft(url: $0.url, saved: true) }
        }
    }

    private func save() {
        guard let revision else { return }
        let endpoint = endpoint
        let relay: RelayConfig
        switch mode {
        case .publicService: relay = .developmentPublic
        case .disabled: relay = .disabled
        case .custom:
            relay = .custom(urls: relayURLs.components(separatedBy: .newlines)
                .map { $0.trimmingCharacters(in: .whitespaces) }.filter { !$0.isEmpty })
        }
        let edits = credentials.map { draft in
            RelayCredentialEdit(url: draft.url.trimmingCharacters(in: .whitespacesAndNewlines),
                credential: draft.token.isEmpty && draft.saved ? .keep : .set(value: draft.token))
        }
        perform { try endpoint().applyRelaySettings(expectedRevision: revision, relay: relay, credentials: edits) } success: { state in
            network = state
            self.revision = state.saved?.revision
            credentials = (state.saved?.credentials ?? []).filter(\.configured).map { CredentialDraft(url: $0.url, saved: true) }
        }
    }

    private func retry() {
        let endpoint = endpoint
        perform { try endpoint().retryNetwork() } success: { network = $0 }
    }

    private func perform(_ work: @escaping @Sendable () throws -> NetworkState, success: @escaping (NetworkState) -> Void) {
        busy = true; failure = nil
        Task {
            let result = await Task.detached { Result { try work() } }.value
            busy = false
            guard visible else { return }
            switch result {
            case .success(let value): success(value)
            case .failure(let error): failure = ClientTelemetry.message(error)
            }
        }
    }

    private func phaseText(_ state: NetworkState) -> String {
        switch state.phase {
        case .unapplied: return "尚未应用"
        case .applying: return "正在应用已保存的设置"
        case .applied: return "已保存的设置已应用；连通性见设施状态"
        case .failed: return "设置已保存，应用失败" + (state.errorCode.map { "（\($0)）" } ?? "")
        }
    }

    private func reachabilityText(_ value: FacilityReachability) -> String {
        switch value {
        case .unknown: return "尚未确认"
        case .connecting: return "正在连接"
        case .reachable: return "可达"
        case .unavailable: return "无法连接"
        case .authenticationDenied: return "认证被拒绝"
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
