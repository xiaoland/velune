//! One Pi conversation: its launch configuration, and once started its RPC
//! process with the projection of what that process shows.
use crate::conversation::{ConversationSnapshot, ConversationSummary, EditPlacement, RunState};
use crate::work::{BackgroundState, BackgroundTask};
use crate::{Client, Config, Error, PiProjection, Result, TurnInput};
use serde_json::{Value, json};
use std::collections::{BTreeMap, HashSet};

pub struct PiSession {
    config: Config,
    running: Option<Running>,
    background: BTreeMap<String, BackgroundState>,
    pending_notifications: HashSet<String>,
    acknowledged_notifications: HashSet<String>,
    background_pending_sync: bool,
    background_error: Option<String>,
    background_diagnostic_error: Option<String>,
}

struct Running {
    client: Client,
    projection: PiProjection,
    /// A turn or a compaction runs, including an accepted prompt whose
    /// agent_start has not arrived yet.
    busy: bool,
}

impl Running {
    fn apply_event(&mut self, event: &Value) {
        self.projection.apply_event(event);
        if self.projection.turn_ended() {
            self.busy = false;
        } else if event["type"] == "agent_start" {
            // Extensions may start follow-up runs without a prompt from
            // Velune, including while a history RPC is in flight.
            self.busy = true;
        }
    }
}

impl PiSession {
    /// A session ready to start; no process runs yet.
    pub fn new(config: Config) -> Self {
        Self {
            config,
            running: None,
            background: BTreeMap::new(),
            pending_notifications: HashSet::new(),
            acknowledged_notifications: HashSet::new(),
            background_pending_sync: false,
            background_error: None,
            background_diagnostic_error: None,
        }
    }

    pub fn config(&self) -> &Config {
        &self.config
    }

    pub fn config_mut(&mut self) -> &mut Config {
        &mut self.config
    }

    /// Runs `client`, which was spawned from this session's configuration,
    /// shown through `projection`.
    pub fn start(&mut self, client: Client, projection: PiProjection) {
        self.reset_background_session();
        let busy = projection.snapshot.as_ref().is_some_and(|snapshot| {
            matches!(snapshot.run_state, RunState::Running | RunState::Stopping)
        });
        self.running = Some(Running {
            client,
            projection,
            busy,
        });
        self.background_pending_sync = true;
        self.refresh_background(json!({"operation":"snapshot"}));
    }

    /// Stop the process; the configuration stays, without its conversation.
    pub fn stop(&mut self) -> Result<()> {
        let result = self.shutdown();
        self.config.working_dir = None;
        self.config.session = None;
        result
    }

    pub fn shutdown(&mut self) -> Result<()> {
        match self.running.take() {
            Some(mut running) => running.client.shutdown(),
            None => Ok(()),
        }
    }

    pub fn turn_running(&self) -> bool {
        self.running.as_ref().is_some_and(|running| running.busy)
    }

    /// Known Pi-owned tasks that outlive the current parent turn.
    pub fn work(&self) -> Vec<BackgroundTask> {
        self.background
            .values()
            .map(|state| BackgroundTask {
                conversation_id: None,
                state: *state,
            })
            .collect()
    }

    /// True while the Host API snapshot, an error, or a completion
    /// notification handoff is not yet settled.
    pub fn has_pending_metadata(&self) -> bool {
        self.background_pending_sync
            || self.background_error.is_some()
            || !self.pending_notifications.is_empty()
    }

    fn set_background_error(&mut self, error: Option<String>) {
        let changed = self.background_error != error;
        self.background_error = error.clone();
        if changed {
            self.project_background_error();
        }
    }

    fn project_background_error(&mut self) {
        let Some(error) = self.background_error.as_deref() else {
            self.background_diagnostic_error = None;
            return;
        };
        if self.background_diagnostic_error.as_deref() == Some(error) {
            return;
        }
        let Some(running) = &mut self.running else {
            return;
        };
        if running.projection.snapshot.is_none() {
            return;
        }
        tracing::warn!(code = "pi_background_work_unknown", error = %error, "Pi 后台工作状态无法确认");
        running.projection.apply_event(&json!({
            "type":"extension_error",
            "extensionPath":"Velune Pi background work bridge",
            "event":"background-work-sync",
            "error":format!("Velune 后台工作桥接：{error}"),
        }));
        self.background_diagnostic_error = Some(error.to_owned());
    }

