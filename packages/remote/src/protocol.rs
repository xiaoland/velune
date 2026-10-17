use crate::{RemoteError, Result, identity::random_id};
use iroh::{EndpointAddr, EndpointId};
use serde::{Deserialize, Serialize};
use std::{net::SocketAddr, time::Duration};
use velune_application_api::{ApplicationReply, ClientViewId, Command, ExecutionContext, Failure};

pub const PROTOCOL_VERSION: u16 = 1;
pub const MAX_FRAME_BYTES: usize = 32 * 1024 * 1024;
pub(crate) const ALPN: &[u8] = b"velune/remote/1";

#[derive(Debug, Clone, Default)]
pub enum RelayConfig {
    #[default]
    Disabled,
    /// Explicit opt-in to n0's public development infrastructure, without a production SLA.
    DevelopmentPublic,
    Custom {
        urls: Vec<String>,
    },
}
#[derive(Debug, Clone, Default)]
pub enum DiscoveryConfig {
    #[default]
    None,
    CustomPkarr {
        url: String,
    },
    /// Publish and resolve addresses using n0's public DNS/pkarr services.
    DevelopmentPublic,
}
#[derive(Debug, Clone, Copy, Default)]
pub enum PathPolicy {
    #[default]
    Automatic,
    /// Controlled diagnostics only: disables direct IP transports.
    RelayOnly,
}
#[derive(Debug, Clone)]
pub struct EndpointConfig {
    pub relay: RelayConfig,
    pub path_policy: PathPolicy,
    pub discovery: DiscoveryConfig,
    /// Empty uses iroh's normal interfaces; loopback addresses support isolated checks.
    pub bind_addresses: Vec<SocketAddr>,
    pub request_timeout: Duration,
}
impl Default for EndpointConfig {
    fn default() -> Self {
        Self {
            relay: RelayConfig::Disabled,
            path_policy: PathPolicy::Automatic,
            discovery: DiscoveryConfig::None,
            bind_addresses: Vec::new(),
            request_timeout: Duration::from_secs(30),
        }
    }
}

/// Public dialing information is not a pairing credential.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct EndpointDescriptor {
    pub version: u16,
    pub endpoint_id: String,
    pub name: String,
    pub incarnation: String,
    pub relay_urls: Vec<String>,
    pub direct_addresses: Vec<SocketAddr>,
}
impl EndpointDescriptor {
    pub(crate) fn address(&self) -> Result<EndpointAddr> {
        if self.version != PROTOCOL_VERSION {
            return Err(RemoteError::IncompatibleVersion);
        }
        crate::identity::validate_name(&self.name)?;
        validate_id(&self.incarnation)?;
        if self.relay_urls.len() > 16 || self.direct_addresses.len() > 32 {
            return Err(RemoteError::Configuration);
        }
        let id = self
            .endpoint_id
            .parse::<EndpointId>()
            .map_err(|_| RemoteError::Configuration)?;
        let mut addr = EndpointAddr::new(id);
        for url in &self.relay_urls {
            addr = addr.with_relay_url(url.parse().map_err(|_| RemoteError::Configuration)?);
        }
        for ip in &self.direct_addresses {
            addr = addr.with_ip_addr(*ip);
        }
        Ok(addr)
    }
}

/// This value contains a one-time secret. Only explicit encode exposes it for pairing.
#[derive(Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Invitation {
    pub endpoint: EndpointDescriptor,
    pub expires_at_unix_seconds: u64,
    pub(crate) token: [u8; 32],
}
impl Invitation {
    pub fn encode(&self) -> Result<String> {
        serde_json::to_string(self).map_err(|_| RemoteError::InvalidInvitation)
    }
    pub fn decode(value: &str) -> Result<Self> {
        if value.len() > 16 * 1024 {
            return Err(RemoteError::InvalidInvitation);
        }
        let invitation: Self =
            serde_json::from_str(value).map_err(|_| RemoteError::InvalidInvitation)?;
        invitation.endpoint.address()?;
        Ok(invitation)
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct RequestId {
    pub incarnation: String,
    pub client: String,
    pub sequence: u64,
}
impl RequestId {
    pub(crate) fn validate(&self, incarnation: &str) -> Result<()> {
        validate_id(&self.incarnation)?;
        validate_id(&self.client)?;
        if self.incarnation != incarnation {
            return Err(RemoteError::ServerRestarted);
        }
        if self.sequence == 0 {
            return Err(RemoteError::InvalidRequestId);
        }
        Ok(())
    }
}
/// Keeps request numbering stable when an application reconnects to an endpoint.
#[derive(Clone)]
pub struct ClientSession {
    pub(crate) id: String,
    pub(crate) next: std::sync::Arc<std::sync::atomic::AtomicU64>,
}
impl ClientSession {
    pub fn new() -> Result<Self> {
        Ok(Self {
            id: random_id()?,
            next: std::sync::Arc::new(std::sync::atomic::AtomicU64::new(1)),
        })
    }
}
#[derive(Clone, Serialize, Deserialize)]
#[serde(tag = "state", content = "result", rename_all = "camelCase")]
pub enum RequestStatus {
    Running,
    Completed(std::result::Result<Box<ApplicationReply>, Failure>),
    /// This incarnation never accepted this request.
    Unknown,
    /// The result is unavailable below the acceptance high-water mark. Reusing the ID never executes it again.
    ResultExpired,
    Failed(RemoteError),
}

pub struct RequestContext {
    pub peer_id: String,
    pub view: ClientViewId,
    pub execution: ExecutionContext,
}

#[derive(Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct Envelope<T> {
    pub version: u16,
    pub body: T,
}
#[derive(Serialize, Deserialize)]
#[serde(tag = "operation", rename_all = "camelCase", deny_unknown_fields)]
pub(crate) enum Request {
    Hello,
    Pair {
        token: [u8; 32],
        name: String,
        incarnation: String,
    },
    Submit {
        id: RequestId,
        view: ClientViewId,
        execution: ExecutionContext,
        command: Box<Command>,
    },
    Query {
        id: RequestId,
    },
}
#[derive(Serialize, Deserialize)]
#[serde(tag = "reply", content = "value", rename_all = "camelCase")]
pub(crate) enum Reply {
    Hello(EndpointDescriptor),
    Paired,
    Status(RequestStatus),
    Error(RemoteError),
}

pub(crate) fn validate_id(id: &str) -> Result<()> {
    if id.len() != 32 || !id.bytes().all(|byte| byte.is_ascii_hexdigit()) {
        return Err(RemoteError::InvalidRequestId);
    }
    Ok(())
}
