use super::*;
use velune_test_support::TempRoot;

struct Fixture {
    client: Client,
    root: TempRoot,
}
impl Fixture {
    fn new() -> Self {
        Self::with_script("import sys\nfor line in sys.stdin: pass\n")
    }
    fn with_script(script: &str) -> Self {
        let root = TempRoot::new("pi-port");
        let helper = root.join("rpc.py");
        std::fs::write(&helper, script).unwrap();
        let config: Config = serde_json::from_value(json!({"execution":{"kind":"local"},"os_home":root.path(),"binary":"/usr/bin/python3","node_binary":"/usr/bin/python3","rpc_entry":helper})).unwrap();
        Self {
            client: Client::spawn(config).unwrap(),
            root,
        }
    }
    fn incoming(&mut self) -> mpsc::SyncSender<Incoming> {
        let (sender, receiver) = mpsc::sync_channel(64);
        self.client.incoming = receiver;
        sender
    }
}
impl Drop for Fixture {
    fn drop(&mut self) {
        self.client.shutdown().unwrap();
    }
}
fn prompt() -> PiOperation {
    PiOperation::Prompt(crate::TurnInput {
        text: "once".into(),
        effort: None,
        permission_mode: None,
        images: vec![],
    })
}
fn response(client: &Client, ticket: PiTicket, success: bool) -> Value {
    let operation = &client.port.as_ref().unwrap().operations[&ticket.sequence];
    json!({"type":"response","command":operation.command,"id":operation.id,"success":success,"data":{"disposition":"handled"}})
}
fn phases(items: Vec<PiPortItem>) -> Vec<PiReceiptPhase> {
    items
        .into_iter()
        .filter_map(|item| match item {
            PiPortItem::Receipt(receipt) => Some(receipt.phase),
            _ => None,
        })
        .collect()
}

#[test]
fn pi_metadata_timeout_does_not_poison_an_active_compaction() {
    for operation in [
        PiOperation::SetName("new title".into()),
        PiOperation::Background(json!({"operation":"snapshot"})),
    ] {
        assert_compaction_survives_metadata_timeout(operation);
    }
}

fn assert_compaction_survives_metadata_timeout(operation: PiOperation) {
    let mut fixture = Fixture::new();
    let sender = fixture.incoming();
    let compact = fixture
        .client
        .enqueue_pi(PiOperation::Compact, None)
        .unwrap();
    let metadata = fixture.client.enqueue_pi(operation, None).unwrap();
    let deadline = Instant::now() + Duration::from_secs(2);
    while !fixture.client.port.as_ref().unwrap().operations[&metadata.sequence].written {
        assert!(
            Instant::now() < deadline,
            "metadata must be written before its reply times out"
        );
        fixture.client.poll_pi();
        thread::sleep(Duration::from_millis(2));
    }
    let late_reply = response(&fixture.client, metadata, true);
    let compact_done = response(&fixture.client, compact, true);
    fixture
        .client
        .port
        .as_mut()
        .unwrap()
        .operations
        .get_mut(&metadata.sequence)
        .unwrap()
        .response_deadline = Some(Instant::now());
    sender.send(Incoming::Record(late_reply)).unwrap();
    sender.send(Incoming::Record(compact_done)).unwrap();
    let output = fixture.client.poll_pi();
    assert!(output.iter().any(|item| matches!(item, PiPortItem::Receipt(receipt)
        if receipt.ticket == metadata && matches!(receipt.phase, PiReceiptPhase::Indeterminate(_)))));
    assert!(
        output
            .iter()
            .any(|item| matches!(item, PiPortItem::Receipt(receipt)
        if receipt.ticket == compact && matches!(receipt.phase, PiReceiptPhase::Outcome(_))))
    );
    assert!(!fixture.client.pi_cleanup_required());
    assert!(fixture.client.port.as_ref().unwrap().operations.is_empty());
    assert!(fixture.client.enqueue_pi(prompt(), None).is_ok());
}

