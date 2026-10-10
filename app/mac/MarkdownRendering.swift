import AppKit
import Markdown

/// Typography shared by measurement and display. Both sides must read the same
/// values, otherwise a measured row height would not match its drawn text.
enum TranscriptTypography {
    static let body = NSFont.systemFont(ofSize: NSFont.systemFontSize)
    static let callout = NSFont.systemFont(ofSize: NSFont.systemFontSize - 1)
    static let caption = NSFont.systemFont(ofSize: NSFont.smallSystemFontSize - 1)
    static let code = NSFont.monospacedSystemFont(ofSize: NSFont.systemFontSize - 1, weight: .regular)
    static let lineSpacing: CGFloat = 2
    static let blockSpacing: CGFloat = 8

    static func paragraph(alignment: NSTextAlignment = .natural) -> NSMutableParagraphStyle {
        let style = NSMutableParagraphStyle()
        style.lineSpacing = lineSpacing
        style.alignment = alignment
        return style
    }

    static func plain(_ text: String, font: NSFont = body, color: NSColor = .labelColor, alignment: NSTextAlignment = .natural) -> NSAttributedString {
        NSAttributedString(string: text, attributes: [.font: font, .foregroundColor: color, .paragraphStyle: paragraph(alignment: alignment)])
    }
}

private extension NSFont {
    func adding(_ trait: NSFontDescriptor.SymbolicTraits) -> NSFont {
        NSFont(descriptor: fontDescriptor.withSymbolicTraits(fontDescriptor.symbolicTraits.union(trait)), size: pointSize) ?? self
    }
}

/// A top-level piece of rendered Markdown. Fenced code is its own segment so
/// the row layout owns its background, label row and copy control; TextKit
/// block backgrounds cannot separate adjacent code blocks reliably.
enum MarkdownSegment {
    case prose(NSAttributedString)
    case code(language: String?, text: NSAttributedString, source: String, indent: CGFloat)
}

/// Converts LLM Markdown into TextKit 1 attributed segments. Parsing belongs to
/// swift-markdown; this type only maps the typed tree to native text attributes.
/// It is pure and runs off the main thread.
struct MarkdownAttributedRenderer {
    private struct Paragraph {
        var text: NSMutableAttributedString
        var style: NSMutableParagraphStyle
    }
    private struct Context {
        var indent: CGFloat = 0
        var blocks: [NSTextBlock] = []
        var color: NSColor = .labelColor
        var listDepth = 0
    }
    private var paragraphs: [Paragraph] = []
    private var pendingSpacing: CGFloat = 0
    private var segments: [MarkdownSegment] = []

    static func render(_ source: String) -> [MarkdownSegment] {
        var renderer = MarkdownAttributedRenderer()
        let document = Document(parsing: source)
        for child in document.children { renderer.block(child, Context()) }
        renderer.flush()
        return renderer.segments
    }

    private mutating func flush() {
        guard !paragraphs.isEmpty else { return }
        segments.append(.prose(joined()))
        paragraphs = []
        pendingSpacing = 0
    }

    private func joined() -> NSAttributedString {
        let result = NSMutableAttributedString()
        for (index, paragraph) in paragraphs.enumerated() {
            let start = result.length
            result.append(paragraph.text)
            if index < paragraphs.count - 1 {
                // The terminator carries the paragraph's own attributes, so a
                // block background does not leak into the next paragraph.
                let attributes = paragraph.text.length > 0 ? paragraph.text.attributes(at: paragraph.text.length - 1, effectiveRange: nil) : [.font: TranscriptTypography.body]
                result.append(NSAttributedString(string: "\n", attributes: attributes))
            }
            result.addAttribute(.paragraphStyle, value: paragraph.style, range: NSRange(location: start, length: result.length - start))
        }
        return result
    }

