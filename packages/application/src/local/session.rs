//! One open conversation's execution and projection state. The core keeps
//! several; requests address one at a time through `CoreRuntime::session`.
use super::*;

/// Identifies a session while it moves between addressed and parked.
#[derive(Clone, Copy, PartialEq, Eq)]
pub(super) struct SessionKey(u64);

impl Default for SessionKey {
    fn default() -> Self {
        static NEXT: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(1);
        Self(NEXT.fetch_add(1, std::sync::atomic::Ordering::Relaxed))
    }
}

#[derive(Default)]
pub(super) struct Session {
    pub(super) key: SessionKey,
    /// Authorizes this session's executions to use Velune's collaboration tools.
    pub(super) collaboration_ticket: Option<collaboration::Ticket>,
    pub(super) logical_projection: Option<continuation::LogicalProjection>,
    pub(super) transcript_projection: velune_agent_runtime::transcript::TranscriptProjection,
    pub(super) transcript_conversation_id: Option<String>,
    pub(super) pi: PiState,
    pub(super) active_state: ActiveState,
    pub(super) gateway_runner: Option<Runner>,
    pub(super) execution_runtime_id: Option<String>,
    pub(super) model_record_key: Option<String>,
    pub(super) model_selection: Option<crate::ModelSelection>,
    pub(super) unsubmitted_draft: Option<UnsubmittedDraft>,
    pub(super) gateway_tunnel: Option<velune_agent_runtime::execution::LoopbackTunnel>,
    /// Private execution files of this session's runtime; see `CoreRuntime::lane`.
    pub(super) lane: Option<usize>,
    pub(super) source: Option<live::SourceWatch>,
    /// When this session was last seen executing; its native writes until
    /// shortly after that are Velune's own.
    pub(super) last_execution: Option<std::time::Instant>,
    /// The user asked to stop the current turn.
    pub(super) cancel_requested: bool,
    pub(super) send_queue: follow_up::SendQueue,
}

impl Session {
    /// The conversation's own turn runs.
    pub(super) fn turn_running(&self) -> bool {
        match &self.active_state {
            ActiveState::Native(session) => session.turn_running(),
            ActiveState::Acp(session) => session.turn_running(),
            ActiveState::Pi => self.pi.busy,
            ActiveState::Empty | ActiveState::History(_) => false,
        }
    }

    /// Work the session's runtime process does besides the turn, such as
    /// native sub-agents the turn started. The process, and the gateway a
    /// turn used, last until it has ended too.
    pub(super) fn work(&self) -> Vec<velune_agent_runtime::work::BackgroundTask> {
        match &self.active_state {
            ActiveState::Native(session) => session.work(),
            _ => Vec::new(),
        }
    }

    /// Its runtime process works: the turn runs, or work besides it does.
    pub(super) fn executing(&self) -> bool {
        self.turn_running() || !self.work().is_empty()
    }

    /// Must stay open: closing it would end work, drop a request the user
    /// has not answered or drop queued messages.
    pub(super) fn pinned(&self) -> bool {
        self.executing()
            || !self.send_queue.is_empty()
            || self
                .projected()
                .is_some_and(|snapshot| !snapshot.pending_interactions.is_empty())
    }

    /// The native view this session shows, before Velune decoration.
    pub(super) fn projected(&self) -> Option<&crate::conversation::ConversationSnapshot> {
        match &self.active_state {
            ActiveState::Native(session) => session.snapshot(),
            ActiveState::Acp(session) => session.snapshot(),
            ActiveState::Pi => self
                .pi
                .projection
                .as_ref()
                .and_then(|p| p.snapshot.as_ref()),
            ActiveState::History(snapshot) => Some(snapshot.as_ref()),
            ActiveState::Empty => None,
        }
    }

    /// Whether this session presents the public conversation `id`: a
    /// continued conversation by its link or origin, an unsent conversation by
    /// its public or native identity, or the native conversation itself.
    pub(super) fn presents(&self, id: &str) -> bool {
        self.logical_projection
            .as_ref()
            .is_some_and(|projection| projection.link.id == id || projection.origin.id == id)
            || self
                .unsubmitted_draft
                .as_ref()
                .is_some_and(|draft| draft.public_id == id || draft.native_id == id)
            || self
                .projected()
                .is_some_and(|snapshot| snapshot.conversation.id == id)
    }

    /// The identity clients know this conversation by when it differs from
    /// the native one: a continued conversation's link, an unsent one's
    /// public identity.
    fn public_id(&self) -> Option<&str> {
        self.logical_projection
            .as_ref()
            .map(|projection| projection.link.id.as_str())
            .or_else(|| {
                self.unsubmitted_draft
                    .as_ref()
                    .map(|draft| draft.public_id.as_str())
            })
    }

