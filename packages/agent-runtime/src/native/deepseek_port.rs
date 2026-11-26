//! DeepSeek Harness 0.2.0-rc.2 ACP v1 port, not a generic ACP capability surface.
use super::*;
use crate::creation::{CreationBudget, CreationOutcome, detail as creation_detail};
use crate::transport::{WriteClass, WriteOutcome, WriteReceipt};
use std::{
    collections::VecDeque,
    sync::atomic::{AtomicU64, Ordering},
    time::Instant,
};

const OPERATIONS: usize = 32;
const REPLY_RESERVE: usize = 8;
const WRITES: usize = 16;
const DEFAULT_BYTES: usize = 8 * 1024 * 1024;
const REPLY_BYTES: usize = 1024 * 1024;
static GENERATION: AtomicU64 = AtomicU64::new(0);

/// Initialization, cwd validation, overlay/catalog IO and restart are blocking preparation,
/// deliberately absent here. Resume uses caller history; the Harness does not replay it.
pub enum DshCommand {
    New {
        cwd: PathBuf,
        runtime_id: String,
    },
    Resume {
        native_id: String,
        cwd: PathBuf,
        runtime_id: String,
        history: Retained<Vec<Message>>,
    },
    Close,
    SetModel {
        model: Retained<RuntimeModel>,
    },
    Prompt {
        text: String,
        run_deadline: Option<Instant>,
    },
    Cancel,
    PermissionReply {
        interaction_id: String,
        reply: RuntimeInteractionReply,
    },
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct DshTicket {
    pub session: String,
    pub generation: u64,
    pub sequence: u64,
    /// Null only for the one-way cancel notification.
    pub wire_id: Value,
}
#[derive(Clone, Debug, PartialEq)]
pub struct DshNativeError {
    pub code: i32,
    pub message: String,
    pub data: Option<Value>,
}
#[derive(Clone, Debug, PartialEq)]
pub enum DshReceiptStage {
    Written,
    /// The physical writer skipped the queued frame without attempting a byte.
    NotWritten {
        message: String,
    },
    ProtocolSuccess {
        result: Value,
    },
    ProtocolRejected {
        error: DshNativeError,
    },
    TurnCompleted {
        stop_reason: String,
    },
    /// A prompt error is a turn outcome, not evidence that execution never started.
    TurnError {
        error: DshNativeError,
    },
    Indeterminate {
        message: String,
    },
}
#[derive(Clone, Debug, PartialEq)]
pub struct DshReceipt {
    pub ticket: DshTicket,
    pub stage: DshReceiptStage,
}
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum DshAdmissionError {
    NeedsPreparation(String),
    NotDispatched { code: String, message: String },
}
impl From<Error> for DshAdmissionError {
    fn from(error: Error) -> Self {
        Self::NotDispatched {
            code: error.code().into(),
            message: error.to_string(),
        }
    }
}

enum Control {
    New { cwd: PathBuf, runtime_id: String },
    Resume { native_id: String },
    Close,
    Model(RuntimeModel),
}
enum Kind {
    Control(Control),
    Prompt { run_deadline: Option<Instant> },
    Reply(Option<String>),
    Cancel,
}
impl Kind {
    fn write_class(&self) -> WriteClass {
        match self {
            Self::Reply(_) | Self::Cancel => WriteClass::ProtocolReplyCancel,
            Self::Prompt { .. } | Self::Control(_) => WriteClass::Ordinary,
        }
    }
}
#[derive(serde::Serialize)]
struct RequestFrame<'a, P> {
    jsonrpc: &'static str,
    #[serde(skip_serializing_if = "Option::is_none")]
    id: Option<&'a Value>,
    method: &'a str,
    params: &'a P,
}
#[derive(serde::Serialize)]
struct ResponseFrame<'a> {
    jsonrpc: &'static str,
    id: &'a Value,
    result: &'a Value,
}
#[derive(serde::Serialize)]
#[serde(rename_all = "camelCase")]
struct PromptParams<'a> {
    session_id: &'a str,
    prompt: [TextBlock<'a>; 1],
}
#[derive(serde::Serialize)]
struct TextBlock<'a> {
    #[serde(rename = "type")]
    kind: &'static str,
    text: &'a str,
}

struct Operation {
    ticket: DshTicket,
    kind: Kind,
    deadline: Instant,
    write: Option<WriteReceipt>,
    bytes: usize,
    terminal: bool,
    outcome_seen: bool,
    evidence: VecDeque<Retained<DshReceiptStage>>,
    retention: DataRetention,
    creation: Option<CreationBudget>,
    creation_result: Option<Retained<CreationOutcome>>,
    creation_known: bool,
    creation_sealed: bool,
}
pub(super) struct Port {
    generation: u64,
    next: u64,
    blocked: bool,
    operations: BTreeMap<u64, Operation>,
    byte_budget: usize,
    reserved_bytes: usize,
    reserved_reply_bytes: usize,
    early: VecDeque<(Retained<Value>, usize)>,
    early_bytes: usize,
    permission_deadlines: BTreeMap<String, Option<Instant>>,
    cancellations: BTreeMap<String, Instant>,
    #[cfg(test)]
    test_now: Option<Instant>,
    #[cfg(test)]
    terminal_now: Option<Instant>,
}
impl Port {
    pub(super) fn blocked(&self) -> bool {
        self.blocked
    }
}

