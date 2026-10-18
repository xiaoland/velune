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
        if !access.ingress {
            return Err(RemoteError::Closed);
        }
        if self.store.grant(&peer)?.is_none() {
            return Err(RemoteError::Unauthorized);
        }
        match request {
            Request::Hello => Ok(Reply::Hello(descriptor)),
            Request::Query { id } => {
                id.validate(&self.incarnation)?;
                Ok(Reply::Status(access.ledger.status(&peer, &id)?))
            }
            Request::Submit {
                id,
                view,
                execution,
                command,
            } => {
                id.validate(&self.incarnation)?;
                if view.0.is_empty() || view.0.len() > 128 || view.0.chars().any(char::is_control) {
                    return Err(RemoteError::Configuration);
                }
                let handler = self.handler.clone().ok_or(RemoteError::NoHandler)?;
                let fingerprint: [u8; 32] = Sha256::digest(
                    serde_json::to_vec(&(&view, &execution, &command))
                        .map_err(|_| RemoteError::Protocol)?,
                )
                .into();
                if access.ledger.accept(&peer, &id, fingerprint)? {
                    let service = self.clone();
                    let task_peer = peer.clone();
                    let task_id = id.clone();
                    let ledger_peer = peer.clone();
                    let task = tokio::spawn(async move {
                        let execution = tokio::spawn(async move {
                            handler
                                .execute(
                                    RequestContext {
                                        peer_id: task_peer.clone(),
                                        view,
                                        execution,
                                    },
                                    *command,
                                )
                                .await
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
            }
            Request::PairBegin { .. }
            | Request::PairReady { .. }
            | Request::PairStatus { .. }
            | Request::PairCancel { .. } => Err(RemoteError::Protocol),
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
    pub(crate) controllers: Mutex<std::collections::BTreeMap<String, Arc<Controller>>>,
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
        service.replace_network(config).await?;
        Ok(service)
    }
    pub fn new(
        identity: DeviceIdentity,
        store: Arc<DeviceStore>,
        handler: Option<Arc<dyn Handler>>,
    ) -> Result<Self> {
        let ingress = handler.is_some();
        Ok(Self {
            state: Arc::new(ServiceState {
                identity,
                incarnation: random_id()?,
                store,
                outgoing: Mutex::new(std::collections::BTreeMap::new()),
                handler,
                access: Mutex::new(Access {
                    generation: None,
                    ingress,
                    closed: false,
                    pairing: crate::pairing::PairingRegistry::default(),
                    ledger: Ledger::default(),
                }),
                tasks: Mutex::new(Vec::new()),
            }),
            network: Arc::new(Mutex::new(None)),
            lifecycle: tokio::sync::Mutex::new(Lifecycle::default()),
            pairing_begin: tokio::sync::Mutex::new(()),
            controllers: Mutex::new(std::collections::BTreeMap::new()),
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
        // Reject invalid input before withdrawing the currently usable network.
        crate::endpoint::validate_config(&config)?;
        let mut lifecycle = self.lifecycle.lock().await;
        let _previous = lifecycle.finish_pending().await?;
        lifecycle.next_generation = lifecycle
            .next_generation
            .checked_add(1)
            .ok_or(RemoteError::Capacity)?;
        let generation = lifecycle.next_generation;
        let state = self.state.clone();
        let network_owner = self.network.clone();
        lifecycle.pending = Some(tokio::spawn(async move {
            let old = {
                let mut access = state.access()?;
                if access.closed {
                    return Err(RemoteError::Closed);
                }
                access.generation = None;
                access.pairing.invalidate(PairingPhase::Disconnected);
                state.store.touch();
                network_owner
                    .lock()
                    .map_err(|_| RemoteError::Unavailable)?
                    .take()
            };
            if let Some(old) = old {
                old.close().await;
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
                network.close().await;
                return Err(RemoteError::Closed);
            }
            network.start();
            Ok(())
        }));
        lifecycle
            .finish_pending()
            .await?
            .ok_or(RemoteError::Unavailable)?
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
            access.pairing.invalidate(PairingPhase::Disconnected);
            self.state.store.touch();
            if let Ok(network) = self.network() {
                network.close_incoming();
            }
        }
        Ok(())
    }
    pub fn revoke_control(&self, id: &str) -> Result<()> {
        crate::store::parse_peer(id)?;
        let mut access = self.state.access()?;
        self.state.store.revoke_grant(id)?;
        access.pairing.revoke(id);
        if let Ok(network) = self.network() {
            network.revoke(id);
        }
        Ok(())
    }
    pub async fn network_change(&self) {
        if let Ok(network) = self.network() {
            let _ = network.network_change().await;
        }
    }
    pub async fn report_network_change(&self, snapshot: PlatformNetworkSnapshot) -> Result<u64> {
        snapshot.validate()?;
        self.network()?.network_change().await?;
        Ok(snapshot.sequence)
    }
    /// End the service, wait for its one network close and all already accepted completion bookkeeping.
    /// This does not close the business owner or implicitly cancel its work.
    /// Cancelling this caller preserves the saved shutdown task for a subsequent close to join.
    pub async fn close(&self) -> Result<()> {
        let mut lifecycle = self.lifecycle.lock().await;
        let _previous = lifecycle.finish_pending().await?;
        let state = self.state.clone();
        let network_owner = self.network.clone();
        lifecycle.pending = Some(tokio::spawn(async move {
            let old = {
                let mut access = state
                    .access
                    .lock()
                    .unwrap_or_else(|poison| poison.into_inner());
                access.closed = true;
                access.generation = None;
                access.pairing.invalidate(PairingPhase::Disconnected);
                state.store.touch();
                network_owner
                    .lock()
                    .unwrap_or_else(|poison| poison.into_inner())
                    .take()
            };
            if let Some(old) = old {
                old.close().await;
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
