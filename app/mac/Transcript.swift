import SwiftUI
import VeluneBindings
import AppKit

/// A tab's transcript reading, kept while its views are gone (the tab is
/// behind another): the native list with its geometry, scroll position and
/// disclosures, and the follow intent around it.
@MainActor final class TranscriptReading: ObservableObject {
    @Published var following = true
    @Published var bottomRequest: UInt64 = 0
    /// The row kept in place by the next outline change.
    @Published var outlineAnchorID: String?
    let table = TranscriptTableController()
}

/// Reading state that belongs to the conversation, not to native geometry:
/// follow intent, the in-place outline and explicit navigation requests.
struct TranscriptView: View {
    @ObservedObject var model: TranscriptModel
    @ObservedObject var reading: TranscriptReading
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
    let openAgentURI: (String) -> Void
    var openArtifact: (ArtifactDocument) -> Void = { _ in }
    /// Edits the user message of this ID, when it may be edited now.
    var editMessage: (String) -> (() -> Void)? = { _ in nil }
    /// Forks the conversation through the turn this user message ID starts.
    var forkConversation: (String) -> Void = { _ in }
    let describeSettings: (TranscriptTurn) -> TurnSettingsSummary?
    var recovery: RuntimeRecovery? = nil
    var stoppingRecovery = false
    var canStopRecovery = false
    var stopRecovery: () -> Void = {}
    /// A fork the user asked for while a turn ran; it fires when the turn ends.
    var heldForkNotice: String? = nil
    @State private var dismissedRecovery: String?
    @State private var scrollerStyle = NSScroller.preferredScrollerStyle

    var body: some View {
        let visibleRows: [TranscriptRow]
        if outlineCollapsed {
            let userIDs = Set(model.userRows.map(\.id))
            visibleRows = model.rows.filter { userIDs.contains($0.id) }
        } else { visibleRows = model.rows }
        return TranscriptTable(controller: reading.table, conversationID: conversationID, items: model.items(for: visibleRows, turnHeaders: !outlineCollapsed), revision: model.contentRevision,
                               activity: recovery?.active == true ? nil : activity, outlineCollapsed: outlineCollapsed, outlineAnchorID: reading.outlineAnchorID,
                               cwd: cwd, remoteHost: remoteHost, bottomRequest: reading.bottomRequest, jump: navigation,
                               followingChanged: { [reading] value in if reading.following != value { reading.following = value } },
                               outlineToggle: { [reading] id in reading.outlineAnchorID = id; outlineCollapsed = false }, loadPayloads: loadPayloads,
                               openConversation: openConversation, openAgentURI: openAgentURI, openArtifact: openArtifact, editMessage: editMessage, forkConversation: forkConversation, describeSettings: describeSettings)
            .background(Color(nsColor: .windowBackgroundColor))
            .overlay(alignment: .bottomTrailing) {
                if !reading.following {
                    Button { reading.following = true; reading.bottomRequest &+= 1 } label: { Image(systemName: "arrow.down") }
                        .buttonStyle(.bordered).controlSize(.large)
                        .help("回到底部").accessibilityLabel("回到底部")
                        .padding(.bottom, 16).padding(.trailing, 16 + scrollerInset)
                }
            }
            .safeAreaInset(edge: .bottom, spacing: 0) {
                if let recovery, recovery.active || dismissedRecovery != recovery.id {
                    RuntimeRecoveryView(recovery: recovery, stopping: stoppingRecovery, canStop: canStopRecovery, stop: stopRecovery) { dismissedRecovery = recovery.id }
                } else if let heldForkNotice {
                    Label(heldForkNotice, systemImage: "clock")
                        .uiFont(.callout).foregroundStyle(.secondary)
                        .padding(.horizontal, 16).padding(.vertical, 8)
                        .frame(maxWidth: .infinity, alignment: .leading)
                        .background(.bar)
                }
            }
            .onReceive(NotificationCenter.default.publisher(for: NSScroller.preferredScrollerStyleDidChangeNotification)) { _ in
                scrollerStyle = NSScroller.preferredScrollerStyle
            }
            .onChange(of: conversationID) { _, _ in reading.following = reading.table.isFollowing; reading.outlineAnchorID = nil }
            .onChange(of: conversationID) { _, _ in dismissedRecovery = nil }
            // A toolbar toggle keeps the section at the top of the viewport.
            .onChange(of: outlineCollapsed) { _, collapsed in if collapsed { reading.outlineAnchorID = nil } }
            .onChange(of: scrollRequest) { _, _ in
                outlineCollapsed = false
                reading.following = true; reading.bottomRequest &+= 1
            }
    }

    /// Room for the vertical scroller: a legacy scroller always occupies the
    /// trailing edge; an overlay scroller widens over content while in use.
    private var scrollerInset: CGFloat {
        NSScroller.scrollerWidth(for: .regular, scrollerStyle: scrollerStyle == .legacy ? .legacy : .overlay)
    }
}
