//! One application owner, advanced independently from its presentation clients.
use crate::local::{
    Focus, Login,
    conversation_history::{
        ConversationHistoryRequest, ConversationRead, LoadedConversationHistory,
    },
    list_refresh::{CodexIndexWarmup, ListRefresh, LoadedListRefresh},
};
use crate::{
    Application, ApplicationReply, ClientViewId, Command, Error, ExecutionContext, Failure,
    FailureKind, Options, Response, Revision,
};
use sha2::{Digest, Sha256};
use std::{
    collections::{BTreeMap, BTreeSet},
    sync::{
        Arc, Mutex,
        atomic::{AtomicBool, Ordering},
        mpsc,
    },
    time::{Duration, Instant},
};
use tokio::sync::broadcast;
use velune_agent_runtime::creation::{DataRetention, Retained};
mod list_refresh;
use velune_application_api::{CommandRoute, ExecutionCommandKind, HandoffRoute, HistoryLookup};
mod queued_handoff;
use list_refresh::PendingListRefresh;

#[derive(Clone)]
pub struct ApplicationService {
    sender: mpsc::Sender<Job>,
    native_user_messages: broadcast::WeakSender<String>,
    queued_handoff_cancellations: Arc<Mutex<BTreeMap<String, QueuedHandoffCancellation>>>,
}

/// An invalidation hint for the ordinary cursor-based transcript reader.
pub struct NativeUserMessageWake {
    /// An absent ID asks the client to refresh all observed conversations.
    pub conversation_id: Option<String>,
}

pub struct NativeUserMessageEvents(tokio::sync::Mutex<broadcast::Receiver<String>>);

impl NativeUserMessageEvents {
    /// Wait for a wake. An absent wake means the application owner closed.
    pub async fn next(&self) -> Option<NativeUserMessageWake> {
        let mut receiver = self.0.lock().await;
        let conversation_id = match receiver.recv().await {
            Ok(id) => Some(id),
            Err(broadcast::error::RecvError::Lagged(_)) => {
                // Refreshing all observed conversations covers the discarded wakes.
                *receiver = receiver.resubscribe();
                None
            }
            Err(broadcast::error::RecvError::Closed) => return None,
        };
        Some(NativeUserMessageWake { conversation_id })
    }
}

struct QueuedHandoffCancellation {
    conversation_id: String,
    queued_message_id: String,
    flag: Arc<AtomicBool>,
}

enum Job {
    Execute {
        view: ClientViewId,
        presentation_isolated: bool,
        context: Option<ExecutionContext>,
        command: Box<Command>,
        cancellation: Option<Arc<AtomicBool>>,
        queued_at: Instant,
        history_completion: mpsc::Sender<Job>,
        dispatch: tracing::Dispatch,
        span: tracing::Span,
        reply: mpsc::Sender<Result<ApplicationReply, Failure>>,
    },
    HistoryFinished(Box<FinishedHistory>),
    Close {
        stop_work: bool,
        reply: mpsc::Sender<Result<(), Failure>>,
    },
}

struct View {
    state: crate::local::ClientState,
    created: u64,
    used: u64,
    /// Secondary local windows and remote clients keep presentation choices
    /// in their own view without replacing the local persisted defaults.
    presentation_isolated: bool,
    observations: ViewObservations,
}

const VIEW_OBSERVATION_LEASE: Duration = Duration::from_secs(90);

#[derive(Default)]
struct ViewObservations {
    conversations: BTreeSet<(String, String)>,
    expires_at: Option<Instant>,
    active: bool,
}

impl ViewObservations {
    fn replace(&mut self, conversations: BTreeSet<(String, String)>, now: Instant) {
        self.conversations = conversations;
        self.active = !self.conversations.is_empty();
        self.expires_at = self.active.then_some(now + VIEW_OBSERVATION_LEASE);
    }

    fn renew(&mut self, now: Instant) {
        if !self.conversations.is_empty() {
            self.active = true;
            self.expires_at = Some(now + VIEW_OBSERVATION_LEASE);
        }
    }

    fn active(&self, now: Instant) -> bool {
        self.active && self.expires_at.is_some_and(|expires_at| now < expires_at)
    }

    fn expire(&mut self, now: Instant) -> bool {
        if self.active && self.expires_at.is_some_and(|expires_at| now >= expires_at) {
            self.active = false;
            return true;
        }
        false
    }
}

type CommandReply = mpsc::Sender<Result<ApplicationReply, Failure>>;

struct PendingHistory {
    view: ClientViewId,
    view_created: u64,
    conversation_id: String,
    affected_ids: Vec<String>,
    invalidated: bool,
    context: Option<ExecutionContext>,
    purpose: HistoryPurpose,
    reply: CommandReply,
}

struct StartHistoryRequest {
    view: ClientViewId,
    context: Option<ExecutionContext>,
    runtime_id: String,
    id: String,
    purpose: HistoryPurpose,
    reply: CommandReply,
    completion: mpsc::Sender<Job>,
}

struct HandoffRequirementsRequest {
    view: ClientViewId,
    context: Option<ExecutionContext>,
    source_runtime_id: String,
    conversation_id: String,
    target_runtime_id: String,
    request_id: Option<String>,
    reply: CommandReply,
    completion: mpsc::Sender<Job>,
}

struct BeginHandoffSummaryRequest {
    source_runtime_id: String,
    conversation_id: String,
    target_runtime_id: String,
    summary_model: Option<velune_application_api::ModelSelection>,
    effort: Option<String>,
    remote: bool,
    reply: CommandReply,
    completion: mpsc::Sender<Job>,
}

enum HistoryPurpose {
    ReadConversation,
    Retry(Box<Command>),
    Refresh,
    FollowSource(u64),
}

struct HistoryJob {
    completion: mpsc::Sender<Job>,
    id: u64,
    work: HistoryWork,
    queued_at: Instant,
    dispatch: tracing::Dispatch,
    span: tracing::Span,
}

enum HistoryWork {
    Conversation(ConversationHistoryRequest),
    ListRefresh(ListRefresh),
    WarmCodexIndex(CodexIndexWarmup),
    HandoffRequirements {
        history: velune_agent_runtime::history::HistoryConfig,
        native: velune_agent_runtime::native::NativeConfig,
        native_id: String,
    },
    StartHandoffSummary {
        native: velune_agent_runtime::native::NativeConfig,
        request: velune_agent_runtime::native::HandoffSummaryRequest,
    },
    RecoverHandoffFork {
        history: velune_agent_runtime::history::HistoryConfig,
        source_native_id: String,
        work_id: String,
        temporary_thread_id: Option<String>,
        temporary_fingerprint: Option<String>,
    },
    PrepareHandoffContext {
        history: velune_agent_runtime::history::HistoryConfig,
        source_native_id: String,
        source_provider_id: String,
        source_model_id: String,
        summary: Arc<Retained<PreparedHandoffSummary>>,
        work_id: String,
    },
}

struct FinishedHistory {
    id: u64,
    result: HistoryResult,
    completion: mpsc::Sender<Job>,
    dispatch: tracing::Dispatch,
    span: tracing::Span,
}

enum HistoryResult {
    Conversation(Box<LoadedConversationHistory>),
    ListRefreshed(Box<LoadedListRefresh>),
    CodexIndexWarmed,
    HandoffRequirements(Box<Result<Retained<HandoffRequirementFacts>, (String, String)>>),
    HandoffSummaryStarted {
        work_id: String,
        result: Box<Result<velune_agent_runtime::native::NativeSession, (String, String)>>,
    },
    HandoffForkRecovered {
        work_id: String,
        result: Result<velune_agent_runtime::native::HandoffSummaryRecovery, (String, String)>,
    },
    PreparedHandoffContext {
        work_id: String,
        result: Box<Result<Retained<velune_agent_runtime::import::Context>, (String, String)>>,
    },
}

struct HandoffRequirementFacts {
    required: bool,
    source_provider_id: Option<String>,
    source_model_id: Option<String>,
    models: Vec<velune_application_api::RuntimeModel>,
    default_model_issue: Option<String>,
}

fn read_handoff_requirement_facts(
    history: &velune_agent_runtime::history::HistoryConfig,
    _native: &velune_agent_runtime::native::NativeConfig,
    native_id: &str,
) -> Result<Retained<HandoffRequirementFacts>, (String, String)> {
    let required =
        velune_agent_runtime::history::codex_requires_portable_summary(history, native_id)
            .map_err(|error| (error.code().to_owned(), error.to_string()))?;
    if !required {
        return Ok(Retained::from_parts(
            HandoffRequirementFacts {
                required: false,
                source_provider_id: None,
                source_model_id: None,
                models: Vec::new(),
                default_model_issue: None,
            },
            DataRetention::default(),
        ));
    }
    let source = velune_agent_runtime::history::codex_thread_model(history, native_id)
        .map_err(|error| (error.code().to_owned(), error.to_string()))?;
    // The summary fork sets the source's base model with serviceTier=null when
    // no choice is supplied; explicit catalog variants select a specific tier.
    let default_model_issue = None;
    let listed = velune_agent_runtime::history::codex_models_for_provider(history, &source.1)
        .map_err(|error| (error.code().to_owned(), error.to_string()))?;
    Ok(source
        .zip(listed)
        .map(|((source_model_id, source_provider_id), listed)| {
            let models = listed
                .into_iter()
                .filter(|item| {
                    item.model.provider_id.as_deref() == Some(source_provider_id.as_str())
                })
                .map(|item| velune_application_api::RuntimeModel {
                    selection: velune_application_api::ModelSelection::Native {
                        provider_id: item.model.provider_id,
                        model_id: item.model.id,
                    },
                    name: item.model.name,
                    reasoning_efforts: item.reasoning_efforts,
                })
                .collect();
            HandoffRequirementFacts {
                required: true,
                source_provider_id: Some(source_provider_id),
                source_model_id: Some(source_model_id),
                models,
                default_model_issue,
            }
        }))
}

struct HistoryWorkers {
    jobs: mpsc::SyncSender<HistoryJob>,
    stopped: Arc<AtomicBool>,
}

impl HistoryWorkers {
    fn start() -> std::io::Result<Self> {
        let (jobs, receiver) = mpsc::sync_channel::<HistoryJob>(8);
        let stopped = Arc::new(AtomicBool::new(false));
        let receiver = Arc::new(Mutex::new(receiver));
        for index in 0..2 {
            let receiver = receiver.clone();
            let stopped = stopped.clone();
            std::thread::Builder::new()
                .name(format!("velune-history-{index}"))
                .spawn(move || {
                    loop {
                        let job = receiver.lock().expect("history job queue").recv();
                        let Ok(HistoryJob { completion, id, work, queued_at, dispatch, span }) = job else {
                            break;
                        };
                        if stopped.load(Ordering::Acquire) {
                            break;
                        }
                        let result = tracing::dispatcher::with_default(&dispatch, || {
                            let _entered = span.enter();
                            match work {
                                HistoryWork::Conversation(request) => {
                                    tracing::info!(target: "velune_application", event = "conversation_history_read", phase = "worker_queue", elapsed_ms = queued_at.elapsed().as_millis() as u64);
                                    HistoryResult::Conversation(Box::new(request.load()))
                                }
                                HistoryWork::ListRefresh(request) => {
                                    tracing::info!(target: "velune_application", event = "conversation_list", phase = "worker_queue", elapsed_ms = queued_at.elapsed().as_millis() as u64);
                                    HistoryResult::ListRefreshed(Box::new(request.load()))
                                }
                                HistoryWork::WarmCodexIndex(warmup) => {
                                    warmup.run();
                                    HistoryResult::CodexIndexWarmed
                                }
                                HistoryWork::HandoffRequirements { history, native, native_id } => {
                                    tracing::info!(target: "velune_application", event = "handoff_requirements_read", phase = "worker_queue", elapsed_ms = queued_at.elapsed().as_millis() as u64);
                                    let result = read_handoff_requirement_facts(&history, &native, &native_id);
                                    HistoryResult::HandoffRequirements(Box::new(result))
                                }
                                HistoryWork::StartHandoffSummary { native, request } => {
                                    let work_id = request.work_id.clone();
                                    tracing::info!(target: "velune_application", event = "handoff_summary_start", phase = "worker_queue", work_id = %work_id, native_id = %request.source_native_id, elapsed_ms = queued_at.elapsed().as_millis() as u64);
                                    let result = match velune_agent_runtime::native::NativeSession::connect(native) {
                                        Ok(mut session) => match session.begin_handoff_summary(request) {
                                            Ok(()) => Ok(session),
                                            Err(error) => {
                                                let failure = (error.code().to_owned(), error.to_string());
                                                let _ = session.shutdown();
                                                Err(failure)
                                            }
                                        },
                                        Err(error) => Err((error.code().to_owned(), error.to_string())),
                                    };
                                    HistoryResult::HandoffSummaryStarted { work_id, result: Box::new(result) }
                                }
                                HistoryWork::RecoverHandoffFork {
                                    history,
                                    source_native_id,
                                    work_id,
                                    temporary_thread_id,
                                    temporary_fingerprint,
                                } => {
                                    tracing::info!(target: "velune_application", event = "handoff_fork_recovery", phase = "worker_queue", elapsed_ms = queued_at.elapsed().as_millis() as u64);
                                    let result = velune_agent_runtime::history::recover_codex_handoff_summary(
                                        &history,
                                        &source_native_id,
                                        &work_id,
                                        temporary_thread_id.as_deref(),
                                        temporary_fingerprint.as_deref(),
                                    )
                                    .map_err(|error| (error.code().to_owned(), error.to_string()));
                                    HistoryResult::HandoffForkRecovered { work_id, result }
                                }
                                HistoryWork::PrepareHandoffContext {
                                    history,
                                    source_native_id,
                                    source_provider_id,
                                    source_model_id,
                                    summary,
                                    work_id,
                                } => {
                                    tracing::info!(target: "velune_application", event = "handoff_context_prepare", phase = "worker_queue", elapsed_ms = queued_at.elapsed().as_millis() as u64);
                                    let result = (|| {
                                        let matches = velune_agent_runtime::history::verify_codex_handoff_summary_source(
                                            &history,
                                            &source_native_id,
                                            &source_provider_id,
                                            &source_model_id,
                                            &summary.source_turn_id,
                                            &summary.source_fingerprint,
                                        )
                                        .map_err(|error| (error.code().to_owned(), error.to_string()))?;
                                        if !matches {
                                            return Err(("source_changed".into(), "摘要准备后来源会话已改变，请重新准备".into()));
                                        }
                                        let context = velune_agent_runtime::history::export_codex_context_with_summary(
                                            &history,
                                            &source_native_id,
                                            &summary.text,
                                        )
                                        .map_err(|error| (error.code().to_owned(), error.to_string()))?;
                                        let still_matches = velune_agent_runtime::history::verify_codex_handoff_summary_source(
                                            &history,
                                            &source_native_id,
                                            &source_provider_id,
                                            &source_model_id,
                                            &summary.source_turn_id,
                                            &summary.source_fingerprint,
                                        )
                                        .map_err(|error| (error.code().to_owned(), error.to_string()))?;
                                        if !still_matches {
                                            return Err(("source_changed".into(), "准备导入期间来源会话已改变，请重新准备".into()));
                                        }
                                        Ok(context)
                                    })();
                                    HistoryResult::PreparedHandoffContext { work_id, result: Box::new(result) }
                                }
                            }
                        });
                        if completion.send(Job::HistoryFinished(Box::new(FinishedHistory { id, result, completion: completion.clone(), dispatch, span }))).is_err() {
                            break;
                        }
                    }
                })?;
        }
        Ok(Self { jobs, stopped })
    }
}

impl Drop for HistoryWorkers {
    fn drop(&mut self) {
        self.stopped.store(true, Ordering::Release);
    }
}

