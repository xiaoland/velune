use super::*;
use crate::repository::Repository;
use std::time::{Duration, Instant};

struct Home(PathBuf);
impl Home {
    fn new() -> Self {
        static NEXT: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(1);
        let n = NEXT.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
        let path =
            std::env::temp_dir().join(format!("velune-links-metadata-{}-{n}", std::process::id()));
        fs::create_dir(&path).unwrap();
        Self(path)
    }
}
impl Drop for Home {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.0);
    }
}
fn limits() -> LinksLimits {
    LinksLimits {
        reservations: 4,
        patch_bytes: 16_384,
        value_bytes: 65_536,
        file_bytes: 65_536,
        retained_bytes: 20_000_000,
    }
}
fn link(id: &str, version: u32) -> ConversationLink {
    serde_json::from_value(serde_json::json!({
        "id": id, "segments": [{ "runtimeInstanceId": "synthetic",
            "runtimeTypeId": "pi-1.0", "nativeConversationId": format!("synthetic:{id}-{version}"),
            "cwd": null, "cutoff": null, "handoff": null, "deleted": false }]
    }))
    .unwrap()
}
fn start(repo: &Repository, hooks: Hooks) -> LinksMetadata {
    start_current(repo, hooks, &Vec::new())
}
fn start_current(
    repo: &Repository,
    hooks: Hooks,
    current: &Vec<ConversationLink>,
) -> LinksMetadata {
    LinksMetadata::start(
        repo.home.clone(),
        repo.home_lock.claim_links().unwrap(),
        limits(),
        current,
        hooks,
    )
    .unwrap()
}
fn hold() -> (
    Hooks,
    mpsc::Receiver<()>,
    mpsc::Sender<()>,
    mpsc::Receiver<()>,
) {
    let (entered, entered_rx) = mpsc::channel();
    let (release, release_rx) = mpsc::channel();
    let (replied, replied_rx) = mpsc::channel();
    (
        Hooks {
            hold_sync: Some((entered, release_rx)),
            replied: Some(replied),
            ..Hooks::default()
        },
        entered_rx,
        release,
        replied_rx,
    )
}
fn receipt(metadata: &mut LinksMetadata, current: &mut Vec<ConversationLink>) -> LinksReceipt {
    let deadline = Instant::now() + Duration::from_secs(5);
    loop {
        if let Some(receipt) = metadata.poll_result(current) {
            return receipt;
        }
        assert!(Instant::now() < deadline, "real disk worker did not return");
        thread::sleep(Duration::from_millis(1));
    }
}
fn close(metadata: &mut LinksMetadata) {
    metadata.begin_close();
    assert!(metadata.poll_drained());
    let deadline = Instant::now() + Duration::from_secs(5);
    loop {
        match metadata.finish_close() {
            Ok(()) => break,
            Err(LinksAdmissionError::NotDrained) => {
                assert!(Instant::now() < deadline);
                thread::yield_now();
            }
            other => panic!("unexpected finish_close: {other:?}"),
        }
    }
}
fn reserve(
    metadata: &mut LinksMetadata,
    id: &str,
    patch_bytes: usize,
) -> Result<LinksReservation, LinksAdmissionError> {
    metadata.reserve(
        id,
        patch_bytes,
        LinksGrowth {
            value_bytes: 8192,
            file_bytes: 8192,
        },
    )
}
fn patch(
    metadata: &mut LinksMetadata,
    r: &LinksReservation,
    old: Option<ConversationLink>,
    new: Option<ConversationLink>,
) -> LinksTicket {
    metadata
        .enqueue_patch(
            r,
            LinksPatch {
                expected_old: old,
                new,
            },
            false,
        )
        .unwrap()
}

#[test]
fn held_real_write_preserves_other_ids_and_rejects_same_id_before_native_effects() {
    let home = Home::new();
    let (repo, _) = Repository::open(&home.0).unwrap();
    let (hooks, entered, release, _) = hold();
    let mut metadata = start(&repo, hooks);
    let mut current = Vec::new();
    let a = reserve(&mut metadata, "a", 4096).unwrap();
    let b = reserve(&mut metadata, "b", 4096).unwrap();
    let ta = patch(&mut metadata, &a, None, Some(link("a", 1)));
    let tb = patch(&mut metadata, &b, None, Some(link("b", 1)));
    assert!(metadata.poll_result(&mut current).is_none());
    entered.recv_timeout(Duration::from_secs(5)).unwrap();
    // A real private temporary file has been written, but not synced/renamed.
    let native_effect = home.0.join("synthetic-native-mutation");
    match reserve(&mut metadata, "a", 4096) {
        Ok(_) => fs::write(&native_effect, b"must not run").unwrap(),
        Err(error) => assert_eq!(error, LinksAdmissionError::Busy),
    }
    assert!(!native_effect.exists());
    for _ in 0..8 {
        assert!(metadata.poll_result(&mut current).is_none());
    }
    assert!(
        current.is_empty(),
        "enqueue/timeout must not install strict metadata"
    );
    assert!(!home.0.join("conversation-links.json").exists());
    metadata.begin_close();
    assert_eq!(
        reserve(&mut metadata, "c", 4096),
        Err(LinksAdmissionError::Closing)
    );
    assert!(!metadata.poll_drained());
    assert_eq!(
        metadata.finish_close(),
        Err(LinksAdmissionError::NotDrained)
    );
    assert!(repo.unlock().is_err());
    assert!(Repository::lock_home(&home.0).is_err());
    release.send(()).unwrap();
    let first = receipt(&mut metadata, &mut current);
    assert_eq!(first.ticket, ta);
    assert_eq!(first.outcome, LinksOutcome::Patched);
    assert_eq!(current, vec![link("a", 1)]);
    let second = receipt(&mut metadata, &mut current);
    assert_eq!(second.ticket, tb);
    assert_eq!(second.outcome, LinksOutcome::Patched);
    assert_eq!(current, vec![link("a", 1), link("b", 1)]);
    assert_eq!(repo.load_conversation_links().unwrap(), current);
    close(&mut metadata);
    repo.unlock().unwrap();
    assert!(Repository::lock_home(&home.0).is_ok());
}

