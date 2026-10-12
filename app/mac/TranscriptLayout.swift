import AppKit

/// Width-independent native content of one message revision. Built once per
/// revision (normally off the main thread) and shared by measurement and display.
final class RenderedMessage: @unchecked Sendable {
    enum Block {
        case text(NSAttributedString)
        case code(language: String?, text: NSAttributedString, source: String, indent: CGFloat)
        /// A nil detail has nothing to expand.
        case disclosure(key: String, title: String, symbol: String?, detail: NSAttributedString?, detailMaxWidth: CGFloat?)
        case attachment(TranscriptAttachment)
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
            case .reasoning(let summary, let text):
                // A summary without further text is a label only.
                let detail: NSAttributedString? = if case .inline("") = text { nil } else { Self.detail(text, font: TranscriptTypography.callout, empty: "") }
                return [.disclosure(key: Self.disclosureKey(block, at: index)!, title: summary, symbol: nil, detail: detail, detailMaxWidth: nil)]
            case .tool(_, let title, let state, let output):
                let symbol = switch state {
                case .running: "circle.dotted"
                case .failed: "exclamationmark.circle"
                case .pending: "circle"
                case .completed: "checkmark.circle"
                }
                return [.disclosure(key: Self.disclosureKey(block, at: index)!, title: title, symbol: symbol,
                                    detail: Self.detail(output ?? .inline(""), font: TranscriptTypography.code, empty: "没有附加输出"), detailMaxWidth: 560)]
            case .image(let mediaType, .path(let path)):
                return [.attachment(TranscriptAttachment(name: URL(fileURLWithPath: path).lastPathComponent, content: .imageFile(path), mediaType: mediaType))]
            case .image(let mediaType, .data(.inline(let data))):
                return [.attachment(TranscriptAttachment(name: "图片", content: .imageData(data), mediaType: mediaType))]
            case .image(let mediaType, .data(.deferred)):
                return [.attachment(TranscriptAttachment(name: "图片", content: .imagePending, mediaType: mediaType))]
            case .file(let path):
                return [.attachment(TranscriptAttachment(name: URL(fileURLWithPath: path).lastPathComponent, content: .file(path), mediaType: nil))]
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
        case .text, .notice, .image, .file: return nil
        }
    }
    /// Disclosures whose content has not been read from the core yet.
    static func deferredDisclosureKeys(_ message: Message) -> Set<String> {
        Set(message.blocks.enumerated().compactMap { index, block in
            switch block {
            case .reasoning(_, .deferred), .tool(_, _, _, .deferred?): return disclosureKey(block, at: index)
            case .text, .notice, .reasoning(_, .inline), .tool, .image, .file: return nil
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
    case activity(TranscriptActivity)
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
        /// `expanded` is nil when there is nothing to expand.
        case disclosure(key: String, title: String, symbol: String?, expanded: Bool?, frame: NSRect)
        case activity(TranscriptActivity, frame: NSRect)
        case attachment(TranscriptAttachment, frame: NSRect)
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
    static let attachmentHeight: CGFloat = 48
    static let attachmentThumbnail: CGFloat = 40
    static let attachmentNameMaxWidth: CGFloat = 180
    static let attachmentSpacing: CGFloat = 6

    /// Attachments flow in rows of chips, against the trailing edge for the
    /// user's own messages.
    static func attachmentElements(_ items: [TranscriptAttachment], column: (x: CGFloat, width: CGFloat), trailing: Bool, y: inout CGFloat, measurer: TextMeasurer) -> [Element] {
        var rows: [[(TranscriptAttachment, CGFloat)]] = [[]]
        var used: CGFloat = 0
        for item in items {
            let name = measurer.size(of: TranscriptTypography.plain(item.name, font: TranscriptTypography.callout), width: 10_000).width
            // A label draws its text inset by a few points on each side.
            let width = min(4 + attachmentThumbnail + 8 + min(ceil(name) + 6, attachmentNameMaxWidth) + 10, column.width)
            if !rows[rows.count - 1].isEmpty, used + attachmentSpacing + width > column.width { rows.append([]); used = 0 }
            used += (rows[rows.count - 1].isEmpty ? 0 : attachmentSpacing) + width
            rows[rows.count - 1].append((item, width))
        }
        var elements: [Element] = []
        for (index, row) in rows.enumerated() {
            if index > 0 { y += attachmentSpacing }
            let total = row.map(\.1).reduce(0, +) + attachmentSpacing * CGFloat(max(row.count - 1, 0))
            var x = trailing ? column.x + column.width - total : column.x
            for (item, width) in row {
                elements.append(.attachment(item, frame: NSRect(x: x, y: y, width: width, height: attachmentHeight)))
                x += width + attachmentSpacing
            }
            y += attachmentHeight
        }
        return elements
    }

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
        case .activity(let activity):
            elements.append(.activity(activity, frame: NSRect(x: column.x, y: y, width: column.width, height: headerHeight)))
            y += headerHeight
        case .end:
            break
        case .message(_, _, let expanded):
            guard let rendered else { preconditionFailure("A message row is laid out from its rendered revision") }
            if rendered.role == .user {
                // Images lead files regardless of which part of the native record carried them.
                let items = rendered.blocks.compactMap { if case .attachment(let item) = $0 { return item }; return nil }
                let attachments = items.filter(\.isImage) + items.filter { !$0.isImage }
                if !attachments.isEmpty {
                    elements += attachmentElements(attachments, column: column, trailing: true, y: &y, measurer: measurer)
                }
                let maxText = floor(max(min(bubbleMaxWidth, column.width - 60) - bubblePadding * 2, 20))
                var measured: [(NSAttributedString, NSSize)] = []
                for block in rendered.blocks {
                    // User content has no disclosures; render any defensively as plain text.
                    let text: NSAttributedString
                    switch block {
                    case .text(let value): text = value
                    case .code(_, let value, _, _): text = value
                    case .disclosure(_, let title, _, let detail, _): text = TranscriptTypography.plain(title + (detail.map { "\n" + $0.string } ?? ""))
                    case .attachment: continue
                    }
                    measured.append((text, measurer.size(of: text, width: maxText)))
                }
                if measured.isEmpty { return TranscriptRowLayout(elements: elements, height: ceil(y)) }
                if !attachments.isEmpty { y += blockSpacing }
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
                    case .attachment(let item):
                        elements += attachmentElements([item], column: column, trailing: false, y: &y, measurer: measurer)
                    case .disclosure(let key, let title, let symbol, let detail, let detailMaxWidth):
                        let isExpanded = detail != nil && expanded.contains(key)
                        elements.append(.disclosure(key: key, title: title, symbol: symbol, expanded: detail == nil ? nil : isExpanded, frame: NSRect(x: column.x, y: y, width: column.width, height: headerHeight)))
                        y += headerHeight
                        if isExpanded, let detail {
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
                case .image, .file:
                    height += attachmentHeight
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

/// A file attached to a message, as shown in the transcript.
struct TranscriptAttachment: Sendable, Equatable {
    enum Content: Sendable, Equatable {
        case imageFile(String)
        /// Base64 image content.
        case imageData(String)
        /// Image content not read from the core yet.
        case imagePending
        case file(String)
    }
    let name: String
    let content: Content
    let mediaType: String?

    var isImage: Bool {
        if case .file = content { return false }
        return true
    }
}