struct Owner {
    application: Application,
    owner_sender: mpsc::Sender<Job>,
    queued_handoff_cancellations: Arc<Mutex<BTreeMap<String, QueuedHandoffCancellation>>>,
    views: BTreeMap<ClientViewId, View>,
    view_clock: u64,
    revision: Revision,
    history: HistoryWorkers,
    pending_history: BTreeMap<u64, PendingHistory>,
    pending_list_refresh: Option<PendingListRefresh>,
    pending_execution:
        BTreeMap<crate::local::execution_workers::ExecutionTicket, PendingExecutionReply>,
    handoff_requirements: BTreeMap<String, HandoffRequirementEntry>,
    pending_handoff_requirements: BTreeMap<u64, String>,
    handoff_summary_works: BTreeMap<String, HandoffSummaryWork>,
    pending_handoff_summary_starts: BTreeMap<u64, String>,
    pending_prepared_handoffs: BTreeMap<u64, PendingPreparedHandoff>,
    pending_handoff_recoveries: BTreeMap<u64, String>,
    attempted_handoff_recoveries: std::collections::BTreeSet<String>,
    next_handoff_recovery_check: Instant,
}

struct PendingExecutionReply {
    dispatch_failure: Option<Failure>,
    delivery: ExecutionReplyDelivery,
    reply: CommandReply,
}

enum ExecutionReplyDelivery {
    Conversation(Box<ConversationExecutionReply>),
    HandoffCancellation,
}

struct ConversationExecutionReply {
    view: ClientViewId,
    view_created: u64,
    context: Option<ExecutionContext>,
    runtime_id: String,
    conversation_id: String,
    creation: bool,
    submission: bool,
    address: Result<crate::local::execution_workers::ExecutionReplyAddress, Failure>,
    response: fn(crate::SnapshotResult) -> Response,
}

struct HandoffRequirementEntry {
    source_runtime_id: String,
    conversation_id: String,
    target_runtime_id: String,
    status: velune_application_api::HandoffRequirements,
    expires: Instant,
    source: Option<crate::local::handoff_summary::HandoffRequirementSource>,
    facts: Option<Arc<Retained<HandoffRequirementFacts>>>,
}

struct PreparedHandoffSummary {
    text: String,
    source_fingerprint: String,
    source_turn_id: String,
}

struct HandoffSummaryWork {
    started_at: Instant,
    dispatch: tracing::Dispatch,
    span: tracing::Span,
    queued: Option<QueuedHandoffBinding>,
    source_anchor: Option<Option<String>>,
    requirement_id: String,
    source_runtime_id: String,
    conversation_id: String,
    target_runtime_id: String,
    status: velune_application_api::HandoffSummaryStatus,
    session: Option<velune_agent_runtime::native::NativeSession>,
    summary: Option<Arc<Retained<PreparedHandoffSummary>>>,
    prepared_token: Option<String>,
    bound_request_digest: Option<String>,
    delivery_accepted: bool,
    send_pending: bool,
    source: crate::local::handoff_summary::HandoffRequirementSource,
    facts: Arc<Retained<HandoffRequirementFacts>>,
    summary_model: Option<velune_application_api::ModelSelection>,
    effort: Option<String>,
    cancel_requested: bool,
    expires: Option<Instant>,
}

impl HandoffSummaryWork {
    fn is_retained(&self, now: Instant) -> bool {
        self.expires.is_none_or(|expires| now < expires)
            || self.send_pending
            || (self.queued.is_some() && !self.delivery_accepted)
    }

    fn fail(&mut self, work_id: &str, phase: &str, code: String, detail: String) {
        tracing::dispatcher::with_default(&self.dispatch, || {
            self.span.in_scope(|| {
                tracing::warn!(
                    target: "velune_application",
                    event = "handoff_summary_failed",
                    work_id,
                    conversation_id = %self.conversation_id,
                    native_id = %self.source.native_id,
                    phase,
                    error_code = %code,
                    elapsed_ms = self.started_at.elapsed().as_millis() as u64
                );
            });
        });
        self.status = velune_application_api::HandoffSummaryStatus::Failed { code, detail };
        self.expires = Some(Instant::now() + HANDOFF_RESULT_RETENTION);
    }
}

const HANDOFF_RESULT_RETENTION: Duration = Duration::from_secs(600);
const HANDOFF_CANCELLED_RETENTION: Duration = Duration::from_secs(30);

struct QueuedHandoffBinding {
    queued_message_id: String,
    view: ClientViewId,
    cancellation: Arc<AtomicBool>,
    accepted_execution_id: Option<String>,
}

fn revoke_prepared_handoff(work: &mut HandoffSummaryWork, now: Instant) -> bool {
    if work.delivery_accepted
        || work.send_pending
        || !matches!(
            work.status,
            velune_application_api::HandoffSummaryStatus::Ready { .. }
        )
    {
        return false;
    }
    work.summary = None;
    work.prepared_token = None;
    work.status = velune_application_api::HandoffSummaryStatus::Cancelled;
    work.expires = Some(now + HANDOFF_CANCELLED_RETENTION);
    true
}

fn cancel_late_handoff_result(work: &mut HandoffSummaryWork, now: Instant) -> bool {
    if !work.cancel_requested || work.delivery_accepted {
        return false;
    }
    work.cancel_requested = false;
    work.summary = None;
    work.prepared_token = None;
    work.status = velune_application_api::HandoffSummaryStatus::Cancelled;
    work.expires = Some(now + HANDOFF_CANCELLED_RETENTION);
    true
}

fn active_handoff_summary_ids(
    pending_starts: &BTreeMap<u64, String>,
    works: &BTreeMap<String, HandoffSummaryWork>,
) -> std::collections::BTreeSet<String> {
    let mut active = pending_starts
        .values()
        .cloned()
        .collect::<std::collections::BTreeSet<_>>();
    active.extend(
        works
            .iter()
            .filter(|(_, work)| {
                work.send_pending
                    || matches!(
                        work.status,
                        velune_application_api::HandoffSummaryStatus::Running
                            | velune_application_api::HandoffSummaryStatus::Cancelling
                    )
            })
            .map(|(work_id, _)| work_id.clone()),
    );
    active
}

fn settle_terminal_handoff_artifact(
    registry: &mut crate::local::handoff_summary::ForkRegistry,
    session: &velune_agent_runtime::native::NativeSession,
    work_id: &str,
    expected_source_native_id: &str,
) -> Result<(), String> {
    let Some(artifact) = session
        .handoff_summary_artifact(work_id)
        .map_err(|error| error.to_string())?
    else {
        return Err("terminal summary work has no artifact receipt".into());
    };
    if artifact.source_native_id != expected_source_native_id
        || artifact.marker != format!("velune_handoff_summary:{work_id}")
    {
        return Err("terminal summary artifact identity does not match its registry record".into());
    }
    match artifact.state {
        velune_agent_runtime::native::HandoffSummaryArtifactState::NotCreated
        | velune_agent_runtime::native::HandoffSummaryArtifactState::Archived => {
            registry.finish(work_id).map_err(|error| error.to_string())
        }
        velune_agent_runtime::native::HandoffSummaryArtifactState::MayExist => registry
            .observe_temporary_artifact(
                work_id,
                artifact.temporary_thread_id.as_deref(),
                artifact.temporary_fingerprint.as_deref(),
            )
            .map_err(|error| error.to_string()),
    }
}

fn latest_summary_requirement_id(
    entries: &BTreeMap<String, HandoffRequirementEntry>,
    source_runtime_id: &str,
    conversation_id: &str,
    target_runtime_id: &str,
    now: Instant,
) -> Option<String> {
    entries
        .iter()
        .filter(|(_, entry)| {
            entry.expires > now
                && entry.source_runtime_id == source_runtime_id
                && entry.conversation_id == conversation_id
                && entry.target_runtime_id == target_runtime_id
                && matches!(
                    entry.status,
                    velune_application_api::HandoffRequirements::PortableSummaryRequired { .. }
                )
        })
        .max_by_key(|(_, entry)| entry.expires)
        .map(|(id, _)| id.clone())
}

struct PendingPreparedHandoff {
    view: ClientViewId,
    context: Option<ExecutionContext>,
    command: Command,
    reply: CommandReply,
    cancellation: Option<Arc<AtomicBool>>,
}

struct LiveExecution {
    kind: ExecutionCommandKind,
    runtime_id: String,
    conversation_id: String,
}

impl Owner {
    fn execute_live_command(
        &mut self,
        execution: LiveExecution,
        view: ClientViewId,
        context: Option<ExecutionContext>,
        command: Command,
        cancellation: Option<Arc<AtomicBool>>,
        reply: CommandReply,
    ) {
        let LiveExecution {
            kind,
            runtime_id,
            conversation_id,
        } = execution;
        let creation = kind == ExecutionCommandKind::Create;
        let response = kind.response();
        if let Err(error) = self.application.runtime.begin_execution_command(kind) {
            let _ = reply.send(Err(Failure::from(error)));
            return;
        }
        self.application.runtime.submission_cancellation = cancellation
            .filter(|_| kind.is_submission())
            .map(|flag| (conversation_id.clone(), flag));
        let result = self.execute(view.clone(), context.clone(), command);
        self.application.runtime.submission_cancellation = None;
        if let Some(ticket) = self.application.runtime.finish_execution_command() {
            let view_created = self
                .views
                .get(&view)
                .expect("command view retained")
                .created;
            let address = if creation {
                self.application.runtime.execution_creation_address(ticket)
            } else {
                self.application
                    .runtime
                    .execution_reply_address(&runtime_id, &conversation_id)
            }
            .map_err(Failure::from);
            self.pending_execution.insert(
                ticket,
                PendingExecutionReply {
                    dispatch_failure: result.err(),
                    delivery: ExecutionReplyDelivery::Conversation(Box::new(
                        ConversationExecutionReply {
                            view,
                            view_created,
                            context,
                            runtime_id,
                            conversation_id,
                            address,
                            creation,
                            submission: kind.is_submission(),
                            response,
                        },
                    )),
                    reply,
                },
            );
        } else {
            let _ = reply.send(result);
        }
    }

    fn finish_execution_commands(&mut self) {
        for (ticket, result) in self.application.runtime.drain_execution_results() {
            let Some(PendingExecutionReply {
                dispatch_failure,
                delivery,
                reply,
            }) = self.pending_execution.remove(&ticket)
            else {
                if let Err(error) = result {
                    tracing::warn!(target: "velune_application", event = "runtime_operation_failed", detail = %error);
                }
                continue;
            };
            let ExecutionReplyDelivery::Conversation(pending) = delivery else {
                let result = result.map_err(Failure::from).and_then(|_| {
                    if let Some(error) = dispatch_failure {
                        return Err(error);
                    }
                    self.handoff_cancelled_response()
                });
                let _ = reply.send(result);
                continue;
            };
            let submission = pending.submission;
            let refresh_error = |error: String| {
                Failure::from(if submission {
                    Error::after_delivery(error)
                } else {
                    Error::operation_completed(error)
                })
            };
            let result = result.map_err(Failure::from).and_then(|success| {
                if let Some(error) = dispatch_failure {
                    return Err(refresh_error(error.to_string()));
                }
                if self
                    .views
                    .get(&pending.view)
                    .is_none_or(|view| view.created != pending.view_created)
                {
                    return Err(refresh_error("原请求视图已关闭".into()));
                }
                let address = pending
                    .address
                    .map_err(|error| refresh_error(error.to_string()))?;
                let remote = pending.context.is_some();
                let mut response = self
                    .with_view(&pending.view, remote, |application| {
                        if pending.creation {
                            application.runtime.completed_creation_snapshot(address)
                        } else {
                            application.runtime.completed_execution_snapshot(
                                address,
                                &pending.runtime_id,
                                &pending.conversation_id,
                                success.submitted_native_message_id,
                            )
                        }
                    })
                    .map_err(|error| refresh_error(error.to_string()))?;
                if pending.submission {
                    response.submission_receipt = Some(crate::SubmissionReceipt::Sending);
                }
                self.response_for(remote, (pending.response)(response))
                    .map_err(|error| refresh_error(error.to_string()))
            });
            let _ = reply.send(result);
        }
    }

    fn execute(
        &mut self,
        view: ClientViewId,
        context: Option<ExecutionContext>,
        command: Command,
    ) -> Result<ApplicationReply, Failure> {
        let traits = command.traits();
        if context.is_some() && traits.requires_revision() {
            let expected = context
                .as_ref()
                .and_then(|context| context.expected_revision);
            if expected != Some(self.revision) {
                let mut error =
                    Failure::new(FailureKind::Conflict, "配置或编辑依据已改变，请刷新后重试");
                error.current_revision = Some(self.revision);
                return Err(error);
            }
        }
        if matches!(command, Command::ReleaseView) {
            self.views.remove(&view);
            self.sync_observed_conversations();
            return Ok(ApplicationReply {
                revision: self.revision,
                response: Response::ReleaseView,
            });
        }
        if let Command::SetObservedConversations { conversations } = command {
            self.ensure_view(&view, context.is_some());
            if let Some(current) = self.views.get_mut(&view) {
                current.observations.replace(
                    conversations
                        .into_iter()
                        .map(|conversation| (conversation.runtime_id, conversation.conversation_id))
                        .collect(),
                    Instant::now(),
                );
            }
            self.sync_observed_conversations();
            return self.response_for(context.is_some(), Response::SetObservedConversations);
        }
        for pending in self.pending_history.values_mut() {
            pending.invalidated |= traits.affects_history(&pending.affected_ids);
        }
        let changes = traits.changes_shared_state();
        let result = self.with_view(&view, context.is_some(), |application| {
            application.execute_command(command, context.is_some(), &view)
        });
        // A failed dispatch can commit association metadata; invalidate old edits too.
        if changes {
            self.revision.0 = self.revision.0.wrapping_add(1);
        }
        result.map(|response| ApplicationReply {
            revision: self.revision,
            response,
        })
    }

    fn with_view<T>(
        &mut self,
        view: &ClientViewId,
        _remote: bool,
        operation: impl FnOnce(&mut Application) -> T,
    ) -> T {
        self.sync_handoff_compaction_events();
        self.ensure_view(view, false);
        self.view_clock = self.view_clock.wrapping_add(1);
        let current = self.views.get_mut(view).expect("view was inserted");
        current.used = self.view_clock;
        let presentation_isolated = current.presentation_isolated;
        self.application
            .runtime
            .enter_client_state(&mut current.state, presentation_isolated);
        let result = operation(&mut self.application);
        self.application
            .runtime
            .leave_client_state(&mut current.state);
        result
    }

    fn sync_handoff_compaction_events(&mut self) {
        use crate::local::handoff_events::{HandoffCompactionEvent, HandoffCompactionState};
        use velune_application_api::HandoffSummaryStatus as Status;
        let mut works: Vec<_> = self.handoff_summary_works.iter().collect();
        works.sort_by_key(|(_, work)| work.started_at);
        self.application.runtime.replace_handoff_compaction_events(
            works
                .into_iter()
                .filter(|(_, work)| !work.delivery_accepted)
                .map(|(id, work)| HandoffCompactionEvent {
                    work_id: id.clone(),
                    conversation_id: work.conversation_id.clone(),
                    physical_runtime_id: work.source.physical_runtime_id.clone(),
                    physical_native_id: work.source.native_id.clone(),
                    source_anchor: work.source_anchor.clone(),
                    state: match &work.status {
                        Status::Running => HandoffCompactionState::Running,
                        Status::Cancelling => HandoffCompactionState::Cancelling,
                        Status::Ready { .. } => HandoffCompactionState::Ready,
                        Status::Failed { code, detail } => HandoffCompactionState::Failed {
                            code: code.clone(),
                            detail: detail.clone(),
                        },
                        Status::Cancelled => HandoffCompactionState::Cancelled,
                    },
                })
                .collect(),
        );
    }

