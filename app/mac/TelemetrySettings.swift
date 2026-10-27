import SwiftUI
#if canImport(VeluneBindings)
import VeluneBindings
#endif

struct TelemetrySettingsView: View {
    let coordinator: () throws -> ClientTelemetry
    var remote: (() throws -> VeluneRemoteEndpoint)? = nil
    @State private var revision: UInt64 = 0
    @State private var preference: BindingTelemetryPreference = .unset
    @State private var endpoint = ""
    @State private var ratio = 1.0
    @State private var headers: [Header] = []
    @State private var status = ""
    @State private var lastHealth = ""
    @State private var failure: String?
    @State private var busy = false
    @State private var loaded = false
    @State private var visible = false
    @State private var source = ""
    @State private var devices: [BindingDeviceContact] = []
    @State private var peers: [SharingPeer] = []
    @State private var incoming: [BindingPairingStatus] = []
    @State private var invitation: BindingRemoteInvitation?
    private struct SharingPeer: Identifiable, Equatable { var id: String; var name: String; var allowed: Bool }
    private struct Header: Identifiable { let id = UUID(); var name: String; var value: String }
    var body: some View {
        Form {
            Section("本机遥测") {
                Picker("启用", selection: $preference) {
                    Text("未设置").tag(BindingTelemetryPreference.unset)
                    Text("关闭").tag(BindingTelemetryPreference.off)
                    Text("开启").tag(BindingTelemetryPreference.on)
                }
                TextField("OTLP/gRPC 地址", text: $endpoint).textContentType(.URL)
                HStack { Text("新链路采样率"); Slider(value: $ratio, in: 0...1); Text(ratio, format: .percent.precision(.fractionLength(0))).monospacedDigit() }
                ForEach($headers) { $header in
                    HStack {
                        TextField("Metadata 名称", text: $header.name)
                        SecureField("值", text: $header.value)
                        Button { headers.removeAll { $0.id == header.id } } label: { Image(systemName: "minus.circle") }.accessibilityLabel("移除认证字段")
                    }
                }
                if headers.count < 16 { Button("添加认证字段") { headers.append(Header(name: "authorization", value: "")) } }
                HStack {
                    Button("保存") { save() }.disabled(!loaded)
                    Button("发送测试事件") { test() }.disabled(endpoint.isEmpty)
                    if busy { ProgressView().controlSize(.small) }
                }
                if !status.isEmpty { Text(status).foregroundStyle(.secondary).textSelection(.enabled) }
                if let failure { Text(failure).foregroundStyle(.red).textSelection(.enabled) }
            }.disabled(busy)
            if remote != nil {
                Section("从设备复制") {
                    Picker("来源", selection: $source) {
                        Text("选择设备").tag("")
                        ForEach(devices, id: \.peerId) { Text($0.name).tag($0.peerId) }
                    }
                    Button("复制遥测配置（含认证）") { copyProfile() }.disabled(source.isEmpty || busy || !loaded)
                }
                Section("允许读取本机配置（含认证）") {
                    ForEach(peers) { peer in
                        HStack {
                            Text(peer.name); Spacer()
                            Button(peer.allowed ? "撤销" : "允许") {
                                remoteAction { endpoint in
                                    if peer.allowed { try endpoint.revokeTelemetryProfile(peerId: peer.id) }
                                    else { try endpoint.allowTelemetryProfile(peerId: peer.id) }
                                }
                            }
                        }
                    }
                    Button("生成遥测分享邀请") {
                        guard let remote else { return }
                        perform { try remote().createTelemetryInvitation(lifetimeSeconds: 300) } success: { invitation = $0 }
                    }
                    if let invitation, invitation.expiresAtUnixSeconds > UInt64(Date().timeIntervalSince1970) {
                        Text(invitation.encoded).font(.caption.monospaced()).textSelection(.enabled)
                        ShareLink("分享邀请", item: invitation.encoded)
                    }
                    ForEach(incoming, id: \.attemptId) { pairing in
                        VStack(alignment: .leading) {
                            Text("\(pairing.name) · 遥测配置（含认证）")
                            Text(pairing.verificationCode ?? "").font(.title.monospaced()).textSelection(.enabled)
                            HStack {
                                Button("核对一致，允许") { remoteAction { _ = try $0.approvePairing(attemptId: pairing.attemptId, expectedPeerId: pairing.peerId) } }
                                Button("拒绝") { remoteAction { _ = try $0.rejectPairing(attemptId: pairing.attemptId, expectedPeerId: pairing.peerId) } }
                            }
                        }
                    }
                }.disabled(busy)
            }
        }
        .formStyle(.grouped)
        .task {
            visible = true; load()
            while !Task.isCancelled {
                await refreshSharing()
                let getCoordinator = coordinator
                if let snapshot = try? await Task.detached(operation: { try getCoordinator().binding.snapshot() }).value, visible {
                    let next = statusText(snapshot)
                    if next != lastHealth { lastHealth = next; status = next }
                }
                do { try await Task.sleep(nanoseconds: 1_000_000_000) } catch { return }
            }
        }
        .onDisappear { visible = false; headers = []; loaded = false; invitation = nil }
    }
    private func load() {
        perform {
            let owner = try coordinator(); let snapshot = try owner.binding.snapshot()
            return (snapshot, try owner.binding.profileForEditing())
        } success: { snapshot, profile in
            revision = snapshot.revision; preference = snapshot.preference
            endpoint = profile?.endpoint ?? snapshot.endpoint ?? ""; ratio = profile?.sampleRatio ?? snapshot.sampleRatio
            headers = profile?.headers.sorted(by: { $0.key < $1.key }).map { Header(name: $0.key, value: $0.value) } ?? []
            status = statusText(snapshot); loaded = snapshot.configurationAvailable
        }
    }
    private func profile() throws -> BindingTelemetryProfile? {
        guard !endpoint.trimmingCharacters(in: .whitespacesAndNewlines).isEmpty else { return nil }
        let names = headers.map(\.name)
        guard Set(names).count == names.count else { throw BindingError.Invalid(detail: "认证字段名称不能重复") }
        return BindingTelemetryProfile(endpoint: endpoint.trimmingCharacters(in: .whitespacesAndNewlines), headers: Dictionary(uniqueKeysWithValues: headers.map { ($0.name, $0.value) }), sampleRatio: ratio)
    }
    private func save() {
        do {
            let profile = try profile(), revision = revision, preference = preference
            perform { let owner = try coordinator(); try owner.save(revision: revision, preference: preference, profile: profile); return try owner.binding.snapshot() } success: { snapshot in
                self.revision = snapshot.revision; status = statusText(snapshot)
            }
        } catch { failure = ClientTelemetry.message(error) }
    }
    private func copyProfile() {
        guard let remote else { return }
        let peer = source, expected = revision
        perform {
            guard let profile = try remote().readTelemetryProfile(peerId: peer) else { throw BindingError.Invalid(detail: "来源设备尚未保存遥测配置") }
            let owner = try coordinator(), state = try owner.binding.snapshot()
            try owner.save(revision: expected, preference: state.preference, profile: profile)
            return (try owner.binding.snapshot(), profile)
        } success: { state, profile in
            revision = state.revision; preference = state.preference; endpoint = profile.endpoint; ratio = profile.sampleRatio
            headers = profile.headers.sorted(by: { $0.key < $1.key }).map { Header(name: $0.key, value: $0.value) }
            let host = URL(string: profile.endpoint)?.host ?? ""
            status = ["localhost", "127.0.0.1", "::1", "[::1]"].contains(host) ? "已复制；来源地址为回环地址，请调整并测试" : statusText(state)
        }
    }
    private func remoteAction(_ work: @escaping (VeluneRemoteEndpoint) throws -> Void) {
        guard let remote else { return }; perform { try work(remote()) } success: { _ in }
    }
    private func refreshSharing() async {
        guard let remote else { return }
        let result = await Task.detached { Result { try remote().readState() } }.value
        guard visible, case .success(let state) = result else { return }
        let nextDevices = state.contacts.filter { if case .confirmed = $0.confirmation { return true }; return false }
        if devices != nextDevices { devices = nextDevices }
        var names = Dictionary(uniqueKeysWithValues: devices.map { ($0.peerId, $0.name) })
        for grant in state.incomingGrants + state.telemetryProfileGrants { names[grant.peerId] = grant.name }
        let granted = Set(state.telemetryProfileGrants.map(\.peerId))
        let nextPeers = names.map { SharingPeer(id: $0.key, name: $0.value, allowed: granted.contains($0.key)) }.sorted { $0.name < $1.name }
        if peers != nextPeers { peers = nextPeers }
        let nextIncoming = state.incomingPairings.filter { $0.permission == .telemetryProfileRead && $0.phase == .awaitingApproval }
        if incoming != nextIncoming { incoming = nextIncoming }
        if !incoming.isEmpty && invitation != nil { invitation = nil }
    }
    private func test() {
        do {
            guard let profile = try profile() else { return }
            perform { try coordinator().binding.sendTest(profile: profile) } success: { result in
                status = "Traces：\(testText(result.traces))；Logs：\(testText(result.logs))；Metrics：\(testText(result.metrics))"
            }
        } catch { failure = ClientTelemetry.message(error) }
    }
    private func perform<T>(_ work: @escaping () throws -> T, success: @escaping (T) -> Void) {
        busy = true; failure = nil
        Task {
            let result = await Task.detached { Result { try work() } }.value
            busy = false
            guard visible else { return }
            switch result { case .success(let value): success(value); case .failure(let error): failure = ClientTelemetry.message(error) }
        }
    }
    private func statusText(_ state: BindingTelemetrySnapshot) -> String {
        if state.status == "development_configuration_unavailable" { return "开发启动配置不可用，遥测未接入" }
        if !state.configurationAvailable { return "配置无法读取，遥测未启用；原文件已保留" }
        let source = state.developmentOverride ? "开发启动配置 · " : ""
        if !state.applied { return source + "已保存，尚未应用" }
        if state.rustActiveRevision == nil { return "已关闭" }
        return source + "已应用 · 修订 \(state.revision) · Rust：\(healthText(state.rustExport)) · Apple：\(healthText(state.nativeExport))"
    }
    private func healthText(_ health: BindingTelemetryExportHealth) -> String {
        switch health.lastTransportResult { case "rpc_completed": return "最近 RPC 完成"; case nil: return "尚无回执"; default: return "最近导出失败" }
    }
    private func testText(_ value: String) -> String {
        switch value {
        case "accepted": return "接收端已接受"
        case "partially_rejected": return "部分拒绝"
        case "unauthenticated", "permission_denied": return "认证被拒绝"
        case "timeout": return "超时"
        case "unavailable": return "无法连接"
        default: return "被拒绝"
        }
    }
}
