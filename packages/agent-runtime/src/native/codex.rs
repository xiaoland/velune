//! Codex app-server control and projection, independent of Pi.
use super::*;
pub(super) const NATIVE_RETRY_DETAIL: &str = "\nCodex 正在自动重试。";

/// What only a Codex app-server session tracks.
#[derive(Default)]
pub(super) struct State {
    pub(super) title: super::codex_title::Title,
    pub(super) terminals: super::codex_background::Terminals,
    pub(super) commands: Commands,
    pub(super) handoff_summary: Option<super::codex_handoff_summary::Job>,
    pub(super) handoff_summary_hidden: std::collections::BTreeSet<String>,
    pub(super) retry: super::codex_retry::Retry,
    pub(super) cancel_pending: bool,
    /// Accepted native async answers remain settled when an item is repeated.
    /// Cold history reconstructs this from its ordered native answer records.
    pub(super) resolved_async_questions: std::collections::BTreeSet<String>,
    /// Last error of the active turn, if its completion omits details.
    pub(super) turn_error: Option<Value>,
    /// Threads of this app-server besides the conversation's own, by thread
    /// id, while they work.
    pub(super) threads: BTreeMap<String, CodexThread>,
    /// The developer instructions Codex's configuration gives, which a side
    /// chat's own follow.
    pub(super) developer_instructions: Option<String>,
    pub(super) side_context: Option<String>,
    /// The speed tier of the selected speed variant, sent per turn; `None`
    /// is Codex's own default.
    pub(super) service_tier: Option<String>,
}

/// Rust-only commands for an initialized Codex connection. `Start` creates a
/// thread using the cwd/config captured by `prepare_codex_commands`; `Resume`
/// starts an empty-input turn, not a history load or a connection bootstrap.
pub enum CodexCommand {
    Start {
        runtime_id: String,
    },
    Send(crate::TurnInput),
    Resume {
        effort: Option<String>,
        permission_mode: Option<String>,
    },
    /// Requires a natively identified, steerable running turn.
    Steer(crate::TurnInput),
    /// Requires a natively identified running turn. Before turn/started or the
    /// start result supplies its id (or during retry backoff), admission rejects
    /// without dispatch; the owner may request cancellation once identity exists.
    Cancel,
    /// Interrupt only the currently identified native background turns. Each
    /// child has an independent delivery result; successful ACKs are not work
    /// completion. An empty target set is reported as an empty result set.
    StopBackground,
    /// Exact public runtime:thread identity from this connection's work ledger.
    StopChild {
        conversation_id: String,
    },
    Reply {
        interaction_id: String,
        reply: RuntimeInteractionReply,
    },
}

/// Local admission failure: no frame for this command was dispatched. In
/// particular, NeedsPreparation never performs bootstrap, filesystem or RPC IO.
#[derive(Debug, thiserror::Error)]
pub enum CodexAdmissionError {
    #[error("Codex requires synchronous preparation at the initialization boundary")]
    NeedsPreparation,
    #[error("{0}")]
    Rejected(#[from] Error),
}

/// Correlation belongs to one NativeSession, one connection generation and one
/// checked local sequence. `request_id` is the actual client/server JSON-RPC id,
/// or null for an aggregate whose child receipts retain their individual ids.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct CodexOperationTicket {
    pub session: String,
    pub generation: u64,
    pub sequence: u64,
    pub request_id: Value,
}

#[derive(Debug)]
pub struct CodexOperationReceipt {
    pub ticket: CodexOperationTicket,
    pub delivery: CodexDelivery,
    pub physical_write: Option<crate::WriteFact>,
}

#[derive(Debug)]
pub enum CodexDelivery {
    /// A native RPC result, not turn completion (including for cancel).
    Accepted {
        thread_id: Option<String>,
        turn_id: Option<String>,
    },
    /// A one-way reply was written and flushed. This is NOT evidence the
    /// server applied it or that its approval/business operation completed.
    Written,
    /// An explicit native JSON-RPC error, distinct from local non-dispatch.
    Rejected { error: Value },
    /// Delivery cannot be proved. Never replay this mutation automatically.
    Indeterminate { detail: String },
    /// Aggregate interrupt operation, including partial acceptance. Never
    /// flatten this to an all-or-nothing rejected or unsent result.
    Interrupts {
        children: Vec<CodexInterruptReceipt>,
    },
}

#[derive(Debug)]
pub struct CodexInterruptReceipt {
    pub thread_id: String,
    pub turn_id: String,
    /// None means this child frame never entered the writer.
    pub request_id: Option<u64>,
    pub delivery: CodexInterruptDelivery,
}
#[derive(Debug)]
pub enum CodexInterruptDelivery {
    Accepted,
    Rejected { error: Value },
    Indeterminate { detail: String },
    NotDispatched,
}

const MAX_INTERRUPT_TARGETS: usize = 64;
const COMMAND_CAPACITY: usize = 16;
const REPLY_RESERVE: usize = 4;

#[derive(Default)]
pub(super) struct Commands {
    pub(super) enabled: bool,
    generation: u64,
    next: u64,
    start_cwd: Option<Option<PathBuf>>,
    pending: BTreeMap<u64, CommandOperation>,
    receipts: std::collections::VecDeque<CodexOperationReceipt>,
    retention: BTreeMap<u64, DataRetention>,
    late_retention: BTreeMap<u64, DataRetention>,
    early: super::retention::RetainedRecords,
    failed_writes: BTreeMap<
        u64,
        (
            Retained<CodexOperationTicket>,
            crate::transport::WriteReceipt,
        ),
    >,
}
struct CommandOperation {
    ticket: CodexOperationTicket,
    deadline: std::time::Instant,
    kind: CommandOperationKind,
}
struct CommandRetention {
    command: DataRetention,
    public_ticket: DataRetention,
    late_write: DataRetention,
}
enum CommandOperationKind {
    Start {
        cwd: Option<PathBuf>,
        runtime_id: String,
    },
    Send {
        title: String,
        started: Option<String>,
        ended: bool,
        resuming: bool,
        effort: Option<String>,
        permission_mode: Option<String>,
    },
    Steer {
        turn: String,
    },
    Cancel {
        turn: String,
    },
    Reply {
        interaction_id: String,
        write: crate::transport::WriteReceipt,
    },
    Interrupts(Interrupts),
}
struct InterruptTarget {
    thread: String,
    turn: String,
}
struct Interrupts {
    waiting: std::collections::VecDeque<InterruptTarget>,
    pending: BTreeMap<u64, InterruptTarget>,
    settled: Vec<CodexInterruptReceipt>,
}
impl InterruptTarget {
    fn receipt(
        self,
        request_id: Option<u64>,
        delivery: CodexInterruptDelivery,
    ) -> CodexInterruptReceipt {
        CodexInterruptReceipt {
            thread_id: self.thread,
            turn_id: self.turn,
            request_id,
            delivery,
        }
    }
}
impl Interrupts {
    fn delivery(mut self, failure: &str) -> CodexDelivery {
        self.settled
            .extend(self.pending.into_iter().map(|(id, target)| {
                target.receipt(
                    Some(id),
                    CodexInterruptDelivery::Indeterminate {
                        detail: failure.into(),
                    },
                )
            }));
        self.settled.extend(
            self.waiting
                .into_iter()
                .map(|target| target.receipt(None, CodexInterruptDelivery::NotDispatched)),
        );
        CodexDelivery::Interrupts {
            children: self.settled,
        }
    }
}
impl Commands {
    fn retain(&mut self, sequence: u64, retention: DataRetention) {
        let old = self.retention.remove(&sequence).unwrap_or_default();
        self.retention.insert(sequence, old.merge(retention));
    }
    pub(super) fn pending(&self) -> bool {
        !self.pending.is_empty()
    }
    pub(super) fn starting_turn(&self) -> bool {
        self.pending
            .values()
            .any(|op| matches!(op.kind, CommandOperationKind::Send { .. }))
    }
    fn finish(&mut self, sequence: u64, delivery: CodexDelivery) {
        if let Some(op) = self.pending.remove(&sequence) {
            self.late_retention.remove(&sequence);
            self.receipts.push_back(CodexOperationReceipt {
                ticket: op.ticket,
                delivery,
                physical_write: None,
            });
        }
    }
    pub(super) fn invalidate(&mut self, detail: &str) {
        for (sequence, op) in std::mem::take(&mut self.pending) {
            let delivery = match op.kind {
                CommandOperationKind::Interrupts(group) => group.delivery(detail),
                CommandOperationKind::Reply { write, .. } => {
                    self.failed_writes.insert(
                        sequence,
                        (
                            Retained::from_parts(
                                op.ticket.clone(),
                                self.late_retention.remove(&sequence).unwrap_or_default(),
                            ),
                            write,
                        ),
                    );
                    CodexDelivery::Indeterminate {
                        detail: detail.into(),
                    }
                }
                _ => CodexDelivery::Indeterminate {
                    detail: detail.into(),
                },
            };
            self.late_retention.remove(&sequence);
            self.receipts.push_back(CodexOperationReceipt {
                ticket: op.ticket,
                delivery,
                physical_write: None,
            });
        }
        self.early = Default::default();
        self.start_cwd = None;
    }
    pub(super) fn next_generation(&mut self) -> Result<()> {
        self.invalidate("Codex connection replaced; mutation was not replayed");
        self.failed_writes.clear();
        self.generation = self
            .generation
            .checked_add(1)
            .ok_or_else(|| Error::new("Codex connection generation exhausted"))?;
        Ok(())
    }
}

impl NativeSession {
    /// Explicitly synchronous initialization/restart boundary. Never call this
    /// from an application's state-owner loop. Existing create/open also prepare
    /// a loaded thread; this method additionally prepares async `Start`.
    pub fn prepare_codex_commands(&mut self, cwd: Option<&Path>) -> Result<()> {
        if self.config.kind != NativeKind::Codex166 {
            return Err(Error::new("Not a Codex session"));
        }
        self.ensure_idle()?;
        self.validate_cwd(cwd)?;
        if self.native_id.is_some() {
            if self.process_ended {
                self.restart_required = true;
            }
            if self.restart_needed() && (self.has_background_work() || self.has_pending_requests())
            {
                return Err(Error::with_code(
                    "Codex still owns native work",
                    "outstanding_work",
                ));
            }
            self.prepare_turn(None, None)?;
        } else {
            if self.failed || self.process_ended {
                return Err(Error::with_code(
                    "Reconnect Codex before preparation",
                    "needs_preparation",
                ));
            }
            if self.restart_required && (self.config.gateway.is_some() || self.gateway_turn) {
                return Err(Error::with_code(
                    "Reconnect Codex with the selected execution configuration before preparing a new thread",
                    "needs_preparation",
                ));
            }
            self.codex_capture_provider(cwd)?;
            // Native model/provider selection is carried by thread/start's
            // options; it does not require replacing an unbound native child.
            self.restart_required = false;
            self.codex.commands.start_cwd = Some(cwd.map(Path::to_path_buf));
        }
        Ok(())
    }

    fn codex_dispatch_deadline(&self, requested: std::time::Instant) -> std::time::Instant {
        self.codex
            .commands
            .pending
            .values()
            .map(|op| op.deadline)
            .fold(requested, std::cmp::min)
    }

