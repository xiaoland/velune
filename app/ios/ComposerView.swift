import SwiftUI

struct ComposerView: View {
    @ObservedObject var model: ConversationModel
    let runtimes: [RuntimeInstance]
    let projects: [ConversationProject]
    var composing: FocusState<Bool>.Binding
    let sendFromKeyboard: () -> Void

    private var runtimeName: String { runtimes.first { $0.id == model.runtimeID }?.name ?? "选择运行时" }
    private var projectName: String {
        guard let cwd = model.cwd else { return "默认目录" }
        return projects.first { $0.primaryFolder == cwd }?.name ?? URL(fileURLWithPath: cwd).lastPathComponent
    }
    private var directoryProjects: [ConversationProject] {
        var seen = Set<String>()
        return projects.filter { project in project.primaryFolder.map { seen.insert($0).inserted } ?? false }
    }
    private var modelName: String {
        guard let selection = model.selection else { return model.isNew ? "默认模型" : "会话模型" }
        return model.models.first { $0.selection == selection }?.name ?? "当前模型"
    }
    var body: some View {
        VStack(alignment: .leading, spacing: 4) {
            if model.failure != nil {
                HStack {
                    Label("操作未完成", systemImage: "exclamationmark.triangle").foregroundStyle(.orange)
                    Spacer()
                    ProblemsButton()
                }.font(.caption).padding(.horizontal, 4)
            }
            HStack(alignment: .bottom, spacing: 8) {
                // Match TextEditor's native text insets; keep Return and text selection native.
                Text(model.draft.isEmpty ? " " : model.draft + "\u{200B}")
                    .lineLimit(1...6).frame(maxWidth: .infinity, alignment: .leading)
                    .padding(.horizontal, 5).padding(.vertical, 8)
                    .frame(minHeight: 44).hidden().accessibilityHidden(true)
                    .overlay {
                        TextEditor(text: $model.draft)
                            .scrollContentBackground(.hidden)
                            .overlay(alignment: .topLeading) {
                                if model.draft.isEmpty {
                                    Text("发送消息").foregroundStyle(.tertiary)
                                        .padding(.horizontal, 5).padding(.vertical, 8)
                                        .allowsHitTesting(false).accessibilityHidden(true)
                                }
                            }
                            .accessibilityLabel("发送消息").focused(composing)
                            .autocorrectionDisabled().textInputAutocapitalization(.never)
                            .onKeyPress(.return, phases: .down) { key in
                                guard key.modifiers.contains(.command) else { return .ignored }
                                sendFromKeyboard(); return .handled
                            }
                    }
                    .font(.body)
                Button {
                    if model.snapshot?.actions.canCancel == true { model.stop() }
                    else { composing.wrappedValue = false; model.send() }
                } label: {
                    Image(systemName: model.snapshot?.actions.canCancel == true ? "stop.fill" : "arrow.up")
                        .font(.body.weight(.semibold)).frame(width: 44, height: 44)
                        .background(.tint, in: Circle()).foregroundStyle(.white)
                        .opacity(sendDisabled ? 0.35 : 1)
                }
                .buttonStyle(.plain).disabled(sendDisabled)
                .accessibilityLabel(model.snapshot?.actions.canCancel == true ? "停止" : "发送")
                .help(model.snapshot?.actions.canCancel == true ? "停止生成（⌘.）" : "发送（⌘Return）")
            }
            HStack(spacing: 4) {
                Menu {
                    if model.isNew {
                        Picker("运行时", selection: Binding(get: { model.runtimeID }, set: model.chooseRuntime)) {
                            if !runtimes.contains(where: { $0.id == model.runtimeID }) { Text("选择运行时").tag(model.runtimeID) }
                            ForEach(runtimes.filter(\.enabled), id: \.id) { Text($0.name).tag($0.id) }
                        }
                        Picker("项目", selection: Binding(get: { model.cwd }, set: model.chooseProject)) {
                            Text("默认目录").tag(nil as String?)
                            ForEach(directoryProjects, id: \.path) { project in
                                Text(project.name ?? URL(fileURLWithPath: project.primaryFolder ?? "").lastPathComponent).tag(project.primaryFolder)
                            }
                        }
                    } else {
                        Text(runtimeName)
                        if let cwd = model.cwd { Button("复制工作目录", systemImage: "doc.on.doc") { UIPasteboard.general.string = cwd } }
                    }
                } label: {
                    Label(runtimeName + " · " + projectName, systemImage: "folder")
                        .lineLimit(1).truncationMode(.middle).padding(.horizontal, 4).frame(minHeight: 44)
                }
                .accessibilityLabel("运行时与项目").accessibilityValue(runtimeName + " · " + projectName)
                .disabled(model.busy)
                Spacer(minLength: 0)
                Menu {
                    Picker("模型", selection: $model.selection) {
                        Text(model.isNew ? "运行时默认" : "沿用会话模型").tag(nil as ModelSelection?)
                        ForEach(model.models, id: \.selection) { Text($0.name).tag(Optional($0.selection)) }
                    }
                } label: {
                    HStack(spacing: 4) {
                        if model.loadingModels { ProgressView().controlSize(.mini) }
                        Text(modelName).lineLimit(1)
                        Image(systemName: "chevron.down").imageScale(.small)
                    }.padding(.horizontal, 4).frame(minHeight: 44)
                }
                .accessibilityLabel("模型").accessibilityValue(modelName)
                .disabled(model.busy || model.loadingModels || model.models.isEmpty)
            }
            .font(.footnote).foregroundStyle(.secondary).menuStyle(.borderlessButton)
        }
        .padding(8)
        .background(Color(.secondarySystemBackground), in: RoundedRectangle(cornerRadius: 18))
    }
    private var sendDisabled: Bool {
        model.busy || (model.snapshot?.actions.canCancel != true && !model.canSend)
    }
}
