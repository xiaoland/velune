import AppKit
import OSLog
@preconcurrency import UserNotifications
import VeluneBindings

/// An empty device ID explicitly means this Mac. A missing device ID is an
/// older notification whose execution device must be recovered separately.
struct ConversationNotificationTarget: Equatable {
    let conversationID: String
    let peerID: String?

    var userInfo: [String: String] {
        ["conversationID": conversationID, "deviceID": peerID ?? ""]
    }

    var requestIdentifier: String {
        let device = peerID ?? ""
        return "conversation:\(device.utf8.count):\(device):\(conversationID)"
    }

    init(conversationID: String, peerID: String?) {
        self.conversationID = conversationID
        self.peerID = peerID
    }

    init?(userInfo: [AnyHashable: Any]) {
        guard let conversation = userInfo["conversationID"] as? String, !conversation.isEmpty,
              let device = userInfo["deviceID"] as? String else { return nil }
        self.init(conversationID: conversation, peerID: device.isEmpty ? nil : device)
    }
}

/// UserNotifications has one delegate per process. Device notifiers only
/// contribute activity; this app-lifetime owner resolves every response.
@MainActor
final class ConversationNotificationRouter: NSObject, UNUserNotificationCenterDelegate {
    static let shared = ConversationNotificationRouter()
    private nonisolated static let logger = Logger(subsystem: "local.velune", category: "notifications")
    private struct Source { weak var notifier: ConversationNotifier? }
    private var sources: [String: Source] = [:]

    private override init() {
        super.init()
        UNUserNotificationCenter.current().delegate = self
        NotificationCenter.default.addObserver(forName: NSApplication.didBecomeActiveNotification, object: nil, queue: .main) { [weak self] _ in
            MainActor.assumeIsolated {
                guard let self else { return }
                for source in self.sources.values { source.notifier?.clearUnseenFinished() }
                self.updateBadge()
            }
        }
    }

    fileprivate func register(_ notifier: ConversationNotifier, application: DeviceStore) {
        sources[application.peerID ?? ""] = Source(notifier: notifier)
    }

    private var applications: [DeviceStore] {
        sources = sources.filter { $0.value.notifier?.application != nil }
        return sources.values.compactMap { $0.notifier?.application }
    }

    private func application(peerID: String?) -> DeviceStore? {
        if let hub = DeviceHub.shared {
            guard let peerID else { return hub.localStore }
            guard let contact = hub.state?.contacts.first(where: { $0.peerId == peerID && $0.controlAuthorized }),
                  case .confirmed = contact.confirmation else { return nil }
            return hub.store(for: peerID)
        }
        return applications.first { $0.peerID == peerID }
    }

    /// Shared by the system callback and the isolated navigation checks.
    /// Legacy notifications can be recovered only when the device is unique.
    @discardableResult func open(userInfo: [AnyHashable: Any]) -> Bool {
        let store: DeviceStore?
        let conversationID: String
        if let target = ConversationNotificationTarget(userInfo: userInfo) {
            conversationID = target.conversationID
            store = application(peerID: target.peerID)
        } else {
            guard userInfo["deviceID"] == nil,
                  let id = userInfo["conversationID"] as? String, !id.isEmpty else { return false }
            conversationID = id
            let candidates = applications.filter { application in
                application.conversations.contains { $0.id == id }
                    || application.runningConversationIDs.contains(id)
                    || application.waitingConversationIDs.contains(id)
                    || ConversationWindowModel.openWindows.contains { $0.tab(showing: id, device: application) != nil }
            }
            store = candidates.count == 1 ? application(peerID: candidates[0].peerID) : nil
        }
        guard let store else {
            (DeviceHub.shared?.localStore ?? applications.first { $0.peerID == nil })?
                .recordProblem("通知的设备无法确定或已不可用，请从会话列表打开目标会话。", source: "打开通知")
            return false
        }
        store.showConversation(conversationID)
        return true
    }

    fileprivate func post(_ request: UNNotificationRequest) {
        let center = UNUserNotificationCenter.current()
        // The center remembers the user's decision. Requests arriving while
        // authorization is pending all wait for that decision before delivery.
        center.requestAuthorization(options: [.alert, .sound]) { granted, error in
            if let error = error as NSError? {
                Self.logger.error("Notification authorization failed: \(error.domain, privacy: .public) (\(error.code))")
            }
            guard granted else { return }
            center.add(request) { error in
                if let error = error as NSError? {
                    Self.logger.error("Notification submission failed: \(error.domain, privacy: .public) (\(error.code))")
                }
            }
        }
    }

    var badgeCount: Int {
        _ = applications
        return sources.values.reduce(0) { $0 + ($1.notifier?.attentionCount ?? 0) }
    }

