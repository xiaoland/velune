//! Bounded IO ownership for initialization, terminal cleanup and legacy controls.
//! Co-located Codex commands keep their adapter in the session (native_control).
//! Transferred adapters are destroyed by their worker before its lease ends.
use super::*;
use std::{
    collections::HashMap,
    sync::{Condvar, Mutex, mpsc},
    thread::JoinHandle,
    time::Instant,
};
use velune_agent_runtime::{
    LiveError,
    control::Target,
    creation::{DataRetention, Retained},
    execution::ExecutionTarget,
    work::{BackgroundTask, WorkStatus},
};

#[derive(Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub(crate) struct ExecutionTicket {
    command: u64,
    session: session::SessionKey,
}

pub(super) struct ExecutionCommand {
    id: u64,
    dispatching: bool,
    kind: ExecutionCommandKind,
    session: Option<session::SessionKey>,
    started: Option<ExecutionTicket>,
}

#[derive(Clone, Copy, PartialEq, Eq)]
pub(crate) enum ExecutionCommandKind {
    Create,
    Cancel,
    StopBackground,
    Reply,
    Send,
    Resume,
    Steer,
    Edit,
    Delete,
}

#[derive(Default)]
pub(crate) struct ExecutionSuccess {
    pub(crate) submitted_native_message_id: Option<String>,
}

impl ExecutionCommandKind {
    pub(crate) fn is_submission(self) -> bool {
        matches!(self, Self::Send | Self::Resume | Self::Steer | Self::Edit)
    }

    fn allows(self, name: &str) -> bool {
        matches!(
            (self, name),
            (Self::Create, "native_create")
                | (Self::Cancel, "cancel" | "stop_child")
                | (Self::StopBackground, "stop_background")
                | (Self::Reply, "reply")
                | (Self::Steer, "steer")
        )
    }
}

pub(crate) struct ExecutionReplyAddress {
    session: session::SessionKey,
    source: runtime_source::RuntimeSourceKey,
}

const WORKERS: usize = 4;
const PER_LOCATION: usize = 2;

type Operation = Box<dyn FnOnce(&mut ActiveState) -> OperationOutcome + Send>;

pub(super) struct OperationOutcome {
    pub(super) result: Result<(), RuntimeError>,
    pub(super) pi_edit: Option<editing::PiEditIoValue>,
    // The adapter accepted cancellation; this does not prove the turn ended.
    cancellation_accepted: bool,
    creation: Option<CreationCommit>,
    pub(super) cleanup: bool,
}
struct CreationCommit {
    prepared_resources: Option<String>,
}
impl From<Result<(), RuntimeError>> for OperationOutcome {
    fn from(result: Result<(), RuntimeError>) -> Self {
        Self {
            result,
            pi_edit: None,
            cancellation_accepted: false,
            creation: None,
            cleanup: false,
        }
    }
}
impl OperationOutcome {
    pub(super) fn after_cancellation(result: Result<(), RuntimeError>) -> Self {
        Self {
            result,
            pi_edit: None,
            cancellation_accepted: true,
            creation: None,
            cleanup: false,
        }
    }

    pub(super) fn after_creation(prepared_resources: Option<String>) -> Self {
        Self {
            result: Ok(()),
            pi_edit: None,
            cancellation_accepted: false,
            creation: Some(CreationCommit { prepared_resources }),
            cleanup: false,
        }
    }
}

pub(super) struct ExecutionWorkers {
    sender: Option<mpsc::SyncSender<Job>>,
    locations: Arc<Mutex<HashMap<ExecutionTarget, usize>>>,
    workers: Vec<JoinHandle<()>>,
}

struct Lease {
    locations: Arc<Mutex<HashMap<ExecutionTarget, usize>>>,
    target: ExecutionTarget,
}
impl Drop for Lease {
    fn drop(&mut self) {
        let mut locations = self
            .locations
            .lock()
            .unwrap_or_else(|error| error.into_inner());
        if let Some(count) = locations.get_mut(&self.target) {
            *count -= 1;
            if *count == 0 {
                locations.remove(&self.target);
            }
        }
    }
}

struct Job {
    state: ActiveState,
    operation: Operation,
    completion: Arc<Completion>,
    lease: Lease,
    owned_control: Option<Target>,
    dispatch: tracing::Dispatch,
    span: tracing::Span,
    queued: Instant,
    name: &'static str,
    _completion_guard: CompletionGuard,
}

struct Completed {
    state: Option<ActiveState>,
    outcome: OperationOutcome,
    owned_control: Option<Target>,
    _lease: Lease,
}

#[derive(Default)]
struct Completion {
    state: Mutex<CompletionState>,
    changed: Condvar,
}

#[derive(Default)]
struct CompletionState {
    completed: Option<Completed>,
    abandoned: bool,
    failed: bool,
}

struct CompletionGuard(Arc<Completion>);
impl Drop for CompletionGuard {
    fn drop(&mut self) {
        let mut slot = self
            .0
            .state
            .lock()
            .unwrap_or_else(|error| error.into_inner());
        slot.failed = true;
        self.0.changed.notify_one();
    }
}

pub(super) struct PendingExecution {
    view: FrozenExecution,
    completion: Arc<Completion>,
    name: &'static str,
    ticket: ExecutionTicket,
    owned_control: Option<Target>,
    initialization: Option<runtime_source::RuntimeSourceKey>,
}

struct FrozenExecution {
    snapshot: Option<Retained<crate::conversation::ConversationSnapshot>>,
    work: Result<Retained<Vec<BackgroundTask>>, String>,
    background: Result<Retained<crate::conversation::BackgroundTaskList>, String>,
    turn_running: bool,
    supports_reopen: bool,
}

impl FrozenExecution {
    fn take_history(&mut self) -> ActiveState {
        self.snapshot
            .take()
            .map(|snapshot| ActiveState::History(Box::new(snapshot.into())))
            .unwrap_or_default()
    }

