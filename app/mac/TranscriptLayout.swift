import AppKit

/// Width-independent native content of one message revision. Built once per
/// revision (normally off the main thread) and shared by measurement and display.
final class RenderedMessage: @unchecked Sendable {
    enum Block {
        case text(NSAttributedString)
        case code(language: String?, text: NSAttributedString, source: String, indent: CGFloat)
        case disclosure(key: String, title: String, symbol: String?, detail: NSAttributedString, detailMaxWidth: CGFloat?)
    }
    let role: MessageRole
    let blocks: [Block]

    init(_ message: Message) {
        role = message.role
        let alignment: NSTextAlignment = message.role == .system ? .center : .natural
        blocks = message.blocks.enumerated().flatMap { index, block -> [Block] in
            switch block {
            case .text(let text):
                guard message.role == .assistant else { return [.text(TranscriptTypography.plain(text, alignment: alignment))] }
                return MarkdownAttributedRenderer.render(text).map { segment in
                    switch segment {
                    case .prose(let text): return .text(text)
                    case .code(let language, let text, let source, let indent): return .code(language: language, text: text, source: source, indent: indent)
                    }
                }
            case .notice(let text):
                return [.text(TranscriptTypography.plain(text, font: TranscriptTypography.callout, color: .secondaryLabelColor, alignment: alignment))]
            case .reasoning(let text):
                return [.disclosure(key: Self.disclosureKey(block, at: index)!, title: "思考过程", symbol: nil,
                                    detail: Self.detail(text, font: TranscriptTypography.callout, empty: ""), detailMaxWidth: nil)]
            case .tool(_, let title, let state, let output):
                let symbol = switch state {
                case .running: "circle.dotted"
                case .failed: "exclamationmark.circle"
                case .pending: "circle"
                case .completed: "checkmark.circle"
                }
                return [.disclosure(key: Self.disclosureKey(block, at: index)!, title: title, symbol: symbol,
                                    detail: Self.detail(output ?? .inline(""), font: TranscriptTypography.code, empty: "没有附加输出"), detailMaxWidth: 560)]
            }
        }
    }
}

extension RenderedMessage {
    /// Local disclosure identity of a reasoning or tool block within its message.
    static func disclosureKey(_ block: MessageBlock, at index: Int) -> String? {
        switch block {
        case .reasoning: return "reasoning:\(index)"
        case .tool(let id, _, _, _): return "tool:" + (id ?? String(index))
        case .text, .notice: return nil
        }
    }
    /// Disclosures whose content has not been read from the core yet.
    static func deferredDisclosureKeys(_ message: Message) -> Set<String> {
        Set(message.blocks.enumerated().compactMap { index, block in
            switch block {
            case .reasoning(.deferred), .tool(_, _, _, .deferred?): return disclosureKey(block, at: index)
            case .text, .notice, .reasoning(.inline), .tool: return nil
            }
        })
    }
    private static func detail(_ payload: MessagePayload, font: NSFont, empty: String) -> NSAttributedString {
        let text = switch payload {
        case .inline(let text): text.isEmpty ? empty : text
        case .deferred(let bytes): "正在载入（\(ByteCountFormatter.string(fromByteCount: Int64(bytes), countStyle: .file))）…"
        }
        return TranscriptTypography.plain(text, font: font, color: .secondaryLabelColor)
    }
}

/// Everything that determines one row's geometry. Immutable values only, so a
/// background pass can lay rows out without touching main-actor state.
enum TranscriptRowContent: @unchecked Sendable {
    case message(Message, revision: UInt64, expanded: Set<String>)
    case work(label: String, expanded: Bool)
    case activity(String)
    /// Fixed breathing room after the last row, above the composer.
    case end
}

/// Absolute frames inside one table cell, computed for an exact cell width.
struct TranscriptRowLayout: @unchecked Sendable {
    enum Element {
        case text(NSAttributedString, frame: NSRect, inBubble: Bool)
        case bubble(NSRect)
        /// Background frame; the code text sits below a label row inside it.
        case code(language: String?, source: String, text: NSAttributedString, frame: NSRect, textFrame: NSRect)
        case disclosure(key: String, title: String, symbol: String?, expanded: Bool, frame: NSRect)
        case activity(String, frame: NSRect)
    }
    var elements: [Element]
    var height: CGFloat

    static let rowInset: CGFloat = 24
    static let columnMaxWidth: CGFloat = 760
    static let headerHeight: CGFloat = 18
    static let blockSpacing: CGFloat = 9
    static let bubblePadding: CGFloat = 12
    static let bubbleMaxWidth: CGFloat = 600
    static let codePadding: CGFloat = 12
    static let codeLabelHeight: CGFloat = 18
    static let codeLabelGap: CGFloat = 6

    static func column(for width: CGFloat) -> (x: CGFloat, width: CGFloat) {
        // Integral frames: layer-backed text views pixel-align fractional
        // frames, which would change the width text was measured at.
        let columnWidth = floor(max(min(width - rowInset * 2, columnMaxWidth), 40))
        return (floor((width - columnWidth) / 2), columnWidth)
    }

