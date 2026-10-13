//! Agent-to-agent collaboration between ordinary conversations.
//!
//! One loopback MCP service serves every conversation Velune executes; each
//! session's ticket identifies the calling conversation. Tool calls never
//! touch the application from the service thread: they wait in the inbox
//! until the next request advances the sessions, and run then on the same
//! sessions clients address. Conversations an agent starts or messages are
//! ordinary conversations; Velune keeps no parent/child relation between them.
use super::*;
use crate::conversation::{MessageBlock, MessageRole};
use serde::Deserialize;
use std::collections::BTreeSet;
use std::sync::{Mutex, mpsc};
use std::time::{Duration, Instant};
mod mcp;

/// Longest single wait an agent may request.
const MAX_WAIT: Duration = Duration::from_secs(30);
/// Reply text returned to the calling agent at most.
const MAX_REPLY_CHARS: usize = 32 * 1024;
const MAX_MESSAGE_BYTES: usize = 128 * 1024;

pub(super) type Reply = mpsc::Sender<Result<Value, RuntimeError>>;

/// Requests waiting for the application, and the tickets it has issued.
#[derive(Default)]
pub(super) struct Inbox {
    envelopes: Mutex<Vec<Envelope>>,
    tickets: Mutex<BTreeSet<String>>,
}

impl Inbox {
    fn post(&self, envelope: Envelope) -> Result<(), RuntimeError> {
        self.envelopes
            .lock()
            .map_err(|_| RuntimeError::invalid("协作收件箱不可用"))?
            .push(envelope);
        Ok(())
    }
    fn take(&self) -> Vec<Envelope> {
        self.envelopes
            .lock()
            .map(|mut envelopes| std::mem::take(&mut *envelopes))
            .unwrap_or_default()
    }
    fn admits(&self, token: &str) -> bool {
        self.tickets
            .lock()
            .is_ok_and(|tickets| tickets.contains(token))
    }
}

pub(super) struct Envelope {
    ticket: String,
    operation: Operation,
    reply: Reply,
}

/// A session's access to the collaboration service; revoked when dropped.
pub(super) struct Ticket {
    token: String,
    inbox: Arc<Inbox>,
}

impl Ticket {
    fn issue(inbox: &Arc<Inbox>) -> Result<Self, RuntimeError> {
        let mut random = [0; 32];
        getrandom::fill(&mut random).map_err(|_| RuntimeError::invalid("无法生成协作票据"))?;
        let token: String = random.iter().map(|byte| format!("{byte:02x}")).collect();
        inbox
            .tickets
            .lock()
            .map_err(|_| RuntimeError::invalid("协作收件箱不可用"))?
            .insert(token.clone());
        Ok(Self {
            token,
            inbox: inbox.clone(),
        })
    }
}

impl Drop for Ticket {
    fn drop(&mut self) {
        if let Ok(mut tickets) = self.inbox.tickets.lock() {
            tickets.remove(&self.token);
        }
    }
}

struct Waiter {
    ticket: String,
    conversation_id: String,
    deadline: Instant,
    reply: Reply,
}

#[derive(Default)]
pub(super) struct Broker {
    inbox: Arc<Inbox>,
    server: Option<mcp::Server>,
    waiters: Vec<Waiter>,
    /// The conversation state each caller last read, so a wait returns news
    /// rather than the same settled state again.
    observed: BTreeMap<(String, String), String>,
}