    /// Admit one mutation without waiting for stdin, protocol response or
    /// cleanup. The owner must keep calling `poll`, drain receipts, and retain
    /// this session/process until explicit synchronous shutdown has returned.
    /// A ticket is local admission, NOT native acceptance. Total pending plus
    /// unconsumed receipts is at most 16; four slots are reserved for replies.
    /// `poll` errors stop admission and settle unknown deliveries, but do not
    /// kill/reap this owner's process or release its locks. The owner must call
    /// synchronous `shutdown` outside its state loop, even if pending metadata
    /// remains. Memory slot release is never a resource-lifecycle confirmation.
    /// No application policy/history is owned here. Do not mix synchronous
    /// mutations with outstanding commands; preparation and cleanup are explicit.
    pub fn enqueue_codex_command(
        &mut self,
        command: CodexCommand,
        deadline: std::time::Instant,
    ) -> std::result::Result<Retained<CodexOperationTicket>, CodexAdmissionError> {
        use std::time::Instant;
        if self.config.kind != NativeKind::Codex166 {
            return Err(Error::new("Not a Codex session").into());
        }
        if self.failed
            || self.process_ended
            || (self.restart_needed()
                && matches!(
                    command,
                    CodexCommand::Start { .. }
                        | CodexCommand::Send(_)
                        | CodexCommand::Resume { .. }
                ))
        {
            return Err(CodexAdmissionError::NeedsPreparation);
        }
        if let Err(error) = self
            .codex_harvest_commands()
            .and_then(|()| self.rpc.check_failure())
        {
            self.fail_connection();
            return Err(error.into());
        }
        let is_reply = matches!(
            command,
            CodexCommand::Reply { .. }
                | CodexCommand::Cancel
                | CodexCommand::StopBackground
                | CodexCommand::StopChild { .. }
        );
        let count = self.codex.commands.pending.len() + self.codex.commands.receipts.len();
        if count
            >= if is_reply {
                COMMAND_CAPACITY
            } else {
                COMMAND_CAPACITY - REPLY_RESERVE
            }
        {
            return Err(
                Error::with_code("Codex command/receipt capacity full", "queue_full").into(),
            );
        }
        if deadline <= Instant::now() {
            return Err(Error::with_code(
                "Codex command deadline already expired; not dispatched",
                "timeout",
            )
            .into());
        }
        let sequence = self
            .codex
            .commands
            .next
            .checked_add(1)
            .ok_or_else(|| Error::new("Codex operation sequence exhausted"))?;
        let command_retention = self.data.try_clone(super::retention::command_data_owner)?;
        let retention = CommandRetention {
            public_ticket: command_retention.try_clone().map_err(retention_error)?,
            late_write: command_retention.try_clone().map_err(retention_error)?,
            command: command_retention,
        };
        let (method, params, kind) = match command {
            CodexCommand::Start { runtime_id } => {
                self.ensure_idle()?;
                if self.native_id.is_some() {
                    return Err(Error::new("Codex already has a thread").into());
                }
                let cwd = self
                    .codex
                    .commands
                    .start_cwd
                    .clone()
                    .ok_or(CodexAdmissionError::NeedsPreparation)?;
                let mut params = self.codex_options(cwd.as_deref());
                if self.side {
                    params["ephemeral"] = json!(true);
                }
                (
                    "thread/start",
                    params,
                    CommandOperationKind::Start { cwd, runtime_id },
                )
            }
            CodexCommand::Send(input) => self.codex_admit_turn(&input, false)?,
            CodexCommand::Resume {
                effort,
                permission_mode,
            } => {
                if self.side {
                    return Err(Error::new("Side contexts cannot resume").into());
                }
                self.codex_admit_turn(
                    &crate::TurnInput {
                        text: String::new(),
                        images: vec![],
                        effort,
                        permission_mode,
                    },
                    true,
                )?
            }
            CodexCommand::Steer(input) => {
                if !self.can_steer() || self.turn_id.is_none() {
                    return Err(Error::new("Codex has no steerable turn").into());
                }
                if input.text.trim().is_empty() && input.images.is_empty() {
                    return Err(Error::new("Empty message").into());
                }
                self.require_codex_steer_permission(&input)?;
                (
                    "turn/steer",
                    json!({"threadId":self.native_id,"expectedTurnId":self.turn_id,"input":codex_input(&input)}),
                    CommandOperationKind::Steer {
                        turn: self.turn_id.clone().expect("validated steer turn"),
                    },
                )
            }
            CodexCommand::Cancel => {
                let turn = self
                    .turn_id
                    .clone()
                    .filter(|_| self.turn_running())
                    .ok_or_else(|| Error::new("Codex has no identified running turn"))?;
                (
                    "turn/interrupt",
                    json!({"threadId":self.native_id,"turnId":turn}),
                    CommandOperationKind::Cancel { turn },
                )
            }
            CodexCommand::StopBackground | CodexCommand::StopChild { .. } => {
                let child = match command {
                    CodexCommand::StopChild { conversation_id } => {
                        let runtime = &self
                            .snapshot
                            .as_ref()
                            .ok_or_else(|| Error::new("Codex has no snapshot"))?
                            .conversation
                            .runtime_id;
                        let thread = conversation_id
                            .strip_prefix(&format!("{runtime}:"))
                            .filter(|id| Some(*id) != self.native_id.as_deref())
                            .ok_or_else(|| {
                                Error::new("Child identity does not belong to this execution")
                            })?;
                        if !self.codex.threads.contains_key(thread) {
                            return Err(Error::new(
                                "Child no longer belongs to this live work ledger",
                            )
                            .into());
                        }
                        Some(thread.to_owned())
                    }
                    _ if self.has_active_background_terminals() => {
                        return Err(Error::new("请在后台任务面板中停止 Codex 后台终端").into());
                    }
                    _ => None,
                };
                let targets = self
                    .codex
                    .threads
                    .iter()
                    .filter(|(thread, _)| child.as_deref().is_none_or(|id| id == thread.as_str()))
                    .filter(|(_, entry)| entry.state != BackgroundState::Stopping)
                    .filter_map(|(thread, entry)| {
                        Some(InterruptTarget {
                            thread: thread.clone(),
                            turn: entry.turn.clone()?,
                        })
                    })
                    .take(MAX_INTERRUPT_TARGETS + 1)
                    .collect::<std::collections::VecDeque<_>>();
                if targets.len() > MAX_INTERRUPT_TARGETS {
                    return Err(Error::with_code(
                        "Codex interrupt target capacity full; nothing dispatched",
                        "queue_full",
                    )
                    .into());
                }
                self.rpc.defer_cleanup();
                self.codex.commands.enabled = true;
                // An aggregate has no single native request id. Its receipt
                // retains every child id, including undispatched targets.
                return Ok(self.codex_install_retained_operation(
                    retention,
                    sequence,
                    Value::Null,
                    deadline,
                    CommandOperationKind::Interrupts(Interrupts {
                        waiting: targets,
                        pending: BTreeMap::new(),
                        settled: vec![],
                    }),
                ));
            }
            CodexCommand::Reply {
                interaction_id,
                reply,
            } => {
                if self.codex.commands.pending.values().any(|op| {
                    matches!(&op.kind,
                    CommandOperationKind::Reply { interaction_id: id, .. } if *id == interaction_id)
                }) {
                    return Err(Error::new("Reply already admitted; do not replay").into());
                }
                let pending = self
                    .interactions
                    .get(&interaction_id)
                    .ok_or_else(|| Error::new("Interaction expired"))?;
                let result = Self::codex_reply_result(pending, reply)?;
                let id = pending.id.clone();
                self.rpc.defer_cleanup();
                self.codex.commands.enabled = true;
                let dispatch_deadline = self.codex_dispatch_deadline(deadline);
                let write = self
                    .rpc
                    .enqueue_reply_receipt(&id, &result, dispatch_deadline)?;
                return Ok(self.codex_install_retained_operation(
                    retention,
                    sequence,
                    id,
                    deadline,
                    CommandOperationKind::Reply {
                        interaction_id,
                        write,
                    },
                ));
            }
        };
        self.rpc.defer_cleanup();
        self.codex.commands.enabled = true;
        if !is_reply && !self.rpc.command_write_available() {
            return Err(Error::with_code(
                "Codex ordinary writer capacity full; not dispatched",
                "queue_full",
            )
            .into());
        }
        let class = if is_reply {
            crate::transport::WriteClass::ProtocolReplyCancel
        } else {
            crate::transport::WriteClass::Ordinary
        };
        if let Err(error) = self
            .codex_harvest_commands()
            .and_then(|()| self.rpc.check_failure())
        {
            self.fail_connection();
            return Err(error.into());
        }
        if std::time::Instant::now() >= deadline {
            return Err(Error::with_code(
                "Codex dispatch deadline expired; not dispatched",
                "timeout",
            )
            .into());
        }
        let dispatch_deadline = self.codex_dispatch_deadline(deadline);
        let request =
            self.rpc
                .enqueue_request_with_class(method, &params, class, dispatch_deadline)?;
        if let CommandOperationKind::Send {
            effort,
            permission_mode,
            ..
        } = &kind
        {
            self.codex.retry = super::codex_retry::Retry {
                effort: effort.clone(),
                permission_mode: permission_mode.clone(),
                ..Default::default()
            };
        }
        Ok(self.codex_install_retained_operation(
            retention,
            sequence,
            json!(request),
            deadline,
            kind,
        ))
    }

    fn codex_admit_turn(
        &self,
        input: &crate::TurnInput,
        resuming: bool,
    ) -> Result<(&'static str, Value, CommandOperationKind)> {
        self.ensure_idle()?;
        if !resuming && input.text.trim().is_empty() && input.images.is_empty() {
            return Err(Error::new("Empty message"));
        }
        Ok((
            "turn/start",
            self.codex_turn_params(input)?,
            CommandOperationKind::Send {
                title: crate::conversation::attachment_list::strip(&input.text)
                    .chars()
                    .take(640)
                    .collect(),
                started: None,
                ended: false,
                resuming,
                effort: input.effort.clone(),
                permission_mode: input.permission_mode.clone(),
            },
        ))
    }

    fn codex_install_retained_operation(
        &mut self,
        retention: CommandRetention,
        sequence: u64,
        request_id: Value,
        deadline: std::time::Instant,
        kind: CommandOperationKind,
    ) -> Retained<CodexOperationTicket> {
        self.codex.commands.retain(sequence, retention.command);
        self.codex
            .commands
            .late_retention
            .insert(sequence, retention.late_write);
        let ticket = self.codex_install_operation(sequence, request_id, deadline, kind);
        Retained::from_parts(ticket, retention.public_ticket)
    }

    fn codex_install_operation(
        &mut self,
        sequence: u64,
        request_id: Value,
        deadline: std::time::Instant,
        kind: CommandOperationKind,
    ) -> CodexOperationTicket {
        let ticket = CodexOperationTicket {
            session: self.epoch.clone(),
            generation: self.codex.commands.generation,
            sequence,
            request_id,
        };
        self.codex.commands.next = sequence;
        self.codex.commands.pending.insert(
            sequence,
            CommandOperation {
                ticket: ticket.clone(),
                deadline,
                kind,
            },
        );
        ticket
    }

    fn codex_begin_command_turn(&mut self, turn: &str, resuming: bool) {
        self.data.replace(DataOwner::Turn);
        self.turn_id = Some(turn.into());
        self.codex.turn_error = None;
        if let Some(snapshot) = &mut self.snapshot {
            snapshot.executions.begin(ExecutionKind::UserTurn, resuming);
            snapshot.follow_up.can_resume = false;
        }
        self.running();
    }

    /// Taking a receipt is pure memory access and releases its bounded result
    /// slot, not native work, a home lock or process capacity.
    pub(super) fn codex_clear_failed_writes(&mut self) {
        self.codex.commands.failed_writes.clear();
    }

    pub fn take_codex_receipt(&mut self) -> Option<Retained<CodexOperationReceipt>> {
        let receipt = self.codex.commands.receipts.pop_front()?;
        let retention = self
            .codex
            .commands
            .retention
            .remove(&receipt.ticket.sequence)
            .unwrap_or_default();
        Some(Retained::from_parts(receipt, retention))
    }

    pub(super) fn codex_poll_commands(&mut self) -> Result<()> {
        // Consume delivery evidence first, then enforce connection-wide
        // failure/deadline barriers even when no interrupt group is pending.
        self.codex_harvest_commands()?;
        self.rpc.check_failure()?;
        let groups: Vec<_> = self
            .codex
            .commands
            .pending
            .iter()
            .filter_map(|(sequence, op)| {
                matches!(op.kind, CommandOperationKind::Interrupts(_)).then_some(*sequence)
            })
            .collect();
        for sequence in groups {
            for _ in 0..REPLY_RESERVE {
                // Another operation's deadline or a newly completed failed
                // write must stop every group's still-undispatched targets.
                self.codex_harvest_commands()?;
                self.rpc.check_failure()?;
                let dispatch_deadline =
                    self.codex_dispatch_deadline(std::time::Instant::now() + rpc::CONTROL_TIMEOUT);
                let Some(op) = self.codex.commands.pending.get_mut(&sequence) else {
                    break;
                };
                let CommandOperationKind::Interrupts(group) = &mut op.kind else {
                    unreachable!("captured interrupt group")
                };
                let Some(target) = group.waiting.front() else {
                    break;
                };
                let request = match self.rpc.enqueue_request_with_class(
                    "turn/interrupt",
                    &json!({"threadId":target.thread,"turnId":target.turn}),
                    crate::transport::WriteClass::ProtocolReplyCancel,
                    op.deadline.min(dispatch_deadline),
                ) {
                    Ok(id) => id,
                    Err(error) if error.code() == "queue_full" => break,
                    Err(error) => return Err(error),
                };
                let target = group.waiting.pop_front().expect("front target");
                group.pending.insert(request, target);
            }
        }
        self.codex_harvest_commands()
    }

