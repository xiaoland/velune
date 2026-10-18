import AppKit
import Combine
import SwiftUI
import VeluneBindings

enum RemoteAccessNetwork: String, CaseIterable, Identifiable, Sendable {
    case development, local
    var id: Self { self }
    var title: String {
        switch self {
        case .development: "公共开发设施"
        case .local: "仅直接连接"
        }
    }
}

struct RemoteAccessState: Sendable {
    var descriptor: BindingRemoteEndpointDescriptor?
    var devices: [BindingTrustedDevice] = []
}

/// Settings own only presentation; Transport keeps ingress alive after this view closes.
@MainActor final class RemoteAccessModel: ObservableObject {
    @Published var network: RemoteAccessNetwork = .development
    @Published private(set) var state = RemoteAccessState()
    @Published private(set) var invitation: BindingRemoteInvitation?
    @Published private(set) var copiedInvitation = false
    @Published private(set) var isWorking = false
    private var refreshPending = false
    private var invitationGeneration: UInt64 = 0
    private let transport: Transport?
    private let onFailure: (Error) -> Void
    private let queue = DispatchQueue(label: "local.velune.remote-access", qos: .userInitiated)
    private var subscriptions: Set<AnyCancellable> = []

    init(transport: Transport?, onFailure: @escaping (Error) -> Void) {
        self.transport = transport
        self.onFailure = onFailure
        NotificationCenter.default.publisher(for: NSApplication.didBecomeActiveNotification)
            .sink { [weak self] _ in
                guard let self, let transport = self.transport else { return }
                self.queue.async { transport.remoteNetworkChanged() }
            }.store(in: &subscriptions)
    }

    var isEnabled: Bool { state.descriptor != nil }
    var isAvailable: Bool { transport != nil }

    func enable() {
        let network = network
        run({ try $0.enableRemoteAccess(network: network) }) { self.state = $0 }
    }

    func stop() {
        run({ transport in try transport.stopRemoteAccess(); return RemoteAccessState() }) { state in
            self.state = state
            self.clearInvitation()
        }
    }

    func generateInvitation() {
        clearInvitation()
        let generation = invitationGeneration
        run({ try $0.createRemoteInvitation() }) { invitation in
            if self.invitationGeneration == generation { self.invitation = invitation }
        }
    }

    func clearInvitation() {
        invitationGeneration += 1
        invitation = nil
        copiedInvitation = false
    }

    func copyInvitation() {
        guard let invitation, Date().timeIntervalSince1970 < Double(invitation.expiresAtUnixSeconds) else {
            clearInvitation()
            return
        }
        NSPasteboard.general.clearContents()
        if NSPasteboard.general.setString(invitation.encoded, forType: .string) { copiedInvitation = true }
        else { onFailure(TransportError.rejected("无法拷贝邀请，请重试。")) }
    }

    func revoke(_ device: BindingTrustedDevice) {
        run({ try $0.revokeRemoteDevice(device.id) }) { self.state = $0 }
    }

    func refresh() {
        if let invitation, Date().timeIntervalSince1970 >= Double(invitation.expiresAtUnixSeconds) { clearInvitation() }
        guard isEnabled, !isWorking, !refreshPending, let transport else { return }
        refreshPending = true
        queue.async { [weak self] in
            let result = Result { try transport.remoteAccessState() }
            DispatchQueue.main.async {
                guard let self else { return }
                self.refreshPending = false
                switch result {
                case .success(let state): self.state = state
                case .failure(let error): self.onFailure(error)
                }
            }
        }
    }

    private func run<T: Sendable>(_ work: @escaping @Sendable (Transport) throws -> T, done: @escaping (T) -> Void) {
        guard !isWorking, let transport else { return }
        isWorking = true
        queue.async { [weak self] in
            let result = Result { try work(transport) }
            DispatchQueue.main.async {
                guard let self else { return }
                self.isWorking = false
                switch result {
                case .success(let value): done(value)
                case .failure(let error): self.onFailure(error)
                }
            }
        }
    }
}

struct RemoteAccessSettingsView: View {
    @ObservedObject var store: AppStore
    @ObservedObject var model: RemoteAccessModel
    @State private var revokingDevice: BindingTrustedDevice?
    private let refresh = Timer.publish(every: 2, on: .main, in: .common).autoconnect()

