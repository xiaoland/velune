//! Pi's type/id protocol. Admission, write completion and native outcomes are distinct.
use super::*;
use crate::transport::{OwnedFrame, WriteClass, WriteOutcome, WriteReceipt};
use std::{
    collections::{BTreeMap, VecDeque},
    sync::atomic::{AtomicU64, Ordering},
};

static GENERATION: AtomicU64 = AtomicU64::new(1);
#[cfg(test)]
thread_local! { static INPUT_ENCODINGS: std::cell::Cell<usize> = const { std::cell::Cell::new(0) }; }

mod encoding;

const OPERATIONS: usize = 64;
const RESERVED: usize = 8;
const PAYLOAD_BYTES: usize = 256 * 1024 * 1024;
const REPLY_BYTES: usize = 8 * 1024 * 1024;

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub struct PiTicket {
    pub generation: u64,
    pub session_generation: u64,
    pub sequence: u64,
}

/// Only supported Pi 1.0.2 commands; this is not a JSON-RPC method namespace.
#[derive(Debug, Clone)]
pub enum PiOperation {
    Prompt(crate::TurnInput),
    Steer(crate::TurnInput),
    DialogReply(Value),
    ClearQueue,
    Abort,
    Compact,
    ThinkingLevel(String),
    SetModel {
        provider: Option<String>,
        model_id: String,
    },
    ProjectionSync,
    GetState,
    GetMessages,
    GetCommands,
    GetModels,
    Background(Value),
    Rewind(String),
    Fork(String),
    NewSession,
    SetName(String),
    SwitchSession(PathBuf),
    SyncSelection,
}
impl PiOperation {
    fn reserved(&self) -> bool {
        matches!(self, Self::DialogReply(_) | Self::ClearQueue | Self::Abort)
    }
    fn turn_result(&self) -> bool {
        matches!(self, Self::Prompt(_) | Self::Compact)
    }
    fn changes_scope(&self) -> bool {
        matches!(
            self,
            Self::Fork(_) | Self::NewSession | Self::SwitchSession(_) | Self::Rewind(_)
        )
    }
    fn command_type(&self) -> Result<&'static str> {
        Ok(match self {
            Self::Prompt(input) => {
                if input.effort.is_some() {
                    return Err(Error::Sdk(
                        "enqueue ThinkingLevel before the dependent Prompt".into(),
                    ));
                }
                "prompt"
            }
            Self::Steer(_) => "steer",
            Self::DialogReply(value) => {
                if value["type"] != "extension_ui_response"
                    || value["id"].as_str().is_none_or(str::is_empty)
                {
                    return Err(Error::InvalidCommand);
                }
                "extension_ui_response"
            }
            Self::ThinkingLevel(level) => {
                if !PI_THINKING_LEVELS.contains(&level.as_str()) {
                    return Err(Error::Sdk("Pi 不接受此推理强度".into()));
                }
                "set_thinking_level"
            }
            Self::ClearQueue => "clear_queue",
            Self::Abort => "abort",
            Self::Compact => "compact",
            Self::SetModel { .. } => "set_model",
            Self::GetState => "get_state",
            Self::GetMessages => "get_messages",
            Self::GetCommands => "get_commands",
            Self::GetModels => "get_available_models",
            Self::ProjectionSync | Self::Background(_) | Self::Rewind(_) | Self::SyncSelection => {
                "prompt"
            }
            Self::Fork(_) => "fork",
            Self::NewSession => "new_session",
            Self::SetName(_) => "set_session_name",
            Self::SwitchSession(_) => "switch_session",
        })
    }
}