#[test]
fn backup_fence_copies_exact_validated_bytes_and_retains_reservation_through_patch() {
    let home = Home::new();
    let (repo, _) = Repository::open(&home.0).unwrap();
    let original = format!(
        " {{\n\"schemaVersion\":3, \"links\":{} }}\n",
        serde_json::to_string(&vec![link("a", 1)]).unwrap()
    )
    .into_bytes();
    let path = home.0.join("conversation-links.json");
    fs::write(&path, &original).unwrap();
    let mut current = vec![link("a", 1)];
    let mut metadata = start_current(&repo, Hooks::default(), &current);
    let a = reserve(&mut metadata, "a", 4096).unwrap();
    let ticket = metadata.enqueue_backup_fence(&a).unwrap();
    let fence = receipt(&mut metadata, &mut current);
    assert_eq!(fence.ticket, ticket);
    let LinksOutcome::BackupReady(Some(backup)) = fence.outcome else {
        panic!("backup not confirmed");
    };
    assert_eq!(fs::read(backup).unwrap(), original);
    assert_eq!(fs::read(&path).unwrap(), original);
    assert_eq!(
        reserve(&mut metadata, "a", 4096),
        Err(LinksAdmissionError::Busy)
    );
    metadata.begin_close();
    assert!(
        !metadata.poll_drained(),
        "fence receipt does not release business reservation"
    );
    // Already-reserved follow-up remains admissible during close.
    patch(&mut metadata, &a, Some(link("a", 1)), Some(link("a", 2)));
    assert_eq!(
        receipt(&mut metadata, &mut current).outcome,
        LinksOutcome::Patched
    );
    assert_eq!(current, vec![link("a", 2)]);
    close(&mut metadata);
}

#[test]
fn invalid_or_legacy_schema_never_overwrites_or_backs_up_as_current() {
    for bytes in [
        br#"{"schemaVersion":1,"links":[]}"#.as_slice(),
        br#"{"schemaVersion":99,"links":[]}"#,
        br#"{"schemaVersion":3,"links":[],"future":"preserve"}"#,
        br#"{"schemaVersion":3,"links":[{"id":"a","segments":[],"future":true}]}"#,
        br#"{"schemaVersion":3"#,
    ] {
        let home = Home::new();
        let (repo, _) = Repository::open(&home.0).unwrap();
        let path = home.0.join("conversation-links.json");
        fs::write(&path, bytes).unwrap();
        let mut metadata = start(&repo, Hooks::default());
        let a = reserve(&mut metadata, "a", 4096).unwrap();
        metadata
            .enqueue_patch(
                &a,
                LinksPatch {
                    expected_old: None,
                    new: Some(link("a", 1)),
                },
                true,
            )
            .unwrap();
        let mut current = Vec::new();
        assert!(matches!(
            receipt(&mut metadata, &mut current).outcome,
            LinksOutcome::KnownNotWritten(_)
        ));
        assert!(current.is_empty());
        assert_eq!(fs::read(path).unwrap(), bytes);
        assert!(!fs::read_dir(&home.0).unwrap().any(|e| {
            e.unwrap()
                .file_name()
                .to_string_lossy()
                .contains("metadata-backup")
        }));
        close(&mut metadata);
    }
}

#[test]
fn rename_then_sync_failure_quarantines_without_replay_and_installs_only_after_verification() {
    let home = Home::new();
    let (repo, _) = Repository::open(&home.0).unwrap();
    let hooks = Hooks {
        fail_after_rename: std::sync::atomic::AtomicBool::new(true),
        ..Hooks::default()
    };
    let mut metadata = start(&repo, hooks);
    let mut current = Vec::new();
    let a = reserve(&mut metadata, "a", 4096).unwrap();
    let b = reserve(&mut metadata, "b", 4096).unwrap();
    let ta = patch(&mut metadata, &a, None, Some(link("a", 1)));
    patch(&mut metadata, &b, None, Some(link("b", 1)));
    let unknown = receipt(&mut metadata, &mut current);
    assert_eq!(unknown.ticket, ta);
    assert!(matches!(
        unknown.outcome,
        LinksOutcome::DurabilityUnknown(_)
    ));
    assert!(current.is_empty());
    assert_eq!(repo.load_conversation_links().unwrap(), vec![link("a", 1)]);
    let committed_bytes = fs::read(home.0.join("conversation-links.json")).unwrap();
    for _ in 0..8 {
        assert!(metadata.poll_result(&mut current).is_none());
    }
    assert_eq!(
        fs::read(home.0.join("conversation-links.json")).unwrap(),
        committed_bytes
    );
    assert_eq!(
        reserve(&mut metadata, "a", 4096),
        Err(LinksAdmissionError::Busy)
    );
    metadata.begin_close();
    assert!(!metadata.poll_drained());
    assert!(repo.unlock().is_err());
    metadata.verify_uncertain().unwrap();
    let verified = receipt(&mut metadata, &mut current);
    assert_eq!(verified.ticket, ta);
    assert_eq!(verified.outcome, LinksOutcome::Patched);
    assert_eq!(current, vec![link("a", 1)]);
    assert_eq!(
        receipt(&mut metadata, &mut current).outcome,
        LinksOutcome::Patched
    );
    assert_eq!(current, vec![link("a", 1), link("b", 1)]);
    close(&mut metadata);
}

