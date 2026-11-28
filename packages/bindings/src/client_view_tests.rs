use super::*;
use std::{collections::BTreeMap, fs};
use velune_test_support::TempRoot;

struct Fixture(TempRoot);
impl Fixture {
    fn new() -> Self {
        let root = TempRoot::new("binding-views");
        fs::write(
            root.join("huihua_sessions.mjs"),
            r#"
import json, sys
request = json.load(sys.stdin)
print(json.dumps({'contractVersion':1,'sessions':[
 {'nativeId':f'row-{row}','title':{'source':'native','text':f'Archive fixture {row}'},
  'cwd':'/synthetic/project','updatedAtUnixMs':100000-row,'createdAtUnixMs':1}
 for row in range(8)]}))
"#,
        )
        .unwrap();
        let runtime = RuntimeInstance {
            enabled: true,
            id: "synthetic".into(),
            name: "Synthetic".into(),
            type_id: RuntimeKind::DeepSeek,
            gateway_id: "default".into(),
            settings: BTreeMap::from([
                ("binary".into(), "/synthetic/unused-agent".into()),
                ("nodeBinary".into(), "/usr/bin/python3".into()),
                (
                    "agentDir".into(),
                    root.join("state").to_string_lossy().into_owned(),
                ),
            ]),
        };
        let blank = VeluneApplication::open(BindingOptions {
            home_directory: root.to_string_lossy().into_owned(),
            resources_directory: root.to_string_lossy().into_owned(),
        })
        .unwrap();
        blank
            .set_conversation_browser_limits(ConversationBrowserLimits { page: 2, total: 2 })
            .unwrap();
        blank.shutdown().unwrap();
        let path = root.join("generic-config.json");
        let mut config: serde_json::Value =
            serde_json::from_slice(&fs::read(&path).unwrap()).unwrap();
        config["runtimeInstances"] = serde_json::json!([runtime]);
        config["gateways"] = serde_json::json!([{"id":"default","name":"Default","providers":[]}]);
        fs::write(path, serde_json::to_vec(&config).unwrap()).unwrap();
        Self(root)
    }
    fn open(&self) -> Arc<VeluneApplication> {
        VeluneApplication::open(BindingOptions {
            home_directory: self.0.to_string_lossy().into_owned(),
            resources_directory: self.0.to_string_lossy().into_owned(),
        })
        .unwrap()
    }
}
fn reference(row: usize) -> ConversationReference {
    ConversationReference {
        runtime_id: "synthetic".into(),
        conversation_id: format!("synthetic:row-{row}"),
        native_read_state: None,
    }
}
fn ids(snapshot: &ConfigurationSnapshot) -> Vec<String> {
    snapshot
        .conversations
        .iter()
        .map(|row| row.conversation.id.clone())
        .collect()
}

#[test]
fn independent_client_views_share_owner_without_sharing_browser_state() {
    let fixture = Fixture::new();
    let main = fixture.open();
    let initial = main.list().unwrap();
    assert_eq!(initial.conversations.len(), 2);
    // Archive more than one page using the real organization owner.
    main.organize_conversations(
        (0..5).map(reference).collect(),
        ConversationOrganizationChange::Archive,
    )
    .unwrap();
    let main_before = main.list().unwrap();
    let archive = main.with_client_view("archive-window".into()).unwrap();
    assert!(
        archive
            .show_archived_conversations(true)
            .unwrap()
            .shows_archived
    );
    let first = archive.list().unwrap();
    assert_eq!(first.conversations.len(), 2);
    assert!(first.conversations.iter().all(|row| row.archived));
    assert!(!main.list().unwrap().shows_archived);
    let key = first.conversation_groups[0].key.clone();
    assert_eq!(
        archive
            .load_more_conversations(key)
            .unwrap()
            .conversations
            .len(),
        4
    );
    assert_eq!(ids(&main.list().unwrap()), ids(&main_before));
    assert_eq!(
        archive
            .search_conversations("fixture 4".into())
            .unwrap()
            .conversations
            .len(),
        1
    );
    assert!(
        main.search_conversations("fixture 4".into())
            .unwrap()
            .conversations
            .is_empty()
    );
    let mut preferences = first.conversation_browser_preferences.clone();
    preferences.runtime_id = Some("missing-runtime".into());
    archive
        .set_conversation_browser_preferences(preferences)
        .unwrap();
    assert!(archive.list().unwrap().conversations.is_empty());
    assert_eq!(
        main.list()
            .unwrap()
            .conversation_browser_preferences
            .runtime_id,
        None
    );
    archive
        .set_conversation_browser_preferences(first.conversation_browser_preferences)
        .unwrap();
    let traced = archive.with_trace_context(None).unwrap();
    assert!(traced.list().unwrap().shows_archived);
    let composed = traced.with_composer_authority(Some(main.clone())).unwrap();
    assert!(composed.list().unwrap().shows_archived);
    archive
        .organize_conversations(
            vec![reference(0)],
            ConversationOrganizationChange::Unarchive,
        )
        .unwrap();
    assert!(ids(&main.list().unwrap()).contains(&reference(0).conversation_id));
    assert!(!ids(&archive.list().unwrap()).contains(&reference(0).conversation_id));
    // Unsupported native deletion must preserve both views and the source record.
    assert!(
        archive
            .delete_conversation("synthetic".into(), reference(1).conversation_id)
            .is_err()
    );
    assert_eq!(
        archive
            .search_conversations("fixture 1".into())
            .unwrap()
            .conversations
            .len(),
        1
    );
    archive.release_view().unwrap();
    assert_eq!(ids(&main.list().unwrap()).len(), 2);
    // Defensive shutdown by a generic transport must never close the owner.
    archive.shutdown_stopping_work().unwrap();
    assert!(archive.list().is_err());
    assert!(main.list().is_ok());
    assert!(composed.list().is_err());
    assert!(main.with_client_view(" ".into()).is_err());
    assert!(main.with_client_view("x".repeat(129)).is_err());
    main.shutdown().unwrap();
}

