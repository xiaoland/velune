//! Private ledger assertions for the public cold facade; never contacts an agent.
use super::*;
use crate::history::{self, HistoryConfig};
use crate::process::command;
use std::{fs, os::unix::fs::PermissionsExt, path::PathBuf};

struct ColdFixture {
    directory: PathBuf,
    config: HistoryConfig,
}
impl ColdFixture {
    fn new(legacy: bool) -> Self {
        static NEXT: AtomicUsize = AtomicUsize::new(0);
        let directory = std::env::temp_dir().join(format!(
            "velune-cold-retention-{}-{}",
            std::process::id(),
            NEXT.fetch_add(1, Ordering::Relaxed)
        ));
        fs::create_dir(&directory).unwrap();
        let binary = directory.join("synthetic-codex");
        fs::write(&binary, include_str!("cold_history_fixture.py")).unwrap();
        fs::set_permissions(&binary, fs::Permissions::from_mode(0o700)).unwrap();
        if legacy {
            fs::write(directory.join("legacy"), b"synthetic").unwrap();
        }
        let config = HistoryConfig {
            kind: crate::version::RuntimeKind::Codex166,
            execution: crate::execution::ExecutionTarget::Local,
            os_home: Some(directory.clone()),
            provider: "codex".into(),
            binary,
            node_binary: PathBuf::new(),
            resources_directory: directory.clone(),
            root: directory.clone(),
            home: directory.clone(),
        };
        Self { directory, config }
    }
    fn assert_process_closed(&self) {
        let pid = fs::read_to_string(self.directory.join("rpc.pid")).unwrap();
        assert!(
            !command("kill")
                .args(["-0", pid.trim()])
                .stdout(std::process::Stdio::null())
                .stderr(std::process::Stdio::null())
                .status()
                .unwrap()
                .success(),
            "cold RPC must be physically gone"
        );
    }
}
impl Drop for ColdFixture {
    fn drop(&mut self) {
        fs::remove_dir_all(&self.directory).unwrap();
    }
}

fn ledger_after_io<T>(value: &Retained<T>) -> Arc<ReadLedger> {
    assert!(
        !value.retention.0.is_empty(),
        "RPC results may not use an empty guard"
    );
    // A short-lived reader can still be unwinding after the child exits.
    // A data guard must never be what keeps its IO anchor alive.
    let io_alive = || {
        value
            .retention
            .0
            .iter()
            .any(|lease| lease.account.upgrade().is_some())
    };
    let deadline = std::time::Instant::now() + std::time::Duration::from_secs(1);
    while io_alive() && std::time::Instant::now() < deadline {
        std::thread::sleep(std::time::Duration::from_millis(5));
    }
    assert!(!io_alive(), "retained history kept its IO anchor alive");
    let ledger = value.retention.0[0].ledger.clone();
    {
        let usage = ledger.0.lock().unwrap();
        assert_eq!(usage.records, 0);
        assert!(usage.data_used > 0);
        assert_eq!(
            usage.used, usage.data_used,
            "only retained data remains after IO cleanup"
        );
    }
    ledger
}

#[test]
fn cold_history_pages_release_framing_headroom_but_keep_payload_credit() {
    let limit = 13 * 1024 * 1024;
    let account = || {
        let account = ReadAccount::new();
        account.state.lock().unwrap().limits.retained_bytes = limit;
        account.ledger.0.lock().unwrap().retained_limit = limit;
        account
    };
    let payload = "x".repeat(128 * 1024);
    let frame = serde_json::to_vec(&serde_json::json!({"data":[{"text":payload}]})).unwrap();
    let read = |account: &Arc<ReadAccount>| {
        read_record(&mut std::io::Cursor::new(&frame), account).map(|record| {
            let ReadRecord { value, lease } = record.unwrap();
            Retained::from_record(value, lease)
        })
    };
    let old = account();
    let _unsettled = [read(&old).unwrap(), read(&old).unwrap()];
    assert!(
        read(&old).is_err(),
        "retaining full input reservations exhausts the budget"
    );

    let account = account();
    let mut pages = Vec::new();
    for _ in 0..6 {
        let mut page = read(&account).unwrap();
        page.settle_history_page();
        pages.push(page);
    }
    assert_eq!(pages[5]["data"][0]["text"].as_str(), Some(payload.as_str()));
    let data_used = account.ledger.0.lock().unwrap().data_used;
    assert!(data_used >= 6 * payload.len() && data_used < limit);
    drop(pages);
    assert_eq!(account.ledger.0.lock().unwrap().data_used, 0);
}