    fn refresh_background(&mut self, command: Value) {
        let result = self
            .running
            .as_mut()
            .ok_or(Error::NotStarted)
            .and_then(|running| running.client.background_work_command(&command));
        match result {
            Ok(frame) => self.apply_background_frame(&frame),
            Err(error) => {
                self.background_pending_sync = true;
                self.set_background_error(Some(error.to_string()));
            }
        }
    }

    fn reset_background_session(&mut self) {
        self.background.clear();
        self.pending_notifications.clear();
        self.acknowledged_notifications.clear();
        self.background_pending_sync = true;
        self.set_background_error(None);
        self.background_diagnostic_error = None;
    }

    fn apply_background_frame(&mut self, frame: &Value) {
        if frame["version"] != 1 {
            self.background_pending_sync = true;
            self.set_background_error(Some("Pi 后台工作桥接返回了未知协议版本".into()));
            return;
        }
        match frame["kind"].as_str() {
            Some("unsupported") => {
                self.background.clear();
                self.pending_notifications.clear();
                self.background_pending_sync = false;
                self.set_background_error(None);
            }
            Some("snapshot") => {
                self.background.clear();
                if let Some(tasks) = frame["tasks"].as_array() {
                    for task in tasks {
                        let Some(id) = task["id"].as_str().filter(|id| !id.is_empty()) else {
                            continue;
                        };
                        if task["status"] == "running" {
                            self.background.insert(id.into(), BackgroundState::Running);
                        } else if task["notifyOnCompletion"] == true
                            && !self.acknowledged_notifications.contains(id)
                        {
                            // The plugin calls Pi.sendMessage only after publishing
                            // terminal state. Keep the receiver until Pi actually
                            // reports the corresponding custom message end.
                            self.pending_notifications.insert(id.into());
                        }
                    }
                }
                self.background_pending_sync = false;
                self.set_background_error(None);
                if let Some(results) = frame["stopResults"].as_array()
                    && let Some(failed) =
                        results.iter().find(|result| result["outcome"] == "failed")
                {
                    self.background_pending_sync = true;
                    self.set_background_error(Some(format!(
                        "Pi 后台任务 {} 停止失败",
                        failed["taskId"].as_str().unwrap_or("(unknown)")
                    )));
                }
            }
            Some("unknown") => {
                self.background_pending_sync = true;
                self.set_background_error(Some(
                    frame["error"]
                        .as_str()
                        .unwrap_or("Pi 后台工作状态暂时无法读取")
                        .to_owned(),
                ));
            }
            _ => {
                self.background_pending_sync = true;
                self.set_background_error(Some("Pi 后台工作桥接返回了未知状态".into()));
            }
        }
    }

    fn observe_background_notification(&mut self, event: &Value) {
        // Pi 1.4 starts a triggered run with agent_start before delivering its
        // custom message. The RPC client preserves that order, so observing
        // message_end can release this handoff without losing the new run.
        // A notification with triggerTurn=false has no new run to await.
        if event["type"] != "message_end"
            || event["message"]["customType"] != "background-task-notification"
        {
            return;
        }
        if let Some(task_id) = event["message"]["details"]["id"].as_str() {
            self.acknowledged_notifications.insert(task_id.to_owned());
            self.pending_notifications.remove(task_id);
        }
    }

    fn observe_persisted_background_notifications(
        messages: &Value,
        acknowledged: &mut HashSet<String>,
        pending: &mut HashSet<String>,
    ) {
        let Some(messages) = messages["data"]["messages"].as_array() else {
            return;
        };
        for message in messages {
            if message["customType"] != "background-task-notification" {
                continue;
            }
            if let Some(task_id) = message["details"]["id"].as_str() {
                acknowledged.insert(task_id.to_owned());
                pending.remove(task_id);
            }
        }
    }

    pub fn stop_background(&mut self) -> Result<()> {
        let task_ids: Vec<_> = self.background.keys().cloned().collect();
        if task_ids.is_empty() {
            if self.background_pending_sync || self.background_error.is_some() {
                return Err(Error::Sdk(
                    self.background_error
                        .clone()
                        .unwrap_or_else(|| "Pi 后台工作状态尚未同步，无法安全停止".into()),
                ));
            }
            return Ok(());
        }
        let result = self
            .running
            .as_mut()
            .ok_or(Error::NotStarted)?
            .client
            .stop_background_tasks(&task_ids);
        match result {
            Ok(frame) => {
                self.apply_background_frame(&frame);
                if self.background_error.is_some() {
                    Err(Error::Sdk(
                        self.background_error.clone().unwrap_or_default(),
                    ))
                } else {
                    Ok(())
                }
            }
            Err(error) => {
                self.background_pending_sync = true;
                self.set_background_error(Some(error.to_string()));
                Err(error)
            }
        }
    }

