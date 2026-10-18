//! The [`Endpoint`] allows establishing connections to other iroh endpoints.
//!
//! The [`Endpoint`] is the main API interface to manage a local iroh endpoint.  It allows
//! connecting to and accepting connections from other endpoints.  See the [module docs] for
//! more details on how iroh connections work.
//!
//! The main items in this module are:
//!
//! - [`Endpoint`] to establish iroh connections with other endpoints.
//! - [`Builder`] to create an [`Endpoint`].
//!
//! [module docs]: crate

use std::{
    collections::{BTreeMap, BTreeSet},
    fmt,
    net::SocketAddr,
    pin::Pin,
    sync::Arc,
};

#[cfg(not(wasm_browser))]
use ipnet::{Ipv4Net, Ipv6Net};
use iroh_base::{EndpointAddr, EndpointId, RelayUrl, SecretKey, TransportAddr};
use iroh_relay::{RelayConfig, RelayMap, tls::CaTlsConfig};
#[cfg(not(wasm_browser))]
use n0_error::bail;
use n0_error::{AnyError, e, ensure, stack_error};
use n0_watcher::Watcher;
use pin_project::pin_project;
use tokio_util::sync::WaitForCancellationFutureOwned;
use tracing::{Instrument, Span, debug, event, info_span, instrument, warn};
use url::Url;

#[cfg(feature = "unstable-custom-transports")]
pub mod transports {
    //! Types for defining custom transports and path selectors.
    //!
    //! <div class="warning">
    //!
    //! These items are unstable and gated behind the `unstable-custom-transport` feature.
    //! They are not covered by semantic versioning guarantees and may change in any release
    //! without a major version bump.
    //!
    //! </div>

    pub use super::socket::{
        remote_map::{PathSelection, PathSelectionContext, PathSelectionData, PathSelector},
        transports::{
            Addr, AddrKind, FourTuple, RecvInfo, Transmit,
            custom::{CustomEndpoint, CustomSender, CustomTransport},
        },
    };
}

use self::hooks::EndpointHooksList;
pub use super::socket::{
    BindError, DirectAddr, DirectAddrType,
    remote_map::{
        Path, PathEvent, PathEventStream, PathList, PathListIter, PathListStream, RemoteInfo,
        TransportAddrInfo, TransportAddrUsage,
    },
    transports::LocalTransportAddr,
};
#[cfg(wasm_browser)]
use crate::address_lookup::PkarrResolver;
#[cfg(not(wasm_browser))]
use crate::dns::DnsResolver;
#[cfg(feature = "unstable-custom-transports")]
use crate::endpoint::transports::CustomTransport;
#[cfg(feature = "unstable-net-report")]
use crate::net_report::Report as NetReport;
pub use crate::tls::TlsConfigError;
use crate::{
    address_lookup::{
        AddrFilter, AddressLookupBuilder, AddressLookupFailed, AddressLookupServices,
        DynAddressLookupBuilder, UserData,
    },
    endpoint::presets::Preset,
    metrics::EndpointMetrics,
    socket::{
        self, EndpointInner, RemoteStateActorStoppedError, StaticConfig,
        biased_rtt_path_selector::BiasedRttPathSelector,
        mapped_addrs::MappedAddr,
        remote_map::PathSelector,
        transports::{RelayConnectionFailure, RelayConnectionState},
    },
    tls::{self, DEFAULT_MAX_TLS_TICKETS, misc::RustlsTokenKey},
};

#[cfg(not(wasm_browser))]
mod bind;
mod connection;
pub(crate) mod hooks;
pub mod presets;
pub(crate) mod quic;

#[cfg(not(wasm_browser))]
pub use bind::{BindOpts, InvalidSocketAddr, ToSocketAddr};
pub use hooks::{AfterHandshakeOutcome, BeforeConnectOutcome, EndpointHooks};

#[cfg(feature = "qlog")]
pub use self::quic::{QlogConfig, QlogFactory, QlogFileFactory};
pub use self::{
    connection::{
        Accept, Accepting, AlpnError, AuthenticationError, Connecting, ConnectingError, Connection,
        ConnectionState, HandshakeCompleted, Incoming, IncomingAddr, IncomingZeroRtt,
        IncomingZeroRttConnection, OutgoingZeroRtt, OutgoingZeroRttConnection,
        RemoteEndpointIdError, RetryError, WeakConnectionHandle, ZeroRttStatus,
    },
    quic::{
        AcceptBi, AcceptUni, AckFrequencyConfig, ApplicationClose, Chunk, Closed, ClosedStream,
        ConnectionClose, ConnectionError, ConnectionStats, Controller, ControllerFactory,
        ControllerMetrics, CryptoError, DecryptedInitial, Dir, ExportKeyingMaterialError,
        FrameStats, FrameType, HandshakeTokenKey, HeaderKey, IdleTimeout, IncomingAlpns, Keys,
        MtuDiscoveryConfig, OpenBi, OpenUni, PacketKey, PathId, PathStats, QuicConnectError,
        QuicTransportConfig, QuicTransportConfigBuilder, ReadDatagram, ReadError, ReadExactError,
        ReadManyDatagrams, ReadToEndError, RecvStream, ResetError, RttEstimator, SendDatagram,
        SendDatagramError, SendStream, ServerConfig, ServerConfigBuilder, Side, StoppedError,
        StreamId, TimeSource, TokenLog, TokenReuseError, TransportError, TransportErrorCode,
        TransportParameters, UdpStats, UnorderedRecvStream, UnsupportedVersion,
        ValidationTokenConfig, VarInt, VarIntBoundsExceeded, WriteError,
    },
};
#[cfg(not(wasm_browser))]
use crate::socket::transports::IpConfig;
use crate::socket::transports::TransportConfig;
pub use crate::{net_report::NetReportConfig, portmapper::PortmapperConfig};

/// Credentials are kept separately from relay selection and never formatted.
#[derive(Clone, Default)]
pub(crate) struct RelayAuthTokens(BTreeMap<RelayUrl, String>);

impl fmt::Debug for RelayAuthTokens {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str("RelayAuthTokens([REDACTED])")
    }
}

impl RelayAuthTokens {
    pub(crate) fn get(&self, url: &RelayUrl) -> Option<&String> {
        self.0.get(url)
    }
}

/// Builder for [`Endpoint`].
///
/// By default the endpoint will generate a new random [`SecretKey`], which will result in a
/// new [`EndpointId`].
///
/// To create the [`Endpoint`] call [`Builder::bind`].
#[derive(Debug)]
pub struct Builder {
    secret_key: Option<SecretKey>,
    alpn_protocols: Vec<Vec<u8>>,
    transport_config: QuicTransportConfig,
    keylog: bool,
    address_lookup: Vec<Box<dyn DynAddressLookupBuilder>>,
    address_lookup_user_data: Option<UserData>,
    /// Default address filter applied to all address lookup services added via
    /// [`Builder::address_lookup`].
    addr_filter: Option<AddrFilter>,
    proxy_url: Option<Url>,
    ca_tls_config: Option<CaTlsConfig>,
    #[cfg(not(wasm_browser))]
    dns_resolver: Option<DnsResolver>,
    transports: Vec<TransportConfig>,
    relay_auth_tokens: RelayAuthTokens,
    max_tls_tickets: usize,
    hooks: EndpointHooksList,
    path_selector: Arc<dyn PathSelector>,
    portmapper_config: PortmapperConfig,
    net_report_config: NetReportConfig,
    crypto_provider: Option<Arc<rustls::crypto::CryptoProvider>>,
    configured_addrs: BTreeSet<SocketAddr>,
}

impl From<RelayMode> for Option<TransportConfig> {
    fn from(mode: RelayMode) -> Self {
        match mode {
            RelayMode::Disabled => None,
            RelayMode::Default => Some(TransportConfig::Relay {
                relay_map: mode.relay_map(),
                is_user_defined: true,
            }),
            RelayMode::Staging => Some(TransportConfig::Relay {
                relay_map: mode.relay_map(),
                is_user_defined: true,
            }),
            RelayMode::Custom(relay_map) => Some(TransportConfig::Relay {
                relay_map,
                is_user_defined: true,
            }),
        }
    }
}

impl Builder {
    // The ordering of public methods is reflected directly in the documentation.  This is
    // roughly ordered by what is most commonly needed by users.

    /// Creates a new [`Builder`] using the given [`Preset`].
    ///
    /// See [`presets`] for more.
    pub fn new(preset: impl Preset) -> Self {
        Self::empty().preset(preset)
    }

    /// Applies the given [`Preset`].
    pub fn preset(mut self, preset: impl Preset) -> Self {
        self = preset.apply(self);
        self
    }

    /// Creates an empty builder with no address lookup services, and [`RelayMode::Disabled`].
    pub fn empty() -> Self {
        let transports = vec![
            #[cfg(not(wasm_browser))]
            TransportConfig::default_ipv4(),
            #[cfg(not(wasm_browser))]
            TransportConfig::default_ipv6(),
        ];

        Self {
            secret_key: Default::default(),
            alpn_protocols: Default::default(),
            transport_config: QuicTransportConfig::default(),
            keylog: Default::default(),
            address_lookup: Default::default(),
            address_lookup_user_data: Default::default(),
            addr_filter: None,
            proxy_url: None,
            ca_tls_config: None,
            #[cfg(not(wasm_browser))]
            dns_resolver: None,
            max_tls_tickets: DEFAULT_MAX_TLS_TICKETS,
            transports,
            relay_auth_tokens: Default::default(),
            hooks: Default::default(),
            path_selector: Arc::new(BiasedRttPathSelector::default()),
            portmapper_config: Default::default(),
            net_report_config: Default::default(),
            crypto_provider: None,
            configured_addrs: Default::default(),
        }
    }

    // # The final constructor that everyone needs.

