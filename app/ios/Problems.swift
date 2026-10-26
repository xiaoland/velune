import SwiftUI

struct AppProblem: Identifiable {
    let id = UUID()
    let occurredAt = Date()
    let source: String
    let context: String
    let detail: String
    var diagnostic: String { "时间：\(occurredAt.formatted())\n操作：\(source)\n位置：\(context)\n\(detail)" }
}

@MainActor
final class ProblemStore: ObservableObject {
    static let shared = ProblemStore()
    @Published private(set) var items: [AppProblem] = []
    private var latest: [String: String] = [:]

    func record(_ error: Error, source: String, context: String = "") {
        var detail = mobileError(error)
        if let binding = error as? BindingError {
            switch binding {
            case .Diagnostic(let kind, _, let code, let phase, let operation):
                detail += "\n类别：\(kind)\n代码：\(code)\n阶段：\(phase)\n操作编号：\(operation)"
            default: break
            }
        }
        record(detail, source: source, context: context)
    }

    func record(_ detail: String, source: String, context: String = "") {
        let key = source + "\n" + context
        guard latest[key] != detail else { return }
        latest[key] = detail
        items.insert(AppProblem(source: source, context: context, detail: detail), at: 0)
    }
    func recovered(source: String, context: String = "") { latest[source + "\n" + context] = nil }
    func clear(_ id: UUID) { items.removeAll { $0.id == id } }
    func clearAll() { items.removeAll() }
}

struct ProblemsView: View {
    @ObservedObject var store = ProblemStore.shared
    @Environment(\.dismiss) private var dismiss
    var body: some View {
        NavigationStack {
            List {
                ForEach(store.items) { problem in
                    NavigationLink {
                        ProblemDetailView(problem: problem)
                    } label: {
                        HStack(alignment: .top, spacing: 12) {
                            Image(systemName: "exclamationmark.triangle").foregroundStyle(.orange).padding(.top, 2)
                            VStack(alignment: .leading, spacing: 5) {
                                HStack {
                                    Text(problem.source).font(.headline)
                                    Spacer()
                                    Text(problem.occurredAt, format: .dateTime.hour().minute()).font(.caption).foregroundStyle(.secondary)
                                }
                                if !problem.context.isEmpty { Text(problem.context).font(.callout).foregroundStyle(.secondary).lineLimit(2) }
                            }
                        }.padding(.vertical, 6)
                    }
                    .swipeActions { Button("清除", role: .destructive) { store.clear(problem.id) } }
                    .contextMenu { Button("复制诊断", systemImage: "doc.on.doc") { UIPasteboard.general.string = problem.diagnostic } }
                }
            }
            .overlay { if store.items.isEmpty { ContentUnavailableView("没有问题", systemImage: "checkmark.circle") } }
            .navigationTitle("问题")
            .navigationBarTitleDisplayMode(.inline)
            .toolbar {
                ToolbarItem(placement: .topBarLeading) { Button("全部清除") { store.clearAll() }.disabled(store.items.isEmpty) }
                ToolbarItem(placement: .confirmationAction) { Button("完成") { dismiss() }.keyboardShortcut(.cancelAction) }
            }
        }
    }
}

private struct ProblemDetailView: View {
    let problem: AppProblem
    @State private var copied = false
    @Environment(\.dismiss) private var dismiss
    var body: some View {
        ScrollView {
            VStack(alignment: .leading, spacing: 16) {
                Text(problem.source).font(.title2.weight(.semibold))
                if !problem.context.isEmpty { Text(problem.context).foregroundStyle(.secondary) }
                Text(problem.occurredAt.formatted()).font(.caption).foregroundStyle(.secondary)
                Divider()
                Text(problem.detail).font(.callout.monospaced()).frame(maxWidth: .infinity, alignment: .leading)
            }.textSelection(.enabled).padding(20)
        }
        .navigationTitle("问题详情").navigationBarTitleDisplayMode(.inline)
        .toolbar {
            ToolbarItem(placement: .primaryAction) {
                Button(copied ? "已复制" : "复制诊断") {
                    UIPasteboard.general.string = problem.diagnostic; copied = true
                }
            }
            ToolbarItem(placement: .bottomBar) {
                Button("清除此条问题", role: .destructive) { ProblemStore.shared.clear(problem.id); dismiss() }
            }
        }
    }
}

struct ProblemsButton: View {
    @ObservedObject var store = ProblemStore.shared
    @State private var presented = false
    var body: some View {
        Button { presented = true } label: {
            HStack { Image(systemName: "exclamationmark.triangle"); Text(store.items.isEmpty ? "问题" : "问题（\(store.items.count)）") }
        }
        .tint(store.items.isEmpty ? .secondary : .orange)
        .labelStyle(.titleAndIcon)
        .sheet(isPresented: $presented) { ProblemsView() }
    }
}
