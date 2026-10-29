use crate::{
    endpoint::NetworkGeneration,
    identity::random_id,
    ledger::Ledger,
    protocol::{Reply, Request},
    *,
};
use sha2::{Digest, Sha256};
use std::{
    future::Future,
    pin::Pin,
    sync::{Arc, Mutex, MutexGuard},
};
use tokio::task::JoinHandle;
use velune_application_api::{ApplicationReply, Command, Failure};

pub type HandlerFuture<'a> =
    Pin<Box<dyn Future<Output = std::result::Result<ApplicationReply, Failure>> + Send + 'a>>;
/// References the execution host's single business owner, independently of its network resources.
pub trait Handler: Send + Sync {
    fn execute(&self, context: RequestContext, command: Command) -> HandlerFuture<'_>;
}
pub(crate) struct Access {
    pub(crate) generation: Option<u64>,
    pub(crate) ingress: bool,
    pub(crate) profile_ingress: bool,
    pub(crate) closed: bool,
    pub(crate) pairing: crate::pairing::PairingRegistry,
    ledger: Ledger,
}
pub(crate) struct ServiceState {
    pub(crate) identity: DeviceIdentity,
    pub(crate) incarnation: String,
    pub(crate) store: Arc<DeviceStore>,
    pub(crate) outgoing: Mutex<std::collections::BTreeMap<String, crate::pairing::OutgoingPairing>>,
    handler: Option<Arc<dyn Handler>>,
    pub(crate) telemetry: Mutex<Option<Arc<velune_telemetry::ClientTelemetry>>>,
    pub(crate) network_facts: Mutex<Option<PlatformNetworkSnapshot>>,
    pub(crate) network_application: Mutex<crate::settings::NetworkApplication>,
    pub(crate) access: Mutex<Access>,
    tasks: Mutex<Vec<JoinHandle<()>>>,
}
impl ServiceState {
    pub(crate) fn access(&self) -> Result<MutexGuard<'_, Access>> {
        self.access.lock().map_err(|_| RemoteError::Unavailable)
    }
    pub(crate) fn check_generation(access: &Access, generation: u64) -> Result<()> {
        if access.closed || access.generation != Some(generation) {
            return Err(RemoteError::Closed);
        }
        Ok(())
    }
    pub(crate) fn dispatch(
        self: &Arc<Self>,
        generation: u64,
        peer: String,
        descriptor: EndpointDescriptor,
        request: Request,
    ) -> Reply {
        self.try_dispatch(generation, peer, descriptor, request)
            .unwrap_or_else(Reply::Error)
    }
    fn try_dispatch(
        self: &Arc<Self>,
        generation: u64,
        peer: String,
        descriptor: EndpointDescriptor,
        request: Request,
    ) -> Result<Reply> {
        let mut access = self.access()?;
        Self::check_generation(&access, generation)?;
        if !access.ingress && !access.profile_ingress {
            return Err(RemoteError::Closed);
        }
        if matches!(request, Request::ReadTelemetryProfile) {
            if !access.profile_ingress || self.store.profile_grant(&peer)?.is_none() {
                return Err(RemoteError::Unauthorized);
            }
            let owner = self
                .telemetry
                .lock()
                .map_err(|_| RemoteError::Unavailable)?
                .clone()
                .ok_or(RemoteError::NoHandler)?;
            // The access lock orders this read against revocation; no result/credential is persisted.
            return Ok(Reply::TelemetryProfile(
                owner
                    .saved_profile()
                    .map_err(|_| RemoteError::Configuration)?,
            ));
        }
        if matches!(request, Request::Hello)
            && access.profile_ingress
            && self.store.profile_grant(&peer)?.is_some()
        {
            return Ok(Reply::Hello(descriptor));
        }
        if !access.ingress {
            return Err(RemoteError::Unauthorized);
        }
        if self.store.grant(&peer)?.is_none() {
            return Err(RemoteError::Unauthorized);
        }
        match request {
            Request::Hello => Ok(Reply::Hello(descriptor)),
            Request::Query { id, trace_context } => {
                id.validate(&self.incarnation)?;
                if let Some(context) = &trace_context {
                    context.validate().map_err(|_| RemoteError::Configuration)?;
                }
                let mut observation = self
                    .telemetry
                    .lock()
                    .map_err(|_| RemoteError::Unavailable)?
                    .as_ref()
                    .map(|owner| {
                        owner.operation_with_kind(
                            "remote.query",
                            velune_telemetry::SpanKind::Server,
                            trace_context.as_ref(),
                            true,
                        )
                    });
                let result = access.ledger.status(&peer, &id).map(Reply::Status);
                if let Some(observation) = &mut observation {
                    observation.request_identity(&id.incarnation, &id.client, id.sequence);
                    observation.complete(if result.is_ok() {
                        velune_telemetry::Outcome::Success
                    } else {
                        velune_telemetry::Outcome::Failure
                    });
                }
                result
            }
            Request::Submit {
                id,
                view,
                execution,
                command,
                trace_context,
            } => {
                id.validate(&self.incarnation)?;
                if let Some(context) = &trace_context {
                    context.validate().map_err(|_| RemoteError::Configuration)?;
                }
                let telemetry = self
                    .telemetry
                    .lock()
                    .map_err(|_| RemoteError::Unavailable)?
                    .clone();
                let mut admission = telemetry.as_ref().map(|owner| {
                    owner.operation_with_kind(
                        "remote.submit",
                        velune_telemetry::SpanKind::Server,
                        trace_context.as_ref(),
                        true,
                    )
                });
                if let Some(observation) = &mut admission {
                    observation.request_identity(&id.incarnation, &id.client, id.sequence);
                }
                let execution_parent = admission
                    .as_ref()
                    .and_then(|observation| observation.carrier());
                let result = (|| {
                    if view.0.is_empty()
                        || view.0.len() > 128
                        || view.0.chars().any(char::is_control)
                    {
                        return Err(RemoteError::Configuration);
                    }
                    let handler = self.handler.clone().ok_or(RemoteError::NoHandler)?;
                    let fingerprint: [u8; 32] = Sha256::digest(
                        serde_json::to_vec(&(&view, &execution, &command))
                            .map_err(|_| RemoteError::Protocol)?,
                    )
                    .into();
                    if access.ledger.accept(&peer, &id, fingerprint)? {
                        let telemetry = self
                            .telemetry
                            .lock()
                            .map_err(|_| RemoteError::Unavailable)?
                            .clone();
                        let service = self.clone();
                        let task_peer = peer.clone();
                        let task_id = id.clone();
                        let observed_id = id.clone();
                        let ledger_peer = peer.clone();
                        let task = tokio::spawn(async move {
                            let execution = tokio::spawn(async move {
                                let mut observation = telemetry.as_ref().map(|owner| {
                                    owner.operation(
                                        "remote.execute",
                                        execution_parent.as_ref(),
                                        false,
                                    )
                                });
                                if let Some(observation) = &mut observation {
                                    observation.request_identity(
                                        &observed_id.incarnation,
                                        &observed_id.client,
                                        observed_id.sequence,
                                    );
                                }
                                let future = handler.execute(
                                    RequestContext {
                                        peer_id: task_peer.clone(),
                                        view,
                                        execution,
                                    },
                                    *command,
                                );
                                let result = if let Some(observation) = &observation {
                                    observation.instrument(future).await
                                } else {
                                    future.await
                                };
                                if let Some(observation) = &mut observation {
                                    observation.complete(if result.is_ok() {
                                        velune_telemetry::Outcome::Success
                                    } else {
                                        velune_telemetry::Outcome::Failure
                                    });
                                }
                                result
                            });
                            let status = match execution.await {
                                Ok(result) => RequestStatus::Completed(result.map(Box::new)),
                                Err(_) => RequestStatus::Failed(RemoteError::Unavailable),
                            };
                            if let Ok(mut access) = service.access.lock() {
                                access.ledger.complete(ledger_peer, task_id, status);
                            }
                            // The service (and a host-provided runtime lease in Handler) outlives completion bookkeeping.
                        });
                        let mut tasks = self.tasks.lock().map_err(|_| RemoteError::Unavailable)?;
                        tasks.retain(|task| !task.is_finished());
                        tasks.push(task);
                    }
                    Ok(Reply::Status(access.ledger.status(&peer, &id)?))
                })();
                if let Some(observation) = &mut admission {
                    observation.complete(if result.is_ok() {
                        velune_telemetry::Outcome::Success
                    } else {
                        velune_telemetry::Outcome::Failure
                    });
                }
                result
            }
            Request::PairBegin { .. }
            | Request::PairReady { .. }
            | Request::PairStatus { .. }
            | Request::PairCancel { .. } => Err(RemoteError::Protocol),
            Request::ReadTelemetryProfile => Err(RemoteError::Protocol),
        }
    }
}

