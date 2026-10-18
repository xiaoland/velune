use crate::wire::{read_frame, write_frame};
use crate::{
    identity::{random_bytes, random_id, validate_name},
    ledger::Ledger,
    protocol::{ALPN, Envelope, Reply, Request},
    *,
};
use iroh::{
    Endpoint, RelayMode,
    endpoint::{Connection, presets},
};
use sha2::{Digest, Sha256};
use std::{
    collections::HashMap,
    future::Future,
    pin::Pin,
    sync::{
        Arc, Mutex,
        atomic::{AtomicBool, AtomicU64, Ordering},
    },
    time::{Duration, Instant, SystemTime, UNIX_EPOCH},
};
use tokio::{sync::Semaphore, task::JoinHandle};
use velune_application_api::{ApplicationReply, ClientViewId, Command, ExecutionContext, Failure};

pub type HandlerFuture<'a> =
    Pin<Box<dyn Future<Output = std::result::Result<ApplicationReply, Failure>> + Send + 'a>>;
/// Implemented by the execution host's single application owner.
pub trait Handler: Send + Sync {
    fn execute(&self, context: RequestContext, command: Command) -> HandlerFuture<'_>;
}

struct PendingInvitation {
    token: [u8; 32],
    deadline: Instant,
}
#[derive(Default)]
struct Access {
    invitations: Vec<PendingInvitation>,
    connections: HashMap<u64, Connection>,
    ledger: Ledger,
}
struct State {
    endpoint: Endpoint,
    name: String,
    incarnation: String,
    trust: Arc<dyn DeviceTrust>,
    handler: Option<Arc<dyn Handler>>,
    access: Mutex<Access>,
    requests: Semaphore,
    closed: AtomicBool,
    next_connection: AtomicU64,
    timeout: Duration,
}

/// Shared native transport assembly; protocol authorization remains with each caller.
pub(crate) async fn bind_transport(
    key: iroh::SecretKey,
    config: EndpointConfig,
    alpn: &[u8],
) -> Result<Endpoint> {
    if config.request_timeout.is_zero()
        || config.request_timeout > Duration::from_secs(300)
        || config.bind_addresses.len() > 16
    {
        return Err(RemoteError::Configuration);
    }
    let relay = match config.relay {
        RelayConfig::Disabled => RelayMode::Disabled,
        RelayConfig::DevelopmentPublic => RelayMode::Default,
        RelayConfig::Custom { urls } => {
            if urls.is_empty() || urls.len() > 16 {
                return Err(RemoteError::Configuration);
            }
            let urls: Vec<iroh::RelayUrl> = urls
                .into_iter()
                .map(|url| validate_relay_url(&url))
                .collect::<Result<_>>()?;
            RelayMode::custom(urls)
        }
    };
    let relay_enabled = !matches!(relay, RelayMode::Disabled);
    if matches!(config.path_policy, PathPolicy::RelayOnly)
        && (!relay_enabled || !config.bind_addresses.is_empty())
    {
        return Err(RemoteError::Configuration);
    }
    let mut builder = Endpoint::builder(presets::Minimal)
        .secret_key(key)
        .alpns(vec![alpn.to_vec()])
        .relay_mode(relay);
    match config.discovery {
        DiscoveryConfig::None => {}
        DiscoveryConfig::DevelopmentPublic => {
            use iroh::address_lookup::{DnsAddressLookup, PkarrPublisher, PkarrResolver};
            builder = builder
                .address_lookup(PkarrPublisher::n0_dns())
                .address_lookup(PkarrResolver::n0_dns())
                .address_lookup(DnsAddressLookup::n0_dns());
        }
        DiscoveryConfig::CustomPkarr { url } => {
            use iroh::address_lookup::{PkarrPublisher, PkarrResolver};
            let url = validate_relay_url(&url)?;
            builder = builder
                .address_lookup(PkarrPublisher::builder(url.clone().into()))
                .address_lookup(PkarrResolver::builder(url.into()));
        }
    }
    if matches!(config.path_policy, PathPolicy::RelayOnly) {
        builder = builder.clear_ip_transports();
    }
    if !config.bind_addresses.is_empty() {
        builder = builder.clear_ip_transports();
        for address in config.bind_addresses {
            builder = builder
                .bind_addr(address)
                .map_err(|_| RemoteError::Configuration)?;
        }
    }
    let endpoint = builder.bind().await.map_err(|_| RemoteError::Unavailable)?;
    if relay_enabled
        && tokio::time::timeout(config.request_timeout, endpoint.online())
            .await
            .is_err()
    {
        endpoint.close().await;
        return Err(RemoteError::Timeout);
    }
    Ok(endpoint)
}

