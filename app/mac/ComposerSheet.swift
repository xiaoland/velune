import SwiftUI
import VeluneBindings

/// A conversation's composer over the window, from the list's menu: a
/// message goes (or waits in the queue, or the turn continues) without the
/// conversation showing. The sheet closes once the core took it; a refused
/// one leaves its text back in the composer and the sheet open.
struct ComposerSheet: View {
    @ObservedObject var window: ConversationWindowModel
    @ObservedObject var tab: ConversationTab
    @ObservedObject var store: ConversationStore

    var body: some View {
        // The composer brings its own margins; the title and buttons line up with them.
        VStack(alignment: .leading, spacing: 0) {
            HStack(spacing: 8) {
                Text(tab.title).uiFont(.headline).lineLimit(1).truncationMode(.middle)
                Spacer(minLength: 12)
                Button("打开会话", action: window.openComposerSheetConversation)
            }
            .padding([.horizontal, .top], 16)
            if store.isOpeningConversation {
                ProgressView("正在加载会话…").frame(maxWidth: .infinity).padding(.top, 12)
            } else if !store.pendingInteractions.isEmpty {
                Text("此会话有待处理的请求，打开会话处理后才能发送。").foregroundStyle(.secondary).padding([.horizontal, .top], 16)
            }
            ComposerView(store: store, model: window.composer(for: tab), layout: window, send: { window.composerContext(for: tab).send() }, steer: { window.composerContext(for: tab).steer() }, submission: { window.composerContext(for: tab).submission() }, resume: { window.composerContext(for: tab).resume() })
                .disabled(store.isOpeningConversation)
            HStack {
                Spacer()
                Button("取消", action: window.closeComposerSheet).keyboardShortcut(.cancelAction)
            }
            .padding([.horizontal, .bottom], 16)
        }
        .frame(width: 560)
        .onChange(of: store.acceptedSubmissions) { _, _ in window.closeComposerSheet() }
    }
}
