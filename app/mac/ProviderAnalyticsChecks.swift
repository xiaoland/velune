import AppKit
import SwiftUI
import VeluneBindings
import ObjectiveC

@MainActor private final class OffscreenProviderAnalyticsWindow: NSWindow {
    override var canBecomeKey: Bool { false }
    override var canBecomeMain: Bool { false }
    private func refusePresentation() -> Never {
        FileHandle.standardError.write(Data("FAIL provider analytics: presentation requested\n".utf8)); exit(1)
    }
    override func makeKey() { refusePresentation() }
    override func makeMain() { refusePresentation() }
    override func makeKeyAndOrderFront(_ sender: Any?) { refusePresentation() }
    override func orderFront(_ sender: Any?) { refusePresentation() }
    override func orderBack(_ sender: Any?) { refusePresentation() }
    override func orderFrontRegardless() { refusePresentation() }
    override func order(_ place: NSWindow.OrderingMode, relativeTo otherWin: Int) {
        if place != .out { refusePresentation() }
        super.order(place, relativeTo: otherWin)
    }
}

/// Production report views with explicit synthetic values, before SwiftUI App startup.
/// No core handle, credentials, network requests, visible windows or system input.
@MainActor func providerAnalyticsOffscreenChecks() -> Never {
    func fail(_ message: String) -> Never {
        FileHandle.standardError.write(Data("FAIL provider analytics: \(message)\n".utf8)); exit(1)
    }
    guard let outputPath = ProcessInfo.processInfo.environment["VELUNE_CHECK_OUTPUT"] else { fail("missing private output directory") }
    let directory = URL(fileURLWithPath: outputPath).standardizedFileURL.resolvingSymlinksInPath()
    guard directory.path.hasPrefix("/private/tmp/") || directory.path.hasPrefix("/tmp/") else { fail("output must be an owned temporary directory") }
    _ = NSApplication.shared
    // LSBackgroundOnly already prohibits activation. AppKit may refuse a
    // redundant policy change; the resulting policy is the safety condition.
    if NSApp.activationPolicy() != .prohibited { _ = NSApp.setActivationPolicy(.prohibited) }
    guard NSApp.activationPolicy() == .prohibited else { fail("activation policy is not prohibited") }
    var activations = 0
    var keyEvents = 0
    var mainEvents = 0
    let active = NotificationCenter.default.addObserver(forName: NSApplication.didBecomeActiveNotification, object: nil, queue: .main) { _ in activations += 1 }
    let keyed = NotificationCenter.default.addObserver(forName: NSWindow.didBecomeKeyNotification, object: nil, queue: .main) { _ in keyEvents += 1 }
    let main = NotificationCenter.default.addObserver(forName: NSWindow.didBecomeMainNotification, object: nil, queue: .main) { _ in mainEvents += 1 }
    defer { NotificationCenter.default.removeObserver(active); NotificationCenter.default.removeObserver(keyed); NotificationCenter.default.removeObserver(main) }
    var hosts: [OffscreenProviderAnalyticsWindow] = []
    let frontmost = NSWorkspace.shared.frontmostApplication?.processIdentifier
    func safe() {
        guard NSApp.activationPolicy() == .prohibited, !NSApp.isActive, !NSRunningApplication.current.isActive,
              activations == 0, keyEvents == 0, mainEvents == 0,
              NSWorkspace.shared.frontmostApplication?.processIdentifier == frontmost else { fail("activation or focus changed") }
        for window in NSApp.windows {
            guard !window.isVisible, !window.isKeyWindow, !window.isMainWindow else { fail("visible window") }
            if !hosts.contains(where: { $0 === window }) {
                let origin = class_getImageName(type(of: window)).map { String(cString: $0) } ?? ""
                guard origin.hasPrefix("/System/Library/") else { fail("unexpected application window") }
            }
        }
    }
    var captures: [String] = []
    func capture<V: View>(_ view: V, name: String, dark: Bool, width: CGFloat = 840, height: CGFloat = 720) {
        let host = NSHostingView(rootView: view.environment(\.colorScheme, dark ? .dark : .light).applicationAppearance().background(Color(nsColor: .windowBackgroundColor)))
        host.appearance = NSAppearance(named: dark ? .darkAqua : .aqua)
        host.frame = NSRect(x: 0, y: 0, width: width, height: height)
        let window = OffscreenProviderAnalyticsWindow(contentRect: host.frame, styleMask: [.borderless], backing: .buffered, defer: true)
        window.isReleasedWhenClosed = false; window.isRestorable = false
        hosts.append(window); window.contentView = host
        host.layoutSubtreeIfNeeded()
        RunLoop.main.run(until: Date().addingTimeInterval(0.2)); safe()
        host.layoutSubtreeIfNeeded(); host.displayIfNeeded()
        guard host.window === window, !window.isVisible, let bitmap = host.bitmapImageRepForCachingDisplay(in: host.bounds) else { fail("no offscreen bitmap") }
        host.cacheDisplay(in: host.bounds, to: bitmap)
        guard let png = bitmap.representation(using: .png, properties: [:]) else { fail("PNG encoding failed") }
        let filename = name + (dark ? "-dark" : "-light") + ".png"
        do { try png.write(to: directory.appendingPathComponent(filename)) } catch { fail(error.localizedDescription) }
        captures.append(filename); safe()
    }
    let start: Int64 = 1_793_491_200_000
    let end: Int64 = 1_794_848_400_000
    let now: Int64 = 1_794_848_400_000
    let profile = ProviderAccountProfile(lifetimeTokens: 1_524_000, peakDailyTokens: 450_000, longestTaskSeconds: 3600,
        currentStreakDays: 3, longestStreakDays: 9, fastModePercent: nil, reasoningEffort: nil, reasoningEffortPercent: nil,
        skillsExplored: nil, totalSkillsUsed: nil, totalThreads: 20,
        daily: [ProviderAccountDailyUsage(date: "2026-11-15", tokens: 0), ProviderAccountDailyUsage(date: "2026-11-16", tokens: 32_001)],
        topInvocations: [], statsAsOf: "2026-11-17", generatedAt: "2026-11-17T05:00:00Z")
    let credits = ProviderAccountCredits(hasCredits: true, unlimited: false, balance: "12.7500")
    let resets = ProviderAccountResets(availableCount: 2, grants: [ProviderAccountResetGrant(id: "synthetic-reset", resetType: "weekly", status: "available", grantedAt: "2026-11-01T00:00:00Z", expiresAt: "2026-12-01T00:00:00Z", title: "合成周限额重置权益", description: "仅为离屏验收提供的合成数据。")])
    let subscription = ProviderAnalyticsSource(id: "subscription", gatewayIds: ["default"], providerIds: ["chatgpt-1", "chatgpt-duplicate"], providerNames: ["ChatGPT 个人订阅", "同一账户的另一配置"], kind: .chatgptSubscription, identityKey: "synthetic-account", profileIdentityKey: "synthetic-profile", organizationId: nil, label: "ChatGPT 订阅账户", state: .available, scope: .subscriptionAccount, scopeId: nil, rangeStartAtMs: nil, rangeEndAtMs: nil, observedAtMs: now, dataRefreshedAtMs: nil, creditsObservedAtMs: now, resetCountObservedAtMs: now, resetGrantsObservedAtMs: now, profileObservedAtMs: now, stale: false, refreshing: false, staleAreas: [], usage: nil, costs: nil, profile: profile, credits: credits, bankedResets: resets, failures: [])
    let organization = ProviderAnalyticsSource(id: "organization", gatewayIds: ["default"], providerIds: ["api-root"], providerNames: ["生产 API（合成组织）"], kind: .openAiOrganization, identityKey: "synthetic-org", profileIdentityKey: nil, organizationId: "report-demo", label: "OpenAI Platform 组织 report-demo", state: .partialFailure, scope: .organization, scopeId: "report-demo", rangeStartAtMs: start, rangeEndAtMs: end, observedAtMs: now, dataRefreshedAtMs: nil, creditsObservedAtMs: nil, resetCountObservedAtMs: nil, resetGrantsObservedAtMs: nil, profileObservedAtMs: nil, stale: true, refreshing: false, staleAreas: [.organizationUsage], usage: ProviderAnalyticsUsage(inputTokens: 5_000_000, outputTokens: 840_000, requests: 2048, cachedInputTokens: 200_000, cacheCreationInputTokens: nil), costs: nil, profile: nil, credits: nil, bankedResets: nil, failures: [ProviderAnalyticsFailure(area: .costs, message: "合成失败：查询权限不足，请检查组织查询 Key 后刷新重试。")])
    let unconfigured = ProviderAnalyticsSource(id: "unconfigured", gatewayIds: ["default"], providerIds: ["unconfigured"], providerNames: ["尚未配置查询的 API 提供商"], kind: .anthropicOrganization, identityKey: nil, profileIdentityKey: nil, organizationId: nil, label: "Anthropic API", state: .unconfigured, scope: .organization, scopeId: nil, rangeStartAtMs: nil, rangeEndAtMs: nil, observedAtMs: nil, dataRefreshedAtMs: nil, creditsObservedAtMs: nil, resetCountObservedAtMs: nil, resetGrantsObservedAtMs: nil, profileObservedAtMs: nil, stale: false, refreshing: false, staleAreas: [], usage: nil, costs: nil, profile: nil, credits: nil, bankedResets: nil, failures: [])
    var secondOrganization = organization
    secondOrganization.id = "second-organization"
    secondOrganization.providerIds = ["api-second"]
    secondOrganization.providerNames = ["第二组织 API（合成）"]
    secondOrganization.identityKey = "synthetic-second-org"
    secondOrganization.organizationId = "second-demo"
    secondOrganization.label = "OpenAI Platform 组织 second-demo"
    secondOrganization.scopeId = "second-demo"
    secondOrganization.rangeEndAtMs = end - 3_600_000
    secondOrganization.state = .available
    secondOrganization.stale = false
    secondOrganization.staleAreas = []
    secondOrganization.failures = []
    secondOrganization.usage = ProviderAnalyticsUsage(inputTokens: 1_000_000, outputTokens: 100_000, requests: 80, cachedInputTokens: nil, cacheCreationInputTokens: nil)
    secondOrganization.costs = [BilledCost(amount: 9.5, currency: "USD")]
    var oldOrganization = organization
    oldOrganization.observedAtMs = now - 86_400_000
    oldOrganization.rangeEndAtMs = end - 86_400_000
    secondOrganization.observedAtMs = now - 1_800_000
    let total = ProviderAnalyticsOrganizationTotal(services: [.openAi], organizationIds: ["report-demo", "second-demo"], providerIds: ["api-root", "api-second"], scope: .organization, scopeIds: [], rangeStartAtMs: start, rangeEndAtMs: nil, rangeEndMinAtMs: end - 86_400_000, rangeEndMaxAtMs: end - 3_600_000, rangeIsHeterogeneous: true, inputTokens: 6_000_000, outputTokens: 940_000, usageSourceCount: 2, sourceCount: 2, costs: [BilledCost(amount: 9.5, currency: "USD")], costSourceCount: 1)
    let profileTotal = ProviderAnalyticsChatgptProfileTotal(profileCount: 1, profileReportedCount: 1, lifetimeTokens: 1_524_000, lifetimeSourceCount: 1, dailyReportedProfileCount: 1, daily: [ProviderAnalyticsDailyTotal(date: "2026-11-15", tokens: 0, sourceCount: 1), ProviderAnalyticsDailyTotal(date: "2026-11-16", tokens: 32_001, sourceCount: 1)])
    let report = ProviderAnalyticsReport(observedAtMs: now, sources: [subscription, oldOrganization, secondOrganization, unconfigured], chatgptProfileTotals: profileTotal, organizationTotals: [total])
    var longName = oldOrganization
    longName.providerNames = ["用于验证小窗口换行与阅读的长提供商名称：同一组织的研发团队共享 API 查询配置（纯合成数据）"]
    let account = ProviderAccountStatus(gatewayId: "default", providerId: "chatgpt-1", identityKey: "synthetic-account", profileIdentityKey: nil, state: .partialFailure, fetchedAt: "2026-11-17T05:00:00Z", creditsObservedAt: "2026-11-17T05:00:00Z", resetCountObservedAt: "2026-11-17T05:00:00Z", resetGrantsObservedAt: nil, profileObservedAt: "2026-11-16T05:00:00Z", refreshing: false, stale: true, staleSources: [.profile, .resets], credits: credits, bankedResets: ProviderAccountResets(availableCount: 2, grants: nil), profile: profile, failures: [ProviderAccountFailure(source: .resets, message: "合成失败：权益明细未能读取，已保留独立报告的可用次数。")])
    let preview = DeviceStore(preview: true)
    let before = ProviderReportReadIdentity(store: preview, providerID: "chatgpt-1")
    guard before == ProviderReportReadIdentity(store: preview, providerID: "chatgpt-1") else { fail("ordinary reads must keep state identity") }
    preview.invalidateProviderReports()
    guard before != ProviderReportReadIdentity(store: preview, providerID: "chatgpt-1"),
          ProviderReportReadIdentity(store: preview, providerID: "chatgpt-1") != ProviderReportReadIdentity(store: preview, providerID: "chatgpt-2"),
          ProviderReportReadIdentity(store: preview) != ProviderReportReadIdentity(store: DeviceStore(preview: true)) else { fail("configuration, provider and device changes must invalidate old report state") }
    for dark in [false, true] {
        capture(ProviderAnalyticsContent(report: report, reading: false, problem: nil, retry: {}), name: "provider-overview", dark: dark, height: 1040)
        capture(Form { Section("订阅账户报告") { ProviderAnalyticsSourceDetail(source: subscription) } }.formStyle(.grouped), name: "provider-subscription", dark: dark, width: 620)
        capture(Form { Section("组织报告") { ProviderAnalyticsSourceDetail(source: longName) } }.formStyle(.grouped), name: "provider-partial-narrow", dark: dark, width: 540)
        capture(Form { Section("订阅账户") { ProviderAccountSnapshot(status: account) } }.formStyle(.grouped), name: "provider-account-partial", dark: dark, width: 540, height: 640)
        capture(ProviderAnalyticsContent(report: report, reading: true, problem: "部分查询仍在后台进行；已有结果继续保留。", retry: {}), name: "provider-retained", dark: dark, height: 1040)
        capture(Form {
            Section("已报告为零") { ProviderAccountAmounts(credits: ProviderAccountCredits(hasCredits: false, unlimited: false, balance: "0"), resets: ProviderAccountResets(availableCount: 0, grants: [])) }
            Section("已报告不限量") { ProviderAccountAmounts(credits: ProviderAccountCredits(hasCredits: true, unlimited: true, balance: nil), resets: nil) }
            Section("尚未报告") { ProviderAccountAmounts(credits: nil, resets: nil) }
        }.formStyle(.grouped), name: "provider-amount-states", dark: dark, width: 540, height: 620)
        capture(Form {
            Section("按来源日期报告") { ProviderAccountDailyContent(daily: profile.daily!) }
            Section("重置权益明细") { ProviderAccountResetGrantContent(grant: resets.grants!.first!) }
        }.formStyle(.grouped), name: "provider-daily-reset-detail", dark: dark, width: 620, height: 620)
        capture(ProviderAnalyticsContent(report: nil, reading: true, problem: nil, retry: {}), name: "provider-loading", dark: dark, width: 540, height: 360)
        capture(ProviderAnalyticsContent(report: ProviderAnalyticsReport(observedAtMs: now, sources: [], chatgptProfileTotals: ProviderAnalyticsChatgptProfileTotal(profileCount: 0, profileReportedCount: 0, lifetimeTokens: nil, lifetimeSourceCount: 0, dailyReportedProfileCount: 0, daily: []), organizationTotals: []), reading: false, problem: nil, retry: {}), name: "provider-empty", dark: dark, width: 540, height: 360)
        capture(ProviderAnalyticsContent(report: nil, reading: false, problem: "合成失败：当前执行设备不可达，请恢复连接后重试。", retry: {}), name: "provider-failed", dark: dark, width: 540, height: 360)
    }
    safe()
    let evidence: [String: Any] = ["synthetic": true, "networkCalls": 0, "visibleWindows": 0, "activationEvents": activations,
        "keyEvents": keyEvents, "mainEvents": mainEvents, "captures": captures, "nativeControls": "SwiftUI Form, DisclosureGroup, LabeledContent, ProgressView"]
    do { try JSONSerialization.data(withJSONObject: evidence, options: [.prettyPrinted, .sortedKeys]).write(to: directory.appendingPathComponent("provider-analytics-check.json")) } catch { fail(error.localizedDescription) }
    print("PASS provider analytics synthetic offscreen captures: \(captures.count), no network or foreground activation")
    exit(0)
}
