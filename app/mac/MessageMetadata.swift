import Foundation

/// Time in the message action row. Missing native time stays absent.
enum MessageMetadata {
    static func time(_ milliseconds: Int64?, now: Date = Date(), calendar: Calendar = .current) -> String? {
        guard let milliseconds else { return nil }
        let date = Date(timeIntervalSince1970: Double(milliseconds) / 1000)
        let format = DateFormatter()
        format.calendar = calendar
        format.timeZone = calendar.timeZone
        format.locale = Locale(identifier: "en_US_POSIX")
        format.dateFormat = calendar.isDate(date, inSameDayAs: now) ? "HH:mm"
            : calendar.component(.year, from: date) == calendar.component(.year, from: now) ? "MM-dd HH:mm" : "yyyy-MM-dd HH:mm"
        return format.string(from: date)
    }
}