    /// Harvest delivery evidence only; failure/cleanup paths must never
    /// dispatch another child mutation after the read side failed.
    pub(super) fn codex_harvest_commands(&mut self) -> Result<()> {
        use crate::transport::WriteOutcome;
        let mut facts = Vec::new();
        let mut failure = None;
        for (sequence, op) in &self.codex.commands.pending {
            if let CommandOperationKind::Reply { write, .. } = &op.kind {
                let Some(outcome) = write.try_outcome() else {
                    continue;
                };
                let (fact, delivery) = match outcome {
                    WriteOutcome::Written(c) if c.met_deadline() => {
                        (crate::WriteFact::Written(c), CodexDelivery::Written)
                    }
                    WriteOutcome::Written(c) => {
                        failure = Some(Error::with_code(
                            "Codex physically Written after deadline",
                            "timeout",
                        ));
                        (
                            crate::WriteFact::Written(c),
                            CodexDelivery::Indeterminate {
                                detail: "Reply physically Written late; cleanup required".into(),
                            },
                        )
                    }
                    WriteOutcome::NotWritten(error) => {
                        failure = Some(Error::with_code(
                            "Codex reply known not written",
                            "transport",
                        ));
                        (
                            crate::WriteFact::NotWritten,
                            CodexDelivery::Indeterminate {
                                detail: format!("Known not written: {error}"),
                            },
                        )
                    }
                    WriteOutcome::Indeterminate(error) => {
                        failure = Some(Error::with_code(
                            "Codex physical write indeterminate",
                            "transport",
                        ));
                        (
                            crate::WriteFact::Indeterminate,
                            CodexDelivery::Indeterminate {
                                detail: error.to_string(),
                            },
                        )
                    }
                    WriteOutcome::ObservationTimeout { .. } => {
                        failure = Some(Error::with_code(
                            "Codex reply not yet observed; receipt retained",
                            "timeout",
                        ));
                        continue;
                    }
                };
                facts.push((*sequence, fact, delivery));
            }
        }
        for (sequence, fact, delivery) in facts {
            let id = match &self.codex.commands.pending[&sequence].kind {
                CommandOperationKind::Reply { interaction_id, .. } => interaction_id.clone(),
                _ => unreachable!(),
            };
            let timely = matches!(delivery, CodexDelivery::Written);
            self.codex.commands.finish(sequence, delivery);
            self.codex
                .commands
                .receipts
                .back_mut()
                .unwrap()
                .physical_write = Some(fact);
            if timely {
                self.interactions.remove(&id);
                if let Some(snapshot) = &mut self.snapshot {
                    snapshot.pending_interactions.retain(|i| i.id != id);
                    snapshot.revision += 1;
                }
            }
        }
        let mut late = Vec::new();
        for (sequence, (_, write)) in &self.codex.commands.failed_writes {
            let fact = match write.try_outcome() {
                Some(WriteOutcome::Written(c)) => Some(crate::WriteFact::Written(c)),
                Some(WriteOutcome::NotWritten(_)) => Some(crate::WriteFact::NotWritten),
                Some(WriteOutcome::Indeterminate(_)) => Some(crate::WriteFact::Indeterminate),
                _ => None,
            };
            if let Some(fact) = fact {
                late.push((*sequence, fact));
            }
        }
        for (sequence, fact) in late {
            let (ticket, _) = self.codex.commands.failed_writes.remove(&sequence).unwrap();
            let (ticket, retention) = ticket.into_parts();
            self.codex.commands.retain(sequence, retention);
            if let Some(receipt) = self
                .codex
                .commands
                .receipts
                .iter_mut()
                .find(|r| r.ticket == ticket)
            {
                receipt.physical_write = Some(fact);
            } else {
                self.codex
                    .commands
                    .receipts
                    .push_back(CodexOperationReceipt {
                        ticket,
                        delivery: CodexDelivery::Indeterminate {
                            detail: "Physical reply settled after failure; not replayed".into(),
                        },
                        physical_write: Some(fact),
                    });
            }
        }
        if self
            .codex
            .commands
            .pending
            .values()
            .any(|op| std::time::Instant::now() >= op.deadline)
        {
            failure.get_or_insert_with(|| {
                Error::with_code("Codex operation response deadline; not replayed", "timeout")
            });
        }
        if let Some(error) = failure {
            return Err(error);
        }
        self.codex_finish_interrupt_groups();
        Ok(())
    }

    fn codex_finish_interrupt_groups(&mut self) {
        let finished: Vec<_> = self.codex.commands.pending.iter().filter_map(|(sequence, op)|
            matches!(&op.kind, CommandOperationKind::Interrupts(group) if group.pending.is_empty() && group.waiting.is_empty()).then_some(*sequence)).collect();
        for sequence in finished {
            let op = self
                .codex
                .commands
                .pending
                .remove(&sequence)
                .expect("finished group");
            self.codex.commands.late_retention.remove(&sequence);
            let CommandOperationKind::Interrupts(group) = op.kind else {
                unreachable!()
            };
            self.codex
                .commands
                .receipts
                .push_back(CodexOperationReceipt {
                    ticket: op.ticket,
                    delivery: group.delivery(""),
                    physical_write: None,
                });
        }
    }

    /// Called only for records from the currently owned RpcClient. An old
    /// connection's receiver is never installed into its replacement.
    fn codex_command_record(&mut self, record: &Value) -> Result<bool> {
        if record.get("method").is_none() {
            if let Some(id) = record["id"].as_u64() {
                for (sequence, op) in self.codex.commands.pending.iter_mut() {
                    if let CommandOperationKind::Interrupts(group) = &mut op.kind
                        && group.pending.contains_key(&id)
                    {
                        let delivery = if let Some(error) = record.get("error") {
                            CodexInterruptDelivery::Rejected {
                                error: error.clone(),
                            }
                        } else if record.get("result").is_some() {
                            CodexInterruptDelivery::Accepted
                        } else {
                            return Err(Error::with_code(
                                "Codex interrupt response missing result",
                                "transport",
                            ));
                        };
                        let target = group
                            .pending
                            .remove(&id)
                            .expect("correlated child interrupt");
                        if matches!(delivery, CodexInterruptDelivery::Accepted)
                            && let Some(entry) = self.codex.threads.get_mut(&target.thread)
                            && entry.turn.as_deref() == Some(&target.turn)
                        {
                            entry.state = BackgroundState::Stopping;
                        }
                        group.settled.push(target.receipt(Some(id), delivery));
                        let piece = self.data.branch();
                        let old = self
                            .codex
                            .commands
                            .retention
                            .remove(sequence)
                            .unwrap_or_default();
                        self.codex
                            .commands
                            .retention
                            .insert(*sequence, old.merge(piece));
                        self.codex_finish_interrupt_groups();
                        return Ok(true);
                    }
                }
            }
            let sequence = self
                .codex
                .commands
                .pending
                .iter()
                .find_map(|(sequence, op)| {
                    (!matches!(
                        op.kind,
                        CommandOperationKind::Reply { .. } | CommandOperationKind::Interrupts(_)
                    ) && op.ticket.generation == self.codex.commands.generation
                        && op.ticket.request_id == record["id"])
                        .then_some(*sequence)
                });
            let Some(sequence) = sequence else {
                return Ok(false);
            };
            self.codex.commands.retain(sequence, self.data.branch());
            if let Some(error) = record.get("error") {
                let contradictory = matches!(
                    &self.codex.commands.pending[&sequence].kind,
                    CommandOperationKind::Send {
                        started: Some(_),
                        ..
                    }
                ) || (matches!(
                    self.codex.commands.pending[&sequence].kind,
                    CommandOperationKind::Send { .. }
                ) && self.codex.commands.early.iter().next().is_some());
                if contradictory {
                    return Err(Error::with_code(
                        "Codex rejected a turn after emitting its facts",
                        "transport",
                    ));
                }
                let sending = matches!(
                    self.codex.commands.pending[&sequence].kind,
                    CommandOperationKind::Send { .. }
                );
                self.codex.commands.finish(
                    sequence,
                    CodexDelivery::Rejected {
                        error: error.clone(),
                    },
                );
                if sending {
                    self.codex.commands.early = Default::default();
                }
                return Ok(true);
            }
            let result = record
                .get("result")
                .ok_or_else(|| Error::with_code("Codex response missing result", "transport"))?;
            let mut op = self
                .codex
                .commands
                .pending
                .remove(&sequence)
                .expect("correlated command");
            // Keep the operation pending on malformed/contradictory acceptance,
            // so the terminal failure path produces its indeterminate receipt.
            let outcome = (|| -> Result<(Option<String>, Option<String>)> {
                if std::time::Instant::now() >= op.deadline {
                    return Err(Error::with_code(
                        "Codex terminal crossed absolute deadline before install",
                        "timeout",
                    ));
                }
                match &mut op.kind {
                    CommandOperationKind::Start { cwd, runtime_id } => {
                        let thread = result["thread"]["id"].as_str().ok_or_else(|| {
                            Error::with_code("Codex start missing thread identity", "transport")
                        })?;
                        self.codex_install_created(result, cwd.as_deref(), runtime_id)?;
                        Ok((Some(thread.into()), None))
                    }
                    CommandOperationKind::Send {
                        started,
                        ended,
                        resuming,
                        ..
                    } => {
                        let turn = result["turn"]["id"].as_str().ok_or_else(|| {
                            Error::with_code("Codex send missing turn identity", "transport")
                        })?;
                        if started.as_deref().is_some_and(|id| id != turn) {
                            return Err(Error::with_code(
                                "Codex ack and early turn identity disagree",
                                "transport",
                            ));
                        }
                        if started.is_none() && !*ended {
                            self.codex_begin_command_turn(turn, *resuming);
                        }
                        Ok((self.native_id.clone(), Some(turn.into())))
                    }
                    CommandOperationKind::Cancel { turn } => {
                        if self.turn_id.as_deref() == Some(turn)
                            && self.turn_running()
                            && let Some(snapshot) = &mut self.snapshot
                        {
                            snapshot.run_state = RunState::Stopping;
                            snapshot.revision += 1;
                        }
                        Ok((self.native_id.clone(), Some(turn.clone())))
                    }
                    CommandOperationKind::Steer { turn } => {
                        Ok((self.native_id.clone(), Some(turn.clone())))
                    }
                    CommandOperationKind::Reply { .. } | CommandOperationKind::Interrupts(_) => {
                        unreachable!("not a single request operation")
                    }
                }
            })();
            self.codex.commands.pending.insert(sequence, op);
            let (thread_id, turn_id) = outcome?;
            let title = match &self.codex.commands.pending[&sequence].kind {
                CommandOperationKind::Send { title, .. } => Some(title.clone()),
                _ => None,
            };
            // Settle delivery before optional metadata. Its failure can never
            // turn this accepted send back into an unsent/rejected operation.
            self.codex
                .commands
                .finish(sequence, CodexDelivery::Accepted { thread_id, turn_id });
            for early in self.codex.commands.early.drain_batch(256) {
                self.codex_receive_record(early)?;
            }
            if let Some(text) = title {
                self.codex_start_title(&crate::TurnInput {
                    text,
                    effort: None,
                    permission_mode: None,
                    images: vec![],
                });
            }
            return Ok(true);
        }
        if record["params"]["threadId"].as_str() != self.native_id.as_deref() {
            return Ok(false);
        }
        let Some(sequence) = self.codex.commands.pending.iter().find_map(|(seq, op)| {
            matches!(op.kind, CommandOperationKind::Send { .. }).then_some(*seq)
        }) else {
            return Ok(false);
        };
        let method = record["method"].as_str().unwrap_or_default();
        // Requests are serviced immediately even before turn/start's ack or
        // turn/started. Only turn-scoped output waits for native identity.
        if record.get("id").is_some() {
            return Ok(false);
        }
        let turn = record["params"]["turnId"]
            .as_str()
            .or_else(|| record["params"]["turn"]["id"].as_str());
        let CommandOperationKind::Send {
            started,
            ended,
            resuming,
            ..
        } = &mut self
            .codex
            .commands
            .pending
            .get_mut(&sequence)
            .expect("pending send")
            .kind
        else {
            unreachable!()
        };
        if method == "turn/started" && started.is_none() {
            let turn = turn.ok_or_else(|| {
                Error::with_code("Codex turn/started missing identity", "transport")
            })?;
            let piece = self.data.branch();
            let old = self
                .codex
                .commands
                .retention
                .remove(&sequence)
                .unwrap_or_default();
            self.codex
                .commands
                .retention
                .insert(sequence, old.merge(piece));
            *started = Some(turn.into());
            let resuming = *resuming;
            self.codex_begin_command_turn(turn, resuming);
            for early in self.codex.commands.early.drain_batch(256) {
                self.codex_receive_record(early)?;
            }
        } else if let Some(turn) = turn {
            if let Some(started) = started {
                if started != turn || *ended {
                    return Ok(true);
                }
                if method == "turn/completed" {
                    *ended = true;
                }
            } else {
                self.codex
                    .commands
                    .early
                    .push(Retained::from_parts(record.clone(), self.data.branch()))
                    .map_err(|error| {
                        Error::with_code(
                            format!("Codex early event capacity: {error}"),
                            "transport",
                        )
                    })?;
                return Ok(true);
            }
        }
        Ok(false)
    }
}

use velune_conversation::model_variant::codex_speed_model as speed_id;

/// A speed tier as the catalog reports it; variants keep id and name.
#[derive(Clone, serde::Deserialize)]
struct CatalogTier {
    id: String,
    name: String,
}

