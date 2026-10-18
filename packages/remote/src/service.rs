use crate::{
    endpoint::NetworkGeneration,
    identity::{random_bytes, random_id, validate_name},
    ledger::Ledger,
    protocol::{Reply, Request},
    *,
};
use sha2::{Digest, Sha256};
use std::{
    future::Future,
    pin::Pin,
    sync::{Arc, Mutex, MutexGuard},
    time::{Duration, Instant, SystemTime, UNIX_EPOCH},
};
use tokio::task::JoinHandle;
use velune_application_api::{ApplicationReply, Command, Failure};

pub type HandlerFuture<'a> =
    Pin<Box<dyn Future<Output = std::result::Result<ApplicationReply, Failure>> + Send + 'a>>;
/// References the execution host's single business owner, independently of its network resources.
pub trait Handler: Send + Sync {
    fn execute(&self, context: RequestContext, command: Command) -> HandlerFuture<'_>;
}
struct PendingInvitation {
    token: [u8; 32],
    deadline: Instant,
}
pub(crate) struct Access {
    pub(crate) generation: Option<u64>,
    pub(crate) ingress: bool,
    pub(crate) closed: bool,
    invitations: Vec<PendingInvitation>,
    ledger: Ledger,
}
pub(crate) struct ServiceState {
    pub(crate) identity: DeviceIdentity,
    pub(crate) incarnation: String,
    pub(crate) trust: Arc<dyn DeviceTrust>,
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
        if let Request::Pair {
            token,
            name,
            incarnation,
        } = request
        {
            validate_name(&name)?;
            if incarnation != self.incarnation {
                return Err(RemoteError::ServerRestarted);
            }
            let now = Instant::now();
            let index = access
                .invitations
                .iter()
                .position(|invitation| {
                    invitation
                        .token
                        .iter()
                        .zip(token)
                        .fold(0u8, |difference, (left, right)| difference | (left ^ right))
                        == 0
                })
                .ok_or(RemoteError::InvalidInvitation)?;
            if access.invitations[index].deadline <= now {
                access.invitations.remove(index);
                return Err(RemoteError::ExpiredInvitation);
            }
            self.trust.trust(TrustedDevice { id: peer, name })?;
            access.invitations.remove(index);
            return Ok(Reply::Paired);
        }
        if !self.trust.is_trusted(&peer)? {
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
            Request::Pair { .. } => Err(RemoteError::Protocol),
        }
    }
}

/// A business owner's incarnation, acceptance ledger and completion tasks survive network replacement.
/// The embedding host must keep its Tokio runtime alive until this service's close completes.
pub struct RemoteAccessService {
    pub(crate) state: Arc<ServiceState>,
    network: Arc<Mutex<Option<Arc<NetworkGeneration>>>>,
    lifecycle: tokio::sync::Mutex<Lifecycle>,
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
        trust: Arc<dyn DeviceTrust>,
        handler: Option<Arc<dyn Handler>>,
    ) -> Result<Self> {
        let service = Self::new(identity, trust, handler)?;
        service.replace_network(config).await?;
        Ok(service)
    }
    pub fn new(
        identity: DeviceIdentity,
        trust: Arc<dyn DeviceTrust>,
        handler: Option<Arc<dyn Handler>>,
    ) -> Result<Self> {
        let ingress = handler.is_some();
        Ok(Self {
            state: Arc::new(ServiceState {
                identity,
                incarnation: random_id()?,
                trust,
                handler,
                access: Mutex::new(Access {
                    generation: None,
                    ingress,
                    closed: false,
                    invitations: Vec::new(),
                    ledger: Ledger::default(),
                }),
                tasks: Mutex::new(Vec::new()),
            }),
            network: Arc::new(Mutex::new(None)),
            lifecycle: tokio::sync::Mutex::new(Lifecycle::default()),
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
                access.invitations.clear();
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
        if !enabled {
            access.invitations.clear();
            if let Ok(network) = self.network() {
                network.close_incoming();
            }
        }
        Ok(())
    }
    pub fn trusted_devices(&self) -> Result<Vec<TrustedDevice>> {
        self.state.trust.devices()
    }
    pub fn create_invitation(&self, lifetime: Duration) -> Result<Invitation> {
        if lifetime.is_zero() || lifetime > Duration::from_secs(600) {
            return Err(RemoteError::Configuration);
        }
        let mut access = self.state.access()?;
        if access.closed || !access.ingress {
            return Err(RemoteError::Closed);
        }
        let endpoint = self.descriptor()?;
        let now = Instant::now();
        access
            .invitations
            .retain(|invitation| invitation.deadline > now);
        if access.invitations.len() >= 16 {
            return Err(RemoteError::Capacity);
        }
        let token = random_bytes()?;
        access.invitations.push(PendingInvitation {
            token,
            deadline: now + lifetime,
        });
        let expires_at_unix_seconds = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .map_err(|_| RemoteError::Unavailable)?
            .as_secs()
            + lifetime.as_secs();
        Ok(Invitation {
            endpoint,
            expires_at_unix_seconds,
            token,
        })
    }
    pub fn revoke(&self, id: &str) -> Result<()> {
        id.parse::<iroh::EndpointId>()
            .map_err(|_| RemoteError::Configuration)?;
        let _access = self.state.access()?;
        self.state.trust.revoke(id)?;
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
                access.invitations.clear();
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
            access.invitations.clear();
        }
        if let Ok(mut network) = self.network.lock()
            && let Some(network) = network.take()
        {
            let _ = network.stop();
        }
    }
}