#[test]
fn uncertain_verification_does_not_rewrite_external_corruption() {
    let home = Home::new();
    let (repo, _) = Repository::open(&home.0).unwrap();
    let mut metadata = start(
        &repo,
        Hooks {
            fail_after_rename: std::sync::atomic::AtomicBool::new(true),
            ..Hooks::default()
        },
    );
    let mut current = Vec::new();
    let a = reserve(&mut metadata, "a", 4096).unwrap();
    patch(&mut metadata, &a, None, Some(link("a", 1)));
    assert!(matches!(
        receipt(&mut metadata, &mut current).outcome,
        LinksOutcome::DurabilityUnknown(_)
    ));
    let path = home.0.join("conversation-links.json");
    fs::write(&path, b"synthetic externally corrupted bytes").unwrap();
    metadata.verify_uncertain().unwrap();
    assert!(matches!(
        receipt(&mut metadata, &mut current).outcome,
        LinksOutcome::DurabilityUnknown(_)
    ));
    assert!(current.is_empty());
    assert_eq!(
        fs::read(path).unwrap(),
        b"synthetic externally corrupted bytes"
    );
    metadata.begin_close();
    assert!(!metadata.poll_drained());
    assert!(repo.unlock().is_err());
    // Explicit teardown disposes the quarantined result; it does NOT fabricate a drained proof.
    drop(metadata);
}

#[test]
fn repository_drop_cannot_unlock_held_io_or_unsettled_real_result() {
    let home = Home::new();
    let (repo, _) = Repository::open(&home.0).unwrap();
    let (hooks, entered, release, replied) = hold();
    let mut metadata = start(&repo, hooks);
    let mut current = Vec::new();
    let a = reserve(&mut metadata, "a", 4096).unwrap();
    patch(&mut metadata, &a, None, Some(link("a", 1)));
    assert!(metadata.poll_result(&mut current).is_none());
    entered.recv_timeout(Duration::from_secs(5)).unwrap();
    metadata.begin_close();
    assert!(!metadata.poll_drained());
    assert!(repo.unlock().is_err());
    drop(repo);
    assert!(Repository::lock_home(&home.0).is_err());
    release.send(()).unwrap();
    replied.recv_timeout(Duration::from_secs(5)).unwrap();
    assert!(
        Repository::lock_home(&home.0).is_err(),
        "returned IO still owns its unconsumed result lease"
    );
    assert!(!metadata.poll_drained());
    assert_eq!(
        receipt(&mut metadata, &mut current).outcome,
        LinksOutcome::Patched
    );
    assert_eq!(
        current,
        vec![link("a", 1)],
        "install is independent of the discarded business continuation"
    );
    close(&mut metadata);
    assert!(Repository::lock_home(&home.0).is_ok());
}

#[test]
fn dropping_component_never_joins_held_io_or_releases_its_home_lease() {
    let home = Home::new();
    let (repo, _) = Repository::open(&home.0).unwrap();
    let (hooks, entered, release, replied) = hold();
    let mut metadata = start(&repo, hooks);
    let a = reserve(&mut metadata, "a", 4096).unwrap();
    patch(&mut metadata, &a, None, Some(link("a", 1)));
    assert!(metadata.poll_result(&mut Vec::new()).is_none());
    entered.recv_timeout(Duration::from_secs(5)).unwrap();
    drop(metadata); // Must return without waiting for the controlled filesystem gate.
    drop(repo);
    assert!(Repository::lock_home(&home.0).is_err());
    release.send(()).unwrap();
    replied.recv_timeout(Duration::from_secs(5)).unwrap();
    assert!(Repository::lock_home(&home.0).is_ok());
    let bytes = fs::read(home.0.join("conversation-links.json")).unwrap();
    assert_eq!(
        decode_conversation_links(serde_json::from_slice(&bytes).unwrap()).unwrap(),
        vec![link("a", 1)]
    );
}

#[test]
fn admission_budget_is_reserved_before_native_work_and_cas_failure_does_not_write() {
    let home = Home::new();
    let (repo, _) = Repository::open(&home.0).unwrap();
    let mut metadata = start(&repo, Hooks::default());
    assert_eq!(
        reserve(&mut metadata, "a", limits().patch_bytes + 1),
        Err(LinksAdmissionError::Capacity)
    );
    let a = reserve(&mut metadata, "a", 4096).unwrap();
    // Simulates a caller whose captured expectedOld was invalidated before it reached head.
    patch(&mut metadata, &a, Some(link("a", 1)), Some(link("a", 2)));
    let mut current = Vec::new();
    assert!(matches!(
        receipt(&mut metadata, &mut current).outcome,
        LinksOutcome::KnownNotWritten(_)
    ));
    assert!(!home.0.join("conversation-links.json").exists());
    let r = reserve(&mut metadata, "a", 4096).unwrap();
    metadata.release(&r).unwrap();
    close(&mut metadata);
}