impl NativeSession {
    pub(super) fn codex_initialize(&mut self) -> Result<()> {
        if self
            .config
            .gateway
            .as_ref()
            .is_some_and(|g| g.protocol != "openai-responses")
        {
            return Err(Error::new("此 Codex 适配器仅支持原生 Responses 模型"));
        }
        self.rpc.request(
            "initialize",
            &json!({
                "clientInfo": {"name":"velune", "title":"Velune", "version":"0.1.0-beta.1"},
                "capabilities": {"experimentalApi":true}
            }),
        )?;
        self.rpc.notify("initialized", &json!({}))?;
        Ok(())
    }
    pub(super) fn codex_capture_provider(&mut self, cwd: Option<&Path>) -> Result<()> {
        let source = self
            .rpc
            .request("config/read", &json!({"includeLayers":false,"cwd":cwd}))?;
        // Velune's services are optional; one named like the user's own is left out.
        for server in &self.config.mcp_servers {
            if source["config"]["mcp_servers"].get(&server.name).is_some() {
                tracing::warn!(server = %server.name, "Codex already has an MCP service of this name; running without Velune's");
                self.shadowed_mcp_servers.insert(server.name.clone());
            }
        }
        let (source, retention) = source.into_parts();
        let (selection, retention) = retention.split();
        if self.native_selection.is_none() {
            self.data.insert(DataOwner::Selection, selection);
        }
        let (instructions, provider) = retention.split();
        self.data.insert(DataOwner::Instructions, instructions);
        self.data.insert(DataOwner::Provider, provider);
        self.codex.developer_instructions = source["config"]["developer_instructions"]
            .as_str()
            .map(str::to_owned);
        self.codex.title.mcp_servers = source["config"]["mcp_servers"]
            .as_object()
            .into_iter()
            .flat_map(|servers| servers.keys().cloned())
            .chain(
                self.config
                    .mcp_servers
                    .iter()
                    .map(|server| server.name.clone()),
            )
            .collect();
        self.native_provider = Some(
            source["config"]["model_provider"]
                .as_str()
                .unwrap_or("openai")
                .into(),
        );
        if self.native_selection.is_none()
            && let Some(model) = source["config"]["model"].as_str()
        {
            self.native_selection = Some(RuntimeModel {
                provider_id: self.native_provider.clone(),
                id: model.into(),
                name: model.into(),
            });
        }
        Ok(())
    }
    /// Where a gateway turn's agent role lives: the execution lane's own
    /// projection, never the Harness's agents directory.
    fn codex_role_path(config: &NativeConfig, name: &str) -> PathBuf {
        config
            .projection_directory
            .join("agents")
            .join(format!("{name}.toml"))
    }

    /// Writes the role file of each per-turn sub-agent on the executing
    /// machine; `codex_options` names them in the thread's config.
    pub(super) fn codex_write_roles(config: &NativeConfig) -> Result<()> {
        for agent in &config.session_agents {
            crate::management::validate_name(&agent.name)
                .map_err(|error| Error::new(error.to_string()))?;
            let path = Self::codex_role_path(config, &agent.name);
            if let Some(parent) = path.parent() {
                config.execution.create_dir_all(parent)?;
            }
            let effort = agent
                .effort
                .as_deref()
                .filter(|level| crate::management::CODEX_EFFORTS.contains(level));
            config.execution.write_file_atomic(
                &path,
                &crate::management::codex_agent(
                    &agent.name,
                    &agent.description,
                    &agent.instructions,
                    &agent.model,
                    effort,
                ),
            )?;
        }
        Ok(())
    }

    pub(super) fn codex_options(&self, cwd: Option<&Path>) -> Value {
        let mut options = json!({"approvalPolicy":"on-request","sandbox":"workspace-write"});
        if let Some(gateway) = &self.config.gateway {
            options["model"] = json!(gateway.model_alias);
            options["modelProvider"] = json!("velune");
            options["config"] = json!({
                "model_provider":"velune",
                "model_providers":{"velune":{"name":"Velune LLM Gateway","base_url":self.tunnel.as_ref().map(|t| t.endpoint()).unwrap_or(&gateway.endpoint),"env_key":"VELUNE_GATEWAY_TOKEN","wire_api":"responses","requires_openai_auth":false}},
                "features":{"responses_websockets":false,"responses_websockets_v2":false}
            });
            if !self.config.session_agents.is_empty() {
                options["config"]["agents"] = Value::Object(
                    self.config
                        .session_agents
                        .iter()
                        .map(|agent| {
                            (
                                agent.name.clone(),
                                json!({
                                    "description": agent.description,
                                    "config_file": Self::codex_role_path(&self.config, &agent.name),
                                }),
                            )
                        })
                        .collect(),
                );
            }
        } else {
            let provider = self
                .native_selection
                .as_ref()
                .and_then(|m| m.provider_id.as_ref())
                .or(self.native_provider.as_ref());
            options["modelProvider"] = json!(provider);
            options["config"] = json!({"model_provider":provider});
            if let Some(model) = &self.native_selection {
                options["model"] = json!(model.id);
            }
        }
        if let Some(cwd) = cwd {
            options["cwd"] = json!(cwd);
        }
        if !self.config.mcp_servers.is_empty() {
            options["config"]["mcp_servers"] = Value::Object(
                self.config
                    .mcp_servers
                    .iter()
                    .enumerate()
                    .filter(|(_, server)| !self.shadowed_mcp_servers.contains(&server.name))
                    .map(|(index, server)| {
                        (
                            server.name.clone(),
                            json!({"url":server.endpoint,"bearer_token_env_var":crate::mcp::token_environment(index)}),
                        )
                    })
                    .collect(),
            );
        }
        if let Some(context) = &self.codex.side_context {
            options["developerInstructions"] =
                json!(match self.codex.developer_instructions.as_deref() {
                    Some(existing) if !existing.trim().is_empty() =>
                        format!("{existing}\n\n{context}"),
                    _ => context.clone(),
                });
        }
        options
    }
    pub(super) fn codex_create(
        &mut self,
        cwd: Option<&Path>,
        runtime_id: &str,
        project: Option<&super::codex_projects::NativeProject>,
    ) -> Result<()> {
        self.codex_capture_provider(cwd)?;
        let mut options = self.codex_options(cwd);
        if let Some(project) = project {
            options["projectId"] = json!(project.id);
            // projectId records membership; Codex does not expand it into the
            // runtime workspace or the model's environment context itself.
            options["runtimeWorkspaceRoots"] = json!(
                project
                    .roots
                    .iter()
                    .map(|root| &root.path)
                    .collect::<Vec<_>>()
            );
        }
        if self.side {
            options["ephemeral"] = json!(true);
        }
        let result = self.rpc.request("thread/start", &options)?;
        self.receive_native_record(result, |session, result| {
            session.codex_install_created(&result, cwd, runtime_id)
        })
    }

    fn codex_install_created(
        &mut self,
        result: &Value,
        cwd: Option<&Path>,
        runtime_id: &str,
    ) -> Result<()> {
        if self.native_selection.is_none()
            && self.config.gateway.is_none()
            && let Some(model) = result["model"].as_str()
        {
            let provider_retention = self
                .data
                .try_clone(|owner| matches!(owner, DataOwner::Provider))?;
            self.data
                .merge_owner(DataOwner::Selection, provider_retention);
            self.data.retain(DataOwner::Selection);
            self.native_selection = Some(RuntimeModel {
                provider_id: self.native_provider.clone(),
                id: model.into(),
                name: model.into(),
            });
        }
        let id = result["thread"]["id"]
            .as_str()
            .ok_or_else(|| Error::new("Codex 未返回会话身份"))?;
        let resolved_cwd = result["thread"]["cwd"]
            .as_str()
            .map(PathBuf::from)
            .or_else(|| cwd.map(Path::to_path_buf));
        self.replace_snapshot(id, resolved_cwd.as_deref(), runtime_id, Vec::new());
        self.data.replace(DataOwner::Identity);
        self.codex.title.eligible = !self.side
            && result["thread"]["name"]
                .as_str()
                .is_none_or(|name| name.trim().is_empty());
        if let Some(snapshot) = self.snapshot.as_mut() {
            // app-server Thread dates are Unix seconds, including before the
            // first rollout is persisted. Preserve that native creation date.
            snapshot.conversation.created_at_unix_ms = result["thread"]["createdAt"]
                .as_i64()
                .and_then(|seconds| seconds.checked_mul(1000));
            snapshot.conversation.updated_at_unix_ms = result["thread"]["updatedAt"]
                .as_i64()
                .and_then(|seconds| seconds.checked_mul(1000));
        }
        Ok(())
    }
    /// Resume acquires the native writer. History is a separate projection
    /// and the server need not serialize every turn in its resume reply.
    pub(super) fn codex_resume_thread(&mut self, id: &str, cwd: Option<&Path>) -> Result<()> {
        self.codex_capture_provider(cwd)?;
        let mut options = self.codex_options(cwd);
        options["threadId"] = json!(id);
        options["excludeTurns"] = json!(true);
        let result = self
            .rpc
            .request_classified("thread/resume", &options, |error| {
                (error["code"].as_i64() == Some(-32600)
                    && error["message"].as_str()
                        == Some(format!("thread {id} already has an active writer").as_str()))
                .then_some("control_held_elsewhere")
            })?;
        if result["thread"]["id"].as_str() != Some(id) {
            return Err(Error::new("Codex 恢复了不同的会话"));
        }
        let (result, retention) = result.into_parts();
        if self.native_selection.is_none() && self.config.gateway.is_none() {
            self.data.insert(DataOwner::Selection, retention);
        }
        if self.native_selection.is_none()
            && self.config.gateway.is_none()
            && let Some(model) = result["model"].as_str()
        {
            let provider_retention = self
                .data
                .try_clone(|owner| matches!(owner, DataOwner::Provider))?;
            self.data
                .merge_owner(DataOwner::Selection, provider_retention);
            self.data.retain(DataOwner::Selection);
            self.native_selection = Some(RuntimeModel {
                provider_id: self.native_provider.clone(),
                id: model.into(),
                name: model.into(),
            });
        }
        Ok(())
    }

    pub(super) fn codex_open(
        &mut self,
        id: &str,
        cwd: Option<&Path>,
        runtime_id: &str,
    ) -> Result<()> {
        self.codex_resume_thread(id, cwd)?;
        let metadata = super::codex_history::read_metadata(&mut self.rpc, id)?;
        let history = super::codex_history::read_history(
            &mut self.rpc,
            runtime_id,
            id,
            &self.config.execution,
            &metadata,
        )?;
        let (history, retention) = metadata
            .zip(history)
            .map(|(_, history)| history)
            .into_parts();
        let resolved_cwd = history.cwd.as_deref().map(Path::new).or(cwd);
        self.replace_snapshot(id, resolved_cwd, runtime_id, history.messages);
        if let Some(snapshot) = &mut self.snapshot {
            snapshot.conversation.title = history.title;
            snapshot.conversation.native_read_state = history.native_read_state;
            snapshot.executions = history.executions;
            snapshot.pending_interactions = history.pending_interactions;
            snapshot.follow_up.can_resume = history.can_resume;
        }
        self.install_history_data(retention);
        Ok(())
    }
    pub(super) fn codex_turn_params(&self, input: &crate::TurnInput) -> Result<Value> {
        let id = self
            .native_id
            .as_deref()
            .ok_or_else(|| Error::new("Codex 没有活动会话"))?;
        let items = codex_input(input);
        // A permission mode is thread state: Codex keeps a turn's override for
        // later turns, so a turn without a mode sends none and keeps the
        // thread's current mode, as Claude Code does.
        let policy = match input.permission_mode.as_deref() {
            None => None,
            Some("read-only") => Some(("on-request", json!({"type":"readOnly"}), "user")),
            Some("workspace") => Some(("on-request", json!({"type":"workspaceWrite"}), "user")),
            Some("auto-review") => Some((
                "on-request",
                json!({"type":"workspaceWrite"}),
                "auto_review",
            )),
            Some("full-access") => Some(("never", json!({"type":"dangerFullAccess"}), "user")),
            Some(_) => return Err(Error::new("Codex 不接受此权限模式")),
        };
        let mut params = json!({
            "threadId": id,
            "model": self.config.gateway.as_ref().map(|g| g.model_alias.as_str()).or_else(|| self.native_selection.as_ref().map(|m| m.id.as_str())),
            "effort": input.effort,
            "input": items
        });
        // A speed variant's tier rides each turn of its selection; a gateway
        // turn's serving belongs to the gateway's own model record.
        if self.config.gateway.is_none()
            && let Some(tier) = &self.codex.service_tier
        {
            params["serviceTierForTurn"] = json!(tier);
        }
        if let Some((approval, sandbox, reviewer)) = policy {
            params["approvalPolicy"] = json!(approval);
            params["sandboxPolicy"] = sandbox;
            params["approvalsReviewer"] = json!(reviewer);
        }
        Ok(params)
    }
    pub(super) fn codex_send(&mut self, input: &crate::TurnInput) -> Result<()> {
        let params = self.codex_turn_params(input)?;
        let result = self.rpc.request("turn/start", &params)?;
        let turn_id = result["turn"]["id"]
            .as_str()
            .ok_or_else(|| Error::new("Codex 未返回轮次身份"))?;
        self.turn_id = Some(turn_id.into());
        self.data.insert(DataOwner::Turn, result.into_parts().1);
        self.codex.turn_error = None;
        self.codex.retry.native = false;
        if let Some(snapshot) = &mut self.snapshot {
            snapshot.follow_up.can_resume = false;
        }
        self.running();
        self.codex_start_title(input);
        Ok(())
    }
    /// `turn/steer` names the turn it expects to be running; Codex refuses
    /// the message when that turn has already ended.
    pub(super) fn codex_steer(&mut self, input: &crate::TurnInput) -> Result<()> {
        self.require_codex_steer_permission(input)?;
        let (Some(thread), Some(turn)) = (self.native_id.as_deref(), self.turn_id.as_deref())
        else {
            return Err(Error::new("Codex 没有正在运行的轮次"));
        };
        let params =
            json!({"threadId": thread, "expectedTurnId": turn, "input": codex_input(input)});
        self.rpc.request("turn/steer", &params)?;
        Ok(())
    }