    fn ensure_view(&mut self, view: &ClientViewId, presentation_isolated: bool) {
        if !self.views.contains_key(view)
            && self.views.len() >= 128
            && let Some(expired) = self
                .views
                .iter()
                .min_by_key(|(_, view)| view.used)
                .map(|(key, _)| key.clone())
        {
            self.views.remove(&expired);
            self.sync_observed_conversations();
        }
        self.view_clock = self.view_clock.wrapping_add(1);
        self.views.entry(view.clone()).or_insert_with(|| View {
            state: self.application.runtime.client_state(),
            created: self.view_clock,
            used: 0,
            presentation_isolated,
            observations: ViewObservations::default(),
        });
    }

    fn touch_client_view(&mut self, view: &ClientViewId, presentation_isolated: bool) {
        self.ensure_view(view, presentation_isolated);
        self.view_clock = self.view_clock.wrapping_add(1);
        if let Some(current) = self.views.get_mut(view) {
            current.used = self.view_clock;
            current.observations.renew(Instant::now());
        }
        self.sync_observed_conversations();
    }

    fn sync_observed_conversations(&mut self) {
        let observed = self
            .views
            .values()
            .filter(|view| view.observations.active(Instant::now()))
            .flat_map(|view| view.observations.conversations.iter().cloned())
            .collect();
        self.application
            .runtime
            .replace_observed_conversations(observed);
    }

    fn expire_observation_leases(&mut self, now: Instant) {
        let mut expired = false;
        for view in self.views.values_mut() {
            expired |= view.observations.expire(now);
        }
        if expired {
            self.sync_observed_conversations();
        }
    }

    fn start_history(&mut self, request: StartHistoryRequest) {
        let StartHistoryRequest {
            view,
            context,
            runtime_id,
            id,
            purpose,
            reply,
            completion,
        } = request;
        let remote = context.is_some();
        let source_generation = match &purpose {
            HistoryPurpose::FollowSource(generation) => Some(*generation),
            _ => None,
        };
        if self
            .pending_history
            .values()
            .any(|pending| pending.conversation_id == id)
        {
            if let Some(generation) = source_generation {
                self.with_view(&view, remote, |application| {
                    application.runtime.fail_source_refresh(generation)
                });
            }
            let _ = reply.send(Err(Failure::new(
                FailureKind::Unavailable,
                "会话历史正在读取，请稍后重试",
            )));
            return;
        }
        let is_refresh = matches!(
            purpose,
            HistoryPurpose::Refresh | HistoryPurpose::FollowSource(_)
        );
        let prepared = self.with_view(&view, remote, |application| {
            application
                .runtime
                .serve(Focus::Conversation(id.clone()), Login::Allows, |core| {
                    if is_refresh {
                        core.prepare_conversation_refresh(&runtime_id, &id)
                    } else {
                        core.prepare_conversation_read(&runtime_id, &id)
                    }
                })
        });
        let sequence = self.view_clock;
        let view_created = self
            .views
            .get(&view)
            .expect("history read has a client view")
            .created;
        match prepared {
            Ok(ConversationRead::Load(request)) => {
                let affected_ids = request.affected_ids();
                let job = HistoryJob {
                    completion,
                    id: sequence,
                    work: HistoryWork::Conversation(*request),
                    queued_at: Instant::now(),
                    dispatch: tracing::dispatcher::get_default(Clone::clone),
                    span: tracing::Span::current(),
                };
                if self.pending_history.len() >= 10 || self.history.jobs.try_send(job).is_err() {
                    if let Some(generation) = source_generation {
                        self.with_view(&view, remote, |application| {
                            application.runtime.fail_source_refresh(generation)
                        });
                    }
                    let _ = reply.send(Err(Failure::new(
                        FailureKind::Unavailable,
                        "会话历史读取繁忙，请重试",
                    )));
                    return;
                }
                self.application.runtime.begin_history_read(&id);
                self.pending_history.insert(
                    sequence,
                    PendingHistory {
                        view,
                        view_created,
                        conversation_id: id,
                        affected_ids,
                        invalidated: false,
                        context,
                        purpose,
                        reply,
                    },
                );
            }
            Ok(ConversationRead::Ready(snapshot)) => {
                if let Some(generation) = source_generation {
                    self.with_view(&view, remote, |application| {
                        application.runtime.fail_source_refresh(generation)
                    });
                }
                let result = match purpose {
                    HistoryPurpose::ReadConversation => self.history_response(*snapshot, remote),
                    HistoryPurpose::Retry(command) => self.execute(view, context, *command),
                    HistoryPurpose::Refresh => self.execute(
                        view,
                        context,
                        Command::OpenConversation {
                            runtime_id,
                            conversation_id: id,
                        },
                    ),
                    HistoryPurpose::FollowSource(_) => Ok(ApplicationReply {
                        revision: self.revision,
                        response: Response::ReleaseView,
                    }),
                };
                let _ = reply.send(result);
            }
            Err(error) => {
                if let Some(generation) = source_generation {
                    self.with_view(&view, remote, |application| {
                        application.runtime.fail_source_refresh(generation)
                    });
                }
                let _ = reply.send(Err(Failure::from(error)));
            }
        }
    }

    fn read_handoff_requirements(&mut self, request: HandoffRequirementsRequest) {
        let HandoffRequirementsRequest {
            view,
            context,
            source_runtime_id,
            conversation_id,
            target_runtime_id,
            request_id,
            reply,
            completion,
        } = request;
        use velune_application_api::HandoffRequirements as Requirements;
        let remote = context.is_some();
        let now = Instant::now();
        self.handoff_requirements
            .retain(|_, entry| entry.expires > now);
        if let Some(request_id) = request_id {
            let status = self
                .handoff_requirements
                .get(&request_id)
                .filter(|entry| {
                    entry.source_runtime_id == source_runtime_id
                        && entry.conversation_id == conversation_id
                        && entry.target_runtime_id == target_runtime_id
                })
                .map(|entry| entry.status.clone())
                .unwrap_or_else(|| Requirements::Failed {
                    code: "request_expired".into(),
                    detail: "预检已过期，请重新检查接续条件".into(),
                });
            let result = self.response_for(remote, Response::ReadHandoffRequirements(status));
            let _ = reply.send(result);
            return;
        }

        // Reuse an in-flight read for the same immutable source/target pair.
        if let Some((_, entry)) = self.handoff_requirements.iter().find(|(_, entry)| {
            entry.expires > now
                && entry.source_runtime_id == source_runtime_id
                && entry.conversation_id == conversation_id
                && entry.target_runtime_id == target_runtime_id
                && matches!(entry.status, Requirements::Checking { .. })
        }) {
            let result = self.response_for(
                remote,
                Response::ReadHandoffRequirements(entry.status.clone()),
            );
            let _ = reply.send(result);
            return;
        }

        let source = self.with_view(&view, remote, |application| {
            application.runtime.serve(
                Focus::Conversation(conversation_id.clone()),
                Login::Allows,
                |core| {
                    core.handoff_requirement_source(
                        &source_runtime_id,
                        &conversation_id,
                        &target_runtime_id,
                    )
                },
            )
        });
        let source = match source {
            Ok(source) => source,
            Err(error) => {
                let status = Requirements::Failed {
                    code: "requirements_unavailable".into(),
                    detail: error.to_string(),
                };
                let result = self.response_for(remote, Response::ReadHandoffRequirements(status));
                let _ = reply.send(result);
                return;
            }
        };
        let Some(source) = source else {
            let result = self.response_for(
                remote,
                Response::ReadHandoffRequirements(Requirements::Ready),
            );
            let _ = reply.send(result);
            return;
        };
        if remote {
            let status = Requirements::Failed {
                code: "portable_summary_unsupported_remote".into(),
                detail: "当前远程控制连接尚不支持跨 Harness 摘要接续".into(),
            };
            let result = self.response_for(remote, Response::ReadHandoffRequirements(status));
            let _ = reply.send(result);
            return;
        }
        if self.handoff_requirements.len() >= 64 || self.pending_handoff_requirements.len() >= 10 {
            let status = Requirements::Failed {
                code: "requirements_busy".into(),
                detail: "接续预检繁忙，请稍后重试".into(),
            };
            let result = self.response_for(remote, Response::ReadHandoffRequirements(status));
            let _ = reply.send(result);
            return;
        }

        self.view_clock = self.view_clock.wrapping_add(1);
        let id = self.view_clock;
        let request_id = crate::new_record_key();
        let status = Requirements::Checking {
            request_id: request_id.clone(),
        };
        self.handoff_requirements.insert(
            request_id.clone(),
            HandoffRequirementEntry {
                source_runtime_id,
                conversation_id,
                target_runtime_id,
                status,
                expires: now + Duration::from_secs(180),
                source: Some(source.clone()),
                facts: None,
            },
        );
        self.pending_handoff_requirements
            .insert(id, request_id.clone());
        let job = HistoryJob {
            completion,
            id,
            work: HistoryWork::HandoffRequirements {
                history: source.history,
                native: source.native,
                native_id: source.native_id,
            },
            queued_at: now,
            dispatch: tracing::dispatcher::get_default(Clone::clone),
            span: tracing::Span::current(),
        };
        if self.history.jobs.try_send(job).is_err() {
            self.pending_handoff_requirements.remove(&id);
            if let Some(entry) = self.handoff_requirements.get_mut(&request_id) {
                entry.status = Requirements::Failed {
                    code: "requirements_busy".into(),
                    detail: "接续预检繁忙，请稍后重试".into(),
                };
            }
        }
        let status = self
            .handoff_requirements
            .get(&request_id)
            .map(|entry| entry.status.clone())
            .unwrap_or_else(|| Requirements::Failed {
                code: "requirements_unavailable".into(),
                detail: "接续预检无法启动，请重试".into(),
            });
        let result = self.response_for(remote, Response::ReadHandoffRequirements(status));
        let _ = reply.send(result);
    }

    fn finish_handoff_requirements(
        &mut self,
        id: u64,
        result: Result<Retained<HandoffRequirementFacts>, (String, String)>,
    ) {
        use velune_application_api::HandoffRequirements as Requirements;
        let Some(request_id) = self.pending_handoff_requirements.remove(&id) else {
            return;
        };
        let Some(entry) = self.handoff_requirements.get(&request_id) else {
            return;
        };
        let source_runtime_id = entry.source_runtime_id.clone();
        let basis_runtime_id = entry
            .source
            .as_ref()
            .map(|source| source.physical_runtime_id.clone())
            .unwrap_or_else(|| source_runtime_id.clone());
        let result = result.map(Arc::new);
        let status = match &result {
            Ok(facts) if !facts.required => Requirements::Ready,
            Ok(facts) => match (&facts.source_provider_id, &facts.source_model_id) {
                (Some(provider), Some(model)) => match self.application.runtime.serve(
                    Focus::Conversation(entry.conversation_id.clone()),
                    Login::Allows,
                    |core| {
                        Ok(core.handoff_summary_basis(
                            &basis_runtime_id,
                            model,
                            provider,
                            facts.models.clone(),
                        ))
                    },
                ) {
                    Ok(summary_basis) => Requirements::PortableSummaryRequired {
                        summary_basis: Box::new(summary_basis),
                        default_model_issue: facts.default_model_issue.clone(),
                    },
                    Err(error) => Requirements::Failed {
                        code: "requirements_unavailable".into(),
                        detail: error.to_string(),
                    },
                },
                _ => Requirements::Failed {
                    code: "source_model_unavailable".into(),
                    detail: "无法确认来源会话当前模型，请刷新后重试".into(),
                },
            },
            Err((code, detail)) => Requirements::Failed {
                code: code.clone(),
                detail: detail.clone(),
            },
        };
        if let Some(entry) = self.handoff_requirements.get_mut(&request_id) {
            entry.status = status;
            entry.expires = Instant::now() + Duration::from_secs(180);
            entry.facts = result.ok();
        }
    }

