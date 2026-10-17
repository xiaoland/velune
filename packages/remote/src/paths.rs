use futures_util::StreamExt;
use iroh::endpoint::{Connection, PathEventStream};
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum PathKind {
    Direct,
    Relay,
    Other,
}

/// Safe transport statistics, without addresses, tokens, or application data.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ConnectionPath {
    pub kind: PathKind,
    pub selected: bool,
    pub rtt_microseconds: u64,
    pub transmitted_bytes: u64,
    pub received_bytes: u64,
    pub lost_packets: u64,
}
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ConnectionStatus {
    pub peer_id: String,
    pub closed: bool,
    pub paths: Vec<ConnectionPath>,
}

/// Subscribes before the initial snapshot so changes cannot be lost between them.
pub struct PathObserver {
    connection: Connection,
    events: PathEventStream,
    initial: bool,
}
impl PathObserver {
    pub(crate) fn new(connection: Connection) -> Self {
        let events = connection.path_events();
        Self {
            connection,
            events,
            initial: true,
        }
    }
    /// Returns the initial snapshot, then a new snapshot on each native path event.
    /// A lagged native event still returns the complete current snapshot.
    pub async fn next(&mut self) -> Option<ConnectionStatus> {
        if self.initial {
            self.initial = false;
        } else {
            self.events.next().await?;
        }
        Some(snapshot(&self.connection))
    }
}
pub(crate) fn snapshot(connection: &Connection) -> ConnectionStatus {
    let paths = connection
        .paths()
        .iter()
        .map(|path| {
            let stats = path.stats();
            ConnectionPath {
                kind: if path.is_ip() {
                    PathKind::Direct
                } else if path.is_relay() {
                    PathKind::Relay
                } else {
                    PathKind::Other
                },
                selected: path.is_selected(),
                rtt_microseconds: u64::try_from(stats.rtt.as_micros()).unwrap_or(u64::MAX),
                transmitted_bytes: stats.udp_tx.bytes,
                received_bytes: stats.udp_rx.bytes,
                lost_packets: stats.lost_packets,
            }
        })
        .collect();
    ConnectionStatus {
        peer_id: connection.remote_id().to_string(),
        closed: connection.close_reason().is_some(),
        paths,
    }
}