impl Broker {
    /// Stop serving; requests still waiting learn that Velune closed.
    pub(super) fn shutdown(&mut self) {
        self.server = None;
        self.waiters.clear();
        self.inbox.take();
    }
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
pub(super) struct ListConversations {
    cwd: Option<String>,
    query: Option<String>,
    limit: Option<usize>,
}
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
pub(super) struct StartConversation {
    runtime_id: String,
    message: String,
    cwd: Option<String>,
    model: Option<String>,
}
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
pub(super) struct SendMessage {
    conversation_id: String,
    message: String,
}
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
pub(super) struct ConversationRef {
    conversation_id: String,
}
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
pub(super) struct WaitConversation {
    conversation_id: String,
    timeout_ms: u64,
}

pub(super) enum Operation {
    ListAgents,
    ListConversations(ListConversations),
    Start(StartConversation),
    Send(SendMessage),
    Get(ConversationRef),
    Wait(WaitConversation),
    Cancel(ConversationRef),
}

/// The calling conversation, read from the session its ticket belongs to.
struct Caller {
    ticket: String,
    conversation_id: String,
    title: String,
    runtime_id: String,
    cwd: Option<String>,
}

enum Served {
    Now(Value),
    Later(String, Instant),
}

fn validate_message(message: &str) -> Result<(), RuntimeError> {
    if message.trim().is_empty() || message.len() > MAX_MESSAGE_BYTES {
        return Err(RuntimeError::invalid("协作消息必须非空且不超过 128 KiB"));
    }
    Ok(())
}

/// The text a receiving conversation gets: the message, then who sent it
/// and that it grants nothing a human has not.
fn envelope_text(caller: &Caller, runtime_name: &str, message: &str) -> String {
    velune_conversation::agent_message::compose(
        message,
        &velune_conversation::agent_message::Sender {
            runtime_name: runtime_name.into(),
            title: caller.title.clone(),
            conversation_id: caller.conversation_id.clone(),
        },
    )
}

/// Same execution machine: a working directory on one is meaningful on the other.
fn same_machine(left: &RuntimeInstance, right: &RuntimeInstance) -> bool {
    use crate::RuntimeTransport::{Local, Ssh};
    match (left.execution(), right.execution()) {
        (Ok(left), Ok(right)) => match (left.transport, right.transport) {
            (Local, Local) => true,
            (
                Ssh {
                    destination: left,
                    port: left_port,
                    ..
                },
                Ssh {
                    destination: right,
                    port: right_port,
                    ..
                },
            ) => left == right && left_port == right_port,
            _ => false,
        },
        _ => false,
    }
}

fn location(runtime: &RuntimeInstance) -> String {
    match runtime.execution().map(|execution| execution.transport) {
        Ok(crate::RuntimeTransport::Ssh { destination, .. }) => format!("ssh:{destination}"),
        _ => "local".into(),
    }
}

/// The latest reply after the last message sent to the conversation.
fn latest_reply(view: &crate::conversation::ConversationSnapshot) -> (Option<String>, bool) {
    let text = view
        .messages
        .iter()
        .rev()
        .take_while(|message| message.role != MessageRole::User)
        .filter(|message| message.role == MessageRole::Assistant)
        .collect::<Vec<_>>()
        .into_iter()
        .rev()
        .flat_map(|message| &message.blocks)
        .filter_map(|block| match block {
            MessageBlock::Text { text } => Some(text.as_str()),
            _ => None,
        })
        .collect::<Vec<_>>()
        .join("\n");
    if text.is_empty() {
        return (None, false);
    }
    match text.char_indices().nth(MAX_REPLY_CHARS) {
        Some((end, _)) => (Some(text[..end].into()), true),
        None => (Some(text), false),
    }
}

impl CoreRuntime {
    /// The collaboration service for the addressed session's next execution.
    /// Collaboration is best-effort: when the service cannot start, the
    /// conversation runs without it.
    pub(super) fn collaboration_servers(&mut self) -> Vec<velune_agent_runtime::mcp::Server> {
        if self.collaboration.server.is_none() {
            match mcp::Server::start(self.collaboration.inbox.clone()) {
                Ok(server) => self.collaboration.server = Some(server),
                Err(error) => {
                    tracing::warn!(target: "velune_application", event = "collaboration_service_failed", detail = %error);
                    return Vec::new();
                }
            }
        }
        if self.session.collaboration_ticket.is_none() {
            match Ticket::issue(&self.collaboration.inbox) {
                Ok(ticket) => self.session.collaboration_ticket = Some(ticket),
                Err(error) => {
                    tracing::warn!(target: "velune_application", event = "collaboration_ticket_failed", detail = %error);
                    return Vec::new();
                }
            }
        }
        let server = self.collaboration.server.as_ref().expect("started");
        let ticket = self.session.collaboration_ticket.as_ref().expect("issued");
        vec![velune_agent_runtime::mcp::Server {
            name: "velune".into(),
            endpoint: server.endpoint().into(),
            token: ticket.token.clone(),
        }]
    }