    fn require_codex_steer_permission(&self, input: &crate::TurnInput) -> Result<()> {
        if let Some(mode) = &input.permission_mode
            && self.codex.retry.permission_mode.as_ref() != Some(mode)
        {
            return Err(Error::new(
                "当前 Codex 轮次不能更改权限模式，请排队发送此消息",
            ));
        }
        Ok(())
    }
    /// In place, `thread/revert` drops the message's turn and all later ones
    /// from the loaded thread. A fork copies the thread through the turn
    /// before it with `thread/fork` and resumes the copy here.
    pub(super) fn codex_branch(&mut self, index: usize, placement: EditPlacement) -> Result<()> {
        let snapshot = self.snapshot.as_ref().expect("branched snapshot");
        let thread = self
            .native_id
            .clone()
            .ok_or_else(|| Error::new("Codex 没有活动会话"))?;
        let turn = snapshot.messages[index]
            .branch_point
            .clone()
            .ok_or_else(|| Error::new("这条消息没有对应的 Codex 轮次"))?;
        match placement {
            EditPlacement::InPlace => {
                self.rpc.request(
                    "thread/revert",
                    &json!({"threadId": thread, "beforeTurnId": turn}),
                )?;
                self.truncate_snapshot(index);
            }
            EditPlacement::Fork => {
                let previous = snapshot.messages[..index]
                    .iter()
                    .rev()
                    .filter(|message| message.role == MessageRole::User)
                    .find_map(|message| message.branch_point.clone().filter(|point| *point != turn))
                    .ok_or_else(|| Error::new("第一条消息没有可复制的上文"))?;
                let cwd = snapshot.conversation.cwd.as_deref().map(PathBuf::from);
                let runtime_id = snapshot.conversation.runtime_id.clone();
                self.codex_copy(&thread, &previous, cwd.as_deref(), &runtime_id)?;
                // Editing immediately sends the replacement as one operation.
                // Acquire only the fork's writer before application admission;
                // an asynchronous control probe must not interrupt that send.
                self.prepare_turn(None, None).map_err(|error| {
                    Error::with_code(
                        format!(
                            "分叉 {} 已创建，但执行准备失败：{error}",
                            self.native_id.as_deref().unwrap_or_default()
                        ),
                        "fork_created",
                    )
                })?;
            }
        }
        Ok(())
    }
    /// `thread/fork` copies the thread through the turn the message at
    /// `index` belongs to; the copy resumes here and the original thread is
    /// left as it is.
    pub(super) fn codex_fork(&mut self, index: usize) -> Result<()> {
        let snapshot = self.snapshot.as_ref().expect("forked snapshot");
        let thread = self
            .native_id
            .clone()
            .ok_or_else(|| Error::new("Codex 没有活动会话"))?;
        // A message steered into a turn has no point of its own; the fork
        // covers the turn it belongs to.
        let turn = snapshot.messages[..=index]
            .iter()
            .rev()
            .filter(|message| message.role == MessageRole::User)
            .find_map(|message| message.branch_point.clone())
            .ok_or_else(|| Error::new("这条消息没有对应的 Codex 轮次"))?;
        let cwd = snapshot.conversation.cwd.as_deref().map(PathBuf::from);
        let runtime_id = snapshot.conversation.runtime_id.clone();
        self.codex_copy(&thread, &turn, cwd.as_deref(), &runtime_id)
    }

    /// The source is only a reference for this operation. The caller may
    /// own a different execution, or no loaded thread at all.
    pub(super) fn codex_copy(
        &mut self,
        source: &str,
        through_turn: &str,
        cwd: Option<&Path>,
        runtime_id: &str,
    ) -> Result<()> {
        let mut options = self.codex_options(cwd);
        options["threadId"] = json!(source);
        options["lastTurnId"] = json!(through_turn);
        options["excludeTurns"] = json!(true);
        let result = self
            .rpc
            .request_classified("thread/fork", &options, |_| Some("fork_rejected"))
            .map_err(|error| {
                if error.code() == "fork_rejected" {
                    error
                } else {
                    Error::with_code(error.to_string(), "fork_outcome_unknown")
                }
            })?;
        let fork = result["thread"]["id"]
            .as_str()
            .ok_or_else(|| Error::with_code("Codex 未返回 fork 会话身份", "fork_outcome_unknown"))?
            .to_owned();
        self.codex_adopt_fork(&fork, cwd, runtime_id)
            .map_err(|error| {
                Error::with_code(
                    format!("分叉 {fork} 已创建，但打开失败：{error}"),
                    "fork_created",
                )
            })?;
        Ok(())
    }

    /// End only this operation's connection after adopting the copy. With
    /// `fork_codex_from` it never owned the source writer; its running owner
    /// is independent and the next turn will reopen only the target.
    fn codex_adopt_fork(&mut self, fork: &str, cwd: Option<&Path>, runtime_id: &str) -> Result<()> {
        self.native_id = None;
        self.codex_open(fork, cwd, runtime_id)?;
        self.shutdown()?;
        self.restart_required = true;
        Ok(())
    }
    /// The configured model first, then the catalog. Codex lists each
    /// model's `supportedReasoningEfforts`.
    pub(super) fn codex_models(&mut self) -> Result<Retained<Vec<ModelOption>>> {
        let cwd = self
            .snapshot
            .as_ref()
            .and_then(|snapshot| snapshot.conversation.cwd.as_deref());
        let source = self
            .rpc
            .request("config/read", &json!({"includeLayers":false,"cwd":cwd}))?;
        let configured = source["config"]["model"].as_str().map(str::to_owned);
        let provider = source["config"]["model_provider"]
            .as_str()
            .unwrap_or("openai")
            .to_owned();
        let config_retention = source.into_parts().1;
        let mut retention = DataRetention::default();
        let mut catalog = Vec::new();
        let mut cursor = Value::Null;
        let mut seen = std::collections::BTreeSet::new();
        loop {
            let page = self.rpc.request(
                "model/list",
                &json!({"cursor":cursor,"limit":100,"includeHidden":false}),
            )?;
            let (page, page_retention) = page.into_parts();
            for model in page["data"]
                .as_array()
                .ok_or_else(|| Error::new("Codex 模型目录格式无效"))?
            {
                let id = model["model"]
                    .as_str()
                    .ok_or_else(|| Error::new("Codex 模型缺少身份"))?;
                let efforts: Vec<String> = model["supportedReasoningEfforts"]
                    .as_array()
                    .into_iter()
                    .flatten()
                    .filter_map(|effort| effort["reasoningEffort"].as_str().map(str::to_owned))
                    .collect();
                let name = model["displayName"].as_str().unwrap_or(id).to_owned();
                // Speed tiers are serving variants of the model, listed as
                // models of their own: selecting one asks for that tier.
                let tiers = model
                    .get("serviceTiers")
                    .cloned()
                    .map(serde_json::from_value::<Vec<CatalogTier>>)
                    .transpose()
                    .map_err(|error| Error::new(format!("Codex 速度档位目录格式无效：{error}")))?
                    .unwrap_or_default();
                let mut variants = Vec::new();
                for tier in tiers.into_iter().filter(|tier| !tier.id.is_empty()) {
                    // Each tier owns a separate efforts Vec and display-name
                    // string. Reserve that copy before allocating it.
                    retention =
                        retention.merge(page_retention.try_clone().map_err(retention_error)?);
                    variants.push(ModelOption {
                        model: RuntimeModel {
                            provider_id: None,
                            id: speed_id(id, &tier.id),
                            name: format!("{name}（{}）", tier.name),
                        },
                        reasoning_efforts: efforts.clone(),
                    });
                }
                catalog.push((
                    ModelOption {
                        model: RuntimeModel {
                            provider_id: None,
                            id: id.into(),
                            name,
                        },
                        reasoning_efforts: efforts,
                    },
                    variants,
                ));
            }
            retention = retention.merge(page_retention);
            let Some(next) = page["nextCursor"].as_str() else {
                break;
            };
            if !seen.insert(next.to_owned()) {
                return Err(Error::new("Codex 模型分页游标重复"));
            }
            cursor = json!(next);
        }
        let Some(configured) = configured else {
            return Ok(Retained::from_parts(
                catalog
                    .into_iter()
                    .flat_map(|(base, variants)| std::iter::once(base).chain(variants))
                    .collect(),
                retention,
            ));
        };
        let (efforts, tiers) = catalog
            .iter()
            .position(|(option, _)| option.model.id == configured)
            .map(|index| {
                let (option, tiers) = catalog.remove(index);
                (option.reasoning_efforts, tiers)
            })
            .unwrap_or_default();
        let mut catalog: Vec<ModelOption> = catalog
            .into_iter()
            .flat_map(|(base, variants)| std::iter::once(base).chain(variants))
            .collect();
        let provider = Some(provider);
        catalog.insert(
            0,
            ModelOption {
                model: RuntimeModel {
                    provider_id: provider.clone(),
                    id: configured.clone(),
                    name: configured.clone(),
                },
                reasoning_efforts: efforts,
            },
        );
        for (offset, mut variant) in tiers.into_iter().enumerate() {
            retention = retention.merge(config_retention.try_clone().map_err(retention_error)?);
            variant.model.provider_id = provider.clone();
            catalog.insert(offset + 1, variant);
        }
        Ok(Retained::from_parts(
            catalog,
            retention.merge(config_retention),
        ))
    }
    pub(super) fn codex_compact(&mut self) -> Result<()> {
        let thread = self
            .native_id
            .as_deref()
            .ok_or_else(|| Error::new("Codex 没有活动会话"))?;
        self.rpc
            .request("thread/compact/start", &json!({"threadId":thread}))?;
        self.turn_id = None;
        self.codex.cancel_pending = false;
        self.running();
        Ok(())
    }

