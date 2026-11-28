//! Bounded per-connection writes and ordered interleaved protocol records.
//! No transcript scratch files or second durable history are created here.
use std::{
    collections::VecDeque,
    io::{self, Write},
    process::ChildStdin,
    sync::{Arc, Mutex, mpsc},
    thread,
    time::Instant,
};

const DEFERRED_RECORDS: usize = 256;
const DEFERRED_BYTES: usize = 8 * 1024 * 1024;

mod writer;
pub(crate) use writer::{NotAdmitted, OwnedFrame, PipeWriter, WriteClass, WriteReceipt};

pub(crate) const fn writer_policy_limit() -> usize {
    writer::LEGACY_ALLOCATION_LIMIT
}

/// Recorded by the physical writer immediately after write_all + flush.
/// Observation/harvest time must not be substituted for either timestamp.
#[derive(Clone, Copy, Debug)]
pub struct WriteCompletion {
    pub started_at: Instant,
    pub completed_at: Instant,
    pub deadline: Instant,
}
impl WriteCompletion {
    pub fn met_deadline(self) -> bool {
        self.started_at < self.deadline && self.completed_at < self.deadline
    }
}

/// Rust-only physical evidence; neither admission nor native acceptance.
#[derive(Clone, Copy, Debug)]
pub enum WriteFact {
    Written(WriteCompletion),
    NotWritten,
    Indeterminate,
}

#[derive(Debug)]
pub(crate) enum WriteOutcome {
    /// Even a late flush is real Written evidence, but is NOT timely/healthy.
    Written(WriteCompletion),
    /// The physical writer skipped this job without attempting a byte.
    NotWritten(io::Error),
    /// An attempted write failed, or the writer disconnected ambiguously.
    Indeterminate(io::Error),
    /// No physical outcome is available yet. Retain the receipt to observe
    /// later Written/NotWritten; this observation does not cancel the job.
    ObservationTimeout {
        deadline: Instant,
        observed_at: Instant,
    },
}

/// A response wait must not turn bounded ingress into an unbounded side queue.
/// Overload is a terminal transport error, never silent loss of approvals or
/// background events. The harness remains the authority for persisted history.
#[derive(Default)]
pub(crate) struct DeferredRecords {
    memory: VecDeque<(serde_json::Value, usize)>,
    bytes: usize,
}
impl DeferredRecords {
    pub(crate) fn push(&mut self, value: serde_json::Value) -> io::Result<()> {
        let bytes = serde_json::to_vec(&value)?.len();
        if self.memory.len() >= DEFERRED_RECORDS
            || (!self.memory.is_empty() && self.bytes.saturating_add(bytes) > DEFERRED_BYTES)
        {
            return Err(io::Error::new(
                io::ErrorKind::WouldBlock,
                "runtime interleaved event budget full",
            ));
        }
        self.bytes += bytes;
        self.memory.push_back((value, bytes));
        Ok(())
    }
    pub(crate) fn pop(&mut self) -> Option<serde_json::Value> {
        let (value, bytes) = self.memory.pop_front()?;
        self.bytes -= bytes;
        Some(value)
    }
    pub(crate) fn append(&mut self, other: &mut Self) -> io::Result<()> {
        if other.memory.is_empty() {
            return Ok(());
        }
        let count = self.memory.len().saturating_add(other.memory.len());
        let bytes = self.bytes.saturating_add(other.bytes);
        if count > DEFERRED_RECORDS || (count > 1 && bytes > DEFERRED_BYTES) {
            return Err(io::Error::new(
                io::ErrorKind::WouldBlock,
                "runtime interleaved event budget full",
            ));
        }
        // Admission is atomic: failure leaves both ordered queues intact.
        self.memory.append(&mut other.memory);
        self.bytes = bytes;
        other.bytes = 0;
        Ok(())
    }
    pub(crate) fn drain_batch(&mut self, maximum: usize) -> Vec<serde_json::Value> {
        let mut values = Vec::new();
        while values.len() < maximum {
            let Some(value) = self.pop() else { break };
            values.push(value);
        }
        values
    }
    pub(crate) fn drain(&mut self) -> Vec<serde_json::Value> {
        let records = self.memory.drain(..).map(|(value, _)| value).collect();
        self.bytes = 0;
        records
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn deferred_records_preserve_order_and_reject_overload_without_truncation() {
        let mut records = DeferredRecords::default();
        for sequence in 0..DEFERRED_RECORDS {
            records.push(json!({"sequence": sequence})).unwrap();
        }
        assert_eq!(
            records.push(json!("overflow")).unwrap_err().kind(),
            io::ErrorKind::WouldBlock
        );
        for sequence in 0..DEFERRED_RECORDS {
            assert_eq!(records.pop().unwrap()["sequence"], sequence);
        }
        assert!(records.pop().is_none());
        let large = "x".repeat(DEFERRED_BYTES + 1);
        records.push(json!({"text": large})).unwrap();
        assert!(records.push(json!("next")).is_err());
        assert_eq!(
            records.pop().unwrap()["text"].as_str().unwrap().len(),
            large.len()
        );
        records.push(json!("recovered")).unwrap();
        let mut later = DeferredRecords::default();
        later.push(json!({"text": large})).unwrap();
        assert!(records.append(&mut later).is_err());
        assert_eq!(records.pop().unwrap(), json!("recovered"));
        records.append(&mut later).unwrap();
        assert!(later.pop().is_none());
        assert_eq!(
            records.pop().unwrap()["text"].as_str().unwrap().len(),
            large.len()
        );
    }
}

#[cfg(test)]
#[path = "transport/writer_tests.rs"]
mod writer_tests;
