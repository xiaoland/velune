//! Device-to-device routing. Relays admit adjacent members and copy opaque
//! bytes; only the destination authenticates the original caller and owns its ledger.
use crate::{
    ClientSession, ContactConfirmation, DeviceContact, DeviceLocator, EndpointDescriptor,
    MAX_FRAME_BYTES, PROTOCOL_VERSION, RemoteClient, RemoteError, Result,
    endpoint::NetworkGeneration,
    paths::ConnectionSlot,
    protocol::{Envelope, Reply, Request},
    service::ServiceState,
    store::parse_peer,
    trust::TrustBundle,
};
use futures_util::FutureExt;
use iroh::endpoint::{RecvStream, SendStream};
use serde::{Deserialize, Serialize, de::DeserializeOwned};
use std::{
    collections::{BTreeSet, HashMap},
    future::Future,
    pin::Pin,
    sync::{
        Arc, Mutex, OnceLock, Weak,
        atomic::{AtomicBool, AtomicU64, Ordering},
    },
    time::{Duration, Instant},
};
use tokio::{
    io::{AsyncRead, AsyncReadExt, AsyncWrite, AsyncWriteExt},
    sync::Semaphore,
};

pub(crate) const ALPN: &[u8] = b"velune/device-mesh/1";
const MAX_HOPS: usize = 8;
const MAX_CONTROL_FRAME: usize = 1024 * 1024;
const MAX_CACHE: usize = 16;
const MAX_HINTS: usize = 256;
const MAX_STREAMS: usize = 32;
const SYNC_PARALLELISM: usize = 4;
const SYNC_INTERVAL: Duration = Duration::from_secs(3);
const DIAL_TIMEOUT: Duration = Duration::from_secs(3);
const DIRECT_PROBE_INTERVAL: Duration = Duration::from_secs(10);
type TunnelIo = tokio::io::Join<RecvStream, SendStream>;

#[derive(Serialize, Deserialize)]
#[serde(tag = "operation", rename_all = "camelCase", deny_unknown_fields)]
enum MeshRequest {
    Sync {
        bundle: TrustBundle,
        local_descriptor: Option<EndpointDescriptor>,
    },
    Tunnel {
        target: String,
        visited: Vec<String>,
    },
}

#[derive(Serialize, Deserialize)]
#[serde(
    tag = "reply",
    content = "value",
    rename_all = "camelCase",
    deny_unknown_fields
)]
enum MeshReply {
    Sync {
        bundle: TrustBundle,
        local_descriptor: Box<EndpointDescriptor>,
    },
    /// Full path, including origin and destination, verified against the adjacent
    /// QUIC peer at every honest forwarding hop. This is diagnostic, not authority.
    Ready {
        path: Vec<String>,
    },
    Error(RemoteError),
}

#[derive(Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct RoutedRequest {
    trust: TrustBundle,
    request: Request,
}

struct CachedConnection {
    slot: Arc<ConnectionSlot>,
    network: Weak<NetworkGeneration>,
    registration: u64,
    used: AtomicU64,
}
impl Drop for CachedConnection {
    fn drop(&mut self) {
        self.slot.close(b"mesh cache retired");
        if let Some(network) = self.network.upgrade() {
            network.remove(self.registration);
        }
    }
}

