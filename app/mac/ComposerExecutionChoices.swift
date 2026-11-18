import SwiftUI
import VeluneBindings

/// The model and reasoning choices shared by an ordinary composer and a
/// scoped preparation surface. The caller owns the choice and its lifetime.
struct ComposerModelChoiceContent: View {
    static let retainedModelExplanation = "当前目录未列出原模型，保存正文或附件时会保留原配置。"

    let next: NextTurn?
    let catalog: RuntimeModelCatalog?
    @Binding var selection: ModelSelection?
    let enabled: Bool
    var defaultTitle: String? = nil
    var defaultAvailable = true
    var defaultHelp: String? = nil
    var unavailableMessage = "所选模型不可用，草稿已保留；请重新选择后发送。"
    var retainedModelName: String? = nil

    private var nativeModels: [RuntimeModel] {
        (next?.models ?? []).filter { if case .managed = $0.selection { false } else { true } }
    }
    private var managedModels: [RuntimeModel] {
        (next?.models ?? []).filter { if case .managed = $0.selection { true } else { false } }
    }
    private var followsCurrent: Bool { next?.hasCurrent == true }
    private var unchosenTitle: String {
        if let defaultTitle { return defaultTitle }
        if followsCurrent, next?.model != nil {
            return "沿用当前（\(next?.chosenModelUnavailable == true ? "所选模型不可用" : next?.modelName ?? "")）"
        }
        return "运行时默认"
    }
    var body: some View {
        switch catalog {
        case .loading: Text("正在读取运行时的模型…")
        case .failed(let detail): Text("无法读取运行时的模型：\(detail)")
        case .ready, nil: EmptyView()
        }
        if next?.chosenModelUnavailable == true {
            Text(retainedModelName == nil ? unavailableMessage : Self.retainedModelExplanation)
        }
        Picker("运行时模型", selection: $selection) {
            Text(unchosenTitle).tag(ModelSelection?.none).disabled(!defaultAvailable)
                .help(defaultHelp ?? (followsCurrent ? "不更改：沿用会话当前的模型" : "不指定：由运行时使用自己的默认模型"))
            if let retainedModelName, let selection {
                Text("\(retainedModelName)（原配置）").tag(Optional(selection)).disabled(true)
            }
            ForEach(nativeModels) { model in Text(model.name).tag(Optional(model.selection)) }
        }.pickerStyle(.inline).disabled(!enabled)
        if !managedModels.isEmpty {
            Picker("Velune 提供商", selection: $selection) {
                ForEach(managedModels) { model in Text(model.name).tag(Optional(model.selection)) }
            }.pickerStyle(.inline).disabled(!enabled)
        }
    }
}

struct ComposerEffortChoiceContent: View {
    let next: NextTurn?
    @Binding var selection: String?
    var enabled = true
    var defaultTitle: String? = nil

    var body: some View {
        Picker("推理强度", selection: $selection) {
            Text(defaultTitle ?? next?.currentEffort.map { "沿用当前（\($0)）" } ?? (next?.hasCurrent == true ? "沿用当前" : "运行时默认"))
                .tag(String?.none)
                .help(next?.hasCurrent == true ? "不更改：沿用会话当前的推理强度" : "不指定：由运行时自己决定")
            ForEach(next?.efforts ?? [], id: \.self) { level in Text(level).tag(Optional(level)) }
        }.pickerStyle(.inline).labelsHidden().disabled(!enabled)
    }
}
