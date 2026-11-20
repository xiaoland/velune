import SwiftUI
import AppKit
import CoreImage.CIFilterBuiltins
import VeluneBindings

/// The remote service of this Mac's core, shared with the transports that
/// reach paired devices from their own queues.
final class RemoteEndpointBox: @unchecked Sendable {
    private let condition = NSCondition()
    private var endpoint: VeluneRemoteEndpoint?
    var value: VeluneRemoteEndpoint? {
        get { condition.withLock { endpoint } }
        set { condition.withLock { endpoint = newValue; condition.broadcast() } }
    }

    /// The service once it is open; windows restored at launch ask before it is.
    func opened(within seconds: TimeInterval) -> VeluneRemoteEndpoint? {
        let deadline = Date().addingTimeInterval(seconds)
        return condition.withLock {
            while endpoint == nil, condition.wait(until: deadline) {}
            return endpoint
        }
    }
}

/// Paired devices: this Mac lets them control it, and controls them. A
/// device's windows talk to its core through the same application methods
/// as this Mac's, so they show the same interface.
@MainActor final class DeviceHub: ObservableObject {
    private(set) static var shared: DeviceHub?
    @Published private(set) var state: RemoteServiceSnapshot?
    @Published private(set) var failure: String?
    /// The invitation shown for adding a device, until it is used or expires.
    @Published private(set) var invitation: BindingRemoteInvitation?
    private let local: Transport
    private let endpoint = RemoteEndpointBox()
    lazy var hapiSharing = HapiSharing(endpoint: endpoint, stateDirectory: local.stateDirectory)
    let networkSettings = RemoteNetworkSettingsModel()
    private var stores: [String: DeviceStore] = [:]
    /// This Mac's own store, for window-facing actions that need it.
    weak var localStore: DeviceStore?
    /// Opens a workspace window when none can show a device (set by the app).
    var openWorkspaceWindow: ((WindowSeed) -> Void)?
    private var prompted: Set<String> = []
    private let queue = DispatchQueue(label: "local.velune.remote", qos: .userInitiated)

    init(local: Transport) {
        self.local = local
        Self.shared = self
    }

    static var deviceName: String { Host.current().localizedName ?? "Mac" }

    /// Opens the service once: incoming control must be reachable whenever the app runs.
    func start() {
        let local = local, endpoint = endpoint
        queue.async { [weak self] in
            do {
                let directory = local.stateDirectory.appendingPathComponent("remote", isDirectory: true)
                try FileManager.default.createDirectory(at: directory, withIntermediateDirectories: true, attributes: [.posixPermissions: 0o700])
                let options = BindingRemoteOptions(
                    identityFile: directory.appendingPathComponent("device-identity").path,
                    stateFile: directory.appendingPathComponent("remote-state.json").path,
                    deviceName: Self.deviceNameOffMain(),
                    relay: .developmentPublic, discovery: .developmentPublic, pathPolicy: .automatic,
                    bindAddresses: [], requestTimeoutMs: 30_000)
                let opened = try local.openRemoteEndpoint(options)
                endpoint.value = opened
                try opened.startLocalControl(directory: directory.path)
                let first = try opened.readState()
                DispatchQueue.main.async { self?.apply(first) }
                Self.observe(opened.observeState()) { snapshot in DispatchQueue.main.async { self?.apply(snapshot) } }
            } catch {
                DispatchQueue.main.async { self?.failure = "远程控制服务无法启动：\(error.localizedDescription)" }
            }
        }
    }

    nonisolated private static func deviceNameOffMain() -> String { Host.current().localizedName ?? "Mac" }

    nonisolated private static func observe(_ observer: VeluneRemoteStateObserver, _ deliver: @escaping @Sendable (RemoteServiceSnapshot) -> Void) {
        let thread = Thread {
            while true {
                do {
                    if let snapshot = try observer.next(timeoutMs: 5_000) { deliver(snapshot) }
                } catch BindingError.Closed {
                    return
                } catch {
                    Thread.sleep(forTimeInterval: 1)
                }
            }
        }
        thread.name = "velune-remote-state"
        thread.start()
    }