/// All native resources belong to the generation. A routed client can close its
/// own requests without closing the connection shared by other destinations.
pub(crate) struct MeshNetwork {
    cache: Mutex<HashMap<String, Arc<CachedConnection>>>,
    hints: Mutex<HashMap<String, (String, Instant)>>,
    clock: AtomicU64,
    incoming: Arc<Semaphore>,
    outgoing: Semaphore,
    dials: Semaphore,
    #[cfg(test)]
    forwarded_streams: AtomicU64,
}
impl Default for MeshNetwork {
    fn default() -> Self {
        Self {
            cache: Mutex::new(HashMap::new()),
            hints: Mutex::new(HashMap::new()),
            clock: AtomicU64::new(1),
            incoming: Arc::new(Semaphore::new(MAX_STREAMS)),
            outgoing: Semaphore::new(MAX_STREAMS),
            dials: Semaphore::new(SYNC_PARALLELISM),
            #[cfg(test)]
            forwarded_streams: AtomicU64::new(0),
        }
    }
}
impl MeshNetwork {
    fn cached(&self, peer: &str) -> Option<Arc<CachedConnection>> {
        let mut cache = self.cache.lock().ok()?;
        if cache
            .get(peer)
            .is_some_and(|entry| entry.slot.status().closed)
        {
            cache.remove(peer);
        }
        let entry = cache.get(peer)?.clone();
        entry.used.store(
            self.clock.fetch_add(1, Ordering::Relaxed),
            Ordering::Relaxed,
        );
        Some(entry)
    }
    fn remember(&self, target: &str, first: &str) {
        let Ok(mut hints) = self.hints.lock() else {
            return;
        };
        if let Some((peer, _)) = hints.get_mut(target) {
            first.clone_into(peer);
        } else if hints.len() < MAX_HINTS {
            hints.insert(target.into(), (first.into(), Instant::now()));
        }
    }
    fn forget_hint(&self, target: &str, first: &str) {
        if let Ok(mut hints) = self.hints.lock()
            && hints.get(target).is_some_and(|(peer, _)| peer == first)
        {
            hints.remove(target);
        }
    }
    #[cfg(test)]
    pub(crate) fn forwarded_streams_for_test(&self) -> u64 {
        self.forwarded_streams.load(Ordering::Acquire)
    }
}

pub(crate) struct RoutedConnection {
    target: String,
    first: Arc<CachedConnection>,
    path: Mutex<Vec<String>>,
    closed: AtomicBool,
    cancelled: tokio::sync::watch::Sender<bool>,
}
impl RoutedConnection {
    pub(crate) fn via_device_ids(&self) -> Vec<String> {
        self.path
            .lock()
            .map(|path| {
                path.iter()
                    .skip(1)
                    .take(path.len().saturating_sub(2))
                    .cloned()
                    .collect()
            })
            .unwrap_or_default()
    }
    pub(crate) fn is_closed(&self) -> bool {
        self.closed.load(Ordering::Acquire) || self.first.slot.status().closed
    }
    pub(crate) fn close(&self) {
        self.closed.store(true, Ordering::Release);
        self.cancelled.send_replace(true);
    }
    pub(crate) fn subscribe_closed(&self) -> tokio::sync::watch::Receiver<bool> {
        self.cancelled.subscribe()
    }
    pub(crate) async fn rpc(
        &self,
        network: &Arc<NetworkGeneration>,
        request: Request,
    ) -> Result<Reply> {
        let mut cancelled = self.cancelled.subscribe();
        if self.is_closed() {
            return Err(RemoteError::Closed);
        }
        let result = tokio::select! {
            biased;
            _ = network.cancelled() => Err(RemoteError::Closed),
            _ = cancelled.changed() => Err(RemoteError::Closed),
            result = tokio::time::timeout(network.timeout, async {
                let _operation = network.operation().await?;
                let _permit = network.mesh.outgoing.try_acquire().map_err(|_| RemoteError::Capacity)?;
                allowed_request(&request)?;
                let _ = member_contact(network, &self.target).await?;
                let origin = network.service.identity.id();
                let opened = open_via(network, self.first.clone(), &self.target, &[origin]).await?;
                let changed = {
                    let mut path = self.path.lock().map_err(|_| RemoteError::Unavailable)?;
                    let changed = *path != opened.path;
                    *path = opened.path.clone();
                    changed
                };
                if changed { network.service.store.touch(); }
                routed_rpc(network, &self.target, opened.io, request).await
            }) => result.unwrap_or(Err(RemoteError::Timeout)),
        };
        if result.is_err() {
            self.close();
        }
        result
    }
}

