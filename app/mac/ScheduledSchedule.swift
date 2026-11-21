import SwiftUI
import VeluneBindings

struct ScheduledScheduleEditor: View {
    @Binding var task: ScheduledTask
    @Binding var ready: Bool
    @ObservedObject var store: DeviceStore
    @State private var mode = ""
    @State private var hour = 9
    @State private var minute = 0
    @State private var interval = 1
    @State private var weekdays: Set<Int> = [1]
    @State private var raw: String
    @State private var preview: SchedulePreview?
    @State private var error: String?
    private var requestID: String { "\(mode):\(hour):\(minute):\(interval):\(weekdays.sorted()):\(task.timeZone):" + ((mode == "custom" || mode.isEmpty) ? raw : "") }
    init(task: Binding<ScheduledTask>, ready: Binding<Bool>, store: DeviceStore) {
        _task = task; _ready = ready; self.store = store; _raw = State(initialValue: task.wrappedValue.rrule)
    }
    var body: some View {
        Group {
            if !mode.isEmpty {
                Picker("重复", selection: $mode) { Text("每天").tag("daily"); Text("每周").tag("weekly"); Text("按小时间隔").tag("hourly"); Text("自定义规则").tag("custom") }
                if mode == "weekly" {
                    HStack(spacing: 4) {
                        ForEach(1...7, id: \.self) { day in
                            Toggle(weekdayName(day), isOn: Binding(get: { weekdays.contains(day) }, set: { if $0 { weekdays.insert(day) } else { weekdays.remove(day) } })).toggleStyle(.button)
                        }
                    }
                }
                if mode == "daily" || mode == "weekly" {
                    LabeledContent("时间") {
                        DatePicker("时间", selection: time, displayedComponents: .hourAndMinute)
                            .datePickerStyle(.field)
                            .labelsHidden()
                            .fixedSize()
                            .environment(\.locale, Locale(identifier: "en_GB"))
                            .environment(\.timeZone, TimeZone(secondsFromGMT: 0)!)
                    }
                }
                if mode == "hourly" { Stepper("每 \(interval) 小时，在整点执行", value: $interval, in: 1...168) }
                if mode == "custom" {
                    Text("保留完整 RFC 5545 规则。复杂导入规则不会被简化。").uiFont(.caption).foregroundStyle(.secondary)
                    TextEditor(text: $raw).uiFont(.body.monospaced()).frame(minHeight: 90).border(.quaternary).accessibilityLabel("自定义日程规则")
                }
            }
            ScheduledTimeZonePicker(zone: $task.timeZone)
            if let error { Label(error, systemImage: "exclamationmark.circle").foregroundStyle(.red).textSelection(.enabled) }
            if !ready && error == nil { HStack { ProgressView().controlSize(.small); Text("正在计算触发时间…").foregroundStyle(.secondary) } }
            if let preview, ready {
                VStack(alignment: .leading, spacing: 5) {
                    Text("接下来三次（\(preview.timeZone)）").uiFont(.caption).foregroundStyle(.secondary)
                    if preview.nextAtUnixMs.isEmpty { Text("规则已无未来触发，不能启用。").foregroundStyle(.secondary) }
                    ForEach(preview.nextAtUnixMs, id: \.self) { Text(scheduleDate($0, zone: preview.timeZone)).monospacedDigit() }
                }
            }
        }
        .onChange(of: raw) { _, value in if mode == "custom" { task.rrule = value; ready = false } }
        .onChange(of: mode) { _, value in if value == "custom" { raw = task.rrule } }
        .onChange(of: requestID) { _, _ in ready = false; preview = nil }
        .task(id: requestID) {
            ready = false; error = nil
            do { try await Task.sleep(for: .milliseconds(150)) } catch { return }
            guard let transport = store.transport else { error = "本地核心未配置"; return }
            let id = requestID
            let pattern: SchedulePattern? = switch mode {
            case "daily": .daily(hour: UInt8(hour), minute: UInt8(minute))
            case "weekly": .weekly(weekdays: Data(weekdays.sorted().map(UInt8.init)), hour: UInt8(hour), minute: UInt8(minute))
            case "hourly": .hourly(interval: UInt16(interval))
            case "custom": .custom
            default: nil
            }
            let rule = mode == "custom" || mode.isEmpty ? raw : task.rrule
            let zone = task.timeZone
            let result = await withCheckedContinuation { continuation in
                store.performOnQueue { continuation.resume(returning: Result { try transport.scheduledTasks(.previewSchedule(rrule: rule, timeZone: zone, pattern: pattern)) }) }
            }
            guard !Task.isCancelled, id == requestID else { return }
            switch result {
            case .success(let state):
                guard let value = state.schedulePreview else { error = "核心未返回日程预览"; return }
                task.rrule = value.rrule; preview = value
                if mode.isEmpty { adopt(value.pattern) }
                if mode == "custom", raw != value.rrule { raw = value.rrule }
                ready = true
            case .failure(let failure):
                error = AppProblem.failure(failure, source: "定时任务").detail
                if mode.isEmpty { mode = "custom" }
            }
        }
    }
    // The picker edits wall-clock components, not an instant in the task's time zone.
    private static var timeCalendar: Calendar {
        var calendar = Calendar(identifier: .gregorian)
        calendar.timeZone = TimeZone(secondsFromGMT: 0)!
        return calendar
    }
    private var time: Binding<Date> {
        let calendar = Self.timeCalendar
        return Binding(get: {
            calendar.date(from: DateComponents(year: 2001, month: 1, day: 1, hour: hour, minute: minute))!
        }, set: { value in
            let components = calendar.dateComponents([.hour, .minute], from: value)
            hour = components.hour!; minute = components.minute!
        })
    }
    private func adopt(_ pattern: SchedulePattern) {
        switch pattern {
        case .daily(let h, let m): hour = Int(h); minute = Int(m); mode = "daily"
        case .weekly(let days, let h, let m): weekdays = Set(days.map(Int.init)); hour = Int(h); minute = Int(m); mode = "weekly"
        case .hourly(let value): interval = Int(value); mode = "hourly"
        case .custom: mode = "custom"
        }
    }
}

