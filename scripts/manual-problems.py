#!/usr/bin/env python3
"""Explicit isolated actual AppStore/Transport/UniFFI Problems acceptance, never CI."""
import argparse
import os
from pathlib import Path
import shutil
import subprocess
import tempfile

SWIFT = r'''
import Foundation
import Darwin
import VeluneBindings
@main struct Manual {
    static func require(_ condition: @autoclosure () -> Bool, _ message: String = "acceptance condition failed") throws {
        guard condition() else { throw NSError(domain: "VeluneManualAcceptance", code: 1, userInfo: [NSLocalizedDescriptionKey: message]) }
    }
    @MainActor static func wait(_ condition: @escaping () -> Bool) async throws {
        let deadline = Date().addingTimeInterval(20)
        while !condition() && Date() < deadline { try? await Task.sleep(nanoseconds: 20_000_000) }
        try require(condition(), "operation did not settle")
    }
    @MainActor static func main() async {
        do { try await run() } catch { print("FAILED: \(error.localizedDescription)"); exit(1) }
    }
    @MainActor static func run() async throws {
        let args = CommandLine.arguments
        let root = URL(fileURLWithPath: args[1]), resources = URL(fileURLWithPath: args[2])
        let transport = Transport(stateDirectory: root.appendingPathComponent("home"), resourcesDirectory: resources)
        defer { try? transport.close() }
        let store = AppStore(transport: transport)
        store.start(); try await wait { !store.isLoading }
        try require(store.problems.isEmpty)
        let invalid = AIProvider(id: "synthetic", name: "Synthetic", protocolID: .chatCompletionsV1, endpoint: "not a url",
            models: [ProviderModel(recordKey: "", providerModelID: "fixture", nickname: "Synthetic")])
        store.saveProvider(invalid, authenticationEdit: .setAPIKey("synthetic-only"))
        try await wait { !store.isLoading }
        try require(store.problems.count == 1)
        let failure = store.problems[0]
        try require(failure.source == "保存 AI 提供商")
        try require(failure.kind == "invalid" && failure.code != nil && failure.phase != nil && failure.operationID != nil)
        try require(failure.diagnosticText.contains(failure.operationID!))
        store.refreshConversations(); try await wait { !store.isLoading }
        try require(store.problems == [failure], "successful read erased retained operation diagnostic")
        store.clearProblem(failure.id); try require(store.problems.isEmpty)
        let runtime = RuntimeInstance(id: "fixture", name: "Synthetic runtime", typeID: "pi-1.0.2", gatewayID: "default",
            settings: ["binary": args[4], "nodeBinary": args[3], "agentDir": root.appendingPathComponent("runtime").path])
        store.saveRuntimeInstance(runtime)
        try await wait { !store.isLoading }
        try require(store.problems.count == 1 && store.problems[0].activityKey == "history:fixture")
        try require(store.problems[0].kind == "history" && store.problems[0].operationID == nil)
        let history = store.problems[0]
        store.refreshConversations(); try await wait { !store.isLoading }
        try require(store.problems == [history], "repeated history read duplicated active issue")
        try! FileManager.default.removeItem(at: root.appendingPathComponent("history-fails"))
        store.refreshConversations(); try await wait { !store.isLoading }
        try require(store.problems.isEmpty, "successful native history read did not resolve issue")
        var importSucceeded = false
        store.previewProviderImport(ProviderImportSource(harnessTypeId: "pi", sourceInstanceId: "fixture", settings: [:])) { _ in importSucceeded = true }
        try await wait { !store.isLoading }
        try require(store.problems.count == 1)
        try require(!importSucceeded)
        let importFailure = store.problems[0]
        try require(importFailure.detail.contains("SYNTHETIC_UNIQUE_IMPORT_CAUSE"))
        try require(importFailure.code != nil && importFailure.phase != nil && importFailure.operationID != nil)
        try require(importFailure.diagnosticText.contains("SYNTHETIC_UNIQUE_IMPORT_CAUSE"))
        store.clearProblems()
        let underlying = NSError(domain: "SyntheticInner", code: 2, userInfo: [NSLocalizedDescriptionKey: "SYNTHETIC_UNDERLYING_CAUSE"])
        let outer = NSError(domain: "SyntheticOuter", code: 1, userInfo: [NSLocalizedDescriptionKey: "SYNTHETIC_DESCRIPTION", NSLocalizedFailureReasonErrorKey: "SYNTHETIC_REASON", NSLocalizedRecoverySuggestionErrorKey: "SYNTHETIC_RECOVERY", NSUnderlyingErrorKey: underlying])
        let boundary = Transport(stateDirectory: root.appendingPathComponent("home"), resourcesDirectory: resources)
        store.recordProblem(boundary.manualMap(outer), source: "Synthetic NSError")
        let detail = store.problems[0].detail
        try require(["SYNTHETIC_DESCRIPTION", "SYNTHETIC_REASON", "SYNTHETIC_RECOVERY", "SYNTHETIC_UNDERLYING_CAUSE"].allSatisfy(detail.contains))
        store.clearProblems()
        // Explicitly exercise the activity identity rule with typed data after
        // actual Core diagnostics above: per-request IDs must not flood a timer.
        let first = TransportError.diagnostic(kind: "io", detail: "SYNTHETIC_POLL_FAILURE", code: "fixture", phase: "snapshot", operationID: "one")
        let second = TransportError.diagnostic(kind: "io", detail: "SYNTHETIC_POLL_FAILURE", code: "fixture", phase: "snapshot", operationID: "two")
        store.recordProblem(first, source: "读取会话", activityKey: "poll:fixture")
        store.recordProblem(second, source: "读取会话", activityKey: "poll:fixture")
        try require(store.problems.count == 1 && store.problems[0].operationID == "one")
        store.clearProblems(); try require(store.problems.isEmpty)
        for index in 0..<105 { store.recordProblem("SYNTHETIC_\(index)") }
        try require(store.problems.count == 100 && store.problems.first?.detail == "SYNTHETIC_5")
        store.clearProblems()
        var stopped: Bool?
        store.shutdown { stopped = $0 }; try await wait { stopped != nil }
        try require(stopped == true)
        print("{\"acceptance\":\"PASSED\",\"actualTypedProviderFailure\":true,\"successRetainsOperation\":true,\"explicitClear\":true,\"actualNativeHistoryFailure\":true,\"historyDeduplicatesAndResolves\":true,\"actualUniqueImportCause\":true,\"NSErrorCauseReasonRecovery\":true,\"typedActivityDeduplicates\":true,\"bounded100\":true}")
    }
}
'''


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    for name in ('bundle', 'swift-build', 'node', 'pi'):
        parser.add_argument('--' + name, required=True, type=Path)
    parser.add_argument('--library', type=Path, help='optional fresh debug dylib')
    args = parser.parse_args()
    for path in vars(args).values():
        if path is not None and (not path.is_absolute() or not path.exists()):
            parser.error('absolute existing paths required')
    repository = Path(__file__).resolve().parent.parent
    with tempfile.TemporaryDirectory(prefix='velune-problems-') as directory:
        root = Path(directory)
        for name in ('home', 'runtime', 'resources', 'frameworks'):
            (root / name).mkdir()
        resources = root / 'resources'
        original = args.bundle / 'Contents/Resources'
        for path in original.iterdir():
            if path.suffix == '.mjs':
                shutil.copy(path, resources / path.name)
            elif path.name == 'node_modules':
                (resources / path.name).symlink_to(path)
        (root / 'history-fails').touch()
        (resources / 'pi_sessions.mjs').write_text('import {existsSync} from "node:fs";\n' +
            'if(existsSync(' + repr(str(root / 'history-fails')) + ')) process.exit(2);\n' +
            'await import(' + repr((original / 'pi_sessions.mjs').as_uri()) + ');\n')
        (resources / 'pi_provider_import.mjs').write_text('console.error("SYNTHETIC_UNIQUE_IMPORT_CAUSE"); process.exit(2);\n')
        library = args.library or args.bundle / 'Contents/Frameworks/libvelune_bindings.dylib'
        (root / 'frameworks/libvelune_bindings.dylib').symlink_to(library)
        transport_source = root / 'Transport.swift'
        transport_source.write_text((repository / 'app/mac/Transport.swift').read_text() + '\nextension Transport { func manualMap(_ e: Error) -> TransportError { map(e) } }\n')
        main_source = root / 'Manual.swift'
        main_source.write_text(SWIFT)
        import_models = root / 'ImportModels.swift'
        import_models.write_text((repository / 'app/mac/ProviderImport.swift').read_text().split('struct ProviderImportView: View {')[0])
        build = args.swift_build
        objects = [str(path) for name in ('VeluneBindings', 'MarkdownView', 'Markdown', 'Highlightr', 'RichText', 'Introspection', 'SwiftMath', 'CAtomic', 'cmark_gfm', 'cmark_gfm_extensions')
                   for path in (build / (name + '.build')).rglob('*.o')]
        command = ['xcrun', 'swiftc', '-parse-as-library', '-swift-version', '5', '-warnings-as-errors', '-I', str(build / 'Modules'), '-I', str(repository / 'target/swift-ffi')]
        command += ['-Xcc', '-I' + str(repository/'.build/checkouts/swift-cmark/src/include'), '-Xcc', '-fmodule-map-file=' + str(repository/'.build/checkouts/swift-markdown/Sources/CAtomic/include/module.modulemap')]
        for path in (repository / '.build/checkouts/swift-cmark/src/include/module.modulemap', repository / '.build/checkouts/swift-cmark/extensions/include/module.modulemap'):
            command += ['-Xcc', '-fmodule-map-file=' + str(path)]
        command += [str(repository / 'app/mac' / name) for name in ('Models.swift', 'TranscriptModel.swift', 'BindingMapping.swift', 'ConversationBrowser.swift', 'Problems.swift', 'Store.swift')]
        command += [str(import_models), str(transport_source), str(main_source), *objects, '-L', str(root / 'frameworks'), '-lvelune_bindings', '-Xlinker', '-rpath', '-Xlinker', str(root / 'frameworks'), '-o', str(root / 'manual')]
        subprocess.run(command, check=True, cwd=repository)
        for name in ('Highlightr_Highlightr.bundle', 'SwiftMath_SwiftMath.bundle'):
            shutil.copytree(build/name, root/name)
        env = {'HOME': str(root / 'home'), 'PATH': str(args.node.parent) + ':/usr/bin:/bin', 'NO_PROXY': '127.0.0.1,localhost'}
        subprocess.run([str(root / 'manual'), str(root), str(resources), str(args.node), str(args.pi)], cwd=root, env=env, check=True, timeout=70)


if __name__ == '__main__':
    main()