impl NativeSession {
    pub fn dsh_enqueue_creation(
        &mut self,
        cwd: PathBuf,
        runtime_id: String,
        budget: CreationBudget,
        deadline: Instant,
    ) -> std::result::Result<Retained<DshTicket>, DshAdmissionError> {
        self.rpc.arm_creation(&budget)?;
        self._creation_cleanup = Some(self.rpc.creation_cleanup_lease());
        match self.dsh_enqueue(DshCommand::New { cwd, runtime_id }, deadline) {
            Ok(ticket) => {
                self.dsh_port
                    .as_mut()
                    .unwrap()
                    .operations
                    .get_mut(&ticket.sequence)
                    .unwrap()
                    .creation = Some(budget);
                Ok(ticket)
            }
            Err(error) => {
                self.rpc.cancel_creation_admission(&budget);
                Err(error)
            }
        }
    }
    #[cfg(test)]
    pub(crate) fn dsh_expire_creation_test(&mut self, ticket: &DshTicket) {
        let op = self
            .dsh_port
            .as_mut()
            .unwrap()
            .operations
            .get_mut(&ticket.sequence)
            .unwrap();
        assert!(op.creation.is_some() && op.write.is_none() && !op.creation_known);
        op.deadline = Instant::now() - std::time::Duration::from_millis(1);
    }
    pub fn dsh_take_creation_result(
        &mut self,
        ticket: &DshTicket,
    ) -> Option<Retained<CreationOutcome>> {
        let op = self
            .dsh_port
            .as_mut()?
            .operations
            .get_mut(&ticket.sequence)?;
        (op.ticket == *ticket)
            .then(|| op.creation_result.take())
            .flatten()
    }
    pub fn dsh_settle_creation_result(&mut self, ticket: &DshTicket) -> Result<()> {
        if self.dsh_dispatch_blocked() || self.process_ended {
            return Err(Error::new("DeepSeek creation requires actual cleanup"));
        }
        let op = self
            .dsh_port
            .as_ref()
            .and_then(|port| port.operations.get(&ticket.sequence))
            .filter(|op| {
                op.ticket == *ticket
                    && op.terminal
                    && op.write.is_none()
                    && op.creation_known
                    && op.creation_result.is_none()
            })
            .ok_or_else(|| Error::new("DeepSeek creation result/write not settled"))?;
        let budget = op
            .creation
            .as_ref()
            .ok_or_else(|| Error::new("DeepSeek creation policy missing"))?
            .clone();
        self.rpc.complete_creation(&budget)?;
        self.dsh_port
            .as_mut()
            .unwrap()
            .operations
            .get_mut(&ticket.sequence)
            .unwrap()
            .creation_sealed = true;
        Ok(())
    }
    /// Acknowledges the caller's strict association commit; until then no prompt
    /// or second creation is admitted, even after receive resources were sealed.
    pub fn dsh_complete_creation(&mut self, ticket: &DshTicket) -> Result<()> {
        let op = self
            .dsh_port
            .as_mut()
            .and_then(|port| port.operations.get_mut(&ticket.sequence))
            .filter(|op| op.ticket == *ticket && op.creation_sealed)
            .ok_or_else(|| {
                Error::new("creation result must be sealed before association acknowledgement")
            })?;
        op.creation = None;
        Ok(())
    }
    fn dsh_capture_creation_record(&mut self, record: &Value) -> Result<bool> {
        if record["jsonrpc"] != "2.0" {
            return Err(Error::new("creation JSON-RPC version invalid"));
        }
        if record.get("method").is_some() {
            return Ok(false);
        }
        let Some(sequence) = self.dsh_port.as_ref().and_then(|port| {
            port.operations.iter().find_map(|(sequence, op)| {
                (op.creation.is_some()
                    && op.ticket.wire_id == record["id"]
                    && op.ticket.generation == port.generation)
                    .then_some(*sequence)
            })
        }) else {
            return Ok(false);
        };
        if self.dsh_port.as_ref().unwrap().operations[&sequence].creation_known {
            return Ok(true);
        }
        let budget = self.dsh_port.as_ref().unwrap().operations[&sequence]
            .creation
            .as_ref()
            .unwrap()
            .clone();
        let _ = self.dsh_settle_barrier();
        let outcome = if let Some(error) = record.get("error") {
            match serde_json::from_value::<agent_client_protocol_schema::v1::Error>(error.clone()) {
                Ok(_) => CreationOutcome::Rejected {
                    message: creation_detail(error),
                },
                // Match the existing non-budgeted schema check; error:null is
                // Unknown and must not suppress a later correlated Created ID.
                Err(cause) => CreationOutcome::Unknown {
                    message: creation_detail(cause),
                },
            }
        } else if let Some(id) = record["result"]["sessionId"]
            .as_str()
            .filter(|id| !id.is_empty())
        {
            CreationOutcome::Created(budget.created(id).retain_data(self.data.branch()))
        } else {
            CreationOutcome::Unknown {
                message: "DeepSeek new response has no complete identity".into(),
            }
        };
        let within = matches!(&outcome, CreationOutcome::Created(id) if id.within_policy());
        let known = !matches!(&outcome, CreationOutcome::Unknown { .. });
        {
            let op = self
                .dsh_port
                .as_mut()
                .unwrap()
                .operations
                .get_mut(&sequence)
                .unwrap();
            if op.creation_known {
                return Ok(true);
            }
            op.creation_known = known;
            op.creation_result = Some(Retained::from_parts(outcome, self.data.branch()));
            op.outcome_seen = true;
            op.terminal = true;
        }
        if within && !self.dsh_dispatch_blocked() {
            let (cwd, runtime_id) =
                match &self.dsh_port.as_ref().unwrap().operations[&sequence].kind {
                    Kind::Control(Control::New { cwd, runtime_id }) => {
                        (cwd.clone(), runtime_id.clone())
                    }
                    _ => unreachable!(),
                };
            let id = record["result"]["sessionId"]
                .as_str()
                .expect("captured complete identity");
            self.replace_snapshot(id, Some(&cwd), &runtime_id, Vec::new());
            self.data.replace(DataOwner::Identity);
            self.dsh_install_catalog(&record["result"])?;
            let early = {
                let port = self.dsh_port.as_mut().unwrap();
                port.early_bytes = 0;
                std::mem::take(&mut port.early)
            };
            for (update, _) in early {
                self.receive_native_record(update, Self::dsh_port_record)?;
            }
        } else {
            self.dsh_latch_failure("DeepSeek creation needs recovery; not replayed");
        }
        Ok(true)
    }

    /// Opt in on an initialized, prepared, idle connection. Synchronous mutators then reject.
    pub fn enable_dsh_command_port(&mut self) -> std::result::Result<(), DshAdmissionError> {
        self.enable_dsh_command_port_with_byte_budget(DEFAULT_BYTES)
    }

    /// Ordinary encoded capacity is bounded separately from the 1 MiB reply reserve.
    /// The exact encoded bytes remain reserved until flush or explicit cleanup, not receipt
    /// consumption. A caller needing larger legal text frames can prepare a larger budget;
    /// over-budget admission is explicit, without truncation or dispatch.
    pub fn enable_dsh_command_port_with_byte_budget(
        &mut self,
        byte_budget: usize,
    ) -> std::result::Result<(), DshAdmissionError> {
        if self.config.kind != NativeKind::DeepSeek || self.failed || self.process_ended {
            return Err(DshAdmissionError::NeedsPreparation(
                "DeepSeek requires a prepared live connection".into(),
            ));
        }
        if self.dsh_port.is_some() {
            return Err(DshAdmissionError::NeedsPreparation(
                "DeepSeek port already enabled; budgets cannot change in flight".into(),
            ));
        }
        if byte_budget == 0
            || self.turn_running()
            || !self.pending_rpc.is_empty()
            || self.has_pending_requests()
            || self.rpc.has_pending_writes()
            || self.restart_needed()
        {
            return Err(DshAdmissionError::NeedsPreparation(
                "DeepSeek requires idle preparation before enabling the port".into(),
            ));
        }
        let generation = GENERATION
            .try_update(Ordering::Relaxed, Ordering::Relaxed, |n| n.checked_add(1))
            .map_err(|_| Error::new("DeepSeek generation exhausted"))?
            + 1;
        self.rpc.defer_cleanup();
        self.dsh_port = Some(Port {
            generation,
            next: 0,
            blocked: false,
            operations: BTreeMap::new(),
            byte_budget,
            reserved_bytes: 0,
            reserved_reply_bytes: 0,
            early: VecDeque::new(),
            early_bytes: 0,
            permission_deadlines: BTreeMap::new(),
            cancellations: BTreeMap::new(),
            #[cfg(test)]
            test_now: None,
            #[cfg(test)]
            terminal_now: None,
        });
        Ok(())
    }

