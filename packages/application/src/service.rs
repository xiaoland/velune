//! One application owner, advanced independently from its presentation clients.
use crate::local::{
    Focus, Login,
    conversation_history::{
        ConversationHistoryRequest, ConversationRead, LoadedConversationHistory,
    },
};
use crate::{
    Application, ApplicationReply, ClientViewId, Command, Error, ExecutionContext, Failure,
    FailureKind, Options, Response, Revision,
};
use std::{
    collections::BTreeMap,
    sync::{
        Arc, Mutex,
        atomic::{AtomicBool, Ordering},
        mpsc,
    },
    time::{Duration, Instant},
};

#[derive(Clone)]
pub struct ApplicationService {
    sender: mpsc::Sender<Job>,
}

enum Job {
    Execute {
        view: ClientViewId,
        presentation_isolated: bool,
        context: Option<ExecutionContext>,
        command: Box<Command>,
        queued_at: Instant,
        history_completion: Option<mpsc::Sender<Job>>,
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
    presentation_isolated: bool,
}

type CommandReply = mpsc::Sender<Result<ApplicationReply, Failure>>;

struct PendingHistory {
    view: ClientViewId,
    view_created: u64,
    conversation_id: String,
    affected_ids: Vec<String>,
    invalidated: bool,
    remote: bool,
    reply: CommandReply,
}

struct HistoryJob {
    completion: mpsc::Sender<Job>,
    id: u64,
    request: ConversationHistoryRequest,
    queued_at: Instant,
    dispatch: tracing::Dispatch,
    span: tracing::Span,
}

struct FinishedHistory {
    id: u64,
    history: LoadedConversationHistory,
    dispatch: tracing::Dispatch,
    span: tracing::Span,
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
                        let Ok(HistoryJob { completion, id, request, queued_at, dispatch, span }) = job else {
                            break;
                        };
                        if stopped.load(Ordering::Acquire) {
                            break;
                        }
                        let history = tracing::dispatcher::with_default(&dispatch, || {
                            let _entered = span.enter();
                            tracing::info!(target: "velune_application", event = "conversation_history_read", phase = "worker_queue", elapsed_ms = queued_at.elapsed().as_millis() as u64);
                            request.load()
                        });
                        if completion.send(Job::HistoryFinished(Box::new(FinishedHistory { id, history, dispatch, span }))).is_err() {
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
    views: BTreeMap<ClientViewId, View>,
    view_clock: u64,
    revision: Revision,
    history: HistoryWorkers,
    pending_history: BTreeMap<u64, PendingHistory>,
}

impl Owner {
    fn execute(
        &mut self,
        view: ClientViewId,
        context: Option<ExecutionContext>,
        command: Command,
    ) -> Result<ApplicationReply, Failure> {
        if context.is_some() && command.requires_revision() {
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
            return Ok(ApplicationReply {
                revision: self.revision,
                response: Response::ReleaseView,
            });
        }
        for pending in self.pending_history.values_mut() {
            pending.invalidated |= affects_history(&command, &pending.affected_ids);
        }
        let changes = command.changes_shared_state();
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
        presentation_isolated: bool,
        operation: impl FnOnce(&mut Application) -> T,
    ) -> T {
        if !self.views.contains_key(view)
            && self.views.len() >= 128
            && let Some(expired) = self
                .views
                .iter()
                .min_by_key(|(_, view)| view.used)
                .map(|(key, _)| key.clone())
        {
            self.views.remove(&expired);
        }
        self.view_clock = self.view_clock.wrapping_add(1);
        let current = self.views.entry(view.clone()).or_insert_with(|| View {
            state: self.application.runtime.client_state(),
            created: self.view_clock,
            used: 0,
            presentation_isolated,
        });
        current.used = self.view_clock;
        self.application
            .runtime
            .enter_client_state(&mut current.state, current.presentation_isolated);
        let result = operation(&mut self.application);
        self.application
            .runtime
            .leave_client_state(&mut current.state);
        result
    }