    pub(super) fn codex_cancel(&mut self) -> Result<()> {
        let thread = self
            .native_id
            .as_deref()
            .ok_or_else(|| Error::new("Codex 没有活动会话"))?;
        let turn = self.turn_id.as_deref();
        if turn.is_none() && self.compacting {
            self.codex.cancel_pending = true;
            if let Some(snapshot) = &mut self.snapshot {
                snapshot.run_state = RunState::Stopping;
                snapshot.revision += 1;
            }
            return Ok(());
        }
        let turn = turn.ok_or_else(|| Error::new("Codex 没有可取消轮次"))?;
        self.rpc.request_with_class(
            "turn/interrupt",
            &json!({"threadId":thread,"turnId":turn}),
            crate::transport::WriteClass::ProtocolReplyCancel,
        )?;
        if let Some(snapshot) = &mut self.snapshot {
            snapshot.run_state = RunState::Stopping;
            snapshot.revision += 1;
        }
        Ok(())
    }
    fn codex_item(&mut self, item: &Value, turn: Option<&str>) {
        let Some(runtime_id) = self
            .snapshot
            .as_ref()
            .map(|snapshot| snapshot.conversation.runtime_id.clone())
        else {
            return;
        };
        if let Some(interaction) = super::codex_questions::project(item)
            && !self
                .codex
                .resolved_async_questions
                .contains(&interaction.id)
            && let Some(snapshot) = &mut self.snapshot
            && !snapshot
                .pending_interactions
                .iter()
                .any(|known| known.id == interaction.id)
        {
            self.data
                .retain(DataOwner::Interaction(interaction.id.clone()));
            snapshot.pending_interactions.push(interaction);
            snapshot.revision += 1;
        }
        if item["type"] == "contextCompaction" {
            self.compaction_message_id = item["id"].as_str().map(str::to_owned);
        }
        if let Some(mut message) = project_item(item, &runtime_id) {
            if message.role == MessageRole::User {
                message.branch_point = turn.map(str::to_owned);
            }
            self.message(message);
        }
        if let Some(snapshot) = &mut self.snapshot {
            let before = snapshot
                .pending_interactions
                .iter()
                .filter(|interaction| interaction.id.starts_with("codex-async:"))
                .map(|interaction| interaction.id.clone())
                .collect::<Vec<_>>();
            let changed =
                super::codex_questions::resolve_item(&mut snapshot.pending_interactions, item)
                    | (item["questions"].is_array()
                        && super::codex_questions::reconcile(
                            &mut snapshot.pending_interactions,
                            &snapshot.messages,
                        ));
            if changed {
                for id in &before {
                    if !snapshot
                        .pending_interactions
                        .iter()
                        .any(|interaction| &interaction.id == id)
                    {
                        self.data.move_owner(
                            DataOwner::Interaction(id.clone()),
                            DataOwner::ResolvedQuestions,
                        );
                    }
                }
                self.codex
                    .resolved_async_questions
                    .extend(before.into_iter().filter(|id| {
                        !snapshot
                            .pending_interactions
                            .iter()
                            .any(|interaction| &interaction.id == id)
                    }));
                snapshot.revision += 1;
            }
        }
    }
    pub(super) fn codex_receive_record(&mut self, record: Retained<Value>) -> Result<()> {
        self.receive_native_record(record, Self::codex_handle_record)
    }
    pub(super) fn codex_handle_record(&mut self, record: Value) -> Result<()> {
        if self.codex.commands.enabled {
            self.codex_harvest_commands()?;
            // RpcClient returns valid records preceding a terminal read error.
            // Settle those facts before poll_commands consumes that error;
            // every enqueue still crosses RpcClient's dispatch barrier.
        }
        if record["method"] == "item/completed"
            && self.native_id.is_some()
            && record["params"]["threadId"].as_str() == self.native_id.as_deref()
        {
            self.codex_terminal_item("item/completed", &record["params"]);
        }
        if self.codex_command_record(&record)? {
            return Ok(());
        }
        if self.codex_terminal_reply(&record) {
            return Ok(());
        }
        if self.codex_handoff_summary_record(&record)? {
            return Ok(());
        }
        if self.codex_title_record(&record) {
            return Ok(());
        }
        if self.codex_recovery_record(&record)? {
            return Ok(());
        }
        let method = record["method"].as_str().unwrap_or_default();
        let params = &record["params"];
        // Request ids belong to this RPC connection. A resolved request may
        // come from a child thread or an earlier turn, so settle its exact id
        // before filtering conversation/turn-specific output notifications.
        if method == "serverRequest/resolved" {
            let id = &params["requestId"];
            let expired = self
                .interactions
                .iter()
                .filter(|(_, pending)| &pending.id == id)
                .map(|(key, _)| key.clone())
                .collect::<Vec<_>>();
            for key in expired {
                self.interactions.remove(&key);
                if let Some(snapshot) = &mut self.snapshot {
                    snapshot
                        .pending_interactions
                        .retain(|interaction| interaction.id != key);
                    snapshot.revision += 1;
                }
            }
            return Ok(());
        }
        if let Some(thread) = params["threadId"].as_str()
            && Some(thread) != self.native_id.as_deref()
        {
            // A native sub-agent's thread runs in this app-server; its requests
            // are answered in this conversation, its own items stay in its own.
            let runtime_id = self
                .snapshot
                .as_ref()
                .map(|snapshot| snapshot.conversation.runtime_id.clone())
                .unwrap_or_default();
            self.codex_track_thread(thread, method, params);
            let sub_agent = self.codex.threads.contains_key(thread)
                || self.sub_agents.contains(&format!("{runtime_id}:{thread}"));
            match record.get("id") {
                Some(_) if sub_agent && record.get("method").is_some() => {
                    return self.codex_request(&record, true);
                }
                Some(id) => self.rpc.enqueue_rejection(id, "Inactive conversation")?,
                None if sub_agent && params["item"]["type"] == "collabAgentToolCall" => {
                    let nested = params["item"]["receiverThreadIds"]
                        .as_array()
                        .into_iter()
                        .flatten()
                        .filter_map(Value::as_str)
                        .map(|nested| format!("{runtime_id}:{nested}"));
                    self.sub_agents.extend(nested);
                }
                None if sub_agent
                    && params["item"]["type"] == "subAgentActivity"
                    && params["item"]["kind"] == "started" =>
                {
                    if let Some(nested) = params["item"]["agentThreadId"].as_str() {
                        self.sub_agents.insert(format!("{runtime_id}:{nested}"));
                    }
                }
                None => {}
            }
            return Ok(());
        }
        if record.get("id").is_some() && record.get("method").is_some() {
            return self.codex_request(&record, false);
        }
        if method == "item/started" {
            self.codex_terminal_item(method, params);
        }
        if let Some(turn) = params["turnId"].as_str()
            && self
                .snapshot
                .as_ref()
                .and_then(|s| s.executions.current.as_ref())
                .is_some()
            && self.turn_id.as_deref() != Some(turn)
        {
            return Ok(());
        }
        if params["turnId"].as_str() == self.turn_id.as_deref()
            && self.turn_id.is_some()
            && ((matches!(
                method,
                "item/agentMessage/delta"
                    | "item/reasoning/textDelta"
                    | "item/reasoning/summaryTextDelta"
            ) && params["delta"]
                .as_str()
                .is_some_and(|delta| !delta.is_empty()))
                || (method == "item/started"
                    && matches!(
                        params["item"]["type"].as_str(),
                        Some("agentMessage" | "reasoning")
                    )))
        {
            self.codex_native_retry_progress();
        }
        match method {
            "thread/readState/changed" => {
                self.data.retain(DataOwner::Metadata);
                if let Some(snapshot) = &mut self.snapshot {
                    snapshot.conversation.native_read_state = super::codex_history::read_state(
                        params.get("readState"),
                        &snapshot.context_runtime_id,
                        self.native_id.as_deref().unwrap_or_default(),
                    )?;
                    snapshot.revision += 1;
                }
            }
            "turn/started" if self.compacting => {
                self.turn_id = params["turn"]["id"].as_str().map(str::to_owned);
                if self.codex.cancel_pending {
                    self.codex.cancel_pending = false;
                    self.codex_cancel()?;
                }
            }
            "item/started" | "item/completed" => {
                self.codex_item(&params["item"], params["turnId"].as_str());
                if let Some(message) = self.snapshot.as_mut().and_then(|snapshot| {
                    snapshot
                        .messages
                        .iter_mut()
                        .find(|message| Some(message.id.as_str()) == params["item"]["id"].as_str())
                }) {
                    message.completed = method == "item/completed";
                    if message.timestamp_unix_ms.is_none() {
                        message.timestamp_unix_ms = params["startedAtMs"].as_i64();
                    }
                    for block in &mut message.blocks {
                        if let MessageBlock::ContextCompaction { text } = block {
                            *text = if message.completed {
                                "上下文已压缩"
                            } else {
                                "正在压缩上下文"
                            }
                            .into();
                        }
                    }
                }
            }
            "item/agentMessage/delta" => {
                if let (Some(id), Some(delta)) =
                    (params["itemId"].as_str(), params["delta"].as_str())
                {
                    let old = self
                        .snapshot
                        .as_mut()
                        .and_then(|s| s.messages.iter_mut().find(|m| m.id == id))
                        .and_then(|m| m.blocks.first_mut())
                        .and_then(|b| {
                            if let MessageBlock::Text { text } = b {
                                Some(std::mem::take(text))
                            } else {
                                None
                            }
                        })
                        .unwrap_or_default();
                    self.message(Message {
                        turn_settings: None,
                        turn_usage: None,
                        branch_point: None,
                        completed: false,
                        id: id.into(),
                        role: crate::conversation::MessageRole::Assistant,
                        timestamp_unix_ms: None,
                        blocks: vec![MessageBlock::Text { text: old + delta }],
                    });
                }
            }
            "turn/completed" => {
                if let Some(turn) = params["turn"]["id"].as_str()
                    && Some(turn) == self.turn_id.as_deref()
                {
                    self.codex_settle_terminals();
                    let status = params["turn"]["status"].as_str();
                    if !matches!(status, Some("completed" | "failed" | "interrupted")) {
                        return Err(Error::new("Codex 轮次缺少有效结束状态"));
                    }
                    if !self.compacting
                        && let Some(snapshot) = &mut self.snapshot
                    {
                        snapshot.follow_up.can_resume =
                            matches!(status, Some("interrupted" | "failed"));
                    }
                    if matches!(status, Some("interrupted" | "failed")) {
                        self.codex_fail_open_tools();
                    }
                    if status == Some("failed") {
                        let error = &params["turn"]["error"];
                        let error = if error.is_null() {
                            self.codex.turn_error.as_ref().unwrap_or(error)
                        } else {
                            error
                        };
                        let mut error =
                            project_error(turn, error, false, turn_completed_at(&params["turn"]))?;
                        let retry = self.codex_retry_transient(&mut error);
                        self.codex_error_message(error);
                        if retry {
                            self.codex.turn_error = None;
                            return Ok(());
                        }
                    }
                    let exhausted = matches!(
                        self.codex.retry.phase,
                        super::codex_retry::Phase::Ended(Some(RuntimeRetry::Exhausted { .. }))
                    );
                    self.codex_end_recovery(turn_outcome(status), exhausted);
                    self.codex.turn_error = None;
                    self.finish_turn(if status == Some("failed") {
                        RunState::Failed
                    } else {
                        RunState::Idle
                    })?;
                }
            }
            // Usage of each model response in the turn; the reread rollout has the turn's total.
            "thread/tokenUsage/updated" => {
                if params["turnId"].as_str() == self.turn_id.as_deref() {
                    use super::codex_history::{APP_SERVER_KEYS, codex_usage, context_tokens};
                    let last = &params["tokenUsage"]["last"];
                    let mut usage = codex_usage(last, APP_SERVER_KEYS);
                    usage.context_tokens = context_tokens(last, APP_SERVER_KEYS);
                    usage.context_window = params["tokenUsage"]["modelContextWindow"].as_u64();
                    self.context_usage(usage.context_tokens, usage.context_window);
                    self.turn_usage(|total| match total {
                        Some(total) => total.add(&usage),
                        None => *total = Some(usage),
                    });
                }
            }
            // The account's limits follow each response's usage; Velune's gateway has none.
            "account/rateLimits/updated" if self.turn_id.is_some() => {
                let limits = super::codex_history::rate_limits(
                    &params["rateLimits"],
                    "usedPercent",
                    "windowDurationMins",
                    "resetsAt",
                );
                if !limits.is_empty() {
                    self.turn_usage(|total| {
                        if let Some(total) = total {
                            total.rate_limits = limits;
                        }
                    });
                }
            }
            "error" => {
                if let Some(turn) = params["turnId"].as_str()
                    && Some(turn) == self.turn_id.as_deref()
                {
                    let retrying = params["willRetry"]
                        .as_bool()
                        .ok_or_else(|| Error::new("Codex 错误通知缺少重试状态"))?;
                    if retrying && self.codex.retry.native_view == Some(RuntimeRetry::Recovered) {
                        self.codex.retry.native_reconnects += 1;
                    }
                    self.codex.retry.native |= retrying;
                    self.codex.retry.native_view = retrying.then_some(RuntimeRetry::Native);
                    let error = project_error(turn, &params["error"], retrying, None)?;
                    self.codex_error_message(error);
                    self.data.replace(DataOwner::TurnError);
                    self.codex.turn_error = Some(params["error"].clone());
                }
            }
            _ => {}
        }
        Ok(())
    }
    fn codex_error_message(&mut self, mut message: Message) {
        if self.codex.retry.native_reconnects > 0 {
            message.id = format!(
                "{}-reconnect-{}",
                message.id, self.codex.retry.native_reconnects
            );
        }
        if self.compacting {
            message.id = self.compaction_message_id.clone().unwrap_or(message.id);
            self.compaction_message_id = Some(message.id.clone());
            message.blocks = message
                .blocks
                .into_iter()
                .map(|block| match block {
                    MessageBlock::Notice { text }
                    | MessageBlock::RuntimeFailure { detail: text, .. } => {
                        MessageBlock::ContextCompaction {
                            text: format!("上下文压缩失败：{text}"),
                        }
                    }
                    block => block,
                })
                .collect();
        }
        self.message(message);
    }

