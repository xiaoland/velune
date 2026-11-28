//! Receive budgets and real cold-data ownership, independent of session creation.
use super::*;
use std::{
    io::Cursor,
    path::PathBuf,
    time::{Duration, Instant},
};
use velune_test_support::TempRoot;

fn account_with_limits(
    record_bytes: usize,
    retained_bytes: usize,
    records: usize,
) -> Arc<ReadAccount> {
    let mut account = ReadAccount::new();
    Arc::get_mut(&mut account).unwrap().limits = ReadLimits {
        record_bytes,
        retained_bytes,
        records,
    };
    account.ledger.0.lock().unwrap().retained_limit = retained_bytes;
    account
}

#[test]
fn queued_and_returned_records_keep_credit_until_the_last_owner_drops() {
    let account = account_with_limits(64 * 1024, READER_BASE + 2 * 1024 * 1024, 2);
    let frame = serde_json::to_vec(&serde_json::json!({"payload":"x".repeat(8192)})).unwrap();
    let record = read_record(&mut Cursor::new(&frame), &account)
        .unwrap()
        .unwrap();
    let charge = account.ledger.0.lock().unwrap().used;
    assert!(charge > READER_BASE + frame.len());
    assert_eq!(account.ledger.0.lock().unwrap().records, 1);
    let value = Retained::from_record(record.value, record.lease);
    assert_eq!(account.ledger.0.lock().unwrap().used, charge);
    assert_eq!(account.ledger.0.lock().unwrap().records, 0);
    let (value, retained) = value.into_parts();
    let cloned = retained.try_clone().unwrap();
    assert!(account.ledger.0.lock().unwrap().used > charge);
    drop(cloned);
    assert_eq!(account.ledger.0.lock().unwrap().used, charge);
    let mut lease = retained.into_record().unwrap();
    lease.transfer_to_queue().unwrap();
    assert_eq!(account.ledger.0.lock().unwrap().records, 1);
    assert_eq!(account.ledger.0.lock().unwrap().used, charge);
    drop((value, lease));
    assert_eq!(account.ledger.0.lock().unwrap().used, READER_BASE);
    assert_eq!(account.ledger.0.lock().unwrap().records, 0);
}

#[test]
fn many_small_retained_records_hit_aggregate_credit_and_release_it_on_drop() {
    let limit = READER_BASE + 1024 * 1024;
    let account = account_with_limits(64 * 1024, limit, 32);
    let frame = serde_json::to_vec(&serde_json::json!({"payload":"x".repeat(8192)})).unwrap();
    let mut held = Vec::new();
    loop {
        match read_record(&mut Cursor::new(&frame), &account) {
            Ok(Some(record)) => held.push(Retained::from_record(record.value, record.lease)),
            Err(error) => {
                assert_eq!(error.kind(), io::ErrorKind::Other);
                break;
            }
            Ok(None) => panic!("a complete frame cannot disappear"),
        }
        assert!(
            held.len() < 32,
            "aggregate budget must precede the record-count limit"
        );
    }
    assert!(!held.is_empty());
    let usage = account.ledger.0.lock().unwrap();
    assert!(usage.used <= limit && usage.data_used >= frame.len());
    drop(usage);
    drop(held);
    assert_eq!(account.ledger.0.lock().unwrap().used, READER_BASE);
    assert_eq!(account.ledger.0.lock().unwrap().data_used, 0);
    assert!(
        read_record(&mut Cursor::new(&frame), &account)
            .unwrap()
            .is_some()
    );
}

#[test]
fn rejected_or_malformed_records_return_credit_without_truncating_input() {
    let account = account_with_limits(64, READER_BASE + 1024 * 1024, 2);
    let frame = serde_json::to_vec(&serde_json::json!({"payload":"x".repeat(128)})).unwrap();
    let mut input = Cursor::new(&frame);
    let error = read_record(&mut input, &account).err().unwrap();
    assert_eq!(error.kind(), io::ErrorKind::Other);
    assert_eq!(
        input.position(),
        0,
        "oversize is refused before consuming its input"
    );
    assert_eq!(account.ledger.0.lock().unwrap().used, READER_BASE);
    assert!(
        read_record(&mut Cursor::new(b"{broken\n"), &account)
            .err()
            .unwrap()
            .to_string()
            .contains("key must be a string")
    );
    assert_eq!(account.ledger.0.lock().unwrap().records, 0);
    assert_eq!(account.ledger.0.lock().unwrap().used, READER_BASE);
}

fn helper(home: &TempRoot) -> PathBuf {
    let script = home.join("reader.py");
    std::fs::write(&script, r#"import json,sys
for line in sys.stdin:
 request=json.loads(line)
 print(json.dumps({'jsonrpc':'2.0','id':request['id'],'result':{'payload':'x'*int(request['params']['bytes'])}}),flush=True)
"#).unwrap();
    script
}

#[test]
fn common_reader_preserves_a_large_record_without_a_creation_policy() {
    let home = TempRoot::new("large-common-reader");
    let mut command = crate::process::command("python3");
    command.arg(helper(&home));
    let mut rpc = crate::native::rpc::RpcClient::spawn_at(
        command,
        &crate::execution::ExecutionTarget::Local,
        Some(home.path()),
    )
    .unwrap();
    let response = rpc
        .request(
            "fixture/read",
            &serde_json::json!({"bytes":9 * 1024 * 1024}),
        )
        .unwrap();
    assert_eq!(response["payload"].as_str().unwrap().len(), 9 * 1024 * 1024);
}

#[test]
fn retained_cold_data_outlives_actual_connection_and_removed_home() {
    let home = TempRoot::new("cold-retained-data");
    let mut command = crate::process::command("python3");
    command.arg(helper(&home));
    let mut rpc = crate::native::rpc::RpcClient::spawn_at(
        command,
        &crate::execution::ExecutionTarget::Local,
        Some(home.path()),
    )
    .unwrap();
    let data = rpc
        .request("fixture/read", &serde_json::json!({"bytes":1024 * 1024}))
        .unwrap();
    let ledger = data.retention.0[0].ledger.clone();
    let credit = ledger.0.lock().unwrap().data_used;
    assert!(credit >= 1024 * 1024);
    rpc.poll().unwrap();
    assert_eq!(ledger.0.lock().unwrap().data_used, credit);
    drop(rpc);
    let deadline = Instant::now() + Duration::from_secs(5);
    while data
        .retention
        .0
        .iter()
        .any(|lease| lease.account.upgrade().is_some())
    {
        assert!(
            Instant::now() < deadline,
            "retained data pinned the actual IO owner"
        );
        std::thread::sleep(Duration::from_millis(2));
    }
    let path = home.to_path_buf();
    drop(home);
    assert!(!path.exists(), "temporary HOME must actually be removed");
    assert_eq!(data["payload"].as_str().unwrap().len(), 1024 * 1024);
    assert_eq!(ledger.0.lock().unwrap().used, credit);
    drop(data);
    assert_eq!(ledger.0.lock().unwrap().used, 0);
}