    /// Explicit blocking preparation; never called from enqueue/poll. Keeps the private
    /// catalog persistence/new/close/finish/remove lifecycle of this Harness version.
    pub fn dsh_prepare_model_catalog_blocking(&mut self) -> Result<Retained<Vec<ModelOption>>> {
        if self.config.kind != NativeKind::DeepSeek || self.dsh_port.is_some() {
            return Err(Error::new(
                "Prepare DeepSeek catalog before enabling its command port",
            ));
        }
        self.ensure_idle()?;
        self.dsh_models()
    }

    pub(super) fn dsh_require_sync_facade(&self) -> Result<()> {
        if self.dsh_port.is_some() {
            return Err(Error::with_code(
                "DeepSeek typed port enabled; use tracked enqueue",
                "operation_pending",
            ));
        }
        Ok(())
    }
    pub(super) fn dsh_dispatch_blocked(&self) -> bool {
        self.failed || self.dsh_port.as_ref().is_some_and(|port| port.blocked)
    }
    pub(super) fn dsh_routing_identity(&self) -> Option<&str> {
        self.native_id.as_deref().or_else(|| {
            self.dsh_port
                .as_ref()?
                .operations
                .values()
                .find_map(|op| match &op.kind {
                    Kind::Control(Control::Resume { native_id }) if !op.outcome_seen => {
                        Some(native_id.as_str())
                    }
                    _ => None,
                })
        })
    }
    fn dsh_active_deadline(&self) -> Option<Instant> {
        let id = self.turn_id.as_ref()?;
        self.dsh_port.as_ref()?.operations.values().find_map(|op| {
            if op.ticket.wire_id.as_str() == Some(id)
                && !op.outcome_seen
                && let Kind::Prompt { run_deadline } = op.kind
            {
                run_deadline
            } else {
                None
            }
        })
    }
    fn dsh_control_pending(&self) -> bool {
        self.dsh_port.as_ref().is_some_and(|port| {
            port.operations
                .values()
                .any(|op| matches!(op.kind, Kind::Control(_)) && !op.outcome_seen)
        })
    }
    fn dsh_capacity(&self, reply: bool) -> Result<()> {
        let port = self.dsh_port.as_ref().unwrap();
        let writes = port
            .operations
            .values()
            .filter(|op| op.write.is_some())
            .count();
        if port.operations.len()
            >= if reply {
                OPERATIONS
            } else {
                OPERATIONS - REPLY_RESERVE
            }
            || writes >= if reply { WRITES } else { WRITES - 4 }
        {
            return Err(Error::with_code(
                "DeepSeek operation/receipt capacity full; not dispatched",
                "queue_full",
            ));
        }
        Ok(())
    }
    fn dsh_next_sequence(&self) -> Result<u64> {
        self.dsh_port
            .as_ref()
            .unwrap()
            .next
            .checked_add(1)
            .ok_or_else(|| Error::new("DeepSeek operation sequence exhausted"))
    }

