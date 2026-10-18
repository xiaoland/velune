//! Lookup the address of an Endpoint ID.
//!
//! To connect to an iroh endpoint a [`EndpointAddr`] is needed, which may contain a
//! [`RelayUrl`] or one or more *direct addresses* in addition to the [`EndpointId`].
//!
//! Since there is a conversion from [`EndpointId`] to [`EndpointAddr`], you can also
//! connect directly with a [`EndpointId`].
//!
//! For this to work however, the endpoint has to get the addressing information by
//! other means.
//!
//! [`AddressLookup`] is an automated system for an [`Endpoint`] to retrieve this addressing
//! information.  Each iroh endpoint will automatically publish their own addressing
//! information.  Usually this means publishing which [`RelayUrl`] to use for their
//! [`EndpointId`], but they could also publish their direct addresses.
//!
//! The [`AddressLookup`] trait is used to define an address lookup system.  This allows multiple
//! implementations to co-exist because there are many possible ways to implement this.
//! Each [`Endpoint`] can use the address lookup mechanisms most suitable to the application.
//! The [`Builder::address_lookup`] method is used to add an address lookup mechanism to an
//! [`Endpoint`].
//!
//! Each address lookup service receives the full set of transport addresses when publishing,
//! but may only publish a subset of them based on its own constraints.
//!
//! To control which addresses are published to a particular service, you can supply an
//! [`AddrFilter`] on its builder (e.g. [`PkarrPublisherBuilder::addr_filter`]).  The filter
//! receives the full set of addresses and returns an ordered [`Vec`], allowing you to both
//! remove addresses you don't want published and prioritize the ones you do. Each service
//! may apply additional filtering on top based on its own constraints, but will not publish
//! addresses outside of what the filter returns.  See each service's documentation for details.
//!
//! Some generally useful Address Lookup implementations are provided:
//!
//! - [`MemoryLookup`] which allows application to add and remove out-of-band addressing
//!   information.
//!
//! - The [`address_lookup::DnsAddressLookup`] which performs lookups via the standard DNS systems.  To publish
//!   to this DNS server a [`PkarrPublisher`] is needed.  [Number 0] runs a public instance
//!   of a [`PkarrPublisher`] with attached DNS server which is globally available and a
//!   reliable default choice.
//!
//! - The [`PkarrResolver`] which can perform lookups from designated [pkarr relay servers]
//!   using HTTP.
//!
//! mDNS-based and Mainline-DHT-based Address Lookup services live in
//! separate crates: [`iroh-mdns-address-lookup`] and
//! [`iroh-mainline-address-lookup`].
//!
//! [`iroh-mdns-address-lookup`]: https://docs.rs/iroh-mdns-address-lookup
//! [`iroh-mainline-address-lookup`]: https://docs.rs/iroh-mainline-address-lookup
//!
//! To use multiple Address Lookups simultaneously you can call [`Builder::address_lookup`].
//! This will use [`AddressLookupServices`] under the hood, which performs lookups to all
//! Address Lookup systems at the same time.
//!
//! [`Builder::address_lookup`] takes any type that implements [`AddressLookupBuilder`]. You can
//! implement that trait on a builder struct if your Address Lookup needs information
//! from the endpoint it is mounted on. After endpoint construction, your Address Lookup
//! is built by calling [`AddressLookupBuilder::into_address_lookup`], passing the finished [`Endpoint`] to your
//! builder.
//!
//! If your Address Lookup does not need any information from its endpoint, you can
//! pass the Address Lookup service directly to [`Builder::address_lookup`]: All types that
//! implement [`AddressLookup`] also have a blanket implementation of [`AddressLookupBuilder`].
//!
//! # Examples
//!
//! A very common setup is to enable DNS Address Lookup, which needs to be done in two parts as a
//! [`PkarrPublisher`] and [`address_lookup::DnsAddressLookup`]:
//!
//! ```no_run
//! # #[cfg(with_crypto_provider)]
//! # {
//! use iroh::{
//!     Endpoint, SecretKey,
//!     address_lookup::{self, AddrFilter, PkarrPublisher},
//!     endpoint::{RelayMode, presets},
//! };
//!
//! # async fn wrapper() -> n0_error::Result<()> {
//! let ep = Endpoint::builder(presets::Minimal)
//!     .addr_filter(AddrFilter::relay_only())
//!     .address_lookup(PkarrPublisher::n0_dns())
//!     .address_lookup(address_lookup::DnsAddressLookup::n0_dns())
//!     .bind()
//!     .await?;
//! # Ok(())
//! # }
//! # }
//! ```
//!
//! [`EndpointAddr`]: iroh_base::EndpointAddr
//! [`RelayUrl`]: crate::RelayUrl
//! [`Builder::address_lookup`]: crate::endpoint::Builder::address_lookup
//! [`address_lookup::DnsAddressLookup`]: crate::address_lookup::DnsAddressLookup
//! [Number 0]: https://n0.computer
//! [`PkarrResolver`]: pkarr::PkarrResolver
//! [`PkarrPublisher`]: pkarr::PkarrPublisher
//! [`PkarrPublisherBuilder::addr_filter`]: pkarr::PkarrPublisherBuilder::addr_filter
//! [pkarr relay servers]: https://pkarr.org/#servers
//! [`MemoryLookup`]: memory::MemoryLookup

