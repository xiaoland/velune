import Foundation

/// Admission to a native conversation mutation. Once begun, cancellation
/// cannot promise that the runtime's rename/archive/delete did not happen.
final class ConversationManagementGate: @unchecked Sendable {
    private enum State { case waiting, running, cancelled }
    private let lock = NSLock()
    private var state = State.waiting

    func begin() -> Bool {
        lock.withLock {
            guard state == .waiting else { return false }
            state = .running
            return true
        }
    }

    func cancel() -> Bool {
        lock.withLock {
            guard state == .waiting else { return false }
            state = .cancelled
            return true
        }
    }
}