    /// Run the tool calls that arrived since the last request and answer the
    /// waits that have news or have run out of time. The addressed session is
    /// addressed again afterwards.
    pub(super) fn serve_collaboration(&mut self) {
        let envelopes = self.collaboration.inbox.take();
        if envelopes.is_empty() && self.collaboration.waiters.is_empty() {
            return;
        }
        let home = self.session.key;
        for Envelope {
            ticket,
            operation,
            reply,
        } in envelopes
        {
            match self.collaborate(&ticket, operation) {
                Ok(Served::Now(value)) => {
                    let _ = reply.send(Ok(value));
                }
                Ok(Served::Later(conversation_id, deadline)) => {
                    self.collaboration.waiters.push(Waiter {
                        ticket,
                        conversation_id,
                        deadline,
                        reply,
                    });
                }
                Err(error) => {
                    let _ = reply.send(Err(error));
                }
            }
        }
        let now = Instant::now();
        for waiter in std::mem::take(&mut self.collaboration.waiters) {
            let expired = now >= waiter.deadline;
            match self.settled_status(&waiter.ticket, &waiter.conversation_id, expired) {
                Ok(Some(status)) => {
                    let _ = waiter.reply.send(Ok(status));
                }
                Ok(None) => self.collaboration.waiters.push(waiter),
                Err(error) => {
                    let _ = waiter.reply.send(Err(error));
                }
            }
        }
        self.focus_key(home);
    }

    fn caller(&self, ticket: &str) -> Result<Caller, RuntimeError> {
        let session = std::iter::once(&self.session)
            .chain(&self.parked)
            .find(|session| {
                session
                    .collaboration_ticket
                    .as_ref()
                    .is_some_and(|issued| issued.token == ticket)
            })
            .ok_or_else(|| RuntimeError::invalid("调用方会话已关闭"))?;
        let view = session
            .projected()
            .ok_or_else(|| RuntimeError::invalid("调用方会话尚未载入"))?;
        Ok(Caller {
            ticket: ticket.into(),
            conversation_id: session.conversation_id().unwrap_or_default().into(),
            title: view.conversation.title.display_text().into(),
            runtime_id: view.context_runtime_id.clone(),
            cwd: view.conversation.cwd.clone(),
        })
    }

    fn collaborate(&mut self, ticket: &str, operation: Operation) -> Result<Served, RuntimeError> {
        let caller = self.caller(ticket)?;
        match operation {
            Operation::ListAgents => Ok(Served::Now(self.collaboration_agents(&caller))),
            Operation::ListConversations(request) => Ok(Served::Now(
                self.collaboration_conversations(&caller, request)?,
            )),
            Operation::Start(request) => {
                self.start_collaboration(&caller, request).map(Served::Now)
            }
            Operation::Send(request) => {
                validate_message(&request.message)?;
                self.address_other(&caller, &request.conversation_id)?;
                self.deliver(&caller, &request.message, None)?;
                self.observed_status(&caller.ticket).map(Served::Now)
            }
            Operation::Get(request) => {
                self.address(&request.conversation_id)?;
                self.observed_status(&caller.ticket).map(Served::Now)
            }
            Operation::Wait(request) => {
                let timeout = Duration::from_millis(request.timeout_ms);
                if timeout > MAX_WAIT {
                    return Err(RuntimeError::invalid("timeout_ms 不能超过 30000"));
                }
                self.address_other(&caller, &request.conversation_id)?;
                let id = self.addressed_id();
                match self.settled_status(&caller.ticket, &id, timeout.is_zero())? {
                    Some(status) => Ok(Served::Now(status)),
                    None => Ok(Served::Later(id, Instant::now() + timeout)),
                }
            }
            Operation::Cancel(request) => {
                self.address_other(&caller, &request.conversation_id)?;
                if self.session.busy() {
                    let runtime_id = self.addressed_runtime()?;
                    let id = self.addressed_id();
                    self.dispatch(&json!({"version":CONTRACT_VERSION,"action":"cancel","runtimeInstanceID":runtime_id,
                        "payload":{"runtimeInstanceID":runtime_id,"conversationID":id}}))?;
                }
                self.observed_status(&caller.ticket).map(Served::Now)
            }
        }
    }

