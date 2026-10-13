import SwiftUI
import AppKit

/// Reading state that belongs to the conversation, not to native geometry:
/// follow intent, the in-place outline and explicit navigation requests.
struct TranscriptView: View {
    @ObservedObject var model: TranscriptModel
    let conversationID: String?
    let scrollRequest: UInt64
    let cwd: String?
    /// The machine `cwd` and file links belong to, when it is not this Mac.
    var remoteHost: String? = nil
    /// The list shows only user messages (in-place outline).
    @Binding var outlineCollapsed: Bool
    /// A jump chosen in the outline popover.
    let navigation: TranscriptJump?
    let activity: TranscriptActivity?
    let loadPayloads: ([String]) -> Void
    let openConversation: (String) -> Void
    let describeSettings: (TurnSettings) -> TurnSettingsSummary?
    @State private var following = true
    @State private var bottomRequest: UInt64 = 0
    @State private var outlineAnchorID: String?
    @State private var scrollerStyle = NSScroller.preferredScrollerStyle

    var body: some View {
        let visibleRows: [TranscriptRow]
        if outlineCollapsed {
            let userIDs = Set(model.userRows.map(\.id))
            visibleRows = model.rows.filter { userIDs.contains($0.id) }
        } else { visibleRows = model.rows }
        return TranscriptTable(conversationID: conversationID, items: model.items(for: visibleRows, turnHeaders: !outlineCollapsed), revision: model.contentRevision,
                               activity: activity, outlineCollapsed: outlineCollapsed, outlineAnchorID: outlineAnchorID,
                               cwd: cwd, remoteHost: remoteHost, bottomRequest: bottomRequest, jump: navigation,
                               followingChanged: { value in if following != value { following = value } },
                               outlineToggle: { id in outlineAnchorID = id; outlineCollapsed = false }, loadPayloads: loadPayloads,
                               openConversation: openConversation, describeSettings: describeSettings)
            .background(Color(nsColor: .windowBackgroundColor))
            .overlay(alignment: .bottomTrailing) {
                if !following {
                    Button { following = true; bottomRequest &+= 1 } label: { Image(systemName: "arrow.down") }
                        .buttonStyle(.bordered).controlSize(.large)
                        .help("回到底部").accessibilityLabel("回到底部")
                        .padding(.bottom, 16).padding(.trailing, 16 + scrollerInset)
                }
            }
            .onReceive(NotificationCenter.default.publisher(for: NSScroller.preferredScrollerStyleDidChangeNotification)) { _ in
                scrollerStyle = NSScroller.preferredScrollerStyle
            }
            .onChange(of: conversationID) { _, _ in following = true; outlineAnchorID = nil }
            // A toolbar toggle keeps the section at the top of the viewport.
            .onChange(of: outlineCollapsed) { _, collapsed in if collapsed { outlineAnchorID = nil } }
            .onChange(of: scrollRequest) { _, _ in
                outlineCollapsed = false
                following = true; bottomRequest &+= 1
            }
    }

    /// Room for the vertical scroller: a legacy scroller always occupies the
    /// trailing edge; an overlay scroller widens over content while in use.
    private var scrollerInset: CGFloat {
        NSScroller.scrollerWidth(for: .regular, scrollerStyle: scrollerStyle == .legacy ? .legacy : .overlay)
    }
}
