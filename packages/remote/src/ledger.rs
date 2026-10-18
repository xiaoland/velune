use crate::{RemoteError, RequestId, RequestStatus, Result};
use std::collections::{BTreeSet, HashMap, VecDeque};

const MAX_NAMESPACES: usize = 4096;
const SEQUENCE_WINDOW: u64 = 256;
const MAX_RUNNING: usize = 64;
const MAX_CACHE_BYTES: usize = 32 * 1024 * 1024;
const MAX_CACHE_RESULTS: usize = 256;

type Key = (String, RequestId);
struct Record {
    fingerprint: [u8; 32],
    result: Option<Vec<u8>>,
}
#[derive(Default)]
struct Namespace {
    highest: u64,
    accepted: BTreeSet<u64>,
}
impl Namespace {
    fn retired(&self, sequence: u64) -> bool {
        sequence <= self.highest.saturating_sub(SEQUENCE_WINDOW)
    }
    fn unavailable(&self, sequence: u64) -> bool {
        self.retired(sequence) || self.accepted.contains(&sequence)
    }
    fn accept(&mut self, sequence: u64) {
        self.highest = self.highest.max(sequence);
        let retired = self.highest.saturating_sub(SEQUENCE_WINDOW);
        self.accepted.retain(|sequence| *sequence > retired);
        self.accepted.insert(sequence);
    }
}
#[derive(Default)]
pub(crate) struct Ledger {
    high_water: HashMap<(String, String), Namespace>,
    records: HashMap<Key, Record>,
    completed: VecDeque<Key>,
    completed_bytes: usize,
    running: usize,
}
impl Ledger {
    /// Returns true exactly once for an accepted request. Old/evicted sequences never run again.
    pub fn accept(&mut self, peer: &str, id: &RequestId, fingerprint: [u8; 32]) -> Result<bool> {
        let key = (peer.to_owned(), id.clone());
        if let Some(record) = self.records.get(&key) {
            if record.fingerprint != fingerprint {
                return Err(RemoteError::RequestIdConflict);
            }
            return Ok(false);
        }
        let namespace = (peer.to_owned(), id.client.clone());
        if self
            .high_water
            .get(&namespace)
            .is_some_and(|namespace| namespace.unavailable(id.sequence))
        {
            return Ok(false);
        }
        if self.running >= MAX_RUNNING
            || (!self.high_water.contains_key(&namespace)
                && self.high_water.len() >= MAX_NAMESPACES)
        {
            return Err(RemoteError::Capacity);
        }
        self.high_water
            .entry(namespace)
            .or_default()
            .accept(id.sequence);
        self.records.insert(
            key,
            Record {
                fingerprint,
                result: None,
            },
        );
        self.running += 1;
        Ok(true)
    }
    pub fn status(&self, peer: &str, id: &RequestId) -> Result<RequestStatus> {
        if let Some(record) = self.records.get(&(peer.to_owned(), id.clone())) {
            return match &record.result {
                Some(bytes) => serde_json::from_slice(bytes).map_err(|_| RemoteError::Protocol),
                None => Ok(RequestStatus::Running),
            };
        }
        if self
            .high_water
            .get(&(peer.to_owned(), id.client.clone()))
            .is_some_and(|namespace| namespace.unavailable(id.sequence))
        {
            Ok(RequestStatus::ResultExpired)
        } else {
            Ok(RequestStatus::Unknown)
        }
    }
    pub fn complete(&mut self, peer: String, id: RequestId, status: RequestStatus) {
        let key = (peer, id);
        let bytes = match crate::wire::encode(&status, crate::MAX_FRAME_BYTES - 1024) {
            Ok(bytes) => bytes,
            Err(RemoteError::FrameTooLarge) => {
                br#"{"state":"failed","result":"frameTooLarge"}"#.to_vec()
            }
            Err(_) => br#"{"state":"failed","result":"protocol"}"#.to_vec(),
        };
        if let Some(record) = self.records.get_mut(&key) {
            if record.result.is_some() {
                return;
            }
            self.running = self.running.saturating_sub(1);
            self.completed_bytes += bytes.len();
            record.result = Some(bytes);
            self.completed.push_back(key);
        }
        while self.completed_bytes > MAX_CACHE_BYTES || self.completed.len() > MAX_CACHE_RESULTS {
            if let Some(key) = self.completed.pop_front() {
                if let Some(record) = self.records.remove(&key) {
                    self.completed_bytes = self
                        .completed_bytes
                        .saturating_sub(record.result.as_ref().map_or(0, Vec::len));
                }
            } else {
                break;
            }
        }
    }
}