    fn collaboration_agents(&self, caller: &Caller) -> Value {
        let types = runtime_types();
        let here = self.runtime_instance(&caller.runtime_id).ok();
        let agents: Vec<Value> = self
            .runtime_instances
            .iter()
            .filter(|runtime| runtime.enabled)
            .map(|runtime| {
                json!({
                    "runtime_id": runtime.id,
                    "name": runtime.name,
                    "type": types.iter().find(|t| t.id == runtime.type_id).map_or(runtime.type_id.as_str(), |t| t.name.as_str()),
                    "location": location(runtime),
                    "same_machine_as_you": here.is_some_and(|here| same_machine(here, runtime)),
                })
            })
            .collect();
        json!({ "agents": agents })
    }

    fn collaboration_conversations(
        &mut self,
        caller: &Caller,
        request: ListConversations,
    ) -> Result<Value, RuntimeError> {
        let limit = request.limit.unwrap_or(20).clamp(1, 100);
        let query = request.query.map(|query| query.to_lowercase());
        let listed: Vec<ConversationSummary> =
            serde_json::from_value(self.list()?["conversations"].take())?;
        let active: BTreeMap<String, crate::ActiveConversation> = self
            .active_conversations()
            .into_iter()
            .map(|item| (item.conversation_id.clone(), item))
            .collect();
        let names: BTreeMap<&str, &str> = self
            .runtime_instances
            .iter()
            .map(|runtime| (runtime.id.as_str(), runtime.name.as_str()))
            .collect();
        let conversations: Vec<Value> = listed
            .iter()
            .filter(|item| item.id != caller.conversation_id)
            // A native sub-agent belongs to its parent's agent.
            .filter(|item| item.parent_conversation_id.is_none())
            .filter(|item| request.cwd.is_none() || item.cwd == request.cwd)
            .filter(|item| {
                query
                    .as_ref()
                    .is_none_or(|query| item.title.display_text().to_lowercase().contains(query))
            })
            .take(limit)
            .map(|item| {
                let state = match active.get(&item.id) {
                    Some(active) if active.pending_interactions > 0 => "waiting_for_approval",
                    Some(active) if active.run_state == RunState::Elsewhere => "running_elsewhere",
                    Some(_) => "running",
                    None => "idle",
                };
                json!({
                    "conversation_id": item.id,
                    "runtime_id": item.runtime_id,
                    "runtime_name": names.get(item.runtime_id.as_str()),
                    "title": item.title.display_text(),
                    "cwd": item.cwd,
                    "updated_at_unix_ms": item.updated_at_unix_ms,
                    "state": state,
                })
            })
            .collect();
        Ok(json!({ "conversations": conversations }))
    }