#[test]
fn pi_port_old_generation_wrong_command_and_local_abandon_do_not_complete_current_operation() {
    let mut fixture = Fixture::new();
    let sender = fixture.incoming();
    let ticket = fixture.client.enqueue_pi(prompt(), None).unwrap();
    assert!(!fixture.client.abandon_queued_pi(PiTicket {
        generation: ticket.generation + 1,
        ..ticket
    }));
    let mut stale = response(&fixture.client, ticket, true);
    stale["id"] = json!(format!(
        "velune-pi-{}-{}",
        ticket.generation + 1,
        ticket.sequence
    ));
    sender.send(Incoming::Record(stale)).unwrap();
    let mut wrong = response(&fixture.client, ticket, true);
    wrong["command"] = json!("get_state");
    sender.send(Incoming::Record(wrong)).unwrap();
    fixture.client.poll_pi();
    assert!(
        fixture
            .client
            .port
            .as_ref()
            .unwrap()
            .operations
            .contains_key(&ticket.sequence)
    );
    assert!(
        !fixture.client.abandon_queued_pi(ticket),
        "dispatch is irrevocable, even before Written"
    );
    sender
        .send(Incoming::Record(response(&fixture.client, ticket, false)))
        .unwrap();
    assert!(
        phases(fixture.client.poll_pi())
            .iter()
            .any(|phase| matches!(phase, PiReceiptPhase::Rejected(_)))
    );
    assert!(
        !fixture.client.pi_cleanup_required(),
        "native rejection is not transport failure"
    );
    let unsent = fixture.client.enqueue_pi(prompt(), None).unwrap();
    assert!(fixture.client.abandon_queued_pi(unsent));
    assert!(phases(fixture.client.poll_pi()).contains(&PiReceiptPhase::NotSent));
    assert!(
        fixture.client.request(json!({"type":"abort"})).is_err(),
        "facade cannot bypass typed correlation"
    );
}

#[test]
fn pi_port_latches_failure_before_late_outcome_and_retains_cleanup_lease() {
    let mut fixture = Fixture::with_script(
        r#"
import sys,json,os
for line in sys.stdin:
 with open(os.path.join(os.path.dirname(__file__),'wire-commands'),'a') as output:
  output.write(json.loads(line)['type']+'\n')
"#,
    );
    let sender = fixture.incoming();
    let ticket = fixture
        .client
        .enqueue_pi(prompt(), Some(Instant::now() + Duration::from_secs(60)))
        .unwrap();
    fixture.client.poll_pi(); // dispatch before simulating a lost/late response
    let commands = fixture.root.join("wire-commands");
    let ready = Instant::now() + Duration::from_secs(2);
    loop {
        if std::fs::read_to_string(&commands).ok().as_deref() == Some("prompt\n") {
            break;
        }
        assert!(
            Instant::now() < ready,
            "the peer must observe the original prompt"
        );
        fixture.client.poll_pi();
        thread::sleep(Duration::from_millis(5));
    }
    let late = response(&fixture.client, ticket, true);
    fixture
        .client
        .port
        .as_mut()
        .unwrap()
        .operations
        .get_mut(&ticket.sequence)
        .unwrap()
        .response_deadline = Some(Instant::now() - Duration::from_millis(1));
    sender.send(Incoming::Record(late)).unwrap();
    let items = fixture.client.poll_pi();
    let failure = items
        .iter()
        .position(|item| {
            matches!(
                item,
                PiPortItem::Receipt(crate::PiReceipt {
                    phase: PiReceiptPhase::Indeterminate(_),
                    ..
                })
            )
        })
        .unwrap();
    let outcome = items
        .iter()
        .position(|item| {
            matches!(
                item,
                PiPortItem::Receipt(crate::PiReceipt {
                    phase: PiReceiptPhase::Outcome(_),
                    ..
                })
            )
        })
        .unwrap();
    assert!(failure < outcome);
    assert!(fixture.client.pi_cleanup_required());
    let next = fixture.client.port.as_ref().unwrap().next;
    for operation in [prompt(), PiOperation::ClearQueue, PiOperation::Abort] {
        assert!(
            matches!(
                fixture.client.enqueue_pi(operation, None),
                Err(Error::Transport(_))
            ),
            "late native evidence cannot reopen failed-connection admission"
        );
    }
    assert_eq!(fixture.client.port.as_ref().unwrap().next, next);
    fixture.client.poll_pi();
    assert_eq!(std::fs::read_to_string(&commands).unwrap(), "prompt\n");
    assert!(
        fixture.client.child.try_wait().unwrap().is_none(),
        "poll and ticket consumption never kill/reap"
    );
    assert!(
        fixture.client.pi_cleanup_required(),
        "failed cancellation needs terminal cleanup, not a new protocol mutation"
    );
}

