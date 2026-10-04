import SwiftUI

extension Notification.Name {
    static let veluneSend = Notification.Name("velune.ui.send")
}

@main
@MainActor
struct VeluneApplication: App {
    @StateObject private var store: AppStore
    private let previewEmpty: Bool
    private let previewSettings: Bool

    init() {
        let preview = Bundle.main.bundleIdentifier == "local.velune.visual-preview" || CommandLine.arguments.contains { $0.hasPrefix("--preview") }
        _store = StateObject(wrappedValue: AppStore(preview: preview))
        previewEmpty = CommandLine.arguments.contains("--preview-empty")
        previewSettings = CommandLine.arguments.contains("--preview-settings")
    }

    var body: some Scene {
        WindowGroup("Velune") {
            VeluneRootView(store: store, previewEmpty: previewEmpty, previewSettings: previewSettings)
                .frame(minWidth: 780, minHeight: 540)
        }
        .defaultSize(width: 1060, height: 760)
        .commands {
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
