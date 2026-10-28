import Foundation
import VeluneBindings

extension Acceptance {
    static func stage(_ value: String) { FileHandle.standardError.write(Data(("distributed: " + value + "\n").utf8)) }
    static func until(_ condition: () throws -> Bool, _ message: String) async throws {
        let deadline = Date().addingTimeInterval(20)
        while Date() < deadline {
            if try condition() { return }
            try await Task.sleep(nanoseconds: 40_000_000)
        }
        throw NSError(domain: "DistributedAcceptance", code: 1, userInfo: [NSLocalizedDescriptionKey: message])
    }
    static func send(_ telemetry: ClientTelemetry, _ app: VeluneApplication, _ runtime: String, _ conversation: String, text: String = "GATE CANARY-private-body") throws -> String {
        var parent = ""
        _ = try telemetry.operation(.send) { carrier in
            parent = carrier?.traceparent ?? ""
            return try app.withTraceContext(carrier: carrier).sendTurnWithSelection(sourceRuntimeId: runtime, conversationId: conversation, targetRuntimeId: runtime,
                selection: nil, turn: BindingComposedTurn(text: text, effort: nil, attachments: []))
        }
        return parent
    }
    static func distributed(_ root: URL, _ resources: String, _ first: BindingTelemetryProfile, _ second: BindingTelemetryProfile, _ peerScript: String, _ python: String) async throws -> [String: Any] {
        stage("open")
        let files = FileManager.default
        let hostHome = root.appendingPathComponent("distributed-host")
        let controllerHome = root.appendingPathComponent("distributed-controller")
        let hostOwner = try ClientTelemetry(home: hostHome, platform: "macos", localLogs: false)
        let controlOwner = try ClientTelemetry(home: controllerHome, platform: "macos", localLogs: false)
        try hostOwner.start(); try controlOwner.start()
        let local = try VeluneApplication.openWithTelemetry(options: BindingOptions(homeDirectory: hostHome.path, resourcesDirectory: resources), telemetry: hostOwner.binding)
        let execution = BindingRuntimeExecution(transport: .local, home: .managed(remoteRoot: nil), remoteResourcesDirectory: nil)
        for name in ["one", "two"] {
            let state = root.appendingPathComponent("fixture-" + name)
            try files.createDirectory(at: state, withIntermediateDirectories: true)
            _ = try local.upsertAcpRuntime(runtime: BindingRuntimeInstance(enabled: true, id: name, name: name, typeId: "acp-v1", gatewayId: "default", settings: ["binary": python]), execution: execution,
                launch: BindingAcpLaunch(args: [peerScript, "literal ; $(not-a-shell)"], environment: ["ACP_MODE": "full", "ACP_STATE": state.path]))
        }
        let one = try local.createConversationWithSelection(runtimeId: "one", cwd: root.path, selection: nil).snapshot!.conversation.id
        let two = try local.createConversationWithSelection(runtimeId: "two", cwd: root.path, selection: nil).snapshot!.conversation.id
        func endpoint(_ name: String, _ telemetry: ClientTelemetry, _ application: VeluneApplication?) throws -> VeluneRemoteEndpoint {
            let directory = root.appendingPathComponent(name)
            try files.createDirectory(at: directory, withIntermediateDirectories: true)
            return try VeluneRemoteEndpoint.openWithTelemetry(options: BindingRemoteOptions(identityFile: directory.appendingPathComponent("identity").path,
                stateFile: directory.appendingPathComponent("state.json").path, deviceName: name, relay: .disabled, discovery: .none, pathPolicy: .automatic,
                bindAddresses: ["127.0.0.1:0"], requestTimeoutMs: name == "controller-endpoint" ? 3000 : 30000), application: application, telemetry: telemetry.binding)
        }
        let host = try endpoint("host-endpoint", hostOwner, local)
        let control = try endpoint("controller-endpoint", controlOwner, nil)
        let invitation = try host.createInvitation(lifetimeSeconds: 120)
        let pending = try control.beginPairing(encoded: invitation.encoded)
        try await until({ try host.readState().incomingPairings.contains { $0.attemptId == pending.attemptId && $0.phase == .awaitingApproval } }, "pairing not ready")
        let incoming = try host.readState().incomingPairings.first { $0.attemptId == pending.attemptId }!
        try require(incoming.verificationCode == pending.verificationCode, "verification code differs")
        _ = try host.approvePairing(attemptId: pending.attemptId, expectedPeerId: incoming.peerId)
        let hostId = try host.descriptor().endpointId
        try await until({ try control.pairingStatus(peerId: hostId).phase == .granted }, "controller did not observe consent")
        let remote = try control.connect(peerId: hostId, viewId: "synthetic-controller")
        stage("paired")
        try hostOwner.save(revision: 0, preference: .on, profile: first)
        try controlOwner.save(revision: 0, preference: .on, profile: first)
        _ = try remote.list()
        var preparationRejectedParent = ""
        var preparationRejected = false
        do {
            _ = try controlOwner.operation(.send) { carrier in
                preparationRejectedParent = carrier?.traceparent ?? ""
                return try remote.withTraceContext(carrier: carrier).sendTurnWithSelection(sourceRuntimeId: "one", conversationId: one, targetRuntimeId: "one", selection: nil,
                    turn: BindingComposedTurn(text: "CANARY-private-body", effort: "CANARY-unsupported-effort", attachments: []))
            }
        } catch is BindingError { preparationRejected = true }
        try require(preparationRejected && hostOwner.binding.snapshot().execution?.activeTurns == 0, "preparation failure started a business turn")
        _ = hostOwner.binding.flush()
        _ = try local.upsertAcpRuntime(runtime: BindingRuntimeInstance(enabled: true, id: "unavailable", name: "unavailable", typeId: "acp-v1", gatewayId: "default", settings: ["binary": python]), execution: execution,
            launch: BindingAcpLaunch(args: [root.appendingPathComponent("CANARY-missing-peer.py").path], environment: [:]))
        var preparationFailedParent = ""
        var preparationFailed = false
        do {
            _ = try controlOwner.operation(.send) { carrier in
                preparationFailedParent = carrier?.traceparent ?? ""
                return try remote.withTraceContext(carrier: carrier).createConversationWithSelection(runtimeId: "unavailable", cwd: root.path, selection: nil)
            }
        } catch is BindingError { preparationFailed = true }
        try require(preparationFailed && hostOwner.binding.snapshot().execution?.activeTurns == 0, "failed harness preparation started work")
        _ = hostOwner.binding.flush()
        _ = try local.deleteRuntime(id: "unavailable")
        let oneGate = root.appendingPathComponent("fixture-one/hold")
        let twoGate = root.appendingPathComponent("fixture-two/hold")
        try Data().write(to: oneGate); try Data().write(to: twoGate)
        async let sentOne = Task.detached { try send(controlOwner, remote, "one", one) }.value
        async let sentTwo = Task.detached { try send(controlOwner, remote, "two", two) }.value
        let parents = try await [sentOne, sentTwo]
        stage("first accepted")
        try require(parents[0] != parents[1], "parallel parents were shared")
        try await until({ try hostOwner.binding.snapshot().execution?.activeTurns == 2 }, "parked + selected turns were not counted")
        _ = hostOwner.binding.flush()
        try control.disconnect(peerId: hostId)
        try await Task.sleep(nanoseconds: 300_000_000)
        try require(try hostOwner.binding.snapshot().execution?.activeTurns == 2, "control disconnect stopped work")
        let disconnectedAt = UInt64(Date().timeIntervalSince1970 * 1000)
        _ = hostOwner.binding.flush()
        try files.removeItem(at: oneGate); try files.removeItem(at: twoGate)
        try await until({ try hostOwner.binding.snapshot().execution?.activeTurns == 0 }, "disconnected turns did not finish")
        _ = hostOwner.binding.flush()
        stage("first finished")
        try remote.reconnect()
        _ = try remote.list()

        // Reuse the same application, service and proxy while both business turns and SDK generations change.
        try Data().write(to: oneGate); try Data().write(to: twoGate)
        let switchedOne = try send(controlOwner, remote, "one", one)
        let switchedTwo = try send(controlOwner, remote, "two", two)
        try await until({ try hostOwner.binding.snapshot().execution?.activeTurns == 2 }, "second wave not running")
        _ = hostOwner.binding.flush()
        try hostOwner.save(revision: try hostOwner.binding.snapshot().revision, preference: .on, profile: second)
        stage("switched")
        try require(try hostOwner.binding.snapshot().execution?.activeTurns == 2, "SDK switch reset business count")
        _ = hostOwner.binding.flush()
        try files.removeItem(at: oneGate)
        try await until({ try hostOwner.binding.snapshot().execution?.activeTurns == 1 }, "successful turn did not decrement count")
        _ = hostOwner.binding.flush()
        _ = try controlOwner.operation(.cancel) { carrier in try remote.withTraceContext(carrier: carrier).cancel(runtimeId: "two", conversationId: two) }
        try await until({ try hostOwner.binding.snapshot().execution?.activeTurns == 0 }, "cancelled turn did not finish")
        _ = hostOwner.binding.flush()
        try files.removeItem(at: twoGate)
        stage("second finished")

        let brokenCancel = try send(controlOwner, remote, "one", one, text: "BREAK_CANCEL CANARY-private-body")
        try await until({ files.fileExists(atPath: root.appendingPathComponent("fixture-one/cancel-ready").path) }, "fixture did not close its command pipe")
        var cancelFailed = false
        do { _ = try controlOwner.operation(.cancel) { carrier in try remote.withTraceContext(carrier: carrier).cancel(runtimeId: "one", conversationId: one) } }
        catch is BindingError { cancelFailed = true }
        try require(cancelFailed, "broken cancel unexpectedly succeeded")
        try await until({ try hostOwner.binding.snapshot().execution?.activeTurns == 0 }, "failed cancellation left active work")
        _ = hostOwner.binding.flush()
        stage("broken cancel finished")

        // The accepted slow command outlives the caller's timeout; query the original ID after reconnect.
        let slowGate = root.appendingPathComponent("slow-gate")
        let slowReady = root.appendingPathComponent("slow-ready")
        try Data().write(to: slowGate)
        let slowState = root.appendingPathComponent("fixture-slow")
        try files.createDirectory(at: slowState, withIntermediateDirectories: true)
        _ = try local.upsertAcpRuntime(runtime: BindingRuntimeInstance(enabled: true, id: "slow", name: "slow", typeId: "acp-v1", gatewayId: "default", settings: ["binary": python]), execution: execution,
            launch: BindingAcpLaunch(args: [root.appendingPathComponent("slow-peer.py").path, "literal ; $(not-a-shell)"], environment: ["ACP_MODE": "full", "ACP_STATE": slowState.path, "SLOW_GATE": slowGate.path, "SLOW_READY": slowReady.path]))
        stage("slow configured")
        let uncertain = Task.detached { () throws -> BindingRemoteRequestId in
            do {
                _ = try controlOwner.operation(.send) { carrier in try remote.withTraceContext(carrier: carrier).createConversationWithSelection(runtimeId: "slow", cwd: root.path, selection: nil) }
                throw Synthetic.expected
            } catch BindingError.RemoteRequest(let kind, _, _, let id) {
                try require(kind == .outcomeUnknown && id != nil, "timeout lost unknown result identity")
                return id!
            }
        }
        try await until({ files.fileExists(atPath: slowReady.path) }, "slow command did not enter actual ACP initialization")
        let request = try await uncertain.value
        stage("unknown received")
        try control.disconnect(peerId: hostId)
        try files.removeItem(at: slowGate)
        try remote.reconnect()
        try await until({ if case .completed = try remote.queryRequest(requestId: request) { return true }; return false }, "original ID could not recover the accepted result")
        try control.acknowledgeRequest(peerId: hostId, requestId: request)
        _ = hostOwner.binding.flush()
        // Ratios govern roots; preserve both sampled and unsampled decisions over native/FFI/QUIC.
        var rootZero = second; rootZero.sampleRatio = 0
        try hostOwner.save(revision: try hostOwner.binding.snapshot().revision, preference: .on, profile: rootZero)
        let sampledIntoZero = try send(controlOwner, remote, "two", two, text: "NORMAL CANARY-private-body")
        try await until({ try hostOwner.binding.snapshot().execution?.activeTurns == 0 }, "sampled-parent turn did not finish")
        _ = hostOwner.binding.flush()
        try await Task.sleep(nanoseconds: 11_000_000_000)
        var controlZero = first; controlZero.sampleRatio = 0
        try controlOwner.save(revision: try controlOwner.binding.snapshot().revision, preference: .on, profile: controlZero)
        try hostOwner.save(revision: try hostOwner.binding.snapshot().revision, preference: .on, profile: second)
        let unsampledIntoOne = try send(controlOwner, remote, "two", two, text: "NORMAL CANARY-private-body")
        try require(sampledIntoZero.hasSuffix("-01") && unsampledIntoOne.hasSuffix("-00"), "sampler erased a valid parent decision")
        try await until({ try hostOwner.binding.snapshot().execution?.activeTurns == 0 }, "unsampled-parent turn did not finish")
        _ = hostOwner.binding.flush()
        let hostInstance = try hostOwner.binding.snapshot().instanceId
        let controllerInstance = try controlOwner.binding.snapshot().instanceId
        try remote.shutdown(); try control.shutdown(); try host.shutdown(); try local.shutdown()
        try controlOwner.shutdown(); try hostOwner.shutdown()
        stage("closed")
        return ["parents": parents, "switchedParents": [switchedOne, switchedTwo], "preparationRejectedParent": preparationRejectedParent,
            "preparationFailedParent": preparationFailedParent,
            "sampledIntoZero": sampledIntoZero, "unsampledIntoOne": unsampledIntoOne,
            "disconnectedAtUnixMs": disconnectedAt, "brokenCancelParent": brokenCancel, "hostInstance": hostInstance, "controllerInstance": controllerInstance,
            "disconnectPreservedTwoTurns": true, "sameServiceReconfigured": true, "gaugeSequence": [2,1,0], "originalRequestRecovered": true]
    }
}