    /// Binds the endpoint.
    pub async fn bind(self) -> Result<Endpoint, BindError> {
        let secret_key = self.secret_key.unwrap_or_else(SecretKey::generate);

        let crypto_provider = self
            .crypto_provider
            .ok_or_else(|| e!(BindError::InvalidCryptoProvider))?;

        let token_key = Arc::new(
            RustlsTokenKey::new(&mut rand::rng(), &crypto_provider)
                .ok_or_else(|| e!(BindError::InvalidCryptoProvider))?,
        );

        let span = info_span!("endpoint", id = %secret_key.public().fmt_short());
        let _guard = span.enter();

        let tls_config = tls::TlsConfig::new(
            secret_key.clone(),
            self.max_tls_tickets,
            crypto_provider.clone(),
        );
        let static_config = StaticConfig {
            server_config: tls_config.make_server_config(self.keylog)?,
            client_config: tls_config.make_client_config(self.keylog)?,
            tls_config,
            transport_config: self.transport_config.clone(),
            token_key,
            token_store: Arc::new(noq::TokenMemoryCache::default()),
        };
        let server_config = static_config.create_server_config(self.alpn_protocols);

        let metrics = EndpointMetrics::default();

        let tls_config = self
            .ca_tls_config
            .unwrap_or_default()
            .client_config(crypto_provider)
            .map_err(|err| e!(BindError::InvalidCaRootConfig, err))?;

        #[cfg(not(wasm_browser))]
        let dns_resolver = self.dns_resolver.unwrap_or_else(|| {
            DnsResolver::builder()
                .with_system_defaults()
                .tls_client_config(tls_config.clone())
                .build()
        });

        let sock_opts = socket::Options {
            transports: self.transports,
            relay_auth_tokens: self.relay_auth_tokens,
            secret_key,
            address_lookup_user_data: self.address_lookup_user_data,
            proxy_url: self.proxy_url,
            #[cfg(not(wasm_browser))]
            dns_resolver,
            server_config,
            tls_config,
            metrics,
            hooks: self.hooks,
            path_selector: self.path_selector,
            portmapper_config: self.portmapper_config,
            net_report_config: self.net_report_config,
            static_config,
            configured_addrs: self.configured_addrs,
        };

        let inner = socket::EndpointInner::bind(sock_opts)
            .instrument(Span::current())
            .await?;
        debug!(
            id = %inner.static_config.tls_config.secret_key.public(),
            iroh_version = %env!("CARGO_PKG_VERSION"),
            "iroh endpoint bound"
        );

        let ep = Endpoint {
            inner: Arc::new(inner),
        };

        // Add Address Lookup mechanisms
        let address_lookup = ep.address_lookup().expect("just created the endpoint");
        if let Some(filter) = self.addr_filter {
            address_lookup.set_addr_filter(filter);
        }
        for create_service in self.address_lookup {
            let service = create_service.into_address_lookup(&ep)?;
            address_lookup.add_boxed(service);
        }

        Ok(ep)
    }

    // # The very common methods everyone basically needs.

    /// Binds an IP socket at the provided socket address.
    ///
    /// This is an advanced API to tightly control the sockets used by the endpoint. Most
    /// uses do not need to explicitly bind sockets.
    ///
    /// # Warning
    ///
    /// - The builder always comes pre-configured with an IPv4 socket to be bound on the
    ///   *unspecified* address: `0.0.0.0`. This is the equivalent of using `INADDR_ANY`
    ///   special bind address and results in a socket listening on *all* interfaces
    ///   available.
    ///
    /// - Likewise the builder always comes pre-configured with an IPv6 socket to be bound
    ///   on the *unspecified* address: `[::]`. This bind is allowed to fail however.
    ///
    /// - Adding a bind address removes the pre-configured unspecified bind address for this
    ///   address family. Use [`Self::bind_addr_with_opts`] to bind additional addresses without
    ///   replacing the default bind address.
    ///
    /// - This should be called at most once for each address family: once for IPv4 and/or
    ///   once for IPv6. Calling it multiple times for the same address family will result
    ///   in undefined routing behaviour. To bind multiple sockets of the same address
    ///   family, use [`Self::bind_addr_with_opts`].
    ///
    /// # Description
    ///
    /// Requests a socket to be bound on a specific address, with an implied netmask of
    /// `/0`. This allows restricting binding to only one network interface for a given
    /// address family.
    ///
    /// If the port specified is already in use, binding the endpoint will fail. Using
    /// port `0` in the socket address assigns a random free port.
    ///
    /// # Example
    ///
    /// ```no_run
    /// # #[cfg(with_crypto_provider)]
    /// # {
    /// # #[tokio::main]
    /// # async fn main() -> n0_error::Result<()> {
    /// # use iroh::{Endpoint, endpoint::presets};
    /// let endpoint = Endpoint::builder(presets::N0)
    ///     .clear_ip_transports()
    ///     .bind_addr("127.0.0.1:0")?
    ///     .bind_addr("[::1]:0")?
    ///     .bind()
    ///     .await?;
    /// # Ok(()) }
    /// # }
    /// ```
    #[cfg(not(wasm_browser))]
    pub fn bind_addr<A>(self, addr: A) -> Result<Self, InvalidSocketAddr>
    where
        A: ToSocketAddr,
        <A as ToSocketAddr>::Err: Into<InvalidSocketAddr>,
    {
        self.bind_addr_with_opts(addr, BindOpts::default())
    }

    /// Binds an IP socket at the provided socket address.
    ///
    /// This is an advanced API to tightly control the sockets used by the endpoint. Most
    /// uses do not need to explicitly bind sockets.
    ///
    /// # Warning
    ///
    /// - The builder always comes pre-configured with an IPv4 socket to be bound on the
    ///   *unspecified* address: `0.0.0.0`. This is the equivalent of using `INADDR_ANY`
    ///   special bind address and results in a socket listening on *all* interfaces
    ///   available.
    ///
    /// - Likewise the builder always comes pre-configured with an IPv6 socket to be bound
    ///   on the *unspecified* address: `[::]`. This bind is allowed to fail however.
    ///
    /// # Description
    ///
    /// Requests a socket to be bound on a specific address. This allows restricting binding
    /// to only one network interface for a given address family.
    ///
    /// [`BindOpts::set_prefix_len`] **should** be used to configure the netmask of the
    /// network interface. This allows outgoing datagrams that start a new network flow to
    /// be sent over the socket which is attached to the subnet of the destination
    /// address. If multiple sockets are bound the standard routing-table semantics are
    /// used: the socket attached to the subnet with the longest prefix matching the
    /// destination is used. Practically this means the smallest subnets are at the top of
    /// the routing table, and the first subnet containing the destination address is
    /// chosen.
    ///
    /// If no socket is bound to a subnet that contains the destination address, the notion
    /// of "default route" is used. At most one socket per address family may be marked as
    /// the default route using [`BindOpts::set_is_default_route`], and this will be used
    /// for destinations not contained by the subnets of the bound sockets. This network is
    /// expected to have a default gateway configured. A socket with a prefix length of `/0`
    /// will be set as a "default route" implicitly, unless [`BindOpts::set_is_default_route`]
    /// is set to `false` explicitly.
    ///
    /// Be aware that using a subnet with a prefix length of `/0` will always contain all
    /// destination addresses. It is valid to configure this, but no more than one such
    /// socket should be bound or the routing will be non-deterministic.
    ///
    /// To use a subnet with a non-zero prefix length as the default route in addition to
    /// being routed when its prefix matches, use [`BindOpts::set_is_default_route`].
    /// Subnets with a prefix length of zero are always marked as default routes.
    ///
    /// Finally note that most outgoing datagrams are part of an existing network flow. That
    /// is, they are in response to an incoming datagram. In this case the outgoing datagram
    /// will be sent over the same socket as the incoming datagram was received on, and the
    /// routing with the prefix length and default route as described above does not apply.
    ///
    /// Using port `0` in the socket address assigns a random free port.
    ///
    /// If the port specified is already in use, binding the endpoint will fail, unless
    /// [`BindOpts::set_is_required`] is set to `false`
    ///
    /// # Example
    /// ```no_run
    /// # #[cfg(with_crypto_provider)]
    /// # {
    /// # #[tokio::main]
    /// # async fn main() -> n0_error::Result<()> {
    /// # use iroh::{Endpoint, endpoint::{BindOpts, presets}};
    /// let endpoint = Endpoint::builder(presets::N0)
    ///     .clear_ip_transports()
    ///     .bind_addr_with_opts("127.0.0.1:0", BindOpts::default().set_prefix_len(24))?
    ///     .bind_addr_with_opts("[::1]:0", BindOpts::default().set_prefix_len(48))?
    ///     .bind()
    ///     .await?;
    /// # Ok(()) }
    /// # }
    /// ```
    #[cfg(not(wasm_browser))]
    pub fn bind_addr_with_opts<A>(
        mut self,
        addr: A,
        opts: BindOpts,
    ) -> Result<Self, InvalidSocketAddr>
    where
        A: ToSocketAddr,
        <A as ToSocketAddr>::Err: Into<InvalidSocketAddr>,
    {
        let addr = addr.to_socket_addr().map_err(Into::into)?;
        match addr {
            SocketAddr::V4(addr) => {
                if self
                    .transports
                    .iter()
                    .any(|t| t.is_ipv4_default() && t.is_user_defined())
                {
                    bail!(InvalidSocketAddr::DuplicateDefaultAddr);
                }

                let ip_net = Ipv4Net::new(*addr.ip(), opts.prefix_len())
                    .map_err(|_| e!(InvalidSocketAddr::InvalidPrefixLength))?;
                self.transports.push(TransportConfig::Ip {
                    config: IpConfig::V4 {
                        ip_net,
                        port: addr.port(),
                        is_required: opts.is_required(),
                        is_default: opts.is_default_route(),
                    },
                    is_user_defined: true,
                });
            }
            SocketAddr::V6(addr) => {
                if self
                    .transports
                    .iter()
                    .any(|t| t.is_ipv6_default() && t.is_user_defined())
                {
                    bail!(InvalidSocketAddr::DuplicateDefaultAddr);
                }

                let ip_net = Ipv6Net::new(*addr.ip(), opts.prefix_len())
                    .map_err(|_| e!(InvalidSocketAddr::InvalidPrefixLength))?;
                self.transports.push(TransportConfig::Ip {
                    config: IpConfig::V6 {
                        ip_net,
                        scope_id: addr.scope_id(),
                        port: addr.port(),
                        is_required: opts.is_required(),
                        is_default: opts.is_default_route(),
                    },
                    is_user_defined: true,
                });
            }
        }
        Ok(self)
    }