    /// Deadline covers dispatch/write; controls also wait for their matching response under
    /// that same deadline. Prompt execution has only its independent optional absolute deadline.
    pub fn dsh_enqueue(
        &mut self,
        command: DshCommand,
        deadline: Instant,
    ) -> std::result::Result<Retained<DshTicket>, DshAdmissionError> {
        if self.dsh_port.is_none() || self.dsh_dispatch_blocked() || self.process_ended {
            return Err(DshAdmissionError::NeedsPreparation(
                "DeepSeek port requires preparation/cleanup".into(),
            ));
        }
        if self
            .dsh_port
            .as_ref()
            .unwrap()
            .operations
            .values()
            .any(|op| op.creation.is_some())
            && !matches!(
                command,
                DshCommand::Cancel | DshCommand::PermissionReply { .. }
            )
        {
            return Err(DshAdmissionError::NeedsPreparation(
                "DeepSeek creation awaits association or cleanup".into(),
            ));
        }
        // Observe physical timestamps before testing budgets, on enqueue as on poll.
        if let Err(error) = self.dsh_settle_barrier() {
            return Err(DshAdmissionError::NeedsPreparation(error.to_string()));
        }
        let reply = matches!(
            command,
            DshCommand::Cancel | DshCommand::PermissionReply { .. }
        );
        self.dsh_capacity(reply)?;
        let sequence = self.dsh_next_sequence()?;
        let wire_id = json!(format!(
            "dsh:{}:{sequence}",
            self.dsh_port.as_ref().unwrap().generation
        ));
        let mut prompt_text = None;
        let mut resume_snapshot = None;
        let mut command_input_retention = DataRetention::default();
        let admission_retention = self.data.try_clone(super::retention::command_data_owner)?;
        let public_retention = admission_retention.try_clone().map_err(retention_error)?;
        let (kind, method, params, wire_id, parent) = match command {
            DshCommand::New { cwd, runtime_id } => {
                self.dsh_establishment_ready(&cwd)?;
                (
                    Kind::Control(Control::New {
                        cwd: cwd.clone(),
                        runtime_id,
                    }),
                    "session/new",
                    json!({"cwd":cwd,"mcpServers":self.dsh_mcp_servers_port()}),
                    wire_id,
                    None,
                )
            }
            DshCommand::Resume {
                native_id,
                cwd,
                runtime_id,
                history,
            } => {
                self.dsh_establishment_ready(&cwd)?;
                if native_id.is_empty() {
                    return Err(Error::new("DeepSeek resume identity empty").into());
                }
                let (history, retention) = history.into_parts();
                command_input_retention = retention.try_clone().map_err(retention_error)?;
                resume_snapshot = Some((
                    native_id.clone(),
                    cwd.clone(),
                    runtime_id,
                    history,
                    retention,
                ));
                (
                    Kind::Control(Control::Resume {
                        native_id: native_id.clone(),
                    }),
                    "session/resume",
                    json!({"sessionId":native_id,"cwd":cwd,"mcpServers":self.dsh_mcp_servers_port()}),
                    wire_id,
                    None,
                )
            }
            DshCommand::Close => {
                self.dsh_idle_control()?;
                (
                    Kind::Control(Control::Close),
                    "session/close",
                    json!({"sessionId":self.native_id}),
                    wire_id,
                    None,
                )
            }
            DshCommand::SetModel { model } => {
                self.dsh_idle_control()?;
                let (model, retention) = model.into_parts();
                command_input_retention = retention;
                let provider_id = model.provider_id.as_deref().ok_or_else(|| {
                    Error::new("DeepSeek native selections require provider identity")
                })?;
                let model_id = model.id.as_str();
                if self.config.gateway.as_ref().is_some_and(|gateway| {
                    provider_id != "velune-gateway" || model_id != gateway.model_alias
                }) || (self.config.gateway.is_none() && provider_id == "velune-gateway")
                {
                    return Err(DshAdmissionError::NeedsPreparation(
                        "DeepSeek provider route change needs blocking preparation".into(),
                    ));
                }
                let advertised = self
                    .available_models
                    .iter()
                    .find(|option| option["id"] == "model")
                    .and_then(|option| option["options"].as_array())
                    .into_iter()
                    .flatten()
                    .flat_map(|group| group["options"].as_array().into_iter().flatten())
                    .filter_map(|option| option["value"].as_str())
                    .find(|value| {
                        serde_json::from_str::<Vec<String>>(value).is_ok_and(|pair| {
                            pair.len() == 2 && pair[0] == provider_id && pair[1] == model_id
                        })
                    })
                    .ok_or_else(|| {
                        Error::new("DeepSeek did not advertise the selected provider/model")
                    })?;
                let params =
                    json!({"sessionId":self.native_id,"configId":"model","value":advertised});
                (
                    Kind::Control(Control::Model(model)),
                    "session/set_config_option",
                    params,
                    wire_id,
                    None,
                )
            }
            DshCommand::Prompt { text, run_deadline } => {
                self.dsh_idle_control()?;
                if self.restart_needed() {
                    return Err(DshAdmissionError::NeedsPreparation(
                        "DeepSeek requires explicit blocking restart preparation".into(),
                    ));
                }
                if text.trim().is_empty() {
                    return Err(Error::new("DeepSeek prompt empty").into());
                }
                let selected_route = self.native_selection.as_ref().is_some_and(|model| {
                    if let Some(gateway) = &self.config.gateway {
                        model.provider_id.as_deref() == Some("velune-gateway")
                            && model.id == gateway.model_alias
                    } else {
                        model
                            .provider_id
                            .as_deref()
                            .is_some_and(|provider| provider != "velune-gateway")
                    }
                });
                if !selected_route {
                    return Err(
                        Error::new("Select an existing DeepSeek provider/model first").into(),
                    );
                }
                // Keep the caller-owned text intact until the bounded codec borrows it.
                // Building a Value here would clone the entire text before its budget check.
                let params = Value::Null;
                prompt_text = Some(text);
                (
                    Kind::Prompt { run_deadline },
                    "session/prompt",
                    params,
                    wire_id,
                    run_deadline,
                )
            }
            DshCommand::Cancel => {
                if !self.turn_running() {
                    return Err(Error::new("DeepSeek has no active turn to cancel").into());
                }
                (
                    Kind::Cancel,
                    "session/cancel",
                    json!({"sessionId":self.native_id}),
                    Value::Null,
                    self.dsh_active_deadline(),
                )
            }
            DshCommand::PermissionReply {
                interaction_id,
                reply,
            } => {
                if self.dsh_reply_pending(&interaction_id) {
                    return Err(Error::with_code(
                        "DeepSeek reply already queued",
                        "operation_pending",
                    )
                    .into());
                }
                let pending = self
                    .interactions
                    .get(&interaction_id)
                    .ok_or_else(|| Error::new("DeepSeek permission expired"))?;
                let cancelling = matches!(reply, RuntimeInteractionReply::Cancel)
                    && self
                        .dsh_port
                        .as_ref()
                        .unwrap()
                        .cancellations
                        .contains_key(&interaction_id);
                let params = if cancelling {
                    // The request was routed under a known expected resume identity before
                    // rejection/turn completion; cancellation still owes its exact wire ID.
                    json!({"outcome":{"outcome":"cancelled"}})
                } else {
                    self.dsh_permission_result(pending, reply)?
                };
                let parent = self
                    .dsh_port
                    .as_ref()
                    .unwrap()
                    .permission_deadlines
                    .get(&interaction_id)
                    .copied()
                    .flatten();
                (
                    Kind::Reply(Some(interaction_id)),
                    "",
                    params,
                    pending.id.clone(),
                    parent,
                )
            }
        };
        let deadline = parent.map_or(deadline, |parent| parent.min(deadline));
        let deadline = if reply {
            deadline.min(Instant::now() + rpc::CONTROL_TIMEOUT)
        } else {
            deadline
        };
        if Instant::now() >= deadline {
            return Err(
                Error::with_code("DeepSeek command expired; not dispatched", "timeout").into(),
            );
        }
        let ticket = if let Some(text) = prompt_text.as_deref() {
            let session_id = self.native_id.as_deref().unwrap().to_owned();
            let params = PromptParams {
                session_id: &session_id,
                prompt: [TextBlock { kind: "text", text }],
            };
            let frame = RequestFrame {
                jsonrpc: "2.0",
                id: Some(&wire_id),
                method,
                params: &params,
            };
            self.dsh_dispatch(sequence, wire_id.clone(), kind, &frame, deadline)?
        } else if matches!(kind, Kind::Reply(_)) {
            let frame = ResponseFrame {
                jsonrpc: "2.0",
                id: &wire_id,
                result: &params,
            };
            self.dsh_dispatch(sequence, wire_id.clone(), kind, &frame, deadline)?
        } else {
            let frame = RequestFrame {
                jsonrpc: "2.0",
                id: if matches!(kind, Kind::Cancel) {
                    None
                } else {
                    Some(&wire_id)
                },
                method,
                params: &params,
            };
            self.dsh_dispatch(sequence, wire_id.clone(), kind, &frame, deadline)?
        };
        let op = self
            .dsh_port
            .as_mut()
            .unwrap()
            .operations
            .get_mut(&ticket.sequence)
            .unwrap();
        op.retention = std::mem::take(&mut op.retention).merge(command_input_retention);
        if let Some((native_id, cwd, runtime_id, history, retention)) = resume_snapshot {
            // Explicit expected identity routes early resume updates, but is not a confirmed send target.
            self.replace_snapshot(&native_id, Some(&cwd), &runtime_id, history);
            self.install_history_data(retention);
            self.native_id = None;
        }
        if let Some(text) = prompt_text {
            if let Some(snapshot) = &mut self.snapshot {
                snapshot.executions.begin(ExecutionKind::UserTurn, false);
            }
            self.turn_id = ticket.wire_id.as_str().map(str::to_owned);
            self.message(Message {
                turn_settings: None,
                turn_usage: None,
                branch_point: None,
                completed: true,
                id: format!("{}:user:{sequence}", self.epoch),
                role: MessageRole::User,
                timestamp_unix_ms: None,
                blocks: vec![MessageBlock::Text { text }],
            });
            self.running();
        }
        if method == "session/cancel" {
            if let Some(snapshot) = &mut self.snapshot {
                snapshot.run_state = RunState::Stopping;
                snapshot.revision += 1;
            }
            self.dsh_mark_cancellations();
        }
        Ok(Retained::from_parts(ticket, public_retention))
    }

