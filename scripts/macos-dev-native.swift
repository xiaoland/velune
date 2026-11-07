import AppKit
import Darwin
import Foundation

// This helper only controls the exact development bundle supplied by the
// launcher. It neither searches by process name nor falls back to force quit.
struct ControlFailure: LocalizedError {
    let detail: String
    var errorDescription: String? { detail }
}

func refuse(_ message: String) -> ControlFailure { ControlFailure(detail: message) }

func canonical(_ url: URL) -> URL { url.standardizedFileURL.resolvingSymlinksInPath() }

func absoluteURL(_ path: String) throws -> URL {
    guard path.hasPrefix("/"), !path.contains("\u{0}") else {
        throw refuse("必须提供不含 NUL 的绝对路径。")
    }
    return canonical(URL(fileURLWithPath: path))
}

func isWithin(_ url: URL, directory: URL) -> Bool {
    url.path == directory.path || url.path.hasPrefix(directory.path + "/")
}

struct DevelopmentBundle {
    private struct Context: Decodable {
        let schemaVersion: Int
        let purpose: String
        let scope: String
        let bundleIdentifier: String
        let executable: String
    }

    let url: URL
    let executableURL: URL
    let identifier: String
    let scope: String

    init(path: String) throws {
        let url = try absoluteURL(path)
        guard url.pathExtension == "app", let bundle = Bundle(url: url),
              let data = try? Data(contentsOf: url.appendingPathComponent("Contents/Info.plist")),
              let info = try? PropertyListSerialization.propertyList(from: data, format: nil) as? [String: Any],
              let raw = info["VeluneExecutionContext"] as? [String: Any],
              let contextData = try? JSONSerialization.data(withJSONObject: raw),
              let context = try? JSONDecoder().decode(Context.self, from: contextData),
              context.schemaVersion == 1, context.purpose == "development",
              context.scope.range(of: "^[0-9a-f]{12}$", options: .regularExpression) != nil,
              context.bundleIdentifier == "local.velune.development.\(context.scope)",
              context.executable == "VeluneDev",
              info["CFBundleIdentifier"] as? String == context.bundleIdentifier,
              info["CFBundleExecutable"] as? String == context.executable,
              let bundleExecutable = bundle.executableURL else {
            throw refuse("应用缺少一致的开发用途身份（schemaVersion 1、scope、bundle 标识与 VeluneDev）。")
        }
        let executable = url.appendingPathComponent("Contents/MacOS/VeluneDev").standardizedFileURL
        guard canonical(bundleExecutable) == executable, canonical(executable) == executable,
              let attributes = try? FileManager.default.attributesOfItem(atPath: executable.path),
              attributes[.type] as? FileAttributeType == .typeRegular,
              FileManager.default.isExecutableFile(atPath: executable.path) else {
            throw refuse("开发应用的真实可执行文件路径无效。")
        }
        self.url = url
        executableURL = executable
        identifier = context.bundleIdentifier
        scope = context.scope
    }
}

struct ApplicationMetadata: Encodable {
    let pid: pid_t
    let bundlePath: String
    let executablePath: String
    let bundleIdentifier: String
    let launchedAt: Double
    let finishedLaunching: Bool
    let hidden: Bool

    init(_ application: NSRunningApplication) throws {
        guard !application.isTerminated, application.processIdentifier > 0,
              let bundleURL = application.bundleURL, let executableURL = application.executableURL,
              let identifier = application.bundleIdentifier, let date = application.launchDate,
              date.timeIntervalSince1970.isFinite, date.timeIntervalSince1970 > 0 else {
            throw refuse("候选应用身份或出生时间不完整；拒绝复用或退出。")
        }
        pid = application.processIdentifier
        bundlePath = canonical(bundleURL).path
        executablePath = canonical(executableURL).path
        bundleIdentifier = identifier
        launchedAt = date.timeIntervalSince1970
        finishedLaunching = application.isFinishedLaunching
        hidden = application.isHidden
    }

