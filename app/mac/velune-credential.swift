import Foundation
import Security

let arguments = CommandLine.arguments
guard arguments.count == 2 else { exit(64) }
let account = arguments[1]

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
