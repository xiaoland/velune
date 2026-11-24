//! Best-effort live view of native conversations on this machine: an open
//! conversation is read again when its native file changes, and whether a
//! conversation is running comes from each Harness's own liveness record (see
//! `velune_agent_runtime::activity`), whoever started the turn. The Harness
//! stays the conversation's source of truth; whatever this misses, a manual
//! reload of the list and conversation still shows.
use super::*;
use crate::{SnapshotResult, local::conversation_history::LoadedConversationHistory};
use std::time::{Duration, Instant};
use velune_agent_runtime::activity::{self, Processes, SessionFormat, SessionStore, Signature};

const SOURCE_CHECK_INTERVAL: Duration = Duration::from_secs(1);
/// Keep failed or queued source refreshes retryable without letting ordinary
/// snapshot polling spin the history workers while a native source is slow.
const SOURCE_REFRESH_RETRY_INTERVAL: Duration = Duration::from_secs(2);
/// A source that could not be located is looked for again after this.
const SOURCE_LOCATE_INTERVAL: Duration = Duration::from_secs(10);
/// Velune's own execution may still write native records after its turn.
const OWN_WRITES_SETTLE: Duration = Duration::from_secs(5);
/// Native liveness is read again at most this often; a Harness process can
/// start or end a turn without Velune noticing a file change first.
const LIVENESS_INTERVAL: Duration = Duration::from_secs(5);

/// The native file behind a session's conversation.
pub(super) struct SourceWatch {
    conversation_id: String,
    path: Option<PathBuf>,
    signature: Option<Signature>,
    next_check: Instant,
    settle_until: Instant,
    pub(super) refresh_generation: u64,
    pub(super) refresh_requested: bool,
    pub(super) refresh_pending: bool,
}

/// Conversations that Harness processes outside Velune's own execution are
/// running, from the latest read of native liveness.
#[derive(Default)]
pub(super) struct ElsewhereScan {
    at: Option<Instant>,
    found: Vec<crate::ActiveConversation>,
}

impl CoreRuntime {
    /// The latest scan found the conversation running outside Velune's own
    /// execution.
    pub(super) fn runs_elsewhere(&self, conversation_id: &str) -> bool {
        self.elsewhere
            .found
            .iter()
            .any(|item| item.conversation_id == conversation_id)
    }
    /// Native sessions of a runtime on this machine, when Velune knows their
    /// layout.
    pub(super) fn session_store(&self, runtime: &RuntimeInstance) -> Option<SessionStore> {
        let format = SessionFormat::for_runtime(runtime.type_id)?;
        let context = self.execution_context(runtime).ok()?;
        if !context.target.is_local() {
            return None;
        }
        let sessions = match format {
            SessionFormat::Codex => context.agent_dir.join("sessions"),
            SessionFormat::ClaudeCode => context.agent_dir.join("projects"),
            SessionFormat::Pi => setting_path_optional(runtime, "sessionDir")
                .unwrap_or_else(|| context.agent_dir.join("sessions")),
        };
        Some(SessionStore {
            format,
            agent_dir: context.agent_dir,
            sessions,
        })
    }

    /// Follow the addressed session's native file: read the conversation
    /// again after a change that Velune's own execution did not make. Native
    /// liveness is sampled separately from history delivery.
    pub(super) fn follow_source(&mut self) {
        if let Err(error) = self.try_follow_source() {
            tracing::warn!(target: "velune_application", event = "native_source_follow_failed", detail = %error);
        }
    }