use std::{
    borrow::{Borrow, Cow},
    pin::Pin,
    sync::{Arc, RwLock},
    task::{Poll, ready},
};

use iroh_base::{EndpointAddr, EndpointId};
pub use iroh_dns::{ParseError, endpoint_info::AddrFilter};
use n0_error::{AnyError, e, stack_error};
use n0_future::{MergeBounded, Stream, boxed::BoxStream};
use tracing::debug;

pub use crate::endpoint_info::{EndpointData, EndpointInfo, UserData};
use crate::{Endpoint, endpoint::EndpointError};

#[cfg(not(wasm_browser))]
pub mod dns;
pub mod memory;
mod metrics;
pub mod pkarr;

#[cfg(not(wasm_browser))]
pub use dns::*;
pub use memory::*;
pub use metrics::{Metrics, ServiceLabels};
pub use pkarr::*;

/// Trait for structs that can be converted into [`AddressLookup`]s.
///
/// This trait is implemented on builders for Address Lookup's. Any type that implements this
/// trait can be added as a Address Lookup in [`Builder::address_lookup`].
///
/// Any type that implements [`AddressLookup`] also implements [`AddressLookupBuilder`].
///
/// Iroh uses this trait to allow configuring the set of address lookup services on
/// the endpoint builder, while also providing them access to information about the
/// endpoint to [`AddressLookupBuilder::into_address_lookup`].
///
/// [`Builder::address_lookup`]: crate::endpoint::Builder::address_lookup
pub trait AddressLookupBuilder: Send + Sync + std::fmt::Debug + 'static {
    /// Turns this builder into a ready-to-use [`AddressLookup`].
    ///
    /// If an error is returned, building the endpoint will fail with this error.
    fn into_address_lookup(
        self,
        endpoint: &Endpoint,
    ) -> Result<impl AddressLookup, AddressLookupBuilderError>;
}

/// An [`AddressLookup`] wrapper that filters addresses before publishing.
#[derive(Debug, Clone)]
pub struct FilteredAddressLookup<T> {
    inner: T,
    filter: AddrFilter,
}

impl<T> FilteredAddressLookup<T> {
    /// Wraps an address lookup with an address filter.
    ///
    /// The filter allows to specify which addresses the address
    /// lookup service will publish.
    pub fn new(inner: T, filter: AddrFilter) -> Self {
        Self { inner, filter }
    }

    /// Removes the filter wrapper and returns the inner address lookup.
    pub fn into_inner(self) -> T {
        self.inner
    }
}

impl<T> AsRef<T> for FilteredAddressLookup<T> {
    fn as_ref(&self) -> &T {
        &self.inner
    }
}

impl<T: AddressLookup> AddressLookup for FilteredAddressLookup<T> {
    fn publish(&self, data: &EndpointData) {
        let data = data.apply_filter(&self.filter);
        self.inner.publish(data.borrow());
    }

    fn resolve(&self, endpoint_id: EndpointId) -> Option<BoxStream<Result<Item, Error>>> {
        self.inner.resolve(endpoint_id)
    }
}

