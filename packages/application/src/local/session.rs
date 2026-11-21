//! One open conversation's execution and projection state. The core keeps
//! several; requests address one at a time through `CoreRuntime::session`.
use super::*;
use velune_agent_runtime::{
    LiveError,
    creation::{DataRetention, Retained},
    work::BackgroundTask,
};

/// Identifies a session while it moves between addressed and parked.
#[derive(Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub(super) struct SessionKey(u64);

impl Default for SessionKey {
    fn default() -> Self {
        static NEXT: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(1);
        Self(NEXT.fetch_add(1, std::sync::atomic::Ordering::Relaxed))
    }
}

#[derive(Default)]
pub(super) struct Session {
    pub(super) pi_io_cleanup: Option<Box<execution_workers::PendingExecution>>,
    pub(super) pi_control: Option<editing::PiControl>,
    pub(super) pi_edit: Option<editing::PiEdit>,
    pub(super) pi_edit_advancing: bool,
    pub(super) pi_cleanup_required: bool,
    pub(super) naming: Option<super::naming::Job>,
    pub(super) naming_attempted: bool,
    pub(super) finished_activity: Option<String>,
    pub(super) scheduled_run: Option<(String, bool)>,
    pub(super) turn_observation: Option<velune_telemetry::Operation>,
    pub(super) key: SessionKey,
    /// Authorizes this session's executions to use Velune's collaboration tools.
    pub(super) collaboration_ticket: Option<collaboration::Ticket>,
    pub(super) logical_projection: Option<continuation::LogicalProjection>,
    pub(super) transcript_projection: velune_agent_runtime::transcript::TranscriptProjection,
    pub(super) transcript_conversation_id: Option<String>,
    pub(super) edit_revision_cache: std::cell::RefCell<Option<editing::RevisionCache>>,
    /// Cached estimate for a parked history projection. Cleared whenever an
    /// addressed session re-enters the cache; never serialized on each poll.
    pub(super) history_cache_weight: Option<(u64, usize, usize)>,
    pub(super) active_state: ActiveState,
    pub(super) native_controls: super::native_control::NativeControls,
    pub(super) gateway_runner: Option<Runner>,
    pub(super) execution_runtime_id: Option<String>,
    /// Immutable launch/provider configuration of the prepared execution.
    pub(super) execution_configuration: Option<super::execution::ExecutionConfiguration>,
    /// A saved edit takes effect when this execution next prepares, never mid-turn.
    pub(super) configuration_changed: bool,
    pub(super) model_record_key: Option<String>,
    /// The model the session's turns are sent with, once known; `Some(None)`
    /// is the runtime's own default.
    pub(super) model_selection: Option<Option<crate::ModelSelection>>,
    pub(super) unsubmitted_draft: Option<UnsubmittedDraft>,
    /// A native fork that is only persisted when its first turn starts.
    /// Unlike an empty new conversation, reconfiguration must keep its
    /// inherited source and boundary instead of replacing it with a blank one.
    pub(super) pending_native_fork:
        Option<velune_agent_runtime::creation::Retained<velune_agent_runtime::native::PendingFork>>,
    /// One new Codex thread's quoted context, consumed during creation.
    pub(super) reference_context: Option<String>,
    pub(super) gateway_tunnel: Option<velune_agent_runtime::execution::LoopbackTunnel>,
    /// Private execution files of this session's runtime; see `CoreRuntime::lane`.
    pub(super) lane: Option<usize>,
    pub(super) source: Option<live::SourceWatch>,
    /// When this session was last seen executing; its native writes until
    /// shortly after that are Velune's own.
    pub(super) last_execution: Option<std::time::Instant>,
    /// The adapter accepted a request to stop the current turn.
    pub(super) cancel_requested: bool,
    /// Context maintenance must not complete a user turn or advance its queue.
    pub(super) compacting: bool,
    pub(super) send_queue: follow_up::SendQueue,
    /// This session is a side chat on another conversation.
    pub(super) side: Option<SideChat>,
}

/// A side chat: a temporary runtime copy of a conversation, never saved,
/// that is open until it is closed. Clients know it only by its own
/// identity; it is not a conversation of the list or its activity.
#[derive(Clone)]
pub(super) struct SideChat {
    pub(super) id: String,
    pub(super) title: velune_conversation::ConversationTitle,
    pub(super) reference: crate::conversation::ConversationSnapshot,
    pub(super) prior: Vec<crate::conversation::Message>,
    pub(super) prior_executions: velune_conversation::Executions,
    pub(super) prior_work_notification_ids: Vec<String>,
    pub(super) segment: u64,
    pub(super) context: String,
    pub(super) context_pending: bool,
    /// Messages of the conversation the copy began with, which it does not show.
    pub(super) inherited: std::collections::BTreeSet<String>,
}