pub(crate) fn start(network: Arc<NetworkGeneration>) {
    let owner = network.clone();
    network.track(tokio::spawn(async move {
        let mut cursor = 0usize;
        loop {
            let work = async {
                let service = owner.service.clone();
                let Ok(contacts) = blocking(move || {
                    let mut members = Vec::new();
                    for contact in service.store.snapshot()?.contacts {
                        if usable_contact(&contact) && service.store.is_member(&contact.peer_id)? {
                            members.push(contact);
                        }
                    }
                    Ok(members)
                })
                .await
                else {
                    return;
                };
                if contacts.is_empty() {
                    return;
                }
                let mut pending = futures_util::stream::FuturesUnordered::new();
                for offset in 0..contacts.len().min(SYNC_PARALLELISM) {
                    let peer = contacts[(cursor + offset) % contacts.len()].peer_id.clone();
                    let network = owner.clone();
                    pending.push(async move {
                        let _ =
                            tokio::time::timeout(network.timeout.min(DIAL_TIMEOUT * 2), async {
                                let _operation = network.operation().await?;
                                let connection = connection(&network, &peer).await?;
                                sync_peer(&network, &connection).await
                            })
                            .await;
                    });
                }
                cursor = cursor.wrapping_add(SYNC_PARALLELISM);
                use futures_util::StreamExt;
                while pending.next().await.is_some() {}
            };
            tokio::select! {
                biased;
                _ = owner.cancelled() => return,
                _ = work => {},
            }
            tokio::select! {
                biased;
                _ = owner.cancelled() => return,
                _ = tokio::time::sleep(SYNC_INTERVAL) => {},
            }
        }
    }));
}

fn usable_contact(contact: &DeviceContact) -> bool {
    !contact.paused && matches!(contact.confirmation, ContactConfirmation::Confirmed { .. })
}
async fn member_contact(network: &Arc<NetworkGeneration>, peer: &str) -> Result<DeviceContact> {
    let service = network.service.clone();
    let peer = peer.to_owned();
    blocking(move || {
        parse_peer(&peer)?;
        let contact = service.store.contact(&peer)?;
        if !usable_contact(&contact) || !service.store.is_member(&peer)? {
            return Err(RemoteError::Unauthorized);
        }
        Ok(contact)
    })
    .await
}
async fn immediate_member(network: &Arc<NetworkGeneration>, peer: &str) -> Result<()> {
    let service = network.service.clone();
    let generation = network.id;
    let peer = peer.to_owned();
    blocking(move || {
        let access = service.access()?;
        ServiceState::check_generation(&access, generation)?;
        // Membership authorizes exchanging topology and forwarding bytes. A
        // device can participate with neither a business ingress nor a handler.
        if !service.store.is_member(&peer)? {
            return Err(RemoteError::Unauthorized);
        }
        Ok(())
    })
    .await
}

async fn connection(network: &Arc<NetworkGeneration>, peer: &str) -> Result<Arc<CachedConnection>> {
    member_contact(network, peer).await?;
    if let Some(connection) = network.mesh.cached(peer) {
        return Ok(connection);
    }
    let _permit = network
        .mesh
        .dials
        .acquire()
        .await
        .map_err(|_| RemoteError::Closed)?;
    if let Some(connection) = network.mesh.cached(peer) {
        return Ok(connection);
    }
    let contact = member_contact(network, peer).await?;
    let endpoint = network.prepare_peer(peer, &contact.locator)?;
    let native = tokio::time::timeout(
        network.timeout.min(DIAL_TIMEOUT),
        endpoint.connect(contact.locator.address(peer)?, ALPN),
    )
    .await
    .map_err(|_| RemoteError::Timeout)?
    .map_err(|_| RemoteError::Unavailable)?;
    let (registration, slot) = network.register(native, false)?;
    let entry = Arc::new(CachedConnection {
        slot: slot.clone(),
        network: Arc::downgrade(network),
        registration,
        used: AtomicU64::new(network.mesh.clock.fetch_add(1, Ordering::Relaxed)),
    });
    {
        let mut cache = network
            .mesh
            .cache
            .lock()
            .map_err(|_| RemoteError::Unavailable)?;
        if let Some(existing) = cache.get(peer).filter(|entry| !entry.slot.status().closed) {
            return Ok(existing.clone());
        }
        cache.retain(|_, entry| !entry.slot.status().closed);
        if cache.len() >= MAX_CACHE {
            let oldest = cache
                .iter()
                .filter(|(_, entry)| Arc::strong_count(entry) == 1)
                .min_by_key(|(_, entry)| entry.used.load(Ordering::Relaxed))
                .map(|(peer, _)| peer.clone())
                .ok_or(RemoteError::Capacity)?;
            cache.remove(&oldest);
        }
        cache.insert(peer.into(), entry.clone());
    }
    let owner = network.clone();
    network.track(tokio::spawn(async move {
        serve(owner, slot).await;
    }));
    Ok(entry)
}