    private mutating func emit(_ text: NSMutableAttributedString, _ style: NSMutableParagraphStyle, spacingAfter: CGFloat = TranscriptTypography.blockSpacing) {
        let spacing = paragraphs.isEmpty ? 0 : max(style.paragraphSpacingBefore, pendingSpacing)
        // TextKit paints a block's margins with its own background, so the gap
        // between blocks goes on whichever side does not paint it. A table
        // cell opens or closes its whole table, not just itself.
        func outer(_ block: NSTextBlock) -> NSTextBlock { (block as? NSTextTableBlock)?.table ?? block }
        let previous = (paragraphs.last?.style.textBlocks ?? []).map(outer)
        let current = style.textBlocks.map(outer)
        let closed = previous.last { block in !current.contains { $0 === block } && block.backgroundColor == nil }
        let opened = current.first { block in !previous.contains { $0 === block } && block.backgroundColor == nil }
        style.paragraphSpacingBefore = 0
        if spacing <= 0 {
        } else if let closed {
            closed.setWidth(spacing, type: .absoluteValueType, for: .margin, edge: .maxY)
        } else if let opened {
            opened.setWidth(spacing, type: .absoluteValueType, for: .margin, edge: .minY)
        } else if let last = paragraphs.last, last.style.textBlocks.isEmpty || current.isEmpty || previous.elementsEqual(current, by: ===) {
            // Plain text on either side carries the gap outside any block.
            if current.isEmpty || previous.elementsEqual(current, by: ===) { style.paragraphSpacingBefore = spacing }
            else { last.style.paragraphSpacing = max(last.style.paragraphSpacing, spacing) }
        } else {
            style.paragraphSpacingBefore = spacing
        }
        if text.length == 0 { text.append(NSAttributedString(string: " ", attributes: [.font: TranscriptTypography.body])) }
        paragraphs.append(Paragraph(text: text, style: style))
        pendingSpacing = spacingAfter
    }

    private func style(_ context: Context) -> NSMutableParagraphStyle {
        let style = TranscriptTypography.paragraph()
        style.firstLineHeadIndent = context.indent
        style.headIndent = context.indent
        style.textBlocks = context.blocks
        return style
    }

    private mutating func block(_ markup: Markup, _ context: Context) {
        switch markup {
        case let paragraph as Markdown.Paragraph:
            emit(inlines(paragraph, font: TranscriptTypography.body, context: context), style(context))
        case let heading as Heading:
            let sizes: [CGFloat] = [20, 17, 15, 13, 13, 13]
            let font = NSFont.systemFont(ofSize: sizes[min(max(heading.level, 1), 6) - 1], weight: .semibold)
            let style = style(context)
            style.paragraphSpacingBefore = 6
            emit(inlines(heading, font: font, context: context), style, spacingAfter: 6)
        case let code as CodeBlock:
            codeBlock(code, context)
        case let quote as BlockQuote:
            var inner = context
            let quoteBlock = NSTextBlock()
            quoteBlock.setWidth(3, type: .absoluteValueType, for: .border, edge: .minX)
            quoteBlock.setBorderColor(.tertiaryLabelColor, for: .minX)
            quoteBlock.setWidth(10, type: .absoluteValueType, for: .padding, edge: .minX)
            quoteBlock.setWidth(context.indent, type: .absoluteValueType, for: .margin, edge: .minX)
            // TextKit lays out a block only with an explicit content width.
            quoteBlock.setValue(100, type: .percentageValueType, for: .width)
            inner.blocks.append(quoteBlock)
            inner.indent = 0
            inner.color = .secondaryLabelColor
            for child in quote.children { block(child, inner) }
        case let list as UnorderedList:
            listItems(Array(list.listItems), ordered: nil, context)
        case let list as OrderedList:
            listItems(Array(list.listItems), ordered: Int(list.startIndex), context)
        case let table as Markdown.Table:
            self.table(table, context)
        case is ThematicBreak:
            let block = NSTextBlock()
            block.setWidth(1, type: .absoluteValueType, for: .border, edge: .maxY)
            block.setBorderColor(.separatorColor, for: .maxY)
            block.setValue(100, type: .percentageValueType, for: .width)
            block.setWidth(context.indent, type: .absoluteValueType, for: .margin, edge: .minX)
            let style = TranscriptTypography.paragraph()
            style.textBlocks = context.blocks + [block]
            style.minimumLineHeight = 4; style.maximumLineHeight = 4; style.lineSpacing = 0
            emit(NSMutableAttributedString(string: " ", attributes: [.font: NSFont.systemFont(ofSize: 2)]), style)
        case let html as HTMLBlock:
            let text = html.rawHTML.trimmingCharacters(in: .newlines)
            emit(NSMutableAttributedString(string: text, attributes: [.font: TranscriptTypography.code, .foregroundColor: NSColor.secondaryLabelColor]), style(context))
        default:
            // Unknown block containers still keep their readable content.
            if markup.childCount > 0 { for child in markup.children { block(child, context) } }
            else if let text = (markup as? PlainTextConvertibleMarkup)?.plainText, !text.isEmpty {
                emit(NSMutableAttributedString(string: text, attributes: [.font: TranscriptTypography.body, .foregroundColor: context.color]), style(context))
            }
        }
    }

