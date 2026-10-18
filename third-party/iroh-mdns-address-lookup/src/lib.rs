//! An address lookup service that uses an mdns-like service to discover and lookup the addresses of local endpoints.
//!
//! This allows you to use an mdns-like swarm discovery service to find address information about endpoints that are on your local network, no relay or outside internet needed.
//! See the [`swarm-discovery`](https://crates.io/crates/swarm-discovery) crate for more details.
//!
//! When [`MdnsAddressLookup`] is enabled, it's possible to get a list of the locally discovered endpoints by filtering a list of `RemoteInfo`s.
//!
//! In order to get a list of locally discovered addresses, you must call `MdnsAddressLookup::subscribe` to subscribe
//! to a stream of discovered addresses.
//!
//! ```no_run
//! use iroh::{Endpoint, endpoint::presets};
//! use iroh_mdns_address_lookup::{DiscoveryEvent, MdnsAddressLookup};
//! use n0_future::StreamExt;
//!
//! #[tokio::main]
//! async fn main() {
//!     let endpoint = Endpoint::bind(presets::Minimal).await.unwrap();
//!
//!     // Register the Address Lookupwith the endpoint
//!     let mdns = MdnsAddressLookup::builder().build(endpoint.id()).unwrap();
//!     endpoint.address_lookup().unwrap().add(mdns.clone());
//!
//!     // Subscribe to the mdns discovery events
//!     let mut events = mdns.subscribe().await;
//!     while let Some(event) = events.next().await {
//!         match event {
//!             DiscoveryEvent::Discovered { endpoint_info, .. } => {
//!                 println!("MDNS discovered: {:?}", endpoint_info);
//!             }
//!             DiscoveryEvent::Expired { endpoint_id } => {
//!                 println!("MDNS expired: {endpoint_id}");
//!             }
//!             _ => {}
//!         }
//!     }
//! }
//! ```
//!
//! ## Filtering
//!
//! By default, [`MdnsAddressLookup`] publishes all addresses it receives:
//! direct IP addresses and up to one [`RelayUrl`]. The following constraints apply regardless
//! of any user-supplied filter:
//!
//! - Only the first [`RelayUrl`] in the address set is published.
//! - A [`RelayUrl`] longer than 249 bytes is silently dropped.
//!
//! You can supply an [`AddrFilter`] via [`MdnsAddressLookupBuilder::addr_filter`] to
//! control which addresses are published and in what order. The filter is applied before the
//! constraints above, so for example you can use it to exclude relay URLs entirely or to
//! prioritize certain addresses.
//!
//! [`AddrFilter`]: iroh::address_lookup::AddrFilter
//! [`RelayUrl`]: iroh_base::RelayUrl
use std::{
    collections::{BTreeSet, HashMap},
    net::{IpAddr, Ipv4Addr, SocketAddr},
    str::FromStr,
    sync::Arc,
};

use iroh::{
    Endpoint,
    address_lookup::{
        AddrFilter, AddressLookup, AddressLookupBuilder, AddressLookupBuilderError, EndpointData,
        EndpointInfo, Error as AddressLookupError, Item as AddressLookupItem,
    },
};
use iroh_base::{EndpointId, PublicKey};
use n0_future::{
    Stream,
    boxed::BoxStream,
    task::{self, AbortOnDropHandle, JoinSet},
    time::{self, Duration},
};
use n0_watcher::{Watchable, Watcher as _};
use swarm_discovery::{Discoverer, DropGuard, IpClass, Peer};
use tokio::sync::{
    mpsc::{self, error::TrySendError},
    oneshot,
};
use tracing::{Instrument, debug, error, info_span, trace, warn};

/// The n0 local service name.
const N0_SERVICE_NAME: &str = "irohv1";

/// Name of this address lookup service.
///
/// Used as the `provenance` field in [`AddressLookupItem`]s.
pub const NAME: &str = "mdns";

/// The key of the attribute under which the `UserData` is stored in
/// the TXT record supported by swarm-discovery.
const USER_DATA_ATTRIBUTE: &str = "user-data";

