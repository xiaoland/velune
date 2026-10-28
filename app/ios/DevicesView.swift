import SwiftUI
import VisionKit
import AVFoundation

struct DevicesView: View {
    @ObservedObject var devices: DeviceStore
    @State private var adding = false
    @State private var forgetting: DeviceContact?
    private var controlling: [DeviceContact] { devices.state?.contacts.filter(\.controlAuthorized) ?? [] }
    var body: some View {
        NavigationStack {
            List {
                if controlling.isEmpty {
                    ContentUnavailableView {
                        Label("添加设备", systemImage: "desktopcomputer")
                    } description: { Text("在桌面 Velune 的设备设置中生成邀请。") } actions: {
                        Button("添加设备") { adding = true }.buttonStyle(.borderedProminent).disabled(devices.state == nil)
                    }.listRowBackground(Color.clear)
                }
                ForEach(controlling, id: \.peerId) { contact in
                    Button { devices.peer = contact.peerId } label: {
                        HStack {
                            Image(systemName: "desktopcomputer").font(.title2)
                            VStack(alignment: .leading) { Text(contact.name); Text(devices.status(contact.peerId)).font(.caption).foregroundStyle(.secondary) }
                            Spacer()
                            Image(systemName: "chevron.right").foregroundStyle(.tertiary)
                        }.padding(.vertical, 8)
                    }.foregroundStyle(.primary)
                    .swipeActions { Button("移除", role: .destructive) { forgetting = contact } }
                    .contextMenu { Button("移除", role: .destructive) { forgetting = contact } }
                }
                Section {
                    NavigationLink("遥测") {
                        TelemetrySettingsView(coordinator: { try RemoteService.shared.resources.telemetry() }, remote: { try RemoteService.shared.resources.endpoint() })
                            .navigationTitle("遥测")
                    }
                    NavigationLink("关于与开源许可") { AboutView() }
                }
            }.navigationTitle("Velune").toolbar {
                ProblemsButton()
                NewWindowButton(peer: nil, target: nil)
                Button { adding = true } label: { Label("添加设备", systemImage: "plus") }.disabled(devices.state == nil)
            }
            .refreshable { await devices.refresh() }
            .sheet(isPresented: $adding) { AddDeviceView(devices: devices) }
            .confirmationDialog("移除这台设备？", isPresented: Binding(get: { forgetting != nil }, set: { if !$0 { forgetting = nil } }), titleVisibility: .visible) {
                Button("移除", role: .destructive) {
                    if let contact = forgetting { devices.perform { try await $0.forget(contact.peerId) } }
                    forgetting = nil
                }
            } message: { Text("只移除本机记录；要撤销授权，请在被控设备上操作。") }
        }
    }
}

