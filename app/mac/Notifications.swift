import AppKit
@preconcurrency import UserNotifications
import VeluneBindings

/// System notifications and the Dock badge for conversations that need the
/// user: a Velune turn that ended, or a runtime request that waits for an
/// answer. Nothing is shown for the conversation the user is looking at, and
/// a turn the user stopped is not announced. Whether and how notifications
/// appear is the user's choice in System Settings.
@MainActor
final class ConversationNotifier: NSObject, UNUserNotificationCenterDelegate {
    private weak var application: AppStore?
    private var authorizationRequested = false
    private var pendingCounts: [String: UInt32] = [:]
    /// Conversations whose turn ended while Velune was in the background.
    private var unseenFinished: Set<String> = []

    init(application: AppStore) {
        self.application = application
        super.init()
        UNUserNotificationCenter.current().delegate = self
        NotificationCenter.default.addObserver(forName: NSApplication.didBecomeActiveNotification, object: nil, queue: .main) { [weak self] _ in
            MainActor.assumeIsolated { self?.unseenFinished.removeAll(); self?.updateBadge() }
        }
    }

    func observe(_ activity: BindingConversationActivity) {
        var counts: [String: UInt32] = [:]
        for item in activity.active where item.pendingInteractions > 0 {
            counts[item.conversationId] = item.pendingInteractions
            if item.pendingInteractions > pendingCounts[item.conversationId, default: 0] {
                post(conversationID: item.conversationId, runtimeID: item.contextRuntimeId, body: "等待你的确认")
            }
        }
        pendingCounts = counts
        for turn in activity.finished {
            let body: String
            switch turn.outcome {
            case .completed: body = "已完成"
            case .failed: body = "执行失败，打开会话查看原因"
            case .cancelled: continue
            }
            if !NSApp.isActive { unseenFinished.insert(turn.conversationId) }
            post(conversationID: turn.conversationId, runtimeID: turn.contextRuntimeId, body: body)
        }
        updateBadge()
    }

    private func updateBadge() {
        let count = Set(pendingCounts.keys).union(unseenFinished).count
        let label = count > 0 ? String(count) : nil
        if NSApp.dockTile.badgeLabel != label { NSApp.dockTile.badgeLabel = label }
    }

    private func post(conversationID: String, runtimeID: String, body: String) {
        guard let application, !application.isShowingConversation(conversationID) else { return }
        let content = UNMutableNotificationContent()
        content.title = application.conversations.first { $0.id == conversationID }?.title ?? "会话"
        if let runtime = application.runtimeInstances.first(where: { $0.id == runtimeID }) { content.subtitle = runtime.name }
        content.body = body
        content.sound = .default
        content.userInfo = ["conversationID": conversationID]
        // One notification per conversation: a newer state replaces the older one.
        let request = UNNotificationRequest(identifier: "conversation:" + conversationID, content: content, trigger: nil)
        let center = UNUserNotificationCenter.current()
        if authorizationRequested { center.add(request); return }
        authorizationRequested = true
        center.requestAuthorization(options: [.alert, .sound]) { granted, _ in
            if granted { center.add(request) }
        }
    }

    nonisolated func userNotificationCenter(_ center: UNUserNotificationCenter, willPresent notification: UNNotification) async -> UNNotificationPresentationOptions {
        [.banner, .sound]
    }

    nonisolated func userNotificationCenter(_ center: UNUserNotificationCenter, didReceive response: UNNotificationResponse) async {
        guard let conversationID = response.notification.request.content.userInfo["conversationID"] as? String else { return }
        await MainActor.run {
            NSApp.activate()
            application?.showConversation(conversationID)
        }
    }
}
