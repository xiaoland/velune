import SwiftUI
import Markdown

struct MarkdownText: View {
    let source: String
    var body: some View {
        VStack(alignment: .leading, spacing: 10) {
            ForEach(Array(Document(parsing: source).children.enumerated()), id: \.offset) { _, node in
                MarkdownNode(node: node)
            }
        }.textSelection(.enabled)
    }
}

private struct MarkdownNode: View {
    let node: any Markup
    var body: some View {
        if let code = node as? CodeBlock {
            VStack(alignment: .leading, spacing: 8) {
                HStack {
                    Text(code.language ?? "代码").font(.caption).foregroundStyle(.secondary)
                    Spacer()
                    Button { UIPasteboard.general.string = code.code } label: { Image(systemName: "doc.on.doc") }
                        .accessibilityLabel("复制代码")
                }
                ScrollView(.horizontal) { Text(code.code).font(.system(.callout, design: .monospaced)).fixedSize(horizontal: true, vertical: false) }
            }.padding(12).background(.quaternary, in: RoundedRectangle(cornerRadius: 10))
        } else if let table = node as? Markdown.Table {
            ScrollView(.horizontal) {
                Grid(alignment: .leading, horizontalSpacing: 20, verticalSpacing: 8) {
                    GridRow {
                        ForEach(Array(table.head.children.enumerated()), id: \.offset) { _, cell in
                            Text(.init(cell.children.map { $0.format() }.joined())).bold()
                        }
                    }
                    Divider()
                    ForEach(Array(table.body.children.enumerated()), id: \.offset) { _, row in
                        GridRow {
                            ForEach(Array(row.children.enumerated()), id: \.offset) { _, cell in Text(.init(cell.children.map { $0.format() }.joined())) }
                        }
                    }
                }.fixedSize(horizontal: true, vertical: false).padding(10)
            }
            .contextMenu { Button("复制表格") { UIPasteboard.general.string = node.format() } }
        } else if let heading = node as? Heading {
            Text(.init(heading.children.map { $0.format() }.joined())).font(heading.level <= 2 ? .title3 : .headline).bold()
        } else if node is UnorderedList || node is OrderedList {
            VStack(alignment: .leading, spacing: 8) {
                ForEach(Array(node.children.enumerated()), id: \.offset) { index, child in
                    HStack(alignment: .top) {
                        Text(node is OrderedList ? "\((node as? OrderedList).map { Int($0.startIndex) + index } ?? (1 + index))." : "•")
                        MarkdownNode(node: child)
                    }
                }
            }
        } else if node is ListItem || node is BlockQuote {
            VStack(alignment: .leading, spacing: 8) {
                ForEach(Array(node.children.enumerated()), id: \.offset) { _, child in MarkdownNode(node: child) }
            }.padding(.leading, node is BlockQuote ? 12 : 0)
        } else if node is ThematicBreak { Divider() }
        else { Text(.init(node.format())).frame(maxWidth: .infinity, alignment: .leading) }
    }
}

/// Block boundaries come from the core's turn projection, including shared work/reply messages.
func transcriptBlocks(_ message: BindingMessage, turn: BindingTranscriptTurn?, reply: Bool) -> [BindingMessageBlock] {
    guard let turn, turn.lastMessageId == message.id, turn.replyStartBlock > 0 else { return message.blocks }
    return reply ? Array(message.blocks.dropFirst(Int(turn.replyStartBlock))) : Array(message.blocks.prefix(Int(turn.replyStartBlock)))
}

struct TranscriptView: View {
    @ObservedObject var model: ConversationModel
    var body: some View {
        LazyVStack(alignment: .leading, spacing: 20) {
            ForEach(model.snapshot?.transcriptItems ?? [], id: \.self) { item in
                switch item {
                case .message(let id, let messageID):
                    if let message = model.message(messageID) {
                        let turn = model.snapshot?.transcriptTurns.first { $0.lastMessageId == messageID }
                        MessageView(message: message, blocks: id.hasPrefix("reply:") ? transcriptBlocks(message, turn: turn, reply: true) : message.blocks, model: model)
                    }
                case .work(let turnID):
                    if let turn = model.snapshot?.transcriptTurns.first(where: { $0.id == turnID }) {
                        DisclosureGroup {
                            ForEach(turn.workMessageIds, id: \.self) { id in
                                if let message = model.message(id) {
                                    MessageView(message: message, blocks: transcriptBlocks(message, turn: turn, reply: false), model: model)
                                }
                            }
                        } label: {
                            HStack {
                                if turn.isRunning { ProgressView().controlSize(.small) }
                                Text("工作过程")
                                if let duration = turn.durationMs { Text("\(duration / 1000) 秒").foregroundStyle(.secondary) }
                            }.font(.callout)
                        }
                    }
                }
            }
            if let sending = model.sending {
                HStack { Spacer(); VStack(alignment: .trailing) { Text(sending); Text("发送中").font(.caption).foregroundStyle(.secondary) } }
            }
        }
    }
}

private struct MessageView: View {
    let message: BindingMessage
    let blocks: [BindingMessageBlock]
    @ObservedObject var model: ConversationModel
    var body: some View {
        HStack(alignment: .top) {
            if message.role == .user { Spacer(minLength: 40) }
            VStack(alignment: .leading, spacing: 10) {
                ForEach(Array(blocks.enumerated()), id: \.offset) { _, block in
                    switch block {
                    case .text(let text): MarkdownText(source: text)
                    case .notice(let text): Text(text).font(.callout).foregroundStyle(.secondary).frame(maxWidth: .infinity)
                    case .reasoning(let summary, let payload):
                        DisclosureGroup(summary.isEmpty ? "思考" : summary) { PayloadView(payload: payload, messageID: message.id, model: model) }
                    case .tool(_, let title, let state, let payload, _):
                        DisclosureGroup {
                            if let payload { PayloadView(payload: payload, messageID: message.id, model: model) }
                        } label: { Label(title, systemImage: state == .failed ? "exclamationmark.triangle" : "terminal") }
                    case .file(let path): Label(URL(fileURLWithPath: path).lastPathComponent, systemImage: "doc")
                    case .image: Label("图片附件", systemImage: "photo")
                    case .agentSender(_, let runtime, let title): Text("\(runtime) · \(title)").font(.caption).foregroundStyle(.secondary)
                    }
                }
            }
            .padding(message.role == .user ? 12 : 0)
            .background(message.role == .user ? Color.accentColor.opacity(0.12) : .clear, in: RoundedRectangle(cornerRadius: 16))
            if message.role != .user { Spacer(minLength: 0) }
        }
    }
}

private struct PayloadView: View {
    let payload: BindingMessagePayload
    let messageID: String
    @ObservedObject var model: ConversationModel
    var body: some View {
        switch payload {
        case .inline(let text): MarkdownText(source: text)
        case .deferred(let bytes):
            Button("读取完整内容（\(bytes / 1024) KB）") { Task { await model.loadPayload(messageID) } }
        }
    }
}
