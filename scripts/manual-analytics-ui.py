#!/usr/bin/env python3
"""Manual actual Mac Store/Transport/UniFFI read acceptance of a synthetic gateway database; never CI."""
import argparse
from pathlib import Path
import shutil
import sqlite3
import subprocess
import tempfile

SWIFT = r'''
import Foundation
import Darwin
import VeluneBindings
@main struct Manual {
    static func require(_ value: @autoclosure () -> Bool, _ message: String = "acceptance failed", line: Int = #line) throws {
        guard value() else { throw NSError(domain: "VeluneManual", code: 1, userInfo: [NSLocalizedDescriptionKey: message + " at line \(line)"]) }
    }
    @MainActor static func wait(_ condition: @escaping () -> Bool) async throws {
        let deadline = Date().addingTimeInterval(20)
        while !condition() && Date() < deadline { try? await Task.sleep(nanoseconds: 20_000_000) }
        try require(condition(), "actual operation did not settle")
    }
    @MainActor static func main() async {
        do { try await run() } catch { print("FAILED: \(error.localizedDescription)"); exit(1) }
    }
    @MainActor static func run() async throws {
        let args = CommandLine.arguments
        let root = URL(fileURLWithPath: args[1])
        let transport = Transport(stateDirectory: root.appendingPathComponent("home"), resourcesDirectory: root.appendingPathComponent("resources"))
        defer { try? transport.close() }
        let store = AppStore(transport: transport)
        store.start(); try await wait { !store.isLoading }
        try require(store.problems.isEmpty && store.loadedConversationID == nil)
        let from = Int64(args[2])!, to = Int64(args[3])!
        let count = UInt64(args[4])!, input = UInt64(args[5])!, output = UInt64(args[6])!, total = UInt64(args[7])!, eligible = UInt64(args[8])!
        func query(limit: UInt32 = 100, provider: String? = nil, model: String? = nil) -> BindingAnalyticsQuery {
            BindingAnalyticsQuery(fromMs: from, toMs: to, bucketBoundariesMs: [from, to], providerId: provider, modelRecordKey: model, requestLimit: limit)
        }
        func read(_ value: BindingAnalyticsQuery) async throws -> BindingAnalyticsReport {
            store.loadAnalytics(value)
            try require(!store.isLoading, "analysis blocked conversation state")
            try await wait { !store.analyticsIsLoading }
            try require(!store.analyticsReadFailed && store.problems.isEmpty)
            try require(store.analyticsReport != nil)
            return store.analyticsReport!
        }
        print("PHASE bounded-query")
        let bounded = try await read(query(limit: 2))
        try require(bounded.requests.count == 2 && bounded.overview.requestCount == count, "page limit truncated aggregate population")
        try require(bounded.overview.inputTokens == input && bounded.overview.outputTokens == output && bounded.overview.totalTokens == total)
        try require(bounded.overview.eligibleSpeedCount == eligible, "eligible speed actual=\(bounded.overview.eligibleSpeedCount) expected=\(eligible)")
        try require(bounded.overview.outputTokensPerSecond != nil)
        try require(store.analyticsUpdatedAt != nil && store.loadedConversationID == nil && store.nextTurnRuntimeID == nil)
        print("PHASE full-details")
        let all = try await read(query(limit: 500))
        try require(all.requests.count == Int(count) && all.storageWarning == nil && all.droppedCount == 0)
        try require(all.requests.contains { $0.providerModelId == "chat-missing" }, "source fixture missing chat-missing")
        let unknown = all.requests.first { $0.providerModelId == "chat-missing" }!
        try require(unknown.inputTokens == nil && unknown.outputTokens == nil && unknown.outputTokensPerSecond == nil)
        try require(unknown.protocol == .chatCompletions && unknown.outcome == .completed)
        store.loadAnalytics(query(provider: unknown.providerId, model: unknown.modelRecordKey))
        try require(store.analyticsReport?.overview.requestCount == count && store.analyticsLoadedQuery?.providerId == nil, "pending scope relabelled previous report")
        try await wait { !store.analyticsIsLoading }
        try require(store.analyticsLoadedQuery?.providerId == unknown.providerId && store.analyticsLoadedQuery?.modelRecordKey == unknown.modelRecordKey, "success did not atomically accept query/report")
        let filtered = store.analyticsReport!
        try require(filtered.overview.requestCount == 1 && filtered.overview.totalTokens == nil && filtered.overview.eligibleSpeedCount == 0)
        try require(filtered.requests.first?.requestId == unknown.requestId && filtered.trend.first?.totals.totalTokens == nil)
        try require(manualAnalyticsTokenText(nil) == "—" && manualAnalyticsTokenText(0) == "0")
        let allAgain = try await read(query(limit: 500))
        for row in allAgain.models {
            try require(row.providerId != nil && row.modelRecordKey != nil, "model breakdown lost typed source filter")
        }
        try require(["chat-error", "chat-truncated", "chat-cancel"].allSatisfy { name in allAgain.requests.contains { $0.providerModelId == name } }, "source fixture missing outcome cases")
        let failed = allAgain.requests.first { $0.providerModelId == "chat-error" }!
        try require(failed.outcome == .failed && failed.outputTokens == 1 && failed.outputTokensPerSecond == nil)
        let truncated = allAgain.requests.first { $0.providerModelId == "chat-truncated" }!
        try require(truncated.outcome == .incomplete && truncated.terminalElapsedMs == nil && truncated.outputTokensPerSecond == nil)
        let cancelled = allAgain.requests.first { $0.providerModelId == "chat-cancel" }!
        try require(cancelled.outcome == .cancelled && cancelled.outputTokensPerSecond == nil)
        try require(allAgain.requests.contains { $0.providerModelId == "messages-uncached-json" } && allAgain.requests.contains { $0.providerModelId == "messages-zero-json" }, "source fixture missing ordinary-input/zero cases")
        let ordinary = allAgain.requests.first { $0.providerModelId == "messages-uncached-json" }!
        try require(ordinary.uncachedInputTokens == 20 && ordinary.inputTokens == nil && ordinary.outputTokens == 3)
        let zero = allAgain.requests.first { $0.providerModelId == "messages-zero-json" }!
        try require(zero.inputTokens == 0 && zero.outputTokens == 0 && zero.usageComplete)
        try require(allAgain.overview.totalReportedCount == 11, "paired cohort lost explicit coverage")
        try require(allAgain.overview.incompleteCount == 1 && allAgain.overview.failedCount == 1 && allAgain.overview.cancelledCount == 1)
        print("PHASE latest-query-and-diagnostics")
        let invalid = BindingAnalyticsQuery(fromMs: to, toMs: from, bucketBoundariesMs: [to, from], providerId: nil, modelRecordKey: nil, requestLimit: 100)
        let successfulTime = store.analyticsUpdatedAt
        let successfulFirstRequest = store.analyticsReport?.requests.first?.requestId
        store.loadAnalytics(invalid)
        try require(store.analyticsIsLoading && store.analyticsReport?.requests.first?.requestId == successfulFirstRequest && store.analyticsUpdatedAt == successfulTime, "refresh removed last successful report/time")
        try require(store.analyticsLoadedQuery?.fromMs == from && store.analyticsLoadedQuery?.toMs == to && store.analyticsLoadedQuery?.providerId == nil, "pending invalid range relabelled previous report")
        try await wait { !store.analyticsIsLoading }
        try require(store.analyticsReadFailed && store.analyticsReport?.requests.first?.requestId == successfulFirstRequest && store.analyticsUpdatedAt == successfulTime && store.problems.count == 1)
        try require(store.analyticsLoadedQuery?.fromMs == from && store.analyticsLoadedQuery?.toMs == to, "failure replaced accepted range")
        try require(store.problems[0].source == "读取分析记录" && store.problems[0].code != nil && store.problems[0].operationID != nil)
        store.clearProblems()
        store.loadAnalytics(invalid); store.loadAnalytics(query(limit: 2))
        try await wait { !store.analyticsIsLoading }
        try require(!store.analyticsReadFailed && store.analyticsReport?.overview.requestCount == count && store.problems.isEmpty, "late old query overwrote latest analysis state")
        var calendar = Calendar(identifier: .gregorian)
        calendar.timeZone = TimeZone(identifier: "America/Los_Angeles")!
        let now = calendar.date(from: DateComponents(year: 2026, month: 3, day: 10, hour: 12))!
        let boundaries = manualAnalyticsDayBoundaries(now: now, calendar: calendar)
        try require(boundaries.last == Int64(now.timeIntervalSince1970 * 1_000), "plot padding changed actual query end")
        try require(manualAnalyticsPlotEnd(now: now, calendar: calendar) == calendar.date(from: DateComponents(year: 2026, month: 3, day: 11))!, "partial day bar domain does not include full local day")
        try require(boundaries.count == 8 && zip(boundaries, boundaries.dropFirst()).contains { $1 - $0 == 23 * 60 * 60 * 1_000 }, "local day buckets ignored DST")
        try require(store.loadedConversationID == nil && !store.isLoading && store.nextTurnRuntimeID == nil)
        var stopped: Bool?
        store.shutdown { stopped = $0 }; try await wait { stopped != nil }; try require(stopped == true)
        print("{\"acceptance\":\"PASSED\",\"actualStoreTransportUniFFI\":true,\"gatewayObservedSyntheticDatabase\":true,\"fullPopulationDespitePageLimit\":true,\"typedSourceFiltering\":true,\"unknownAndZeroDistinct\":true,\"typedFailedTruncatedCancelledDetails\":true,\"latestQueryWins\":true,\"diagnosticProblem\":true,\"localDSTBuckets\":true,\"conversationStateIndependent\":true,\"refreshKeepsSuccessfulDataAndScope\":true}")
    }
}
'''