    pub fn snapshot(&self) -> Option<&ConversationSnapshot> {
        self.projection()
            .and_then(|projection| projection.snapshot.as_ref())
    }

    pub fn projection(&self) -> Option<&PiProjection> {
        self.running.as_ref().map(|running| &running.projection)
    }

    /// The last turn failed.
    pub fn turn_failed(&self) -> bool {
        self.projection().is_some_and(PiProjection::turn_failed)
    }

    pub fn restore_context(&mut self, source: &ConversationSnapshot) {
        if let Some(running) = &mut self.running
            && let Some(snapshot) = &mut running.projection.snapshot
        {
            snapshot
                .executions
                .restore(&source.executions, &snapshot.messages);
            snapshot.conversation = source.conversation.clone();
        }
    }

    pub fn set_conversation(&mut self, conversation: ConversationSummary) {
        if let Some(running) = &mut self.running {
            running.projection.set_conversation(conversation);
        }
    }

    /// Read the native history again at the next synchronization.
    pub fn invalidate_history(&mut self) {
        if let Some(running) = &mut self.running {
            running.projection.invalidate_history();
        }
    }

    fn running(&mut self) -> Result<&mut Running> {
        self.running.as_mut().ok_or(Error::NotStarted)
    }

    /// Apply what the process reported since the last poll.
    pub fn poll(&mut self) {
        let Some(running) = &mut self.running else {
            return;
        };
        let events = running.client.poll();
        let mut settled = false;
        for event in events {
            self.observe_background_notification(&event);
            if event["type"] == "extension_ui_request"
                && event["method"] == "setStatus"
                && event["statusKey"] == "velune.background-work-v1"
                && let Some(text) = event["statusText"].as_str()
                && let Ok(frame) = serde_json::from_str::<Value>(text)
            {
                self.apply_background_frame(&frame);
            }
            if let Some(running) = &mut self.running {
                running.apply_event(&event);
                settled |= event["type"] == "agent_settled";
            }
        }
        if settled && !self.turn_running() {
            // Close the admission-coalescing window at the parent turn fence:
            // read the public task snapshot before callers can conclude that
            // an empty cached list is settled and release Pi/the gateway.
            self.background_pending_sync = true;
            self.refresh_background(json!({"operation":"snapshot"}));
        }
    }

    /// Replace the projection's history with the process's authoritative
    /// one, unless a turn runs or the history is already current. Its
    /// conversation identity is `runtime_id`'s and the session file's.
    /// Whether it read the history.
    pub fn sync_history(
        &mut self,
        runtime_id: &str,
        model_record_key: Option<String>,
    ) -> Result<bool> {
        let Some(running) = &mut self.running else {
            return Ok(false);
        };
        if running.busy
            || running.projection.history_synchronized()
            || !running.projection.can_sync_history()
        {
            return Ok(false);
        }
        let (state, messages) = running.client.state()?;
        Self::observe_persisted_background_notifications(
            &messages,
            &mut self.acknowledged_notifications,
            &mut self.pending_notifications,
        );
        // Events received before get_messages' response are covered by that
        // authoritative context. Do not replay them after replacing history.
        for event in running.client.take_buffered_events() {
            running.apply_event(&event);
        }
        let projection = &mut running.projection;
        if let Some(path) = state["data"]["sessionFile"].as_str() {
            let previous = projection.snapshot.as_ref().map(|s| &s.conversation);
            projection.set_conversation(ConversationSummary {
                native_read_state: None,
                can_rename: false,
                can_delete: false,
                id: format!("{runtime_id}:{path}"),
                title: crate::conversation::conversation_title(
                    state["data"]["sessionName"].as_str(),
                    None,
                ),
                created_at_unix_ms: previous.and_then(|c| c.created_at_unix_ms),
                updated_at_unix_ms: previous.and_then(|c| c.updated_at_unix_ms),
                runtime_id: runtime_id.into(),
                cwd: previous.and_then(|c| c.cwd.clone()),
                parent_conversation_id: None,
            });
        }
        projection.replace_history(&messages);
        if let Some(snapshot) = projection.snapshot.as_mut() {
            snapshot.model_record_key = model_record_key;
            snapshot.actions.can_send &= !running.busy;
            snapshot.actions.can_cancel &= running.busy;
        }
        if let Some(error) = self.background_error.clone()
            && self.background_diagnostic_error.as_deref() != Some(error.as_str())
        {
            tracing::warn!(code = "pi_background_work_unknown", error = %error, "Pi 后台工作状态无法确认");
            projection.apply_event(&json!({
                "type":"extension_error",
                "extensionPath":"Velune Pi background work bridge",
                "event":"background-work-sync",
                "error":format!("Velune 后台工作桥接：{error}"),
            }));
            self.background_diagnostic_error = Some(error);
        }
        Ok(true)
    }