    fn try_follow_source(&mut self) -> Result<(), RuntimeError> {
        let now = Instant::now();
        // Continued conversations span several native sources; they are
        // reloaded manually. A side chat's copy follows no native source.
        if self.session.owns_native_control()
            || self.session.native_controls.enabled
            || self.session.native_controls.cleanup_required
            || self.session.logical_projection.is_some()
            || self.session.unsubmitted_draft.is_some()
            || self.session.side.is_some()
        {
            return Ok(());
        }
        let Some(view) = self.session.projected() else {
            return Ok(());
        };
        let (id, runtime_id) = (
            view.conversation.id.clone(),
            view.conversation.runtime_id.clone(),
        );
        // Native children execute inside the parent's process. Its work ledger
        // is authoritative even when this read-only child has no writer registry.
        let finished_child = view
            .conversation
            .parent_conversation_id
            .as_deref()
            .and_then(|parent| {
                self.parked
                    .iter()
                    .find(|session| session.conversation_id() == Some(parent))
            })
            .is_some_and(|parent| parent_finished_child(parent, &id));
        if finished_child {
            self.clear_finished_child_activity();
        }
        if self
            .session
            .source
            .as_ref()
            .is_none_or(|watch| watch.conversation_id != id)
        {
            self.session.source = Some(SourceWatch {
                conversation_id: id.clone(),
                path: None,
                signature: None,
                next_check: now,
                settle_until: now,
                refresh_generation: 0,
                refresh_requested: false,
                refresh_pending: false,
            });
        }
        // Velune's own execution writes the native records, sub-agents too.
        let busy = self.session.executing();
        let watch = self.session.source.as_ref().expect("watch");
        if now < watch.next_check && !busy {
            return Ok(());
        }
        let path = match watch.path.clone() {
            Some(path) => path,
            None => {
                let located = self
                    .runtime_instance(&runtime_id)
                    .ok()
                    .and_then(|runtime| self.session_store(runtime))
                    .and_then(|store| store.locate(id.strip_prefix(&format!("{runtime_id}:"))?));
                let watch = self.session.source.as_mut().expect("watch");
                let Some(path) = located else {
                    watch.next_check = now + SOURCE_LOCATE_INTERVAL;
                    return Ok(());
                };
                // The view was read from this file; follow changes from here.
                watch.signature = activity::signature(&path);
                watch.path = Some(path.clone());
                path
            }
        };
        let signature = activity::signature(&path);
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
            return Ok(());
        }
        watch.signature = signature;
        watch.refresh_generation = watch.refresh_generation.wrapping_add(1);
        watch.refresh_requested = true;
        watch.next_check = now + SOURCE_CHECK_INTERVAL;
        Ok(())
    }

    /// Called by ApplicationService after a snapshot command. The native read
    /// itself is submitted to the shared bounded HistoryWorkers pool.
    pub(crate) fn take_source_refresh(&mut self) -> Option<(String, String, u64)> {
        if self.session.owns_native_control()
            || self.session.native_controls.enabled
            || self.session.native_controls.cleanup_required
        {
            return None;
        }
        let runtime_id = self.session.projected()?.conversation.runtime_id.clone();
        let watch = self.session.source.as_mut()?;
        if !watch.refresh_requested || watch.refresh_pending || Instant::now() < watch.next_check {
            return None;
        }
        watch.refresh_requested = false;
        watch.refresh_pending = true;
        Some((
            watch.conversation_id.clone(),
            runtime_id,
            watch.refresh_generation,
        ))
    }

    pub(crate) fn finish_source_refresh(
        &mut self,
        loaded: LoadedConversationHistory,
        generation: u64,
    ) -> Result<SnapshotResult, RuntimeError> {
        let Some(current_generation) = self
            .session
            .source
            .as_ref()
            .map(|watch| watch.refresh_generation)
        else {
            return self.snapshot_result();
        };
        if let Some(watch) = self.session.source.as_mut() {
            watch.refresh_pending = false;
        }
        let stale = current_generation != generation
            || self.session.owns_native_control()
            || self.session.native_controls.enabled
            || self.session.native_controls.cleanup_required;
        if stale || self.session.executing() || self.session.requires_live_connection() {
            if let Some(watch) = self.session.source.as_mut() {
                watch.refresh_requested = true;
                watch.next_check = Instant::now() + SOURCE_REFRESH_RETRY_INTERVAL;
            }
            return self.snapshot_result();
        }
        let previous = self.native_snapshot().expect("projected view");
        let path = self
            .session
            .source
            .as_ref()
            .and_then(|watch| watch.path.clone());
        let signature = self
            .session
            .source
            .as_ref()
            .and_then(|watch| watch.signature);
        let (mut snapshot, projection, _) = self.finish_loaded_history(loaded)?;
        self.invalidate_execution()?;
        snapshot.revision = previous.revision.saturating_add(1);
        snapshot.model_record_key = snapshot
            .model_record_key
            .take()
            .or(previous.model_record_key);
        let conversation_id = snapshot.conversation.id.clone();
        self.present_history(snapshot, projection);
        self.session.source = Some(SourceWatch {
            conversation_id,
            path,
            signature,
            next_check: Instant::now() + SOURCE_CHECK_INTERVAL,
            settle_until: Instant::now(),
            refresh_generation: generation,
            refresh_requested: false,
            refresh_pending: false,
        });
        self.snapshot_result()
    }

    pub(crate) fn fail_source_refresh(&mut self, generation: u64) {
        if let Some(watch) = self.session.source.as_mut() {
            Self::defer_source_refresh(watch, generation);
        }
    }

    /// A worker may finish after its originating view was evicted. Session
    /// state is shared across views, so release that generation by conversation
    /// identity instead of depending on the old view still being addressable.
    pub(crate) fn fail_source_refresh_for(
        &mut self,
        runtime_id: &str,
        conversation_id: &str,
        generation: u64,
    ) {
        let session = if self.session.projected().is_some_and(|snapshot| {
            snapshot.conversation.runtime_id == runtime_id
                && snapshot.conversation.id == conversation_id
        }) {
            Some(&mut self.session)
        } else {
            self.parked.iter_mut().find(|session| {
                session.projected().is_some_and(|snapshot| {
                    snapshot.conversation.runtime_id == runtime_id
                        && snapshot.conversation.id == conversation_id
                })
            })
        };
        if let Some(watch) = session.and_then(|session| session.source.as_mut())
            && watch.conversation_id == conversation_id
        {
            Self::defer_source_refresh(watch, generation);
        }
    }

    fn defer_source_refresh(watch: &mut SourceWatch, generation: u64) {
        watch.refresh_pending = false;
        // The signature was already advanced when the change was noticed.
        // Retain this generation on admission/read failure so the same
        // unchanged source is retried after a bounded delay.
        watch.refresh_requested = true;
        if watch.refresh_generation != generation {
            watch.next_check = Instant::now() + SOURCE_CHECK_INTERVAL;
        } else {
            watch.next_check = Instant::now() + SOURCE_REFRESH_RETRY_INTERVAL;
        }
    }

    /// Clear stale child history after its own parent observes completion.
    fn clear_finished_child_activity(&mut self) {
        let should_update = matches!(
            &self.session.active_state,
            ActiveState::History(view) if view.run_state == RunState::Elsewhere
        );
        if should_update && let Some(view) = self.session.history_mut() {
            view.run_state = RunState::Idle;
            view.revision = view.revision.saturating_add(1);
        }
    }

    /// One liveness sample feeds both activity lists and addressed snapshots.
    fn refresh_external_turns(&mut self) {
        let now = Instant::now();
        if self
            .elsewhere
            .at
            .is_none_or(|at| now.duration_since(at) >= LIVENESS_INTERVAL)
        {
            let mut processes = Processes::default();
            let mut found = Vec::new();
            for runtime in self
                .runtime_instances
                .iter()
                .filter(|runtime| runtime.enabled)
            {
                let Some(store) = self.session_store(runtime) else {
                    continue;
                };
                found.extend(store.running(&mut processes).into_iter().map(|native| {
                    crate::ActiveConversation {
                        context_runtime_id: runtime.id.clone(),
                        conversation_id: format!("{}:{native}", runtime.id),
                        run_state: RunState::Elsewhere,
                        pending_interactions: 0,
                        parent_conversation_id: None,
                        blocks_idle_submission: true,
                    }
                }));
            }
            self.elsewhere = ElsewhereScan {
                at: Some(now),
                found,
            };
        }
    }

    /// The native observation is also made for Velune's own Harness. Exclude
    /// its current activity and evidence sampled during the local settling period.
    fn external_turn_unowned(&self, item: &crate::ActiveConversation) -> bool {
        let Some(scanned) = self.elsewhere.at else {
            return false;
        };
        !std::iter::once(&self.session)
            .chain(&self.parked)
            .any(|session| {
                session.projected().is_some_and(|snapshot| {
                    snapshot.conversation.id == item.conversation_id
                        && snapshot.conversation.runtime_id == item.context_runtime_id
                }) && (session.executing()
                    || session
                        .last_execution
                        .is_some_and(|at| at + OWN_WRITES_SETTLE >= scanned))
            })
    }

    fn external_turn_is_active(&self, runtime_id: &str, native_id: &str) -> bool {
        self.elsewhere.found.iter().any(|item| {
            item.context_runtime_id == runtime_id
                && item.conversation_id == native_id
                && self.external_turn_unowned(item)
        })
    }

    pub(super) fn observe_current_external_turn(&mut self) -> bool {
        if !matches!(self.session.active_state, ActiveState::History(_))
            || self.session.side.is_some()
            || self.session.unsubmitted_draft.is_some()
        {
            return false;
        }
        self.refresh_external_turns();
        self.session.projected().is_some_and(|snapshot| {
            self.external_turn_is_active(
                &snapshot.conversation.runtime_id,
                &snapshot.conversation.id,
            ) && !self.child_finished_by_local_runtime(
                snapshot.conversation.parent_conversation_id.as_deref(),
                &snapshot.conversation.id,
            )
        })
    }

    pub(super) fn child_finished_by_local_runtime(
        &self,
        parent_id: Option<&str>,
        child: &str,
    ) -> bool {
        let Some(parent_id) = parent_id else {
            return false;
        };
        std::iter::once(&self.session)
            .chain(&self.parked)
            .find(|parent| {
                parent.conversation_id() == Some(parent_id) && parent.active_state.live().is_some()
            })
            .is_some_and(|parent| parent_finished_child(parent, child))
    }

    pub(super) fn elsewhere_conversations(&mut self) -> Vec<crate::ActiveConversation> {
        self.refresh_external_turns();
        self.elsewhere
            .found
            .iter()
            .filter(|item| self.external_turn_unowned(item))
            .filter_map(|item| {
                let matches = |segment: &crate::config::ConversationSegmentReference| {
                    segment.runtime_instance_id == item.context_runtime_id
                        && segment.native_conversation_id == item.conversation_id
                        && !segment.deleted
                };
                if let Some(link) = self
                    .conversation_links
                    .iter()
                    .find(|link| link.segments.last().is_some_and(matches))
                {
                    let mut item = item.clone();
                    item.conversation_id = link.id.clone();
                    Some(item)
                } else if self
                    .conversation_links
                    .iter()
                    .any(|link| link.segments.iter().any(matches))
                {
                    // Older segments are sealed portions of another visible conversation.
                    None
                } else {
                    Some(item.clone())
                }
            })
            .collect()
    }
}

