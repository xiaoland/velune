#if VELUNE_DEV_CHECKS
import AppKit
import VeluneBindings

extension TranscriptTableController {
    /// Exercises the production model, virtualized controller, cell reuse and
    /// AppKit's unignored accessibility children in an offscreen native window.
    static func checkMaterializedRowOrder() {
        let model = TranscriptModel()
        let controller = TranscriptTableController()
        let scroll = controller.view()
        let window = NSWindow(contentRect: NSRect(x: 0, y: 0, width: 900, height: 800), styleMask: [.borderless], backing: .buffered, defer: false)
        window.isReleasedWhenClosed = false
        window.contentView = scroll
        scroll.layoutSubtreeIfNeeded()
        guard let document = scroll.documentView else { fatalError("合成消息列表未创建原生document") }
        func cells(_ ids: [String]) -> [TranscriptCellView] {
            document.subviews.compactMap { $0 as? TranscriptCellView }.filter { $0.rowID.map(ids.contains) == true }
        }
        func rowID(of element: Any) -> String? {
            var view = element as? NSView ?? (element as? NSCell)?.controlView
            while let current = view {
                if let cell = current as? TranscriptCellView { return cell.rowID }
                view = current.superview
            }
            return nil
        }
        func apply(_ ids: [String]) {
            let messages = ids.map { DeliveredMessage(id: $0, role: $0 == "N" ? .client : .assistant, blocks: [.text(text: "native order \($0)")]) }
            model.apply(messages, turns: [], items: ids.map { .message(id: $0, messageId: $0) }, outline: [], identities: ids.map { .init(id: $0, messageId: $0) }, editable: [])
            let items = model.items(for: model.rows, turnHeaders: false)
            precondition(items.map(\.id) == ids, "权威消息模型顺序必须保持")
            controller.update(TranscriptTable(controller: controller, conversationID: "synthetic-native-order", items: items, revision: model.contentRevision,
                activity: nil, outlineCollapsed: false, outlineAnchorID: nil, cwd: nil, bottomRequest: 0, jump: nil,
                followingChanged: { _ in }, outlineToggle: { _ in }, loadPayloads: { _ in }, openConversation: { _ in }, openAgentURI: { _ in }, openArtifact: { _ in },
                editMessage: { _ in nil }, forkConversation: { _ in }, describeSettings: { _ in nil }))
            controller.materializeVisibleRows(extra: document.bounds)
            let materialized = cells(ids)
            precondition(materialized.map(\.rowID) == ids.map(Optional.some), "原生cell子视图必须与消息顺序一致")
            precondition(zip(materialized, materialized.dropFirst()).allSatisfy { $0.0.frame.minY < $0.1.frame.minY }, "原生cell几何顺序必须一致")
            let axIDs = NSAccessibility.unignoredChildren(from: document.subviews).compactMap(rowID).filter(ids.contains).reduce(into: [String]()) {
                if $0.last != $1 { $0.append($1) }
            }
            precondition(axIDs == ids, "AppKit可访问性顺序不能把中间插入的消息堆到末尾")
        }
        apply(["A", "C"])
        let kept = Dictionary(uniqueKeysWithValues: cells(["A", "C"]).map { ($0.rowID!, ObjectIdentifier($0)) })
        controller.focusFromKeyboard()
        precondition(controller.selectionOutlineIsFrontmostForCheck)
        apply(["A", "N", "C"])
        precondition(cells(["A", "C"]).allSatisfy { kept[$0.rowID!] == ObjectIdentifier($0) }, "中间插入不能重建已有cell")
        precondition(controller.selectionOutlineIsFrontmostForCheck, "selection overlay必须保留在所有cells之上")
        apply(["A", "C"])
        precondition(cells(["A", "C"]).allSatisfy { kept[$0.rowID!] == ObjectIdentifier($0) })
        let unchanged = document.subviews.map(ObjectIdentifier.init)
        controller.materializeVisibleRows(extra: document.bounds)
        precondition(document.subviews.map(ObjectIdentifier.init) == unchanged, "稳定物化不能改变子视图顺序或身份")
        let preferences = AppearancePreferences.shared
        let originalAppearance = preferences.values
        let originalRevision = model.contentRevision
        let originalHeight = cells(["A"]).first!.frame.height
        controller.viewRemoved()
        preferences.values.zoomPercent = originalAppearance.zoomPercent < 150 ? 150 : 100
        precondition(cells(["A", "C"]).count == 2, "外观变化不能清除后台标签的行")
        let reattached = controller.view()
        precondition(reattached === scroll, "返回标签时复用原生列表")
        apply(["A", "C"])
        precondition(model.contentRevision == originalRevision, "本地外观变化不推进消息版本")
        precondition(cells(["A"]).first!.frame.height != originalHeight, "重新挂载后按新字号排版")
        preferences.values = originalAppearance
        controller.viewRemoved()
        window.contentView = nil
        FileHandle.standardError.write(Data("PASS native transcript: row and AX order, stable cells, and background-tab zoom refresh at the same message revision\n".utf8))
    }
}
#endif