    fn start_history(
        &mut self,
        view: ClientViewId,
        remote: bool,
        runtime_id: String,
        id: String,
        reply: CommandReply,
        completion: mpsc::Sender<Job>,
    ) {
        let prepared = self.with_view(&view, remote, |application| {
            application
                .runtime
                .serve(Focus::Existing(id.clone()), Login::Blocks, |core| {
                    core.prepare_conversation_read(&runtime_id, &id)
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
                    request: *request,
                    queued_at: Instant::now(),
                    dispatch: tracing::dispatcher::get_default(Clone::clone),
                    span: tracing::Span::current(),
                };
                if self.pending_history.len() >= 10 || self.history.jobs.try_send(job).is_err() {
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
                        remote,
                        reply,
                    },
                );
            }
            Ok(ConversationRead::Ready(snapshot)) => {
                let result = self.history_response(*snapshot, remote);
                let _ = reply.send(result);
            }
            Err(error) => {
                let _ = reply.send(Err(Failure::from(error)));
            }
        }
    }

    fn finish_history(&mut self, job: FinishedHistory) {
        let FinishedHistory {
            id,
            history,
            dispatch,
            span,
        } = job;
        let Some(pending) = self.pending_history.remove(&id) else {
            return;
        };
        self.application
            .runtime
            .end_history_read(&pending.conversation_id);
        let result = tracing::dispatcher::with_default(&dispatch, || {
            let _entered = span.enter();
            if self
                .views
                .get(&pending.view)
                .is_none_or(|view| view.created != pending.view_created)
            {
                return Err(Failure::new(
                    FailureKind::Conflict,
                    "已切换会话视图，历史读取结果已过期",
                ));
            }
            let result = self
                .with_view(&pending.view, pending.remote, |application| {
                    application.runtime.serve(
                        Focus::Existing(history.id().to_owned()),
                        Login::Blocks,
                        |core| {
                            if pending.invalidated {
                                return core
                                    .read_present_conversation(history.runtime_id(), history.id())?
                                    .ok_or_else(|| {
                                        Error::Contract(Failure::new(
                                            FailureKind::Conflict,
                                            "读取期间会话已改变，请重新打开",
                                        ))
                                    });
                            }
                            core.finish_conversation_read(history)
                        },
                    )
                })
                .map_err(Failure::from)?;
            self.history_response(result, pending.remote)
        });
        let _ = pending.reply.send(result);
    }

    fn history_response(
        &mut self,
        snapshot: crate::SnapshotResult,
        remote: bool,
    ) -> Result<ApplicationReply, Failure> {
        let response = Response::ReadConversation(snapshot);
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
        loop {
            match receiver.recv_timeout(Duration::from_millis(40)) {
                Ok(Job::Execute {
                    view,
                    presentation_isolated,
                    context,
                    command,
                    queued_at,
                    history_completion,
                    dispatch,
                    span,
                    reply,
                }) => {
                    if !matches!(command.as_ref(), Command::ReleaseView) {
                        self.with_view(&view, presentation_isolated || context.is_some(), |_| ());
                    }
                    tracing::dispatcher::with_default(&dispatch, || {
                        let _entered = span.enter();
                        let elapsed_ms = queued_at.elapsed().as_millis() as u64;
                        let kind = command.kind();
                        if elapsed_ms >= 100
                            || matches!(
                                *command,
                                Command::ReadConversation { .. }
                                    | Command::OpenConversation { .. }
                                    | Command::SendTurn { .. }
                                    | Command::ReplyRuntimeInteraction { .. }
                                    | Command::Cancel { .. }
                                    | Command::SetConversationHandoff { .. }
                            )
                        {
                            tracing::info!(target: "velune_application", event = "application_command", phase = "owner_queue", command = ?kind, elapsed_ms);
                        }
                        match *command {
                            Command::ReadConversation {
                                runtime_id,
                                conversation_id,
                            } => {
                                self.start_history(
                                    view,
                                    context.is_some(),
                                    runtime_id,
                                    conversation_id,
                                    reply,
                                    history_completion.expect("history completion channel"),
                                );
                            }
                            command => {
                                let _ = reply.send(self.execute(view, context, command));
                            }
                        }
                    });
                    self.application.runtime.publish_execution();
                }
                Ok(Job::HistoryFinished(history)) => self.finish_history(*history),
                Ok(Job::Close { stop_work, reply }) => {
                    let result = if stop_work {
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
            let mut advance = || {
                if self.application.runtime.advance().is_err() {
                    tracing::warn!(target: "velune_application", event = "application_advance_failed");
                }
                self.application.runtime.publish_execution();
            };
            if let Some(telemetry) = &telemetry {
                tracing::dispatcher::with_default(&telemetry.dispatch(), advance);
            } else {
                advance();
            }
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
                        views: BTreeMap::new(),
                        view_clock: 0,
                        revision,
                        history,
                        pending_history: BTreeMap::new(),
                    }
                    .run(receiver, telemetry);
                });
            })?;
        initialization
            .recv()
            .map_err(|_| Error::invalid("application owner initialization failed"))?
            .map_err(Error::Contract)?;
        Ok(Self { sender })
    }

