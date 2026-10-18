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
mod network;
mod pairing;
mod paths;
#[cfg(feature = "diagnostics")]
mod probe;
mod protocol;
mod service;
mod store;
mod wire;
pub(crate) use client::RemoteClient;
pub use client::{CallError, CallFailure};
pub use controller::*;
pub use error::{RemoteError, Result};
pub use identity::DeviceIdentity;
#[cfg(target_os = "android")]
pub use iroh::dns::install_android_jni_context;
pub use network::{NetworkChangeReason, PlatformNetwork, PlatformNetworkSnapshot};
pub use pairing::{PairingOffer, PairingPhase, PairingStatus};
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
pub use store::*;
