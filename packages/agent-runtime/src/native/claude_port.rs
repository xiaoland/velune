//! Claude stream-json owner port. Preparation and process cleanup remain explicit IO.
use super::*;
use std::{collections::VecDeque, time::Instant};

const CAPACITY: usize = 16;
const REPLY_RESERVE: usize = 4;
const REPLY_BYTES: usize = 64 * 1024;

/// Rust-only commands for the existing stream-json owner, after explicit preparation.
pub enum ClaudeCommand {
    Initialize,
    User(crate::TurnInput),
    Resume,
    Steer(crate::TurnInput),
    Compact,
    Interrupt,
    StopTask {
        task_id: String,
    },
    PermissionReply {
        interaction_id: String,
        reply: RuntimeInteractionReply,
    },
}

#[derive(Debug, thiserror::Error)]
pub enum ClaudeAdmissionError {
    #[error("Claude requires explicit blocking preparation")]
    NeedsPreparation,
    #[error("{0}")]
    Rejected(#[from] Error),
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ClaudeOperationTicket {
    pub session: String,
    pub generation: u64,
    pub sequence: u64,
    /// Null for user messages, which have no native wire request identity.
    pub request_id: Value,
}

#[derive(Debug)]
pub enum ClaudeDelivery {
    /// This one-way frame was written and flushed, not natively accepted.
    Written,
    /// Only a matching control_response success, never a User admission ACK.
    Accepted {
        response: Value,
    },
    Rejected {
        error: Value,
    },
    /// A side_question response is its whole answer, not admission acceptance.
    SideResult {
        response: Value,
    },
    Indeterminate {
        detail: String,
    },
}

#[derive(Debug)]
pub struct ClaudeOperationReceipt {
    pub ticket: ClaudeOperationTicket,
    pub delivery: ClaudeDelivery,
    pub physical_write: Option<crate::WriteFact>,
    /// Evidence observed after a latched failure cannot restore normal delivery.
    pub late_response: Option<Value>,
}

#[derive(Clone)]
enum Kind {
    OneWay,
    Reply(String),
    Initialize,
    Interrupt { turn_id: Option<String> },
    StopTask,
    Side,
}
struct Operation {
    ticket: ClaudeOperationTicket,
    kind: Kind,
    ack_deadline: Option<Instant>,
    physical_write: Option<crate::WriteFact>,
}
struct PendingWrite {
    receipt: crate::transport::WriteReceipt,
    bytes: usize,
    ticket: ClaudeOperationTicket,
    retention: DataRetention,
}
struct Dispatch {
    deadline: Instant,
    reserved: bool,
    write: Option<crate::transport::WriteReceipt>,
    request_id: Value,
    bytes: usize,
}
// Only the activity ended synthetically by this connection failure may be
// refined by a later native terminal. This is not a live turn or dispatch lease.
struct FailedTurnFacts {
    generation: u64,
    turn_id: String,
    activity_id: String,
    execution_id: String,
}
#[derive(Default)]
pub(super) struct Commands {
    pub(super) enabled: bool,
    pub(super) generation: u64,
    pub(super) cleanup_required: bool,
    pub(super) context_refresh_required: bool,
    // A real result record, not an API/refusal observation awaiting recovery.
    pub(super) result_is_terminal: bool,
    preparing: bool,
    prepared: bool,
    effort: Option<String>,
    permission_mode: Option<String>,
    next: u64,
    max_bytes: usize,
    bytes: usize,
    evidence_bytes: usize,
    pending: BTreeMap<u64, Operation>,
    writes: BTreeMap<u64, PendingWrite>,
    automatic: Vec<(crate::transport::WriteReceipt, usize, DataRetention)>,
    receipts: VecDeque<ClaudeOperationReceipt>,
    retention: BTreeMap<u64, DataRetention>,
    dispatch: Option<Dispatch>,
    run_deadline: Option<Instant>,
    pub(super) barrier: bool,
    failed_turn: Option<FailedTurnFacts>,
}
impl Commands {
    pub(super) fn pending(&self) -> bool {
        !self.pending.is_empty() || !self.writes.is_empty() || !self.automatic.is_empty()
    }
    fn finish(&mut self, sequence: u64, delivery: ClaudeDelivery) {
        if let Some(op) = self.pending.remove(&sequence) {
            self.receipts.push_back(ClaudeOperationReceipt {
                ticket: op.ticket,
                delivery,
                physical_write: op.physical_write,
                late_response: None,
            });
        }
    }
    pub(super) fn invalidate(&mut self, detail: &str) {
        for (_, op) in std::mem::take(&mut self.pending) {
            self.receipts.push_back(ClaudeOperationReceipt {
                ticket: op.ticket,
                delivery: ClaudeDelivery::Indeterminate {
                    detail: detail.into(),
                },
                physical_write: op.physical_write,
                late_response: None,
            });
        }
        self.prepared = false;
    }
    pub(super) fn cleanup_complete(&mut self) {
        self.writes.clear();
        self.automatic.clear();
        self.bytes = 0;
        self.run_deadline = None;
        self.failed_turn = None;
        self.result_is_terminal = false;
        self.cleanup_required = false;
    }
    fn occupied(&self) -> usize {
        self.pending.len()
            + self.receipts.len()
            + self.automatic.len()
            + self
                .writes
                .keys()
                .filter(|sequence| {
                    !self.pending.contains_key(sequence)
                        && !self
                            .receipts
                            .iter()
                            .any(|receipt| receipt.ticket.sequence == **sequence)
                })
                .count()
    }
    pub(super) fn next_generation(&mut self) -> Result<()> {
        self.invalidate("Claude connection replaced; mutation not replayed");
        self.generation = self
            .generation
            .checked_add(1)
            .ok_or_else(|| Error::new("Claude generation exhausted"))?;
        self.writes.clear();
        self.automatic.clear();
        self.bytes = 0;
        self.run_deadline = None;
        self.failed_turn = None;
        self.result_is_terminal = false;
        self.barrier = false;
        self.cleanup_required = false;
        Ok(())
    }
}
impl NativeSession {
    pub(super) fn ensure_claude_sync_mutation(&self) -> Result<()> {
        if self.claude.commands.enabled && !self.claude.commands.preparing {
            return Err(Error::with_code(
                "Claude port enabled; use typed commands or explicit blocking preparation",
                "operation_pending",
            ));
        }
        Ok(())
    }

    /// Explicit blocking IO boundary for existing create/open/side/fork/model
    /// operations. This never lends the adapter to another owner/thread. Once a
    /// port is enabled, synchronous mutators are prohibited outside this boundary.
    pub fn with_claude_blocking_preparation<T>(
        &mut self,
        prepare: impl FnOnce(&mut Self) -> Result<T>,
    ) -> Result<T> {
        if self.config.kind != NativeKind::ClaudeCode {
            return Err(Error::new("Not a Claude session"));
        }
        if self.claude.commands.pending()
            || self.turn_running()
            || self.has_pending_requests()
            || self.claude_has_ongoing_background()
            || self.rpc.has_pending_writes()
        {
            return Err(Error::with_code(
                "Claude still owns work, requests or writes",
                "outstanding_work",
            ));
        }
        self.claude.commands.prepared = false;
        self.claude.commands.preparing = true;
        let result = prepare(self);
        self.claude.commands.preparing = false;
        result
    }

    /// Blocking launch/initialize/restart boundary, including fork history and
    /// model/effort/permission launch options. max_encoded_bytes is a caller
    /// supplied retained-wire budget, not a new image/input capability limit;
    /// size it for the actual legal input plus 64 KiB reserved protocol replies.
    /// No run lifetime is inferred from the short write budget.
    pub fn prepare_claude_commands(
        &mut self,
        effort: Option<&str>,
        permission_mode: Option<&str>,
        max_encoded_bytes: usize,
    ) -> Result<()> {
        if max_encoded_bytes <= REPLY_BYTES {
            return Err(Error::new("Claude wire budget must include reply reserve"));
        }
        self.with_claude_blocking_preparation(|session| {
            session.ensure_idle()?;
            session.claude.commands.enabled = true;
            session.rpc.defer_cleanup();
            if let Err(error) = session.prepare_turn(effort, permission_mode) {
                session.claude.commands.cleanup_required = true;
                session.fail_connection();
                return Err(error);
            }
            session.claude.commands.max_bytes = max_encoded_bytes;
            session.claude.commands.effort = effort.map(str::to_owned);
            session.claude.commands.permission_mode = permission_mode.map(str::to_owned);
            session.claude.commands.prepared = true;
            Ok(())
        })
    }

    /// Explicit blocking history refresh requested by compaction projection.
    pub fn refresh_claude_context_blocking(&mut self) -> Result<()> {
        if self.config.kind != NativeKind::ClaudeCode {
            return Err(Error::new("Not a Claude session"));
        }
        let result = self.claude_read_context();
        if result.is_ok() {
            self.claude.commands.context_refresh_required = false;
        }
        result
    }
    pub fn claude_context_refresh_required(&self) -> bool {
        self.claude.commands.context_refresh_required
    }
    /// Persistent owner fact; receipt consumption and parent completion do not
    /// release the child, HOME, tunnel or MCP leases. Call shutdown explicitly.
    pub fn claude_cleanup_required(&self) -> bool {
        self.claude.commands.cleanup_required || self.requires_failure_cleanup()
    }

    /// Local admission only. Poll continues projection while stdin or ACKs are
    /// pending. write_deadline also bounds control ACKs; optional run_deadline is
    /// one absolute turn deadline, retained after Written (never renewed).
    pub fn enqueue_claude_command(
        &mut self,
        command: ClaudeCommand,
        write_deadline: Instant,
        run_deadline: Option<Instant>,
    ) -> std::result::Result<Retained<ClaudeOperationTicket>, ClaudeAdmissionError> {
        if self.config.kind != NativeKind::ClaudeCode {
            return Err(Error::new("Not a Claude session").into());
        }
        if !self.claude.commands.enabled
            || self.claude.commands.preparing
            || self.failed
            || self.process_ended
            || self.claude.commands.barrier
        {
            return Err(ClaudeAdmissionError::NeedsPreparation);
        }
        if self.side && matches!(command, ClaudeCommand::Interrupt) {
            self.claude.commands.cleanup_required = true;
            return Err(ClaudeAdmissionError::NeedsPreparation);
        }
        let starts = matches!(
            command,
            ClaudeCommand::User(_) | ClaudeCommand::Resume | ClaudeCommand::Compact
        );
        let reuses_background_child = matches!(&command, ClaudeCommand::User(input)
            if !self.restart_needed_for_send(input.effort.as_deref(), input.permission_mode.as_deref()));
        if starts
            && (!self.claude.commands.prepared
                || self.claude.bound_session_id != self.native_id
                || self.restart_required
                || (self.claude.release_when_idle && !reuses_background_child))
        {
            return Err(ClaudeAdmissionError::NeedsPreparation);
        }
        if run_deadline.is_some() && !starts {
            return Err(Error::new("Run deadline belongs to an explicit new turn").into());
        }
        let reserved = matches!(
            command,
            ClaudeCommand::PermissionReply { .. }
                | ClaudeCommand::Interrupt
                | ClaudeCommand::StopTask { .. }
        );
        let count = self.claude.commands.occupied();
        if count
            >= if reserved {
                CAPACITY
            } else {
                CAPACITY - REPLY_RESERVE
            }
        {
            return Err(
                Error::with_code("Claude command/evidence capacity full", "queue_full").into(),
            );
        }
        let deadline = run_deadline.map_or(write_deadline, |run| run.min(write_deadline));
        if deadline <= Instant::now() {
            return Err(
                Error::with_code("Claude deadline expired; not dispatched", "timeout").into(),
            );
        }
        // Observe already recorded failure before dispatch, without consuming or
        // waiting for protocol records and without process cleanup.
        if let Err(error) = self
            .claude_settle_barrier()
            .and_then(|()| self.rpc.check_failure())
        {
            self.claude.commands.barrier = true;
            self.claude_fail_connection();
            return Err(error.into());
        }
        let sequence = self
            .claude
            .commands
            .next
            .checked_add(1)
            .ok_or_else(|| Error::new("Claude operation sequence exhausted"))?;
        let admission_retention = self.data.try_clone(super::retention::command_data_owner)?;
        let public_retention = admission_retention.try_clone().map_err(retention_error)?;
        let writer_retention = admission_retention.try_clone().map_err(retention_error)?;
        let wire_id = format!(
            "velune-port-{}-{}-{sequence}",
            self.epoch, self.claude.commands.generation
        );
        self.claude.commands.dispatch = Some(Dispatch {
            deadline,
            reserved,
            write: None,
            request_id: Value::Null,
            bytes: 0,
        });
        let result = self.claude_dispatch_command(command, &wire_id);
        let dispatch = self
            .claude
            .commands
            .dispatch
            .take()
            .expect("Claude dispatch scope");
        let kind = match result {
            Ok(kind) => kind,
            Err(error) => {
                if error.code() == "transport" {
                    self.claude.commands.barrier = true;
                    self.claude.commands.cleanup_required = true;
                    self.claude_fail_connection();
                }
                return Err(error.into());
            }
        };
        let control = matches!(
            kind,
            Kind::Initialize | Kind::Interrupt { .. } | Kind::StopTask
        );
        let ticket = ClaudeOperationTicket {
            session: self.epoch.clone(),
            generation: self.claude.commands.generation,
            sequence,
            request_id: dispatch.request_id,
        };
        if let Some(write) = dispatch.write {
            self.claude.commands.writes.insert(
                sequence,
                PendingWrite {
                    receipt: write,
                    bytes: dispatch.bytes,
                    ticket: ticket.clone(),
                    retention: writer_retention,
                },
            );
        }
        self.claude
            .commands
            .retention
            .insert(sequence, admission_retention);
        self.claude.commands.pending.insert(
            sequence,
            Operation {
                ticket: ticket.clone(),
                kind,
                ack_deadline: control.then_some(deadline),
                physical_write: None,
            },
        );
        self.claude.commands.next = sequence;
        if starts {
            self.claude.commands.prepared = false;
            self.claude.commands.run_deadline = run_deadline;
        }
        Ok(Retained::from_parts(ticket, public_retention))
    }

    fn claude_dispatch_command(&mut self, command: ClaudeCommand, wire_id: &str) -> Result<Kind> {
        let control = |subtype: &str| json!({"type":"control_request","request_id":wire_id,"request":{"subtype":subtype}});
        match command {
            ClaudeCommand::Initialize => {
                self.claude_write(&json!({"type":"control_request","request_id":wire_id,"request":{"subtype":"initialize","hooks":{}}}))?;
                Ok(Kind::Initialize)
            }
            ClaudeCommand::User(input) => {
                if input.text.trim().is_empty() && input.images.is_empty() {
                    return Err(Error::new("Empty message"));
                }
                self.claude_admit_user(&input)?;
                Ok(if self.side { Kind::Side } else { Kind::OneWay })
            }
            ClaudeCommand::Resume => {
                if self.side {
                    return Err(Error::new("Side context requires a question"));
                }
                self.claude_admit_user(&crate::TurnInput {
                    text: String::new(),
                    images: vec![],
                    effort: None,
                    permission_mode: None,
                })?;
                Ok(Kind::OneWay)
            }
            ClaudeCommand::Steer(input) => {
                if !self.can_steer() || (input.text.trim().is_empty() && input.images.is_empty()) {
                    return Err(Error::new("No steerable turn or empty message"));
                }
                self.claude_steer(&input)?;
                Ok(Kind::OneWay)
            }
            ClaudeCommand::Compact => {
                self.claude_admit_idle(false)?;
                if self.side {
                    return Err(Error::new("Side context cannot compact"));
                }
                self.claude_compact()?;
                self.compacting = true;
                if let Some(snapshot) = &mut self.snapshot {
                    snapshot
                        .executions
                        .begin(ExecutionKind::ContextMaintenance, false);
                }
                Ok(Kind::OneWay)
            }
            ClaudeCommand::Interrupt => {
                if self.side {
                    return Err(Error::with_code(
                        "Side cancellation requires explicit blocking shutdown",
                        "needs_preparation",
                    ));
                }
                if !self.turn_running() {
                    return Err(Error::new("No running Claude turn"));
                }
                self.claude_write(&control("interrupt"))?;
                if let Some(snapshot) = &mut self.snapshot {
                    snapshot.run_state = RunState::Stopping;
                    snapshot.revision += 1;
                }
                Ok(Kind::Interrupt {
                    turn_id: self.turn_id.clone(),
                })
            }
            ClaudeCommand::StopTask { task_id } => {
                let native = self.claude_stop_target(&task_id)?;
                self.claude_write(&json!({"type":"control_request","request_id":wire_id,"request":{"subtype":"stop_task","task_id":native}}))?;
                self.claude_mark_task_stopping(wire_id.into(), &task_id);
                Ok(Kind::StopTask)
            }
            ClaudeCommand::PermissionReply {
                interaction_id,
                reply,
            } => {
                if self
                    .claude
                    .commands
                    .pending
                    .values()
                    .any(|op| matches!(&op.kind, Kind::Reply(id) if id == &interaction_id))
                {
                    return Err(Error::new("Claude reply already admitted; not replayed"));
                }
                let _copy_retention = self.data.try_clone(
                    |owner| matches!(owner, DataOwner::Interaction(id) if id == &interaction_id),
                )?;
                let pending = self
                    .interactions
                    .get(&interaction_id)
                    .cloned()
                    .ok_or_else(|| Error::new("Interaction expired"))?;
                self.claude_reply(&pending, reply)?;
                Ok(Kind::Reply(interaction_id))
            }
        }
    }
    fn claude_admit_idle(&self, reuse_background: bool) -> Result<()> {
        if self.turn_running()
            || self.has_pending_requests()
            || (!reuse_background && self.claude_has_ongoing_background())
        {
            return Err(Error::with_code(
                "Claude still owns live work or interaction",
                "outstanding_work",
            ));
        }
        if self.native_id.is_none() {
            return Err(Error::new("No Claude session"));
        }
        Ok(())
    }
    fn claude_admit_user(&mut self, input: &crate::TurnInput) -> Result<()> {
        let reuse_background = !self
            .restart_needed_for_send(input.effort.as_deref(), input.permission_mode.as_deref());
        self.claude_admit_idle(reuse_background)?;
        // Options must be captured at the explicitly blocking launch boundary.
        if input
            .effort
            .as_ref()
            .is_some_and(|effort| Some(effort) != self.claude.commands.effort.as_ref())
            || input
                .permission_mode
                .as_ref()
                .is_some_and(|mode| Some(mode) != self.claude.commands.permission_mode.as_ref())
        {
            return Err(Error::with_code(
                "Pass launch options to prepare_claude_commands",
                "needs_preparation",
            ));
        }
        self.claude_send(input)?;
        if let Some(snapshot) = &mut self.snapshot {
            snapshot.executions.begin(
                ExecutionKind::UserTurn,
                input.text.is_empty() && input.images.is_empty(),
            );
            if let Some(message) = snapshot
                .messages
                .last()
                .filter(|message| message.role == MessageRole::User)
            {
                snapshot.executions.attach(&message.id);
            }
        }
        Ok(())
    }

    /// Common writer seam used by the existing projection-preserving mutators.
    /// Only the typed dispatch scope can enqueue a caller mutation in port mode.
    fn claude_dispatch_deadline(&self, requested: Instant) -> Instant {
        self.claude
            .commands
            .pending
            .values()
            .filter_map(|op| op.ack_deadline)
            .chain(self.claude.commands.run_deadline)
            .fold(requested, std::cmp::min)
    }

    pub(super) fn claude_write(&mut self, frame: &Value) -> Result<()> {
        self.claude_write_with_class(frame, crate::transport::WriteClass::Ordinary)
    }
    pub(super) fn claude_write_with_class(
        &mut self,
        frame: &Value,
        legacy_class: crate::transport::WriteClass,
    ) -> Result<()> {
        if !self.claude.commands.enabled || self.claude.commands.preparing {
            return self.rpc.write_with_class(frame, legacy_class);
        }
        self.claude_settle_barrier()?;
        if self.claude.commands.barrier {
            return Err(Error::with_code(
                "Claude failure dispatch barrier",
                "transport",
            ));
        }
        let dispatch = self
            .claude
            .commands
            .dispatch
            .as_ref()
            .ok_or_else(|| Error::new("Claude mutation outside typed dispatch"))?;
        let limit = self
            .claude
            .commands
            .max_bytes
            .saturating_sub(if dispatch.reserved { 0 } else { REPLY_BYTES });
        let available = limit.saturating_sub(self.claude.commands.bytes);
        let request_id = frame
            .get("request_id")
            .or_else(|| frame["response"].get("request_id"))
            .cloned()
            .unwrap_or(Value::Null);
        let evidence_bytes = serde_json::to_vec(&request_id)
            .map_err(|error| Error::new(error.to_string()))?
            .len();
        if self
            .claude
            .commands
            .evidence_bytes
            .saturating_add(evidence_bytes)
            > self.claude.commands.max_bytes
        {
            return Err(Error::with_code(
                "Claude identity evidence capacity full; not dispatched",
                "queue_full",
            ));
        }
        let (write, bytes) = self.rpc.enqueue_claude_frame_receipt(
            frame,
            if dispatch.reserved {
                crate::transport::WriteClass::ProtocolReplyCancel
            } else {
                crate::transport::WriteClass::Ordinary
            },
            self.claude_dispatch_deadline(dispatch.deadline),
            available,
        )?;
        self.claude.commands.evidence_bytes += evidence_bytes;
        let dispatch = self
            .claude
            .commands
            .dispatch
            .as_mut()
            .expect("Claude dispatch");
        dispatch.request_id = request_id;
        dispatch.write = Some(write);
        dispatch.bytes = bytes;
        self.claude.commands.bytes += bytes;
        Ok(())
    }

    pub(super) fn claude_automatic_reply(&mut self, frame: &Value) -> Result<()> {
        self.claude_settle_barrier()?;
        if self.claude.commands.barrier {
            return Ok(());
        }
        if !self.claude.commands.enabled {
            return self.rpc.enqueue_claude_automatic_reply(frame);
        }
        if self.claude.commands.occupied() >= CAPACITY {
            return Err(Error::with_code(
                "Claude automatic reply capacity full",
                "queue_full",
            ));
        }
        // Unknown inbound requests have no known parent. Do not infer one from
        // an unrelated pending command or the current turn's optional deadline.
        let deadline = Instant::now() + rpc::CONTROL_TIMEOUT;
        let available = self
            .claude
            .commands
            .max_bytes
            .saturating_sub(self.claude.commands.bytes);
        let (receipt, bytes) = self.rpc.enqueue_claude_frame_receipt(
            frame,
            crate::transport::WriteClass::ProtocolReplyCancel,
            self.claude_dispatch_deadline(deadline),
            available,
        )?;
        self.claude.commands.bytes += bytes;
        self.claude
            .commands
            .automatic
            .push((receipt, bytes, self.data.branch()));
        Ok(())
    }

    pub fn take_claude_receipts(&mut self) -> Vec<Retained<ClaudeOperationReceipt>> {
        self.claude.commands.evidence_bytes = self
            .claude
            .commands
            .pending
            .values()
            .map(|op| op.ticket.request_id.to_string().len())
            .sum();
        self.claude
            .commands
            .receipts
            .drain(..)
            .map(|receipt| {
                let retention = self
                    .claude
                    .commands
                    .retention
                    .remove(&receipt.ticket.sequence)
                    .unwrap_or_default();
                Retained::from_parts(receipt, retention)
            })
            .collect()
    }

    pub(super) fn claude_harvest_writes(&mut self) -> Result<()> {
        use crate::transport::WriteOutcome;
        let mut failure = None;
        let mut facts = Vec::new();
        for (sequence, write) in &self.claude.commands.writes {
            let Some(outcome) = write.receipt.try_outcome() else {
                continue;
            };
            let fact = match outcome {
                WriteOutcome::Written(completion) => {
                    if !completion.met_deadline() {
                        failure.get_or_insert_with(|| {
                            Error::with_code("Claude physically Written after deadline", "timeout")
                        });
                    }
                    Some(crate::WriteFact::Written(completion))
                }
                WriteOutcome::NotWritten(_) => {
                    failure.get_or_insert_with(|| {
                        Error::with_code("Claude known not written", "transport")
                    });
                    Some(crate::WriteFact::NotWritten)
                }
                WriteOutcome::Indeterminate(_) => {
                    failure.get_or_insert_with(|| {
                        Error::with_code("Claude physical write indeterminate", "transport")
                    });
                    Some(crate::WriteFact::Indeterminate)
                }
                WriteOutcome::ObservationTimeout { .. } => {
                    failure.get_or_insert_with(|| {
                        Error::with_code(
                            "Claude write not yet observed; receipt retained",
                            "timeout",
                        )
                    });
                    None
                }
            };
            if let Some(fact) = fact {
                facts.push((*sequence, fact));
            }
        }
        for (sequence, fact) in facts {
            let write = self.claude.commands.writes.remove(&sequence).unwrap();
            self.claude.commands.bytes = self.claude.commands.bytes.saturating_sub(write.bytes);
            let old = self
                .claude
                .commands
                .retention
                .remove(&sequence)
                .unwrap_or_default();
            self.claude
                .commands
                .retention
                .insert(sequence, old.merge(write.retention));
            if let Some(op) = self.claude.commands.pending.get_mut(&sequence) {
                op.physical_write = Some(fact);
            }
            let timely = matches!(fact, crate::WriteFact::Written(c) if c.met_deadline());
            let one_way = self
                .claude
                .commands
                .pending
                .get(&sequence)
                .is_some_and(|op| matches!(op.kind, Kind::OneWay | Kind::Reply(_)));
            if timely && one_way && !self.claude.commands.barrier {
                if let Some(Operation {
                    kind: Kind::Reply(id),
                    ..
                }) = self.claude.commands.pending.get(&sequence)
                {
                    let id = id.clone();
                    self.interactions.remove(&id);
                    if let Some(snapshot) = &mut self.snapshot {
                        snapshot
                            .pending_interactions
                            .retain(|interaction| interaction.id != id);
                        snapshot.retained_reply_ids.push(id);
                        snapshot.revision += 1;
                    }
                }
                self.claude
                    .commands
                    .finish(sequence, ClaudeDelivery::Written);
            }
            if let Some(receipt) = self
                .claude
                .commands
                .receipts
                .iter_mut()
                .find(|r| r.ticket == write.ticket)
            {
                receipt.physical_write = Some(fact);
            } else if !timely || self.claude.commands.barrier {
                // A prior failure receipt may already have been consumed. This is
                // physical evidence only, never a revived Accepted/Written success.
                self.claude
                    .commands
                    .receipts
                    .push_back(ClaudeOperationReceipt {
                        ticket: write.ticket,
                        delivery: ClaudeDelivery::Indeterminate {
                            detail: "Physical write settled after failure; not replayed".into(),
                        },
                        physical_write: Some(fact),
                        late_response: None,
                    });
            }
        }
        let mut index = 0;
        while index < self.claude.commands.automatic.len() {
            let Some(outcome) = self.claude.commands.automatic[index].0.try_outcome() else {
                index += 1;
                continue;
            };
            match outcome {
                WriteOutcome::Written(c) if c.met_deadline() => {}
                WriteOutcome::ObservationTimeout { .. } => {
                    failure.get_or_insert_with(|| {
                        Error::with_code("Claude automatic write not yet observed", "timeout")
                    });
                    index += 1;
                    continue;
                }
                _ => {
                    failure.get_or_insert_with(|| {
                        Error::with_code("Claude automatic write failed/late", "transport")
                    });
                }
            }
            let (_, bytes, _) = self.claude.commands.automatic.remove(index);
            self.claude.commands.bytes = self.claude.commands.bytes.saturating_sub(bytes);
        }
        if let Some(error) = failure {
            return Err(error);
        }
        Ok(())
    }

    pub(super) fn claude_settle_barrier(&mut self) -> Result<()> {
        let failure = self
            .claude_harvest_writes()
            .err()
            .or_else(|| self.claude_check_deadlines().err());
        if let Some(error) = failure {
            self.claude.commands.barrier = true;
            self.claude.commands.cleanup_required = true;
            self.claude.commands.invalidate(&error.to_string());
            return Err(error);
        }
        Ok(())
    }

    fn claude_fail_connection(&mut self) {
        if !self.failed
            && let Some(turn_id) = &self.turn_id
            && let Some(snapshot) = &self.snapshot
            && let Some(activity) = &snapshot.executions.current
            && !activity.ended
            && snapshot
                .executions
                .records
                .iter()
                .any(|record| record.id == activity.execution_id && record.outcome.is_none())
        {
            self.claude.commands.failed_turn = Some(FailedTurnFacts {
                generation: self.claude.commands.generation,
                turn_id: turn_id.clone(),
                activity_id: activity.id.clone(),
                execution_id: activity.execution_id.clone(),
            });
        }
        self.claude.commands.cleanup_required = true;
        self.fail_connection();
    }

    pub(super) fn claude_has_failed_turn(&self) -> bool {
        self.failed
            && self
                .claude
                .commands
                .failed_turn
                .as_ref()
                .is_some_and(|owner| {
                    owner.generation == self.claude.commands.generation
                        && self.turn_id.as_ref().is_none_or(|id| id == &owner.turn_id)
                        && self.snapshot.as_ref().is_some_and(|snapshot| {
                            snapshot
                                .executions
                                .current
                                .as_ref()
                                .is_some_and(|activity| {
                                    activity.ended
                                        && activity.id == owner.activity_id
                                        && activity.execution_id == owner.execution_id
                                })
                                && snapshot.executions.records.iter().any(|record| {
                                    record.id == owner.execution_id
                                        && record.outcome == Some(TurnOutcome::Failed)
                                })
                        })
                })
    }

    pub(super) fn claude_preserves_terminal_result(&self) -> bool {
        self.claude.commands.barrier
            && self.claude.commands.result_is_terminal
            && self.claude.result.is_some()
            && (self.turn_running() || self.claude_has_failed_turn())
    }

    pub(super) fn claude_received_turn_id(&self) -> Option<&str> {
        self.turn_id.as_deref().or_else(|| {
            self.claude_has_failed_turn().then(|| {
                self.claude
                    .commands
                    .failed_turn
                    .as_ref()
                    .unwrap()
                    .turn_id
                    .as_str()
            })
        })
    }

    pub(super) fn claude_finish_received_outcome(&mut self, outcome: TurnOutcome) {
        self.claude.commands.result_is_terminal = false;
        if self.claude_has_failed_turn() {
            let owner = self.claude.commands.failed_turn.take().unwrap();
            if let Some(snapshot) = &mut self.snapshot {
                snapshot.executions.refine_connection_failure(
                    &owner.activity_id,
                    &owner.execution_id,
                    outcome,
                );
            }
        } else if !self.failed
            && let Some(snapshot) = &mut self.snapshot
        {
            snapshot.executions.finish(Some(outcome));
        }
    }

    // A received terminal remains authoritative after dispatch/acceptance fails.
    // Observe its remaining budget before settling, but retain the failed owner.
    pub(super) fn claude_finish_received_turn(&mut self, state: RunState) -> Result<()> {
        if !self.claude.commands.enabled {
            return self.finish_turn(state);
        }
        let result = self.claude_settle_barrier();
        self.settle(if self.claude.commands.barrier {
            RunState::Failed
        } else {
            state
        });
        result
    }

    fn claude_port_response(&mut self, record: &Value) -> Result<()> {
        if record["type"] != "control_response" {
            return Ok(());
        }
        let response = &record["response"];
        let id = &response["request_id"];
        let sequence = self
            .claude
            .commands
            .pending
            .iter()
            .find_map(|(sequence, op)| {
                (op.ticket.request_id == *id
                    && !id.is_null()
                    && op.ticket.generation == self.claude.commands.generation)
                    .then_some(*sequence)
            });
        let Some(sequence) = sequence else {
            if let Some(receipt) = self.claude.commands.receipts.iter_mut().find(|receipt| {
                receipt.ticket.generation == self.claude.commands.generation
                    && receipt.ticket.request_id == *id
                    && matches!(receipt.delivery, ClaudeDelivery::Indeterminate { .. })
            }) {
                // Retain bounded late evidence; repeated duplicates are ignored.
                if receipt.late_response.is_none() {
                    let bytes = serde_json::to_vec(response)
                        .map_err(|e| Error::new(e.to_string()))?
                        .len();
                    if self.claude.commands.evidence_bytes.saturating_add(bytes)
                        > self.claude.commands.max_bytes
                    {
                        return Err(Error::new("Claude late evidence byte capacity full"));
                    }
                    self.claude.commands.evidence_bytes += bytes;
                    let piece = self.data.branch();
                    let sequence = receipt.ticket.sequence;
                    let old = self
                        .claude
                        .commands
                        .retention
                        .remove(&sequence)
                        .unwrap_or_default();
                    self.claude
                        .commands
                        .retention
                        .insert(sequence, old.merge(piece));
                    receipt.late_response = Some(response.clone());
                }
            }
            return Ok(());
        };
        let piece = self.data.branch();
        let old = self
            .claude
            .commands
            .retention
            .remove(&sequence)
            .unwrap_or_default();
        self.claude
            .commands
            .retention
            .insert(sequence, old.merge(piece));
        let bytes = serde_json::to_vec(response)
            .map_err(|e| Error::new(e.to_string()))?
            .len();
        if self.claude.commands.evidence_bytes.saturating_add(bytes)
            > self.claude.commands.max_bytes
        {
            return Err(Error::new("Claude evidence byte capacity full"));
        }
        self.claude.commands.evidence_bytes += bytes;
        if let Err(error) = self.claude_settle_barrier() {
            self.claude_port_response(record)?;
            return Err(error);
        }
        let Some(op) = self.claude.commands.pending.get(&sequence) else {
            return self.claude_port_response(record);
        };
        let kind = op.kind.clone();
        if matches!(kind, Kind::OneWay | Kind::Reply(_)) {
            return Ok(());
        }
        if !matches!(response["subtype"].as_str(), Some("success" | "error")) {
            return Err(Error::with_code(
                "Malformed Claude control response",
                "transport",
            ));
        }
        let success = response["subtype"] == "success";
        let models = if success && matches!(kind, Kind::Initialize) {
            let retention = self.data.try_clone_current()?;
            Some((retention, {
                response["response"]["models"]
                    .as_array()
                    .cloned()
                    .unwrap_or_default()
            }))
        } else {
            None
        };
        let delivery = if matches!(kind, Kind::Side) {
            ClaudeDelivery::SideResult {
                response: response.clone(),
            }
        } else if success {
            ClaudeDelivery::Accepted {
                response: response["response"].clone(),
            }
        } else {
            ClaudeDelivery::Rejected {
                error: response["error"].clone(),
            }
        };
        // Cloning a large native result/catalog may cross D. Only install it
        // after the last absolute barrier, while its operation is still live.
        if let Err(error) = self.claude_settle_barrier() {
            self.claude_port_response(record)?;
            return Err(error);
        }
        if let Some((retention, models)) = models {
            self.data.insert(DataOwner::Catalog, retention);
            self.available_models = models;
        }
        if !success
            && let Kind::Interrupt { turn_id } = &kind
            && *turn_id == self.turn_id
            && self.turn_running()
            && let Some(snapshot) = &mut self.snapshot
        {
            snapshot.run_state = RunState::Running;
            snapshot.revision += 1;
        }
        self.claude.commands.finish(sequence, delivery);
        Ok(())
    }

    fn claude_check_deadlines(&self) -> Result<()> {
        let now = Instant::now();
        if self
            .claude
            .commands
            .pending
            .values()
            .any(|op| op.ack_deadline.is_some_and(|deadline| now >= deadline))
            // Retained until the batch is checked: projecting idle must not
            // erase the deadline applicable to its received terminal record.
            || self
                .claude
                .commands
                .run_deadline
                .is_some_and(|deadline| now >= deadline)
        {
            return Err(Error::with_code(
                "Claude absolute deadline expired; not replayed",
                "timeout",
            ));
        }
        Ok(())
    }

    pub(super) fn claude_poll_port(&mut self) -> Result<()> {
        if self.process_ended {
            return Ok(());
        }
        let revision = self.snapshot.as_ref().map(|snapshot| snapshot.revision);
        let mut failure = self.claude_settle_barrier().err();
        if let Err(error) = self.claude_check_deadlines() {
            failure = Some(error);
        }
        // Latch expiry before consuming this batch. A late ACK may add outcome
        // evidence, but must never turn an expired operation into Accepted.
        if failure.is_some() {
            self.claude.commands.barrier = true;
            self.claude
                .commands
                .invalidate("Claude deadline/write failure; not replayed");
        }
        let records = match self.rpc.poll() {
            Ok(records) => records,
            Err(error) => {
                failure = Some(error);
                vec![]
            }
        };
        if let Err(error) = self.rpc.check_failure() {
            failure = Some(error);
        }
        if failure.is_some() {
            self.claude.commands.barrier = true;
            self.claude
                .commands
                .invalidate("Claude transport failure; not replayed");
        }
        for record in records {
            if let Err(error) = self.claude_settle_barrier() {
                failure.get_or_insert(error);
            }
            let result = self.receive_native_record(record, |session, record| {
                let response = session.claude_port_response(&record);
                if response.is_err() {
                    session.claude.commands.barrier = true;
                    session
                        .claude
                        .commands
                        .invalidate("Claude batch failure barrier; not replayed");
                }
                let projection = session.claude_handle_record(record);
                response.and(projection)
            });
            if let Err(error) = result {
                failure = Some(error);
                self.claude.commands.barrier = true;
                self.claude
                    .commands
                    .invalidate("Claude batch failure barrier; not replayed");
            }
        }
        if let Err(error) = self.claude_harvest_writes() {
            failure = Some(error);
        }
        self.claude_expire_background_controls();
        self.claude_check_terminal_timeout();
        if let Err(error) = self.claude_settle_barrier() {
            failure.get_or_insert(error);
        }
        if !self.turn_running() && !self.claude.commands.barrier {
            self.claude.commands.run_deadline = None;
        }
        self.end_spent_process();
        if let Some(snapshot) = &mut self.snapshot {
            if revision != Some(snapshot.revision) {
                snapshot
                    .executions
                    .confirm(&snapshot.message_identity_confirmations);
                snapshot.executions.retain(&snapshot.messages);
            }
            if self.failed {
                snapshot.run_state = RunState::Failed;
                snapshot.actions.can_send = false;
                snapshot.actions.can_cancel = false;
            }
        }
        if let Some(error) = failure {
            self.claude.commands.barrier = true;
            self.claude.commands.cleanup_required = true;
            if !self.failed {
                self.claude_fail_connection();
            }
            return Err(error);
        }
        Ok(())
    }
}

#[cfg(all(test, unix))]
mod integration_tests {
    use super::*;
    use std::time::Duration;
    #[test]
    fn claude_integration_late_written_fact_survives_consumed_failure_receipt() {
        let mut fixture = super::super::claude_port_tests::Fixture::new("hold");
        let d = Instant::now() - Duration::from_millis(1);
        let (sender, receipt) = crate::transport::WriteReceipt::test_pending(d);
        let ticket = ClaudeOperationTicket {
            session: fixture.session.epoch.clone(),
            generation: fixture.session.claude.commands.generation,
            sequence: 100,
            request_id: Value::Null,
        };
        fixture.session.claude.commands.pending.insert(
            100,
            Operation {
                ticket: ticket.clone(),
                kind: Kind::OneWay,
                ack_deadline: None,
                physical_write: None,
            },
        );
        fixture.session.claude.commands.writes.insert(
            100,
            PendingWrite {
                receipt,
                bytes: 0,
                ticket: ticket.clone(),
                retention: DataRetention::default(),
            },
        );
        assert!(fixture.session.claude_settle_barrier().is_err());
        fixture.session.take_claude_receipts();
        sender
            .send(crate::transport::WriteOutcome::Written(
                crate::WriteCompletion {
                    started_at: d - Duration::from_millis(1),
                    completed_at: d + Duration::from_millis(1),
                    deadline: d,
                },
            ))
            .unwrap();
        assert!(fixture.session.claude_settle_barrier().is_err());
        let receipts = fixture.session.take_claude_receipts();
        assert!(receipts.iter().any(|r| r.ticket==ticket && matches!(r.delivery, ClaudeDelivery::Indeterminate { .. }) && matches!(r.physical_write, Some(crate::WriteFact::Written(c)) if !c.met_deadline())));
        assert!(fixture.session.claude_cleanup_required());
    }
}