    fileprivate func updateBadge() {
        guard NSApp.activationPolicy() != .prohibited else { return }
        let count = badgeCount
        let label = count > 0 ? String(count) : nil
        if NSApp.dockTile.badgeLabel != label { NSApp.dockTile.badgeLabel = label }
    }

    nonisolated func userNotificationCenter(_ center: UNUserNotificationCenter, willPresent notification: UNNotification) async -> UNNotificationPresentationOptions {
        [.banner, .list, .sound]
    }

    nonisolated func userNotificationCenter(_ center: UNUserNotificationCenter, didReceive response: UNNotificationResponse) async {
        guard response.actionIdentifier == UNNotificationDefaultActionIdentifier else { return }
        await MainActor.run {
            NSApp.activate()
            open(userInfo: response.notification.request.content.userInfo)
        }
    }
}

/// System notifications and the Dock badge for conversations that need the
/// user: a Velune turn that ended, or a runtime request that waits for an
/// answer. Nothing is shown for the conversation the user is looking at, and
/// a turn the user stopped is not announced. Whether and how notifications
/// appear is the user's choice in System Settings.
@MainActor
final class ConversationNotifier {
    fileprivate weak var application: DeviceStore?
    private let submit: (UNNotificationRequest) -> Void
    private var pendingCounts: [String: UInt32] = [:]
    /// Conversations whose turn ended while Velune was in the background.
    private var unseenFinished: Set<String> = []

    init(application: DeviceStore, submit: ((UNNotificationRequest) -> Void)? = nil) {
        self.application = application
        let router = ConversationNotificationRouter.shared
        self.submit = submit ?? router.post
        router.register(self, application: application)
    }

    func observe(_ activity: ConversationActivity) {
        var counts: [String: UInt32] = [:]
        for item in activity.active where item.pendingInteractions > 0 {
            counts[item.conversationId] = item.pendingInteractions
            if item.pendingInteractions > pendingCounts[item.conversationId, default: 0] {
                post(conversationID: item.conversationId, runtimeID: item.contextRuntimeId, body: "等待你的确认")
            }
        }
        pendingCounts = counts
        for turn in activity.finished where !turn.silent {
            let body: String
            switch turn.outcome {
            case .completed: body = "已完成"
            case .failed: body = "执行失败，打开会话查看原因"
            case .cancelled: continue
            }
            if !NSApp.isActive { unseenFinished.insert(turn.conversationId) }
            post(conversationID: turn.conversationId, runtimeID: turn.contextRuntimeId, body: body)
        }
        ConversationNotificationRouter.shared.updateBadge()
    }

    /// A provider newly paused by its quota. One notification per provider
    /// and condition; a later one replaces it.
    func observe(_ alert: QuotaAlert) {
        guard let application, let provider = application.providers.first(where: { $0.id == alert.providerId }) else { return }
        let window = alert.windowId.map(QuotaWindowObservation.title) ?? ""
        let body: String
        switch alert.kind {
        case .capReached:
            body = "\(window)\(UsageFormat.ceiling(alert.maxUsedPercent ?? 100, application.usageDisplay, reached: true))，直到额度回到上限以内。"
        case .exhausted:
            body = "\(window)已用完；额外额度尚未完整查询，实际可用性以服务商响应为准。"
        case .unavailable:
            body = QuotaStatusText.limitedObservation
        }
        let content = UNMutableNotificationContent()
        content.title = provider.name
        content.subtitle = "额度提醒"
        content.body = body
        let device = application.peerID ?? ""
        let request = UNNotificationRequest(identifier: "quota:\(device.utf8.count):\(device):\(alert.providerId):\(alert.kind):\(alert.windowId ?? "")", content: content, trigger: nil)
        submit(request)
    }

    fileprivate var attentionCount: Int { Set(pendingCounts.keys).union(unseenFinished).count }

    fileprivate func clearUnseenFinished() {
        unseenFinished.removeAll()
    }

    private func post(conversationID: String, runtimeID: String, body: String) {
        guard let application, !application.isShowingConversation(conversationID) else { return }
        let content = UNMutableNotificationContent()
        content.title = application.conversations.first { $0.id == conversationID }?.title ?? "会话"
        if let runtime = application.runtimeInstances.first(where: { $0.id == runtimeID }) { content.subtitle = runtime.name }
        content.body = body
        content.sound = .default
        let target = ConversationNotificationTarget(conversationID: conversationID, peerID: application.peerID)
        content.userInfo = target.userInfo
        // One notification per conversation: a newer state replaces the older one.
        let request = UNNotificationRequest(identifier: target.requestIdentifier, content: content, trigger: nil)
        submit(request)
    }
}
