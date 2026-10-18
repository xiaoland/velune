use crate::{
    paths::{ConnectionSlot, ObservedConnection},
    protocol::{ALPN, Envelope, Reply, Request},
    service::{RemoteAccessService, ServiceState},
    wire::{read_frame, write_frame},
    *,
};
use iroh::{
    Endpoint, RelayMode,
    endpoint::{Connection, presets},
};
use std::{
    collections::HashMap,
    sync::{
        Arc, Mutex,
        atomic::{AtomicBool, AtomicU64, Ordering},
    },
    time::Duration,
};
use tokio::{
    sync::{OwnedRwLockReadGuard, RwLock, Semaphore},
    task::JoinHandle,
};

pub(crate) fn validate_config(config: &EndpointConfig) -> Result<()> {
    if config.request_timeout.is_zero()
        || config.request_timeout > Duration::from_secs(300)
        || config.bind_addresses.len() > 16
    {
        return Err(RemoteError::Configuration);
    }
    if let RelayConfig::Custom { urls } = &config.relay {
        if urls.is_empty() || urls.len() > 16 {
            return Err(RemoteError::Configuration);
        }
        for url in urls {
            validate_relay_url(url)?;
        }
    }
    if let DiscoveryConfig::CustomPkarr { url } = &config.discovery {
        validate_relay_url(url)?;
    }
    if matches!(config.path_policy, PathPolicy::RelayOnly)
        && (matches!(config.relay, RelayConfig::Disabled) || !config.bind_addresses.is_empty())
    {
        return Err(RemoteError::Configuration);
    }
    Ok(())
}
/// Shared native transport assembly; protocol authorization remains with each caller.
pub(crate) async fn bind_transport(
    key: iroh::SecretKey,
    config: EndpointConfig,
    alpn: &[u8],
) -> Result<Endpoint> {
    validate_config(&config)?;
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

struct RegisteredConnection {
    incoming: bool,
    slot: Arc<ConnectionSlot>,
}
/// Only this generation and its tracked IO own native endpoint/connection references.
pub(crate) struct NetworkGeneration {
    endpoint: Mutex<Option<Endpoint>>,
    pub(crate) service: Arc<ServiceState>,
    pub(crate) id: u64,
    pub(crate) timeout: Duration,
    lookup_url: Option<String>,
    closed: AtomicBool,
    cancelled: tokio::sync::watch::Sender<bool>,
    operations: Arc<RwLock<()>>,
    connections: Mutex<HashMap<u64, RegisteredConnection>>,
    next_connection: AtomicU64,
    tasks: Mutex<Vec<JoinHandle<()>>>,
    requests: Semaphore,
}
impl NetworkGeneration {
    pub(crate) async fn bind(
        service: Arc<ServiceState>,
        id: u64,
        config: EndpointConfig,
    ) -> Result<Arc<Self>> {
        let endpoint = bind_transport(service.identity.key.clone(), config.clone(), ALPN).await?;
        Ok(Arc::new(Self {
            endpoint: Mutex::new(Some(endpoint)),
            service,
            id,
            timeout: config.request_timeout,
            lookup_url: match &config.discovery {
                DiscoveryConfig::CustomPkarr { url } => Some(url.clone()),
                _ => None,
            },
            closed: AtomicBool::new(false),
            cancelled: tokio::sync::watch::channel(false).0,
            operations: Arc::new(RwLock::new(())),
            connections: Mutex::new(HashMap::new()),
            next_connection: AtomicU64::new(1),
            tasks: Mutex::new(Vec::new()),
            requests: Semaphore::new(8),
        }))
    }
    pub(crate) fn start(self: &Arc<Self>) {
        let owner = self.clone();
        self.track(tokio::spawn(async move {
            accept_loop(owner).await;
        }));
    }
    pub(crate) fn endpoint(&self) -> Result<Endpoint> {
        self.endpoint
            .lock()
            .map_err(|_| RemoteError::Unavailable)?
            .clone()
            .ok_or(RemoteError::Closed)
    }
    pub(crate) fn descriptor(&self) -> Result<EndpointDescriptor> {
        let endpoint = self.endpoint.lock().map_err(|_| RemoteError::Unavailable)?;
        let addr = endpoint.as_ref().ok_or(RemoteError::Closed)?.addr();
        Ok(EndpointDescriptor {
            version: PROTOCOL_VERSION,
            endpoint_id: addr.id.to_string(),
            name: self.service.identity.name.clone(),
            incarnation: self.service.incarnation.clone(),
            relay_urls: addr.relay_urls().map(ToString::to_string).collect(),
            direct_addresses: addr.ip_addrs().copied().collect(),
            lookup_url: self.lookup_url.clone(),
        })
    }
    pub(crate) async fn operation(&self) -> Result<OwnedRwLockReadGuard<()>> {
        let guard = self.operations.clone().read_owned().await;
        if self.closed.load(Ordering::Acquire) {
            return Err(RemoteError::Closed);
        }
        Ok(guard)
    }
    pub(crate) async fn cancelled(&self) {
        let mut cancelled = self.cancelled.subscribe();
        if !*cancelled.borrow() {
            let _ = cancelled.changed().await;
        }
    }
    pub(crate) async fn network_change(&self) -> Result<()> {
        let _operation = self.operation().await?;
        let endpoint = self.endpoint()?;
        tokio::select! {
            biased;
            _=self.cancelled()=>Err(RemoteError::Closed),
            _=endpoint.network_change()=>Ok(()),
        }
    }
    pub(crate) fn register(
        &self,
        connection: Connection,
        incoming: bool,
    ) -> Result<(u64, Arc<ConnectionSlot>)> {
        let access = self.service.access()?;
        ServiceState::check_generation(&access, self.id)?;
        if self.closed.load(Ordering::Acquire) || (incoming && !access.ingress) {
            connection.close(0u8.into(), b"ingress closed");
            return Err(RemoteError::Closed);
        }
        let mut connections = self
            .connections
            .lock()
            .map_err(|_| RemoteError::Unavailable)?;
        if connections.len() >= 128 {
            connection.close(2u8.into(), b"capacity");
            return Err(RemoteError::Capacity);
        }
        let id = self.next_connection.fetch_add(1, Ordering::Relaxed);
        let slot = ConnectionSlot::new(ObservedConnection::new(connection));
        connections.insert(
            id,
            RegisteredConnection {
                incoming,
                slot: slot.clone(),
            },
        );
        Ok((id, slot))
    }
    pub(crate) fn remove(&self, id: u64) {
        if let Ok(mut connections) = self.connections.lock() {
            connections.remove(&id);
        }
    }
    pub(crate) fn close_incoming(&self) {
        if let Ok(connections) = self.connections.lock() {
            for connection in connections.values().filter(|value| value.incoming) {
                connection.slot.close(b"ingress closed");
            }
        }
    }
    pub(crate) fn revoke(&self, peer: &str) {
        if let Ok(connections) = self.connections.lock() {
            for connection in connections
                .values()
                .filter(|value| value.incoming && value.slot.status().peer_id == peer)
            {
                connection.slot.close(b"authorization revoked");
            }
        }
    }
    fn track(&self, task: JoinHandle<()>) {
        let mut tasks = self
            .tasks
            .lock()
            .unwrap_or_else(|poison| poison.into_inner());
        tasks.retain(|task| !task.is_finished());
        if self.closed.load(Ordering::Acquire) {
            task.abort();
        }
        // Even cancelled tasks are retained until the unique close joins their resource destructors.
        tasks.push(task);
    }
    pub(crate) fn stop(&self) -> Option<Endpoint> {
        self.closed.store(true, Ordering::Release);
        self.cancelled.send_replace(true);
        if let Ok(connections) = self.connections.lock() {
            for connection in connections.values() {
                connection.slot.close(b"network generation closed");
            }
        }
        if let Ok(tasks) = self.tasks.lock() {
            for task in tasks.iter() {
                task.abort();
            }
        }
        self.endpoint
            .lock()
            .unwrap_or_else(|poison| poison.into_inner())
            .take()
    }
    pub(crate) async fn close(&self) {
        let endpoint = self.stop();
        if let Some(endpoint) = &endpoint {
            endpoint.close().await;
        }
        loop {
            let tasks = std::mem::take(
                &mut *self
                    .tasks
                    .lock()
                    .unwrap_or_else(|poison| poison.into_inner()),
            );
            if tasks.is_empty() {
                break;
            }
            for task in tasks {
                let _ = task.await;
            }
        }
        let _drained = self.operations.write().await;
        self.connections
            .lock()
            .unwrap_or_else(|poison| poison.into_inner())
            .clear();
        drop(endpoint);
    }
}
impl RemoteAccessService {
    pub(crate) async fn connect_client(
        &self,
        peer: &str,
        session: ClientSession,
    ) -> Result<RemoteClient> {
        let contact = self.state.store.contact(peer)?;
        if !matches!(contact.confirmation, ContactConfirmation::Confirmed { .. }) {
            return Err(RemoteError::Unauthorized);
        }
        let mut client = self
            .dial_locator(peer, contact.name, &contact.locator, session, true)
            .await?;
        match client.rpc(Request::Hello).await? {
            Reply::Hello(current) => {
                current.validate(peer)?;
                {
                    let _access = self.state.access()?;
                    self.state.store.update_locator(
                        peer,
                        current.name.clone(),
                        DeviceLocator {
                            relay_urls: current.relay_urls.clone(),
                            direct_addresses: current.direct_addresses.clone(),
                            lookup_url: current.lookup_url.clone(),
                        },
                    )?;
                }
                client.descriptor = current;
                Ok(client)
            }
            _ => Err(RemoteError::Protocol),
        }
    }
    pub(crate) async fn dial_locator(
        &self,
        peer: &str,
        name: String,
        locator: &DeviceLocator,
        session: ClientSession,
        authorized: bool,
    ) -> Result<RemoteClient> {
        if authorized && !self.state.store.is_pinned(peer)? {
            return Err(RemoteError::Unauthorized);
        }
        let address = locator.address(peer)?;
        let network = self.network()?;
        let _operation = network.operation().await?;
        let endpoint = network.endpoint()?;
        let connection = tokio::select! {
            biased;
            _=network.cancelled()=>return Err(RemoteError::Closed),
            result=tokio::time::timeout(network.timeout,endpoint.connect(address,ALPN))=>result.map_err(|_|RemoteError::Timeout)?.map_err(|_|RemoteError::Unavailable)?,
        };
        let (registration, connection) = network.register(connection, false)?;
        Ok(RemoteClient {
            connection,
            network: Arc::downgrade(&network),
            store: self.state.store.clone(),
            registration,
            descriptor: EndpointDescriptor {
                version: PROTOCOL_VERSION,
                endpoint_id: peer.into(),
                name,
                incarnation: String::new(),
                relay_urls: locator.relay_urls.clone(),
                direct_addresses: locator.direct_addresses.clone(),
                lookup_url: locator.lookup_url.clone(),
            },
            session,
            authorized,
            timeout: network.timeout,
        })
    }
}
async fn accept_loop(network: Arc<NetworkGeneration>) {
    let Ok(endpoint) = network.endpoint() else {
        return;
    };
    let permits = Arc::new(Semaphore::new(64));
    while let Some(incoming) = endpoint.accept().await {
        let allowed = network.service.access().is_ok_and(|access| {
            access.ingress && access.generation == Some(network.id) && !access.closed
        });
        if !allowed {
            incoming.refuse();
            continue;
        }
        let Ok(permit) = permits.clone().try_acquire_owned() else {
            incoming.refuse();
            continue;
        };
        let owner = network.clone();
        let task = tokio::spawn(async move {
            let _permit = permit;
            let Ok(Ok(connection)) = tokio::time::timeout(owner.timeout, incoming).await else {
                return;
            };
            let Ok((registration, connection)) = owner.register(connection, true) else {
                return;
            };
            let connection_id = connection.status().connection_id;
            serve_connection(owner.clone(), connection).await;
            owner.service.pairing_disconnected(connection_id);
            owner.remove(registration);
        });
        network.track(task);
    }
}
async fn serve_connection(network: Arc<NetworkGeneration>, slot: Arc<ConnectionSlot>) {
    let Some(connection) = slot.native() else {
        return;
    };
    let permits = Arc::new(Semaphore::new(8));
    loop {
        let streams = tokio::time::timeout(Duration::from_secs(300), connection.accept_bi()).await;
        let Ok(Ok((mut send, mut recv))) = streams else {
            slot.close(b"idle or closed");
            break;
        };
        let Ok(permit) = permits.clone().try_acquire_owned() else {
            let _ = send.reset(2u8.into());
            let _ = recv.stop(2u8.into());
            continue;
        };
        let owner = network.clone();
        let request_slot = slot.clone();
        let peer = connection.remote_id().to_string();
        let task = tokio::spawn(async move {
            let _permit = permit;
            let result = tokio::time::timeout(owner.timeout, async {
                let _request = owner
                    .requests
                    .acquire()
                    .await
                    .map_err(|_| RemoteError::Closed)?;
                let limit = if owner.service.store.grant(&peer)?.is_some() {
                    MAX_FRAME_BYTES
                } else {
                    16 * 1024
                };
                let envelope =
                    read_frame::<Envelope<crate::protocol::Request>>(&mut recv, limit).await;
                let reply = match envelope {
                    Ok(envelope) if envelope.version == PROTOCOL_VERSION => {
                        if matches!(
                            envelope.body,
                            Request::PairBegin { .. }
                                | Request::PairReady { .. }
                                | Request::PairStatus { .. }
                                | Request::PairCancel { .. }
                        ) {
                            let native = request_slot.native().ok_or(RemoteError::Closed)?;
                            owner.service.dispatch_pairing(
                                owner.id,
                                &native,
                                &request_slot,
                                envelope.body,
                            )
                        } else {
                            // Blocking authorization/storage only retains stable service state, never this generation.
                            let service = owner.service.clone();
                            let generation = owner.id;
                            let descriptor = owner.descriptor()?;
                            tokio::task::spawn_blocking(move || {
                                service.dispatch(generation, peer, descriptor, envelope.body)
                            })
                            .await
                            .map_err(|_| RemoteError::Unavailable)?
                        }
                    }
                    Ok(_) => crate::protocol::Reply::Error(RemoteError::IncompatibleVersion),
                    Err(error) => crate::protocol::Reply::Error(error),
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
        network.track(task);
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