    /// Follows another thread of this app-server: a native sub-agent the
    /// conversation started, or one of theirs, works while its status is
    /// active. Tracking every thread, not only those the spawning item
    /// names, does not depend on which of their events arrives first.
    fn codex_track_thread(&mut self, thread: &str, method: &str, params: &Value) {
        if matches!(method, "thread/status/changed" | "turn/started") {
            self.data.retain(DataOwner::Thread(thread.into()));
        }
        match method {
            "thread/status/changed" => {
                let status = &params["status"];
                if status["type"] != "active" {
                    self.codex.threads.remove(thread);
                    return;
                }
                let flags = status["activeFlags"].as_array();
                let flagged = |flag: &str| flags.is_some_and(|f| f.iter().any(|v| v == flag));
                let entry = self.codex.threads.entry(thread.into()).or_default();
                if entry.state != BackgroundState::Stopping {
                    entry.state = if flagged("waitingOnApproval") {
                        BackgroundState::AwaitingApproval
                    } else if flagged("waitingOnUserInput") {
                        BackgroundState::AwaitingInput
                    } else {
                        BackgroundState::Running
                    };
                }
            }
            "turn/started" => {
                self.codex.threads.entry(thread.into()).or_default().turn =
                    params["turn"]["id"].as_str().map(str::to_owned);
            }
            "turn/completed" => {
                if let Some(entry) = self.codex.threads.get_mut(thread) {
                    entry.turn = None;
                }
            }
            _ => {}
        }
    }
    /// Codex sends no `item/completed` for the tool calls a stopped or failed
    /// turn aborted, nor keeps them in its history; they did not finish.
    fn codex_fail_open_tools(&mut self) {
        let Some(snapshot) = &mut self.snapshot else {
            return;
        };
        let turn_start = snapshot
            .messages
            .iter()
            .rposition(|message| message.role == MessageRole::User)
            .unwrap_or(0);
        for message in &mut snapshot.messages[turn_start..] {
            for block in &mut message.blocks {
                if let MessageBlock::Tool { state, .. } = block
                    && matches!(state, ToolState::Pending | ToolState::Running)
                {
                    *state = ToolState::Failed;
                }
            }
        }
    }
    /// Interrupts each sub-agent thread's turn; interrupting the
    /// conversation's own turn does not stop them. They end when their
    /// status turns idle.
    pub(super) fn codex_stop_background(&mut self) -> Result<()> {
        if self.has_active_background_terminals() {
            return Err(Error::new("请在后台任务面板中停止 Codex 后台终端"));
        }
        let turns = self
            .codex
            .threads
            .iter()
            .filter(|(_, entry)| entry.state != BackgroundState::Stopping)
            .filter_map(|(thread, entry)| Some((thread.clone(), entry.turn.clone()?)))
            .collect::<Vec<_>>();
        for (thread, turn) in turns {
            self.rpc.request_with_class(
                "turn/interrupt",
                &json!({"threadId":thread,"turnId":turn}),
                crate::transport::WriteClass::ProtocolReplyCancel,
            )?;
            if let Some(entry) = self.codex.threads.get_mut(&thread) {
                entry.state = BackgroundState::Stopping;
            }
        }
        Ok(())
    }
    /// `sub_agent` marks a request from a native sub-agent's thread.
    fn codex_request(&mut self, record: &Value, sub_agent: bool) -> Result<()> {
        let method = record["method"].as_str().unwrap_or_default();
        let params = &record["params"];
        let id = record["id"].clone();
        // A sub-agent may outlive the turn that started it; its requests
        // still come to this conversation.
        if !self.turn_running()
            && !sub_agent
            && !self.codex.commands.enabled
            && method != "mcpServer/elicitation/request"
        {
            self.rpc.enqueue_rejection(&id, "No active turn")?;
            return Ok(());
        }
        match method {
            "item/commandExecution/requestApproval" | "item/fileChange/requestApproval" => {
                let allowed = [
                    ("accept", "允许这一次", ApprovalOptionKind::AllowOnce),
                    ("decline", "拒绝", ApprovalOptionKind::RejectOnce),
                    ("cancel", "取消", ApprovalOptionKind::Cancel),
                ];
                let options = allowed
                    .iter()
                    .filter(|(choice, _, _)| {
                        params["availableDecisions"]
                            .as_array()
                            .is_none_or(|a| a.iter().any(|v| v.as_str() == Some(choice)))
                    })
                    .map(|(choice, label, approval_kind)| InteractionOption {
                        id: (*choice).into(),
                        label: (*label).into(),
                        approval_kind: Some(*approval_kind),
                    })
                    .collect::<Vec<_>>();
                if options.is_empty() {
                    self.rpc
                        .enqueue_rejection(&id, "Unsupported approval decisions")?;
                    self.notice("此 Codex 审批方式尚不受支持。");
                    return Ok(());
                }
                let action = if method.contains("commandExecution") {
                    "允许执行命令？"
                } else {
                    "允许修改文件？"
                };
                let title = if sub_agent {
                    format!("子 Agent：{action}")
                } else {
                    action.to_owned()
                };
                let detail = [
                    params["command"].as_str(),
                    params["reason"].as_str(),
                    params["cwd"].as_str(),
                ]
                .into_iter()
                .flatten()
                .collect::<Vec<_>>()
                .join("\n");
                self.add_interaction(
                    PendingRequest {
                        background: sub_agent,
                        id,
                        method: method.into(),
                        options: options.iter().map(|o| o.id.clone()).collect(),
                        question_ids: Vec::new(),
                        payload: Value::Null,
                    },
                    InteractionKind::Approval {
                        title,
                        detail,
                        options,
                    },
                );
            }
            "mcpServer/elicitation/request" => match crate::elicitation::questions(params) {
                Ok(questions) => self.add_interaction(
                    PendingRequest {
                        background: true,
                        id,
                        method: method.into(),
                        options: Vec::new(),
                        question_ids: questions.iter().map(|q| q.id.clone()).collect(),
                        payload: params.clone(),
                    },
                    InteractionKind::UserInput { questions },
                ),
                Err(error) => {
                    self.rpc.enqueue_response(
                        &id,
                        &json!({"action":"cancel","content":null,"_meta":null}),
                    )?;
                    self.notice(error.detail());
                }
            },
            "item/tool/requestUserInput" => {
                let blocking = params["isBlocking"].as_bool().unwrap_or(true);
                let questions = params["questions"]
                    .as_array()
                    .ok_or_else(|| Error::new("Codex 用户输入请求缺少问题"))?
                    .iter()
                    .map(|q| {
                        let options = q["options"]
                            .as_array()
                            .into_iter()
                            .flatten()
                            .filter_map(|o| {
                                o["label"].as_str().map(|label| InteractionOption {
                                    id: label.into(),
                                    label: label.into(),
                                    approval_kind: None,
                                })
                            })
                            .collect();
                        InteractionQuestion {
                            id: q["id"].as_str().unwrap_or_default().into(),
                            text: q["question"].as_str().unwrap_or_default().into(),
                            options,
                            secret: q["isSecret"].as_bool().unwrap_or(false),
                            default_text: None,
                            multiline: false,
                            allow_other: q["isOther"].as_bool().unwrap_or(false),
                            multiple: false,
                        }
                    })
                    .collect::<Vec<_>>();
                let unique = questions
                    .iter()
                    .map(|q| q.id.as_str())
                    .collect::<std::collections::HashSet<_>>();
                if questions.is_empty()
                    || questions.iter().any(|q| q.id.is_empty())
                    || unique.len() != questions.len()
                {
                    self.rpc.enqueue_rejection(&id, "Invalid questions")?;
                    return Ok(());
                }
                self.add_interaction(
                    PendingRequest {
                        background: sub_agent || !blocking,
                        id,
                        method: method.into(),
                        options: Vec::new(),
                        question_ids: questions.iter().map(|q| q.id.clone()).collect(),
                        payload: Value::Null,
                    },
                    if blocking {
                        InteractionKind::UserInput { questions }
                    } else {
                        InteractionKind::AsyncUserInput {
                            questions,
                            delivery: crate::conversation::AsyncQuestionDelivery::NativeReply,
                        }
                    },
                );
            }
            _ => {
                self.rpc
                    .enqueue_rejection(&id, "This interaction is not supported by Velune")?;
                self.notice("运行时请求了一种尚未支持的交互，已明确拒绝。");
            }
        }
        Ok(())
    }
    pub(super) fn codex_reply(
        &mut self,
        pending: &PendingRequest,
        reply: RuntimeInteractionReply,
    ) -> Result<()> {
        let result = Self::codex_reply_result(pending, reply)?;
        self.rpc.respond(&pending.id, &result)
    }

    fn codex_reply_result(
        pending: &PendingRequest,
        reply: RuntimeInteractionReply,
    ) -> Result<Value> {
        let result = if pending.method == "mcpServer/elicitation/request" {
            crate::elicitation::reply(&pending.payload, reply)?
        } else if pending.method == "item/tool/requestUserInput" {
            match reply {
                RuntimeInteractionReply::Answers { answers } => {
                    let mut map = serde_json::Map::new();
                    for answer in answers {
                        if !pending.question_ids.contains(&answer.question_id)
                            || map.contains_key(&answer.question_id)
                        {
                            return Err(Error::new("用户输入答案与当前问题不匹配"));
                        }
                        map.insert(answer.question_id, json!({"answers":answer.values}));
                    }
                    if map.len() != pending.question_ids.len() {
                        return Err(Error::new("请回答所有问题"));
                    }
                    json!({"answers":map})
                }
                RuntimeInteractionReply::Cancel => json!({"answers":{}}),
                _ => return Err(Error::new("此请求需要用户输入答案")),
            }
        } else {
            let decision = match reply {
                RuntimeInteractionReply::Decision { option_id } => {
                    if !pending.options.contains(&option_id) {
                        return Err(Error::new("审批选项已经失效"));
                    }
                    option_id
                }
                RuntimeInteractionReply::Cancel => pending
                    .options
                    .iter()
                    .find(|choice| choice.as_str() == "cancel")
                    .or_else(|| {
                        pending
                            .options
                            .iter()
                            .find(|choice| choice.as_str() == "decline")
                    })
                    .cloned()
                    .ok_or_else(|| Error::new("此审批请求没有可拒绝的选项"))?,
                _ => return Err(Error::new("此请求需要审批决定")),
            };
            json!({"decision":decision})
        };
        Ok(result)
    }
}

/// A change to a native thread's metadata.
#[derive(Debug, Clone, Copy)]
pub enum ThreadChange<'a> {
    Rename(&'a str),
    Delete,
    Archive,
    Unarchive,
}

/// Native thread metadata management without model selection or gateway startup.
pub fn manage_thread(binary: &Path, home: &Path, id: &str, change: ThreadChange) -> Result<()> {
    manage_thread_at(
        crate::version::RuntimeKind::Codex166,
        binary,
        home,
        id,
        change,
        &crate::execution::ExecutionTarget::Local,
        None,
    )
}
pub fn manage_thread_at(
    kind: crate::version::RuntimeKind,
    binary: &Path,
    home: &Path,
    id: &str,
    change: ThreadChange,
    execution: &crate::execution::ExecutionTarget,
    os_home: Option<&Path>,
) -> Result<()> {
    let mut rpc = history_client_at(kind, binary, home, execution, os_home)?;
    let result = match change {
        ThreadChange::Rename(name) => {
            rpc.request("thread/name/set", &json!({"threadId":id,"name":name}))
        }
        ThreadChange::Delete => rpc.request("thread/delete", &json!({"threadId":id})),
        ThreadChange::Archive => rpc.request("thread/archive", &json!({"threadId":id})),
        ThreadChange::Unarchive => rpc.request("thread/unarchive", &json!({"threadId":id})),
    };
    rpc.shutdown();
    result.map(|_| ())
}
pub(super) fn history_client_at(
    kind: crate::version::RuntimeKind,
    binary: &Path,
    home: &Path,
    execution: &crate::execution::ExecutionTarget,
    os_home: Option<&Path>,
) -> Result<rpc::RpcClient> {
    history_client_at_with_provider(kind, binary, home, execution, os_home, None)
}

pub(super) fn history_client_at_with_provider(
    kind: crate::version::RuntimeKind,
    binary: &Path,
    home: &Path,
    execution: &crate::execution::ExecutionTarget,
    os_home: Option<&Path>,
    provider: Option<&str>,
) -> Result<rpc::RpcClient> {
    if !binary.is_absolute() || !home.is_absolute() {
        return Err(Error::new("Codex 历史配置必须使用绝对路径"));
    }
    crate::version::check_version_at(kind, binary, None, execution, os_home)?;
    let mut command = crate::process_tree::command(binary);
    command.arg("app-server");
    if let Some(provider) = provider {
        command.arg("-c").arg(format!(
            "model_provider={}",
            serde_json::to_string(provider).map_err(|error| Error::new(error.to_string()))?
        ));
        if provider == "velune" {
            // Only the import-indexing child gets this inert, unauthenticated
            // endpoint. The execution child supplies the selected live gateway.
            command.arg("-c").arg("model_providers.velune={name=\"Velune history materialization\",base_url=\"http://127.0.0.1:9/v1\",wire_api=\"responses\",requires_openai_auth=false}");
        }
    }
    command.env("CODEX_HOME", home);
    let mut rpc = rpc::RpcClient::spawn_at(command, execution, os_home)?;
    rpc.request("initialize", &json!({"clientInfo":{"name":"velune","version":"0.1.0-beta.1"},"capabilities":{"experimentalApi":true}}))?;
    rpc.notify("initialized", &json!({}))?;
    Ok(rpc)
}

