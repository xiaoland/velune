import Foundation

enum IPadInstaller {
    struct Device: Identifiable, Equatable {
        let id: String
        let name: String
        let xcodeDestinationID: String
    }

    private struct ResponseInfo: Decodable {
        let commandType: String
        let outcome: String
    }

    private struct DeviceList: Decodable {
        struct Result: Decodable { let devices: [RawDevice] }
        let info: ResponseInfo
        let result: Result
    }

    private struct RawDevice: Decodable {
        struct Hardware: Decodable { let platform: String; let deviceType: String; let udid: String }
        struct Connection: Decodable { let pairingState: String; let transportType: String }
        struct Properties: Decodable { let name: String }
        let identifier: String
        let hardwareProperties: Hardware
        let connectionProperties: Connection
        let deviceProperties: Properties
    }

    private struct InstallResult: Decodable {
        struct Result: Decodable {
            struct InstalledApplication: Decodable { let bundleID: String }
            let deviceIdentifier: String
            let installedApplications: [InstalledApplication]
        }
        let info: ResponseInfo
        let result: Result
    }

    private struct ProvisioningProfile {
        let teams: [String]
        let devices: [String]
        let applicationIdentifier: String
        let expiration: Date
    }

    private final class CapturedOutput {
        private let lock = NSLock()
        private var data = Data()
        func append(_ chunk: Data) { lock.lock(); data.append(chunk); lock.unlock() }
        var text: String { lock.lock(); defer { lock.unlock() }; return String(decoding: data, as: UTF8.self) }
    }

    private enum InstallerError: LocalizedError {
        case invalidRepository
        case invalidTeam
        case unavailableDevice
        case commandFailed(String, Int32, String)
        case invalidOutput(String)
        case signingMismatch(String)

        var errorDescription: String? {
            switch self {
            case .invalidRepository: return "所选目录不是Velune仓库，缺少 iPad 工程或 Rust 构建脚本。"
            case .invalidTeam: return "Team ID 应为 10 位大写字母或数字。"
            case .unavailableDevice: return "所选 iPad 已断开或不再处于可安装状态，请重新选择设备。"
            case let .commandFailed(step, status, details):
                return details.isEmpty ? "\(step)失败（退出码 \(status)）。" : "\(step)失败（退出码 \(status)）：\(details)"
            case let .invalidOutput(step): return "\(step)返回了无法识别的结果。"
            case let .signingMismatch(reason): return "签名校验失败：\(reason)"
            }
        }
    }

    static func devices() throws -> [Device] {
        let temporary = FileManager.default.temporaryDirectory
            .appendingPathComponent("velune-devices-\(UUID().uuidString)", isDirectory: true)
        try FileManager.default.createDirectory(at: temporary, withIntermediateDirectories: true)
        defer { try? FileManager.default.removeItem(at: temporary) }
        let json = temporary.appendingPathComponent("devices.json")
        _ = try run("列出已配对设备", executable: "/usr/bin/xcrun", arguments: [
            "devicectl", "list", "devices", "--json-output", json.path, "--timeout", "30"
        ])
        return try parseDevices(Data(contentsOf: json))
    }

    static func parseDevices(_ data: Data) throws -> [Device] {
        let response: DeviceList
        do { response = try JSONDecoder().decode(DeviceList.self, from: data) }
        catch { throw InstallerError.invalidOutput("devicectl 设备列表") }
        guard response.info.commandType == "devicectl.list.devices", response.info.outcome == "success" else {
            throw InstallerError.invalidOutput("devicectl 设备列表")
        }
        return response.result.devices.compactMap { device in
            guard device.hardwareProperties.platform == "iOS",
                  device.hardwareProperties.deviceType == "iPad",
                  device.connectionProperties.pairingState == "paired",
                  !device.identifier.isEmpty, !device.hardwareProperties.udid.isEmpty,
                  !device.deviceProperties.name.isEmpty else { return nil }
            return Device(id: device.identifier, name: device.deviceProperties.name,
                          xcodeDestinationID: device.hardwareProperties.udid)
        }
    }