    /// Removes all IP based transports.
    #[cfg(not(wasm_browser))]
    pub fn clear_ip_transports(mut self) -> Self {
        self.transports
            .retain(|t| !matches!(t, TransportConfig::Ip { .. }));
        self
    }

    /// Removes all relay based transports.
    pub fn clear_relay_transports(mut self) -> Self {
        self.transports
            .retain(|t| !matches!(t, TransportConfig::Relay { .. }));
        self
    }

    /// Sets a secret key to authenticate with other peers.
    ///
    /// This secret key's public key will be the [`PublicKey`] of this endpoint and thus
    /// also its [`EndpointId`]
    ///
    /// If not set, a new secret key will be generated.
    ///
    /// [`PublicKey`]: iroh_base::PublicKey
    pub fn secret_key(mut self, secret_key: SecretKey) -> Self {
        self.secret_key = Some(secret_key);
        self
    }

    /// Sets the [ALPN] protocols that this endpoint will accept on incoming connections.
    ///
    /// Not setting this will still allow creating connections, but to accept incoming
    /// connections at least one [ALPN] must be set.
    ///
    /// Ordering matters for protocol negotiation. When an incoming connection offers multiple ALPNs,
    /// the first matching ALPN will be chosen. This means that `alpns` should be ordered such
    /// that the preferred protocols come first.
    ///
    /// [ALPN]: https://en.wikipedia.org/wiki/Application-Layer_Protocol_Negotiation
    pub fn alpns(mut self, alpn_protocols: Vec<Vec<u8>>) -> Self {
        self.alpn_protocols = alpn_protocols;
        self
    }

    // # Methods for common customisation items.

    /// Sets the relay servers to assist in establishing connectivity.
    ///
    /// Relay servers are used to establish initial connection with another iroh endpoint.
    /// They also perform various functions related to hole punching, see the [crate docs]
    /// for more details.
    ///
    /// By default the [number 0] relay servers are used, see [`RelayMode::Default`].
    ///
    /// When using [RelayMode::Custom], the provided `relay_map` must contain at least one
    /// configured relay endpoint.  If an invalid RelayMap is provided [`bind`]
    /// will result in an error.
    ///
    /// [`bind`]: Builder::bind
    /// [crate docs]: crate
    /// [number 0]: https://n0.computer
    pub fn relay_mode(mut self, relay_mode: RelayMode) -> Self {
        let transport: Option<_> = relay_mode.into();
        match transport {
            Some(transport) => {
                if let Some(og) = self
                    .transports
                    .iter_mut()
                    .find(|t| matches!(t, TransportConfig::Relay { .. }))
                {
                    *og = transport;
                } else {
                    self.transports.push(transport);
                }
            }
            None => {
                self.transports
                    .retain(|t| !matches!(t, TransportConfig::Relay { .. }));
            }
        }
        self
    }

    /// Sets authentication tokens for exact relay URLs, independently of relay selection.
    ///
    /// This replaces the token table without adding any home relay candidates. The last
    /// entry for a duplicate URL wins. Entries override [`RelayConfig::auth_token`] for
    /// the same URL; URLs absent from this table retain the relay configuration's token.
    /// Tokens are fixed for this endpoint's lifetime and are excluded from its debug output.
    pub fn relay_auth_tokens(
        mut self,
        tokens: impl IntoIterator<Item = (RelayUrl, String)>,
    ) -> Self {
        self.relay_auth_tokens = RelayAuthTokens(tokens.into_iter().collect());
        self
    }

    /// Removes all Address Lookup services from the builder.
    ///
    /// If no Address Lookup is set, connecting to an endpoint without providing its
    /// direct addresses or relay URLs will fail.
    ///
    /// See the documentation of the [`crate::address_lookup::AddressLookup`] trait for details.
    pub fn clear_address_lookup(mut self) -> Self {
        self.address_lookup.clear();
        self
    }

    /// Adds an additional Address Lookup for this endpoint.
    ///
    /// Once the endpoint is created the provided [`AddressLookupBuilder::into_address_lookup`] will be
    /// called. This allows Address Lookup's to finalize their configuration by e.g. using
    /// the secret key from the endpoint which can be needed to sign published information.
    ///
    /// This method can be called multiple times and all the Address Lookup's passed in
    /// will be combined using an internal instance of the
    /// [`crate::address_lookup::AddressLookupServices`]. To clear all Address Lookup's, use
    /// [`Self::clear_address_lookup`].
    ///
    /// If no Address Lookup is set, connecting to an endpoint without providing its
    /// direct addresses or relay URLs will fail.
    ///
    /// See the documentation of the [`crate::address_lookup::AddressLookup`] trait for details.
    pub fn address_lookup(mut self, address_lookup: impl AddressLookupBuilder) -> Self {
        self.address_lookup.push(Box::new(address_lookup));
        self
    }

    /// Sets the address filter applied to all address data before publishing.
    ///
    /// This filter is applied once, at the [`AddressLookupServices`] level, before
    /// distributing data to any individual address lookup service. This ensures
    /// consistent filtering regardless of how the services are configured.
    ///
    /// [`AddressLookupServices`]: crate::address_lookup::AddressLookupServices
    pub fn addr_filter(mut self, filter: AddrFilter) -> Self {
        self.addr_filter = Some(filter);
        self
    }

    /// Clears the address filter, allowing all addresses to be published.
    ///
    /// This removes any filter previously set via [`Self::addr_filter`], including
    /// filters set by presets.
    pub fn clear_addr_filter(mut self) -> Self {
        self.addr_filter = None;
        self
    }

    /// Sets the initial user-defined data to be published in Address Lookup's for this node.
    ///
    /// When using Address Lookup's, this string of [`UserData`] will be published together
    /// with the endpoint's addresses and relay URL. When other endpoints discover this endpoint,
    /// they retrieve the [`UserData`] in addition to the addressing info.
    ///
    /// Iroh itself does not interpret the user-defined data in any way, it is purely left
    /// for applications to parse and use.
    pub fn user_data_for_address_lookup(mut self, user_data: UserData) -> Self {
        self.address_lookup_user_data = Some(user_data);
        self
    }

    /// Adds an external address on which this endpoint is directly reachable.
    ///
    /// This address will be advertised to peers together with any discovered external addresses
    /// and will be used in NAT traversal and to establish direct connections.
    ///
    /// Can be called multiple times. See also [`Endpoint::add_external_addr`] for
    /// adding addresses at runtime.
    pub fn external_addr(mut self, addr: SocketAddr) -> Self {
        self.configured_addrs.insert(addr);
        self
    }

    // # Methods for more specialist customisation.

    /// Sets a custom [`QuicTransportConfig`] for this endpoint.
    ///
    /// The transport config contains parameters governing the QUIC state machine.
    ///
    /// If unset, the default config is used. Default values should be suitable for most
    /// internet applications. Applications protocols which forbid remotely-initiated
    /// streams should set `max_concurrent_bidi_streams` and `max_concurrent_uni_streams` to
    /// zero.
    ///
    /// Please be aware that changing some settings may have adverse effects on establishing
    /// and maintaining direct connections.
    pub fn transport_config(mut self, transport_config: QuicTransportConfig) -> Self {
        self.transport_config = transport_config;
        self
    }

    /// Optionally sets a custom DNS resolver to use for this endpoint.
    ///
    /// The DNS resolver is used to resolve relay hostnames, and endpoint addresses if
    /// [`crate::address_lookup::DnsAddressLookup`] is configured.
    ///
    /// By default, a new DNS resolver is created which is configured to use the
    /// host system's DNS configuration. You can pass a custom instance of [`DnsResolver`]
    /// here to use a differently configured DNS resolver for this endpoint, or to share
    /// a [`DnsResolver`] between multiple endpoints.
    #[cfg(not(wasm_browser))]
    pub fn dns_resolver(mut self, dns_resolver: DnsResolver) -> Self {
        self.dns_resolver = Some(dns_resolver);
        self
    }

    /// Sets an explicit proxy url to proxy all HTTP(S) traffic through.
    pub fn proxy_url(mut self, url: Url) -> Self {
        self.proxy_url.replace(url);
        self
    }

    /// Sets the proxy url from the environment, in this order:
    ///
    /// - `HTTP_PROXY`
    /// - `http_proxy`
    /// - `HTTPS_PROXY`
    /// - `https_proxy`
    pub fn proxy_from_env(mut self) -> Self {
        self.proxy_url = proxy_url_from_env();
        self
    }

    /// Sets the trusted CA root certificates for non-iroh TLS connections.
    ///
    /// These Certificate Authority roots are used as trust anchors for verifying
    /// the validity of TLS certificates presented by external services, such as
    /// iroh relays, pkarr servers, or DNS-over-HTTPS resolvers.
    /// They don't need to be trusted for the integrity or authenticity of native
    /// iroh connections, which rely on iroh's own cryptographic authentication mechanisms.
    pub fn ca_tls_config(mut self, ca_tls_config: CaTlsConfig) -> Self {
        self.ca_tls_config = Some(ca_tls_config);
        self
    }

    /// Renamed to [`Builder::ca_tls_config`].
    #[deprecated(since = "1.0.0", note = "Renamed to `ca_tls_config`")]
    pub fn ca_roots_config(self, ca_roots_config: CaTlsConfig) -> Self {
        self.ca_tls_config(ca_roots_config)
    }

    /// Enables saving the TLS pre-master key for connections.
    ///
    /// This key should normally remain secret but can be useful to debug networking issues
    /// by decrypting captured traffic.
    ///
    /// If *keylog* is `true` then setting the `SSLKEYLOGFILE` environment variable to a
    /// filename will result in this file being used to log the TLS pre-master keys.
    pub fn keylog(mut self, keylog: bool) -> Self {
        self.keylog = keylog;
        self
    }

    /// Set the maximum number of TLS tickets to cache.
    ///
    /// Set this to a larger value if you want to do 0rtt connections to a large
    /// number of clients.
    ///
    /// The default is 256, taking about 150 KiB in memory.
    pub fn max_tls_tickets(mut self, n: usize) -> Self {
        self.max_tls_tickets = n;
        self
    }