    /// The identity clients know this session's conversation by.
    pub(super) fn conversation_id(&self) -> Option<&str> {
        self.public_id()
            .or_else(|| self.projected().map(|view| view.conversation.id.as_str()))
    }

    fn is_empty(&self) -> bool {
        matches!(self.active_state, ActiveState::Empty)
            && self.unsubmitted_draft.is_none()
            && self.logical_projection.is_none()
    }
}

/// Idle sessions kept open besides pinned ones, so several windows can each
/// keep showing their conversation.
const RETAINED_IDLE_SESSIONS: usize = 8;

/// Turns that ended recently, for clients that poll activity.
#[derive(Default)]
pub(super) struct FinishedTurns {
    sequence: u64,
    turns: std::collections::VecDeque<crate::FinishedTurn>,
}

/// Ended turns kept for clients that poll less often than turns end.
const RETAINED_FINISHED_TURNS: usize = 64;

impl CoreRuntime {
    /// Address the open session of conversation `id`, if there is one.
    pub(super) fn focus_existing(&mut self, id: &str) -> bool {
        if self.session.presents(id) {
            return true;
        }
        let Some(index) = self.parked.iter().position(|session| session.presents(id)) else {
            return false;
        };
        let session = self.parked.remove(index);
        let previous = std::mem::replace(&mut self.session, session);
        self.park(previous);
        true
    }

    /// Address conversation `id`: its open session, or a new empty one that
    /// the request then loads.
    pub(super) fn focus_conversation(&mut self, id: &str) {
        if !self.focus_existing(id) {
            self.focus_new();
        }
    }

    /// Address the session `key` again, or a new empty one when it has
    /// closed; an empty session is not kept when another is addressed.
    pub(super) fn focus_key(&mut self, key: SessionKey) {
        if self.session.key == key {
            return;
        }
        match self.parked.iter().position(|session| session.key == key) {
            Some(index) => {
                let session = self.parked.remove(index);
                let previous = std::mem::replace(&mut self.session, session);
                self.park(previous);
            }
            None => self.focus_new(),
        }
    }

    /// Address a new empty session; the current one stays open.
    pub(super) fn focus_new(&mut self) {
        let previous = std::mem::take(&mut self.session);
        self.park(previous);
    }

    fn park(&mut self, session: Session) {
        if !session.is_empty() {
            self.parked.push(session);
        }
    }

    /// Run `operation` with each open session addressed in turn, ending on the
    /// session that was addressed before. Every session runs even if one fails;
    /// the first failure is returned.
    pub(super) fn for_each_session(
        &mut self,
        mut operation: impl FnMut(&mut Self) -> Result<(), RuntimeError>,
    ) -> Result<(), RuntimeError> {
        let mut result = operation(self);
        for index in 0..self.parked.len() {
            std::mem::swap(&mut self.session, &mut self.parked[index]);
            let outcome = operation(self);
            std::mem::swap(&mut self.session, &mut self.parked[index]);
            result = result.and(outcome);
        }
        result
    }

    /// Advance every open session's execution.
    pub(super) fn drain_sessions(&mut self) -> Result<(), RuntimeError> {
        self.for_each_session(Self::drain_runtime)
    }

    /// Any session executes; configuration that execution depends on must
    /// not change underneath it.
    pub(super) fn execution_active(&self) -> bool {
        std::iter::once(&self.session)
            .chain(&self.parked)
            .any(Session::executing)
    }

    /// Refuses what would end the addressed session's runtime process while
    /// work besides its turn still runs there.
    pub(super) fn ensure_work_ended(&self) -> Result<(), RuntimeError> {
        if self.session.work().is_empty() {
            Ok(())
        } else {
            Err(RuntimeError::invalid(
                "子 Agent 仍在运行；请等待其结束，或先停止子 Agent",
            ))
        }
    }

    /// A native sub-agent of the session still runs; it may run in the
    /// session's own runtime process.
    fn runs_sub_agents(&self, session: &Session) -> bool {
        let ActiveState::Native(native) = &session.active_state else {
            return false;
        };
        !native.work().is_empty()
            || native.snapshot().is_some_and(|view| {
                view.messages
                    .iter()
                    .flat_map(|message| &message.blocks)
                    .any(|block| {
                        matches!(block, crate::conversation::MessageBlock::Tool { conversation_id: Some(id), .. } if self.runs_elsewhere(id))
                    })
            })
    }