/// Minimum time to wait while resolving an endpoint ID to an address.
const MIN_LOOKUP_DURATION: Duration = Duration::from_secs(10);

/// The key of the attribute under which the `RelayUrl` is stored in
/// the TXT record supported by swarm-discovery.
const RELAY_URL_ATTRIBUTE: &str = "relay";

/// Address Lookup using `swarm-discovery`, a variation on mdns.
#[derive(Debug, Clone)]
pub struct MdnsAddressLookup {
    #[allow(dead_code)]
    handle: Arc<AbortOnDropHandle<()>>,
    sender: mpsc::Sender<Message>,
    advertise: bool,
    /// When `local_addrs` changes, we re-publish our info.
    local_addrs: Watchable<Option<EndpointData>>,
}

#[derive(Debug)]
enum Message {
    Discovered {
        generation: u64,
        endpoint_id: String,
        peer: Peer,
    },
    SetIpv4Interfaces {
        interfaces: BTreeSet<Ipv4Addr>,
        applied: oneshot::Sender<Result<u64, AddressLookupBuilderError>>,
    },
    Resolve(
        EndpointId,
        mpsc::Sender<Result<AddressLookupItem, AddressLookupError>>,
    ),
    Timeout(EndpointId, usize),
    Subscribe(mpsc::Sender<DiscoveryEvent>),
}

/// Manages the list of subscribers that are subscribed to this Address Lookup.
#[derive(Debug)]
struct Subscribers(Vec<mpsc::Sender<DiscoveryEvent>>);

impl Subscribers {
    fn new() -> Self {
        Self(vec![])
    }

    /// Add the subscriber to the list of subscribers.
    fn push(&mut self, subscriber: mpsc::Sender<DiscoveryEvent>) {
        self.0.push(subscriber);
    }

    /// Sends the `endpoint_id` and `item` to each subscriber.
    ///
    /// Cleans up any subscribers that have been dropped.
    fn send(&mut self, item: DiscoveryEvent) {
        let mut clean_up = vec![];
        for (i, subscriber) in self.0.iter().enumerate() {
            // assume subscriber was dropped
            if let Err(err) = subscriber.try_send(item.clone()) {
                match err {
                    TrySendError::Full(_) => {
                        warn!(?item, idx = i, "mdns subscriber is blocked, dropping item")
                    }
                    TrySendError::Closed(_) => clean_up.push(i),
                }
            }
        }
        for i in clean_up.into_iter().rev() {
            self.0.swap_remove(i);
        }
    }
}

/// Builder for [`MdnsAddressLookup`].
#[derive(Debug)]
pub struct MdnsAddressLookupBuilder {
    advertise: bool,
    service_name: String,
    filter: AddrFilter,
    cadence: Duration,
    ipv4_interfaces: Option<BTreeSet<Ipv4Addr>>,
}

impl MdnsAddressLookupBuilder {
    /// Creates a new [`MdnsAddressLookupBuilder`] with default settings.
    fn new() -> Self {
        Self {
            advertise: true,
            service_name: N0_SERVICE_NAME.to_string(),
            filter: AddrFilter::default(),
            cadence: Duration::from_millis(700),
            ipv4_interfaces: None,
        }
    }

    /// Sets whether this endpoint should advertise its presence.
    ///
    /// Default is true.
    pub fn advertise(mut self, advertise: bool) -> Self {
        self.advertise = advertise;
        self
    }

    /// Configure explicit IPv4 multicast interfaces with IPv4-only discovery.
    /// An empty set starts the actor without a discoverer or multicast sockets.
    /// Without this option the upstream automatic interface policy is retained.
    pub fn ipv4_interfaces(mut self, interfaces: BTreeSet<Ipv4Addr>) -> Self {
        self.ipv4_interfaces = Some(interfaces);
        self
    }

