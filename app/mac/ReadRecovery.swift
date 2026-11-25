import SwiftUI

struct ReadRefreshButton: View {
    let label: String
    let action: () -> Void

    var body: some View {
        Button(label, systemImage: "arrow.clockwise", action: action)
            .labelStyle(.iconOnly).controlSize(.small).help(label)
    }
}

struct ReadFailureNotice: View {
    let title: String
    let detail: String?
    var icon = "exclamationmark.circle"
    var retryDisabled = false
    var retryLabel = "重试"
    var retry: (() -> Void)? = nil

    var body: some View {
        HStack(alignment: .top, spacing: 8) {
            Image(systemName: icon).foregroundStyle(.secondary).accessibilityHidden(true)
            VStack(alignment: .leading, spacing: 3) {
                Text(title).uiFont(.callout.weight(.semibold))
                if let detail {
                    Text(detail).uiFont(.caption).foregroundStyle(.secondary)
                        .lineLimit(3).help(detail).textSelection(.enabled)
                }
            }
            .frame(maxWidth: .infinity, alignment: .leading)
            if let retry { Button(retryLabel, action: retry).controlSize(.small).disabled(retryDisabled) }
        }
        .padding(.horizontal, 12).padding(.vertical, 10)
    }
}