    fn take_failed_history(&mut self) -> ActiveState {
        self.snapshot
            .take()
            .map(|snapshot| {
                let failed = snapshot.map(|mut snapshot| {
                    snapshot.run_state = RunState::Failed;
                    snapshot.executions.finish(None);
                    snapshot
                });
                ActiveState::History(Box::new(failed.into()))
            })
            .unwrap_or_default()
    }

    fn capture(state: &ActiveState, cleanup: bool) -> Result<Self, RuntimeError> {
        let live = state.live();
        let snapshot = match live.map(LiveConversation::owned_snapshot).transpose() {
            Ok(snapshot) => snapshot.flatten(),
            Err(error) if cleanup => {
                // Closing a failed runtime must release its process and location
                // even if no receive credit remains for another history copy.
                tracing::warn!(target: "velune_application", event = "execution_snapshot_credit_refused", detail = %error);
                None
            }
            Err(error) => return Err(RuntimeError::context("运行时执行快照", error)),
        };
        Ok(Self {
            snapshot,
            // Unknown work must remain distinguishable from an observed empty list.
            work: live
                .map_or_else(
                    || Ok(Retained::from_parts(Vec::new(), DataRetention::default())),
                    LiveConversation::work,
                )
                .map_err(|error| error.to_string()),
            background: live
                .map_or_else(
                    || {
                        Ok(Retained::from_parts(
                            Default::default(),
                            DataRetention::default(),
                        ))
                    },
                    LiveConversation::background_tasks,
                )
                .map_err(|error| error.to_string()),
            turn_running: live.is_some_and(LiveConversation::turn_running),
            supports_reopen: live.is_none_or(LiveConversation::supports_reopen),
        })
    }
}

impl Job {
    fn run(mut self) {
        tracing::debug!(target: "velune_application", event = "runtime_operation", phase = "execution_queue", operation = self.name, elapsed_ms = self.queued.elapsed().as_millis() as u64);
        let result = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
            (self.operation)(&mut self.state)
        }));
        let owned_control = self.owned_control.as_ref().filter(|target| {
            matches!(&self.state, ActiveState::Native(native) if native.owns_control(target))
        }).cloned();
        let (state, outcome) = match result {
            Ok(outcome) => (Some(self.state), outcome),
            Err(_) => {
                // Never return an adapter whose mutation unwound halfway through.
                drop(self.state);
                (
                    None,
                    OperationOutcome::from(Err(RuntimeError::operation_indeterminate(
                        "运行时操作异常终止",
                    ))),
                )
            }
        };
        let completed = Completed {
            state,
            outcome,
            owned_control,
            _lease: self.lease,
        };
        let mut slot = self
            .completion
            .state
            .lock()
            .unwrap_or_else(|error| error.into_inner());
        if slot.abandoned {
            drop(slot);
            drop(completed);
            return;
        }
        slot.completed = Some(completed);
        self.completion.changed.notify_all();
        while !slot.abandoned && slot.completed.is_some() {
            slot = self
                .completion
                .changed
                .wait(slot)
                .unwrap_or_else(|error| error.into_inner());
        }
        let abandoned = slot.completed.take();
        drop(slot);
        // A dropped view/service cannot move native destruction onto ingress.
        drop(abandoned);
    }
}

impl ExecutionWorkers {
    pub(super) fn start() -> std::io::Result<Self> {
        let (sender, receiver) = mpsc::sync_channel::<Job>(WORKERS);
        let receiver = Arc::new(Mutex::new(receiver));
        let mut pool = Self {
            sender: Some(sender),
            locations: Arc::new(Mutex::new(HashMap::new())),
            workers: Vec::new(),
        };
        for index in 0..WORKERS {
            let receiver = receiver.clone();
            pool.workers.push(
                std::thread::Builder::new()
                    .name(format!("velune-execution-{index}"))
                    .spawn(move || {
                        loop {
                            let job = receiver
                                .lock()
                                .unwrap_or_else(|error| error.into_inner())
                                .recv();
                            let Ok(job) = job else { break };
                            let dispatch = job.dispatch.clone();
                            let span = job.span.clone();
                            tracing::dispatcher::with_default(&dispatch, || {
                                let _entered = span.enter();
                                job.run();
                            });
                        }
                    })?,
            );
        }
        Ok(pool)
    }

    pub(super) fn shutdown(&mut self) {
        self.sender.take();
        for worker in self.workers.drain(..) {
            if worker.join().is_err() {
                tracing::error!(target: "velune_application", event = "execution_worker_panicked");
            }
        }
    }

    /// Failed admission returns the original adapter without running anything.
    pub(super) fn submit(
        &self,
        state: ActiveState,
        target: ExecutionTarget,
        ticket: ExecutionTicket,
        owned_control: Option<Target>,
        name: &'static str,
        operation: impl FnOnce(&mut ActiveState) -> OperationOutcome + Send + 'static,
    ) -> Result<PendingExecution, (ActiveState, RuntimeError)> {
        let Some(sender) = &self.sender else {
            return Err((
                state,
                RuntimeError::invalid("运行时执行端已关闭；操作尚未开始"),
            ));
        };
        let mut locations = self
            .locations
            .lock()
            .unwrap_or_else(|error| error.into_inner());
        if locations.values().sum::<usize>() >= WORKERS
            || locations.get(&target).copied().unwrap_or_default() >= PER_LOCATION
        {
            return Err((
                state,
                RuntimeError::invalid("运行时执行名额繁忙；操作尚未开始"),
            ));
        }
        *locations.entry(target.clone()).or_default() += 1;
        drop(locations);
        let lease = Lease {
            locations: self.locations.clone(),
            target,
        };
        let view = match FrozenExecution::capture(&state, name == "native_cleanup") {
            Ok(view) => view,
            Err(error) => return Err((state, error)),
        };
        let completion = Arc::new(Completion::default());
        let job = Job {
            state,
            operation: Box::new(operation),
            completion: completion.clone(),
            lease,
            owned_control: owned_control.clone(),
            dispatch: tracing::dispatcher::get_default(Clone::clone),
            span: tracing::Span::current(),
            queued: Instant::now(),
            name,
            _completion_guard: CompletionGuard(completion.clone()),
        };
        match sender.try_send(job) {
            Ok(()) => Ok(PendingExecution {
                view,
                completion,
                name,
                ticket,
                owned_control,
                initialization: None,
            }),
            Err(error) => {
                let job = match error {
                    mpsc::TrySendError::Full(job) | mpsc::TrySendError::Disconnected(job) => job,
                };
                Err((
                    job.state,
                    RuntimeError::invalid("运行时执行队列不可用；操作尚未开始"),
                ))
            }
        }
    }
}