    /// Sets a custom service name.
    ///
    /// The default is `irohv1`, which will show up on a record in the
    /// following form, for example:
    /// `7rutqynuzu65fcdgoerbt4uoh3p62wuto2mp56x3uvhitqzssxga._irohv1._udp.local`
    ///
    /// Any custom service name will take the form, for example:
    /// `7rutqynuzu65fcdgoerbt4uoh3p62wuto2mp56x3uvhitqzssxga._{service_name}._udp.local`
    pub fn service_name(mut self, service_name: impl Into<String>) -> Self {
        self.service_name = service_name.into();
        self
    }

    /// Sets the base interval between mDNS discovery query cycles.
    ///
    /// Queries ask peers on the local network to announce their addresses.
    /// Before sending a query, `swarm-discovery` waits for this interval plus
    /// a random delay that grows with the number of currently known peers.
    /// If another peer sends a query during that wait, it uses that query
    /// instead of sending its own, reducing duplicate traffic.
    ///
    /// The default is 700 ms. Longer intervals reduce background traffic and
    /// also lengthen peer-expiry detection. Must be nonzero.
    /// The address lookup deadline grows with the cadence, so longer intervals
    /// also make unsuccessful lookups take longer.
    ///
    /// # Panics
    /// Panics if the cadence is zero.
    pub fn discovery_cadence(mut self, cadence: Duration) -> Self {
        assert!(!cadence.is_zero());
        self.cadence = cadence;
        self
    }

    /// Sets a filter to control which addresses are published by this service.
    pub fn addr_filter(mut self, filter: AddrFilter) -> Self {
        self.filter = filter;
        self
    }

    /// Builds an [`MdnsAddressLookup`] instance with the configured settings.
    ///
    /// # Errors
    /// Returns an error if the network does not allow ipv4 OR ipv6.
    ///
    /// # Panics
    /// This relies on [`tokio::runtime::Handle::current`] and will panic if called outside of the context of a tokio runtime.
    pub fn build(
        self,
        endpoint_id: EndpointId,
    ) -> Result<MdnsAddressLookup, AddressLookupBuilderError> {
        MdnsAddressLookup::new(
            endpoint_id,
            self.advertise,
            self.service_name,
            self.filter,
            self.cadence,
            self.ipv4_interfaces,
        )
    }
}

impl Default for MdnsAddressLookupBuilder {
    fn default() -> Self {
        Self::new()
    }
}

impl AddressLookupBuilder for MdnsAddressLookupBuilder {
    fn into_address_lookup(
        self,
        endpoint: &Endpoint,
    ) -> Result<impl AddressLookup, AddressLookupBuilderError> {
        self.build(endpoint.id())
    }
}

/// An event emitted from the [`MdnsAddressLookup`] service.
#[derive(Debug, Clone, Eq, PartialEq)]
#[non_exhaustive]
pub enum DiscoveryEvent {
    /// A peer was discovered or it's information was updated.
    Discovered {
        /// The endpoint info for the endpoint, as discovered.
        endpoint_info: EndpointInfo,
        /// Optional timestamp when this endpoint address info was last updated.
        last_updated: Option<u64>,
    },
    /// A peer was expired due to being inactive, unreachable, or otherwise
    /// unavailable. This also includes invalidation of the service's previous
    /// interface scope; it does not establish that the remote device went offline.
    Expired {
        /// The id of the endpoint that expired.
        endpoint_id: EndpointId,
    },
}

struct DiscovererConfig {
    endpoint_id: EndpointId,
    advertise: bool,
    service_name: String,
    cadence: Duration,
}

impl MdnsAddressLookup {
    /// Returns a [`MdnsAddressLookupBuilder`] used to construct [`MdnsAddressLookup`].
    pub fn builder() -> MdnsAddressLookupBuilder {
        MdnsAddressLookupBuilder::default()
    }

