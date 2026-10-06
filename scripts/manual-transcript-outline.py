#!/usr/bin/env python3
"""Explicit actual Store/UniFFI/Pi transcript acceptance with synthetic loopback only; never CI."""
import argparse
import json
from pathlib import Path
import shutil
import subprocess
import tempfile
import threading
import time
from http.server import BaseHTTPRequestHandler, ThreadingHTTPServer

SWIFT = r'''
import Foundation
import Darwin
import VeluneBindings
@main struct Manual {
    @MainActor static var stores: [AppStore] = []
    static func require(_ condition: @autoclosure () -> Bool, _ message: String = "acceptance condition failed", line: Int = #line) throws {
        guard condition() else { throw NSError(domain: "VeluneManualAcceptance", code: 1, userInfo: [NSLocalizedDescriptionKey: message + " at Swift line \(line)"]) }
    }
    @MainActor static func wait(_ condition: @escaping () -> Bool) async throws {
        let deadline = Date().addingTimeInterval(35)
        while !condition() && Date() < deadline { stores.forEach { $0.manualPoll() }; try? await Task.sleep(nanoseconds: 25_000_000) }
        if !condition() { for store in stores { print("STATE loading=\(store.isLoading) rows=\(store.transcript.rows.map { $0.message.role.rawValue + ":" + String(reflecting: $0.message.blocks) }) turns=\(store.transcript.turns) problems=\(store.problems.map(\.detail))") } }
        try require(condition(), "actual operation did not settle")
    }
    @MainActor static func main() async {
        do { try await run() }
        catch { print("FAILED: \(error.localizedDescription)"); exit(1) }
    }
    @MainActor static func run() async throws {
        setbuf(stdout, nil)
        let args = CommandLine.arguments
        let root = URL(fileURLWithPath: args[1]), resources = URL(fileURLWithPath: args[2])
        var transports: [Transport] = []
        func opened() -> AppStore {
            let transport = Transport(stateDirectory: root.appendingPathComponent("home"), resourcesDirectory: resources)
            transports.append(transport)
            let store = AppStore(transport: transport); stores.append(store); return store
        }
        defer { for transport in transports { try? transport.close() } }
        let store = opened()
        print("PHASE start"); store.start(); try await wait { !store.isLoading }
        print("PHASE provider"); store.saveProvider(AIProvider(id: "fixture", name: "Synthetic", protocolID: .chatCompletionsV1, endpoint: args[5],
            models: [ProviderModel(recordKey: "", providerModelID: "synthetic", nickname: "Synthetic", contextWindow: 4096, maxOutputTokens: 128)]), authenticationEdit: .setAPIKey("synthetic-only"))
        try await wait { !store.isLoading }
        try require(store.problems.isEmpty)
        let key = store.providers[0].models[0].recordKey
        print("PHASE runtime"); store.saveRuntimeInstance(RuntimeInstance(id: "fixture", name: "Synthetic Pi", typeID: "pi-1.0.2", gatewayID: "default",
            settings: ["binary": args[4], "nodeBinary": args[3], "agentDir": root.appendingPathComponent("runtime").path]))
        try await wait { !store.isLoading }
        var created = false
        print("PHASE create"); store.createConversation(runtimeID: "fixture", cwd: root.appendingPathComponent("project").path, modelRecordKey: key) { created = true }
        try await wait { !store.isLoading }; try require(created, "create failed: \(store.problems.map(\.detail))")
        print("PHASE first-send canSend=\(store.canSend) problems=\(store.problems.map(\.detail))"); store.send(text: "READ_FIXTURE")
        print("PHASE first-prefix"); try await wait { FileManager.default.fileExists(atPath: root.appendingPathComponent("ready-first").path) && store.transcript.rows.contains { $0.message.role == .assistant } }
        let firstAssistant = store.transcript.rows.first { $0.message.role == .assistant }!
        let firstIdentity = ObjectIdentifier(firstAssistant)
        let initialParse = firstAssistant.parseCount
        try require(store.transcript.turns.count == 1 && store.transcript.turns[0].isRunning, "first-prefix turns=\(store.transcript.turns) rows=\(store.transcript.rows.map { $0.message.role.rawValue + ":" + String(reflecting: $0.message.blocks) })")
        try! FileManager.default.removeItem(at: root.appendingPathComponent("gate-first"))
        print("PHASE final-prefix"); try await wait { FileManager.default.fileExists(atPath: root.appendingPathComponent("ready-final").path) && store.transcript.rows.filter { $0.message.role == .assistant }.count >= 2 }
        try require(store.transcript.rows.contains { ObjectIdentifier($0) == firstIdentity })
        try require(firstAssistant.parseCount >= initialParse)
        let streamingTurn = store.transcript.turns[0]
        try require(!streamingTurn.workMessageIDs.isEmpty && !streamingTurn.workMessageIDs.contains(streamingTurn.userMessageID))
        try require(streamingTurn.lastMessageID != nil && !streamingTurn.workMessageIDs.contains(streamingTurn.lastMessageID!))
        var outline = TranscriptOutlineState()
        try require(outline.expand(from: streamingTurn.userMessageID, in: store.transcript.rows))
        let expandedBeforeConfirmation: Set<String> = [streamingTurn.id]
        try! FileManager.default.removeItem(at: root.appendingPathComponent("gate-final"))
        print("PHASE final-settle"); try await wait { store.canSend && store.transcript.rows.contains { $0.message.text == "SYNTHETIC_FINAL" } }
        try require(store.problems.isEmpty)
        let finished = store.transcript.turns[0]
        try require(!finished.isRunning && (finished.durationMs ?? 0) > 0, "final lifecycle=\(finished)")
        let visibleItems = store.transcript.items(for: store.transcript.rows)
        let directIDs = visibleItems.compactMap { item -> String? in if case .message(let row) = item { return row.id }; return nil }
        try require(directIDs.contains(finished.userMessageID) && directIDs.contains(finished.lastMessageID!))
        try require(finished.workMessageIDs.allSatisfy { !directIDs.contains($0) })
        try require(visibleItems.contains { item in if case .work(let group, let rows) = item { return group.id == finished.id && rows.map(\.id) == finished.workMessageIDs }; return false })
        let parseAfterFirstTurn = firstAssistant.parseCount
        outline.confirmIdentities(store.transcript.confirmedMessageIDs)
        try require(!outline.reconcile(store.transcript.rows) && outline.expandedFromUserID == finished.userMessageID)
        try require(store.transcript.retainedExpandedTurnIDs(expandedBeforeConfirmation) == [finished.id])
        let finalFirstAssistantID = firstAssistant.id
        outline.collapse()
        try require(outline.visibleRows(store.transcript.rows).allSatisfy { $0.message.role == .user })
        try require(outline.expand(from: finished.userMessageID, in: store.transcript.rows))
        try require(outline.visibleRows(store.transcript.rows).contains { $0.id == finished.lastMessageID })
        store.setTranscriptPresentation(.userOutline); try await wait { !store.isLoading }
        try require(store.transcriptPresentation == .userOutline)
        outline.followSentUser(after: store.transcript.userRows.last?.id, in: store.transcript.rows)
        try require(outline.expandedFromUserID == finished.userMessageID, "send before native arrival expanded a different user")
        print("PHASE second-send"); store.send(text: "SECOND_USER")
        try await wait { store.canSend && store.transcript.userRows.count == 2 && store.transcript.turns.count == 2 }
        try require(store.problems.isEmpty)
        try require(firstAssistant.parseCount == parseAfterFirstTurn, "unchanged Markdown was reparsed")
        try require(store.transcript.rows.contains { ObjectIdentifier($0) == firstIdentity })
        let secondUser = store.transcript.userRows.last!.id
        outline.confirmIdentities(store.transcript.confirmedMessageIDs)
        _ = outline.reconcile(store.transcript.rows)
        try require(outline.expandedFromUserID == secondUser, "late native user did not fulfill explicit send-follow intent")
        try require(outline.expand(from: secondUser, in: store.transcript.rows))
        let suffix = outline.visibleRows(store.transcript.rows)
        try require(suffix.contains { $0.id == finished.userMessageID } && !suffix.contains { $0.id == firstAssistant.id })
        // Feed actual native projected messages into the same UI state function.
        // Position changes do not change the selected identity; deleting that
        // identity returns to the user-only outline instead of guessing a row.
        let ui = TranscriptModel()
        let shifted = [Message(id: "synthetic-position-shift", role: .system, blocks: [.notice("Synthetic")])] + store.transcript.rows.map(\.message)
        ui.apply(shifted, turns: store.transcript.turns)
        try require(!outline.reconcile(ui.rows) && outline.expandedFromUserID == secondUser)
        try require(outline.visibleRows(ui.rows).map(\.id) == suffix.map(\.id))
        ui.apply(shifted.filter { $0.id != secondUser }, turns: [])
        try require(outline.reconcile(ui.rows) && outline.expandedFromUserID == nil)
        try require(outline.visibleRows(ui.rows).allSatisfy { $0.message.role == .user })
        outline.collapse(); try require(outline.expandedFromUserID == nil)
        let originalConversationID = store.loadedConversationID!
        let turnIDs = store.transcript.turns.map(\.id)
        var stopped: Bool?
        store.shutdown { stopped = $0 }; try await wait { stopped != nil }; try require(stopped == true)
        stores.removeAll { $0 === store }
        let reopened = opened()
        print("PHASE reopen"); reopened.start(); try await wait { !reopened.isLoading && reopened.loadedConversationID == originalConversationID }
        try require(reopened.transcriptPresentation == .userOutline)
        try require(reopened.transcript.turns.map(\.id) == turnIDs)
        try require(reopened.transcript.turns.allSatisfy { $0.durationMs == nil && !$0.isRunning }, "historical work duration was invented")
        try require(reopened.transcript.userRows.count == 2)
        try require(reopened.transcript.rows.contains { $0.id == finalFirstAssistantID }, "canonical finalized message identity changed on reopen")
        var nextCreated = false
        reopened.createConversation(runtimeID: "fixture", cwd: root.appendingPathComponent("project").path, modelRecordKey: key) { nextCreated = true }
        try await wait { !reopened.isLoading && nextCreated }
        try require(reopened.loadedConversationID != originalConversationID && reopened.transcript.rows.isEmpty)
        stopped = nil; reopened.shutdown { stopped = $0 }; try await wait { stopped != nil }; try require(stopped == true)
        print("{\"acceptance\":\"PASSED\",\"actualStoreAndPiToolStream\":true,\"typedCoreWorkReferences\":true,\"userAndLastVisible\":true,\"observedDurationAndUnknownHistory\":true,\"stableRowAndMarkdownCache\":true,\"canonicalConfirmationState\":true,\"lateNativeUserFollowsExplicitSend\":true,\"outlineSuffixAndAnchorChanges\":true,\"nativeConversationSwitchResetsRows\":true,\"preferenceSurvivesReopen\":true}")
    }
}
'''


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    for name in ('bundle', 'swift-build', 'node', 'pi'):
        parser.add_argument('--' + name, required=True, type=Path)
    parser.add_argument('--library', type=Path, help='optional freshly built debug library; default is the given bundle library')
    args = parser.parse_args()
    for path in vars(args).values():
        if path is not None and (not path.is_absolute() or not path.exists()):
            parser.error('absolute existing paths required')
    repository = Path(__file__).resolve().parent.parent
    requests = []
    with tempfile.TemporaryDirectory(prefix='velune-transcript-') as directory:
        root = Path(directory)
        for name in ('home', 'runtime', 'project', 'frameworks', 'resources'):
            (root / name).mkdir()
        for helper in (repository / 'packages/agent-runtime/resources').glob('*.mjs'): shutil.copy(helper, root / 'resources')
        (root / 'resources/node_modules').symlink_to(args.bundle / 'Contents/Resources/node_modules')
        file = root / 'project/fixture.txt'
        file.write_text('SYNTHETIC_TOOL_RESULT\n')
        for name in ('gate-first', 'gate-final'):
            (root / name).touch()

        class Upstream(BaseHTTPRequestHandler):
            def log_message(self, *_):
                pass

            def do_POST(self):
                body = json.loads(self.rfile.read(int(self.headers['Content-Length'])))
                requests.append(body)
                print("UPSTREAM request", len(requests), flush=True)
                number = len(requests)
                self.send_response(200)
                self.send_header('Content-Type', 'text/event-stream')
                self.end_headers()

                def chunk(delta, finish=None):
                    value = {'id': f'synthetic-{number}', 'object': 'chat.completion.chunk', 'model': body['model'], 'created': 1,
                             'choices': [{'index': 0, 'delta': delta, 'finish_reason': finish}]}
                    if finish is not None: value['usage'] = {'prompt_tokens': 2, 'completion_tokens': 3, 'total_tokens': 5}
                    self.wfile.write(('data: ' + json.dumps(value) + '\n\n').encode())
                    self.wfile.flush()

                if number <= 2:
                    suffix = 'first' if number == 1 else 'final'
                    chunk({'role': 'assistant', 'content': 'Checking ' if number == 1 else 'SYNTHETIC'})
                    (root / ('ready-' + suffix)).touch()
                    deadline = time.monotonic() + 35
                    while (root / ('gate-' + suffix)).exists() and time.monotonic() < deadline:
                        time.sleep(.02)
                    if (root / ('gate-' + suffix)).exists():
                        raise TimeoutError('acceptance did not explicitly release ' + suffix + ' stream gate')
                    if number == 1:
                        chunk({'content': 'fixture', 'tool_calls': [{'index': 0, 'id': 'fixture_read', 'type': 'function',
                              'function': {'name': 'read', 'arguments': json.dumps({'path': str(file)})}}]})
                    else:
                        assert 'SYNTHETIC_TOOL_RESULT' in json.dumps(body), 'actual tool result missing'
                        chunk({'content': '_FINAL'})
                else:
                    chunk({'role': 'assistant', 'content': 'SECOND_FINAL'})
                chunk({}, 'tool_calls' if number == 1 else 'stop')
                self.wfile.write(b'data: [DONE]\n\n')
                self.wfile.flush()

        server = ThreadingHTTPServer(('127.0.0.1', 0), Upstream)
        threading.Thread(target=server.serve_forever, daemon=True).start()
        try:
            main_source = root / 'Manual.swift'; main_source.write_text(SWIFT)
            store_source = root / 'Store.swift'
            store_source.write_text((repository / 'app/mac/Store.swift').read_text() + '\nextension AppStore { func manualPoll() { poll() } }\n')
            import_models = root / 'ImportModels.swift'
            import_models.write_text((repository / 'app/mac/ProviderImport.swift').read_text().split('struct ProviderImportView: View {')[0])
            build = args.swift_build
            objects = [str(path) for name in ('VeluneBindings', 'MarkdownUI', 'NetworkImage', 'cmark_gfm', 'cmark_gfm_extensions')
                       for path in (build / (name + '.build')).rglob('*.o')]
            library = args.library or args.bundle / 'Contents/Frameworks/libvelune_bindings.dylib'
            (root / 'frameworks/libvelune_bindings.dylib').symlink_to(library)
            command = ['xcrun', 'swiftc', '-parse-as-library', '-swift-version', '5', '-warnings-as-errors', '-I', str(build / 'Modules'), '-I', str(repository / 'target/swift-ffi')]
            for path in (repository / '.build/checkouts/swift-cmark/src/include/module.modulemap', repository / '.build/checkouts/swift-cmark/extensions/include/module.modulemap'):
                command += ['-Xcc', '-fmodule-map-file=' + str(path)]
            command += [str(repository / 'app/mac' / name) for name in ('Models.swift', 'TranscriptModel.swift', 'Transport.swift', 'BindingMapping.swift', 'ConversationBrowser.swift', 'Problems.swift')]
            command += [str(store_source), str(import_models), str(main_source), *objects, '-L', str(root / 'frameworks'), '-lvelune_bindings', '-Xlinker', '-rpath', '-Xlinker', str(root / 'frameworks'), '-o', str(root / 'manual')]
            subprocess.run(command, check=True, cwd=repository)
            env = {'HOME': str(root / 'home'), 'PATH': str(args.node.parent) + ':/usr/bin:/bin', 'NO_PROXY': '127.0.0.1,localhost'}
            subprocess.run([str(root / 'manual'), str(root), str(root / 'resources'), str(args.node), str(args.pi),
                            'http://127.0.0.1:%d/v1' % server.server_port], cwd=root, env=env, check=True, timeout=100)
            assert len(requests) == 3, 'unexpected provider retries'
        finally:
            server.shutdown(); server.server_close()


if __name__ == '__main__':
    main()
