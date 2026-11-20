use super::*;
use std::{
    io::{BufRead, BufReader},
    path::PathBuf,
    process::Stdio,
    time::Duration,
};

static NEXT_GATE: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(0);

struct Gate {
    child: crate::process_tree::ProcessTree,
    root: PathBuf,
    output: mpsc::Receiver<String>,
}
impl Gate {
    fn new() -> (Self, PipeWriter) {
        Self::with_limits(writer::WriteLimits::default())
    }
    fn with_limits(limits: writer::WriteLimits) -> (Self, PipeWriter) {
        let root = std::env::temp_dir().join(format!(
            "velune-transport-{}-{}-{}",
            NEXT_GATE.fetch_add(1, std::sync::atomic::Ordering::Relaxed),
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ));
        std::fs::create_dir(&root).unwrap();
        let mut command = crate::process::command("python3");
        command
            .arg("-u")
            .arg("-c")
            .arg(
                r#"
import sys, pathlib, time, json
root = pathlib.Path(sys.argv[1])
print('notification', flush=True)
while not (root / 'release').exists(): time.sleep(.005)
for line in sys.stdin.buffer:
    value = json.loads(line)
    print(json.dumps({'kind': value['kind'], 'size': len(value.get('text', ''))}), flush=True)
"#,
            )
            .arg(&root)
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::null());
        let mut child = crate::process_tree::ProcessTree::spawn(&mut command).unwrap();
        let writer = PipeWriter::with_limits(child.take_stdin().unwrap(), limits).unwrap();
        let mut output = BufReader::new(child.take_stdout().unwrap());
        let (tx, output_rx) = mpsc::channel();
        thread::spawn(move || {
            let mut line = String::new();
            while output.read_line(&mut line).unwrap_or(0) != 0 {
                if tx.send(line.trim().to_owned()).is_err() {
                    break;
                }
                line.clear();
            }
        });
        (
            Self {
                child,
                root,
                output: output_rx,
            },
            writer,
        )
    }
    fn line(&mut self) -> String {
        self.output
            .recv_timeout(Duration::from_secs(5))
            .expect("helper record before test deadline")
    }
    fn release(&self) {
        std::fs::write(self.root.join("release"), b"").unwrap();
    }
}
impl Drop for Gate {
    fn drop(&mut self) {
        self.child.stop();
        std::fs::remove_dir_all(&self.root).unwrap();
    }
}
fn large() -> Vec<u8> {
    let mut bytes = Vec::with_capacity(10 * 1024 * 1024);
    bytes.extend_from_slice(b"{\"kind\":\"prompt\",\"text\":\"");
    bytes.resize(bytes.len() + 9 * 1024 * 1024, b'x');
    bytes.extend_from_slice(b"\"}\n");
    bytes
}

#[test]
fn blocked_large_prompt_reserves_real_reply_admission() {
    let (mut gate, writer) = Gate::new();
    let deadline = Instant::now() + Duration::from_secs(5);
    let prompt = writer
        .try_enqueue(
            OwnedFrame::new(large(), 16 * 1024 * 1024),
            WriteClass::Ordinary,
            deadline,
        )
        .unwrap();
    assert_eq!(gate.line(), "notification");
    assert!(prompt.try_complete().is_none());
    let reply = writer
        .try_enqueue(
            OwnedFrame::new(b"{\"kind\":\"reply\"}\n".to_vec(), 1024),
            WriteClass::ProtocolReplyCancel,
            deadline,
        )
        .expect("reply must enter actual writer behind oversized prompt");
    let cancel = control(&writer, "cancel", deadline).unwrap();
    assert!(reply.try_complete().is_none());
    assert!(cancel.try_complete().is_none());
    {
        let usage = writer.usage.lock().unwrap();
        assert_eq!((usage.ordinary_count, usage.control_count), (1, 2));
        assert_eq!(usage.ordinary_bytes, 10 * 1024 * 1024);
    }
    gate.release();
    assert!(gate.line().contains("prompt"));
    assert!(gate.line().contains("reply"));
    assert!(gate.line().contains("cancel"));
    prompt.wait().unwrap();
    reply.wait().unwrap();
    cancel.wait().unwrap();
    assert_eq!(writer.usage.lock().unwrap().ordinary_count, 0);
    assert_eq!(writer.usage.lock().unwrap().control_count, 0);
    assert!(gate.output.recv_timeout(Duration::from_millis(40)).is_err());
}