#[test]
fn cumulative_growth_is_rejected_before_native_and_held_or_unknown_credit_is_not_reused() {
    for typed_limit in [true, false] {
        for unknown in [false, true] {
            let home = Home::new();
            let (repo, _) = Repository::open(&home.0).unwrap();
            let mut base = link("base", 1);
            // Controls exercise six-byte JSON escaping; nested map/list entries
            // exercise variable indentation. No synthetic string is used as an IO path.
            base.segments[0].cwd = Some(
                format!("/synthetic/{}", "\0".repeat(4096))
                    .into_boxed_str()
                    .into_string(),
            );
            base.segments[0].message_origins.insert(
                "key\"\\\n".into(),
                vec!["native\0\n\"\\".into(), "目标".into()],
            );
            let mut prefix = base.segments[0].clone();
            prefix.native_conversation_id = "synthetic:base-prefix".into();
            prefix.cutoff = Some(crate::config::ConversationCutoff {
                message_count: u64::MAX,
                prefix_digest: "a".repeat(64),
            });
            prefix.excluded_message_ids = vec!["removed\n\"\\".into()];
            prefix.handoff = Some(crate::config::ConversationHandoff {
                marker: "b".repeat(64),
                payload_digest: "c".repeat(64),
                user_message_ordinal: u64::MAX,
            });
            prefix.imported = Some(crate::config::ConversationImport {
                prefix: crate::config::ConversationCutoff {
                    message_count: u64::MAX,
                    prefix_digest: "d".repeat(64),
                },
                summary_work_id: Some("synthetic-summary".into()),
            });
            // Only the prefix has a cutoff/tombstones; the final segment remains open.
            base.segments.insert(0, prefix);
            let mut current = vec![base];
            let baseline = LinksGrowth::baseline(&current);
            let mut a_value = link("a", 1);
            a_value.segments[0].cwd = Some("\0\"\\".repeat(32));
            // Credit counts actual spare Vec capacity, not just element lengths.
            a_value.segments.reserve_exact(2);
            let mut b_value = link("b", 1);
            b_value.segments[0].cwd = Some("\0\"\\".repeat(32));
            b_value.segments.reserve_exact(2);
            for new in [&a_value, &b_value] {
                assert!(
                    std::mem::size_of::<LinksPatch>() + link_cost(new) <= 4096,
                    "both complete new-link envelopes are individually legal before native work"
                );
            }
            let a_growth = LinksGrowth::for_new_link(Some(&a_value));
            let b_growth = LinksGrowth::for_new_link(Some(&b_value));
            let both = baseline
                .checked_add(a_growth)
                .unwrap()
                .checked_add(b_growth)
                .unwrap();
            let mut capacity = limits();
            capacity.value_bytes = baseline.value_bytes
                + if typed_limit {
                    a_growth.value_bytes.max(b_growth.value_bytes)
                } else {
                    a_growth.value_bytes + b_growth.value_bytes
                };
            capacity.file_bytes = baseline.file_bytes
                + if typed_limit {
                    a_growth.file_bytes + b_growth.file_bytes
                } else {
                    a_growth.file_bytes.max(b_growth.file_bytes)
                };
            assert_eq!(both.value_bytes > capacity.value_bytes, typed_limit);
            assert_eq!(both.file_bytes > capacity.file_bytes, !typed_limit);
            let path = home.0.join("conversation-links.json");
            let original = serde_json::to_vec_pretty(
                &serde_json::json!({"schemaVersion": 3, "links": &current}),
            )
            .unwrap();
            assert!(original.len() <= baseline.file_bytes);
            fs::write(&path, &original).unwrap();
            let (hooks, entered, release, replied) = hold();
            hooks
                .fail_after_rename
                .store(unknown, std::sync::atomic::Ordering::Relaxed);
            let mut metadata = LinksMetadata::start(
                repo.home.clone(),
                repo.home_lock.claim_links().unwrap(),
                capacity,
                &current,
                hooks,
            )
            .unwrap();
            let a = metadata.reserve("a", 4096, a_growth).unwrap();
            let deleting = metadata
                .reserve("base", capacity.patch_bytes, LinksGrowth::default())
                .unwrap();
            let native_a = home.0.join("native-a");
            let native_b = home.0.join("native-b");
            match metadata.reserve("b", 4096, b_growth) {
                Ok(_) => fs::write(&native_b, b"must not execute").unwrap(),
                Err(error) => assert_eq!(error, LinksAdmissionError::Capacity),
            }
            assert!(
                !native_b.exists(),
                "combined growth must reject before native work"
            );
            metadata.enqueue_backup_fence(&a).unwrap();
            assert!(matches!(
                receipt(&mut metadata, &mut current).outcome,
                LinksOutcome::BackupReady(Some(_))
            ));
            replied.recv_timeout(Duration::from_secs(5)).unwrap();
            assert_eq!(
                metadata.reserve("b", 4096, b_growth),
                Err(LinksAdmissionError::Capacity),
                "a successful backup fence does not return its future patch credit"
            );
            // A legal retained patch envelope can still violate its promised growth.
            let mut oversized_growth = a_value.clone();
            oversized_growth.segments[0].cwd = Some("\0".repeat(1024));
            let invalid = LinksPatch {
                expected_old: None,
                new: Some(oversized_growth),
            };
            assert!(patch_cost(&invalid) <= 4096);
            assert_eq!(
                metadata.enqueue_patch(&a, invalid, false),
                Err(LinksAdmissionError::InvalidPatch)
            );
            let mut oversized_envelope = a_value.clone();
            oversized_envelope.segments.reserve_exact(4096);
            assert_eq!(
                metadata.enqueue_patch(
                    &a,
                    LinksPatch {
                        expected_old: None,
                        new: Some(oversized_envelope)
                    },
                    false
                ),
                Err(LinksAdmissionError::InvalidPatch)
            );
            fs::write(&native_a, b"admitted native effect").unwrap();
            let a_ticket = patch(&mut metadata, &a, None, Some(a_value));
            let base_old = current[0].clone();
            patch(&mut metadata, &deleting, Some(base_old), None);
            assert!(metadata.poll_result(&mut current).is_none());
            entered.recv_timeout(Duration::from_secs(5)).unwrap();
            assert_eq!(
                metadata.reserve("b", 4096, b_growth),
                Err(LinksAdmissionError::Capacity),
                "held IO and an uncommitted deletion cannot donate capacity"
            );
            release.send(()).unwrap();
            replied.recv_timeout(Duration::from_secs(5)).unwrap();
            assert_eq!(
                metadata.reserve("b", 4096, b_growth),
                Err(LinksAdmissionError::Capacity),
                "an unconsumed real result retains its growth credit"
            );
            let result = receipt(&mut metadata, &mut current);
            assert_eq!(result.ticket, a_ticket);
            if unknown {
                assert!(matches!(result.outcome, LinksOutcome::DurabilityUnknown(_)));
                assert_eq!(current.len(), 1);
                assert_eq!(
                    metadata.reserve("b", 4096, b_growth),
                    Err(LinksAdmissionError::Capacity),
                    "quarantine is nonterminal and cannot donate capacity"
                );
                metadata.verify_uncertain().unwrap();
                assert_eq!(
                    receipt(&mut metadata, &mut current).outcome,
                    LinksOutcome::Patched
                );
            } else {
                assert_eq!(result.outcome, LinksOutcome::Patched);
            }
            assert_eq!(current.len(), 2);
            assert!(native_a.exists());
            assert!(!native_b.exists());
            assert_eq!(repo.load_conversation_links().unwrap(), current);
            assert!(fs::read(&path).unwrap().len() <= capacity.file_bytes);
            assert!(
                LinksGrowth::baseline(&current).fits(baseline.checked_add(a_growth).unwrap()),
                "normalizing outer capacity must keep the admitted typed/file proof"
            );
            assert_eq!(
                metadata.reserve("b", 4096, b_growth),
                Err(LinksAdmissionError::Capacity),
                "queued deletion has not committed its reclaimed space"
            );
            assert_eq!(
                receipt(&mut metadata, &mut current).outcome,
                LinksOutcome::Patched
            );
            assert_eq!(current.len(), 1);
            assert_eq!(current[0].id, "a");
            let b = metadata.reserve("b", 4096, b_growth).unwrap();
            metadata.release(&b).unwrap();
            close(&mut metadata);
        }
    }
}