#[derive(Debug, Clone, PartialEq)]
pub enum PiReceiptPhase {
    Written,
    NotSent,
    AwaitingUser(Value),
    Accepted(Value),
    Rejected(Value),
    /// Prompt/compact responses describe execution, not admission ACKs.
    Outcome(Value),
    Indeterminate(String),
}
#[derive(Debug, Clone)]
pub struct PiReceipt {
    pub ticket: PiTicket,
    pub wire_id: String,
    pub phase: PiReceiptPhase,
    pub physical_write: Option<crate::WriteFact>,
    pub deadline: Option<Instant>,
}
#[derive(Debug)]
pub enum PiPortItem {
    Event(Value),
    OperationEvent {
        ticket: PiTicket,
        event: Value,
        deadline: Option<Instant>,
    },
    Receipt(PiReceipt),
}

struct Operation {
    ticket: PiTicket,
    id: String,
    command: String,
    bytes: Option<Vec<u8>>,
    size: usize,
    write: Option<crate::transport::WriteReceipt>,
    written: bool,
    one_way: bool,
    reserved: bool,
    turn_result: bool,
    deadline_is_local: bool,
    write_deadline: Instant,
    response_deadline: Option<Instant>,
    failed: bool,
    dependency: Option<PiTicket>,
    changes_scope: bool,
    outcome_seen: bool,
    physical_write: Option<crate::WriteFact>,
}

impl Operation {
    fn install_deadline(&self) -> Option<Instant> {
        if self.deadline_is_local {
            None
        } else {
            self.response_deadline
        }
    }
}

pub(super) struct Port {
    generation: u64,
    session_generation: u64,
    next: u64,
    operations: BTreeMap<u64, Operation>,
    evidence: VecDeque<PiReceipt>,
    bytes: usize,
    failed: Option<String>,
    reported: bool,
    transition: Option<PiTicket>,
}
impl Port {
    pub(super) fn new() -> Result<Self> {
        let generation = GENERATION
            .try_update(Ordering::Relaxed, Ordering::Relaxed, |n| n.checked_add(1))
            .map_err(|_| Error::Transport("Pi generation exhausted".into()))?;
        Ok(Self {
            generation,
            session_generation: 1,
            next: 1,
            operations: BTreeMap::new(),
            evidence: VecDeque::new(),
            bytes: 0,
            failed: None,
            reported: false,
            transition: None,
        })
    }
    fn evidence(&mut self, sequence: u64, phase: PiReceiptPhase) {
        let operation = &self.operations[&sequence];
        self.evidence.push_back(PiReceipt {
            ticket: operation.ticket,
            wire_id: operation.id.clone(),
            deadline: operation.install_deadline(),
            phase,
            physical_write: operation.physical_write,
        });
    }
    fn fail_operation(&mut self, sequence: u64, detail: String) {
        let not_sent = self.operations[&sequence].bytes.is_some();
        self.operations.get_mut(&sequence).unwrap().failed = true;
        self.evidence(
            sequence,
            if not_sent {
                PiReceiptPhase::NotSent
            } else {
                PiReceiptPhase::Indeterminate(detail)
            },
        );
        if not_sent {
            self.remove(sequence);
        }
    }
    fn fail(&mut self, error: String) {
        if self.failed.is_some() {
            return;
        }
        self.failed = Some(error.clone());
        let affected = self
            .operations
            .iter()
            .filter_map(|(sequence, operation)| (!operation.failed).then_some(*sequence))
            .collect::<Vec<_>>();
        for sequence in affected {
            self.fail_operation(sequence, error.clone());
        }
    }
    fn remove(&mut self, sequence: u64) {
        if let Some(op) = self.operations.remove(&sequence) {
            self.bytes -= op.size;
        }
    }
    fn settle(&mut self) {
        let sequences = self.operations.keys().copied().collect::<Vec<_>>();
        for seq in sequences {
            let Some(outcome) = self
                .operations
                .get(&seq)
                .and_then(|op| op.write.as_ref())
                .and_then(WriteReceipt::try_outcome)
            else {
                continue;
            };
            let terminal = !matches!(outcome, WriteOutcome::ObservationTimeout { .. });
            let current =
                self.operations[&seq].ticket.session_generation == self.session_generation;
            let deadline_is_local = self.operations[&seq].deadline_is_local;
            match outcome {
                WriteOutcome::Written(completion) => {
                    let op = self.operations.get_mut(&seq).unwrap();
                    op.written = true;
                    op.physical_write = Some(crate::WriteFact::Written(completion));
                    self.evidence(seq, PiReceiptPhase::Written);
                    if !completion.met_deadline() && current && !deadline_is_local {
                        self.fail("Pi physically Written after deadline; cleanup required".into());
                    }
                }
                WriteOutcome::NotWritten(error) => {
                    let op = self.operations.get_mut(&seq).unwrap();
                    op.physical_write = Some(crate::WriteFact::NotWritten);
                    op.failed = true;
                    self.evidence(seq, PiReceiptPhase::NotSent);
                    if current && !deadline_is_local {
                        self.fail(format!("Pi known not written: {error}"));
                    }
                }
                WriteOutcome::Indeterminate(error) => {
                    self.operations.get_mut(&seq).unwrap().physical_write =
                        Some(crate::WriteFact::Indeterminate);
                    if current {
                        self.fail(format!("Pi physical write indeterminate: {error}"));
                    }
                }
                WriteOutcome::ObservationTimeout { .. } => {
                    if current && !deadline_is_local {
                        self.fail("Pi write not yet observed; receipt retained".into());
                    }
                }
            }
            if terminal {
                let op = self.operations.get_mut(&seq).unwrap();
                op.write = None;
                if op.one_way || op.outcome_seen || !op.written || (deadline_is_local && op.failed)
                {
                    self.remove(seq);
                }
            }
        }
        self.latch(Instant::now());
    }