    fn dsh_mcp_servers_port(&self) -> Vec<Value> {
        self.config
            .mcp_servers
            .iter()
            .map(crate::mcp::Server::acp_http)
            .collect()
    }
    fn dsh_establishment_ready(&self, cwd: &Path) -> std::result::Result<(), DshAdmissionError> {
        if !cwd.is_absolute()
            || self.native_id.is_some()
            || self.dsh_control_pending()
            || self.turn_running()
            || self.has_pending_requests()
        {
            return Err(DshAdmissionError::NeedsPreparation(
                "Use a validated absolute cwd and explicitly close the previous DeepSeek session"
                    .into(),
            ));
        }
        Ok(())
    }
    fn dsh_idle_control(&self) -> Result<()> {
        if self.native_id.is_none()
            || self.turn_running()
            || self.dsh_control_pending()
            || self.has_pending_requests()
            || self
                .dsh_port
                .as_ref()
                .unwrap()
                .operations
                .values()
                .any(|op| !op.terminal)
        {
            return Err(Error::with_code(
                "DeepSeek requires a confirmed idle session",
                "operation_pending",
            ));
        }
        Ok(())
    }
    fn dsh_dispatch(
        &mut self,
        sequence: u64,
        wire_id: Value,
        kind: Kind,
        frame: &impl serde::Serialize,
        deadline: Instant,
    ) -> Result<DshTicket> {
        self.dsh_settle_barrier()?;
        if self.dsh_dispatch_blocked() {
            return Err(Error::with_code(
                "DeepSeek failed owner rejects dispatch",
                "transport",
            ));
        }
        let retention = self
            .data
            .try_clone(super::retention::command_data_owner)?
            .merge(self.data.try_clone_current()?);
        let class = kind.write_class();
        let port = self.dsh_port.as_ref().unwrap();
        let remaining = match class {
            WriteClass::Ordinary => port
                .byte_budget
                .saturating_sub(port.reserved_bytes - port.reserved_reply_bytes),
            WriteClass::ProtocolReplyCancel => {
                REPLY_BYTES.saturating_sub(port.reserved_reply_bytes)
            }
        };
        if remaining == 0 {
            return Err(Error::with_code(
                "DeepSeek encoded bytes budget full; not dispatched",
                "queue_full",
            ));
        }
        let limit = remaining.min(crate::transport::writer_policy_limit());
        let frame = match self.rpc.encode_frame(frame, class, limit, deadline) {
            Ok(frame) => frame,
            Err(error) => {
                let mut error = rpc::admission_error(error);
                if error.code() == "allocation_budget"
                    && remaining < crate::transport::writer_policy_limit()
                {
                    error = Error::with_code(
                        "DeepSeek encoded bytes exceed prepared allocation budget; not dispatched",
                        "queue_full",
                    );
                }
                if error.code() == "transport" {
                    self.dsh_latch_failure(&error.to_string());
                }
                return Err(error);
            }
        };
        // Encoding can consume the remaining parent budget. Recheck before moving
        // this original allocation into the single physical FIFO.
        self.dsh_settle_barrier()?;
        let bytes = frame.bytes.capacity();
        let write = match self.rpc.try_enqueue_owned_frame(frame, class, deadline) {
            Ok(write) => write,
            Err(rejected) => {
                // Rejection returns the exact owned allocation, class and deadline.
                // Reclaim it here: no clone, silent replay or second pending queue.
                let error = rpc::admission_error(rejected.into_error());
                if error.code() == "transport" {
                    self.dsh_latch_failure(&error.to_string());
                }
                return Err(error);
            }
        };
        let port = self.dsh_port.as_mut().unwrap();
        let ticket = DshTicket {
            session: self.epoch.clone(),
            generation: port.generation,
            sequence,
            wire_id,
        };
        port.next = sequence;
        port.reserved_bytes += bytes;
        if class == WriteClass::ProtocolReplyCancel {
            port.reserved_reply_bytes += bytes;
        }
        port.operations.insert(
            sequence,
            Operation {
                ticket: ticket.clone(),
                kind,
                deadline,
                write: Some(write),
                bytes,
                terminal: false,
                outcome_seen: false,
                evidence: VecDeque::new(),
                retention,
                creation: None,
                creation_result: None,
                creation_known: false,
                creation_sealed: false,
            },
        );
        Ok(ticket)
    }