    /// Create a new [`MdnsAddressLookup`] Service.
    ///
    /// This starts a [`Discoverer`] that broadcasts your addresses (if advertise is set to true)
    /// and receives addresses from other endpoints in your local network.
    ///
    /// # Errors
    /// Returns an error if the network does not allow ipv4 OR ipv6.
    ///
    /// # Panics
    /// This relies on [`tokio::runtime::Handle::current`] and will panic if called outside of the context of a tokio runtime.
    fn new(
        endpoint_id: EndpointId,
        advertise: bool,
        service_name: String,
        filter: AddrFilter,
        cadence: Duration,
        mut ipv4_interfaces: Option<BTreeSet<Ipv4Addr>>,
    ) -> Result<Self, AddressLookupBuilderError> {
        debug!("Creating new Mdns service");
        let lookup_duration = lookup_duration(cadence);
        let (send, mut recv) = mpsc::channel(64);
        let task_sender = send.clone();
        let rt = tokio::runtime::Handle::current();
        let config = DiscovererConfig {
            endpoint_id,
            advertise,
            service_name,
            cadence,
        };
        let mut generation = 0;
        let mut address_lookup = Self::spawn_discoverer(
            &config,
            task_sender.clone(),
            generation,
            ipv4_interfaces.as_ref(),
            None,
            &rt,
        )?;

        let local_addrs: Watchable<Option<EndpointData>> = Watchable::default();
        let mut addrs_change = local_addrs.watch();
        let address_lookup_fut = async move {
            let mut endpoint_addrs: HashMap<PublicKey, AddressLookupItem> = HashMap::default();
            let mut published_data: Option<EndpointData> = None;
            let mut subscribers = Subscribers::new();
            let mut last_id = 0;
            let mut senders: HashMap<
                PublicKey,
                HashMap<usize, mpsc::Sender<Result<AddressLookupItem, AddressLookupError>>>,
            > = HashMap::default();
            let mut timeouts = JoinSet::new();
            loop {
                trace!(?endpoint_addrs, "Mdns Service loop tick");
                let msg = tokio::select! {
                    msg = recv.recv() => {
                        msg
                    }
                    Ok(Some(data)) = addrs_change.updated() => {
                        tracing::trace!(?data, "Mdns address changed");
                        let data = data.apply_filter(&filter).into_owned();
                        if let Some(discoverer) = &address_lookup {
                            Self::publish_data(discoverer, &data);
                        }
                        published_data = Some(data);
                        continue;
                    }
                };
                let msg = match msg {
                    None => {
                        error!("Mdns channel closed");
                        error!("closing Mdns");
                        timeouts.abort_all();
                        drop(address_lookup.take());
                        return;
                    }
                    Some(msg) => msg,
                };
                match msg {
                    Message::SetIpv4Interfaces {
                        interfaces,
                        applied,
                    } => {
                        let same = ipv4_interfaces.as_ref() == Some(&interfaces);
                        if same && (interfaces.is_empty() || address_lookup.is_some()) {
                            let _ = applied.send(Ok(generation));
                            continue;
                        }
                        let Some(next_generation) = generation.checked_add(1) else {
                            let _ =
                                applied.send(Err(Self::control_error("mDNS generation exhausted")));
                            continue;
                        };
                        // Drop aborts the old guardian. Its callbacks remain queued but are fenced below.
                        drop(address_lookup.take());
                        generation = next_generation;
                        ipv4_interfaces = Some(interfaces);
                        // Records from the old interface scope have no remaining expiry source.
                        for (endpoint_id, _) in endpoint_addrs.drain() {
                            subscribers.send(DiscoveryEvent::Expired { endpoint_id });
                        }
                        let result = Self::spawn_discoverer(
                            &config,
                            task_sender.clone(),
                            generation,
                            ipv4_interfaces.as_ref(),
                            published_data.as_ref(),
                            &rt,
                        )
                        .map(|discoverer| {
                            address_lookup = discoverer;
                            generation
                        });
                        let _ = applied.send(result);
                    }
                    Message::Discovered {
                        generation: observed_generation,
                        endpoint_id: discovered_endpoint_id,
                        peer: peer_info,
                    } => {
                        if observed_generation != generation {
                            continue;
                        }
                        trace!(
                            ?discovered_endpoint_id,
                            ?peer_info,
                            "Mdns Message::Discovered"
                        );
                        let discovered_endpoint_id =
                            match PublicKey::from_str(&discovered_endpoint_id) {
                                Ok(endpoint_id) => endpoint_id,
                                Err(e) => {
                                    warn!(
                                        discovered_endpoint_id,
                                        "couldn't parse endpoint_id from mdns Address Lookup: {e:?}"
                                    );
                                    continue;
                                }
                            };

                        if discovered_endpoint_id == endpoint_id {
                            continue;
                        }

                        if peer_info.is_expiry() {
                            trace!(
                                ?discovered_endpoint_id,
                                "removing endpoint from Mdns address book"
                            );
                            endpoint_addrs.remove(&discovered_endpoint_id);
                            subscribers.send(DiscoveryEvent::Expired {
                                endpoint_id: discovered_endpoint_id,
                            });
                            continue;
                        }

                        let item = peer_to_discovery_item(&peer_info, &discovered_endpoint_id);
                        if !cache_discovery(&mut endpoint_addrs, item.clone()) {
                            continue;
                        }
                        debug!(
                            ?discovered_endpoint_id,
                            ?peer_info,
                            "adding endpoint to Mdns address book"
                        );
                        let mut resolved = false;
                        if let Some(senders) = senders.get(&discovered_endpoint_id) {
                            trace!(?item, senders = senders.len(), "sending AddressLookupItem");
                            resolved = true;
                            for sender in senders.values() {
                                sender.send(Ok(item.clone())).await.ok();
                            }
                        }

                        // only send endpoints to the `subscriber` if they weren't explicitly resolved
                        // in other words, endpoints sent to the `subscribers` should only be the ones that
                        // have been "passively" discovered
                        if !resolved {
                            subscribers.send(DiscoveryEvent::Discovered {
                                endpoint_info: item.endpoint_info().clone(),
                                last_updated: item.last_updated(),
                            });
                        }
                    }
                    Message::Resolve(endpoint_id, sender) => {
                        let id = last_id + 1;
                        last_id = id;
                        trace!(?endpoint_id, "Mdns Message::SendAddrs");
                        if let Some(item) = endpoint_addrs.get(&endpoint_id) {
                            debug!(?item, "sending AddressLookupItem");
                            sender.send(Ok(item.clone())).await.ok();
                        }
                        if let Some(senders_for_endpoint_id) = senders.get_mut(&endpoint_id) {
                            senders_for_endpoint_id.insert(id, sender);
                        } else {
                            let mut senders_for_endpoint_id = HashMap::new();
                            senders_for_endpoint_id.insert(id, sender);
                            senders.insert(endpoint_id, senders_for_endpoint_id);
                        }
                        let timeout_sender = task_sender.clone();
                        timeouts.spawn(async move {
                            time::sleep(lookup_duration).await;
                            trace!(?endpoint_id, "resolution timeout");
                            timeout_sender
                                .send(Message::Timeout(endpoint_id, id))
                                .await
                                .ok();
                        });
                    }
                    Message::Timeout(endpoint_id, id) => {
                        trace!(?endpoint_id, "Mdns Message::Timeout");
                        if let Some(senders_for_endpoint_id) = senders.get_mut(&endpoint_id) {
                            senders_for_endpoint_id.remove(&id);
                            if senders_for_endpoint_id.is_empty() {
                                senders.remove(&endpoint_id);
                            }
                        }
                    }
                    Message::Subscribe(subscriber) => {
                        trace!("Mdns Message::Subscribe");
                        subscribers.push(subscriber);
                    }
                }
            }
        };
        let handle =
            task::spawn(address_lookup_fut.instrument(info_span!("swarm-discovery.actor")));
        Ok(Self {
            handle: Arc::new(AbortOnDropHandle::new(handle)),
            sender: send,
            advertise,
            local_addrs,
        })
    }