    private func apply(_ snapshot: RemoteServiceSnapshot) {
        state = snapshot
        let knownPeers = Set(snapshot.contacts.map(\.peerId))
        stores = stores.filter { knownPeers.contains($0.key) }
        prepareStores(snapshot)
        // The invitation has done its job once a device answers it, or is no longer usable.
        if let invitation, Date().timeIntervalSince1970 > Double(invitation.expiresAtUnixSeconds) || snapshot.incomingPairings.contains(where: { $0.phase == .awaitingApproval }) {
            self.invitation = nil
        }
        for pairing in snapshot.incomingPairings where pairing.phase == .awaitingApproval && !prompted.contains(pairing.attemptId) {
            prompted.insert(pairing.attemptId)
            promptApproval(pairing)
        }
    }

    /// Every device this Mac is authorized to control keeps a live store,
    /// so the aggregated list reads it without waiting for a window. Called
    /// on the first snapshot and whenever pairing changes the contact set.
    private func prepareStores(_ snapshot: RemoteServiceSnapshot) {
        for contact in snapshot.contacts where contact.controlAuthorized {
            guard case .confirmed = contact.confirmation else { continue }
            let store = store(for: contact.peerId)
            store.start()
            if let status = snapshot.controllers.first(where: { $0.peerId == contact.peerId }), status.phase == .connected {
                store.connected(epoch: status.connectionEpoch)
            }
        }
    }

    /// The user asked to try a device again: forget the cached open failure
    /// and read its list, which opens the connection again.
    func retryConnection(_ peerID: String) {
        stores[peerID]?.retryConnection()
    }

    /// The paired device a store belongs to, if it is one of the hub's.
    func peerID(of store: DeviceStore) -> String? {
        stores.first { $0.value === store }?.key
    }

    /// Present this Mac's aggregated window showing `peerID`: an existing
    /// workspace window comes forward and filters to the device; none opens.
    /// Devices no longer get a window of their own.
    func openDevice(_ peerID: String) {
        if let window = ConversationWindowModel.openWorkspaceWindow() {
            window.showDevice(peerID)
            window.window?.deminiaturize(nil)
            window.window?.makeKeyAndOrderFront(nil)
            return
        }
        var seed = WindowSeed()
        seed.device = peerID
        openWorkspaceWindow?(seed)
    }

    /// The status line a device's row shows, shared by the settings list and
    /// the aggregated conversation list.
    func statusText(for peerID: String) -> String {
        if let outgoing = state?.outgoingPairings.first(where: { $0.peerId == peerID }), outgoing.phase != .granted {
            switch outgoing.phase {
            case .awaitingReady, .awaitingApproval:
                let scope = outgoing.permission == .telemetryProfileRead ? "遥测配置（含认证）：" : ""
                return scope + "等待对方允许，核对码 \(Self.grouped(outgoing.verificationCode ?? ""))"
            case .rejected: return "对方已拒绝"
            case .expired: return "邀请已过期"
            case .disconnected: return "配对时连接中断"
            case .granted: break
            }
        }
        guard let controller = controller(peerID) else { return "未连接" }
        switch controller.phase {
        case .connected:
            guard let first = controller.viaDeviceIds.first else { return "已连接" }
            let name = self.name(of: first)?.trimmingCharacters(in: .whitespacesAndNewlines) ?? ""
            let device = name.isEmpty ? "设备" : name
            let via = controller.viaDeviceIds.count == 1 ? device : "\(device)等\(controller.viaDeviceIds.count)台设备"
            return "已连接，经\(via)中转"
        case .connecting, .discovering: return "正在连接"
        case .recovering: return "正在重新连接"
        case .suspended, .disconnected: return "未连接"
        case .blocked: return "无法连接" + (controller.errorCode.map { "（\($0)）" } ?? "")
        }
    }