fn control(
    writer: &PipeWriter,
    kind: &str,
    deadline: Instant,
) -> Result<WriteReceipt, NotAdmitted> {
    let bytes = format!("{{\"kind\":\"{kind}\"}}\n").into_bytes();
    writer.try_enqueue(
        OwnedFrame::new(bytes, 1024),
        WriteClass::ProtocolReplyCancel,
        deadline,
    )
}
fn assert_returned(
    rejected: NotAdmitted,
    pointer: *const u8,
    capacity: usize,
    deadline: Instant,
    class: WriteClass,
) -> OwnedFrame {
    assert_eq!(rejected.frame.bytes.as_ptr(), pointer);
    assert_eq!(rejected.frame.bytes.capacity(), capacity);
    assert_eq!(rejected.deadline, deadline);
    assert_eq!(rejected.class, class);
    rejected.frame
}

#[test]
fn blocked_lease_survives_receipt_timeout_and_drop_and_queued_controls_expire() {
    let (mut gate, writer) = Gate::new();
    let deadline = Instant::now() + Duration::from_millis(150);
    let prompt = writer
        .try_enqueue(
            OwnedFrame::new(large(), 16 * 1024 * 1024),
            WriteClass::Ordinary,
            deadline,
        )
        .unwrap();
    assert_eq!(gate.line(), "notification");
    let reply = control(&writer, "expired-reply", deadline).unwrap();
    let cancel = control(&writer, "expired-cancel", deadline).unwrap();
    thread::sleep(deadline.saturating_duration_since(Instant::now()) + Duration::from_millis(20));
    assert!(matches!(
        prompt.try_outcome(),
        Some(WriteOutcome::ObservationTimeout { .. })
    ));
    assert!(matches!(
        reply.try_outcome(),
        Some(WriteOutcome::ObservationTimeout { .. })
    ));
    drop(prompt);
    assert_eq!(writer.usage.lock().unwrap().ordinary_count, 1);
    assert_eq!(writer.usage.lock().unwrap().control_count, 2);
    assert_eq!(
        writer.usage.lock().unwrap().ordinary_bytes,
        10 * 1024 * 1024
    );
    gate.release();
    assert!(gate.line().contains("prompt"));
    // Read the actual worker outcomes, not the observation-time timeout.
    let until = Instant::now() + Duration::from_secs(3);
    loop {
        if writer.usage.lock().unwrap().control_count == 0 {
            break;
        }
        assert!(Instant::now() < until);
        thread::sleep(Duration::from_millis(2));
    }
    assert!(matches!(
        reply.try_outcome(),
        Some(WriteOutcome::NotWritten(_))
    ));
    assert!(matches!(
        cancel.try_outcome(),
        Some(WriteOutcome::NotWritten(_))
    ));
    assert_eq!(writer.usage.lock().unwrap().ordinary_bytes, 0);
    assert!(gate.output.recv_timeout(Duration::from_millis(40)).is_err());
    // A skipped expired frame does not prevent subsequent valid FIFO writes.
    control(
        &writer,
        "after-expiry",
        Instant::now() + Duration::from_secs(3),
    )
    .unwrap()
    .wait()
    .unwrap();
    assert!(gate.line().contains("after-expiry"));
}