/// Blanket no-op impl of `AddressLookupBuilder` for `T: AddressLookup`.
impl<T: AddressLookup> AddressLookupBuilder for T {
    fn into_address_lookup(
        self,
        _endpoint: &Endpoint,
    ) -> Result<impl AddressLookup, AddressLookupBuilderError> {
        Ok(self)
    }
}

/// Non-public dyn-compatible version of [`AddressLookupBuilder`], used in [`crate::endpoint::Builder`].
pub(crate) trait DynAddressLookupBuilder: Send + Sync + std::fmt::Debug + 'static {
    /// See [`AddressLookupBuilder::into_address_lookup`]
    fn into_address_lookup(
        self: Box<Self>,
        endpoint: &Endpoint,
    ) -> Result<Box<dyn AddressLookup>, AddressLookupBuilderError>;
}

impl<T: AddressLookupBuilder> DynAddressLookupBuilder for T {
    fn into_address_lookup(
        self: Box<Self>,
        endpoint: &Endpoint,
    ) -> Result<Box<dyn AddressLookup>, AddressLookupBuilderError> {
        let addr_lookup: Box<dyn AddressLookup> =
            Box::new(AddressLookupBuilder::into_address_lookup(*self, endpoint)?);
        Ok(addr_lookup)
    }
}

/// [`AddressLookupBuilder`] errors
#[allow(missing_docs)]
#[stack_error(derive, add_meta, from_sources, std_sources)]
#[non_exhaustive]
pub enum AddressLookupBuilderError {
    #[error("Service '{provenance}' error")]
    User {
        provenance: &'static str,
        source: AnyError,
    },
    #[error(transparent)]
    EndpointClosed { source: EndpointError },
}

impl AddressLookupBuilderError {
    /// Creates a new user error from an arbitrary error type.
    pub fn from_err<T: std::error::Error + Send + Sync + 'static>(
        provenance: &'static str,
        source: T,
    ) -> Self {
        e!(AddressLookupBuilderError::User {
            provenance,
            source: AnyError::from_std(source)
        })
    }

    /// Creates a new user error from an arbitrary boxed error type.
    pub fn from_err_box(
        provenance: &'static str,
        source: Box<dyn std::error::Error + Send + Sync + 'static>,
    ) -> Self {
        e!(AddressLookupBuilderError::User {
            provenance,
            source: AnyError::from_std_box(source)
        })
    }
}

/// [`AddressLookup`] errors
#[allow(missing_docs)]
#[stack_error(derive, add_meta)]
#[non_exhaustive]
#[derive(Clone)]
pub enum AddressLookupFailed {
    #[error("No address lookup configured")]
    NoServiceConfigured,
    #[error(
        "All address lookup services failed or produced no results{}",
        {
            let errors = errors.iter().map(|err| format!("{err:#}")).collect::<Vec<_>>();
            if !errors.is_empty() {
                format!("\n    {}", errors.join("\n    "))
            } else {
                String::new()
            }
        }
    )]
    NoResults { errors: Vec<Error> },
}

/// Error returned by address lookup services when failing to perform the lookup.
#[stack_error(derive, add_meta)]
#[error("Service '{provenance}' failed")]
#[derive(Clone)]
pub struct Error {
    provenance: &'static str,
    #[error(source)]
    source: Arc<AnyError>,
}

impl Error {
    /// Creates a new user error from an arbitrary error type.
    #[track_caller]
    pub fn from_err<T: std::error::Error + Send + Sync + 'static>(
        provenance: &'static str,
        source: T,
    ) -> Self {
        Self::from_err_any(provenance, AnyError::from_std(source))
    }

    /// Creates a new user error from an arbitrary boxed error type.
    #[track_caller]
    pub fn from_err_box(
        provenance: &'static str,
        source: Box<dyn std::error::Error + Send + Sync + 'static>,
    ) -> Self {
        Self::from_err_any(provenance, AnyError::from_std_box(source))
    }

    /// Creates a new user error from an arbitrary error type that can be converted into [`AnyError`].
    #[track_caller]
    pub fn from_err_any(provenance: &'static str, source: impl Into<AnyError>) -> Self {
        Self::new(provenance, Arc::new(source.into()))
    }
}