    /// Specify the rustls cryptography to use for all TLS operations.
    ///
    /// This includes
    /// - TLS for encryption and authentication of iroh connections themselves, but also
    /// - HTTPS connections to relays
    /// - Pkarr relay publishing HTTPS connections
    /// - and any other Address Lookup services that decide to use [`Endpoint::tls_config`].
    ///
    /// The two most common crypto providers in use today are `ring` as well as `aws-lc-rs`.
    ///
    /// If either the `tls-ring` or `tls-aws-lc-rs` feature is set in iroh, this function doesn't
    /// need to be called.
    ///
    /// If none of these features are set, then calling this function in the builder is mandatory.
    pub fn crypto_provider(mut self, crypto_provider: Arc<rustls::crypto::CryptoProvider>) -> Self {
        self.crypto_provider = Some(crypto_provider);
        self
    }

    /// Install hooks onto the endpoint.
    ///
    /// Endpoint hooks intercept the connection establishment process of an [`Endpoint`].
    ///
    /// You can install multiple [`EndpointHooks`] by calling this function multiple times.
    /// Order matters: hooks are invoked in the order they were installed onto the endpoint
    /// builder. Once a hook returns reject, further processing
    /// is aborted and other hooks won't be invoked.
    ///
    /// See [`EndpointHooks`] for details on the possible interception points in the connection lifecycle.
    pub fn hooks(mut self, hooks: impl EndpointHooks + 'static) -> Self {
        self.hooks.push(hooks);
        self
    }

    /// Configures the portmapper service (UPnP, PCP, NAT-PMP).
    ///
    /// Defaults to [`PortmapperConfig::Enabled`]. Pass
    /// [`PortmapperConfig::Disabled`] to avoid gateway probing (e.g. if it
    /// triggers firewall prompts).
    pub fn portmapper_config(mut self, config: PortmapperConfig) -> Self {
        self.portmapper_config = config;
        self
    }

    /// Configures the net report.
    ///
    /// The net report component is responsible for figuring out if and how the endpoint is connected to the internet.
    /// It does this by doing various probes to the configured relay servers to get public addresses, NAT behaviour, and
    /// relay latencies. In addition it tries to detect captive portals.
    ///
    /// Some non-essential features of the net report component can be disabled via this configuration.
    pub fn net_report_config(mut self, config: NetReportConfig) -> Self {
        self.net_report_config = config;
        self
    }

    /// Adds a custom transport to the endpoint.
    ///
    /// <div class="warning">
    ///
    /// This API is unstable and gated behind the `unstable-custom-transport` feature.
    /// It is not covered by semantic versioning guarantees and may change in any release
    /// without a major version bump.
    ///
    /// </div>
    #[cfg(feature = "unstable-custom-transports")]
    pub fn add_custom_transport(mut self, factory: Arc<dyn CustomTransport>) -> Self {
        self.transports.push(TransportConfig::Custom(factory));
        self
    }

    /// Sets a custom [`PathSelector`] for this endpoint.
    ///
    /// The path selector decides which path to use among the candidate paths to a
    /// remote endpoint.  By default iroh uses a built-in selector that sorts paths by
    /// biased RTT (with IPv6 preferred over IPv4 and relay treated as backup) and is
    /// sticky to avoid flapping.  Pass a custom [`PathSelector`] here to override that
    /// policy — for example, to make a custom transport always win over IP.
    ///
    /// Takes an `Arc<dyn PathSelector>` so the same selector instance can be shared
    /// across multiple endpoints if desired.  See `examples/custom-transport.rs` for
    /// an example implementation.
    ///
    /// <div class="warning">
    ///
    /// This API is unstable and gated behind the `unstable-custom-transport` feature.
    /// It is not covered by semantic versioning guarantees and may change in any release
    /// without a major version bump.
    ///
    /// </div>
    ///
    /// [`PathSelector`]: socket::remote_map::PathSelector
    #[cfg(feature = "unstable-custom-transports")]
    pub fn path_selector(mut self, selector: Arc<dyn PathSelector>) -> Self {
        self.path_selector = selector;
        self
    }
}

#[allow(missing_docs)]
#[stack_error(derive, add_meta)]
#[non_exhaustive]
pub enum EndpointError {
    #[error("Endpoint is closed")]
    Closed,
}

/// Controls an iroh endpoint, establishing connections with other endpoints.
///
/// This is the main API interface to create connections to, and accept connections from
/// other iroh endpoints.  The connections are peer-to-peer and encrypted, a Relay server is
/// used to make the connections reliable.  See the [crate docs] for a more detailed
/// overview of iroh.
///
/// It is recommended to only create a single instance per application.  This ensures all
/// the connections made share the same peer-to-peer connections to other iroh endpoints,
/// while still remaining independent connections.  This will result in more optimal network
/// behaviour.
///
/// The endpoint is created using the [`Builder`], which can be created using
/// [`Endpoint::builder`].
///
/// Once an endpoint exists, new connections are typically created using the
/// [`Endpoint::connect`] and [`Endpoint::accept`] methods.  Once established, the
/// [`Connection`] gives access to most [QUIC] features.  Individual streams to send data to
/// the peer are created using the [`Connection::open_bi`], [`Connection::accept_bi`],
/// [`Connection::open_uni`] and [`Connection::accept_uni`] functions.
///
/// Note that due to the light-weight properties of streams a stream will only be accepted
/// once the initiating peer has sent some data on it.
///
/// # Usage on Android
///
/// The endpoint's default [`DnsResolver`] reads the system DNS configuration
/// through JNI, which needs a JVM context published to [`ndk_context`]. Apps
/// should initialize that context before constructing the endpoint. See
/// [`iroh_dns::install_android_jni_context`] for details (the function is also
/// exported as `iroh::dns::install_android_jni_context`).
///
/// If no JNI context is installed, iroh relies on panic unwinding to detect
/// the error, and will then use the fallback nameservers instead, subject to the
/// resolver's [`FallbackMode`]. Note that if your compilation profile sets
/// `panic = "abort"`, this can't work, and thus your app will panic if using a
/// default `DnsResolver` without first initializing the JNI context.
///
/// [QUIC]: https://quicwg.org
/// [`DnsResolver`]: crate::dns::DnsResolver
/// [`FallbackMode`]: crate::dns::FallbackMode
/// [`ndk_context`]: https://docs.rs/ndk-context
/// [`iroh_dns::install_android_jni_context`]: https://docs.rs/iroh-dns/latest/iroh_dns/fn.install_android_jni_context.html
// The last link can't be a normal doclink, because #[cfg(doc)] can't cross crate boundaries unfortunately.
#[derive(Clone, Debug)]
pub struct Endpoint {
    inner: Arc<EndpointInner>,
}

#[allow(missing_docs)]
#[stack_error(derive, add_meta, from_sources)]
#[non_exhaustive]
#[allow(private_interfaces)]
pub enum ConnectWithOptsError {
    #[error("Connecting to ourself is not supported")]
    SelfConnect,
    #[error("No addressing information available")]
    NoAddress { source: AddressLookupFailed },
    #[error("Unable to connect to remote")]
    Noq {
        #[error(std_err)]
        source: QuicConnectError,
    },
    #[error("Internal consistency error")]
    InternalConsistencyError {
        /// Private source type, cannot be created publicly.
        source: RemoteStateActorStoppedError,
    },
    #[error("Connection was rejected locally")]
    LocallyRejected,
    #[error("Endpoint is closed")]
    EndpointClosed,
    #[error("Invalid ALPN")]
    InvalidAlpn,
}

#[allow(missing_docs)]
#[stack_error(derive, add_meta, from_sources)]
#[non_exhaustive]
pub enum ConnectError {
    #[error(transparent)]
    Connect { source: ConnectWithOptsError },
    #[error(transparent)]
    Connecting { source: ConnectingError },
    #[error(transparent)]
    Connection {
        #[error(std_err)]
        source: ConnectionError,
    },
}

impl Endpoint {
    // The ordering of public methods is reflected directly in the documentation.  This is
    // roughly ordered by what is most commonly needed by users, but grouped in similar
    // items.

    // # Methods relating to construction.

    /// Returns the builder for an [`Endpoint`], with the given [`Preset`] configuration.
    pub fn builder(preset: impl Preset) -> Builder {
        Builder::new(preset)
    }

    /// Constructs a default [`Endpoint`] using the provided [`Preset`] and binds it immediately.
    pub async fn bind(preset: impl Preset) -> Result<Self, BindError> {
        Self::builder(preset).bind().await
    }

    /// Sets the list of accepted ALPN protocols.
    ///
    /// Ordering matters for protocol negotiation. When an incoming connection offers multiple ALPNs,
    /// the first matching ALPN will be chosen. This means that `alpns` should be ordered such
    /// that the preferred protocols come first.
    ///
    /// This will only affect new incoming connections.
    /// Note that this *overrides* the current list of ALPNs.
    ///
    /// If the endpoint is closed, this method will log a warning and ignore
    /// the request to set new ALPNs.
    pub fn set_alpns(&self, alpns: Vec<Vec<u8>>) {
        if self.is_closed() {
            warn!("Attempting to set ALPNs for a closed endpoint. Ignoring.");
            return;
        }
        let server_config = self.inner.static_config.create_server_config(alpns);
        self.inner
            .noq_endpoint()
            .set_server_config(Some(server_config));
    }

    /// Adds the provided configuration to the [`RelayMap`].
    ///
    /// Replacing and returning any existing configuration for [`RelayUrl`].
    ///
    /// Will also return `None` if the endpoint is closed.
    pub async fn insert_relay(
        &self,
        relay: RelayUrl,
        config: Arc<RelayConfig>,
    ) -> Option<Arc<RelayConfig>> {
        if self.is_closed() {
            return None;
        }
        self.inner.insert_relay(relay, config).await
    }

    /// Removes the configuration from the [`RelayMap`] for the provided [`RelayUrl`].
    ///
    /// Returns any existing configuration if it exists. Will also return `None` if the endpoint is closed.
    pub async fn remove_relay(&self, relay: &RelayUrl) -> Option<Arc<RelayConfig>> {
        if self.is_closed() {
            return None;
        }
        self.inner.remove_relay(relay).await
    }

