import AppKit
import SwiftUI
import VeluneBindings

/// Uncommitted git changes of the loaded conversation's working directory,
/// shown in the detail inspector: changed files above, the selected file's
/// diff below. Read-only; files open in their default application.
@MainActor
struct ChangeReviewView: View {
    @ObservedObject var store: AppStore
    @State private var changes: BindingWorkingChanges?
    @State private var selection: String?
    @State private var diff: BindingFileDiff?
    @State private var failure: String?
    @State private var loading = false
    @State private var generation: UInt64 = 0

    var body: some View {
        VStack(spacing: 0) {
            header
            Divider()
            content
        }
        .onAppear(perform: reload)
        .onChange(of: store.loadedConversationID) { _, _ in changes = nil; selection = nil; diff = nil; reload() }
        // A finished turn is the usual moment files changed.
        .onChange(of: store.isGenerating) { _, generating in if !generating { reload() } }
        .onChange(of: selection) { _, path in loadDiff(path) }
    }

    private var header: some View {
        HStack(spacing: 8) {
            VStack(alignment: .leading, spacing: 2) {
                Text(changes?.repository.map { URL(fileURLWithPath: $0).lastPathComponent } ?? "变更").font(.headline).lineLimit(1)
                if let branch = changes?.branch { Label(branch, systemImage: "arrow.triangle.branch").font(.caption).foregroundStyle(.secondary).lineLimit(1) }
                else if changes?.repository != nil { Text("分离的 HEAD").font(.caption).foregroundStyle(.secondary) }
            }
            .help(changes?.repository ?? changes?.directory ?? "")
            Spacer(minLength: 0)
            if loading { ProgressView().controlSize(.small) }
            Button { reload() } label: { Label("刷新变更", systemImage: "arrow.clockwise") }
                .labelStyle(.iconOnly).buttonStyle(.borderless).help("重新读取 git 变更").disabled(loading)
        }
        .padding(.horizontal, 12).padding(.vertical, 8)
    }

    @ViewBuilder private var content: some View {
        if let failure {
            ContentUnavailableView { Label("无法读取变更", systemImage: "exclamationmark.triangle") } description: { Text(failure) } actions: { Button("重试", action: reload) }
        } else if let changes, changes.repository == nil {
            ContentUnavailableView("不是 git 仓库", systemImage: "folder", description: Text(changes.directory))
        } else if let changes, changes.files.isEmpty {
            ContentUnavailableView("没有未提交的变更", systemImage: "checkmark.circle", description: Text("工作区与 HEAD 一致。"))
        } else if let changes {
            VSplitView {
                fileList(changes).frame(minHeight: 90, idealHeight: 200)
                diffView.frame(minHeight: 120, maxHeight: .infinity)
            }
        } else {
            ProgressView().frame(maxWidth: .infinity, maxHeight: .infinity)
        }
    }

    private func fileList(_ changes: BindingWorkingChanges) -> some View {
        List(changes.files, id: \.path, selection: $selection) { file in
            ChangedFileRow(file: file)
        }
        .contextMenu(forSelectionType: String.self) { paths in
            if let path = paths.first, let url = url(path) {
                Button("打开") { NSWorkspace.shared.open(url) }.disabled(!FileManager.default.fileExists(atPath: url.path))
                Button("在访达中显示") { NSWorkspace.shared.activateFileViewerSelecting([url]) }.disabled(!FileManager.default.fileExists(atPath: url.path))
                Divider()
                Button("拷贝路径") { NSPasteboard.general.clearContents(); NSPasteboard.general.setString(url.path, forType: .string) }
            }
        } primaryAction: { paths in
            if let path = paths.first, let url = url(path), FileManager.default.fileExists(atPath: url.path) { NSWorkspace.shared.open(url) }
        }
    }