/// AddressLookup system for [`super::Endpoint`].
///
/// This trait defines publishing and resolving addressing information for a [`EndpointId`].
/// This enables connecting to other endpoints with only knowing the [`EndpointId`], by using this
/// [`AddressLookup`] system to look up the actual addressing information.  It is common for
/// implementations to require each endpoint to publish their own information before it can be
/// looked up by other endpoints.
///
/// The published addressing information can include both a [`RelayUrl`] and/or direct
/// addresses. See [`EndpointData`] for details.
///
/// To allow for Address Lookup, the [`super::Endpoint`] will call `publish` whenever
/// Address Lookup information changes. If an Address Lookup mechanism requires a periodic
/// refresh, it should start its own task.
///
/// [`RelayUrl`]: crate::RelayUrl
pub trait AddressLookup: std::fmt::Debug + Send + Sync + 'static {
    /// Publishes the given [`EndpointData`] to the Address Lookup mechanism.
    ///
    /// This is fire and forget, since the [`Endpoint`] can not wait for successful
    /// publishing. If publishing is async, the implementation should start its own task.
    ///
    /// This will be called from a tokio task, so it is safe to spawn new tasks.
    /// These tasks will be run on the runtime of the [`super::Endpoint`].
    fn publish(&self, _data: &EndpointData) {}

    /// Resolves the [`Item`] for the given [`EndpointId`].
    ///
    /// Once the returned [`BoxStream`] is dropped, the service should stop any pending
    /// work.
    fn resolve(&self, _endpoint_id: EndpointId) -> Option<BoxStream<Result<Item, Error>>> {
        None
    }
}

impl<T: AddressLookup> AddressLookup for Arc<T> {
    fn publish(&self, data: &EndpointData) {
        self.as_ref().publish(data);
    }

    fn resolve(&self, endpoint_id: EndpointId) -> Option<BoxStream<Result<Item, Error>>> {
        self.as_ref().resolve(endpoint_id)
    }
}

/// Address lookup results from [`AddressLookup`]s.
///
/// This is the item in the streams returned from [`AddressLookup::resolve`].
/// It contains the [`EndpointData`] about the resolved endpoint addresses,
/// and some additional metadata about the address lookup system.
///
/// This struct derefs to [`EndpointData`], so you can access the methods from [`EndpointData`]
/// directly from [`Item`].
#[derive(Debug, Clone, Eq, PartialEq)]
pub struct Item {
    /// The endpoint info for the endpoint, as discovered by the Address Lookup.
    endpoint_info: EndpointInfo,
    /// A static string to identify the Address Lookup source.
    ///
    /// Should be uniform per Address Lookup.
    provenance: &'static str,
    /// Optional timestamp when this endpoint address info was last updated.
    ///
    /// Must be microseconds since the unix epoch.
    // TODO(ramfox): this is currently unused. As we develop more `AddressLookup`s, we may discover that we do not need this. It is only truly relevant when comparing `relay_urls`, since we can attempt to dial any number of socket addresses, but expect each endpoint to have one "home relay" that we will attempt to contact them on. This means we would need some way to determine which relay url to choose between, if more than one relay url is reported.
    last_updated: Option<u64>,
}

impl Item {
    /// Creates a new [`Item`] from a [`EndpointInfo`].
    pub fn new(
        endpoint_info: EndpointInfo,
        provenance: &'static str,
        last_updated: Option<u64>,
    ) -> Self {
        Self {
            endpoint_info,
            provenance,
            last_updated,
        }
    }

    /// Returns the endpoint id of the discovered endpoint.
    pub fn endpoint_id(&self) -> EndpointId {
        self.endpoint_info.endpoint_id
    }

    /// Returns the [`EndpointInfo`] for the discovered endpoint.
    pub fn endpoint_info(&self) -> &EndpointInfo {
        &self.endpoint_info
    }

