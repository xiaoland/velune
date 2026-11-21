import AppKit
import SwiftUI
import VeluneBindings

/// Unsubmitted answers live with the conversation store, not its mounted view.
/// Deliberately memory-only: passwords and partial answers never enter persistence.
@MainActor final class RuntimeInteractionDraft: ObservableObject {
    @Published var values: [String: String] = [:]
    @Published var selectedOptions: [String: Set<String>] = [:]
    @Published var otherAnswers: Set<String> = []
    @Published var page = 0

    init(interaction: RuntimeInteraction? = nil) {
        guard let interaction else { return }
        let questions: [InteractionQuestion]
        switch interaction.kind {
        case .userInput(let items), .asyncUserInput(let items, _): questions = items
        case .approval: questions = []
        }
        for question in questions {
            guard let value = question.defaultText else { continue }
            if question.options.isEmpty { values[question.id] = value }
            else if question.options.contains(where: { $0.id == value }) { selectedOptions[question.id] = [value] }
            else if question.allowOther { values[question.id] = value; otherAnswers.insert(question.id) }
        }
    }
    func answerValues(_ question: InteractionQuestion) -> [String] {
        var result = question.options.filter { selectedOptions[question.id, default: []].contains($0.id) }.map(\.id)
        if question.options.isEmpty || (question.allowOther && (question.multiple || otherAnswers.contains(question.id))), let value = values[question.id], !value.trimmingCharacters(in: .whitespacesAndNewlines).isEmpty { result.append(value) }
        return result
    }
    func clear() {
        values = [:]; selectedOptions = [:]; otherAnswers = []; page = 0
    }
}

struct RuntimeInteractionView: View {
    @ObservedObject var store: ConversationStore
    @ObservedObject var tab: ConversationTab
    let interaction: RuntimeInteraction
    var maximumBodyHeight: CGFloat = 220
    var contentWidth: CGFloat = 760

    var body: some View {
        RuntimeInteractionForm(store: store, tab: tab, interaction: interaction,
            maximumBodyHeight: maximumBodyHeight, contentWidth: contentWidth,
            draft: store.interactionDraft(for: interaction))
            .id(store.loadedConversationID)
    }
}

/// A bounded form above the composer. Focus changes only the native focus ring;
/// it never inserts content or changes the space allocated to the conversation.
private struct RuntimeInteractionForm: View {
    @ObservedObject var store: ConversationStore
    @ObservedObject var tab: ConversationTab
    let interaction: RuntimeInteraction
    var maximumBodyHeight: CGFloat = 220
    var contentWidth: CGFloat = 760
    @ObservedObject var draft: RuntimeInteractionDraft
    private var values: [String: String] { get { draft.values } nonmutating set { draft.values = newValue } }
    private var selectedOptions: [String: Set<String>] { get { draft.selectedOptions } nonmutating set { draft.selectedOptions = newValue } }
    private var otherAnswers: Set<String> { get { draft.otherAnswers } nonmutating set { draft.otherAnswers = newValue } }
    @State private var bodyHeight: CGFloat = 0
    @State private var replyRequested = false
    @State private var originalConversationID: String?
    @FocusState private var cardFocused: Bool
    @FocusState private var focusedField: String?
    @State private var current: String?
    private var page: Int { get { min(draft.page, max(0, questions.count - 1)) } nonmutating set { draft.page = newValue } }

    private enum Choice: Hashable { case none, option(String), other }