    /// Adds an external address on which this endpoint is directly reachable.
    ///
    /// This address will be advertised to peers together with any discovered external addresses
    /// and will be used in NAT traversal and to establish direct connections.
    ///
    /// See also [`Builder::external_addr`] for setting addresses at build time.
    pub async fn add_external_addr(&self, addr: SocketAddr) {
        if self.is_closed() {
            warn!("Attempting to add external addr for a closed endpoint. Ignoring.");
            return;
        }
        self.inner.add_external_addr(addr).await;
    }

    /// Removes a configured external address. Returns `true` if it was present.
    pub async fn remove_external_addr(&self, addr: &SocketAddr) -> bool {
        if self.is_closed() {
            return false;
        }
        self.inner.remove_external_addr(addr).await
    }

    // # Methods for establishing connectivity.

    /// Connects to a remote [`Endpoint`].
    ///
    /// A value that can be converted into an [`EndpointAddr`] is required. This can be either an
    /// [`EndpointAddr`] or an [`EndpointId`].
    ///
    /// The [`EndpointAddr`] must contain the [`EndpointId`] to dial and may also contain a [`RelayUrl`]
    /// and direct addresses. If direct addresses are provided, they will be used to try and
    /// establish a direct connection without involving a relay server.
    ///
    /// If neither a [`RelayUrl`] or direct addresses are configured in the [`EndpointAddr`] it
    /// may still be possible a connection can be established.  This depends on which, if any,
    /// [`crate::address_lookup::AddressLookup`]s were configured using [`Builder::address_lookup`].  The Address Lookup
    /// service will also be used if the remote endpoint is not reachable on the provided direct
    /// addresses and there is no [`RelayUrl`].
    ///
    /// If addresses or relay servers are neither provided nor can be discovered, the
    /// connection attempt will fail with an error.
    ///
    /// The `alpn`, or application-level protocol identifier, is also required. The remote
    /// endpoint must support this `alpn`, otherwise the connection attempt will fail with
    /// an error.
    ///
    /// [`RelayUrl`]: crate::RelayUrl
    pub async fn connect(
        &self,
        endpoint_addr: impl Into<EndpointAddr>,
        alpn: &[u8],
    ) -> Result<Connection, ConnectError> {
        let endpoint_addr = endpoint_addr.into();
        let remote = endpoint_addr.id;
        let connecting = self
            .connect_with_opts(endpoint_addr, alpn, Default::default())
            .await?;
        let conn = connecting.await?;

        debug!(
            me = %self.id().fmt_short(),
            remote = %remote.fmt_short(),
            alpn = %String::from_utf8_lossy(alpn),
            "Connection established."
        );
        Ok(conn)
    }