    /// Approval happens on the controlled device, after the user compares the codes both devices show.
    private func promptApproval(_ pairing: PairingStatus) {
        NSApp.requestUserAttention(.criticalRequest)
        let alert = NSAlert()
        let sharing = pairing.permission == .telemetryProfileRead
        alert.messageText = sharing ? "“\(pairing.name)”请求读取本机遥测配置（含认证）" : "“\(pairing.name)”请求加入可信设备网"
        let scope = sharing ? "允许后，它可以复制接收地址、认证与采样率。" : "允许后，两台设备将加入同一可信设备网。网内的 Mac、Windows 可相互控制，使用目标电脑的会话、运行时与配置；运行中的可信设备可中转。\n\n手机不执行 Agent，手机端应用不保证后台持续运行；遥测配置共享需单独允许。"
        alert.informativeText = "请确认对方设备上显示的核对码与下面一致：\n\n\(Self.grouped(pairing.verificationCode ?? ""))\n\n\(scope)"
        alert.addButton(withTitle: "允许")
        alert.addButton(withTitle: "拒绝")
        // Granting control takes a click: a stray Return must not allow a device.
        alert.buttons[0].keyEquivalent = ""
        alert.buttons[1].keyEquivalent = "\u{1b}"
        // CLI approval changes the same service while this nested modal loop is running.
        let timer = Timer(timeInterval: 0.25, repeats: true) { [endpoint] _ in
            guard let snapshot = try? endpoint.value?.readState() else { return }
            if !snapshot.incomingPairings.contains(where: { $0.attemptId == pairing.attemptId && $0.phase == .awaitingApproval }) {
                NSApp.abortModal()
            }
        }
        RunLoop.main.add(timer, forMode: .modalPanel)
        defer { timer.invalidate(); alert.window.orderOut(nil) }
        let response = alert.runModal()
        if response == .alertFirstButtonReturn {
            perform("允许") { _ = try $0.approvePairing(attemptId: pairing.attemptId, expectedPeerId: pairing.peerId) }
        } else if response == .alertSecondButtonReturn {
            perform("拒绝") { _ = try $0.rejectPairing(attemptId: pairing.attemptId, expectedPeerId: pairing.peerId) }
        }
    }

    static func grouped(_ code: String) -> String {
        code.count == 8 ? "\(code.prefix(4)) \(code.suffix(4))" : code
    }
    nonisolated func telemetryEndpoint() throws -> VeluneRemoteEndpoint {
        guard let value = endpoint.value else { throw TransportError.unavailable }; return value
    }

    private func perform(_ action: String, _ body: @escaping (VeluneRemoteEndpoint) throws -> Void) {
        let endpoint = endpoint
        queue.async { [weak self] in
            do {
                guard let opened = endpoint.value else { throw TransportError.unavailable }
                try body(opened)
                let snapshot = try opened.readState()
                DispatchQueue.main.async { self?.failure = nil; self?.apply(snapshot) }
            } catch {
                DispatchQueue.main.async { self?.failure = "\(action)失败：\(error.localizedDescription)" }
            }
        }
    }

    func setIngress(_ enabled: Bool) { perform(enabled ? "开启远程控制" : "关闭远程控制") { try $0.setControlIngress(enabled: enabled) } }

    func createInvitation() {
        let endpoint = endpoint
        queue.async { [weak self] in
            do {
                guard let opened = endpoint.value else { throw TransportError.unavailable }
                if !(try opened.readState().controlIngress) { try opened.setControlIngress(enabled: true) }
                let invitation = try opened.createInvitation(lifetimeSeconds: 300)
                let snapshot = try opened.readState()
                DispatchQueue.main.async { self?.invitation = invitation; self?.apply(snapshot) }
            } catch {
                DispatchQueue.main.async { self?.failure = "生成邀请失败：\(error.localizedDescription)" }
            }
        }
    }

    func cancelInvitation() { invitation = nil }

    /// Pairs this Mac as a controller of the device that issued the invitation.
    func beginPairing(_ encoded: String) {
        let text = encoded.trimmingCharacters(in: .whitespacesAndNewlines)
        perform("连接设备") { _ = try $0.beginPairing(encoded: text) }
    }

