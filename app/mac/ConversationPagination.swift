import SwiftUI

/// The same page action is shown for ordinary and cross-device project groups.
struct ConversationPaginationButton: View {
    let loaded: UInt32
    let total: UInt32
    let isLoading: Bool
    var sourceName: String? = nil
    let load: () -> Void

    private var title: String {
        let action = isLoading ? "正在加载…" : sourceName.map { "加载 \($0) 来源分组更多" } ?? "加载更多"
        return "\(action)（\(loaded) / \(total)）"
    }

    var body: some View {
        Button(title, action: load)
            .buttonStyle(.plain).foregroundStyle(.secondary)
            .disabled(isLoading)
    }
}