    /// Starts a connection attempt with a remote [`Endpoint`].
    ///
    /// Like [`Endpoint::connect`] (see also its docs for general details), but allows for a more
    /// advanced connection setup with more customization in two aspects:
    /// 1. The returned future resolves to a [`Connecting`], which can be further processed into
    ///    a [`Connection`] by awaiting, or alternatively allows connecting with 0-RTT via
    ///    [`Connecting::into_0rtt`].
    ///    **Note:** Please read the documentation for `into_0rtt` carefully to assess
    ///    security concerns.
    /// 2. The [`QuicTransportConfig`] for the connection can be modified via the provided
    ///    [`ConnectOptions`].
    ///    **Note:** Please be aware that changing transport config settings may have adverse effects on
    ///    establishing and maintaining direct connections.  Carefully test settings you use and
    ///    consider this currently as still rather experimental.
    #[instrument(name = "connect", skip_all, fields(
        me = %self.id().fmt_short(),
        remote = tracing::field::Empty,
        alpn = %String::from_utf8_lossy(alpn).to_string(),
    ))]
    pub async fn connect_with_opts(
        &self,
        endpoint_addr: impl Into<EndpointAddr>,
        alpn: &[u8],
        options: ConnectOptions,
    ) -> Result<Connecting, ConnectWithOptsError> {
        if self.is_closed() {
            return Err(e!(ConnectWithOptsError::EndpointClosed));
        }

        let endpoint_addr: EndpointAddr = endpoint_addr.into();
        let endpoint_id = endpoint_addr.id;

        Span::current().record("remote", tracing::field::display(endpoint_id.fmt_short()));

        if let BeforeConnectOutcome::Reject =
            self.inner.hooks.before_connect(&endpoint_addr, alpn).await
        {
            return Err(e!(ConnectWithOptsError::LocallyRejected));
        }

        // Connecting to ourselves is not supported.
        ensure!(endpoint_id != self.id(), ConnectWithOptsError::SelfConnect);
        ensure!(!alpn.is_empty(), ConnectWithOptsError::InvalidAlpn);

        event!(
            target: "iroh::_events::conn::connecting",
            tracing::Level::DEBUG,
            remote_id = %endpoint_id.fmt_short(),
            alpn = %String::from_utf8_lossy(alpn),
        );

        debug!(
            relay_url = ?endpoint_addr.relay_urls().next().cloned(),
            ip_addresses = ?endpoint_addr.ip_addrs().cloned().collect::<Vec<_>>(),
            "connecting",
        );

        let mapped_addr = self.inner.resolve_remote(endpoint_addr).await??;

        let transport_config = options
            .transport_config
            .map(|cfg| cfg.to_inner_arc())
            .unwrap_or(self.inner.static_config.transport_config.to_inner_arc());

        // Start connecting via noq. This will time out after 10 seconds if no reachable
        // address is available.

        let mut alpn_protocols = vec![alpn.to_vec()];
        alpn_protocols.extend(options.additional_alpns);
        let client_config = self
            .inner
            .static_config
            .create_client_config(alpn_protocols, transport_config.clone());

        let dest_addr = mapped_addr.private_socket_addr();
        let server_name = &tls::name::encode(endpoint_id);
        let connect =
            self.inner
                .noq_endpoint()
                .connect_with(client_config, dest_addr, server_name)?;

        Ok(Connecting::new(connect, self.clone(), endpoint_id))
    }

    /// Accepts an incoming connection on the endpoint.
    ///
    /// Only connections with the ALPNs configured in [`Builder::alpns`] will be accepted.
    /// If multiple ALPNs have been configured the ALPN can be inspected before accepting
    /// the connection using [`Connecting::alpn`].
    ///
    /// The returned future will yield `None` if the endpoint is closed by calling
    /// [`Endpoint::close`].
    pub fn accept(&self) -> Accept<'_> {
        Accept {
            inner: self.inner.noq_endpoint().accept(),
            ep: self.clone(),
        }
    }

    // # Getter methods for properties of this Endpoint itself.

    /// Returns the secret_key of this endpoint.
    pub fn secret_key(&self) -> &SecretKey {
        &self.inner.static_config.tls_config.secret_key
    }

    /// Returns the endpoint id of this endpoint.
    ///
    /// This ID is the unique addressing information of this endpoint and other peers must know
    /// it to be able to connect to this endpoint.
    pub fn id(&self) -> EndpointId {
        self.inner.static_config.tls_config.secret_key.public()
    }

    /// Returns the current [`EndpointAddr`].
    /// As long as the endpoint was able to bind to a network interface, some
    /// local addresses will be available.
    ///
    /// The state of other fields depends on the state of networking and connectivity.
    /// Use the [`Endpoint::online`] method to ensure that the endpoint is considered
    /// "online" (has contacted a relay server) before calling this method, if you want
    /// to ensure that the `EndpointAddr` will contain enough information to allow this endpoint
    /// to be dialable by a remote endpoint over the internet.
    ///
    /// You can use the [`Endpoint::watch_addr`] method to get updates when the `EndpointAddr`
    /// changes.
    pub fn addr(&self) -> EndpointAddr {
        self.watch_addr().get()
    }

    /// Returns a [`Watcher`] for the current [`EndpointAddr`] for this endpoint.
    ///
    /// The observed [`EndpointAddr`] will have the current [`RelayUrl`] and direct addresses.
    ///
    /// ```no_run
    /// # #[cfg(with_crypto_provider)]
    /// # {
    /// # async fn wrapper() -> n0_error::Result<()> {
    /// use iroh::{Endpoint, Watcher, endpoint::presets};
    ///
    /// let endpoint = Endpoint::builder(presets::N0)
    ///     .alpns(vec![b"my-alpn".to_vec()])
    ///     .bind()
    ///     .await?;
    /// let endpoint_addr = endpoint.watch_addr().get();
    /// # let _ = endpoint_addr;
    /// # Ok(())
    /// # }
    /// # }
    /// ```
    ///
    /// The [`Endpoint::online`] method can be used as a convenience method to
    /// understand if the endpoint has ever been considered "online". But after
    /// that initial call to [`Endpoint::online`], to understand if your
    /// endpoint is no longer able to be connected to by endpoints outside
    /// of the private or local network, watch for changes in its [`EndpointAddr`].
    /// If there are no `addrs` in the [`EndpointAddr`], you may not be dialable by other endpoints
    /// on the internet.
    ///
    /// The `EndpointAddr` will change as:
    /// - network conditions change
    /// - the endpoint connects to a relay server
    /// - the endpoint changes its preferred relay server
    /// - more addresses are discovered for this endpoint
    ///
    /// ## Closing behavior
    ///
    /// The returned watcher only becomes disconnected once the last clone of the [`Endpoint`]
    /// is dropped. Closing the endpoint does not disconnect the watcher. Thus, a stream created
    /// via [`Watcher::stream`] only terminates once the endpoint is fully dropped. To stop a task
    /// that loops over a watcher stream once the endpoint stops, combine with [`Self::closed`]:
    ///
    /// ```no_run
    /// # #[cfg(with_crypto_provider)]
    /// # {
    /// # use iroh::{Watcher, Endpoint, endpoint::presets};
    /// # use n0_future::StreamExt;
    /// # use tracing::info;
    /// # async fn wrapper() -> n0_error::Result<()> {
    /// let endpoint = Endpoint::bind(presets::N0).await?;
    /// // We want to watch address changes in a different task, and stop our task
    /// // once the endpoint stops.
    /// let mut addr_stream = endpoint.watch_addr().stream();
    /// let endpoint_closed = endpoint.closed();
    /// tokio::spawn(endpoint_closed.run_until(async move {
    ///     while let Some(addr) = addr_stream.next().await {
    ///         info!("our address changed: {addr:?}");
    ///     }
    ///     info!("endpoint closed");
    /// }));
    /// // Do fancy things, then close the endpoint.
    /// // Our task above will stop even if there are still clones of `Endpoint` alive somewhere.
    /// endpoint.close().await;
    /// # Ok(())
    /// # }
    /// # }
    /// ```
    ///
    /// [`RelayUrl`]: crate::RelayUrl
    #[cfg(not(wasm_browser))]
    pub fn watch_addr(&self) -> impl n0_watcher::Watcher<Value = EndpointAddr> + use<> {
        let watch_addrs = self.inner.ip_addrs();
        let watch_relay = self.inner.home_relay();
        let endpoint_id = self.id();

        watch_addrs.or(watch_relay).map(move |(addrs, relays)| {
            EndpointAddr::from_parts(
                endpoint_id,
                relays
                    .into_iter()
                    .map(TransportAddr::Relay)
                    .chain(addrs.into_iter().map(|x| TransportAddr::Ip(x.addr))),
            )
        })
    }

    /// Returns a [`Watcher`] for the current [`EndpointAddr`] for this endpoint.
    ///
    /// When compiled to Wasm, this function returns a watcher that initializes
    /// with an [`EndpointAddr`] that only contains a relay URL, but no direct addresses,
    /// as there are no APIs for directly using sockets in browsers.
    ///
    /// The returned watcher only becomes disconnected once the last clone of the [`Endpoint`]
    /// is dropped. Closing the endpoint does not disconnect the watcher. Thus, a stream created
    /// via [`Watcher::stream`] only terminates once the endpoint stops. If you want to stop a
    /// task once the endpoint stops combine with [`Self::closed`].
    #[cfg(wasm_browser)]
    pub fn watch_addr(&self) -> impl n0_watcher::Watcher<Value = EndpointAddr> + use<> {
        // In browsers, there will never be any direct addresses, so we wait
        // for the home relay instead. This makes the `EndpointAddr` have *some* way
        // of connecting to us.
        let watch_relay = self.inner.home_relay();
        let endpoint_id = self.id();
        watch_relay.map(move |mut relays| {
            EndpointAddr::from_parts(endpoint_id, relays.into_iter().map(TransportAddr::Relay))
        })
    }

    /// A convenience method that waits for the endpoint to be considered "online".
    ///
    /// This currently means at least one relay server has completed its
    /// connection handshake (i.e. the endpoint is registered and reachable
    /// via that relay). Merely selecting a relay URL is not sufficient.
    ///
    /// If no relays are configured, this will pend forever.
    ///
    /// This has no timeout, so if that is needed, you need to wrap it in a
    /// timeout. We recommend using a timeout close to
    /// [`crate::NET_REPORT_TIMEOUT`]s, so you can be sure that at least one
    /// net report has been attempted.
    ///
    /// To understand if the endpoint has gone back "offline",
    /// you must use the [`Endpoint::watch_addr`] method, to
    /// get information on the current relay and direct address information.
    ///
    /// In the common case where the endpoint's configured relay servers are
    /// only accessible via a wide area network (WAN) connection, this method
    /// will await indefinitely when the endpoint has no WAN connection. If you're
    /// writing an app that's designed to work without a WAN connection, defer
    /// any calls to `online` as long as possible, or avoid calling `online`
    /// entirely.
    ///
    /// The online method does not interact with [`crate::address_lookup::AddressLookup`]
    /// services, which means that any Address Lookup that relies on a WAN
    /// connection is independent of the endpoint's online status.
    ///
    /// # Examples
    ///
    /// ```no_run
    /// # #[cfg(with_crypto_provider)]
    /// # {
    /// # #[tokio::main]
    /// # async fn main() -> n0_error::Result<()> {
    /// # use iroh::{Endpoint, endpoint::presets};
    /// // After this await returns, the endpoint is bound to a local socket.
    /// // It can be dialed, but almost certainly hasn't finished picking a
    /// // relay.
    /// let endpoint = Endpoint::bind(presets::N0).await?;
    ///
    /// // After this await returns we have a connection to at least one relay
    /// // and holepunching should work as expected.
    /// endpoint.online().await;
    /// # Ok(()) }
    /// # }
    /// ```
    pub async fn online(&self) {
        let mut watcher = self.inner.home_relay_status();
        let mut value = watcher.get();
        loop {
            if value.into_iter().any(|status| status.is_connected()) {
                return;
            }
            value = match watcher.updated().await {
                Ok(value) => value,
                Err(_disconnected) => {
                    std::future::pending::<()>().await;
                    break;
                }
            }
        }
    }

    /// Returns a [`Watcher`] over the connection status of the endpoint's home relays.
    ///
    /// The watched value has one entry per home relay whose URL is known,
    /// and is empty when no relays are configured or before the endpoint has
    /// selected a home relay from the list of configured relays.
    /// The watcher updates whenever any home relay's connection status changes.
    /// See [`RelayStatus`] for the information available on each entry.
    ///
    /// This may be used to observe connection failures to the home relay:
    /// [`RelayStatus::last_error`] reports the most recent error, and
    /// [`RelayStatus::auth_denied_reason`] singles out the case of the relay
    /// server denying the endpoint's authentication.
    ///
    /// The returned watcher only becomes disconnected once the last clone of
    /// the [`Endpoint`] is dropped. Closing the endpoint does not disconnect
    /// the watcher. To stop a task once the endpoint stops, combine with
    /// [`Self::closed`].
    pub fn home_relay_status(&self) -> impl Watcher<Value = Vec<RelayStatus>> + use<> {
        self.inner.home_relay_status()
    }

    /// Returns a [`Watcher`] for any net report runs from this [`Endpoint`].
    ///
    /// <div class="warning">
    ///
    /// This API is unstable and gated behind the `unstable-net-report` feature.
    /// It is not covered by semantic versioning guarantees and may change in any release
    /// without a major version bump.
    ///
    /// </div>
    ///
    /// A net report checks the network conditions of the [`Endpoint`], such as
    /// whether it is connected to the internet via IPv4 and/or IPv6, its NAT
    /// status, its latency to the relay servers, and its public addresses.
    ///
    /// The [`Endpoint`] continuously runs net reports to monitor if network
    /// conditions have changed. This [`Watcher`] will return the latest
    /// net report.
    ///
    /// When issuing the first call to this method the first report might
    /// still be underway, in this case the [`Watcher`] might not be initialized
    /// with [`Some`] value yet.  Once the net report has been successfully
    /// run, the [`Watcher`] will always return [`Some`] immediately, which
    /// is the most recently run net report.
    ///
    /// The returned watcher only becomes disconnected once the last clone of the [`Endpoint`]
    /// is dropped. Closing the endpoint does not disconnect the watcher. Thus, a stream created
    /// via [`Watcher::stream`] only terminates once the endpoint stops. If you want to stop a
    /// task once the endpoint stops combine with [`Self::closed`].
    ///
    /// # Examples
    ///
    /// To get the first report use [`Watcher::initialized`]:
    /// ```no_run
    /// # #[cfg(with_crypto_provider)]
    /// # {
    /// use iroh::{Endpoint, Watcher as _, endpoint::presets};
    ///
    /// # let rt = tokio::runtime::Builder::new_current_thread().enable_all().build().unwrap();
    /// # rt.block_on(async move {
    /// let ep = Endpoint::bind(presets::N0).await.unwrap();
    /// let _report = ep.net_report().initialized().await;
    /// # });
    /// # }
    /// ```
    #[cfg(feature = "unstable-net-report")]
    pub fn net_report(&self) -> impl Watcher<Value = Option<NetReport>> + use<> {
        self.inner.net_report()
    }

    /// Returns the local socket addresses on which the underlying sockets are bound.
    ///
    /// The [`Endpoint`] always binds on an IPv4 address and also tries to bind on an IPv6
    /// address if available.
    #[cfg(not(wasm_browser))]
    pub fn bound_sockets(&self) -> Vec<SocketAddr> {
        self.inner
            .local_addr()
            .into_iter()
            .filter_map(|addr| addr.into_socket_addr())
            .collect()
    }

    // # Methods for less common getters.
    //
    // Partially they return things passed into the builder.

    /// Returns the DNS resolver used in this [`Endpoint`].
    ///
    /// # Errors
    ///
    /// Returns an `EndpointError::Closed` error if the endpoint is closed.
    ///
    /// See [`Builder::dns_resolver`].
    #[cfg(not(wasm_browser))]
    pub fn dns_resolver(&self) -> Result<&DnsResolver, EndpointError> {
        if self.is_closed() {
            return Err(e!(EndpointError::Closed));
        }
        Ok(self.inner.dns_resolver())
    }

    /// Returns the [`rustls::ClientConfig`] used by the endpoint for connecting to external services.
    ///
    /// This might be useful for address lookup services or other functions
    /// that want to use the same trust anchors as iroh does for verifying the
    /// validity of TLS certificates presented by external services.
    ///
    /// Note that this TLS config is unrelated to how iroh validates the authenticity
    /// of iroh connections itself.
    ///
    /// The config is based on the trust anchors set via [`Builder::ca_tls_config`].
    pub fn tls_config(&self) -> &rustls::ClientConfig {
        &self.inner.tls_config
    }

    /// Returns the Address Lookup service, if configured.
    ///
    /// # Errors
    ///
    /// Returns a `EndpointError::Closed` error if the endpoint is closed.
    ///
    /// See [`Builder::address_lookup`].
    pub fn address_lookup(&self) -> Result<&AddressLookupServices, EndpointError> {
        if self.is_closed() {
            return Err(e!(EndpointError::Closed));
        }
        Ok(self.inner.address_lookup())
    }

    /// Returns metrics collected for this endpoint.
    ///
    /// The endpoint internally collects various metrics about its operation.
    /// The returned [`EndpointMetrics`] struct contains all of these metrics.
    ///
    /// You can access individual metrics directly by using the public fields:
    /// ```rust
    /// # use std::collections::BTreeMap;
    /// # use iroh::endpoint::{Endpoint, presets};
    /// # async fn wrapper() -> n0_error::Result<()> {
    /// let endpoint = Endpoint::bind(presets::N0).await?;
    /// assert_eq!(endpoint.metrics().socket.recv_datagrams.get(), 0);
    /// # Ok(())
    /// # }
    /// ```
    ///
    /// [`EndpointMetrics`] implements [`MetricsGroupSet`], and each field
    /// implements [`MetricsGroup`]. These traits provide methods to iterate over
    /// the groups in the set, and over the individual metrics in each group, without having
    /// to access each field manually. With these methods, it is straightforward to collect
    /// all metrics into a map or push their values to a metrics collector.
    ///
    /// For example, the following snippet collects all metrics into a map:
    /// ```rust
    /// # use std::collections::BTreeMap;
    /// # use iroh_metrics::{Metric, MetricsGroup, MetricValue, MetricsGroupSet};
    /// # use iroh::endpoint::{Endpoint, presets};
    /// # async fn wrapper() -> n0_error::Result<()> {
    /// let endpoint = Endpoint::bind(presets::N0).await?;
    /// let metrics: BTreeMap<String, MetricValue> = endpoint
    ///     .metrics()
    ///     .iter()
    ///     .map(|(group, metric)| {
    ///         let name = [group, metric.name()].join(":");
    ///         (name, metric.value())
    ///     })
    ///     .collect();
    ///
    /// assert_eq!(metrics["socket:recv_datagrams"], MetricValue::Counter(0));
    /// # Ok(())
    /// # }
    /// ```
    ///
    /// The metrics can also be encoded into the OpenMetrics text format, as used by Prometheus.
    /// To do so, use the [`iroh_metrics::Registry`], add the endpoint metrics to the
    /// registry with [`Registry::register_all`], and encode the metrics to a string with
    /// [`encode_openmetrics_to_string`]:
    /// ```rust
    /// # use iroh_metrics::{Registry, MetricsSource};
    /// # use iroh::endpoint::{Endpoint, presets};
    /// # async fn wrapper() -> n0_error::Result<()> {
    /// let endpoint = Endpoint::bind(presets::N0).await?;
    /// let mut registry = Registry::default();
    /// registry.register_all(endpoint.metrics());
    /// let s = registry.encode_openmetrics_to_string()?;
    /// assert!(s.contains(r#"TYPE socket_recv_datagrams counter"#));
    /// assert!(s.contains(r#"socket_recv_datagrams_total 0"#));
    /// # Ok(())
    /// # }
    /// ```
    ///
    /// Through a registry, you can also add labels or prefixes to metrics with
    /// [`Registry::sub_registry_with_label`] or [`Registry::sub_registry_with_prefix`].
    /// Furthermore, [`iroh_metrics::service`] provides functions to easily start services
    /// to serve the metrics with a HTTP server, dump them to a file, or push them
    /// to a Prometheus gateway.
    ///
    /// For example, the following snippet launches an HTTP server that serves the metrics in the
    /// OpenMetrics text format:
    /// ```no_run
    /// # use std::sync::{Arc, RwLock};
    /// # use iroh_metrics::{Registry, MetricsSource};
    /// # use iroh::endpoint::{Endpoint, presets};
    /// # use n0_error::{StackResultExt, StdResultExt};
    /// # async fn wrapper() -> n0_error::Result<()> {
    /// // Create a registry, wrapped in a read-write lock so that we can register and serve
    /// // the metrics independently.
    /// let registry = Arc::new(RwLock::new(Registry::default()));
    /// // Spawn an OpenMetrics HTTP server backed by the registry.
    /// let addr = "0.0.0.0:9100".parse().unwrap();
    /// let metrics_server = iroh_metrics::service::MetricsServer::spawn(addr, registry.clone())
    ///     .await
    ///     .std_context("spawn metrics server")?;
    ///
    /// // Spawn an endpoint and add the metrics to the registry.
    /// let endpoint = Endpoint::bind(presets::N0).await?;
    /// registry.write().unwrap().register_all(endpoint.metrics());
    ///
    /// // Fetch the metrics via HTTP.
    /// let res = reqwest::get("http://localhost:9100/metrics")
    ///     .await
    ///     .std_context("get")?
    ///     .text()
    ///     .await
    ///     .std_context("text")?;
    ///
    /// assert!(res.contains(r#"TYPE socket_recv_datagrams counter"#));
    /// assert!(res.contains(r#"socket_recv_datagrams_total 0"#));
    /// # metrics_server.shutdown().await;
    /// # Ok(())
    /// # }
    /// ```
    ///
    /// [`Registry`]: iroh_metrics::Registry
    /// [`Registry::register_all`]: iroh_metrics::Registry::register_all
    /// [`Registry::sub_registry_with_label`]: iroh_metrics::Registry::sub_registry_with_label
    /// [`Registry::sub_registry_with_prefix`]: iroh_metrics::Registry::sub_registry_with_prefix
    /// [`encode_openmetrics_to_string`]: iroh_metrics::MetricsSource::encode_openmetrics_to_string
    /// [`MetricsGroup`]: iroh_metrics::MetricsGroup
    /// [`MetricsGroupSet`]: iroh_metrics::MetricsGroupSet
    #[cfg(feature = "metrics")]
    pub fn metrics(&self) -> &EndpointMetrics {
        &self.inner.metrics
    }

    /// Returns addressing information about a recently used remote endpoint.
    ///
    /// The returned [`RemoteInfo`] contains a list of all transport addresses for the remote
    /// that we know about. This is a snapshot in time and not a watcher.
    ///
    /// Returns `None` if the endpoint doesn't have information about the remote or if the endpoint is closed.
    /// When remote endpoints are no longer used, our endpoint will keep information around
    /// for a little while, and then drop it. Afterwards, this will return `None`.
    pub async fn remote_info(&self, endpoint_id: EndpointId) -> Option<RemoteInfo> {
        if self.is_closed() {
            return None;
        }
        self.inner.remote_info(endpoint_id).await
    }

    // # Methods for less common state updates.

    /// Notifies the system of potential network changes.
    ///
    /// On many systems iroh is able to detect network changes by itself, however
    /// some systems like android do not expose this functionality to native code.
    /// Android does however provide this functionality to Java code.  This
    /// function allows for notifying iroh of any potential network changes like
    /// this.
    ///
    /// Even when the network did not change, or iroh was already able to detect
    /// the network change itself, there is no harm in calling this function.
    ///
    /// If the endpoint is closed, this method will log a warning and ignore the request.
    pub async fn network_change(&self) {
        if self.is_closed() {
            debug!("Attempting to notify a closed endpoint about a network change. Ignoring.");
            return;
        }
        self.inner.network_change().await;
    }

    // # Methods to update internal state.

    /// Sets the initial user-defined data to be published in Address Lookups for this endpoint.
    ///
    /// If the user-defined data passed to this function is different to the previous one,
    /// the endpoint will republish its endpoint info to the configured Address Lookups.
    ///
    /// See also [`Builder::user_data_for_address_lookup`] for setting an initial value when
    /// building the endpoint.
    ///
    /// If the endpoint is closed, this method will log a warning and ignore the
    /// request.
    pub fn set_user_data_for_address_lookup(&self, user_data: Option<UserData>) {
        if self.is_closed() {
            warn!("Attempting to set user data for a closed endpoint. Ignoring.");
            return;
        }
        self.inner.set_user_data_for_address_lookup(user_data);
    }

    // # Methods for terminating the endpoint.

    /// Closes the QUIC endpoint and the socket.
    ///
    /// This will close any remaining open [`Connection`]s with an error code
    /// of `0` and an empty reason.  Though it is best practice to close those
    /// explicitly before with a custom error code and reason.
    ///
    /// It will then make a best effort to wait for all close notifications to be
    /// acknowledged by the peers, re-transmitting them if needed. This ensures the
    /// peers are aware of the closed connections instead of having to wait for a timeout
    /// on the connection. Once all connections are closed or timed out, the future
    /// finishes.
    ///
    /// The maximum time-out that this future will wait for depends on QUIC transport
    /// configurations of non-drained connections at the time of calling, and their current
    /// estimates of round trip time. With default parameters and a conservative estimate
    /// of round trip time, this call's future should take 3 seconds to resolve in cases of
    /// bad connectivity or failed connections. In the usual case, this call's future should
    /// return much more quickly.
    ///
    /// It is highly recommended you *do* wait for this close call to finish, if possible.
    /// Not doing so will make connections that were still open while closing the endpoint
    /// time out on the remote end. Thus remote ends will assume connections to have failed
    /// even if all application data was transmitted successfully.
    ///
    /// Note: Someone used to closing TCP sockets might wonder why it is necessary to wait
    /// for timeouts when closing QUIC endpoints, while they don't have to do this for TCP
    /// sockets. This is due to QUIC and its acknowledgments being implemented in user-land,
    /// while TCP sockets usually get closed and drained by the operating system in the
    /// kernel during the "Time-Wait" period of the TCP socket.
    ///
    /// Be aware however that the underlying UDP sockets are only closed once all clones of
    /// the respective [`Endpoint`] are dropped.
    pub async fn close(&self) {
        self.inner.close().await;
    }

    /// Check if this endpoint is still alive, or already closed.
    pub fn is_closed(&self) -> bool {
        self.inner.is_closed()
    }

    /// Returns a future that resolves once the endpoint closes.
    ///
    /// The returned future does not contain a clone or reference to the [`Endpoint`],
    /// so keeping the returned future alive does not prevent the endpoint from being dropped.
    ///
    /// To run a task and stop it once the endpoint closes, you can use
    /// [`EndpointClosed::run_until`]:
    /// ```no_run
    /// # #[cfg(with_crypto_provider)]
    /// # {
    /// # use iroh::endpoint::{Endpoint, presets};
    /// # async fn wrapper() -> n0_error::Result<()> {
    /// let endpoint = Endpoint::bind(presets::N0).await?;
    /// tokio::spawn(endpoint.closed().run_until(async move {
    ///     // the future will be aborted once the endpoint closes.
    /// }));
    /// # Ok(())
    /// # }
    /// # }
    /// ```
    pub fn closed(&self) -> EndpointClosed {
        EndpointClosed {
            inner: self.inner.closed(),
        }
    }

    /// Create a [`ServerConfigBuilder`] for this endpoint that includes the given alpns.
    ///
    /// Use the [`ServerConfigBuilder`] to customize the [`ServerConfig`] connection configuration
    /// for a connection accepted using the [`Incoming::accept_with`] method.
    pub fn create_server_config_builder(&self, alpns: Vec<Vec<u8>>) -> ServerConfigBuilder {
        let inner = self.inner.static_config.create_server_config(alpns);
        ServerConfigBuilder::new(inner, self.inner.static_config.transport_config.clone())
    }

    // # Remaining private methods

    /// Translates a possible IP-mapped [`SocketAddr`] into a transport address.
    pub(crate) fn to_transport_addr(
        &self,
        addr: SocketAddr,
    ) -> Option<crate::socket::transports::Addr> {
        self.inner.to_transport_addr(addr)
    }
}