    /// Send a user turn. Pi may handle it at once (an extension command);
    /// otherwise the turn runs until its events end it.
    pub fn send(&mut self, input: &TurnInput) -> Result<()> {
        let running = self.running()?;
        let response = running.client.prompt(input)?;
        let busy = response["data"]["disposition"] != "handled";
        if busy {
            running.projection.begin_turn();
            if let Some(snapshot) = running.projection.snapshot.as_mut() {
                snapshot.run_state = RunState::Running;
                snapshot.actions.can_send = false;
                snapshot.actions.can_cancel = true;
            }
        } else {
            running.projection.invalidate_history();
        }
        running.busy = busy;
        Ok(())
    }

    pub fn steer(&mut self, input: &TurnInput) -> Result<()> {
        self.running()?.client.steer(input).map(drop)
    }

    pub fn cancel(&mut self) -> Result<()> {
        self.running()?.client.cancel().map(drop)
    }

    /// Compaction runs like a turn until its result arrives.
    pub fn compact(&mut self) -> Result<()> {
        let running = self.running()?;
        let id = running.client.compact()?;
        running.projection.begin_compaction(id);
        running.busy = true;
        Ok(())
    }

    /// Continue from before the user entry `entry_id`, here or in a new
    /// session as `placement` says.
    pub fn branch(&mut self, entry_id: &str, placement: EditPlacement) -> Result<()> {
        match placement {
            EditPlacement::InPlace => self.running()?.client.rewind(entry_id)?,
            EditPlacement::Fork => {
                self.running()?.client.fork(entry_id)?;
                self.reset_background_session();
                self.refresh_background(json!({"operation":"snapshot"}));
            }
        }
        self.running()?.projection.invalidate_history();
        Ok(())
    }

    /// Continue in a new session holding what precedes the user entry
    /// `entry_id`, or the whole current branch when there is none; the
    /// original session stays as it is.
    pub fn fork_at(&mut self, entry_id: Option<&str>) -> Result<()> {
        match entry_id {
            Some(entry_id) => self.running()?.client.fork(entry_id)?,
            None => self.running()?.client.clone()?,
        }
        self.reset_background_session();
        self.refresh_background(json!({"operation":"snapshot"}));
        self.running()?.projection.invalidate_history();
        Ok(())
    }

    /// Start a new session in the running process.
    pub fn new_session(&mut self) -> Result<()> {
        self.request(json!({"type":"new_session"}))?;
        self.reset_background_session();
        self.refresh_background(json!({"operation":"snapshot"}));
        Ok(())
    }

    pub fn set_name(&mut self, name: &str) -> Result<()> {
        self.request(json!({"type":"set_session_name","name":name}))?;
        let state = self.request(json!({"type":"get_state"}))?;
        if let Some(snapshot) = self.running()?.projection.snapshot.as_mut() {
            snapshot.conversation.title = crate::conversation::conversation_title(
                state["data"]["sessionName"].as_str(),
                None,
            );
            snapshot.revision = snapshot.revision.saturating_add(1);
        }
        self.invalidate_history();
        Ok(())
    }

    /// Select one of Pi's own models.
    pub fn set_model(&mut self, provider: Option<&str>, model_id: &str) -> Result<()> {
        self.request(json!({"type":"set_model","provider":provider,"modelId":model_id}))
            .map(drop)
    }

    fn request(&mut self, command: Value) -> Result<Value> {
        self.running()?.client.request(command)
    }
}

#[cfg(all(test, unix))]
mod tests {
    use super::*;
    use crate::LiveConversation;