async fn sync_peer(network: &Arc<NetworkGeneration>, connection: &CachedConnection) -> Result<()> {
    let peer = connection.slot.status().peer_id;
    member_contact(network, &peer).await?;
    let descriptor = network.descriptor()?;
    let service = network.service.clone();
    let advertised = descriptor.clone();
    let bundle = blocking(move || service.store.export_trust_bundle(Some(&advertised))).await?;
    let native = connection.slot.native().ok_or(RemoteError::Closed)?;
    let (send, recv) = native
        .open_bi()
        .await
        .map_err(|_| RemoteError::Unavailable)?;
    let mut io = tokio::io::join(recv, send);
    write_prefixed(
        &mut io,
        MeshRequest::Sync {
            bundle,
            local_descriptor: Some(descriptor),
        },
        MAX_CONTROL_FRAME,
    )
    .await?;
    let reply: MeshReply = read_prefixed(&mut io, MAX_CONTROL_FRAME).await?;
    match reply {
        MeshReply::Sync {
            bundle,
            local_descriptor,
        } => {
            local_descriptor.validate(&peer)?;
            let service = network.service.clone();
            blocking(move || {
                service.store.merge_trust_bundle(&peer, &bundle)?;
                service.store.update_locator(
                    &peer,
                    local_descriptor.name.clone(),
                    locator(&local_descriptor),
                )
            })
            .await
        }
        MeshReply::Error(error) => Err(error),
        _ => Err(RemoteError::Protocol),
    }
}

fn locator(descriptor: &EndpointDescriptor) -> DeviceLocator {
    DeviceLocator {
        relay_urls: descriptor.relay_urls.clone(),
        direct_addresses: descriptor.direct_addresses.clone(),
        lookup_urls: descriptor.lookup_urls.clone(),
    }
}

/// The owner already tried the business ALPN. Mesh reuses a successful path and
/// periodically probes its own direct target, then tries other member neighbors.
pub(crate) async fn connect(
    network: Arc<NetworkGeneration>,
    target: &str,
    session: ClientSession,
) -> Result<RemoteClient> {
    let _operation = network.operation().await?;
    member_contact(&network, target).await?;
    let origin = network.service.identity.id();
    if origin == target {
        return Err(RemoteError::Unauthorized);
    }
    tokio::select! {
        biased;
        _ = network.cancelled() => Err(RemoteError::Closed),
        result = tokio::time::timeout(network.timeout, async {
            let _permit = network.mesh.outgoing.try_acquire().map_err(|_| RemoteError::Capacity)?;
            let opened = open_route(&network, target, &[origin]).await?;
            let first = opened.first.clone();
            let path = opened.path.clone();
            // Hello uses the same opened tunnel. A relay never supplies C's descriptor.
            let descriptor = match routed_rpc(&network, target, opened.io, Request::Hello).await? {
                Reply::Hello(descriptor) => { descriptor.validate(target)?; descriptor },
                _ => return Err(RemoteError::Protocol),
            };
            let route = Arc::new(RoutedConnection {
                target: target.into(), first: first.clone(), path: Mutex::new(path),
                closed: AtomicBool::new(false), cancelled: tokio::sync::watch::channel(false).0,
            });
            let service = network.service.clone();
            let learned = descriptor.clone();
            let peer = target.to_owned();
            blocking(move || service.store.update_locator(&peer, learned.name.clone(), locator(&learned))).await?;
            Ok(RemoteClient {
                connection: first.slot.clone(), network: Arc::downgrade(&network),
                store: network.service.store.clone(), registration: 0, descriptor,
                session, authorized: true, timeout: network.timeout, route: Some(route),
            })
        }) => result.map_err(|_| RemoteError::Timeout)?,
    }
}

