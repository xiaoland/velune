//! One Pi conversation: its launch configuration, and once started its RPC
//! process with the projection of what that process shows.
use crate::conversation::{ConversationSnapshot, ConversationSummary, EditPlacement, RunState};
use crate::{Client, Config, Error, PiProjection, Result, TurnInput};
use serde_json::{Value, json};

pub struct PiSession {
    config: Config,
    running: Option<Running>,
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
        let busy = projection.snapshot.as_ref().is_some_and(|snapshot| {
            matches!(snapshot.run_state, RunState::Running | RunState::Stopping)
        });
        self.running = Some(Running {
            client,
            projection,
            busy,
        });
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
        for event in running.client.poll() {
            running.apply_event(&event);
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
        let running = self.running()?;
        match placement {
            EditPlacement::InPlace => running.client.rewind(entry_id)?,
            EditPlacement::Fork => running.client.fork(entry_id)?,
        }
        running.projection.invalidate_history();
        Ok(())
    }

    /// Continue in a new session holding what precedes the user entry
    /// `entry_id`, or the whole current branch when there is none; the
    /// original session stays as it is.
    pub fn fork_at(&mut self, entry_id: Option<&str>) -> Result<()> {
        let running = self.running()?;
        match entry_id {
            Some(entry_id) => running.client.fork(entry_id)?,
            None => running.client.clone()?,
        }
        running.projection.invalidate_history();
        Ok(())
    }

    /// Start a new session in the running process.
    pub fn new_session(&mut self) -> Result<()> {
        self.request(json!({"type":"new_session"})).map(drop)
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
