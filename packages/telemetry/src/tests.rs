use super::*;
fn open(home: &Path, dev: Option<&Path>) -> Arc<ClientTelemetry> {
    ClientTelemetry::open(
        home,
        TelemetryIdentity::new("test", "test").unwrap(),
        false,
        dev,
    )
    .unwrap()
}
fn profile() -> TelemetryProfile {
    TelemetryProfile {
        endpoint: "http://127.0.0.1:1".into(),
        headers: Default::default(),
        sample_ratio: 1.0,
    }
}
#[test]
fn default_is_inert_off_wins_and_revisions_are_atomic() {
    let dir = tempfile::tempdir().unwrap();
    let owner = open(dir.path(), None);
    assert!(!dir.path().join("telemetry").exists());
    assert!(owner.snapshot().unwrap().rust_active_revision.is_none());
    let pending = owner
        .prepare(0, TelemetryPreference::On, Some(profile()))
        .unwrap();
    assert!(owner.snapshot().unwrap().rust_active_revision.is_none());
    assert!(!dir.path().join("telemetry/config.json").exists());
    assert!(matches!(
        owner.prepare(9, TelemetryPreference::Off, None),
        Err(Error::Conflict)
    ));
    let rev = owner.commit(&pending.id).unwrap();
    assert!(!owner.snapshot().unwrap().applied);
    owner.confirm_native(rev, true).unwrap();
    assert!(owner.snapshot().unwrap().applied);
    let off = owner
        .prepare(rev, TelemetryPreference::Off, Some(profile()))
        .unwrap();
    owner.commit(&off.id).unwrap();
    assert!(!owner.snapshot().unwrap().applied);
    owner.confirm_native(off.revision, true).unwrap();
    assert!(owner.snapshot().unwrap().rust_active_revision.is_none());
    let dev = dir.path().join("dev.json");
    config::write(&dev, &profile()).unwrap();
    drop(owner);
    let reopened = open(dir.path(), Some(&dev));
    assert!(reopened.prepare_saved().unwrap().profile.is_none());
}
#[test]
fn failed_write_preserves_active_revision_and_profile() {
    let dir = tempfile::tempdir().unwrap();
    let owner = open(dir.path(), None);
    let pending = owner
        .prepare(0, TelemetryPreference::On, Some(profile()))
        .unwrap();
    std::fs::create_dir_all(dir.path().join("telemetry/config.json")).unwrap();
    assert!(owner.commit(&pending.id).is_err());
    assert_eq!(owner.snapshot().unwrap().revision, 0);
    assert!(owner.saved_profile().unwrap().is_none());
    owner.cancel_preparation(&pending.id).unwrap();
}
#[test]
fn status_failure_does_not_turn_a_committed_change_into_a_save_failure() {
    let dir = tempfile::tempdir().unwrap();
    let owner = open(dir.path(), None);
    let pending = owner
        .prepare(0, TelemetryPreference::On, Some(profile()))
        .unwrap();
    std::fs::create_dir_all(dir.path().join("telemetry/status.json")).unwrap();
    let revision = owner.commit(&pending.id).unwrap();
    assert_eq!(owner.snapshot().unwrap().revision, revision);
    owner.confirm_native(revision, true).unwrap();
    assert!(owner.snapshot().unwrap().applied);
    assert!(owner.snapshot().unwrap().status_write_failed);
}
#[test]
fn failed_activation_cannot_be_confirmed_and_second_owner_cannot_overwrite() {
    let dir = tempfile::tempdir().unwrap();
    let first = open(dir.path(), None);
    let second = open(dir.path(), None);
    let pending = first
        .prepare(0, TelemetryPreference::On, Some(profile()))
        .unwrap();
    first.commit(&pending.id).unwrap();
    first.confirm_native(pending.revision, false).unwrap();
    assert!(first.confirm_native(pending.revision, true).is_err());
    assert!(!first.snapshot().unwrap().applied);
    assert!(matches!(
        second.prepare(0, TelemetryPreference::Off, None),
        Err(Error::Conflict)
    ));
}
#[test]
fn configuration_io_lock_does_not_block_business_observation_or_profile_read() {
    let dir = tempfile::tempdir().unwrap();
    let owner = open(dir.path(), None);
    let candidate = owner
        .prepare(0, TelemetryPreference::On, Some(profile()))
        .unwrap();
    owner.commit(&candidate.id).unwrap();
    owner.confirm_native(candidate.revision, true).unwrap();
    let guard = owner.state.lock().unwrap();
    let (tx, rx) = std::sync::mpsc::channel();
    let copy = owner.clone();
    let thread = std::thread::spawn(move || {
        let mut op = copy.operation("list", None, false);
        op.complete(Outcome::Success);
        let _ = copy.saved_profile().unwrap();
        tx.send(()).unwrap();
    });
    let result = rx.recv_timeout(Duration::from_millis(200));
    drop(guard);
    thread.join().unwrap();
    assert!(
        result.is_ok(),
        "application telemetry must not wait for configuration storage"
    );
}
#[test]
fn invalid_development_profile_does_not_prevent_business_startup_or_explicit_off() {
    let dir = tempfile::tempdir().unwrap();
    let owner = open(dir.path(), Some(&dir.path().join("missing-profile")));
    assert_eq!(
        owner.snapshot().unwrap().status,
        "development_configuration_unavailable"
    );
    assert!(!owner.snapshot().unwrap().applied);
    let mut operation = owner.operation("list", None, false);
    operation.complete(Outcome::Success);
    let off = owner.prepare(0, TelemetryPreference::Off, None).unwrap();
    owner.commit(&off.id).unwrap();
    owner.confirm_native(off.revision, true).unwrap();
    assert!(owner.snapshot().unwrap().applied);
    assert_eq!(owner.snapshot().unwrap().status, "disabled");
}
#[test]
fn delayed_native_work_cannot_reactivate_a_shutdown_owner() {
    let dir = tempfile::tempdir().unwrap();
    let owner = open(dir.path(), None);
    let pending = owner
        .prepare(0, TelemetryPreference::On, Some(profile()))
        .unwrap();
    owner.commit(&pending.id).unwrap();
    owner.confirm_native(pending.revision, true).unwrap();
    owner.shutdown().unwrap();
    assert!(matches!(owner.prepare_saved(), Err(Error::Closed)));
    assert!(matches!(
        owner.confirm_native(pending.revision, true),
        Err(Error::Closed)
    ));
    assert!(matches!(owner.send_test(&profile()), Err(Error::Closed)));
    assert_eq!(owner.snapshot().unwrap().status, "stopped");
    owner.shutdown().unwrap();
}
#[test]
fn future_storage_is_never_overwritten() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("telemetry/config.json");
    let mut future = Stored::empty().unwrap();
    future.profile = Some(profile());
    config::write(&path, &future).unwrap();
    let current = open_result(dir.path()).unwrap();
    assert!(current.snapshot().unwrap().configuration_available);
    current.shutdown().unwrap();
    drop(current);
    future.version = 99;
    config::write(&path, &future).unwrap();
    let before = std::fs::read(&path).unwrap();
    let owner = open_result(dir.path()).unwrap();
    assert!(!owner.snapshot().unwrap().configuration_available);
    assert!(matches!(
        owner.prepare(0, TelemetryPreference::Off, None),
        Err(Error::StorageFormat)
    ));
    assert_eq!(std::fs::read(path).unwrap(), before);
}
fn open_result(path: &Path) -> Result<Arc<ClientTelemetry>> {
    ClientTelemetry::open(path, TelemetryIdentity::new("test", "test")?, false, None)
}