/// A business owner's incarnation, acceptance ledger and completion tasks survive network replacement.
/// The embedding host must keep its Tokio runtime alive until this service's close completes.
pub struct RemoteAccessService {
    pub(crate) state: Arc<ServiceState>,
    network: Arc<Mutex<Option<Arc<NetworkGeneration>>>>,
    lifecycle: tokio::sync::Mutex<Lifecycle>,
    pub(crate) pairing_begin: tokio::sync::Mutex<()>,
    pub(crate) controllers: Arc<Mutex<std::collections::BTreeMap<String, Arc<Controller>>>>,
}
#[derive(Default)]
struct Lifecycle {
    next_generation: u64,
    pending: Option<JoinHandle<Result<()>>>,
    failed: bool,
}
impl Lifecycle {
    async fn finish_pending(&mut self) -> Result<Option<Result<()>>> {
        if self.failed {
            return Err(RemoteError::Unavailable);
        }
        let Some(pending) = self.pending.as_mut() else {
            return Ok(None);
        };
        // Borrow the saved handle: cancelling a caller cannot detach the unique close/bind barrier.
        let result = pending.await;
        self.pending = None;
        match result {
            Ok(result) => Ok(Some(result)),
            Err(_) => {
                self.failed = true;
                Err(RemoteError::Unavailable)
            }
        }
    }
}
impl RemoteAccessService {
    pub async fn bind(
        identity: DeviceIdentity,
        config: EndpointConfig,
        store: Arc<DeviceStore>,
        handler: Option<Arc<dyn Handler>>,
    ) -> Result<Self> {
        let service = Self::new(identity, store, handler)?;
        let config = service
            .state
            .store
            .network_settings
            .get()?
            .map(|stored| stored.config)
            .unwrap_or(config);
        service
            .replace_network_expected(config, None, Vec::new())
            .await?;
        Ok(service)
    }
    pub fn new(
        identity: DeviceIdentity,
        store: Arc<DeviceStore>,
        handler: Option<Arc<dyn Handler>>,
    ) -> Result<Self> {
        let ingress = handler.is_some();
        let profile_ingress = !store.snapshot()?.telemetry_profile_grants.is_empty();
        Ok(Self {
            state: Arc::new(ServiceState {
                identity,
                incarnation: random_id()?,
                store,
                outgoing: Mutex::new(std::collections::BTreeMap::new()),
                handler,
                telemetry: Mutex::new(None),
                network_facts: Mutex::new(None),
                network_application: Mutex::new(crate::settings::NetworkApplication::default()),
                access: Mutex::new(Access {
                    generation: None,
                    ingress,
                    profile_ingress,
                    closed: false,
                    pairing: crate::pairing::PairingRegistry::default(),
                    ledger: Ledger::default(),
                }),
                tasks: Mutex::new(Vec::new()),
            }),
            network: Arc::new(Mutex::new(None)),
            lifecycle: tokio::sync::Mutex::new(Lifecycle::default()),
            pairing_begin: tokio::sync::Mutex::new(()),
            controllers: Arc::new(Mutex::new(std::collections::BTreeMap::new())),
        })
    }
    pub(crate) fn network(&self) -> Result<Arc<NetworkGeneration>> {
        self.network
            .lock()
            .map_err(|_| RemoteError::Unavailable)?
            .clone()
            .ok_or(RemoteError::Closed)
    }
    pub fn descriptor(&self) -> Result<EndpointDescriptor> {
        self.network()?.descriptor()
    }
    /// Seal and fully retire the old network before binding its replacement. Failure leaves no old fallback.
    /// Once queued, the complete transaction survives caller cancellation; the next lifecycle call joins it.
    pub async fn replace_network(&self, config: EndpointConfig) -> Result<()> {
        let outcome = self
            .replace_network_expected(config, None, Vec::new())
            .await?;
        if outcome.phase == NetworkApplicationPhase::Applied {
            Ok(())
        } else {
            Err(self
                .state
                .network_application
                .lock()
                .map_err(|_| RemoteError::Unavailable)?
                .error
                .unwrap_or(RemoteError::Unavailable))
        }
    }
    pub(crate) async fn replace_network_expected(
        &self,
        config: EndpointConfig,
        expected: Option<u64>,
        restore_lookup_urls: Vec<String>,
    ) -> Result<NetworkState> {
        // Reject invalid input before withdrawing the currently usable network.
        let config = crate::settings::normalize_config(config)?;
        let mut lifecycle = self.lifecycle.lock().await;
        let _previous = lifecycle.finish_pending().await?;
        if self.state.access()?.closed {
            return Err(RemoteError::Closed);
        }
        let saved = self.state.store.network_settings.save(
            config.clone(),
            expected,
            &restore_lookup_urls,
        )?;
        *self
            .state
            .network_application
            .lock()
            .map_err(|_| RemoteError::Unavailable)? = crate::settings::NetworkApplication {
            phase: NetworkApplicationPhase::Applying,
            revision: None,
            generation: None,
            error: None,
        };
        self.state.store.touch();
        lifecycle.next_generation = lifecycle
            .next_generation
            .checked_add(1)
            .ok_or(RemoteError::Capacity)?;
        let generation = lifecycle.next_generation;
        let state = self.state.clone();
        let network_owner = self.network.clone();
        let controllers = self.controllers.clone();
        lifecycle.pending = Some(tokio::spawn(async move {
            let outcome = async {
                let old = {
                    let mut access = state.access()?;
                    if access.closed {
                        return Err(RemoteError::Closed);
                    }
                    access.generation = None;
                    wake_controllers(&controllers);
                    access.pairing.invalidate(PairingPhase::Disconnected);
                    state.store.touch();
                    network_owner
                        .lock()
                        .map_err(|_| RemoteError::Unavailable)?
                        .take()
                };
                if let Some(old) = old {
                    old.close().await?;
                    drop(old);
                }
                let network = NetworkGeneration::bind(state.clone(), generation, config).await?;
                let installed = {
                    let mut access = state.access()?;
                    if access.closed {
                        false
                    } else {
                        access.generation = Some(generation);
                        *network_owner.lock().map_err(|_| RemoteError::Unavailable)? =
                            Some(network.clone());
                        true
                    }
                };
                if !installed {
                    network.close().await?;
                    return Err(RemoteError::Closed);
                }
                network.start();
                wake_controllers(&controllers);
                *state
                    .network_application
                    .lock()
                    .map_err(|_| RemoteError::Unavailable)? = crate::settings::NetworkApplication {
                    phase: NetworkApplicationPhase::Applied,
                    revision: Some(saved.revision),
                    generation: Some(generation),
                    error: None,
                };
                state.store.touch();
                Ok(())
            }
            .await;
            if let Err(error) = outcome {
                *state
                    .network_application
                    .lock()
                    .map_err(|_| RemoteError::Unavailable)? = crate::settings::NetworkApplication {
                    phase: NetworkApplicationPhase::Failed,
                    revision: None,
                    generation: None,
                    error: Some(error),
                };
                state.store.touch();
            }
            outcome
        }));
        let outcome = lifecycle.finish_pending().await;
        match outcome {
            Ok(Some(_)) => self.network_state(),
            Ok(None) => Err(RemoteError::Unavailable),
            Err(error) => {
                *self
                    .state
                    .network_application
                    .lock()
                    .map_err(|_| RemoteError::Unavailable)? = crate::settings::NetworkApplication {
                    phase: NetworkApplicationPhase::Failed,
                    revision: None,
                    generation: None,
                    error: Some(error),
                };
                self.state.store.touch();
                Err(error)
            }
        }
    }
    /// Ingress policy only: existing and newly dialed outgoing control remain usable.
    pub fn set_control_ingress(&self, enabled: bool) -> Result<()> {
        let mut access = self.state.access()?;
        if access.closed {
            return Err(RemoteError::Closed);
        }
        access.ingress = enabled;
        self.state.store.touch();
        if !enabled {
            access
                .pairing
                .invalidate_permission(PairingPermission::Control, PairingPhase::Disconnected);
            self.state.store.touch();
            if !access.profile_ingress
                && let Ok(network) = self.network()
            {
                network.close_incoming();
            }
        }
        Ok(())
    }
    pub fn revoke_control(&self, id: &str) -> Result<()> {
        crate::store::parse_peer(id)?;
        let mut access = self.state.access()?;
        self.state.store.revoke_grant(id)?;
        access
            .pairing
            .revoke_permission(id, PairingPermission::Control);
        if self.state.store.profile_grant(id)?.is_none()
            && let Ok(network) = self.network()
        {
            network.revoke(id);
        }
        Ok(())
    }
    pub fn attach_telemetry(&self, owner: Arc<velune_telemetry::ClientTelemetry>) -> Result<()> {
        *self
            .state
            .telemetry
            .lock()
            .map_err(|_| RemoteError::Unavailable)? = Some(owner);
        Ok(())
    }
    /// Explicit local consent, including authentication material, for an already trusted identity.
    pub fn allow_telemetry_profile(&self, peer: &str) -> Result<()> {
        let mut access = self.state.access()?;
        if access.closed {
            return Err(RemoteError::Closed);
        }
        let (name, attempt_id) = if let Some(grant) = self.state.store.grant(peer)? {
            (grant.name, grant.attempt_id)
        } else {
            let contact = self.state.store.contact(peer)?;
            let ContactConfirmation::Confirmed { attempt_id } = contact.confirmation else {
                return Err(RemoteError::Unauthorized);
            };
            (contact.name, attempt_id)
        };
        self.state
            .store
            .commit_profile_grant(IncomingControlGrant {
                peer_id: peer.into(),
                name,
                attempt_id,
            })?;
        access.profile_ingress = true;
        self.state.store.touch();
        Ok(())
    }
    pub fn revoke_telemetry_profile(&self, peer: &str) -> Result<()> {
        let mut access = self.state.access()?;
        self.state.store.revoke_profile_grant(peer)?;
        access
            .pairing
            .revoke_permission(peer, PairingPermission::TelemetryProfileRead);
        Ok(())
    }
    /// One read over a pinned encrypted channel. It never enters the business request ledger.
    pub async fn read_telemetry_profile(
        &self,
        peer: &str,
    ) -> Result<Option<velune_telemetry::TelemetryProfile>> {
        let contact = self.state.store.contact(peer)?;
        if !matches!(contact.confirmation, ContactConfirmation::Confirmed { .. }) {
            return Err(RemoteError::Unauthorized);
        }
        let client = self
            .dial_locator(
                peer,
                contact.name,
                &contact.locator,
                ClientSession::new()?,
                true,
            )
            .await?;
        match client.rpc(Request::ReadTelemetryProfile).await? {
            Reply::TelemetryProfile(profile) => {
                if let Some(profile) = &profile {
                    profile.validate().map_err(|_| RemoteError::Configuration)?;
                }
                Ok(profile)
            }
            _ => Err(RemoteError::Protocol),
        }
    }
    pub async fn report_network_change(&self, snapshot: PlatformNetworkSnapshot) -> Result<u64> {
        snapshot.validate()?;
        let sequence = snapshot.sequence;
        {
            let mut facts = self
                .state
                .network_facts
                .lock()
                .map_err(|_| RemoteError::Unavailable)?;
            if facts
                .as_ref()
                .is_some_and(|previous| previous.sequence >= sequence)
            {
                return Ok(sequence);
            }
            *facts = Some(snapshot);
        }
        let network = self.network()?;
        network.refresh_local_discovery().await?;
        network.network_change().await?;
        wake_controllers(&self.controllers);
        Ok(sequence)
    }
    /// End the service, wait for its one network close and all already accepted completion bookkeeping.
    /// This does not close the business owner or implicitly cancel its work.
    /// Cancelling this caller preserves the saved shutdown task for a subsequent close to join.
    pub async fn close(&self) -> Result<()> {
        let mut lifecycle = self.lifecycle.lock().await;
        let _previous = lifecycle.finish_pending().await?;
        let state = self.state.clone();
        let network_owner = self.network.clone();
        let controllers = self.controllers.clone();
        lifecycle.pending = Some(tokio::spawn(async move {
            let old = {
                let mut access = state
                    .access
                    .lock()
                    .unwrap_or_else(|poison| poison.into_inner());
                access.closed = true;
                access.generation = None;
                *state
                    .network_application
                    .lock()
                    .unwrap_or_else(|poison| poison.into_inner()) =
                    crate::settings::NetworkApplication::default();
                wake_controllers(&controllers);
                access.pairing.invalidate(PairingPhase::Disconnected);
                state.store.touch();
                network_owner
                    .lock()
                    .unwrap_or_else(|poison| poison.into_inner())
                    .take()
            };
            if let Some(old) = old {
                old.close().await?;
            }
            let tasks = std::mem::take(
                &mut *state
                    .tasks
                    .lock()
                    .unwrap_or_else(|poison| poison.into_inner()),
            );
            for task in tasks {
                let _ = task.await;
            }
            Ok(())
        }));
        lifecycle
            .finish_pending()
            .await?
            .ok_or(RemoteError::Unavailable)?
    }
}
impl Drop for RemoteAccessService {
    fn drop(&mut self) {
        if let Ok(mut access) = self.state.access.lock() {
            access.closed = true;
            access.generation = None;
            access.pairing.invalidate(PairingPhase::Disconnected);
        }
        if let Ok(mut network) = self.network.lock()
            && let Some(network) = network.take()
        {
            let _ = network.stop();
        }
    }
}

fn wake_controllers(controllers: &Mutex<std::collections::BTreeMap<String, Arc<Controller>>>) {
    if let Ok(controllers) = controllers.lock() {
        for controller in controllers.values() {
            controller.network_changed();
        }
    }
}
