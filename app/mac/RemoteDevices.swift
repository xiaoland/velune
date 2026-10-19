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
    @Published private(set) var state: BindingRemoteServiceSnapshot?
    @Published private(set) var failure: String?
    /// The invitation shown for adding a device, until it is used or expires.
    @Published private(set) var invitation: BindingRemoteInvitation?
    private let local: Transport
    private let endpoint = RemoteEndpointBox()
    private var stores: [String: AppStore] = [:]
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
                let first = try opened.readState()
                DispatchQueue.main.async { self?.apply(first) }
                Self.observe(opened.observeState()) { snapshot in DispatchQueue.main.async { self?.apply(snapshot) } }
            } catch {
                DispatchQueue.main.async { self?.failure = "远程控制服务无法启动：\(error.localizedDescription)" }
            }
        }
    }

    nonisolated private static func deviceNameOffMain() -> String { Host.current().localizedName ?? "Mac" }

    nonisolated private static func observe(_ observer: VeluneRemoteStateObserver, _ deliver: @escaping @Sendable (BindingRemoteServiceSnapshot) -> Void) {
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

    private func apply(_ snapshot: BindingRemoteServiceSnapshot) {
        state = snapshot
        // The invitation has done its job once a device answers it, or is no longer usable.
        if let invitation, Date().timeIntervalSince1970 > Double(invitation.expiresAtUnixSeconds) || snapshot.incomingPairings.contains(where: { $0.phase == .awaitingApproval }) {
            self.invitation = nil
        }
        for pairing in snapshot.incomingPairings where pairing.phase == .awaitingApproval && !prompted.contains(pairing.attemptId) {
            prompted.insert(pairing.attemptId)
            promptApproval(pairing)
        }
    }

    /// Approval happens on the controlled device, after the user compares the codes both devices show.
    private func promptApproval(_ pairing: BindingPairingStatus) {
        NSApp.requestUserAttention(.criticalRequest)
        let alert = NSAlert()
        alert.messageText = "“\(pairing.name)”请求控制这台 Mac"
        alert.informativeText = "请确认对方设备上显示的核对码与下面一致：\n\n\(Self.grouped(pairing.verificationCode ?? ""))\n\n允许后，它可以使用这台 Mac 上的会话、运行时与配置。"
        alert.addButton(withTitle: "允许")
        alert.addButton(withTitle: "拒绝")
        // Granting control takes a click: a stray Return must not allow a device.
        alert.buttons[0].keyEquivalent = ""
        alert.buttons[1].keyEquivalent = "\u{1b}"
        if alert.runModal() == .alertFirstButtonReturn {
            perform("允许") { _ = try $0.approvePairing(attemptId: pairing.attemptId, expectedPeerId: pairing.peerId) }
        } else {
            perform("拒绝") { _ = try $0.rejectPairing(attemptId: pairing.attemptId, expectedPeerId: pairing.peerId) }
        }
    }

    static func grouped(_ code: String) -> String {
        code.count == 8 ? "\(code.prefix(4)) \(code.suffix(4))" : code
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
    func revoke(_ peerID: String) { perform("撤销控制") { try $0.revokeControl(peerId: peerID) } }
    func forget(_ peerID: String) {
        perform("移除设备") { try $0.forgetDevice(peerId: peerID) }
        stores[peerID] = nil
    }

    /// The application store of a paired device, shared by its windows.
    func store(for peerID: String) -> AppStore {
        if let store = stores[peerID] { return store }
        let endpoint = endpoint
        let view = "mac-\(UUID().uuidString)"
        let transport = Transport(stateDirectory: local.stateDirectory) {
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
        let store = AppStore(transport: transport)
        stores[peerID] = store
        return store
    }

    func name(of peerID: String) -> String? { state?.contacts.first { $0.peerId == peerID }?.name }

    func controller(_ peerID: String) -> BindingControllerStatus? { state?.controllers.first { $0.peerId == peerID } }
}

/// Settings for the devices that control this Mac and the ones it controls.
struct DeviceSettingsView: View {
    @ObservedObject var hub: DeviceHub
    @Environment(\.openWindow) private var openWindow
    @State private var offer = ""
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
                    Text("在另一台设备上扫描二维码或粘贴邀请，两台设备显示的核对码一致时，在这台 Mac 上允许。").font(.caption).foregroundStyle(.secondary)
                }
                ForEach(hub.state?.incomingGrants ?? [], id: \.peerId) { grant in
                    LabeledContent {
                        Button("撤销") { hub.revoke(grant.peerId) }
                    } label: {
                        Text(grant.name)
                        Text("可以控制这台 Mac").foregroundStyle(.secondary)
                    }
                }
            }
            Section("控制其他设备") {
                ForEach(hub.state?.contacts ?? [], id: \.peerId) { contact in
                    DeviceRow(contact: contact, controller: hub.controller(contact.peerId), outgoing: hub.state?.outgoingPairings.first { $0.peerId == contact.peerId }) {
                        openWindow(id: ConversationWindowKind.workspace.sceneID, value: WindowSeed(device: contact.peerId))
                    } forget: { hub.forget(contact.peerId) } cancel: { hub.cancelPairing(contact.peerId) }
                }
                HStack {
                    TextField("邀请", text: $offer, prompt: Text("粘贴另一台设备的邀请"))
                    Button("连接") { hub.beginPairing(offer); offer = "" }.disabled(offer.trimmingCharacters(in: .whitespacesAndNewlines).isEmpty || hub.state == nil)
                }
                Text("在要控制的设备上选择添加控制设备，把它显示的邀请粘贴到这里。连接后核对两台设备的核对码，并在那台设备上允许。").font(.caption).foregroundStyle(.secondary)
            }
            if let failure = hub.failure {
                Section { Text(failure).foregroundStyle(.red) }
            }
        }.formStyle(.grouped)
    }
}