    fn begin_handoff_summary(&mut self, request: BeginHandoffSummaryRequest) {
        let BeginHandoffSummaryRequest {
            source_runtime_id,
            conversation_id,
            target_runtime_id,
            summary_model,
            effort,
            remote,
            reply,
            completion,
        } = request;
        use velune_application_api::{HandoffSummaryStart, HandoffSummaryStatus};
        if remote {
            let mut failure =
                Failure::new(FailureKind::Unsupported, "当前远程控制连接尚不支持摘要接续");
            failure.code = "portable_summary_unsupported_remote".into();
            let _ = reply.send(Err(failure));
            return;
        }
        let requirement_id = latest_summary_requirement_id(
            &self.handoff_requirements,
            &source_runtime_id,
            &conversation_id,
            &target_runtime_id,
            Instant::now(),
        );
        let Some(requirement_id) = requirement_id else {
            let _ = reply.send(Err(Failure::new(
                FailureKind::Conflict,
                "接续预检已过期或来源已改变，请重新检查后准备摘要",
            )));
            return;
        };
        if let Some((id, _)) = self.handoff_summary_works.iter().find(|(_, work)| {
            work.requirement_id == requirement_id
                && work.summary_model == summary_model
                && work.effort == effort
                && work.is_retained(Instant::now())
                && matches!(
                    work.status,
                    HandoffSummaryStatus::Running
                        | HandoffSummaryStatus::Cancelling
                        | HandoffSummaryStatus::Ready { .. }
                )
        }) {
            let _ = reply.send(self.response_for(
                remote,
                Response::BeginHandoffSummary(HandoffSummaryStart {
                    work_id: id.clone(),
                }),
            ));
            return;
        }
        let active_summary_work = self
            .handoff_summary_works
            .values()
            .filter(|work| {
                matches!(
                    work.status,
                    velune_application_api::HandoffSummaryStatus::Running
                        | velune_application_api::HandoffSummaryStatus::Cancelling
                )
            })
            .count();
        if active_summary_work + self.pending_handoff_summary_starts.len() >= 2 {
            let _ = reply.send(Err(Failure::new(
                FailureKind::Unavailable,
                "摘要准备队列繁忙，请稍后重试",
            )));
            return;
        }
        let entry = self
            .handoff_requirements
            .get(&requirement_id)
            .expect("selected requirement");
        let Some(source) = entry.source.clone() else {
            let _ = reply.send(Err(Failure::new(
                FailureKind::Conflict,
                "摘要来源配置已过期，请重新检查接续条件",
            )));
            return;
        };
        let Some(facts) = entry.facts.clone() else {
            let _ = reply.send(Err(Failure::new(
                FailureKind::Conflict,
                "摘要来源事实已过期，请重新检查接续条件",
            )));
            return;
        };
        let (Some(source_provider_id), Some(source_model_id)) = (
            facts.source_provider_id.clone(),
            facts.source_model_id.clone(),
        ) else {
            let _ = reply.send(Err(Failure::new(
                FailureKind::Unavailable,
                "无法确认来源模型，请刷新目录后重试",
            )));
            return;
        };
        if summary_model.is_none() && facts.default_model_issue.is_some() {
            let mut failure = Failure::new(
                FailureKind::Invalid,
                facts.default_model_issue.clone().unwrap_or_default(),
            );
            failure.code = "summary_model_required".into();
            let _ = reply.send(Err(failure));
            return;
        }
        let selected_native = match &summary_model {
            None => None,
            Some(velune_application_api::ModelSelection::Native {
                provider_id: Some(provider),
                model_id,
            }) if provider == &source_provider_id => facts
                .models
                .iter()
                .find(|candidate| {
                    matches!(&candidate.selection, velune_application_api::ModelSelection::Native { provider_id: Some(candidate_provider), model_id: candidate_model } if candidate_provider == provider && candidate_model == model_id)
                })
                .map(|candidate| velune_agent_runtime::native::RuntimeModel {
                    provider_id: Some(provider.clone()),
                    id: model_id.clone(),
                    name: candidate.name.clone(),
                }),
            _ => None,
        };
        if summary_model.is_some() && selected_native.is_none() {
            let mut failure = Failure::new(
                FailureKind::Invalid,
                "摘要模型必须来自来源会话当前 provider 的可用模型目录",
            );
            failure.code = "incompatible_summary_model".into();
            let _ = reply.send(Err(failure));
            return;
        }
        if let Some(effort) = &effort {
            let supported = match &summary_model {
                Some(selection) => facts.models.iter().any(|candidate| {
                    candidate.selection == *selection
                        && candidate
                            .reasoning_efforts
                            .iter()
                            .any(|level| level == effort)
                }),
                None => true,
            };
            if !supported {
                let mut failure =
                    Failure::new(FailureKind::Invalid, "所选摘要模型不支持此推理强度");
                failure.code = "unsupported_summary_effort".into();
                let _ = reply.send(Err(failure));
                return;
            }
        }
        let work_id = crate::new_record_key();
        let request = velune_agent_runtime::native::HandoffSummaryRequest {
            work_id: work_id.clone(),
            source_native_id: source.native_id.clone(),
            source_provider_id,
            source_model_id,
            selected_model: selected_native,
            effort: effort.clone(),
        };
        if let Err(error) = self.application.runtime.handoff_forks.begin(
            &work_id,
            &source.physical_runtime_id,
            &source.native_id,
            &source.native.agent_dir,
        ) {
            let mut failure = Failure::new(FailureKind::Unavailable, error.to_string());
            failure.code = "summary_registry_unavailable".into();
            let _ = reply.send(Err(failure));
            return;
        }
        let mut summary_native = source.native.clone();
        // The summary fork is intentionally isolated from Velune-managed MCP
        // processes. The app-server fork additionally disables inherited MCP
        // definitions in its own config overlay.
        summary_native.mcp_servers.clear();
        self.view_clock = self.view_clock.wrapping_add(1);
        let job_id = self.view_clock;
        self.pending_handoff_summary_starts
            .insert(job_id, work_id.clone());
        let source_anchor = self.application.runtime.handoff_compaction_anchor(
            &conversation_id,
            &source.physical_runtime_id,
            &source.native_id,
        );
        let dispatch = tracing::dispatcher::get_default(Clone::clone);
        let span = tracing::Span::current();
        self.handoff_summary_works.insert(
            work_id.clone(),
            HandoffSummaryWork {
                started_at: Instant::now(),
                dispatch: dispatch.clone(),
                span: span.clone(),
                queued: None,
                source_anchor,
                requirement_id,
                source_runtime_id,
                conversation_id,
                target_runtime_id,
                status: HandoffSummaryStatus::Running,
                session: None,
                summary: None,
                prepared_token: None,
                bound_request_digest: None,
                delivery_accepted: false,
                send_pending: false,
                source,
                facts,
                summary_model,
                effort,
                cancel_requested: false,
                expires: None,
            },
        );
        let job = HistoryJob {
            completion,
            id: job_id,
            work: HistoryWork::StartHandoffSummary {
                native: summary_native,
                request,
            },
            queued_at: Instant::now(),
            dispatch,
            span,
        };
        if self.history.jobs.try_send(job).is_err() {
            self.pending_handoff_summary_starts.remove(&job_id);
            let _ = self.application.runtime.handoff_forks.finish(&work_id);
            if let Some(work) = self.handoff_summary_works.get_mut(&work_id) {
                work.fail(
                    &work_id,
                    "worker_queue",
                    "summary_queue_full".into(),
                    "摘要准备队列繁忙，请稍后重试".into(),
                );
            }
        }
        let _ = reply.send(self.response_for(
            remote,
            Response::BeginHandoffSummary(HandoffSummaryStart { work_id }),
        ));
    }

    fn read_handoff_summary(&mut self, work_id: String, remote: bool, reply: CommandReply) {
        if remote {
            let mut failure =
                Failure::new(FailureKind::Unsupported, "当前远程控制连接尚不支持摘要接续");
            failure.code = "portable_summary_unsupported_remote".into();
            let _ = reply.send(Err(failure));
            return;
        }
        let status = self
            .handoff_summary_works
            .get(&work_id)
            .filter(|work| work.is_retained(Instant::now()))
            .map(|work| work.status.clone())
            .unwrap_or_else(|| velune_application_api::HandoffSummaryStatus::Failed {
                code: "summary_expired".into(),
                detail: "摘要准备已过期，请重新检查来源并再次准备".into(),
            });
        let _ = reply.send(self.response_for(remote, Response::ReadHandoffSummary(status)));
    }

    fn send_prepared_handoff(
        &mut self,
        view: ClientViewId,
        context: Option<ExecutionContext>,
        command: Command,
        cancellation: Option<Arc<AtomicBool>>,
        reply: CommandReply,
        completion: mpsc::Sender<Job>,
    ) {
        if context.is_some() {
            let mut failure =
                Failure::new(FailureKind::Unsupported, "当前远程控制连接尚不支持摘要接续");
            failure.code = "portable_summary_unsupported_remote".into();
            let _ = reply.send(Err(failure));
            return;
        }
        let Command::SendPreparedHandoff {
            source_runtime_id,
            conversation_id,
            target_runtime_id,
            selection,
            turn,
            prepared_token,
            ..
        } = &command
        else {
            let _ = reply.send(Err(Failure::new(
                FailureKind::Contract,
                "prepared handoff command mismatch",
            )));
            return;
        };
        let request_bytes = match serde_json::to_vec(&(
            source_runtime_id,
            conversation_id,
            target_runtime_id,
            selection,
            turn,
        )) {
            Ok(bytes) => bytes,
            Err(error) => {
                let _ = reply.send(Err(Failure::from(error)));
                return;
            }
        };
        let request_digest = format!("{:x}", Sha256::digest(&request_bytes));
        let prepared = {
            let Some((actual_work_id, work)) =
                self.handoff_summary_works.iter_mut().find(|(_, work)| {
                    work.prepared_token.as_deref() == Some(prepared_token)
                        && work.source_runtime_id == *source_runtime_id
                        && work.conversation_id == *conversation_id
                        && work.target_runtime_id == *target_runtime_id
                })
            else {
                let _ = reply.send(Err(Failure::new(
                    FailureKind::Conflict,
                    "摘要凭据已过期或与当前来源、目标不匹配；请重新准备接续",
                )));
                return;
            };
            let failure = if !matches!(
                work.status,
                velune_application_api::HandoffSummaryStatus::Ready { .. }
            ) {
                Some(Failure::new(
                    FailureKind::Conflict,
                    "摘要尚未准备完成，请刷新准备状态",
                ))
            } else if work.delivery_accepted {
                let mut failure = Failure::new(
                    FailureKind::Conflict,
                    "此请求已提交；请刷新会话状态，避免重复发送",
                );
                failure.code = "handoff_already_delivered".into();
                Some(failure)
            } else if work.send_pending {
                let mut failure = Failure::new(
                    FailureKind::Unavailable,
                    "接续请求正在提交，请等待结果后刷新会话",
                );
                failure.code = "handoff_send_pending".into();
                Some(failure)
            } else if work
                .bound_request_digest
                .as_ref()
                .is_some_and(|bound| bound != &request_digest)
            {
                let mut failure = Failure::new(
                    FailureKind::Conflict,
                    "摘要凭据已绑定另一条输入，请重新准备接续",
                );
                failure.code = "prepared_request_mismatch".into();
                Some(failure)
            } else if work.summary.is_none() {
                Some(Failure::new(
                    FailureKind::Conflict,
                    "摘要或来源校验信息不可用，请重新准备接续",
                ))
            } else {
                None
            };
            if let Some(failure) = failure {
                let _ = reply.send(Err(failure));
                return;
            }
            work.bound_request_digest = Some(request_digest.clone());
            work.send_pending = true;
            (
                actual_work_id.clone(),
                work.source.clone(),
                work.facts.clone(),
                work.summary.clone().expect("validated summary"),
            )
        };
        let (work_id, source, facts, summary) = prepared;
        let (Some(source_provider_id), Some(source_model_id)) = (
            facts.source_provider_id.clone(),
            facts.source_model_id.clone(),
        ) else {
            if let Some(work) = self.handoff_summary_works.get_mut(&work_id) {
                work.send_pending = false;
            }
            let _ = reply.send(Err(Failure::new(
                FailureKind::Conflict,
                "摘要来源模型已失效，请重新准备接续",
            )));
            return;
        };
        let mut job_id = self.view_clock.wrapping_add(1);
        if job_id == 0 {
            job_id = 1;
        }
        self.view_clock = job_id;
        let job = HistoryJob {
            completion,
            id: job_id,
            work: HistoryWork::PrepareHandoffContext {
                history: source.history,
                source_native_id: source.native_id,
                source_provider_id,
                source_model_id,
                summary,
                work_id: work_id.clone(),
            },
            queued_at: Instant::now(),
            dispatch: tracing::dispatcher::get_default(Clone::clone),
            span: tracing::Span::current(),
        };
        if self.history.jobs.try_send(job).is_err() {
            if let Some(work) = self.handoff_summary_works.get_mut(&work_id) {
                work.send_pending = false;
                work.bound_request_digest = None;
            }
            self.application.runtime.unbind_queued_preparation(&work_id);
            let mut failure = Failure::new(
                FailureKind::Unavailable,
                "接续校验队列繁忙，请稍后重试；摘要仍可复用",
            );
            failure.code = "prepared_send_busy".into();
            let _ = reply.send(Err(failure));
            return;
        }
        self.pending_prepared_handoffs.insert(
            job_id,
            PendingPreparedHandoff {
                view,
                context,
                command,
                reply,
                cancellation,
            },
        );
    }

    fn finish_prepared_handoff_context(
        &mut self,
        id: u64,
        work_id: String,
        result: Result<Retained<velune_agent_runtime::import::Context>, (String, String)>,
    ) {
        let Some(pending) = self.pending_prepared_handoffs.remove(&id) else {
            return;
        };
        let remote = pending.context.is_some();
        let Some(work) = self.handoff_summary_works.get_mut(&work_id) else {
            let _ = pending.reply.send(Err(Failure::new(
                FailureKind::Conflict,
                "摘要凭据已过期，请重新准备接续",
            )));
            return;
        };
        work.send_pending = false;
        let queued_message_id = work
            .queued
            .as_ref()
            .map(|binding| binding.queued_message_id.clone());
        if cancel_late_handoff_result(work, Instant::now()) {
            let mut failure = Failure::new(FailureKind::Unavailable, "接续提交已取消");
            failure.code = "handoff_cancelled".into();
            let _ = pending.reply.send(Err(failure));
            return;
        }
        let context = match result {
            Ok(context) => context,
            Err((code, detail)) => {
                self.application.runtime.pause_queued_preparation(&work_id);
                work.bound_request_digest = None;
                self.application.runtime.unbind_queued_preparation(&work_id);
                if code == "source_changed" {
                    work.fail(&work_id, "verify_source", code.clone(), detail.clone());
                    work.prepared_token = None;
                }
                let mut failure = Failure::new(FailureKind::Conflict, detail);
                failure.code = code;
                let _ = pending.reply.send(Err(failure));
                return;
            }
        };
        let traits = pending.command.traits();
        let changes = traits.changes_shared_state();
        for history in self.pending_history.values_mut() {
            history.invalidated |= traits.affects_history(&history.affected_ids);
        }
        let conversation_id = match &pending.command {
            Command::SendPreparedHandoff {
                conversation_id, ..
            } => conversation_id.clone(),
            _ => String::new(),
        };
        self.application.runtime.submission_cancellation =
            pending.cancellation.map(|flag| (conversation_id, flag));
        let response = self.with_view(&pending.view, remote, |application| {
            application.execute_prepared_handoff(
                pending.command,
                remote,
                &pending.view,
                context.as_ref(),
                &work_id,
                queued_message_id.as_deref(),
            )
        });
        self.application.runtime.submission_cancellation = None;
        if response.is_err() {
            self.application.runtime.pause_queued_preparation(&work_id);
        }
        if changes {
            self.revision.0 = self.revision.0.wrapping_add(1);
        }
        if (response.is_ok()
            || response
                .as_ref()
                .is_err_and(|failure| failure.code == "message_delivery_accepted"))
            && let Some(work) = self.handoff_summary_works.get_mut(&work_id)
        {
            work.delivery_accepted = true;
            if let Some(binding) = &mut work.queued {
                binding.accepted_execution_id = self
                    .application
                    .runtime
                    .queued_handoff_execution(&work.conversation_id, &work.target_runtime_id);
            }
            work.summary = None;
            work.prepared_token = None;
        }
        let _ = pending
            .reply
            .send(response.map(|response| ApplicationReply {
                revision: self.revision,
                response,
            }));
    }

    fn cancel_handoff_summary(&mut self, work_id: String, remote: bool, reply: CommandReply) {
        use velune_application_api::HandoffSummaryStatus;
        if remote {
            let mut failure =
                Failure::new(FailureKind::Unsupported, "当前远程控制连接尚不支持摘要接续");
            failure.code = "portable_summary_unsupported_remote".into();
            let _ = reply.send(Err(failure));
            return;
        }
        let accepted = self
            .handoff_summary_works
            .get(&work_id)
            .filter(|work| work.delivery_accepted)
            .and_then(|work| {
                work.queued.as_ref().map(|binding| {
                    (
                        work.conversation_id.clone(),
                        work.target_runtime_id.clone(),
                        binding.view.clone(),
                        binding.accepted_execution_id.clone(),
                    )
                })
            });
        if let Some((conversation_id, target_runtime_id, view, execution_id)) = accepted {
            use crate::local::execution_workers::ExecutionCommandKind;
            if let Err(error) = self
                .application
                .runtime
                .begin_execution_command(ExecutionCommandKind::Cancel)
            {
                let _ = reply.send(Err(Failure::from(error)));
                return;
            }
            // Prevent the tail from dispatching if the accepted turn ends while
            // its native stop is in flight. The dispatch below rechecks the
            // recorded execution ID after entering the ordinary control scope.
            self.application
                .runtime
                .pause_queued_conversation(&conversation_id);
            let result = self
                .with_view(&view, false, |application| {
                    application.runtime.serve_live_control(
                        Focus::Conversation(conversation_id.clone()),
                        |core| {
                            core.cancel_accepted_queued_handoff(
                                &conversation_id,
                                &target_runtime_id,
                                execution_id.as_deref(),
                            )
                        },
                    )
                })
                .map_err(Failure::from);
            if let Some(ticket) = self.application.runtime.finish_execution_command() {
                self.pending_execution.insert(
                    ticket,
                    PendingExecutionReply {
                        dispatch_failure: result.err(),
                        delivery: ExecutionReplyDelivery::HandoffCancellation,
                        reply,
                    },
                );
            } else {
                let _ = reply.send(result.and_then(|()| self.handoff_cancelled_response()));
            }
            return;
        }
        let Some(work) = self.handoff_summary_works.get_mut(&work_id) else {
            // Cancellation is idempotent, including after a short-lived
            // cancelled record has naturally expired.
            let _ = reply.send(self.response_for(
                remote,
                Response::CancelHandoffSummary(HandoffSummaryStatus::Cancelled),
            ));
            return;
        };
        if work.delivery_accepted {
            let mut failure = Failure::new(
                FailureKind::Conflict,
                "接续请求已提交；请使用普通会话停止操作管理目标执行",
            );
            failure.code = "handoff_already_delivered".into();
            let _ = reply.send(Err(failure));
            return;
        }
        match work.status {
            HandoffSummaryStatus::Running | HandoffSummaryStatus::Cancelling => {
                work.cancel_requested = true;
                if let Some(mut session) = work.session.take() {
                    // This native process belongs only to this summary job.
                    // Stop its transport now; the marked fork's separate safe
                    // recovery is not an acknowledgement required from the model.
                    if let Err(error) = settle_terminal_handoff_artifact(
                        &mut self.application.runtime.handoff_forks,
                        &session,
                        &work_id,
                        &work.source.native_id,
                    ) {
                        tracing::warn!(target:"velune_application",event="handoff_cancel_receipt_pending",work_id=%work_id,detail=%error);
                    }
                    work.status = match session.shutdown() {
                        Ok(()) => HandoffSummaryStatus::Cancelled,
                        Err(error) => HandoffSummaryStatus::Failed {
                            code: error.code().to_owned(),
                            detail: error.to_string(),
                        },
                    };
                } else {
                    // A worker may still be opening its inert state machine.
                    // The late-start path terminates it before its first poll.
                    work.status = HandoffSummaryStatus::Cancelled;
                }
                work.expires = Some(Instant::now() + HANDOFF_CANCELLED_RETENTION);
            }
            HandoffSummaryStatus::Ready { .. } if work.send_pending => {
                // The source CAS/export is already running on a worker. Its
                // completion checks this flag before any target dispatch.
                work.cancel_requested = true;
            }
            HandoffSummaryStatus::Ready { .. } => {
                let _ = revoke_prepared_handoff(work, Instant::now());
            }
            HandoffSummaryStatus::Cancelled => {
                work.expires = Some(Instant::now() + HANDOFF_CANCELLED_RETENTION);
            }
            HandoffSummaryStatus::Failed { .. } => {}
        }
        let status = work.status.clone();
        let _ = reply.send(self.response_for(remote, Response::CancelHandoffSummary(status)));
    }