#[test]
fn pi_port_written_does_not_ack_prompt_or_renew_explicit_deadline() {
    let mut fixture = Fixture::new();
    let _sender = fixture.incoming();
    let deadline = Instant::now() + Duration::from_secs(20);
    let ticket = fixture.client.enqueue_pi(prompt(), Some(deadline)).unwrap();
    let until = Instant::now() + Duration::from_secs(2);
    loop {
        let evidence = phases(fixture.client.poll_pi());
        assert!(
            !evidence
                .iter()
                .any(|phase| matches!(phase, PiReceiptPhase::Accepted(_)))
        );
        if evidence.contains(&PiReceiptPhase::Written) {
            break;
        }
        assert!(Instant::now() < until);
        thread::sleep(Duration::from_millis(5));
    }
    let operation = &fixture.client.port.as_ref().unwrap().operations[&ticket.sequence];
    assert_eq!(operation.response_deadline, Some(deadline));
    assert!(operation.bytes.is_none());
    let default = fixture.client.enqueue_pi(prompt(), None).unwrap();
    assert!(
        fixture.client.port.as_ref().unwrap().operations[&default.sequence]
            .response_deadline
            .is_none()
    );
    let reply = fixture
        .client
        .enqueue_pi(
            PiOperation::DialogReply(
                json!({"type":"extension_ui_response","id":"dialog","value":"answer"}),
            ),
            Some(deadline),
        )
        .unwrap();
    assert!(
        fixture.client.port.as_ref().unwrap().operations[&reply.sequence].write_deadline
            <= Instant::now() + RESPONSE_TIMEOUT
    );
}

#[test]
fn pi_port_pending_and_unconsumed_evidence_reserve_replies_and_count_encoded_bytes() {
    let mut fixture = Fixture::new();
    let _sender = fixture.incoming();
    let assert_capacity_full = |result: Result<PiTicket>| {
        assert!(
            matches!(&result, Err(Error::Transport(detail))
                if detail == "Pi operation/evidence capacity full"),
            "expected capacity rejection: {result:?}"
        );
    };
    for _ in 0..OPERATIONS - RESERVED {
        fixture.client.enqueue_pi(prompt(), None).unwrap();
    }
    assert_capacity_full(fixture.client.enqueue_pi(prompt(), None));
    for index in 0..RESERVED - 2 {
        fixture.client.enqueue_pi(PiOperation::DialogReply(json!({"type":"extension_ui_response","id":format!("reply-{index}"),"value":"answer"})),None).unwrap();
    }
    assert_capacity_full(fixture.client.enqueue_pi(
        PiOperation::DialogReply(
            json!({"type":"extension_ui_response","id":"overflow","value":"answer"}),
        ),
        None,
    ));
    fixture
        .client
        .enqueue_pi(PiOperation::ClearQueue, None)
        .unwrap();
    fixture.client.enqueue_pi(PiOperation::Abort, None).unwrap();
    assert_capacity_full(fixture.client.enqueue_pi(PiOperation::Abort, None));
    let port = fixture.client.port.as_ref().unwrap();
    assert_eq!(
        port.bytes,
        port.operations
            .values()
            .map(|operation| operation.bytes.as_ref().unwrap().capacity())
            .sum::<usize>()
    );
    // Abandonment leaves evidence owning a slot until the owner consumes it.
    let ticket = port.operations.values().next().unwrap().ticket;
    assert!(fixture.client.abandon_queued_pi(ticket));
    assert_capacity_full(fixture.client.enqueue_pi(PiOperation::Abort, None));
    // Poll also dispatches writes. A later enqueue can harvest fresh Written
    // evidence after that poll returned, so keep consuming until capacity is
    // available rather than assuming one poll consumed every physical receipt.
    let deadline = Instant::now() + Duration::from_secs(2);
    let mut abandoned_receipts = 0;
    loop {
        abandoned_receipts += fixture
            .client
            .poll_pi()
            .iter()
            .filter(|item| {
                matches!(item, PiPortItem::Receipt(receipt)
                    if receipt.ticket == ticket && matches!(receipt.phase, PiReceiptPhase::NotSent))
            })
            .count();
        let admitted = fixture.client.enqueue_pi(PiOperation::Abort, None);
        if admitted.is_ok() {
            break;
        }
        assert_capacity_full(admitted);
        assert!(
            Instant::now() < deadline,
            "consumed evidence did not release capacity"
        );
        thread::yield_now();
    }
    assert_eq!(abandoned_receipts, 1);
    fixture.client.port.as_mut().unwrap().next = u64::MAX;
    // Exhaustion is checked, never wrapped into an existing wire id.
    fixture.client.port.as_mut().unwrap().operations.clear();
    let exhausted = fixture.client.enqueue_pi(PiOperation::Abort, None);
    assert!(
        matches!(&exhausted, Err(Error::Transport(detail)) if detail == "Pi sequence exhausted"),
        "expected checked sequence exhaustion: {exhausted:?}"
    );
}

