//! One application owner, advanced independently from its presentation clients.
use crate::{
    Application, ApplicationReply, ClientViewId, Command, Error, ExecutionContext, Failure,
    FailureKind, Options, Response, Revision,
};
use std::{collections::BTreeMap, sync::mpsc, time::Duration};

#[derive(Clone)]
pub struct ApplicationService {
    sender: mpsc::Sender<Job>,
}

enum Job {
    Execute {
        view: ClientViewId,
        context: Option<ExecutionContext>,
        command: Box<Command>,
        dispatch: tracing::Dispatch,
        span: tracing::Span,
        reply: mpsc::Sender<Result<ApplicationReply, Failure>>,
    },
    Close {
        stop_work: bool,
        reply: mpsc::Sender<Result<(), Failure>>,
    },
}

struct Owner {
    application: Application,
    views: BTreeMap<ClientViewId, (crate::implementation::ClientState, u64)>,
    view_clock: u64,
    revision: Revision,
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
        if !self.views.contains_key(&view)
            && self.views.len() >= 128
            && let Some(expired) = self
                .views
                .iter()
                .min_by_key(|(_, (_, used))| *used)
                .map(|(key, _)| key.clone())
        {
            self.views.remove(&expired);
        }
        self.view_clock = self.view_clock.wrapping_add(1);
        let (state, used) = self
            .views
            .entry(view.clone())
            .or_insert_with(|| (self.application.runtime.client_state(), 0));
        *used = self.view_clock;
        self.application
            .runtime
            .enter_client_state(state, context.is_some());
        let changes = command.changes_shared_state();
        let result = self
            .application
            .execute_command(command, context.is_some(), &view);
        self.application.runtime.leave_client_state(state);
        // A failed dispatch can commit association metadata; invalidate old edits too.
        if changes {
            self.revision.0 = self.revision.0.wrapping_add(1);
        }
        result.map(|response| ApplicationReply {
            revision: self.revision,
            response,
        })
    }

    fn run(&mut self, receiver: mpsc::Receiver<Job>) {
        loop {
            match receiver.recv_timeout(Duration::from_millis(40)) {
                Ok(Job::Execute {
                    view,
                    context,
                    command,
                    dispatch,
                    span,
                    reply,
                }) => {
                    let outcome = tracing::dispatcher::with_default(&dispatch, || {
                        let _entered = span.enter();
                        self.execute(view, context, *command)
                    });
                    let _ = reply.send(outcome);
                }
                Ok(Job::Close { stop_work, reply }) => {
                    let result = if stop_work {
                        self.application.close_stopping_work()
                    } else {
                        self.application.close()
                    }
                    .map_err(Failure::from);
                    let closed = result.is_ok();
                    let _ = reply.send(result);
                    if closed {
                        break;
                    }
                }
                Err(mpsc::RecvTimeoutError::Timeout) => {}
                Err(mpsc::RecvTimeoutError::Disconnected) => break,
            }
            if self.application.runtime.advance().is_err() {
                tracing::warn!(target: "velune_application", event = "application_advance_failed");
            }
        }
    }
}

impl ApplicationService {
    /// The execution host retains this handle while it intends to keep work alive.
    /// A controller connection owns no Application and dropping it cannot cancel work.
    pub fn open(options: Options) -> Result<Self, Error> {
        let (sender, receiver) = mpsc::channel();
        let (initialized, initialization) = mpsc::sync_channel(1);
        let dispatch = tracing::dispatcher::get_default(Clone::clone);
        std::thread::Builder::new()
            .name("velune-application".into())
            .spawn(move || {
                tracing::dispatcher::with_default(&dispatch, || {
                    let application = match Application::open(options) {
                        Ok(application) => application,
                        Err(error) => {
                            let _ = initialized.send(Err(Failure::from(error)));
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
                    }
                    .run(receiver);
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
        self.request(view, None, command)
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
        self.request(view, Some(context), command)
    }

    fn request(
        &self,
        view: ClientViewId,
        context: Option<ExecutionContext>,
        command: Command,
    ) -> Result<ApplicationReply, Failure> {
        if view.0.is_empty() || view.0.len() > 512 || view.0.chars().any(char::is_control) {
            return Err(Failure::invalid("客户端视图标识无效"));
        }
        let (reply, result) = mpsc::channel();
        self.sender
            .send(Job::Execute {
                view,
                context,
                command: Box::new(command),
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