    /// Subscribe to discovered endpoints.
    pub async fn subscribe(&self) -> impl Stream<Item = DiscoveryEvent> + Unpin + use<> {
        let (sender, recv) = mpsc::channel(20);
        let address_lookup_sender = self.sender.clone();
        address_lookup_sender
            .send(Message::Subscribe(sender))
            .await
            .ok();
        tokio_stream::wrappers::ReceiverStream::new(recv)
    }

    /// Apply a new exact IPv4 multicast scope without replacing this lookup actor or subscriptions.
    /// The initial generation is zero. A successfully applied identical set is a no-op.
    /// Empty pauses discovery; changes invalidate old cached records and emit `Expired`.
    /// Pending resolve windows keep their original deadlines and can receive fresh records.
    ///
    /// Reconfiguration drops the entire old discoverer guard and rejects its queued callbacks.
    /// This requests asynchronous cancellation, not a join or proof that its sockets have closed.
    /// If spawn fails, the old scope stays stopped with no fallback; the same set can be retried.
    pub async fn set_ipv4_interfaces(
        &self,
        interfaces: BTreeSet<Ipv4Addr>,
    ) -> Result<u64, AddressLookupBuilderError> {
        let (applied, result) = oneshot::channel();
        self.sender
            .send(Message::SetIpv4Interfaces {
                interfaces,
                applied,
            })
            .await
            .map_err(|_| Self::control_error("mDNS actor stopped"))?;
        result
            .await
            .map_err(|_| Self::control_error("mDNS actor stopped"))?
    }