impl Session {
    /// Keep the unpersisted fork's actual recovery provenance across reconfiguration.
    pub(super) fn refresh_pending_native_fork(&mut self) -> Result<(), RuntimeError> {
        if let ActiveState::Native(native) = &self.active_state {
            self.pending_native_fork = native
                .pending_fork()
                .map_err(|error| RuntimeError::context("保留待创建原生分支", error))?;
        }
        Ok(())
    }

    /// The conversation's own turn runs.
    pub(super) fn turn_running(&self) -> bool {
        self.active_state
            .live()
            .is_some_and(LiveConversation::turn_running)
    }

    /// The Pi conversation this session runs.
    pub(super) fn pi(&mut self) -> Result<&mut pi::PiSession, RuntimeError> {
        match &mut self.active_state {
            ActiveState::Pi(session) => Ok(session),
            _ => Err(RuntimeError::invalid("会话执行尚未准备")),
        }
    }

    /// Work the session's runtime process does besides the turn, such as
    /// native sub-agents the turn started. The process, and the gateway a
    /// turn used, last until it has ended too.
    pub(super) fn work(&self) -> Result<Retained<Vec<BackgroundTask>>, LiveError> {
        self.active_state.live().map_or_else(
            || Ok(Retained::from_parts(Vec::new(), DataRetention::default())),
            LiveConversation::work,
        )
    }

    pub(super) fn work_settled(&self) -> bool {
        self.active_state
            .live()
            .is_none_or(LiveConversation::work_settled)
    }

    /// Its runtime connection is still needed by execution or metadata;
    /// visible activity remains the user turn and native background work.
    pub(super) fn executing(&self) -> bool {
        self.pi_io_cleanup.is_some()
            || self.pi_edit.is_some()
            || self.pi_edit_advancing
            || self.pi_cleanup_required
            || self.pi_control.is_some()
            || self.native_controls.pending()
            || self
                .active_state
                .live()
                .is_some_and(LiveConversation::has_pending_requests)
            || self.turn_running()
            || !self.work_settled()
            || self
                .active_state
                .live()
                .is_some_and(LiveConversation::has_pending_metadata)
    }

    /// Editing or removing a source also invalidates a continued conversation's projection.
    pub(super) fn uses_runtime(&self, id: &str) -> bool {
        self.execution_runtime_id.as_deref() == Some(id)
            || self.logical_projection.as_ref().is_some_and(|projection| {
                projection
                    .link
                    .segments
                    .iter()
                    .any(|segment| segment.runtime_instance_id == id)
            })
            || self
                .projected()
                .is_some_and(|snapshot| snapshot.conversation.runtime_id == id)
    }

    /// Whether closing execution preserves the ability to continue this conversation.
    pub(super) fn can_reopen_execution(&self) -> bool {
        self.side.is_none()
            && self.unsubmitted_draft.is_none()
            && self.pending_native_fork.is_none()
            && !self.requires_live_connection()
    }

    pub(super) fn requires_live_connection(&self) -> bool {
        self.active_state
            .live()
            .is_some_and(|live| !live.supports_reopen())
    }

    /// Retain continuation that depends on this connection, active work,
    /// approvals and queued messages.
    pub(super) fn pinned(&self) -> bool {
        !self.can_reopen_execution()
            || self
                .source
                .as_ref()
                .is_some_and(|watch| watch.refresh_pending)
            || self.naming.is_some()
            || self.executing()
            || !self.send_queue.is_empty()
            || self.projected().is_some_and(|snapshot| {
                snapshot
                    .pending_interactions
                    .iter()
                    .any(|interaction| interaction.blocks_turn())
            })
    }

    /// The native view this session shows, before Velune decoration.
    pub(super) fn projected(&self) -> Option<&crate::conversation::ConversationSnapshot> {
        match &self.active_state {
            ActiveState::History(snapshot) => Some(snapshot.as_ref()),
            state => state.live().and_then(LiveConversation::snapshot),
        }
    }