    func matches(_ bundle: DevelopmentBundle) -> Bool {
        bundlePath == bundle.url.path && executablePath == bundle.executableURL.path && bundleIdentifier == bundle.identifier
    }

    func hasBirth(_ expected: Double) -> Bool { abs(launchedAt - expected) <= 0.000_001 }
}

struct Inspection: Encodable {
    var applications: [ApplicationMetadata] = []
    var sameIdentityApplications: [ApplicationMetadata] = []
}

func inspect(_ bundle: DevelopmentBundle) throws -> Inspection {
    var result = Inspection()
    // Include exact-path apps even if their reported identifier is inconsistent.
    // Reading this list does not inspect windows, titles, or application content.
    for application in NSWorkspace.shared.runningApplications where !application.isTerminated {
        let exactPath = application.bundleURL.map { canonical($0) == bundle.url } ?? false
        guard exactPath || application.bundleIdentifier == bundle.identifier else { continue }
        let metadata = try ApplicationMetadata(application)
        if exactPath {
            guard metadata.matches(bundle) else { throw refuse("准确 bundle 路径已有身份矛盾的应用；拒绝操作。") }
            result.applications.append(metadata)
        } else {
            result.sameIdentityApplications.append(metadata)
        }
    }
    result.applications.sort { $0.pid < $1.pid }
    result.sameIdentityApplications.sort { $0.pid < $1.pid }
    return result
}

// The launcher owns the environment allowlist. Only its private prepared map is
// read here; the helper never merges in its own shell environment.
func privateData(path: String) throws -> Data {
    guard path.hasPrefix("/"), !path.contains("\u{0}") else { throw refuse("私有 JSON 文件必须使用绝对路径。") }
    let descriptor = Darwin.open(path, O_RDONLY | O_NOFOLLOW | O_CLOEXEC)
    guard descriptor >= 0 else { throw refuse("无法打开私有 JSON 文件。") }
    let handle = FileHandle(fileDescriptor: descriptor, closeOnDealloc: true)
    defer { try? handle.close() }
    var attributes = stat()
    guard fstat(descriptor, &attributes) == 0, attributes.st_mode & S_IFMT == S_IFREG,
          attributes.st_uid == getuid(), attributes.st_mode & 0o077 == 0,
          attributes.st_size >= 0, attributes.st_size <= 65_536 else {
        throw refuse("JSON 文件必须是当前用户拥有、无组或其它用户权限的普通小文件。")
    }
    return try handle.readToEnd() ?? Data()
}

func preparedEnvironment(path: String, bundle: DevelopmentBundle) throws -> [String: String] {
    let data = try privateData(path: path)
    guard let environment = try? JSONDecoder().decode([String: String].self, from: data),
          environment.allSatisfy({ !$0.key.isEmpty && !$0.key.contains("=") && !$0.key.contains("\u{0}") && !$0.value.contains("\u{0}") }),
          let homePath = environment["VELUNE_HOME"] else {
        throw refuse("启动环境必须是无 NUL 的字符串映射，并提供 VELUNE_HOME。")
    }
    let home = try absoluteURL(homePath)
    guard let userRecord = getpwuid(getuid()), let userPath = userRecord.pointee.pw_dir else {
        throw refuse("无法核对当前用户的数据目录边界。")
    }
    let userHome = canonical(URL(fileURLWithPath: String(cString: userPath)))
    let dailyHome = canonical(userHome.appendingPathComponent(".velune"))
    guard home.path != "/", home != userHome, !isWithin(home, directory: dailyHome) else {
        throw refuse("开发启动环境不能指向日常数据目录、用户主目录或根目录。")
    }
    struct Marker: Decodable { let schemaVersion: Int; let purpose: String; let scope: String }
    let markerData = try privateData(path: home.appendingPathComponent(".velune-development.json").path)
    guard let marker = try? JSONDecoder().decode(Marker.self, from: markerData),
          marker.schemaVersion == 1, marker.purpose == "development", marker.scope == bundle.scope else {
        throw refuse("启动环境的数据目录归属与开发 bundle scope 不一致。")
    }
    let harness = canonical(home.appendingPathComponent("harness-user"))
    guard harness != home, isWithin(harness, directory: home) else { throw refuse("原生 Agent HOME 的符号链接越过开发 scope。") }
    let directories = ["HOME": "", "CODEX_HOME": ".codex", "CLAUDE_CONFIG_DIR": ".claude", "PI_CODING_AGENT_DIR": ".pi/agent", "DSH_HOME": ".dsh"]
    for (key, suffix) in directories {
        let expected = canonical(suffix.isEmpty ? harness : harness.appendingPathComponent(suffix))
        guard let path = environment[key], try absoluteURL(path) == expected,
              isWithin(expected, directory: harness) else {
            throw refuse("启动环境 \(key) 不属于当前开发 scope 的原生 Agent 目录。")
        }
    }
    return environment
}