/// Options for the [`Endpoint::connect_with_opts`] function.
#[derive(Default, Debug, Clone)]
pub struct ConnectOptions {
    transport_config: Option<QuicTransportConfig>,
    additional_alpns: Vec<Vec<u8>>,
}

impl ConnectOptions {
    /// Initializes new connection options.
    ///
    /// By default, the connection will use the same options
    /// as [`Endpoint::connect`], e.g. a default [`QuicTransportConfig`].
    pub fn new() -> Self {
        Self::default()
    }

    /// Sets the QUIC transport config options for this connection.
    pub fn with_transport_config(mut self, transport_config: QuicTransportConfig) -> Self {
        self.transport_config = Some(transport_config);
        self
    }

    /// Sets [ALPN] identifiers that should be signaled as supported on connection, *in
    /// addition* to the main [ALPN] identifier used in [`Endpoint::connect_with_opts`].
    ///
    /// This allows connecting to servers that may only support older versions of your
    /// protocol. In this case, you would add the older [ALPN] identifiers with this
    /// function.
    ///
    /// You'll know the final negotiated [ALPN] identifier once your connection was
    /// established using [`Connection::alpn`], or even slightly earlier in the
    /// handshake by using [`Connecting::alpn`].
    /// The negotiated [ALPN] identifier may be any of the [ALPN] identifiers in this
    /// list or the main [ALPN] used in [`Endpoint::connect_with_opts`].
    ///
    /// The [ALPN] identifier order on the connect side doesn't matter, since it's the
    /// accept side that determines the protocol.
    ///
    /// For setting the supported [ALPN] identifiers on the accept side, see the endpoint
    /// builder's [`Builder::alpns`] function.
    ///
    /// [ALPN]: https://en.wikipedia.org/wiki/Application-Layer_Protocol_Negotiation
    pub fn with_additional_alpns(mut self, alpns: Vec<Vec<u8>>) -> Self {
        self.additional_alpns = alpns;
        self
    }
}