    func cancelPairing(_ peerID: String) { perform("取消配对") { _ = try $0.cancelPairing(peerId: peerID) } }
    func revoke(_ peerID: String) { perform("在本机撤销信任") { try $0.revokeControl(peerId: peerID) } }
    func forget(_ peerID: String) {
        perform("移除设备") { try $0.forgetDevice(peerId: peerID) }
    }

    /// The application store of a paired device, shared by its windows.
    func store(for peerID: String) -> DeviceStore {
        if let store = stores[peerID] { return store }
        let endpoint = endpoint
        let view = "mac-\(UUID().uuidString)"
        let transport = Transport(stateDirectory: local.stateDirectory, telemetry: local.telemetryResult) {
            guard let opened = endpoint.opened(within: 30) else { throw TransportError.unavailable }
            // The core confirms a pairing this Mac began once the device answers.
            let deadline = Date().addingTimeInterval(30)
            while Date() < deadline, case .pending = try opened.readState().contacts.first(where: { $0.peerId == peerID })?.confirmation {
                Thread.sleep(forTimeInterval: 0.5)
            }
            let application = try opened.connect(peerId: peerID, viewId: view)
            try? opened.setControllerActivity(peerId: peerID, selected: true, foreground: true)
            return application
        }
        let store = DeviceStore(transport: transport, peerID: peerID)
        stores[peerID] = store
        return store
    }

    func name(of peerID: String) -> String? { state?.contacts.first { $0.peerId == peerID }?.name }

    func controller(_ peerID: String) -> ControllerStatus? { state?.controllers.first { $0.peerId == peerID } }
}

/// Settings for the devices that control this Mac and the ones it controls.
struct DeviceSettingsView: View {
    @ObservedObject var hub: DeviceHub
    @Environment(\.openWindow) private var openWindow
    @State private var offer = ""
    @State private var showsIPadInstallation = false
    @State private var showsNetworkSettings = false
    @StateObject private var iPadInstallation = IPadInstallationModel()
    var body: some View {
        Form {
            Section("这台 Mac") {
                Toggle("允许其他设备控制这台 Mac", isOn: Binding(get: { hub.state?.controlIngress ?? false }, set: { hub.setIngress($0) }))
                    .disabled(hub.state == nil)
                LabeledContent("设备名称", value: hub.state?.descriptor?.name ?? DeviceHub.deviceName)
                if let invitation = hub.invitation {
                    InvitationView(invitation: invitation) { hub.cancelInvitation() }
                } else {
                    Button("添加控制设备…") { hub.createInvitation() }.disabled(hub.state == nil)
                    Text("在另一台设备上扫描二维码或粘贴邀请，两台设备显示的核对码一致时，在这台 Mac 上允许。").uiFont(.caption).foregroundStyle(.secondary)
                }
                ForEach(hub.state?.members ?? [], id: \.peerId) { member in
                    LabeledContent {
                        Button("在本机撤销信任") { hub.revoke(member.peerId) }
                    } label: {
                        Text(member.name)
                        Text("本机已信任的设备网成员").foregroundStyle(.secondary)
                    }
                }
                Text("撤销信任会阻止此设备控制本机及在本机传递信任；其他设备上的关系保持不变。")
                    .uiFont(.caption).foregroundStyle(.secondary)
            }
            HapiSharingSettings(sharing: hub.hapiSharing)
            Section("本机连接") {
                Button("连接设置…") { showsNetworkSettings = true }.disabled(hub.state == nil)
            }
            Section {
                Button("安装到 iPad…") { showsIPadInstallation = true }
            }
            Section("其他设备") {
                ForEach(hub.state?.contacts ?? [], id: \.peerId) { contact in
                    DeviceRow(contact: contact, member: hub.state?.members.first(where: { $0.peerId == contact.peerId }), status: hub.statusText(for: contact.peerId)) {
                        hub.openDevice(contact.peerId)
                    } forget: { hub.forget(contact.peerId) } cancel: { hub.cancelPairing(contact.peerId) }
                }
                HStack {
                    TextField("邀请", text: $offer, prompt: Text("粘贴另一台设备的邀请"))
                    Button("连接") { hub.beginPairing(offer); offer = "" }.disabled(offer.trimmingCharacters(in: .whitespacesAndNewlines).isEmpty || hub.state == nil)
                }
                Text("粘贴来源设备的控制或遥测分享邀请，核对两端数字，并在来源设备上批准对应权限。").uiFont(.caption).foregroundStyle(.secondary)
                Text("控制配对会让设备加入同一可信设备网。网内的 Mac、Windows 可相互控制，运行中的可信设备可中转。手机不执行 Agent，手机端应用不保证后台持续运行。").uiFont(.caption).foregroundStyle(.secondary)
                Text("如果对方使用需要认证的中继，请先在本机“连接设置”添加对应访问令牌；邀请不包含中继令牌。").uiFont(.caption).foregroundStyle(.secondary)
                Text("在本机移除后，此设备不再由普通发现重新加入；其他设备上的关系保持不变。")
                    .uiFont(.caption).foregroundStyle(.secondary)
            }
            if let failure = hub.failure {
                Section { Text(failure).foregroundStyle(.red) }
            }
        }.formStyle(.grouped)
        .sheet(isPresented: $showsIPadInstallation) { IPadInstallationPanel(model: iPadInstallation) }
        .sheet(isPresented: $showsNetworkSettings) {
            RemoteNetworkSettingsView(model: hub.networkSettings, endpoint: { [hub] in try hub.telemetryEndpoint() }, onClose: { showsNetworkSettings = false })
                .frame(width: 620, height: 620)
        }
    }
}