    fn handoff_cancelled_response(&mut self) -> Result<ApplicationReply, Failure> {
        self.response_for(
            false,
            Response::CancelHandoffSummary(velune_application_api::HandoffSummaryStatus::Cancelled),
        )
    }

    fn finish_handoff_summary_start(
        &mut self,
        job_id: u64,
        work_id: String,
        result: Result<velune_agent_runtime::native::NativeSession, (String, String)>,
    ) {
        use velune_application_api::HandoffSummaryStatus as Status;
        self.pending_handoff_summary_starts.remove(&job_id);
        let Some(work) = self.handoff_summary_works.get_mut(&work_id) else {
            if let Ok(mut session) = result {
                let _ = session.shutdown();
            }
            return;
        };
        match result {
            Ok(mut session) => {
                if work.cancel_requested {
                    let _ = settle_terminal_handoff_artifact(
                        &mut self.application.runtime.handoff_forks,
                        &session,
                        &work_id,
                        &work.source.native_id,
                    );
                    work.status = match session.shutdown() {
                        Ok(()) => Status::Cancelled,
                        Err(error) => Status::Failed {
                            code: error.code().to_owned(),
                            detail: error.to_string(),
                        },
                    };
                    work.session = None;
                } else {
                    work.session = Some(session);
                }
            }
            Err((code, detail)) => {
                if !work.cancel_requested {
                    work.fail(&work_id, "start_native_session", code, detail);
                }
                // The start worker returns before any fork is created; the
                // native summary RPC sequence only begins after this session
                // has been installed and polled by the owner.
                let _ = self.application.runtime.handoff_forks.finish(&work_id);
            }
        }
    }

    fn advance_handoff_summaries(&mut self) {
        use velune_application_api::HandoffSummaryStatus as Status;
        let now = Instant::now();
        self.handoff_summary_works
            .retain(|_, work| work.is_retained(now));
        for (work_id, work) in &mut self.handoff_summary_works {
            let Some(session) = &mut work.session else {
                continue;
            };
            if !matches!(work.status, Status::Running | Status::Cancelling) {
                continue;
            }
            let poll_error = session.poll().err();
            let native_status = session.handoff_summary_status(work_id);
            let status = match (poll_error, native_status) {
                (None, Ok(status)) => status,
                (Some(_), Ok(status))
                    if matches!(
                        status.as_deref(),
                        Some(
                            velune_agent_runtime::native::HandoffSummaryStatus::Ready { .. }
                                | velune_agent_runtime::native::HandoffSummaryStatus::Failed { .. }
                        )
                    ) =>
                {
                    status
                }
                (Some(error), _) | (None, Err(error)) => {
                    if let Err(receipt_error) = settle_terminal_handoff_artifact(
                        &mut self.application.runtime.handoff_forks,
                        session,
                        work_id,
                        &work.source.native_id,
                    ) {
                        tracing::warn!(target: "velune_application", event = "handoff_fork_terminal_receipt_failed", work_id = %work_id, detail = %receipt_error);
                    }
                    let _ = session.shutdown();
                    work.session = None;
                    work.fail(
                        work_id,
                        "poll_native_session",
                        error.code().to_owned(),
                        error.to_string(),
                    );
                    continue;
                }
            };
            let Some(status) = status else {
                let _ = session.shutdown();
                work.session = None;
                work.fail(
                    work_id,
                    "poll_native_session",
                    "summary_status_missing".into(),
                    "摘要任务状态已失效，请重新准备".into(),
                );
                continue;
            };
            let (status, retention) = status.into_parts();
            match status {
                velune_agent_runtime::native::HandoffSummaryStatus::Running {
                    phase,
                    temporary_thread_id,
                    temporary_fingerprint,
                    ..
                } => {
                    if let Some(temporary_thread_id) = temporary_thread_id
                        && let Err(error) =
                            self.application.runtime.handoff_forks.observe_temporary(
                                work_id,
                                &temporary_thread_id,
                                temporary_fingerprint.as_deref(),
                            )
                    {
                        tracing::warn!(target: "velune_application", event = "handoff_fork_registry_update_failed", work_id = %work_id, detail = %error);
                    }
                    work.status =
                        if phase == velune_agent_runtime::native::HandoffSummaryPhase::Cancelling {
                            Status::Cancelling
                        } else {
                            Status::Running
                        };
                }
                velune_agent_runtime::native::HandoffSummaryStatus::Ready {
                    summary,
                    source_fingerprint,
                    source_turn_id,
                    cleanup_state,
                    temporary_thread_id,
                    temporary_fingerprint,
                    ..
                } => {
                    if cleanup_state
                        == velune_agent_runtime::native::HandoffSummaryCleanupState::Archived
                    {
                        if let Err(error) = self.application.runtime.handoff_forks.finish(work_id) {
                            tracing::warn!(target: "velune_application", event = "handoff_fork_registry_finish_failed", work_id = %work_id, detail = %error);
                        }
                    } else if let Err(error) =
                        self.application.runtime.handoff_forks.observe_temporary(
                            work_id,
                            &temporary_thread_id,
                            Some(&temporary_fingerprint),
                        )
                    {
                        tracing::warn!(target: "velune_application", event = "handoff_fork_registry_update_failed", work_id = %work_id, detail = %error);
                    }
                    // `session` is borrowed from `work.session`, so keep this
                    // cleanup local instead of calling the helper that borrows
                    // the whole work item. The native result is terminal here;
                    // we still record its fork cleanup outcome above, then
                    // discard the prepared summary without exposing a token.
                    if work.cancel_requested && !work.delivery_accepted {
                        work.cancel_requested = false;
                        work.summary = None;
                        work.prepared_token = None;
                        work.status = Status::Cancelled;
                        work.expires = Some(now + HANDOFF_CANCELLED_RETENTION);
                        let _ = session.shutdown();
                        work.session = None;
                        continue;
                    }
                    let token = crate::new_record_key();
                    work.summary = Some(Arc::new(Retained::from_parts(
                        PreparedHandoffSummary {
                            text: summary,
                            source_fingerprint,
                            source_turn_id,
                        },
                        retention,
                    )));
                    work.prepared_token = Some(token.clone());
                    work.bound_request_digest = None;
                    work.delivery_accepted = false;
                    work.status = Status::Ready {
                        prepared_token: token,
                    };
                    work.expires = Some(now + HANDOFF_RESULT_RETENTION);
                    let _ = session.shutdown();
                    work.session = None;
                }
                velune_agent_runtime::native::HandoffSummaryStatus::Failed { code, detail } => {
                    if let Err(error) = settle_terminal_handoff_artifact(
                        &mut self.application.runtime.handoff_forks,
                        session,
                        work_id,
                        &work.source.native_id,
                    ) {
                        tracing::warn!(target: "velune_application", event = "handoff_fork_terminal_receipt_failed", work_id = %work_id, detail = %error);
                    }
                    let _ = session.shutdown();
                    work.session = None;
                    work.fail(work_id, "native_summary", code, detail);
                }
                velune_agent_runtime::native::HandoffSummaryStatus::Cancelled => {
                    if let Err(error) = settle_terminal_handoff_artifact(
                        &mut self.application.runtime.handoff_forks,
                        session,
                        work_id,
                        &work.source.native_id,
                    ) {
                        tracing::warn!(target: "velune_application", event = "handoff_fork_terminal_receipt_failed", work_id = %work_id, detail = %error);
                    }
                    work.status = Status::Cancelled;
                    work.expires = Some(now + HANDOFF_CANCELLED_RETENTION);
                    let _ = session.shutdown();
                    work.session = None;
                }
            }
        }
    }

    fn schedule_handoff_fork_recovery(&mut self) {
        let now = Instant::now();
        if now < self.next_handoff_recovery_check || self.pending_handoff_recoveries.len() >= 2 {
            return;
        }
        self.next_handoff_recovery_check = now + Duration::from_secs(2);
        let plans = match self.application.runtime.handoff_forks.recovery_plans() {
            Ok(plans) => plans,
            Err(error) => {
                tracing::warn!(target: "velune_application", event = "handoff_fork_recovery_unavailable", detail = %error);
                return;
            }
        };
        let active_work_ids = active_handoff_summary_ids(
            &self.pending_handoff_summary_starts,
            &self.handoff_summary_works,
        );
        for plan in plans {
            if self
                .pending_handoff_recoveries
                .values()
                .any(|id| id == &plan.work_id)
                || self.attempted_handoff_recoveries.contains(&plan.work_id)
                || active_work_ids.contains(&plan.work_id)
            {
                continue;
            }
            let history = self
                .application
                .runtime
                .serve(Focus::None, Login::Allows, |core| {
                    core.handoff_recovery_history(&plan.source_runtime_id, &plan.source_home)
                });
            let history = match history {
                Ok(Some(history)) => history,
                Ok(None) => {
                    self.attempted_handoff_recoveries.insert(plan.work_id);
                    continue;
                }
                Err(error) => {
                    tracing::warn!(target: "velune_application", event = "handoff_fork_recovery_config_unavailable", work_id = %plan.work_id, detail = %error);
                    self.attempted_handoff_recoveries.insert(plan.work_id);
                    continue;
                }
            };
            let mut id = self.view_clock.wrapping_add(1);
            if id == 0 {
                id = 1;
            }
            self.view_clock = id;
            let job = HistoryJob {
                completion: self.owner_sender.clone(),
                id,
                work: HistoryWork::RecoverHandoffFork {
                    history,
                    source_native_id: plan.source_native_id,
                    work_id: plan.work_id.clone(),
                    temporary_thread_id: plan.temporary_thread_id,
                    temporary_fingerprint: plan.temporary_fingerprint,
                },
                queued_at: now,
                dispatch: tracing::dispatcher::get_default(Clone::clone),
                span: tracing::Span::current(),
            };
            if self.history.jobs.try_send(job).is_err() {
                self.next_handoff_recovery_check = now + Duration::from_secs(5);
                return;
            }
            self.pending_handoff_recoveries.insert(id, plan.work_id);
            if self.pending_handoff_recoveries.len() >= 2 {
                break;
            }
        }
    }

    fn finish_handoff_fork_recovery(
        &mut self,
        id: u64,
        work_id: String,
        result: Result<velune_agent_runtime::native::HandoffSummaryRecovery, (String, String)>,
    ) {
        self.pending_handoff_recoveries.remove(&id);
        self.attempted_handoff_recoveries.insert(work_id.clone());
        match result {
            Ok(
                velune_agent_runtime::native::HandoffSummaryRecovery::Archived
                | velune_agent_runtime::native::HandoffSummaryRecovery::NotFound,
            ) => {
                if let Err(error) = self.application.runtime.handoff_forks.finish(&work_id) {
                    tracing::warn!(target: "velune_application", event = "handoff_fork_registry_finish_failed", work_id = %work_id, detail = %error);
                }
            }
            Ok(
                velune_agent_runtime::native::HandoffSummaryRecovery::RetainedChanged
                | velune_agent_runtime::native::HandoffSummaryRecovery::RetainedUnverified,
            ) => {
                tracing::warn!(target: "velune_application", event = "handoff_fork_retained", work_id = %work_id);
            }
            Err((code, detail)) => {
                tracing::warn!(target: "velune_application", event = "handoff_fork_recovery_failed", work_id = %work_id, code, detail);
            }
        }
    }

    fn schedule_source_refresh(
        &mut self,
        view: ClientViewId,
        context: Option<ExecutionContext>,
        completion: mpsc::Sender<Job>,
    ) {
        let remote = context.is_some();
        let request = self.with_view(&view, remote, |application| {
            application.runtime.take_source_refresh()
        });
        let Some((id, runtime_id, generation)) = request else {
            return;
        };
        let (reply, receiver) = mpsc::channel();
        drop(receiver);
        self.start_history(StartHistoryRequest {
            view,
            context,
            runtime_id,
            id,
            purpose: HistoryPurpose::FollowSource(generation),
            reply,
            completion,
        });
    }

