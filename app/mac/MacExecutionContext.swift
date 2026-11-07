import Foundation
import Darwin

/// The signed bundle declares its purpose. Neither the working directory nor
/// a missing environment variable may turn a development build into a daily app.
struct MacExecutionContext {
    enum Purpose: String, Codable { case development, daily }

    struct Metadata: Codable, Equatable {
        let schemaVersion: Int
        let purpose: Purpose
        let scope: String?
        let bundleIdentifier: String
        let executable: String
    }

    private struct DevelopmentMarker: Codable, Equatable {
        let schemaVersion: Int
        let purpose: String
        let scope: String
    }

    struct Failure: LocalizedError {
        let detail: String
        var errorDescription: String? { "启动隔离校验失败：\(detail)" }
    }

    static let markerName = ".velune-development.json"
    private static let cached: Result<MacExecutionContext, Error> = Result {
        try prepare(info: Bundle.main.infoDictionary ?? [:], executableName: Bundle.main.executableURL?.lastPathComponent,
                    userHome: realUserHome(), configuredHome: ProcessInfo.processInfo.environment["VELUNE_HOME"])
    }

    let metadata: Metadata
    let userHome: URL
    let stateDirectory: URL
    let harnessUserHome: URL?
    var isDevelopment: Bool { metadata.purpose == .development }
    var conversationScheme: String { metadata.scope.map { "velune-dev-\($0)" } ?? "velune" }
    var externalEventSchemes: Set<String> { isDevelopment ? [conversationScheme] : [conversationScheme, "agents"] }

    static func application() throws -> MacExecutionContext { try cached.get() }

    /// Link consumers are reached after the entry point has admitted this app.
    static var admitted: MacExecutionContext {
        do { return try application() }
        catch { failAndExit(error) }
    }

    static func failAndExit(_ error: Error) -> Never {
        FileHandle.standardError.write(Data((error.localizedDescription + "\n").utf8))
        exit(2)
    }

    /// Uses the account record, even when a launcher supplied a synthetic HOME.
    static func realUserHome() throws -> URL {
        guard let record = getpwuid(getuid()), let pointer = record.pointee.pw_dir else {
            throw Failure(detail: "无法确定当前用户的主目录。")
        }
        return URL(fileURLWithPath: String(cString: pointer), isDirectory: true).standardizedFileURL.resolvingSymlinksInPath()
    }

    static func validateMetadata(info: [String: Any], executableName: String?) throws -> Metadata {
        guard let raw = info["VeluneExecutionContext"] as? [String: Any],
              let data = try? JSONSerialization.data(withJSONObject: raw),
              let metadata = try? JSONDecoder().decode(Metadata.self, from: data), metadata.schemaVersion == 1 else {
            throw Failure(detail: "应用缺少有效的 VeluneExecutionContext（schemaVersion 1）；请重新构建完整应用。")
        }
        guard info["CFBundleIdentifier"] as? String == metadata.bundleIdentifier,
              info["CFBundleExecutable"] as? String == metadata.executable,
              executableName == metadata.executable else {
            throw Failure(detail: "应用用途元数据与 bundle 标识或可执行文件不一致。")
        }
        switch metadata.purpose {
        case .development:
            guard let scope = metadata.scope,
                  scope.range(of: "^[0-9a-f]{12}$", options: .regularExpression) != nil,
                  metadata.bundleIdentifier == "local.velune.development.\(scope)", metadata.executable == "VeluneDev" else {
                throw Failure(detail: "开发应用的 scope、bundle 标识或可执行文件无效。")
            }
            let schemes = (info["CFBundleURLTypes"] as? [[String: Any]] ?? [])
                .flatMap { $0["CFBundleURLSchemes"] as? [String] ?? [] }.map { $0.lowercased() }
            guard Set(schemes).isDisjoint(with: ["velune", "agents"]) else {
                throw Failure(detail: "开发应用不能注册日常 velune 或 agents 链接协议。")
            }
        case .daily:
            guard metadata.scope == nil, metadata.bundleIdentifier == "local.velune.prototype", metadata.executable == "Velune" else {
                throw Failure(detail: "日常应用身份无效；不能把未知应用当作日常版启动。")
            }
        }
        return metadata
    }

