import SwiftUI

@main
struct VeluneApp: App {
    @StateObject private var devices = DeviceStore()
    @Environment(\.scenePhase) private var phase
    @State private var pendingURL: URL?
    @State private var choosingDevice = false
    var body: some Scene {
        WindowGroup {
            Group {
                if ProcessInfo.processInfo.arguments.contains("--preview") { PreviewConversation() }
                else if let peer = devices.peer { WorkspaceView(devices: devices, peer: peer, pendingURL: $pendingURL).id(peer) }
                else { DevicesView(devices: devices) }
            }
            .task(id: phase) {
                guard !ProcessInfo.processInfo.arguments.contains("--preview") else { return }
                await devices.activity(foreground: phase == .active)
                guard phase == .active else { return }
                while !Task.isCancelled {
                    await devices.refresh()
                    do { try await Task.sleep(for: .seconds(1)) } catch { return }
                }
            }
            .onChange(of: devices.state?.contacts) { _, contacts in
                guard pendingURL != nil, devices.peer == nil else { return }
                if contacts?.count == 1 { devices.peer = contacts?.first?.peerId }
                else if (contacts?.count ?? 0) > 1 { choosingDevice = true }
            }
            .onChange(of: devices.peer) { _, _ in Task { await devices.activity(foreground: phase == .active) } }
            .onOpenURL { url in
                guard url.scheme?.lowercased() == "agents" else { return }
                pendingURL = url
                let contacts = devices.state?.contacts ?? []
                if contacts.count == 1 { devices.peer = contacts[0].peerId }
                else if contacts.count > 1 { choosingDevice = true }
            }
            .confirmationDialog("选择会话所在设备", isPresented: $choosingDevice, titleVisibility: .visible) {
                ForEach(devices.state?.contacts ?? [], id: \.peerId) { contact in Button(contact.name) { devices.peer = contact.peerId } }
                Button("取消", role: .cancel) { pendingURL = nil }
            }
        }
    }
}
