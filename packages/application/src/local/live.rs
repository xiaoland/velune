//! Best-effort live view of native conversations on this machine: an open
//! conversation is read again when its native file changes, and turns that
//! run outside Velune are noticed from native records. The Harness stays the
//! conversation's source of truth; whatever this misses, a manual reload of
//! the list and conversation still shows.
use super::*;
use std::time::{Duration, Instant, SystemTime};
use velune_agent_runtime::activity::{self, SessionFormat, Signature};

/// An unfinished turn written within this window is presented as running
/// elsewhere; an older one is assumed abandoned by a process that ended.
const ELSEWHERE_WINDOW: Duration = Duration::from_secs(10 * 60);
const SOURCE_CHECK_INTERVAL: Duration = Duration::from_secs(1);
/// A source that could not be located is looked for again after this.
const SOURCE_LOCATE_INTERVAL: Duration = Duration::from_secs(10);
/// Velune's own execution may still write native records after its turn.
const OWN_WRITES_SETTLE: Duration = Duration::from_secs(5);
const ELSEWHERE_SCAN_INTERVAL: Duration = Duration::from_secs(5);

/// The native file behind a session's conversation.
pub(super) struct SourceWatch {
    conversation_id: String,
    source: Option<(SessionFormat, PathBuf)>,
    signature: Option<Signature>,
    next_check: Instant,
    settle_until: Instant,
}

/// Conversations running elsewhere, from the latest scan of native files.
#[derive(Default)]
pub(super) struct ElsewhereScan {
    at: Option<Instant>,
    found: Vec<crate::ActiveConversation>,
}

fn recent(signature: Option<Signature>) -> bool {
    signature.is_some_and(|signature| {
        SystemTime::now()
            .duration_since(signature.modified)
            .is_ok_and(|age| age < ELSEWHERE_WINDOW)
            || signature.modified > SystemTime::now()
    })
}

impl CoreRuntime {
    /// Native session files of a runtime on this machine, when Velune knows
    /// their layout.
    fn session_root(&self, runtime: &RuntimeInstance) -> Option<(SessionFormat, PathBuf)> {
        let format = SessionFormat::for_runtime_type(&runtime.type_id)?;
        let context = self.execution_context(runtime).ok()?;
        if !context.target.is_local() {
            return None;
        }
        let root = match format {
            SessionFormat::Codex => context.agent_dir.join("sessions"),
            SessionFormat::ClaudeCode => context.agent_dir.join("projects"),
            SessionFormat::Pi => setting_path_optional(runtime, "sessionDir")
                .unwrap_or_else(|| context.agent_dir.join("sessions")),
        };
        Some((format, root))
    }

    /// Follow the addressed session's native file: read the conversation
    /// again after a change that Velune's own execution did not make, and
    /// present an unfinished recent turn as running elsewhere. Failures only
    /// leave the current view in place.
    pub(super) fn follow_source(&mut self) {
        if let Err(error) = self.try_follow_source() {
            tracing::warn!(target: "velune_application", event = "native_source_follow_failed", detail = %error);
        }
    }

