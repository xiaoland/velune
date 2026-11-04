import SwiftUI
import VeluneBindings

/// One current error and its authoritative recovery state. This is derived
/// from the delivered transcript, never a second retry scheduler.
struct RuntimeRecovery: Identifiable, Equatable {
    let id: String
    let failure: RuntimeFailureKind
    let detail: String
    let retry: RuntimeRetry?

    var active: Bool {
        switch retry {
        case .native, .waiting, .attempted: true
        case .exhausted, .cancelled, .recovered, nil: false
        }
    }

    static func latest(in messages: [DeliveredMessage]) -> Self? {
        var laterUser = false
        for message in messages.reversed() {
            if message.role == .user { laterUser = true }
            for (index, block) in message.blocks.enumerated().reversed() {
                if case let .runtimeFailure(failure, detail, retry) = block {
                    let value = Self(id: message.id + ":\(index)", failure: failure, detail: detail, retry: retry)
                    return value.active || !laterUser ? value : nil
                }
            }
        }
        return nil
    }
}

/// Recovery stays beside the live transcript even when its historical work
/// group is collapsed. It takes transcript space, not composer height.
struct RuntimeRecoveryView: View {
    let recovery: RuntimeRecovery
    let stopping: Bool
    let canStop: Bool
    let stop: () -> Void
    let dismiss: () -> Void
    @State private var showingDetails = false

    var body: some View {
        VStack(spacing: 0) {
            Divider()
            HStack(alignment: .top, spacing: 10) {
                if recovery.active { ProgressView().controlSize(.small).padding(.top, 2) }
                else { Image(systemName: recovery.retry == .recovered ? "checkmark.circle" : "exclamationmark.circle").foregroundStyle(.secondary) }
                VStack(alignment: .leading, spacing: 4) {
                    Text(recovery.failure.title).uiFont(.callout.weight(.medium))
                    Text(recovery.detail).uiFont(.callout).lineLimit(3).textSelection(.enabled)
                    TimelineView(.periodic(from: .now, by: 1)) { context in
                        Text(status(at: context.date)).uiFont(.caption).foregroundStyle(.secondary)
                    }
                    Button("错误详情…") { showingDetails = true }.buttonStyle(.link).uiFont(.caption)
                }.frame(maxWidth: .infinity, alignment: .leading)
                if recovery.active {
                    Button(stopping ? "正在停止…" : "停止重试", action: stop).disabled(!canStop || stopping)
                } else {
                    Button(action: dismiss) { Image(systemName: "xmark") }.buttonStyle(.borderless)
                        .accessibilityLabel("关闭重试结果").help("关闭此结果，错误记录仍保留在工作过程中")
                }
            }.padding(12).frame(maxWidth: 760, alignment: .leading).frame(maxWidth: .infinity)
        }
        .background(.bar)
        .popover(isPresented: $showingDetails) {
            ScrollView { Text(recovery.detail).textSelection(.enabled).frame(maxWidth: .infinity, alignment: .leading).padding(16) }
                .frame(width: 560, height: 280)
        }
        .accessibilityElement(children: .contain)
        .accessibilityLabel("请求恢复状态")
    }

    private func status(at date: Date) -> String {
        if stopping { return "正在停止自动重试…" }
        switch recovery.retry {
        case .waiting(let attempt, let maximum, let at):
            let seconds = max(Int(ceil(Double(at) / 1000 - date.timeIntervalSince1970)), 0)
            return seconds > 0 ? "将在 \(seconds) 秒后自动重试 · 第 \(attempt)/\(maximum) 次" : "正在开始自动重试 · 第 \(attempt)/\(maximum) 次"
        case .attempted(let attempt, let maximum): return "正在自动重试 · 第 \(attempt)/\(maximum) 次"
        case .native: return "运行时正在自动重试"
        case .exhausted(let attempts): return "自动重试已用尽 · 共 \(attempts) 次"
        case .cancelled: return "自动重试已停止"
        case .recovered: return "重试成功，已恢复"
        case nil: return "当前没有待进行的自动重试"
        }
    }
}