#[test]
fn ordinary_flood_cannot_consume_reserved_count_and_control_full_returns_owned_frame() {
    let limits = writer::WriteLimits {
        ordinary_count: 3,
        control_count: 2,
        total_count: 5,
        ordinary_bytes: 12 * 1024 * 1024,
        control_bytes: 128,
        ..Default::default()
    };
    let (mut gate, writer) = Gate::with_limits(limits);
    let deadline = Instant::now() + Duration::from_secs(5);
    let prompt = writer
        .try_enqueue(
            OwnedFrame::new(large(), 16 * 1024 * 1024),
            WriteClass::Ordinary,
            deadline,
        )
        .unwrap();
    assert_eq!(gate.line(), "notification");
    let a = writer
        .enqueue(b"{\"kind\":\"ordinary-a\"}\n".to_vec(), deadline)
        .unwrap();
    let b = writer
        .enqueue(b"{\"kind\":\"ordinary-b\"}\n".to_vec(), deadline)
        .unwrap();
    assert_eq!(
        writer
            .enqueue(b"{}\n".to_vec(), deadline)
            .err()
            .unwrap()
            .kind(),
        io::ErrorKind::WouldBlock
    );
    let reply = control(&writer, "reply", deadline).unwrap();
    let cancel = control(&writer, "cancel", deadline).unwrap();
    let frame = OwnedFrame::new(Vec::with_capacity(64), 1024);
    let pointer = frame.bytes.as_ptr();
    let rejection = writer
        .try_enqueue(frame, WriteClass::ProtocolReplyCancel, deadline)
        .err()
        .unwrap();
    assert_eq!(rejection.error.kind(), io::ErrorKind::WouldBlock);
    let frame = assert_returned(
        rejection,
        pointer,
        64,
        deadline,
        WriteClass::ProtocolReplyCancel,
    );
    assert_eq!(frame.allocation_limit, 1024);
    {
        let usage = writer.usage.lock().unwrap();
        assert_eq!(usage.ordinary_count + usage.control_count, 5);
        assert!(usage.ordinary_bytes <= limits.ordinary_bytes);
        assert!(usage.control_bytes <= limits.control_bytes);
    }
    gate.release();
    for expected in ["prompt", "ordinary-a", "ordinary-b", "reply", "cancel"] {
        assert!(gate.line().contains(expected));
    }
    for receipt in [prompt, a, b, reply, cancel] {
        receipt.wait().unwrap();
    }
    assert_eq!(writer.usage.lock().unwrap().control_bytes, 0);
}

#[test]
fn oversized_exclusivity_capacity_budget_and_control_byte_limit_are_real() {
    let (mut gate, writer) = Gate::new();
    let deadline = Instant::now() + Duration::from_secs(5);
    let prompt = writer
        .try_enqueue(
            OwnedFrame::new(large(), 16 * 1024 * 1024),
            WriteClass::Ordinary,
            deadline,
        )
        .unwrap();
    assert_eq!(gate.line(), "notification");
    // Even an empty ordinary allocation cannot bypass oversized exclusivity.
    assert!(writer.enqueue(Vec::new(), deadline).is_err());
    let frame = OwnedFrame::new(Vec::with_capacity(1024 * 1024 + 1), 2 * 1024 * 1024);
    let pointer = frame.bytes.as_ptr();
    let capacity = frame.bytes.capacity();
    let rejected = writer
        .try_enqueue(frame, WriteClass::ProtocolReplyCancel, deadline)
        .err()
        .unwrap();
    assert_eq!(rejected.error.kind(), io::ErrorKind::WouldBlock);
    assert_returned(
        rejected,
        pointer,
        capacity,
        deadline,
        WriteClass::ProtocolReplyCancel,
    );
    assert_eq!(writer.usage.lock().unwrap().control_count, 0);
    gate.release();
    prompt.wait().unwrap();
}

