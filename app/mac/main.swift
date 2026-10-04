import AppKit
import SwiftUI

extension Notification.Name {
    static let veluneSend = Notification.Name("velune.ui.send")
}

@MainActor
final class VeluneApplicationDelegate: NSObject, NSApplicationDelegate {
    weak var store: AppStore?
    private var terminationPending = false

    func applicationShouldTerminate(_ sender: NSApplication) -> NSApplication.TerminateReply {
        guard !terminationPending else { return .terminateLater }
        guard let store else { return .terminateNow }
        terminationPending = true
        store.shutdown { [weak self] closed in
            // Preview and unopened cores can complete synchronously. Reply only
            // after AppKit has entered its terminateLater state.
            DispatchQueue.main.async {
                self?.terminationPending = false
                sender.reply(toApplicationShouldTerminate: closed)
            }
        }
        return .terminateLater
    }
}

@main
@MainActor
struct VeluneApplication: App {
    @NSApplicationDelegateAdaptor(VeluneApplicationDelegate.self) private var appDelegate
    @StateObject private var store: AppStore
    private let previewEmpty: Bool
    private let previewSettings: Bool

    init() {
        let preview = Bundle.main.bundleIdentifier == "local.velune.visual-preview" || CommandLine.arguments.contains { $0.hasPrefix("--preview") }
        let applicationStore = AppStore(preview: preview)
        _store = StateObject(wrappedValue: applicationStore)
        previewEmpty = CommandLine.arguments.contains("--preview-empty")
        previewSettings = CommandLine.arguments.contains("--preview-settings")
        appDelegate.store = applicationStore
    }

    var body: some Scene {
        WindowGroup("Velune") {
            VeluneRootView(store: store, previewEmpty: previewEmpty, previewSettings: previewSettings)
                .frame(minWidth: 780, minHeight: 540)
        }
        .defaultSize(width: 1060, height: 760)
        .commands {
            CommandGroup(replacing: .appInfo) {
                Button("关于 Velune") {
                    var options: [NSApplication.AboutPanelOptionKey: Any] = [:]
                    if let version = Bundle.main.object(forInfoDictionaryKey: "VeluneDisplayVersion") as? String {
                        options[.applicationVersion] = version
                    }
                    NSApplication.shared.orderFrontStandardAboutPanel(options: options)
                }
            }
            CommandGroup(replacing: .newItem) {
                Button("新建会话") { store.createConversation() }
                    .keyboardShortcut("n")
                    .disabled(store.isGenerating || store.isLoading)
            }
            CommandMenu("会话") {
                Button("发送消息") { NotificationCenter.default.post(name: .veluneSend, object: nil) }
                    .keyboardShortcut(.return, modifiers: .command)
                    .disabled(!store.canSend)
                Button("停止生成") { store.cancel() }
                    .keyboardShortcut(.escape, modifiers: [])
                    .disabled(!store.canCancel)
            }
        }
        Settings {
            SettingsView(store: store)
        }
    }
}
