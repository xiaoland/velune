use futures_util::StreamExt;
#[cfg(feature = "diagnostics")]
use iroh::endpoint::LocalTransportAddr;
use iroh::endpoint::{Connection, PathEvent, PathEventStream, PathList};
use serde::{Deserialize, Serialize};
use std::{
    ops::Deref,
    sync::{
        OnceLock,
        atomic::{AtomicU64, Ordering},
    },
    time::Instant,
};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum PathKind {
    Direct,
    Relay,
    Other,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum PathAddressFamily {
    Ipv4,
    Ipv6,
    Other,
}

/// The native path API does not expose candidate discovery provenance.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum PathSource {
    Unknown,
}

/// Safe transport statistics, without addresses, tokens, or application data.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ConnectionPath {
    pub path_id: u64,
    pub kind: PathKind,
    pub address_family: PathAddressFamily,
    pub source: PathSource,
    pub selected: bool,
    pub rtt_microseconds: u64,
    pub transmitted_bytes: u64,
    pub received_bytes: u64,
    pub lost_packets: u64,
}
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ConnectionStatus {
    /// Process-local handle; clones and observers retain it, a fresh dial gets another handle.
    pub connection_id: u64,
    /// Monotonic microseconds from this process's first path observation.
    pub sampled_at_micros: u64,
    pub peer_id: String,
    pub closed: bool,
    pub paths: Vec<ConnectionPath>,
}

/// A complete current snapshot plus explicit loss on this observer's event channel.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct PathObservation {
    pub status: ConnectionStatus,
    pub missed_events: u64,
}

#[derive(Clone)]
pub(crate) struct ObservedConnection {
    id: u64,
    connection: Connection,
}
impl ObservedConnection {
    pub(crate) fn new(connection: Connection) -> Self {
        static NEXT_ID: AtomicU64 = AtomicU64::new(1);
        Self {
            id: NEXT_ID.fetch_add(1, Ordering::Relaxed),
            connection,
        }
    }
}
impl Deref for ObservedConnection {
    type Target = Connection;
    fn deref(&self) -> &Self::Target {
        &self.connection
    }
}

/// Subscribes before the initial snapshot so changes cannot be lost between them.
pub struct PathObserver {
    connection: ObservedConnection,
    events: PathEventStream,
    initial: bool,
}
impl PathObserver {
    pub(crate) fn new(connection: ObservedConnection) -> Self {
        let events = connection.path_events();
        Self {
            connection,
            events,
            initial: true,
        }
    }
    /// Returns the initial snapshot, then the current snapshot on each native path event.
    /// A lagged native event still returns the complete current snapshot.
    pub async fn next(&mut self) -> Option<PathObservation> {
        let missed_events = if self.initial {
            self.initial = false;
            0
        } else {
            match self.events.next().await? {
                PathEvent::Lagged { missed, .. } => missed,
                _ => 0,
            }
        };
        Some(PathObservation {
            status: snapshot(&self.connection),
            missed_events,
        })
    }
}
pub(crate) fn snapshot(connection: &ObservedConnection) -> ConnectionStatus {
    snapshot_paths(connection, &connection.paths())
}
fn snapshot_paths(connection: &ObservedConnection, native_paths: &PathList) -> ConnectionStatus {
    let paths = native_paths
        .iter()
        .map(|path| {
            let stats = path.stats();
            ConnectionPath {
                // Pinned noq 1.3 exposes PathId as a decimal u32 through Display only.
                path_id: path
                    .id()
                    .to_string()
                    .parse()
                    .expect("noq PathId Display is a u32"),
                kind: if path.is_ip() {
                    PathKind::Direct
                } else if path.is_relay() {
                    PathKind::Relay
                } else {
                    PathKind::Other
                },
                address_family: match path.remote_addr() {
                    iroh::TransportAddr::Ip(address) if address.is_ipv4() => {
                        PathAddressFamily::Ipv4
                    }
                    iroh::TransportAddr::Ip(_) => PathAddressFamily::Ipv6,
                    _ => PathAddressFamily::Other,
                },
                source: PathSource::Unknown,
                selected: path.is_selected(),
                rtt_microseconds: u64::try_from(stats.rtt.as_micros()).unwrap_or(u64::MAX),
                transmitted_bytes: stats.udp_tx.bytes,
                received_bytes: stats.udp_rx.bytes,
                lost_packets: stats.lost_packets,
            }
        })
        .collect();
    ConnectionStatus {
        connection_id: connection.id,
        sampled_at_micros: monotonic_microseconds(),
        peer_id: connection.remote_id().to_string(),
        closed: connection.close_reason().is_some(),
        paths,
    }
}

/// Explicitly private transport evidence. Never send this record to ordinary logs or UI.
#[cfg(feature = "diagnostics")]
#[derive(Clone, Serialize)]
pub struct ConnectionDiagnostics {
    pub status: ConnectionStatus,
    pub paths: Vec<PathDiagnostic>,
}
#[cfg(feature = "diagnostics")]
#[derive(Clone, Serialize)]
pub struct PathDiagnostic {
    pub path_id: u64,
    pub local_address: Option<String>,
    pub remote_address: String,
}
#[cfg(feature = "diagnostics")]
pub(crate) fn diagnostics(connection: &ObservedConnection) -> ConnectionDiagnostics {
    let native_paths = connection.paths();
    let status = snapshot_paths(connection, &native_paths);
    let paths = native_paths
        .iter()
        .map(|path| PathDiagnostic {
            path_id: path
                .id()
                .to_string()
                .parse()
                .expect("noq PathId Display is a u32"),
            local_address: match path.local_addr() {
                LocalTransportAddr::Ip(ip) => ip.map(|ip| ip.to_string()),
                LocalTransportAddr::Relay(url) => Some(url.to_string()),
                _ => None,
            },
            remote_address: match path.remote_addr() {
                iroh::TransportAddr::Ip(address) => address.to_string(),
                iroh::TransportAddr::Relay(url) => url.to_string(),
                _ => "custom".into(),
            },
        })
        .collect();
    ConnectionDiagnostics { status, paths }
}

/// Process-local transport observation clock, unrelated to wall-clock time.
pub fn monotonic_microseconds() -> u64 {
    static ORIGIN: OnceLock<Instant> = OnceLock::new();
    u64::try_from(ORIGIN.get_or_init(Instant::now).elapsed().as_micros()).unwrap_or(u64::MAX)
}
