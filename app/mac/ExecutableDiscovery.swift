import Foundation
import Darwin

/// Discovers a descriptor-declared executable without reading credentials or
/// inheriting the app's possibly incomplete PATH.
enum MacExecutableDiscovery {
    enum Failure: LocalizedError {
        case notFound(minimumVersion: String)

        var errorDescription: String? {
            switch self {
            case .notFound(let version): return "未找到满足 \(version)+ 要求的可执行文件。可手动选择路径。"
            }
        }
    }

    private struct Version: Comparable {
        let major: Int
        let minor: Int
        let patch: Int
        static func < (lhs: Version, rhs: Version) -> Bool {
            if lhs.major != rhs.major { return lhs.major < rhs.major }
            if lhs.minor != rhs.minor { return lhs.minor < rhs.minor }
            return lhs.patch < rhs.patch
        }
    }

    static func discover(_ descriptor: ExecutableDiscoverySpec,
                         completion: @escaping (Result<String, Error>) -> Void) {
        DispatchQueue.global(qos: .userInitiated).async {
            let candidates = candidatePaths(command: descriptor.command)
            for path in candidates where isExecutable(path) {
                if isCompatible(path, minimumVersion: descriptor.minimumVersion) {
                    DispatchQueue.main.async { completion(.success(path)) }
                    return
                }
            }
            DispatchQueue.main.async {
                completion(.failure(Failure.notFound(minimumVersion: descriptor.minimumVersion)))
            }
        }
    }

    /// Returns installed executable paths without accepting a runtime version.
    /// The application adapter checks its exact supported variant afterwards.
    static func candidates(command: String, completion: @escaping ([String]) -> Void) {
        DispatchQueue.global(qos: .userInitiated).async {
            let paths = candidatePaths(command: command).filter(isExecutable)
            DispatchQueue.main.async { completion(paths) }
        }
    }

    private static func candidatePaths(command: String) -> [String] {
        guard command.range(of: #"^[A-Za-z0-9._-]+$"#, options: .regularExpression) != nil else { return [] }
        var paths = ["/usr/local/bin/\(command)", "/opt/homebrew/bin/\(command)", "/usr/bin/\(command)"]
        let shell = Process()
        let output = Pipe()
        shell.executableURL = URL(fileURLWithPath: loginShell())
        shell.arguments = ["-ilc", "command -v \(shellQuote(command))"]
        shell.standardOutput = output
        shell.standardError = FileHandle.nullDevice
        if (try? shell.run()) != nil {
            if wait(for: shell, timeout: 2), shell.terminationStatus == 0,
               let value = String(data: output.fileHandleForReading.readDataToEndOfFile(), encoding: .utf8)?
                    .split(whereSeparator: { $0 == "\n" || $0 == "\r" }).first,
               value.first == "/" {
                paths.insert(String(value), at: 0)
            }
        }
        var unique: [String] = []
        for path in paths where !unique.contains(path) { unique.append(path) }
        return unique
    }

    private static func loginShell() -> String {
        guard let record = getpwuid(getuid()), let pointer = record.pointee.pw_shell else { return "/bin/zsh" }
        return String(cString: pointer)
    }

    private static func shellQuote(_ value: String) -> String {
        "'" + value.replacingOccurrences(of: "'", with: "'\\''") + "'"
    }

    private static func isExecutable(_ path: String) -> Bool {
        FileManager.default.isExecutableFile(atPath: path)
    }

    private static func isCompatible(_ path: String, minimumVersion: String) -> Bool {
        let process = Process()
        let output = Pipe()
        process.executableURL = URL(fileURLWithPath: path)
        process.arguments = ["--version"]
        process.standardOutput = output
        process.standardError = output
        guard (try? process.run()) != nil else { return false }
        guard wait(for: process, timeout: 2), process.terminationStatus == 0,
              let text = String(data: output.fileHandleForReading.readDataToEndOfFile(), encoding: .utf8),
              let actual = version(in: text),
              let minimum = version(in: minimumVersion) else { return false }
        return actual >= minimum
    }

    private static func wait(for process: Process, timeout: TimeInterval) -> Bool {
        let deadline = Date().addingTimeInterval(timeout)
        while process.isRunning && Date() < deadline {
            Thread.sleep(forTimeInterval: 0.05)
        }
        if process.isRunning {
            process.terminate()
            return false
        }
        return true
    }

    private static func version(in text: String) -> Version? {
        guard let match = text.range(of: #"v?(\d+)\.(\d+)(?:\.(\d+))?"#, options: .regularExpression) else { return nil }
        let value = String(text[match]).trimmingCharacters(in: CharacterSet(charactersIn: "v"))
        let parts = value.split(separator: ".").compactMap { Int($0) }
        guard parts.count >= 2 else { return nil }
        return Version(major: parts[0], minor: parts[1], patch: parts.count > 2 ? parts[2] : 0)
    }
}