    fn start_collaboration(
        &mut self,
        caller: &Caller,
        request: StartConversation,
    ) -> Result<Value, RuntimeError> {
        validate_message(&request.message)?;
        let runtime = self.runtime_instance(&request.runtime_id)?.clone();
        if !runtime.enabled {
            return Err(RuntimeError::invalid("此运行时已停用"));
        }
        let cwd = match request.cwd {
            Some(cwd) => Some(cwd),
            None if self
                .runtime_instance(&caller.runtime_id)
                .is_ok_and(|here| same_machine(here, &runtime)) =>
            {
                caller.cwd.clone()
            }
            None => None,
        };
        self.focus_new();
        let selection = self.collaboration_selection(&runtime.id, request.model.as_deref())?;
        self.dispatch(
            &json!({"version":CONTRACT_VERSION,"action":"create","runtimeInstanceID":runtime.id,
            "payload":{"runtimeInstanceID":runtime.id,"cwd":cwd,"modelSelection":selection}}),
        )?;
        self.deliver(caller, &request.message, Some(selection))?;
        self.observed_status(&caller.ticket)
    }

    /// The model a turn an agent sends uses: the one named, else the
    /// addressed conversation's own, else the runtime's first offered model.
    fn collaboration_selection(
        &mut self,
        runtime_id: &str,
        named: Option<&str>,
    ) -> Result<crate::ModelSelection, RuntimeError> {
        if named.is_none()
            && let Some(selection) = self.session.model_selection.clone().or_else(|| {
                self.session
                    .model_record_key
                    .clone()
                    .map(|key| crate::ModelSelection::Managed {
                        model_record_key: key,
                    })
            })
        {
            return Ok(selection);
        }
        let models: Vec<crate::RuntimeModel> = serde_json::from_value(
            self.runtime_models_action(&json!({"payload":{"runtimeInstanceID":runtime_id}}))?,
        )?;
        let chosen = match named {
            None => models.into_iter().next(),
            Some(named) => models.into_iter().find(|model| {
                model.name == named
                    || match &model.selection {
                        crate::ModelSelection::Native { model_id, .. } => model_id == named,
                        crate::ModelSelection::Managed { model_record_key } => {
                            model_record_key == named
                        }
                        crate::ModelSelection::RuntimeDefault => false,
                    }
            }),
        };
        chosen
            .map(|model| model.selection)
            .ok_or_else(|| RuntimeError::invalid("此运行时没有该模型；不指定 model 即使用默认模型"))
    }

    /// Send `message` from `caller` as the addressed conversation's next
    /// turn, with `selection` or the conversation's own model.
    fn deliver(
        &mut self,
        caller: &Caller,
        message: &str,
        selection: Option<crate::ModelSelection>,
    ) -> Result<(), RuntimeError> {
        let view = self
            .session
            .projected()
            .ok_or_else(|| RuntimeError::invalid("会话尚未载入"))?;
        if self.session.busy() || matches!(view.run_state, RunState::Running | RunState::Stopping) {
            return Err(RuntimeError::invalid(
                "对方会话正在执行，未排队；请等待后再发送",
            ));
        }
        if view.run_state == RunState::Elsewhere {
            return Err(RuntimeError::invalid(
                "对方会话正在 Velune 之外运行，不能发送",
            ));
        }
        if !view.pending_interactions.is_empty() {
            return Err(RuntimeError::invalid("对方会话在等待用户审批，不能发送"));
        }
        let target = view.context_runtime_id.clone();
        let source = self
            .session
            .logical_projection
            .as_ref()
            .map(|projection| projection.origin.runtime_id.clone())
            .unwrap_or_else(|| view.conversation.runtime_id.clone());
        let id = self.addressed_id();
        let runtime_name = self.runtime_instance(&caller.runtime_id)?.name.clone();
        let selection = match selection {
            Some(selection) => selection,
            None => self.collaboration_selection(&target, None)?,
        };
        self.dispatch(&json!({"version":CONTRACT_VERSION,"action":"sendTurn","runtimeInstanceID":target,
            "payload":{"runtimeInstanceID":target,"sourceRuntimeID":source,"conversationID":id,"modelSelection":selection,
                "text":envelope_text(caller, &runtime_name, message),"effort":null,"attachments":[]}}))?;
        Ok(())
    }