#[cfg(unix)]
fn opaque_limits() -> LinksLimits {
    LinksLimits {
        reservations: 4,
        patch_bytes: 128 * 1024,
        value_bytes: 256 * 1024,
        file_bytes: 128 * 1024,
        retained_bytes: 64 * 1024 * 1024,
    }
}
#[cfg(unix)]
fn opaque_skeleton(id: &str, home: &Home) -> ConversationLink {
    let mut value = link(id, 1);
    value.segments[0].runtime_type_id = crate::config::RuntimeKind::Acp;
    value.segments[0].native_conversation_id = "synthetic:".into();
    value.segments[0].cwd = Some(home.0.to_string_lossy().into_owned());
    value
}
#[cfg(unix)]
fn opaque_client(home: &Home, mode: &str, identity: &str) -> velune_agent_runtime::acp::Client {
    use std::os::unix::fs::PermissionsExt;
    fs::set_permissions(&home.0, fs::Permissions::from_mode(0o700)).unwrap();
    let script = home.0.join("opaque.py");
    fs::write(
        home.0.join("opaque-id.json"),
        serde_json::to_vec(identity).unwrap(),
    )
    .unwrap();
    let source = r#"import json,sys,time,pathlib
root=pathlib.Path(ROOT)
mode=MODE
for line in sys.stdin:
 r=json.loads(line)
 with (root/'opaque-wire.jsonl').open('a') as f: f.write(json.dumps(r)+'\n')
 def emit(value): print(json.dumps({'jsonrpc':'2.0','id':r['id'],'result':value}),flush=True)
 if r.get('method')=='initialize': emit({'protocolVersion':1,'agentCapabilities':{}})
 elif r.get('method')=='session/new':
  (root/'opaque-native-created').touch()
  if mode=='hold':
   while not (root/'opaque-release').exists(): time.sleep(.001)
  if mode=='incomplete':
   sys.stdout.write('{"jsonrpc":"2.0","id":'+json.dumps(r['id'])+',"result":{"sessionId":"'+'x'*262144)
   sys.stdout.flush()
  else: emit({'sessionId':json.loads((root/'opaque-id.json').read_text())})
 else: emit({})
"#.replace("ROOT", &format!("{:?}", home.0.to_string_lossy())).replace("MODE", &format!("{mode:?}"));
    fs::write(&script, source).unwrap();
    let mut client =
        velune_agent_runtime::acp::Client::connect(velune_agent_runtime::acp::Config {
            binary: "/usr/bin/python3".into(),
            args: vec![script.to_string_lossy().into_owned()],
            environment: BTreeMap::new(),
            execution: velune_agent_runtime::execution::ExecutionTarget::Local,
            os_home: Some(home.0.clone()),
            mcp_servers: Vec::new(),
        })
        .unwrap();
    client.enable_command_port().unwrap();
    client
}
#[cfg(unix)]
fn opaque_until(mut predicate: impl FnMut() -> bool) {
    let deadline = Instant::now() + Duration::from_secs(5);
    while !predicate() {
        assert!(
            Instant::now() < deadline,
            "actual opaque helper/cleanup did not return"
        );
        thread::sleep(Duration::from_millis(1));
    }
}
#[cfg(unix)]
fn opaque_outcome(
    client: &mut velune_agent_runtime::acp::Client,
    ticket: &velune_agent_runtime::acp::AcpTicket,
) -> velune_agent_runtime::creation::CreationOutcome {
    let mut outcome = None;
    opaque_until(|| {
        let _ = client.poll();
        outcome = client.take_creation_result(ticket);
        outcome.is_some()
    });
    outcome.unwrap()
}
#[cfg(unix)]
fn opaque_wire_counts(home: &Home) -> (usize, usize) {
    let values: Vec<serde_json::Value> = fs::read_to_string(home.0.join("opaque-wire.jsonl"))
        .unwrap()
        .lines()
        .map(|line| serde_json::from_str(line).unwrap())
        .collect();
    (
        values
            .iter()
            .filter(|v| v["method"] == "session/new")
            .count(),
        values
            .iter()
            .filter(|v| v["method"] == "session/prompt")
            .count(),
    )
}
#[cfg(unix)]
fn opaque_prompt_refused(client: &mut velune_agent_runtime::acp::Client) -> bool {
    client
        .enqueue(
            velune_agent_runtime::acp::AcpCommand::Prompt {
                text: "must wait for strict association".into(),
                images: Vec::new(),
                run_deadline: None,
            },
            Instant::now() + Duration::from_secs(5),
        )
        .is_err()
}