    var body: some View {
        VStack(alignment: .leading, spacing: 0) {
            header.padding(.horizontal, 16).padding(.top, 12).padding(.bottom, 4)
            ScrollViewReader { scroll in
            ScrollView {
                VStack(alignment: .leading, spacing: 16) {
                    switch interaction.kind {
                    case .approval(_, let detail, _):
                        if !detail.isEmpty { Text(detail).textSelection(.enabled).frame(maxWidth: .infinity, alignment: .leading) }
                    case .userInput(let questions), .asyncUserInput(let questions, _):
                        if readOnlyQuestion { Text("请回到父会话回答这个问题。").foregroundStyle(.secondary) }
                        ZStack(alignment: .topLeading) {
                            ForEach(Array(questions.enumerated()), id: \.element.id) { index, question in
                                questionForm(question, index: index, count: questions.count)
                                    .opacity(index == page ? 1 : 0)
                                    .allowsHitTesting(index == page)
                                    .disabled(index != page)
                                    .accessibilityHidden(index != page)
                            }
                        }
                    }
                }
                .frame(maxWidth: .infinity, alignment: .leading)
                .padding(.horizontal, 16).padding(.vertical, 12)
                .fixedSize(horizontal: false, vertical: true)
                .onGeometryChange(for: CGFloat.self) { $0.size.height } action: { bodyHeight = $0 }
                .id("runtime-question-top")
            }
            .onChange(of: page) { _, _ in scroll.scrollTo("runtime-question-top", anchor: .top) }
            }
            .frame(height: min(bodyHeight > 0 ? bodyHeight : 120, max(64, maximumBodyHeight)))
            .accessibilityIdentifier("interaction-form")
            if let failure = store.interactionReplyFailure(interaction.id) {
                HStack(alignment: .top, spacing: 6) {
                    Image(systemName: "exclamationmark.circle").accessibilityHidden(true)
                    Text("回答未发送：\(failure)").fixedSize(horizontal: false, vertical: true)
                }
                    .foregroundStyle(.red).uiFont(.callout).lineLimit(3)
                    .frame(maxWidth: .infinity, alignment: .leading)
                    .padding(.horizontal, 14).padding(.vertical, 8)
                    .help(failure)
            }
            actions.padding(.horizontal, 16).padding(.top, 4).padding(.bottom, 12)
        }
        .background(Color(nsColor: .controlBackgroundColor), in: RoundedRectangle(cornerRadius: 8))
        .overlay { RoundedRectangle(cornerRadius: 8).strokeBorder(cardFocused && focusedField == nil ? Color(nsColor: .keyboardFocusIndicatorColor) : Color.clear, lineWidth: 2) }
        .focusable().focusEffectDisabled().focused($cardFocused)
        .onKeyPress(phases: .down) { press in key(press) }
        .frame(maxWidth: contentWidth)
        .padding(.horizontal, 16).padding(.vertical, 8)
        .onChange(of: interaction.id, initial: true) { _, _ in
            replyRequested = false
            originalConversationID = store.loadedConversationID
            current = questions.indices.contains(page) ? questions[page].id : nil
        }
        .onChange(of: tab.interactionFocusRequested, initial: true) { _, requested in
            if requested != nil, tab.takeInteractionFocusRequest() { takeKeyboard() }
        }
        .onDisappear {
            if replyRequested, store.loadedConversationID == originalConversationID,
               !store.pendingInteractions.contains(where: { $0.id == interaction.id }) { tab.composer.focusWhenShown() }
        }
        .accessibilityElement(children: .contain)
        .accessibilityLabel("运行时请求")
        .accessibilityIdentifier("runtime-interaction")
    }

    private var isApproval: Bool { if case .approval = interaction.kind { true } else { false } }
    private var isAsyncQuestion: Bool { if case .asyncUserInput = interaction.kind { true } else { false } }
    private var readOnlyQuestion: Bool { if case .asyncUserInput(_, .conversationInput) = interaction.kind { store.loadedParentConversationID != nil } else { false } }

    private var header: some View {
        HStack(alignment: .firstTextBaseline, spacing: 8) {
            if isApproval { Image(systemName: "hand.raised").foregroundStyle(.secondary) }
            Text(title).uiFont(isApproval ? .headline : .callout).foregroundStyle(isApproval ? Color.primary : Color.secondary).fixedSize(horizontal: false, vertical: true)
            if questions.count > 1 {
                Text("\(page + 1) / \(questions.count)").uiFont(.callout).monospacedDigit().foregroundStyle(.secondary)
            }
            Spacer(minLength: 8)
            if store.pendingInteractions.count > 1 {
                Text("另有 \(store.pendingInteractions.count - 1) 项").uiFont(.callout).foregroundStyle(.secondary).fixedSize()
            } else if isAsyncQuestion && !readOnlyQuestion {
                Text("可稍后回答").uiFont(.callout).foregroundStyle(.secondary).fixedSize()
            }
        }
        .contentShape(Rectangle()).onTapGesture { takeKeyboard() }
    }
    private var questions: [InteractionQuestion] {
        switch interaction.kind {
        case .userInput(let questions), .asyncUserInput(let questions, _): questions
        case .approval: []
        }
    }
    private var title: String {
        if case .approval(let title, _, _) = interaction.kind { return title }
        return isAsyncQuestion ? "Agent 提问" : "需要你的回答"
    }

    private func questionForm(_ question: InteractionQuestion, index: Int, count: Int) -> some View {
        VStack(alignment: .leading, spacing: 12) {
            Text(question.text).uiFont(.title3.weight(.semibold)).fixedSize(horizontal: false, vertical: true).textSelection(.enabled)
            if readOnlyQuestion {
                ForEach(question.options) { option in Text(option.label).foregroundStyle(.secondary).fixedSize(horizontal: false, vertical: true) }
                if let text = question.defaultText, !text.isEmpty {
                    if question.secret { Text("默认回答已隐藏").foregroundStyle(.secondary) }
                    else { Text(text).textSelection(.enabled) }
                }
            } else {
                field(question)
                    .disabled(store.interactionReplyStatus(interaction.id) != nil)
            }
        }
        .frame(maxWidth: .infinity, alignment: .leading)
    }