    static func make(_ content: TranscriptRowContent, rendered: RenderedMessage?, width: CGFloat, measurer: TextMeasurer) -> TranscriptRowLayout {
        let column = column(for: width)
        var y = rowInset
        var elements: [Element] = []
        switch content {
        case .work(let label, let expanded):
            elements.append(.disclosure(key: "", title: label, symbol: nil, expanded: expanded, frame: NSRect(x: column.x, y: y, width: column.width, height: headerHeight)))
            y += headerHeight
        case .activity(let text):
            elements.append(.activity(text, frame: NSRect(x: column.x, y: y, width: column.width, height: headerHeight)))
            y += headerHeight
        case .end:
            break
        case .message(_, _, let expanded):
            guard let rendered else { preconditionFailure("A message row is laid out from its rendered revision") }
            if rendered.role == .user {
                let maxText = floor(max(min(bubbleMaxWidth, column.width - 60) - bubblePadding * 2, 20))
                var measured: [(NSAttributedString, NSSize)] = []
                for block in rendered.blocks {
                    // User content has no disclosures; render any defensively as plain text.
                    let text: NSAttributedString = switch block {
                    case .text(let text): text
                    case .code(_, let text, _, _): text
                    case .disclosure(_, let title, _, let detail, _): TranscriptTypography.plain(title + "\n" + detail.string)
                    }
                    measured.append((text, measurer.size(of: text, width: maxText)))
                }
                let textWidth = min(ceil(measured.map(\.1.width).max() ?? 0) + 1, maxText)
                var inner = y + bubblePadding
                var texts: [Element] = []
                for (index, (text, _)) in measured.enumerated() {
                    if index > 0 { inner += blockSpacing }
                    // Re-measure at the shrink-wrapped width that is actually displayed.
                    let height = measurer.size(of: text, width: textWidth).height
                    texts.append(.text(text, frame: NSRect(x: column.x + column.width - textWidth - bubblePadding, y: inner, width: textWidth, height: height), inBubble: true))
                    inner += height
                }
                let bubbleHeight = inner + bubblePadding - y
                elements.append(.bubble(NSRect(x: column.x + column.width - textWidth - bubblePadding * 2, y: y, width: textWidth + bubblePadding * 2, height: bubbleHeight)))
                elements += texts
                y += bubbleHeight
            } else {
                for (index, block) in rendered.blocks.enumerated() {
                    if index > 0 { y += blockSpacing }
                    switch block {
                    case .text(let text):
                        let height = measurer.size(of: text, width: column.width).height
                        elements.append(.text(text, frame: NSRect(x: column.x, y: y, width: column.width, height: height), inBubble: false))
                        y += height
                    case .code(let language, let text, let source, let indent):
                        let frameWidth = max(column.width - indent, codePadding * 2 + 20)
                        let textWidth = frameWidth - codePadding * 2
                        let textHeight = measurer.size(of: text, width: textWidth).height
                        let frame = NSRect(x: column.x + indent, y: y, width: frameWidth, height: codePadding * 2 + codeLabelHeight + codeLabelGap + textHeight)
                        let textFrame = NSRect(x: frame.minX + codePadding, y: y + codePadding + codeLabelHeight + codeLabelGap, width: textWidth, height: textHeight)
                        elements.append(.code(language: language, source: source, text: text, frame: frame, textFrame: textFrame))
                        y += frame.height
                    case .disclosure(let key, let title, let symbol, let detail, let detailMaxWidth):
                        let isExpanded = expanded.contains(key)
                        elements.append(.disclosure(key: key, title: title, symbol: symbol, expanded: isExpanded, frame: NSRect(x: column.x, y: y, width: column.width, height: headerHeight)))
                        y += headerHeight
                        if isExpanded {
                            y += 12
                            let detailWidth = min(column.width, detailMaxWidth ?? column.width)
                            let height = measurer.size(of: detail, width: detailWidth).height
                            elements.append(.text(detail, frame: NSRect(x: column.x, y: y, width: detailWidth, height: height), inBubble: false))
                            y += height
                        }
                    }
                }
            }
        }
        return TranscriptRowLayout(elements: elements, height: ceil(y))
    }

    /// Cheap pre-measurement guess; replaced by an exact layout before display.
    static func estimate(_ content: TranscriptRowContent, width: CGFloat) -> CGFloat {
        let column = column(for: width)
        switch content {
        case .work, .activity: return rowInset + headerHeight
        case .end: return rowInset
        case .message(let message, _, let expanded):
            let charactersPerLine = max(column.width / 7.5, 8)
            var height: CGFloat = 0
            for (index, block) in message.blocks.enumerated() {
                if index > 0 { height += blockSpacing }
                switch block {
                case .text(let text), .notice(let text):
                    // Linear byte scans only: this runs for every row on reload.
                    var newlines = 0
                    for byte in text.utf8 where byte == 10 { newlines += 1 }
                    let wrapped = ceil(CGFloat(text.utf16.count) / charactersPerLine)
                    height += CGFloat(max(newlines + 1, Int(wrapped))) * 19
                case .reasoning, .tool:
                    height += headerHeight
                    if !expanded.isEmpty { height += 60 }
                }
            }
            return rowInset + max(height, 19) + (message.role == .user ? bubblePadding * 2 : 0)
        }
    }
}

/// TextKit 1 measurement. Each call uses a fresh stack: a reused layout
/// manager retains block/table state from earlier strings and can disagree
/// with the display text view by a fraction of a point.
final class TextMeasurer {
    func size(of text: NSAttributedString, width: CGFloat) -> NSSize {
        let storage = NSTextStorage(attributedString: text)
        let layoutManager = NSLayoutManager()
        let container = NSTextContainer(size: NSSize(width: width, height: CGFloat.greatestFiniteMagnitude))
        container.lineFragmentPadding = 0
        layoutManager.addTextContainer(container)
        storage.addLayoutManager(layoutManager)
        layoutManager.ensureLayout(for: container)
        let used = layoutManager.usedRect(for: container)
        return NSSize(width: used.width, height: ceil(used.height))
    }
}
