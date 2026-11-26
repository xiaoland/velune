use super::*;
use std::os::unix::fs::PermissionsExt;

fn loaded_rpc_history(core: &mut CoreRuntime, root: &Path) -> LoadedHistory {
    let binary = root.join("synthetic-codex");
    fs::write(&binary, include_str!("../../../../agent-runtime/src/creation/cold_history_fixture.py")).unwrap();
    fs::set_permissions(&binary, fs::Permissions::from_mode(0o700)).unwrap();
    let mut instance = runtime(RuntimeKind::Codex166);
    instance.settings.insert("binary".into(), binary.to_string_lossy().into_owned());
    core.runtime_instances.push(instance.clone());
    let source = core.runtime_source_key(&instance).unwrap();
    let config = history::HistoryConfig {
        kind: RuntimeKind::Codex166,
        execution: velune_agent_runtime::execution::ExecutionTarget::Local,
        os_home: Some(root.to_owned()), binary,
        node_binary: PathBuf::new(), resources_directory: root.to_owned(),
        root: root.to_owned(), home: root.to_owned(),
    };
    let read = history::read_codex_with_summary(&config, "synthetic", "synthetic").unwrap();
    let snapshot = read.map(|(summary, read)| {
        let mut snapshot: ConversationSnapshot = serde_json::from_value(json!({
            "revision":1, "contextRuntimeId":"synthetic", "conversation": summary,
            "resourceId":null, "modelRecordKey":null, "runState":"idle", "messages":[],
            "transcriptTurns":[], "transcriptItems":[], "transcriptOutline":[],
            "transcriptMessageIdentities":[],
            "actions":{"canSend":true,"canCancel":false,"canSwitch":true}
        })).unwrap();
        snapshot.messages = read.messages;
        snapshot.executions = read.executions;
        snapshot
    });
    LoadedHistory { source, prepared_resources:None,
        snapshot:HistorySnapshot::from_retained(snapshot), acp_capabilities:None }
}

/// Use real independent reservations to make premature release observable
/// without exposing SDK ledger internals to production application code.
struct SaturatedCredit {
    probe: DataRetention,
    _reservations: Vec<DataRetention>,
}
impl SaturatedCredit {
    fn from_history(history: &HistorySnapshot) -> Self {
        let probe = history.retention.try_clone().unwrap();
        let mut reservations = Vec::new();
        for _ in 0..100_000 {
            match probe.try_clone() {
                Ok(reservation) => reservations.push(reservation),
                Err(_) => return Self { probe, _reservations: reservations },
            }
        }
        panic!("RPC history must retain nonzero, finite data credit; an empty guard never refuses");
    }
    fn assert_still_owned(&self) { assert!(self.probe.try_clone().is_err()); }
    fn assert_returned(&self) { assert!(self.probe.try_clone().is_ok()); }
}

#[test]
fn loaded_history_credit_moves_through_session_and_parked_cache_until_eviction() {
    let (mut app, root) = open();
    let core = &mut app.runtime;
    let loaded = loaded_rpc_history(core, &root);
    let credit = SaturatedCredit::from_history(&loaded.snapshot);
    let snapshot = core.finish_history_read(loaded).unwrap();
    core.present_history(snapshot, None);
    credit.assert_still_owned();
    core.focus_new();
    credit.assert_still_owned();
    assert_eq!(core.parked.len(), 1);
    core.parked[0].history_cache_weight = Some((1, 2, usize::MAX));
    session::trim_history_sessions(&mut core.parked, &mut vec![false], &Default::default());
    assert!(core.parked.is_empty());
    credit.assert_returned();
    drop(credit);
    drop(app);
    fs::remove_dir_all(root).unwrap();
}

#[test]
fn replacing_history_returns_old_credit_not_only_its_temporary_loaded_wrapper() {
    let (mut app, root) = open();
    let core = &mut app.runtime;
    let loaded = loaded_rpc_history(core, &root);
    let credit = SaturatedCredit::from_history(&loaded.snapshot);
    let snapshot = core.finish_history_read(loaded).unwrap();
    core.present_history(snapshot, None);
    credit.assert_still_owned();
    core.session.active_state = ActiveState::Empty;
    credit.assert_returned();
    drop(credit);
    drop(app);
    fs::remove_dir_all(root).unwrap();
}

#[test]
fn unused_loaded_history_returns_transient_credit_without_a_permanent_cache_permit() {
    let (mut app, root) = open();
    let loaded = loaded_rpc_history(&mut app.runtime, &root);
    let credit = SaturatedCredit::from_history(&loaded.snapshot);
    credit.assert_still_owned();
    drop(loaded);
    credit.assert_returned();
    drop(credit);
    drop(app);
    fs::remove_dir_all(root).unwrap();
}

#[test]
fn accepted_read_receipt_updates_open_history_when_summary_credit_is_exhausted() {
    let (mut app, root) = open();
    let core = &mut app.runtime;
    let mut loaded = loaded_rpc_history(core, &root);
    let id = loaded.snapshot.conversation.id.clone();
    let before = velune_conversation::NativeReadState {
        conversation_id: id.clone(),
        revision: "before".into(),
        unread: true,
    };
    loaded.snapshot.snapshot.conversation.native_read_state = Some(before.clone());
    let credit = SaturatedCredit::from_history(&loaded.snapshot);
    let (snapshot, retention) = loaded.snapshot.into_retained().into_parts();
    let (history_charge, summary_charge) = retention.split();
    let summary = SharedSummary::retained(Retained::from_parts(
        snapshot.conversation.clone(),
        summary_charge,
    ));
    core.history_listings.borrow_mut().insert(
        "synthetic".into(),
        HistoryListing {
            source: loaded.source,
            entries: vec![IndexedConversation {
                id,
                identity: "synthetic".into(),
                hint: ConversationHint::default(),
                state: EntryState::Read(summary),
            }],
        },
    );
    core.present_history(
        HistorySnapshot {
            snapshot,
            retention: history_charge,
        },
        None,
    );
    let generation = core.history_index_generation.get();
    let committed = Retained::from_parts(
        velune_conversation::NativeReadState {
            revision: "committed".into(),
            unread: false,
            ..before.clone()
        },
        DataRetention::default(),
    );
    core.keep_native_read_state(&committed);
    assert_eq!(
        core.session
            .projected()
            .unwrap()
            .conversation
            .native_read_state
            .as_ref(),
        Some(committed.as_ref())
    );
    assert_ne!(core.history_index_generation.get(), generation);
    let listing = core.history_listings.borrow();
    let EntryState::Read(cached) = &listing["synthetic"].entries[0].state else {
        panic!("summary still retained with its original credit");
    };
    assert_eq!(cached.native_read_state.as_ref(), Some(&before));
    credit.assert_still_owned();
    drop(listing);
    drop(credit);
    drop(app);
    fs::remove_dir_all(root).unwrap();
}