    var body: some View {
        Form {
            Section {
                // LabeledContent can merge the action into its status accessibility text.
                HStack {
                    Text("状态")
                    Spacer()
                    if model.isWorking { ProgressView().controlSize(.small) }
                    Text(model.isEnabled ? "已启用" : "未启用")
                    if model.isEnabled {
                        Button("停止接入") { model.stop() }
                            .accessibilityLabel("停止远程接入")
                    } else {
                        Button("启用远程接入") { model.enable() }
                            .accessibilityLabel("启用远程接入")
                    }
                }.accessibilityElement(children: .contain)
                if let descriptor = model.state.descriptor {
                    LabeledContent("本设备", value: descriptor.name)
                    LabeledContent("设备 ID") {
                        Text(descriptor.endpointId).font(.caption.monospaced()).textSelection(.enabled).lineLimit(1).truncationMode(.middle)
                    }
                }
                Picker("连接设施", selection: $model.network) {
                    ForEach(RemoteAccessNetwork.allCases) { Text($0.title).tag($0) }
                }.disabled(model.isEnabled)
                Text(model.network == .development
                     ? "使用 iroh 公共开发设施完成发现和必要中继，连接后自动尝试直连；这些设施不提供 Velune 正式服务承诺。"
                     : "不使用公共发现或中继。仅尝试邀请中携带的直接地址，适合同一局域网；网络变化后可能需要重新邀请。")
                    .font(.caption).foregroundStyle(.secondary)
                Text("只允许已配对的本人设备使用本机 Velune。停止接入或控制端离线不会停止任务；退出本机应用或机器睡眠会影响执行。")
                    .font(.caption).foregroundStyle(.secondary)
            } header: {
                HStack { Text("本机远程接入"); Spacer(); if !store.problems.isEmpty { ProblemsButton(store: store) } }
            }
            Section("配对设备") {
                HStack {
                    Button("生成一次性邀请") { model.generateInvitation() }.disabled(!model.isEnabled)
                    if model.invitation != nil { Button(model.copiedInvitation ? "再次拷贝邀请" : "拷贝邀请") { model.copyInvitation() } }
                }
                if let invitation = model.invitation {
                    Text("邀请有效至 \(Date(timeIntervalSince1970: Double(invitation.expiresAtUnixSeconds)).formatted(date: .omitted, time: .standard))，仅可成功配对一次。")
                        .font(.caption).foregroundStyle(.secondary)
                    Text(model.copiedInvitation ? "已拷贝。请将邀请粘贴到你自己的另一台设备；持有邀请者可获得完整业务访问权限。" : "邀请只在本页暂存，点击“拷贝邀请”后由你传给另一台设备。")
                        .font(.caption).foregroundStyle(.secondary)
                }
            }
            Section("已配对设备") {
                if model.state.devices.isEmpty {
                    Text(model.isEnabled ? "还没有已配对设备。" : "启用接入后可查看和撤销已配对设备。")
                        .foregroundStyle(.secondary)
                } else {
                    ForEach(model.state.devices, id: \.id) { device in
                        LabeledContent {
                            Button("撤销…", role: .destructive) { revokingDevice = device }
                        } label: {
                            Text(device.name)
                            Text(device.id).font(.caption.monospaced()).foregroundStyle(.secondary).lineLimit(1).truncationMode(.middle)
                        }
                    }
                }
            }
        }
        .formStyle(.grouped)
        .disabled(model.isWorking || !model.isAvailable || store.isShuttingDown)
        .onReceive(refresh) { _ in model.refresh() }
        .onAppear { model.refresh() }
        .onDisappear { model.clearInvitation() }
        .confirmationDialog("撤销“\(revokingDevice?.name ?? "")”的访问？", isPresented: Binding(get: { revokingDevice != nil }, set: { if !$0 { revokingDevice = nil } }), titleVisibility: .visible) {
            if let device = revokingDevice {
                Button("撤销访问", role: .destructive) { model.revoke(device); revokingDevice = nil }
            }
            Button("取消", role: .cancel) { revokingDevice = nil }
        } message: {
            Text("该设备的现有连接将断开，之后需要重新邀请配对。本机已接纳的任务继续执行。")
        }
    }
}
