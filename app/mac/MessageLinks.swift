import SwiftUI
import AppKit

/// Markdown destinations use the system's registered application. Relative files
/// belong to the conversation's working directory, never the app process directory.
struct MessageLinkOpener: ViewModifier {
    let cwd: String?
    @State private var failedDestination: String?

    func body(content: Content) -> some View {
        content.environment(\.openURL, OpenURLAction { original in
            let destination = resolved(original)
            if let destination, NSWorkspace.shared.open(destination) { return .handled }
            failedDestination = original.relativeString
            return .handled
        })
        .alert("无法打开链接", isPresented: Binding(get: { failedDestination != nil }, set: { if !$0 { failedDestination = nil } })) {
            Button("复制链接") {
                if let value = failedDestination {
                    NSPasteboard.general.clearContents()
                    NSPasteboard.general.setString(value, forType: .string)
                }
                failedDestination = nil
            }
            Button("关闭", role: .cancel) { failedDestination = nil }
        } message: { Text(failedDestination ?? "") }
    }

    private func resolved(_ url: URL) -> URL? {
        guard url.scheme == nil else { return url }
        let path = url.path
        if path.hasPrefix("/") { return URL(fileURLWithPath: path) }
        guard let cwd else { return nil }
        return URL(fileURLWithPath: cwd, isDirectory: true).appendingPathComponent(path).standardizedFileURL
    }
}
