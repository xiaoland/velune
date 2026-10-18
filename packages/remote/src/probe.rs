//! Isolated transport marker traffic. It has no application, pairing or persistent identity.
use crate::{
    ConnectionDiagnostics, ConnectionStatus, EndpointConfig, PathObserver, PlatformNetworkSnapshot,
    RemoteError, Result,
    endpoint::{bind_transport, validate_relay_url},
    identity::random_bytes,
    paths::ObservedConnection,
};
use futures_util::StreamExt;
use iroh::{Endpoint, EndpointAddr, EndpointId, SecretKey, address_lookup::AddressLookup};
use iroh_mdns_address_lookup::{DiscoveryEvent, MdnsAddressLookup};
use serde::{Deserialize, Serialize};
use std::{
    collections::{HashMap, VecDeque},
    net::SocketAddr,
    sync::{
        Arc, Mutex,
        atomic::{AtomicBool, Ordering},
    },
    time::Duration,
};
use tokio::{
    sync::{OwnedSemaphorePermit, Semaphore},
    task::JoinHandle,
};

const ALPN: &[u8] = b"velune/transport-probe/1";
const MAGIC: &[u8; 8] = b"VLNPRB01";
const HEADER_BYTES: usize = 20;
const MAX_PAYLOAD_BYTES: usize = 1024;
const MAX_CONNECTIONS: usize = 16;
const MAX_STREAMS: usize = 8;
const MAX_ROUND_TRIP_TIMEOUT: Duration = Duration::from_secs(30);
const MAX_MDNS_EVENTS: usize = 512;
const MAX_MDNS_RESOLVES: usize = 8;
const MAX_MDNS_TIMEOUT: Duration = Duration::from_secs(15);

#[derive(Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ProbeTarget {
    pub endpoint_id: String,
    pub relay_urls: Vec<String>,
    pub direct_addresses: Vec<SocketAddr>,
}
impl ProbeTarget {
    fn address(&self) -> Result<EndpointAddr> {
        if self.relay_urls.len() > 16 || self.direct_addresses.len() > 32 {
            return Err(RemoteError::Configuration);
        }
        let id = self
            .endpoint_id
            .parse::<EndpointId>()
            .map_err(|_| RemoteError::Configuration)?;
        let mut address = EndpointAddr::new(id);
        for relay in &self.relay_urls {
            address = address.with_relay_url(validate_relay_url(relay)?);
        }
        for ip in &self.direct_addresses {
            address = address.with_ip_addr(*ip);
        }
        Ok(address)
    }
}

#[derive(Clone, Serialize, Deserialize)]
pub struct ProbeReply {
    pub sequence: u64,
    pub payload_matches: bool,
    pub elapsed_microseconds: u64,
    pub received_bytes: u64,
}

/// Private discovery evidence, independent of connection path selection.
#[derive(Clone, Serialize, Deserialize)]
#[serde(tag = "event", rename_all = "camelCase")]
pub enum ProbeMdnsEvent {
    Discovered {
        sampled_at_micros: u64,
        endpoint_id: String,
        direct_addresses: Vec<SocketAddr>,
    },
    Expired {
        sampled_at_micros: u64,
        endpoint_id: String,
        direct_addresses: Vec<SocketAddr>,
    },
    /// Loss since the previous drain, excluding any loss inside upstream subscriber queues.
    Dropped { count: u64 },
}
#[derive(Default)]
struct MdnsEvents {
    events: VecDeque<ProbeMdnsEvent>,
    dropped: u64,
}
impl MdnsEvents {
    fn record(&mut self, event: ProbeMdnsEvent) {
        if self.events.len() == MAX_MDNS_EVENTS {
            self.events.pop_front();
            self.dropped = self.dropped.saturating_add(1);
        }
        self.events.push_back(event);
    }
}
struct EnabledMdns {
    service: MdnsAddressLookup,
    subscription: JoinHandle<()>,
}

struct State {
    endpoint: Endpoint,
    closed: AtomicBool,
    connections: Mutex<HashMap<usize, ObservedConnection>>,
    tasks: Mutex<Vec<JoinHandle<()>>>,
    slots: Arc<Semaphore>,
    handshake_timeout: Duration,
}

