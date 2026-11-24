import Combine
import Foundation
import VeluneBindings

/// Tracks queue admission across receipts and projections. Only the core's
/// transcript projection supplies message rows.
@MainActor final class OutgoingSubmissions {
    private struct Submission {
        let id: String
        let text: String
        let attachments: [String]
        var pendingQueueID: String?
        var queueObserved = false
        var visible = true
    }
    private var submissions: [Submission] = []
    let changes = PassthroughSubject<Void, Never>()
    var queued: [QueuedMessage] {
        submissions.compactMap { submission in
            guard let queueID = submission.pendingQueueID else { return nil }
            return QueuedMessage(id: queueID, submissionId: submission.id, text: submission.text, attachments: submission.attachments)
        }
    }

    func begin(text: String, attachments: [String]) -> String {
        let id = "submission:" + UUID().uuidString.lowercased()
        submissions.append(Submission(id: id, text: text, attachments: attachments))
        return id
    }

    /// Even an ordinary send can be admitted to the queue if the core became
    /// busy. Only a receipt with an exact queue ID can fill a missing projection.
    func accept(_ id: String, receipt: SubmissionReceipt?, hasSnapshot: Bool) {
        guard let index = submissions.firstIndex(where: { $0.id == id }) else { return }
        if case .queued(let queueID) = receipt, !hasSnapshot, !submissions[index].queueObserved, submissions[index].visible {
            submissions[index].pendingQueueID = queueID
            changes.send()
        } else {
            submissions.remove(at: index)
        }
    }

    /// A poll can confirm admission before the request reports its result.
    func fail(_ id: String) -> Bool {
        guard let index = submissions.firstIndex(where: { $0.id == id }) else { return true }
        return !submissions.remove(at: index).queueObserved
    }

    func reset() {
        retireQueueReceipts()
        // Keep in-flight admission evidence so a late failure after navigation
        // still settles the original captured composer input.
        for index in submissions.indices { submissions[index].visible = false }
    }

    func reconcile(_ queue: [QueuedMessage]) {
        guard !submissions.isEmpty else { return }
        let observed = Set(queue.compactMap(\.submissionId))
        for index in submissions.indices where observed.contains(submissions[index].id) {
            submissions[index].queueObserved = true
        }
        // A complete projection either owns the queue row or has already
        // observed its dispatch or removal.
        retireQueueReceipts()
    }

    private func retireQueueReceipts() {
        let oldCount = submissions.count
        submissions.removeAll { $0.pendingQueueID != nil }
        if submissions.count != oldCount { changes.send() }
    }

    static func queuedCancellation(_ receipt: SubmissionReceipt?) -> SendQueueOperation? {
        if case .queued(let id) = receipt { return .remove(id: id) }
        return nil
    }
}