    /// Returns the provenance of this Address Lookup item.
    ///
    /// The provenance is a static string which identifies the Address Lookup service that produced
    /// this item.
    pub fn provenance(&self) -> &'static str {
        self.provenance
    }

    /// Returns the optional timestamp when this endpoint info was last updated.
    ///
    /// The value is microseconds since the unix epoch.
    pub fn last_updated(&self) -> Option<u64> {
        self.last_updated
    }

    /// Returns an [`EndpointAddr`] by cloning the needed fields.
    pub fn to_endpoint_addr(&self) -> EndpointAddr {
        self.endpoint_info.to_endpoint_addr()
    }

    /// Converts into an [`EndpointAddr`] without cloning.
    pub fn into_endpoint_addr(self) -> EndpointAddr {
        self.endpoint_info.into_endpoint_addr()
    }

    /// Returns any user-defined data.
    pub fn user_data(&self) -> Option<UserData> {
        self.endpoint_info().data.user_data().cloned()
    }
}

impl std::ops::Deref for Item {
    type Target = EndpointData;
    fn deref(&self) -> &Self::Target {
        &self.endpoint_info.data
    }
}

impl From<Item> for EndpointInfo {
    fn from(item: Item) -> Self {
        item.endpoint_info
    }
}

/// Registry for address lookup services for an [`Endpoint`].
///
/// The endpoint will use this registry both for publishing its own [`EndpointInfo`]
/// and for resolving addresses of remote endpoints.
///
/// See [`AddressLookup`] and [`Self::resolve`] for details.
///
/// [`Endpoint`]: crate::Endpoint
#[derive(Debug, Default, Clone)]
pub struct AddressLookupServices {
    services: Arc<RwLock<Vec<Box<dyn AddressLookup>>>>,
    /// The data last published, used to publish when adding a new service.
    last_data: Arc<RwLock<Option<EndpointData>>>,
    /// Optional filter applied to all data before publishing to any service.
    addr_filter: Arc<RwLock<Option<AddrFilter>>>,
    /// Metrics for lookup outcomes.
    metrics: Arc<Metrics>,
}

impl AddressLookupServices {
    /// Creates a registry recording lookup outcomes in `metrics`.
    pub(crate) fn with_metrics(metrics: Arc<Metrics>) -> Self {
        Self {
            metrics,
            ..Default::default()
        }
    }

    /// Sets the address filter applied before publishing to any service.
    ///
    /// When set, all address data is filtered once before being distributed
    /// to the individual address lookup services. This ensures consistent
    /// filtering regardless of how many services are configured.
    pub fn set_addr_filter(&self, filter: AddrFilter) {
        *self.addr_filter.write().expect("poisoned") = Some(filter);
    }

    /// Adds an [`AddressLookup`] service.
    ///
    /// If there is historical Address Lookup data, it will be published immediately on this service.
    pub fn add(&self, service: impl AddressLookup + 'static) {
        self.add_boxed(Box::new(service))
    }

    /// Adds an already `Box`ed [`AddressLookup`] service.
    ///
    /// If there is historical Address Lookup data, it will be published immediately on this service.
    pub fn add_boxed(&self, service: Box<dyn AddressLookup>) {
        {
            let data = self.last_data.read().expect("poisoned");
            if let Some(data) = &*data {
                service.publish(data)
            }
        }
        self.services.write().expect("poisoned").push(service);
    }

    /// Are there any services configured?
    pub fn is_empty(&self) -> bool {
        self.services.read().expect("poisoned").is_empty()
    }

    /// Returns the number of services configured.
    pub fn len(&self) -> usize {
        self.services.read().expect("poisoned").len()
    }

    /// Removes all configured services.
    pub fn clear(&self) {
        let mut services = self.services.write().expect("poisoned");
        services.clear();
    }

    /// Publish endpoint data on all configured services.
    pub(crate) fn publish(&self, data: &EndpointData) {
        let data = match &*self.addr_filter.read().expect("poisoned") {
            Some(filter) => data.apply_filter(filter),
            None => Cow::Borrowed(data),
        };
        let services = self.services.read().expect("poisoned");
        for service in &*services {
            service.publish(&data);
        }

        self.last_data
            .write()
            .expect("poisoned")
            .replace(data.into_owned());
    }

