import SwiftUI
import MarkdownUI

struct MarkdownText: View {
    let source: String
    var body: some View {
        Markdown(source)
            .markdownBlockStyle(\.codeBlock) { code in
                VStack(alignment: .leading, spacing: 8) {
                    HStack {
                        Text(code.language ?? "代码").font(.caption).foregroundStyle(.secondary)
                        Spacer()
                        Button { UIPasteboard.general.string = code.content } label: { Image(systemName: "doc.on.doc").frame(width: 44, height: 44) }
                            .accessibilityLabel("复制代码")
                    }
                    ScrollView(.horizontal) {
                        code.label.fixedSize(horizontal: true, vertical: false)
                            .markdownTextStyle { FontFamilyVariant(.monospaced); FontSize(.em(0.85)) }
                    }
                }.padding(12).background(Color(.secondarySystemBackground), in: RoundedRectangle(cornerRadius: 12))
                .markdownMargin(top: 0, bottom: 16)
            }
            .markdownBlockStyle(\.table) { table in
                ScrollView(.horizontal) {
                    table.label.fixedSize(horizontal: true, vertical: false)
                        .markdownTableBorderStyle(.init(color: Color(.separator)))
                }
                    .contextMenu { Button("复制表格") { UIPasteboard.general.string = table.content.renderMarkdown() } }
                .markdownMargin(top: 0, bottom: 16)
            }
            .markdownTheme(.gitHub)
            .textSelection(.enabled)
    }
}

/// Block boundaries come from the core's turn projection, including shared work/reply messages.
func transcriptBlocks(_ message: DeliveredMessage, turn: TranscriptTurn?, reply: Bool) -> [DeliveredBlock] {
    guard let turn, turn.lastMessageId == message.id, turn.replyStartBlock > 0 else { return message.blocks }
    return reply ? Array(message.blocks.dropFirst(Int(turn.replyStartBlock))) : Array(message.blocks.prefix(Int(turn.replyStartBlock)))
}

struct TranscriptView: View {
    @ObservedObject var model: ConversationModel
    var openLinked: (String, Bool) -> Void = { _, _ in }
    var body: some View {
        LazyVStack(alignment: .leading, spacing: 20) {
            ForEach(model.snapshot?.transcriptItems ?? [], id: \.self) { item in
                switch item {
                case .message(let id, let messageID):
                    if let message = model.message(messageID) {
                        let turn = model.snapshot?.transcriptTurns.first { $0.lastMessageId == messageID }
                        MessageView(message: message, blocks: id.hasPrefix("reply:") ? transcriptBlocks(message, turn: turn, reply: true) : message.blocks, model: model, openLinked: openLinked).id(id)
                    }
                case .work(let turnID):
                    if let turn = model.snapshot?.transcriptTurns.first(where: { $0.id == turnID }) {
                        DisclosureGroup {
                            ForEach(turn.workMessageIds, id: \.self) { id in
                                if let message = model.message(id) {
                                    MessageView(message: message, blocks: transcriptBlocks(message, turn: turn, reply: false), model: model, openLinked: openLinked)
                                }
                            }
                        } label: {
                            HStack {
                                if turn.isRunning { ProgressView().controlSize(.small) }
                                Text(turn.isRunning ? "正在处理" : "处理过程")
                                if let duration = turn.durationMs { Text("\(duration / 1000) 秒").foregroundStyle(.secondary) }
                            }.font(.callout).foregroundStyle(.secondary)
                        }
                        .tint(.secondary)
                        .padding(.vertical, 6).id("work:" + turnID)
                    }
                }
            }
            if let sending = model.sending {
                HStack { Spacer(); VStack(alignment: .trailing) { Text(sending); Text("发送中").font(.caption).foregroundStyle(.secondary) } }
            }
        }.scrollTargetLayout()
    }
}

private struct MessageView: View {
    let message: DeliveredMessage
    let blocks: [DeliveredBlock]
    @ObservedObject var model: ConversationModel
    var openLinked: (String, Bool) -> Void
    var body: some View {
        HStack(alignment: .top) {
            if message.role == .user { Spacer(minLength: 40) }
            VStack(alignment: .leading, spacing: 10) {
                ForEach(Array(blocks.enumerated()), id: \.offset) { _, block in
                    switch block {
                    case .text(let text):
                        if message.role == .user { Text(text).textSelection(.enabled) }
                        else { MarkdownText(source: text) }
                    case .notice(let text), .contextCompaction(let text): Text(text).font(.callout).foregroundStyle(.secondary).frame(maxWidth: .infinity)
                    case .reasoning(let summary, let payload):
                        DisclosureGroup(summary.isEmpty ? "思考" : summary) { PayloadView(payload: payload, messageID: message.id, model: model) }
                    case .tool(_, let title, let state, let payload, let linked):
                        DisclosureGroup {
                            if let linked { Button("打开会话", systemImage: "arrow.up.forward.app") { openLinked(linked, false) } }
                            if let payload { PayloadView(payload: payload, messageID: message.id, model: model) }
                        } label: {
                            HStack {
                                Image(systemName: "terminal")
                                Text(title).lineLimit(2)
                                Spacer()
                                if state == .completed { Image(systemName: "checkmark.circle").foregroundStyle(.green) }
                                else if state == .failed { Image(systemName: "exclamationmark.circle").foregroundStyle(.red) }
                                else { ProgressView().controlSize(.small) }
                            }.font(.callout)
                        }
                        .padding(10).background(Color(.tertiarySystemBackground), in: RoundedRectangle(cornerRadius: 10))
                    case .file(let path): Label(URL(fileURLWithPath: path).lastPathComponent, systemImage: "doc")
                    case .image: Label("图片附件", systemImage: "photo")
                    case .agentSender(let id, let runtime, let title):
                        Button { openLinked(id, false) } label: { Label("\(runtime) · \(title)", systemImage: "arrow.turn.down.right") }.font(.caption)
                    }
                }
            }
            .padding(message.role == .user ? 12 : 0)
            .background(message.role == .user ? Color.accentColor.opacity(0.12) : .clear, in: RoundedRectangle(cornerRadius: 16))
            if message.role != .user { Spacer(minLength: 0) }
        }
        .contextMenu {
            Button("复制消息", systemImage: "doc.on.doc") {
                UIPasteboard.general.string = blocks.compactMap { if case .text(let text) = $0 { return text }; return nil }.joined(separator: "\n\n")
            }
        }
    }
}

private struct PayloadView: View {
    let payload: MessagePayload
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
