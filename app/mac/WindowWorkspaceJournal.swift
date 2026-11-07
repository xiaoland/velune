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
    private var blockedIDs: Set<String> = []
    private var checkedIDs: Set<String> = []

    init(directory: URL?, report: @escaping @Sendable (String) -> Void) {
        self.directory = directory?.appendingPathComponent("mac-window-workspaces", isDirectory: true)
        self.report = report
    }
    func save(_ layout: SavedWindowLayout, id: String) {
        guard directory != nil, UUID(uuidString: id) != nil else { return }
        do {
            var layout = layout
            layout.schemaVersion = 1
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
                for (id, data) in batch {
                    guard prepareForWrite(id: id) else { continue }
                    try data.write(to: directory.appendingPathComponent(id + ".json"), options: .atomic)
                }
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
                let fileID = url.deletingPathExtension().lastPathComponent
                do {
                    var layout = try SavedWindowLayout.decodeKnown(Data(contentsOf: url))
                    guard layout.isWhole else {
                        block(fileID)
                        report("恢复点 \(url.lastPathComponent) 的窗口布局不完整；原文件保留，后续保存已禁用")
                        return nil
                    }
                    if layout.schemaVersion == 0 {
                        let bytes = try Data(contentsOf: url)
                        let date = try url.resourceValues(forKeys: [.contentModificationDateKey]).contentModificationDate
                        try backupLegacy(bytes, id: fileID)
                        layout.schemaVersion = 1
                        try JSONEncoder().encode(layout).write(to: url, options: .atomic)
                        if let date { try FileManager.default.setAttributes([.modificationDate: date], ofItemAtPath: url.path) }
                    }
                    lock.withLock { _ = checkedIDs.insert(fileID) }
                    return (fileID, try url.resourceValues(forKeys: [.contentModificationDateKey]).contentModificationDate ?? .distantPast, layout)
                } catch {
                    block(url.deletingPathExtension().lastPathComponent)
                    report("无法读取恢复点 \(url.lastPathComponent)：\(error.localizedDescription)；原文件保留，后续保存已禁用。升级到支持该格式的版本，或在确认后恢复 \(url.lastPathComponent).before-beta.bak")
                    return nil
                }
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
            return chosen?.layout
        } catch { report(error.localizedDescription); return nil }
    }

    private func block(_ id: String) {
        lock.withLock { _ = blockedIDs.insert(id) }
    }

    private func prepareForWrite(id: String) -> Bool {
        guard let directory else { return false }
        let state = lock.withLock { (blockedIDs.contains(id), checkedIDs.contains(id)) }
        if state.0 { return false }
        if state.1 { return true }
        let target = directory.appendingPathComponent(id + ".json")
        guard FileManager.default.fileExists(atPath: target.path) else {
            lock.withLock { _ = checkedIDs.insert(id) }
            return true
        }
        do {
            let bytes = try Data(contentsOf: target)
            var layout = try SavedWindowLayout.decodeKnown(bytes)
            guard layout.isWhole else { throw NSError(domain: "VelunePersistence", code: 23, userInfo: [NSLocalizedDescriptionKey: "恢复点布局不完整"]) }
            if layout.schemaVersion == 0 {
                let date = try target.resourceValues(forKeys: [.contentModificationDateKey]).contentModificationDate
                try backupLegacy(bytes, id: id)
                layout.schemaVersion = 1
                try JSONEncoder().encode(layout).write(to: target, options: .atomic)
                if let date { try FileManager.default.setAttributes([.modificationDate: date], ofItemAtPath: target.path) }
            }
            lock.withLock { _ = checkedIDs.insert(id) }
            return true
        } catch {
            block(id)
            report("无法保存恢复点 \(id).json：\(error.localizedDescription)；原文件保留，恢复该窗口的写入已禁用")
            return false
        }
    }

    private func backupLegacy(_ bytes: Data, id: String) throws {
        guard let directory else { return }
        let backup = directory.appendingPathComponent(id + ".json.before-beta.bak")
        if FileManager.default.fileExists(atPath: backup.path) {
            guard try Data(contentsOf: backup) == bytes else {
                throw NSError(domain: "VelunePersistence", code: 24, userInfo: [NSLocalizedDescriptionKey: "恢复点备份与原文件不一致"])
            }
            return
        }
        try bytes.write(to: backup, options: .atomic)
        try FileManager.default.setAttributes([.posixPermissions: 0o600], ofItemAtPath: backup.path)
    }
}