private struct DeviceRow: View {
    let contact: BindingDeviceContact
    let controller: BindingControllerStatus?
    let outgoing: BindingPairingStatus?
    let open: () -> Void
    let forget: () -> Void
    let cancel: () -> Void
    var body: some View {
        LabeledContent {
            if case .confirmed = contact.confirmation {
                Button("打开") { open() }
            } else {
                Button("取消") { cancel() }
            }
        } label: {
            Text(contact.name)
            Text(status).foregroundStyle(.secondary)
        }
        .contextMenu { Button("移除此设备", role: .destructive) { forget() } }
    }

    private var status: String {
        if let outgoing, outgoing.phase != .granted {
            switch outgoing.phase {
            case .awaitingReady, .awaitingApproval:
                return "等待对方允许 · 核对码 \(DeviceHub.grouped(outgoing.verificationCode ?? ""))"
            case .rejected: return "对方已拒绝"
            case .expired: return "邀请已过期"
            case .disconnected: return "配对时连接中断"
            case .granted: break
            }
        }
        guard let controller else { return "未连接" }
        switch controller.phase {
        case .connected: return "已连接"
        case .connecting, .discovering: return "正在连接"
        case .recovering: return "正在重新连接"
        case .suspended, .disconnected: return "未连接"
        case .blocked: return "无法连接" + (controller.errorCode.map { "（\($0)）" } ?? "")
        }
    }
}

private struct InvitationView: View {
    let invitation: BindingRemoteInvitation
    let done: () -> Void
    var body: some View {
        VStack(alignment: .leading, spacing: 8) {
            HStack(alignment: .top, spacing: 16) {
                if let image = Self.qrCode(invitation.encoded) {
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

    static func qrCode(_ text: String) -> NSImage? {
        let filter = CIFilter.qrCodeGenerator()
        filter.message = Data(text.utf8)
        filter.correctionLevel = "M"
        guard let output = filter.outputImage else { return nil }
        let representation = NSCIImageRep(ciImage: output)
        let image = NSImage(size: representation.size)
        image.addRepresentation(representation)
        return image
    }
}
