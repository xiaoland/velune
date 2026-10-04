import Foundation
import Security

let arguments = CommandLine.arguments
let isSource = arguments.count == 3 && arguments[1] == "--source-json"
guard arguments.count == 2 || isSource else { exit(64) }
let account = arguments[isSource ? 2 : 1]

// A typed Harness source is resolved by its bundled adapter. A failed explicit
// source never falls through to Keychain or ambient environment credentials.
if isSource {
    struct Source: Decodable {
        let kind: String
        let harnessTypeId: String
        let providerId: String
        let settings: [String: String]
    }
    struct Result: Decodable {
        let contractVersion: Int
        let bearer: String
        let capabilities: Capabilities
    }
    struct Capabilities: Decodable {
        let `protocol`: String
        let endpoint: String
        let explicitOutputCap: Bool
        let temperature: Bool
    }
    guard let input = account.data(using: .utf8),
          let source = try? JSONDecoder().decode(Source.self, from: input),
          source.kind == "harness",
          let node = source.settings["nodeBinary"], node.hasPrefix("/"), !node.contains("\u{0}"),
          let authPath = source.settings["authPath"], authPath.hasPrefix("/"), !authPath.contains("\u{0}") else { exit(64) }
    let executable = URL(fileURLWithPath: CommandLine.arguments[0]).standardizedFileURL
    let helper = executable.deletingLastPathComponent().deletingLastPathComponent()
        .appendingPathComponent("Resources/credential_source.mjs")
    let process = Process()
    process.executableURL = URL(fileURLWithPath: node)
    process.arguments = [helper.path, "--operation", "resolve", "--source-json", account]
    process.environment = [:]
    process.standardInput = FileHandle.nullDevice
    process.standardError = FileHandle.nullDevice
    let output = Pipe(); process.standardOutput = output
    do { try process.run() } catch { exit(1) }
    let data = output.fileHandleForReading.readDataToEndOfFile()
    process.waitUntilExit()
    guard process.terminationStatus == 0,
          let result = try? JSONDecoder().decode(Result.self, from: data),
          result.contractVersion == 1, !result.bearer.isEmpty else { exit(1) }
    FileHandle.standardOutput.write(data)
    exit(0)
}

let query: [String: Any] = [
    kSecClass as String: kSecClassGenericPassword,
    kSecAttrService as String: "local.velune.provider",
    kSecAttrAccount as String: account,
    kSecReturnData as String: true,
    kSecMatchLimit as String: kSecMatchLimitOne,
]
var item: CFTypeRef?
guard SecItemCopyMatching(query as CFDictionary, &item) == errSecSuccess,
      let data = item as? Data else { exit(1) }
FileHandle.standardOutput.write(data)