impl Drop for ExecutionWorkers {
    fn drop(&mut self) {
        self.shutdown();
    }
}

impl PendingExecution {
    pub(super) fn mark_initialization(&mut self, source: runtime_source::RuntimeSourceKey) {
        self.initialization = Some(source);
    }
    pub(super) fn initialization_source(&self) -> Option<&runtime_source::RuntimeSourceKey> {
        self.initialization.as_ref()
    }
    pub(super) fn discard_changed_initialization(&self) -> (ExecutionTicket, RuntimeError) {
        let completed = self
            .completion
            .state
            .lock()
            .unwrap_or_else(|e| e.into_inner());
        let created = completed
            .completed
            .as_ref()
            .is_some_and(|result| result.outcome.creation.is_some());
        drop(completed);
        self.abandon();
        let error = if created {
            RuntimeError::operation_completed("会话已创建，但原请求的运行时来源已改变")
        } else {
            RuntimeError::operation_indeterminate(
                "创建期间运行时来源已改变；请先查看原运行时，不要重复创建",
            )
        };
        (self.ticket, error)
    }

    pub(super) fn live(&self) -> &dyn LiveConversation {
        &self.view
    }

    pub(super) fn owns_control(&self, target: &Target) -> bool {
        let slot = self
            .completion
            .state
            .lock()
            .unwrap_or_else(|error| error.into_inner());
        if let Some(completed) = &slot.completed {
            completed.owned_control.as_ref() == Some(target)
        } else {
            !slot.failed && !slot.abandoned && self.owned_control.as_ref() == Some(target)
        }
    }

    pub(super) fn abandon(&self) {
        let mut slot = self
            .completion
            .state
            .lock()
            .unwrap_or_else(|error| error.into_inner());
        slot.abandoned = true;
        self.completion.changed.notify_one();
    }

    /// After abandonment, the guard drops only after the worker has destroyed
    /// the actual returned adapter and output and released its location lease.
    pub(super) fn abandoned_cleanup_complete(&self) -> bool {
        let slot = self
            .completion
            .state
            .lock()
            .unwrap_or_else(|error| error.into_inner());
        slot.abandoned && slot.failed
    }

    pub(super) fn complete(&mut self) -> Option<(ExecutionTicket, ActiveState, OperationOutcome)> {
        let mut slot = self
            .completion
            .state
            .lock()
            .unwrap_or_else(|error| error.into_inner());
        let completed = slot.completed.take();
        let failed = slot.failed;
        drop(slot);
        let Some(completed) = completed else {
            return failed.then(|| {
                (
                    self.ticket,
                    self.view.take_failed_history(),
                    OperationOutcome {
                        cleanup: true,
                        ..OperationOutcome::from(Err(RuntimeError::operation_indeterminate(
                            "运行时执行者未返回确认",
                        )))
                    },
                )
            });
        };
        self.completion.changed.notify_one();
        tracing::debug!(target: "velune_application", event = "runtime_operation", phase = "execution_complete", operation = self.name);
        let mut outcome = completed.outcome;
        outcome.cleanup |= self.name == "native_cleanup" || completed.state.is_none();
        let state = match completed.state {
            Some(ActiveState::Empty) if outcome.cleanup => self.view.take_history(),
            Some(state) => state,
            None => self.view.take_failed_history(),
        };
        Some((self.ticket, state, outcome))
    }
}

impl Drop for PendingExecution {
    fn drop(&mut self) {
        self.abandon();
    }
}

impl LiveConversation for FrozenExecution {
    fn snapshot(&self) -> Option<&crate::conversation::ConversationSnapshot> {
        self.snapshot.as_deref()
    }
    fn owned_snapshot(
        &self,
    ) -> Result<Option<Retained<crate::conversation::ConversationSnapshot>>, LiveError> {
        Ok(self.snapshot.as_ref().map(Retained::try_clone).transpose()?)
    }
    fn turn_running(&self) -> bool {
        self.turn_running
    }
    fn supports_reopen(&self) -> bool {
        self.supports_reopen
    }
    fn work(&self) -> Result<Retained<Vec<BackgroundTask>>, LiveError> {
        let work = self
            .work
            .as_ref()
            .map_err(|error| -> LiveError { error.as_str().into() })?;
        Ok(work.try_clone()?)
    }
    fn background_tasks(&self) -> Result<Retained<crate::conversation::BackgroundTaskList>, LiveError> {
        let tasks = self
            .background
            .as_ref()
            .map_err(|error| -> LiveError { error.as_str().into() })?;
        Ok(tasks.try_clone()?)
    }
    fn work_status(&self) -> WorkStatus {
        WorkStatus::Settling
    }
    fn has_pending_metadata(&self) -> bool {
        true
    }
    fn set_conversation(&mut self, _: Retained<ConversationSummary>) {}
    fn poll(&mut self) -> pi::LiveResult {
        Ok(())
    }
    fn send(&mut self, _: &pi::TurnInput) -> pi::LiveResult {
        Err("运行时操作尚未完成".into())
    }
    fn cancel(&mut self) -> pi::LiveResult {
        Err("运行时操作尚未完成".into())
    }
    fn shutdown(&mut self) -> pi::LiveResult {
        Err("运行时操作尚未完成".into())
    }
}