#[test]
fn pi_reserved_writer_backpressure_keeps_frames_and_cancellation_order() {
    let mut fixture = Fixture::with_script(
        r#"
import json,pathlib,sys,time
root=pathlib.Path(__file__).parent
first=sys.stdin.buffer.read(1)
(root/'entered').write_text('actual first byte')
while not (root/'release').exists(): time.sleep(.001)
count=0
for line in iter(sys.stdin.buffer.readline,b''):
    if first:
        line=first+line
        first=b''
    value=json.loads(line)
    with (root/'wire').open('a') as output:
        output.write(json.dumps({'type':value['type'],'id':value['id']})+'\n')
    count+=1
    (root/'count').write_text(str(count))
"#,
    );
    let mut input = match prompt() {
        PiOperation::Prompt(input) => input,
        _ => unreachable!(),
    };
    input.text = "x".repeat(256 * 1024);
    let prompt = fixture
        .client
        .enqueue_pi(PiOperation::Prompt(input), None)
        .unwrap();
    fixture.client.poll_pi();
    let deadline = Instant::now() + Duration::from_secs(2);
    while !fixture.root.join("entered").exists() {
        assert!(
            Instant::now() < deadline,
            "helper did not enter the actual pipe"
        );
        thread::sleep(Duration::from_millis(2));
    }
    let mut controls = Vec::new();
    for index in 0..RESERVED - 2 {
        controls.push(fixture.client.enqueue_pi(PiOperation::DialogReply(json!({
            "type":"extension_ui_response","id":format!("reply-{index}"),"value":"answer"
        })), None).unwrap());
    }
    controls.push(
        fixture
            .client
            .enqueue_pi(PiOperation::ClearQueue, None)
            .unwrap(),
    );
    controls.push(fixture.client.enqueue_pi(PiOperation::Abort, None).unwrap());
    let last = *controls.last().unwrap();
    let original_deadline =
        fixture.client.port.as_ref().unwrap().operations[&last.sequence].write_deadline;
    let items = fixture.client.poll_pi();
    assert!(
        !fixture.client.pi_cleanup_required(),
        "temporary writer backpressure failed the connection: {items:?}"
    );
    assert!(
        items.is_empty(),
        "blocked input cannot publish delivery: {items:?}"
    );
    let port = fixture.client.port.as_ref().unwrap();
    assert!(
        controls
            .iter()
            .all(|ticket| port.operations.contains_key(&ticket.sequence))
    );
    assert!(port.operations[&last.sequence].bytes.is_some());
    assert_eq!(
        port.operations[&last.sequence].write_deadline,
        original_deadline
    );

    std::fs::write(fixture.root.join("release"), []).unwrap();
    let mut written = Vec::new();
    let deadline = Instant::now() + Duration::from_secs(2);
    while written.len() < controls.len() + 1
        || std::fs::read_to_string(fixture.root.join("count"))
            .ok()
            .as_deref()
            != Some("9")
    {
        for item in fixture.client.poll_pi() {
            if let PiPortItem::Receipt(receipt) = item {
                assert_eq!(receipt.phase, PiReceiptPhase::Written);
                assert!(receipt.ticket == prompt || controls.contains(&receipt.ticket));
                assert!(
                    !written.contains(&receipt.ticket),
                    "physical write was reported twice"
                );
                written.push(receipt.ticket);
            }
        }
        assert!(!fixture.client.pi_cleanup_required());
        assert!(
            Instant::now() < deadline,
            "queued frames did not drain: {written:?}"
        );
        thread::sleep(Duration::from_millis(2));
    }
    let wire: Vec<Value> = std::fs::read_to_string(fixture.root.join("wire"))
        .unwrap()
        .lines()
        .map(|line| serde_json::from_str(line).unwrap())
        .collect();
    let commands: Vec<_> = wire
        .iter()
        .map(|item| item["type"].as_str().unwrap())
        .collect();
    assert_eq!(
        commands,
        [
            "prompt",
            "extension_ui_response",
            "extension_ui_response",
            "extension_ui_response",
            "extension_ui_response",
            "extension_ui_response",
            "extension_ui_response",
            "clear_queue",
            "abort"
        ]
    );
    let replies: Vec<_> = wire[1..7]
        .iter()
        .map(|item| item["id"].as_str().unwrap())
        .collect();
    assert_eq!(
        replies,
        [
            "reply-0", "reply-1", "reply-2", "reply-3", "reply-4", "reply-5"
        ]
    );
}