#[test]
fn cold_paginated_history_keeps_all_page_credit_after_rpc_and_home_cleanup() {
    let fixture = ColdFixture::new(false);
    let read = history::read_codex_with_summary(&fixture.config, "runtime", "synthetic").unwrap();
    fixture.assert_process_closed();
    assert_eq!(read.1.messages.len(), 2);
    assert!(
        read.retention.0.len() >= 4,
        "metadata, two item pages and turns must all survive"
    );
    let ledger = ledger_after_io(&read);
    drop(read);
    let usage = ledger.0.lock().unwrap();
    assert_eq!(usage.used, 0);
    assert_eq!(usage.data_used, 0);
}

#[test]
#[ignore = "165 MiB synthetic history for the full receive-budget regression"]
fn cold_long_codex_history_retains_every_page_and_tool_output() {
    let fixture = ColdFixture::new(false);
    fs::write(fixture.directory.join("long-history"), []).unwrap();
    let read = history::read_codex_with_summary(&fixture.config, "runtime", "synthetic").unwrap();
    fixture.assert_process_closed();
    assert_eq!(read.1.messages.len(), 165 * 4);
    for (index, message) in read.1.messages.iter().enumerate() {
        let prefix = format!("page-{}-{}\n", index / 4, index % 4);
        let [
            crate::conversation::MessageBlock::Tool {
                output: Some(output),
                ..
            },
        ] = message.blocks.as_slice()
        else {
            panic!("tools must retain their output");
        };
        assert!(output.starts_with(&prefix));
        assert_eq!(output.len(), prefix.len() + 256 * 1024);
    }
    let ledger = ledger_after_io(&read);
    drop(read);
    assert_eq!(ledger.0.lock().unwrap().used, 0);
}

#[test]
fn cold_legacy_summary_and_history_split_return_credit_only_after_the_last_piece() {
    let fixture = ColdFixture::new(true);
    let read = history::read_codex_with_summary(&fixture.config, "runtime", "synthetic").unwrap();
    fixture.assert_process_closed();
    let ledger = ledger_after_io(&read);
    let (summary, history) = read.split_map(|parts| parts);
    assert_eq!(history.messages.len(), 2);
    drop(history);
    assert!(ledger.0.lock().unwrap().data_used > 0);
    assert_eq!(summary.id, "runtime:synthetic");
    drop(summary);
    assert_eq!(ledger.0.lock().unwrap().used, 0);
}

#[test]
fn cold_projection_failure_and_replacement_do_not_keep_transient_credit_forever() {
    let fixture = ColdFixture::new(false);
    let read = history::read(&fixture.config, "runtime", "synthetic").unwrap();
    let ledger = ledger_after_io(&read);
    let failed: Result<Retained<()>, &str> = read.try_map(|_| Err("synthetic projection refusal"));
    assert!(failed.is_err());
    assert_eq!(ledger.0.lock().unwrap().used, 0);
    let first = history::read(&fixture.config, "runtime", "synthetic").unwrap();
    let first_ledger = ledger_after_io(&first);
    let mut cache = Some(first);
    let second = history::read(&fixture.config, "runtime", "synthetic").unwrap();
    let second_ledger = ledger_after_io(&second);
    drop(cache.replace(second));
    assert_eq!(first_ledger.0.lock().unwrap().used, 0);
    assert!(second_ledger.0.lock().unwrap().data_used > 0);
    drop(cache.take());
    assert_eq!(second_ledger.0.lock().unwrap().used, 0);
}
