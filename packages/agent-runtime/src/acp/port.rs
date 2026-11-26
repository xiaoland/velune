//! The connected ACP owner stays on its caller's event loop. Admission performs no IO wait.
use super::*;
use crate::creation::{CreationBudget, CreationOutcome, detail as creation_detail};
use crate::transport::{WriteOutcome, WriteReceipt};
use std::collections::VecDeque;
use std::sync::atomic::{AtomicU64, Ordering};

const OPERATIONS: usize = 32;
const REPLY_RESERVE: usize = 8;
const WRITES: usize = 16;
const MAX_FRAME: usize = 256 * 1024;
static GENERATION: AtomicU64 = AtomicU64::new(0);

/// Session establishment requires an already prepared absolute cwd. This port never probes it.
pub enum AcpCommand {
    Prompt {
        text: String,
        images: Vec<crate::TurnImage>,
        /// Optional hard execution budget, independent of enqueue's dispatch/write deadline.
        run_deadline: Option<Instant>,
    },
    Cancel,
    ReplyPermission {
        interaction_id: String,
        reply: RuntimeInteractionReply,
    },
    ReplyElicitation {
        interaction_id: String,
        reply: RuntimeInteractionReply,
    },
    New {
        cwd: PathBuf,
        runtime_id: String,
    },
    Load {
        native_id: String,
        cwd: PathBuf,
        runtime_id: String,
    },
    Resume {
        native_id: String,
        cwd: PathBuf,
        runtime_id: String,
    },
    Close,
    /// One page, with an opaque agent-owned cursor; no hidden session is created.
    List {
        cwd: Option<PathBuf>,
        cursor: Option<String>,
    },
    Delete {
        native_id: String,
    },
    SetConfig {
        config_id: String,
        value: ConfigValue,
    },
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct AcpTicket {
    /// Connection owner identity, independent of the agent's reusable wire IDs.
    pub session: String,
    pub generation: u64,
    pub sequence: u64,
    /// Actual request/reply ID; null for session/cancel notifications.
    pub wire_id: Value,
}

#[derive(Clone, Debug, PartialEq)]
pub struct AcpNativeError {
    pub code: i32,
    pub message: String,
    pub data: Option<Value>,
}

#[derive(Clone, Debug, PartialEq)]
pub enum AcpReceiptStage {
    Written,
    NotWritten {
        message: String,
    },
    /// Only control requests have a protocol success. Prompt responses are turn outcomes.
    ProtocolSuccess {
        result: Value,
    },
    ProtocolRejected {
        error: AcpNativeError,
    },
    TurnCompleted {
        stop_reason: String,
    },
    /// May follow output/tool execution. Never means known-not-executed.
    TurnError {
        error: AcpNativeError,
    },
    Indeterminate {
        message: String,
    },
}

#[derive(Clone, Debug, PartialEq)]
pub struct AcpReceipt {
    pub ticket: AcpTicket,
    pub stage: AcpReceiptStage,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum AcpAdmissionError {
    NeedsPreparation(String),
    NotDispatched { code: String, message: String },
}
impl From<Error> for AcpAdmissionError {
    fn from(error: Error) -> Self {
        Self::NotDispatched {
            code: error.code().into(),
            message: error.to_string(),
        }
    }
}

#[derive(Clone)]
enum Kind {
    Prompt { run_deadline: Option<Instant> },
    Control(Control),
    Reply(Option<String>),
    Cancel,
}
#[derive(Clone)]
enum Control {
    New {
        cwd: PathBuf,
        runtime_id: String,
    },
    Load {
        native_id: String,
        cwd: PathBuf,
        runtime_id: String,
    },
    Resume {
        native_id: String,
        cwd: PathBuf,
        runtime_id: String,
    },
    Close,
    List,
    Delete(String),
    Config,
}
struct Operation {
    ticket: AcpTicket,
    kind: Kind,
    deadline: Instant,
    write: Option<WriteReceipt>,
    terminal: bool,
    evidence: VecDeque<AcpReceiptStage>,
    creation: Option<CreationBudget>,
    creation_result: Option<CreationOutcome>,
    creation_known: bool,
    creation_sealed: bool,
    response_data: Option<crate::creation::DataRetention>,
}
pub(super) struct Port {
    generation: u64,
    blocked: bool,
    next: u64,
    operations: BTreeMap<u64, Operation>,
    /// Fixed write budgets for cancellation intents, including those not yet enqueued.
    cancellations: BTreeMap<String, Instant>,
}

pub(super) fn generation() -> Result<u64> {
    GENERATION
        .try_update(Ordering::Relaxed, Ordering::Relaxed, |n| n.checked_add(1))
        .map(|n| n + 1)
        .map_err(|_| Error::new("ACP connection generation exhausted"))
}

impl Client {
    /// Budget is armed BEFORE session/new can reach the physical writer. Only
    /// this typed result API reports bounded creation's terminal (no cloned raw
    /// ProtocolSuccess result). Written/NotWritten receipt evidence stays intact.
    pub fn enqueue_creation(
        &mut self,
        cwd: PathBuf,
        runtime_id: String,
        budget: CreationBudget,
        deadline: Instant,
    ) -> std::result::Result<AcpTicket, AcpAdmissionError> {
        self.rpc.arm_creation(&budget)?;
        self._creation_cleanup = Some(self.rpc.creation_cleanup_lease());
        match self.enqueue(AcpCommand::New { cwd, runtime_id }, deadline) {
            Ok(ticket) => {
                self.port
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
    pub(crate) fn expire_creation_test(&mut self, ticket: &AcpTicket) {
        let op = self
            .port
            .as_mut()
            .unwrap()
            .operations
            .get_mut(&ticket.sequence)
            .unwrap();
        assert!(op.creation.is_some() && op.write.is_none() && !op.creation_known);
        op.deadline = Instant::now() - std::time::Duration::from_millis(1);
    }
    pub fn take_creation_result(&mut self, ticket: &AcpTicket) -> Option<CreationOutcome> {
        let op = self.port.as_mut()?.operations.get_mut(&ticket.sequence)?;
        (op.ticket == *ticket)
            .then(|| op.creation_result.take())
            .flatten()
    }
    /// Called after the caller has consumed Created and settled its association.
    /// A late/oversize/unknown outcome must instead retain ownership for cleanup.
    pub fn settle_creation_result(&mut self, ticket: &AcpTicket) -> Result<()> {
        if self.failed || self.port.as_ref().is_none_or(|port| port.blocked) {
            return Err(Error::new("ACP creation requires actual cleanup"));
        }
        let op = self
            .port
            .as_ref()
            .unwrap()
            .operations
            .get(&ticket.sequence)
            .filter(|op| {
                op.ticket == *ticket
                    && op.terminal
                    && op.write.is_none()
                    && op.creation_known
                    && op.creation_result.is_none()
            })
            .ok_or_else(|| Error::new("ACP creation result/write not settled"))?;
        let budget = op
            .creation
            .as_ref()
            .ok_or_else(|| Error::new("ACP creation policy missing"))?
            .clone();
        self.rpc.complete_creation(&budget)?;
        self.port
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
    pub fn complete_creation(&mut self, ticket: &AcpTicket) -> Result<()> {
        let op = self
            .port
            .as_mut()
            .and_then(|port| port.operations.get_mut(&ticket.sequence))
            .filter(|op| op.ticket == *ticket && op.creation_sealed)
            .ok_or_else(|| {
                Error::new("creation result must be sealed before association acknowledgement")
            })?;
        op.creation = None;
        Ok(())
    }
    fn capture_creation_record(
        &mut self,
        record: crate::creation::Retained<Value>,
    ) -> Result<Option<crate::creation::Retained<Value>>> {
        if record["jsonrpc"] != "2.0" {
            return Err(Error::new("creation JSON-RPC version invalid"));
        }
        if record.get("method").is_some() {
            return Ok(Some(record));
        }
        let Some(sequence) = self.port.as_ref().and_then(|port| {
            port.operations.iter().find_map(|(sequence, op)| {
                (op.creation.is_some()
                    && op.ticket.wire_id == record["id"]
                    && op.ticket.generation == self.connection_generation)
                    .then_some(*sequence)
            })
        }) else {
            return Ok(Some(record));
        };
        if self.port.as_ref().unwrap().operations[&sequence].creation_known {
            return Ok(None);
        }
        let budget = self.port.as_ref().unwrap().operations[&sequence]
            .creation
            .as_ref()
            .unwrap()
            .clone();
        let _ = self.settle_port_barrier();
        let (record, ownership) = record.into_parts();
        let mut ownership = Some(ownership);
        let outcome = if let Some(error) = record.get("error") {
            match native_error(error) {
                Ok(_) => CreationOutcome::Rejected {
                    message: creation_detail(error),
                },
                // Malformed wire data is not an authoritative rejection. Keep
                // correlation/credit for cleanup and any later complete ID.
                Err(cause) => CreationOutcome::Unknown {
                    message: creation_detail(cause),
                },
            }
        } else if let Some(id) = record["result"]["sessionId"]
            .as_str()
            .filter(|id| !id.is_empty())
        {
            CreationOutcome::Created(budget.created(id).retain_data(ownership.take().unwrap()))
        } else {
            CreationOutcome::Unknown {
                message: "ACP new response has no complete identity".into(),
            }
        };
        let within = matches!(&outcome, CreationOutcome::Created(id) if id.within_policy());
        let known = !matches!(&outcome, CreationOutcome::Unknown { .. });
        let installed_data = match &outcome {
            CreationOutcome::Created(identity) => Some(identity.projection_retention()),
            _ => None,
        };
        {
            let op = self
                .port
                .as_mut()
                .unwrap()
                .operations
                .get_mut(&sequence)
                .unwrap();
            if op.creation_known {
                return Ok(None);
            }
            op.creation_known = known;
            op.creation_result = Some(outcome);
            op.terminal = true;
        }
        if within && !self.failed && !self.port.as_ref().unwrap().blocked {
            // Preserve Created before catalog/projection validation can fail.
            self._session_data =
                installed_data.map(|data| crate::creation::Retained::from_parts((), data));
            if let Err(error) = self.apply_control(sequence, &record["result"]) {
                self.fail();
                return Err(error);
            }
            for update in self.initial_updates.drain() {
                let data = self
                    ._initial_update_data
                    .pop_front()
                    .expect("initial update ownership");
                self.handle(crate::creation::Retained::from_parts(update, data))?;
            }
        } else {
            self.port.as_mut().unwrap().blocked = true;
            self.fail();
        }
        Ok(None)
    }
    /// Opt in at a connected, idle boundary. Synchronous mutation APIs then reject;
    /// bootstrap/history/reconnection and final shutdown remain explicit IO boundaries.
    pub fn enable_command_port(&mut self) -> std::result::Result<(), AcpAdmissionError> {
        if self.failed || self.port.as_ref().is_some_and(|port| port.blocked) {
            return Err(AcpAdmissionError::NeedsPreparation(
                "ACP connection requires cleanup/reconnection".into(),
            ));
        }
        if self.port.is_some() {
            return Ok(());
        }
        if self.failed
            || self.turn_running()
            || self.rpc.has_pending_writes()
            || !self.permissions.is_empty()
            || !self.elicitations.is_empty()
        {
            return Err(AcpAdmissionError::NeedsPreparation(
                "ACP requires a connected idle boundary".into(),
            ));
        }
        self.rpc.defer_cleanup();
        self.port = Some(Port {
            generation: self.connection_generation,
            blocked: false,
            next: 0,
            operations: BTreeMap::new(),
            cancellations: BTreeMap::new(),
        });
        Ok(())
    }

    /// A persistent memory-only fact. Only explicit shutdown reaps the failed child.
    pub fn needs_cleanup(&self) -> bool {
        self.port
            .as_ref()
            .is_some_and(|port| self.failed || port.blocked)
            && !self.cleaned_up
    }

    /// Prompt's deadline covers dispatch/write only; its optional run_deadline
    /// covers execution. Control request deadlines continue through matching response.
    pub fn enqueue(
        &mut self,
        command: AcpCommand,
        deadline: Instant,
    ) -> std::result::Result<AcpTicket, AcpAdmissionError> {
        if self.port.is_some() {
            self.settle_port_barrier()
                .map_err(AcpAdmissionError::from)?;
        }
        if self.port.as_ref().is_none_or(|port| port.blocked) || self.failed {
            return Err(AcpAdmissionError::NeedsPreparation(
                "ACP command port is not ready".into(),
            ));
        }
        if self
            .port
            .as_ref()
            .unwrap()
            .operations
            .values()
            .any(|op| op.creation.is_some())
            && !matches!(
                command,
                AcpCommand::Cancel
                    | AcpCommand::ReplyPermission { .. }
                    | AcpCommand::ReplyElicitation { .. }
            )
        {
            return Err(AcpAdmissionError::NeedsPreparation(
                "ACP creation awaits association or cleanup".into(),
            ));
        }
        let reply = matches!(
            command,
            AcpCommand::Cancel
                | AcpCommand::ReplyPermission { .. }
                | AcpCommand::ReplyElicitation { .. }
        );
        let parent_deadline = match &command {
            AcpCommand::ReplyPermission { interaction_id, .. } => self
                .permissions
                .get(interaction_id)
                .and_then(|pending| pending.parent_deadline),
            AcpCommand::ReplyElicitation { interaction_id, .. } => self
                .elicitations
                .get(interaction_id)
                .and_then(|pending| pending.parent_deadline),
            _ => self.active_run_deadline(),
        };
        let parent_deadline = match &command {
            AcpCommand::Prompt { run_deadline, .. } => *run_deadline,
            _ => parent_deadline,
        };
        let deadline = if reply || matches!(command, AcpCommand::Prompt { .. }) {
            parent_deadline.map_or(deadline, |parent| parent.min(deadline))
        } else {
            deadline
        };
        if Instant::now() >= deadline {
            return Err(Error::with_code(
                "ACP command deadline expired; not dispatched",
                "timeout",
            )
            .into());
        }
        self.port_capacity(reply)?;
        let sequence = self
            .port
            .as_ref()
            .unwrap()
            .next
            .checked_add(1)
            .ok_or_else(|| Error::new("ACP command sequence exhausted"))?;
        let session_id = || self.session_id().map(str::to_owned);
        let idle = || -> std::result::Result<(), AcpAdmissionError> {
            if self.turn_running()
                || self
                    .port
                    .as_ref()
                    .unwrap()
                    .operations
                    .values()
                    .any(|op| !op.terminal || op.write.is_some())
                || !self.permissions.is_empty()
                || !self.elicitations.is_empty()
            {
                Err(AcpAdmissionError::NeedsPreparation(
                    "ACP owner is not idle".into(),
                ))
            } else {
                Ok(())
            }
        };
        let prompt_projection = match &command {
            AcpCommand::Prompt { text, images, .. } => Some((text.clone(), images.clone())),
            _ => None,
        };
        let (method, params, kind, reply_id) = match command {
            AcpCommand::Prompt {
                text,
                images,
                run_deadline,
            } => {
                idle()?;
                if text.trim().is_empty() && images.is_empty() {
                    return Err(Error::new("消息内容为空").into());
                }
                if !images.is_empty() && !self.prompt_image {
                    return Err(unsupported("图片").into());
                }
                let prompt = (!text.is_empty())
                    .then(|| json!({"type":"text","text":text}))
                    .into_iter()
                    .chain(images.iter().map(|image| {
                        json!({"type":"image",
                        "mimeType":image.kind.media_type(),"data":image.base64()})
                    }))
                    .collect::<Vec<_>>();
                (
                    "session/prompt",
                    json!({"sessionId":session_id()?,"prompt":prompt}),
                    Kind::Prompt { run_deadline },
                    None,
                )
            }
            AcpCommand::Cancel => {
                if !self.turn_running() {
                    return Err(AcpAdmissionError::NeedsPreparation(
                        "No active ACP prompt".into(),
                    ));
                }
                (
                    "session/cancel",
                    json!({"sessionId":session_id()?}),
                    Kind::Cancel,
                    None,
                )
            }
            AcpCommand::ReplyPermission {
                interaction_id,
                reply,
            } => {
                let pending = self
                    .permissions
                    .get(&interaction_id)
                    .ok_or_else(|| Error::new("ACP 权限请求已失效"))?;
                let outcome = match reply {
                    RuntimeInteractionReply::Decision { option_id }
                        if pending.options.contains(&option_id) =>
                    {
                        json!({"outcome":"selected","optionId":option_id})
                    }
                    RuntimeInteractionReply::Cancel => json!({"outcome":"cancelled"}),
                    _ => return Err(Error::new("ACP 权限回复不属于当前公布的选项").into()),
                };
                (
                    "protocol/reply",
                    json!({"outcome":outcome}),
                    Kind::Reply(Some(interaction_id)),
                    Some(pending.rpc_id.clone()),
                )
            }
            AcpCommand::ReplyElicitation {
                interaction_id,
                reply,
            } => {
                let pending = self
                    .elicitations
                    .get(&interaction_id)
                    .ok_or_else(|| Error::new("ACP 提问已失效"))?;
                let result = crate::elicitation::reply(&pending.params, reply)?;
                (
                    "protocol/reply",
                    result,
                    Kind::Reply(Some(interaction_id)),
                    Some(pending.rpc_id.clone()),
                )
            }
            AcpCommand::New { cwd, runtime_id } => {
                idle()?;
                self.port_prepare(&cwd)?;
                (
                    "session/new",
                    json!({"cwd":cwd,"mcpServers":self.mcp_servers}),
                    Kind::Control(Control::New { cwd, runtime_id }),
                    None,
                )
            }
            AcpCommand::Load {
                native_id,
                cwd,
                runtime_id,
            } => {
                idle()?;
                self.port_prepare(&cwd)?;
                if !self.capabilities.load_session {
                    return Err(unsupported("session/load").into());
                }
                if native_id.is_empty() {
                    return Err(Error::new("ACP 会话身份为空").into());
                }
                // Installed only after successful admission, below.
                (
                    "session/load",
                    json!({"sessionId":native_id,"cwd":cwd,"mcpServers":self.mcp_servers}),
                    Kind::Control(Control::Load {
                        native_id,
                        cwd,
                        runtime_id,
                    }),
                    None,
                )
            }
            AcpCommand::Resume {
                native_id,
                cwd,
                runtime_id,
            } => {
                idle()?;
                self.port_prepare(&cwd)?;
                if !self.capabilities.resume_session {
                    return Err(unsupported("session/resume").into());
                }
                if native_id.is_empty() {
                    return Err(Error::new("ACP 会话身份为空").into());
                }
                (
                    "session/resume",
                    json!({"sessionId":native_id,"cwd":cwd,"mcpServers":self.mcp_servers}),
                    Kind::Control(Control::Resume {
                        native_id,
                        cwd,
                        runtime_id,
                    }),
                    None,
                )
            }
            AcpCommand::Close => {
                idle()?;
                if !self.capabilities.close_session {
                    return Err(unsupported("session/close").into());
                }
                (
                    "session/close",
                    json!({"sessionId":session_id()?}),
                    Kind::Control(Control::Close),
                    None,
                )
            }
            AcpCommand::List { cwd, cursor } => {
                idle()?;
                if !self.capabilities.list_sessions {
                    return Err(unsupported("session/list").into());
                }
                if cwd.as_ref().is_some_and(|path| !path.is_absolute()) {
                    return Err(Error::new("ACP 会话筛选目录必须为绝对路径").into());
                }
                let mut params = json!({});
                if let Some(cwd) = cwd {
                    params["cwd"] = json!(cwd);
                }
                if let Some(cursor) = cursor {
                    params["cursor"] = json!(cursor);
                }
                ("session/list", params, Kind::Control(Control::List), None)
            }
            AcpCommand::Delete { native_id } => {
                idle()?;
                if !self.capabilities.delete_session {
                    return Err(unsupported("session/delete").into());
                }
                if native_id.is_empty() {
                    return Err(Error::new("ACP 会话身份为空").into());
                }
                (
                    "session/delete",
                    json!({"sessionId":native_id}),
                    Kind::Control(Control::Delete(native_id)),
                    None,
                )
            }
            AcpCommand::SetConfig { config_id, value } => {
                idle()?;
                let option = self
                    .options
                    .iter()
                    .find(|option| option.id == config_id)
                    .ok_or_else(|| Error::new("ACP 会话未公布此配置项"))?;
                let mut params = json!({"sessionId":session_id()?,"configId":config_id});
                match (&option.value, value) {
                    (ConfigValue::Select(_), ConfigValue::Select(value))
                        if option.choices.iter().any(|choice| choice.value == value) =>
                    {
                        params["value"] = json!(value)
                    }
                    (ConfigValue::Boolean(_), ConfigValue::Boolean(value)) => {
                        params["type"] = json!("boolean");
                        params["value"] = json!(value);
                    }
                    _ => return Err(Error::new("ACP 配置值不属于当前公布的选项或类型").into()),
                }
                (
                    "session/set_config_option",
                    params,
                    Kind::Control(Control::Config),
                    None,
                )
            }
        };
        if let Kind::Reply(Some(id)) = &kind
            && self.reply_pending(id)
        {
            return Err(Error::with_code(
                "ACP interaction reply already queued",
                "already_pending",
            )
            .into());
        }
        check_size(&params)?;
        // Encoding/validation can itself cross an absolute owner deadline.
        self.settle_port_barrier()?;
        // The physical codec/admission uses the earliest applicable owner barrier
        // too, so encoding cannot dispatch across another live operation's budget.
        let dispatch_deadline = self.port_dispatch_deadline(deadline);
        // All validation precedes this single enqueue boundary.
        let dispatch = if let Some(id) = reply_id {
            self.rpc
                .enqueue_reply_receipt(&id, &params, dispatch_deadline)
                .map(|write| (id, write))
        } else if matches!(kind, Kind::Cancel) {
            self.rpc
                .enqueue_notification_receipt_with_class(
                    method,
                    &params,
                    crate::transport::WriteClass::ProtocolReplyCancel,
                    dispatch_deadline,
                )
                .map(|write| (Value::Null, write))
        } else {
            self.rpc
                .enqueue_request_receipt(method, &params, dispatch_deadline)
                .map(|(id, write)| (json!(id), write))
        };
        let (wire_id, write) = match dispatch {
            Ok(dispatched) => dispatched,
            Err(error) => {
                if error.code() == "transport" {
                    self.port.as_mut().unwrap().blocked = true;
                }
                return Err(error.into());
            }
        };
        let establishment = match &kind {
            Kind::Control(
                Control::Load {
                    native_id,
                    cwd,
                    runtime_id,
                }
                | Control::Resume {
                    native_id,
                    cwd,
                    runtime_id,
                },
            ) => Some((native_id.clone(), cwd.clone(), runtime_id.clone())),
            _ => None,
        };
        let ticket = self.install_operation(sequence, wire_id, kind, write, deadline);
        match method {
            "session/prompt" => {
                let (text, images) = prompt_projection.unwrap();
                self.project_prompt(&text, &images, ticket.wire_id.as_u64().unwrap());
            }
            "session/cancel" => {
                self.mark_cancellations();
                self.state(RunState::Stopping, false, true);
            }
            "session/new" => {
                self.establishing = true;
            }
            "session/load" | "session/resume" => {
                let (native_id, cwd, runtime_id) = establishment.unwrap();
                self.replace_snapshot(&native_id, &cwd, &runtime_id);
            }
            _ => {}
        }
        Ok(ticket)
    }

    /// Consuming Written does not free a prompt's operation slot or terminal evidence budget.
    pub fn take_receipts(&mut self) -> Vec<crate::creation::Retained<AcpReceipt>> {
        let Some(port) = &mut self.port else {
            return Vec::new();
        };
        let mut result = Vec::new();
        for op in port.operations.values_mut() {
            for stage in op.evidence.drain(..) {
                let data = if matches!(
                    &stage,
                    AcpReceiptStage::ProtocolSuccess { .. }
                        | AcpReceiptStage::ProtocolRejected { .. }
                        | AcpReceiptStage::TurnCompleted { .. }
                        | AcpReceiptStage::TurnError { .. }
                ) {
                    op.response_data.take().unwrap_or_default()
                } else {
                    crate::creation::DataRetention::default()
                };
                result.push(crate::creation::Retained::from_parts(
                    AcpReceipt {
                        ticket: op.ticket.clone(),
                        stage,
                    },
                    data,
                ));
            }
        }
        port.operations.retain(|_, op| {
            !op.terminal
                || op.write.is_some()
                || op.creation.is_some()
                || (self.failed && !self.cleaned_up)
        });
        result
    }

    /// Includes unconsumed evidence and terminal-turn reply writes, not just the parent prompt.
    pub fn has_pending_operations(&self) -> bool {
        self.port.as_ref().is_some_and(|port| {
            port.operations.values().any(|op| {
                !op.terminal
                    || op.write.is_some()
                    || op.creation.is_some()
                    || !op.evidence.is_empty()
            })
        }) || self.needs_cleanup()
            || !self.permissions.is_empty()
            || !self.elicitations.is_empty()
    }

    /// Session scope carries no control-request correlation. Only the active
    /// prompt's explicit run budget is attributable here; idle forms have no TTL.
    pub(super) fn interaction_parent_deadline(&self) -> Option<Instant> {
        self.port.as_ref().and_then(|_| self.active_run_deadline())
    }

    fn active_run_deadline(&self) -> Option<Instant> {
        let id = self.prompt_id?;
        self.port.as_ref()?.operations.values().find_map(|op| {
            if op.ticket.wire_id.as_u64() == Some(id)
                && !op.terminal
                && let Kind::Prompt { run_deadline } = op.kind
            {
                run_deadline
            } else {
                None
            }
        })
    }

    fn port_prepare(&self, cwd: &Path) -> std::result::Result<(), AcpAdmissionError> {
        if !cwd.is_absolute() || self.native_id.is_some() {
            return Err(AcpAdmissionError::NeedsPreparation(
                "Use a prepared absolute cwd and explicitly close the prior session".into(),
            ));
        }
        Ok(())
    }
    fn port_capacity(&self, reply: bool) -> Result<()> {
        let port = self.port.as_ref().unwrap();
        let limit = if reply {
            OPERATIONS
        } else {
            OPERATIONS - REPLY_RESERVE
        };
        let writes = port
            .operations
            .values()
            .filter(|op| op.write.is_some())
            .count();
        if port.operations.len() >= limit || writes >= if reply { WRITES } else { WRITES - 4 } {
            return Err(Error::with_code(
                "ACP operation/receipt capacity full; not dispatched",
                "queue_full",
            ));
        }
        Ok(())
    }
    fn install_operation(
        &mut self,
        sequence: u64,
        wire_id: Value,
        kind: Kind,
        write: WriteReceipt,
        deadline: Instant,
    ) -> AcpTicket {
        let port = self.port.as_mut().unwrap();
        let ticket = AcpTicket {
            session: self.epoch.clone(),
            generation: port.generation,
            sequence,
            wire_id,
        };
        port.next = sequence;
        port.operations.insert(
            sequence,
            Operation {
                ticket: ticket.clone(),
                kind,
                deadline,
                write: Some(write),
                terminal: false,
                evidence: VecDeque::new(),
                creation: None,
                creation_result: None,
                creation_known: false,
                creation_sealed: false,
                response_data: None,
            },
        );
        ticket
    }
    fn reply_pending(&self, id: &str) -> bool {
        self.port
            .as_ref()
            .unwrap()
            .operations
            .values()
            .any(|op| matches!(&op.kind, Kind::Reply(Some(pending)) if pending == id))
    }

    pub(super) fn port_reply(
        &mut self,
        id: &Value,
        result: &Value,
        rejection: Option<&str>,
    ) -> Result<()> {
        self.settle_port_barrier()?;
        self.port_capacity(true)?;
        check_size(result)?;
        self.settle_port_barrier()?;
        if self.failed || self.port.as_ref().unwrap().blocked {
            return Err(Error::with_code(
                "ACP terminal connection rejects new frames",
                "transport",
            ));
        }
        let port = self.port.as_ref().unwrap();
        let next = port
            .next
            .checked_add(1)
            .ok_or_else(|| Error::new("ACP command sequence exhausted"))?;
        let write_deadline = Instant::now() + CONTROL_TIMEOUT;
        let deadline = self
            .active_run_deadline()
            .map_or(write_deadline, |parent| parent.min(write_deadline));
        let deadline = self.port_dispatch_deadline(deadline);
        let write = if let Some(message) = rejection {
            self.rpc.enqueue_rejection_receipt(id, message, deadline)?
        } else {
            self.rpc.enqueue_reply_receipt(id, result, deadline)?
        };
        self.install_operation(next, id.clone(), Kind::Reply(None), write, deadline);
        Ok(())
    }

    pub(super) fn respond_auto(&mut self, id: &Value, result: &Value) -> Result<()> {
        if self.port.is_some() {
            self.port_reply(id, result, None)
        } else {
            self.rpc.respond(id, result)
        }
    }
    pub(super) fn reject_auto(&mut self, id: &Value, message: &str) -> Result<()> {
        if self.port.is_some() {
            self.port_reply(id, &Value::Null, Some(message))
        } else {
            self.rpc.reject(id, message)
        }
    }
    pub(super) fn interaction_capacity(&self) -> Result<()> {
        if self.permissions.len() + self.elicitations.len() >= 32 {
            return Err(Error::with_code(
                "ACP interaction capacity full",
                "transport",
            ));
        }
        Ok(())
    }
    fn mark_cancellation(&mut self, id: &str) {
        let parent = self
            .permissions
            .get(id)
            .and_then(|pending| pending.parent_deadline)
            .or_else(|| {
                self.elicitations
                    .get(id)
                    .and_then(|pending| pending.parent_deadline)
            });
        let write_deadline = Instant::now() + CONTROL_TIMEOUT;
        let deadline = parent.map_or(write_deadline, |parent| parent.min(write_deadline));
        self.port
            .as_mut()
            .unwrap()
            .cancellations
            .entry(id.into())
            .or_insert(deadline);
    }
    fn mark_cancellations(&mut self) {
        let ids: Vec<_> = self
            .permissions
            .keys()
            .chain(self.elicitations.keys())
            .cloned()
            .collect();
        for id in ids {
            self.mark_cancellation(&id);
        }
    }
    pub(super) fn cancel_one_interaction(&mut self, id: &str) -> Result<()> {
        self.mark_cancellation(id);
        self.schedule_cancellations()
    }
    pub(super) fn settle_port_shutdown(&mut self) {
        if self.port.is_none() {
            return;
        }
        let _ = self.harvest_port_writes();
        for op in self.port.as_mut().unwrap().operations.values_mut() {
            if op.creation.is_some() && !op.creation_known && op.creation_result.is_none() {
                op.creation_result = Some(CreationOutcome::Unknown {
                    message: "ACP creation owner explicitly shut down".into(),
                });
            }
            if !op.terminal {
                op.evidence.push_back(AcpReceiptStage::Indeterminate {
                    message: "ACP explicitly shut down; not replayed".into(),
                });
                op.terminal = true;
            }
            // Called only after the explicit process shutdown/finish boundary.
            op.write = None;
        }
    }

    pub(super) fn port_cancel_interactions(&mut self) -> Result<()> {
        self.mark_cancellations();
        self.schedule_cancellations()
    }
    fn schedule_cancellations(&mut self) -> Result<()> {
        let intents: Vec<_> = self
            .port
            .as_ref()
            .unwrap()
            .cancellations
            .iter()
            .map(|(id, deadline)| (id.clone(), *deadline))
            .collect();
        for (id, deadline) in intents {
            if self.reply_pending(&id) {
                continue;
            }
            let command = if self.permissions.contains_key(&id) {
                AcpCommand::ReplyPermission {
                    interaction_id: id,
                    reply: RuntimeInteractionReply::Cancel,
                }
            } else {
                AcpCommand::ReplyElicitation {
                    interaction_id: id,
                    reply: RuntimeInteractionReply::Cancel,
                }
            };
            match self.enqueue(command, deadline) {
                Ok(_) => {}
                Err(AcpAdmissionError::NotDispatched { code, .. }) if code == "queue_full" => break,
                Err(error) => {
                    return Err(Error::new(format!(
                        "ACP cancellation not dispatched: {error:?}"
                    )));
                }
            }
        }
        Ok(())
    }

    pub(super) fn poll_port(&mut self) -> Result<()> {
        if self.failed {
            // A timed-out writer may eventually finish. Preserve late flush
            // evidence without dispatching anything or releasing the child.
            let _ = self.harvest_port_writes();
            if self
                .port
                .as_ref()
                .is_some_and(|port| port.operations.values().any(|op| op.creation.is_some()))
                && let Ok(records) = self.rpc.poll()
            {
                for record in records {
                    let _ = self.capture_creation_record(record);
                }
            }
            return Ok(());
        }
        let outcome = self.poll_port_inner();
        if let Err(error) = &outcome {
            let port = self.port.as_mut().unwrap();
            for op in port.operations.values_mut() {
                if op.creation.is_some() && !op.creation_known && op.creation_result.is_none() {
                    op.creation_result = Some(CreationOutcome::Unknown {
                        message: creation_detail(error),
                    });
                }
                if !op.terminal {
                    op.evidence.push_back(AcpReceiptStage::Indeterminate {
                        message: error.to_string(),
                    });
                    op.terminal = true;
                }
            }
            self.fail();
        }
        outcome
    }
    fn poll_port_inner(&mut self) -> Result<()> {
        // Physical write timestamps settle only dispatch budgets. Native run/control
        // deadlines still use owner observation, before each terminal can erase them.
        let mut deadline_error = self.settle_port_barrier().err();
        let write_error = self.harvest_port_writes().err();
        let records = match self.rpc.poll() {
            Ok(records) => records,
            Err(error) => {
                if deadline_error.is_none() {
                    deadline_error = self.check_port_deadlines().err();
                }
                let _ = self.harvest_port_writes();
                return Err(deadline_error.unwrap_or(error));
            }
        };
        if write_error.is_some() {
            self.failed = true;
        }
        let mut handler_error = None;
        for record in records {
            if let Err(error) = self.settle_port_barrier() {
                deadline_error.get_or_insert(error);
            }
            if let Err(error) = self.port_record(record) {
                handler_error.get_or_insert(error);
                self.failed = true;
            }
        }
        if deadline_error.is_none() {
            deadline_error = self.check_port_deadlines().err();
        }
        let later_write_error = self.harvest_port_writes().err();
        // Cross the transport barrier even when expiry or a handler has failed;
        // preserve valid facts, but never let either error swallow that barrier.
        let transport_error = self.rpc.check_failure().err();
        if let Some(error) = deadline_error
            .or(handler_error)
            .or(write_error)
            .or(later_write_error)
            .or(transport_error)
        {
            return Err(error);
        }
        self.check_port_deadlines()?;
        self.schedule_cancellations()?;
        Ok(())
    }

    fn check_port_deadlines(&mut self) -> Result<()> {
        let now = Instant::now();
        let expired = self.port.as_ref().unwrap().operations.values().any(|op| {
            (op.write.is_some() && now >= op.deadline)
                || (!op.terminal
                    && match op.kind {
                        Kind::Prompt { run_deadline } => {
                            run_deadline.is_some_and(|deadline| now >= deadline)
                        }
                        Kind::Control(_) => now >= op.deadline,
                        Kind::Reply(_) | Kind::Cancel => false,
                    })
        }) || self
            .permissions
            .values()
            .filter_map(|pending| pending.parent_deadline)
            .chain(
                self.elicitations
                    .values()
                    .filter_map(|pending| pending.parent_deadline),
            )
            .chain(self.port.as_ref().unwrap().cancellations.values().copied())
            .any(|deadline| now >= deadline);
        if expired {
            // This is a dispatch barrier, not process cleanup or a claim that
            // native execution did not occur. Keep parsing this batch's facts.
            self.port.as_mut().unwrap().blocked = true;
            return Err(Error::with_code(
                "ACP operation deadline; not automatically replayed",
                "timeout",
            ));
        }
        Ok(())
    }
    fn port_dispatch_deadline(&self, requested: Instant) -> Instant {
        let port = self.port.as_ref().unwrap();
        let operation_deadlines = port.operations.values().filter_map(|op| {
            if op.write.is_some() {
                Some(op.deadline)
            } else if !op.terminal {
                match op.kind {
                    Kind::Prompt { run_deadline } => run_deadline,
                    Kind::Control(_) => Some(op.deadline),
                    _ => None,
                }
            } else {
                None
            }
        });
        operation_deadlines
            .chain(self.permissions.values().filter_map(|p| p.parent_deadline))
            .chain(self.elicitations.values().filter_map(|p| p.parent_deadline))
            .chain(port.cancellations.values().copied())
            .fold(requested, std::cmp::min)
    }

    fn settle_port_barrier(&mut self) -> Result<()> {
        let failure = self.harvest_port_writes().err();
        let deadline = self.check_port_deadlines().err();
        if let Some(error) = failure.or(deadline) {
            self.port.as_mut().unwrap().blocked = true;
            return Err(error);
        }
        Ok(())
    }
    fn harvest_port_writes(&mut self) -> Result<()> {
        let mut settled = Vec::new();
        let mut failure = None;
        for op in self.port.as_mut().unwrap().operations.values_mut() {
            let Some(outcome) = op.write.as_ref().and_then(WriteReceipt::try_outcome) else {
                continue;
            };
            let terminal = !matches!(outcome, WriteOutcome::ObservationTimeout { .. });
            match outcome {
                WriteOutcome::Written(completion) => {
                    op.evidence.push_front(AcpReceiptStage::Written);
                    if !completion.met_deadline() {
                        failure.get_or_insert_with(|| {
                            Error::with_code(
                                "ACP physically Written after deadline; cleanup required",
                                "timeout",
                            )
                        });
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
                        op.creation_result = Some(CreationOutcome::NotWritten {
                            message: creation_detail(&error),
                        });
                        op.creation_known = true;
                    }
                    op.evidence.push_back(AcpReceiptStage::NotWritten {
                        message: error.to_string(),
                    });
                    op.terminal = true;
                    failure.get_or_insert_with(|| {
                        Error::with_code(format!("ACP known not written: {error}"), "transport")
                    });
                }
                WriteOutcome::Indeterminate(error) => {
                    failure.get_or_insert_with(|| {
                        Error::with_code(format!("ACP write indeterminate: {error}"), "transport")
                    });
                }
                WriteOutcome::ObservationTimeout { .. } => {
                    failure.get_or_insert_with(|| {
                        Error::with_code("ACP write not yet observed; receipt retained", "timeout")
                    });
                }
            }
            if terminal {
                op.write = None;
            }
        }
        for id in settled {
            self.permissions.remove(&id);
            self.elicitations.remove(&id);
            self.remove_interaction(&id);
            self.port.as_mut().unwrap().cancellations.remove(&id);
        }
        if let Some(error) = failure {
            return Err(error);
        }
        Ok(())
    }

    fn port_record(&mut self, record: crate::creation::Retained<Value>) -> Result<()> {
        if record["jsonrpc"] != "2.0" {
            return Err(Error::new("ACP 返回的 JSON-RPC 版本无效"));
        }
        let Some(record) = self.capture_creation_record(record)? else {
            return Ok(());
        };
        if record.get("method").is_some() {
            return self.handle(record);
        }
        let (record, data) = record.into_parts();
        let (native_data, receipt_data) = data.split();
        let mut native_data = Some(native_data);
        let sequence = self
            .port
            .as_ref()
            .unwrap()
            .operations
            .iter()
            .find_map(|(sequence, op)| {
                (!op.terminal
                    && op.ticket.generation == self.connection_generation
                    && op.ticket.wire_id == record["id"]
                    && matches!(op.kind, Kind::Prompt { .. } | Kind::Control(_)))
                .then_some(*sequence)
            });
        let Some(sequence) = sequence else {
            return Ok(());
        };
        let native_error = record.get("error").map(native_error).transpose()?;
        let prompt = matches!(
            self.port.as_ref().unwrap().operations[&sequence].kind,
            Kind::Prompt { .. }
        );
        let stage = if let Some(error) = native_error {
            if prompt {
                AcpReceiptStage::TurnError { error }
            } else {
                AcpReceiptStage::ProtocolRejected { error }
            }
        } else {
            let result = record
                .get("result")
                .filter(|value| value.is_object())
                .ok_or_else(|| Error::new("ACP 响应缺少对象结果"))?;
            if prompt {
                let response: wire::PromptResponse = decode(result)?;
                AcpReceiptStage::TurnCompleted {
                    stop_reason: serde_json::to_value(response.stop_reason)
                        .map_err(|error| Error::new(error.to_string()))?
                        .as_str()
                        .ok_or_else(|| Error::new("ACP invalid stop reason"))?
                        .into(),
                }
            } else {
                let _ = self.settle_port_barrier();
                if !self.failed && !self.port.as_ref().unwrap().blocked {
                    let outcome = self.apply_control(sequence, result);
                    match &self.port.as_ref().unwrap().operations[&sequence].kind {
                        Kind::Control(
                            Control::New { .. } | Control::Load { .. } | Control::Resume { .. },
                        ) => {
                            self._session_data = native_data
                                .take()
                                .map(|data| crate::creation::Retained::from_parts((), data));
                        }
                        Kind::Control(Control::Config) => {
                            self._options_data = native_data
                                .take()
                                .map(|data| crate::creation::Retained::from_parts((), data));
                        }
                        _ => {}
                    }
                    if let Err(error) = outcome
                        && !self.port.as_ref().unwrap().blocked
                    {
                        return Err(error);
                    }
                }
                AcpReceiptStage::ProtocolSuccess {
                    result: result.clone(),
                }
            }
        };
        let _ = self.settle_port_barrier();
        let healthy = !self.failed && !self.port.as_ref().unwrap().blocked;
        let rejected = matches!(stage, AcpReceiptStage::ProtocolRejected { .. });
        let op = self
            .port
            .as_mut()
            .unwrap()
            .operations
            .get_mut(&sequence)
            .unwrap();
        op.response_data = Some(receipt_data);
        op.evidence.push_back(stage);
        op.terminal = true;
        if prompt && healthy {
            self.handle(crate::creation::Retained::from_parts(
                record,
                native_data.take().unwrap(),
            ))?;
        }
        if healthy && !rejected && !prompt && !self.establishing {
            for record in self.initial_updates.drain() {
                let data = self
                    ._initial_update_data
                    .pop_front()
                    .expect("initial update ownership");
                self.handle(crate::creation::Retained::from_parts(record, data))?;
            }
        }
        if rejected && healthy {
            let provisional = matches!(
                self.port.as_ref().unwrap().operations[&sequence].kind,
                Kind::Control(Control::Load { .. } | Control::Resume { .. })
            );
            if provisional {
                // With unanswered peer requests the connection cannot safely be
                // reused: keep their leases and require explicit cleanup instead.
                if !self.permissions.is_empty()
                    || !self.elicitations.is_empty()
                    || self
                        .port
                        .as_ref()
                        .unwrap()
                        .operations
                        .values()
                        .any(|op| matches!(op.kind, Kind::Reply(_)) && op.write.is_some())
                {
                    return Err(Error::with_code(
                        "ACP rejected open with outstanding interactions; cleanup required",
                        "transport",
                    ));
                }
                self.clear_session();
            }
            if self.establishing {
                self.establishing = false;
                self.initial_updates.drain();
            }
        }
        Ok(())
    }
    fn apply_control(&mut self, sequence: u64, result: &Value) -> Result<()> {
        let kind = self.port.as_ref().unwrap().operations[&sequence]
            .kind
            .clone();
        match &kind {
            Kind::Control(Control::New { cwd, runtime_id }) => {
                let response: wire::NewSessionResponse = decode(result)?;
                let id = response.session_id.to_string();
                if id.is_empty() {
                    return Err(Error::new("ACP 会话身份为空"));
                }
                let options = config::project(response.config_options.unwrap_or_default())?;
                let (cwd, runtime_id) = (cwd.clone(), runtime_id.clone());
                self.settle_port_barrier()?;
                self.replace_snapshot(&id, &cwd, &runtime_id);
                self.options = options;
                self.establishing = false;
            }
            Kind::Control(Control::Load { .. }) => {
                let response: wire::LoadSessionResponse = decode(result)?;
                let options = config::project(response.config_options.unwrap_or_default())?;
                self.settle_port_barrier()?;
                self.options = options;
                self.complete_messages();
            }
            Kind::Control(Control::Resume { .. }) => {
                let response: wire::ResumeSessionResponse = decode(result)?;
                let options = config::project(response.config_options.unwrap_or_default())?;
                self.settle_port_barrier()?;
                self.options = options;
                self.notice("Agent 已恢复原生上下文；session/resume 不回放会话历史。");
            }
            Kind::Control(Control::Close) => {
                let _: wire::CloseSessionResponse = decode(result)?;
                self.settle_port_barrier()?;
                self.clear_session();
            }
            Kind::Control(Control::Delete(id)) => {
                let _: wire::DeleteSessionResponse = decode(result)?;
                self.settle_port_barrier()?;
                if self.native_id.as_ref() == Some(id) {
                    self.clear_session();
                }
            }
            Kind::Control(Control::List) => {
                let _: wire::ListSessionsResponse = decode(result)?;
            }
            Kind::Control(Control::Config) => {
                let response: wire::SetSessionConfigOptionResponse = decode(result)?;
                let options = config::project(response.config_options)?;
                self.settle_port_barrier()?;
                self.options = options;
            }
            _ => unreachable!(),
        }
        Ok(())
    }
}

fn check_size(params: &Value) -> Result<()> {
    if serde_json::to_vec(params)
        .map_err(|error| Error::new(error.to_string()))?
        .len()
        > MAX_FRAME
    {
        return Err(Error::with_code(
            "ACP frame exceeds bounded admission budget; not dispatched",
            "queue_full",
        ));
    }
    Ok(())
}
fn decode<T: serde::de::DeserializeOwned>(value: &Value) -> Result<T> {
    serde_json::from_value(value.clone())
        .map_err(|error| Error::new(format!("ACP response invalid: {error}")))
}
fn native_error(value: &Value) -> Result<AcpNativeError> {
    let _: wire::Error = decode(value)?;
    Ok(AcpNativeError {
        code: value["code"]
            .as_i64()
            .and_then(|n| i32::try_from(n).ok())
            .ok_or_else(|| Error::new("ACP error code invalid"))?,
        message: value["message"]
            .as_str()
            .ok_or_else(|| Error::new("ACP error message invalid"))?
            .into(),
        data: value.get("data").cloned(),
    })
}

#[cfg(all(test, unix))]
mod integration_tests;