#[test]
fn pi_port_awaiting_user_is_intermediate_and_one_way_reply_can_only_be_written() {
    let mut fixture = Fixture::new();
    let sender = fixture.incoming();
    let ticket = fixture.client.enqueue_pi(prompt(), None).unwrap();
    fixture.client.poll_pi();
    let mut awaiting = response(&fixture.client, ticket, true);
    awaiting["data"]["disposition"] = json!("awaiting_user");
    sender.send(Incoming::Record(awaiting)).unwrap();
    let evidence = phases(fixture.client.poll_pi());
    assert!(
        evidence
            .iter()
            .any(|phase| matches!(phase, PiReceiptPhase::AwaitingUser(_)))
    );
    assert!(!evidence.iter().any(|phase| matches!(
        phase,
        PiReceiptPhase::Accepted(_) | PiReceiptPhase::Outcome(_)
    )));
    assert!(
        fixture
            .client
            .port
            .as_ref()
            .unwrap()
            .operations
            .contains_key(&ticket.sequence)
    );
    let reply = fixture
        .client
        .enqueue_pi(
            PiOperation::DialogReply(
                json!({"type":"extension_ui_response","id":"native-dialog","value":"answer"}),
            ),
            None,
        )
        .unwrap();
    // Even a forged response must not turn this one-way message into Accepted.
    sender
        .send(Incoming::Record(response(&fixture.client, reply, true)))
        .unwrap();
    let deadline = Instant::now() + Duration::from_secs(2);
    let mut written = false;
    while !written {
        for item in fixture.client.poll_pi() {
            if let PiPortItem::Receipt(receipt) = item
                && receipt.ticket == reply
            {
                assert_eq!(receipt.phase, PiReceiptPhase::Written);
                written = true;
            }
        }
        assert!(Instant::now() < deadline);
        thread::sleep(Duration::from_millis(5));
    }
    sender
        .send(Incoming::Record(response(&fixture.client, ticket, true)))
        .unwrap();
    assert!(
        phases(fixture.client.poll_pi())
            .iter()
            .any(|phase| matches!(phase, PiReceiptPhase::Outcome(_)))
    );
}

#[test]
fn pi_port_retains_a_legal_maximum_image_and_reserves_its_actual_encoded_allocation() {
    let mut fixture = Fixture::new();
    let image_path = fixture.root.join("synthetic.jpg");
    let mut bytes = vec![0; 20 * 1024 * 1024];
    bytes[..3].copy_from_slice(&[0xff, 0xd8, 0xff]);
    std::fs::write(&image_path, &bytes).unwrap();
    drop(bytes);
    let image = crate::TurnImage::read(&image_path, crate::ImageType::Jpeg).unwrap();
    let ticket = fixture
        .client
        .enqueue_pi(
            PiOperation::Prompt(crate::TurnInput {
                text: String::new(),
                effort: None,
                permission_mode: None,
                images: vec![image],
            }),
            None,
        )
        .unwrap();
    let port = fixture.client.port.as_ref().unwrap();
    let encoded = port.operations[&ticket.sequence].bytes.as_ref().unwrap();
    assert!(
        encoded.len() > 26 * 1024 * 1024,
        "full image is encoded, never truncated to the transport's 8 MiB queue budget"
    );
    assert_eq!(port.bytes, encoded.capacity());
    assert!(port.bytes <= PAYLOAD_BYTES);
    assert!(fixture.client.abandon_queued_pi(ticket));
}

#[test]
fn pi_port_rejected_thinking_never_dispatches_the_reserved_dependent_prompt() {
    let mut fixture = Fixture::new();
    let sender = fixture.incoming();
    let thinking = fixture
        .client
        .enqueue_pi(PiOperation::ThinkingLevel("high".into()), None)
        .unwrap();
    let PiOperation::Prompt(input) = prompt() else {
        unreachable!()
    };
    let dependent = fixture
        .client
        .enqueue_pi_after_thinking(input, thinking, None)
        .unwrap();
    assert!(
        fixture.client.port.as_ref().unwrap().bytes
            > fixture.client.port.as_ref().unwrap().operations[&thinking.sequence].size
    );
    sender
        .send(Incoming::Record(response(&fixture.client, thinking, false)))
        .unwrap();
    let items = fixture.client.poll_pi();
    let evidence = items
        .iter()
        .filter_map(|item| match item {
            PiPortItem::Receipt(receipt) if receipt.ticket == dependent => Some(&receipt.phase),
            _ => None,
        })
        .collect::<Vec<_>>();
    assert_eq!(evidence, vec![&PiReceiptPhase::NotSent]);
    assert!(!fixture.client.pi_cleanup_required());
    assert!(
        !fixture
            .client
            .port
            .as_ref()
            .unwrap()
            .operations
            .contains_key(&dependent.sequence)
    );
}

#[test]
fn pi_port_disconnect_reports_indeterminate_without_waiting_for_a_prompt_result_deadline() {
    let mut fixture = Fixture::new();
    let sender = fixture.incoming();
    let ticket = fixture.client.enqueue_pi(prompt(), None).unwrap();
    let deadline = Instant::now() + Duration::from_secs(2);
    loop {
        if phases(fixture.client.poll_pi()).contains(&PiReceiptPhase::Written) {
            break;
        }
        assert!(Instant::now() < deadline);
        thread::sleep(Duration::from_millis(5));
    }
    assert!(
        fixture.client.port.as_ref().unwrap().operations[&ticket.sequence]
            .response_deadline
            .is_none()
    );
    sender.send(Incoming::Closed).unwrap();
    assert!(
        phases(fixture.client.poll_pi())
            .iter()
            .any(|phase| matches!(phase, PiReceiptPhase::Indeterminate(_)))
    );
    assert!(fixture.client.pi_cleanup_required());
    assert!(
        fixture
            .client
            .port
            .as_ref()
            .unwrap()
            .operations
            .contains_key(&ticket.sequence)
    );
}

