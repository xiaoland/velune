import Foundation

/// Crash recovery for unfinished editing uses the same typed window snapshot
/// as scene restoration, but does not depend on macOS committing scene storage.
final class WindowWorkspaceJournal: @unchecked Sendable {
    private let directory: URL?
    private let report: @Sendable (String) -> Void
    private let queue = DispatchQueue(label: "local.velune.workspace-journal", qos: .utility)
    private let lock = NSLock()
    private var pending: [String: Data] = [:]
    private var scheduled = false

    init(directory: URL?, report: @escaping @Sendable (String) -> Void) {
        self.directory = directory?.appendingPathComponent("mac-window-workspaces", isDirectory: true)
        self.report = report
    }
    func save(_ layout: SavedWindowLayout, id: String) {
        guard directory != nil, UUID(uuidString: id) != nil else { return }
        do {
            let data = try JSONEncoder().encode(layout)
            let start = lock.withLock {
                pending[id] = data
                if scheduled { return false }
                scheduled = true
                return true
            }
            if start { queue.async { [self] in drain() } }
        } catch { report(error.localizedDescription) }
    }
    private func drain() {
        guard let directory else { return }
        while true {
            let batch = lock.withLock {
                let batch = pending
                pending.removeAll()
                if batch.isEmpty { scheduled = false }
                return batch
            }
            if batch.isEmpty { return }
            do {
                try FileManager.default.createDirectory(at: directory, withIntermediateDirectories: true, attributes: [.posixPermissions: 0o700])
                for (id, data) in batch { try data.write(to: directory.appendingPathComponent(id + ".json"), options: .atomic) }
            } catch { report(error.localizedDescription) }
        }
    }
    func flush() { queue.sync {} }

    func restore(id: String, recoverLatest: Bool, kind: ConversationWindowKind = .workspace, device: String? = nil) -> SavedWindowLayout? {
        guard let directory else { return nil }
        flush()
        do {
            guard FileManager.default.fileExists(atPath: directory.path) else { return nil }
            let files = try FileManager.default.contentsOfDirectory(at: directory, includingPropertiesForKeys: [.contentModificationDateKey])
                .filter { $0.pathExtension == "json" && UUID(uuidString: $0.deletingPathExtension().lastPathComponent) != nil }
            let snapshots = files.compactMap { url -> (id: String, date: Date, layout: SavedWindowLayout)? in
                do {
                    let layout = try JSONDecoder().decode(SavedWindowLayout.self, from: Data(contentsOf: url))
                    return (url.deletingPathExtension().lastPathComponent, try url.resourceValues(forKeys: [.contentModificationDateKey]).contentModificationDate ?? .distantPast, layout)
                } catch { report("无法读取恢复点 \(url.lastPathComponent)：\(error.localizedDescription)"); return nil }
            }.sorted { $0.date < $1.date }
            // Remote transports share this Mac's state directory. A local
            // arrangement must never resolve remote IDs against another core.
            let scope = SavedWindowLayout.ExecutionDevice(device)
            // An explicit old scene identity may still restore a pre-kind
            // snapshot. Unqualified recovery must never borrow another kind.
            let chosen = snapshots.first { $0.id == id && ($0.layout.windowDevice == nil || $0.layout.windowDevice == scope) && ($0.layout.windowKind == nil || $0.layout.windowKind == kind) }
                ?? (recoverLatest ? snapshots.last { $0.layout.windowDevice == scope && $0.layout.windowKind == kind } : nil)
            // Shared content is authoritative. A window snapshot restores its cursor
            // and any explicitly pending edit; never copy text between input identities.
            guard let layout = chosen?.layout, layout.isWhole else { return nil }
            return layout
        } catch { report(error.localizedDescription); return nil }
    }
}
