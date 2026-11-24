#if VELUNE_DEV_CHECKS
import AppKit
import VeluneBindings

@MainActor private final class CompactionCheckCanvas: NSView {
    override var isFlipped: Bool { true }
    override func draw(_ dirtyRect: NSRect) {
        NSColor.textBackgroundColor.setFill()
        dirtyRect.fill()
    }
}

/// Production transcript cells, without creating or presenting an NSWindow.
@MainActor func compactionTranscriptChecks(home: URL) {
    let detail = "合成摘要：保留用户目标、已完成的修改和未决事项。\n" +
        "下一步核对消息顺序；此内容只在用户展开压缩事件后出现。\n" +
        "保留文件：/tmp/synthetic/document.md\n" + String(repeating: "长摘要内容用于检查换行与完整呈现。", count: 14)
    let compact = DeliveredMessage(id: "compact", role: .system, blocks: [.contextCompaction(text: "上下文已压缩\n\n" + detail)])
    let rendered = RenderedMessage(compact)
    guard case .disclosure(let key, let title, _, let body, _) = rendered.blocks.first else { fatalError("摘要必须可折叠") }
    precondition(title == "上下文已压缩" && body?.string == detail)
    precondition(TranscriptSearch.matches(compact, query: "保留文件"))
    precondition(TranscriptSearch.disclosureKeys(in: compact, query: "保留文件") == [key], "搜索摘要必须能展开对应详情")
    let nativeNewlines = contextCompactionParts("上下文已压缩\r\n\r\n    原生缩进\r\n尾行\r\n")
    precondition(nativeNewlines.status == "上下文已压缩" && nativeNewlines.detail == "    原生缩进\r\n尾行\r\n")
    precondition(contextCompactionParts("状态\n\n \t\n").detail == nil)
    for status in ["正在压缩上下文", "上下文已压缩", "压缩失败：连接已关闭", ""] {
        let single = RenderedMessage(DeliveredMessage(id: "status", role: .system, blocks: [.contextCompaction(text: status)]))
        guard case .text(let value) = single.blocks.first else { fatalError("单行状态无需折叠") }
        precondition(value.string == status)
    }
    let failed = RenderedMessage(DeliveredMessage(id: "failed", role: .system,
        blocks: [.contextCompaction(text: "上下文压缩失败\n\n合成错误详情")]))
    guard case .disclosure(_, let failureTitle, _, _, _) = failed.blocks.first else { fatalError("缺少失败状态") }
    precondition(failureTitle == "上下文压缩失败", "折叠不能把失败显示成成功")

    for dark in [false, true] {
        let appearance = NSAppearance(named: dark ? .darkAqua : .aqua)!
        NSApp.appearance = appearance
        for width: CGFloat in [420, 900] {
            let cell = TranscriptCellView()
            let delegate = TranscriptTableController()
            var expanded: Set<String> = []
            func layout() -> TranscriptRowLayout {
                TranscriptRowLayout.make(.message(compact, revision: 0, expanded: expanded,
                    copies: false, edits: false, forks: false), rendered: rendered, width: width, measurer: TextMeasurer())
            }
            func apply() {
                let value = layout()
                cell.frame = NSRect(x: 0, y: 0, width: width, height: value.height)
                cell.apply(value, rowID: compact.id, outlineToggle: nil, edit: nil, fork: nil, toggleHelp: nil,
                    textDelegate: delegate, openArtifact: { _ in }, toggle: { changed in
                        if !expanded.insert(changed).inserted { expanded.remove(changed) }
                        apply()
                    })
                cell.layoutSubtreeIfNeeded()
            }
            func capture(_ state: String) {
                let canvas = CompactionCheckCanvas(frame: NSRect(x: 0, y: 0, width: width, height: cell.frame.height + 18))
                canvas.appearance = appearance
                canvas.addSubview(cell)
                canvas.layoutSubtreeIfNeeded()
                guard let bitmap = canvas.bitmapImageRepForCachingDisplay(in: canvas.bounds) else { fatalError("无法绘制压缩状态") }
                canvas.cacheDisplay(in: canvas.bounds, to: bitmap)
                try! bitmap.representation(using: .png, properties: [:])!.write(to: home.appendingPathComponent("compaction-\(dark ? "dark" : "light")-\(Int(width))-\(state).png"))
                cell.removeFromSuperview()
            }
            func descendants(_ view: NSView) -> [NSView] { [view] + view.subviews.flatMap(descendants) }
            apply()
            let closed = layout()
            precondition(!closed.elements.contains { if case .text(let text, _, _) = $0 { return text.string.contains("合成摘要") }; return false })
            capture("collapsed")
            guard let button = descendants(cell).compactMap({ $0 as? NSButton }).first(where: { $0.attributedTitle.string == title }) else { fatalError("缺少原生展开按钮") }
            button.performClick(nil)
            precondition(expanded == [key] && layout().height > closed.height)
            precondition(layout().elements.contains { if case .text(let text, _, _) = $0 { return text.string == detail }; return false })
            capture("expanded")
            button.performClick(nil)
            precondition(expanded.isEmpty && layout().height == closed.height, "收起应恢复原高度")
        }
    }
    precondition(!NSApp.isActive && NSApp.windows.allSatisfy { !$0.isVisible }, "检查不显示或激活窗口")
    FileHandle.standardError.write(Data("PASS compaction: default collapse, exact detail, native expand/collapse, search reveal, single-line and failure states; 8 offscreen captures, zero windows/models\n".utf8))
}
#endif