    /// Close the least recently used idle sessions beyond the retained number.
    /// One whose native sub-agents still run is not idle.
    pub(super) fn trim_sessions(&mut self) {
        let mut kept: Vec<bool> = self
            .parked
            .iter()
            .map(|session| session.pinned() || self.runs_sub_agents(session))
            .collect();
        let mut idle = kept.iter().filter(|kept| !**kept).count();
        let mut index = 0;
        while idle > RETAINED_IDLE_SESSIONS && index < self.parked.len() {
            if kept[index] {
                index += 1;
                continue;
            }
            kept.remove(index);
            let session = self.parked.remove(index);
            let previous = std::mem::replace(&mut self.session, session);
            if let Err(error) = self.shutdown_active() {
                tracing::warn!(target: "velune_application", event = "session_close_failed", detail = %error);
            }
            self.session = previous;
            idle -= 1;
        }
    }

    /// Execution files private to the addressed session, below the runtime's
    /// projection directory: concurrent sessions of one runtime would
    /// otherwise overwrite each other's model and selection files.
    pub(super) fn lane_projection(&mut self, runtime_projection: &Path) -> PathBuf {
        let lane = self.session.lane.unwrap_or_else(|| {
            let taken: std::collections::BTreeSet<usize> = self
                .parked
                .iter()
                .filter_map(|session| session.lane)
                .collect();
            (0..).find(|lane| !taken.contains(lane)).expect("free lane")
        });
        self.session.lane = Some(lane);
        runtime_projection.join(format!("lane-{lane}"))
    }

    /// Record how the addressed session's turn ended.
    pub(super) fn record_finished_turn(&mut self, failed: bool) -> Option<crate::TurnOutcome> {
        let view = self.session.projected()?;
        let outcome = if self.session.cancel_requested {
            crate::TurnOutcome::Cancelled
        } else if failed
            || view.run_state == RunState::Failed
            || matches!(self.session.active_state, ActiveState::Pi)
                && self
                    .session
                    .pi
                    .projection
                    .as_ref()
                    .is_some_and(PiProjection::turn_failed)
        {
            crate::TurnOutcome::Failed
        } else {
            crate::TurnOutcome::Completed
        };
        let finished = &mut self.finished_turns;
        finished.sequence += 1;
        finished.turns.push_back(crate::FinishedTurn {
            sequence: finished.sequence,
            context_runtime_id: view.conversation.runtime_id.clone(),
            conversation_id: self.session.conversation_id().unwrap_or_default().into(),
            outcome,
        });
        if finished.turns.len() > RETAINED_FINISHED_TURNS {
            finished.turns.pop_front();
        }
        self.session.cancel_requested = false;
        Some(outcome)
    }

    pub(super) fn conversation_activity(&mut self, since: u64) -> crate::ConversationActivity {
        crate::ConversationActivity {
            active: self.active_conversations(),
            finished: self
                .finished_turns
                .turns
                .iter()
                .filter(|turn| turn.sequence > since)
                .cloned()
                .collect(),
            cursor: self.finished_turns.sequence,
        }
    }

    /// Conversations that execute or wait for the user, across sessions, with
    /// the native sub-agents they run, then those running outside Velune.
    /// Polled often, so it reads each native view without building a snapshot.
    pub(super) fn active_conversations(&mut self) -> Vec<crate::ActiveConversation> {
        let mut active = Vec::new();
        for session in std::iter::once(&self.session).chain(&self.parked) {
            let Some(view) = session.projected() else {
                continue;
            };
            let id = session.conversation_id().unwrap_or_default();
            let pending = view.pending_interactions.len();
            if session.turn_running() || pending > 0 {
                active.push(crate::ActiveConversation {
                    context_runtime_id: view.conversation.runtime_id.clone(),
                    conversation_id: id.into(),
                    run_state: view.run_state.clone(),
                    pending_interactions: pending as u32,
                    parent_conversation_id: None,
                });
            }
            if let ActiveState::Native(native) = &session.active_state {
                active.extend(
                    native
                        .work()
                        .into_iter()
                        .map(|task| crate::ActiveConversation {
                            context_runtime_id: view.conversation.runtime_id.clone(),
                            conversation_id: task.conversation_id,
                            run_state: match task.state {
                                velune_agent_runtime::work::BackgroundState::Stopping => {
                                    RunState::Stopping
                                }
                                _ => RunState::Running,
                            },
                            pending_interactions: 0,
                            parent_conversation_id: Some(id.into()),
                        }),
                );
            }
        }
        for mut item in self.elsewhere_conversations() {
            if !active
                .iter()
                .any(|known| known.conversation_id == item.conversation_id)
            {
                item.parent_conversation_id = self.sub_agent_parent(&item.conversation_id);
                active.push(item);
            }
        }
        active
    }
}