struct OpenedTunnel {
    io: TunnelIo,
    path: Vec<String>,
    // Pins a native connection while its stream is in use, even under cache pressure.
    first: Arc<CachedConnection>,
}

async fn candidates(
    network: &Arc<NetworkGeneration>,
    target: &str,
    visited: &[String],
) -> Result<Vec<String>> {
    let mut order = Vec::new();
    if let Some((hint, probe)) = network
        .mesh
        .hints
        .lock()
        .map_err(|_| RemoteError::Unavailable)?
        .get_mut(target)
    {
        if probe.elapsed() >= DIRECT_PROBE_INTERVAL {
            order.push(target.to_owned());
            *probe = Instant::now();
        }
        order.push(hint.clone());
    }
    order.push(target.to_owned());
    let service = network.service.clone();
    let target = target.to_owned();
    let (introducers, contacts) = blocking(move || {
        let mut contacts = Vec::new();
        for contact in service.store.snapshot()?.contacts {
            if usable_contact(&contact) && service.store.is_member(&contact.peer_id)? {
                contacts.push(contact);
            }
        }
        Ok((service.store.introducers_for(&target)?, contacts))
    })
    .await?;
    order.extend(introducers);
    {
        let cache = network
            .mesh
            .cache
            .lock()
            .map_err(|_| RemoteError::Unavailable)?;
        let mut cached: Vec<_> = cache.keys().cloned().collect();
        cached.sort();
        order.extend(cached);
    }
    order.extend(contacts.into_iter().map(|contact| contact.peer_id));
    let mut seen = BTreeSet::new();
    order.retain(|peer| !visited.contains(peer) && seen.insert(peer.clone()));
    Ok(order)
}

async fn open_route(
    network: &Arc<NetworkGeneration>,
    target: &str,
    visited: &[String],
) -> Result<OpenedTunnel> {
    if visited.len() > MAX_HOPS {
        return Err(RemoteError::Unavailable);
    }
    member_contact(network, target).await?;
    let mut failure = RemoteError::Unavailable;
    for peer in candidates(network, target, visited).await? {
        let attempt = async {
            let first = connection(network, &peer).await?;
            // Push proof before Tunnel: a new transitive neighbor may not yet know
            // our chain. Failure must not fall through to unauthenticated routing.
            sync_peer(network, &first).await?;
            open_via(network, first, target, visited).await
        };
        let budget = DIAL_TIMEOUT * (MAX_HOPS + 1 - visited.len()) as u32;
        match tokio::time::timeout(network.timeout.min(budget), attempt).await {
            Ok(Ok(opened)) => {
                network.mesh.remember(target, &peer);
                return Ok(opened);
            }
            Ok(Err(error)) => {
                failure = error;
            }
            Err(_) => {
                failure = RemoteError::Timeout;
            }
        }
        network.mesh.forget_hint(target, &peer);
    }
    Err(failure)
}

async fn open_via(
    network: &Arc<NetworkGeneration>,
    first: Arc<CachedConnection>,
    target: &str,
    visited: &[String],
) -> Result<OpenedTunnel> {
    let native = first.slot.native().ok_or(RemoteError::Closed)?;
    let peer = native.remote_id().to_string();
    member_contact(network, &peer).await?;
    let (send, recv) = native
        .open_bi()
        .await
        .map_err(|_| RemoteError::Unavailable)?;
    let mut io = tokio::io::join(recv, send);
    write_prefixed(
        &mut io,
        MeshRequest::Tunnel {
            target: target.into(),
            visited: visited.to_vec(),
        },
        MAX_CONTROL_FRAME,
    )
    .await?;
    match read_prefixed::<_, MeshReply>(&mut io, MAX_CONTROL_FRAME).await? {
        MeshReply::Ready { path } => {
            validate_ready(&path, visited, &peer, target)?;
            Ok(OpenedTunnel { io, path, first })
        }
        MeshReply::Error(error) => Err(error),
        _ => Err(RemoteError::Protocol),
    }
}

