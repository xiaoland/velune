import Foundation
import VeluneBindings

/// Enters before App/delegate/scene initialization. Core owns the one local
/// synthetic Codex process; no window, account or native user profile is opened.
@MainActor func conversationRequestChecks() -> Never {
    let home = MacExecutionContext.admitted.stateDirectory
    let peer = home.appendingPathComponent("synthetic-codex")
    let script = #"""
#!/usr/bin/python3
import json, os, pathlib, sys, threading, time
root = pathlib.Path(__file__).parent
if '--version' in sys.argv:
    print('codex-cli 0.166.0'); sys.exit(0)
lock = threading.Lock()
turn_count = 0
start_count = 0
current_turn = None
thread_id = 'synthetic-thread-' + str(os.getpid())
def emit(value):
    with lock:
        print(json.dumps(value), flush=True)
def notify(method, params): emit({'method':method,'params':params})
def ack(request, result): emit({'id':request['id'],'result':result})
def mark(name, text='1'):
    with (root / name).open('a') as f: f.write(text + '\n')
def release_send(request, turn):
    deadline = time.monotonic() + 30
    while not (root / 'release-send').exists():
        if time.monotonic() >= deadline:
            emit({'id':request['id'],'error':{'code':-32600,'message':'synthetic gate release timed out'}})
            return
        time.sleep(.01)
    ack(request, {'turn':{'id':turn}})
def release_user_echo(thread, turn, items):
    deadline = time.monotonic() + 12
    while not (root / 'release-user-echo').exists():
        if time.monotonic() >= deadline: return
        time.sleep(.01)
    notify('item/completed', {'threadId':thread,'turnId':turn,'item':{'id':'late-user','type':'userMessage','content':items}})
for line in sys.stdin:
    request = json.loads(line)
    with (root / 'wire.jsonl').open('a') as f: f.write(json.dumps(request) + '\n')
    method = request.get('method')
    if method == 'initialize': ack(request, {})
    elif method == 'config/read': ack(request, {'config':{'model_provider':'openai','model':'synthetic-native'}})
    elif method == 'thread/start':
        start_count += 1
        thread_id = 'synthetic-thread-' + str(os.getpid()) + '-' + str(start_count)
        ack(request, {'thread':{'id':thread_id,'name':'Synthetic','cwd':str(root)},'model':'synthetic-native'})
    elif method == 'thread/resume':
        thread_id = request['params']['threadId']
        ack(request, {'thread':{'id':thread_id,'modelProvider':'openai'}})
    elif method == 'thread/read':
        target = request['params']['threadId']
        thread = {'id':target,'modelProvider':'openai','historyMode':'legacy','turns':[]}
        if target == 'cold-child':
            thread.update(name='Synthetic child', parentThreadId='parent-thread')
        ack(request, {'thread':thread})
    elif method == 'thread/list': ack(request, {'data':[]})
    elif method == 'model/list': ack(request, {'data':[]})
    elif method == 'turn/start':
        turn_count += 1
        current_turn = 'turn-' + str(turn_count)
        mark('prompt-wire', str(os.getpid()))
        if any('created-failure' in item.get('text', '') for item in request['params']['input']):
            mark('rejected-send-wire')
            emit({'id':request['id'],'error':{'code':-32600,'message':'synthetic created Send rejected'}})
            continue
        if any(item.get('text') == 'hold for queue withdrawal' for item in request['params']['input']):
            notify('turn/started', {'threadId':thread_id,'turn':{'id':current_turn}})
            ack(request, {'turn':{'id':current_turn}})
            continue
        if any(item.get('text') == 'synthetic late user echo' for item in request['params']['input']):
            notify('turn/started', {'threadId':thread_id,'turn':{'id':current_turn}})
            ack(request, {'turn':{'id':current_turn}})
            threading.Thread(target=release_user_echo, args=(thread_id, current_turn, request['params']['input']), daemon=True).start()
            continue
        if turn_count == 1: ack(request, {'turn':{'id':current_turn}})
        notify('turn/started', {'threadId':thread_id,'turn':{'id':current_turn}})
        if request['params']['input']:
            notify('item/completed', {'threadId':thread_id,'turnId':current_turn,'item':{'id':'user-' + str(turn_count),'type':'userMessage','content':request['params']['input']}})
        notify('item/started', {'threadId':thread_id,'turnId':current_turn,'item':{'id':'assistant-' + str(turn_count),'type':'agentMessage','text':'output before send ACK'}})
        if turn_count == 1:
            notify('turn/completed', {'threadId':thread_id,'turn':{'id':current_turn,'status':'completed'}})
        else:
            for index in range(40):
                notify('item/completed', {'threadId':thread_id,'turnId':current_turn,'item':{'id':'reasoning-' + str(turn_count) + '-' + str(index),'type':'reasoning','summary':['Synthetic reasoning summary\n\n' + 'deferred synthetic reasoning ' * 300]}})
            emit({'id':'independent-question','method':'item/commandExecution/requestApproval','params':{'threadId':thread_id,'turnId':current_turn,'command':'true','availableDecisions':['accept','decline']}})
            mark('send-entered')
            threading.Thread(target=release_send, args=(request, current_turn), daemon=True).start()
    elif method == 'turn/interrupt':
        mark('cancel-wire')
        ack(request, {})
        notify('turn/completed', {'threadId':thread_id,'turn':{'id':current_turn,'status':'interrupted'}})
    elif method is None:
        if request.get('id') == 'independent-question': mark('reply-wire', json.dumps(request.get('result')))
    elif 'id' in request: ack(request, {})
"""#
    do {
        try Data(script.utf8).write(to: peer)
        try FileManager.default.setAttributes([.posixPermissions: 0o700], ofItemAtPath: peer.path)
        let transport = Transport(stateDirectory: home.appendingPathComponent("core"), resourcesDirectory: Bundle.main.resourceURL!)
        let defaults = try transport.list()
        _ = try transport.upsertRuntime(RuntimeInstance(enabled: true, id: "synthetic-codex", name: "Synthetic Codex", typeId: .codex166,
            gatewayId: defaults.gateways.first?.id ?? "default", settings: ["binary": peer.path, "nodeBinary": "/synthetic/not-executed-node", "agentDir": home.appendingPathComponent("native").path]),
            execution: RuntimeExecution(transport: .local, home: .existing(path: home.path), remoteResourcesDirectory: nil))
        if CommandLine.arguments.contains("--check-child-open") {
            let configuration = try transport.list()
            let wire = home.appendingPathComponent("wire.jsonl")
            func count() -> Int { ((try? String(contentsOf: wire, encoding: .utf8)) ?? "").split(separator: "\n").count }
            let before = count()
            let device = DeviceStore(transport: transport)
            device.applyList(configuration)
            let store = ConversationStore(device: device)
            let id = "synthetic-codex:cold-child"
            let started = ProcessInfo.processInfo.systemUptime
            store.openLinkedConversation(id: id)
            guard store.pendingConversationID == id, store.reachingConversationID == nil else {
                throw TransportError.rejected("linked open must go straight to pending history")
            }
            while store.isLoading && ProcessInfo.processInfo.systemUptime - started < 10 {
                RunLoop.main.run(until: Date().addingTimeInterval(0.01))
            }
            let elapsed = Int((ProcessInfo.processInfo.systemUptime - started) * 1000)
            let records = try String(contentsOf: wire, encoding: .utf8).split(separator: "\n").dropFirst(before)
            let requests = try records.map { try JSONSerialization.jsonObject(with: Data($0.utf8)) as? [String: Any] ?? [:] }
            guard store.loadedConversationID == id,
                  store.loadedParentConversationID == "synthetic-codex:parent-thread",
                  store.inputAccess == .readOnly,
                  requests.filter({ ($0["method"] as? String) == "initialize" }).count == 1,
                  requests.filter({ ($0["method"] as? String) == "thread/read" }).count == 2,
                  !requests.contains(where: { ($0["method"] as? String) == "thread/list" }) else {
                throw TransportError.rejected("expected one exact legacy history reader, no index and the child snapshot")
            }
            store.close()
            try transport.close(stoppingWork: true)
            print("PASS Mac exact linked child open: \(elapsed)ms, one app-server, no index, loaded read-only child")
            exit(0)
        }
        let seed = try transport.createAndSend(runtimeID: "synthetic-codex", cwd: home.path, project: nil,
            selection: .native(providerId: "openai", modelId: "synthetic-native"),
            turn: ComposedTurn(text: "synthetic seed", effort: nil, permissionMode: nil, attachments: []), cancellation: VeluneSendCancellation(), onCreated: { _ in })
        if CommandLine.arguments.contains("--check-queue-withdrawal") {
            try ConversationStore.checkQueueWithdrawal(transport: transport, seed: seed, home: home)
        } else if CommandLine.arguments.contains("--check-late-send-echo") {
            try ConversationStore.checkLateSendEcho(transport: transport, seed: seed, home: home)
        } else {
            try ConversationStore.checkIndependentRequests(transport: transport, seed: seed, home: home)
            FileHandle.standardError.write(Data("PASS ConversationStore independent request checks (headless, no App/scene/window)\n".utf8))
        }
        try transport.close(stoppingWork: true)
        exit(0)
    } catch {
        FileHandle.standardError.write(Data("FAIL independent requests: \(error)\n".utf8))
        exit(1)
    }
}