pub(super) fn turn_outcome(status: Option<&str>) -> Option<TurnOutcome> {
    match status {
        Some("completed") => Some(TurnOutcome::Completed),
        Some("failed") => Some(TurnOutcome::Failed),
        Some("interrupted") => Some(TurnOutcome::Cancelled),
        _ => None,
    }
}

fn error_id(turn: &str) -> String {
    format!("codex-turn-error-{turn}")
}

fn turn_completed_at(turn: &Value) -> Option<i64> {
    turn["completedAt"].as_i64()?.checked_mul(1000)
}

pub(super) fn project_turn_error(turn: &Value) -> Result<Option<Message>> {
    if turn["status"] != "failed" && turn["error"].is_null() {
        return Ok(None);
    }
    let id = turn["id"]
        .as_str()
        .filter(|id| !id.is_empty())
        .ok_or_else(|| Error::new("Codex 失败回合缺少身份"))?;
    project_error(id, &turn["error"], false, turn_completed_at(turn)).map(Some)
}

fn project_error(
    turn: &str,
    error: &Value,
    retrying: bool,
    timestamp_unix_ms: Option<i64>,
) -> Result<Message> {
    let mut text = if error.is_null() {
        "Codex 执行失败（上游未提供错误详情）。".into()
    } else {
        error["message"]
            .as_str()
            .filter(|message| !message.trim().is_empty())
            .ok_or_else(|| Error::new("Codex 错误详情缺少 message"))?
            .to_owned()
    };
    if !error["additionalDetails"].is_null() {
        let details = error["additionalDetails"]
            .as_str()
            .ok_or_else(|| Error::new("Codex 错误附加详情不是文本"))?;
        if !details.trim().is_empty() {
            text.push('\n');
            text.push_str(details);
        }
    }
    if !error["codexErrorInfo"].is_null() {
        let info = &error["codexErrorInfo"];
        text.push_str("\nCodex 错误类型：");
        text.push_str(
            &info
                .as_str()
                .map(str::to_owned)
                .unwrap_or_else(|| info.to_string()),
        );
    }
    if retrying {
        text.push_str(NATIVE_RETRY_DETAIL);
    }
    Ok(Message {
        id: error_id(turn),
        role: MessageRole::System,
        timestamp_unix_ms,
        blocks: vec![MessageBlock::RuntimeFailure {
            failure: failure_kind(error),
            detail: text,
            retry: retrying.then_some(RuntimeRetry::Native),
        }],
        completed: true,
        turn_settings: None,
        turn_usage: None,
        branch_point: None,
    })
}

/// Codex distinguishes temporary frequency limits from account usage caps.
/// Status codes are forwarded on the transport error variants, not the root.
fn failure_kind(error: &Value) -> RuntimeFailureKind {
    let info = &error["codexErrorInfo"];
    if info == "serverOverloaded" {
        return RuntimeFailureKind::ServerOverloaded;
    }
    if info == "rateLimitExceeded"
        || [
            "httpConnectionFailed",
            "responseStreamConnectionFailed",
            "responseStreamDisconnected",
            "responseTooManyFailedAttempts",
        ]
        .iter()
        .any(|variant| info[variant]["httpStatusCode"] == 429)
    {
        return RuntimeFailureKind::RateLimited;
    }
    RuntimeFailureKind::Other
}

pub(super) fn project_item(item: &Value, runtime_id: &str) -> Option<Message> {
    let id = item["id"].as_str()?;
    let (kind, blocks) = match item["type"].as_str().unwrap_or_default() {
        "userMessage" => {
            let blocks = item["content"]
                .as_array()
                .into_iter()
                .flatten()
                .filter_map(|part| match part["type"].as_str() {
                    Some("text") => part["text"]
                        .as_str()
                        .map(|t| MessageBlock::Text { text: t.into() }),
                    Some("localImage") => part["path"].as_str().map(|path| MessageBlock::Image {
                        media_type: None,
                        source: crate::conversation::ImageSource::Path { path: path.into() },
                    }),
                    Some("image") => Some(
                        part["url"]
                            .as_str()
                            .and_then(crate::conversation::ImageSource::from_data_url)
                            .map_or_else(
                                || MessageBlock::Notice {
                                    text: "图片输入".into(),
                                },
                                |(media_type, source)| MessageBlock::Image { media_type, source },
                            ),
                    ),
                    _ => None,
                })
                .collect();
            (MessageRole::User, blocks)
        }
        "agentMessage" => (
            MessageRole::Assistant,
            vec![MessageBlock::Text {
                text: item["text"].as_str().unwrap_or_default().into(),
            }],
        ),
        "reasoning" => {
            let text = item["summary"]
                .as_array()
                .into_iter()
                .flatten()
                .filter_map(Value::as_str)
                .collect::<Vec<_>>()
                .join("\n");
            if text.is_empty() {
                return None;
            }
            (
                MessageRole::Assistant,
                vec![MessageBlock::Reasoning { text }],
            )
        }
        "commandExecution" => (
            MessageRole::Tool,
            vec![MessageBlock::Tool {
                tool_id: Some(id.into()),
                title: item["command"].as_str().unwrap_or("命令执行").into(),
                state: match item["status"].as_str() {
                    Some("completed") => ToolState::Completed,
                    Some("failed" | "declined") => ToolState::Failed,
                    Some("inProgress") => ToolState::Running,
                    _ => ToolState::Pending,
                },
                output: item["aggregatedOutput"].as_str().map(str::to_owned),
                conversation_id: None,
            }],
        ),
        "fileChange" => (
            MessageRole::Tool,
            vec![MessageBlock::Tool {
                tool_id: Some(id.into()),
                title: "文件修改".into(),
                state: match item["status"].as_str() {
                    Some("completed") => ToolState::Completed,
                    Some("failed" | "declined") => ToolState::Failed,
                    Some("inProgress") => ToolState::Running,
                    _ => ToolState::Pending,
                },
                output: item["aggregatedOutput"].as_str().map(str::to_owned),
                conversation_id: None,
            }],
        ),
        "mcpToolCall" | "dynamicToolCall" => (
            MessageRole::Tool,
            vec![MessageBlock::Tool {
                tool_id: Some(id.into()),
                title: item["tool"].as_str().unwrap_or("工具调用").into(),
                state: match item["status"].as_str() {
                    Some("completed") => ToolState::Completed,
                    Some("failed" | "declined") => ToolState::Failed,
                    Some("inProgress") => ToolState::Running,
                    _ => ToolState::Pending,
                },
                output: item["aggregatedOutput"]
                    .as_str()
                    .map(str::to_owned)
                    .or_else(|| mcp_output(item)),
                conversation_id: None,
            }],
        ),
        // A native sub-agent call names the sub-agent threads it started or
        // addressed; one thread is the call's conversation.
        "collabAgentToolCall" => {
            let receivers = item["receiverThreadIds"]
                .as_array()
                .map(Vec::as_slice)
                .unwrap_or_default();
            let states = receivers.iter().filter_map(|thread| {
                let state = &item["agentsStates"][thread.as_str()?];
                let status = state["status"].as_str()?;
                Some(match state["message"].as_str() {
                    Some(message) => format!("{status}: {message}"),
                    None => status.to_owned(),
                })
            });
            let output = item["prompt"]
                .as_str()
                .map(str::to_owned)
                .into_iter()
                .chain(states)
                .collect::<Vec<_>>()
                .join("\n");
            (
                MessageRole::Tool,
                vec![MessageBlock::Tool {
                    tool_id: Some(id.into()),
                    title: item["tool"].as_str().unwrap_or("sub-agent").into(),
                    state: match item["status"].as_str() {
                        Some("completed") => ToolState::Completed,
                        Some("failed") => ToolState::Failed,
                        Some("inProgress") => ToolState::Running,
                        _ => ToolState::Pending,
                    },
                    output: (!output.is_empty()).then_some(output),
                    conversation_id: match receivers {
                        [thread] => thread
                            .as_str()
                            .map(|thread| format!("{runtime_id}:{thread}")),
                        _ => None,
                    },
                }],
            )
        }
        // Codex records a native sub-agent's life in its parent's history
        // instead of a call: its start is the conversation's link to it, named
        // by the last segment of its agent path; the rest shows in its own.
        "subAgentActivity" if item["kind"] == "started" => {
            let thread = item["agentThreadId"].as_str()?;
            (
                MessageRole::Tool,
                vec![MessageBlock::Tool {
                    tool_id: Some(id.into()),
                    title: item["agentPath"]
                        .as_str()
                        .and_then(|path| path.rsplit('/').find(|segment| !segment.is_empty()))
                        .unwrap_or("sub-agent")
                        .into(),
                    state: ToolState::Completed,
                    output: None,
                    conversation_id: Some(format!("{runtime_id}:{thread}")),
                }],
            )
        }
        "plan" => (
            MessageRole::System,
            vec![MessageBlock::Notice {
                text: item["text"].as_str().unwrap_or_default().into(),
            }],
        ),
        "contextCompaction" => (
            MessageRole::System,
            vec![MessageBlock::ContextCompaction {
                text: "上下文已压缩".into(),
            }],
        ),
        _ => return None,
    };
    Some(Message {
        turn_settings: None,
        turn_usage: None,
        branch_point: None,
        completed: true,
        id: id.into(),
        role: kind,
        timestamp_unix_ms: item["startedAtMs"].as_i64(),
        blocks,
    })
}

/// An MCP call's result as text: its structured content when the server gave
/// one, else its text content, or the error message.
fn mcp_output(item: &Value) -> Option<String> {
    let result = &item["result"];
    if !result["structuredContent"].is_null() {
        return Some(result["structuredContent"].to_string());
    }
    let text = result["content"]
        .as_array()
        .into_iter()
        .flatten()
        .filter_map(|content| content["text"].as_str())
        .collect::<Vec<_>>()
        .join("\n");
    (!text.is_empty())
        .then_some(text)
        .or_else(|| item["error"]["message"].as_str().map(str::to_owned))
}

/// A user message as Codex input items; Codex reads attached images from
/// their paths itself.
fn codex_input(input: &crate::TurnInput) -> Vec<Value> {
    (!input.text.is_empty())
        .then(|| json!({"type":"text", "text":input.text}))
        .into_iter()
        .chain(
            input
                .images
                .iter()
                .map(|image| json!({"type":"localImage", "path":image.path})),
        )
        .collect()
}

/// Another thread of the conversation's app-server while it works.
pub(super) struct CodexThread {
    pub(super) state: BackgroundState,
    /// Its running turn, which stopping it interrupts.
    turn: Option<String>,
}
impl Default for CodexThread {
    fn default() -> Self {
        Self {
            state: BackgroundState::Running,
            turn: None,
        }
    }
}

/// 0.166 resumes a saved provider only when it matches startup selection.
/// A change of provider requires a new context; never rewrite saved metadata
/// or overlay a native provider's credentials with gateway configuration.
pub fn resume_provider_matches(
    config: &crate::history::HistoryConfig,
    id: &str,
    managed: bool,
    cwd: Option<&str>,
) -> Result<bool> {
    let mut rpc = history_client_at(
        config.kind,
        &config.binary,
        &config.home,
        &config.execution,
        config.os_home.as_deref(),
    )?;
    let metadata = rpc.request("thread/read", &json!({"threadId":id,"includeTurns":false}))?;
    let saved = metadata["thread"]["modelProvider"]
        .as_str()
        .ok_or_else(|| Error::new("Codex 会话缺少提供商身份"))?;
    let selected = if managed {
        "velune".to_owned()
    } else {
        let config = rpc.request("config/read", &json!({"includeLayers":false,"cwd":cwd}))?;
        config["config"]["model_provider"]
            .as_str()
            .unwrap_or("openai")
            .to_owned()
    };
    Ok(saved == selected)
}

pub fn update_read_state(
    config: &crate::history::HistoryConfig,
    runtime: &str,
    receipt: &NativeReadState,
    read: bool,
) -> Result<Retained<NativeReadState>> {
    let id = receipt
        .conversation_id
        .strip_prefix(&format!("{runtime}:"))
        .ok_or_else(|| Error::new("原生已读状态不属于此运行时"))?;
    let mut rpc = history_client_at(
        config.kind,
        &config.binary,
        &config.home,
        &config.execution,
        config.os_home.as_deref(),
    )?;
    let result = rpc.request_classified(
        "thread/readState/update",
        &json!({"threadId":id,"expectedRevision":receipt.revision,
        "operation":{"type":if read {"read"} else {"unread"}}}),
        |error| {
            (error["code"] == -32600 && error["data"]["reason"] == "readStateConflict")
                .then_some("read_state_conflict")
        },
    )?;
    result.try_map(|result| {
        super::codex_history::read_state(result.get("readState"), runtime, id)?
            .ok_or_else(|| Error::new("Codex 未返回已读状态"))
    })
}

#[cfg(test)]
mod tests;

#[cfg(all(test, unix))]
mod question_reconciliation_tests;