    pub fn dsh_take_receipts(&mut self) -> Result<Vec<Retained<DshReceipt>>> {
        let Some(port) = &mut self.dsh_port else {
            return Ok(Vec::new());
        };
        // Reserve every ticket copy before draining anything: a failed clone
        // must leave all already-observed native facts available to the caller.
        let mut credits = BTreeMap::new();
        for (sequence, op) in &port.operations {
            let mut copies = Vec::new();
            for _ in &op.evidence {
                copies.push(op.retention.try_clone().map_err(retention_error)?);
            }
            credits.insert(*sequence, copies);
        }
        let mut result = Vec::new();
        for (sequence, op) in &mut port.operations {
            for (stage, credit) in op.evidence.drain(..).zip(credits.remove(sequence).unwrap()) {
                let ticket = Retained::from_parts(op.ticket.clone(), credit);
                result.push(
                    ticket
                        .zip(stage)
                        .map(|(ticket, stage)| DshReceipt { ticket, stage }),
                );
            }
        }
        port.operations.retain(|_, op| {
            !op.terminal
                || op.write.is_some()
                || op.creation.is_some()
                || (port.blocked && !self.process_ended)
        });
        Ok(result)
    }
    pub fn dsh_has_pending_operations(&self) -> bool {
        self.dsh_port.as_ref().is_some_and(|port| {
            (port.blocked && !self.process_ended)
                || !port.cancellations.is_empty()
                || !port.early.is_empty()
                || port.operations.values().any(|op| {
                    !op.terminal
                        || op.write.is_some()
                        || op.creation.is_some()
                        || !op.evidence.is_empty()
                })
        }) || (self.config.kind == NativeKind::DeepSeek && self.has_pending_requests())
    }
    fn dsh_reply_pending(&self, id: &str) -> bool {
        self.dsh_port
            .as_ref()
            .unwrap()
            .operations
            .values()
            .any(|op| {
                matches!(&op.kind, Kind::Reply(Some(pending)) if pending == id) && !op.terminal
            })
    }
    pub(super) fn dsh_permission_capacity(&self, request_id: &Value) -> Result<()> {
        if self.dsh_port.is_some()
            && (self.interactions.len() >= OPERATIONS
                || self.interaction_sequence == u64::MAX
                || self
                    .interactions
                    .values()
                    .any(|pending| pending.id == *request_id))
        {
            return Err(Error::with_code(
                "DeepSeek permission identity/capacity invalid",
                "transport",
            ));
        }
        Ok(())
    }
    pub(super) fn dsh_capture_permission_deadline(&mut self) {
        let parent = self.dsh_active_deadline();
        if let Some(port) = &mut self.dsh_port {
            port.permission_deadlines.insert(
                format!("{}-{}", self.epoch, self.interaction_sequence),
                parent,
            );
        }
    }
    pub(super) fn dsh_auto_reply(
        &mut self,
        id: &Value,
        result: &Value,
        rejection: Option<&str>,
    ) -> Result<()> {
        if self.dsh_port.is_none() {
            return if let Some(message) = rejection {
                self.rpc.reject(id, message)
            } else {
                self.rpc.respond(id, result)
            };
        }
        self.dsh_settle_barrier()?;
        if self.dsh_dispatch_blocked() {
            return Ok(());
        }
        self.dsh_capacity(true)?;
        let sequence = self.dsh_next_sequence()?;
        // Wrong-session/unknown capability requests have no attributable parent run.
        let deadline = Instant::now() + rpc::CONTROL_TIMEOUT;
        let frame = if let Some(message) = rejection {
            json!({"jsonrpc":"2.0","id":id,"error":{"code":-32601,"message":message}})
        } else {
            json!({"jsonrpc":"2.0","id":id,"result":result})
        };
        self.dsh_dispatch(sequence, id.clone(), Kind::Reply(None), &frame, deadline)?;
        Ok(())
    }
    fn dsh_mark_cancellations(&mut self) {
        let short = Instant::now() + rpc::CONTROL_TIMEOUT;
        let port = self.dsh_port.as_mut().unwrap();
        for id in self.interactions.keys() {
            let parent = port.permission_deadlines.get(id).copied().flatten();
            port.cancellations
                .entry(id.clone())
                .or_insert_with(|| parent.map_or(short, |parent| parent.min(short)));
        }
    }
    fn dsh_schedule_cancellations(&mut self) -> Result<()> {
        let intents: Vec<_> = self
            .dsh_port
            .as_ref()
            .unwrap()
            .cancellations
            .iter()
            .map(|(id, deadline)| (id.clone(), *deadline))
            .collect();
        for (id, deadline) in intents {
            if self.dsh_reply_pending(&id) {
                continue;
            }
            match self.dsh_enqueue(
                DshCommand::PermissionReply {
                    interaction_id: id,
                    reply: RuntimeInteractionReply::Cancel,
                },
                deadline,
            ) {
                Ok(_) => {}
                Err(DshAdmissionError::NotDispatched { code, .. }) if code == "queue_full" => break,
                Err(error) => {
                    return Err(Error::new(format!(
                        "DeepSeek permission cancellation failed: {error:?}"
                    )));
                }
            }
        }
        Ok(())
    }
    fn dsh_remove_permission(&mut self, id: &str) {
        self.interactions.remove(id);
        if let Some(snapshot) = &mut self.snapshot {
            snapshot
                .pending_interactions
                .retain(|pending| pending.id != id);
            snapshot.revision += 1;
        }
        let port = self.dsh_port.as_mut().unwrap();
        port.permission_deadlines.remove(id);
        port.cancellations.remove(id);
    }
    fn dsh_now(&self) -> Instant {
        #[cfg(test)]
        if let Some(now) = self.dsh_port.as_ref().and_then(|port| port.test_now) {
            return now;
        }
        Instant::now()
    }
    fn dsh_expired(&self) -> bool {
        let port = self.dsh_port.as_ref().unwrap();
        let now = self.dsh_now();
        port.operations.values().any(|op| {
            !op.terminal
                && match op.kind {
                    Kind::Prompt { run_deadline } => {
                        run_deadline.is_some_and(|deadline| now >= deadline)
                            || (op.write.is_some() && now >= op.deadline)
                    }
                    _ => now >= op.deadline,
                }
        }) || port
            .permission_deadlines
            .values()
            .flatten()
            .chain(port.cancellations.values())
            .any(|deadline| now >= *deadline)
    }
    fn dsh_latch_failure(&mut self, message: &str) {
        let port = self.dsh_port.as_mut().unwrap();
        port.blocked = true;
        for op in port.operations.values_mut() {
            if op.creation.is_some() && !op.creation_known && op.creation_result.is_none() {
                op.creation_result = Some(Retained::from_parts(
                    CreationOutcome::Unknown {
                        message: creation_detail(message),
                    },
                    DataRetention::default(),
                ));
            }
            if !op.terminal {
                op.evidence
                    .push_back(local_evidence(DshReceiptStage::Indeterminate {
                        message: message.into(),
                    }));
                op.terminal = true;
            }
        }
    }
    fn dsh_harvest_writes(&mut self) -> Result<()> {
        let mut settled = Vec::new();
        let mut failure = None;
        let port = self.dsh_port.as_mut().unwrap();
        for op in port.operations.values_mut() {
            let Some(outcome) = op.write.as_ref().and_then(WriteReceipt::try_outcome) else {
                continue;
            };
            let physical_terminal = !matches!(outcome, WriteOutcome::ObservationTimeout { .. });
            match outcome {
                WriteOutcome::Written(completion) => {
                    op.evidence
                        .push_front(local_evidence(DshReceiptStage::Written));
                    if !completion.met_deadline() {
                        let message =
                            "DeepSeek actual flush completed after write deadline; not replayed";
                        if !op.terminal {
                            op.evidence
                                .push_back(local_evidence(DshReceiptStage::Indeterminate {
                                    message: message.into(),
                                }));
                            op.terminal = true;
                        }
                        failure.get_or_insert_with(|| Error::with_code(message, "timeout"));
                    }
                    match &op.kind {
                        Kind::Reply(id) => {
                            if let Some(id) = id {
                                settled.push(id.clone());
                            }
                            op.terminal = true;
                        }
                        Kind::Cancel => op.terminal = true,
                        _ => {}
                    }
                }
                WriteOutcome::NotWritten(error) => {
                    if op.creation.is_some() && !op.creation_known {
                        op.creation_result = Some(Retained::from_parts(
                            CreationOutcome::NotWritten {
                                message: creation_detail(&error),
                            },
                            DataRetention::default(),
                        ));
                        op.creation_known = true;
                    }
                    op.evidence
                        .push_back(local_evidence(DshReceiptStage::NotWritten {
                            message: error.to_string(),
                        }));
                    op.terminal = true;
                    failure.get_or_insert_with(|| {
                        Error::with_code(
                            format!("DeepSeek frame known not written: {error}"),
                            "transport",
                        )
                    });
                }
                WriteOutcome::Indeterminate(error) => {
                    failure.get_or_insert_with(|| {
                        Error::with_code(
                            format!("DeepSeek physical write indeterminate: {error}"),
                            "transport",
                        )
                    });
                }
                WriteOutcome::ObservationTimeout { .. } => {
                    failure.get_or_insert_with(|| Error::with_code("DeepSeek write outcome not yet observed; receipt and actual lease retained", "timeout"));
                }
            }
            if physical_terminal {
                // The shared writer publishes physical terminal facts only after dropping
                // its actual encoded buffer/lease. ObservationTimeout never enters here.
                op.write = None;
                port.reserved_bytes -= op.bytes;
                if op.kind.write_class() == WriteClass::ProtocolReplyCancel {
                    port.reserved_reply_bytes -= op.bytes;
                }
                op.bytes = 0;
            }
        }
        for id in settled {
            self.dsh_remove_permission(&id);
        }
        if let Some(error) = failure {
            return Err(error);
        }
        Ok(())
    }

