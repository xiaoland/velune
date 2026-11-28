import Foundation

/// Stores window contents and draft recovery data under the AppKit window's UUID.
final class WindowWorkspaceJournal: @unchecked Sendable {
    private let directory: URL?
    private let report: @Sendable (String) -> Void
    private let queue = DispatchQueue(label: "local.velune.workspace-journal", qos: .utility)
    private let lock = NSLock()
    private var pending: [String: Data] = [:]
    private var scheduled = false
    // Accessed only on queue, alongside file reads and writes.
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
            } catch {
                report(error.localizedDescription)
                continue
            }
            for (id, data) in batch {
                let target = directory.appendingPathComponent(id + ".json")
                guard prepareForWrite(at: target, id: id) else { continue }
                do { try data.write(to: target, options: .atomic) }
                catch { report("无法保存恢复点 \(target.lastPathComponent)：\(error.localizedDescription)") }
            }
        }
    }
    func flush() { queue.sync {} }

    func restore(id: String, recoverLatest: Bool, kind: ConversationWindowKind = .workspace, device: String? = nil, aggregatingDevices: Bool = false) -> SavedWindowLayout? {
        guard let directory else { return nil }
        return queue.sync {
            // Remote transports share this Mac's state directory. A local
            // arrangement must never resolve remote IDs against another core.
            let scope = SavedWindowLayout.ExecutionDevice(device)
            let aggregate = aggregatingDevices && kind == .workspace && device == nil
            // A window's own entry may be a snapshot from before kinds were
            // recorded. Unqualified recovery must never borrow another kind.
            let exact = directory.appendingPathComponent(id + ".json")
            if UUID(uuidString: id) != nil, FileManager.default.fileExists(atPath: exact.path),
               let snapshot = readSnapshot(at: exact, aggregatingDevices: aggregate),
               snapshot.layout.windowDevice == nil || snapshot.layout.windowDevice == scope,
               snapshot.layout.windowKind == nil || snapshot.layout.windowKind == kind {
                return snapshot.layout
            }
            guard recoverLatest, FileManager.default.fileExists(atPath: directory.path) else { return nil }
            do {
                let files = try FileManager.default.contentsOfDirectory(at: directory, includingPropertiesForKeys: [.contentModificationDateKey])
                let snapshots = files.compactMap { url -> (date: Date, layout: SavedWindowLayout)? in
                    let fileID = url.deletingPathExtension().lastPathComponent
                    guard url.pathExtension == "json", fileID != id, UUID(uuidString: fileID) != nil else { return nil }
                    return readSnapshot(at: url, aggregatingDevices: aggregate)
                }.sorted { $0.date < $1.date }
                return snapshots.last { $0.layout.windowDevice == scope && $0.layout.windowKind == kind }?.layout
            } catch { report(error.localizedDescription); return nil }
        }
    }

    private func readSnapshot(at url: URL, aggregatingDevices: Bool) -> (date: Date, layout: SavedWindowLayout)? {
        let id = url.deletingPathExtension().lastPathComponent
        do {
            var layout = try readLayout(at: url, id: id)
            if aggregatingDevices, layout.windowKind != .single, case .remote(_)? = layout.windowDevice {
                try backupLegacy(Data(contentsOf: url), id: id, suffix: "before-aggregation")
                layout = layout.migratingToAggregatedWorkspace()
            }
            let date = try url.resourceValues(forKeys: [.contentModificationDateKey]).contentModificationDate ?? .distantPast
            checkedIDs.insert(id)
            return (date, layout)
        } catch {
            blockedIDs.insert(id)
            report("无法读取恢复点 \(url.lastPathComponent)：\(error.localizedDescription)；原文件保留，后续保存已禁用。升级到支持该格式的版本，或在确认后使用对应的 .bak 副本恢复")
            return nil
        }
    }

    private func prepareForWrite(at target: URL, id: String) -> Bool {
        if blockedIDs.contains(id) { return false }
        if checkedIDs.contains(id) { return true }
        do {
            if FileManager.default.fileExists(atPath: target.path) { _ = try readLayout(at: target, id: id) }
            checkedIDs.insert(id)
            return true
        } catch {
            blockedIDs.insert(id)
            report("无法保存恢复点 \(id).json：\(error.localizedDescription)；原文件保留，恢复该窗口的写入已禁用")
            return false
        }
    }

    private func readLayout(at url: URL, id: String) throws -> SavedWindowLayout {
        let bytes = try Data(contentsOf: url)
        var layout = try SavedWindowLayout.decodeKnown(bytes)
        try upgrade(&layout, from: bytes, at: url, id: id)
        return layout
    }

    /// Known older formats (the pre-beta shape, floating tools) are backed
    /// up, then rewritten as now encoded, keeping the file's date for
    /// latest-window selection.
    private func upgrade(_ layout: inout SavedWindowLayout, from bytes: Data, at url: URL, id: String) throws {
        let unversioned = layout.schemaVersion == 0
        let floating = ToolWindowLayout.migratingFloatingTools(in: bytes) != nil
        guard unversioned || floating else { return }
        let date = try url.resourceValues(forKeys: [.contentModificationDateKey]).contentModificationDate
        if unversioned { try backupLegacy(bytes, id: id) }
        if floating { try backupLegacy(bytes, id: id, suffix: "before-floating-tools") }
        layout.schemaVersion = 1
        try JSONEncoder().encode(layout).write(to: url, options: .atomic)
        if let date { try FileManager.default.setAttributes([.modificationDate: date], ofItemAtPath: url.path) }
    }

    private func backupLegacy(_ bytes: Data, id: String, suffix: String = "before-beta") throws {
        guard let directory else { return }
        let backup = directory.appendingPathComponent(id + ".json.\(suffix).bak")
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