fn parent_finished_child(parent: &super::session::Session, child: &str) -> bool {
    let Ok(work) = parent.work() else {
        return false;
    };
    !work
        .iter()
        .any(|task| task.conversation_id.as_deref() == Some(child))
        // A parent without a live connection has nothing left running; a live
        // one must have observed its work settle.
        && (parent.active_state.live().is_none()
            || parent.work_status() == velune_agent_runtime::work::WorkStatus::Settled
            || (!work.is_empty()
                && work.iter().all(|task| task.conversation_id.is_some())
                && !parent
                    .active_state
                    .live()
                    .is_some_and(LiveConversation::has_pending_metadata)))
}

#[cfg(all(test, unix))]
mod activity_tests;

#[cfg(test)]
mod pi_child_tests {
    use super::*;
    use crate::local::session::Session;
    use serde_json::json;

    fn drain_owners_until(core: &mut CoreRuntime, ready: impl Fn(&CoreRuntime) -> bool) {
        let deadline = Instant::now() + Duration::from_secs(4);
        loop {
            // Child control can leave the parent's operation on a parked worker.
            core.for_each_session(CoreRuntime::drain_runtime).unwrap();
            if ready(core) {
                return;
            }
            assert!(
                Instant::now() < deadline,
                "Pi owners did not reach the expected confirmed state"
            );
            std::thread::yield_now();
        }
    }

