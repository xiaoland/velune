import SwiftUI
import AppKit
import VeluneBindings

/// Reading state that belongs to the conversation, not to native geometry:
/// follow intent, outline mode and explicit navigation requests.
struct TranscriptView: View {
    @ObservedObject var model: TranscriptModel
    let conversationID: String?
    let scrollRequest: UInt64
    let sentAfterUserID: String?
    let cwd: String?
    @Binding var showsOutline: Bool
    let activity: String?
    let presentation: BindingTranscriptPresentation
    @State private var following = true
    @State private var outline = TranscriptOutlineState()
    @State private var jump: TranscriptJump?
    @State private var bottomRequest: UInt64 = 0

    var body: some View {
        let outlineMode = presentation == .userOutline
        let userIDs = Set(model.userRows.map(\.id))
        let visibleRows = outlineMode ? outline.visibleRows(model.rows, userIDs: userIDs) : model.rows
        TranscriptTable(conversationID: conversationID.map { $0 + "|" + String(describing: presentation) },
                        items: model.items(for: visibleRows), revision: model.contentRevision,
                        activity: activity, outlineToggleIDs: outlineMode ? userIDs : [], outlineExpandedID: outline.expandedFromUserID,
                        cwd: cwd, bottomRequest: bottomRequest, jump: jump,
                        followingChanged: { value in if following != value { following = value } },
                        outlineToggle: toggleOutline)
            .background(Color(nsColor: .windowBackgroundColor))
            .overlay(alignment: .bottomTrailing) {
                if !following {
                    Button { following = true; bottomRequest &+= 1 } label: { Image(systemName: "arrow.down") }
                        .buttonStyle(.bordered).help("回到底部").accessibilityLabel("回到底部").padding(16)
                }
            }
            .sheet(isPresented: $showsOutline) {
                TranscriptOutlineView(model: model) { id in
                    if outlineMode { _ = outline.expand(from: id, in: model.rows) }
                    navigate(to: id)
                }
            }
            .onChange(of: conversationID) { _, _ in outline = TranscriptOutlineState(); following = true; jump = nil }
            .onChange(of: presentation) { _, _ in outline = TranscriptOutlineState(); following = true; jump = nil }
            .onChange(of: scrollRequest) { _, _ in
                if outlineMode { outline.followSentUser(after: sentAfterUserID, in: model.rows) }
                following = true; bottomRequest &+= 1
            }
            .onChange(of: model.contentRevision) { _, _ in _ = outline.reconcile(model.rows) }
    }

    private func toggleOutline(_ id: String) {
        if outline.expandedFromUserID == id { outline.collapse() }
        else { _ = outline.expand(from: id, in: model.rows) }
        navigate(to: id)
    }

    private func navigate(to id: String) {
        following = false
        jump = TranscriptJump(id: id, token: (jump?.token ?? 0) &+ 1)
    }
}