    private mutating func listItems(_ items: [ListItem], ordered start: Int?, _ context: Context) {
        let bullets = ["•", "◦", "▪︎"]
        let markerWidth: CGFloat = start == nil ? 16 : 24
        for (offset, item) in items.enumerated() {
            var marker: String
            if let start { marker = "\(start + offset)." } else { marker = bullets[context.listDepth % bullets.count] }
            switch item.checkbox {
            case .checked: marker = "☑︎"
            case .unchecked: marker = "☐"
            case nil: break
            }
            var inner = context
            inner.indent = context.indent + markerWidth
            inner.listDepth = context.listDepth + 1
            var first = true
            for child in item.children {
                if first, let paragraph = child as? Markdown.Paragraph {
                    let text = NSMutableAttributedString(string: marker + "\t", attributes: [.font: TranscriptTypography.body, .foregroundColor: context.color == .labelColor ? NSColor.secondaryLabelColor : context.color])
                    text.append(inlines(paragraph, font: TranscriptTypography.body, context: context))
                    let style = self.style(inner)
                    style.firstLineHeadIndent = context.indent
                    style.tabStops = [NSTextTab(textAlignment: .left, location: inner.indent)]
                    style.defaultTabInterval = markerWidth
                    if offset > 0 { style.paragraphSpacingBefore = 3 }
                    emit(text, style, spacingAfter: 3)
                } else {
                    block(child, inner)
                }
                first = false
            }
        }
        pendingSpacing = TranscriptTypography.blockSpacing
    }

    private mutating func codeBlock(_ code: CodeBlock, _ context: Context) {
        flush()
        let source = code.code.hasSuffix("\n") ? String(code.code.dropLast()) : code.code
        let style = TranscriptTypography.paragraph()
        style.lineSpacing = 1
        style.lineBreakMode = .byCharWrapping
        let text = NSAttributedString(string: source, attributes: [.font: TranscriptTypography.code, .foregroundColor: NSColor.labelColor, .paragraphStyle: style])
        let language = code.language.flatMap { $0.isEmpty ? nil : $0 }
        segments.append(.code(language: language, text: text, source: source, indent: context.indent))
    }