    @ViewBuilder private func field(_ question: InteractionQuestion) -> some View {
        if !question.options.isEmpty {
            if question.multiple {
                VStack(alignment: .leading, spacing: 8) {
                    ForEach(Array(question.options.enumerated()), id: \.element.id) { index, option in
                        Toggle(optionTitle(option, index: index), isOn: Binding(get: {
                            selectedOptions[question.id, default: []].contains(option.id)
                        }, set: { checked in
                            current = question.id
                            if checked { selectedOptions[question.id, default: []].insert(option.id) }
                            else { selectedOptions[question.id, default: []].remove(option.id) }
                        })).toggleStyle(.checkbox)
                    }
                }
            } else {
                Picker("回答", selection: Binding<Choice>(get: {
                    if otherAnswers.contains(question.id) { return .other }
                    return selectedOptions[question.id, default: []].first.map(Choice.option) ?? .none
                }, set: { choice in
                    current = question.id
                    switch choice {
                    case .none: selectedOptions[question.id] = []; otherAnswers.remove(question.id)
                    case .option(let id): selectedOptions[question.id] = [id]; otherAnswers.remove(question.id)
                    case .other: selectedOptions[question.id] = []; otherAnswers.insert(question.id)
                    }
                })) {
                    ForEach(Array(question.options.enumerated()), id: \.element.id) { index, option in
                        Text(optionTitle(option, index: index)).fixedSize(horizontal: false, vertical: true).tag(Choice.option(option.id))
                    }
                    if question.allowOther { Text("其他回答").tag(Choice.other) }
                }.pickerStyle(.radioGroup).labelsHidden()
            }
            if question.allowOther {
                let custom = Group {
                    if question.secret { SecureField("输入其他回答", text: value(question.id)).privacySensitive() }
                    else { TextField(question.multiple ? "补充回答（可选）" : "输入其他回答", text: value(question.id)) }
                }
                    .textFieldStyle(.roundedBorder)
                    .focused($focusedField, equals: question.id)
                    .accessibilityIdentifier("interaction-answer-\(question.id)")
                    .onSubmit { submitOrAdvance() }
                if question.multiple || otherAnswers.contains(question.id) { custom }
                else { custom.hidden() }
            }
        } else if question.secret {
            SecureField("回答", text: value(question.id)).textFieldStyle(.roundedBorder).privacySensitive()
                .focused($focusedField, equals: question.id).onSubmit { submitOrAdvance() }
                .accessibilityIdentifier("interaction-answer-\(question.id)")
        } else if question.multiline {
            TextEditor(text: value(question.id)).font(.body).scrollContentBackground(.hidden)
                .padding(5).frame(height: 68)
                .background(Color(nsColor: .textBackgroundColor), in: RoundedRectangle(cornerRadius: 5))
                .overlay { RoundedRectangle(cornerRadius: 5).strokeBorder(focusedField == question.id ? Color(nsColor: .keyboardFocusIndicatorColor) : Color(nsColor: .separatorColor), lineWidth: focusedField == question.id ? 2 : 1) }
                .focused($focusedField, equals: question.id)
                .accessibilityLabel(question.text).accessibilityIdentifier("interaction-answer-\(question.id)")
        } else {
            TextField("回答", text: value(question.id)).textFieldStyle(.roundedBorder)
                .focused($focusedField, equals: question.id).onSubmit { submitOrAdvance() }
                .accessibilityIdentifier("interaction-answer-\(question.id)")
        }
    }

    private func optionTitle(_ option: InteractionOption, index: Int) -> String {
        index < 9 ? "\(index + 1)  \(option.label)" : option.label
    }