    /// Whether this session presents the public conversation `id`: a
    /// continued conversation by its link or origin, an unsent conversation by
    /// its public or native identity, or the native conversation itself.
    pub(super) fn presents(&self, id: &str) -> bool {
        if let Some(side) = &self.side {
            return side.id == id;
        }
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

    pub(super) fn observes(&self, runtime_id: &str, conversation_id: &str) -> bool {
        self.projected().is_some_and(|snapshot| {
            snapshot.context_runtime_id == runtime_id && self.presents(conversation_id)
        })
    }

    /// The identity clients know this conversation by when it differs from
    /// the native one: a side chat's own, a continued conversation's link,
    /// an unsent one's public identity.
    fn public_id(&self) -> Option<&str> {
        self.side
            .as_ref()
            .map(|side| side.id.as_str())
            .or_else(|| {
                self.logical_projection
                    .as_ref()
                    .map(|projection| projection.link.id.as_str())
            })
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

    /// Replace the active history projection and invalidate its parked-size
    /// estimate. Callers use this at every history reload/reprojection path.
    pub(super) fn set_history(&mut self, snapshot: impl Into<browsing::HistorySnapshot>) {
        self.active_state = ActiveState::History(Box::new(snapshot.into()));
        self.invalidate_history_cache_weight();
    }

    /// Access history for an in-place mutation. Invalidating before the borrow
    /// makes same-revision, same-count body/header changes safe to account.
    pub(super) fn history_mut(&mut self) -> Option<&mut crate::conversation::ConversationSnapshot> {
        self.invalidate_history_cache_weight();
        match &mut self.active_state {
            ActiveState::History(snapshot) => Some(snapshot),
            _ => None,
        }
    }

    fn history_cache_weight(&mut self) -> usize {
        let key = match &self.active_state {
            ActiveState::History(snapshot) => (snapshot.revision, snapshot.messages.len()),
            _ => {
                self.history_cache_weight = None;
                return 0;
            }
        };
        if let Some((revision, messages, bytes)) = self.history_cache_weight
            && (revision, messages) == key
        {
            return bytes;
        }
        let bytes = match &self.active_state {
            ActiveState::History(snapshot) => serialized_size_estimate(&snapshot.snapshot),
            _ => 0,
        };
        self.history_cache_weight = Some((key.0, key.1, bytes));
        bytes
    }

    pub(super) fn invalidate_history_cache_weight(&mut self) {
        self.history_cache_weight = None;
    }
}

/// Idle sessions kept open besides pinned ones, so several windows can each
/// keep showing their conversation.
const RETAINED_IDLE_SESSIONS: usize = 8;
/// History-only projections are cheap to reopen and back the bounded set of
/// conversation views that may be polling concurrently. Live runtimes remain
/// separately capped by `RETAINED_IDLE_SESSIONS`.
const RETAINED_HISTORY_SESSIONS: usize = 128;
/// A count cap alone lets a handful of very long conversations retain many
/// gigabytes. This is a serialized-size estimate for parked history snapshots,
/// not a claim about allocator or resident memory.
const RETAINED_HISTORY_BYTES: usize = 64 * 1024 * 1024;

/// Count serialized bytes without allocating a second copy of a full history.
/// The estimate is computed only when a history projection changes.
#[derive(Default)]
struct ByteCounter(usize);

impl std::io::Write for ByteCounter {
    fn write(&mut self, bytes: &[u8]) -> std::io::Result<usize> {
        self.0 = self.0.saturating_add(bytes.len());
        Ok(bytes.len())
    }

    fn flush(&mut self) -> std::io::Result<()> {
        Ok(())
    }
}

fn serialized_size_estimate(value: &impl serde::Serialize) -> usize {
    let mut counter = ByteCounter::default();
    if serde_json::to_writer(&mut counter, value).is_ok() {
        counter.0
    } else {
        0
    }
}

/// Turns that ended recently, for clients that poll activity.
#[derive(Default)]
pub(super) struct FinishedTurns {
    sequence: u64,
    turns: std::collections::VecDeque<crate::FinishedTurn>,
}

/// Ended turns kept for clients that poll less often than turns end.
const RETAINED_FINISHED_TURNS: usize = 64;

impl CoreRuntime {
    /// Replace the complete set observed by active client views. The same
    /// projection identities protect both parked history and delivered payloads.
    pub(crate) fn replace_observed_conversations(
        &mut self,
        observed: std::collections::BTreeSet<(String, String)>,
    ) {
        if self.observed_conversations == observed {
            return;
        }
        self.observed_conversations = observed;
        self.trim_history_projections();
        self.delivery
            .trim_for_observations(&self.observed_conversations);
    }

    /// Configuration for new connections can change while a live-only session
    /// keeps the configuration it was created with.
    pub(super) fn preserves_connection_configuration(&self) -> bool {
        self.session.requires_live_connection()
            && self
                .session
                .execution_configuration
                .as_ref()
                .is_some_and(|saved| {
                    self.runtime_instance(&saved.runtime.id)
                        .is_ok_and(|current| {
                            current.enabled && current.type_id == saved.runtime.type_id
                        })
                })
    }

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
            // History writers invalidate at their authoritative replacement
            // or mutation point; preserving the weight here avoids rescanning
            // every unchanged client-view enter/leave cycle.
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

    /// Work still depends on this execution instance or a history source;
    /// its configuration and native resources must not change underneath it.
    pub(super) fn runtime_in_use(&self, runtime_id: &str) -> bool {
        std::iter::once(&self.session)
            .chain(&self.parked)
            .any(|session| session.executing() && session.uses_runtime(runtime_id))
    }

    /// Refuses what would end the addressed session's runtime process while
    /// work besides its turn still runs there.
    pub(super) fn ensure_work_ended(&self) -> Result<(), RuntimeError> {
        if self.session.work_settled() {
            Ok(())
        } else {
            Err(RuntimeError::OutstandingWork)
        }
    }

    /// A native sub-agent of the session still runs; it may run in the
    /// session's own runtime process.
    fn runs_sub_agents(&self, session: &Session) -> bool {
        let ActiveState::Native(native) = &session.active_state else {
            return false;
        };
        native.work().map_or(true, |work| !work.is_empty())
            || native.snapshot().is_some_and(|view| {
                view.messages
                    .iter()
                    .flat_map(|message| &message.blocks)
                    .any(|block| {
                        matches!(block, crate::conversation::MessageBlock::Tool { conversation_id: Some(id), .. } if self.runs_elsewhere(id))
                    })
            })
    }

    /// Close the least recently used reopenable idle sessions beyond the
    /// retained number. Sessions with outstanding work stay connected.
    pub(super) fn trim_sessions(&mut self) {
        let kept: Vec<bool> = self
            .parked
            .iter()
            .map(|session| session.pinned() || self.runs_sub_agents(session))
            .collect();
        let mut live_idle = self
            .parked
            .iter()
            .zip(&kept)
            .filter(|(session, pinned)| !**pinned && session.active_state.live().is_some())
            .count();
        let mut index = 0;
        while live_idle > RETAINED_IDLE_SESSIONS && index < self.parked.len() {
            if kept[index] {
                index += 1;
                continue;
            }
            if self.parked[index].active_state.live().is_some() {
                let session = self.parked.remove(index);
                let previous = std::mem::replace(&mut self.session, session);
                let snapshot = match self.owned_native_snapshot() {
                    Ok(snapshot) => snapshot,
                    Err(error) => {
                        tracing::warn!(target:"velune_application",event="session_history_credit_refused",detail=%error);
                        let session = std::mem::replace(&mut self.session, previous);
                        self.parked.insert(index, session);
                        index += 1;
                        continue;
                    }
                };
                if let Err(error) = self.shutdown_active() {
                    tracing::warn!(target: "velune_application", event = "session_close_failed", detail = %error);
                }
                if let Some(snapshot) = snapshot {
                    self.session.set_history(snapshot);
                    self.session.history_cache_weight();
                }
                let mut previous = std::mem::replace(&mut self.session, previous);
                previous.invalidate_history_cache_weight();
                self.parked.insert(index, previous);
                live_idle -= 1;
            }
            index += 1;
        }
        self.trim_history_projections();
    }

    fn trim_history_projections(&mut self) {
        // Live-to-history conversion swaps the addressed session back into
        // `parked`; recompute protection against the new order so work,
        // approvals and accepted queue entries remain pinned. This method is
        // also used by visibility updates and performs no native IO.
        let mut history_kept: Vec<bool> = self
            .parked
            .iter()
            .map(|session| session.pinned() || self.runs_sub_agents(session))
            .collect();
        trim_history_sessions(
            &mut self.parked,
            &mut history_kept,
            &self.observed_conversations,
        );
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
    pub(super) fn record_finished_turn(&mut self) -> Option<crate::TurnOutcome> {
        if self.session.native_controls.awaiting_turn_acceptance() {
            return None;
        }
        let view = self.session.projected()?;
        let activity = view.executions.current.as_ref()?.clone();
        if !activity.ended || self.session.finished_activity.as_ref() == Some(&activity.id) {
            return None;
        }
        let record = view
            .executions
            .records
            .iter()
            .find(|r| r.id == activity.execution_id)?;
        let outcome = record.outcome;
        let maintenance = record.kind == velune_conversation::ExecutionKind::ContextMaintenance;
        self.session.finished_activity = Some(activity.id);
        self.session.cancel_requested = false;
        self.complete_turn_observation(match outcome {
            Some(crate::TurnOutcome::Completed) => velune_telemetry::Outcome::Success,
            Some(crate::TurnOutcome::Failed) => velune_telemetry::Outcome::Failure,
            Some(crate::TurnOutcome::Cancelled) => velune_telemetry::Outcome::Cancelled,
            None => velune_telemetry::Outcome::Unknown,
        });
        let outcome = outcome?;
        if maintenance || self.session.side.is_some() {
            return None;
        }
        let silent = self.settle_scheduled_turn(outcome);
        let view = self.session.projected()?;
        let finished = &mut self.finished_turns;
        finished.sequence += 1;
        finished.turns.push_back(crate::FinishedTurn {
            silent,
            sequence: finished.sequence,
            context_runtime_id: view.conversation.runtime_id.clone(),
            conversation_id: self.session.conversation_id().unwrap_or_default().into(),
            outcome,
        });
        if finished.turns.len() > RETAINED_FINISHED_TURNS {
            finished.turns.pop_front();
        }
        Some(outcome)
    }

    pub(crate) fn conversation_activity(&mut self, since: u64) -> crate::ConversationActivity {
        crate::ConversationActivity {
            active: self.active_conversations(),
            finished: self
                .finished_turns
                .turns
                .iter()
                .filter(|turn| turn.sequence > since)
                .cloned()
                .collect(),
            resumable: std::iter::once(&self.session)
                .chain(&self.parked)
                .filter(|session| session.side.is_none())
                .filter_map(|session| {
                    let view = session.projected()?;
                    let conversation_id = session.conversation_id()?;
                    self.resumable(session, view)
                        .then(|| crate::ConversationReference {
                            native_read_state: view.conversation.native_read_state.clone(),
                            runtime_id: view.conversation.runtime_id.clone(),
                            conversation_id: conversation_id.into(),
                        })
                })
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
            let Some(view) = session.projected().filter(|_| session.side.is_none()) else {
                continue;
            };
            let id = session.conversation_id().unwrap_or_default();
            let pending = view
                .pending_interactions
                .iter()
                .filter(|interaction| {
                    interaction.blocks_turn() && self.interaction_is_visible(id, interaction)
                })
                .count();
            let observation = session.work();
            let work = observation
                .as_ref()
                .map_or(&[][..], |tasks| tasks.as_slice());
            let local_work = observation.is_err()
                || work.iter().any(|task| task.conversation_id.is_none())
                || (work.is_empty() && !session.work_settled());
            if session.turn_running() || pending > 0 || local_work {
                active.push(crate::ActiveConversation {
                    context_runtime_id: view.conversation.runtime_id.clone(),
                    conversation_id: id.into(),
                    run_state: if local_work && !session.turn_running() {
                        if !work.is_empty()
                            && work.iter().all(|task| {
                                task.state == velune_agent_runtime::work::BackgroundState::Stopping
                            })
                        {
                            RunState::Stopping
                        } else {
                            RunState::Running
                        }
                    } else {
                        view.run_state.clone()
                    },
                    pending_interactions: pending as u32,
                    parent_conversation_id: None,
                    blocks_idle_submission: session.turn_running() || pending > 0,
                });
            }
            active.extend(work.iter().filter_map(|task| {
                task.conversation_id
                    .as_ref()
                    .cloned()
                    .map(|conversation_id| crate::ActiveConversation {
                        context_runtime_id: view.conversation.runtime_id.clone(),
                        conversation_id,
                        run_state: match task.state {
                            velune_agent_runtime::work::BackgroundState::Stopping => {
                                RunState::Stopping
                            }
                            _ => RunState::Running,
                        },
                        pending_interactions: 0,
                        parent_conversation_id: Some(id.into()),
                        blocks_idle_submission: true,
                    })
            }));
        }
        for mut item in self.elsewhere_conversations() {
            if !active
                .iter()
                .any(|known| known.conversation_id == item.conversation_id)
            {
                item.parent_conversation_id = self.sub_agent_parent(&item.conversation_id);
                if !self.child_finished_by_local_runtime(
                    item.parent_conversation_id.as_deref(),
                    &item.conversation_id,
                ) {
                    active.push(item);
                }
            }
        }
        active
    }
}

/// Drop least-recently-used reopenable history sessions until both limits
/// hold. The addressed session and every pinned parked session are protected.
pub(super) fn trim_history_sessions(
    parked: &mut Vec<Session>,
    kept: &mut Vec<bool>,
    observed: &std::collections::BTreeSet<(String, String)>,
) {
    let mut history_count = 0usize;
    let mut history_bytes = 0usize;
    for session in parked.iter_mut() {
        if matches!(session.active_state, ActiveState::History(_)) {
            history_count += 1;
            history_bytes = history_bytes.saturating_add(session.history_cache_weight());
        }
    }
    let mut index = 0;
    while (history_count > RETAINED_HISTORY_SESSIONS || history_bytes > RETAINED_HISTORY_BYTES)
        && index < parked.len()
    {
        if kept[index]
            || !matches!(parked[index].active_state, ActiveState::History(_))
            || observed
                .iter()
                .any(|(runtime_id, id)| parked[index].observes(runtime_id, id))
        {
            index += 1;
            continue;
        }
        let removed = parked.remove(index);
        kept.remove(index);
        history_count -= 1;
        history_bytes = history_bytes.saturating_sub(
            removed
                .history_cache_weight
                .map_or(0, |(_, _, bytes)| bytes),
        );
        index = 0;
    }
}

#[cfg(test)]
mod history_cache_tests {
    use super::*;

    fn history_session(id: &str, weight: usize) -> Session {
        let mut snapshot: crate::conversation::ConversationSnapshot = serde_json::from_value(
            serde_json::json!({
                "revision": 1,
                "conversation": {"id": id, "title": {"source": "untitled"}, "runtimeId": "synthetic"},
                "contextRuntimeId": "synthetic", "resourceId": null, "modelRecordKey": null,
                "runState": "idle", "messages": [], "transcriptTurns": [], "transcriptItems": [],
                "transcriptOutline": [], "transcriptMessageIdentities": [],
                "actions": {"canSend": true, "canCancel": false, "canSwitch": true}
            }),
        )
        .unwrap();
        snapshot.messages.push(crate::conversation::Message {
            completed: true,
            id: "synthetic-message".into(),
            role: crate::conversation::MessageRole::Assistant,
            timestamp_unix_ms: None,
            blocks: vec![crate::conversation::MessageBlock::Text {
                text: "x".repeat(weight),
            }],
            turn_settings: None,
            turn_usage: None,
            branch_point: None,
        });
        Session {
            active_state: ActiveState::History(Box::new(snapshot.into())),
            ..Session::default()
        }
    }

    #[test]
    fn byte_budget_evicts_oldest_reopenable_history_and_preserves_pinned_views() {
        let mut parked = vec![
            history_session("protected", 20 * 1024 * 1024),
            history_session("oldest", 30 * 1024 * 1024),
            history_session("newest", 20 * 1024 * 1024),
        ];
        let mut kept = vec![true, false, false];

        trim_history_sessions(&mut parked, &mut kept, &std::collections::BTreeSet::new());

        let ids: Vec<_> = parked
            .iter()
            .filter_map(|session| {
                session
                    .projected()
                    .map(|view| view.conversation.id.as_str())
            })
            .collect();
        assert_eq!(ids, ["protected", "newest"]);
        let retained_bytes: usize = parked
            .iter()
            .filter_map(|session| session.history_cache_weight.map(|(_, _, bytes)| bytes))
            .sum();
        assert!(retained_bytes <= RETAINED_HISTORY_BYTES);
        let content_length = |index: usize| {
            let snapshot = parked[index].projected().unwrap();
            let crate::conversation::MessageBlock::Text { text } = &snapshot.messages[0].blocks[0]
            else {
                panic!("synthetic history message should remain complete")
            };
            text.len()
        };
        assert_eq!(content_length(0), 20 * 1024 * 1024);
        assert_eq!(content_length(1), 20 * 1024 * 1024);
    }

    #[test]
    fn visible_history_exceeds_soft_byte_budget_until_its_observation_is_removed() {
        let mut first = history_session("visible-first", 1);
        let mut second = history_session("visible-second", 1);
        let mut hidden = history_session("hidden", 1);
        first.history_cache_weight = Some((1, 1, 40 * 1024 * 1024));
        second.history_cache_weight = Some((1, 1, 40 * 1024 * 1024));
        hidden.history_cache_weight = Some((1, 1, 1024 * 1024));
        let mut parked = vec![first, second, hidden];
        let mut kept = vec![false; parked.len()];
        let visible = std::collections::BTreeSet::from([
            ("synthetic".into(), "visible-first".into()),
            ("synthetic".into(), "visible-second".into()),
        ]);

        trim_history_sessions(&mut parked, &mut kept, &visible);
        assert_eq!(parked.len(), 2);
        let retained = parked
            .iter()
            .filter_map(|session| session.history_cache_weight.map(|(_, _, bytes)| bytes))
            .sum::<usize>();
        assert_eq!(retained, 80 * 1024 * 1024);
        assert!(
            parked
                .iter()
                .all(|session| session.projected().is_some_and(|snapshot| {
                    visible.contains(&(
                        snapshot.context_runtime_id.clone(),
                        session.conversation_id().unwrap().to_owned(),
                    ))
                }))
        );

        let remaining =
            std::collections::BTreeSet::from([("synthetic".into(), "visible-second".into())]);
        kept = vec![false; parked.len()];
        trim_history_sessions(&mut parked, &mut kept, &remaining);
        assert_eq!(parked.len(), 1);
        assert_eq!(parked[0].conversation_id(), Some("visible-second"));
    }

    #[test]
    fn parked_weight_is_invalidated_after_same_revision_message_growth() {
        let mut session = history_session("synthetic", 32);
        let before = session.history_cache_weight();
        let ActiveState::History(snapshot) = &session.active_state else {
            unreachable!()
        };
        let mut replacement = snapshot.try_clone().unwrap();
        let crate::conversation::MessageBlock::Text { text } =
            &mut replacement.messages[0].blocks[0]
        else {
            unreachable!()
        };
        text.push_str(&"y".repeat(1024));
        assert_eq!(replacement.revision, 1, "the source may reuse its revision");
        assert_eq!(
            replacement.messages.len(),
            1,
            "the message count is unchanged"
        );

        session.set_history(replacement);
        assert!(session.history_cache_weight.is_none());
        assert!(session.history_cache_weight() > before);
    }

    #[cfg(unix)]
    #[test]
    fn history_eviction_does_not_reset_shared_control_observation() {
        use std::{
            fs,
            time::{Duration, Instant},
        };

        let agent_dir =
            std::env::temp_dir().join(format!("velune-memory-control-{}", std::process::id()));
        let locks = agent_dir.join("thread-writer-locks");
        fs::create_dir_all(&locks).unwrap();
        let native_id = "aaaaaaaa-aaaa-aaaa-aaaa-aaaaaaaaaaaa";
        let target = velune_agent_runtime::control::Target {
            execution: velune_agent_runtime::execution::ExecutionTarget::Local,
            os_home: None,
            kind: velune_agent_runtime::version::RuntimeKind::Codex166,
            agent_dir: agent_dir.clone(),
            native_id: native_id.into(),
            node_binary: PathBuf::from("/usr/bin/node"),
            resources: PathBuf::from("/synthetic/resources"),
        };
        let oversized = history_session("oversized", RETAINED_HISTORY_BYTES + 1);
        let observations = control::ControlObservations::new().unwrap();
        assert_eq!(
            observations.access(target.clone()),
            crate::conversation::InputAccess::CheckingControl
        );
        let mut parked = vec![oversized];
        let mut kept = vec![false];
        trim_history_sessions(&mut parked, &mut kept, &std::collections::BTreeSet::new());
        assert!(
            parked.is_empty(),
            "the transcript exceeded the byte budget and was evicted"
        );
        let deadline = Instant::now() + Duration::from_secs(2);
        let status = loop {
            let status = observations.access(target.clone());
            if status != crate::conversation::InputAccess::CheckingControl
                || Instant::now() >= deadline
            {
                break status;
            }
            std::thread::sleep(Duration::from_millis(10));
        };
        assert_eq!(status, crate::conversation::InputAccess::Editable);
        fs::remove_dir_all(agent_dir).unwrap();
    }
}
