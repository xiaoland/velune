import SwiftUI

/// The same page action is shown for ordinary and cross-device project groups.
struct ConversationPaginationButton: View {
    let loaded: UInt32
    let total: UInt32
    let isLoading: Bool
    var sourceName: String? = nil
    var error: String? = nil
    let load: () -> Void

    private var title: String {
        let action = isLoading ? "正在加载…" : sourceName.map { "加载 \($0) 来源分组更多" } ?? "加载更多"
        return "\(action)（\(loaded) / \(total)）"
    }

    var body: some View {
        if let error {
            HStack(alignment: .firstTextBaseline, spacing: 6) {
                Label(error, systemImage: "exclamationmark.circle").lineLimit(2).help(error)
                Spacer(minLength: 0)
                Button("重试", action: load).buttonStyle(.borderless).disabled(isLoading)
                    .accessibilityLabel("重试加载更多会话（\(loaded) / \(total)）")
            }.uiFont(.caption).foregroundStyle(.secondary)
        } else {
            Button(title, action: load).buttonStyle(.plain).foregroundStyle(.secondary).disabled(isLoading)
        }
    }
}
