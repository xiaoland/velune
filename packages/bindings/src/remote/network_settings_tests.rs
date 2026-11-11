use super::*;
use velune_remote::{CredentialEdit, NetworkApplicationPhase};

struct TestDirectory(std::path::PathBuf);
impl TestDirectory {
    fn new() -> Self {
        let nonce = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_nanos();
        let path = std::env::temp_dir().join(format!(
            "velune-relay-bindings-{}-{nonce}",
            std::process::id()
        ));
        std::fs::create_dir(&path).unwrap();
        Self(path)
    }
}
impl Drop for TestDirectory {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.0);
    }
}

#[test]
fn relay_editor_preserves_other_settings_and_private_credentials_across_reopen() {
    let directory = TestDirectory::new();
    let options = BindingRemoteOptions {
        identity_file: directory.0.join("identity").to_str().unwrap().into(),
        state_file: directory.0.join("state.json").to_str().unwrap().into(),
        device_name: "Synthetic relay editor".into(),
        relay: RelayConfig::Disabled,
        discovery: DiscoveryConfig::CustomPkarr {
            urls: vec!["https://lookup.invalid/".into()],
        },
        path_policy: PathPolicy::Automatic,
        bind_addresses: vec!["127.0.0.1:0".into()],
        request_timeout_ms: 1_234,
    };
    let endpoint = VeluneRemoteEndpoint::open(options.clone(), None).unwrap();
    let identity = endpoint.owner.service.descriptor().unwrap().endpoint_id;
    let initial = endpoint.read_network_state().unwrap().saved.unwrap();
    let prepared = endpoint
        .owner
        .runtime
        .block_on(endpoint.owner.service.apply_network(NetworkSettingsUpdate {
            expected_revision: initial.revision,
            config: EndpointConfig {
                relay: RelayConfig::Disabled,
                relay_credentials: Vec::new(),
                discovery: DiscoveryConfig::None,
                local_discovery: false,
                path_policy: PathPolicy::Automatic,
                bind_addresses: vec!["127.0.0.1:0".parse().unwrap()],
                request_timeout: Duration::from_millis(1_234),
            },
            credentials: Vec::new(),
            restore_lookup_urls: Vec::new(),
        }))
        .unwrap()
        .saved
        .unwrap();
    let relay_url = "https://relay.invalid/";
    let token = "SYNTHETIC_BINDINGS_RELAY_CREDENTIAL";
    let applied = endpoint
        .apply_relay_settings(
            prepared.revision,
            RelayConfig::Custom {
                urls: vec![relay_url.into()],
            },
            vec![RelayCredentialEdit {
                url: relay_url.into(),
                credential: CredentialEdit::Set {
                    value: token.into(),
                },
            }],
        )
        .unwrap();
    assert!(applied.phase == NetworkApplicationPhase::Applied);
    let safe_json = serde_json::to_string(&applied).unwrap();
    assert!(!safe_json.contains(token));
    let saved = applied.saved.unwrap();
    assert!(matches!(saved.discovery, DiscoveryConfig::None));
    assert!(!saved.local_discovery);
    assert_eq!(saved.request_timeout_ms, 1_234);
    assert_eq!(saved.bind_addresses, prepared.bind_addresses);
    assert_eq!(saved.withdrawn_lookup_urls, prepared.withdrawn_lookup_urls);
    assert_eq!(saved.withdrawn_lookup_urls, ["https://lookup.invalid/"]);
    assert_eq!(saved.credentials.len(), 1);
    assert!(saved.credentials[0].configured);

    // A stale editor cannot retire the active generation or replace its credentials.
    let generation = endpoint.read_network_state().unwrap().generation;
    let stale = endpoint.apply_relay_settings(prepared.revision, RelayConfig::Disabled, Vec::new());
    assert!(matches!(
        stale,
        Err(BindingError::Diagnostic {
            kind: BindingFailureKind::Conflict,
            ..
        })
    ));
    assert_eq!(
        endpoint.read_network_state().unwrap().generation,
        generation
    );

    // Keep is resolved inside Rust; the editor never needs to read the bearer back.
    endpoint
        .apply_relay_settings(
            saved.revision,
            saved.relay,
            vec![RelayCredentialEdit {
                url: relay_url.into(),
                credential: CredentialEdit::Keep,
            }],
        )
        .unwrap();
    endpoint.shutdown().unwrap();
    drop(endpoint);
    let reopened = VeluneRemoteEndpoint::open(options, None).unwrap();
    let restored = reopened.read_network_state().unwrap().saved.unwrap();
    assert!(matches!(restored.relay, RelayConfig::Custom { .. }));
    assert!(matches!(restored.discovery, DiscoveryConfig::None));
    assert!(!restored.local_discovery);
    assert_eq!(restored.revision, saved.revision);
    assert_eq!(
        reopened.owner.service.descriptor().unwrap().endpoint_id,
        identity
    );
    let private_json: serde_json::Value = serde_json::from_slice(
        &std::fs::read(directory.0.join("state.json.network-private.json")).unwrap(),
    )
    .unwrap();
    assert!(private_json["config"]["relay_credentials"][0]["bearer_token"] == token);
    reopened.shutdown().unwrap();
}
