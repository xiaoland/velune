import AppKit
import Foundation
import UniformTypeIdentifiers

/// The callback can finish on an ItemProvider worker while the caller blocks
/// the main thread. Protect its result without sending work back to main.
private final class DragProviderLoadResult: @unchecked Sendable {
    let completed = DispatchSemaphore(value: 0)
    private let lock = NSLock()
    private var data: Data?
    private var error: Error?
    private var finished = false

    func finish(_ data: Data?, error: Error?) {
        lock.withLock { self.data = data; self.error = error; finished = true }
        completed.signal()
    }

    func value() -> (Data?, Error?) { lock.withLock { (data, error) } }
    var isFinished: Bool { lock.withLock { finished } }
}

/// Reproduces termination's synchronous demand for a promised drag payload.
/// Uses no scene, pasteboard, runtime or global input. Production loads never
/// pump the main loop; the deliberately broken control is drained separately.
@MainActor func dragProviderChecks() -> Never {
    precondition(Thread.isMainThread && NSApp.activationPolicy() == .prohibited)
    func note(_ text: String) { FileHandle.standardError.write(Data((text + "\n").utf8)) }
    func load(_ provider: NSItemProvider, type: UTType) -> Data {
        precondition(provider.registeredTypeIdentifiers == [type.identifier])
        let result = DragProviderLoadResult()
        _ = provider.loadDataRepresentation(forTypeIdentifier: type.identifier) { data, error in
            result.finish(data, error: error)
        }
        // Do not pump main: a provider that schedules completion there must
        // fail this bound, just as it deadlocks SwiftUI's synchronous demand.
        precondition(result.completed.wait(timeout: .now() + 2) == .success,
                     "拖放数据等待主线程才能完成：\(type.identifier)")
        let (data, error) = result.value()
        precondition(error == nil, "拖放数据加载失败：\(String(describing: error))")
        guard let data else { preconditionFailure("拖放数据未返回内容：\(type.identifier)") }
        return data
    }

    let oldType = UTType(exportedAs: "local.velune.drag-provider-main-queue-check")
    let oldProvider = NSItemProvider()
    oldProvider.registerDataRepresentation(forTypeIdentifier: oldType.identifier, visibility: .ownProcess) { done in
        DispatchQueue.main.async { done(Data(), nil) }
        return nil
    }
    let oldResult = DragProviderLoadResult()
    _ = oldProvider.loadDataRepresentation(forTypeIdentifier: oldType.identifier) { data, error in oldResult.finish(data, error: error) }
    precondition(oldResult.completed.wait(timeout: .now() + 0.2) == .timedOut,
                 "负对照必须识别依赖主线程的拖放数据回调")
    // Release this deliberate negative control before testing the production
    // providers. Only the negative control is allowed to pump the main loop.
    let deadline = Date().addingTimeInterval(2)
    while !oldResult.isFinished, Date() < deadline { RunLoop.main.run(until: Date().addingTimeInterval(0.01)) }
    precondition(oldResult.isFinished && oldResult.value().0 == Data())
    note("PASS negative control detects a main-queue drag callback during the bounded wait, then drains it without leaving a pending worker")

    let runtime = "synthetic runtime"
    let conversation = "synthetic &/id?中文"
    let expected = ConversationLink.url(runtimeID: runtime, conversationID: conversation)
    let data = load(ConversationLink.itemProvider(runtimeID: runtime, conversationID: conversation), type: ConversationLink.dragType)
    precondition(URL(dataRepresentation: data, relativeTo: nil) == expected)
    let decoded = ConversationLink.conversation(in: expected)
    precondition(decoded?.runtimeID == runtime && decoded?.conversationID == conversation)
    note("PASS conversation drag provider completes during a bounded main-thread wait and preserves the exact conversation URL")

    let groupData = load(ConversationLink.groupItemProvider(), type: ConversationLink.groupDragType)
    precondition(groupData == Data())
    precondition(!NSApp.isActive && NSApp.activationPolicy() == .prohibited)
    note("PASS group drag provider completes during a bounded main-thread wait and returns its empty payload; no pasteboard or UI was used")
    exit(0)
}