#[test]
fn pi_port_session_generation_discards_unsent_old_leaf_input_but_keeps_old_wire_outcomes() {
    let mut fixture = Fixture::new();
    let sender = fixture.incoming();
    let old = fixture
        .client
        .enqueue_pi(PiOperation::GetState, None)
        .unwrap();
    fixture.client.poll_pi();
    let queued = fixture.client.enqueue_pi(prompt(), None).unwrap();
    let next = fixture.client.advance_pi_session_generation().unwrap();
    assert_eq!(next, old.session_generation + 1);
    assert!(!fixture.client.abandon_queued_pi(old));
    sender
        .send(Incoming::Record(response(&fixture.client, old, true)))
        .unwrap();
    let items = fixture.client.poll_pi();
    assert!(items.iter().any(|item|matches!(item,PiPortItem::Receipt(receipt) if receipt.ticket==queued && receipt.phase==PiReceiptPhase::NotSent)));
    assert!(items.iter().any(|item|matches!(item,PiPortItem::Receipt(receipt) if receipt.ticket==old && matches!(receipt.phase,PiReceiptPhase::Accepted(_)))));
    let current = fixture.client.enqueue_pi(prompt(), None).unwrap();
    assert_eq!(current.generation, old.generation);
    assert_eq!(current.session_generation, next);
    assert!(current.sequence > queued.sequence);
}

#[test]
fn pi_review_full_slots_reject_before_input_encoding() {
    let mut fixture = Fixture::new();
    for _ in 0..OPERATIONS - RESERVED {
        fixture.client.enqueue_pi(prompt(), None).unwrap();
    }
    INPUT_ENCODINGS.with(|count| count.set(0));
    assert!(fixture.client.enqueue_pi(prompt(), None).is_err());
    assert_eq!(
        INPUT_ENCODINGS.with(|count| count.get()),
        0,
        "slot rejection must precede encoding"
    );
}

#[test]
fn pi_review_aggregate_budget_rejects_before_input_encoding() {
    let mut fixture = Fixture::new();
    fixture.client.enable_port().unwrap();
    fixture.client.port.as_mut().unwrap().bytes = PAYLOAD_BYTES - 16;
    INPUT_ENCODINGS.with(|count| count.set(0));
    assert!(
        fixture
            .client
            .enqueue_pi(
                PiOperation::Prompt(crate::TurnInput {
                    text: "\u{0}".repeat(1024),
                    effort: None,
                    permission_mode: None,
                    images: vec![]
                }),
                None
            )
            .is_err()
    );
    assert_eq!(
        INPUT_ENCODINGS.with(|count| count.get()),
        0,
        "JSON escaping preflight must precede allocation"
    );
}

#[test]
fn pi_review_encoder_counts_escaping_base64_and_never_grows_output() {
    let text = "\0\n\t\"\\🌙".repeat(37);
    let operation = PiOperation::Prompt(crate::TurnInput {
        text: text.clone(),
        effort: None,
        permission_mode: None,
        images: vec![],
    });
    let encoded = encoding::encode(&operation, "wire-id", 8192).unwrap();
    let exact = encoded.len();
    assert!(exact > text.len());
    assert_eq!(
        serde_json::from_slice::<Value>(&encoded).unwrap()["message"],
        text
    );
    INPUT_ENCODINGS.with(|count| count.set(0));
    assert!(encoding::encode(&operation, "wire-id", exact - 1).is_err());
    assert_eq!(INPUT_ENCODINGS.with(|count| count.get()), 0);
    let exact_output = encoding::encode(&operation, "wire-id", exact).unwrap();
    assert_eq!(exact_output, encoded);
    assert_eq!(
        exact_output.capacity(),
        exact,
        "fixed-slice serialization cannot grow the Vec"
    );
    // Background payload also streams JSON -> base64 -> escaped wire JSON.
    let value = json!({"operation":"task_output","text":text});
    let background = encoding::encode(
        &PiOperation::Background(value.clone()),
        "background-id",
        8192,
    )
    .unwrap();
    let wire: Value = serde_json::from_slice(&background).unwrap();
    let base64 = wire["message"]
        .as_str()
        .unwrap()
        .strip_prefix("/velune-background-work ")
        .unwrap();
    assert_eq!(
        serde_json::from_slice::<Value>(&URL_SAFE_NO_PAD.decode(base64).unwrap()).unwrap(),
        value
    );
}