    private var actions: some View {
        HStack(spacing: 8) {
            if readOnlyQuestion {
                Image(systemName: "arrow.turn.up.left").foregroundStyle(.secondary)
                Text("由父会话协调").uiFont(.callout).foregroundStyle(.secondary)
                Spacer(minLength: 8)
                if questions.count > 1 {
                    Button { navigate(to: max(0, page - 1)) } label: { Image(systemName: "chevron.left") }.disabled(page == 0).help("上一题")
                    Button { navigate(to: min(questions.count - 1, page + 1)) } label: { Image(systemName: "chevron.right") }.disabled(page + 1 == questions.count).help("下一题")
                }
            } else {
                Button(isAsyncQuestion ? "忽略" : "取消请求", role: .cancel) { reply(.cancel) }
                    .help("卡片有焦点时按 ⌘⌫")
                Spacer(minLength: 8)
                if let status = store.interactionReplyStatus(interaction.id) {
                    ProgressView().controlSize(.small).accessibilityLabel(status)
                }
                switch interaction.kind {
                case .approval(_, _, let options):
                    ViewThatFits(in: .horizontal) {
                        HStack(spacing: 8) { approvalButtons(options) }
                        VStack(alignment: .trailing, spacing: 8) { approvalButtons(options) }
                    }
                case .userInput(let questions), .asyncUserInput(let questions, _):
                    if page > 0 {
                        Button { navigate(to: page - 1) } label: { Image(systemName: "chevron.left") }
                            .help("上一题").accessibilityLabel("上一题").accessibilityIdentifier("interaction-previous")
                    }
                    if page + 1 < questions.count {
                        Button("下一题") { navigate(to: page + 1) }
                            .buttonStyle(.borderedProminent).accessibilityIdentifier("interaction-next")
                    } else {
                        Button("提交回答") { submit(questions) }.disabled(!answered(questions))
                            .buttonStyle(.borderedProminent)
                            .help("回答完毕后，卡片有焦点时按 Return")
                            .accessibilityIdentifier("interaction-submit")
                    }
                }
            }
        }.disabled(!store.canReply(to: interaction))
    }

    private func approvalButtons(_ options: [InteractionOption]) -> some View {
        ForEach(Array(options.enumerated()), id: \.element.id) { index, option in
            Button(optionTitle(option, index: index)) { reply(.decision(optionId: option.id)) }
                .help(index < 9 ? "卡片有焦点时按 \(index + 1)" : option.label)
        }
    }

    private func takeKeyboard() {
        guard !readOnlyQuestion else { return }
        DispatchQueue.main.async {
            switch interaction.kind {
            case .userInput(let questions), .asyncUserInput(let questions, _): advance(questions)
            case .approval: cardFocused = true
            }
        }
    }
    private func navigate(to index: Int) {
        guard questions.indices.contains(index) else { return }
        page = index
        current = questions[index].id
        focusedField = nil
        if !readOnlyQuestion {
            if questions[index].options.isEmpty { focusedField = questions[index].id }
            else { cardFocused = true }
        }
    }
    private func advance(_ questions: [InteractionQuestion]) {
        let next = questions.first { answerValues($0).isEmpty }
        if let next, let index = questions.firstIndex(where: { $0.id == next.id }) { page = index }
        else if !questions.isEmpty { page = questions.count - 1 }
        if let next, next.options.isEmpty {
            current = nil; focusedField = next.id
        } else {
            current = next?.id; focusedField = nil; cardFocused = true
        }
    }
    private func key(_ press: KeyPress) -> KeyPress.Result {
        guard cardFocused, focusedField == nil, store.canReply(to: interaction), !readOnlyQuestion else { return .ignored }
        if press.key == .delete, press.modifiers == .command { reply(.cancel); return .handled }
        switch interaction.kind {
        case .approval(_, _, let options):
            guard press.modifiers.isEmpty, let digit = Int(press.characters), (1...min(options.count, 9)).contains(digit) else { return .ignored }
            reply(.decision(optionId: options[digit - 1].id))
        case .userInput(let questions), .asyncUserInput(let questions, _):
            if press.key == .return, press.modifiers.isEmpty, answered(questions) { submit(questions); return .handled }
            guard press.modifiers.isEmpty, let digit = Int(press.characters), let question = questions.first(where: { $0.id == current }), (1...min(question.options.count, 9)).contains(digit) else { return .ignored }
            let option = question.options[digit - 1].id
            if question.multiple {
                if selectedOptions[question.id, default: []].contains(option) { selectedOptions[question.id, default: []].remove(option) }
                else { selectedOptions[question.id, default: []].insert(option) }
            } else { selectedOptions[question.id] = [option]; otherAnswers.remove(question.id); advance(questions) }
        }
        return .handled
    }
    private func answerValues(_ question: InteractionQuestion) -> [String] { draft.answerValues(question) }
    private func answered(_ questions: [InteractionQuestion]) -> Bool { !questions.contains { answerValues($0).isEmpty } }
    private func submit(_ questions: [InteractionQuestion]) {
        guard answered(questions), store.canReply(to: interaction) else { return }
        reply(.answers(answers: questions.map { InteractionAnswer(questionId: $0.id, values: answerValues($0)) }))
    }
    private func submitOrAdvance() {
        switch interaction.kind {
        case .userInput(let questions), .asyncUserInput(let questions, _): answered(questions) ? submit(questions) : advance(questions)
        case .approval: break
        }
    }
    private func value(_ id: String) -> Binding<String> { Binding(get: { values[id] ?? "" }, set: { values[id] = $0 }) }
    private func reply(_ value: RuntimeInteractionReply) {
        replyRequested = cardFocused || focusedField != nil
        store.replyInteraction(interaction, reply: value)
    }
}
