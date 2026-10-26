import Foundation

enum TokenCount {
    static let inputError = "Token 数须为正整数，可用 K、M、B（如 272K、1.5M），且不超过 4.294967295B；最大输出不能超过上下文窗口。"

    // 显示值也会填入编辑器，保留余数才能让未修改的配置无损保存。
    static func format<T: BinaryInteger>(_ value: T) -> String {
        for (scale, suffix) in [(1_000_000_000, "B"), (1_000_000, "M"), (1_000, "K")] {
            let divisor = T(scale)
            if value >= divisor {
                let remainder = value % divisor
                guard remainder != 0 else { return "\(value / divisor)\(suffix)" }
                var fraction = String(remainder)
                fraction = String(repeating: "0", count: String(scale).count - 1 - fraction.count) + fraction
                while fraction.last == "0" { fraction.removeLast() }
                return "\(value / divisor).\(fraction)\(suffix)"
            }
        }
        return String(value)
    }

    static func rate(_ value: Double) -> String {
        let (scale, suffix): (Double, String) = value >= 1_000_000_000 ? (1_000_000_000, "B") : value >= 1_000_000 ? (1_000_000, "M") : value >= 1_000 ? (1_000, "K") : (1, "")
        return (value / scale).formatted(.number.locale(Locale(identifier: "en_US_POSIX")).grouping(.never).precision(.fractionLength(0...2))) + suffix
    }

    static func parse(_ text: String) -> UInt32? {
        var value = text.trimmingCharacters(in: .whitespacesAndNewlines)
        guard value.range(of: #"\A(?:[0-9]+|[0-9]{1,3}(?:,[0-9]{3})+)(?:\.[0-9]+)? *[kKmMbB]?\z"#, options: .regularExpression) != nil else { return nil }
        let power: Int
        switch value.last {
        case "k", "K": power = 3; value.removeLast()
        case "m", "M": power = 6; value.removeLast()
        case "b", "B": power = 9; value.removeLast()
        default: power = 0
        }
        let parts = value.trimmingCharacters(in: .whitespaces).replacingOccurrences(of: ",", with: "").split(separator: ".")
        var fraction = parts.count == 2 ? String(parts[1]) : ""
        while fraction.last == "0" { fraction.removeLast() }
        guard fraction.count <= power else { return nil }
        let digits = String(parts[0]) + fraction + String(repeating: "0", count: power - fraction.count)
        guard let count = UInt32(digits), count > 0 else { return nil }
        return count
    }

    static func validLimits(context: String, output: String) -> Bool {
        guard validOptional(context), validOptional(output) else { return false }
        if let window = parse(context), let maximum = parse(output) { return maximum <= window }
        return true
    }

    static func validOptional(_ value: String) -> Bool {
        value.trimmingCharacters(in: .whitespacesAndNewlines).isEmpty || parse(value) != nil
    }
}
