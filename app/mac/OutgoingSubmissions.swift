import Combine
import Foundation
import VeluneBindings

struct SubmittedMessage: Equatable {
    var message: DeliveredMessage
    var status: String?
}

/// A submission is visible from the local action through acknowledgement and
/// canonical presentation. Send, queue and steer differ in delivery, not ownership.
@MainActor final class OutgoingSubmissions {
    enum Route: Equatable { case send, queue, steer }
    private struct Submission {
        let id: String
        var route: Route
        let message: DeliveredMessage
        let knownUserIDs: Set<String>
        var queueID: String?
        var queueObserved = false
        var receiptSettled = false
        var visible = true
        var accepted = false
    }
    private var submissions: [Submission] = []
    private var claimedUserIDs = Set<String>()
    let changes = PassthroughSubject<Void, Never>()
    var displayed: [SubmittedMessage] {
        submissions.filter { $0.visible && $0.route != .queue }.map { submission in
            let status: String?
            switch (submission.route, submission.accepted) {
            case (.send, false): status = "发送中"
            case (.queue, false): status = "排队中"
            case (.steer, false): status = "插入中"
            case (.steer, true): status = "已插入本轮"
            default: status = nil
            }
            return SubmittedMessage(message: submission.message, status: status)
        }
    }
    var queued: [QueuedMessage] {
        submissions.filter { $0.visible && $0.route == .queue && $0.receiptSettled && !$0.queueObserved }.map { submission in
            QueuedMessage(id: submission.queueID ?? submission.id, submissionId: submission.id, text: submission.message.blocks.compactMap {
                if case .text(let text) = $0 { return text }; return nil
            }.joined(separator: "\n"), attachments: submission.message.blocks.compactMap {
                switch $0 {
                case .file(let path), .image(_, .path(let path)): return path
                default: return nil
                }
            })
        }
    }
    @discardableResult func begin(_ route: Route, text: String, attachments: [String], snapshot: ConversationSnapshot?) -> String {
        let id = "submission:" + UUID().uuidString.lowercased()
        let images: Set<String> = ["png", "jpg", "jpeg", "gif", "webp"]
        let blocks: [DeliveredBlock] = (text.isEmpty ? [] : [.text(text: text)]) + attachments.map {
            images.contains(URL(fileURLWithPath: $0).pathExtension.lowercased()) ? .image(mediaType: nil, source: .path(path: $0)) : .file(path: $0)
        }
        let message = DeliveredMessage(id: id, role: .user, timestampUnixMs: Int64(Date().timeIntervalSince1970 * 1000), blocks: blocks)
        submissions.append(Submission(id: id, route: route, message: message, knownUserIDs: Set(userIDs(snapshot))))
        changes.send()
        return id
    }
    /// Business admission is independent of projection installation. A send
    /// intent may have been queued by the core while the UI still looked idle.
    func accept(_ id: String, receipt: SubmissionReceipt? = nil, snapshot: ConversationSnapshot?) {
        guard let index = submissions.firstIndex(where: { $0.id == id }) else { return }
        submissions[index].receiptSettled = true
        submissions[index].accepted = true
        switch receipt {
        case .queued(let queueID):
            submissions[index].route = .queue
            submissions[index].queueID = queueID
            // This complete post-admission receipt already owns the queue, or
            // has observed its dispatch/removal. Never turn absence into send.
            if snapshot != nil || submissions[index].queueObserved || !submissions[index].visible {
                submissions.remove(at: index); changes.send(); return
            }
        case .sending:
            if submissions[index].route != .steer { submissions[index].route = .send }
        case nil:
            break
        }
        reconcile(snapshot)
        submissions.removeAll { $0.id == id && !$0.visible }
        changes.send()
    }
    /// A canonical queue observation is admission even when the RPC response
    /// fails later. Restoring its draft would invite a duplicate retry.
    @discardableResult func fail(_ id: String) -> Bool {
        let restoreDraft = submissions.first(where: { $0.id == id }).map { !$0.accepted && !$0.queueObserved } ?? true
        submissions.removeAll { $0.id == id }
        changes.send()
        return restoreDraft
    }
    func reset() {
        // Keep only in-flight receipt ownership, detached from the new view.
        // Its callback still settles the original captured composer input.
        submissions.removeAll { $0.receiptSettled }
        for index in submissions.indices { submissions[index].visible = false }
        claimedUserIDs.removeAll(); changes.send()
    }
    func reconcile(_ snapshot: ConversationSnapshot?) {
        guard let snapshot, !submissions.isEmpty else { return }
        let ids = userIDs(snapshot)
        claimedUserIDs.formIntersection(ids)
        var changed = false
        for index in submissions.indices {
            if let canonical = snapshot.followUp.queue.first(where: {
                $0.submissionId == submissions[index].id || $0.id == submissions[index].queueID
            }) {
                if submissions[index].route != .queue || !submissions[index].queueObserved { changed = true }
                submissions[index].route = .queue
                submissions[index].queueID = canonical.id
                submissions[index].queueObserved = true
                submissions[index].accepted = true
            }
        }
        let oldCount = submissions.count
        submissions.removeAll { submission in
            if submission.route == .queue {
                return submission.receiptSettled && (submission.queueObserved || submission.queueID != nil)
            }
            guard submission.accepted else { return false }
            if let own = ids.first(where: { !submission.knownUserIDs.contains($0) && !claimedUserIDs.contains($0) }) {
                claimedUserIDs.insert(own); return true
            }
            return false
        }
        if changed || oldCount != submissions.count { changes.send() }
    }
    static func queuedCancellation(_ receipt: SubmissionReceipt?) -> SendQueueOperation? {
        if case .queued(let id) = receipt { return .remove(id: id) }
        return nil
    }
    private func userIDs(_ snapshot: ConversationSnapshot?) -> [String] {
        guard let snapshot else { return [] }
        let own = Set(snapshot.messages.filter { message in
            message.role == .user && !message.blocks.contains { if case .agentSender = $0 { return true }; return false }
        }.map(\.id))
        // Application presentation identities survive native ID confirmation;
        // replacing old history cannot be mistaken for this submission's echo.
        return snapshot.transcriptMessageIdentities.filter { own.contains($0.messageId) }.map(\.id)
    }
}