    fn control_error(message: &'static str) -> AddressLookupBuilderError {
        AddressLookupBuilderError::from_err("mdns", std::io::Error::other(message))
    }

    fn spawn_discoverer(
        config: &DiscovererConfig,
        sender: mpsc::Sender<Message>,
        generation: u64,
        interfaces: Option<&BTreeSet<Ipv4Addr>>,
        data: Option<&EndpointData>,
        rt: &tokio::runtime::Handle,
    ) -> Result<Option<DropGuard>, AddressLookupBuilderError> {
        if interfaces.is_some_and(BTreeSet::is_empty) {
            return Ok(None);
        }
        let spawn_rt = rt.clone();
        let callback = move |endpoint_id: &str, peer: &Peer| {
            let sender = sender.clone();
            let endpoint_id = endpoint_id.to_string();
            let peer = peer.clone();
            spawn_rt.spawn(async move {
                sender
                    .send(Message::Discovered {
                        generation,
                        endpoint_id,
                        peer,
                    })
                    .await
                    .ok();
            });
        };
        let endpoint_id_str = data_encoding::BASE32_NOPAD
            .encode(config.endpoint_id.as_bytes())
            .to_ascii_lowercase();
        let mut discoverer =
            Discoverer::new_interactive(config.service_name.clone(), endpoint_id_str)
                .with_cadence(config.cadence)
                .with_callback(callback);
        discoverer = match interfaces {
            Some(interfaces) => discoverer
                .with_ip_class(IpClass::V4Only)
                .with_multicast_interfaces_v4(interfaces.iter().copied().collect()),
            None => discoverer.with_ip_class(IpClass::Auto),
        };
        let discoverer = discoverer
            .spawn(rt)
            .map_err(|error| AddressLookupBuilderError::from_err("mdns", error))?;
        if config.advertise
            && let Some(data) = data
        {
            Self::publish_data(&discoverer, data);
        }
        Ok(Some(discoverer))
    }

    fn publish_data(discoverer: &DropGuard, data: &EndpointData) {
        discoverer.remove_all();
        for (port, addresses) in Self::socketaddrs_to_addrs(data.ip_addrs()) {
            discoverer.add(port, addresses);
        }
        let relay = data.relay_urls().next().map(ToString::to_string);
        if let Err(error) = discoverer.set_txt_attribute(RELAY_URL_ATTRIBUTE.to_string(), relay) {
            warn!("Failed to set the relay url in mDNS: {error:?}");
        }
        let user_data = data.user_data().map(ToString::to_string);
        if let Err(error) = discoverer.set_txt_attribute(USER_DATA_ATTRIBUTE.to_string(), user_data)
        {
            warn!("Failed to set the user-defined data in mDNS: {error:?}");
        }
    }