#[test]
fn pi_review_aggregate_images_reject_before_any_base64_allocation() {
    let fixture = Fixture::new();
    let path = fixture.root.join("aggregate.jpg");
    let mut data = vec![0; 1024 * 1024];
    data[..3].copy_from_slice(&[0xff, 0xd8, 0xff]);
    std::fs::write(&path, data).unwrap();
    let image = crate::TurnImage::read(&path, crate::ImageType::Jpeg).unwrap();
    let operation = PiOperation::Prompt(crate::TurnInput {
        text: String::new(),
        effort: None,
        permission_mode: None,
        images: vec![image.clone(), image.clone(), image],
    });
    INPUT_ENCODINGS.with(|count| count.set(0));
    assert!(
        encoding::encode(&operation, "aggregate", 4 * 1024 * 1024).is_err(),
        "base64 expansion exceeds the aggregate budget before a wire Vec exists"
    );
    assert_eq!(INPUT_ENCODINGS.with(|count| count.get()), 0);
    let encoded = encoding::encode(&operation, "aggregate", 5 * 1024 * 1024).unwrap();
    assert_eq!(encoded.capacity(), encoded.len());
    assert_eq!(
        serde_json::from_slice::<Value>(&encoded).unwrap()["images"]
            .as_array()
            .unwrap()
            .len(),
        3
    );
}

#[test]
fn pi_review_rejected_transition_unblocks_without_advancing_generation() {
    let mut fixture = Fixture::new();
    let sender = fixture.incoming();
    let branch = fixture
        .client
        .enqueue_pi(PiOperation::Fork("entry".into()), None)
        .unwrap();
    let queued = fixture.client.enqueue_pi(prompt(), None).unwrap();
    let reply = fixture.client.enqueue_pi(PiOperation::Abort, None).unwrap();
    let deadline = Instant::now() + Duration::from_secs(2);
    while fixture.client.port.as_ref().unwrap().operations[&branch.sequence]
        .bytes
        .is_some()
        || fixture.client.port.as_ref().unwrap().operations[&reply.sequence]
            .bytes
            .is_some()
    {
        fixture.client.poll_pi();
        assert!(Instant::now() < deadline);
        thread::sleep(Duration::from_millis(5));
    }
    assert!(
        fixture.client.port.as_ref().unwrap().operations[&queued.sequence]
            .bytes
            .is_some()
    );
    sender
        .send(Incoming::Record(response(&fixture.client, branch, false)))
        .unwrap();
    assert!(
        phases(fixture.client.poll_pi())
            .iter()
            .any(|phase| matches!(phase, PiReceiptPhase::Rejected(_)))
    );
    assert_eq!(
        fixture.client.pi_session_generation(),
        Some(branch.session_generation)
    );
    while fixture.client.port.as_ref().unwrap().operations[&queued.sequence]
        .bytes
        .is_some()
    {
        fixture.client.poll_pi();
        assert!(Instant::now() < deadline);
        thread::sleep(Duration::from_millis(5));
    }
    assert!(!fixture.client.pi_cleanup_required());
}

#[test]
fn pi_review_terminal_latches_deadline_crossed_after_an_earlier_batch_record() {
    let mut fixture = Fixture::new();
    let _sender = fixture.incoming();
    let ticket = fixture
        .client
        .enqueue_pi(prompt(), Some(Instant::now() + Duration::from_secs(60)))
        .unwrap();
    fixture.client.poll_pi();
    let terminal = response(&fixture.client, ticket, true);
    let port = fixture.client.port.as_mut().unwrap();
    port.operations
        .get_mut(&ticket.sequence)
        .unwrap()
        .response_deadline = Some(Instant::now() + Duration::from_millis(10));
    let mut output = Vec::new();
    port.observe_record(json!({"type":"agent_start"}), &mut output);
    thread::sleep(Duration::from_millis(20)); // scheduler pause between records in one owner batch
    port.observe_record(terminal, &mut output);
    let phases = phases(output);
    let failure = phases
        .iter()
        .position(|phase| matches!(phase, PiReceiptPhase::Indeterminate(_)))
        .unwrap();
    let outcome = phases
        .iter()
        .position(|phase| matches!(phase, PiReceiptPhase::Outcome(_)))
        .unwrap();
    assert!(failure < outcome);
    assert!(fixture.client.pi_cleanup_required());
}

