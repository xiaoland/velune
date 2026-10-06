import SwiftUI
import AppKit

struct TranscriptView: View {
    @ObservedObject var model: TranscriptModel
    let conversationID: String?
    let scrollRequest: UInt64
    let activity: String?
    @State private var following = true
    @State private var userScrolling = false
    var body: some View {
        ScrollViewReader { proxy in
            ScrollView {
                LazyVStack(alignment: .leading, spacing: 24) {
                    ForEach(model.rows) { row in ConversationMessageView(row: row).id(row.id) }
                    if let activity { HStack { ProgressView().controlSize(.small); Text(activity).foregroundStyle(.secondary) }.font(.callout) }
                    Color.clear.frame(height: 1).id("transcript-bottom")
                }
                .frame(maxWidth: 760).padding(24).frame(maxWidth: .infinity)
                .background(GeometryReader { geometry in Color.clear.preference(key: TranscriptHeight.self, value: geometry.size.height) })
                .background(TranscriptScrollObserver { scrolling, nearBottom in
                    userScrolling = scrolling
                    following = nearBottom
                })
            }
            .overlay(alignment: .bottomTrailing) {
                if !following {
                    Button("回到底部", systemImage: "arrow.down") {
                        following = true
                        proxy.scrollTo("transcript-bottom", anchor: .bottom)
                    }.buttonStyle(.bordered).padding(16)
                }
            }
            .onAppear { following = true; scrollToBottom(proxy) }
            .onChange(of: conversationID) { _, _ in following = true; scrollToBottom(proxy) }
            .onChange(of: scrollRequest) { _, _ in following = true; scrollToBottom(proxy) }
            .onChange(of: model.contentRevision) { _, _ in if following && !userScrolling { scrollToBottom(proxy) } }
            .onPreferenceChange(TranscriptHeight.self) { _ in if following && !userScrolling { scrollToBottom(proxy) } }
        }
    }
    private func scrollToBottom(_ proxy: ScrollViewProxy) {
        // Wait for the new Markdown/Disclosure layout before moving the anchor.
        DispatchQueue.main.async { proxy.scrollTo("transcript-bottom", anchor: .bottom) }
    }
}
private struct TranscriptHeight: PreferenceKey {
    static let defaultValue: CGFloat = 0
    static func reduce(value: inout CGFloat, nextValue: () -> CGFloat) { value = nextValue() }
}

/// Native live-scroll notifications distinguish reading history from programmatic
/// scrolling and content layout changes (including streamed Markdown and images).
private struct TranscriptScrollObserver: NSViewRepresentable {
    let changed: (Bool, Bool) -> Void
    func makeNSView(context: Context) -> Probe { Probe(changed: changed) }
    func updateNSView(_ view: Probe, context: Context) { view.changed = changed; view.attach() }
    final class Probe: NSView {
        var changed: (Bool, Bool) -> Void
        private weak var observedScroll: NSScrollView?
        private var observations: [NSObjectProtocol] = []
        init(changed: @escaping (Bool, Bool) -> Void) { self.changed = changed; super.init(frame: .zero) }
        required init?(coder: NSCoder) { fatalError("init(coder:) is unavailable") }
        override func viewDidMoveToWindow() { super.viewDidMoveToWindow(); attach() }
        override func viewDidMoveToSuperview() { super.viewDidMoveToSuperview(); attach() }
        func attach() {
            guard let scroll = enclosingScrollView, scroll !== observedScroll else { return }
            observations.forEach(NotificationCenter.default.removeObserver)
            observations = []; observedScroll = scroll
            for (name, scrolling) in [(NSScrollView.willStartLiveScrollNotification, true), (NSScrollView.didLiveScrollNotification, true), (NSScrollView.didEndLiveScrollNotification, false)] {
                observations.append(NotificationCenter.default.addObserver(forName: name, object: scroll, queue: .main) { [weak self, weak scroll] _ in
                    guard let self, let scroll, let document = scroll.documentView else { return }
                    let bounds = scroll.contentView.bounds
                    let remaining = document.isFlipped ? document.bounds.maxY - bounds.maxY : bounds.minY - document.bounds.minY
                    self.changed(scrolling, remaining <= 48)
                })
            }
        }
        deinit { observations.forEach(NotificationCenter.default.removeObserver) }
    }
}