    fn finish_history(&mut self, job: FinishedHistory) {
        let FinishedHistory {
            id,
            result,
            completion,
            dispatch,
            span,
        } = job;
        let history = match result {
            HistoryResult::Conversation(history) => *history,
            HistoryResult::ListRefreshed(loaded) => {
                tracing::dispatcher::with_default(&dispatch, || {
                    span.in_scope(|| self.finish_list_refresh(*loaded, completion));
                });
                return;
            }
            HistoryResult::CodexIndexWarmed => {
                self.finish_codex_index_warmup(completion);
                return;
            }
            HistoryResult::HandoffRequirements(result) => {
                self.finish_handoff_requirements(id, *result);
                return;
            }
            HistoryResult::HandoffSummaryStarted { work_id, result } => {
                self.finish_handoff_summary_start(id, work_id, *result);
                return;
            }
            HistoryResult::HandoffForkRecovered { work_id, result } => {
                self.finish_handoff_fork_recovery(id, work_id, result);
                return;
            }
            HistoryResult::PreparedHandoffContext { work_id, result } => {
                self.finish_prepared_handoff_context(id, work_id, *result);
                return;
            }
        };
        let Some(pending) = self.pending_history.remove(&id) else {
            return;
        };
        self.application
            .runtime
            .end_history_read(&pending.conversation_id);
        let retry_after_read = matches!(&pending.purpose, HistoryPurpose::Retry(_));
        let pending_invalidated = pending.invalidated;
        let source_view = pending.view.clone();
        let source_context = pending.context.clone();
        let follow_generation = match &pending.purpose {
            HistoryPurpose::FollowSource(generation) => Some(*generation),
            _ => None,
        };
        let history_runtime_id = history.runtime_id().to_owned();
        let history_id = history.id().to_owned();
        let stale_view = self
            .views
            .get(&pending.view)
            .is_none_or(|view| view.created != pending.view_created);
        let result = tracing::dispatcher::with_default(&dispatch, || {
            let _entered = span.enter();
            if stale_view {
                return Err(Failure::new(
                    FailureKind::Conflict,
                    "已切换会话视图，历史读取结果已过期",
                ));
            }
            let remote = pending.context.is_some();
            let is_refresh = matches!(&pending.purpose, HistoryPurpose::Refresh);
            let purpose = pending.purpose;
            let result = self
                .with_view(&pending.view, remote, |application| {
                    application.runtime.serve(
                        Focus::Conversation(history.id().to_owned()),
                        Login::Allows,
                        |core| {
                            if pending_invalidated {
                                let result = core
                                    .read_present_conversation(history.runtime_id(), history.id())?
                                    .ok_or_else(|| {
                                        Error::Contract(Failure::new(
                                            FailureKind::Conflict,
                                            "读取期间会话已改变，请重新打开",
                                        ))
                                    })?;
                                return Ok(result);
                            }
                            if let Some(generation) = follow_generation {
                                core.finish_source_refresh(history, generation)
                            } else if is_refresh {
                                core.finish_conversation_refresh(history)
                            } else {
                                core.finish_conversation_read(history)
                            }
                        },
                    )
                })
                .map_err(Failure::from)?;
            match purpose {
                HistoryPurpose::ReadConversation => self.history_response(result, remote),
                HistoryPurpose::Retry(command) => match *command {
                    // A degraded history can still deliver its readable prefix.
                    // It is intentionally not installed as an execution seed,
                    // so repeating a snapshot query would report it as absent.
                    Command::ReadSnapshot { .. } => {
                        self.response_for(remote, Response::ReadSnapshot(result))
                    }
                    Command::ReadSnapshotUpdate { since, .. } => {
                        let update = match result.snapshot {
                            Some(snapshot) if snapshot.cursor == since => {
                                crate::SnapshotUpdate::Unchanged
                            }
                            Some(snapshot) => crate::SnapshotUpdate::Changed { snapshot },
                            None => crate::SnapshotUpdate::Absent,
                        };
                        self.response_for(remote, Response::ReadSnapshotUpdate(update))
                    }
                    command => self.execute(pending.view, pending.context, command),
                },
                HistoryPurpose::Refresh => {
                    self.response_for(remote, Response::OpenConversation(result))
                }
                HistoryPurpose::FollowSource(generation) => {
                    // The snapshot publication belongs to the ordinary poll;
                    // this worker only updates the retained source projection.
                    let _ = result;
                    let _ = generation;
                    Ok(ApplicationReply {
                        revision: self.revision,
                        response: Response::ReleaseView,
                    })
                }
            }
        });
        if let Some(generation) = follow_generation
            && (stale_view || pending_invalidated || result.is_err())
        {
            self.application.runtime.fail_source_refresh_for(
                &history_runtime_id,
                &history_id,
                generation,
            );
        }
        if retry_after_read {
            self.schedule_source_refresh(source_view, source_context, completion);
        }
        let _ = pending.reply.send(result);
    }

    fn history_response(
        &mut self,
        snapshot: crate::SnapshotResult,
        remote: bool,
    ) -> Result<ApplicationReply, Failure> {
        self.response_for(remote, Response::ReadConversation(snapshot))
    }

    fn response_for(
        &mut self,
        remote: bool,
        response: Response,
    ) -> Result<ApplicationReply, Failure> {
        let response = if remote {
            self.application.remote_response(response, None)?
        } else {
            response
        };
        Ok(ApplicationReply {
            revision: self.revision,
            response,
        })
    }

    fn run(
        &mut self,
        receiver: mpsc::Receiver<Job>,
        telemetry: Option<std::sync::Arc<velune_telemetry::ClientTelemetry>>,
    ) {
        self.schedule_codex_index_warmup();
        self.schedule_handoff_fork_recovery();
        loop {
            match receiver.recv_timeout(Duration::from_millis(40)) {
                Ok(Job::Execute {
                    view,
                    presentation_isolated,
                    context,
                    command,
                    cancellation,
                    queued_at,
                    history_completion,
                    dispatch,
                    span,
                    reply,
                }) => {
                    let dequeued = Instant::now();
                    if command.traits().touch_view {
                        self.touch_client_view(&view, presentation_isolated || context.is_some());
                    }
                    tracing::dispatcher::with_default(&dispatch, || {
                        let _entered = span.enter();
                        // Routes borrow the command; each arm copies what it needs
                        // before taking the command itself.
                        let traits = command.traits();
                        let elapsed_ms = (dequeued - queued_at).as_millis() as u64;
                        let kind = command.kind();
                        if elapsed_ms >= 100 || traits.trace_owner_queue {
                            tracing::info!(target: "velune_application", event = "application_command", phase = "owner_queue", command = ?kind, elapsed_ms);
                        }
                        match traits.route {
                            CommandRoute::Live(route) => {
                                let execution = LiveExecution {
                                    kind: route.kind,
                                    runtime_id: route.address.runtime_id.to_owned(),
                                    conversation_id: route
                                        .address
                                        .conversation_id
                                        .unwrap_or_default()
                                        .to_owned(),
                                };
                                self.execute_live_command(
                                    execution,
                                    view,
                                    context,
                                    *command,
                                    cancellation,
                                    reply,
                                );
                            }
                            CommandRoute::Handoff(route) => match route {
                                HandoffRoute::QueueSummary => {
                                    self.queue_handoff_summary(
                                        view,
                                        context,
                                        *command,
                                        reply,
                                        history_completion.clone(),
                                    );
                                }
                                HandoffRoute::BeginSummary {
                                    source,
                                    target_runtime_id,
                                    summary_model,
                                    effort,
                                } => {
                                    self.begin_handoff_summary(BeginHandoffSummaryRequest {
                                        source_runtime_id: source.runtime_id.to_owned(),
                                        conversation_id: source.conversation_id.to_owned(),
                                        target_runtime_id: target_runtime_id.to_owned(),
                                        summary_model: summary_model.cloned(),
                                        effort: effort.map(str::to_owned),
                                        remote: context.is_some(),
                                        reply,
                                        completion: history_completion.clone(),
                                    });
                                }
                                HandoffRoute::ReadSummary(work_id) => {
                                    self.read_handoff_summary(
                                        work_id.to_owned(),
                                        context.is_some(),
                                        reply,
                                    );
                                }
                                HandoffRoute::CancelSummary(work_id) => {
                                    self.cancel_handoff_summary(
                                        work_id.to_owned(),
                                        context.is_some(),
                                        reply,
                                    );
                                }
                                HandoffRoute::SendPrepared => {
                                    self.send_prepared_handoff(
                                        view,
                                        context,
                                        *command,
                                        cancellation,
                                        reply,
                                        history_completion.clone(),
                                    );
                                }
                                HandoffRoute::ReadRequirements {
                                    source,
                                    target_runtime_id,
                                    request_id,
                                } => {
                                    self.read_handoff_requirements(HandoffRequirementsRequest {
                                        view,
                                        context,
                                        source_runtime_id: source.runtime_id.to_owned(),
                                        conversation_id: source.conversation_id.to_owned(),
                                        target_runtime_id: target_runtime_id.to_owned(),
                                        request_id: request_id.map(str::to_owned),
                                        reply,
                                        completion: history_completion.clone(),
                                    });
                                }
                            },
                            CommandRoute::ListRefresh => {
                                self.start_list_refresh(
                                    view,
                                    context.is_some(),
                                    reply,
                                    history_completion,
                                );
                            }
                            CommandRoute::ReadHistory(address) => {
                                self.start_history(StartHistoryRequest {
                                    view,
                                    context,
                                    runtime_id: address.runtime_id.to_owned(),
                                    id: address.conversation_id.to_owned(),
                                    purpose: HistoryPurpose::ReadConversation,
                                    reply,
                                    completion: history_completion.clone(),
                                });
                            }
                            CommandRoute::HistoryWithSourceRefresh(lookup) => {
                                let (address, refresh) = match lookup {
                                    HistoryLookup::Refresh(address) => (Some(address), true),
                                    HistoryLookup::Retry(address) => (address, false),
                                };
                                let source_completion = history_completion.clone();
                                let source_view = view.clone();
                                let source_context = context.clone();
                                if let Some(address) = address {
                                    let runtime_id = address.runtime_id.to_owned();
                                    let id = address.conversation_id.to_owned();
                                    self.start_history(StartHistoryRequest {
                                        view,
                                        context,
                                        runtime_id,
                                        id,
                                        purpose: if refresh {
                                            HistoryPurpose::Refresh
                                        } else {
                                            HistoryPurpose::Retry(command)
                                        },
                                        reply,
                                        completion: history_completion.clone(),
                                    });
                                } else {
                                    let result = self.execute(view, context, *command);
                                    let _ = reply.send(result);
                                }
                                self.schedule_source_refresh(
                                    source_view,
                                    source_context,
                                    source_completion,
                                );
                            }
                            CommandRoute::Owner {
                                submission_conversation_id,
                            } => {
                                self.application.runtime.submission_cancellation = cancellation
                                    .zip(submission_conversation_id)
                                    .map(|(flag, id)| (id.to_owned(), flag));
                                let result = self.execute(view, context, *command);
                                self.application.runtime.submission_cancellation = None;
                                let _ = reply.send(result);
                            }
                        }
                        let elapsed_ms = dequeued.elapsed().as_millis() as u64;
                        if elapsed_ms >= 100 {
                            tracing::info!(target: "velune_application", event = "application_command", phase = "owner_execute", command = ?kind, elapsed_ms);
                        }
                    });
                    self.application.runtime.publish_execution();
                }
                Ok(Job::HistoryFinished(history)) => self.finish_history(*history),
                Ok(Job::Close { stop_work, reply }) => {
                    let result = if !self.pending_execution.is_empty() {
                        Err(Error::Busy)
                    } else if stop_work {
                        self.application.close_stopping_work()
                    } else {
                        self.application.close()
                    }
                    .map_err(Failure::from);
                    self.application.runtime.publish_execution();
                    let closed = result.is_ok();
                    let _ = reply.send(result);
                    if closed {
                        break;
                    }
                }
                Err(mpsc::RecvTimeoutError::Timeout) => {}
                Err(mpsc::RecvTimeoutError::Disconnected) => break,
            }
            self.expire_observation_leases(Instant::now());
            let mut advance = || {
                let started = Instant::now();
                if self.application.runtime.advance().is_err() {
                    tracing::warn!(target: "velune_application", event = "application_advance_failed");
                }
                self.application.runtime.publish_execution();
                let elapsed_ms = started.elapsed().as_millis() as u64;
                if elapsed_ms >= 100 {
                    tracing::info!(target: "velune_application", event = "application_advance", elapsed_ms);
                }
            };
            if let Some(telemetry) = &telemetry {
                tracing::dispatcher::with_default(&telemetry.dispatch(), advance);
            } else {
                advance();
            }
            self.finish_execution_commands();
            self.advance_handoff_summaries();
            self.advance_queued_handoffs();
            self.schedule_handoff_fork_recovery();
        }
    }
}

impl ApplicationService {
    /// The execution host retains this handle while it intends to keep work alive.
    /// A controller connection owns no Application and dropping it cannot cancel work.
    pub fn open(options: Options) -> Result<Self, Error> {
        Self::open_owner(options, None)
    }

    pub fn open_with_telemetry(
        options: Options,
        telemetry: std::sync::Arc<velune_telemetry::ClientTelemetry>,
    ) -> Result<Self, Error> {
        Self::open_owner(options, Some(telemetry))
    }

    fn open_owner(
        options: Options,
        telemetry: Option<std::sync::Arc<velune_telemetry::ClientTelemetry>>,
    ) -> Result<Self, Error> {
        let (sender, receiver) = mpsc::channel();
        let owner_sender = sender.clone();
        // The runtime owns the sole strong sender. A subscription does not extend
        // the owner's lifetime or prevent listeners from observing its closure.
        let (native_user_messages, _) = broadcast::channel(64);
        let subscription = native_user_messages.downgrade();
        let queued_handoff_cancellations = Arc::new(Mutex::new(BTreeMap::new()));
        let owner_cancellations = queued_handoff_cancellations.clone();
        let (initialized, initialization) = mpsc::sync_channel(1);
        let dispatch = tracing::dispatcher::get_default(Clone::clone);
        std::thread::Builder::new()
            .name("velune-application".into())
            .spawn(move || {
                tracing::dispatcher::with_default(&dispatch, || {
                    let mut application = match Application::open(options) {
                        Ok(application) => application,
                        Err(error) => {
                            let _ = initialized.send(Err(Failure::from(error)));
                            return;
                        }
                    };
                    application.runtime.native_user_messages = Some(native_user_messages);
                    if let Some(owner) = &telemetry {
                        application.runtime.set_telemetry(owner.clone());
                    }
                    let history = match HistoryWorkers::start() {
                        Ok(history) => history,
                        Err(error) => {
                            let _ = initialized.send(Err(Failure::from(Error::from(error))));
                            return;
                        }
                    };
                    let _ = initialized.send(Ok(()));
                    let revision = Revision(
                        std::time::SystemTime::now()
                            .duration_since(std::time::UNIX_EPOCH)
                            .unwrap_or_default()
                            .as_nanos() as u64,
                    );
                    Owner {
                        application,
                        owner_sender,
                        queued_handoff_cancellations: owner_cancellations,
                        views: BTreeMap::new(),
                        view_clock: 0,
                        revision,
                        history,
                        pending_history: BTreeMap::new(),
                        pending_list_refresh: None,
                        pending_execution: BTreeMap::new(),
                        handoff_requirements: BTreeMap::new(),
                        pending_handoff_requirements: BTreeMap::new(),
                        handoff_summary_works: BTreeMap::new(),
                        pending_handoff_summary_starts: BTreeMap::new(),
                        pending_prepared_handoffs: BTreeMap::new(),
                        pending_handoff_recoveries: BTreeMap::new(),
                        attempted_handoff_recoveries: Default::default(),
                        next_handoff_recovery_check: Instant::now(),
                    }
                    .run(receiver, telemetry);
                });
            })?;
        initialization
            .recv()
            .map_err(|_| Error::invalid("application owner initialization failed"))?
            .map_err(Error::Contract)?;
        Ok(Self {
            sender,
            native_user_messages: subscription,
            queued_handoff_cancellations,
        })
    }

    /// Subscribe to local native user-message wakes.
    pub fn native_user_message_events(&self) -> Option<NativeUserMessageEvents> {
        let receiver = self.native_user_messages.upgrade()?.subscribe();
        Some(NativeUserMessageEvents(tokio::sync::Mutex::new(receiver)))
    }

    /// Local, trusted presentation API. Filesystem attachment paths are local to this host.
    pub fn execute(&self, view: ClientViewId, command: Command) -> Result<Response, Failure> {
        self.request(view, None, false, command, None)
            .map(|reply| reply.response)
    }

    /// Local execution with presentation state scoped to this client view.
    /// Command authorization and local attachment semantics remain unchanged.
    pub fn execute_isolated(
        &self,
        view: ClientViewId,
        command: Command,
    ) -> Result<Response, Failure> {
        self.request(view, None, true, command, None)
            .map(|reply| reply.response)
    }

    /// The flag is independent of the owner queue, so a caller can withdraw a
    /// send while its runtime or gateway is still being prepared.
    pub fn execute_cancellable(
        &self,
        view: ClientViewId,
        command: Command,
        cancellation: Arc<AtomicBool>,
    ) -> Result<Response, Failure> {
        self.request(view, None, false, command, Some(cancellation))
            .map(|reply| reply.response)
    }

    /// Cancellable local execution whose presentation state belongs to this view.
    pub fn execute_isolated_cancellable(
        &self,
        view: ClientViewId,
        command: Command,
        cancellation: Arc<AtomicBool>,
    ) -> Result<Response, Failure> {
        self.request(view, None, true, command, Some(cancellation))
            .map(|reply| reply.response)
    }