fn enter_path(
    local: &str,
    peer: &str,
    target: &str,
    mut visited: Vec<String>,
) -> Result<Vec<String>> {
    parse_peer(target)?;
    if visited.is_empty()
        || visited.len() > MAX_HOPS
        || visited.last().map(String::as_str) != Some(peer)
        || visited.iter().any(|id| id == local || id == target)
    {
        return Err(RemoteError::Protocol);
    }
    let mut seen = BTreeSet::new();
    for id in &visited {
        parse_peer(id)?;
        if !seen.insert(id) {
            return Err(RemoteError::Protocol);
        }
    }
    visited.push(local.into());
    Ok(visited)
}
fn validate_ready(path: &[String], visited: &[String], peer: &str, target: &str) -> Result<()> {
    if path.len() > MAX_HOPS + 1
        || path.len() <= visited.len()
        || !path.starts_with(visited)
        || path.get(visited.len()).map(String::as_str) != Some(peer)
        || path.last().map(String::as_str) != Some(target)
    {
        return Err(RemoteError::Protocol);
    }
    let mut seen = BTreeSet::new();
    for id in path {
        parse_peer(id)?;
        if !seen.insert(id) {
            return Err(RemoteError::Protocol);
        }
    }
    Ok(())
}

pub(crate) fn serve(
    network: Arc<NetworkGeneration>,
    slot: Arc<ConnectionSlot>,
) -> Pin<Box<dyn Future<Output = ()> + Send + 'static>> {
    Box::pin(async move { serve_connection(network, slot).await })
}

async fn serve_connection(network: Arc<NetworkGeneration>, slot: Arc<ConnectionSlot>) {
    let Some(native) = slot.native() else {
        return;
    };
    let peer = native.remote_id().to_string();
    let local_permits = Arc::new(Semaphore::new(8));
    loop {
        let streams = tokio::select! {
            biased;
            _ = network.cancelled() => break,
            result = tokio::time::timeout(Duration::from_secs(300), native.accept_bi()) => result,
        };
        let Ok(Ok((mut send, mut recv))) = streams else {
            break;
        };
        let permit = local_permits.clone().try_acquire_owned();
        let global = network.mesh.incoming.clone().try_acquire_owned();
        // Acquire before spawning, so a peer cannot create an unbounded waiter queue.
        if permit.is_err() || global.is_err() {
            let _ = send.reset(2u8.into());
            let _ = recv.stop(2u8.into());
            continue;
        }
        let global = global.unwrap();
        let owner = network.clone();
        let peer = peer.clone();
        let permit = permit.unwrap();
        network.track(tokio::spawn(async move {
            let _permit = permit;
            let _global = global;
            let mut io = tokio::io::join(recv, send);
            let _ = tokio::select! {
                biased;
                _ = owner.cancelled() => Err(RemoteError::Closed),
                result = tokio::time::timeout(owner.timeout, serve_stream(&owner, &peer, &mut io)) => result.unwrap_or(Err(RemoteError::Timeout)),
            };
        }));
    }
    slot.close(b"mesh idle or closed");
    if let Ok(mut cache) = network.mesh.cache.lock() {
        cache.retain(|_, entry| !Arc::ptr_eq(&entry.slot, &slot));
    }
}