def main():
    parser = argparse.ArgumentParser(description=__doc__)
    for name in ('database', 'library', 'swift-build'):
        parser.add_argument('--' + name, type=Path, required=True)
    for name, default in (('requests', 14), ('input', 112), ('output', 50), ('total', 159), ('eligible', 9)):
        parser.add_argument('--expected-' + name, type=int, default=default)
    args = parser.parse_args()
    for path in (args.database, args.library, args.swift_build):
        if not path.is_absolute() or not path.exists():
            parser.error('absolute existing paths required')
    repository = Path(__file__).resolve().parent.parent
    # This database is explicitly produced by the synthetic manual gateway run.
    # Only its clock bounds are read here; all assertions consume real typed queries.
    with sqlite3.connect(f'file:{args.database}?mode=ro', uri=True) as db:
        first, last = db.execute('SELECT MIN(started_at_ms),MAX(terminal_at_ms) FROM request_usage').fetchone()
    if first is None:
        parser.error('synthetic gateway fixture must contain observed requests')
    with tempfile.TemporaryDirectory(prefix='velune-analytics-ui-') as directory:
        root = Path(directory)
        for name in ('home', 'resources', 'frameworks'):
            (root / name).mkdir()
        shutil.copy(args.database, root / 'home/analytics.sqlite')
        (root / 'frameworks/libvelune_bindings.dylib').symlink_to(args.library)
        analytics = root / 'Analytics.swift'
        analytics.write_text('import Foundation\nimport VeluneBindings\n' + 'private enum AnalyticsFormatting {' + (repository / 'app/mac/Analytics.swift').read_text().split('private enum AnalyticsFormatting {', 1)[1] + '\nfunc manualAnalyticsTokenText(_ value: UInt64?) -> String { AnalyticsFormatting.tokens(value) }\nfunc manualAnalyticsDayBoundaries(now: Date, calendar: Calendar) -> [Int64] { AnalyticsPeriod.sevenDays.timeRange(now: now, calendar: calendar).boundaries }\nfunc manualAnalyticsPlotEnd(now: Date, calendar: Calendar) -> Date { AnalyticsPeriod.sevenDays.timeRange(now: now, calendar: calendar).plotEnd }\n')
        source = root / 'Manual.swift'; source.write_text(SWIFT)
        build = args.swift_build
        objects = [str(path) for name in ('VeluneBindings', 'MarkdownView', 'Markdown', 'Highlightr', 'RichText', 'Introspection', 'SwiftMath', 'CAtomic', 'cmark_gfm', 'cmark_gfm_extensions') for path in (build / (name + '.build')).rglob('*.o')]
        command = ['xcrun', 'swiftc', '-parse-as-library', '-swift-version', '5', '-warnings-as-errors', '-I', str(build / 'Modules'), '-I', str(repository / 'target/swift-ffi')]
        command += ['-Xcc', '-I' + str(repository/'.build/checkouts/swift-cmark/src/include'), '-Xcc', '-fmodule-map-file=' + str(repository/'.build/checkouts/swift-markdown/Sources/CAtomic/include/module.modulemap')]
        for path in (repository / '.build/checkouts/swift-cmark/src/include/module.modulemap', repository / '.build/checkouts/swift-cmark/extensions/include/module.modulemap'):
            command += ['-Xcc', '-fmodule-map-file=' + str(path)]
        import_models = root / 'ImportModels.swift'
        import_models.write_text((repository / 'app/mac/ProviderImport.swift').read_text().split('struct ProviderImportView: View {')[0])
        command += [str(repository / 'app/mac' / name) for name in ('Models.swift', 'TranscriptModel.swift', 'BindingMapping.swift', 'ConversationBrowser.swift', 'Problems.swift', 'Store.swift', 'Transport.swift')]
        command += [str(import_models)]
        command += [str(analytics), str(source), *objects, '-L', str(root / 'frameworks'), '-lvelune_bindings', '-Xlinker', '-rpath', '-Xlinker', str(root / 'frameworks'), '-o', str(root / 'manual')]
        subprocess.run(command, check=True, cwd=repository)
        for name in ('Highlightr_Highlightr.bundle', 'SwiftMath_SwiftMath.bundle'):
            shutil.copytree(build/name, root/name)
        env = {'HOME': str(root / 'home'), 'VELUNE_HOME': str(root / 'home'), 'PATH': '/usr/bin:/bin'}
        subprocess.run([str(root / 'manual'), str(root), str(first - 1), str(last + 1), str(args.expected_requests), str(args.expected_input), str(args.expected_output), str(args.expected_total), str(args.expected_eligible)], env=env, cwd=root, check=True, timeout=60)

if __name__ == '__main__':
    main()
