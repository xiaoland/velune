import AppKit
import SwiftUI

enum AppearanceSize: String, Codable, CaseIterable, Sendable {
    case small, standard, large, extraLarge
    var title: String { switch self { case .small: "小"; case .standard: "默认"; case .large: "大"; case .extraLarge: "超大" } }
    var scale: CGFloat { switch self { case .small: 0.92; case .standard: 1; case .large: 1.15; case .extraLarge: 1.3 } }
}
enum AppearanceDensity: String, Codable, CaseIterable, Sendable {
    case compact, standard, spacious
    var title: String { switch self { case .compact: "紧凑"; case .standard: "默认"; case .spacious: "宽松" } }
    var spacing: CGFloat { switch self { case .compact: 0.75; case .standard: 1; case .spacious: 1.3 } }
    var controls: ControlSize { self == .compact ? .small : self == .spacious ? .large : .regular }
}
struct AppearanceValues: Codable, Equatable, Sendable {
    var ui = AppearanceSize.standard
    var messages = AppearanceSize.standard
    var code = AppearanceSize.standard
    var density = AppearanceDensity.standard
    var composerWidth = 960.0
}

/// Native measurement can run off the main thread; it reads a complete value.
final class AppearanceRendering: @unchecked Sendable {
    private let lock = NSLock()
    private var value = AppearanceValues()
    func read() -> AppearanceValues { lock.withLock { value } }
    func set(_ value: AppearanceValues) { lock.withLock { self.value = value } }
}

extension Notification.Name {
    static let veluneAppearanceChanged = Notification.Name("VeluneAppearanceChanged")
}

@MainActor final class AppearancePreferences: ObservableObject {
    static let shared = AppearancePreferences()
    nonisolated static let rendering = AppearanceRendering()
    @Published var values = AppearanceValues() {
        didSet {
            guard values != oldValue else { return }
            Self.rendering.set(values)
            NotificationCenter.default.post(name: .veluneAppearanceChanged, object: nil)
            if let file { persist(file) }
        }
    }
    private var file: URL?
    private var report: (String) -> Void = { _ in }
    func load(home: URL?, report: @escaping (String) -> Void) {
        self.report = report
        file = nil
        values = AppearanceValues()
        guard let home else { values = AppearanceValues(); return }
        let target = home.appendingPathComponent("mac-appearance.json")
        if FileManager.default.fileExists(atPath: target.path) {
            switch Result(catching: { try JSONDecoder().decode(AppearanceValues.self, from: Data(contentsOf: target)) }) {
            case .success(let saved):
                if saved.composerWidth.isFinite && (640...1440).contains(saved.composerWidth) { values = saved }
                else { report("外观配置中的输入区宽度无效") }
            case .failure(let error): report(error.localizedDescription)
            }
        }
        file = target
    }
    private func persist(_ target: URL) {
        let result = Result {
            try FileManager.default.createDirectory(at: target.deletingLastPathComponent(), withIntermediateDirectories: true)
            try JSONEncoder().encode(values).write(to: target, options: .atomic)
        }
        if case .failure(let error) = result { report(error.localizedDescription) }
    }
}

/// Semantic UI sizes remain relative to the chosen UI size. Code and transcript
/// text use their own native typography, rather than inheriting this scale.
struct UITypeStyle {
    let size: CGFloat
    var weight = Font.Weight.regular
    var design = Font.Design.default
    var digits = false
    static let body = Self(size: 13)
    static let callout = Self(size: 12)
    static let caption = Self(size: 10)
    static let caption2 = Self(size: 10)
    static let headline = Self(size: 13, weight: .semibold)
    static let subheadline = Self(size: 12)
    static let footnote = Self(size: 10)
    static let title = Self(size: 22)
    static let title2 = Self(size: 17)
    static let title3 = Self(size: 15)
    static let largeTitle = Self(size: 34)
    func weight(_ value: Font.Weight) -> Self { var copy = self; copy.weight = value; return copy }
    func bold() -> Self { weight(.bold) }
    func monospaced() -> Self { var copy = self; copy.design = .monospaced; return copy }
    func monospacedDigit() -> Self { var copy = self; copy.digits = true; return copy }
    func font(scale: CGFloat) -> Font {
        let value = Font.system(size: size * scale, weight: weight, design: design)
        return digits ? value.monospacedDigit() : value
    }
}
private struct UIFontModifier: ViewModifier {
    let style: UITypeStyle
    @ObservedObject private var appearance = AppearancePreferences.shared
    func body(content: Content) -> some View { content.font(style.font(scale: appearance.values.ui.scale)) }
}
private struct AppearanceModifier: ViewModifier {
    @ObservedObject private var appearance = AppearancePreferences.shared
    func body(content: Content) -> some View {
        content.font(UITypeStyle.body.font(scale: appearance.values.ui.scale))
            .controlSize(appearance.values.density.controls)
    }
}
extension View {
    func uiFont(_ style: UITypeStyle) -> some View { modifier(UIFontModifier(style: style)) }
    func applicationAppearance() -> some View { modifier(AppearanceModifier()) }
}

struct AppearanceSettingsSection: View {
    @ObservedObject private var appearance = AppearancePreferences.shared
    var body: some View {
        Section("外观") {
            size("界面字号", selection: $appearance.values.ui)
            size("消息字号", selection: $appearance.values.messages)
            size("代码字号", selection: $appearance.values.code)
            Picker("紧凑程度", selection: $appearance.values.density) {
                ForEach(AppearanceDensity.allCases, id: \.self) { Text($0.title).tag($0) }
            }.pickerStyle(.segmented)
            LabeledContent("输入区最大宽度") {
                Slider(value: $appearance.values.composerWidth, in: 640...1440, step: 40)
                Text("\(Int(appearance.values.composerWidth)) pt").monospacedDigit().frame(width: 64, alignment: .trailing)
            }
            Button("恢复外观默认值") { appearance.values = AppearanceValues() }.disabled(appearance.values == AppearanceValues())
        }
    }
    private func size(_ title: String, selection: Binding<AppearanceSize>) -> some View {
        Picker(title, selection: selection) {
            ForEach(AppearanceSize.allCases, id: \.self) { Text($0.title).tag($0) }
        }.pickerStyle(.segmented)
    }
}
