import SwiftUI
import Darwin
import VeluneBindings

/// Discovery only probes declared paths and public versions. Importing copies
/// runtime configuration; provider import remains an explicit separate action.
struct RuntimeDiscoveryView: View {
    @ObservedObject var store: AppStore
    @Environment(\.dismiss) private var dismiss
    @State private var candidates: [BindingRuntimeDiscoveryCandidate] = []
    @State private var selectedIDs: Set<String> = []
    @State private var active = true
    @State private var discovering = false
    @State private var status = ""
    @State private var importedCount = 0
    @State private var importingProviders = false
    private var selected: [BindingRuntimeDiscoveryCandidate] { candidates.filter { selectedIDs.contains($0.runtime.id) && $0.supported && !$0.alreadyConfigured } }

    var body: some View {
        VStack(spacing: 0) {
            HStack { Text("快速导入 Agent 运行时").font(.headline); Spacer() }.padding(20)
            Text("选择已安装的运行时，预览版本与目录后添加配置。提供商与模型可在下一步单独导入。")
                .font(.callout).foregroundStyle(.secondary).padding(.horizontal, 20).padding(.bottom, 12)
            List(selection: $selectedIDs) {
                ForEach(candidates, id: \.runtime.id) { candidate in
                    VStack(alignment: .leading, spacing: 4) {
                        HStack { Text(candidate.runtime.name); if let version = candidate.version { Text(version).foregroundStyle(.secondary) }; if candidate.alreadyConfigured { Text("已配置").foregroundStyle(.secondary) } }
                        Text(candidate.runtime.settings["agentDir"] ?? "").font(.caption).foregroundStyle(.secondary).lineLimit(1).truncationMode(.middle)
                        Text(candidate.runtime.settings["binary"] ?? "").help("Node：" + (candidate.runtime.settings["nodeBinary"] ?? "")).font(.caption).foregroundStyle(.secondary).lineLimit(1).truncationMode(.middle)
                        if !candidate.supported { Text(candidate.detail).font(.caption).foregroundStyle(.secondary) }
                    }.tag(candidate.runtime.id).selectionDisabled(!candidate.supported || candidate.alreadyConfigured)
                }
            }.listStyle(.bordered).padding(.horizontal, 20).disabled(discovering || store.isBusy)
            HStack { if discovering { ProgressView().controlSize(.small) }; Text(status).font(.caption).foregroundStyle(.secondary); Spacer() }.padding(.horizontal, 20).padding(.top, 10)
            SettingsError(message: store.error)
            HStack {
                Button("重新发现") { discover() }.disabled(discovering || store.isBusy)
                if importedCount > 0 { Button("导入提供商与模型…") { importingProviders = true }.disabled(store.isBusy) }
                Spacer(); Button("完成") { dismiss() }.keyboardShortcut(.cancelAction)
                Button("导入所选") { store.importRuntimes(selected) { imported in
                    importedCount += imported.count
                    for index in candidates.indices where imported.contains(candidates[index].runtime.id) { candidates[index].alreadyConfigured = true }
                    selectedIDs.subtract(imported); status = "已导入 \(importedCount) 个运行时"
                } }.disabled(selected.isEmpty || discovering || store.isBusy).keyboardShortcut(.defaultAction)
            }.padding(16)
        }.frame(width: 600, height: 430)
        .sheet(isPresented: $importingProviders) { ProviderImportView(store: store) }
        .onAppear { active = true; discover() }
        .onDisappear { active = false }
    }

    private func discover() {
        guard active, !discovering, !store.isBusy else { return }
        discovering = true; selectedIDs.removeAll(); candidates = []; status = "正在发现运行时…"
        var overrides: [String: String] = [:]
        for key in ["PI_CODING_AGENT_DIR", "CODEX_HOME", "DSH_HOME"] { if let pointer = getenv(key) { overrides[key] = String(cString: pointer) } }
        store.runtimeDiscoveryHints(userHome: FileManager.default.homeDirectoryForCurrentUser.path, overrides: overrides, onFailure: { discovering = false }) { hints in
            guard active else { return }
            guard let node = store.runtimeTypes.flatMap(\.fields).first(where: { $0.key == "nodeBinary" })?.executableDiscovery else { discovering = false; status = "没有可用的 Node 发现配置。"; return }
            MacExecutableDiscovery.discover(node) { result in
                guard active else { return }
                switch result {
                case .success(let path): gather(hints[...], node: path, probes: [])
                case .failure(let error): discovering = false; status = error.localizedDescription
                }
            }
        }
    }

    private func gather(_ hints: ArraySlice<BindingRuntimeDiscoveryHint>, node: String, probes: [BindingRuntimeDiscoveryProbe]) {
        guard active else { return }
        guard let hint = hints.first else {
            let unique = Dictionary(grouping: probes) { probe in
                URL(fileURLWithPath: probe.binary).resolvingSymlinksInPath().path + "\0" + URL(fileURLWithPath: probe.agentDirectory).resolvingSymlinksInPath().path
            }.values.compactMap(\.first).sorted { $0.binary < $1.binary }
            store.discoverRuntimes(unique, onFailure: { discovering = false }) { values in
                guard active else { return }
                candidates = values; discovering = false
                status = values.isEmpty ? "未发现运行时，可在设置中手动添加。" : "按住 ⌘ 或 Shift 选择多个运行时"
            }
            return
        }
        MacExecutableDiscovery.candidates(command: hint.command) { installed in
            guard active else { return }
            var paths = installed
            for type in store.runtimeTypes where type.familyID == hint.familyId {
                if let value = type.fields.first(where: { $0.key == "binary" })?.value, !value.isEmpty { paths.append(value) }
            }
            let next = paths.map { BindingRuntimeDiscoveryProbe(familyId: hint.familyId, binary: $0, nodeBinary: node, agentDirectory: hint.agentDirectory) }
            gather(hints.dropFirst(), node: node, probes: probes + next)
        }
    }
}