#[test]
fn finite_allocation_and_expired_admission_return_original_buffer_without_dispatch() {
    let (mut gate, writer) = Gate::new();
    assert_eq!(gate.line(), "notification");
    for (capacity, budget, deadline, expected) in [
        (
            1024,
            512,
            Instant::now() + Duration::from_secs(5),
            io::ErrorKind::InvalidInput,
        ),
        (
            0,
            writer::LEGACY_ALLOCATION_LIMIT + 1,
            Instant::now() + Duration::from_secs(5),
            io::ErrorKind::InvalidInput,
        ),
        (
            128,
            1024,
            Instant::now() - Duration::from_secs(1),
            io::ErrorKind::TimedOut,
        ),
    ] {
        let frame = OwnedFrame::new(Vec::with_capacity(capacity), budget);
        let pointer = frame.bytes.as_ptr();
        let rejection = writer
            .try_enqueue(frame, WriteClass::Ordinary, deadline)
            .err()
            .unwrap();
        assert_eq!(rejection.error.kind(), expected);
        let frame = assert_returned(rejection, pointer, capacity, deadline, WriteClass::Ordinary);
        assert_eq!(frame.allocation_limit, budget);
    }
    assert_eq!(writer.usage.lock().unwrap().ordinary_count, 0);
    assert_eq!(writer.usage.lock().unwrap().ordinary_bytes, 0);
    gate.release();
    assert!(gate.output.recv_timeout(Duration::from_millis(30)).is_err());
}

#[test]
fn writer_failure_releases_actual_leases_and_settles_queued_jobs_without_writing() {
    let (mut gate, writer) = Gate::new();
    let deadline = Instant::now() + Duration::from_secs(5);
    let prompt = writer
        .try_enqueue(
            OwnedFrame::new(large(), 16 * 1024 * 1024),
            WriteClass::Ordinary,
            deadline,
        )
        .unwrap();
    assert_eq!(gate.line(), "notification");
    let reply = control(&writer, "reply", deadline).unwrap();
    gate.child.stop();
    assert!(prompt.wait().is_err());
    let until = Instant::now() + Duration::from_secs(3);
    loop {
        if let Some(outcome) = reply.try_outcome() {
            assert!(matches!(outcome, WriteOutcome::NotWritten(_)));
            break;
        }
        assert!(Instant::now() < until);
        thread::sleep(Duration::from_millis(2));
    }
    let usage = writer.usage.lock().unwrap();
    assert_eq!(usage.ordinary_count + usage.control_count, 0);
    assert_eq!(usage.ordinary_bytes + usage.control_bytes, 0);
    drop(usage);
    assert_eq!(
        control(&writer, "later", deadline)
            .err()
            .unwrap()
            .error
            .kind(),
        io::ErrorKind::BrokenPipe
    );
}

#[test]
fn timely_flush_is_timely_even_when_harvested_after_deadline() {
    let (mut gate, writer) = Gate::new();
    assert_eq!(gate.line(), "notification");
    let deadline = Instant::now() + Duration::from_millis(500);
    let receipt = control(&writer, "timely", deadline).unwrap();
    gate.release();
    assert!(gate.line().contains("timely"));
    assert!(Instant::now() < deadline);
    thread::sleep(deadline.saturating_duration_since(Instant::now()) + Duration::from_millis(20));
    let Some(WriteOutcome::Written(completion)) = receipt.try_outcome() else {
        panic!("actual Written must survive delayed harvesting");
    };
    assert_eq!(completion.deadline, deadline);
    assert!(completion.met_deadline());
    assert!(completion.started_at <= completion.completed_at);
    control(
        &writer,
        "future-control",
        Instant::now() + Duration::from_secs(3),
    )
    .unwrap()
    .wait()
    .unwrap();
}

#[test]
fn blocked_write_retains_late_written_fact_after_observation_timeout() {
    let (mut gate, writer) = Gate::new();
    let deadline = Instant::now() + Duration::from_millis(500);
    let receipt = writer
        .try_enqueue(
            OwnedFrame::new(large(), 16 * 1024 * 1024),
            WriteClass::Ordinary,
            deadline,
        )
        .unwrap();
    assert_eq!(gate.line(), "notification");
    thread::sleep(deadline.saturating_duration_since(Instant::now()) + Duration::from_millis(20));
    assert!(matches!(
        receipt.try_outcome(),
        Some(WriteOutcome::ObservationTimeout { .. })
    ));
    assert_eq!(writer.usage.lock().unwrap().ordinary_count, 1);
    gate.release();
    assert!(gate.line().contains("prompt"));
    let until = Instant::now() + Duration::from_secs(3);
    loop {
        match receipt.try_outcome() {
            Some(WriteOutcome::Written(completion)) => {
                assert!(completion.started_at < deadline);
                assert!(completion.completed_at >= deadline);
                assert!(!completion.met_deadline());
                break;
            }
            Some(WriteOutcome::ObservationTimeout { .. }) | None => {}
            outcome => panic!("late flush must remain real Written: {outcome:?}"),
        }
        assert!(Instant::now() < until);
        thread::sleep(Duration::from_millis(2));
    }
}