#[test]
fn derived_preferences_never_replace_main_defaults_across_reopen() {
    let fixture = Fixture::new();
    let main = fixture.open();
    let mut preferred = main.list().unwrap().conversation_browser_preferences;
    preferred.oldest_first = true;
    main.set_conversation_browser_preferences(preferred.clone())
        .unwrap();
    let before = fs::read(fixture.0.join("generic-config.json")).unwrap();
    let archive = main.with_client_view("archive-preferences".into()).unwrap();
    archive.show_archived_conversations(true).unwrap();
    let mut filter = preferred.clone();
    filter.oldest_first = false;
    filter.runtime_id = Some("missing-runtime".into());
    archive
        .set_conversation_browser_preferences(filter)
        .unwrap();
    archive
        .set_conversation_browser_limits(ConversationBrowserLimits { page: 1, total: 1 })
        .unwrap();
    assert_eq!(
        before,
        fs::read(fixture.0.join("generic-config.json")).unwrap()
    );
    archive.release_view().unwrap();
    archive.shutdown().unwrap();
    main.shutdown().unwrap();
    let reopened = fixture.open();
    let snapshot = reopened.list().unwrap();
    assert_eq!(snapshot.conversation_browser_preferences, preferred);
    assert_eq!(
        snapshot.conversation_browser_limits,
        ConversationBrowserLimits { page: 2, total: 2 }
    );
    assert!(!snapshot.shows_archived);
    reopened.shutdown().unwrap();
}

#[cfg(feature = "remote")]
#[test]
fn remote_derived_view_reuses_connection_and_releases_only_its_browser() {
    use velune_remote::{DiscoveryConfig, PathPolicy, RelayConfig};
    let fixture = Fixture::new();
    let application = fixture.open();
    application.list().unwrap();
    application
        .organize_conversations(vec![reference(0)], ConversationOrganizationChange::Archive)
        .unwrap();
    let options = |name: &str| BindingRemoteOptions {
        identity_file: fixture
            .0
            .join(format!("{name}-identity"))
            .to_string_lossy()
            .into_owned(),
        state_file: fixture
            .0
            .join(format!("{name}-remote-state.json"))
            .to_string_lossy()
            .into_owned(),
        device_name: format!("Synthetic {name}"),
        relay: RelayConfig::Disabled,
        discovery: DiscoveryConfig::None,
        path_policy: PathPolicy::Automatic,
        bind_addresses: vec!["127.0.0.1:0".into()],
        request_timeout_ms: 5000,
    };
    let telemetry = Arc::new(VeluneTelemetry {
        owner: application.diagnostics.telemetry.clone(),
    });
    let host = VeluneRemoteEndpoint::open_with_telemetry(
        options("host"),
        Some(application.clone()),
        telemetry.clone(),
    )
    .unwrap();
    let client =
        VeluneRemoteEndpoint::open_with_telemetry(options("client"), None, telemetry).unwrap();
    let offer = host.create_invitation(60).unwrap();
    let pending = client.begin_pairing(offer.encoded).unwrap();
    host.approve_pairing(pending.attempt_id, client.descriptor().unwrap().endpoint_id)
        .unwrap();
    let peer = host.descriptor().unwrap().endpoint_id;
    client.pairing_status(peer.clone()).unwrap();
    let main = client.connect(peer.clone(), "remote-main".into()).unwrap();
    let epoch = client
        .controller_status(peer.clone())
        .unwrap()
        .connection_epoch;
    let archive = main.with_client_view("remote-archive".into()).unwrap();
    assert_eq!(
        client
            .controller_status(peer.clone())
            .unwrap()
            .connection_epoch,
        epoch
    );
    archive.show_archived_conversations(true).unwrap();
    assert_eq!(
        ids(&archive.list().unwrap()),
        [reference(0).conversation_id]
    );
    assert!(!main.list().unwrap().shows_archived);
    assert!(
        main.search_conversations("fixture 0".into())
            .unwrap()
            .conversations
            .is_empty()
    );
    assert_eq!(
        archive
            .search_conversations("fixture 0".into())
            .unwrap()
            .conversations
            .len(),
        1
    );
    archive.release_view().unwrap();
    archive.shutdown().unwrap();
    assert!(!main.list().unwrap().shows_archived);
    assert!(application.list().is_ok());
    main.shutdown().unwrap();
    client.shutdown().unwrap();
    host.shutdown().unwrap();
    application.shutdown().unwrap();
}

