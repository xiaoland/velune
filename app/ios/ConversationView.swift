import SwiftUI

struct ConversationView: View {
    @StateObject private var model: ConversationModel
    @Environment(\.scenePhase) private var phase
    @State private var atBottom = true
    init(client: RemoteClient, peer: String, target: ConversationTarget) {
        _model = StateObject(wrappedValue: ConversationModel(client: client, peer: peer, target: target))
    }
    var body: some View {
        ScrollViewReader { proxy in
            ScrollView {
                VStack(spacing: 16) {
                    if model.loading { ProgressView() }
                    TranscriptView(model: model)
                    Color.clear.frame(height: 1).id("bottom")
                        .onAppear { atBottom = true }.onDisappear { atBottom = false }
                }.padding().frame(maxWidth: 900).frame(maxWidth: .infinity)
            }
            .defaultScrollAnchor(.bottom)
            .overlay(alignment: .bottomTrailing) {
                if !atBottom {
                    Button { withAnimation { proxy.scrollTo("bottom", anchor: .bottom) } } label: { Image(systemName: "arrow.down").padding(12) }
                        .buttonStyle(.borderedProminent).clipShape(Circle()).padding().accessibilityLabel("回到最新")
                }
            }
            .onChange(of: model.snapshot?.cursor) { _, _ in if atBottom { proxy.scrollTo("bottom", anchor: .bottom) } }
            .onChange(of: model.sending) { _, value in if value != nil { proxy.scrollTo("bottom", anchor: .bottom) } }
        }
        .navigationTitle(model.snapshot?.conversation.title ?? model.target.title)
        .navigationBarTitleDisplayMode(.inline)
        .safeAreaInset(edge: .bottom) {
            VStack(spacing: 8) {
                if let error = model.failure { FailureView(text: error) }
                ForEach(model.snapshot?.pendingInteractions ?? [], id: \.id) { interaction in
                    InteractionView(interaction: interaction, busy: model.busy) { model.reply(interaction.id, $0) }
                }
                HStack(alignment: .bottom, spacing: 10) {
                    VStack(alignment: .leading, spacing: 6) {
                        TextField("发送消息", text: $model.draft, axis: .vertical).lineLimit(1...8)
                        if !model.models.isEmpty {
                            Picker("模型", selection: $model.selection) {
                                Text("运行时默认").tag(nil as BindingModelSelection?)
                                ForEach(model.models, id: \.selection) { Text($0.name).tag(Optional($0.selection)) }
                            }.labelsHidden().font(.caption).disabled(model.busy)
                        }
                    }
                    if model.snapshot?.actions.canCancel == true {
                        Button(action: model.stop) { Image(systemName: "stop.fill") }.accessibilityLabel("停止")
                    } else {
                        Button(action: model.send) { Image(systemName: "arrow.up.circle.fill").font(.title) }
                            .accessibilityLabel("发送")
                            .disabled(model.loading || (model.target.conversation != nil && model.snapshot == nil) || model.draft.trimmingCharacters(in: .whitespacesAndNewlines).isEmpty || model.snapshot?.actions.canSend == false)
                    }
                }.padding(12).background(.quaternary, in: RoundedRectangle(cornerRadius: 20)).disabled(model.busy)
            }.padding(.horizontal).padding(.vertical, 8).background(.bar)
        }
        .task(id: phase) {
            guard phase == .active else { return }
            if model.loading { await model.open() }
            while !Task.isCancelled {
                do { try await Task.sleep(for: .milliseconds(model.snapshot?.runState == .running ? 800 : 3000)) }
                catch { return }
                await model.poll()
            }
        }
    }
}

private struct InteractionView: View {
    let interaction: BindingRuntimeInteraction
    let busy: Bool
    let reply: (BindingRuntimeInteractionReply) -> Void
    @State private var answers: [String: String] = [:]
    var body: some View {
        VStack(alignment: .leading, spacing: 10) {
            switch interaction.kind {
            case .approval(let title, let detail, let options):
                Text(title).font(.headline)
                if !detail.isEmpty { Text(detail).font(.callout).textSelection(.enabled) }
                ViewThatFits {
                    HStack { approvalButtons(options) }
                    VStack(alignment: .leading) { approvalButtons(options) }
                }
            case .userInput(let questions):
                ForEach(questions, id: \.id) { question in
                    Text(question.text).font(.callout)
                    if question.secret {
                        SecureField("回答", text: answer(question.id)).textFieldStyle(.roundedBorder)
                    } else {
                        TextField("回答", text: answer(question.id), axis: .vertical).textFieldStyle(.roundedBorder)
                    }
                    if !question.options.isEmpty {
                        ScrollView(.horizontal) {
                            HStack { ForEach(question.options, id: \.id) { option in
                                Button(option.label) { answers[question.id] = option.id }.buttonStyle(.bordered)
                            } }
                        }
                    }
                }
                HStack {
                    Button("取消") { reply(.cancel) }
                    Spacer()
                    Button("提交") { reply(.answers(answers: questions.map { .init(questionId: $0.id, values: [answers[$0.id] ?? ""]) })) }
                        .disabled(questions.contains { (answers[$0.id] ?? "").trimmingCharacters(in: .whitespacesAndNewlines).isEmpty })
                }
            }
        }.padding(12).background(.quaternary, in: RoundedRectangle(cornerRadius: 12)).disabled(busy)
    }
    private func answer(_ id: String) -> Binding<String> { Binding(get: { answers[id] ?? "" }, set: { answers[id] = $0 }) }
    private func approvalButtons(_ options: [BindingInteractionOption]) -> some View {
        ForEach(options, id: \.id) { option in Button(option.label) { reply(.decision(optionId: option.id)) }.buttonStyle(.bordered) }
    }
}

struct FailureView: View {
    let text: String
    var body: some View { Label(text, systemImage: "exclamationmark.triangle").font(.callout).foregroundStyle(.red).textSelection(.enabled) }
}
