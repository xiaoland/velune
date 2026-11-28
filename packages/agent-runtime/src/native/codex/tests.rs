use super::*;

#[cfg(unix)]
struct CommandFixture {
    root: PathBuf,
    session: NativeSession,
}
#[cfg(unix)]
impl CommandFixture {
    fn new(mode: &str) -> Self {
        use std::os::unix::fs::PermissionsExt;
        let mut nonce = [0; 8];
        getrandom::fill(&mut nonce).unwrap();
        let root = std::env::temp_dir().join(format!(
            "velune-codex-commands-{:x}",
            u64::from_le_bytes(nonce)
        ));
        std::fs::create_dir(&root).unwrap();
        std::fs::write(root.join("mode"), mode).unwrap();
        let binary = root.join("codex");
        std::fs::write(&binary, r#"#!/usr/bin/python3
import json, os, pathlib, select, sys, time
root = pathlib.Path(__file__).parent
if '--version' in sys.argv:
    print('codex-cli 0.166.0'); sys.exit(0)
def emit(method, params):
    print(json.dumps({'method':method,'params':params}), flush=True)
def ack(request, result):
    print(json.dumps({'id':request['id'],'result':result}), flush=True)
def approval(id):
    print(json.dumps({'id':id,'method':'item/commandExecution/requestApproval','params':{'threadId':'thread','turnId':'turn','command':'true','availableDecisions':['accept','decline']}}), flush=True)
held = None
held_child = None
replies = set()
ended = False
active_terminals = {'11': 'shell-a', '12': 'shell-b'} if (root / 'mode').read_text() == 'background-terminals' else {}
late_terminal = None
buffer = ''
while True:
    if late_terminal and (root / 'finish-terminal').exists():
        emit('item/completed', {'threadId':'thread','turnId':'turn','item':{'id':late_terminal,'type':'commandExecution','source':'unifiedExecStartup','processId':'11','status':'failed','exitCode':137}})
        late_terminal = None
    if held and (root / 'release').exists():
        ack(held, {'turn':{'id':'turn'}}); held = None
    if '\n' not in buffer:
        ready, _, _ = select.select([sys.stdin], [], [], 0.005)
        if not ready: continue
        chunk = os.read(0, 65536)
        if not chunk: break
        buffer += chunk.decode()
        if '\n' not in buffer: continue
    line, buffer = buffer.split('\n', 1)
    request = json.loads(line)
    with (root / 'wire').open('a') as log: log.write(json.dumps(request) + '\n')
    method = request.get('method')
    if method == 'initialize': ack(request, {})
    elif method == 'config/read': ack(request, {'config':{'model_provider':'openai','model':'synthetic'}})
    elif method == 'thread/start':
        ack(request, {'thread':{'id':'thread','name':'Synthetic','createdAt':12}})
        if (root / 'mode').read_text().startswith('children-'):
            for child in ('child-a', 'child-b'):
                emit('thread/status/changed', {'threadId':child,'status':{'type':'active','activeFlags':[]}})
                emit('turn/started', {'threadId':child,'turn':{'id':'turn-' + child}})
        if (root / 'mode').read_text() == 'block-input':
            emit('thread/status/changed', {'threadId':'child','status':{'type':'active','activeFlags':[]}})
            deadline = time.monotonic() + 8
            unknown_sent = False
            while not (root / 'release-stdin').exists():
                if (root / 'send-enqueued').exists() and not unknown_sent:
                    unknown_sent = True
                    print(json.dumps({'id':'unsupported','method':'synthetic/unsupported','params':{'threadId':'thread'}}), flush=True)
                if time.monotonic() >= deadline: sys.exit(2)
                time.sleep(0.005)
    elif method == 'thread/resume': ack(request, {'thread':{'id':request['params']['threadId']}})
    elif method == 'turn/start':
        mode = (root / 'mode').read_text()
        if mode == 'reject':
            print(json.dumps({'id':request['id'],'error':{'code':-32600,'message':'explicit reject'}}), flush=True)
        elif mode == 'disconnect': sys.exit(0)
        elif mode == 'accept-disconnect':
            ack(request, {'turn':{'id':'accepted'}}); sys.exit(0)
        elif mode in ('hold', 'children-deadline'): held = request
        elif mode == 'children-malformed':
            sys.stdout.write(json.dumps({'id':request['id'],'result':{'turn':{'id':'accepted'}}}) + '\n{malformed\n')
            sys.stdout.flush()
            # The reader closes its stdout handle after enqueuing the parse error.
            while True:
                try: os.write(sys.stdout.fileno(), b'\n')
                except BrokenPipeError: break
                time.sleep(0.001)
            (root / 'reader-closed').touch()
        elif mode == 'background-terminals':
            ack(request, {'turn':{'id':'turn'}})
            emit('turn/started', {'threadId':'thread','turn':{'id':'turn'}})
            emit('item/started', {'threadId':'thread','turnId':'turn','item':{'id':'shell-a','type':'commandExecution','source':'unifiedExecStartup','processId':'11','status':'inProgress'}})
            emit('turn/completed', {'threadId':'thread','turn':{'id':'turn','status':'completed'}})
        elif mode == 'resume':
            ack(request, {'turn':{'id':'resumed'}})
            emit('turn/completed', {'threadId':'thread','turn':{'id':'resumed','status':'completed'}})
        else:
            held = request
            emit('item/completed', {'threadId':'thread','turnId':'turn','item':{'id':'user','type':'userMessage','content':request['params']['input']}})
            emit('turn/started', {'threadId':'thread','turn':{'id':'turn'}})
            emit('item/started', {'threadId':'thread','turnId':'turn','item':{'id':'assistant','type':'agentMessage','text':'early output'}})
            approval('approval-a'); approval('approval-b')
            emit('thread/status/changed', {'threadId':'child','status':{'type':'active','activeFlags':[]}})
    elif method == 'thread/backgroundTerminals/list':
        assert request['params']['threadId'] == 'thread'
        refreshed = (root / 'refresh-terminal').exists()
        ack(request, {'data':[{'itemId':i,'processId':p,'command':i+' updated' if refreshed else i} for p,i in active_terminals.items()], 'nextCursor':None})
    elif method == 'thread/backgroundTerminals/terminate':
        assert request['params']['threadId'] == 'thread'
        late_terminal = active_terminals.pop(request['params']['processId'], None)
        ack(request, {'terminated':late_terminal is not None})
    elif method == 'turn/interrupt' and request['params']['threadId'] == 'child-b':
        mode = (root / 'mode').read_text()
        if mode == 'children-disconnect': sys.exit(0)
        held_child = request
        print(json.dumps({'id':'child-approval','method':'item/commandExecution/requestApproval','params':{'threadId':'child-b','turnId':'turn-child-b','command':'true','availableDecisions':['accept']}}), flush=True)
    elif method == 'turn/interrupt' and (root / 'mode').read_text() == 'cancel-reject':
        print(json.dumps({'id':request['id'],'error':{'code':-32600,'message':'interrupt rejected'}}), flush=True)
        emit('item/agentMessage/delta', {'threadId':'thread','turnId':'turn','itemId':'assistant','delta':' still running'})
    elif method in ('turn/steer', 'turn/interrupt'): ack(request, {})
    elif method is None:
        if request['id'] == 'child-approval' and held_child:
            if (root / 'mode').read_text() == 'children-reject':
                print(json.dumps({'id':held_child['id'],'error':{'code':-32600,'message':'child interrupt rejected'}}), flush=True)
                held_child = None
            elif (root / 'mode').read_text() != 'children-hold':
                ack(held_child, {}); held_child = None
        replies.add(request['id'])
        if {'approval-a', 'approval-b'} <= replies and not ended:
            ended = True
            emit('turn/completed', {'threadId':'thread','turn':{'id':'turn','status':'completed'}})
            approval('approval-after-terminal')
"#).unwrap();
        std::fs::set_permissions(&binary, std::fs::Permissions::from_mode(0o700)).unwrap();
        let session = NativeSession::connect(NativeConfig {
            execution: crate::execution::ExecutionTarget::Local,
            os_home: Some(root.clone()),
            kind: NativeKind::Codex166,
            binary,
            node_binary: None,
            agent_dir: root.clone(),
            resources_directory: root.clone(),
            projection_directory: root.clone(),
            gateway: None,
            auto_compaction: None,
            mcp_servers: vec![],
            session_agents: vec![],
        })
        .unwrap();
        Self { root, session }
    }
    fn input(text: &str) -> crate::TurnInput {
        crate::TurnInput {
            text: text.into(),
            effort: None,
            permission_mode: None,
            images: vec![],
        }
    }
    fn enqueue(&mut self, command: CodexCommand) -> Retained<CodexOperationTicket> {
        self.session
            .enqueue_codex_command(
                command,
                std::time::Instant::now() + std::time::Duration::from_secs(4),
            )
            .unwrap()
    }
    fn poll_until(&mut self, condition: impl Fn(&NativeSession) -> bool) {
        let deadline = std::time::Instant::now() + std::time::Duration::from_secs(3);
        while !condition(&self.session) {
            assert!(
                std::time::Instant::now() < deadline,
                "condition did not settle"
            );
            self.session.poll().unwrap();
            std::thread::sleep(std::time::Duration::from_millis(5));
        }
    }
    fn start(&mut self) {
        let opened = self.session.create(Some(&self.root), "codex").unwrap();
        assert_eq!(opened.conversation.id, "codex:thread");
        assert_eq!(opened.conversation.created_at_unix_ms, Some(12_000));
    }
    fn wire(&self) -> Vec<Value> {
        std::fs::read_to_string(self.root.join("wire"))
            .unwrap()
            .lines()
            .map(|line| serde_json::from_str(line).unwrap())
            .collect()
    }
}
#[cfg(unix)]
#[test]
fn codex_background_terminals_use_live_inventory_and_exact_stop() {
    let mut fixture = CommandFixture::new("background-terminals");
    fixture.start();
    fixture.session.poll().unwrap();
    assert_eq!(
        fixture.session.work_status(),
        crate::work::WorkStatus::Settled
    );
    assert!(!fixture.session.background_tasks().unwrap().live);
    fixture.enqueue(CodexCommand::Send(CommandFixture::input("synthetic")));
    fixture.poll_until(|session| {
        !session.turn_running() && session.background_tasks().unwrap().tasks.len() == 2
    });
    let tasks = fixture.session.background_tasks().unwrap();
    assert!(
        tasks.live
            && tasks.tasks.iter().all(|task| {
                task.kind == "shell"
                    && task.can_stop
                    && !task.can_read_output
                    && task.conversation_id.is_none()
            })
    );
    assert_eq!(
        fixture.session.work_status(),
        crate::work::WorkStatus::Active
    );
    std::fs::write(fixture.root.join("refresh-terminal"), "").unwrap();
    fixture.session.refresh_background_tasks(true);
    fixture.poll_until(|session| {
        session
            .background_tasks()
            .unwrap()
            .tasks
            .iter()
            .any(|task| task.title == "shell-a updated")
    });
    assert!(
        fixture
            .session
            .enqueue_codex_command(
                CodexCommand::StopBackground,
                std::time::Instant::now() + std::time::Duration::from_secs(1),
            )
            .is_err(),
        "the sub-agent batch must not acknowledge a terminal stop"
    );
    let a = tasks
        .tasks
        .iter()
        .find(|task| task.title == "shell-a")
        .unwrap()
        .id
        .clone();
    let b = tasks
        .tasks
        .iter()
        .find(|task| task.title == "shell-b")
        .unwrap()
        .id
        .clone();
    assert!(
        fixture
            .session
            .stop_background_task("foreign-task")
            .is_err()
    );
    fixture.session.stop_background_task(&a).unwrap();
    let status = |session: &NativeSession, id: &str| {
        session
            .background_tasks()
            .unwrap()
            .tasks
            .iter()
            .find(|task| task.id == id)
            .unwrap()
            .status
    };
    assert_eq!(status(&fixture.session, &a), BackgroundTaskStatus::Stopping);
    fixture.poll_until(|session| status(session, &a) == BackgroundTaskStatus::Unknown);
    assert_eq!(status(&fixture.session, &b), BackgroundTaskStatus::Running);
    std::fs::write(fixture.root.join("finish-terminal"), "").unwrap();
    fixture.poll_until(|session| status(session, &a) == BackgroundTaskStatus::Cancelled);
    let wire = std::fs::read_to_string(fixture.root.join("wire")).unwrap();
    let stops: Vec<Value> = wire
        .lines()
        .map(|line| serde_json::from_str(line).unwrap())
        .filter(|record: &Value| record["method"] == "thread/backgroundTerminals/terminate")
        .collect();
    assert_eq!(stops.len(), 1);
    assert_eq!(
        stops[0]["params"],
        json!({"threadId":"thread","processId":"11"})
    );
}

#[cfg(unix)]
impl Drop for CommandFixture {
    fn drop(&mut self) {
        self.session.shutdown().unwrap();
        std::fs::remove_dir_all(&self.root).unwrap();
    }
}

#[cfg(unix)]
#[test]
fn codex_integration_late_reply_fact_survives_consumed_failure_receipt() {
    use std::time::{Duration, Instant};
    let mut fixture = CommandFixture::new("hold");
    fixture.start();
    let d = Instant::now() - Duration::from_millis(1);
    let (sender, write) = crate::transport::WriteReceipt::test_pending(d);
    let ticket = fixture.session.codex_install_operation(
        100,
        json!("synthetic-reply"),
        d,
        CommandOperationKind::Reply {
            interaction_id: "synthetic".into(),
            write,
        },
    );
    assert!(fixture.session.codex_harvest_commands().is_err());
    fixture.session.fail_connection();
    while fixture.session.take_codex_receipt().is_some() {}
    sender
        .send(crate::transport::WriteOutcome::Written(
            crate::WriteCompletion {
                started_at: d - Duration::from_millis(1),
                completed_at: d + Duration::from_millis(1),
                deadline: d,
            },
        ))
        .unwrap();
    fixture.session.poll().unwrap();
    let receipt = fixture.session.take_codex_receipt().unwrap();
    assert_eq!(receipt.ticket, ticket);
    assert!(matches!(
        receipt.delivery,
        CodexDelivery::Indeterminate { .. }
    ));
    assert!(
        matches!(receipt.physical_write,Some(crate::WriteFact::Written(c)) if !c.met_deadline())
    );
    assert!(fixture.session.requires_failure_cleanup());
}

#[cfg(unix)]
#[test]
fn codex_commands_child_interrupts_keep_approvals_live_and_preserve_partial_delivery() {
    for mode in ["children-reject", "children-disconnect", "children-hold"] {
        let mut f = CommandFixture::new(mode);
        f.start();
        f.poll_until(|s| s.work().unwrap().len() == 2);
        let ticket = f
            .session
            .enqueue_codex_command(
                CodexCommand::StopBackground,
                std::time::Instant::now() + std::time::Duration::from_millis(300),
            )
            .unwrap();
        let deadline = std::time::Instant::now() + std::time::Duration::from_secs(2);
        let mut replied = false;
        while f.session.codex.commands.receipts.is_empty() {
            assert!(std::time::Instant::now() < deadline);
            let _ = f.session.poll();
            if !replied && let Some(id) = f.session.interactions.keys().next().cloned() {
                f.enqueue(CodexCommand::Reply {
                    interaction_id: id,
                    reply: RuntimeInteractionReply::Decision {
                        option_id: "accept".into(),
                    },
                });
                replied = true;
            }
            std::thread::sleep(std::time::Duration::from_millis(5));
        }
        // A reply write receipt may precede the aggregate result.
        while f
            .session
            .codex
            .commands
            .pending
            .contains_key(&ticket.sequence)
        {
            assert!(std::time::Instant::now() < deadline);
            let _ = f.session.poll();
            std::thread::sleep(std::time::Duration::from_millis(5));
        }
        let receipts: Vec<_> = std::iter::from_fn(|| f.session.take_codex_receipt()).collect();
        let receipt = receipts.into_iter().find(|r| r.ticket == *ticket).unwrap();
        let CodexDelivery::Interrupts { children } = &receipt.delivery else {
            panic!("aggregate receipt")
        };
        assert_eq!(children.len(), 2);
        assert!(
            children.iter().any(|r| r.thread_id == "child-a"
                && matches!(r.delivery, CodexInterruptDelivery::Accepted))
        );
        let b = children.iter().find(|r| r.thread_id == "child-b").unwrap();
        if mode == "children-reject" {
            assert!(matches!(
                b.delivery,
                CodexInterruptDelivery::Rejected { .. }
            ));
            assert!(replied);
            assert_eq!(
                f.session.work().unwrap().len(),
                2,
                "interrupt ACK does not remove native work"
            );
        } else {
            assert!(matches!(
                b.delivery,
                CodexInterruptDelivery::Indeterminate { .. }
            ));
            assert!(
                !f.session.process_ended,
                "failed group retains its process owner"
            );
        }
        let wire = f.wire();
        let interrupts: Vec<_> = wire
            .iter()
            .filter(|r| r["method"] == "turn/interrupt")
            .collect();
        assert_eq!(interrupts.len(), 2);
        assert!(
            interrupts
                .iter()
                .all(|r| r["params"]["threadId"] != "thread")
        );
    }
}

#[cfg(unix)]
#[test]
fn codex_commands_expired_send_blocks_longer_lived_child_interrupts() {
    let mut f = CommandFixture::new("children-deadline");
    f.start();
    f.poll_until(|s| s.work().unwrap().len() == 2);
    let deadline = std::time::Instant::now() + std::time::Duration::from_millis(150);
    let send = f
        .session
        .enqueue_codex_command(CodexCommand::Send(CommandFixture::input("once")), deadline)
        .unwrap();
    let stop = f.enqueue(CodexCommand::StopBackground);
    std::thread::sleep(
        deadline.saturating_duration_since(std::time::Instant::now())
            + std::time::Duration::from_millis(20),
    );
    assert!(f.session.poll().is_err());
    assert!(f.session.failed);
    assert!(
        !f.session.process_ended,
        "failure barrier must not perform cleanup"
    );
    let receipts: Vec<_> = std::iter::from_fn(|| f.session.take_codex_receipt()).collect();
    assert!(receipts.iter().any(
            |r| r.ticket == *send && matches!(r.delivery, CodexDelivery::Indeterminate { .. })
        ));
    let receipt = receipts.into_iter().find(|r| r.ticket == *stop).unwrap();
    let CodexDelivery::Interrupts { children } = &receipt.delivery else {
        panic!()
    };
    assert_eq!(children.len(), 2);
    assert!(children.iter().all(|child| child.request_id.is_none()
        && matches!(child.delivery, CodexInterruptDelivery::NotDispatched)));
    std::thread::sleep(std::time::Duration::from_millis(30));
    let wire = f.wire();
    assert_eq!(
        wire.iter().filter(|r| r["method"] == "turn/start").count(),
        1
    );
    assert!(!wire.iter().any(|r| r["method"] == "turn/interrupt"));
    f.session.shutdown().unwrap();
    assert!(f.session.process_ended);
}

#[cfg(unix)]
#[test]
fn codex_commands_valid_ack_before_malformed_record_blocks_child_dispatch_and_keeps_acceptance() {
    let mut f = CommandFixture::new("children-malformed");
    f.start();
    // An unnamed new conversation is eligible for the automatic title
    // request triggered while processing its successful send acknowledgement.
    f.session.codex.title.eligible = true;
    f.session.snapshot.as_mut().unwrap().conversation.title = ConversationTitle::Untitled;
    f.poll_until(|s| s.work().unwrap().len() == 2);
    let send = f.enqueue(CodexCommand::Send(CommandFixture::input("once")));
    let stop = f.enqueue(CodexCommand::StopBackground);
    let deadline = std::time::Instant::now() + std::time::Duration::from_secs(2);
    while !f.root.join("reader-closed").exists() {
        assert!(std::time::Instant::now() < deadline);
        std::thread::sleep(std::time::Duration::from_millis(5));
    }
    let error = f
        .session
        .poll()
        .expect_err("queued malformed frame must end this poll");
    assert_eq!(error.code(), "transport");
    assert!(f.session.failed);
    assert!(!f.session.process_ended);
    let receipts: Vec<_> = std::iter::from_fn(|| f.session.take_codex_receipt()).collect();
    assert!(receipts.iter().any(|r| r.ticket == *send
        && matches!(r.delivery,
            CodexDelivery::Accepted { turn_id: Some(ref id), .. } if id == "accepted")));
    let receipt = receipts.into_iter().find(|r| r.ticket == *stop).unwrap();
    let CodexDelivery::Interrupts { children } = &receipt.delivery else {
        panic!()
    };
    assert_eq!(children.len(), 2);
    assert!(children.iter().all(|child| child.request_id.is_none()
        && matches!(child.delivery, CodexInterruptDelivery::NotDispatched)));
    std::thread::sleep(std::time::Duration::from_millis(30));
    let wire = f.wire();
    assert_eq!(
        wire.iter().filter(|r| r["method"] == "turn/start").count(),
        1
    );
    assert!(
        !wire.iter().any(|r| r["method"] == "turn/interrupt"),
        "a still-reading child must not receive mutations after detected malformed output"
    );
    assert_eq!(
        wire.iter()
            .filter(|r| r["method"] == "thread/start")
            .count(),
        1,
        "accepted delivery must not launch an optional title thread after a known terminal failure"
    );
    f.session.shutdown().unwrap();
    assert!(f.session.process_ended);
}

#[cfg(unix)]
#[test]
fn codex_commands_stop_child_accepts_only_the_runtime_prefix_published_by_work() {
    for context in ["native", "set", "restore"] {
        let mut f = CommandFixture::new("children-reject");
        f.start();
        f.poll_until(|s| s.work().unwrap().len() == 2);
        let prefix = if context == "native" {
            "codex"
        } else {
            let source = f.session.owned_snapshot().unwrap().map(|mut source| {
                source.conversation.runtime_id = "source-runtime".into();
                source
            });
            if context == "restore" {
                f.session.restore_context(&source).unwrap();
            } else {
                f.session.set_conversation(source.map(|s| s.conversation));
            }
            "source-runtime"
        };
        assert_eq!(f.session.snapshot().unwrap().context_runtime_id, "codex");
        let work = f.session.work().unwrap();
        let expected_id = format!("{prefix}:child-a");
        let child = work
            .iter()
            .filter_map(|task| task.conversation_id.as_ref())
            .find(|id| *id == &expected_id)
            .expect("work publishes the target's full runtime identity");
        let foreign = if context == "native" {
            "other"
        } else {
            "codex"
        };
        for (invalid, cause) in [
            (
                format!("{foreign}:child-a"),
                "does not belong to this execution",
            ),
            (
                format!("{prefix}:thread"),
                "does not belong to this execution",
            ),
            (
                format!("{prefix}:not-owned"),
                "no longer belongs to this live work ledger",
            ),
        ] {
            let error = f
                .session
                .enqueue_codex_command(
                    CodexCommand::StopChild {
                        conversation_id: invalid.clone(),
                    },
                    std::time::Instant::now() + std::time::Duration::from_secs(1),
                )
                .err()
                .expect("invalid child accepted");
            assert!(error.to_string().contains(cause), "{invalid}: {error}");
        }
        f.enqueue(CodexCommand::StopChild {
            conversation_id: child.clone(),
        });
        f.poll_until(|s| !s.codex.commands.pending());
        let receipt = f.session.take_codex_receipt().unwrap();
        let CodexDelivery::Interrupts { children } = &receipt.delivery else {
            panic!()
        };
        assert_eq!(children.len(), 1);
        assert_eq!(children[0].thread_id, "child-a");
        assert!(matches!(
            children[0].delivery,
            CodexInterruptDelivery::Accepted
        ));
        let wire = f.wire();
        let interrupts: Vec<_> = wire
            .iter()
            .filter(|r| r["method"] == "turn/interrupt")
            .collect();
        assert_eq!(interrupts.len(), 1);
        assert_eq!(interrupts[0]["params"]["threadId"], "child-a");
        assert_eq!(
            f.session.codex.threads["child-a"].state,
            BackgroundState::Stopping
        );
        assert_eq!(f.session.work().unwrap().len(), 2);
        assert_eq!(
            f.session.codex.threads["child-b"].state,
            BackgroundState::Running
        );
    }
}

#[cfg(unix)]
#[test]
fn codex_commands_interrupt_batch_deadline_preserves_undispatched_targets_and_reply_capacity() {
    let mut f = CommandFixture::new("block-input");
    f.start();
    f.enqueue(CodexCommand::Send(CommandFixture::input(
        &"x".repeat(2 * 1024 * 1024),
    )));
    for index in 0..40 {
        f.session
            .codex_handle_record(json!({"method":"turn/started","params":{
                "threadId":format!("child-{index}"),"turn":{"id":format!("turn-{index}")}
            }}))
            .unwrap();
    }
    let ticket = f
        .session
        .enqueue_codex_command(
            CodexCommand::StopBackground,
            std::time::Instant::now() + std::time::Duration::from_millis(150),
        )
        .unwrap();
    let deadline = std::time::Instant::now() + std::time::Duration::from_secs(2);
    while !f.session.failed {
        assert!(std::time::Instant::now() < deadline);
        let _ = f.session.poll();
        std::thread::sleep(std::time::Duration::from_millis(5));
    }
    let receipt = std::iter::from_fn(|| f.session.take_codex_receipt())
        .find(|r| r.ticket == *ticket)
        .unwrap();
    let CodexDelivery::Interrupts { children } = &receipt.delivery else {
        panic!()
    };
    assert_eq!(children.len(), 40);
    let dispatched = children.iter().filter(|r| r.request_id.is_some()).count();
    assert!(
        dispatched > 0 && dispatched < COMMAND_CAPACITY - REPLY_RESERVE,
        "one blocked send plus child batches must leave four ordinary writer slots free"
    );
    assert!(
        children
            .iter()
            .any(|r| matches!(r.delivery, CodexInterruptDelivery::NotDispatched))
    );
    assert!(
        children
            .iter()
            .any(|r| matches!(r.delivery, CodexInterruptDelivery::Indeterminate { .. }))
    );
    assert!(!f.session.process_ended);
}

#[cfg(unix)]
#[test]
fn codex_commands_unknown_request_does_not_wait_for_blocked_stdin_or_cleanup() {
    let mut f = CommandFixture::new("block-input");
    f.start();
    f.session
        .enqueue_codex_command(
            CodexCommand::Send(CommandFixture::input(&"x".repeat(2 * 1024 * 1024))),
            std::time::Instant::now() + std::time::Duration::from_millis(250),
        )
        .unwrap();
    std::fs::write(f.root.join("send-enqueued"), []).unwrap();
    let began = std::time::Instant::now();
    while !f.session.failed {
        let _ = f.session.poll();
        assert!(
            began.elapsed() < std::time::Duration::from_secs(2),
            "poll waited for stdin or cleanup"
        );
        std::thread::sleep(std::time::Duration::from_millis(5));
    }
    assert!(
        f.session
            .snapshot()
            .unwrap()
            .messages
            .iter()
            .any(|message| message.blocks.iter().any(
                |block| matches!(block, MessageBlock::Notice { text } if text.contains("尚未支持"))
            ))
    );
    assert!(!f.session.process_ended);
    f.session.shutdown().unwrap();
    assert!(f.session.process_ended);
}

#[cfg(unix)]
#[test]
fn codex_commands_rejected_cancel_preserves_running_turn_and_steering() {
    let mut f = CommandFixture::new("cancel-reject");
    f.start();
    f.enqueue(CodexCommand::Send(CommandFixture::input("continue")));
    f.poll_until(|s| s.turn_id.as_deref() == Some("turn"));
    let cancel = f.enqueue(CodexCommand::Cancel);
    assert!(
        f.session.can_steer(),
        "local cancel admission cannot assert Stopping"
    );
    f.poll_until(|s| !s.codex.commands.receipts.is_empty());
    let receipt = f.session.take_codex_receipt().unwrap();
    assert_eq!(receipt.ticket, *cancel);
    assert!(matches!(receipt.delivery, CodexDelivery::Rejected { .. }));
    assert!(f.session.can_steer());
    let steer = f.enqueue(CodexCommand::Steer(CommandFixture::input(
        "still steerable",
    )));
    f.poll_until(|s| !s.codex.commands.receipts.is_empty());
    assert_eq!(f.session.take_codex_receipt().unwrap().ticket, *steer);
    assert!(f.session.can_steer());
    assert!(f.session.snapshot().unwrap().messages.iter().any(|message|
            message.blocks.iter().any(|block| matches!(block, MessageBlock::Text { text } if text.contains("still running")))));
    std::fs::write(f.root.join("mode"), "gated").unwrap();
    f.enqueue(CodexCommand::Cancel);
    f.poll_until(|s| s.snapshot().unwrap().run_state == RunState::Stopping);
    std::fs::write(f.root.join("mode"), "cancel-reject").unwrap();
    f.enqueue(CodexCommand::Cancel);
    f.poll_until(|s| s.codex.commands.receipts.len() == 2);
    assert_eq!(
        f.session.snapshot().unwrap().run_state,
        RunState::Stopping,
        "another rejected cancel cannot undo an already accepted one"
    );
}

#[cfg(unix)]
#[test]
fn codex_auto_compaction_applies_to_start_and_cold_resume() {
    let mut f = CommandFixture::new("hold");
    let initial = Some(AutoCompactionSetting::Codex {
        token_limit: 50_000,
    });
    f.session.set_auto_compaction(initial.clone());
    f.start();
    f.session.set_auto_compaction(initial);
    assert!(!f.session.restart_needed());

    f.session
        .set_auto_compaction(Some(AutoCompactionSetting::Codex {
            token_limit: 80_000,
        }));
    assert!(f.session.restart_needed());
    f.session.prepare_fork_turn().unwrap();
    f.session.set_auto_compaction(None);
    assert!(f.session.restart_needed());
    f.session.prepare_fork_turn().unwrap();
    assert!(!f.session.restart_needed());

    let wire = f.wire();
    let start = wire.iter().find(|r| r["method"] == "thread/start").unwrap();
    assert_eq!(
        start["params"]["config"]["model_auto_compact_token_limit"],
        50_000
    );
    let mut resumed = wire.iter().filter(|r| r["method"] == "thread/resume");
    let changed = &resumed.next().unwrap()["params"];
    assert_eq!(changed["threadId"], "thread");
    assert_eq!(changed["config"]["model_auto_compact_token_limit"], 80_000);
    let cleared = &resumed.next().unwrap()["params"];
    assert_eq!(cleared["threadId"], "thread");
    assert!(
        cleared["config"]
            .get("model_auto_compact_token_limit")
            .is_none()
    );
    assert!(resumed.next().is_none());
}

#[cfg(unix)]
#[test]
fn codex_auto_compaction_matches_a_configured_default_by_provider_and_model() {
    for (provider, expected) in [("openai", Some(50_000)), ("different", None)] {
        let mut f = CommandFixture::new("hold");
        f.session
            .set_default_model_auto_compactions(vec![ModelAutoCompaction {
                model: ModelSelection::Native {
                    provider_id: Some(provider.into()),
                    model_id: "synthetic".into(),
                },
                setting: AutoCompactionSetting::Codex {
                    token_limit: 50_000,
                },
            }]);
        f.start();
        let wire = f.wire();
        let start = wire.iter().find(|r| r["method"] == "thread/start").unwrap();
        assert_eq!(start["params"]["model"], "synthetic");
        assert_eq!(
            start["params"]["config"]["model_auto_compact_token_limit"].as_u64(),
            expected,
        );
    }
}

#[cfg(unix)]
#[test]
fn native_creation_carries_selection_and_a_changed_gateway_blocks_unprepared_sends() {
    let mut f = CommandFixture::new("hold");
    f.session
        .select_native_model(Retained::from_parts(
            (Some("openai".into()), "selected-model".into()),
            Default::default(),
        ))
        .unwrap();
    f.start();
    assert!(!f.session.restart_needed());
    let wire = f.wire();
    let start = wire.iter().find(|r| r["method"] == "thread/start").unwrap();
    assert_eq!(start["params"]["model"], "selected-model");
    assert_eq!(start["params"]["modelProvider"], "openai");
    let mut changed = CommandFixture::new("hold");
    changed
        .session
        .select_model(GatewayInjection {
            endpoint: "http://127.0.0.1:9/v1".into(),
            token: "synthetic-not-used".into(),
            model_alias: "synthetic".into(),
            protocol: "openai-responses".into(),
            context_window: None,
            max_output_tokens: None,
            reasoning_levels: None,
        })
        .unwrap();
    assert!(matches!(
        changed.session.enqueue_codex_command(
            CodexCommand::Send(CommandFixture::input("held")),
            std::time::Instant::now() + std::time::Duration::from_secs(1)
        ),
        Err(CodexAdmissionError::NeedsPreparation)
    ));
    assert!(
        !changed
            .wire()
            .iter()
            .any(|r| matches!(r["method"].as_str(), Some("thread/start" | "turn/start")))
    );
}

#[cfg(unix)]
#[test]
fn codex_commands_admission_and_poll_do_not_wait_for_blocked_stdin() {
    let mut f = CommandFixture::new("block-input");
    f.start();
    let ticket = f.enqueue(CodexCommand::Send(CommandFixture::input(
        &"x".repeat(2 * 1024 * 1024),
    )));
    f.poll_until(|s| !s.work().unwrap().is_empty());
    for _ in 0..3 {
        f.session.poll().unwrap();
    }
    assert!(f.session.take_codex_receipt().is_none());
    assert!(f.session.has_pending_metadata());
    assert!(
        !f.session.turn_running(),
        "local admission is not native acceptance"
    );
    assert_eq!(
        f.wire()
            .iter()
            .filter(|r| r["method"] == "turn/start")
            .count(),
        0,
        "a real oversized stdin frame must remain blocked before gate release"
    );
    std::fs::write(f.root.join("mode"), "resume").unwrap();
    std::fs::write(f.root.join("release-stdin"), []).unwrap();
    f.poll_until(|s| !s.codex.commands.pending());
    assert_eq!(f.session.take_codex_receipt().unwrap().ticket, *ticket);
    assert_eq!(
        f.wire()
            .iter()
            .filter(|r| r["method"] == "turn/start")
            .count(),
        1
    );
}

#[cfg(unix)]
#[test]
fn codex_commands_service_approvals_output_steer_cancel_and_terminal_before_send_ack() {
    let mut f = CommandFixture::new("gated");
    f.start();
    let send = f.enqueue(CodexCommand::Send(CommandFixture::input("once")));
    f.poll_until(|s| s.interactions.len() == 2 && s.snapshot().unwrap().messages.len() == 2);
    assert!(f.session.take_codex_receipt().is_none());
    assert_eq!(f.session.turn_id.as_deref(), Some("turn"));
    assert_eq!(f.session.work().unwrap().len(), 1);
    let steer = f.enqueue(CodexCommand::Steer(CommandFixture::input("steer once")));
    let cancel = f.enqueue(CodexCommand::Cancel);
    f.poll_until(|s| s.codex.commands.receipts.len() == 2);
    assert!(
        f.session.turn_running(),
        "interrupt acceptance must not fabricate turn completion"
    );
    let receipts: Vec<_> = std::iter::from_fn(|| f.session.take_codex_receipt()).collect();
    assert!(
        receipts
            .iter()
            .any(|r| r.ticket == *steer && matches!(r.delivery, CodexDelivery::Accepted { .. }))
    );
    assert!(
        receipts
            .iter()
            .any(|r| r.ticket == *cancel && matches!(r.delivery, CodexDelivery::Accepted { .. }))
    );
    let interactions: Vec<_> = f.session.interactions.keys().cloned().collect();
    for id in interactions {
        f.enqueue(CodexCommand::Reply {
            interaction_id: id,
            reply: RuntimeInteractionReply::Decision {
                option_id: "accept".into(),
            },
        });
    }
    f.poll_until(|s| !s.turn_running() && s.interactions.len() == 1);
    assert!(f.session.codex.commands.starting_turn());
    assert_eq!(
        f.session.work().unwrap().len(),
        1,
        "parent terminal does not release background work"
    );
    let id = f.session.interactions.keys().next().unwrap().clone();
    f.enqueue(CodexCommand::Reply {
        interaction_id: id.clone(),
        reply: RuntimeInteractionReply::Decision {
            option_id: "accept".into(),
        },
    });
    assert!(
        f.session
            .enqueue_codex_command(
                CodexCommand::Reply {
                    interaction_id: id,
                    reply: RuntimeInteractionReply::Decision {
                        option_id: "accept".into()
                    },
                },
                std::time::Instant::now() + std::time::Duration::from_secs(1)
            )
            .is_err(),
        "do not duplicate a pending reply"
    );
    f.poll_until(|s| s.interactions.is_empty());
    let replies: Vec<_> = std::iter::from_fn(|| f.session.take_codex_receipt()).collect();
    assert_eq!(replies.len(), 3);
    assert!(
        replies
            .iter()
            .all(|r| matches!(r.delivery, CodexDelivery::Written))
    );
    std::fs::write(f.root.join("release"), []).unwrap();
    f.poll_until(|s| !s.codex.commands.pending());
    let receipt = f.session.take_codex_receipt().unwrap();
    assert_eq!(receipt.ticket, *send);
    assert!(
        matches!(receipt.delivery, CodexDelivery::Accepted { turn_id: Some(ref id), .. } if id == "turn")
    );
    assert!(
        !f.session.turn_running(),
        "late ack must not revive a completed turn"
    );
    assert!(f.session.turn_id.is_none());
    assert_eq!(
        f.session
            .snapshot()
            .unwrap()
            .messages
            .iter()
            .map(|m| m.id.as_str())
            .collect::<Vec<_>>(),
        vec!["user", "assistant"]
    );
    assert!(
        f.session
            .snapshot()
            .unwrap()
            .executions
            .current
            .as_ref()
            .unwrap()
            .ended
    );
    std::fs::write(f.root.join("mode"), "resume").unwrap();
    let resume = f.enqueue(CodexCommand::Resume {
        effort: Some("high".into()),
        permission_mode: None,
    });
    f.poll_until(|s| !s.codex.commands.pending());
    assert_eq!(f.session.take_codex_receipt().unwrap().ticket, *resume);
    let wire = f.wire();
    let turns: Vec<_> = wire
        .iter()
        .filter(|r| r["method"] == "turn/start")
        .collect();
    assert_eq!(turns.len(), 2);
    assert_eq!(turns[0]["params"]["input"][0]["text"], "once");
    assert_eq!(turns[1]["params"]["input"], json!([]));
    assert_eq!(turns[1]["params"]["effort"], "high");
    for id in ["approval-a", "approval-b", "approval-after-terminal"] {
        assert_eq!(
            wire.iter()
                .filter(|r| r.get("method").is_none() && r["id"] == id)
                .count(),
            1
        );
    }
}

#[cfg(unix)]
#[test]
fn codex_commands_distinguish_native_reject_disconnect_timeout_without_replay() {
    for mode in ["reject", "disconnect", "hold"] {
        let mut f = CommandFixture::new(mode);
        f.start();
        f.session.snapshot.as_mut().unwrap().follow_up.can_resume = true;
        let ticket = f
            .session
            .enqueue_codex_command(
                CodexCommand::Send(CommandFixture::input("once")),
                std::time::Instant::now() + std::time::Duration::from_millis(250),
            )
            .unwrap();
        let deadline = std::time::Instant::now() + std::time::Duration::from_secs(2);
        while f.session.codex.commands.receipts.is_empty() {
            assert!(std::time::Instant::now() < deadline);
            let _ = f.session.poll();
            std::thread::sleep(std::time::Duration::from_millis(5));
        }
        let receipt = f.session.take_codex_receipt().unwrap();
        assert_eq!(receipt.ticket, *ticket);
        if mode == "reject" {
            assert!(
                matches!(&receipt.delivery, CodexDelivery::Rejected { error } if error["code"] == -32600)
            );
            assert!(!f.session.failed);
            assert!(
                f.session.snapshot().unwrap().follow_up.can_resume,
                "native rejection must not consume the previous resumable context"
            );
            assert!(f.session.snapshot().unwrap().executions.current.is_none());
        } else {
            assert!(matches!(
                receipt.delivery,
                CodexDelivery::Indeterminate { .. }
            ));
            assert!(f.session.failed);
            assert!(
                !f.session.process_ended,
                "deadline/failure is not process cleanup"
            );
            assert!(matches!(
                f.session.enqueue_codex_command(
                    CodexCommand::Send(CommandFixture::input("never replay")),
                    std::time::Instant::now() + std::time::Duration::from_secs(1)
                ),
                Err(CodexAdmissionError::NeedsPreparation)
            ));
        }
        if mode == "hold" {
            std::fs::write(f.root.join("release"), []).unwrap();
            std::thread::sleep(std::time::Duration::from_millis(30));
            f.session.poll().unwrap();
            assert!(f.session.turn_id.is_none());
            assert!(
                f.session.take_codex_receipt().is_none(),
                "late response cannot replace indeterminate"
            );
        }
        assert_eq!(
            f.wire()
                .iter()
                .filter(|r| r["method"] == "turn/start")
                .count(),
            1
        );
    }
}

#[cfg(unix)]
#[test]
fn codex_commands_old_connection_late_ack_cannot_install_in_replacement_generation() {
    let mut f = CommandFixture::new("hold");
    f.start();
    let old = f.enqueue(CodexCommand::Send(CommandFixture::input("old once")));
    // Ensure the old helper actually received the mutation, then keep its
    // process and receiver owned while constructing the replacement seam.
    let deadline = std::time::Instant::now() + std::time::Duration::from_secs(2);
    while !f.wire().iter().any(|r| r["method"] == "turn/start") {
        assert!(std::time::Instant::now() < deadline);
        std::thread::sleep(std::time::Duration::from_millis(5));
    }
    let mut retired = NativeSession::connect(f.session.config.clone()).unwrap();
    f.session.codex.commands.next_generation().unwrap();
    std::mem::swap(&mut f.session.rpc, &mut retired.rpc);
    f.session.rpc.defer_cleanup();
    f.session.settle(RunState::Idle);
    f.session.codex_capture_provider(None).unwrap();
    f.session
        .rpc
        .request("thread/resume", &json!({"threadId":"thread"}))
        .unwrap();
    std::fs::write(f.root.join("mode"), "resume").unwrap();
    let new = f.enqueue(CodexCommand::Send(CommandFixture::input("new once")));
    assert_eq!(
        old.request_id, new.request_id,
        "exercise reused wire ids across connections"
    );
    assert_ne!(old.generation, new.generation);
    std::fs::write(f.root.join("release"), []).unwrap();
    let mut late = false;
    while !late {
        assert!(std::time::Instant::now() < deadline);
        late = retired
            .rpc
            .poll()
            .unwrap()
            .iter()
            .any(|r| r["id"] == old.request_id && r["result"]["turn"]["id"] == "turn");
        std::thread::sleep(std::time::Duration::from_millis(5));
    }
    f.poll_until(|s| !s.codex.commands.pending());
    let receipt = f.session.take_codex_receipt().unwrap();
    assert_eq!(receipt.ticket, *old);
    assert!(matches!(
        receipt.delivery,
        CodexDelivery::Indeterminate { .. }
    ));
    let receipt = f.session.take_codex_receipt().unwrap();
    assert_eq!(receipt.ticket, *new);
    assert!(
        matches!(&receipt.delivery, CodexDelivery::Accepted { turn_id: Some(id), .. } if id == "resumed")
    );
    assert!(f.session.turn_id.is_none());
    retired.shutdown().unwrap();
    assert_eq!(
        f.wire()
            .iter()
            .filter(|r| r["method"] == "turn/start")
            .count(),
        2
    );
}

#[cfg(unix)]
#[test]
fn codex_commands_accepted_delivery_survives_following_metadata_transport_failure() {
    let mut f = CommandFixture::new("accept-disconnect");
    f.start();
    f.session.codex.title.eligible = true;
    let ticket = f.enqueue(CodexCommand::Send(CommandFixture::input("accepted once")));
    let deadline = std::time::Instant::now() + std::time::Duration::from_secs(2);
    while !f.session.failed {
        assert!(std::time::Instant::now() < deadline);
        let _ = f.session.poll();
        std::thread::sleep(std::time::Duration::from_millis(5));
    }
    let receipt = f.session.take_codex_receipt().unwrap();
    assert_eq!(receipt.ticket, *ticket);
    assert!(
        matches!(&receipt.delivery, CodexDelivery::Accepted { turn_id: Some(id), .. } if id == "accepted")
    );
    assert!(f.session.take_codex_receipt().is_none());
    assert!(!f.session.process_ended);
    assert_eq!(
        f.wire()
            .iter()
            .filter(|r| r["method"] == "turn/start")
            .count(),
        1
    );
}

#[cfg(unix)]
#[test]
fn codex_commands_bound_unconsumed_results_and_check_sequences() {
    let mut f = CommandFixture::new("reject");
    f.start();
    for _ in 0..COMMAND_CAPACITY - REPLY_RESERVE {
        f.enqueue(CodexCommand::Send(CommandFixture::input("rejected once")));
        f.poll_until(|s| !s.codex.commands.pending());
    }
    assert_eq!(
        f.session.codex.commands.receipts.len(),
        COMMAND_CAPACITY - REPLY_RESERVE
    );
    assert!(
        matches!(f.session.enqueue_codex_command(CodexCommand::Send(CommandFixture::input("not dispatched")),
            std::time::Instant::now() + std::time::Duration::from_secs(1)), Err(CodexAdmissionError::Rejected(error)) if error.code() == "queue_full")
    );
    // Reserved reply slots remain usable with twelve unread ordinary
    // results, but cannot become an unbounded bypass around that budget.
    for index in 0..REPLY_RESERVE + 1 {
        f.session
            .codex_handle_record(json!({"id":format!("reserved-{index}"),
            "method":"item/commandExecution/requestApproval", "params":{
                "threadId":"thread", "command":"true", "availableDecisions":["accept"]
            }}))
            .unwrap();
    }
    let ids: Vec<_> = f.session.interactions.keys().cloned().collect();
    for id in &ids[..REPLY_RESERVE] {
        f.enqueue(CodexCommand::Reply {
            interaction_id: id.clone(),
            reply: RuntimeInteractionReply::Decision {
                option_id: "accept".into(),
            },
        });
    }
    assert!(
        matches!(f.session.enqueue_codex_command(CodexCommand::Reply {
            interaction_id: ids[REPLY_RESERVE].clone(),
            reply: RuntimeInteractionReply::Decision { option_id: "accept".into() },
        }, std::time::Instant::now() + std::time::Duration::from_secs(1)),
            Err(CodexAdmissionError::Rejected(error)) if error.code() == "queue_full")
    );
    f.poll_until(|s| !s.codex.commands.pending());
    assert_eq!(f.session.codex.commands.receipts.len(), COMMAND_CAPACITY);
    // Make room for ordinary commands, then exercise checked arithmetic.
    for _ in 0..REPLY_RESERVE + 1 {
        f.session.take_codex_receipt().unwrap();
    }
    f.session.codex.commands.next = u64::MAX;
    assert!(
        f.session
            .enqueue_codex_command(
                CodexCommand::Send(CommandFixture::input("not dispatched")),
                std::time::Instant::now() + std::time::Duration::from_secs(1)
            )
            .is_err()
    );
    assert_eq!(
        f.wire()
            .iter()
            .filter(|r| r["method"] == "turn/start")
            .count(),
        COMMAND_CAPACITY - REPLY_RESERVE
    );
}

#[cfg(unix)]
#[test]
fn fork_rejection_is_distinct_from_unknown_or_already_created_results() {
    use std::os::unix::fs::PermissionsExt;
    let mut nonce = [0; 8];
    getrandom::fill(&mut nonce).unwrap();
    let root = std::env::temp_dir().join(format!(
        "velune-fork-result-{:x}",
        u64::from_le_bytes(nonce)
    ));
    std::fs::create_dir(&root).unwrap();
    let binary = root.join("codex");
    std::fs::write(&binary, r#"#!/usr/bin/python3
import json, pathlib, sys
root = pathlib.Path(__file__).parent
if '--version' in sys.argv:
    print('codex-cli 0.166.0')
    sys.exit(0)
for line in sys.stdin:
    request = json.loads(line)
    if 'id' not in request:
        continue
    method = request['method']
    with (root / 'calls').open('a') as log:
        log.write(method + '\t' + request.get('params', {}).get('threadId', '') + '\n')
    response = {'jsonrpc':'2.0','id':request['id'],'result':{}}
    if method == 'thread/fork':
        mode = (root / 'mode').read_text()
        if mode == 'rejected':
            response = {'jsonrpc':'2.0','id':request['id'],'error':{'code':-32601,'message':'fork unsupported'}}
        elif mode == 'created':
            response['result'] = {'thread':{'id':'copy'}}
    elif method == 'thread/resume':
        response = {'jsonrpc':'2.0','id':request['id'],'error':{'code':-1,'message':'cannot open copy'}}
    print(json.dumps(response), flush=True)
"#).unwrap();
    std::fs::set_permissions(&binary, std::fs::Permissions::from_mode(0o700)).unwrap();
    for (mode, code) in [
        ("rejected", "fork_rejected"),
        ("unknown", "fork_outcome_unknown"),
        ("created", "fork_created"),
    ] {
        std::fs::write(root.join("mode"), mode).unwrap();
        let mut session = NativeSession::connect(NativeConfig {
            execution: crate::execution::ExecutionTarget::Local,
            os_home: Some(root.clone()),
            kind: NativeKind::Codex166,
            binary: binary.clone(),
            node_binary: None,
            agent_dir: root.clone(),
            resources_directory: root.clone(),
            projection_directory: root.clone(),
            gateway: None,
            auto_compaction: None,
            mcp_servers: Vec::new(),
            session_agents: Vec::new(),
        })
        .unwrap();
        let error = session
            .fork_codex_from("source", Some(&root), "codex", "turn")
            .unwrap_err();
        assert_eq!(error.code(), code, "{mode}: {error}");
        session.shutdown().unwrap();
    }
    let calls = std::fs::read_to_string(root.join("calls")).unwrap();
    assert_eq!(
        calls
            .lines()
            .filter(|method| method.starts_with("thread/fork\t"))
            .count(),
        3
    );
    assert!(!calls.contains("turn/start") && !calls.contains("thread/revert"));
    assert!(
        !calls.contains("thread/resume\tsource"),
        "fork must not acquire the source writer"
    );
    std::fs::remove_dir_all(root).unwrap();
}

#[test]
fn error_projection_preserves_details_and_distinguishes_retry() {
    let error = json!({"message":"Selected model is at capacity. Please try a different model.",
            "codexErrorInfo":"serverOverloaded","additionalDetails":"synthetic request detail"});
    let retry = project_error("turn", &error, true, None).unwrap();
    let failed = project_turn_error(&json!({"id":"turn","status":"failed",
            "error":error,"completedAt":123}))
    .unwrap()
    .unwrap();
    assert_eq!(retry.id, failed.id);
    assert!(retry.completed);
    assert!(failed.completed);
    assert_eq!(failed.timestamp_unix_ms, Some(123_000));
    let MessageBlock::RuntimeFailure {
        detail: text,
        failure: RuntimeFailureKind::ServerOverloaded,
        ..
    } = &failed.blocks[0]
    else {
        panic!()
    };
    assert_eq!(
        text,
        "Selected model is at capacity. Please try a different model.\nsynthetic request detail\nCodex 错误类型：serverOverloaded"
    );
    let MessageBlock::RuntimeFailure {
        detail: text,
        retry: Some(RuntimeRetry::Native),
        ..
    } = &retry.blocks[0]
    else {
        panic!()
    };
    assert!(text.ends_with("Codex 正在自动重试。"));
    assert!(project_error("turn", &json!({"message":42}), false, None).is_err());
}