    fn try_follow_source(&mut self) -> Result<(), RuntimeError> {
        let now = Instant::now();
        // Continued conversations span several native sources; they are
        // reloaded manually.
        if self.session.logical_projection.is_some() || self.session.unsubmitted_draft.is_some() {
            return Ok(());
        }
        let Some(view) = self.session.projected() else {
            return Ok(());
        };
        let (id, runtime_id) = (
            view.conversation.id.clone(),
            view.conversation.runtime_id.clone(),
        );
        if self
            .session
            .source
            .as_ref()
            .is_none_or(|watch| watch.conversation_id != id)
        {
            self.session.source = Some(SourceWatch {
                conversation_id: id.clone(),
                source: None,
                signature: None,
                next_check: now,
                settle_until: now,
            });
        }
        let busy = self.session.busy();
        let watch = self.session.source.as_ref().expect("watch");
        if now < watch.next_check && !busy {
            return Ok(());
        }
        let source = match watch.source.clone() {
            Some(source) => source,
            None => {
                let located = self
                    .runtime_instance(&runtime_id)
                    .ok()
                    .and_then(|runtime| self.session_root(runtime))
                    .and_then(|(format, root)| {
                        let native = id.strip_prefix(&format!("{runtime_id}:"))?;
                        activity::locate(format, &root, native).map(|path| (format, path))
                    });
                let watch = self.session.source.as_mut().expect("watch");
                let Some(source) = located else {
                    watch.next_check = now + SOURCE_LOCATE_INTERVAL;
                    return Ok(());
                };
                // The view was read from this file; follow changes from here.
                watch.signature = activity::signature(&source.1);
                watch.source = Some(source.clone());
                if let ActiveState::History(view) = &mut self.session.active_state
                    && recent(watch.signature)
                    && activity::unfinished(source.0, &source.1)
                {
                    view.run_state = RunState::Elsewhere;
                    view.revision = view.revision.saturating_add(1);
                }
                source
            }
        };
        let signature = activity::signature(&source.1);
        let watch = self.session.source.as_mut().expect("watch");
        watch.next_check = now + SOURCE_CHECK_INTERVAL;
        if busy || now < watch.settle_until {
            watch.signature = signature;
            if busy {
                watch.settle_until = now + OWN_WRITES_SETTLE;
            }
            return Ok(());
        }
        if signature == watch.signature {
            // An abandoned turn stops counting as running elsewhere.
            if let ActiveState::History(view) = &mut self.session.active_state
                && view.run_state == RunState::Elsewhere
                && !recent(signature)
            {
                view.run_state = RunState::Idle;
                view.revision = view.revision.saturating_add(1);
            }
            return Ok(());
        }
        watch.signature = signature;
        let elsewhere = recent(signature) && activity::unfinished(source.0, &source.1);
        let started = Instant::now();
        // An idle execution holds the native conversation as Velune last
        // left it; release it so the next turn resumes the changed one.
        let previous = self.native_snapshot().expect("projected view");
        if !matches!(self.session.active_state, ActiveState::History(_)) {
            self.invalidate_execution()?;
        }
        let mut snapshot = self.read_history(&runtime_id, &id)?;
        snapshot.revision = previous.revision.saturating_add(1);
        snapshot.model_record_key = snapshot.model_record_key.or(previous.model_record_key);
        if elsewhere {
            snapshot.run_state = RunState::Elsewhere;
        }
        self.session.active_state = ActiveState::History(Box::new(snapshot));
        // Reading can be slow (Codex reads through its app server); keep
        // following a busy source from using most of the time.
        let watch = self.session.source.as_mut().expect("watch");
        watch.next_check = Instant::now() + SOURCE_CHECK_INTERVAL.max(started.elapsed() * 3);
        Ok(())
    }

    /// Conversations whose native records show a recent unfinished turn that
    /// no Velune session is executing. Scans at most every few seconds.
    pub(super) fn elsewhere_conversations(&mut self) -> Vec<crate::ActiveConversation> {
        let now = Instant::now();
        if self
            .elsewhere
            .at
            .is_none_or(|at| now.duration_since(at) >= ELSEWHERE_SCAN_INTERVAL)
        {
            let since = SystemTime::now() - ELSEWHERE_WINDOW;
            let mut found = Vec::new();
            for runtime in self
                .runtime_instances
                .iter()
                .filter(|runtime| runtime.enabled)
            {
                let Some((format, root)) = self.session_root(runtime) else {
                    continue;
                };
                found.extend(
                    activity::unfinished_since(format, &root, since)
                        .into_iter()
                        .map(|native| crate::ActiveConversation {
                            context_runtime_id: runtime.id.clone(),
                            conversation_id: format!("{}:{native}", runtime.id),
                            run_state: RunState::Elsewhere,
                            pending_interactions: 0,
                        }),
                );
            }
            self.elsewhere = ElsewhereScan {
                at: Some(now),
                found,
            };
        }
        // The scan may have seen a session's own writes: those of its running
        // turn, or those shortly after it ended.
        let scanned = self.elsewhere.at.expect("scanned");
        let executing: Vec<&session::Session> = std::iter::once(&self.session)
            .chain(&self.parked)
            .filter(|session| {
                session.busy()
                    || session
                        .last_execution
                        .is_some_and(|at| at + OWN_WRITES_SETTLE >= scanned)
            })
            .collect();
        self.elsewhere
            .found
            .iter()
            .filter(|item| {
                !executing
                    .iter()
                    .any(|session| session.presents(&item.conversation_id))
            })
            .cloned()
            .collect()
    }
}