/// Network lifecycle is independent of the business application lifecycle.
pub struct RemoteEndpoint {
    state: Arc<State>,
    accept_task: Mutex<Option<JoinHandle<()>>>,
}
impl RemoteEndpoint {
    pub async fn bind(
        identity: DeviceIdentity,
        config: EndpointConfig,
        trust: Arc<dyn DeviceTrust>,
        handler: Option<Arc<dyn Handler>>,
    ) -> Result<Self> {
        let endpoint = bind_transport(identity.key, config.clone(), ALPN).await?;
        let state = Arc::new(State {
            endpoint,
            name: identity.name,
            incarnation: random_id()?,
            trust,
            handler,
            access: Mutex::new(Access::default()),
            requests: Semaphore::new(8),
            closed: AtomicBool::new(false),
            next_connection: AtomicU64::new(1),
            timeout: config.request_timeout,
        });
        let accept_task = tokio::spawn(accept_loop(state.clone()));
        Ok(Self {
            state,
            accept_task: Mutex::new(Some(accept_task)),
        })
    }

    pub fn descriptor(&self) -> EndpointDescriptor {
        self.state.descriptor()
    }
    pub async fn network_change(&self) {
        self.state.endpoint.network_change().await;
    }
    /// Acknowledges validated platform facts and requests native network reevaluation.
    /// This does not acknowledge discovery, socket rebinding, or path recovery.
    pub async fn report_network_change(&self, snapshot: PlatformNetworkSnapshot) -> Result<u64> {
        snapshot.validate()?;
        if self.state.closed.load(Ordering::Acquire) {
            return Err(RemoteError::Closed);
        }
        self.state.endpoint.network_change().await;
        Ok(snapshot.sequence)
    }
    pub fn trusted_devices(&self) -> Result<Vec<TrustedDevice>> {
        self.state.trust.devices()
    }
    pub fn create_invitation(&self, lifetime: Duration) -> Result<Invitation> {
        if lifetime.is_zero() || lifetime > Duration::from_secs(600) {
            return Err(RemoteError::Configuration);
        }
        if self.state.closed.load(Ordering::Acquire) {
            return Err(RemoteError::Closed);
        }
        let now = Instant::now();
        let mut access = self
            .state
            .access
            .lock()
            .map_err(|_| RemoteError::Unavailable)?;
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
            endpoint: self.descriptor(),
            expires_at_unix_seconds,
            token,
        })
    }
    /// Persistence succeeds before existing access is revoked. Already accepted business work continues.
    pub fn revoke(&self, id: &str) -> Result<()> {
        id.parse::<iroh::EndpointId>()
            .map_err(|_| RemoteError::Configuration)?;
        let access = self
            .state
            .access
            .lock()
            .map_err(|_| RemoteError::Unavailable)?;
        self.state.trust.revoke(id)?;
        for connection in access
            .connections
            .values()
            .filter(|connection| connection.remote_id().to_string() == id)
        {
            connection.close(1u8.into(), b"authorization revoked");
        }
        Ok(())
    }
    pub async fn pair(&self, invitation: &Invitation) -> Result<RemoteClient> {
        let mut client = self
            .dial(&invitation.endpoint, ClientSession::new()?, false)
            .await?;
        let reply = client
            .rpc(Request::Pair {
                token: invitation.token,
                name: self.state.name.clone(),
                incarnation: invitation.endpoint.incarnation.clone(),
            })
            .await?;
        if !matches!(reply, Reply::Paired) {
            return Err(RemoteError::Protocol);
        }
        self.state.trust.trust(TrustedDevice {
            id: invitation.endpoint.endpoint_id.clone(),
            name: invitation.endpoint.name.clone(),
        })?;
        client.authorized = true;
        Ok(client)
    }
    pub async fn connect(&self, descriptor: &EndpointDescriptor) -> Result<RemoteClient> {
        self.connect_with_session(descriptor, ClientSession::new()?)
            .await
    }
    pub async fn connect_with_session(
        &self,
        descriptor: &EndpointDescriptor,
        session: ClientSession,
    ) -> Result<RemoteClient> {
        let mut client = self.dial(descriptor, session, true).await?;
        match client.rpc(Request::Hello).await? {
            Reply::Hello(current) => {
                // Authentication pins the device key; a new host incarnation allows fresh work,
                // while old request IDs still return ServerRestarted instead of executing again.
                client.incarnation = current.incarnation;
                Ok(client)
            }
            _ => Err(RemoteError::Protocol),
        }
    }
    async fn dial(
        &self,
        descriptor: &EndpointDescriptor,
        session: ClientSession,
        authorized: bool,
    ) -> Result<RemoteClient> {
        if self.state.closed.load(Ordering::Acquire) {
            return Err(RemoteError::Closed);
        }
        if authorized && !self.state.trust.is_trusted(&descriptor.endpoint_id)? {
            return Err(RemoteError::Unauthorized);
        }
        let address = descriptor.address()?;
        let connection = tokio::time::timeout(
            self.state.timeout,
            self.state.endpoint.connect(address, ALPN),
        )
        .await
        .map_err(|_| RemoteError::Timeout)?
        .map_err(|_| RemoteError::Unavailable)?;
        let registration = register_connection(&self.state, connection.clone())?;
        Ok(RemoteClient {
            connection: crate::paths::ObservedConnection::new(connection),
            state: self.state.clone(),
            registration,
            incarnation: descriptor.incarnation.clone(),
            session,
            authorized,
        })
    }
    pub async fn close(&self) {
        self.stop();
        self.state.endpoint.close().await;
        let task = self
            .accept_task
            .lock()
            .ok()
            .and_then(|mut task| task.take());
        if let Some(task) = task {
            let _ = task.await;
        }
    }
    fn stop(&self) {
        self.state.closed.store(true, Ordering::Release);
        if let Ok(mut access) = self.state.access.lock() {
            access.invitations.clear();
            for connection in access.connections.values() {
                connection.close(0u8.into(), b"endpoint closed");
            }
        }
        if let Ok(task) = self.accept_task.lock()
            && let Some(task) = task.as_ref()
        {
            task.abort();
        }
    }
}
impl Drop for RemoteEndpoint {
    fn drop(&mut self) {
        self.stop();
    }
}
impl State {
    fn descriptor(&self) -> EndpointDescriptor {
        let addr = self.endpoint.addr();
        EndpointDescriptor {
            version: PROTOCOL_VERSION,
            endpoint_id: addr.id.to_string(),
            name: self.name.clone(),
            incarnation: self.incarnation.clone(),
            relay_urls: addr.relay_urls().map(ToString::to_string).collect(),
            direct_addresses: addr.ip_addrs().copied().collect(),
        }
    }
}