#[cfg(unix)]
#[test]
fn opaque_real_created_shrinks_credit_preserves_other_id_and_commits_complex_identity() {
    use velune_agent_runtime::creation::CreationOutcome;
    let home = Home::new();
    let (repo, _) = Repository::open(&home.0).unwrap();
    let mut current = vec![link("kept", 1)];
    repo.store_conversation_links(&current).unwrap(); // synthetic setup BEFORE domain claim
    let mut metadata = LinksMetadata::start(
        repo.home.clone(),
        repo.home_lock.claim_links().unwrap(),
        opaque_limits(),
        &current,
        Hooks::default(),
    )
    .unwrap();
    let other_new = link("kept", 2);
    let other = metadata
        .reserve("kept", 4096, LinksGrowth::for_new_link(Some(&other_new)))
        .unwrap();
    let mut new = opaque_skeleton("alias", &home);
    let opaque = metadata.reserve_opaque("alias", None, &new).unwrap();
    assert!(opaque.budget.limits().identity_bytes > 4096);
    assert_eq!(opaque.budget.peak_record_capacity(), 0);
    assert_eq!(
        metadata
            .reserve_opaque("second", None, &opaque_skeleton("second", &home))
            .err(),
        Some(LinksAdmissionError::Busy)
    );
    let identity = "名\"\\\n\0".repeat(768);
    assert!(identity.len() > 4096 && identity.len() <= opaque.budget.limits().identity_bytes);
    let mut client = opaque_client(&home, "hold", &identity);
    let budget = metadata.start_opaque_io(&opaque.reservation).unwrap();
    let native = client
        .enqueue_creation(
            home.0.clone(),
            "synthetic".into(),
            budget.clone(),
            Instant::now() + Duration::from_secs(5),
        )
        .unwrap();
    opaque_until(|| home.0.join("opaque-native-created").exists());
    assert_eq!(
        budget.peak_record_capacity(),
        0,
        "max credit is not max-buffer allocation"
    );
    patch(
        &mut metadata,
        &other,
        Some(link("kept", 1)),
        Some(other_new),
    );
    assert_eq!(
        receipt(&mut metadata, &mut current).outcome,
        LinksOutcome::Patched
    );
    assert_eq!(
        current,
        vec![link("kept", 2)],
        "other ID makes progress while native creation is held"
    );
    assert!(opaque_prompt_refused(&mut client));
    fs::write(home.0.join("opaque-release"), []).unwrap();
    let CreationOutcome::Created(created) = opaque_outcome(&mut client, &native) else {
        panic!("no actual identity");
    };
    assert_eq!(created.native_id(), identity);
    assert!(created.within_policy());
    new.segments[0].native_conversation_id = created.prefixed("synthetic:").unwrap();
    metadata
        .shrink_opaque(&opaque.reservation, &created, &new)
        .unwrap();
    assert!(opaque_prompt_refused(&mut client));
    opaque_until(|| {
        let _ = client.poll();
        client.settle_creation_result(&native).is_ok()
    });
    assert!(!budget.is_in_flight());
    assert!(
        opaque_prompt_refused(&mut client),
        "receive settlement is not file acknowledgement"
    );
    patch(&mut metadata, &opaque.reservation, None, Some(new.clone()));
    assert!(metadata.poll_result(&mut current).is_none());
    assert_eq!(
        current,
        vec![link("kept", 2)],
        "strict result is installed only at actual disk completion"
    );
    assert_eq!(
        receipt(&mut metadata, &mut current).outcome,
        LinksOutcome::Patched
    );
    assert_eq!(current, vec![link("kept", 2), new]);
    assert_eq!(repo.load_conversation_links().unwrap(), current);
    client.complete_creation(&native).unwrap();
    assert_eq!(opaque_wire_counts(&home), (1, 0));
    let reused = metadata
        .reserve_opaque("second", None, &opaque_skeleton("second", &home))
        .unwrap();
    assert_eq!(reused.budget.peak_record_capacity(), 0);
    metadata.release(&reused.reservation).unwrap();
    close(&mut metadata);
    assert!(
        repo.unlock().is_err(),
        "live controlled native owner still retains HOME"
    );
    drop(client);
    opaque_until(|| repo.unlock().is_ok());
    assert!(Repository::lock_home(&home.0).is_ok());
}