final class OpenCompletion: @unchecked Sendable {
    private let lock = NSLock()
    private var stored: Result<NSRunningApplication, Error>?
    func finish(application: NSRunningApplication?, error: Error?) {
        lock.lock()
        defer { lock.unlock() }
        if let error { stored = .failure(error) }
        else if let application { stored = .success(application) }
        else { stored = .failure(refuse("LaunchServices 未返回应用身份。")) }
    }
    var result: Result<NSRunningApplication, Error>? {
        lock.lock()
        defer { lock.unlock() }
        return stored
    }
}

struct StartResult: Encodable { let application: ApplicationMetadata; let reused: Bool }

func start(_ bundle: DevelopmentBundle, environmentPath: String) throws -> StartResult {
    let before = try inspect(bundle)
    guard before.sameIdentityApplications.isEmpty, before.applications.count <= 1 else {
        throw refuse("存在其它路径的同身份应用或多个准确路径实例；拒绝启动，也不会退出它们。")
    }
    if let application = before.applications.first { return StartResult(application: application, reused: true) }
    let environment = try preparedEnvironment(path: environmentPath, bundle: bundle)
    let configuration = NSWorkspace.OpenConfiguration()
    configuration.activates = false
    configuration.createsNewApplicationInstance = false
    configuration.allowsRunningApplicationSubstitution = false
    configuration.addsToRecentItems = false
    configuration.promptsUserIfNeeded = false
    // AppKit documents that the system may include additional variables. This
    // map supplies controlled values; it is not a complete OS credential sandbox.
    configuration.environment = environment
    let completion = OpenCompletion()
    let deadline = Date().addingTimeInterval(30)
    NSWorkspace.shared.openApplication(at: bundle.url, configuration: configuration) { application, error in
        completion.finish(application: application, error: error)
    }
    while completion.result == nil {
        guard Date() < deadline else { throw refuse("等待 LaunchServices 启动超时；未发送退出或强制终止。") }
        RunLoop.current.run(until: Date().addingTimeInterval(0.05))
    }
    guard let result = completion.result else { throw refuse("LaunchServices 启动结果缺失。") }
    let application: NSRunningApplication
    do { application = try result.get() }
    catch { throw refuse("LaunchServices 未能启动指定开发应用：\(error.localizedDescription)") }
    while !application.isFinishedLaunching {
        guard !application.isTerminated, Date() < deadline else { throw refuse("开发应用未完成启动或已经退出；未发送强制终止。") }
        RunLoop.current.run(until: Date().addingTimeInterval(0.05))
    }
    let metadata = try ApplicationMetadata(application)
    let after = try inspect(bundle)
    guard metadata.matches(bundle), after.sameIdentityApplications.isEmpty,
          after.applications.count == 1, after.applications[0].pid == metadata.pid,
          after.applications[0].hasBirth(metadata.launchedAt) else {
        throw refuse("启动结果与准确开发 bundle 身份不一致或有歧义；未发送退出或强制终止。")
    }
    return StartResult(application: metadata, reused: false)
}

struct StopResult: Encodable { let pid: pid_t; let launchedAt: Double; let alreadyStopped: Bool; let stopped = true }