#[test]
fn native_delete_and_unarchive_are_visible_across_live_views() {
    use std::os::unix::fs::PermissionsExt;
    let fixture = Fixture::new();
    let binary = fixture.0.join("codex");
    fs::write(&binary, r#"#!/usr/bin/env python3
import json, pathlib, sys
root = pathlib.Path(__file__).parent
state_path = root / 'native-threads.json'
if '--version' in sys.argv:
    print('codex-cli 0.166.0')
    sys.exit(0)
for line in sys.stdin:
    request = json.loads(line)
    if 'id' not in request: continue
    state = json.loads(state_path.read_text())
    method, params = request['method'], request.get('params', {})
    result = {}
    if method == 'thread/list':
        result = {'data':[row for row in state if row['archived'] == params['archived']], 'nextCursor':None}
    elif method == 'project/list':
        result = {'data':[], 'nextCursor':None}
    elif method == 'thread/read':
        result = {'thread':next(row for row in state if row['id'] == params['threadId'])}
    elif method == 'thread/delete' and not (root / 'retain-deleted').exists():
        state = [row for row in state if row['id'] != params['threadId']]
        state_path.write_text(json.dumps(state))
    elif method == 'thread/unarchive':
        for row in state:
            if row['id'] == params['threadId']: row['archived'] = False
        state_path.write_text(json.dumps(state))
    print(json.dumps({'id':request['id'], 'result':result}), flush=True)
"#).unwrap();
    fs::set_permissions(&binary, fs::Permissions::from_mode(0o700)).unwrap();
    fs::write(fixture.0.join("native-threads.json"), serde_json::to_vec(&serde_json::json!([
        {"id":"active","name":"Active","cwd":"/synthetic/project","updatedAt":10,"createdAt":1,"archived":false},
        {"id":"restore","name":"Restore","cwd":"/synthetic/project","updatedAt":9,"createdAt":1,"archived":true},
        {"id":"delete","name":"Delete","cwd":"/synthetic/project","updatedAt":8,"createdAt":1,"archived":true}
    ])).unwrap()).unwrap();
    let path = fixture.0.join("generic-config.json");
    let mut config: serde_json::Value = serde_json::from_slice(&fs::read(&path).unwrap()).unwrap();
    let runtime = &mut config["runtimeInstances"][0];
    runtime["typeId"] = serde_json::to_value(RuntimeKind::Codex166).unwrap();
    runtime["settings"]["binary"] = binary.to_string_lossy().into_owned().into();
    fs::write(path, serde_json::to_vec(&config).unwrap()).unwrap();
    let main = fixture.open();
    let archive = main.with_client_view("native-archive".into()).unwrap();
    let initial = main.list().unwrap();
    assert!(
        initial.history_failures.is_empty(),
        "{:?}",
        initial.history_failures
    );
    assert_eq!(ids(&initial), ["synthetic:active"]);
    assert_eq!(
        archive
            .show_archived_conversations(true)
            .unwrap()
            .conversations
            .len(),
        2
    );
    archive
        .organize_conversations(
            vec![ConversationReference {
                runtime_id: "synthetic".into(),
                conversation_id: "synthetic:restore".into(),
                native_read_state: None,
            }],
            ConversationOrganizationChange::Unarchive,
        )
        .unwrap();
    assert!(ids(&main.list().unwrap()).contains(&"synthetic:restore".into()));
    // An acknowledgement alone is not proof: the native owner may retain it.
    fs::write(fixture.0.join("retain-deleted"), []).unwrap();
    assert!(
        archive
            .delete_conversation("synthetic".into(), "synthetic:delete".into())
            .is_err()
    );
    assert!(ids(&archive.list().unwrap()).contains(&"synthetic:delete".into()));
    fs::remove_file(fixture.0.join("retain-deleted")).unwrap();
    archive
        .delete_conversation("synthetic".into(), "synthetic:delete".into())
        .unwrap();
    assert!(archive.list().unwrap().conversations.is_empty());
    assert!(
        main.find_conversation("synthetic:delete".into())
            .unwrap()
            .is_none()
    );
    let native: serde_json::Value =
        serde_json::from_slice(&fs::read(fixture.0.join("native-threads.json")).unwrap()).unwrap();
    assert_eq!(native.as_array().unwrap().len(), 2);
    archive.release_view().unwrap();
    assert_eq!(main.list().unwrap().conversations.len(), 2);
    main.shutdown().unwrap();
}