pub struct RemoteClient {
    connection: crate::paths::ObservedConnection,
    state: Arc<State>,
    registration: u64,
    incarnation: String,
    session: ClientSession,
    authorized: bool,
}
impl RemoteClient {
    pub fn peer_id(&self) -> String {
        self.connection.remote_id().to_string()
    }
    pub fn session(&self) -> ClientSession {
        self.session.clone()
    }
    pub fn connection_status(&self) -> ConnectionStatus {
        crate::paths::snapshot(&self.connection)
    }
    pub fn observe_paths(&self) -> PathObserver {
        PathObserver::new(self.connection.clone())
    }
    /// Allocate before transmitting so a lost reply can be reconciled by ID.
    pub fn next_request_id(&self) -> Result<RequestId> {
        let sequence = self
            .session
            .next
            .try_update(Ordering::AcqRel, Ordering::Acquire, |sequence| {
                sequence.checked_add(1)
            })
            .map_err(|_| RemoteError::Capacity)?;
        Ok(RequestId {
            incarnation: self.incarnation.clone(),
            client: self.session.id.clone(),
            sequence,
        })
    }
    /// A bounded sequence window permits concurrent streams; no operation is retried implicitly.
    pub async fn submit(
        &self,
        id: RequestId,
        view: ClientViewId,
        execution: ExecutionContext,
        command: Command,
    ) -> Result<RequestStatus> {
        id.validate(&self.incarnation)?;
        match self
            .rpc(Request::Submit {
                id,
                view,
                execution,
                command: Box::new(command),
            })
            .await?
        {
            Reply::Status(status) => Ok(status),
            _ => Err(RemoteError::Protocol),
        }
    }
    pub async fn query(&self, id: RequestId) -> Result<RequestStatus> {
        match self.rpc(Request::Query { id }).await? {
            Reply::Status(status) => Ok(status),
            _ => Err(RemoteError::Protocol),
        }
    }
    pub async fn call(
        &self,
        view: ClientViewId,
        execution: ExecutionContext,
        command: Command,
    ) -> std::result::Result<ApplicationReply, CallError> {
        let id = self.next_request_id().map_err(|error| CallError {
            request_id: None,
            failure: CallFailure::Transport(error),
        })?;
        let result = async {
            let mut status = self
                .submit(id.clone(), view, execution, command)
                .await
                .map_err(CallFailure::Transport)?;
            let deadline = Instant::now() + self.state.timeout;
            loop {
                match status {
                    RequestStatus::Completed(result) => {
                        return result
                            .map(|reply| *reply)
                            .map_err(|failure| CallFailure::Business(Box::new(failure)));
                    }
                    RequestStatus::Failed(error) => return Err(CallFailure::Transport(error)),
                    RequestStatus::Unknown | RequestStatus::ResultExpired => {
                        return Err(CallFailure::ResultUnavailable);
                    }
                    RequestStatus::Running => {}
                }
                if Instant::now() >= deadline {
                    return Err(CallFailure::Transport(RemoteError::Timeout));
                }
                tokio::time::sleep(Duration::from_millis(25)).await;
                status = self
                    .query(id.clone())
                    .await
                    .map_err(CallFailure::Transport)?;
            }
        }
        .await;
        result.map_err(|failure| CallError {
            request_id: Some(id),
            failure,
        })
    }
    pub fn close(&self) {
        self.connection.close(0u8.into(), b"client closed");
    }
    async fn rpc(&self, request: Request) -> Result<Reply> {
        if self.state.closed.load(Ordering::Acquire) {
            return Err(RemoteError::Closed);
        }
        if self.authorized && !self.state.trust.is_trusted(&self.peer_id())? {
            return Err(RemoteError::Unauthorized);
        }
        tokio::time::timeout(self.state.timeout, async {
            let (mut send, mut recv) = self
                .connection
                .open_bi()
                .await
                .map_err(|_| RemoteError::Unavailable)?;
            write_frame(
                &mut send,
                Envelope {
                    version: PROTOCOL_VERSION,
                    body: request,
                },
            )
            .await?;
            let reply: Envelope<Reply> = read_frame(&mut recv, MAX_FRAME_BYTES).await?;
            if reply.version != PROTOCOL_VERSION {
                return Err(RemoteError::IncompatibleVersion);
            }
            match reply.body {
                Reply::Error(error) => Err(error),
                reply => Ok(reply),
            }
        })
        .await
        .map_err(|_| RemoteError::Timeout)?
    }
}
impl Drop for RemoteClient {
    fn drop(&mut self) {
        self.close();
        if let Ok(mut access) = self.state.access.lock() {
            access.connections.remove(&self.registration);
        }
    }
}

