import SwiftUI

/// Device-wide work belongs to window chrome, not an editor's input state.
struct ConversationOperationStatus: View {
    @ObservedObject var aggregated: AggregatedListModel
    @State private var showsDetails = false

    var body: some View {
        let operations = aggregated.conversationManagement
        if let current = operations.first?.request {
            Button { showsDetails.toggle() } label: {
                HStack(spacing: 6) {
                    ProgressView().controlSize(.mini)
                    Text(current.running ? "正在\(current.source)" : "已排队：\(current.source)")
                        .lineLimit(1).truncationMode(.middle).frame(maxWidth: 200)
                    if operations.count > 1 { Text("+\(operations.count - 1)").monospacedDigit() }
                }.uiFont(.callout).foregroundStyle(.secondary)
            }
            .buttonStyle(.borderless)
            .popover(isPresented: $showsDetails) {
                ConversationOperationDetails(aggregated: aggregated)
            }
        }
    }
}

struct ConversationOperationDetails: View {
    @ObservedObject var aggregated: AggregatedListModel

    var body: some View {
        VStack(alignment: .leading, spacing: 8) {
            ForEach(aggregated.conversationManagement) { operation in
                let request = operation.request
                HStack(spacing: 12) {
                    VStack(alignment: .leading, spacing: 3) {
                        Label(request.source, systemImage: request.running ? "arrow.triangle.2.circlepath" : "clock")
                        if aggregated.devices.count > 1 { Text(operation.device.name).uiFont(.caption).foregroundStyle(.secondary) }
                    }
                    Spacer(minLength: 12)
                    if request.running {
                        Text("执行中").foregroundStyle(.secondary)
                            .help("操作已开始，不能取消；结果会在完成后更新。")
                    } else {
                        Button("取消") { operation.device.store.cancelConversationManagement(request.id) }
                            .help("取消排队中的\(request.source)")
                            .accessibilityLabel("取消\(request.source)")
                            .accessibilityIdentifier("cancel-management-\(request.id)")
                    }
                }
            }
        }.padding(16).frame(maxWidth: 420)
    }
}