pub struct TransportProbe {
    state: Arc<State>,
    accept_task: Mutex<Option<JoinHandle<()>>>,
    shutdown_gate: tokio::sync::Mutex<()>,
    shutdown_complete: AtomicBool,
    mdns: Mutex<Option<EnabledMdns>>,
    mdns_events: Arc<Mutex<MdnsEvents>>,
    mdns_resolves: Semaphore,
}
impl TransportProbe {
    pub async fn bind(config: EndpointConfig) -> Result<Self> {
        let handshake_timeout = config.request_timeout;
        let endpoint =
            bind_transport(SecretKey::from_bytes(&random_bytes()?), config, ALPN).await?;
        let state = Arc::new(State {
            endpoint,
            closed: AtomicBool::new(false),
            connections: Mutex::new(HashMap::new()),
            tasks: Mutex::new(Vec::new()),
            slots: Arc::new(Semaphore::new(MAX_CONNECTIONS)),
            handshake_timeout,
        });
        let accept_task = tokio::spawn(accept_loop(state.clone()));
        Ok(Self {
            state,
            accept_task: Mutex::new(Some(accept_task)),
            shutdown_gate: tokio::sync::Mutex::new(()),
            shutdown_complete: AtomicBool::new(false),
            mdns: Mutex::new(None),
            mdns_events: Arc::new(Mutex::new(MdnsEvents::default())),
            mdns_resolves: Semaphore::new(MAX_MDNS_RESOLVES),
        })
    }

    pub fn descriptor(&self) -> ProbeTarget {
        let address = self.state.endpoint.addr();
        ProbeTarget {
            endpoint_id: address.id.to_string(),
            relay_urls: address.relay_urls().map(ToString::to_string).collect(),
            direct_addresses: address.ip_addrs().copied().collect(),
        }
    }

    /// Creates exactly one connection. No failure or network event triggers redial.
    pub async fn connect(&self, target: &ProbeTarget) -> Result<ProbeConnection> {
        if self.state.closed.load(Ordering::Acquire) {
            return Err(RemoteError::Closed);
        }
        let address = target.address()?;
        let permit = self
            .state
            .slots
            .clone()
            .try_acquire_owned()
            .map_err(|_| RemoteError::Capacity)?;
        let connection = tokio::time::timeout(
            self.state.handshake_timeout,
            self.state.endpoint.connect(address, ALPN),
        )
        .await
        .map_err(|_| RemoteError::Timeout)?
        .map_err(|_| RemoteError::Unavailable)?;
        let observed = ObservedConnection::new(connection);
        register(&self.state, observed.clone(), permit)?;
        Ok(ProbeConnection {
            connection: observed,
            state: self.state.clone(),
            streams: Semaphore::new(MAX_STREAMS),
        })
    }

    /// An acknowledged platform sequence is not evidence that paths or sockets changed.
    pub async fn report_network_change(&self, snapshot: PlatformNetworkSnapshot) -> Result<u64> {
        snapshot.validate()?;
        if self.state.closed.load(Ordering::Acquire) {
            return Err(RemoteError::Closed);
        }
        self.state.endpoint.network_change().await;
        Ok(snapshot.sequence)
    }

    /// Includes both accepted and dialed live connections; raw addresses are private evidence.
    pub fn connections(&self) -> Vec<ConnectionDiagnostics> {
        self.state
            .connections
            .lock()
            .unwrap_or_else(|poison| poison.into_inner())
            .values()
            .map(crate::paths::diagnostics)
            .collect()
    }