pub enum CallFailure {
    Transport(RemoteError),
    Business(Box<Failure>),
    ResultUnavailable,
}
/// The request ID is retained on uncertain outcomes; callers query it instead of resending a mutation.
pub struct CallError {
    pub request_id: Option<RequestId>,
    pub failure: CallFailure,
}
impl std::fmt::Debug for CallError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("CallError")
            .field("request_id", &self.request_id)
            .finish_non_exhaustive()
    }
}
impl std::fmt::Display for CallError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match &self.failure {
            CallFailure::Transport(error) => error.fmt(f),
            CallFailure::Business(_) => f.write_str("远程业务请求未成功"),
            CallFailure::ResultUnavailable => f.write_str("请求结果已不可用；不得自动重新提交"),
        }
    }
}
impl std::error::Error for CallError {}

fn register_connection(state: &State, connection: Connection) -> Result<u64> {
    let mut access = state.access.lock().map_err(|_| RemoteError::Unavailable)?;
    if state.closed.load(Ordering::Acquire) {
        return Err(RemoteError::Closed);
    }
    if access.connections.len() >= 128 {
        connection.close(2u8.into(), b"capacity");
        return Err(RemoteError::Capacity);
    }
    let id = state.next_connection.fetch_add(1, Ordering::Relaxed);
    access.connections.insert(id, connection);
    Ok(id)
}
async fn accept_loop(state: Arc<State>) {
    let permits = Arc::new(Semaphore::new(64));
    while let Some(incoming) = state.endpoint.accept().await {
        let Ok(permit) = permits.clone().try_acquire_owned() else {
            incoming.refuse();
            continue;
        };
        let state = state.clone();
        tokio::spawn(async move {
            let _permit = permit;
            let Ok(Ok(connection)) = tokio::time::timeout(state.timeout, incoming).await else {
                return;
            };
            let Ok(registration) = register_connection(&state, connection.clone()) else {
                return;
            };
            serve_connection(state.clone(), connection).await;
            if let Ok(mut access) = state.access.lock() {
                access.connections.remove(&registration);
            }
        });
    }
}
async fn serve_connection(state: Arc<State>, connection: Connection) {
    let permits = Arc::new(Semaphore::new(8));
    loop {
        let streams = tokio::time::timeout(Duration::from_secs(300), connection.accept_bi()).await;
        let Ok(Ok((mut send, mut recv))) = streams else {
            connection.close(0u8.into(), b"idle or closed");
            break;
        };
        let Ok(permit) = permits.clone().try_acquire_owned() else {
            let _ = send.reset(2u8.into());
            let _ = recv.stop(2u8.into());
            continue;
        };
        let state = state.clone();
        let connection = connection.clone();
        tokio::spawn(async move {
            let _permit = permit;
            let result = tokio::time::timeout(state.timeout, async {
                let _request_permit = state
                    .requests
                    .acquire()
                    .await
                    .map_err(|_| RemoteError::Closed)?;
                let limit = if state
                    .trust
                    .is_trusted(&connection.remote_id().to_string())?
                {
                    MAX_FRAME_BYTES
                } else {
                    16 * 1024
                };
                let request = read_frame::<Envelope<Request>>(&mut recv, limit).await;
                let reply = match request {
                    Ok(envelope) if envelope.version == PROTOCOL_VERSION => {
                        let owner = state.clone();
                        let peer = connection.remote_id().to_string();
                        tokio::task::spawn_blocking(move || dispatch(&owner, peer, envelope.body))
                            .await
                            .map_err(|_| RemoteError::Unavailable)?
                    }
                    Ok(_) => Reply::Error(RemoteError::IncompatibleVersion),
                    Err(error) => Reply::Error(error),
                };
                write_frame(
                    &mut send,
                    Envelope {
                        version: PROTOCOL_VERSION,
                        body: reply,
                    },
                )
                .await
            })
            .await;
            if !matches!(result, Ok(Ok(()))) {
                let _ = send.reset(1u8.into());
                let _ = recv.stop(1u8.into());
            }
        });
    }
}
pub(crate) fn validate_relay_url(value: &str) -> Result<iroh::RelayUrl> {
    let url: iroh::RelayUrl = value.parse().map_err(|_| RemoteError::Configuration)?;
    if !matches!(url.scheme(), "https" | "http")
        || url.host_str().is_none()
        || !url.username().is_empty()
        || url.password().is_some()
        || url.query().is_some()
        || url.fragment().is_some()
    {
        return Err(RemoteError::Configuration);
    }
    Ok(url)
}
fn dispatch(state: &Arc<State>, peer: String, request: Request) -> Reply {
    match try_dispatch(state, peer, request) {
        Ok(reply) => reply,
        Err(error) => Reply::Error(error),
    }
}
fn try_dispatch(state: &Arc<State>, peer: String, request: Request) -> Result<Reply> {
    let mut access = state.access.lock().map_err(|_| RemoteError::Unavailable)?;
    if state.closed.load(Ordering::Acquire) {
        return Err(RemoteError::Closed);
    }
    if let Request::Pair {
        token,
        name,
        incarnation,
    } = request
    {
        validate_name(&name)?;
        if incarnation != state.incarnation {
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
        state.trust.trust(TrustedDevice { id: peer, name })?;
        access.invitations.remove(index);
        return Ok(Reply::Paired);
    }
    if !state.trust.is_trusted(&peer)? {
        return Err(RemoteError::Unauthorized);
    }
    match request {
        Request::Hello => Ok(Reply::Hello(state.descriptor())),
        Request::Query { id } => {
            id.validate(&state.incarnation)?;
            Ok(Reply::Status(access.ledger.status(&peer, &id)?))
        }
        Request::Submit {
            id,
            view,
            execution,
            command,
        } => {
            id.validate(&state.incarnation)?;
            if view.0.is_empty() || view.0.len() > 128 || view.0.chars().any(char::is_control) {
                return Err(RemoteError::Configuration);
            }
            let handler = state.handler.clone().ok_or(RemoteError::NoHandler)?;
            let fingerprint: [u8; 32] = Sha256::digest(
                serde_json::to_vec(&(&view, &execution, &command))
                    .map_err(|_| RemoteError::Protocol)?,
            )
            .into();
            if access.ledger.accept(&peer, &id, fingerprint)? {
                let state = state.clone();
                let task_peer = peer.clone();
                let ledger_peer = peer.clone();
                let task_id = id.clone();
                tokio::spawn(async move {
                    // The accepted command survives connection shutdown. Panics become a bounded failure.
                    let task = tokio::spawn(async move {
                        handler
                            .execute(
                                RequestContext {
                                    peer_id: task_peer,
                                    view,
                                    execution,
                                },
                                *command,
                            )
                            .await
                    });
                    let status = match task.await {
                        Ok(result) => RequestStatus::Completed(result.map(Box::new)),
                        Err(_) => RequestStatus::Failed(RemoteError::Unavailable),
                    };
                    if let Ok(mut access) = state.access.lock() {
                        access.ledger.complete(ledger_peer, task_id, status);
                    }
                });
            }
            Ok(Reply::Status(access.ledger.status(&peer, &id)?))
        }
        Request::Pair { .. } => Err(RemoteError::Protocol),
    }
}