    /// Paired remote clients must carry the revision of their last observed reply when editing.
    /// Their attachment strings are opaque upload references, never machine-local paths.
    pub fn execute_checked(
        &self,
        view: ClientViewId,
        context: ExecutionContext,
        command: Command,
    ) -> Result<ApplicationReply, Failure> {
        self.request(view, Some(context), true, command, None)
    }

    fn request(
        &self,
        view: ClientViewId,
        context: Option<ExecutionContext>,
        presentation_isolated: bool,
        command: Command,
        cancellation: Option<Arc<AtomicBool>>,
    ) -> Result<ApplicationReply, Failure> {
        if view.0.is_empty() || view.0.len() > 512 || view.0.chars().any(char::is_control) {
            return Err(Failure::invalid("客户端视图标识无效"));
        }
        // Only trusted local callers create this feature. Signalling the
        // existing per-send flag never waits for the owner or does IO.
        if context.is_none()
            && let Ok(cancellations) = self.queued_handoff_cancellations.lock()
        {
            for (work_id, entry) in cancellations.iter() {
                let stop = match &command {
                    Command::CancelHandoffSummary { work_id: id } => id == work_id,
                    Command::Cancel {
                        conversation_id, ..
                    } => conversation_id == &entry.conversation_id,
                    Command::EditSendQueue {
                        conversation_id,
                        operation: velune_application_api::SendQueueOperation::Remove { id },
                        ..
                    } => {
                        conversation_id == &entry.conversation_id && id == &entry.queued_message_id
                    }
                    _ => false,
                };
                if stop {
                    entry.flag.store(true, Ordering::Release);
                }
            }
        }
        let (reply, result) = mpsc::channel();
        self.sender
            .send(Job::Execute {
                view,
                presentation_isolated,
                context,
                // Every execute job carries its completion route. In particular, the handoff
                // commands below may enqueue history work even though they are not ordinary
                // conversation reads; keeping this mandatory avoids a command-list footgun.
                history_completion: self.sender.clone(),
                command: Box::new(command),
                cancellation,
                queued_at: Instant::now(),
                dispatch: tracing::dispatcher::get_default(Clone::clone),
                span: tracing::Span::current(),
                reply,
            })
            .map_err(|_| closed())?;
        result.recv().map_err(|_| closed())?
    }

    /// A busy refusal leaves the service usable. A successful close ends its execution owner.
    pub fn close(&self, stop_work: bool) -> Result<(), Failure> {
        let (reply, result) = mpsc::channel();
        self.sender
            .send(Job::Close { stop_work, reply })
            .map_err(|_| closed())?;
        result.recv().map_err(|_| closed())?
    }
}

fn closed() -> Failure {
    Failure::new(FailureKind::Closed, "应用执行端已关闭")
}

#[cfg(test)]
#[path = "service/handoff_tests.rs"]
mod handoff_tests;

#[cfg(test)]
#[path = "service/observation_tests.rs"]
mod observation_tests;

#[cfg(all(test, unix))]
#[path = "service/execution_tests.rs"]
mod execution_tests;

#[cfg(test)]
mod usage_tests {
    use super::*;
    use crate::config::UsageDisplay::{Remaining, Used};
    use std::fs;
    use velune_test_support::TempRoot;

    struct Fixture(TempRoot);
    impl Fixture {
        fn new() -> Self {
            Self(TempRoot::new("usage"))
        }
        fn open(&self) -> ApplicationService {
            ApplicationService::open(Options {
                home_directory: self.0.to_path_buf(),
                resources_directory: self.0.to_path_buf(),
            })
            .unwrap()
        }
    }
    fn display(
        service: &ApplicationService,
        view: &str,
    ) -> (crate::config::UsageDisplay, crate::config::UsageDisplay) {
        let Response::ReadConfiguration(value) = service
            .execute(ClientViewId(view.into()), Command::ReadConfiguration)
            .unwrap()
        else {
            panic!("list response")
        };
        (value.usage_display, value.context_usage_display)
    }

    fn browser_preferences(
        service: &ApplicationService,
        view: &str,
    ) -> crate::config::ConversationBrowserPreferences {
        let Response::ReadConfiguration(value) = service
            .execute(ClientViewId(view.into()), Command::ReadConfiguration)
            .unwrap()
        else {
            panic!("configuration response")
        };
        value.conversation_browser_preferences
    }

    #[test]
    fn usage_preferences_persist_and_isolate_views() {
        let fixture = Fixture::new();
        let app = fixture.open();
        app.execute(
            ClientViewId("local".into()),
            Command::SetUsageDisplay { display: Remaining },
        )
        .unwrap();
        assert_eq!(display(&app, "local"), (Remaining, Used));
        app.execute(
            ClientViewId("local".into()),
            Command::SetContextUsageDisplay { display: Remaining },
        )
        .unwrap();
        assert_eq!(display(&app, "local"), (Remaining, Remaining));
        assert_eq!(display(&app, "existing"), (Remaining, Remaining));
        app.execute(
            ClientViewId("local".into()),
            Command::SetContextUsageDisplay { display: Used },
        )
        .unwrap();
        assert_eq!(display(&app, "local"), (Remaining, Used));
        assert_eq!(display(&app, "existing"), (Remaining, Remaining));
        assert_eq!(display(&app, "new"), (Remaining, Used));
        app.execute_checked(
            ClientViewId("remote".into()),
            ExecutionContext::default(),
            Command::SetUsageDisplay { display: Used },
        )
        .unwrap();
        assert_eq!(display(&app, "remote"), (Used, Used));
        assert_eq!(display(&app, "local"), (Remaining, Used));
        assert_eq!(display(&app, "after-remote"), (Remaining, Used));
        app.close(false).unwrap();
        let reopened = fixture.open();
        assert_eq!(display(&reopened, "restarted"), (Remaining, Used));
        reopened.close(false).unwrap();
    }

    #[test]
    fn isolated_local_view_preferences_do_not_replace_persisted_defaults() {
        let fixture = Fixture::new();
        let app = fixture.open();
        let main = crate::config::ConversationBrowserPreferences {
            grouping: crate::config::ConversationBrowserGrouping::Project,
            runtime_id: Some("main-runtime".into()),
            ..Default::default()
        };
        let archive = crate::config::ConversationBrowserPreferences {
            grouping: crate::config::ConversationBrowserGrouping::Runtime,
            runtime_id: Some("archive-runtime".into()),
            ..Default::default()
        };
        app.execute(
            ClientViewId("local".into()),
            Command::SetConversationBrowserPreferences {
                preferences: main.clone(),
            },
        )
        .unwrap();
        app.execute_isolated(
            ClientViewId("archive-window".into()),
            Command::SetConversationBrowserPreferences {
                preferences: archive.clone(),
            },
        )
        .unwrap();

        assert_eq!(browser_preferences(&app, "local"), main);
        assert_eq!(
            browser_preferences_isolated(&app, "archive-window"),
            archive
        );
        app.execute(ClientViewId("archive-window".into()), Command::ReleaseView)
            .unwrap();
        assert_eq!(browser_preferences(&app, "local"), main);
        app.close(false).unwrap();

        let reopened = fixture.open();
        assert_eq!(browser_preferences(&reopened, "restarted"), main);
        let updated_main = crate::config::ConversationBrowserPreferences {
            grouping: crate::config::ConversationBrowserGrouping::None,
            runtime_id: Some("new-main-runtime".into()),
            ..Default::default()
        };
        reopened
            .execute(
                ClientViewId("local".into()),
                Command::SetConversationBrowserPreferences {
                    preferences: updated_main.clone(),
                },
            )
            .unwrap();
        reopened.close(false).unwrap();

        let reopened_again = fixture.open();
        assert_eq!(
            browser_preferences(&reopened_again, "restarted"),
            updated_main
        );
        reopened_again.close(false).unwrap();
    }

    fn browser_preferences_isolated(
        service: &ApplicationService,
        view: &str,
    ) -> crate::config::ConversationBrowserPreferences {
        let Response::ReadConfiguration(value) = service
            .execute_isolated(ClientViewId(view.into()), Command::ReadConfiguration)
            .unwrap()
        else {
            panic!("configuration response")
        };
        value.conversation_browser_preferences
    }

    #[test]
    fn failed_context_save_does_not_change_view_or_defaults() {
        let fixture = Fixture::new();
        let app = fixture.open();
        assert_eq!(display(&app, "local"), (Used, Used));
        let path = fixture.0.join("generic-config.json");
        if path.is_file() {
            fs::remove_file(&path).unwrap();
        }
        fs::create_dir(&path).unwrap();
        assert!(
            app.execute(
                ClientViewId("local".into()),
                Command::SetContextUsageDisplay { display: Remaining }
            )
            .is_err()
        );
        assert_eq!(display(&app, "local"), (Used, Used));
        assert_eq!(display(&app, "new"), (Used, Used));
        fs::remove_dir(&path).unwrap();
        app.close(false).unwrap();
    }

    #[test]
    fn handoff_service_commands_keep_owner_alive_when_source_is_missing() {
        let fixture = Fixture::new();
        let app = fixture.open();
        let view = ClientViewId("handoff-invalid-source".into());

        let requirements = app.execute(
            view.clone(),
            Command::ReadHandoffRequirements {
                source_runtime_id: "missing-source".into(),
                conversation_id: "missing-thread".into(),
                target_runtime_id: "pi".into(),
                request_id: None,
            },
        );
        assert!(matches!(
            requirements,
            Ok(Response::ReadHandoffRequirements(_))
        ));

        assert!(
            app.execute(
                view.clone(),
                Command::BeginHandoffSummary {
                    source_runtime_id: "missing-source".into(),
                    conversation_id: "missing-thread".into(),
                    target_runtime_id: "pi".into(),
                    summary_model: None,
                    effort: None,
                },
            )
            .is_err()
        );

        assert!(
            app.execute(
                view.clone(),
                Command::SendPreparedHandoff {
                    source_runtime_id: "missing-source".into(),
                    conversation_id: "missing-thread".into(),
                    target_runtime_id: "pi".into(),
                    selection: None,
                    turn: velune_application_api::ComposedTurn {
                        text: "synthetic request".into(),
                        effort: None,
                        permission_mode: None,
                        attachments: Vec::new(),
                    },
                    prepared_token: "invalid-token".into(),
                },
            )
            .is_err()
        );

        assert!(matches!(
            app.execute(view, Command::ReadConfiguration),
            Ok(Response::ReadConfiguration(_))
        ));
        app.close(false).unwrap();
    }

    #[cfg(unix)]
    #[test]
    fn slow_list_refresh_shares_scan_without_blocking_activity() {
        let fixture = Fixture::new();
        let script = fixture.0.join("agent.py");
        let gate = fixture.0.join("hold-list");
        let reads = fixture.0.join("list-reads");
        fs::write(
            &script,
            r#"
import json, pathlib, sys, time
root = pathlib.Path(sys.argv[1])
for line in sys.stdin:
    message = json.loads(line)
    if message['method'] == 'initialize':
        value = {'protocolVersion':1, 'agentCapabilities':{'sessionCapabilities':{'list':{}}}}
    elif message['method'] == 'session/list':
        with (root / 'list-reads').open('a') as reads: reads.write('list\n')
        while (root / 'hold-list').exists(): time.sleep(.01)
        value = {'sessions':[{'sessionId':sys.argv[2], 'cwd':str(root), 'title':'Synthetic'}]}
    else:
        value = {}
    if 'id' in message:
        print(json.dumps({'jsonrpc':'2.0', 'id':message['id'], 'result':value}), flush=True)
"#,
        )
        .unwrap();
        let app = fixture.open();
        let configure = |session: &str| Command::UpsertAcpRuntime {
            runtime: serde_json::from_value(serde_json::json!({
                "id":"synthetic", "name":"Synthetic", "enabled":true,
                "typeId":"acp-v1", "gatewayId":"default", "settings":{"binary":"/usr/bin/python3"}
            }))
            .unwrap(),
            execution: crate::RuntimeExecution {
                home: crate::RuntimeHome::Existing {
                    path: fixture.0.to_string_lossy().into_owned(),
                },
                ..Default::default()
            },
            launch: crate::AcpLaunch {
                args: vec![
                    script.to_string_lossy().into_owned(),
                    fixture.0.to_string_lossy().into_owned(),
                    session.into(),
                ],
                environment: Default::default(),
            },
        };
        app.execute(ClientViewId("local".into()), configure("thread"))
            .unwrap();
        fs::write(&reads, "").unwrap();
        fs::write(&gate, "hold").unwrap();
        // Submit in order on one sender so activity also proves that the
        // second list request reached its owner before the scan completed.
        let send = |view: &str, remote: bool, command: Command| {
            let (reply, response) = mpsc::channel();
            app.sender
                .send(Job::Execute {
                    view: ClientViewId(view.into()),
                    presentation_isolated: remote,
                    context: remote.then(ExecutionContext::default),
                    command: Box::new(command),
                    cancellation: None,
                    queued_at: Instant::now(),
                    history_completion: app.sender.clone(),
                    dispatch: tracing::dispatcher::get_default(Clone::clone),
                    span: tracing::Span::none(),
                    reply,
                })
                .unwrap();
            response
        };
        let wait_for_scan = || {
            let deadline = Instant::now() + Duration::from_secs(10);
            while fs::metadata(&reads).unwrap().len() == 0 && Instant::now() < deadline {
                std::thread::sleep(Duration::from_millis(10));
            }
            fs::metadata(&reads).unwrap().len() > 0
        };
        let local_list = send("local", false, Command::ReadConfiguration);
        let scan_started = wait_for_scan();
        let remote_list = send("remote", true, Command::ReadConfiguration);
        let activity = send(
            "remote",
            true,
            Command::ReadConversationActivity { since: 0 },
        );
        let prompt = activity.recv_timeout(Duration::from_secs(2));
        fs::remove_file(&gate).unwrap();
        assert!(scan_started, "ACP listing did not enter the gated read");
        assert!(matches!(
            prompt.unwrap().unwrap().response,
            Response::ReadConversationActivity(_)
        ));
        let read_list = |result: mpsc::Receiver<Result<ApplicationReply, Failure>>| {
            let reply = result
                .recv_timeout(Duration::from_secs(10))
                .unwrap()
                .unwrap();
            let Response::ReadConfiguration(value) = reply.response else {
                panic!("expected list response");
            };
            (reply.revision, value)
        };
        for result in [local_list, remote_list] {
            let (_, value) = read_list(result);
            assert!(
                value
                    .conversations
                    .iter()
                    .any(|row| row.native_id == "thread")
            );
            let runtime = value
                .runtime_instances
                .iter()
                .find(|runtime| runtime.id == "synthetic")
                .unwrap();
            assert!(!runtime.settings.contains_key("acpLaunch"));
        }
        assert_eq!(fs::read_to_string(&reads).unwrap().lines().count(), 1);

        fs::write(&reads, "").unwrap();
        fs::write(&gate, "hold").unwrap();
        let list = send("local", false, Command::ReadConfiguration);
        let scan_started = wait_for_scan();
        let updated =
            send("local", false, configure("replacement")).recv_timeout(Duration::from_secs(2));
        fs::remove_file(&gate).unwrap();
        assert!(scan_started);
        let (revision, value) = read_list(list);
        assert_eq!(revision, updated.unwrap().unwrap().revision);
        assert_eq!(value.conversations.len(), 1);
        assert_eq!(value.conversations[0].native_id, "replacement");
        assert_eq!(fs::read_to_string(reads).unwrap().lines().count(), 2);
        app.close(false).unwrap();
    }