#[cfg(unix)]
#[test]
fn opaque_actual_native_io_and_dropped_view_keep_home_and_credit_until_cleanup() {
    let home = Home::new();
    let (repo, _) = Repository::open(&home.0).unwrap();
    let mut metadata = LinksMetadata::start(
        repo.home.clone(),
        repo.home_lock.claim_links().unwrap(),
        opaque_limits(),
        &Vec::new(),
        Hooks::default(),
    )
    .unwrap();
    let opaque = metadata
        .reserve_opaque("alias", None, &opaque_skeleton("alias", &home))
        .unwrap();
    let mut client = opaque_client(&home, "hold", "eventual-identity");
    let budget = metadata.start_opaque_io(&opaque.reservation).unwrap();
    let ticket = client
        .enqueue_creation(
            home.0.clone(),
            "synthetic".into(),
            budget.clone(),
            Instant::now() + Duration::from_secs(5),
        )
        .unwrap();
    opaque_until(|| home.0.join("opaque-native-created").exists());
    // Discard observation, NOT the accepted process/IO owner.
    drop(client.take_receipts());
    drop(opaque.budget);
    metadata.begin_close();
    assert_eq!(
        metadata.release(&opaque.reservation),
        Err(LinksAdmissionError::Busy)
    );
    assert!(!metadata.poll_drained());
    assert!(repo.unlock().is_err());
    drop(repo);
    assert!(Repository::lock_home(&home.0).is_err());
    assert!(client.take_creation_result(&ticket).is_none());
    assert!(budget.is_in_flight());
    assert_eq!(opaque_wire_counts(&home), (1, 0));
    drop(client); // actual child/read/result cleanup, no automatic retry
    opaque_until(|| !budget.is_in_flight());
    metadata.release(&opaque.reservation).unwrap();
    close(&mut metadata);
    assert!(!home.0.join("conversation-links.json").exists());
    assert!(Repository::lock_home(&home.0).is_ok());
}

#[cfg(unix)]
#[test]
fn opaque_policy_failure_retains_created_or_unknown_without_touching_original_association() {
    use velune_agent_runtime::creation::CreationOutcome;
    for incomplete in [false, true] {
        let home = Home::new();
        let (repo, _) = Repository::open(&home.0).unwrap();
        let mut current = vec![link("kept", 1)];
        repo.store_conversation_links(&current).unwrap();
        let original = fs::read(home.0.join("conversation-links.json")).unwrap();
        let mut metadata = LinksMetadata::start(
            repo.home.clone(),
            repo.home_lock.claim_links().unwrap(),
            opaque_limits(),
            &current,
            Hooks::default(),
        )
        .unwrap();
        let mut new = opaque_skeleton("alias", &home);
        let opaque = metadata.reserve_opaque("alias", None, &new).unwrap();
        let identity = "x".repeat(opaque.budget.limits().identity_bytes + 1);
        assert!(identity.len() + 256 < opaque.budget.limits().record_bytes);
        let mut client = opaque_client(
            &home,
            if incomplete { "incomplete" } else { "full" },
            &identity,
        );
        let budget = metadata.start_opaque_io(&opaque.reservation).unwrap();
        let native = client
            .enqueue_creation(
                home.0.clone(),
                "synthetic".into(),
                budget.clone(),
                Instant::now() + Duration::from_secs(5),
            )
            .unwrap();
        let outcome = opaque_outcome(&mut client, &native);
        if incomplete {
            assert!(matches!(outcome, CreationOutcome::Unknown { .. }));
        } else {
            let CreationOutcome::Created(created) = outcome else {
                panic!("full identity must remain recoverable Created");
            };
            assert_eq!(created.native_id(), identity);
            assert!(!created.within_policy());
            new.segments[0].native_conversation_id = created.prefixed("synthetic:").unwrap();
            assert_eq!(
                metadata.shrink_opaque(&opaque.reservation, &created, &new),
                Err(LinksAdmissionError::CreatedOverBudget)
            );
        }
        assert!(metadata.poll_result(&mut current).is_none());
        assert_eq!(current, vec![link("kept", 1)]);
        assert_eq!(
            fs::read(home.0.join("conversation-links.json")).unwrap(),
            original
        );
        assert!(opaque_prompt_refused(&mut client));
        assert_eq!(opaque_wire_counts(&home), (1, 0));
        assert_eq!(
            metadata.release(&opaque.reservation),
            Err(LinksAdmissionError::Busy)
        );
        assert!(Repository::lock_home(&home.0).is_err());
        drop(client);
        opaque_until(|| !budget.is_in_flight());
        metadata.release(&opaque.reservation).unwrap();
        close(&mut metadata);
    }
}

