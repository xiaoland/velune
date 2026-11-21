use super::*;

struct Directory(std::path::PathBuf);
impl Drop for Directory {
    fn drop(&mut self) {
        std::fs::remove_dir_all(&self.0).unwrap();
    }
}

#[test]
fn physical_reply_and_explicit_cancel_reserve_cross_rpc_barrier_and_poll() {
    let root = Directory(std::env::temp_dir().join(format!(
                "velune-rpc-admission-{}-{}",
                std::process::id(),
                std::time::SystemTime::now()
                    .duration_since(std::time::UNIX_EPOCH)
                    .unwrap()
                    .as_nanos()
            )));
    std::fs::create_dir(&root.0).unwrap();
    let mut command = crate::process::command("python3");
    command.args(["-u", "-c", r#"
import json, pathlib, sys, time
root = pathlib.Path(sys.argv[1])
print(json.dumps({'method':'synthetic/notice'}), flush=True)
while not (root / 'release').exists(): time.sleep(.005)
for line in sys.stdin:
    record = json.loads(line)
    print(json.dumps({'method':'synthetic/wire', 'params': {'method':record.get('method'), 'id':record.get('id'), 'reply':'result' in record, 'rejection':'error' in record}}), flush=True)
"#]).arg(&root.0);
    let mut rpc =
        RpcClient::spawn_at(command, &crate::execution::ExecutionTarget::Local, None).unwrap();
    rpc.defer_cleanup();
    let deadline = Instant::now() + Duration::from_secs(5);
    let (id, prompt) = rpc
        .enqueue_request_receipt(
            "synthetic/prompt",
            &json!({"text":"x".repeat(9 * 1024 * 1024)}),
            deadline,
        )
        .unwrap();
    let reply = rpc
        .enqueue_reply_receipt(&json!(900), &json!({}), deadline)
        .unwrap();
    let reject = rpc
        .enqueue_rejection_receipt(&json!(901), "unsupported", deadline)
        .unwrap();
    let cancel = rpc
        .enqueue_notification_receipt_with_class(
            "synthetic/cancel",
            &json!({}),
            WriteClass::ProtocolReplyCancel,
            deadline,
        )
        .unwrap();
    let (_, interrupt) = rpc
        .enqueue_request_receipt_with_class(
            "synthetic/interrupt",
            &json!({}),
            WriteClass::ProtocolReplyCancel,
            deadline,
        )
        .unwrap();
    assert!(
        rpc.enqueue_notification_receipt("synthetic/not-control", &json!({}), deadline)
            .is_err()
    );
    // Control reserve is bounded too. Local rejection preserves the frame,
    // deadline and a healthy connection; it never clears a failure barrier.
    let frame = OwnedFrame::new(b"{}\n".to_vec(), 1024);
    let pointer = frame.bytes.as_ptr();
    let rejected = rpc
        .try_enqueue_owned_frame(frame, WriteClass::ProtocolReplyCancel, deadline)
        .err()
        .unwrap();
    assert_eq!(rejected.error.kind(), std::io::ErrorKind::WouldBlock);
    assert_eq!(rejected.frame.bytes.as_ptr(), pointer);
    assert_eq!(rejected.deadline, deadline);
    assert!(rpc.input.is_some());
    let mut noticed = false;
    while !noticed && Instant::now() < deadline {
        noticed = rpc
            .poll()
            .unwrap()
            .iter()
            .any(|v| v["method"] == "synthetic/notice");
        thread::sleep(Duration::from_millis(2));
    }
    assert!(noticed);
    for receipt in [&prompt, &reply, &reject, &cancel, &interrupt] {
        assert!(receipt.try_complete().is_none());
    }
    // The same terminal barrier applies to ALL classes and synchronous helpers.
    rpc.failed = Some(Error::with_code("synthetic recorded terminal", "transport"));
    let rejected = rpc
        .try_enqueue_owned_frame(rejected.frame, rejected.class, rejected.deadline)
        .err()
        .unwrap();
    assert_eq!(rejected.error.kind(), std::io::ErrorKind::BrokenPipe);
    assert_eq!(rejected.frame.bytes.as_ptr(), pointer);
    assert!(rpc.respond(&json!(902), &json!({})).is_err());
    assert!(rpc.failed.is_some());
    assert!(rpc.check_failure().is_err());
    std::fs::write(root.0.join("release"), []).unwrap();
    let mut wire = Vec::new();
    while wire.len() < 5 && Instant::now() < deadline {
        wire.extend(rpc.poll().unwrap());
        thread::sleep(Duration::from_millis(2));
    }
    assert_eq!(wire.len(), 5);
    assert_eq!(wire[0]["params"]["id"], id);
    assert_eq!(wire[1]["params"]["reply"], true);
    assert_eq!(wire[2]["params"]["rejection"], true);
    assert_eq!(wire[3]["params"]["method"], "synthetic/cancel");
    assert_eq!(wire[4]["params"]["method"], "synthetic/interrupt");
    for receipt in [prompt, reply, reject, cancel, interrupt] {
        receipt.wait().unwrap();
    }
    rpc.shutdown();
}

#[test]
fn local_allocation_refusal_is_not_a_transport_failure() {
    let mut command = crate::process::command("python3");
    command.args(["-u", "-c", "import sys, json\nfor line in sys.stdin: print(json.dumps({'method':'synthetic/received'}), flush=True)"]);
    let mut rpc =
        RpcClient::spawn_at(command, &crate::execution::ExecutionTarget::Local, None).unwrap();
    let deadline = Instant::now() + Duration::from_secs(3);
    let frame = OwnedFrame::new(Vec::with_capacity(1024), 512);
    let pointer = frame.bytes.as_ptr();
    let rejected = rpc
        .try_enqueue_owned_frame(frame, WriteClass::Ordinary, deadline)
        .err()
        .unwrap();
    assert_eq!(rejected.error.kind(), std::io::ErrorKind::InvalidInput);
    assert_eq!(rejected.frame.bytes.as_ptr(), pointer);
    assert!(rpc.failed.is_none());
    rpc.write(&json!({"synthetic":"after-budget-refusal"}))
        .unwrap();
    assert!(rpc.input.is_some());
    rpc.shutdown();
}

#[test]
fn queued_request_keeps_protocol_notifications_live_while_stdin_is_blocked() {
    let root = std::env::temp_dir().join(format!(
        "velune-rpc-write-gate-{}-{}",
        std::process::id(),
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_nanos()
    ));
    std::fs::create_dir(&root).unwrap();
    let mut command = crate::process::command("python3");
    command.args([
        "-c",
        r#"
import json, pathlib, sys, time
root = pathlib.Path(sys.argv[1])
print(json.dumps({'method':'synthetic/notice','params':{'sequence':1}}), flush=True)
deadline = time.monotonic() + 8
while not (root / 'release').exists():
    if time.monotonic() >= deadline: sys.exit(2)
    time.sleep(0.005)
for line in sys.stdin:
    record = json.loads(line)
    with (root / 'wire').open('a') as log: log.write(str(record['id']) + '\n')
    print(json.dumps({'id':record['id'],'result':{}}), flush=True)
"#,
    ]);
    command.arg(&root);
    let mut rpc =
        RpcClient::spawn_at(command, &crate::execution::ExecutionTarget::Local, None).unwrap();
    let deadline = Instant::now() + Duration::from_secs(5);
    let id = rpc
        .enqueue_request(
            "synthetic/request",
            &serde_json::json!({
                "payload": "x".repeat(2 * 1024 * 1024),
            }),
            deadline,
        )
        .unwrap();
    let mut observed = false;
    while Instant::now() < deadline {
        let records = rpc.poll().unwrap();
        if records
            .iter()
            .any(|record| record["method"] == "synthetic/notice")
        {
            observed = true;
            break;
        }
        thread::sleep(Duration::from_millis(5));
    }
    assert!(
        observed,
        "notification must arrive before stdin gate is released"
    );
    assert_eq!(
        rpc.writes.len(),
        1,
        "the blocked frame must still own its receipt"
    );
    assert!(!root.join("wire").exists());
    std::fs::write(root.join("release"), []).unwrap();
    let mut answered = false;
    while Instant::now() < deadline {
        let records = rpc.poll().unwrap();
        if records.iter().any(|record| record["id"] == id) {
            answered = true;
            break;
        }
        thread::sleep(Duration::from_millis(5));
    }
    rpc.shutdown();
    assert!(answered);
    assert_eq!(
        std::fs::read_to_string(root.join("wire")).unwrap(),
        format!("{id}\n"),
        "write completion must not replay a request"
    );
    std::fs::remove_dir_all(root).unwrap();
}
