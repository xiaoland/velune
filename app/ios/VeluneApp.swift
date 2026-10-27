import SwiftUI

@main
struct VeluneApp: App {
    @StateObject private var directory = DeviceDirectory()
    var body: some Scene {
        WindowGroup("Velune", id: "workspace", for: WindowRoute.self) { $route in
            if ProcessInfo.processInfo.arguments.contains("--preview") { PreviewConversation() }
            else { WorkWindow(route: route, directory: directory) }
        } defaultValue: { WindowRoute() }
        .commands { WorkspaceCommands() }
    }
}

private struct WorkWindow: View {
    @StateObject private var devices: DeviceStore
    @Environment(\.scenePhase) private var phase
    @Environment(\.openWindow) private var openWindow
    @Environment(\.supportsMultipleWindows) private var multipleWindows
    @State private var showingProblems = false
    @State private var pendingURL: URL?
    @State private var choosingDevice = false
    init(route: WindowRoute, directory: DeviceDirectory) {
        _devices = StateObject(wrappedValue: DeviceStore(route: route, directory: directory))
    }
    var body: some View {
        Group {
            if devices.state == nil {
                NavigationStack {
                    ProgressView("连接设备…")
                        .toolbar { ProblemsButton() }
                }
            } else if let peer = devices.peer { WorkspaceView(devices: devices, peer: peer, pendingURL: $pendingURL).id(peer) }
            else { DevicesView(devices: devices) }
        }
        .focusedSceneValue(\.newWorkspaceWindow, multipleWindows ? { openWindow(id: "workspace", value: WindowRoute(peer: devices.peer)) } : nil)
        .focusedSceneValue(\.showProblems, { showingProblems = true })
        .sheet(isPresented: $showingProblems) { ProblemsView() }
        .task(id: phase) { await devices.activity(foreground: phase == .active) }
        .onChange(of: devices.peer) { _, _ in Task { await devices.activity(foreground: phase == .active) } }
        .onChange(of: devices.state?.contacts) { _, contacts in
            if let peer = devices.peer, contacts?.contains(where: { $0.peerId == peer }) == false { devices.peer = nil }
            if pendingURL != nil, devices.peer == nil { chooseDevice() }
        }
        .onOpenURL { url in
            guard url.scheme?.lowercased() == "agents" else { return }
            pendingURL = url
            if devices.peer == nil { chooseDevice() }
        }
        .confirmationDialog("选择会话所在设备", isPresented: $choosingDevice, titleVisibility: .visible) {
            ForEach(devices.state?.contacts.filter(\.controlAuthorized) ?? [], id: \.peerId) { contact in Button(contact.name) { devices.peer = contact.peerId } }
            Button("取消", role: .cancel) { pendingURL = nil }
        }
    }
    private func chooseDevice() {
        let contacts = devices.state?.contacts.filter(\.controlAuthorized) ?? []
        if contacts.count == 1 { devices.peer = contacts[0].peerId }
        else if contacts.count > 1 { choosingDevice = true }
    }
}