async fn serve_stream(
    network: &Arc<NetworkGeneration>,
    peer: &str,
    io: &mut TunnelIo,
) -> Result<()> {
    let _operation = network.operation().await?;
    let request: MeshRequest = read_prefixed(io, MAX_CONTROL_FRAME).await?;
    let mut ready = false;
    let result = async {
        match request {
            MeshRequest::Sync {
                bundle,
                local_descriptor,
            } => {
                if let Some(descriptor) = &local_descriptor {
                    descriptor.validate(peer)?;
                }
                // A first transitive neighbor carries its signed proof on Sync;
                // merge authenticates the source using the current local roots.
                let service = network.service.clone();
                let generation = network.id;
                let peer = peer.to_string();
                blocking(move || {
                    let access = service.access()?;
                    ServiceState::check_generation(&access, generation)?;
                    service.store.merge_trust_bundle(&peer, &bundle)?;
                    if !service.store.is_member(&peer)? {
                        return Err(RemoteError::Unauthorized);
                    }
                    if let Some(descriptor) = local_descriptor
                        && service.store.is_pinned(&peer)?
                    {
                        service.store.update_locator(
                            &peer,
                            descriptor.name.clone(),
                            locator(&descriptor),
                        )?;
                    }
                    Ok::<_, RemoteError>(())
                })
                .await?;
                let local_descriptor = network.descriptor()?;
                let advertised = local_descriptor.clone();
                let service = network.service.clone();
                let bundle =
                    blocking(move || service.store.export_trust_bundle(Some(&advertised))).await?;
                write_prefixed(
                    io,
                    MeshReply::Sync {
                        bundle,
                        local_descriptor: Box::new(local_descriptor),
                    },
                    MAX_CONTROL_FRAME,
                )
                .await
            }
            MeshRequest::Tunnel { target, visited } => {
                immediate_member(network, peer).await?;
                let path = enter_path(&network.service.identity.id(), peer, &target, visited)?;
                if target == network.service.identity.id() {
                    ready = true;
                    write_prefixed(io, MeshReply::Ready { path }, MAX_CONTROL_FRAME).await?;
                    serve_target(network, io).await
                } else {
                    member_contact(network, &target).await?;
                    let mut downstream = open_route(network, &target, &path).await?;
                    ready = true;
                    write_prefixed(
                        io,
                        MeshReply::Ready {
                            path: downstream.path.clone(),
                        },
                        MAX_CONTROL_FRAME,
                    )
                    .await?;
                    // No business parsing, re-encoding, identity substitution, or retry here.
                    #[cfg(test)]
                    network
                        .mesh
                        .forwarded_streams
                        .fetch_add(1, Ordering::AcqRel);
                    tokio::io::copy_bidirectional(io, &mut downstream.io)
                        .await
                        .map_err(|_| RemoteError::Unavailable)?;
                    Ok(())
                }
            }
        }
    }
    .await;
    // Once Ready is written, remaining bytes belong exclusively to TLS. Errors
    // then reset/drop the stream; injecting a plaintext error would corrupt TLS.
    if let Err(error) = result {
        if !ready {
            let _ = write_prefixed(io, MeshReply::Error(error), MAX_CONTROL_FRAME).await;
        }
        return Err(error);
    }
    result
}

async fn serve_target(network: &Arc<NetworkGeneration>, io: &mut TunnelIo) -> Result<()> {
    let (mut tls, origin) = crate::tunnel_tls::accept(&network.service.identity.key, io).await?;
    let request: Envelope<RoutedRequest> = read_prefixed(&mut tls, MAX_FRAME_BYTES).await?;
    let reply = if request.version != PROTOCOL_VERSION {
        Reply::Error(RemoteError::IncompatibleVersion)
    } else if let Err(error) = allowed_request(&request.body.request) {
        Reply::Error(error)
    } else {
        let service = network.service.clone();
        let generation = network.id;
        let descriptor = network.descriptor()?;
        blocking(move || {
            let admitted = (|| {
                let access = service.access()?;
                ServiceState::check_generation(&access, generation)?;
                service
                    .store
                    .merge_trust_bundle(&origin, &request.body.trust)?;
                if !service.store.is_member(&origin)? {
                    return Err(RemoteError::Unauthorized);
                }
                Ok(())
            })();
            // Only the target service decides execution authority, ingress and
            // handler availability for the authenticated original caller.
            Ok(match admitted {
                Ok(()) => service.dispatch(generation, origin, descriptor, request.body.request),
                Err(error) => Reply::Error(error),
            })
        })
        .await?
    };
    write_prefixed(
        &mut tls,
        Envelope {
            version: PROTOCOL_VERSION,
            body: reply,
        },
        MAX_FRAME_BYTES,
    )
    .await?;
    tls.shutdown().await.map_err(|_| RemoteError::Unavailable)
}