    private mutating func table(_ table: Markdown.Table, _ context: Context) {
        let columns = max(table.maxColumnCount, 1)
        let textTable = NSTextTable()
        textTable.numberOfColumns = columns
        textTable.collapsesBorders = true
        textTable.layoutAlgorithm = .automaticLayoutAlgorithm
        textTable.setWidth(context.indent, type: .absoluteValueType, for: .margin, edge: .minX)
        let rows: [[Markdown.Table.Cell]] = [Array(table.head.cells)] + table.body.rows.map { Array($0.cells) }
        let alignments = table.columnAlignments
        for (rowIndex, row) in rows.enumerated() {
            var column = 0
            for cell in row where column < columns {
                let span = max(Int(cell.colspan), 1)
                guard cell.rowspan > 0 else { column += span; continue }
                let cellBlock = NSTextTableBlock(table: textTable, startingRow: rowIndex, rowSpan: max(Int(cell.rowspan), 1), startingColumn: column, columnSpan: min(span, columns - column))
                cellBlock.setWidth(0.5, type: .absoluteValueType, for: .border)
                cellBlock.setBorderColor(.separatorColor)
                cellBlock.setWidth(6, type: .absoluteValueType, for: .padding)
                if rowIndex == 0 { cellBlock.backgroundColor = .quaternarySystemFill }
                let style = TranscriptTypography.paragraph()
                style.textBlocks = context.blocks + [cellBlock]
                switch column < alignments.count ? alignments[column] : nil {
                case .center: style.alignment = .center
                case .right: style.alignment = .right
                default: style.alignment = .natural
                }
                let font = rowIndex == 0 ? NSFont.systemFont(ofSize: NSFont.systemFontSize, weight: .semibold) : TranscriptTypography.body
                emit(inlines(cell, font: font, context: context), style, spacingAfter: 0)
                column += span
            }
        }
        pendingSpacing = TranscriptTypography.blockSpacing
    }

    private func inlines(_ markup: Markup, font: NSFont, context: Context) -> NSMutableAttributedString {
        let result = NSMutableAttributedString()
        func walk(_ node: Markup, _ attributes: [NSAttributedString.Key: Any]) {
            let font = attributes[.font] as? NSFont ?? TranscriptTypography.body
            switch node {
            case let text as Markdown.Text:
                result.append(NSAttributedString(string: text.string, attributes: attributes))
            case is SoftBreak:
                result.append(NSAttributedString(string: " ", attributes: attributes))
            case is LineBreak:
                result.append(NSAttributedString(string: "\u{2028}", attributes: attributes))
            case let code as InlineCode:
                var attributes = attributes
                attributes[.font] = NSFont.monospacedSystemFont(ofSize: font.pointSize - 1, weight: .regular)
                attributes[.backgroundColor] = NSColor.quaternarySystemFill
                result.append(NSAttributedString(string: code.code, attributes: attributes))
            case is Emphasis:
                var attributes = attributes
                attributes[.font] = font.adding(.italic)
                for child in node.children { walk(child, attributes) }
            case is Strong:
                var attributes = attributes
                attributes[.font] = font.adding(.bold)
                for child in node.children { walk(child, attributes) }
            case is Strikethrough:
                var attributes = attributes
                attributes[.strikethroughStyle] = NSUnderlineStyle.single.rawValue
                for child in node.children { walk(child, attributes) }
            case let link as Markdown.Link:
                var attributes = attributes
                if let destination = link.destination { attributes[.link] = destination }
                for child in node.children { walk(child, attributes) }
            case let image as Markdown.Image:
                var attributes = attributes
                if let source = image.source { attributes[.link] = source }
                let alt = image.plainText
                result.append(NSAttributedString(string: alt.isEmpty ? (image.source ?? "图片") : alt, attributes: attributes))
            case let html as InlineHTML:
                result.append(NSAttributedString(string: html.rawHTML, attributes: attributes))
            default:
                if node.childCount > 0 { for child in node.children { walk(child, attributes) } }
                else if let text = (node as? PlainTextConvertibleMarkup)?.plainText { result.append(NSAttributedString(string: text, attributes: attributes)) }
            }
        }
        let base: [NSAttributedString.Key: Any] = [.font: font, .foregroundColor: context.color]
        for child in markup.children { walk(child, base) }
        return result
    }
}
