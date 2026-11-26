use super::*;
use std::path::PathBuf;

fn ready_work() -> HandoffSummaryWork {
    let history = velune_agent_runtime::history::HistoryConfig {
        kind: crate::conversation::RuntimeKind::Codex166,
        execution: velune_agent_runtime::execution::ExecutionTarget::Local,
        os_home: None,
        binary: PathBuf::from("/synthetic/codex"),
        node_binary: PathBuf::new(),
        resources_directory: PathBuf::from("/synthetic/resources"),
        root: PathBuf::from("/synthetic/codex/sessions"),
        home: PathBuf::from("/synthetic/codex"),
    };
    let native = velune_agent_runtime::native::NativeConfig {
        execution: velune_agent_runtime::execution::ExecutionTarget::Local,
        os_home: None,
        kind: velune_agent_runtime::native::NativeKind::Codex166,
        binary: PathBuf::from("/synthetic/codex"),
        node_binary: None,
        agent_dir: PathBuf::from("/synthetic/codex"),
        resources_directory: PathBuf::from("/synthetic/resources"),
        projection_directory: PathBuf::from("/synthetic/projection"),
        gateway: None,
        mcp_servers: Vec::new(),
        session_agents: Vec::new(),
    };
    HandoffSummaryWork {
        started_at: Instant::now(),
        dispatch: tracing::dispatcher::get_default(Clone::clone),
        span: tracing::Span::none(),
        queued: None,
        source_anchor: Some(None),
        requirement_id: "requirement-1".into(),
        source_runtime_id: "codex".into(),
        conversation_id: "codex:thread-1".into(),
        target_runtime_id: "pi".into(),
        status: velune_application_api::HandoffSummaryStatus::Ready {
            prepared_token: "token-1".into(),
        },
        session: None,
        summary: Some(Arc::new(Retained::from_parts(
            PreparedHandoffSummary {
                text: "synthetic portable summary".into(),
                source_fingerprint: "source-fingerprint".into(),
                source_turn_id: "turn-1".into(),
            },
            DataRetention::default(),
        ))),
        prepared_token: Some("token-1".into()),
        bound_request_digest: None,
        delivery_accepted: false,
        send_pending: false,
        source: crate::local::handoff_summary::HandoffRequirementSource {
            physical_runtime_id: "codex".into(),
            history,
            native,
            native_id: "thread-1".into(),
        },
        facts: Arc::new(Retained::from_parts(
            HandoffRequirementFacts {
                required: true,
                source_provider_id: Some("openai".into()),
                source_model_id: Some("gpt-6-sol".into()),
                models: Vec::new(),
                default_model_issue: None,
            },
            DataRetention::default(),
        )),
        summary_model: None,
        effort: None,
        cancel_requested: false,
        expires: Some(Instant::now() + HANDOFF_RESULT_RETENTION),
    }
}

fn requirement(expires: Instant, model_id: &str) -> HandoffRequirementEntry {
    HandoffRequirementEntry {
        source_runtime_id: "codex".into(),
        conversation_id: "logical:conversation".into(),
        target_runtime_id: "pi".into(),
        status: velune_application_api::HandoffRequirements::PortableSummaryRequired {
            summary_basis: Box::new(velune_application_api::NextTurnInput {
                runtime_id: "codex".into(),
                runtime_type: None,
                providers: Vec::new(),
                catalog: Some(Vec::new()),
                turn_settings: Vec::new(),
                conversation_runtime_id: None,
                model_record_key: None,
                kept: None,
                runtime_choices: None,
                new_conversation: Default::default(),
            }),
            default_model_issue: None,
        },
        expires,
        source: None,
        facts: Some(Arc::new(Retained::from_parts(
            HandoffRequirementFacts {
                required: true,
                source_provider_id: Some("openai".into()),
                source_model_id: Some(model_id.into()),
                models: Vec::new(),
                default_model_issue: None,
            },
            DataRetention::default(),
        ))),
    }
}

#[test]
fn active_summary_and_pending_delivery_outlive_the_result_retention_window() {
    let now = Instant::now();
    let later = now + HANDOFF_RESULT_RETENTION + Duration::from_secs(1);
    let mut work = ready_work();
    work.expires = Some(now);
    assert!(!work.is_retained(later));
    work.send_pending = true;
    assert!(work.is_retained(later));
    work.send_pending = false;
    work.status = velune_application_api::HandoffSummaryStatus::Running;
    work.expires = None;
    assert!(work.is_retained(later));
}

#[test]
fn cancelling_ready_summary_revokes_token_idempotently() {
    let now = Instant::now();
    let mut work = ready_work();
    assert!(revoke_prepared_handoff(&mut work, now));
    assert!(work.summary.is_none());
    assert!(work.prepared_token.is_none());
    assert!(matches!(
        work.status,
        velune_application_api::HandoffSummaryStatus::Cancelled
    ));
    assert!(!revoke_prepared_handoff(&mut work, now));
}

#[test]
fn cancelling_inflight_prepared_send_blocks_late_context_completion() {
    let mut work = ready_work();
    work.send_pending = true;
    work.cancel_requested = true;
    let now = Instant::now() - Duration::from_secs(3);
    assert!(cancel_late_handoff_result(&mut work, now));
    assert!(work.summary.is_none());
    assert!(work.prepared_token.is_none());
    assert!(!work.cancel_requested);
    assert!(matches!(
        work.status,
        velune_application_api::HandoffSummaryStatus::Cancelled
    ));
}

#[test]
fn accepted_handoff_cannot_be_revoked_by_summary_cancel() {
    let mut work = ready_work();
    work.delivery_accepted = true;
    assert!(!revoke_prepared_handoff(&mut work, Instant::now()));
    work.cancel_requested = true;
    assert!(!cancel_late_handoff_result(&mut work, Instant::now()));
    assert_eq!(work.prepared_token.as_deref(), Some("token-1"));
}

#[test]
fn recovery_excludes_running_and_send_pending_summary_work() {
    let mut running = ready_work();
    running.status = velune_application_api::HandoffSummaryStatus::Running;
    let mut sending = ready_work();
    sending.send_pending = true;
    let mut starts = BTreeMap::new();
    starts.insert(1, "start-pending".into());
    let works = BTreeMap::from([
        ("running-summary".to_owned(), running),
        ("send-pending".to_owned(), sending),
        ("ready-summary".to_owned(), ready_work()),
    ]);
    let active = active_handoff_summary_ids(&starts, &works);
    assert!(active.contains("start-pending"));
    assert!(active.contains("running-summary"));
    assert!(active.contains("send-pending"));
    assert!(!active.contains("ready-summary"));
}

#[test]
fn begin_uses_the_latest_source_model_and_catalog_facts() {
    let now = Instant::now();
    let entries = BTreeMap::from([
        (
            "old-check".into(),
            requirement(now + Duration::from_secs(30), "old-model"),
        ),
        (
            "fresh-check".into(),
            requirement(now + Duration::from_secs(180), "new-model"),
        ),
    ]);
    let selected =
        latest_summary_requirement_id(&entries, "codex", "logical:conversation", "pi", now)
            .expect("latest requirements");
    assert_eq!(selected, "fresh-check");
    assert_eq!(
        entries[&selected]
            .facts
            .as_ref()
            .and_then(|facts| facts.source_model_id.as_deref()),
        Some("new-model")
    );
}
