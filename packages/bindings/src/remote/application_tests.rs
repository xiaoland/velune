use super::*;
use velune_application_api::{CommandKind, FailureKind, RuntimeUpdate};

struct Invocation {
    peer: String,
    kind: CommandKind,
    expected: Option<Revision>,
}

struct OwnerState {
    revision: Revision,
    calls: Vec<Invocation>,
    mutations: usize,
}

struct RecordingOwner(Mutex<OwnerState>);

impl RecordingOwner {
    fn new() -> Self {
        Self(Mutex::new(OwnerState {
            revision: Revision(41),
            calls: Vec::new(),
            mutations: 0,
        }))
    }

    fn kinds(&self) -> Vec<CommandKind> {
        self.0
            .lock()
            .unwrap()
            .calls
            .iter()
            .map(|call| call.kind)
            .collect()
    }
}

impl velune_remote::Handler for RecordingOwner {
    fn execute(
        &self,
        context: velune_remote::RequestContext,
        command: Command,
    ) -> velune_remote::HandlerFuture<'_> {
        Box::pin(async move {
            let mut state = self.0.lock().unwrap();
            state.calls.push(Invocation {
                peer: context.peer_id,
                kind: command.kind(),
                expected: context.execution.expected_revision,
            });
            if command.requires_revision()
                && context.execution.expected_revision != Some(state.revision)
            {
                let mut error = Failure::new(FailureKind::Conflict, "synthetic stale editor");
                error.current_revision = Some(state.revision);
                return Err(error);
            }
            let response = match command {
                // This observation predates self-hosted relay settings and does not
                // require a native history reader or an idle authentication flow.
                Command::ReadComposerOwner => Response::ReadComposerOwner("synthetic-owner".into()),
                Command::ReadConfiguration => {
                    return Err(Failure::new(
                        FailureKind::Unavailable,
                        "synthetic history unavailable",
                    ));
                }
                Command::ReadConversationActivity { .. } => {
                    Response::ReadConversationActivity(ConversationActivity {
                        active: Vec::new(),
                        finished: Vec::new(),
                        resumable: Vec::new(),
                        cursor: 0,
                    })
                }
                Command::DeleteRuntime { .. } => {
                    state.revision.0 += 1;
                    state.mutations += 1;
                    Response::DeleteRuntime(RuntimeUpdate {
                        runtime_instances: Vec::new(),
                        runtime_types: Vec::new(),
                        execution_invalidated: false,
                    })
                }
                _ => panic!("unexpected synthetic command"),
            };
            Ok(ApplicationReply {
                revision: state.revision,
                response,
            })
        })
    }
}

struct Fixture {
    directory: std::path::PathBuf,
    host: VeluneRemoteEndpoint,
    client: Arc<VeluneRemoteEndpoint>,
    handler: Arc<RecordingOwner>,
}

impl Fixture {
    fn new() -> Self {
        let nonce = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_nanos();
        let directory = std::env::temp_dir().join(format!(
            "velune-remote-preparation-{}-{nonce}",
            std::process::id()
        ));
        std::fs::create_dir(&directory).unwrap();
        let options = |name: &str| BindingRemoteOptions {
            identity_file: directory
                .join(format!("{name}-identity"))
                .to_string_lossy()
                .into_owned(),
            state_file: directory
                .join(format!("{name}-state.json"))
                .to_string_lossy()
                .into_owned(),
            device_name: format!("Synthetic {name}"),
            relay: RelayConfig::Disabled,
            discovery: DiscoveryConfig::None,
            path_policy: PathPolicy::Automatic,
            bind_addresses: vec!["127.0.0.1:0".into()],
            request_timeout_ms: 5000,
        };
        let client = VeluneRemoteEndpoint::open(options("client"), None).unwrap();
        let handler = Arc::new(RecordingOwner::new());
        let host = VeluneRemoteEndpoint {
            owner: ServiceOwner::bind(
                options("host"),
                RemoteRuntime::new().unwrap(),
                Some(handler.clone()),
                client.owner.telemetry.clone(),
                #[cfg(all(unix, feature = "external-control"))]
                None,
            )
            .unwrap(),
        };
        Self {
            directory,
            host,
            client,
            handler,
        }
    }

    fn connect(&self) -> Arc<VeluneApplication> {
        let invitation = self.host.create_invitation(60).unwrap();
        assert!(invitation.encoded.starts_with("velune://pair/v3/"));
        let pending = self.client.begin_pairing(invitation.encoded).unwrap();
        let peer = self.host.descriptor().unwrap().endpoint_id;
        assert!(
            self.client
                .connect(peer.clone(), "unapproved".into())
                .is_err()
        );
        assert!(
            self.handler.kinds().is_empty(),
            "preparation must remain behind control authorization"
        );
        self.host
            .approve_pairing(
                pending.attempt_id,
                self.client.descriptor().unwrap().endpoint_id,
            )
            .unwrap();
        self.client.pairing_status(peer.clone()).unwrap();
        self.client.connect(peer, "synthetic-view".into()).unwrap()
    }
}

