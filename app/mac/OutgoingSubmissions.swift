import Combine
import Foundation
import VeluneBindings

/// Presents only this composer's submissions while the Core returns the
/// delivery result and the native transcript catches up.
@MainActor final class OutgoingSubmissions {
    private struct Submission {
        let id: String
        let text: String
        let attachments: [String]
        let showsInTranscript: Bool
        let previousUserRowID: String?
        var receiptSettled = false
        var submittedMessageID: String?
        var echoVisible = false
        var pendingQueueID: String?
        var queueObserved = false
        var visible = true
    }
    private var submissions: [Submission] = []
    let changes = PassthroughSubject<Void, Never>()
    var message: TranscriptRow? {
        guard let submission = submissions.last(where: {
            $0.showsInTranscript && $0.visible && !$0.echoVisible && !$0.queueObserved && $0.pendingQueueID == nil
        }) else { return nil }
        let blocks = (submission.text.isEmpty ? [] : [DeliveredBlock.text(text: submission.text)])
            + submission.attachments.map(Self.attachmentBlock)
        return TranscriptRow(DeliveredMessage(id: submission.id, role: .user, blocks: blocks),
                             id: submission.id, sending: !submission.receiptSettled, copies: false)
    }
    private static func attachmentBlock(_ path: String) -> DeliveredBlock {
        let mediaType: String? = switch URL(fileURLWithPath: path).pathExtension.lowercased() {
        case "png": "image/png"
        case "jpg", "jpeg": "image/jpeg"
        case "gif": "image/gif"
        case "webp": "image/webp"
        default: nil
        }
        return mediaType.map { .image(mediaType: $0, source: .path(path: path)) } ?? .file(path: path)
    }
    var queued: [QueuedMessage] {
        submissions.compactMap { submission in
            guard let queueID = submission.pendingQueueID else { return nil }
            return QueuedMessage(id: queueID, submissionId: submission.id, text: submission.text, attachments: submission.attachments)
        }
    }

    func begin(text: String, attachments: [String], sendingFrom snapshot: ConversationSnapshot? = nil) -> String {
        let id = "submission:" + UUID().uuidString.lowercased()
        submissions.append(Submission(id: id, text: text, attachments: attachments,
                                      showsInTranscript: snapshot != nil, previousUserRowID: snapshot?.transcriptOutline.last?.id))
        if snapshot != nil { changes.send() }
        return id
    }

    /// Even an ordinary send can be admitted to the queue if the core became
    /// busy. Only a receipt with an exact queue ID can fill a missing projection.
    func accept(_ id: String, receipt: SubmissionReceipt?, hasSnapshot: Bool, submittedMessageID: String?) {
        guard let index = submissions.firstIndex(where: { $0.id == id }) else { return }
        switch receipt {
        case .queued(let queueID):
            if !hasSnapshot, !submissions[index].queueObserved, submissions[index].visible {
                submissions[index].pendingQueueID = queueID
            } else {
                submissions.remove(at: index)
            }
        case .sending, nil:
            if submissions[index].showsInTranscript && submissions[index].visible && !submissions[index].queueObserved {
                submissions[index].receiptSettled = true
                submissions[index].submittedMessageID = submittedMessageID
            } else {
                submissions.remove(at: index)
            }
        }
        changes.send()
    }

    /// A poll can confirm queue admission before the request reports its result.
    func fail(_ id: String) -> Bool {
        guard let index = submissions.firstIndex(where: { $0.id == id }) else { return true }
        let submission = submissions.remove(at: index)
        changes.send()
        return !submission.queueObserved
    }

    func reset() {
        submissions.removeAll { $0.pendingQueueID != nil || $0.receiptSettled }
        // Late callbacks still settle the captured input, without a row in the new view.
        for index in submissions.indices { submissions[index].visible = false }
        changes.send()
    }

    func reconcile(_ snapshot: ConversationSnapshot, afterReceipt: Bool = false) {
        guard !submissions.isEmpty else { return }
        let observed = Set(snapshot.followUp.queue.compactMap(\.submissionId))
        var changed = false
        submissions.removeAll { submission in
            // A complete projection either owns the queue row or has already seen its dispatch.
            if !afterReceipt && submission.pendingQueueID != nil {
                changed = true
                return true
            }
            return false
        }
        for index in submissions.indices {
            if observed.contains(submissions[index].id), !submissions[index].queueObserved {
                submissions[index].queueObserved = true
                changed = true
            }
            guard submissions[index].visible, submissions[index].showsInTranscript else { continue }
            let echoVisible = Self.echoVisible(for: submissions[index], in: snapshot)
            if submissions[index].echoVisible != echoVisible {
                submissions[index].echoVisible = echoVisible
                changed = true
            }
        }
        let count = submissions.count
        submissions.removeAll { submission in
            guard submission.receiptSettled else { return false }
            return submission.echoVisible || submission.queueObserved
                || (!afterReceipt && snapshot.runState != .running && snapshot.runState != .stopping)
        }
        if changed || submissions.count != count { changes.send() }
    }

    private static func echoVisible(for submission: Submission, in snapshot: ConversationSnapshot) -> Bool {
        if let id = submission.submittedMessageID {
            let currentID = snapshot.messageIdentityConfirmations.first { $0.previousId == id }?.currentId ?? id
            return snapshot.transcriptOutline.contains { $0.messageId == currentID }
        }
        // Some adapters echo after the response. Only consider the next native
        // user entry past the stable pre-send anchor. This only reconciles local
        // presentation. It does not settle the RPC or remove any native row.
        let next: TranscriptOutlineEntry?
        if let anchor = submission.previousUserRowID {
            guard let index = snapshot.transcriptOutline.firstIndex(where: { $0.id == anchor }) else { return false }
            next = snapshot.transcriptOutline.dropFirst(index + 1).first
        } else {
            next = snapshot.transcriptOutline.first
        }
        guard let next, let message = snapshot.messages.first(where: { $0.id == next.messageId && $0.role == .user }) else { return false }
        guard !message.blocks.contains(where: { if case .agentSender = $0 { return true }; return false }) else { return false }
        let attachments = message.blocks.compactMap { block -> DeliveredBlock? in
            switch block { case .file, .image: block; default: nil }
        }
        let expected = submission.attachments.map(attachmentBlock)
        guard attachments.count == expected.count else { return false }
        // Images may arrive as native payload bytes; file references keep their paths.
        func attachmentIdentity(_ block: DeliveredBlock) -> String? {
            switch block {
            case .file(let path): path
            case .image: "image"
            default: nil
            }
        }
        return message.copiedText.trimmingCharacters(in: .whitespacesAndNewlines) == submission.text
            && attachments.compactMap(attachmentIdentity).sorted() == expected.compactMap(attachmentIdentity).sorted()
    }

    static func queuedCancellation(_ receipt: SubmissionReceipt?) -> SendQueueOperation? {
        if case .queued(let id) = receipt { return .remove(id: id) }
        return nil
    }
}
