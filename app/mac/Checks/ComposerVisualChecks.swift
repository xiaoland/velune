#if VELUNE_DEV_CHECKS
import AppKit
import SwiftUI
import VeluneBindings

/// Synthetic native hosts preserve the real composer's content and action row.
/// Focus is assigned directly to this window; no global input is posted.
@MainActor func composerVisualStateChecks(directory: URL) {
    var observations: [String] = []
    ConversationStore.inspectComposerVisualStates { workspace, state, snapshot in
        for width: CGFloat in [360, 520, 1000] {
            let model = workspace.composer(for: workspace.focused)
            let store = workspace.focused.store
            let text = state == "idle" ? "" : String(repeating: "检查输入长内容增长与滚动，保留执行配置。\n", count: 18)
            model.restore(text, attachments: [])
            let view = ComposerView(store: store, model: model, layout: workspace, send: {}, steer: {},
                submission: { workspace.focused.submission(using: model) }, resume: {}, fillsHeight: true)
            let host = NSHostingView(rootView: view.applicationAppearance().background(Color(nsColor: .windowBackgroundColor)))
            let window = NSWindow(contentRect: .init(x: 0, y: 0, width: width, height: 360), styleMask: [.borderless], backing: .buffered, defer: false)
            window.contentView = host
            @MainActor func settle() { host.layoutSubtreeIfNeeded(); RunLoop.main.run(until: Date().addingTimeInterval(0.12)); host.layoutSubtreeIfNeeded() }
            @MainActor func capture(_ suffix: String) {
                host.displayIfNeeded()
                guard let bitmap = host.bitmapImageRepForCachingDisplay(in: host.bounds) else { fatalError("Missing native composer bitmap") }
                host.cacheDisplay(in: host.bounds, to: bitmap)
                try! bitmap.representation(using: .png, properties: [:])!.write(to: directory.appendingPathComponent("\(state)-\(Int(width))-\(suffix).png"))
            }
            settle(); capture("blur")
            let before = model.textView.map { host.convert($0.bounds, from: $0) }
            if let editor = model.textView, editor.isEditable { precondition(window.makeFirstResponder(editor), "Synthetic editor must actually receive focus") }
            settle(); capture("focus")
            let after = model.textView.map { host.convert($0.bounds, from: $0) }
            if let before, let after { precondition(abs(before.height - after.height) < 1 && abs(before.minY - after.minY) < 1, "Composer focus moved layout: \(state)") }
            observations.append("\(state) \(Int(width)): focus frame stable; viewport \(Int(model.textView?.enclosingScrollView?.frame.height ?? 0))pt")
            let pane = NSHostingView(rootView: PaneLayoutView(window: workspace, layout: workspace.layout, path: [], previewEmpty: false).applicationAppearance().background(Color(nsColor: .windowBackgroundColor)))
            let paneWindow = NSWindow(contentRect: .init(x: 0, y: 0, width: width, height: 700), styleMask: [.borderless], backing: .buffered, defer: false)
            paneWindow.contentView = pane
            pane.layoutSubtreeIfNeeded(); RunLoop.main.run(until: Date().addingTimeInterval(0.15)); pane.layoutSubtreeIfNeeded()
            if let bitmap = pane.bitmapImageRepForCachingDisplay(in: pane.bounds) {
                pane.cacheDisplay(in: pane.bounds, to: bitmap)
                try! bitmap.representation(using: .png, properties: [:])!.write(to: directory.appendingPathComponent("\(state)-\(Int(width))-pane.png"))
            }
            @MainActor func hasCompleteTooltip(_ view: NSView) -> Bool {
                if let handle = view as? TabHandle.HandleView, handle.toolTip == snapshot.conversation.title { return true }
                return view.subviews.contains(where: hasCompleteTooltip)
            }
            precondition(hasCompleteTooltip(pane), "Truncated tab must retain complete native tooltip")
            if state == "model-missing" || state == "model-failed", width == 520, let message = snapshot.messages.first(where: { $0.role == .user }) {
                store.device.keepPreviewSnapshot(snapshot)
                let session = MessageEditingSession(source: store, request: .init(message: message, conversationID: store.loadedConversationID, inPlace: true, isLast: true, revision: nil), presentation: .sheet)
                let sheet = NSHostingView(rootView: MessageEditorView(session: session, window: workspace, source: workspace.focused).applicationAppearance().background(Color(nsColor: .windowBackgroundColor)))
                let native = NSWindow(contentRect: .init(x: 0, y: 0, width: 620, height: 540), styleMask: [.borderless], backing: .buffered, defer: false)
                native.contentView = sheet
                sheet.layoutSubtreeIfNeeded(); RunLoop.main.run(until: Date().addingTimeInterval(0.15)); sheet.layoutSubtreeIfNeeded()
                if let bitmap = sheet.bitmapImageRepForCachingDisplay(in: sheet.bounds) {
                    sheet.cacheDisplay(in: sheet.bounds, to: bitmap)
                    try! bitmap.representation(using: .png, properties: [:])!.write(to: directory.appendingPathComponent("\(state)-editor.png"))
                }
                native.contentView = nil
            }
            if state == "compacted", width == 520 {
                let details = NSHostingView(rootView: StatusDetails(status: ConversationStatus(conversationID: store.loadedConversationID, directory: "/tmp/synthetic", usage: snapshot.usage), accounts: [], branch: "synthetic", display: .used, quotaDisplay: .remaining).applicationAppearance().background(Color(nsColor: .windowBackgroundColor)))
                let native = NSWindow(contentRect: .init(x: 0, y: 0, width: 380, height: 520), styleMask: [.borderless], backing: .buffered, defer: false)
                native.contentView = details
                details.layoutSubtreeIfNeeded(); RunLoop.main.run(until: Date().addingTimeInterval(0.15)); details.layoutSubtreeIfNeeded()
                if let bitmap = details.bitmapImageRepForCachingDisplay(in: details.bounds) {
                    details.cacheDisplay(in: details.bounds, to: bitmap)
                    try! bitmap.representation(using: .png, properties: [:])!.write(to: directory.appendingPathComponent("compacted-status.png"))
                }
                native.contentView = nil
            }
            paneWindow.contentView = nil
            window.contentView = nil
        }
    }
    try! observations.joined(separator: "\n").write(to: directory.appendingPathComponent("measurements.txt"), atomically: true, encoding: .utf8)
    FileHandle.standardError.write(Data("PASS native composer visual states: idle/running/queue/paused/background/external/child at 360/520/1000pt, blur/focus frame stability\n".utf8))
}
#endif