    /// The injectable inputs keep the filesystem contract testable without
    /// launching AppKit, opening a core, or touching the account's real data.
    static func prepare(info: [String: Any], executableName: String?, userHome: URL, configuredHome: String?) throws -> MacExecutionContext {
        let metadata = try validateMetadata(info: info, executableName: executableName)
        let userHome = userHome.standardizedFileURL.resolvingSymlinksInPath()
        let configured = configuredHome.flatMap { $0.trimmingCharacters(in: .whitespacesAndNewlines).isEmpty ? nil : $0 }
        if let configured, !configured.hasPrefix("/") || configured.contains("\u{0}") {
            throw Failure(detail: "VELUNE_HOME 必须使用绝对目录路径；留空可使用用途对应的默认目录。")
        }
        let daily = userHome.appendingPathComponent(".velune", isDirectory: true).resolvingSymlinksInPath()
        let chosen = configured.map { URL(fileURLWithPath: $0, isDirectory: true) }
            ?? (metadata.scope.map { userHome.appendingPathComponent(".velune-dev/\($0)", isDirectory: true) } ?? daily)
        let home = chosen.standardizedFileURL.resolvingSymlinksInPath()
        if let scope = metadata.scope {
            guard !isWithin(home, directory: daily), !isWithin(daily, directory: home), home != userHome, home.path != "/" else {
                throw Failure(detail: "开发应用不能使用日常数据目录、用户主目录或根目录：\(home.path)")
            }
            try admitDevelopmentDirectory(home, scope: scope)
            let harness = home.appendingPathComponent("harness-user", isDirectory: true)
            try privateDirectory(harness)
            return MacExecutionContext(metadata: metadata, userHome: userHome, stateDirectory: home, harnessUserHome: harness)
        }
        guard (try? FileManager.default.attributesOfItem(atPath: home.appendingPathComponent(markerName).path)) == nil else {
            throw Failure(detail: "日常应用不能使用带开发归属标记的目录：\(home.path)")
        }
        return MacExecutionContext(metadata: metadata, userHome: userHome, stateDirectory: home, harnessUserHome: nil)
    }

    private static func admitDevelopmentDirectory(_ home: URL, scope: String) throws {
        let manager = FileManager.default
        let marker = home.appendingPathComponent(markerName)
        let expected = DevelopmentMarker(schemaVersion: 1, purpose: "development", scope: scope)
        if manager.fileExists(atPath: home.path) {
            try ownedDirectory(home)
            if let attributes = try? manager.attributesOfItem(atPath: marker.path) {
                guard attributes[.type] as? FileAttributeType == .typeRegular,
                      (attributes[.ownerAccountID] as? NSNumber)?.uint32Value == getuid(),
                      let actual = try? JSONDecoder().decode(DevelopmentMarker.self, from: Data(contentsOf: marker)), actual == expected else {
                    throw Failure(detail: "开发目录归属标记无效或属于其它 scope：\(home.path)")
                }
                try manager.setAttributes([.posixPermissions: 0o700], ofItemAtPath: home.path)
                try manager.setAttributes([.posixPermissions: 0o600], ofItemAtPath: marker.path)
                return
            }
            guard try manager.contentsOfDirectory(atPath: home.path).isEmpty else {
                throw Failure(detail: "已有非空目录没有当前开发 scope 的归属标记，拒绝使用：\(home.path)")
            }
        }
        try privateDirectory(home)
        let data = try JSONEncoder().encode(expected)
        // Exclusive creation cannot overwrite another scope's concurrent marker.
        let descriptor = Darwin.open(marker.path, O_WRONLY | O_CREAT | O_EXCL | O_NOFOLLOW, mode_t(0o600))
        guard descriptor >= 0 else { throw Failure(detail: "无法创建开发归属标记：\(marker.path)（\(String(cString: strerror(errno)))）") }
        let handle = FileHandle(fileDescriptor: descriptor, closeOnDealloc: true)
        try handle.write(contentsOf: data)
        try handle.close()
    }

    private static func ownedDirectory(_ directory: URL) throws {
        let attributes = try FileManager.default.attributesOfItem(atPath: directory.path)
        guard attributes[.type] as? FileAttributeType == .typeDirectory,
              (attributes[.ownerAccountID] as? NSNumber)?.uint32Value == getuid() else {
            throw Failure(detail: "目录必须是当前用户拥有的普通目录：\(directory.path)")
        }
    }

