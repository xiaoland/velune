import AppKit
import SwiftUI

@MainActor final class IPadInstallationModel: ObservableObject {
    @Published private(set) var repository: String
    @Published var teamID: String {
        didSet { UserDefaults.standard.set(teamID, forKey: "iPadInstallationTeam") }
    }
    @Published var selectedDeviceID = ""
    @Published private(set) var devices = [IPadInstaller.Device]()
    @Published private(set) var refreshing = false
    @Published private(set) var installing = false
    @Published private(set) var status = ""

    init() {
        let defaults = UserDefaults.standard
        let buildRepository = URL(fileURLWithPath: #filePath)
            .deletingLastPathComponent().deletingLastPathComponent().deletingLastPathComponent().path
        repository = defaults.string(forKey: "iPadInstallationRepository") ??
            (FileManager.default.fileExists(atPath: buildRepository + "/app/ios/Velune.xcodeproj/project.pbxproj")
                ? buildRepository : "")
        teamID = defaults.string(forKey: "iPadInstallationTeam") ?? ""
    }

    func chooseRepository() {
        guard !installing else { return }
        let panel = NSOpenPanel()
        panel.message = "选择Velune源码目录，将构建并安装此目录中的 iPad 版本。"
        panel.canChooseDirectories = true
        panel.canChooseFiles = false
        panel.allowsMultipleSelection = false
        if !repository.isEmpty { panel.directoryURL = URL(fileURLWithPath: repository) }
        guard panel.runModal() == .OK, let url = panel.url else { return }
        guard FileManager.default.fileExists(atPath: url.appendingPathComponent("app/ios/Velune.xcodeproj/project.pbxproj").path) else {
            status = "所选目录没有Velune iPad 工程，请选择Velune源码目录。"
            return
        }
        repository = url.path
        UserDefaults.standard.set(repository, forKey: "iPadInstallationRepository")
        status = ""
    }

    func refreshDevices() {
        guard !refreshing && !installing else { return }
        refreshing = true
        status = "正在查找 iPad…"
        Task {
            let result = await Task.detached(priority: .userInitiated) {
                Result { try IPadInstaller.devices() }
            }.value
            refreshing = false
            switch result {
            case .success(let found):
                devices = found
                if !found.contains(where: { $0.id == selectedDeviceID }) {
                    selectedDeviceID = found.first?.id ?? ""
                }
                status = found.isEmpty ? "未找到已配对的 iPad。请连接并解锁 iPad，确认信任此 Mac 后刷新。" : ""
            case .failure(let error):
                devices = []
                selectedDeviceID = ""
                status = error.localizedDescription
            }
        }
    }

    func install() {
        guard !installing && !refreshing,
              devices.contains(where: { $0.id == selectedDeviceID }) else { return }
        let source = URL(fileURLWithPath: repository)
        let deviceID = selectedDeviceID
        let team = teamID.trimmingCharacters(in: .whitespacesAndNewlines).uppercased()
        teamID = team
        installing = true
        status = "正在构建、签名并安装到 iPad，请稍候…"
        Task {
            let result = await Task.detached(priority: .userInitiated) {
                Result { try IPadInstaller.install(repository: source, deviceID: deviceID, teamID: team) }
            }.value
            installing = false
            switch result {
            case .success: status = "已安装到 iPad。"
            case .failure(let error): status = "安装未完成：\(error.localizedDescription)"
            }
        }
    }
}

struct IPadInstallationPanel: View {
    @ObservedObject var model: IPadInstallationModel
    @Environment(\.dismiss) private var dismiss

    var body: some View {
        VStack(alignment: .leading, spacing: 16) {
            Text("安装到 iPad").uiFont(.title2)
            Form {
                HStack {
                    Text("源码目录")
                    Text(model.repository.isEmpty ? "尚未选择" : model.repository)
                        .lineLimit(2).truncationMode(.middle).help(model.repository)
                        .frame(maxWidth: .infinity, alignment: .leading)
                    Button("选择…", action: model.chooseRepository)
                        .accessibilityLabel("选择Velune源码目录")
                        .disabled(model.installing)
                }
                TextField("签名团队 ID", text: $model.teamID)
                    .disabled(model.installing)
                HStack {
                    Picker("iPad", selection: $model.selectedDeviceID) {
                        Text("请选择 iPad").tag("")
                        ForEach(model.devices) { device in
                            Text(device.name).tag(device.id)
                        }
                    }
                    Button("刷新", action: model.refreshDevices)
                }
                .disabled(model.installing || model.refreshing)
            }
            Text("需要安装 Xcode，并在其中登录签名团队对应的 Apple 账号。连接并解锁 iPad，信任此 Mac 并开启开发者模式。安装所选源码目录的版本，保留应用数据。")
                .uiFont(.caption).foregroundStyle(.secondary)
            if !model.status.isEmpty {
                ScrollView {
                    Text(model.status).uiFont(.caption).textSelection(.enabled)
                        .frame(maxWidth: .infinity, alignment: .leading)
                }
                .frame(maxHeight: 100)
            }
            HStack {
                if model.installing || model.refreshing {
                    ProgressView().controlSize(.small)
                }
                Spacer()
                Button("关闭") { dismiss() }.disabled(model.installing)
                Button("安装", action: model.install)
                    .buttonStyle(.borderedProminent)
                    .disabled(model.installing || model.refreshing || model.repository.isEmpty ||
                        model.teamID.trimmingCharacters(in: .whitespacesAndNewlines).isEmpty ||
                        model.selectedDeviceID.isEmpty)
            }
        }
        .padding(24)
        .frame(width: 560)
        .interactiveDismissDisabled(model.installing)
        .onAppear { model.refreshDevices() }
    }
}