    #[test]
    fn failed_source_refresh_retries_same_generation_after_backoff() {
        let mut nonce = [0; 8];
        getrandom::fill(&mut nonce).unwrap();
        let root = std::env::temp_dir().join(format!(
            "velune-source-refresh-retry-{:x}",
            u64::from_le_bytes(nonce)
        ));
        std::fs::create_dir_all(&root).unwrap();
        let mut application = crate::Application::open(crate::Options {
            home_directory: root.clone(),
            resources_directory: root.clone(),
        })
        .unwrap();
        let summary = crate::conversation::ConversationSummary {
            native_read_state: None,
            id: "pi:synthetic".into(),
            runtime_id: "pi".into(),
            title: crate::conversation::ConversationTitle::Untitled,
            cwd: None,
            created_at_unix_ms: None,
            updated_at_unix_ms: None,
            can_rename: false,
            can_delete: false,
            parent_conversation_id: None,
        };
        let snapshot = PiProjection::new(summary).snapshot.unwrap();
        let core = &mut application.runtime;
        core.session.active_state = ActiveState::History(Box::new(snapshot.into()));
        core.session.source = Some(SourceWatch {
            conversation_id: "pi:synthetic".into(),
            path: None,
            signature: None,
            next_check: Instant::now(),
            settle_until: Instant::now(),
            refresh_generation: 7,
            refresh_requested: false,
            refresh_pending: true,
        });

        core.fail_source_refresh(7);
        let watch = core.session.source.as_ref().unwrap();
        assert!(watch.refresh_requested);
        assert!(!watch.refresh_pending);
        assert!(watch.next_check > Instant::now());
        assert!(core.take_source_refresh().is_none());

        core.session.source.as_mut().unwrap().next_check = Instant::now() - Duration::from_secs(1);
        let (id, runtime, generation) = core.take_source_refresh().unwrap();
        assert_eq!(
            (id.as_str(), runtime.as_str(), generation),
            ("pi:synthetic", "pi", 7)
        );
        assert!(core.session.source.as_ref().unwrap().refresh_pending);
        application.close().unwrap();
        std::fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn persisted_pi_notification_receipts_clear_activity_without_enabling_an_idle_queue() {
        for (delivered, running, turn_running) in [
            (false, false, false),
            (true, false, false),
            (true, true, false),
            (true, false, true),
        ] {
            let mut nonce = [0; 8];
            getrandom::fill(&mut nonce).unwrap();
            let root = std::env::temp_dir().join(format!(
                "velune-pi-notification-activity-{:x}",
                u64::from_le_bytes(nonce)
            ));
            std::fs::create_dir_all(&root).unwrap();
            if delivered {
                std::fs::write(root.join("delivered"), "").unwrap();
            }
            if running {
                std::fs::write(root.join("running"), "").unwrap();
            }
            if turn_running {
                std::fs::write(root.join("turn-active"), "").unwrap();
            }
            let helper = root.join("rpc.py");
            std::fs::write(&helper, r#"
import json,pathlib,sys
root=pathlib.Path(__file__).parent
for line in sys.stdin:
 r=json.loads(line); kind=r['type']; data={}
 if kind=='get_commands': data={'commands':[{'name':'velune-background-work'}]}
 elif kind=='prompt':
  frame={'version':1,'kind':'snapshot','tasks':[{'id':'before-compaction','status':'completed','notifyOnCompletion':True,'notified':True}],
         'persistedNotificationIds':['before-compaction'] if (root/'delivered').exists() else []}
  if (root/'running').exists(): frame['tasks'].append({'id':'still-alive','status':'running'})
  if (root/'turn-active').exists(): print(json.dumps({'type':'agent_start'}),flush=True)
  print(json.dumps({'type':'extension_ui_request','method':'setStatus','statusKey':'velune.background-work-v1','statusText':json.dumps(frame)}),flush=True)
  data={'disposition':'handled'}
 print(json.dumps({'type':'response','id':r['id'],'command':kind,'success':True,'data':data}),flush=True)
"#).unwrap();
            let mut application = crate::Application::open(crate::Options {
                home_directory: root.clone(),
                resources_directory: root.clone(),
            })
            .unwrap();
            let config: PiConfig = serde_json::from_value(json!({
                "execution":{"kind":"local"},"os_home":root,
                "binary":"/usr/bin/python3","node_binary":"/usr/bin/python3","rpc_entry":helper,
            }))
            .unwrap();
            let mut pi = pi::PiSession::new(config.clone());
            pi.start(
                pi::Client::spawn(config).unwrap(),
                PiProjection::new(crate::conversation::ConversationSummary {
                    native_read_state: None,
                    id: "pi:synthetic".into(),
                    runtime_id: "pi".into(),
                    title: crate::conversation::ConversationTitle::Untitled,
                    cwd: None,
                    created_at_unix_ms: None,
                    updated_at_unix_ms: None,
                    can_rename: false,
                    can_delete: false,
                    parent_conversation_id: None,
                }),
            );
            let core = &mut application.runtime;
            core.runtime_instances = vec![RuntimeInstance {
                enabled: true,
                id: "pi".into(),
                name: "Synthetic Pi".into(),
                type_id: RuntimeKind::Pi,
                gateway_id: String::new(),
                settings: Default::default(),
            }];
            core.session.active_state = ActiveState::Pi(Box::new(pi));
            core.session.execution_runtime_id = Some("pi".into());
            // This test addresses the owned adapter's state, not external processes.
            core.elsewhere = ElsewhereScan {
                at: Some(Instant::now()),
                found: Vec::new(),
            };
            assert!(
                core.session.executing(),
                "startup discovery is still pending"
            );
            drain_owners_until(core, |core| {
                let live = core.session.active_state.live().unwrap();
                let Ok(tasks) = live.background_tasks() else {
                    return false;
                };
                tasks.live
                    && tasks
                        .tasks
                        .iter()
                        .any(|task| task.id == "before-compaction")
                    && live.turn_running() == turn_running
                    && (!delivered || !live.has_pending_metadata())
            });
            assert_eq!(
                core.session.executing(),
                turn_running || running || !delivered
            );
            let view = core.current_snapshot().unwrap();
            assert_eq!(
                view.run_state,
                if turn_running {
                    RunState::Running
                } else {
                    RunState::Idle
                }
            );
            assert_eq!(view.actions.can_send, !turn_running);
            assert_eq!(
                view.follow_up.can_queue, turn_running,
                "only an active parent turn offers queue instead of normal send"
            );
            let activity = core.active_conversations();
            let active = activity
                .iter()
                .find(|item| item.conversation_id == "pi:synthetic");
            assert_eq!(active.is_some(), turn_running || running || !delivered);
            assert_eq!(
                active.is_some_and(|item| item.blocks_idle_submission),
                turn_running,
                "only native turn activity revokes stale idle admission; background and notification handoff do not"
            );
            core.shutdown_active().unwrap();
            std::fs::remove_dir_all(root).unwrap();
        }
    }

    #[test]
    fn public_cancel_routes_to_owned_pi_child_without_starting_or_aborting_sessions() {
        let mut nonce = [0; 8];
        getrandom::fill(&mut nonce).unwrap();
        let root = std::env::temp_dir().join(format!(
            "velune-pi-child-control-{:x}",
            u64::from_le_bytes(nonce)
        ));
        std::fs::create_dir_all(&root).unwrap();
        let helper = root.join("rpc.py");
        std::fs::write(&helper, r#"
import base64,json,pathlib,sys
root=pathlib.Path(__file__).parent
stopped=False
for line in sys.stdin:
 r=json.loads(line); kind=r['type']; data={}
 with (root/'calls').open('a') as f:f.write(kind+'\n')
 if kind=='get_commands':data={'commands':[{'name':'velune-background-work'}]}
 elif kind=='prompt':
  assert r['message'].startswith('/velune-background-work ')
  encoded=r['message'].split(' ')[1]; command=json.loads(base64.urlsafe_b64decode(encoded+'='*((-len(encoded))%4)))
  with (root/'calls').open('a') as f:f.write('operation:'+command['operation']+'\n')
  if command['operation']=='stop_child':
   assert command['sessionFile']==str(root/'child.jsonl')
   stopped=True
   (root/'stopped').write_text(command['sessionFile'])
  children=[{'sessionFile':str(root/'child.jsonl'),'state':'stopping' if stopped else 'running','canStop':not stopped}, {'sessionFile':str(root/'sibling.jsonl'),'state':'running','canStop':True}]
  frame={'version':1,'kind':'snapshot','tasks':[{'id':'shell','status':'running'}],'children':children}
  if (root/'unknown').exists():frame.update(children=[],childrenUnknown=True)
  print(json.dumps({'type':'extension_ui_request','method':'setStatus','statusKey':'velune.background-work-v1','statusText':json.dumps(frame)}),flush=True)
  data={'disposition':'handled'}
 elif kind=='get_messages':data={'messages':[]}
 print(json.dumps({'type':'response','id':r['id'],'command':kind,'success':True,'data':data}),flush=True)
"#).unwrap();
        let mut application = crate::Application::open(crate::Options {
            home_directory: root.clone(),
            resources_directory: root.clone(),
        })
        .unwrap();
        let summary = |id: String, parent| crate::conversation::ConversationSummary {
            id,
            runtime_id: "pi".into(),
            title: crate::conversation::ConversationTitle::Untitled,
            cwd: Some(root.to_string_lossy().into_owned()),
            created_at_unix_ms: None,
            updated_at_unix_ms: None,
            can_rename: false,
            can_delete: false,
            parent_conversation_id: parent,
            native_read_state: None,
        };
        let parent_id = "pi:parent".to_string();
        let child_id = format!("pi:{}", root.join("child.jsonl").display());
        let sibling_id = format!("pi:{}", root.join("sibling.jsonl").display());
        let config: PiConfig =
            serde_json::from_value(json!({"execution":{"kind":"local"},"os_home":root,
            "binary":"/usr/bin/python3","node_binary":"/usr/bin/python3","rpc_entry":helper}))
            .unwrap();
        let client = pi::Client::spawn(config.clone()).unwrap();
        let mut pi = pi::PiSession::new(config);
        pi.start(client, PiProjection::new(summary(parent_id.clone(), None)));
        let parent = Session {
            active_state: ActiveState::Pi(Box::new(pi)),
            execution_runtime_id: Some("pi".into()),
            ..Default::default()
        };
        let mut child = PiProjection::new(summary(child_id.clone(), Some(parent_id)))
            .snapshot
            .unwrap();
        child.actions.can_send = false;
        child.run_state = RunState::Elsewhere;
        let core = &mut application.runtime;
        core.runtime_instances = vec![RuntimeInstance {
            enabled: true,
            id: "pi".into(),
            name: "Synthetic Pi".into(),
            type_id: RuntimeKind::Pi,
            gateway_id: String::new(),
            settings: Default::default(),
        }];
        core.parked.push(parent);
        core.session.active_state = ActiveState::History(Box::new(child.into()));
        core.session.execution_runtime_id = Some("pi".into());
        let later = Instant::now() + Duration::from_secs(60);
        core.session.source = Some(SourceWatch {
            conversation_id: child_id.clone(),
            path: None,
            signature: None,
            next_check: later,
            settle_until: later,
            refresh_generation: 0,
            refresh_requested: false,
            refresh_pending: false,
        });
        drain_owners_until(core, |core| {
            let parent = core
                .parked
                .iter()
                .find(|s| s.conversation_id() == Some("pi:parent"))
                .unwrap();
            parent.active_state.live().is_some_and(|live| {
                live.background_tasks().is_ok_and(|tasks| tasks.live)
                    && !live.has_pending_metadata()
                    && live.can_stop_child(&child_id)
                    && live.can_stop_child(&sibling_id)
            })
        });
        core.try_follow_source().unwrap();
        let view = core.current_snapshot().unwrap();
        assert_eq!(view.run_state, RunState::Running);
        assert!(view.actions.can_cancel);
        assert!(!view.actions.can_send);
        assert_eq!(
            view.actions.input_access,
            crate::conversation::InputAccess::ReadOnly
        );
        let response = application
            .execute_command(
                crate::Command::Cancel {
                    runtime_id: "pi".into(),
                    conversation_id: child_id.clone(),
                },
                false,
                &crate::ClientViewId("synthetic".into()),
            )
            .unwrap();
        let crate::Response::Cancel(result) = response else {
            panic!("wrong response")
        };
        assert!(result.snapshot.is_some());
        drain_owners_until(&mut application.runtime, |core| {
            let parent = core
                .parked
                .iter()
                .find(|s| s.conversation_id() == Some("pi:parent"))
                .unwrap();
            parent.active_state.live().is_some_and(|live| {
                !live.has_pending_metadata()
                    && !live.can_stop_child(&child_id)
                    && live.work().is_ok_and(|work| {
                        work.iter().any(|work| {
                            work.conversation_id.as_deref() == Some(child_id.as_str())
                                && work.state
                                    == velune_agent_runtime::work::BackgroundState::Stopping
                        })
                    })
            })
        });
        let view = application.runtime.current_snapshot().unwrap();
        assert_eq!(view.run_state, RunState::Stopping);
        assert!(!view.actions.can_cancel);
        assert_eq!(
            std::fs::read_to_string(root.join("stopped")).unwrap(),
            root.join("child.jsonl").to_string_lossy()
        );
        let parent = application
            .runtime
            .parked
            .iter()
            .find(|s| s.conversation_id() == Some("pi:parent"))
            .unwrap();
        assert!(!parent.cancel_requested);
        assert!(
            parent
                .work()
                .unwrap()
                .iter()
                .any(|work| work.conversation_id.as_deref() == Some(&sibling_id))
        );
        assert!(
            parent
                .work()
                .unwrap()
                .iter()
                .any(|work| work.conversation_id.is_none())
        );
        let error = application.execute_command(
            crate::Command::Cancel {
                runtime_id: "pi".into(),
                conversation_id: child_id,
            },
            false,
            &crate::ClientViewId("synthetic".into()),
        );
        assert!(
            error.is_err(),
            "already stopping child cannot be stopped twice"
        );
        let calls = std::fs::read_to_string(root.join("calls")).unwrap();
        assert_eq!(
            calls
                .lines()
                .filter(|line| *line == "operation:stop_child")
                .count(),
            1
        );
        assert!(
            !calls
                .lines()
                .any(|line| matches!(line, "abort" | "switch_session" | "new_session"))
        );
        // Unknown/unmapped work must not turn a running child into idle, or
        // authorize cancellation through a remembered, stale route.
        std::fs::write(root.join("unknown"), "unknown").unwrap();
        let parent = &mut application.runtime.parked[0];
        let ActiveState::Pi(pi) = &mut parent.active_state else {
            unreachable!()
        };
        let config = pi.config().clone();
        let parent_summary = pi.snapshot().unwrap().conversation.clone();
        pi.shutdown().unwrap();
        pi.start(
            pi::Client::spawn(config).unwrap(),
            PiProjection::new(parent_summary),
        );
        if let ActiveState::History(view) = &mut application.runtime.session.active_state {
            view.run_state = RunState::Elsewhere;
        }
        drain_owners_until(&mut application.runtime, |core| {
            let parent = &core.parked[0];
            parent.active_state.live().is_some_and(|live| {
                let Ok(tasks) = live.background_tasks() else {
                    return false;
                };
                !tasks.live
                    && tasks.issue.is_none()
                    && tasks.tasks.iter().any(|task| task.id == "shell")
                    && live
                        .work()
                        .is_ok_and(|work| work.iter().all(|work| work.conversation_id.is_none()))
                    && !live.can_stop_child(&view.conversation.id)
            })
        });
        application.runtime.try_follow_source().unwrap();
        let view = application.runtime.current_snapshot().unwrap();
        assert_eq!(view.run_state, RunState::Elsewhere);
        assert!(!view.actions.can_cancel);
        let external_parent = application.runtime.parked.remove(0);
        let child_id = view.conversation.id;
        assert!(
            application
                .execute_command(
                    crate::Command::Cancel {
                        runtime_id: "pi".into(),
                        conversation_id: child_id
                    },
                    false,
                    &crate::ClientViewId("synthetic".into())
                )
                .is_err()
        );
        application.runtime.parked.push(external_parent);
        application.runtime.parked[0]
            .active_state
            .live_mut()
            .unwrap()
            .shutdown()
            .unwrap();
        std::fs::remove_dir_all(root).unwrap();
    }
}