    fn latch(&mut self, now: Instant) {
        let expired = self
            .operations
            .iter()
            .filter_map(|(seq, op)| {
                (!op.failed
                    && !op.outcome_seen
                    && op.ticket.session_generation == self.session_generation
                    && ((!op.written && now >= op.write_deadline)
                        || op.response_deadline.is_some_and(|deadline| now >= deadline)))
                .then_some(*seq)
            })
            .collect::<Vec<_>>();
        for seq in expired {
            let Some(operation) = self
                .operations
                .get(&seq)
                .filter(|operation| !operation.failed)
            else {
                continue;
            };
            let detail = format!(
                "Pi {} deadline expired; delivery/outcome indeterminate",
                operation.command
            );
            let deadline_is_local = operation.deadline_is_local;
            let write_settled = operation.write.is_none();
            self.fail_operation(seq, detail.clone());
            if deadline_is_local {
                // A late or missing metadata response does not affect turn
                // delivery. Keep only physical writes still being observed.
                if write_settled {
                    self.remove(seq);
                }
            } else {
                self.fail(detail);
            }
        }
    }
    fn observe_record(&mut self, value: Value, output: &mut Vec<PiPortItem>) {
        self.settle();
        self.flush_evidence(output);
        if let Some(error) = &self.failed
            && !self.reported
        {
            output.push(PiPortItem::Event(
                json!({"type":"velune_error","error":error}),
            ));
            self.reported = true;
        }
        let matched = if value["type"] == "response" && value["success"].is_boolean() {
            self.operations.iter().find_map(|(seq, op)| {
                (!op.one_way
                    && !op.outcome_seen
                    && !(op.deadline_is_local && op.failed)
                    && value["id"].as_str() == Some(&op.id)
                    && value["command"].as_str() == Some(&op.command))
                .then_some(*seq)
            })
        } else {
            None
        };
        // Never turn a late outcome into a healthy connection. Still retain
        // and report the actual outcome, even after the failure barrier.
        if let Some(seq) = matched {
            let op = &self.operations[&seq];
            // Only execution responses are projection events. Do not duplicate
            // the authoritative get_messages payload into an event and receipt.
            if matches!(op.command.as_str(), "prompt" | "compact") {
                output.push(PiPortItem::OperationEvent {
                    ticket: op.ticket,
                    event: value.clone(),
                    deadline: op.install_deadline(),
                });
            }
            if op.turn_result
                && value["success"] == true
                && value["data"]["disposition"] == "awaiting_user"
            {
                self.evidence(seq, PiReceiptPhase::AwaitingUser(value));
                self.flush_evidence(output);
                return;
            }
            let phase = if value["success"] == false {
                PiReceiptPhase::Rejected(value)
            } else if op.turn_result {
                PiReceiptPhase::Outcome(value)
            } else {
                PiReceiptPhase::Accepted(value)
            };
            let success = !matches!(phase, PiReceiptPhase::Rejected(_));
            if op.changes_scope
                && (!success
                    || matches!(&phase, PiReceiptPhase::Accepted(response) if response["data"]["cancelled"] == true))
                && self.transition == Some(op.ticket)
            {
                self.transition = self
                    .operations
                    .values()
                    .find(|operation| operation.changes_scope && operation.ticket.sequence > seq)
                    .map(|operation| operation.ticket);
            }

            self.evidence(seq, phase);
            let dependencies = self
                .operations
                .iter()
                .filter_map(|(dependent, op)| {
                    (op.dependency == Some(self.operations[&seq].ticket)).then_some(*dependent)
                })
                .collect::<Vec<_>>();
            for dependent in dependencies {
                if success {
                    self.operations.get_mut(&dependent).unwrap().dependency = None;
                } else {
                    self.evidence(dependent, PiReceiptPhase::NotSent);
                    self.remove(dependent);
                }
            }
            if self.operations[&seq].write.is_some() {
                self.operations.get_mut(&seq).unwrap().outcome_seen = true;
            } else {
                self.remove(seq);
            }
            self.flush_evidence(output);
        } else {
            output.push(PiPortItem::Event(value));
        }
    }