/// Future returned from [`Endpoint::closed`].
#[derive(derive_more::Debug)]
#[pin_project]
#[debug("EndpointClosed")]
pub struct EndpointClosed {
    #[pin]
    inner: WaitForCancellationFutureOwned,
}

impl Future for EndpointClosed {
    type Output = ();

    fn poll(
        self: Pin<&mut Self>,
        cx: &mut std::task::Context<'_>,
    ) -> std::task::Poll<Self::Output> {
        let this = self.project();
        this.inner.poll(cx)
    }
}

impl EndpointClosed {
    /// Runs a future to completion, or until the [`Endpoint`] is closed.
    ///
    /// Returns the output of `fut` if it completes before the endpoint closes,
    /// or `None` otherwise.
    pub async fn run_until<F: Future>(self, fut: F) -> Option<F::Output> {
        n0_future::future::or(async { Some(fut.await) }, async {
            self.await;
            None
        })
        .await
    }
}

/// Read a proxy url from the environment, in this order
///
/// - `HTTP_PROXY`
/// - `http_proxy`
/// - `HTTPS_PROXY`
/// - `https_proxy`
fn proxy_url_from_env() -> Option<Url> {
    if let Some(url) = std::env::var("HTTP_PROXY")
        .ok()
        .and_then(|s| s.parse::<Url>().ok())
    {
        if is_cgi() {
            warn!("HTTP_PROXY environment variable ignored in CGI");
        } else {
            return Some(url);
        }
    }
    if let Some(url) = std::env::var("http_proxy")
        .ok()
        .and_then(|s| s.parse::<Url>().ok())
    {
        return Some(url);
    }
    if let Some(url) = std::env::var("HTTPS_PROXY")
        .ok()
        .and_then(|s| s.parse::<Url>().ok())
    {
        return Some(url);
    }
    if let Some(url) = std::env::var("https_proxy")
        .ok()
        .and_then(|s| s.parse::<Url>().ok())
    {
        return Some(url);
    }

    None
}

/// Connection status of a single home relay.
///
/// Observed via [`Endpoint::home_relay_status`].
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RelayStatus {
    url: RelayUrl,
    state: RelayConnectionState,
}

impl RelayStatus {
    pub(crate) fn new(url: RelayUrl, state: RelayConnectionState) -> Self {
        Self { url, state }
    }

    /// Returns the URL of the home relay.
    pub fn url(&self) -> &RelayUrl {
        &self.url
    }

    /// Returns `true` if the endpoint is connected to the relay.
    pub fn is_connected(&self) -> bool {
        self.state.is_connected()
    }

    /// Returns the most recent connection error.
    ///
    /// Returns `None` when the relay is connected, or when the endpoint has
    /// not yet observed a failed connection attempt.
    ///
    /// The error is meant to be logged or displayed, not matched on: it is an
    /// [`AnyError`] wrapping a chain of private error types, none of which are
    /// covered by semver guarantees. Use [`Self::auth_denied_reason`] to
    /// distinguish the one failure that usually calls for a different reaction
    /// than retrying.
    pub fn last_error(&self) -> Option<&AnyError> {
        self.state.last_failure().map(RelayConnectionFailure::error)
    }

    /// Returns the reason if the relay server denied our authentication.
    ///
    /// Unlike most connection failures, this one will not usually resolve
    /// itself. The endpoint keeps retrying with a backoff, but it presents the
    /// same credentials every time, so unless the relay's access policy
    /// changes it will keep being denied and [`Endpoint::online`] will never
    /// resolve. An application that configures a relay auth token should
    /// surface this to the user rather than wait to come online.
    ///
    /// The returned string is the reason reported by the relay server. It is
    /// meant to be human-readable, don't attempt to match on it.
    ///
    /// Returns `None` when the relay is connected, when no connection attempt
    /// has failed yet, or when the last failure had another cause.
    ///
    /// ```no_run
    /// # async fn wrapper() -> n0_error::Result<()> {
    /// # #[cfg(with_crypto_provider)]
    /// # {
    /// use iroh::{Endpoint, Watcher, endpoint::presets};
    /// use n0_future::StreamExt;
    ///
    /// let endpoint = Endpoint::builder(presets::Minimal).bind().await?;
    /// let mut status = endpoint.home_relay_status().stream();
    /// while let Some(relays) = status.next().await {
    ///     for relay in relays {
    ///         if let Some(reason) = relay.auth_denied_reason() {
    ///             println!("{}: authentication denied ({reason})", relay.url());
    ///         }
    ///     }
    /// }
    /// # }
    /// # Ok(()) }
    /// ```
    pub fn auth_denied_reason(&self) -> Option<&str> {
        self.state
            .last_failure()
            .and_then(RelayConnectionFailure::auth_denied_reason)
    }
}

/// Configuration of the relay servers for an [`Endpoint`].
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum RelayMode {
    /// Disable relay servers completely.
    /// This means that neither listening nor dialing relays will be available.
    Disabled,
    /// Use the default relay map, with production relay servers from n0.
    ///
    /// See [`crate::defaults::prod`] for the severs used.
    Default,
    /// Use the staging relay servers from n0.
    Staging,
    /// Use a custom relay map.
    Custom(RelayMap),
}

impl RelayMode {
    /// Returns the relay map for this mode.
    pub fn relay_map(&self) -> RelayMap {
        match self {
            RelayMode::Disabled => RelayMap::empty(),
            RelayMode::Default => crate::defaults::prod::default_relay_map(),
            RelayMode::Staging => crate::defaults::staging::default_relay_map(),
            RelayMode::Custom(relay_map) => relay_map.clone(),
        }
    }

    /// Create a custom relay mode from a list of [`RelayUrl`]s.
    ///
    /// # Example
    ///
    /// ```
    /// # fn main() -> n0_error::Result<()> {
    /// # use iroh::RelayMode;
    /// RelayMode::custom([
    ///     "https://use1-1.relay.n0.iroh.link.".parse()?,
    ///     "https://euw-1.relay.n0.iroh.link.".parse()?,
    /// ]);
    /// # Ok(()) }
    /// ```
    pub fn custom(map: impl IntoIterator<Item = RelayUrl>) -> Self {
        let m = RelayMap::from_iter(map);
        Self::Custom(m)
    }
}

/// Environment variable to force the use of staging relays.
pub const ENV_FORCE_STAGING_RELAYS: &str = "IROH_FORCE_STAGING_RELAYS";

/// Returns `true` if the use of staging relays is forced.
pub fn force_staging_infra() -> bool {
    matches!(std::env::var(ENV_FORCE_STAGING_RELAYS), Ok(value) if !value.is_empty())
}

/// Returns the default relay mode.
///
/// If the `IROH_FORCE_STAGING_RELAYS` environment variable is non empty, it will return `RelayMode::Staging`.
/// Otherwise, it will return `RelayMode::Default`.
pub fn default_relay_mode() -> RelayMode {
    // Use staging in testing
    match force_staging_infra() {
        true => RelayMode::Staging,
        false => RelayMode::Default,
    }
}

/// Check if we are being executed in a CGI context.
///
/// If so, a malicious client can send the `Proxy:` header, and it will
/// be in the `HTTP_PROXY` env var. So we don't use it :)
fn is_cgi() -> bool {
    std::env::var_os("REQUEST_METHOD").is_some()
}
