use super::*;

fn state_path() -> (tempfile::TempDir, PathBuf) {
    let directory = tempfile::tempdir().unwrap();
    let path = directory.path().join("remote-state.json");
    (directory, path)
}

fn empty_v3_state() -> Vec<u8> {
    serde_json::to_vec(&serde_json::json!({
        "version":3,
        "contacts":{},
        "grants":{},
        "pending":[],
        "reset_notice":null,
        "legacy_reset_recorded":false
    }))
    .unwrap()
}

#[test]
fn unknown_future_version_is_not_reset_or_overwritten() {
    let (_directory, path) = state_path();
    let bytes = br#"{"version":99,"synthetic":"preserve"}"#;
    atomic_write(&path, bytes).unwrap();
    assert!(DeviceStore::open(&path).is_err());
    assert_eq!(std::fs::read(path).unwrap(), bytes);
}

#[test]
fn malformed_or_unknown_state_is_rejected_without_rewrite() {
    for bytes in [
        br#"{"version":4,"contacts":[]"#.as_slice(),
        br#"{"version":4,"contacts":{},"future":"preserve"}"#,
    ] {
        let (_directory, path) = state_path();
        atomic_write(&path, bytes).unwrap();
        assert!(DeviceStore::open(&path).is_err());
        assert_eq!(std::fs::read(path).unwrap(), bytes);
    }
}

#[test]
fn v3_with_v4_fields_is_rejected_without_overwrite_or_backup() {
    let mut profile_grant = serde_json::from_slice::<serde_json::Value>(&empty_v3_state()).unwrap();
    profile_grant["profile_grants"] = serde_json::json!({"synthetic": {}});

    let mut pairing_contact =
        serde_json::from_slice::<serde_json::Value>(&empty_v3_state()).unwrap();
    pairing_contact["contacts"] = serde_json::json!({
        "synthetic": {
            "control_authorized": false,
            "pairing_permission": "telemetryProfileRead"
        }
    });

    for value in [profile_grant, pairing_contact] {
        let (_directory, path) = state_path();
        let source = serde_json::to_vec(&value).unwrap();
        atomic_write(&path, &source).unwrap();

        assert!(DeviceStore::open(&path).is_err());
        assert_eq!(std::fs::read(&path).unwrap(), source);
        assert!(
            !path
                .with_file_name("remote-state.before-beta-v3.json")
                .exists()
        );
    }
}

#[test]
fn interrupted_backup_files_do_not_block_v3_migration_retry() {
    let (_directory, path) = state_path();
    let source = empty_v3_state();
    atomic_write(&path, &source).unwrap();

    let previous_backup = path.with_file_name("remote-state.before-beta-v3.json");
    atomic_write(&previous_backup, b"partial backup").unwrap();
    let abandoned_temporary = path.with_file_name(".remote-state-backup-interrupted.tmp");
    std::fs::write(&abandoned_temporary, b"partial temporary backup").unwrap();

    drop(DeviceStore::open(&path).unwrap());

    assert_eq!(std::fs::read(&previous_backup).unwrap(), b"partial backup");
    assert_eq!(
        std::fs::read(&abandoned_temporary).unwrap(),
        b"partial temporary backup"
    );
    let digest = Sha256::digest(&source);
    let mut suffix = String::new();
    for byte in digest {
        use std::fmt::Write as _;
        write!(&mut suffix, "{byte:02x}").unwrap();
    }
    let complete_backup = path.with_file_name(format!("remote-state.before-beta-v3-{suffix}.json"));
    assert_eq!(std::fs::read(complete_backup).unwrap(), source);
    let migrated: StoredState = serde_json::from_slice(&std::fs::read(&path).unwrap()).unwrap();
    assert_eq!(migrated.version, 6);
}

#[test]
fn known_v3_state_is_backed_up_and_migrated() {
    let (_directory, path) = state_path();
    let peer = crate::identity::DeviceIdentity::generate("Synthetic Peer".into())
        .unwrap()
        .id();
    let incarnation = "1".repeat(32);
    let source = serde_json::to_vec(&serde_json::json!({
            "version":3,
            "contacts":{
                peer.clone(): {
                    "peer_id":peer,
                    "name":"Synthetic Peer",
                    "locator":{"relay_urls":[],"direct_addresses":[],"lookup_urls":[]},
                    "confirmation":{"state":"confirmed","attempt_id":"aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa"},
                    "paused":false,
                    "locator_source":"pairingOffer"
                }
            },
            "grants":{
                peer.clone():{"peer_id":peer.clone(),"name":"Synthetic Peer","attempt_id":"bbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbb"}
            },
            "pending":[{
                "peer_id":peer.clone(),
                "request_id":{"incarnation":incarnation,"client":"cccccccccccccccccccccccccccccccc","sequence":1},
                "view":"synthetic-view","execution":{"expected_revision":null},"kind":"readComposerOwner",
                "runtime_id":null,"conversation_id":null,"status":"recorded"
            }],
            "reset_notice":{"id":"dddddddddddddddddddddddddddddddd"},
            "legacy_reset_recorded":true
        }))
        .unwrap();
    atomic_write(&path, &source).unwrap();
    let store = DeviceStore::open(&path).unwrap();
    let snapshot = store.snapshot().unwrap();
    assert_eq!(snapshot.contacts.len(), 1);
    assert!(snapshot.contacts[0].control_authorized);
    assert_eq!(
        snapshot.contacts[0].pairing_permission,
        PairingPermission::Control
    );
    assert_eq!(snapshot.incoming_grants.len(), 1);
    assert_eq!(snapshot.pending.len(), 1);
    assert_eq!(
        snapshot.reset_notice.as_ref().unwrap().id,
        "dddddddddddddddddddddddddddddddd"
    );
    drop(store);
    assert_eq!(
        std::fs::read(path.with_file_name("remote-state.before-beta-v3.json")).unwrap(),
        source
    );
    let migrated_bytes = std::fs::read(&path).unwrap();
    let migrated: StoredState = serde_json::from_slice(&migrated_bytes).unwrap();
    assert_eq!(migrated.version, 6);
    assert_eq!(migrated.contacts.len(), 1);
    assert_eq!(migrated.grants.len(), 1);
    assert_eq!(migrated.pending.len(), 1);
    assert!(migrated.pending_pairings.is_empty());
    assert!(migrated.profile_grants.is_empty());
    drop(DeviceStore::open(&path).unwrap());
    assert_eq!(std::fs::read(&path).unwrap(), migrated_bytes);
}

#[test]
fn current_state_is_read_without_rewrite_and_missing_state_initializes() {
    let (_directory, path) = state_path();
    let bytes = serde_json::to_vec(&StoredState::default()).unwrap();
    atomic_write(&path, &bytes).unwrap();
    let store = DeviceStore::open(&path).unwrap();
    assert!(store.snapshot().unwrap().contacts.is_empty());
    drop(store);
    assert_eq!(std::fs::read(&path).unwrap(), bytes);

    let (_directory, path) = state_path();
    let store = DeviceStore::open(&path).unwrap();
    assert!(store.snapshot().unwrap().contacts.is_empty());
    let initialized: StoredState = serde_json::from_slice(&std::fs::read(path).unwrap()).unwrap();
    assert_eq!(initialized.version, 6);
}