    static func install(repository: URL, deviceID: String, teamID: String) throws {
        guard teamID.count == 10,
              teamID.unicodeScalars.allSatisfy({ (65...90).contains(Int($0.value)) || (48...57).contains(Int($0.value)) })
        else { throw InstallerError.invalidTeam }
        let repository = repository.standardizedFileURL
        guard FileManager.default.fileExists(atPath: repository.appendingPathComponent("app/ios/Velune.xcodeproj").path),
              FileManager.default.isExecutableFile(atPath: repository.appendingPathComponent("scripts/build-ios.sh").path)
        else { throw InstallerError.invalidRepository }
        guard let device = try devices().first(where: { $0.id == deviceID }) else {
            throw InstallerError.unavailableDevice
        }

        let artifactRoot = repository.appendingPathComponent("target/IPadInstall.noindex", isDirectory: true)
        let runDirectory = artifactRoot.appendingPathComponent(UUID().uuidString + ".noindex", isDirectory: true)
        try FileManager.default.createDirectory(at: runDirectory, withIntermediateDirectories: true)
        defer { try? FileManager.default.removeItem(at: runDirectory) }

        var environment = ProcessInfo.processInfo.environment
        let path = environment["PATH"] ?? "/usr/bin:/bin"
        let additions = ["/opt/homebrew/bin", "/usr/local/bin", FileManager.default.homeDirectoryForCurrentUser
            .appendingPathComponent(".cargo/bin").path]
        environment["PATH"] = ([path] + additions).joined(separator: ":")

        // GUI launches do not inherit the shell's Cargo PATH or registry symlink workaround.
        let cargoHome = environment["CARGO_HOME"].map { URL(fileURLWithPath: $0) }
            ?? FileManager.default.homeDirectoryForCurrentUser.appendingPathComponent(".cargo")
        let registry = cargoHome.appendingPathComponent("registry").resolvingSymlinksInPath()
        if FileManager.default.fileExists(atPath: registry.path) {
            environment["CARGO_HOME"] = registry.deletingLastPathComponent().path
        }
        let derivedData = runDirectory.appendingPathComponent("DerivedData.noindex", isDirectory: true)
        environment["VELUNE_IOS_DERIVED_DATA"] = derivedData.path
        environment["VELUNE_IOS_DEVICE_ID"] = device.xcodeDestinationID
        _ = try run("构建并签名 iPad App", executable: "/bin/bash", arguments: [
            repository.appendingPathComponent("scripts/build-ios.sh").path, "iphoneos",
            "DEVELOPMENT_TEAM=\(teamID)", "-allowProvisioningUpdates", "-allowProvisioningDeviceRegistration"
        ], directory: repository, environment: environment, redacting: device.xcodeDestinationID)

        let app = derivedData.appendingPathComponent("Build/Products/Debug-iphoneos/Velune.app", isDirectory: true)
        try verifySignedApp(app, teamID: teamID, deviceID: device.xcodeDestinationID, redacting: deviceID)

        guard try devices().contains(where: { $0.id == deviceID }) else { throw InstallerError.unavailableDevice }
        let resultURL = runDirectory.appendingPathComponent("install.json")
        _ = try run("安装到 iPad", executable: "/usr/bin/xcrun", arguments: [
            "devicectl", "device", "install", "app", "--device", deviceID, app.path,
            "--json-output", resultURL.path, "--timeout", "300"
        ], directory: repository, environment: environment, redacting: deviceID)
        try validateInstallResult(Data(contentsOf: resultURL), deviceID: deviceID)
    }

    static func validateInstallResult(_ data: Data, deviceID: String) throws {
        let response: InstallResult
        do { response = try JSONDecoder().decode(InstallResult.self, from: data) }
        catch { throw InstallerError.invalidOutput("devicectl 安装") }
        guard response.info.commandType == "devicectl.device.install.app",
              response.info.outcome == "success",
              response.result.deviceIdentifier == deviceID,
              response.result.installedApplications.contains(where: { $0.bundleID == "app.velune.ios" })
        else { throw InstallerError.invalidOutput("devicectl 安装") }
    }

