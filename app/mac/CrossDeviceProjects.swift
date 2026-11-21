import Foundation
import SwiftUI

/// Controller-owned organization. Peer IDs are paired cryptographic identities,
/// never names, addresses or runtime-local paths.
struct CrossDeviceProject: Codable, Identifiable, Equatable {
    var id = UUID()
    var name: String
    var folders: [Folder]
    mutating func setPrimaryFolder(_ id: UUID) {
        guard let index = folders.firstIndex(where: { $0.id == id }),
              let first = folders.firstIndex(where: { $0.peerID == folders[index].peerID }), first != index else { return }
        let folder = folders.remove(at: index)
        folders.insert(folder, at: first)
    }
    struct Folder: Codable, Equatable, Identifiable {
        var id = UUID()
        var peerID: String?
        var path: String
        func contains(_ cwd: String) -> Bool {
            func normalize(_ s: String) -> String {
                let p = s.replacingOccurrences(of: "\\", with: "/")
                var value = p
                while value.count > 1 && value.hasSuffix("/") { value.removeLast() }
                return p.dropFirst().first == ":" || p.hasPrefix("//") ? value.lowercased() : value
            }
            let root = normalize(path), candidate = normalize(cwd)
            return candidate == root || candidate.hasPrefix(root == "/" ? root : root + "/")
        }
    }
}

@MainActor final class CrossDeviceProjectCatalog: ObservableObject {
    // Windows sharing a controller state directory observe one catalog.
    // Preview/check catalogs remain explicit and isolated.
    private static var controllers: [String: CrossDeviceProjectCatalog] = [:]
    static func controller(home: URL?) -> CrossDeviceProjectCatalog {
        guard let home else { return CrossDeviceProjectCatalog(home: nil) }
        let key = home.standardizedFileURL.path
        if let existing = controllers[key] { return existing }
        let value = CrossDeviceProjectCatalog(home: home)
        controllers[key] = value
        return value
    }
    private struct Document: Codable { var schemaVersion = 1; var projects: [CrossDeviceProject] }
    @Published private(set) var projects: [CrossDeviceProject] = []
    @Published private(set) var failure: String?
    private let file: URL?
    private var writable = true
    private var originalBytes: Data?
    private(set) var revision: UInt64 = 0
    init(home: URL?) {
        file = home?.appendingPathComponent("cross-device-projects.json")
        reloadIfChanged()
    }
    /// Reopening an editor can adopt a valid external change, without replacing
    /// the in-progress drafts of another editor.
    func reloadIfChanged() {
        guard let file else { return }
        do {
            let bytes = FileManager.default.fileExists(atPath: file.path) ? try Data(contentsOf: file) : nil
            guard bytes != originalBytes || !writable else { return }
            guard let bytes else {
                originalBytes = nil; projects = []; writable = true; failure = nil; revision += 1; return
            }
            guard let raw = try JSONSerialization.jsonObject(with: bytes) as? [String: Any], Set(raw.keys).isSubset(of: ["schemaVersion", "projects"]),
                  let rows = raw["projects"] as? [[String: Any]], rows.allSatisfy({ row in
                      Set(row.keys).isSubset(of: ["id", "name", "folders"]) && (row["folders"] as? [[String: Any]])?.allSatisfy { Set($0.keys).isSubset(of: ["id", "peerID", "path"]) } == true
                  }) else { throw CocoaError(.coderReadCorrupt) }
            let value = try JSONDecoder().decode(Document.self, from: bytes)
            guard value.schemaVersion == 1, Self.validationFailure(value.projects) == nil else { throw CocoaError(.coderReadCorrupt) }
            originalBytes = bytes; projects = value.projects; writable = true; failure = nil; revision += 1
        } catch { writable = false; failure = "跨设备项目无法读取，原文件已保留：\(error.localizedDescription)" }
    }
    private static func validationFailure(_ values: [CrossDeviceProject]) -> String? {
        guard Set(values.map(\.id)).count == values.count else { return "项目身份重复。" }
        for project in values {
            guard Set(project.folders.map(\.id)).count == project.folders.count else { return "目录身份重复。" }
            for (index, folder) in project.folders.enumerated() {
                if folder.path.contains("\0") || project.folders.dropFirst(index + 1).contains(where: { $0.peerID == folder.peerID && $0.contains(folder.path) && folder.contains($0.path) }) { return "同一设备的目录不能重复。" }
            }
        }
        guard values.allSatisfy({ !$0.name.trimmingCharacters(in: .whitespacesAndNewlines).isEmpty && !$0.folders.isEmpty && $0.folders.allSatisfy { ($0.path.hasPrefix("/") || $0.path.hasPrefix("\\\\") || ($0.path.count > 2 && $0.path.dropFirst().first == ":" && ["/", "\\"].contains(String($0.path.dropFirst(2).first!)))) } }) else {
            return "每个项目需要名称和至少一个绝对设备目录。"
        }
        for (index, project) in values.enumerated() {
            for other in values.dropFirst(index + 1) {
                if project.folders.contains(where: { folder in other.folders.contains { $0.peerID == folder.peerID && ($0.contains(folder.path) || folder.contains($0.path)) } }) {
                    return "不同项目在同一设备的目录不能重叠。"
                }
            }
        }
        return nil
    }
    func save(_ values: [CrossDeviceProject], expectedRevision: UInt64? = nil) -> Bool {
        guard writable else { return false }
        guard expectedRevision == nil || expectedRevision == revision else {
            failure = "项目已在其他窗口中更改，输入仍保留。请重新打开编辑后再保存。"; return false
        }
        if let problem = Self.validationFailure(values) { failure = problem; return false }
        do {
            if let file {
                let currentBytes = FileManager.default.fileExists(atPath: file.path) ? try Data(contentsOf: file) : nil
                guard currentBytes == originalBytes else {
                    failure = "项目文件已由其他程序更改，输入仍保留。请重新打开编辑后再保存。"; return false
                }
                let bytes = try JSONEncoder().encode(Document(projects: values))
                if FileManager.default.fileExists(atPath: file.path) {
                    let backup = file.deletingLastPathComponent().appendingPathComponent("cross-device-projects-backup-\(UUID().uuidString).json")
                    try FileManager.default.copyItem(at: file, to: backup)
                }
                try bytes.write(to: file, options: .atomic)
                originalBytes = bytes
            }
            projects = values; revision += 1; failure = nil; return true
        } catch { failure = "项目未保存，输入仍保留：\(error.localizedDescription)"; return false }
    }
}