#[cfg(unix)]
#[test]
fn opaque_fence_precedes_handing_out_policy_and_cannot_settle_an_undispatched_policy() {
    let home = Home::new();
    let (repo, _) = Repository::open(&home.0).unwrap();
    let mut current = Vec::new();
    let mut metadata = LinksMetadata::start(
        repo.home.clone(),
        repo.home_lock.claim_links().unwrap(),
        opaque_limits(),
        &current,
        Hooks::default(),
    )
    .unwrap();
    let opaque = metadata
        .reserve_opaque("alias", None, &opaque_skeleton("alias", &home))
        .unwrap();
    // The legitimate order still uses the actual worker and succeeds.
    metadata.enqueue_backup_fence(&opaque.reservation).unwrap();
    assert!(matches!(
        receipt(&mut metadata, &mut current).outcome,
        LinksOutcome::BackupReady(None)
    ));
    let mut client = opaque_client(&home, "full", "must-not-be-created");
    let policy = metadata.start_opaque_io(&opaque.reservation).unwrap();
    assert!(
        !policy.is_in_flight(),
        "worker has policy but has not dispatched yet"
    );
    let bytes = br#"{"schemaVersion":99,"links":[]}"#;
    fs::write(home.0.join("conversation-links.json"), bytes).unwrap();
    let retained = metadata.retained;
    assert_eq!(
        metadata.enqueue_backup_fence(&opaque.reservation),
        Err(LinksAdmissionError::Busy)
    );
    assert!(metadata.poll_result(&mut current).is_none());
    assert_eq!(
        metadata.retained, retained,
        "rejected fence returns no SDK/growth credit"
    );
    assert_eq!(
        metadata
            .reserve_opaque("other", None, &opaque_skeleton("other", &home))
            .err(),
        Some(LinksAdmissionError::Busy)
    );
    assert_eq!(
        metadata.reserve("alias", 4096, LinksGrowth::default()),
        Err(LinksAdmissionError::Busy)
    );
    assert_eq!(
        fs::read(home.0.join("conversation-links.json")).unwrap(),
        bytes
    );
    // Only explicit cancellation can settle an unstarted policy; its CAS
    // also prevents the handed-out policy from starting after settlement.
    metadata.release(&opaque.reservation).unwrap();
    assert!(policy.is_settled());
    assert!(
        client
            .enqueue_creation(
                home.0.clone(),
                "synthetic".into(),
                policy,
                Instant::now() + Duration::from_secs(5)
            )
            .is_err()
    );
    assert_eq!(opaque_wire_counts(&home), (0, 0));
    drop(client);
    close(&mut metadata);
}

#[cfg(unix)]
#[test]
fn opaque_fence_cannot_return_credit_after_real_written_creation_with_held_response() {
    let home = Home::new();
    let (repo, _) = Repository::open(&home.0).unwrap();
    let mut current = Vec::new();
    let mut metadata = LinksMetadata::start(
        repo.home.clone(),
        repo.home_lock.claim_links().unwrap(),
        opaque_limits(),
        &current,
        Hooks::default(),
    )
    .unwrap();
    let opaque = metadata
        .reserve_opaque("alias", None, &opaque_skeleton("alias", &home))
        .unwrap();
    metadata.enqueue_backup_fence(&opaque.reservation).unwrap();
    assert!(matches!(
        receipt(&mut metadata, &mut current).outcome,
        LinksOutcome::BackupReady(None)
    ));
    let mut client = opaque_client(&home, "hold", "created-but-held");
    let policy = metadata.start_opaque_io(&opaque.reservation).unwrap();
    let ticket = client
        .enqueue_creation(
            home.0.clone(),
            "synthetic".into(),
            policy.clone(),
            Instant::now() + Duration::from_secs(5),
        )
        .unwrap();
    opaque_until(|| home.0.join("opaque-native-created").exists());
    let mut written = false;
    opaque_until(|| {
        let _ = client.poll();
        written |= client.take_receipts().iter().any(|receipt| {
            matches!(
                receipt.stage,
                velune_agent_runtime::acp::AcpReceiptStage::Written
            )
        });
        written
    });
    let bytes = br#"{"schemaVersion":99,"links":[]}"#;
    fs::write(home.0.join("conversation-links.json"), bytes).unwrap();
    let retained = metadata.retained;
    assert_eq!(
        metadata.enqueue_backup_fence(&opaque.reservation),
        Err(LinksAdmissionError::Busy)
    );
    assert!(metadata.poll_result(&mut current).is_none());
    assert_eq!(metadata.retained, retained);
    assert_eq!(
        metadata
            .reserve_opaque("other", None, &opaque_skeleton("other", &home))
            .err(),
        Some(LinksAdmissionError::Busy)
    );
    assert_eq!(
        metadata.release(&opaque.reservation),
        Err(LinksAdmissionError::Busy)
    );
    assert!(policy.is_in_flight());
    assert!(client.take_creation_result(&ticket).is_none());
    assert_eq!(opaque_wire_counts(&home), (1, 0));
    metadata.begin_close();
    assert!(!metadata.poll_drained());
    assert!(repo.unlock().is_err());
    drop(client);
    opaque_until(|| !policy.is_in_flight());
    metadata.release(&opaque.reservation).unwrap();
    close(&mut metadata);
    assert_eq!(
        fs::read(home.0.join("conversation-links.json")).unwrap(),
        bytes
    );
}