    /// Address conversation `id`, opening it from its runtime's history when
    /// no session shows it.
    fn address(&mut self, id: &str) -> Result<(), RuntimeError> {
        if self.focus_existing(id) {
            return Ok(());
        }
        let listed: Vec<ConversationSummary> =
            serde_json::from_value(self.list()?["conversations"].take())?;
        let summary = listed
            .into_iter()
            .find(|item| item.id == id)
            .ok_or_else(|| RuntimeError::invalid("没有这个会话；用 list_conversations 查找"))?;
        self.focus_new();
        self.dispatch(&json!({"version":CONTRACT_VERSION,"action":"open","runtimeInstanceID":summary.runtime_id,
            "payload":{"runtimeInstanceID":summary.runtime_id,"conversationID":id}}))?;
        Ok(())
    }

    fn address_other(&mut self, caller: &Caller, id: &str) -> Result<(), RuntimeError> {
        if id == caller.conversation_id {
            return Err(RuntimeError::invalid("这是调用方自己的会话"));
        }
        self.address(id)
    }

    fn addressed_id(&self) -> String {
        self.session.conversation_id().unwrap_or_default().into()
    }

    fn addressed_runtime(&self) -> Result<String, RuntimeError> {
        self.session
            .projected()
            .map(|view| view.context_runtime_id.clone())
            .ok_or_else(|| RuntimeError::invalid("会话尚未载入"))
    }

    /// The addressed conversation as the calling agent sees it, and a
    /// signature of the state a wait compares.
    fn conversation_status(&self) -> Result<(Value, String, bool), RuntimeError> {
        let view = self
            .session
            .projected()
            .ok_or_else(|| RuntimeError::invalid("会话尚未载入"))?;
        let running =
            self.session.busy() || matches!(view.run_state, RunState::Running | RunState::Stopping);
        let state = if !view.pending_interactions.is_empty() {
            "waiting_for_approval"
        } else if running {
            "running"
        } else {
            match view.run_state {
                RunState::Failed => "failed",
                RunState::Elsewhere => "running_elsewhere",
                _ => "idle",
            }
        };
        let (reply, truncated) = latest_reply(view);
        let pending: Vec<&str> = view
            .pending_interactions
            .iter()
            .map(|interaction| interaction.id.as_str())
            .collect();
        let signature = format!(
            "{state}|{}|{}|{}",
            pending.join(","),
            view.messages.len(),
            view.messages
                .last()
                .map_or("", |message| message.id.as_str())
        );
        let status = json!({
            "conversation_id": self.addressed_id(),
            "runtime_id": view.context_runtime_id,
            "runtime_name": self.runtime_instance(&view.context_runtime_id).ok().map(|runtime| runtime.name.clone()),
            "title": view.conversation.title.display_text(),
            "cwd": view.conversation.cwd,
            "state": state,
            "pending_approvals": pending.len(),
            "reply": reply,
            "reply_truncated": truncated,
        });
        Ok((
            status,
            signature,
            state == "running" || state == "running_elsewhere",
        ))
    }

    fn observed_status(&mut self, ticket: &str) -> Result<Value, RuntimeError> {
        let (status, signature, _) = self.conversation_status()?;
        self.collaboration
            .observed
            .insert((ticket.into(), self.addressed_id()), signature);
        Ok(status)
    }

    /// The conversation's status once it has settled into a state the caller
    /// has not read yet, or when `expired`; `None` while it should wait more.
    fn settled_status(
        &mut self,
        ticket: &str,
        id: &str,
        expired: bool,
    ) -> Result<Option<Value>, RuntimeError> {
        self.address(id)?;
        let (mut status, signature, running) = self.conversation_status()?;
        let key = (ticket.to_owned(), id.to_owned());
        let news = !running && self.collaboration.observed.get(&key) != Some(&signature);
        if !news && !expired {
            return Ok(None);
        }
        status["timed_out"] = json!(!news);
        self.collaboration.observed.insert(key, signature);
        Ok(Some(status))
    }
}