#[test]
fn pi_integration_timely_physical_write_precedes_owner_deadline_latch() {
    let mut fixture = Fixture::new();
    let ticket = fixture.client.enqueue_pi(prompt(), None).unwrap();
    let d = Instant::now() - Duration::from_millis(1);
    let (sender, write) = WriteReceipt::test_pending(d);
    let op = fixture
        .client
        .port
        .as_mut()
        .unwrap()
        .operations
        .get_mut(&ticket.sequence)
        .unwrap();
    op.bytes = None;
    op.write = Some(write);
    op.write_deadline = d;
    sender
        .send(WriteOutcome::Written(crate::WriteCompletion {
            started_at: d - Duration::from_millis(2),
            completed_at: d - Duration::from_millis(1),
            deadline: d,
        }))
        .unwrap();
    let items = fixture.client.poll_pi();
    assert!(!fixture.client.pi_cleanup_required());
    assert!(items.iter().any(|i| matches!(i, PiPortItem::Receipt(r) if r.ticket == ticket && matches!(r.physical_write, Some(crate::WriteFact::Written(c)) if c.met_deadline()))));
}

#[test]
fn pi_integration_late_physical_write_survives_observation_timeout() {
    let mut fixture = Fixture::new();
    let ticket = fixture.client.enqueue_pi(prompt(), None).unwrap();
    let d = Instant::now() - Duration::from_millis(1);
    let (sender, write) = WriteReceipt::test_pending(d);
    let op = fixture
        .client
        .port
        .as_mut()
        .unwrap()
        .operations
        .get_mut(&ticket.sequence)
        .unwrap();
    op.bytes = None;
    op.write = Some(write);
    op.write_deadline = d;
    fixture.client.poll_pi();
    assert!(fixture.client.pi_cleanup_required());
    sender
        .send(WriteOutcome::Written(crate::WriteCompletion {
            started_at: d - Duration::from_millis(1),
            completed_at: d + Duration::from_millis(1),
            deadline: d,
        }))
        .unwrap();
    let items = fixture.client.poll_pi();
    assert!(items.iter().any(|i| matches!(i, PiPortItem::Receipt(r) if r.ticket == ticket && matches!(r.physical_write, Some(crate::WriteFact::Written(c)) if !c.met_deadline()))));
    assert!(fixture.client.pi_cleanup_required());
}

#[test]
fn pi_integration_reserved_reply_admits_while_ordinary_receipt_is_pending() {
    let mut fixture = Fixture::new();
    let ticket = fixture.client.enqueue_pi(prompt(), None).unwrap();
    let d = Instant::now() + Duration::from_secs(30);
    let (_sender, write) = WriteReceipt::test_pending(d);
    let op = fixture
        .client
        .port
        .as_mut()
        .unwrap()
        .operations
        .get_mut(&ticket.sequence)
        .unwrap();
    op.bytes = None;
    op.write = Some(write);
    op.write_deadline = d;
    let reply = fixture
        .client
        .enqueue_pi(
            PiOperation::DialogReply(
                json!({"type":"extension_ui_response","id":"permission","cancelled":true}),
            ),
            None,
        )
        .unwrap();
    fixture.client.poll_pi();
    assert!(
        fixture
            .client
            .port
            .as_ref()
            .unwrap()
            .operations
            .get(&reply.sequence)
            .is_none_or(|op| op.bytes.is_none()),
        "protocol frame must reach the same physical FIFO without waiting for an ordinary receipt"
    );
}

#[test]
fn pi_integration_old_scope_physical_deadline_cannot_fail_new_owner() {
    let mut fixture = Fixture::new();
    let old = fixture.client.enqueue_pi(prompt(), None).unwrap();
    let d = Instant::now() - Duration::from_millis(1);
    let (sender, write) = WriteReceipt::test_pending(d);
    let op = fixture
        .client
        .port
        .as_mut()
        .unwrap()
        .operations
        .get_mut(&old.sequence)
        .unwrap();
    op.bytes = None;
    op.write = Some(write);
    op.write_deadline = d;
    fixture.client.advance_pi_session_generation().unwrap();
    let current = fixture.client.enqueue_pi(prompt(), None).unwrap();
    fixture.client.poll_pi();
    assert!(
        !fixture.client.pi_cleanup_required(),
        "old observation timeout is not the new scope's deadline"
    );
    sender
        .send(WriteOutcome::Written(crate::WriteCompletion {
            started_at: d - Duration::from_millis(1),
            completed_at: d + Duration::from_millis(1),
            deadline: d,
        }))
        .unwrap();
    let items = fixture.client.poll_pi();
    assert!(items.iter().any(|item| matches!(item, PiPortItem::Receipt(r) if r.ticket == old && matches!(r.physical_write, Some(crate::WriteFact::Written(c)) if !c.met_deadline()))));
    assert!(fixture.client.pi_owns_ticket(current));
    assert!(
        !fixture.client.pi_cleanup_required(),
        "late old-scope Written must remain evidence without poisoning the current owner"
    );
}