func processExists(_ pid: pid_t) -> Bool {
    // Signal zero only probes existence; it sends no signal to the process.
    Darwin.kill(pid, 0) == 0 || errno != ESRCH
}

func stop(_ bundle: DevelopmentBundle, pid: pid_t, launchedAt: Double) throws -> StopResult {
    guard let initial = NSRunningApplication(processIdentifier: pid), !initial.isTerminated else {
        guard !processExists(pid) else { throw refuse("PID 仍存在，但无法取得完整应用身份；拒绝退出。") }
        return StopResult(pid: pid, launchedAt: launchedAt, alreadyStopped: true)
    }
    let candidates = try inspect(bundle)
    guard candidates.sameIdentityApplications.isEmpty, candidates.applications.count == 1,
          candidates.applications[0].pid == pid, candidates.applications[0].hasBirth(launchedAt) else {
        throw refuse("停止目标不唯一、路径不符或出生时间已改变；拒绝退出。")
    }
    // Look up the PID again immediately before normal Quit. A stale PID must
    // never be treated as authority to quit its current owner.
    guard let application = NSRunningApplication(processIdentifier: pid), !application.isTerminated else {
        guard !processExists(pid) else { throw refuse("停止前 PID 身份发生变化；拒绝退出。") }
        return StopResult(pid: pid, launchedAt: launchedAt, alreadyStopped: true)
    }
    let metadata = try ApplicationMetadata(application)
    guard metadata.matches(bundle), metadata.hasBirth(launchedAt), application == initial else {
        throw refuse("停止前应用路径、可执行文件、标识或出生时间发生变化；拒绝退出。")
    }
    guard application.terminate() else { throw refuse("开发应用拒绝正常退出；未发送强制终止。") }
    let deadline = Date().addingTimeInterval(30)
    while !application.isTerminated {
        guard Date() < deadline else { throw refuse("等待正常退出超时或退出被取消；未发送强制终止。") }
        RunLoop.current.run(until: Date().addingTimeInterval(0.1))
    }
    return StopResult(pid: pid, launchedAt: launchedAt, alreadyStopped: false)
}

func emit<T: Encodable>(_ value: T) throws {
    let encoder = JSONEncoder()
    encoder.outputFormatting = [.sortedKeys]
    let data = try encoder.encode(value)
    try FileHandle.standardOutput.write(contentsOf: data + Data([0x0A]))
}

do {
    let arguments = Array(CommandLine.arguments.dropFirst())
    guard let command = arguments.first, ["inspect", "start", "stop"].contains(command), arguments.count % 2 == 1 else {
        throw refuse("用法：macos-dev-native inspect|start|stop --bundle <绝对路径>；start 需要 --environment <私有 JSON>；stop 需要 --pid <PID> --launched-at <Unix 秒>。")
    }
    var options: [String: String] = [:]
    for index in stride(from: 1, to: arguments.count, by: 2) {
        guard options[arguments[index]] == nil else { throw refuse("参数不能重复。") }
        options[arguments[index]] = arguments[index + 1]
    }
    let allowed = command == "inspect" ? Set(["--bundle"]) : command == "start" ? Set(["--bundle", "--environment"]) : Set(["--bundle", "--pid", "--launched-at"])
    guard Set(options.keys) == allowed, let path = options["--bundle"] else { throw refuse("参数缺失或含不支持的参数。") }
    let bundle = try DevelopmentBundle(path: path)
    switch command {
    case "inspect": try emit(inspect(bundle))
    case "start": try emit(start(bundle, environmentPath: options["--environment"]!))
    default:
        guard let pid = pid_t(options["--pid"]!), pid > 0,
              let birth = Double(options["--launched-at"]!), birth.isFinite, birth > 0 else {
            throw refuse("停止操作需要正 PID 和有效的 Unix 出生时间。")
        }
        try emit(stop(bundle, pid: pid, launchedAt: birth))
    }
} catch {
    FileHandle.standardError.write(Data((error.localizedDescription + "\n").utf8))
    exit(1)
}
