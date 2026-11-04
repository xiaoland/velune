import SwiftUI

/// Device-wide work belongs to window chrome, not an editor's input state.
struct ConversationOperationStatus: View {
    @ObservedObject var device: DeviceStore
    @State private var showsDetails = false

    var body: some View {
        if let current = device.conversationManagement.first {
            Button { showsDetails.toggle() } label: {
                HStack(spacing: 6) {
                    ProgressView().controlSize(.mini)
                    Text(current.running ? "正在\(current.source)" : "已排队：\(current.source)")
                        .lineLimit(1).truncationMode(.middle).frame(maxWidth: 200)
                    if device.conversationManagement.count > 1 { Text("+\(device.conversationManagement.count - 1)").monospacedDigit() }
                }.uiFont(.callout).foregroundStyle(.secondary)
            }
            .buttonStyle(.borderless)
            .popover(isPresented: $showsDetails) {
                VStack(alignment: .leading, spacing: 8) {
                    ForEach(device.conversationManagement) { request in
                        Label(request.source, systemImage: request.running ? "arrow.triangle.2.circlepath" : "clock")
                    }
                }.padding(16).frame(maxWidth: 420)
            }
        }
    }
}