    fn dsh_settle_barrier(&mut self) -> Result<()> {
        let mut failure = self.dsh_harvest_writes().err();
        if self.dsh_expired() {
            failure.get_or_insert_with(|| {
                Error::with_code(
                    "DeepSeek absolute deadline expired; not replayed",
                    "timeout",
                )
            });
        }
        if let Some(error) = failure {
            self.dsh_latch_failure(&error.to_string());
            return Err(error);
        }
        Ok(())
    }

    pub(super) fn dsh_poll_port(&mut self) -> Result<()> {
        if self.process_ended {
            return Ok(());
        }
        // Enqueue and poll settle the same physical facts before budget checks.
        let mut failure = self.dsh_settle_barrier().err();
        let records = match self.rpc.poll() {
            Ok(records) => records,
            Err(error) => {
                failure.get_or_insert(error);
                Vec::new()
            }
        };
        // poll may retain a valid response alongside a read failure. Cross the failure
        // barrier before handlers may dispatch automatic replies or accept a send target.
        if let Err(error) = self.rpc.check_failure() {
            failure.get_or_insert(error);
        }
        if let Some(error) = &failure {
            self.dsh_latch_failure(&error.to_string());
        }
        for record in records {
            if let Err(error) = self.receive_native_record(record, Self::dsh_port_record) {
                self.dsh_latch_failure(&error.to_string());
                failure.get_or_insert(error);
            }
        }
        if let Err(error) = self.dsh_settle_barrier() {
            failure.get_or_insert(error);
        }
        if failure.is_none()
            && !self.dsh_dispatch_blocked()
            && let Err(error) = self.dsh_schedule_cancellations()
        {
            failure = Some(error);
        }
        if let Some(error) = failure {
            self.dsh_latch_failure(&error.to_string());
            if !self.failed {
                self.fail_connection();
            }
            return Err(error);
        }
        if self.dsh_dispatch_blocked() && !self.failed {
            self.fail_connection();
        }
        Ok(())
    }
    fn dsh_port_record(&mut self, record: Value) -> Result<()> {
        if record["jsonrpc"] != "2.0" {
            return Err(Error::new("DeepSeek JSON-RPC version invalid"));
        }
        if self.dsh_capture_creation_record(&record)? {
            return Ok(());
        }
        if record.get("method").is_some() {
            let establishing_new = self
                .dsh_port
                .as_ref()
                .unwrap()
                .operations
                .values()
                .any(|op| {
                    matches!(op.kind, Kind::Control(Control::New { .. })) && !op.outcome_seen
                });
            if establishing_new
                && matches!(
                    record["method"].as_str(),
                    Some("session/update" | "session/request_permission")
                )
            {
                let bytes = serde_json::to_vec(&record)
                    .map_err(|error| Error::new(error.to_string()))?
                    .len();
                let port = self.dsh_port.as_mut().unwrap();
                if port.early.len() >= OPERATIONS
                    || bytes > port.byte_budget.saturating_sub(port.early_bytes)
                {
                    return Err(Error::with_code(
                        "DeepSeek unknown-identity early record budget full",
                        "transport",
                    ));
                }
                port.early_bytes += bytes;
                port.early
                    .push_back((Retained::from_parts(record, self.data.branch()), bytes));
                return Ok(());
            }
            if self.dsh_dispatch_blocked() {
                // Keep output facts, but never revive a permission channel or dispatch on failure.
                if record["method"] == "session/update" {
                    return self.dsh_handle_record(record);
                }
                return Ok(());
            }
            return self.dsh_handle_record(record);
        }
        let sequence =
            self.dsh_port
                .as_ref()
                .unwrap()
                .operations
                .iter()
                .find_map(|(sequence, op)| {
                    (!op.outcome_seen
                        && op.ticket.generation == self.dsh_port.as_ref().unwrap().generation
                        && op.ticket.wire_id == record["id"]
                        && matches!(op.kind, Kind::Control(_) | Kind::Prompt { .. }))
                    .then_some(*sequence)
                });
        let Some(sequence) = sequence else {
            return Ok(());
        };
        let stage = if let Some(error) = record.get("error") {
            let error: agent_client_protocol_schema::v1::Error =
                serde_json::from_value(error.clone()).map_err(|error| {
                    Error::new(format!("DeepSeek native error invalid: {error}"))
                })?;
            let error = DshNativeError {
                code: i32::from(error.code),
                message: error.message,
                data: error.data,
            };
            if matches!(
                self.dsh_port.as_ref().unwrap().operations[&sequence].kind,
                Kind::Prompt { .. }
            ) {
                DshReceiptStage::TurnError { error }
            } else {
                DshReceiptStage::ProtocolRejected { error }
            }
        } else if matches!(
            self.dsh_port.as_ref().unwrap().operations[&sequence].kind,
            Kind::Prompt { .. }
        ) {
            let response: agent_client_protocol_schema::v1::PromptResponse =
                serde_json::from_value(record["result"].clone()).map_err(|error| {
                    Error::new(format!("DeepSeek prompt result invalid: {error}"))
                })?;
            DshReceiptStage::TurnCompleted {
                stop_reason: serde_json::to_value(response.stop_reason)
                    .map_err(|error| Error::new(error.to_string()))?
                    .as_str()
                    .ok_or_else(|| Error::new("DeepSeek stop reason invalid"))?
                    .into(),
            }
        } else {
            if !record["result"].is_object() {
                return Err(Error::new("DeepSeek control result missing"));
            }
            match &self.dsh_port.as_ref().unwrap().operations[&sequence].kind {
                Kind::Control(Control::New { .. })
                    if record["result"]["sessionId"]
                        .as_str()
                        .is_none_or(str::is_empty) =>
                {
                    return Err(Error::new("DeepSeek new result missing identity"));
                }
                Kind::Control(Control::Resume { native_id })
                    if record["result"]
                        .get("sessionId")
                        .is_some_and(|id| id.as_str() != Some(native_id)) =>
                {
                    return Err(Error::new("DeepSeek resume result identity mismatch"));
                }
                _ => {}
            }
            DshReceiptStage::ProtocolSuccess {
                result: record["result"].clone(),
            }
        };
        let prompt = matches!(
            self.dsh_port.as_ref().unwrap().operations[&sequence].kind,
            Kind::Prompt { .. }
        );
        let rejected = matches!(stage, DshReceiptStage::ProtocolRejected { .. });
        #[cfg(test)]
        if let Some(now) = self.dsh_port.as_mut().unwrap().terminal_now.take() {
            self.dsh_port.as_mut().unwrap().test_now = Some(now);
        }
        // A batch can enter before D and reach this terminal after D. Preserve the
        // real response, but latch expiry before terminal state can erase its budget.
        let terminal_failure = self.dsh_settle_barrier().err();
        let healthy = !self.dsh_dispatch_blocked();
        // Remove only for the duration of state installation; generation correlation is fixed.
        let mut op = self
            .dsh_port
            .as_mut()
            .unwrap()
            .operations
            .remove(&sequence)
            .unwrap();
        op.outcome_seen = true;
        op.terminal = true;
        op.evidence
            .push_back(Retained::from_parts(stage, self.data.branch()));
        let install: Result<()> = (|| {
            if let Kind::Control(control) = &op.kind {
                if rejected {
                    if matches!(control, Control::New { .. } | Control::Resume { .. }) {
                        self.native_id = None;
                        self.dsh_mark_cancellations();
                        self.settle(RunState::Failed);
                    }
                } else if healthy {
                    match control {
                        Control::New { cwd, runtime_id } => {
                            let id = record["result"]["sessionId"]
                                .as_str()
                                .filter(|id| !id.is_empty())
                                .ok_or_else(|| {
                                    Error::new("DeepSeek new result missing identity")
                                })?;
                            self.replace_snapshot(id, Some(cwd), runtime_id, Vec::new());
                            self.data.replace(DataOwner::Identity);
                            self.dsh_install_catalog(&record["result"])?;
                        }
                        Control::Resume { native_id } => {
                            self.native_id = Some(native_id.clone());
                            self.dsh_install_catalog(&record["result"])?;
                        }
                        Control::Close => {
                            self.native_id = None;
                            self.clear_requests();
                        }
                        Control::Model(model) => {
                            let catalog_retention = self.data.try_clone_current()?;
                            self.data.insert(DataOwner::Catalog, catalog_retention);
                            self.available_models = record["result"]["configOptions"]
                                .as_array()
                                .cloned()
                                .unwrap_or_default();
                            let selection_retention =
                                op.retention.try_clone().map_err(retention_error)?;
                            self.native_selection = Some(model.clone());
                            self.data.insert(DataOwner::Selection, selection_retention);
                        }
                    }
                }
            }
            Ok(())
        })();
        self.dsh_port
            .as_mut()
            .unwrap()
            .operations
            .insert(sequence, op);
        install?;
        if prompt {
            self.dsh_mark_cancellations();
            self.dsh_prompt_completed(&record)?;
        }
        // New's identity is now known. Dispose every early record explicitly via the usual
        // exact-session routing (mismatched permission -> cancelled), never guess a parent.
        let early = {
            let port = self.dsh_port.as_mut().unwrap();
            port.early_bytes = 0;
            std::mem::take(&mut port.early)
        };
        for (record, _) in early {
            if healthy {
                self.dsh_receive_record(record)?;
            }
        }
        if let Some(error) = terminal_failure {
            return Err(error);
        }
        Ok(())
    }
    fn dsh_install_catalog(&mut self, result: &Value) -> Result<()> {
        let catalog_retention = self.data.try_clone_current()?;
        let selection_retention = catalog_retention.try_clone().map_err(retention_error)?;
        self.data.insert(DataOwner::Catalog, catalog_retention);
        self.data.insert(DataOwner::Selection, selection_retention);
        self.available_models = result["configOptions"]
            .as_array()
            .cloned()
            .unwrap_or_default();
        self.native_selection = self
            .available_models
            .iter()
            .find(|option| option["id"] == "model")
            .and_then(|option| option["currentValue"].as_str())
            .and_then(|value| serde_json::from_str::<Vec<String>>(value).ok())
            .filter(|pair| pair.len() == 2)
            .map(|pair| RuntimeModel {
                provider_id: Some(pair[0].clone()),
                id: pair[1].clone(),
                name: pair[1].clone(),
            });
        Ok(())
    }
    pub(super) fn dsh_port_shutdown(&mut self) {
        if self.dsh_port.is_none() {
            return;
        }
        let _ = self.dsh_harvest_writes();
        self.dsh_latch_failure("DeepSeek explicitly shut down; not replayed");
        let port = self.dsh_port.as_mut().unwrap();
        for op in port.operations.values_mut() {
            op.write = None;
            op.bytes = 0;
        }
        port.reserved_bytes = 0;
        port.reserved_reply_bytes = 0;
        port.early.clear();
        port.early_bytes = 0;
        port.cancellations.clear();
        port.permission_deadlines.clear();
    }
}

#[cfg(all(test, unix))]
#[path = "deepseek_port_tests.rs"]
mod tests;

fn local_evidence(stage: DshReceiptStage) -> Retained<DshReceiptStage> {
    Retained::from_parts(stage, DataRetention::default())
}