impl CoreRuntime {
    pub(super) fn execution_control_ticket(
        &self,
        kind: ExecutionCommandKind,
    ) -> Result<Option<ExecutionTicket>, RuntimeError> {
        let Some(command) = self.execution_command.as_ref().filter(|command| {
            command.dispatching
                && command.kind == kind
                && (!kind.is_submission() || command.session == Some(self.session.key))
        }) else {
            return Ok(None);
        };
        if command.started.is_some() {
            return Err(RuntimeError::Busy);
        }
        Ok(Some(ExecutionTicket {
            command: command.id,
            session: self.session.key,
        }))
    }

    pub(super) fn mark_execution_started(&mut self, ticket: ExecutionTicket) {
        self.execution_command
            .as_mut()
            .expect("command scope retained")
            .started = Some(ticket);
    }

    pub(super) fn start_native_cleanup_worker(&mut self) -> Result<(), RuntimeError> {
        if matches!(self.session.active_state, ActiveState::Pi(_))
            && (self.session.pi_edit.is_some()
                || self.session.pi_control.is_some()
                || self.session.pi_submission.is_some())
        {
            // Keep both business fact channels reachable until their owner has
            // harvested ready outcomes. Failed protocol tickets settle on poll.
            return Err(RuntimeError::Busy);
        }
        let (target, _) = self.runtime_location(self.active_instance()?)?;
        let ticket = ExecutionTicket {
            command: 0,
            session: self.session.key,
        };
        #[cfg(all(test, unix))]
        let cleanup_binary = self.active_instance()?.settings.get("binary").cloned();
        let state = std::mem::take(&mut self.session.active_state);
        match self.execution_workers.submit(
            state,
            target,
            ticket,
            None,
            "native_cleanup",
            move |state| {
                #[cfg(all(test, unix))]
                if let Some((entered, release)) = cleanup_test_gate::take(cleanup_binary) {
                    let _ = entered.send(());
                    let _ = release.recv();
                }
                // Preserve already-received Pi facts before terminal destruction;
                // its failed fence forbids any new protocol dispatch.
                if let ActiveState::Pi(pi) = state {
                    pi.poll();
                }
                let snapshot = match state.live().map(LiveConversation::owned_snapshot).transpose() {
                    Ok(snapshot) => snapshot.flatten(),
                    Err(error) => {
                        tracing::warn!(target: "velune_application", event = "execution_cleanup_snapshot_credit_refused", detail = %error);
                        None
                    }
                };
                let result = state.live_mut().map_or(Ok(()), |live| {
                    live.shutdown()
                        .map_err(|error| RuntimeError::context("运行时终态清理", error))
                });
                // Destruction, including the protocol writer/process tree, is part
                // of this worker's lifecycle and precedes lease release.
                *state = snapshot
                    .map(|snapshot| ActiveState::History(Box::new(snapshot.into())))
                    .unwrap_or_default();
                OperationOutcome {
                    result,
                    pi_edit: None,
                    cancellation_accepted: false,
                    creation: None,
                    cleanup: true,
                }
            },
        ) {
            Ok(pending) => {
                self.session.active_state = ActiveState::Pending(Box::new(pending));
                Ok(())
            }
            Err((state, error)) => {
                self.session.active_state = state;
                Err(error)
            }
        }
    }

    pub(super) fn dispatches_native_creation(&self) -> bool {
        self.execution_command.as_ref().is_some_and(|command| {
            command.dispatching && command.kind == ExecutionCommandKind::Create
        })
    }

    pub(super) fn submission_scope(&self, kind: ExecutionCommandKind) -> bool {
        self.execution_command.as_ref().is_some_and(|command| {
            command.dispatching
                && command.kind == kind
                && command.started.is_none()
                && command.session == Some(self.session.key)
        })
    }

    pub(super) fn execution_submission_started(&self) -> bool {
        self.execution_command
            .as_ref()
            .is_some_and(|command| command.kind.is_submission() && command.started.is_some())
    }

    pub(crate) fn begin_execution_command(
        &mut self,
        kind: ExecutionCommandKind,
    ) -> Result<(), RuntimeError> {
        if self.execution_command.is_some() {
            return Err(RuntimeError::Busy);
        }
        static NEXT: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(1);
        let id = NEXT
            .try_update(
                std::sync::atomic::Ordering::Relaxed,
                std::sync::atomic::Ordering::Relaxed,
                |id| id.checked_add(1),
            )
            .map_err(|_| RuntimeError::invalid("运行时操作序列已耗尽；操作尚未开始"))?;
        self.execution_command = Some(ExecutionCommand {
            id,
            dispatching: false,
            kind,
            session: None,
            started: None,
        });
        Ok(())
    }

    pub(crate) fn serve_live_control<T>(
        &mut self,
        focus: Focus,
        run: impl FnOnce(&mut Self) -> Result<T, RuntimeError>,
    ) -> Result<T, RuntimeError> {
        self.prepare(focus, Login::Blocks)?;
        if let Some(command) = &mut self.execution_command {
            command.dispatching = true;
            command.session = Some(self.session.key);
        }
        let result = run(self);
        if let Some(command) = &mut self.execution_command {
            command.dispatching = false;
        }
        self.trim_sessions();
        result
    }

    pub(crate) fn finish_execution_command(&mut self) -> Option<ExecutionTicket> {
        self.execution_command
            .take()
            .and_then(|command| command.started)
    }

    pub(crate) fn drain_execution_results(
        &mut self,
    ) -> BTreeMap<ExecutionTicket, Result<ExecutionSuccess, RuntimeError>> {
        std::mem::take(&mut self.execution_results)
    }