private struct DeviceRow: View {
    let contact: DeviceContact
    let member: DeviceMember?
    let status: String
    let open: () -> Void
    let forget: () -> Void
    let cancel: () -> Void
    var body: some View {
        LabeledContent {
            if case .confirmed = contact.confirmation {
                if contact.controlAuthorized { Button("打开") { open() } }
                else if member != nil { Text(member?.canExecute == false ? "仅控制与中转" : "等待设备信息").foregroundStyle(.secondary) }
                else if contact.pairingPermission == .telemetryProfileRead { SettingsTabLink(tab: .telemetry, title: "遥测设置") }
                else { Text("信任已撤销").foregroundStyle(.secondary) }
            } else {
                Button("取消") { cancel() }
            }
        } label: {
            Text(contact.name)
            if let member, !contact.controlAuthorized {
                Text(member.canExecute == false ? "可信网络成员，不执行 Agent" : "可信网络成员，正在同步设备能力").foregroundStyle(.secondary)
            } else {
                if !contact.controlAuthorized, contact.pairingPermission == .telemetryProfileRead { Text("遥测来源").foregroundStyle(.secondary) }
                Text(status).foregroundStyle(.secondary)
            }
        }
        .contextMenu { Button("在本机移除此设备", role: .destructive) { forget() } }
    }
}

private struct InvitationView: View {
    let invitation: BindingRemoteInvitation
    let done: () -> Void
    var body: some View {
        VStack(alignment: .leading, spacing: 8) {
            HStack(alignment: .top, spacing: 16) {
                if let image = pairingQRCode(invitation.encoded) {
                    Image(nsImage: image).interpolation(.none).resizable().frame(width: 180, height: 180)
                }
                VStack(alignment: .leading, spacing: 8) {
                    Text("在另一台设备上扫描二维码，或复制邀请粘贴到那台设备。")
                    Text("到期：\(Date(timeIntervalSince1970: Double(invitation.expiresAtUnixSeconds)).formatted(date: .omitted, time: .shortened))").foregroundStyle(.secondary)
                    HStack {
                        Button("复制邀请") {
                            NSPasteboard.general.clearContents()
                            NSPasteboard.general.setString(invitation.encoded, forType: .string)
                        }
                        Button("完成") { done() }
                    }
                }
            }
        }
    }

}

func pairingQRCode(_ text: String) -> NSImage? {
    let filter = CIFilter.qrCodeGenerator()
    filter.message = Data(text.utf8)
    filter.correctionLevel = "M"
    guard let output = filter.outputImage else { return nil }
    let representation = NSCIImageRep(ciImage: output)
    let image = NSImage(size: representation.size)
    image.addRepresentation(representation)
    return image
}