    fn flush_evidence(&mut self, output: &mut Vec<PiPortItem>) {
        output.extend(self.evidence.drain(..).map(PiPortItem::Receipt));
    }
}
impl Client {
    /// Irreversible for this connection. The old synchronous facade must not mix requests.
    pub fn enable_port(&mut self) -> Result<u64> {
        if self.port.is_none() {
            let mut port = Port::new()?;
            if let Some(error) = &self.terminal_error {
                port.fail(error.clone());
            }
            self.port = Some(port);
        }
        Ok(self.port.as_ref().unwrap().generation)
    }
    /// An explicit absolute run deadline is never renewed at Written or by dialogs.
    /// Default prompt/compact budgets apply only to writing, not model/human time.
    pub fn enqueue_pi(
        &mut self,
        operation: PiOperation,
        run_deadline: Option<Instant>,
    ) -> Result<PiTicket> {
        self.enable_port()?;
        let reserved = operation.reserved();
        let cancellation = matches!(operation, PiOperation::ClearQueue | PiOperation::Abort);
        let turn_result = operation.turn_result();
        let deadline_is_local = matches!(
            operation,
            PiOperation::SetName(_) | PiOperation::Background(_)
        );
        let port = self.port.as_mut().unwrap();
        port.settle();
        if let Some(error) = &port.failed {
            return Err(Error::Transport(error.clone()));
        }
        // Evidence and pending operations share admission capacity; consumers cannot
        // free an unanswered on-wire operation by dropping a ticket.
        let used = port.operations.len() + port.evidence.len();
        if used
            >= if cancellation {
                OPERATIONS
            } else if reserved {
                OPERATIONS - 2
            } else {
                OPERATIONS - RESERVED
            }
        {
            return Err(Error::Transport(
                "Pi operation/evidence capacity full".into(),
            ));
        }
        if matches!(operation, PiOperation::Compact) && port.transition.is_some() {
            return Err(Error::ScopeTransitionBusy);
        }
        let sequence = port.next;
        let next = sequence
            .checked_add(1)
            .ok_or_else(|| Error::Transport("Pi sequence exhausted".into()))?;
        let method = operation.command_type()?;
        let id = if let PiOperation::DialogReply(response) = &operation {
            std::borrow::Cow::Borrowed(response["id"].as_str().unwrap())
        } else {
            std::borrow::Cow::Owned(format!("velune-pi-{}-{sequence}", port.generation))
        };
        let budget = if cancellation {
            PAYLOAD_BYTES + REPLY_BYTES
        } else if reserved {
            PAYLOAD_BYTES + REPLY_BYTES - 64 * 1024
        } else {
            PAYLOAD_BYTES
        };
        let available = budget
            .checked_sub(port.bytes)
            .ok_or_else(|| Error::Transport("Pi encoded payload capacity full".into()))?;
        let allocation_limit = available.min(if reserved {
            1024 * 1024
        } else {
            crate::transport::writer_policy_limit()
        });
        let bytes = encoding::encode(&operation, &id, allocation_limit)?;
        let size = bytes.capacity();
        let changes_scope = operation.changes_scope();
        let id = if matches!(operation, PiOperation::DialogReply(_)) {
            drop(id);
            let PiOperation::DialogReply(mut response) = operation else {
                unreachable!()
            };
            let Value::String(id) = response.as_object_mut().unwrap().remove("id").unwrap() else {
                unreachable!()
            };
            id
        } else {
            id.into_owned()
        };
        let now = Instant::now();
        let write_deadline = run_deadline.map_or(now + RESPONSE_TIMEOUT, |deadline| {
            deadline.min(now + RESPONSE_TIMEOUT)
        });
        let response_deadline = if turn_result {
            run_deadline
        } else {
            Some(write_deadline)
        };
        let ticket = PiTicket {
            generation: port.generation,
            session_generation: port.session_generation,
            sequence,
        };
        port.next = next;
        if changes_scope && port.transition.is_none() {
            port.transition = Some(ticket);
        }
        port.bytes += size;
        port.operations.insert(
            sequence,
            Operation {
                ticket,
                id,
                command: method.to_owned(),
                bytes: Some(bytes),
                size,
                write: None,
                written: false,
                one_way: method == "extension_ui_response",
                reserved,
                turn_result,
                deadline_is_local,
                write_deadline,
                response_deadline,
                failed: false,
                dependency: None,
                changes_scope,
                outcome_seen: false,
                physical_write: None,
            },
        );
        Ok(ticket)
    }
    /// Reserve the dependent payload now, but dispatch it only after a matching
    /// successful predecessor response. No caller-owned unbudgeted input queue.
    pub fn enqueue_pi_after_thinking(
        &mut self,
        input: crate::TurnInput,
        predecessor: PiTicket,
        run_deadline: Option<Instant>,
    ) -> Result<PiTicket> {
        let port = self.port.as_ref().ok_or(Error::NotStarted)?;
        if predecessor.generation != port.generation
            || predecessor.session_generation != port.session_generation
            || !port
                .operations
                .get(&predecessor.sequence)
                .is_some_and(|operation| {
                    operation.command == "set_thinking_level" && !operation.failed
                })
        {
            return Err(Error::Transport(
                "Pi predecessor is not pending on this generation".into(),
            ));
        }
        let ticket = self.enqueue_pi(PiOperation::Prompt(input), run_deadline)?;
        self.port
            .as_mut()
            .unwrap()
            .operations
            .get_mut(&ticket.sequence)
            .unwrap()
            .dependency = Some(predecessor);
        Ok(ticket)
    }