struct MustNotSerialize;
impl serde::Serialize for MustNotSerialize {
    fn serialize<S: serde::Serializer>(&self, _: S) -> Result<S::Ok, S::Error> {
        panic!("slot preflight must precede encoding");
    }
}

#[test]
fn bounded_codec_checks_slot_and_deadline_before_touching_source() {
    let (mut gate, writer) = Gate::new();
    let deadline = Instant::now() + Duration::from_secs(5);
    let prompt = writer
        .try_enqueue(
            OwnedFrame::new(large(), 16 * 1024 * 1024),
            WriteClass::Ordinary,
            deadline,
        )
        .unwrap();
    assert_eq!(gate.line(), "notification");
    assert_eq!(
        writer
            .encode_json(&MustNotSerialize, WriteClass::Ordinary, 1024, deadline)
            .unwrap_err()
            .kind(),
        io::ErrorKind::WouldBlock
    );
    assert_eq!(
        writer
            .encode_json(
                &MustNotSerialize,
                WriteClass::ProtocolReplyCancel,
                1024,
                Instant::now()
            )
            .unwrap_err()
            .kind(),
        io::ErrorKind::TimedOut
    );
    let encoded = writer
        .encode_json(
            &serde_json::json!({"kind":"reply"}),
            WriteClass::ProtocolReplyCancel,
            1024,
            deadline,
        )
        .unwrap();
    assert!(encoded.bytes.capacity() <= 1024);
    let reply = writer
        .try_enqueue(encoded, WriteClass::ProtocolReplyCancel, deadline)
        .unwrap();
    gate.release();
    prompt.wait().unwrap();
    reply.wait().unwrap();
}

fn compatibility_then_physical_terminal(receipt: &WriteReceipt) -> (io::Result<()>, WriteOutcome) {
    let until = Instant::now() + Duration::from_secs(3);
    loop {
        let compatibility = receipt.try_complete();
        let outcome = receipt.try_outcome();
        match (compatibility, outcome) {
            (
                Some(result),
                Some(outcome @ (WriteOutcome::Written(_) | WriteOutcome::NotWritten(_))),
            ) => {
                return (result, outcome);
            }
            (_, Some(WriteOutcome::Indeterminate(error))) => {
                panic!("known physical outcome lost after compatibility read: {error}");
            }
            _ => {}
        }
        assert!(
            Instant::now() < until,
            "physical terminal outcome did not arrive"
        );
        thread::sleep(Duration::from_millis(2));
    }
}

fn assert_same_written(receipt: &WriteReceipt, expected: WriteCompletion) {
    for _ in 0..2 {
        let Some(WriteOutcome::Written(actual)) = receipt.try_outcome() else {
            panic!("compatibility observation must not consume real Written");
        };
        assert_eq!(actual.started_at, expected.started_at);
        assert_eq!(actual.completed_at, expected.completed_at);
        assert_eq!(actual.deadline, expected.deadline);
        assert_eq!(actual.met_deadline(), expected.met_deadline());
    }
}

#[test]
fn compatibility_receipt_preserves_timely_written_timestamps() {
    let (mut gate, writer) = Gate::new();
    assert_eq!(gate.line(), "notification");
    let deadline = Instant::now() + Duration::from_secs(3);
    let receipt = control(&writer, "timely-compat", deadline).unwrap();
    gate.release();
    assert!(gate.line().contains("timely-compat"));
    let (compatibility, outcome) = compatibility_then_physical_terminal(&receipt);
    compatibility.unwrap();
    let WriteOutcome::Written(completion) = outcome else {
        panic!("actual timely flush must remain Written");
    };
    assert!(completion.met_deadline());
    assert_eq!(completion.deadline, deadline);
    assert_same_written(&receipt, completion);
    receipt.try_complete().unwrap().unwrap();
    assert_same_written(&receipt, completion);
}