    /// Opt in once, with no competing lookup service; ordinary QNT probes remain unchanged.
    pub async fn enable_mdns(&self) -> Result<()> {
        let _guard = self.shutdown_gate.lock().await;
        if self.state.closed.load(Ordering::Acquire) {
            return Err(RemoteError::Closed);
        }
        let registry = self
            .state
            .endpoint
            .address_lookup()
            .map_err(|_| RemoteError::Unavailable)?;
        if !registry.is_empty()
            || self
                .mdns
                .lock()
                .unwrap_or_else(|poison| poison.into_inner())
                .is_some()
        {
            return Err(RemoteError::Configuration);
        }
        let service = MdnsAddressLookup::builder()
            .build(self.state.endpoint.id())
            .map_err(|_| RemoteError::Unavailable)?;
        let mut events = service.subscribe().await;
        let ring = self.mdns_events.clone();
        let subscription = tokio::spawn(async move {
            while let Some(event) = events.next().await {
                let sampled_at_micros = crate::monotonic_microseconds();
                let event = match event {
                    DiscoveryEvent::Discovered { endpoint_info, .. } => {
                        let addresses = endpoint_info
                            .ip_addrs()
                            .copied()
                            .take(33)
                            .collect::<Vec<_>>();
                        if addresses.len() > 32 {
                            let mut ring = ring.lock().unwrap_or_else(|poison| poison.into_inner());
                            ring.dropped = ring.dropped.saturating_add(1);
                            continue;
                        }
                        ProbeMdnsEvent::Discovered {
                            sampled_at_micros,
                            endpoint_id: endpoint_info.endpoint_id.to_string(),
                            direct_addresses: addresses,
                        }
                    }
                    DiscoveryEvent::Expired { endpoint_id } => ProbeMdnsEvent::Expired {
                        sampled_at_micros,
                        endpoint_id: endpoint_id.to_string(),
                        direct_addresses: Vec::new(),
                    },
                    _ => continue,
                };
                ring.lock()
                    .unwrap_or_else(|poison| poison.into_inner())
                    .record(event);
            }
        });
        registry.add(service.clone());
        *self
            .mdns
            .lock()
            .unwrap_or_else(|poison| poison.into_inner()) = Some(EnabledMdns {
            service,
            subscription,
        });
        Ok(())
    }

    /// Drain the bounded ring. Dropped counts loss since the previous drain, not upstream loss.
    /// Addresses are private diagnostic data; consumers share this one event queue.
    pub fn mdns_events(&self) -> Vec<ProbeMdnsEvent> {
        let mut ring = self
            .mdns_events
            .lock()
            .unwrap_or_else(|poison| poison.into_inner());
        let mut events = Vec::with_capacity(ring.events.len() + usize::from(ring.dropped > 0));
        if ring.dropped > 0 {
            events.push(ProbeMdnsEvent::Dropped {
                count: std::mem::take(&mut ring.dropped),
            });
        }
        events.extend(ring.events.drain(..));
        events
    }

    /// Read this mDNS service's first result, with at most eight concurrent queries and no dial.
    pub async fn mdns_resolve(
        &self,
        endpoint_id: String,
        timeout: Duration,
    ) -> Result<ProbeTarget> {
        if timeout.is_zero() || timeout > MAX_MDNS_TIMEOUT {
            return Err(RemoteError::Configuration);
        }
        let endpoint_id = endpoint_id
            .parse::<EndpointId>()
            .map_err(|_| RemoteError::Configuration)?;
        if self.state.closed.load(Ordering::Acquire) {
            return Err(RemoteError::Closed);
        }
        let _permit = self
            .mdns_resolves
            .try_acquire()
            .map_err(|_| RemoteError::Capacity)?;
        // The returned stream owns channel endpoints, not a clone of the service's publisher handle.
        let mut stream = self
            .mdns
            .lock()
            .unwrap_or_else(|poison| poison.into_inner())
            .as_ref()
            .ok_or(RemoteError::Configuration)?
            .service
            .resolve(endpoint_id)
            .ok_or(RemoteError::Unavailable)?;
        let item = tokio::time::timeout(timeout, stream.next())
            .await
            .map_err(|_| RemoteError::Timeout)?
            .ok_or(RemoteError::Unavailable)?
            .map_err(|_| RemoteError::Unavailable)?;
        if self.state.closed.load(Ordering::Acquire) {
            return Err(RemoteError::Closed);
        }
        if item.endpoint_id() != endpoint_id {
            return Err(RemoteError::Protocol);
        }
        let info = item.endpoint_info();
        let target = ProbeTarget {
            endpoint_id: endpoint_id.to_string(),
            relay_urls: info
                .relay_urls()
                .take(17)
                .map(ToString::to_string)
                .collect(),
            direct_addresses: info.ip_addrs().take(33).copied().collect(),
        };
        target.address()?;
        Ok(target)
    }

    pub async fn close(&self) {
        let _guard = self.shutdown_gate.lock().await;
        if self.shutdown_complete.load(Ordering::Acquire) {
            return;
        }
        let mdns_subscription = self.stop();
        self.state.endpoint.close().await;
        if let Some(task) = mdns_subscription {
            let _ = task.await;
        }
        let accept = self
            .accept_task
            .lock()
            .unwrap_or_else(|poison| poison.into_inner())
            .take();
        if let Some(task) = accept {
            let _ = task.await;
        }
        let tasks = std::mem::take(
            &mut *self
                .state
                .tasks
                .lock()
                .unwrap_or_else(|poison| poison.into_inner()),
        );
        for task in tasks {
            let _ = task.await;
        }
        self.state
            .connections
            .lock()
            .unwrap_or_else(|poison| poison.into_inner())
            .clear();
        self.shutdown_complete.store(true, Ordering::Release);
    }