    pub(crate) fn has_pending_execution(&self) -> bool {
        std::iter::once(&self.session)
            .chain(&self.parked)
            .any(|session| {
                matches!(session.active_state, ActiveState::Pending(_))
                    || session.native_controls.pending()
                    || session.claude_submission.is_some()
                    || session.pi_submission.is_some()
                    || session.pi_edit.is_some()
                    || session.pi_control.is_some()
                    || session.pi_io_cleanup.is_some()
            })
    }

    pub(super) fn shutdown_execution_workers(&mut self) {
        // A captured Pi IO job can hold the very capacity terminal cleanup
        // needs. Abandon it before retrying cleanup; its worker owns destruction.
        for session in std::iter::once(&mut self.session).chain(&mut self.parked) {
            session.pi_edit.take();
            session.pi_control.take();
            session.pi_io_cleanup.take();
        }
        // An error barrier may have met a saturated pool. At this explicit
        // teardown boundary wait for admission rather than dropping its native
        // owner (and locks) on the application thread.
        loop {
            let retained = std::iter::once(&self.session)
                .chain(&self.parked)
                .find(|session| {
                    (session.native_controls.cleanup_required
                        && matches!(session.active_state, ActiveState::Native(_)))
                        || (session.pi_cleanup_required
                            && matches!(session.active_state, ActiveState::Pi(_)))
                })
                .map(|session| session.key);
            let Some(key) = retained else { break };
            for session in std::iter::once(&self.session).chain(&self.parked) {
                if let ActiveState::Pending(pending) = &session.active_state {
                    pending.abandon();
                }
            }
            self.focus_key(key);
            let admitted = if self.session.pi_cleanup_required {
                self.start_native_cleanup_worker()
            } else {
                self.schedule_native_cleanup()
            };
            if admitted.is_err() {
                std::thread::sleep(std::time::Duration::from_millis(1));
            }
        }
        for session in std::iter::once(&self.session).chain(&self.parked) {
            if let ActiveState::Pending(pending) = &session.active_state {
                pending.abandon();
            }
        }
        self.execution_workers.shutdown();
    }

    pub(crate) fn execution_reply_address(
        &self,
        runtime_id: &str,
        conversation_id: &str,
    ) -> Result<ExecutionReplyAddress, RuntimeError> {
        let session = std::iter::once(&self.session)
            .chain(&self.parked)
            .find(|session| session.presents(conversation_id))
            .ok_or_else(|| RuntimeError::invalid("原操作的会话已关闭"))?;
        let runtime_id = self
            .conversation_read_address(runtime_id, conversation_id)?
            .context_runtime_id;
        let runtime = session
            .execution_configuration
            .as_ref()
            .filter(|configuration| configuration.runtime.id == runtime_id)
            .map(|configuration| &configuration.runtime)
            .map_or_else(|| self.runtime_instance(&runtime_id), Ok)?;
        Ok(ExecutionReplyAddress {
            session: session.key,
            source: self.runtime_source_key(runtime)?,
        })
    }

    pub(crate) fn execution_creation_address(
        &self,
        ticket: ExecutionTicket,
    ) -> Result<ExecutionReplyAddress, RuntimeError> {
        let session = std::iter::once(&self.session)
            .chain(&self.parked)
            .find(|session| session.key == ticket.session)
            .ok_or_else(|| RuntimeError::invalid("原创建会话已关闭"))?;
        let runtime = &session
            .execution_configuration
            .as_ref()
            .ok_or_else(|| RuntimeError::invalid("原创建配置已改变"))?
            .runtime;
        Ok(ExecutionReplyAddress {
            session: session.key,
            source: self.runtime_source_key(runtime)?,
        })
    }

    pub(crate) fn completed_creation_snapshot(
        &mut self,
        address: ExecutionReplyAddress,
    ) -> Result<crate::SnapshotResult, RuntimeError> {
        if !address.source.is_current(self)
            || !std::iter::once(&self.session)
                .chain(&self.parked)
                .any(|session| {
                    session.key == address.session
                        && session.projected().is_some_and(|snapshot| {
                            snapshot.context_runtime_id == address.source.runtime_id()
                        })
                })
        {
            return Err(RuntimeError::invalid("已创建会话的来源或身份已改变"));
        }
        let previous = self.session.key;
        self.focus_key(address.session);
        let result = self.snapshot_result();
        self.focus_key(previous);
        result
    }

    pub(crate) fn completed_execution_snapshot(
        &mut self,
        address: ExecutionReplyAddress,
        runtime_id: &str,
        conversation_id: &str,
        submitted_native_message_id: Option<String>,
    ) -> Result<crate::SnapshotResult, RuntimeError> {
        if !std::iter::once(&self.session)
            .chain(&self.parked)
            .any(|session| session.key == address.session && session.presents(conversation_id))
            || !address.source.is_current(self)
        {
            return Err(RuntimeError::invalid("原操作的会话或来源已改变"));
        }
        let previous = self.session.key;
        self.focus_key(address.session);
        let result =
            (|| {
                let address = self.conversation_read_address(runtime_id, conversation_id)?;
                if !self.session.projected().is_some_and(|snapshot| {
                    snapshot.context_runtime_id == address.context_runtime_id
                }) {
                    return Err(RuntimeError::invalid("原操作的会话来源已改变"));
                }
                // Completion refresh must not undo navigation performed while waiting.
                let submitted = submitted_native_message_id.and_then(|id| {
                    self.session.projected().map(|native| {
                        self.projected_conversation_message_id(&native.conversation.id, &id)
                    })
                });
                let mut result = self.snapshot_result()?;
                if let (Some(id), Some(snapshot)) = (submitted, &result.snapshot) {
                    let id = snapshot
                        .message_identity_confirmations
                        .iter()
                        .find(|confirmation| confirmation.previous_id == id)
                        .map_or(id.as_str(), |confirmation| &confirmation.current_id);
                    result.submitted_message_id = snapshot
                        .messages
                        .iter()
                        .find(|message| message.id == id)
                        .map(|message| message.id.clone());
                }
                Ok(result)
            })();
        self.focus_key(previous);
        result
    }