    #[cfg(unix)]
    #[test]
    fn slow_cold_history_does_not_hold_owner_and_more_than_eight_tabs_reuse_history() {
        let fixture = Fixture::new();
        let script = fixture.0.join("agent.py");
        let gate = fixture.0.join("hold-s0");
        let started = fixture.0.join("started-s0");
        let loads = fixture.0.join("loads");
        fs::write(&script, r#"
import json, pathlib, sys, time
root = pathlib.Path(sys.argv[1])
def emit(value): print(json.dumps({'jsonrpc':'2.0', **value}), flush=True)
for line in sys.stdin:
    msg = json.loads(line); method = msg.get('method'); req = msg.get('id')
    if method == 'initialize':
        value = {'protocolVersion':1, 'agentCapabilities':{'loadSession':True, 'sessionCapabilities':{'list':{}}}}
    elif method == 'session/list':
        value = {'sessions':[{'sessionId':f's{i}', 'cwd':str(root), 'title':f'S{i}'} for i in range(11)]}
    elif method == 'session/load':
        sid = msg['params']['sessionId']
        with (root / 'loads').open('a') as f: f.write(sid + '\n')
        if sid == 's0' and (root / 'hold-s0').exists():
            (root / 'started-s0').touch()
            while (root / 'hold-s0').exists(): time.sleep(.01)
        value = {}
    else: value = {}
    if req is not None: emit({'id':req, 'result':value})
"#).unwrap();
        let app = fixture.open();
        app.execute(ClientViewId("local".into()), Command::UpsertAcpRuntime {
            runtime: serde_json::from_value(serde_json::json!({
                "id":"synthetic", "name":"Synthetic", "enabled":true,
                "typeId":"acp-v1", "gatewayId":"default", "settings":{"binary":"/usr/bin/python3"}
            })).unwrap(),
            execution: crate::RuntimeExecution {
                home: crate::RuntimeHome::Existing { path: fixture.0.to_string_lossy().into_owned() },
                ..Default::default()
            },
            launch: crate::AcpLaunch {
                args: vec![script.to_string_lossy().into_owned(), fixture.0.to_string_lossy().into_owned()],
                environment: Default::default(),
            },
        }).unwrap();
        fs::write(&gate, "hold").unwrap();
        let read = {
            let app = app.clone();
            std::thread::spawn(move || {
                app.execute(
                    ClientViewId("tab-0".into()),
                    Command::ReadConversation {
                        runtime_id: "synthetic".into(),
                        conversation_id: "synthetic:s0".into(),
                    },
                )
            })
        };
        let deadline = Instant::now() + Duration::from_secs(10);
        while !started.exists() && Instant::now() < deadline {
            std::thread::sleep(Duration::from_millis(10));
        }
        assert!(
            started.exists(),
            "cold history did not reach the synthetic delay"
        );
        let (reply, result) = mpsc::channel();
        let config_read = {
            let app = app.clone();
            std::thread::spawn(move || {
                let value = app.execute(
                    ClientViewId("another-tab".into()),
                    Command::ReadConfiguration,
                );
                let _ = reply.send(value);
            })
        };
        let owner_response = result.recv_timeout(Duration::from_secs(2));
        fs::remove_file(&gate).unwrap();
        assert!(
            owner_response.is_ok(),
            "unrelated owner request stalled behind history IO"
        );
        assert!(read.join().unwrap().is_ok());
        config_read.join().unwrap();
        assert!(matches!(
            app.execute(
                ClientViewId("tab-0".into()),
                Command::SetObservedConversations {
                    conversations: vec![velune_application_api::ObservedConversation {
                        runtime_id: "synthetic".into(),
                        conversation_id: "synthetic:s0".into(),
                    }],
                },
            )
            .unwrap(),
            Response::SetObservedConversations
        ));
        for index in 1..11 {
            let response = app
                .execute(
                    ClientViewId(format!("tab-{index}")),
                    Command::ReadConversation {
                        runtime_id: "synthetic".into(),
                        conversation_id: format!("synthetic:s{index}"),
                    },
                )
                .unwrap();
            assert!(matches!(response, Response::ReadConversation(_)));
        }
        let before = fs::read_to_string(&loads).unwrap().lines().count();
        assert_eq!(before, 11);
        app.execute(
            ClientViewId("tab-0".into()),
            Command::ReadSnapshotUpdate {
                runtime_id: "synthetic".into(),
                conversation_id: "synthetic:s0".into(),
                since: crate::TranscriptCursor {
                    generation: 0,
                    sequence: 0,
                },
            },
        )
        .unwrap();
        let after = fs::read_to_string(&loads).unwrap().lines().count();
        assert_eq!(
            after, before,
            "an observed history projection was reloaded after eight other tabs"
        );
        app.close(false).unwrap();
    }

    #[cfg(unix)]
    #[test]
    fn snapshot_poll_retains_a_linked_prefix_until_the_native_tail_recovers() {
        use crate::{DeliveredBlock, SnapshotUpdate, conversation::RunState};
        use serde_json::json;

        const SOURCE: &str = "00000000-0000-4000-8000-000000000001";
        const TAIL: &str = "00000000-0000-4000-8000-000000000002";
        const PREFIX: &str = "synthetic sealed prefix";
        const RESTORED: &str = "synthetic recovered tail";

        let fixture = Fixture::new();
        let app = fixture.open();
        let view = ClientViewId("linked-history-tab".into());
        let workspace = fixture.0.join("workspace");
        fs::create_dir(&workspace).unwrap();
        let project = workspace
            .to_string_lossy()
            .chars()
            .map(|c| if c.is_ascii_alphanumeric() { c } else { '-' })
            .collect::<String>();
        for runtime in ["source", "tail"] {
            let agent_dir = fixture.0.join(runtime);
            fs::create_dir_all(agent_dir.join("projects").join(&project)).unwrap();
            app.execute(
                view.clone(),
                Command::UpsertRuntimeWithExecution {
                    runtime: serde_json::from_value(json!({
                        "id": runtime, "name": runtime, "enabled": true,
                        "typeId": "claude-code-2.1", "gatewayId": "default",
                        "settings": { "binary": "/usr/bin/false", "agentDir": agent_dir },
                    }))
                    .unwrap(),
                    execution: crate::RuntimeExecution {
                        home: crate::RuntimeHome::Existing {
                            path: fixture.0.to_string_lossy().into_owned(),
                        },
                        ..Default::default()
                    },
                },
            )
            .unwrap();
        }
        app.close(false).unwrap();

        let write_native = |runtime: &str, native_id: &str, text: &str| {
            let path = fixture.0.join(runtime).join("projects").join(&project);
            let record = json!({
                "uuid": format!("user-{native_id}"), "parentUuid": null,
                "sessionId": native_id, "cwd": workspace,
                "type": "user", "message": { "role": "user", "content": text },
            });
            let path = path.join(format!("{native_id}.jsonl"));
            fs::write(&path, format!("{record}\n")).unwrap();
            path
        };
        let source_path = write_native("source", SOURCE, PREFIX);
        let id = format!("source:{SOURCE}");
        let prefix = [(
            crate::conversation::MessageRole::User,
            [crate::conversation::MessageBlock::Text {
                text: PREFIX.into(),
            }],
        )];
        let prefix_digest = format!("{:x}", Sha256::digest(serde_json::to_vec(&prefix).unwrap()));
        let associations = json!({
            "schemaVersion": 3, "links": [{ "id": id, "segments": [
                { "runtimeInstanceId": "source", "runtimeTypeId": "claude-code-2.1",
                  "nativeConversationId": id, "cwd": workspace,
                  "cutoff": { "messageCount": 1, "prefixDigest": prefix_digest }, "deleted": false },
                { "runtimeInstanceId": "tail", "runtimeTypeId": "claude-code-2.1",
                  "nativeConversationId": format!("tail:{TAIL}"), "cwd": workspace, "deleted": false }
            ] }]
        });
        fs::write(
            fixture.0.join("conversation-links.json"),
            associations.to_string(),
        )
        .unwrap();

        let app = fixture.open();
        let execute = |command| app.execute(view.clone(), command).unwrap();
        let Response::ReadConversation(crate::SnapshotResult {
            snapshot: Some(opened),
            ..
        }) = execute(Command::ReadConversation {
            runtime_id: "source".into(),
            conversation_id: id.clone(),
        })
        else {
            panic!("a damaged tail should not prevent opening the sealed prefix");
        };
        assert_eq!(opened.run_state, RunState::Failed);
        assert!(!opened.actions.can_send);
        assert!(opened.messages.iter().any(|message| {
            message
                .blocks
                .iter()
                .any(|block| matches!(block, DeliveredBlock::Text { text } if text == PREFIX))
        }));
        assert!(
            opened
                .message_ids
                .iter()
                .any(|id| id.ends_with(":context-unavailable"))
        );

        let poll = || {
            execute(Command::ReadSnapshotUpdate {
                runtime_id: "source".into(),
                conversation_id: id.clone(),
                since: opened.cursor,
            })
        };
        for _ in 0..2 {
            assert!(matches!(
                poll(),
                Response::ReadSnapshotUpdate(SnapshotUpdate::Unchanged)
            ));
        }
        let Response::ReadSnapshot(crate::SnapshotResult {
            snapshot: Some(full),
            ..
        }) = execute(Command::ReadSnapshot {
            runtime_id: "source".into(),
            conversation_id: id.clone(),
        })
        else {
            panic!("a full poll should keep showing the sealed prefix");
        };
        assert_eq!(full.message_ids, opened.message_ids);
        assert!(!full.actions.can_send);

        let Response::ReadSnapshotUpdate(SnapshotUpdate::Changed {
            snapshot: cold_delta,
        }) = execute(Command::ReadSnapshotUpdate {
            runtime_id: "source".into(),
            conversation_id: id.clone(),
            since: crate::TranscriptCursor {
                generation: 0,
                sequence: 0,
            },
        })
        else {
            panic!("an unknown cursor needs a complete degraded projection");
        };
        assert_eq!(cold_delta.base, None);
        assert_eq!(cold_delta.message_ids, opened.message_ids);

        fs::remove_file(source_path).unwrap();
        assert!(
            app.execute(
                view.clone(),
                Command::ReadSnapshotUpdate {
                    runtime_id: "source".into(),
                    conversation_id: id.clone(),
                    since: opened.cursor,
                }
            )
            .is_err(),
            "an unreadable sealed prefix should remain a read failure"
        );
        write_native("source", SOURCE, PREFIX);
        write_native("tail", TAIL, RESTORED);
        let Response::ReadSnapshotUpdate(SnapshotUpdate::Changed {
            snapshot: recovered,
        }) = poll()
        else {
            panic!("an available tail should replace the read-only projection");
        };
        assert_eq!(recovered.run_state, RunState::Idle);
        assert!(recovered.actions.can_send);
        assert_eq!(recovered.message_ids[0], opened.message_ids[0]);
        assert!(
            !recovered
                .message_ids
                .iter()
                .any(|id| id.ends_with(":context-unavailable"))
        );
        assert!(recovered.messages.iter().any(|message| {
            message
                .blocks
                .iter()
                .any(|block| matches!(block, DeliveredBlock::Text { text } if text == RESTORED))
        }));
        app.close(false).unwrap();
    }

    #[test]
    fn cancelled_submission_is_scoped_to_one_owner_job() {
        let fixture = Fixture::new();
        let app = fixture.open();
        let send = || Command::SendTurn {
            submission_id: None,
            source_runtime_id: "missing".into(),
            conversation_id: "missing:synthetic".into(),
            target_runtime_id: "missing".into(),
            selection: None,
            turn: crate::ComposedTurn {
                text: "synthetic".into(),
                attachments: vec![],
                effort: None,
                permission_mode: None,
            },
        };
        let cancelled = Arc::new(AtomicBool::new(true));
        let error = app
            .execute_cancellable(ClientViewId("local".into()), send(), cancelled)
            .err()
            .unwrap();
        assert_eq!(error.code, "submission_cancelled");
        let error = app
            .execute(ClientViewId("local".into()), send())
            .err()
            .unwrap();
        assert_ne!(error.code, "submission_cancelled");
        app.close(false).unwrap();
    }

    #[cfg(unix)]
    #[test]
    fn cancellation_during_runtime_preparation_never_dispatches_prompt() {
        let fixture = Fixture::new();
        let script = fixture.0.join("agent.py");
        let events = fixture.0.join("events");
        let gate = fixture.0.join("hold");
        fs::write(&script, r#"
import json, os, pathlib, sys, time
root = pathlib.Path(sys.argv[1])
def emit(value): print(json.dumps({'jsonrpc':'2.0', **value}), flush=True)
for line in sys.stdin:
    msg = json.loads(line); method = msg.get('method'); req = msg.get('id')
    with (root / 'events').open('a') as f: f.write(str(method) + '\n')
    if method == 'initialize':
        if (root / 'hold').exists():
            (root / 'preparing').touch()
            while (root / 'hold').exists(): time.sleep(.01)
        value = {'protocolVersion':1, 'agentCapabilities':{'loadSession':True, 'sessionCapabilities':{'list':{}}}}
    elif method == 'session/list': value = {'sessions':[{'sessionId':'synthetic','cwd':str(root),'title':'Synthetic'}]}
    elif method == 'session/load': value = {}
    elif method == 'session/new': value = {'sessionId':'synthetic'}
    elif method == 'session/prompt': value = {'stopReason':'end_turn'}
    else: value = {}
    if req is not None: emit({'id':req, 'result':value})
"#).unwrap();
        let app = fixture.open();
        app.execute(ClientViewId("local".into()), Command::UpsertAcpRuntime {
            runtime: serde_json::from_value(serde_json::json!({
                "id":"synthetic", "name":"Synthetic", "enabled":true, "typeId":"acp-v1", "gatewayId":"default", "settings":{"binary":"/usr/bin/python3"}
            })).unwrap(),
            execution: crate::RuntimeExecution { home: crate::RuntimeHome::Existing { path: fixture.0.to_string_lossy().into_owned() }, ..Default::default() },
            launch: crate::AcpLaunch { args:vec![script.to_string_lossy().into_owned(),fixture.0.to_string_lossy().into_owned()], environment:Default::default() },
        }).unwrap();
        app.execute(
            ClientViewId("local".into()),
            Command::ReadConversation {
                runtime_id: "synthetic".into(),
                conversation_id: "synthetic:synthetic".into(),
            },
        )
        .unwrap();
        fs::write(&gate, "hold").unwrap();
        let cancellation = Arc::new(AtomicBool::new(false));
        let worker = {
            let app = app.clone();
            let cancellation = cancellation.clone();
            std::thread::spawn(move || {
                app.execute_cancellable(
                    ClientViewId("local".into()),
                    Command::SendTurn {
                        submission_id: None,
                        source_runtime_id: "synthetic".into(),
                        conversation_id: "synthetic:synthetic".into(),
                        target_runtime_id: "synthetic".into(),
                        selection: None,
                        turn: crate::ComposedTurn {
                            text: "must not reach runtime".into(),
                            attachments: vec![],
                            effort: None,
                            permission_mode: None,
                        },
                    },
                    cancellation,
                )
            })
        };
        let deadline = Instant::now() + Duration::from_secs(10);
        while !fixture.0.join("preparing").exists()
            && Instant::now() < deadline
            && !worker.is_finished()
        {
            std::thread::sleep(Duration::from_millis(10));
        }
        let preparing = fixture.0.join("preparing").exists();
        cancellation.store(true, Ordering::Release);
        fs::remove_file(gate).unwrap();
        let result = worker.join().unwrap();
        assert!(
            preparing,
            "send did not reach synthetic runtime preparation: {:?}",
            result.as_ref().err()
        );
        assert_eq!(result.err().unwrap().code, "submission_cancelled");
        assert!(
            !fs::read_to_string(events)
                .unwrap()
                .lines()
                .any(|line| line == "session/prompt")
        );
        app.close(false).unwrap();
    }
}

#[cfg(all(test, unix))]
#[path = "service/pi_edit_tests.rs"]
mod pi_edit_tests;