    fn stop(&self) -> Option<JoinHandle<()>> {
        self.state.closed.store(true, Ordering::Release);
        if let Ok(registry) = self.state.endpoint.address_lookup() {
            registry.clear();
        }
        let mdns = self
            .mdns
            .lock()
            .unwrap_or_else(|poison| poison.into_inner())
            .take();
        let mdns_subscription = mdns.map(|enabled| {
            enabled.subscription.abort();
            drop(enabled.service);
            enabled.subscription
        });
        if let Some(task) = self
            .accept_task
            .lock()
            .unwrap_or_else(|poison| poison.into_inner())
            .as_ref()
        {
            task.abort();
        }
        for task in self
            .state
            .tasks
            .lock()
            .unwrap_or_else(|poison| poison.into_inner())
            .iter()
        {
            task.abort();
        }
        for connection in self
            .state
            .connections
            .lock()
            .unwrap_or_else(|poison| poison.into_inner())
            .values()
        {
            connection.close(0u8.into(), b"probe closed");
        }
        mdns_subscription
    }
}
impl Drop for TransportProbe {
    fn drop(&mut self) {
        let _ = self.stop();
    }
}

pub struct ProbeConnection {
    connection: ObservedConnection,
    state: Arc<State>,
    streams: Semaphore,
}
impl ProbeConnection {
    pub fn connection_status(&self) -> ConnectionStatus {
        crate::paths::snapshot(&self.connection)
    }
    pub fn diagnostics(&self) -> ConnectionDiagnostics {
        crate::paths::diagnostics(&self.connection)
    }
    pub fn observe_paths(&self) -> PathObserver {
        PathObserver::new(self.connection.clone())
    }

    /// Synthetic markers only: one bounded stream, no retries and no business dispatch.
    pub async fn round_trip(
        &self,
        sequence: u64,
        payload: Vec<u8>,
        timeout: Duration,
    ) -> Result<ProbeReply> {
        if payload.len() > MAX_PAYLOAD_BYTES {
            return Err(RemoteError::FrameTooLarge);
        }
        if timeout.is_zero() || timeout > MAX_ROUND_TRIP_TIMEOUT {
            return Err(RemoteError::Configuration);
        }
        if self.state.closed.load(Ordering::Acquire) || self.connection.close_reason().is_some() {
            return Err(RemoteError::Closed);
        }
        let _permit = self
            .streams
            .try_acquire()
            .map_err(|_| RemoteError::Capacity)?;
        let started = tokio::time::Instant::now();
        let deadline = started + timeout;
        let (mut send, mut recv) = tokio::time::timeout_at(deadline, self.connection.open_bi())
            .await
            .map_err(|_| RemoteError::Timeout)?
            .map_err(|_| RemoteError::Unavailable)?;
        let frame = encode(sequence, &payload);
        let outcome = tokio::time::timeout_at(deadline, async {
            send.write_all(&frame)
                .await
                .map_err(|_| RemoteError::Unavailable)?;
            send.finish().map_err(|_| RemoteError::Unavailable)?;
            let response = recv
                .read_to_end(HEADER_BYTES + MAX_PAYLOAD_BYTES)
                .await
                .map_err(|_| RemoteError::Protocol)?;
            let (received_sequence, bytes) = decode(&response)?;
            Ok(ProbeReply {
                sequence: received_sequence,
                payload_matches: received_sequence == sequence && bytes == payload,
                elapsed_microseconds: u64::try_from(started.elapsed().as_micros())
                    .unwrap_or(u64::MAX),
                received_bytes: bytes.len() as u64,
            })
        })
        .await;
        match outcome {
            Ok(Ok(reply)) => Ok(reply),
            failed => {
                let _ = send.reset(1u8.into());
                let _ = recv.stop(1u8.into());
                match failed {
                    Ok(Err(error)) => Err(error),
                    Err(_) => Err(RemoteError::Timeout),
                    Ok(Ok(_)) => unreachable!(),
                }
            }
        }
    }
    pub fn close(&self) {
        self.connection
            .close(0u8.into(), b"probe connection closed");
    }
}
impl Drop for ProbeConnection {
    fn drop(&mut self) {
        self.close();
    }
}

