import AppKit
import SwiftUI

struct HapiShareView: View {
    @ObservedObject var sharing: HapiSharing
    let target: HapiShareTarget?
    @Environment(\.dismiss) private var dismiss
    @State private var useRelay = true
    @State private var address = ""

    private var existingOrigin: URL? {
        guard var components = URLComponents(string: address.trimmingCharacters(in: .whitespacesAndNewlines)),
              components.scheme?.lowercased() == "https", let host = components.host, !host.isEmpty,
              components.user == nil, components.password == nil, components.query == nil, components.fragment == nil,
              components.path.isEmpty || components.path == "/",
              components.port.map({ (1...65535).contains($0) }) ?? true else { return nil }
        components.scheme = "https"
        components.path = ""
        return components.url
    }

    var body: some View {
        VStack(alignment: .leading, spacing: 16) {
            Text("分享到 HAPI").font(.title2.bold())
            if let pairing = sharing.pairing {
                pairingDetails(pairing)
            } else if let target {
                Text(target.title).font(.headline).lineLimit(2)
                Text("允许持有配对码的 HAPI 客户端读取此会话、发送消息、停止当前轮次和处理支持的审批。")
                    .foregroundStyle(.secondary)
                Picker("连接方式", selection: $useRelay) {
                    Text("HAPI 公共中继").tag(true)
                    Text("已有 HTTPS 地址").tag(false)
                }.disabled(sharing.busy)
                if useRelay {
                    Text("建立可从公网访问的 HTTPS 地址。配对码用于授权，TLS 在这台 Mac 上终止。")
                        .foregroundStyle(.secondary)
                } else {
                    TextField("HTTPS 地址", text: $address, prompt: Text("https://hub.example.com"))
                        .disabled(sharing.busy)
                    Text("将整个 HTTPS 地址转发到 127.0.0.1:\(HapiSharing.port)，并允许 SSE 实时传输。")
                        .foregroundStyle(.secondary)
                }
            } else {
                Text("当前没有正在共享的 HAPI 会话。")
            }
            if let error = sharing.errorMessage {
                Text(error).foregroundStyle(.red).textSelection(.enabled)
            }
            HStack {
                if sharing.busy {
                    ProgressView().controlSize(.small)
                    Text(sharing.pairing == nil ? "正在建立连接…" : "正在结束共享…").foregroundStyle(.secondary)
                }
                Spacer()
                if sharing.pairing != nil {
                    Button("结束共享", role: .destructive) { sharing.stop() }.disabled(sharing.busy)
                }
                if sharing.busy && sharing.pairing == nil {
                    Button("取消") { sharing.cancelStart(); dismiss() }.keyboardShortcut(.cancelAction)
                } else {
                    Button("关闭") { dismiss() }.keyboardShortcut(.cancelAction)
                }
                if sharing.pairing == nil, let target {
                    Button("开启共享") { sharing.start(target, existingOrigin: useRelay ? nil : existingOrigin) }
                        .keyboardShortcut(.defaultAction)
                        .disabled(sharing.busy || (!useRelay && existingOrigin == nil))
                }
            }
        }.padding(24).frame(width: 490)
    }

    private func pairingDetails(_ pairing: HapiPairing) -> some View {
        VStack(alignment: .leading, spacing: 8) {
            Text(pairing.target.title).font(.headline).lineLimit(2)
            Text("在手机的 HAPI 配对页选「扫描二维码」。你也可以手动输入地址和令牌。")
                .foregroundStyle(.secondary)
            if let image = pairingQRCode(pairing.link) {
                Image(nsImage: image).interpolation(.none).resizable().frame(width: 184, height: 184)
                    .padding(10).background(.white).accessibilityLabel("HAPI 配对二维码")
                    .frame(maxWidth: .infinity)
            }
            Text(pairing.origin.absoluteString).textSelection(.enabled)
            HStack {
                Button("复制地址") { copy(pairing.origin.absoluteString) }
                Button("复制令牌") { copy(pairing.grant.accessToken) }
            }
            Text("关闭此窗口后共享继续。可在「设置 → 设备」结束共享。")
                .foregroundStyle(.secondary)
        }
    }

    private func copy(_ text: String) {
        NSPasteboard.general.clearContents()
        NSPasteboard.general.setString(text, forType: .string)
    }
}

struct HapiSharingSettings: View {
    @ObservedObject var sharing: HapiSharing
    @State private var showsPairing = false

    var body: some View {
        Section("HAPI 会话共享") {
            if let pairing = sharing.pairing {
                LabeledContent(pairing.target.title) {
                    Button("查看配对二维码…") { showsPairing = true }
                    Button("结束共享", role: .destructive) { sharing.stop() }.disabled(sharing.busy)
                }
            } else if sharing.busy {
                ProgressView("正在建立 HAPI 连接…").controlSize(.small)
            } else {
                Text("在本机会话菜单选择「分享到 HAPI…」。").foregroundStyle(.secondary)
            }
            if let error = sharing.errorMessage { Text(error).foregroundStyle(.red) }
        }
        .sheet(isPresented: $showsPairing) { HapiShareView(sharing: sharing, target: nil) }
    }
}
