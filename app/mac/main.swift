import AppKit
import SwiftUI

struct ConversationCommandTarget {
    let store: AppStore
    let send: () -> Void
}
private struct ConversationCommandTargetKey: FocusedValueKey {
    typealias Value = ConversationCommandTarget
}
extension FocusedValues {
    var conversationCommands: ConversationCommandTarget? {
        get { self[ConversationCommandTargetKey.self] }
        set { self[ConversationCommandTargetKey.self] = newValue }
    }
}

@MainActor
struct ConversationCommands: Commands {
    @FocusedValue(\.conversationCommands) private var target
    var body: some Commands {
        CommandGroup(replacing: .newItem) {
            Button("新建会话") { target?.store.createConversation() }
                .keyboardShortcut("n")
                .disabled(target == nil || target?.store.applicationIsGenerating == true || target?.store.isLoading == true)
        }
        CommandMenu("会话") {
            Button("发送消息") { target?.send() }
                .keyboardShortcut(.return, modifiers: .command)
                .disabled(target?.store.canSend != true)
            Button("停止生成") { target?.store.cancel() }
                .keyboardShortcut(.escape, modifiers: [])
                .disabled(target?.store.canCancel != true)
        }
    }
}

@MainActor
struct ConversationWindow: View {
    @StateObject private var workspace: AppStore
    let previewEmpty: Bool
    let previewSettings: Bool
    init(application: AppStore, previewEmpty: Bool, previewSettings: Bool) {
        _workspace = StateObject(wrappedValue: application.makeWorkspace())
        self.previewEmpty = previewEmpty
        self.previewSettings = previewSettings
    }
    var body: some View {
        VeluneRootView(store: workspace, previewEmpty: previewEmpty, previewSettings: previewSettings)
    }
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
struct VeluneApp: App {
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
            ConversationWindow(application: store, previewEmpty: previewEmpty, previewSettings: previewSettings)
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
                Button("开源许可…") {
                    if let url = Bundle.main.url(forResource: "THIRD_PARTY_NOTICES", withExtension: "md", subdirectory: "ThirdParty") { NSWorkspace.shared.open(url) }
                }
            }
            ConversationCommands()
            CommandGroup(after: .sidebar) {
                AnalyticsMenuItem()
                ProblemsMenuItem(store: store)
            }

        }
        Window("问题", id: "problems") {
            ProblemsView(store: store).frame(minWidth: 600, minHeight: 320)
        }.defaultSize(width: 760, height: 480)
        Window("分析", id: "analytics") {
            AnalyticsView(store: store).frame(minWidth: 680, minHeight: 460)
        }.defaultSize(width: 860, height: 640)
        Settings {
            SettingsView(store: store)
        }
    }
}