impl Drop for Fixture {
    fn drop(&mut self) {
        self.client.shutdown().unwrap();
        self.host.shutdown().unwrap();
        let _ = std::fs::remove_dir_all(&self.directory);
    }
}

#[test]
fn preparation_does_not_read_history_and_retains_the_edit_revision_contract() {
    let fixture = Fixture::new();
    let proxy = fixture.connect();
    let remote = proxy.remote_backend().unwrap();
    assert_eq!(fixture.handler.kinds(), [CommandKind::ReadComposerOwner]);
    assert_eq!(remote.state().unwrap().expected, Some(Revision(41)));
    assert!(
        proxy.list().is_err(),
        "only an explicit list depends on the history reader"
    );
    assert_eq!(
        fixture.handler.kinds(),
        [
            CommandKind::ReadComposerOwner,
            CommandKind::ReadConfiguration
        ]
    );

    proxy.delete_runtime("synthetic".into()).unwrap();
    assert_eq!(remote.state().unwrap().expected, Some(Revision(42)));
    // A write by another view invalidates this editor. Ordinary observations
    // may report the newer revision without silently rebasing stale input.
    fixture.handler.0.lock().unwrap().revision = Revision(43);
    proxy.conversation_activity(0).unwrap();
    assert_eq!(remote.state().unwrap().observed, Some(Revision(43)));
    assert_eq!(remote.state().unwrap().expected, Some(Revision(42)));
    assert!(matches!(
        proxy.delete_runtime("synthetic".into()),
        Err(BindingError::Diagnostic {
            kind: BindingFailureKind::Conflict,
            ..
        })
    ));
    let state = fixture.handler.0.lock().unwrap();
    assert_eq!(state.mutations, 1);
    assert_eq!(state.calls.last().unwrap().expected, Some(Revision(42)));
    let client_id = fixture.client.descriptor().unwrap().endpoint_id;
    assert!(state.calls.iter().all(|call| call.peer == client_id));
}

#[test]
fn preparation_revision_survives_network_replacement_but_not_owner_replacement() {
    let fixture = Fixture::new();
    let proxy = fixture.connect();
    let remote = proxy.remote_backend().unwrap();
    let before = remote.context().unwrap();
    let saved = fixture
        .client
        .owner
        .service
        .network_state()
        .unwrap()
        .saved
        .unwrap();
    let applied = fixture
        .client
        .owner
        .runtime
        .block_on(
            fixture
                .client
                .owner
                .service
                .apply_network(velune_remote::NetworkSettingsUpdate {
                    expected_revision: saved.revision,
                    config: EndpointConfig {
                        relay: saved.relay.clone(),
                        discovery: saved.discovery.clone(),
                        local_discovery: saved.local_discovery,
                        path_policy: saved.path_policy,
                        bind_addresses: saved.bind_addresses.clone(),
                        request_timeout: std::time::Duration::from_millis(saved.request_timeout_ms),
                        relay_credentials: Vec::new(),
                    },
                    credentials: saved
                        .credentials
                        .iter()
                        .map(|credential| velune_remote::RelayCredentialEdit {
                            url: credential.url.clone(),
                            credential: velune_remote::CredentialEdit::Keep,
                        })
                        .collect(),
                    restore_lookup_urls: Vec::new(),
                }),
        )
        .unwrap();
    assert!(matches!(
        applied.phase,
        velune_remote::NetworkApplicationPhase::Applied
    ));
    assert_eq!(applied.applied_revision, Some(saved.revision + 1));
    assert_eq!(applied.saved.unwrap().revision, saved.revision + 1);
    fixture
        .client
        .owner
        .runtime
        .block_on(remote.controller.connect())
        .unwrap();
    let after = remote.context().unwrap();
    assert_ne!(before.connection_epoch, after.connection_epoch);
    assert_eq!(after.execution.expected_revision, Some(Revision(41)));
    remote.observe(&before, Revision(99), true).unwrap();
    assert_eq!(remote.state().unwrap().expected, Some(Revision(41)));
    assert_eq!(remote.state().unwrap().observed, Some(Revision(41)));

    // The descriptor is learned from authenticated Hello. Make the retained
    // descriptor older than that Hello to exercise the existing incarnation gate.
    remote.state().unwrap().descriptor.incarnation = "previous-owner".into();
    let changed = remote.context().unwrap();
    assert_eq!(changed.execution.expected_revision, None);
    assert_ne!(changed.user_epoch, after.user_epoch);
    remote.observe(&after, Revision(100), true).unwrap();
    assert_eq!(remote.state().unwrap().expected, None);
}
