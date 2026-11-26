use super::*;

#[test]
fn begun_summary_job_is_running_before_its_first_poll() {
    let request = HandoffSummaryRequest {
        work_id: "synthetic-work".into(),
        source_native_id: "synthetic-thread".into(),
        source_provider_id: "synthetic-provider".into(),
        source_model_id: "synthetic-model".into(),
        selected_model: None,
        effort: None,
    };
    let mut job = Job::new(
        request,
        "velune_handoff_summary:synthetic-work".into(),
        1,
        std::collections::BTreeSet::new(),
    );

    assert!(!job.is_terminal());
    job.update_status();
    assert!(matches!(
        job.status,
        HandoffSummaryStatus::Running {
            phase: HandoffSummaryPhase::ReadingSource,
            ..
        }
    ));
    assert!(job.artifact().is_none());
}

#[test]
fn terminal_artifact_receipt_distinguishes_not_created_archived_and_maybe_exists() {
    let request = || HandoffSummaryRequest {
        work_id: "synthetic-work".into(),
        source_native_id: "synthetic-thread".into(),
        source_provider_id: "synthetic-provider".into(),
        source_model_id: "synthetic-model".into(),
        selected_model: None,
        effort: None,
    };
    let mut not_created = Job::new(
        request(),
        "velune_handoff_summary:synthetic-work".into(),
        1,
        std::collections::BTreeSet::new(),
    );
    not_created.failure = Some(("synthetic".into(), "before fork".into()));
    not_created.finish();
    assert_eq!(
        not_created.artifact().unwrap().state,
        HandoffSummaryArtifactState::NotCreated
    );

    let mut maybe_exists = Job::new(
        request(),
        "velune_handoff_summary:synthetic-work".into(),
        1,
        std::collections::BTreeSet::new(),
    );
    maybe_exists.fork_requested = true;
    maybe_exists.cancelled = true;
    maybe_exists.finish();
    let receipt = maybe_exists.artifact().unwrap();
    assert_eq!(receipt.state, HandoffSummaryArtifactState::MayExist);
    assert_eq!(receipt.temporary_thread_id, None);

    let mut archived = Job::new(
        request(),
        "velune_handoff_summary:synthetic-work".into(),
        1,
        std::collections::BTreeSet::new(),
    );
    archived.fork_requested = true;
    archived.cleanup_state = HandoffSummaryCleanupState::Archived;
    archived.failure = Some((
        "synthetic".into(),
        "finished with an earlier failure".into(),
    ));
    archived.finish();
    assert_eq!(
        archived.artifact().unwrap().state,
        HandoffSummaryArtifactState::Archived
    );
}

#[test]
fn post_fork_dispatch_error_keeps_failed_job_and_maybe_exists_receipt() {
    let request = HandoffSummaryRequest {
        work_id: "synthetic-work".into(),
        source_native_id: "synthetic-source".into(),
        source_provider_id: "synthetic-provider".into(),
        source_model_id: "synthetic-model".into(),
        selected_model: None,
        effort: None,
    };
    let mut job = Job::new(
        request,
        "velune_handoff_summary:synthetic-work".into(),
        1,
        std::collections::BTreeSet::new(),
    );
    job.fork_requested = true;
    job.temporary_thread_id = Some("synthetic-fork".into());
    job.settle_error(&Error::with_code("synthetic dispatch failure", "transport"));

    assert!(matches!(
        job.status,
        HandoffSummaryStatus::Failed { ref code, .. } if code == "transport"
    ));
    let receipt = job.artifact().expect("terminal job keeps its receipt");
    assert_eq!(receipt.state, HandoffSummaryArtifactState::MayExist);
    assert_eq!(
        receipt.temporary_thread_id.as_deref(),
        Some("synthetic-fork")
    );
}