    @ViewBuilder private var diffView: some View {
        if let diff, diff.path == selection {
            ScrollView {
                LazyVStack(alignment: .leading, spacing: 0) {
                    ForEach(Array(diff.lines.enumerated()), id: \.offset) { _, line in DiffLineRow(line: line) }
                    if diff.truncated { Text("差异过长，只显示开头部分。").font(.caption).foregroundStyle(.secondary).padding(8) }
                    if diff.lines.isEmpty { Text("没有可显示的文本差异。").font(.caption).foregroundStyle(.secondary).padding(8) }
                }
                .textSelection(.enabled)
            }
            .background(Color(nsColor: .textBackgroundColor))
        } else if selection == nil {
            Text("选择文件查看差异").foregroundStyle(.secondary).frame(maxWidth: .infinity, maxHeight: .infinity)
        } else {
            ProgressView().frame(maxWidth: .infinity, maxHeight: .infinity)
        }
    }

    private func url(_ path: String) -> URL? {
        changes?.repository.map { URL(fileURLWithPath: $0).appendingPathComponent(path) }
    }

    private func reload() {
        generation &+= 1
        let request = generation
        loading = true
        store.readWorkingChanges { result in
            guard request == generation else { return }
            loading = false
            switch result {
            case .success(let value):
                failure = nil
                changes = value
                if let selected = selection, !value.files.contains(where: { $0.path == selected }) { selection = nil; diff = nil }
                else { loadDiff(selection) }
            case .failure(let error): failure = error.localizedDescription
            }
        }
    }

    private func loadDiff(_ path: String?) {
        guard let path else { diff = nil; return }
        store.readFileDiff(path: path) { result in
            guard selection == path else { return }
            switch result {
            case .success(let value): diff = value
            case .failure(let error): diff = BindingFileDiff(path: path, lines: [BindingDiffLine(kind: .notice, text: error.localizedDescription)], truncated: false)
            }
        }
    }
}

private struct ChangedFileRow: View {
    let file: BindingChangedFile
    var body: some View {
        let name = (file.path as NSString).lastPathComponent, directory = (file.path as NSString).deletingLastPathComponent
        HStack(spacing: 8) {
            Text(kind.letter).font(.caption.monospaced().bold()).foregroundStyle(kind.color).frame(width: 14)
                .help(kind.title).accessibilityLabel(kind.title)
            VStack(alignment: .leading, spacing: 1) {
                Text(name).lineLimit(1).truncationMode(.middle)
                if !directory.isEmpty || kind.from != nil {
                    Text(kind.from.map { "← " + $0 } ?? directory).font(.caption).foregroundStyle(.secondary).lineLimit(1).truncationMode(.head)
                }
            }
            Spacer(minLength: 0)
            if let additions = file.additions, let deletions = file.deletions {
                HStack(spacing: 4) {
                    Text("+\(additions)").foregroundStyle(.green)
                    Text("−\(deletions)").foregroundStyle(.red)
                }.font(.caption.monospacedDigit())
            }
        }
        .help(file.path)
    }
    private var kind: (letter: String, title: String, color: Color, from: String?) {
        switch file.kind {
        case .added: ("A", "新增", .green, nil)
        case .modified: ("M", "修改", .orange, nil)
        case .deleted: ("D", "删除", .red, nil)
        case .renamed(let from): ("R", "重命名", .blue, from)
        case .untracked: ("?", "未跟踪", .secondary, nil)
        case .conflicted: ("U", "冲突", .red, nil)
        }
    }
}

private struct DiffLineRow: View {
    let line: BindingDiffLine
    var body: some View {
        HStack(alignment: .firstTextBaseline, spacing: 6) {
            Text(marker).foregroundStyle(.secondary).frame(width: 10, alignment: .center)
            Text(line.text.isEmpty ? " " : line.text).fixedSize(horizontal: false, vertical: true)
        }
        .font(.system(size: NSFont.smallSystemFontSize, design: .monospaced))
        .foregroundStyle(line.kind == .hunk || line.kind == .notice ? .secondary : .primary)
        .padding(.horizontal, 8).padding(.vertical, line.kind == .hunk ? 3 : 0)
        .frame(maxWidth: .infinity, alignment: .leading)
        .background(background)
    }
    private var marker: String {
        switch line.kind { case .added: "+"; case .removed: "−"; case .hunk, .context, .notice: "" }
    }
    private var background: Color {
        switch line.kind {
        case .added: .green.opacity(0.14)
        case .removed: .red.opacity(0.14)
        case .hunk: .secondary.opacity(0.1)
        case .context, .notice: .clear
        }
    }
}