    pub fn pi_session_generation(&self) -> Option<u64> {
        self.port.as_ref().map(|port| port.session_generation)
    }

    pub(crate) fn pi_owns_ticket(&self, ticket: PiTicket) -> bool {
        self.port.as_ref().is_some_and(|port| {
            ticket.generation == port.generation
                && ticket.session_generation == port.session_generation
        })
    }

    /// The owner calls this only after a confirmed native session/branch change.
    /// Old on-wire outcomes remain observable, but unsent old-session work is
    /// NotSent and must never leak into the new native leaf.
    pub fn advance_pi_session_generation(&mut self) -> Result<u64> {
        let port = self.port.as_mut().ok_or(Error::NotStarted)?;
        let Some(next) = port.session_generation.checked_add(1) else {
            port.fail("Pi session generation exhausted".into());
            return Err(Error::Transport("Pi session generation exhausted".into()));
        };
        let queued = port
            .operations
            .iter()
            .filter_map(|(sequence, operation)| operation.bytes.is_some().then_some(*sequence))
            .collect::<Vec<_>>();
        for sequence in queued {
            port.evidence(sequence, PiReceiptPhase::NotSent);
            port.remove(sequence);
        }
        port.session_generation = next;
        port.transition = None;
        Ok(next)
    }

    /// Release a native transition which the owner confirmed did not change scope.
    pub fn release_pi_transition(&mut self, ticket: PiTicket) {
        if let Some(port) = &mut self.port
            && port.transition == Some(ticket)
        {
            port.transition = port
                .operations
                .values()
                .filter(|operation| operation.changes_scope && operation.ticket != ticket)
                .map(|operation| operation.ticket)
                .next();
        }
    }