    private static func verifySignedApp(_ app: URL, teamID: String, deviceID: String, redacting: String) throws {
        let infoURL = app.appendingPathComponent("Info.plist")
        guard let info = NSDictionary(contentsOf: infoURL),
              info["CFBundleIdentifier"] as? String == "app.velune.ios",
              FileManager.default.fileExists(atPath: app.appendingPathComponent("Velune").path)
        else { throw InstallerError.signingMismatch("应用标识或构建产物不正确") }

        _ = try run("验证 App 签名", executable: "/usr/bin/codesign",
                arguments: ["--verify", "--deep", "--strict", app.path], redacting: redacting)
        let display = try run("读取签名团队", executable: "/usr/bin/codesign",
                              arguments: ["--display", "--verbose=4", app.path], redacting: redacting)
        let signingDetails = String(decoding: display.stderr, as: UTF8.self)
        guard signingDetails.split(separator: "\n").contains(where: { $0 == "TeamIdentifier=\(teamID)" }) else {
            throw InstallerError.signingMismatch("构建签名与所选 Team ID 不一致")
        }

        let profileURL = app.appendingPathComponent("embedded.mobileprovision")
        let profileData = try run("验证开发配置文件", executable: "/usr/bin/security",
                                  arguments: ["cms", "-D", "-i", profileURL.path], redacting: redacting).stdout
        let profile = try decodeProfile(profileData)
        guard profile.teams.contains(teamID), profile.devices.contains(deviceID),
              profile.applicationIdentifier.hasSuffix(".app.velune.ios"), profile.expiration > Date()
        else { throw InstallerError.signingMismatch("开发配置文件未授权所选团队、iPad 或应用标识，或已过期") }
    }

    private static func decodeProfile(_ data: Data) throws -> ProvisioningProfile {
        guard let plist = try? PropertyListSerialization.propertyList(from: data, format: nil) as? [String: Any],
              let teams = plist["TeamIdentifier"] as? [String],
              let devices = plist["ProvisionedDevices"] as? [String],
              let entitlements = plist["Entitlements"] as? [String: Any],
              let appID = entitlements["application-identifier"] as? String,
              let expiration = plist["ExpirationDate"] as? Date
        else { throw InstallerError.invalidOutput("开发配置文件") }
        return ProvisioningProfile(teams: teams, devices: devices,
                                   applicationIdentifier: appID, expiration: expiration)
    }

    private static func run(_ step: String, executable: String, arguments: [String],
                            directory: URL? = nil, environment: [String: String]? = nil,
                            redacting value: String? = nil) throws -> (stdout: Data, stderr: Data) {
        let process = Process()
        process.executableURL = URL(fileURLWithPath: executable)
        process.arguments = arguments
        process.currentDirectoryURL = directory
        process.environment = environment ?? ProcessInfo.processInfo.environment
        let stdout = Pipe(), stderr = Pipe()
        process.standardOutput = stdout
        process.standardError = stderr
        do { try process.run() }
        catch { throw InstallerError.commandFailed(step, -1, error.localizedDescription) }

        let output = CapturedOutput(), errors = CapturedOutput()
        let readers = DispatchGroup()
        for (pipe, capture) in [(stdout, output), (stderr, errors)] {
            readers.enter()
            DispatchQueue.global(qos: .utility).async {
                capture.append(pipe.fileHandleForReading.readDataToEndOfFile())
                readers.leave()
            }
        }
        process.waitUntilExit()
        readers.wait()
        if process.terminationStatus != 0 {
            var detail = [output.text, errors.text].filter { !$0.isEmpty }.joined(separator: "\n")
            if let value, !value.isEmpty { detail = detail.replacingOccurrences(of: value, with: "<设备>") }
            if detail.count > 6000 { detail = String(detail.suffix(6000)) }
            throw InstallerError.commandFailed(step, process.terminationStatus, detail)
        }
        return (Data(output.text.utf8), Data(errors.text.utf8))
    }
}