private struct AddDeviceView: View {
    @ObservedObject var devices: DeviceStore
    @Environment(\.dismiss) private var dismiss
    @State private var offer = ""
    @State private var decoded: BindingPairingOfferInfo?
    @State private var pairingPeer: String?
    @State private var failure: String?
    @State private var busy = false
    @State private var scanning = false
    private var pairing: PairingStatus? { devices.state?.outgoingPairings.first { $0.peerId == pairingPeer } }
    var body: some View {
        NavigationStack {
            Form {
                if let failure { FailureView(text: failure) }
                if let pairing {
                    Section(pairing.name) {
                        if let code = pairing.verificationCode {
                            Text("\(code.prefix(4)) \(code.suffix(4))").font(.largeTitle.monospaced()).textSelection(.enabled)
                            Text(pairing.permission == .telemetryProfileRead ? "核对两端数字，在来源设备上允许读取遥测配置（含认证）。" : "核对两端数字，并在被控设备上允许连接。")
                        }
                        Text(pairingStatus(pairing.phase))
                        if pairing.phase == .rejected || pairing.phase == .expired || pairing.phase == .disconnected {
                            Button("重新输入邀请") { pairingPeer = nil; decoded = nil }
                        }
                    }
                } else {
                    Section {
                        TextField("粘贴设备邀请", text: $offer, axis: .vertical).lineLimit(3...6).textInputAutocapitalization(.never).autocorrectionDisabled()
                            .onChange(of: offer) { _, _ in decoded = nil }
                        PasteButton(payloadType: String.self) { offer = $0.first ?? "" }
                        Button("扫描二维码", systemImage: "qrcode.viewfinder") {
                            Task {
                                if await AVCaptureDevice.requestAccess(for: .video), DataScannerViewController.isSupported, DataScannerViewController.isAvailable { scanning = true }
                                else { failure = "无法使用相机，请在系统设置允许相机访问，或粘贴邀请。" }
                            }
                        }
                    }
                    if let decoded {
                        Section("连接设备") {
                            Text(decoded.name)
                            if decoded.permission == .telemetryProfileRead { Text("读取遥测配置（含认证）") }
                            Button("连接") { begin() }.disabled(busy)
                        }
                    } else {
                        Button("读取邀请") {
                            do { decoded = try decodeRemoteOffer(encoded: offer.trimmingCharacters(in: .whitespacesAndNewlines)); failure = nil }
                            catch { failure = mobileError(error) }
                        }.disabled(offer.trimmingCharacters(in: .whitespacesAndNewlines).isEmpty)
                    }
                }
            }.navigationTitle("添加设备").toolbar {
                ToolbarItem(placement: .cancellationAction) { Button("取消") {
                    if let pairingPeer { devices.perform { client in _ = try await client.service { try $0.cancelPairing(peerId: pairingPeer) } } }
                    dismiss()
                } }
            }
            .interactiveDismissDisabled(pairingPeer != nil)
            .sheet(isPresented: $scanning) {
                NavigationStack {
                    QRScanner { offer = $0; scanning = false }.ignoresSafeArea(edges: .bottom)
                        .navigationTitle("扫描邀请").toolbar { Button("取消") { scanning = false } }
                }
            }
            .onChange(of: pairing?.phase) { _, phase in
                if phase == .granted, let pairingPeer {
                    if pairing?.permission == .control { devices.peer = pairingPeer }
                    dismiss()
                }
            }
        }
    }
    private func begin() {
        busy = true
        let encoded = offer.trimmingCharacters(in: .whitespacesAndNewlines)
        Task {
            defer { busy = false }
            do {
                let pairing = try await devices.client.service { try $0.beginPairing(encoded: encoded) }
                pairingPeer = pairing.peerId; offer = ""; decoded = nil
                await devices.refresh()
            } catch { failure = mobileError(error) }
        }
    }
    private func pairingStatus(_ phase: PairingPhase) -> String {
        switch phase {
        case .awaitingReady: return "正在连接"
        case .awaitingApproval: return "等待来源设备允许"
        case .granted: return "已配对"
        case .rejected: return "被控设备拒绝了连接"
        case .expired: return "邀请已过期"
        case .disconnected: return "连接已断开"
        }
    }
}

private struct QRScanner: UIViewControllerRepresentable {
    let found: (String) -> Void
    func makeCoordinator() -> Coordinator { Coordinator(found: found) }
    func makeUIViewController(context: Context) -> DataScannerViewController {
        let scanner = DataScannerViewController(recognizedDataTypes: [.barcode(symbologies: [.qr])], qualityLevel: .balanced, recognizesMultipleItems: false, isGuidanceEnabled: true, isHighlightingEnabled: true)
        scanner.delegate = context.coordinator
        do { try scanner.startScanning() } catch { context.coordinator.error = mobileError(error) }
        return scanner
    }
    func updateUIViewController(_ controller: DataScannerViewController, context: Context) {
        if let error = context.coordinator.error {
            let alert = UIAlertController(title: "无法扫描", message: error, preferredStyle: .alert)
            alert.addAction(UIAlertAction(title: "关闭", style: .default))
            controller.present(alert, animated: true)
            context.coordinator.error = nil
        }
    }
    static func dismantleUIViewController(_ controller: DataScannerViewController, coordinator: Coordinator) { controller.stopScanning() }
    final class Coordinator: NSObject, DataScannerViewControllerDelegate {
        let found: (String) -> Void
        var finished = false
        var error: String?
        init(found: @escaping (String) -> Void) { self.found = found }
        func dataScanner(_ dataScanner: DataScannerViewController, didAdd addedItems: [RecognizedItem], allItems: [RecognizedItem]) {
            guard !finished else { return }
            for item in addedItems {
                if case .barcode(let code) = item, let value = code.payloadStringValue { finished = true; dataScanner.stopScanning(); found(value); return }
            }
        }
    }
}

private struct AboutView: View {
    @State private var licenses = ""
    var body: some View {
        ScrollView {
            VStack(alignment: .leading, spacing: 16) {
                Text("Velune \(Bundle.main.infoDictionary?["CFBundleShortVersionString"] as? String ?? "")").font(.title2)
                Text("iPad 与 iPhone 远程控制端")
                Text(licenses).font(.caption).textSelection(.enabled)
            }.padding()
        }.navigationTitle("关于与开源许可").task {
            do {
                guard let url = Bundle.main.url(forResource: "rust-licenses", withExtension: "txt") else { throw CocoaError(.fileNoSuchFile) }
                licenses = try String(contentsOf: url, encoding: .utf8)
                if let swift = Bundle.main.url(forResource: "swift-licenses", withExtension: "txt") { licenses += "\n\n" + (try String(contentsOf: swift, encoding: .utf8)) }
            } catch { licenses = mobileError(error) }
        }
    }
}