    /// Resolves the addressing information for an [`EndpointId`] across all configured services.
    ///
    /// All services are queried concurrently and their results are merged into a
    /// single stream. Each [`Item`] is yielded as `Ok(Ok(item))` as soon as it
    /// is produced, allowing the caller to act on the first usable address
    /// while slower services are still working.
    ///
    /// Errors from individual services are yielded inline as `Ok(Err(error))`
    /// and do not terminate the stream: a single failing service must not hide
    /// results from others still in flight. Inline errors are also buffered;
    /// if all services finish without yielding any [`Item`], the stream ends
    /// with a single [`AddressLookupFailed::NoResults`] carrying the buffered
    /// errors. If at least one [`Item`] was yielded, buffered errors are
    /// discarded and the stream ends with `None`.
    ///
    /// If no services are configured, the stream yields a single
    /// [`AddressLookupFailed::NoServiceConfigured`] error and then ends.
    ///
    /// Dropping the returned stream signals all underlying services to stop any
    /// pending work, as documented on [`AddressLookup::resolve`].
    pub fn resolve(
        &self,
        endpoint_id: EndpointId,
    ) -> impl Stream<Item = Result<Result<Item, Error>, AddressLookupFailed>> + use<> {
        self.metrics.lookups.inc();
        let services = self.services.read().expect("poisoned");
        if services.is_empty() {
            AddressLookupStream::empty(self.metrics.clone())
        } else {
            let streams = services
                .iter()
                .filter_map(|service| service.resolve(endpoint_id));
            AddressLookupStream::new(streams, self.metrics.clone())
        }
    }
}

/// Stream returned by [`AddressLookupServices::resolve`].
///
/// Merges the per-service result streams. Yields each successful item as
/// `Ok(Ok(item))` as soon as it is produced, and each per-service error
/// inline as `Ok(Err(error))`, so a single failing service cannot hide
/// results from others still in flight. Inline errors are also buffered.
///
/// Once all services are done, the stream ends with `None` if at least one
/// item was yielded; otherwise it yields a single
/// [`AddressLookupFailed::NoResults`] carrying all buffered errors, then ends.
///
/// If no services are configured, the stream yields a single
/// [`AddressLookupFailed::NoServiceConfigured`] error, then ends.
struct AddressLookupStream {
    streams: Option<MergeBounded<BoxStream<Result<Item, Error>>>>,
    errors: Vec<Error>,
    did_emit: bool,
    closed: bool,
    metrics: Arc<Metrics>,
}

impl AddressLookupStream {
    fn empty(metrics: Arc<Metrics>) -> Self {
        Self {
            streams: None,
            errors: Vec::new(),
            did_emit: false,
            closed: false,
            metrics,
        }
    }

    fn new(
        streams: impl Iterator<Item = BoxStream<Result<Item, Error>>>,
        metrics: Arc<Metrics>,
    ) -> Self {
        Self {
            streams: Some(MergeBounded::from_iter(streams)),
            errors: Vec::new(),
            did_emit: false,
            closed: false,
            metrics,
        }
    }
}

impl Stream for AddressLookupStream {
    type Item = Result<Result<Item, Error>, AddressLookupFailed>;

    fn poll_next(
        self: Pin<&mut Self>,
        cx: &mut std::task::Context<'_>,
    ) -> Poll<Option<Self::Item>> {
        let this = self.get_mut();
        if this.closed {
            return Poll::Ready(None);
        }
        let mut inner = match this.streams.as_mut() {
            Some(inner) => inner,
            None => {
                this.closed = true;
                this.metrics.lookups_failed.inc();
                return Poll::Ready(Some(Err(e!(AddressLookupFailed::NoServiceConfigured))));
            }
        };
        let item = match ready!(Pin::new(&mut inner).poll_next(cx)) {
            Some(Ok(item)) => {
                this.did_emit = true;
                this.metrics
                    .service_results
                    .get_or_create(&ServiceLabels::new(item.provenance()))
                    .inc();
                Some(Ok(Ok(item)))
            }
            Some(Err(error)) => {
                debug!("address lookup error: {error:#}");
                this.metrics
                    .service_errors
                    .get_or_create(&ServiceLabels::new(error.provenance))
                    .inc();
                this.errors.push(error.clone());
                Some(Ok(Err(error)))
            }
            None => {
                this.closed = true;
                if !this.did_emit {
                    this.metrics.lookups_failed.inc();
                    let errors = std::mem::take(&mut this.errors);
                    Some(Err(e!(AddressLookupFailed::NoResults { errors })))
                } else {
                    None
                }
            }
        };
        Poll::Ready(item)
    }
}
