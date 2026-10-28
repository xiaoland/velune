//! Encrypted, explicitly paired device control over the shared application contract.
//!
//! Hosts supply identity paths, trust storage, relay/discovery choices, and the single business
//! handler. Dropping clients or stopping this network endpoint never shuts down that handler.
mod client;
mod controller;
mod endpoint;
mod error;
mod identity;
mod ledger;
mod lookup;
mod network;
mod pairing;
mod paths;
#[cfg(feature = "diagnostics")]
mod probe;
mod protocol;
mod service;
mod settings;
mod store;
mod wire;
pub(crate) use client::RemoteClient;
pub use client::{CallError, CallFailure};
pub use controller::*;
pub use error::{RemoteError, Result};
pub use identity::{DeviceIdentity, validate_device_storage_paths};
#[cfg(target_os = "android")]
pub use iroh::dns::install_android_jni_context;
pub use network::{NetworkChangeReason, PlatformNetwork, PlatformNetworkSnapshot};
pub use pairing::{PairingOffer, PairingPermission, PairingPhase, PairingStatus};
#[cfg(feature = "diagnostics")]
pub use paths::{ConnectionDiagnostics, PathDiagnostic, monotonic_microseconds};
pub use paths::{
    ConnectionPath, ConnectionStatus, PathAddressFamily, PathKind, PathObservation, PathObserver,
    PathSource,
};
#[cfg(feature = "diagnostics")]
pub use probe::*;
pub use protocol::{
    ClientSession, DiscoveryConfig, EndpointConfig, EndpointDescriptor, MAX_FRAME_BYTES,
    PROTOCOL_VERSION, PathPolicy, RelayConfig, RequestContext, RequestId, RequestStatus,
};
pub use service::{Handler, HandlerFuture, RemoteAccessService};
pub use settings::*;
pub use store::*;

uniffi::setup_scaffolding!();

/// Addresses cross the foreign boundary as text.
pub type SocketAddr = std::net::SocketAddr;
uniffi::custom_type!(SocketAddr, String, {
    remote,
    lower: |address| address.to_string(),
    try_lift: |text| Ok(text.parse()?),
});
#[cfg(feature = "diagnostics")]
pub type Ipv4Addr = std::net::Ipv4Addr;
#[cfg(feature = "diagnostics")]
uniffi::custom_type!(Ipv4Addr, String, {
    remote,
    lower: |address| address.to_string(),
    try_lift: |text| Ok(text.parse()?),
});