    /// Local, trusted presentation API. Filesystem attachment paths are local to this host.
    pub fn execute(&self, view: ClientViewId, command: Command) -> Result<Response, Failure> {
        self.request(view, None, false, command)
            .map(|reply| reply.response)
    }

    /// Local execution with presentation preferences scoped to this client view.
    /// This does not change local command permissions or attachment semantics.
    pub fn execute_isolated(
        &self,
        view: ClientViewId,
        command: Command,
    ) -> Result<Response, Failure> {
        self.request(view, None, true, command)
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
        self.request(view, Some(context), true, command)
    }

    fn request(
        &self,
        view: ClientViewId,
        context: Option<ExecutionContext>,
        presentation_isolated: bool,
        command: Command,
    ) -> Result<ApplicationReply, Failure> {
        if view.0.is_empty() || view.0.len() > 512 || view.0.chars().any(char::is_control) {
            return Err(Failure::invalid("客户端视图标识无效"));
        }
        let (reply, result) = mpsc::channel();
        self.sender
            .send(Job::Execute {
                view,
                presentation_isolated,
                context,
                history_completion: matches!(command, Command::ReadConversation { .. })
                    .then(|| self.sender.clone()),
                command: Box::new(command),
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

fn affects_history(command: &Command, ids: &[String]) -> bool {
    use Command::*;
    match command {
        RenameConversation {
            conversation_id, ..
        }
        | DeleteConversation {
            conversation_id, ..
        }
        | SendTurn {
            conversation_id, ..
        }
        | EditMessage {
            conversation_id, ..
        }
        | CompactConversation {
            conversation_id, ..
        }
        | ResumeTurn {
            conversation_id, ..
        }
        | SteerTurn {
            conversation_id, ..
        }
        | EditSendQueue {
            conversation_id, ..
        }
        | ReplyRuntimeInteraction {
            conversation_id, ..
        }
        | Cancel {
            conversation_id, ..
        }
        | StopSubAgents {
            conversation_id, ..
        }
        | SetConversationHandoff {
            conversation_id, ..
        } => ids.contains(conversation_id),
        OrganizeConversations { conversations, .. } => conversations
            .iter()
            .any(|reference| ids.contains(&reference.conversation_id)),
        _ => false,
    }
}

fn closed() -> Failure {
    Failure::new(FailureKind::Closed, "应用执行端已关闭")
}

#[cfg(test)]
mod usage_tests {
    use super::*;
    use crate::config::UsageDisplay::{Remaining, Used};
    use std::{fs, path::PathBuf};

    struct Fixture(PathBuf);
    impl Fixture {
        fn new() -> Self {
            // Tests run in parallel; a clock-based name can repeat within its resolution.
            let mut nonce = [0; 8];
            getrandom::fill(&mut nonce).unwrap();
            let path =
                std::env::temp_dir().join(format!("velune-usage-{:x}", u64::from_le_bytes(nonce)));
            fs::create_dir(&path).unwrap();
            Self(path)
        }
        fn open(&self) -> ApplicationService {
            ApplicationService::open(Options {
                home_directory: self.0.clone(),
                resources_directory: self.0.clone(),
            })
            .unwrap()
        }
    }
    impl Drop for Fixture {
        fn drop(&mut self) {
            let _ = fs::remove_dir_all(&self.0);
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
}