struct CrossDeviceProjectsEditor: View {
    @ObservedObject var catalog: CrossDeviceProjectCatalog
    let devices: [AggregatedListModel.Device]
    @Environment(\.dismiss) private var dismiss
    @State private var drafts: [CrossDeviceProject]
    @State private var expectedRevision: UInt64
    init(catalog: CrossDeviceProjectCatalog, devices: [AggregatedListModel.Device]) {
        catalog.reloadIfChanged()
        self.catalog = catalog; self.devices = devices; _drafts = State(initialValue: catalog.projects)
        _expectedRevision = State(initialValue: catalog.revision)
    }
    var body: some View {
        VStack(alignment: .leading, spacing: 12) {
            Text("跨设备项目").font(.headline)
            Text("同一项目可关联多个设备目录。目录只用于会话归组，不改变会话的执行位置。每台设备的首个目录为该设备的主目录。")
                .font(.caption).foregroundStyle(.secondary)
            ScrollView {
                VStack(alignment: .leading, spacing: 16) {
                    ForEach($drafts) { $project in
                        VStack(alignment: .leading, spacing: 8) {
                            HStack { TextField("项目名称", text: $project.name); Button("移除项目") { drafts.removeAll { $0.id == project.id } } }
                            ForEach($project.folders) { $folder in
                                HStack {
                                    Picker("设备", selection: $folder.peerID) {
                                        ForEach(devices) { device in Text(device.name).tag(device.peerID) }
                                        if let peer = folder.peerID, !devices.contains(where: { $0.peerID == peer }) { Text("未连接设备").tag(Optional(peer)) }
                                    }.frame(width: 160)
                                    VStack(alignment: .leading, spacing: 3) {
                                        Text("设备上的绝对目录").font(.caption).foregroundStyle(.secondary)
                                        TextField("例如 /work/project 或 C:\\work\\project", text: $folder.path).accessibilityLabel("设备上的绝对目录")
                                    }
                                    if project.folders.first(where: { $0.peerID == folder.peerID })?.id != folder.id {
                                        Button("设为主目录") { project.setPrimaryFolder(folder.id) }
                                    } else { Text("主目录").font(.caption).foregroundStyle(.secondary) }
                                    Button { project.folders.removeAll { $0.id == folder.id } } label: { Image(systemName: "minus.circle") }.accessibilityLabel("移除设备目录")
                                }
                            }
                            Button("添加设备目录") { project.folders.append(.init(peerID: nil, path: "")) }
                        }
                        Divider()
                    }
                }
            }.frame(minHeight: 180, maxHeight: 400)
            Button("添加项目") { drafts.append(.init(name: "新项目", folders: [.init(peerID: nil, path: "")])) }
            if let failure = catalog.failure { Text(failure).foregroundStyle(.red).textSelection(.enabled) }
            HStack { Spacer(); Button("取消") { dismiss() }; Button("存储") { if catalog.save(drafts, expectedRevision: expectedRevision) { dismiss() } }.keyboardShortcut(.defaultAction) }
        }.padding(20).frame(width: 680)
    }
}