    /// Abandon only an operation which has never reached PipeWriter. Written or
    /// partially written input is never replayed or removed by ticket abandonment.
    pub fn abandon_queued_pi(&mut self, ticket: PiTicket) -> bool {
        let Some(port) = &mut self.port else {
            return false;
        };
        if ticket.generation != port.generation
            || ticket.session_generation != port.session_generation
        {
            return false;
        }
        if !port
            .operations
            .get(&ticket.sequence)
            .is_some_and(|op| op.bytes.is_some() && !op.failed)
        {
            return false;
        }
        port.evidence(ticket.sequence, PiReceiptPhase::NotSent);
        port.remove(ticket.sequence);
        if port.transition == Some(ticket) {
            port.transition = port
                .operations
                .values()
                .find(|operation| operation.changes_scope)
                .map(|operation| operation.ticket);
        }
        true
    }

    /// Persistent cleanup lease fact, independent of any last enqueue ticket.
    /// Only explicit shutdown/drop reaps the subprocess.
    pub fn pi_cleanup_required(&self) -> bool {
        self.terminal_error.is_some()
            || self.port.as_ref().is_some_and(|port| port.failed.is_some())
    }

    /// Recheck a queued native terminal at the owner installation boundary.
    /// Physical write timeliness remains a separate shared-writer fact.
    pub(crate) fn pi_install_failure(&mut self, deadline: Option<Instant>) -> Option<String> {
        let port = self.port.as_mut()?;
        let now = Instant::now();
        port.settle();
        if deadline.is_some_and(|deadline| now >= deadline) {
            port.fail("Pi native outcome deadline expired before owner installation".into());
        }
        if port.failed.is_some() {
            port.reported = true;
        }
        port.failed.clone()
    }

