#if VELUNE_DEV_CHECKS
import Foundation

extension MacExecutionContext {
    /// Older hidden checks deliberately reopen nonempty synthetic fixtures.
    /// They keep their explicit Transport initializers, within a temporary tree.
    func validateCheckDirectories(environment: [String: String]) throws {
        let temporaryRoots = [URL(fileURLWithPath: "/tmp", isDirectory: true), FileManager.default.temporaryDirectory]
            .map { $0.standardizedFileURL.resolvingSymlinksInPath() }
        let daily = userHome.appendingPathComponent(".velune", isDirectory: true).resolvingSymlinksInPath()
        for key in ["VELUNE_CONTINUATION_CHECK_HOME", "VELUNE_COMPOSER_CHECK_HOME", "VELUNE_COMPOSER_NATIVE_HOME", "VELUNE_EXPERIENCE_CHECK_HOME", "VELUNE_CHECK_OUTPUT"] {
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
}
#endif
