use super::*;

struct Fixture {
    application: crate::Application,
    root: std::path::PathBuf,
}
impl Fixture {
    fn new() -> Self {
        let mut nonce = [0; 8];
        getrandom::fill(&mut nonce).unwrap();
        let root = std::env::temp_dir().join(format!(
            "velune-submission-receipt-{:x}",
            u64::from_le_bytes(nonce)
        ));
        std::fs::create_dir(&root).unwrap();
        let mut application = crate::Application::open(crate::Options {
            home_directory: root.clone(),
            resources_directory: root.clone(),
        })
        .unwrap();
        let core = &mut application.runtime;
        core.runtime_instances = vec![RuntimeInstance {
            enabled: true,
            id: "runtime".into(),
            name: "Synthetic ACP".into(),
            type_id: RuntimeKind::Acp,
            gateway_id: String::new(),
            settings: std::collections::BTreeMap::from([(
                "binary".into(),
                root.join("nonexistent-synthetic-agent")
                    .to_string_lossy()
                    .into_owned(),
            )]),
        }];
        let snapshot = serde_json::from_value(serde_json::json!({"revision":1,"conversation":{"id":"runtime:synthetic","runtimeId":"runtime","title":{"source":"untitled"}},"contextRuntimeId":"runtime","resourceId":null,"modelRecordKey":null,"runState":"idle","messages":[],"transcriptTurns":[],"transcriptItems":[],"transcriptOutline":[],"transcriptMessageIdentities":[],"actions":{"canSend":true,"canCancel":false,"canSwitch":true}})).unwrap();
        core.session.active_state = ActiveState::History(Box::new(snapshot));
        Self { application, root }
    }
    fn add(&mut self, token: &str) -> SnapshotResult {
        self.application
            .runtime
            .send_queue_submission(
                "runtime:synthetic",
                SendQueueOperation::Add {
                    source_runtime_id: "runtime".into(),
                    target_runtime_id: "runtime".into(),
                    selection: None,
                    turn: ComposedTurn {
                        text: "same input".into(),
                        effort: None,
                        permission_mode: None,
                        attachments: vec![],
                    },
                },
                Some(token.into()),
            )
            .unwrap()
    }
}
impl Drop for Fixture {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.root);
    }
}
fn queued(result: &SnapshotResult) -> String {
    match &result.submission_receipt {
        Some(crate::SubmissionReceipt::Queued { queue_id }) => queue_id.clone(),
        other => panic!("expected queue receipt, got {other:?}"),
    }
}
#[test]
fn submission_receipts_bind_identical_inputs_to_exact_queue_and_removal() {
    let mut fixture = Fixture::new();
    fixture
        .application
        .runtime
        .follow_finished_turn(crate::TurnOutcome::Cancelled);
    let a = fixture.add("client-a");
    let a_id = queued(&a);
    let b = fixture.add("client-b");
    let b_id = queued(&b);
    assert_ne!(a_id, b_id);
    let view = b.snapshot.unwrap();
    assert_eq!(
        view.follow_up
            .queue
            .iter()
            .map(|q| q.submission_id.as_deref())
            .collect::<Vec<_>>(),
        [Some("client-a"), Some("client-b")]
    );
    let core = &mut fixture.application.runtime;
    let removed = core
        .send_queue(
            "runtime:synthetic",
            SendQueueOperation::Remove { id: a_id.clone() },
        )
        .unwrap();
    assert!(removed.submission_receipt.is_none());
    let poll = core.snapshot_result().unwrap().snapshot.unwrap();
    assert_eq!(poll.follow_up.queue.len(), 1);
    assert_eq!(poll.follow_up.queue[0].id, b_id);
    assert!(
        core.send_queue("runtime:synthetic", SendQueueOperation::Remove { id: a_id })
            .is_err()
    );
    assert_eq!(
        core.snapshot_result()
            .unwrap()
            .snapshot
            .unwrap()
            .follow_up
            .queue
            .len(),
        1
    );
}
#[test]
fn submission_receipt_retains_queue_identity_when_idle_dispatch_is_rejected() {
    let mut fixture = Fixture::new();
    let result = fixture.add("dispatch-rejected");
    let id = queued(&result);
    let view = result.snapshot.unwrap();
    assert_eq!(view.follow_up.queue[0].id, id);
    assert_eq!(
        view.follow_up.queue[0].submission_id.as_deref(),
        Some("dispatch-rejected")
    );
    assert_eq!(view.follow_up.paused, Some(QueuePause::SendFailed));
}
#[test]
fn submission_receipt_preserves_admission_without_a_presentable_snapshot() {
    let mut fixture = Fixture::new();
    fixture
        .application
        .runtime
        .follow_finished_turn(crate::TurnOutcome::Cancelled);
    let result = fixture.add("accepted-no-view");
    let id = queued(&result);
    let core = &mut fixture.application.runtime;
    core.session.active_state = ActiveState::Empty;
    let receipt = core
        .accepted_submission_result(crate::SubmissionReceipt::Queued {
            queue_id: id.clone(),
        })
        .unwrap();
    assert!(receipt.snapshot.is_none());
    assert_eq!(queued(&receipt), id);
    assert_eq!(
        core.session.send_queue.turns[0]
            .request
            .submission_id
            .as_deref(),
        Some("accepted-no-view")
    );
}