struct ScheduledScheduleSummary: View {
    let task: ScheduledTask
    @ObservedObject var store: DeviceStore
    @State private var preview: SchedulePreview?
    @State private var error: String?
    var body: some View {
        VStack(alignment: .leading, spacing: 12) {
            VStack(alignment: .leading, spacing: 4) {
                Text(preview?.pattern.scheduleTitle ?? (error == nil ? "正在读取日程…" : "日程不可用")).fontWeight(.medium)
                Text("时区：" + task.timeZone).uiFont(.callout).foregroundStyle(.secondary)
            }
            if let error {
                Label("日程需要处理：" + error, systemImage: "exclamationmark.circle")
                    .foregroundStyle(.secondary).textSelection(.enabled)
            }
            if let next = task.nextRunAtUnixMs, task.status == .active {
                scheduleTimes("下次执行", times: [next])
            }
            if let preview {
                let upcoming = preview.nextAtUnixMs.filter { task.status != .active || $0 != task.nextRunAtUnixMs }
                if preview.nextAtUnixMs.isEmpty {
                    Text("此日程已无后续执行时间。")
                        .uiFont(.callout).foregroundStyle(.secondary)
                } else if !upcoming.isEmpty {
                    scheduleTimes(task.status == .active ? "后续时间" : "规则预览", times: upcoming)
                }
            }
            DisclosureGroup("查看原始规则") {
                Text(task.rrule)
                    .uiFont(.caption.monospaced())
                    .multilineTextAlignment(.leading)
                    .textSelection(.enabled)
                    .frame(maxWidth: .infinity, alignment: .leading)
                    .padding(.top, 6)
            }
            .uiFont(.callout)
        }
        .frame(maxWidth: .infinity, alignment: .leading)
        .task {
            guard let transport = store.transport else { return }
            let result = await withCheckedContinuation { continuation in
                store.performOnQueue { continuation.resume(returning: Result { try transport.scheduledTasks(.previewSchedule(rrule: task.rrule, timeZone: task.timeZone, pattern: nil)) }) }
            }
            guard !Task.isCancelled else { return }
            switch result {
            case .success(let value):
                preview = value.schedulePreview
                if preview == nil { error = "核心未返回日程预览" }
            case .failure(let failure): error = AppProblem.failure(failure, source: "定时任务").detail
            }
        }
    }
    private func scheduleTimes(_ title: String, times: [Int64]) -> some View {
        HStack(alignment: .firstTextBaseline, spacing: 16) {
            Text(title).foregroundStyle(.secondary)
            VStack(alignment: .leading, spacing: 4) {
                ForEach(times, id: \.self) { Text(scheduleDate($0, zone: task.timeZone)).monospacedDigit() }
            }
        }
        .uiFont(.callout)
    }
}

struct ScheduledTimeZonePicker: View {
    @Binding var zone: String
    @State private var showing = false
    @State private var search = ""
    var body: some View {
        LabeledContent("时区") {
            Button(zone) { showing = true }.popover(isPresented: $showing) {
                VStack(spacing: 10) {
                    TextField("搜索城市或 IANA 时区", text: $search)
                    List((["UTC"] + TimeZone.knownTimeZoneIdentifiers.filter { $0 != "UTC" }).filter { search.isEmpty || $0.localizedCaseInsensitiveContains(search) }, id: \.self) { value in
                        Button(value) { zone = value; showing = false }.buttonStyle(.plain)
                    }
                }.padding(12).frame(width: 330, height: 340)
            }
        }
    }
}
private func weekdayName(_ value: Int) -> String { ["周一", "周二", "周三", "周四", "周五", "周六", "周日"][value - 1] }
extension SchedulePattern {
    var scheduleTitle: String {
        switch self {
        case .daily(let h, let m): "每天 " + String(format: "%02d:%02d", h, m)
        case .weekly(let days, let h, let m): days.map { weekdayName(Int($0)) }.joined(separator: "、") + " " + String(format: "%02d:%02d", h, m)
        case .hourly(let interval): "每 \(interval) 小时，整点执行"
        case .custom: "自定义日程"
        }
    }
}