    /// Nonblocking owner pump. Receipt positions are wire fences: in particular
    /// GetMessages evidence follows all older events and precedes newer events.
    pub fn poll_pi(&mut self) -> Vec<PiPortItem> {
        let Some(port) = self.port.as_mut() else {
            return Vec::new();
        };
        let mut output = Vec::new();
        port.settle(); // latch before consuming even an already queued terminal
        let sequences = port.operations.keys().copied().collect::<Vec<_>>();
        // Reserved replies/cancellation get dispatch capacity before business writes.
        'dispatch: for reserved in [true, false] {
            for seq in &sequences {
                port.settle();
                let writer_busy = port
                    .operations
                    .values()
                    .any(|op| op.write.is_some() && !op.written);
                // An unsent compaction must not start *after* its cancellation.
                // Order its write before clear_queue/abort, without waiting for
                // compaction's (potentially long-running) native result.
                let compact_precedes_cancel = port.operations.get(seq).is_some_and(|operation| {
                    matches!(operation.command.as_str(), "clear_queue" | "abort")
                }) && port.operations.range(..*seq).any(
                    |(_, operation)| operation.command == "compact" && operation.bytes.is_some(),
                );
                if compact_precedes_cancel {
                    continue;
                }
                let Some(op) = port.operations.get_mut(seq) else {
                    continue;
                };
                if op.dependency.is_some()
                    || op.reserved != reserved
                    || op.failed
                    || port.failed.is_some()
                    || (!reserved
                        && port
                            .transition
                            .is_some_and(|transition| *seq > transition.sequence))
                {
                    continue;
                }
                if op.bytes.is_some() && writer_busy && !reserved {
                    continue;
                }
                if op.bytes.is_some() {
                    // At most one ordinary frame is outstanding. Protocol frames
                    // may reserve bounded capacity in the same non-preemptive FIFO.
                    let Some(stdin) = &self.stdin else {
                        port.fail_operation(*seq, "Pi stdin unavailable".into());
                        port.fail("Pi stdin unavailable".into());
                        continue;
                    };
                    let bytes = op.bytes.take().unwrap();
                    // Admission failure proves these bytes never reached the writer.
                    // Do not retry or replay them under a new request identity.
                    let class = if op.reserved {
                        WriteClass::ProtocolReplyCancel
                    } else {
                        WriteClass::Ordinary
                    };
                    let limit = if op.reserved {
                        1024 * 1024
                    } else {
                        crate::transport::writer_policy_limit()
                    };
                    match stdin.try_enqueue(OwnedFrame::new(bytes, limit), class, op.write_deadline)
                    {
                        Ok(receipt) => {
                            op.write = Some(receipt);
                        }
                        Err(rejected)
                            if rejected.error.kind() == std::io::ErrorKind::WouldBlock =>
                        {
                            // The bounded writer has not accepted any bytes. Keep
                            // the same queued frame, identity and deadline until
                            // physical capacity returns; later controls cannot pass it.
                            op.bytes = Some(rejected.frame.bytes);
                            break 'dispatch;
                        }
                        Err(rejected) => {
                            // The writer returned the original buffer/deadline without
                            // dispatch. Report NotSent; do not silently replay it or
                            // let a dependent abort escape a refused clear_queue.
                            let detail = format!("Pi write not admitted: {}", rejected.error);
                            drop(rejected);
                            let deadline_is_local = op.deadline_is_local;
                            port.evidence(*seq, PiReceiptPhase::NotSent);
                            port.remove(*seq);
                            if !deadline_is_local {
                                port.fail(detail);
                            }
                            continue;
                        }
                    }
                }
            }
        }
        port.flush_evidence(&mut output);
        if let Some(error) = &port.failed
            && !port.reported
        {
            output.push(PiPortItem::Event(
                json!({"type":"velune_error","error":error}),
            ));
            port.reported = true;
        }
        let mut records = self.pending.drain_batch(64);
        for _ in records.len()..64 {
            match self.incoming.try_recv() {
                Ok(Incoming::Record(value)) => records.push(value),
                Ok(Incoming::Error(error)) => {
                    port.fail(error);
                    break;
                }
                Ok(Incoming::Closed) | Err(mpsc::TryRecvError::Disconnected) => {
                    port.fail("Pi RPC reader closed".into());
                    break;
                }
                Err(mpsc::TryRecvError::Empty) => break,
            }
        }
        port.flush_evidence(&mut output);
        if let Some(error) = &port.failed
            && !port.reported
        {
            output.push(PiPortItem::Event(
                json!({"type":"velune_error","error":error}),
            ));
            port.reported = true;
        }
        for value in records {
            port.observe_record(value, &mut output);
        }
        output
    }
}

#[cfg(all(test, unix))]
mod tests;