    pub(super) fn apply_execution_outcome(
        &mut self,
        outcome: OperationOutcome,
    ) -> Result<(), RuntimeError> {
        if outcome.cleanup {
            self.finish_managed_backend_turn();
            if outcome.result.is_err() {
                self.session.send_queue.pause_after_stop();
                // The scheduled-run sweep will settle this orphan as unknown.
                // A later, explicitly submitted turn must not settle it.
                self.session.scheduled_run = None;
            }
            self.session.native_controls = Default::default();
            self.session.pi_cleanup_required = false;
            self.session.execution_runtime_id = None;
            self.session.execution_configuration = None;
            self.session.configuration_changed = false;
        }
        if let Some(creation) = outcome.creation {
            self.finish_native_creation(creation.prepared_resources)?;
        }
        if outcome.cancellation_accepted {
            self.session.cancel_requested = true;
            if let Some((gateway_id, session)) = self.backend_session_identity() {
                self.session_backends()
                    .cancel_session(&gateway_id, &session);
            }
        }
        outcome.result
    }

    pub(super) fn start_live_operation(
        &mut self,
        name: &'static str,
        operation: impl FnOnce(&mut ActiveState) -> Result<(), RuntimeError> + Send + 'static,
    ) -> Result<(), RuntimeError> {
        self.start_live_operation_with_outcome(name, move |state| operation(state).into())
    }

