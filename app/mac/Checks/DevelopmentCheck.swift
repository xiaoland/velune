#if VELUNE_DEV_CHECKS
import AppKit

/// Keep launch checks in their original lifecycle phase. Window checks run
/// after workspace restoration, in order, and can be combined.
@MainActor
enum DevelopmentCheck {
    private enum Phase { case beforeSwiftUI, inApp }

    private struct LaunchCheck {
        let flag: String
        let phase: Phase
        let requires: String?
        let run: @MainActor () -> Never

        init(_ flag: String, _ phase: Phase, requires: String? = nil, run: @escaping @MainActor () -> Never) {
            self.flag = flag
            self.phase = phase
            self.requires = requires
            self.run = run
        }
    }

    private static let arguments = Set(CommandLine.arguments.dropFirst())
    private static let earlyExperienceOptions: Set<String> = [
        "--experience-message-editor-offscreen",
        "--experience-fork-send-only",
        "--experience-claude-compact-edit-only",
        "--experience-appearance-zoom-only",
        "--experience-runtime-editor-only",
        "--experience-compaction-only",
        "--experience-sidebar-only",
        "--experience-runtime-recovery-only",
        "--experience-workspace-only",
    ]
    private static let retiredComposerChecks: Set<String> = [
        "--check-composer-state",
        "--check-composer-recovery-write",
        "--check-composer-recovery-read",
    ]

    // First match wins within each lifecycle phase.
    private static let launchChecks: [LaunchCheck] = [
        .init("--check-conversation-requests", .beforeSwiftUI, run: conversationRequestChecks),
        .init("--check-notification-navigation", .beforeSwiftUI, run: notificationNavigationChecks),
        .init("--check-outgoing-submissions", .beforeSwiftUI, run: outgoingSubmissionChecks),
        .init("--check-sidebar-toolbar", .beforeSwiftUI, run: sidebarToolbarChecks),
        .init("--check-background-notifications", .beforeSwiftUI, run: backgroundNotificationChecks),
        .init("--check-provider-analytics", .beforeSwiftUI, run: providerAnalyticsOffscreenChecks),
        .init("--check-archived-conversations", .beforeSwiftUI, run: archivedConversationsChecks),
        .init("--check-palette-ime", .beforeSwiftUI, run: commandPaletteChecks),
        .init("--check-list-pagination", .beforeSwiftUI, run: conversationPaginationChecks),
        .init("--check-palette-search", .beforeSwiftUI, run: conversationPaginationChecks),
        .init("--check-background-tasks", .beforeSwiftUI, run: backgroundTaskChecks),
        .init("--check-continuation-preparation", .beforeSwiftUI, run: continuationPreparationChecks),
        .init("--check-queued-message-editor", .beforeSwiftUI, run: queuedMessageEditorChecks),
        .init("--check-composer-ownership", .beforeSwiftUI, run: composerOwnershipCheck),
        .init("--check-composer-controls", .beforeSwiftUI, requires: "--check-editor-visual", run: runEditorVisualCheck),
        .init("--check-new-tab", .inApp, run: composerOwnershipCheck),
        .init("--check-composer-controls", .inApp, run: composerControlsCheck),
        .init("--check-runtime-interactions", .inApp, run: runtimeInteractionChecks),
        .init("--check-aggregated-list", .inApp, run: aggregatedListChecks),
        .init("--check-message-experience", earlyExperienceOptions.isDisjoint(with: arguments) ? .inApp : .beforeSwiftUI,
              run: messageExperienceChecks),
    ]

    static func runBeforeSwiftUI(admit: () -> Void) {
        guard let check = requestedLaunchCheck(in: .beforeSwiftUI) else { return }
        admit()
        check.run()
    }

    static func runInApp() {
        if !retiredComposerChecks.isDisjoint(with: arguments) {
            FileHandle.standardError.write(Data("旧 Composer 双所有权检查已退役；请使用 --check-composer-ownership --composer-check-content-target 与 --check-composer-controls。\n".utf8))
            exit(2)
        }
        if let check = requestedLaunchCheck(in: .inApp) { check.run() }
    }

    private static func requestedLaunchCheck(in phase: Phase) -> LaunchCheck? {
        launchChecks.first { check in
            check.phase == phase
                && arguments.contains(check.flag)
                && (check.requires.map(arguments.contains) ?? true)
        }
    }

    private static func runEditorVisualCheck() -> Never {
        let path = ProcessInfo.processInfo.environment["VELUNE_CHECK_OUTPUT"] ?? "/tmp/velune-next-ux/composer"
        let directory = URL(fileURLWithPath: path, isDirectory: true)
        try! FileManager.default.createDirectory(at: directory, withIntermediateDirectories: true)
        messageEditorVisualCheck(directory: directory)
        exit(0)
    }

    /// Check fixtures can have no NSWindow while exercising data-driven polling.
    static var allowsSyntheticPolling: Bool {
        MacExecutionContext.admitted.isDevelopment && arguments.contains { $0.hasPrefix("--check-") }
    }

    static func runAfterWindowStart(_ window: ConversationWindowModel) {
        if arguments.contains("--check-split-panes"), !window.isSplit {
            window.split(.horizontal)
            window.openInNewTab()
            window.split(.vertical)
        }
        if arguments.contains("--check-tool-windows") {
            window.tools.move(.list, to: .floating)
            window.tools.move(.outline, to: .window)
            window.tools.move(.composer, to: .floating)
        }
        if arguments.contains("--check-send-queue") { window.focused.checkSendQueue() }
        if arguments.contains("--check-edit-message") { window.focused.checkEditMessage() }
        if arguments.contains("--check-list-actions") { window.checkListActions() }
        if arguments.contains("--check-side-chat") { window.checkSideChat() }
        if arguments.contains("--check-group-creation") { window.checkGroupCreation() }
        if arguments.contains("--check-dock-areas") {
            window.tools.move(.outline, to: .docked(.left))
            window.tools.move(.list, to: .docked(.right))
            window.tools.move(.composer, to: .docked(.bottom))
        }
        guard window.application.isPreview else { return }
        if arguments.contains("--check-composer-minimum") {
            window.tools.bottomHeight = Double(ComposerTextView.minimumHeight + 54)
            UserDefaults.standard.set(true, forKey: "NSQuitAlwaysKeepsWindows")
        }
        if arguments.contains("--check-composer-sharing") { window.checkComposerSharing() }
    }
}
#endif