    private static func privateDirectory(_ directory: URL) throws {
        let manager = FileManager.default
        try manager.createDirectory(at: directory, withIntermediateDirectories: true, attributes: [.posixPermissions: 0o700])
        try ownedDirectory(directory)
        try manager.setAttributes([.posixPermissions: 0o700], ofItemAtPath: directory.path)
    }

    private static func isWithin(_ url: URL, directory: URL) -> Bool {
        url.path == directory.path || url.path.hasPrefix(directory.path + "/")
    }

    var runtimeDiscovery: (userHome: String, overrides: [String: String]) {
        if let harnessUserHome { return (harnessUserHome.path, [:]) }
        let environment = ProcessInfo.processInfo.environment
        let keys = ["PI_CODING_AGENT_DIR", "CODEX_HOME", "CLAUDE_CONFIG_DIR", "DSH_HOME"]
        return (userHome.path, Dictionary(uniqueKeysWithValues: keys.compactMap { key in environment[key].map { (key, $0) } }))
    }

    /// Native agent directories remain explicit as well as HOME. This isolates
    /// filesystem configuration; it does not isolate the login user's Keychain.
    func activateDevelopmentEnvironment() throws {
        guard let harnessUserHome else { return }
        let home = harnessUserHome.path
        let values = ["HOME": home, "VELUNE_HOME": stateDirectory.path,
                      "PI_CODING_AGENT_DIR": home + "/.pi/agent", "CODEX_HOME": home + "/.codex",
                      "CLAUDE_CONFIG_DIR": home + "/.claude", "DSH_HOME": home + "/.dsh",
                      "XDG_CONFIG_HOME": home + "/.config", "XDG_CACHE_HOME": home + "/.cache", "XDG_DATA_HOME": home + "/.local/share"]
        for (key, value) in values where setenv(key, value, 1) != 0 {
            throw Self.Failure(detail: "无法设置开发运行环境：\(key)")
        }
        // Login-shell executable discovery must not source a supplied daily
        // startup-file directory or shell-specific startup file override.
        for key in ["ZDOTDIR", "ENV", "BASH_ENV"] { unsetenv(key) }
    }

    /// Older hidden checks deliberately reopen nonempty synthetic fixtures.
    /// They keep their explicit Transport initializers, within a temporary tree.
    func validateCheckDirectories(environment: [String: String]) throws {
        let temporaryRoots = [URL(fileURLWithPath: "/tmp", isDirectory: true), FileManager.default.temporaryDirectory]
            .map { $0.standardizedFileURL.resolvingSymlinksInPath() }
        let daily = userHome.appendingPathComponent(".velune", isDirectory: true).resolvingSymlinksInPath()
        for key in ["VELUNE_COMPOSER_CHECK_HOME", "VELUNE_COMPOSER_NATIVE_HOME", "VELUNE_EXPERIENCE_CHECK_HOME", "VELUNE_CHECK_OUTPUT"] {
            guard let value = environment[key] else { continue }
            let directory = URL(fileURLWithPath: value, isDirectory: true).standardizedFileURL.resolvingSymlinksInPath()
            guard value.hasPrefix("/"), !value.contains("\u{0}"),
                  temporaryRoots.contains(where: { Self.isWithin(directory, directory: $0) && directory != $0 }),
                  !Self.isWithin(directory, directory: daily) else {
                throw Self.Failure(detail: "\(key) 必须是系统临时目录下的专用合成目录：\(value)")
            }
            if FileManager.default.fileExists(atPath: directory.path) { try Self.ownedDirectory(directory) }
        }
    }
    /// Development checks and previews must not run under the daily app identity.
    func validateDevelopmentChecks(_ arguments: [String]) throws {
        let prefixes = ["--check-", "--preview", "--experience-", "--composer-check-"]
        if !isDevelopment, let argument = arguments.dropFirst().first(where: { argument in
            prefixes.contains(where: { argument.hasPrefix($0) })
        }) {
            throw Self.Failure(detail: "日常应用不能执行开发检查或预览（\(argument)）；请使用开发产物。")
        }
    }
}