    pub(super) fn start_live_operation_with_outcome(
        &mut self,
        name: &'static str,
        operation: impl FnOnce(&mut ActiveState) -> OperationOutcome + Send + 'static,
    ) -> Result<(), RuntimeError> {
        if matches!(self.session.active_state, ActiveState::Pending(_))
            || self.session.native_controls.pending()
        {
            return Err(RuntimeError::Busy);
        }
        // Direct in-process use retains synchronous completion. Only a service
        // command with a delayed reply route may transfer adapter ownership.
        let Some(command) = self
            .execution_command
            .as_ref()
            .filter(|command| command.dispatching && command.kind.allows(name))
        else {
            if self.session.native_controls.pending() {
                return Err(RuntimeError::Busy);
            }
            let outcome = operation(&mut self.session.active_state);
            return self.apply_execution_outcome(outcome);
        };
        if command.started.is_some() {
            return Err(RuntimeError::Busy);
        }
        let ticket = ExecutionTicket {
            command: command.id,
            session: self.session.key,
        };
        let runtime = self.active_instance()?;
        let (target, _) = self.runtime_location(runtime)?;
        let owned_control = self.control_target().ok().flatten().filter(|target| {
            matches!(&self.session.active_state, ActiveState::Native(native) if native.owns_control(target))
        });
        let state = std::mem::take(&mut self.session.active_state);
        match self
            .execution_workers
            .submit(state, target, ticket, owned_control, name, operation)
        {
            Ok(pending) => {
                self.execution_command
                    .as_mut()
                    .expect("command scope retained")
                    .started = Some(ticket);
                self.session.active_state = ActiveState::Pending(Box::new(pending));
                Ok(())
            }
            Err((state, error)) => {
                self.session.active_state = state;
                Err(error)
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::time::Duration;

    fn submit(
        workers: &ExecutionWorkers,
        target: ExecutionTarget,
        command: u64,
        operation: impl FnOnce(&mut ActiveState) -> Result<(), RuntimeError> + Send + 'static,
    ) -> PendingExecution {
        workers
            .submit(
                ActiveState::Empty,
                target,
                ExecutionTicket {
                    command,
                    session: session::SessionKey::default(),
                },
                None,
                "synthetic",
                move |state| operation(state).into(),
            )
            .unwrap_or_else(|(_, error)| panic!("synthetic admission: {error}"))
    }

    fn wait_completed(pending: &PendingExecution) {
        let slot = pending.completion.state.lock().unwrap();
        let (slot, timeout) = pending
            .completion
            .changed
            .wait_timeout_while(slot, Duration::from_secs(2), |slot| {
                slot.completed.is_none() && !slot.failed
            })
            .unwrap();
        assert!(!timeout.timed_out());
        assert!(slot.completed.is_some());
    }

    #[cfg(unix)]
    #[test]
    fn recovery_worker_loss_rechecks_native_source_without_replaying_work() {
        use std::os::unix::fs::PermissionsExt;
        use velune_conversation::{ExecutionKind, InputAccess};

        let home = std::env::temp_dir().join(format!("velune-recovery-{}", crate::new_record_key()));
        fs::create_dir(&home).unwrap();
        let home = home.canonicalize().unwrap();
        let binary = home.join("claude");
        fs::write(&binary, r#"#!/usr/bin/python3
import json, sys
if "--version" in sys.argv:
    print("2.1.286 (Claude Code)")
    sys.exit(0)
for line in sys.stdin:
    record = json.loads(line)
    assert record["request"]["subtype"] == "initialize"
    print(json.dumps({"type": "control_response", "response": {
        "subtype": "success", "request_id": record["request_id"], "response": {"models": []},
    }}), flush=True)
"#).unwrap();
        fs::set_permissions(&binary, fs::Permissions::from_mode(0o700)).unwrap();
        let mut application = crate::Application::open(crate::Options {
            home_directory: home.clone(),
            resources_directory: home.clone(),
        })
        .unwrap();
        let core = &mut application.runtime;
        let runtime = RuntimeInstance {
            enabled: true,
            id: "claude".into(),
            name: "Claude".into(),
            type_id: RuntimeKind::ClaudeCode,
            gateway_id: "default".into(),
            settings: [
                ("binary".into(), binary.display().to_string()),
                ("agentDir".into(), home.display().to_string()),
            ]
            .into(),
        };
        for (child, source_changed) in [(false, false), (true, false), (false, true)] {
            core.session.send_queue = Default::default();
            core.runtime_instances = vec![runtime.clone()];
            let mut native = NativeSession::connect(core.native_config(&runtime, None).unwrap()).unwrap();
            let created = native.create(Some(&home), "claude").unwrap();
            let id = created.conversation.id.clone();
            if child {
                let mut summary = created.conversation.clone();
                summary.parent_conversation_id = Some("claude:parent".into());
                native.set_conversation(Retained::from_parts(summary, DataRetention::default()));
            }
            core.queue_after_work(crate::local::TurnRequest {
                submission_id: None,
                source_runtime_id: runtime.id.clone(),
                conversation_id: id.clone(),
                runtime_id: runtime.id.clone(),
                selection: None,
                turn: crate::ComposedTurn {
                    text: "still queued".into(),
                    effort: None,
                    permission_mode: None,
                    attachments: Vec::new(),
                },
            });
            core.session.execution_runtime_id = Some(runtime.id.clone());
            core.capture_execution_configuration(&runtime, None);
            let ticket = ExecutionTicket { command: 1, session: core.session.key };
            let pending = core.execution_workers.submit(
                ActiveState::Native(Box::new(native)),
                ExecutionTarget::Local,
                ticket,
                None,
                "synthetic execution loss",
                |_| panic!("synthetic execution worker unwind"),
            ).unwrap_or_else(|(_, error)| panic!("worker admission: {error}"));
            if source_changed {
                let other = home.join("other-source");
                fs::create_dir(&other).unwrap();
                core.runtime_instances[0].settings.insert("agentDir".into(), other.display().to_string());
            }
            wait_completed(&pending);
            core.session.active_state = ActiveState::Pending(Box::new(pending));
            core.session.scheduled_run = Some(("interrupted-schedule".into(), false));
            core.drain_runtime().unwrap();
            let failure = core.execution_results.remove(&ticket).unwrap().err().unwrap();
            assert_eq!(crate::Failure::from(failure).code, "runtime_operation_indeterminate");
            let view = core.current_snapshot().unwrap();
            assert_eq!(view.conversation.id, id);
            assert_eq!(view.run_state, RunState::Failed);
            assert_eq!(view.actions.can_send, !child && !source_changed);
            assert_eq!(view.actions.input_access == InputAccess::ReadOnly, child || source_changed);
            assert!(!view.actions.can_cancel);
            assert!(view.follow_up.paused.is_some());
            assert_eq!(view.follow_up.queue[0].text, "still queued");
            assert!(!view.executions.records.iter().any(|record| record.kind == ExecutionKind::UserTurn));
            assert!(core.session.scheduled_run.is_none());
            core.invalidate_execution().unwrap();
            assert_eq!(core.current_snapshot().unwrap().actions.can_send, !child && !source_changed);
        }
        drop(application);
        fs::remove_dir_all(home).unwrap();
    }

    #[test]
    fn completed_operations_retain_location_capacity_until_consumed() {
        let mut workers = ExecutionWorkers::start().unwrap();
        let mut first = submit(&workers, ExecutionTarget::Local, 1, |_| Ok(()));
        let second = submit(&workers, ExecutionTarget::Local, 2, |_| Ok(()));
        wait_completed(&first);
        wait_completed(&second);
        assert!(
            workers
                .submit(
                    ActiveState::Empty,
                    ExecutionTarget::Local,
                    first.ticket,
                    None,
                    "rejected",
                    |_| Ok(()).into(),
                )
                .is_err()
        );
        let remote = ExecutionTarget::Ssh(velune_agent_runtime::execution::SshTarget {
            binary: "/synthetic/not-executed".into(),
            destination: "synthetic".into(),
            port: None,
        });
        let third = submit(&workers, remote, 3, |_| Ok(()));
        wait_completed(&third);
        let outcome = first.complete().unwrap().2;
        assert!(outcome.result.is_ok());
        assert!(!outcome.cancellation_accepted);
        let mut fourth = workers
            .submit(
                ActiveState::Empty,
                ExecutionTarget::Local,
                ExecutionTicket {
                    command: 4,
                    session: session::SessionKey::default(),
                },
                None,
                "synthetic partial cancellation",
                |_| {
                    OperationOutcome::after_cancellation(Err(
                        RuntimeError::operation_indeterminate("synthetic background stop failure"),
                    ))
                },
            )
            .unwrap_or_else(|(_, error)| panic!("synthetic admission: {error}"));
        wait_completed(&fourth);
        let (_, _, outcome) = fourth.complete().unwrap();
        assert!(outcome.cancellation_accepted);
        assert_eq!(
            crate::Failure::from(outcome.result.unwrap_err()).code,
            "runtime_operation_indeterminate"
        );
        let mut fifth = submit(&workers, ExecutionTarget::Local, 5, |_| {
            panic!("synthetic operation unwind")
        });
        wait_completed(&fifth);
        let (_, _, outcome) = fifth.complete().unwrap();
        assert!(!outcome.cancellation_accepted);
        assert_eq!(
            crate::Failure::from(outcome.result.unwrap_err()).code,
            "runtime_operation_indeterminate"
        );
        let mut rejected = submit(&workers, ExecutionTarget::Local, 6, |_| {
            Err(RuntimeError::invalid("synthetic explicit rejection"))
        });
        wait_completed(&rejected);
        let (_, _, outcome) = rejected.complete().unwrap();
        assert!(!outcome.cancellation_accepted);
        assert_eq!(
            crate::Failure::from(outcome.result.unwrap_err()).kind,
            crate::FailureKind::Invalid
        );
        drop((first, second, third, fourth, fifth, rejected));
        workers.shutdown();
        assert!(workers.locations.lock().unwrap().is_empty());
    }

    #[test]
    fn shutdown_waits_for_abandoned_operation_cleanup() {
        let home = std::env::temp_dir().join(format!(
            "velune-execution-cleanup-{}",
            crate::new_record_key()
        ));
        fs::create_dir(&home).unwrap();
        let mut application = crate::Application::open(crate::Options {
            home_directory: home.clone(),
            resources_directory: home.clone(),
        })
        .unwrap();
        let workers = &application.runtime.execution_workers;
        let (started, start) = mpsc::sync_channel(1);
        let (release, released) = mpsc::sync_channel(1);
        let pending = submit(workers, ExecutionTarget::Local, 1, move |_| {
            started.send(()).unwrap();
            released.recv().unwrap();
            Ok(())
        });
        start.recv_timeout(Duration::from_secs(2)).unwrap();
        let locations = workers.locations.clone();
        application.runtime.session.active_state = ActiveState::Pending(Box::new(pending));
        assert_eq!(
            locations.lock().unwrap().get(&ExecutionTarget::Local),
            Some(&1)
        );
        let (finished, finish) = mpsc::sync_channel(1);
        let owner = std::thread::spawn(move || {
            drop(application);
            finished.send(()).unwrap();
        });
        assert!(matches!(
            finish.recv_timeout(Duration::from_millis(30)),
            Err(mpsc::RecvTimeoutError::Timeout)
        ));
        assert!(
            crate::repository::Repository::lock_home(&home).is_err(),
            "home must remain locked until execution cleanup returns"
        );
        release.send(()).unwrap();
        finish.recv_timeout(Duration::from_secs(2)).unwrap();
        owner.join().unwrap();
        assert!(locations.lock().unwrap().is_empty());
        drop(crate::repository::Repository::lock_home(&home).unwrap());
        fs::remove_dir_all(home).unwrap();
    }
}

#[cfg(all(test, unix))]
pub(crate) mod cleanup_test_gate {
    use super::*;
    type Gate = (mpsc::Sender<()>, mpsc::Receiver<()>);
    static GATES: std::sync::LazyLock<Mutex<HashMap<String, Gate>>> =
        std::sync::LazyLock::new(Default::default);

    pub(crate) fn register(binary: &Path) -> (mpsc::Receiver<()>, mpsc::Sender<()>) {
        let (entered, entry) = mpsc::channel();
        let (release, released) = mpsc::channel();
        GATES
            .lock()
            .unwrap()
            .insert(binary.to_string_lossy().into_owned(), (entered, released));
        (entry, release)
    }

    pub(super) fn take(binary: Option<String>) -> Option<Gate> {
        GATES.lock().unwrap().remove(&binary?)
    }
}

#[cfg(all(test, unix))]
pub(crate) mod pi_delete_test_gate {
    use super::*;
    use std::sync::atomic::{AtomicBool, Ordering};
    type MutationGate = (mpsc::Sender<()>, mpsc::Receiver<()>);
    static MUTATION: std::sync::LazyLock<Mutex<HashMap<String, MutationGate>>> =
        std::sync::LazyLock::new(Default::default);
    type InstallGate = (Option<mpsc::Sender<()>>, Arc<AtomicBool>);
    static INSTALL: std::sync::LazyLock<Mutex<HashMap<String, InstallGate>>> =
        std::sync::LazyLock::new(Default::default);

    pub(crate) fn register_mutation(binary: &Path) -> (mpsc::Receiver<()>, mpsc::Sender<()>) {
        let (entered, entry) = mpsc::channel();
        let (release, released) = mpsc::channel();
        MUTATION
            .lock()
            .unwrap()
            .insert(binary.to_string_lossy().into_owned(), (entered, released));
        (entry, release)
    }
    pub(crate) fn after_mutation(binary: Option<String>) {
        let gate = binary.and_then(|binary| MUTATION.lock().unwrap().remove(&binary));
        if let Some((entered, release)) = gate {
            let _ = entered.send(());
            let _ = release.recv();
        }
    }
    pub(crate) struct InstallRelease(Arc<AtomicBool>);
    impl InstallRelease {
        pub(crate) fn release(&self) {
            self.0.store(false, Ordering::Release);
        }
    }
    impl Drop for InstallRelease {
        fn drop(&mut self) {
            self.release();
        }
    }

    pub(crate) fn register_install(binary: &Path) -> (mpsc::Receiver<()>, InstallRelease) {
        let (entered, entry) = mpsc::channel();
        let held = Arc::new(AtomicBool::new(true));
        INSTALL.lock().unwrap().insert(
            binary.to_string_lossy().into_owned(),
            (Some(entered), held.clone()),
        );
        (entry, InstallRelease(held))
    }
    pub(crate) fn install_held(binary: Option<&String>) -> bool {
        let Some(binary) = binary else {
            return false;
        };
        let mut gates = INSTALL.lock().unwrap();
        let Some((entered, held)) = gates.get_mut(binary) else {
            return false;
        };
        if held.load(Ordering::Acquire) {
            if let Some(entered) = entered.take() {
                let _ = entered.send(());
            }
            true
        } else {
            gates.remove(binary);
            false
        }
    }
}

#[cfg(all(test, unix))]
pub(crate) mod pi_edit_deadline_test_gate {
    use super::*;
    static REQUESTS: std::sync::LazyLock<Mutex<HashMap<String, mpsc::Sender<()>>>> =
        std::sync::LazyLock::new(Default::default);

    pub(crate) fn expire_after_prompt(binary: &Path) -> mpsc::Receiver<()> {
        let (expired, receiver) = mpsc::channel();
        REQUESTS
            .lock()
            .unwrap()
            .insert(binary.to_string_lossy().into_owned(), expired);
        receiver
    }

    pub(in crate::local) fn take(binary: Option<&String>) -> Option<mpsc::Sender<()>> {
        REQUESTS.lock().unwrap().remove(binary?)
    }
}