#[test]
fn compatibility_receipt_preserves_late_written_after_observation_timeout() {
    let (mut gate, writer) = Gate::new();
    let deadline = Instant::now() + Duration::from_millis(500);
    let receipt = writer
        .try_enqueue(
            OwnedFrame::new(large(), 16 * 1024 * 1024),
            WriteClass::Ordinary,
            deadline,
        )
        .unwrap();
    assert_eq!(gate.line(), "notification");
    thread::sleep(deadline.saturating_duration_since(Instant::now()) + Duration::from_millis(20));
    assert_eq!(
        receipt.try_complete().unwrap().unwrap_err().kind(),
        io::ErrorKind::TimedOut
    );
    assert!(matches!(
        receipt.try_outcome(),
        Some(WriteOutcome::ObservationTimeout { .. })
    ));
    assert_eq!(writer.usage.lock().unwrap().ordinary_count, 1);
    gate.release();
    assert!(gate.line().contains("prompt"));
    let (compatibility, outcome) = compatibility_then_physical_terminal(&receipt);
    assert_eq!(compatibility.unwrap_err().kind(), io::ErrorKind::TimedOut);
    let WriteOutcome::Written(completion) = outcome else {
        panic!("late physical flush must not become timeout-only evidence");
    };
    assert!(completion.started_at < deadline);
    assert!(completion.completed_at >= deadline);
    assert!(!completion.met_deadline());
    assert_eq!(completion.deadline, deadline);
    assert_same_written(&receipt, completion);
    assert_eq!(
        receipt.try_complete().unwrap().unwrap_err().kind(),
        io::ErrorKind::TimedOut
    );
    assert_same_written(&receipt, completion);
}

#[test]
fn compatibility_receipt_preserves_queued_expiry_known_not_written() {
    let (mut gate, writer) = Gate::new();
    let prompt = writer
        .try_enqueue(
            OwnedFrame::new(large(), 16 * 1024 * 1024),
            WriteClass::Ordinary,
            Instant::now() + Duration::from_secs(5),
        )
        .unwrap();
    assert_eq!(gate.line(), "notification");
    let deadline = Instant::now() + Duration::from_millis(150);
    let receipt = control(&writer, "expired-compat", deadline).unwrap();
    thread::sleep(deadline.saturating_duration_since(Instant::now()) + Duration::from_millis(20));
    assert_eq!(
        receipt.try_complete().unwrap().unwrap_err().kind(),
        io::ErrorKind::TimedOut
    );
    assert_eq!(writer.usage.lock().unwrap().ordinary_count, 1);
    assert_eq!(writer.usage.lock().unwrap().control_count, 1);
    gate.release();
    assert!(gate.line().contains("prompt"));
    prompt.wait().unwrap();
    let (compatibility, outcome) = compatibility_then_physical_terminal(&receipt);
    assert_eq!(compatibility.unwrap_err().kind(), io::ErrorKind::TimedOut);
    let WriteOutcome::NotWritten(expected) = outcome else {
        panic!("queued expiry must remain provably NotWritten");
    };
    assert_eq!(expected.kind(), io::ErrorKind::TimedOut);
    assert_eq!(expected.to_string(), "runtime write expired in queue");
    for _ in 0..2 {
        assert_eq!(
            receipt.try_complete().unwrap().unwrap_err().kind(),
            expected.kind()
        );
        let Some(WriteOutcome::NotWritten(actual)) = receipt.try_outcome() else {
            panic!("known NotWritten cannot degrade after compatibility reads");
        };
        assert_eq!(actual.kind(), expected.kind());
        assert_eq!(actual.to_string(), expected.to_string());
    }
    assert!(gate.output.recv_timeout(Duration::from_millis(40)).is_err());
}
