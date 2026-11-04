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
        let knownQueueIDs: Set<String>
        var accepted = false
    }
    private var submissions: [Submission] = []
    private var claimedUserIDs = Set<String>()
    let changes = PassthroughSubject<Void, Never>()
    var displayed: [SubmittedMessage] {
        submissions.map { submission in
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
    @discardableResult func begin(_ route: Route, text: String, attachments: [String], snapshot: ConversationSnapshot?) -> String {
        let id = "submission:" + UUID().uuidString.lowercased()
        let images: Set<String> = ["png", "jpg", "jpeg", "gif", "webp"]
        let blocks: [DeliveredBlock] = (text.isEmpty ? [] : [.text(text: text)]) + attachments.map {
            images.contains(URL(fileURLWithPath: $0).pathExtension.lowercased()) ? .image(mediaType: nil, source: .path(path: $0)) : .file(path: $0)
        }
        let message = DeliveredMessage(id: id, role: .user, timestampUnixMs: Int64(Date().timeIntervalSince1970 * 1000), blocks: blocks)
        submissions.append(Submission(id: id, route: route, message: message, knownUserIDs: Set(userIDs(snapshot)), knownQueueIDs: Set(snapshot?.followUp.queue.map(\.id) ?? [])))
        changes.send()
        return id
    }
    func accept(_ id: String, snapshot: ConversationSnapshot?) {
        guard let index = submissions.firstIndex(where: { $0.id == id }) else { return }
        if submissions[index].route == .queue {
            // The accepted queue is already visible in the conversation detail.
            if snapshot?.followUp.queue.contains(where: { !submissions[index].knownQueueIDs.contains($0.id) }) == true {
                submissions.remove(at: index); changes.send(); return
            }
            // The running turn may have ended before the queue request arrived.
            submissions[index].route = .send
        }
        submissions[index].accepted = true
        reconcile(snapshot)
        changes.send()
    }
    func fail(_ id: String) {
        submissions.removeAll { $0.id == id }
        changes.send()
    }
    func reset() { submissions.removeAll(); claimedUserIDs.removeAll(); changes.send() }
    func reconcile(_ snapshot: ConversationSnapshot?) {
        guard let snapshot, !submissions.isEmpty else { return }
        let ids = userIDs(snapshot)
        claimedUserIDs.formIntersection(ids)
        let oldCount = submissions.count
        submissions.removeAll { submission in
            guard submission.accepted else { return false }
            if let own = ids.first(where: { !submission.knownUserIDs.contains($0) && !claimedUserIDs.contains($0) }) {
                claimedUserIDs.insert(own); return true
            }
            return snapshot.runState != .running && snapshot.runState != .stopping
        }
        if oldCount != submissions.count { changes.send() }
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
