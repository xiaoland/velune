#!/usr/bin/env python3
"""Explicit Mac AppStore/Transport loading acceptance with isolated Pi history.
Compiles the production Store plus a same-file visibility shim for one poll;
uses actual UniFFI/core/helper execution, not mocked Store or response models.
No window, real credentials or external upstream is used. Three controlled loopback
turns exercise Pi execution and cross-instance continuation; this is not an automated-test entry point.
Requires an external Pi 1.0.2 CLI; the seed imports that installation's public SDK export.
"""
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
import AppKit
import Combine
import Foundation
import VeluneBindings
@main struct ManualLoading {
    @MainActor static func main() throws {
        let args = CommandLine.arguments
        let root = URL(fileURLWithPath: args[1]), resources = URL(fileURLWithPath: args[2])
        let ids = Array(args[3...6]), node = args[7], endpoint = args[8], pi = args[9]
        let application = try VeluneApplication.open(options: BindingOptions(homeDirectory: root.appendingPathComponent("application").path, resourcesDirectory: resources.path))
        _ = try application.upsertRuntime(runtime: BindingRuntimeInstance(enabled: true, id: "fixture", name: "Synthetic", typeId: "pi-1.0.2", gatewayId: "default", settings: ["binary":pi,"nodeBinary":node,"agentDir":root.appendingPathComponent("runtime").path]))
        _ = try application.upsertRuntime(runtime: BindingRuntimeInstance(enabled: true, id: "other", name: "Other", typeId: "pi-1.0.2", gatewayId: "default", settings: ["binary":pi,"nodeBinary":node,"agentDir":root.appendingPathComponent("runtime-other").path]))
        _ = try application.saveProvider(gatewayId: "default", provider: BindingProviderDraft(id: "synthetic-provider", name: "Synthetic", protocol: .chatCompletionsV1, endpoint: endpoint, models: [BindingProviderModel(recordKey: "", providerModelId: "synthetic", nickname: "Synthetic", icon: nil, contextWindow: 8192, maxOutputTokens: 128, reasoningLevels: nil, adapterMetadataJson: nil)]), authenticationEdit: .setApiKey(value: "SYNTHETIC_ONLY"))
        _ = try application.selectRuntime(id: "fixture")
        try application.shutdown()
        let store = AppStore(transport: Transport(stateDirectory: root.appendingPathComponent("application"), resourcesDirectory: resources))
        func wait(_ predicate: () -> Bool) {
            let deadline = Date().addingTimeInterval(12)
            while !predicate() && Date() < deadline { RunLoop.main.run(until: Date().addingTimeInterval(0.01)) }
            precondition(predicate(), "operation did not reach expected state")
        }
        store.start()
        wait { store.loadedConversationID == ids[0] && !store.isLoading }
        let historyModelBeforeSelection = store.manualSnapshotModelKey()
        let nextModel = store.models.first!.recordKey
        store.selectNextTurnRuntime("fixture"); store.selectModel(modelRecordKey: nextModel)
        precondition(store.nextTurnRuntimeID == "fixture" && store.nextTurnModelRecordKey == nextModel)
        precondition(store.manualSnapshotModelKey() == historyModelBeforeSelection)
        let otherID = "other:" + ids[0].dropFirst("fixture:".count).replacingOccurrences(of: "/runtime/", with: "/runtime-other/")
        precondition(store.conversations.contains { $0.id == otherID })
        store.selectConversation(id: otherID)
        wait { !store.isLoading && store.loadedConversationID == otherID }
        precondition(store.projectionRuntimeID == "other")
        precondition(store.nextTurnRuntimeID == "fixture" && store.nextTurnModelRecordKey == nextModel)
        precondition(store.canSend, "valid next-turn intent must not be bound to the history source")
        store.selectNextTurnRuntime("other")
        precondition(store.nextTurnModelRecordKey == nextModel && store.canSend)

        store.selectConversation(id: ids[0])
        store.selectNextTurnRuntime("other"); store.selectModel(modelRecordKey: nextModel)
        wait { !store.isLoading && store.loadedConversationID == ids[0] }
        precondition(store.projectionRuntimeID == "fixture")
        precondition(store.nextTurnRuntimeID == "other" && store.nextTurnModelRecordKey == nextModel)
        store.selectNextTurnRuntime("fixture")

        let loadedBeforeBrowsing = store.loadedConversationID
        var browser = ConversationBrowser()
        let runtimeB = RuntimeInstance(id: "other", name: "Other", typeID: "pi-1.0.2", gatewayID: "default", settings: [:])
        let runtimes = store.runtimeInstances
        let rows = [
            Conversation(id: "fixture:same", title: "Same", updatedAtUnixMs: 300, createdAtUnixMs: 100, runtimeID: "fixture", cwd: "/one/project"),
            Conversation(id: "other:same", title: "Same", updatedAtUnixMs: 200, createdAtUnixMs: 400, runtimeID: "other", cwd: "/two/project"),
            Conversation(id: "unknown", title: "Unknown", updatedAtUnixMs: nil, runtimeID: "other", cwd: nil)
        ]
        precondition(browser.sections(conversations: rows, runtimes: runtimes).flatMap(\.conversations).map(\.id) == ["fixture:same", "other:same", "unknown"])
        browser.sort = .created
        precondition(browser.sections(conversations: rows, runtimes: runtimes)[0].conversations.first?.id == "other:same")
        browser.oldestFirst = true
        precondition(browser.sections(conversations: rows, runtimes: runtimes)[0].conversations.last?.id == "unknown")
        browser.grouping = .project
        precondition(browser.sections(conversations: rows, runtimes: runtimes).count == 3)
        browser.project = .path("/one/project")
        precondition(browser.sections(conversations: rows, runtimes: runtimes).flatMap(\.conversations).map(\.id) == ["fixture:same"])
        browser.project = .all; browser.grouping = .runtime
        precondition(browser.sections(conversations: rows, runtimes: runtimes).count == 2)
        var disabledB = runtimeB; disabledB.enabled = false
        precondition(browser.sections(conversations: rows, runtimes: store.runtimeInstances.filter { $0.id != "other" } + [disabledB]).flatMap(\.conversations).count == 1)
        precondition(store.loadedConversationID == loadedBeforeBrowsing)
        browser = ConversationBrowser(); browser.grouping = .runtime
        let many = (0..<45).flatMap { index in [Conversation(id: "fixture:page\(index)", title: "Page", updatedAtUnixMs: Int64(index), runtimeID: "fixture", cwd: "/one/project"), Conversation(id: "other:page\(index)", title: "Page", updatedAtUnixMs: Int64(index), runtimeID: "other", cwd: "/two/project")] }
        var pages = browser.sections(conversations: many, runtimes: runtimes)
        precondition(pages.count == 2 && pages.allSatisfy { $0.conversations.count == 20 && $0.totalCount == 45 && $0.hasMore })
        browser.loadMore(.runtime("fixture")); pages = browser.sections(conversations: many, runtimes: runtimes)
        precondition(pages.first { $0.id == .runtime("fixture") }?.conversations.count == 40)
        precondition(pages.first { $0.id == .runtime("other") }?.conversations.count == 20)
        browser.resetPagination(); browser.initialLimit = 7
        precondition(browser.sections(conversations: many, runtimes: runtimes).allSatisfy { $0.conversations.count == 7 })
        store.setConversationBrowserGroupLimit(7); wait { !store.isLoading && store.conversationBrowserGroupLimit == 7 }

        let firstRows = store.transcript.rows.map(ObjectIdentifier.init)
        var selections: [String?] = []
        let observer = store.$selectedConversationID.sink { selections.append($0) }
        store.manualPoll()
        Thread.sleep(forTimeInterval: 0.15) // Queue the old poll's main-thread delivery.
        selections.removeAll()
        try "hold".write(to: root.appendingPathComponent("gate"), atomically: true, encoding: .utf8)
        store.selectConversation(id: ids[1])
        precondition(store.selectedConversationID == ids[1] && store.pendingConversationID == ids[1])
        wait { FileManager.default.fileExists(atPath: root.appendingPathComponent("ready").path) }
        precondition(store.isLoading && store.loadedConversationID == ids[0])
        precondition(store.transcript.rows.map(ObjectIdentifier.init) == firstRows)
        precondition(selections.allSatisfy { $0 == ids[1] }, "late poll restored old selection")
        store.selectConversation(id: ids[2]) // Serialized: selection-disabled sidebar cannot issue this in the UI.
        precondition(store.selectedConversationID == ids[1], "concurrent selection changed pending target")
        try FileManager.default.removeItem(at: root.appendingPathComponent("gate"))
        wait { !store.isLoading && store.pendingConversationID == nil }
        precondition(store.loadedConversationID == ids[1] && store.selectedConversationID == ids[1])
        precondition(selections.allSatisfy { $0 == ids[1] }, "selection bounced during successful load")
        let loadedRows = store.transcript.rows.map(ObjectIdentifier.init)
        store.clearProblems()
        selections.removeAll()
        store.selectConversation(id: ids[2])
        precondition(store.selectedConversationID == ids[2] && store.pendingConversationID == ids[2])
        wait { !store.isLoading && store.pendingConversationID == nil }
        precondition(store.loadedConversationID == ids[1] && store.selectedConversationID == ids[1])
        precondition(store.transcript.rows.map(ObjectIdentifier.init) == loadedRows && !store.problems.isEmpty)
        precondition(selections == [ids[2],ids[1]], "failed load had unexpected selection transitions")
        func nativePath(_ id: String) -> URL { URL(fileURLWithPath: String(id.split(separator: ":", maxSplits: 1)[1])) }
        func beginHeldLoad(_ id: String) throws {
            let ready = root.appendingPathComponent("ready")
            if FileManager.default.fileExists(atPath: ready.path) { try FileManager.default.removeItem(at: ready) }
            try "hold".write(to: root.appendingPathComponent("gate"), atomically: true, encoding: .utf8)
            store.selectConversation(id: id)
            wait { FileManager.default.fileExists(atPath: ready.path) }
            precondition(store.pendingConversationID == id && store.canManageConversations)
        }
        func releaseHeldLoad() throws {
            try FileManager.default.removeItem(at: root.appendingPathComponent("gate"))
            wait { !store.isLoading && store.pendingConversationID == nil && store.conversationManagementStatus == nil }
        }
        func conversation(_ id: String) -> Conversation { store.conversations.first { $0.id == id }! }
        // Rename the pending destination, then rename the original loaded session
        // while another destination is opening. Native writes start after open.
        try beginHeldLoad(ids[0])
        var renamedTarget = false
        store.renameConversation(conversation(ids[0]), title: "A_RENAMED") { renamedTarget = true }
        precondition(store.conversationManagementStatus?.contains("加载完成后") == true)
        precondition(!renamedTarget && store.loadedConversationID == ids[1])
        try releaseHeldLoad()
        precondition(renamedTarget && store.loadedConversationID == ids[0])
        precondition(store.selectedConversationTitle == "A_RENAMED")
        try beginHeldLoad(ids[1])
        var renamedOriginal = false
        store.renameConversation(conversation(ids[0]), title: "A_RENAMED_AGAIN") { renamedOriginal = true }
        try releaseHeldLoad()
        precondition(renamedOriginal && store.loadedConversationID == ids[1])
        precondition(conversation(ids[0]).title == "A_RENAMED_AGAIN")
        // Deleting the old loaded source must not clear the successfully loaded destination.
        try beginHeldLoad(ids[0])
        store.deleteConversation(conversation(ids[1]))
        try releaseHeldLoad()
        precondition(store.loadedConversationID == ids[0] && !FileManager.default.fileExists(atPath: nativePath(ids[1]).path))
        // Failed loading can still rename the requested source, preserving the
        // explicit loading error and the previously loaded conversation.
        store.clearProblems()
        try beginHeldLoad(ids[2])
        var renamedAfterFailure = false
        store.renameConversation(conversation(ids[2]), title: "C_RENAMED") { renamedAfterFailure = true }
        try releaseHeldLoad()
        precondition(renamedAfterFailure && store.loadedConversationID == ids[0] && !store.problems.isEmpty)
        precondition(conversation(ids[2]).title == "C_RENAMED")
        // A failed native deletion of the pending destination keeps its loaded
        // source and gives an explicit failure, rather than an optimistic removal.
        store.clearProblems()
        let hardlink = root.appendingPathComponent("deletion-hardlink")
        try FileManager.default.linkItem(at: nativePath(ids[0]), to: hardlink)
        try beginHeldLoad(ids[0])
        store.deleteConversation(conversation(ids[0]))
        try releaseHeldLoad()
        precondition(store.loadedConversationID == ids[0] && store.selectedConversationID == ids[0] && !store.problems.isEmpty)
        precondition(FileManager.default.fileExists(atPath: nativePath(ids[0]).path))
        try FileManager.default.removeItem(at: hardlink)
        // A successful deletion of the pending destination completes after its
        // open; the delayed open can never reinsert it or become the visible view.
        try beginHeldLoad(ids[3])
        store.deleteConversation(conversation(ids[3]))
        precondition(store.loadedConversationID == ids[0] && store.pendingConversationID == ids[3])
        try releaseHeldLoad()
        precondition(store.loadedConversationID == nil && store.selectedConversationID == nil)
        precondition(!store.conversations.contains { $0.id == ids[3] })
        precondition(!FileManager.default.fileExists(atPath: nativePath(ids[3]).path))
        store.manualPoll(); RunLoop.main.run(until: Date().addingTimeInterval(0.1))
        precondition(!store.conversations.contains { $0.id == ids[3] })
        // If opening the destination fails, deleting the old loaded source still
        // clears that exact source without pretending the destination was loaded.
        store.selectConversation(id: ids[0]); wait { !store.isLoading && store.loadedConversationID == ids[0] }
        store.clearProblems()
        try beginHeldLoad(ids[2])
        store.deleteConversation(conversation(ids[0]))
        try releaseHeldLoad()
        precondition(store.loadedConversationID == nil && !FileManager.default.fileExists(atPath: nativePath(ids[0]).path))
        precondition(!store.problems.isEmpty && conversation(ids[2]).title == "C_RENAMED")
        // A native Set selection can span runtimes without opening its members.
        let otherB = "other:" + ids[1].dropFirst("fixture:".count).replacingOccurrences(of: "/runtime/", with: "/runtime-other/")
        store.selectConversation(id: otherID); wait { !store.isLoading && store.loadedConversationID == otherID }
        var hints: [BindingRuntimeDiscoveryHint] = []
        var discoveredHints = false
        store.runtimeDiscoveryHints(userHome: root.appendingPathComponent("discovery-home").path, overrides: [:]) { values in hints = values; discoveredHints = true }
        wait { !store.isLoading && discoveredHints }
        let piHint = hints.first { $0.familyId == "pi" }!
        precondition(piHint.directoryExists && piHint.agentDirectory == root.appendingPathComponent("discovery-home/.pi/agent").path)
        var discovered: [BindingRuntimeDiscoveryCandidate] = []
        var discoveredVersions = false
        let cli = pi
        store.discoverRuntimes([
            BindingRuntimeDiscoveryProbe(familyId: "pi", binary: cli, nodeBinary: node, agentDirectory: piHint.agentDirectory),
            BindingRuntimeDiscoveryProbe(familyId: "pi", binary: root.appendingPathComponent("unsupported/dist/bundle/cli.js").path, nodeBinary: node, agentDirectory: piHint.agentDirectory)
        ]) { values in discovered = values; discoveredVersions = true }
        wait { !store.isLoading && discoveredVersions }
        precondition(discovered.count == 2 && discovered.filter(\.supported).count == 1)
        precondition(discovered.first { !$0.supported }?.version == "9.9.9")
        var importedRuntime = false
        store.importRuntimes(discovered.filter(\.supported)) { ids in precondition(ids.count == 1); importedRuntime = true }
        wait { !store.isLoading && importedRuntime }
        precondition(store.loadedConversationID == otherID && store.projectionRuntimeID == "other")
        precondition(store.runtimeInstances.contains { $0.settings["agentDir"] == piHint.agentDirectory && $0.gatewayID == store.gateway.id })
        store.selectConversations(ids: [otherID, otherB])
        precondition(store.loadedConversationID == otherID && store.projectionRuntimeID == "other")
        store.manualPoll(); RunLoop.main.run(until: Date().addingTimeInterval(0.3))
        precondition(store.selectedConversationIDs == [otherID, otherB])
        let otherPath = nativePath(otherB), failedLink = root.appendingPathComponent("bulk-failed-link")
        try FileManager.default.linkItem(at: otherPath, to: failedLink)
        store.clearProblems()
        store.deleteConversations([conversation(otherID), conversation(otherB)])
        wait { !store.isLoading && store.conversationManagementStatus == nil }
        precondition(store.loadedConversationID == nil && !store.conversations.contains { $0.id == otherID })
        precondition(store.conversations.contains { $0.id == otherB } && store.selectedConversationIDs == [otherB])
        precondition(store.problems.contains { $0.source.hasPrefix("删除会话") })
        precondition(FileManager.default.fileExists(atPath: otherPath.path))
        try FileManager.default.removeItem(at: failedLink)
        store.deleteConversations([conversation(otherB), conversation(ids[2])])
        wait { !store.isLoading && store.conversationManagementStatus == nil }
        precondition(!store.conversations.contains { $0.id == otherB || $0.id == ids[2] })
        var templatesAdded = false
        store.saveTemplates([ModelTemplate(name: "Synthetic One", suggestedProviderModelID: "one"), ModelTemplate(name: "Synthetic Two", suggestedProviderModelID: "two")]) { count in precondition(count == 2); templatesAdded = true }
        wait { !store.isLoading && templatesAdded }
        precondition(store.modelTemplates.filter { $0.name.hasPrefix("Synthetic ") }.count == 2)
        // The next-turn draft is mutable while an actual source turn runs.
        // It must never redirect polling or cancellation to that draft target.
        store.selectNextTurnRuntime("fixture"); store.selectModel(modelRecordKey: nextModel)
        var createdTurn = false
        store.createConversation(runtimeID: "fixture", cwd: root.appendingPathComponent("project").path, modelRecordKey: nextModel) { createdTurn = true }
        wait { !store.isLoading && createdTurn }
        let actualTurnID = store.loadedConversationID!
        func holdTurn(_ number: Int) throws { try "hold".write(to: root.appendingPathComponent("turn-gate-\(number)"), atomically: true, encoding: .utf8) }
        try holdTurn(1)
        store.send(text: "Reply directly without tools.")
        wait { store.isGenerating && FileManager.default.fileExists(atPath: root.appendingPathComponent("turn-ready-1").path) }
        store.selectNextTurnRuntime("other"); store.selectModel(modelRecordKey: nextModel)
        precondition(store.projectionRuntimeID == "fixture" && store.loadedConversationID == actualTurnID && store.nextTurnRuntimeID == "other" && store.canCancel)
        store.manualRefreshResources()
        wait { !store.isLoading }
        store.manualPoll()
        RunLoop.main.run(until: Date().addingTimeInterval(0.2))
        precondition(store.isGenerating && store.projectionRuntimeID == "fixture" && store.nextTurnRuntimeID == "other", "configuration refresh redirected the running projection")
        try FileManager.default.removeItem(at: root.appendingPathComponent("turn-gate-1"))
        wait { !store.isGenerating && !store.isLoading }
        precondition(store.projectionRuntimeID == "fixture" && store.loadedConversationID == actualTurnID && store.nextTurnRuntimeID == "other")
        precondition(store.manualSnapshotModelKey() == nextModel)
        precondition(store.manualSnapshot()?.messages.contains { message in message.blocks.contains { block in if case .text(let text) = block { return text.contains("SYNTHETIC_RUNNING_REPLY") }; return false } } == true)

        store.selectNextTurnRuntime("fixture")
        try holdTurn(2)
        store.send(text: "Reply directly without tools again.")
        wait { store.isGenerating && FileManager.default.fileExists(atPath: root.appendingPathComponent("turn-ready-2").path) }
        store.selectNextTurnRuntime("other"); store.selectModel(modelRecordKey: nextModel)
        store.cancel()
        wait { !store.isGenerating && !store.isLoading }
        try FileManager.default.removeItem(at: root.appendingPathComponent("turn-gate-2"))
        precondition(store.projectionRuntimeID == "fixture" && store.loadedConversationID == actualTurnID && store.nextTurnRuntimeID == "other")
        var editedProvider = store.gateway.providers.first!
        editedProvider.endpoint = endpoint + "/changed"
        var providerSaved = false
        store.saveProvider(editedProvider, authenticationEdit: .keep) { providerSaved = true }
        wait { providerSaved && !store.isLoading }
        precondition(store.loadedConversationID == actualTurnID && store.projectionRuntimeID == "fixture" && store.nextTurnRuntimeID == "other" && store.nextTurnModelRecordKey == nextModel, "provider invalidation discarded history or changed the next-turn intent")
        store.manualPoll()
        RunLoop.main.run(until: Date().addingTimeInterval(0.2))
        precondition(store.loadedConversationID == actualTurnID && store.manualSnapshot()?.messages.contains { $0.role == .assistant } == true)
        func nativeSessions(_ directory: URL) -> Set<URL> {
            let enumerator = FileManager.default.enumerator(at: directory.appendingPathComponent("sessions"), includingPropertiesForKeys: nil)
            return Set((enumerator?.allObjects as? [URL] ?? []).filter { $0.pathExtension == "jsonl" })
        }
        let originalNativeFile = nativePath(actualTurnID)
        let originalNativeBytes = try Data(contentsOf: originalNativeFile)
        let otherNativeBefore = nativeSessions(root.appendingPathComponent("runtime-other"))
        let sourceMessages = store.manualSnapshot()!.messages
        try holdTurn(3)
        store.send(text: "Continue this same conversation in the selected runtime without tools.")
        wait { store.isGenerating && FileManager.default.fileExists(atPath: root.appendingPathComponent("turn-ready-3").path) }
        store.selectNextTurnRuntime("fixture"); store.selectModel(modelRecordKey: nextModel)
        store.manualPoll()
        RunLoop.main.run(until: Date().addingTimeInterval(0.2))
        precondition(store.isGenerating && store.loadedConversationID == actualTurnID && store.projectionRuntimeID == "other")
        precondition(store.manualSnapshot()?.conversation.runtimeID == "fixture", "continuation changed the origin grouping")
        try FileManager.default.removeItem(at: root.appendingPathComponent("turn-gate-3"))
        wait { !store.isGenerating && !store.isLoading }
        let continuedMessages = store.manualSnapshot()!.messages
        precondition(store.loadedConversationID == actualTurnID && store.nextTurnRuntimeID == "fixture")
        precondition(continuedMessages.filter { $0.role == .assistant }.count == sourceMessages.filter { $0.role == .assistant }.count + 1, "handoff duplicated history or lost the new reply")
        precondition(continuedMessages.last { $0.role == .user }?.text == "Continue this same conversation in the selected runtime without tools.", "handoff implementation text leaked into the user bubble")
        precondition(continuedMessages.contains { $0.role == .system }, "runtime switch has no readable boundary")
        let originalAfterHandoff = try Data(contentsOf: originalNativeFile)
        precondition(originalAfterHandoff == originalNativeBytes, "handoff rewrote the origin native history")
        let targetNativeFiles = nativeSessions(root.appendingPathComponent("runtime-other")).subtracting(otherNativeBefore)
        precondition(targetNativeFiles.count == 1, "continuation did not create one target native session")
        store.manualRefreshResources(); wait { !store.isLoading }
        precondition(store.conversations.filter { $0.id == actualTurnID }.count == 1)
        precondition(!store.conversations.contains { targetNativeFiles.contains(URL(fileURLWithPath: String($0.id.dropFirst("other:".count)))) }, "target segment appeared as another independent row")
        var savedAfterContinuation = false
        store.saveTemplates([ModelTemplate(name: "After Continuation", suggestedProviderModelID: "synthetic")]) { count in precondition(count == 1); savedAfterContinuation = true }
        wait { savedAfterContinuation && !store.isLoading }
        observer.cancel()
        var closed = false
        store.shutdown { closed = $0 }; wait { closed }
        let verifier = try VeluneApplication.open(options: BindingOptions(homeDirectory: root.appendingPathComponent("application").path, resourcesDirectory: resources.path))
        let persisted = try verifier.list()
        precondition(persisted.conversationBrowserGroupLimit == 7)
        try verifier.shutdown()
        let reopened = AppStore(transport: Transport(stateDirectory: root.appendingPathComponent("application"), resourcesDirectory: resources))
        reopened.start(); wait { !reopened.isLoading && !reopened.conversations.isEmpty }
        if reopened.loadedConversationID != actualTurnID { reopened.selectConversation(id: actualTurnID) }
        wait { !reopened.isLoading && reopened.loadedConversationID == actualTurnID }
        precondition(reopened.projectionRuntimeID == "other" && reopened.manualSnapshot()?.conversation.runtimeID == "fixture")
        precondition(reopened.manualSnapshot()!.messages.map(\.id) == continuedMessages.map(\.id), "restart changed logical message identity")
        precondition(reopened.manualSnapshot()!.messages == continuedMessages, "restart failed to rebuild native segment projection")
        let linked = reopened.conversations.first { $0.id == actualTurnID }!
        var renamedLinked = false
        reopened.renameConversation(linked, title: "LINKED_NATIVE_TITLE") { renamedLinked = true }
        wait { renamedLinked && !reopened.isLoading }
        let nativeEntries = try String(contentsOf: originalNativeFile, encoding: .utf8).split(separator: "\n").map { try JSONSerialization.jsonObject(with: Data($0.utf8)) as! [String: Any] }
        precondition(nativeEntries.last { $0["type"] as? String == "session_info" }?["name"] as? String == "LINKED_NATIVE_TITLE", "rename did not change the first native segment")
        let renamedLinkedConversation = reopened.conversations.first { $0.id == actualTurnID }!
        let protectedTarget = targetNativeFiles.first!
        let protectedTargetLink = root.appendingPathComponent("linked-target-hardlink")
        try FileManager.default.linkItem(at: protectedTarget, to: protectedTargetLink)
        let nextBeforeFailedDelete = reopened.nextTurnRuntimeID
        reopened.clearProblems()
        reopened.deleteConversation(renamedLinkedConversation)
        wait { !reopened.isLoading && reopened.conversationManagementStatus == nil }
        precondition(!FileManager.default.fileExists(atPath: originalNativeFile.path) && FileManager.default.fileExists(atPath: protectedTarget.path), "partial native delete did not preserve the failed target")
        precondition(reopened.loadedConversationID == nil && !reopened.canSend && reopened.nextTurnRuntimeID == nextBeforeFailedDelete, "partial delete left the obsolete projection sendable")
        precondition(!reopened.problems.isEmpty)
        try FileManager.default.removeItem(at: protectedTargetLink)
        precondition(reopened.conversations.contains { $0.id == actualTurnID && $0.canDelete }, "incomplete logical conversation disappeared or cannot be retried")
        reopened.deleteConversation(reopened.conversations.first { $0.id == actualTurnID }!)
        wait { !reopened.isLoading && reopened.conversationManagementStatus == nil }
        precondition(reopened.loadedConversationID == nil && !reopened.conversations.contains { $0.id == actualTurnID })
        precondition(!FileManager.default.fileExists(atPath: originalNativeFile.path) && targetNativeFiles.allSatisfy { !FileManager.default.fileExists(atPath: $0.path) }, "logical delete left native segment files")
        var closedReopened = false
        reopened.shutdown { closedReopened = $0 }; wait { closedReopened }

        print("{\"actualAppStoreAndTransport\":true,\"crossRuntimeUnifiedBrowsing\":true,\"fullPathGroupsAndNativeDateSorting\":true,\"slowLoadSelectionStable\":true,\"latePollIgnored\":true,\"concurrentSelectionSerialized\":true,\"failedLoadRestoredOnce\":true,\"loadedRowsPreservedDuringLoadAndFailure\":true,\"queuedNativeRenameAndDelete\":true,\"deletedDestinationCannotReturn\":true,\"failedMutationPreservesLoadedSource\":true,\"nativeMultiSelectionAndPartialBulkDelete\":true,\"perGroupPaginationAndPersistedLimit\":true,\"batchTemplateSave\":true,\"typedQuickImportPreservesLoadedSource\":true,\"nextTurnIntentIndependentOfHistoryAndPendingOpen\":true,\"runningTurnFinishAndCancelKeepActualOwner\":true,\"crossInstanceNativeContinuationAndRestart\":true,\"linkedNativeRenameAndDelete\":true,\"upstreamRequests\":3}")
    }
}
'''


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    for name in ('bundle', 'node', 'pi'):
        parser.add_argument('--' + name, type=Path, required=True)
    parser.add_argument('--swift-build', type=Path)
    args = parser.parse_args()
    if not args.pi.is_absolute() or not args.pi.is_file(): parser.error('--pi must be an absolute external CLI path')
    if args.pi.resolve().is_relative_to(args.bundle.resolve()): parser.error('--pi must not come from the app bundle')
    sdk = None
    for directory in args.pi.resolve().parents:
        package = directory / 'package.json'
        if not package.is_file(): continue
        metadata = json.loads(package.read_text())
        if metadata.get('name') != '@earendil-works/pi-coding-agent': continue
        if metadata.get('version') != '1.0.2': parser.error('this manual fixture requires external Pi 1.0.2')
        sdk = directory / metadata['exports']['.']['import']
        break
    if sdk is None or not sdk.is_file(): parser.error('--pi must belong to the Pi 1.0.2 package with its public SDK export')
    root = Path(__file__).resolve().parent.parent
    build = args.swift_build or Path(subprocess.check_output(['swift','build','--show-bin-path'],cwd=root,text=True).strip())
    with tempfile.TemporaryDirectory(prefix='velune-session-loading-') as directory:
        temporary = Path(directory)
        resources = temporary / 'resources'; resources.mkdir()
        (resources / 'node_modules').symlink_to(args.bundle / 'Contents/Resources/node_modules')
        for resource in (args.bundle / 'Contents/Resources').glob('*.mjs'): shutil.copy(resource, resources)
        (temporary / 'home').mkdir(); (temporary / 'project').mkdir()
        (temporary / 'discovery-home/.pi/agent').mkdir(parents=True)
        unsupported = temporary / 'unsupported'
        (unsupported / 'dist/bundle').mkdir(parents=True)
        (unsupported / 'package.json').write_text(json.dumps({'name':'@earendil-works/pi-coding-agent','version':'9.9.9','type':'module','bin':{'pi':'dist/bundle/cli.js'}}))
        unsupported_cli = unsupported / 'dist/bundle/cli.js'
        unsupported_cli.write_text('#!/usr/bin/env node\nif (process.argv.slice(2).join(" ") === "--version") console.log("pi 9.9.9"); else process.exit(2);\n')
        unsupported_cli.chmod(0o755)
        env = {'HOME':str(temporary / 'home'),'PATH':str(args.node.parent) + ':/usr/bin:/bin'}
        requests = []
        class Upstream(BaseHTTPRequestHandler):
            def log_message(self, *_): pass
            def do_POST(self):
                body = json.loads(self.rfile.read(int(self.headers['Content-Length'])))
                requests.append(body)
                number = len(requests)
                self.send_response(200); self.send_header('Content-Type', 'text/event-stream'); self.end_headers()
                initial = {'id':'synthetic','object':'chat.completion.chunk','created':1,'model':body['model'],'choices':[{'index':0,'delta':{'role':'assistant'},'finish_reason':None}]}
                try:
                    self.wfile.write(('data: '+json.dumps(initial)+'\n\n').encode()); self.wfile.flush()
                    (temporary / f'turn-ready-{number}').write_text('ready')
                    deadline = time.monotonic() + 20
                    while (temporary / f'turn-gate-{number}').exists() and time.monotonic() < deadline: time.sleep(.02)
                    chunks = [
                        {'id':'synthetic','object':'chat.completion.chunk','created':1,'model':body['model'],'choices':[{'index':0,'delta':{'content':'SYNTHETIC_RUNNING_REPLY'},'finish_reason':None}]},
                        {'id':'synthetic','object':'chat.completion.chunk','created':1,'model':body['model'],'choices':[{'index':0,'delta':{},'finish_reason':'stop'}],'usage':{'prompt_tokens':1,'completion_tokens':2,'total_tokens':3}}
                    ]
                    for chunk in chunks: self.wfile.write(('data: '+json.dumps(chunk)+'\n\n').encode())
                    self.wfile.write(b'data: [DONE]\n\n'); self.wfile.flush()
                except (BrokenPipeError, ConnectionResetError): pass # Native cancellation closes the stream.
        server = ThreadingHTTPServer(('127.0.0.1',0),Upstream)
        threading.Thread(target=server.serve_forever,daemon=True).start()
        endpoint = f'http://127.0.0.1:{server.server_port}/v1'

        seed = resources / 'seed.mjs'
        seed.write_text('import {SessionManager} from ' + json.dumps(sdk.resolve().as_uri()) + ';\n' + """
const [cwd,dir,title]=process.argv.slice(2); const manager=SessionManager.create(cwd,dir);
manager.appendMessage({role:'user',content:[{type:'text',text:title}],timestamp:Date.now()});
manager.appendMessage({role:'assistant',content:[{type:'text',text:'SYNTHETIC '+title}],api:'openai-completions',provider:'synthetic',model:'synthetic',stopReason:'stop',timestamp:Date.now()});
manager.appendSessionInfo(title); console.log(manager.getSessionFile());
""")
        sessions = temporary / 'runtime/sessions/project'; sessions.mkdir(parents=True)
        paths = {}
        for name in ['D','C','B','A']:
            paths[name] = subprocess.check_output([str(args.node),str(seed),str(temporary / 'project'),str(sessions),name],env=env,text=True).strip()
        shutil.copytree(temporary / 'runtime', temporary / 'runtime-other')
        helper = resources / 'pi_sessions.mjs'
        original_helper = args.bundle / 'Contents/Resources/pi_sessions.mjs'
        helper.write_text('import {existsSync,writeFileSync} from "node:fs";\n'
            + 'const args=process.argv.slice(2), index=args.indexOf("--inspect-session"), path=index < 0 ? null : args[index+1];\n'
            + 'if(path !== null && existsSync(' + json.dumps(str(temporary / 'gate')) + ')) { writeFileSync(' + json.dumps(str(temporary / 'ready')) + ',"ready"); while(existsSync(' + json.dumps(str(temporary / 'gate')) + ')) await new Promise(resolve=>setTimeout(resolve,15)); }\n'
            + 'if(path === ' + json.dumps(paths['C']) + ') process.exit(1);\n'
            + 'await import(' + json.dumps(original_helper.as_uri()) + ');\n')
        main = temporary / 'Manual.swift'; main.write_text(SWIFT)
        store = temporary / 'Store.swift'
        store.write_text((root / 'app/mac/Store.swift').read_text() + '\nextension AppStore { func manualPoll() { poll() }; func manualSnapshotModelKey() -> String? { snapshot?.modelRecordKey }; func manualSnapshot() -> ConversationSnapshot? { snapshot }; func manualRefreshResources() { guard let transport else { return }; enqueue({ try transport.list() }) { [weak self] in self?.applyList($0) } } }\n')
        import_models = temporary / 'ImportModels.swift'
        import_models.write_text((root / 'app/mac/ProviderImport.swift').read_text().split('struct ProviderImportView: View {')[0])
        objects = []
        for name in ['VeluneBindings','MarkdownView','Markdown','Highlightr','RichText','Introspection','SwiftMath','CAtomic','cmark_gfm','cmark_gfm_extensions']:
            objects.extend(str(path) for path in (build / (name + '.build')).rglob('*.o'))
        command = ['xcrun','swiftc','-parse-as-library','-swift-version','5','-warnings-as-errors','-I',str(build / 'Modules'),'-I',str(root / 'target/swift-ffi')]
        command += ['-Xcc', '-I' + str(root/'.build/checkouts/swift-cmark/src/include'), '-Xcc', '-fmodule-map-file=' + str(root/'.build/checkouts/swift-markdown/Sources/CAtomic/include/module.modulemap')]
        for path in [root/'.build/checkouts/swift-cmark/src/include/module.modulemap',root/'.build/checkouts/swift-cmark/extensions/include/module.modulemap']:
            command += ['-Xcc','-fmodule-map-file=' + str(path)]
        command += [str(root / 'app/mac' / name) for name in ['Models.swift','TranscriptModel.swift','Transport.swift','BindingMapping.swift','ConversationBrowser.swift','Problems.swift']]
        command += [str(import_models),str(store),str(main),*objects,'-L',str(args.bundle / 'Contents/Frameworks'),'-lvelune_bindings','-Xlinker','-rpath','-Xlinker',str(args.bundle / 'Contents/Frameworks'),'-o',str(temporary / 'manual')]
        subprocess.run(command,check=True,cwd=root)
        for name in ('Highlightr_Highlightr.bundle', 'SwiftMath_SwiftMath.bundle'):
            shutil.copytree(build/name, temporary/name)
        identities = ['fixture:' + paths[name] for name in ['A','B','C','D']]
        subprocess.run([str(temporary / 'manual'),str(temporary),str(resources),*identities,str(args.node),endpoint,str(args.pi)],check=True,cwd=temporary,env=env)
        server.shutdown(); server.server_close()
        assert [request['model'] for request in requests] == ['synthetic'] * 3, 'unexpected loopback dispatch'
        def strings(value):
            if isinstance(value, str): yield value
            elif isinstance(value, list):
                for item in value: yield from strings(item)
            elif isinstance(value, dict):
                for item in value.values(): yield from strings(item)
        envelopes = [value for value in strings(requests[2]) if value.startswith('<velune-context:')]
        assert len(envelopes) == 1, 'target received no unique context envelope'
        opening, body = envelopes[0].split('\n', 1)
        assert opening.endswith('>')
        marker = opening[len('<velune-context:'):-1]
        closing = '\n</velune-context:' + marker + '>'
        assert marker and body.endswith(closing), 'context envelope boundary mismatch'
        handoff = json.loads(body[:-len(closing)])
        assert handoff['request'] == 'Continue this same conversation in the selected runtime without tools.'
        assert 'SYNTHETIC_RUNNING_REPLY' in json.dumps(handoff['history']), 'target did not receive the source conversation context'
        assert 'data' in handoff['contextMeaning'], 'context was not explicitly quoted as data'

if __name__ == '__main__': main()