    #[test]
    fn background_snapshot_keeps_running_work_and_notification_handoff_separate() {
        let mut config: Config = serde_json::from_value(json!({
            "execution":{"kind":"local"}, "binary":"/bin/pi", "node_binary":"/bin/node",
        }))
        .unwrap();
        config.working_dir = None;
        let mut session = PiSession::new(config);
        session.apply_background_frame(&json!({
            "version":1,
            "kind":"snapshot",
            "tasks":[
                {"id":"running-1","status":"running"},
                {"id":"done-1","status":"completed","notifyOnCompletion":true,"notified":true},
                {"id":"done-2","status":"completed","notifyOnCompletion":false}
            ]
        }));
        let work = session.work();
        assert_eq!(work.len(), 1);
        assert_eq!(work[0].conversation_id, None);
        assert_eq!(work[0].state, BackgroundState::Running);
        assert!(session.has_pending_metadata());
        assert_eq!(session.background_error, None);

        session.observe_background_notification(&json!({
            "type":"message_end",
            "message":{"customType":"background-task-notification","details":{"id":"done-1"}}
        }));
        assert!(!session.has_pending_metadata());

        session.apply_background_frame(&json!({
            "version":1,"kind":"snapshot",
            "tasks":[
                {"id":"running-1","status":"running"},
                {"id":"previous-session-task","status":"completed","notifyOnCompletion":true,"notified":true}
            ]
        }));
        assert!(session.has_pending_metadata());
        PiSession::observe_persisted_background_notifications(
            &json!({"data":{"messages":[
                {"customType":"background-task-notification","details":{"id":"previous-session-task"}}
            ]}}),
            &mut session.acknowledged_notifications,
            &mut session.pending_notifications,
        );
        assert!(!session.has_pending_metadata());

        session.apply_background_frame(&json!({
            "version":1,"kind":"unknown","supported":true,
            "error":"host query failed","tasks":[]
        }));
        assert_eq!(session.work().len(), 1);
        assert!(session.has_pending_metadata());
        assert_eq!(
            session.background_error.as_deref(),
            Some("host query failed")
        );

        session.apply_background_frame(
            &json!({"version":1,"kind":"unsupported","supported":false,"tasks":[]}),
        );
        assert!(session.work().is_empty());
        assert!(!session.has_pending_metadata());
    }

    #[test]
    fn extension_start_during_history_sync_keeps_steering_and_cancellation_available() {
        let mut random = [0; 8];
        getrandom::fill(&mut random).unwrap();
        let root = std::env::temp_dir().join(format!(
            "velune-pi-lifecycle-{:x}",
            u64::from_le_bytes(random)
        ));
        std::fs::create_dir(&root).unwrap();
        let helper = root.join("rpc.py");
        std::fs::write(
            &helper,
            r#"
import json, sys
def emit(value):
    print(json.dumps(value), flush=True)
started = False
for line in sys.stdin:
    request = json.loads(line)
    command = request['type']
    data = {}
    if command == 'prompt':
        emit({'type':'extension_ui_request','method':'setStatus',
              'statusKey':'velune.session-metadata',
              'statusText':json.dumps({'messageIds':[], 'identityConfirmations':[]})})
        data = {'disposition':'handled'}
    elif command == 'get_state':
        if not started:
            emit({'type':'agent_start'})
            started = True
    elif command == 'get_messages':
        data = {'messages':[]}
    elif command == 'abort':
        emit({'type':'agent_settled','aborted':True})
    emit({'type':'response','id':request['id'],'command':command,'success':True,'data':data})
"#,
        )
        .unwrap();
        let config: Config = serde_json::from_value(json!({
            "execution":{"kind":"local"}, "os_home":root,
            "binary":"/usr/bin/python3", "node_binary":"/usr/bin/python3",
            "rpc_entry":helper,
        }))
        .unwrap();
        let client = Client::spawn(config.clone()).unwrap();
        let mut session = PiSession::new(config);
        session.start(
            client,
            PiProjection::new(ConversationSummary {
                native_read_state: None,
                id: "pi:synthetic".into(),
                runtime_id: "pi".into(),
                title: crate::conversation::conversation_title(None, None),
                cwd: None,
                created_at_unix_ms: None,
                updated_at_unix_ms: None,
                can_rename: false,
                can_delete: false,
                parent_conversation_id: None,
            }),
        );
        assert!(session.sync_history("pi", None).unwrap());
        assert!(session.turn_running());
        assert!(session.can_steer());
        let snapshot = session.snapshot().unwrap();
        assert_eq!(snapshot.run_state, RunState::Running);
        assert!(!snapshot.actions.can_send);
        assert!(snapshot.actions.can_cancel);
        session.cancel().unwrap();
        session.poll();
        assert!(!session.turn_running());
        assert!(!session.can_steer());
        assert!(session.snapshot().unwrap().actions.can_send);
        session.shutdown().unwrap();
        std::fs::remove_dir_all(root).unwrap();
    }
}
