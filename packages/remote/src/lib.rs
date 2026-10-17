//! Encrypted, explicitly paired device control over the shared application contract.
//!
//! Hosts supply identity paths, trust storage, relay/discovery choices, and the single business
//! handler. Dropping clients or stopping this network endpoint never shuts down that handler.
mod endpoint;
mod error;
mod identity;
mod ledger;
mod paths;
mod protocol;
mod wire;
pub use endpoint::{CallError, CallFailure, Handler, HandlerFuture, RemoteClient, RemoteEndpoint};
pub use error::{RemoteError, Result};
pub use identity::{DeviceIdentity, DeviceTrust, FileTrustStore, MemoryTrustStore, TrustedDevice};
pub use paths::{ConnectionPath, ConnectionStatus, PathKind, PathObserver};
pub use protocol::{
    ClientSession, DiscoveryConfig, EndpointConfig, EndpointDescriptor, Invitation,
    MAX_FRAME_BYTES, PROTOCOL_VERSION, PathPolicy, RelayConfig, RequestContext, RequestId,
    RequestStatus,
};