fn encode(sequence: u64, payload: &[u8]) -> Vec<u8> {
    let mut frame = Vec::with_capacity(HEADER_BYTES + payload.len());
    frame.extend_from_slice(MAGIC);
    frame.extend_from_slice(&sequence.to_be_bytes());
    frame.extend_from_slice(&(payload.len() as u32).to_be_bytes());
    frame.extend_from_slice(payload);
    frame
}
fn decode(frame: &[u8]) -> Result<(u64, &[u8])> {
    if frame.len() < HEADER_BYTES || &frame[..8] != MAGIC {
        return Err(RemoteError::Protocol);
    }
    let sequence = u64::from_be_bytes(frame[8..16].try_into().map_err(|_| RemoteError::Protocol)?);
    let length = u32::from_be_bytes(
        frame[16..20]
            .try_into()
            .map_err(|_| RemoteError::Protocol)?,
    ) as usize;
    if length > MAX_PAYLOAD_BYTES || frame.len() != HEADER_BYTES + length {
        return Err(RemoteError::Protocol);
    }
    Ok((sequence, &frame[HEADER_BYTES..]))
}

async fn accept_loop(state: Arc<State>) {
    while let Some(incoming) = state.endpoint.accept().await {
        let Ok(permit) = state.slots.clone().try_acquire_owned() else {
            incoming.refuse();
            continue;
        };
        let accepted = state.clone();
        let task = tokio::spawn(async move {
            let Ok(Ok(connection)) =
                tokio::time::timeout(accepted.handshake_timeout, incoming).await
            else {
                return;
            };
            let connection = ObservedConnection::new(connection);
            if insert(&accepted, &connection).is_err() {
                return;
            }
            serve_connection(accepted, connection, permit).await;
        });
        track(&state, task);
    }
}
fn insert(state: &State, connection: &ObservedConnection) -> Result<()> {
    let mut connections = state
        .connections
        .lock()
        .unwrap_or_else(|poison| poison.into_inner());
    if state.closed.load(Ordering::Acquire) {
        connection.close(0u8.into(), b"probe closed");
        return Err(RemoteError::Closed);
    }
    connections.insert(connection.stable_id(), connection.clone());
    Ok(())
}
fn register(
    state: &Arc<State>,
    connection: ObservedConnection,
    permit: OwnedSemaphorePermit,
) -> Result<()> {
    insert(state, &connection)?;
    let owner = state.clone();
    let task = tokio::spawn(async move {
        serve_connection(owner, connection, permit).await;
    });
    track(state, task);
    Ok(())
}
fn track(state: &State, task: JoinHandle<()>) {
    let mut tasks = state
        .tasks
        .lock()
        .unwrap_or_else(|poison| poison.into_inner());
    tasks.retain(|task| !task.is_finished());
    if state.closed.load(Ordering::Acquire) {
        task.abort();
        return;
    }
    tasks.push(task);
}
async fn serve_connection(
    state: Arc<State>,
    connection: ObservedConnection,
    _permit: OwnedSemaphorePermit,
) {
    let mut workers = tokio::task::JoinSet::new();
    loop {
        while workers.try_join_next().is_some() {}
        let Ok((mut send, mut recv)) = connection.accept_bi().await else {
            break;
        };
        if workers.len() >= MAX_STREAMS {
            let _ = send.reset(2u8.into());
            let _ = recv.stop(2u8.into());
            continue;
        }
        workers.spawn(async move {
            let result = tokio::time::timeout(MAX_ROUND_TRIP_TIMEOUT, async {
                let frame = recv
                    .read_to_end(HEADER_BYTES + MAX_PAYLOAD_BYTES)
                    .await
                    .map_err(|_| RemoteError::Protocol)?;
                decode(&frame)?;
                send.write_all(&frame)
                    .await
                    .map_err(|_| RemoteError::Unavailable)?;
                send.finish().map_err(|_| RemoteError::Unavailable)
            })
            .await;
            if !matches!(result, Ok(Ok(()))) {
                let _ = send.reset(1u8.into());
                let _ = recv.stop(1u8.into());
            }
        });
    }
    state
        .connections
        .lock()
        .unwrap_or_else(|poison| poison.into_inner())
        .remove(&connection.stable_id());
}