    fn socketaddrs_to_addrs<'a>(
        socketaddrs: impl Iterator<Item = &'a SocketAddr>,
    ) -> HashMap<u16, Vec<IpAddr>> {
        let mut addrs: HashMap<u16, Vec<IpAddr>> = HashMap::default();
        for socketaddr in socketaddrs {
            addrs
                .entry(socketaddr.port())
                .and_modify(|a| a.push(socketaddr.ip()))
                .or_insert(vec![socketaddr.ip()]);
        }
        addrs
    }
}

/// Computes the lookup timeout from the query cadence.
///
/// Allows for swarm-discovery's worst-case query delay of 1.2 times the cadence,
/// plus five seconds for the response, with a minimum of [`MIN_LOOKUP_DURATION`].
fn lookup_duration(cadence: Duration) -> Duration {
    MIN_LOOKUP_DURATION.max(
        cadence
            .saturating_add(cadence / 5)
            .saturating_add(Duration::from_secs(5)),
    )
}

/// Caches the latest discovery item for an endpoint.
///
/// Returns `true` if the endpoint is new or its advertised content changed.
/// Observation timestamps do not affect the comparison. The cached item is
/// always replaced so later lookups use the latest addresses.
fn cache_discovery(
    cache: &mut HashMap<PublicKey, AddressLookupItem>,
    item: AddressLookupItem,
) -> bool {
    let id = item.endpoint_id();
    let changed = cache
        .get(&id)
        .is_none_or(|old| old.endpoint_info() != item.endpoint_info());
    cache.insert(id, item);
    changed
}

fn peer_to_discovery_item(peer: &Peer, endpoint_id: &EndpointId) -> AddressLookupItem {
    let ip_addrs: BTreeSet<SocketAddr> = peer
        .addrs()
        .iter()
        .map(|(ip, port)| SocketAddr::new(*ip, *port))
        .collect();

    // Get the relay url from the resolved peer info. We expect an attribute that parses as
    // a `RelayUrl`. Otherwise, omit.
    let relay_url = if let Some(Some(relay_url)) = peer.txt_attribute(RELAY_URL_ATTRIBUTE) {
        match relay_url.parse() {
            Err(err) => {
                debug!("failed to parse relay url from TXT attribute: {err}");
                None
            }
            Ok(url) => Some(url),
        }
    } else {
        None
    };

    // Get the user-defined data from the resolved peer info. We expect an attribute with a value
    // that parses as `UserData`. Otherwise, omit.
    let user_data = if let Some(Some(user_data)) = peer.txt_attribute(USER_DATA_ATTRIBUTE) {
        match user_data.parse() {
            Err(err) => {
                debug!("failed to parse user data from TXT attribute: {err}");
                None
            }
            Ok(data) => Some(data),
        }
    } else {
        None
    };

    let mut data = EndpointData::from(ip_addrs);
    if let Some(relay_url) = relay_url {
        data.add_relay_url(relay_url);
    }
    data.set_user_data(user_data);

    let endpoint_info = EndpointInfo::from_parts(*endpoint_id, data);
    AddressLookupItem::new(endpoint_info, NAME, None)
}

impl AddressLookup for MdnsAddressLookup {
    fn resolve(
        &self,
        endpoint_id: EndpointId,
    ) -> Option<BoxStream<Result<AddressLookupItem, AddressLookupError>>> {
        use futures_util::FutureExt;

        let (send, recv) = mpsc::channel(20);
        let address_lookup_sender = self.sender.clone();
        let stream = async move {
            address_lookup_sender
                .send(Message::Resolve(endpoint_id, send))
                .await
                .ok();
            tokio_stream::wrappers::ReceiverStream::new(recv)
        };
        Some(Box::pin(stream.flatten_stream()))
    }

    fn publish(&self, data: &EndpointData) {
        if self.advertise {
            self.local_addrs.set(Some(data.clone())).ok();
        }
    }
}