fn allowed_request(request: &Request) -> Result<()> {
    match request {
        Request::Hello | Request::Submit { .. } | Request::Query { .. } => Ok(()),
        _ => Err(RemoteError::Unauthorized),
    }
}
async fn routed_rpc(
    network: &Arc<NetworkGeneration>,
    target: &str,
    io: TunnelIo,
    request: Request,
) -> Result<Reply> {
    allowed_request(&request)?;
    let mut tls = crate::tunnel_tls::connect(&network.service.identity.key, target, io).await?;
    let descriptor = network.descriptor()?;
    let service = network.service.clone();
    let trust = blocking(move || service.store.export_trust_bundle(Some(&descriptor))).await?;
    write_prefixed(
        &mut tls,
        Envelope {
            version: PROTOCOL_VERSION,
            body: RoutedRequest { trust, request },
        },
        MAX_FRAME_BYTES,
    )
    .await?;
    let reply: Envelope<Reply> = read_prefixed(&mut tls, MAX_FRAME_BYTES).await?;
    if reply.version != PROTOCOL_VERSION {
        return Err(RemoteError::IncompatibleVersion);
    }
    // The complete authenticated reply determines the RPC result. The target
    // may already have dropped its QUIC receive half, rejecting close_notify
    // with STOP_SENDING. Attempt cleanup without letting an error or backpressure
    // turn a known result into an uncertain request or delay its delivery.
    let _ = tls.shutdown().now_or_never();
    match reply.body {
        Reply::Error(error) => Err(error),
        reply => Ok(reply),
    }
}

/// A length prefix leaves the stream open for TLS and prevents reading a large
/// allocation before validating its declared size. Product v3 EOF frames stay unchanged.
async fn read_prefixed<R: AsyncRead + Unpin, T: DeserializeOwned + Send + 'static>(
    io: &mut R,
    limit: usize,
) -> Result<T> {
    let length = io.read_u32().await.map_err(|_| RemoteError::Unavailable)? as usize;
    if length > limit {
        return Err(RemoteError::FrameTooLarge);
    }
    if length == 0 {
        return Err(RemoteError::Protocol);
    }
    let mut bytes = vec![0; length];
    io.read_exact(&mut bytes)
        .await
        .map_err(|_| RemoteError::Unavailable)?;
    blocking(move || serde_json::from_slice(&bytes).map_err(|_| RemoteError::Protocol)).await
}
async fn write_prefixed<W: AsyncWrite + Unpin, T: Serialize + Send + 'static>(
    io: &mut W,
    value: T,
    limit: usize,
) -> Result<()> {
    let bytes = blocking(move || crate::wire::encode(&value, limit)).await?;
    io.write_u32(u32::try_from(bytes.len()).map_err(|_| RemoteError::FrameTooLarge)?)
        .await
        .map_err(|_| RemoteError::Unavailable)?;
    io.write_all(&bytes)
        .await
        .map_err(|_| RemoteError::Unavailable)?;
    io.flush().await.map_err(|_| RemoteError::Unavailable)
}

struct BlockingCancellation(Arc<AtomicBool>);
impl Drop for BlockingCancellation {
    fn drop(&mut self) {
        self.0.store(false, Ordering::Release);
    }
}

/// Keep the permit inside the closure: cancelling a stream cannot increase the
/// amount of outstanding disk/JSON work. The process-wide bound survives network
/// replacement, and queued work observes cancellation before touching the store.
async fn blocking<T: Send + 'static>(
    work: impl FnOnce() -> Result<T> + Send + 'static,
) -> Result<T> {
    static WORK: OnceLock<Arc<Semaphore>> = OnceLock::new();
    let permit = WORK
        .get_or_init(|| Arc::new(Semaphore::new(MAX_STREAMS)))
        .clone()
        .acquire_owned()
        .await
        .map_err(|_| RemoteError::Closed)?;
    let active = Arc::new(AtomicBool::new(true));
    let _cancel = BlockingCancellation(active.clone());
    tokio::task::spawn_blocking(move || {
        let _permit = permit;
        if !active.load(Ordering::Acquire) {
            return Err(RemoteError::Closed);
        }
        work()
    })
    .await
    .map_err(|_| RemoteError::Unavailable)?
}

#[cfg(test)]
mod tests;
