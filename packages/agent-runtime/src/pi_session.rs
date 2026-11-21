//! One Pi conversation: its launch configuration, and once started its RPC
//! process with the projection of what that process shows.
use crate::conversation::{
    BackgroundTaskInfo, BackgroundTaskList, BackgroundTaskOutput, BackgroundTaskStatus,
    ConversationSnapshot, ConversationSummary, EditPlacement, RunState,
};
use crate::work::{BackgroundState, BackgroundTask};
use crate::{
    Client, Config, Error, PiOperation, PiPortItem, PiProjection, PiReceiptPhase, PiTicket, Result,
    TurnInput,
};
use serde_json::{Value, json};
use std::collections::{BTreeMap, HashSet};
use std::time::{Duration, Instant};

const EDIT_OPERATION_TIMEOUT: Duration = Duration::from_secs(5);
const EDIT_POLL_INTERVAL: Duration = Duration::from_millis(5);

pub struct PiSession {
    config: Config,
    running: Option<Running>,
    background: BTreeMap<String, BackgroundState>,
    task_items: BTreeMap<String, BackgroundTaskInfo>,
    task_reads: crate::background::Reads,
    task_issue: Option<String>,
    children: BTreeMap<String, (BackgroundState, bool)>,
    pending_notifications: HashSet<String>,
    acknowledged_notifications: HashSet<String>,
    background_pending_sync: bool,
    background_refresh_queued: bool,
    background_error: Option<String>,
    background_diagnostic_error: Option<String>,
    conversation_data: Option<crate::creation::DataRetention>,
}

enum PendingAction {
    Control,
    Steer,
    DialogReply,
    Prompt,
    Thinking,
    DiscoverBackground,
    Background(u64),
    HistoryMetadata(HistoryRequest),

    Branch,
    Rewind(String),
    Rename,
    RenameState,
}

#[derive(Clone)]
struct HistoryRequest {
    runtime_id: String,
    model_record_key: Option<String>,
    snapshot: HistorySnapshot,
    deadline: Option<std::time::Instant>,
}

#[derive(Clone)]
enum HistorySnapshot {
    Awaiting,
    Installed,
    Deferred,
    Invalid(String),
}

struct Running {
    client: Client,
    pending: BTreeMap<PiTicket, PendingAction>,
    edit_wait: Option<EditWait>,
    // One reserved control confirmation, independent of the business wait.
    control_wait: Option<ControlWait>,
    edit_fallback: Option<PiTicket>,
    rewind: Option<Value>,
    history_pending: bool,
    history_completed: bool,
    background_frame_revision: u64,
    projection: PiProjection,
    /// A turn or a compaction runs, including an accepted prompt whose
    /// agent_start has not arrived yet.
    busy: bool,
    dialogs: crate::pi_dialogs::Dialogs,
    pending_prompt: Option<String>,
    pending_prompt_started: bool,
}

struct EditWait {
    ticket: PiTicket,
    installed_generation: u64,
    prompt: bool,
    // Exact admission response identity; unbound native user events are projection only.
    prompt_wire_id: Option<String>,
    prompt_accepted: bool,
    result: Option<Result<()>>,
}

struct ControlWait {
    wait: EditWait,
    native: PiTicket,
    cancellation: bool,
    clear_queue_confirmed: bool,
    confirmed: bool,
}

impl ControlWait {
    fn new(ticket: PiTicket, cancellation: bool) -> Self {
        Self {
            wait: EditWait {
                ticket,
                installed_generation: ticket.session_generation,
                prompt: false,
                prompt_wire_id: None,
                prompt_accepted: false,
                result: None,
            },
            native: ticket,
            cancellation,
            clear_queue_confirmed: false,
            confirmed: false,
        }
    }
}

impl Running {
    fn wait(&self, ticket: PiTicket) -> Option<&EditWait> {
        self.edit_wait
            .as_ref()
            .filter(|wait| wait.ticket == ticket)
            .or_else(|| {
                self.control_wait
                    .as_ref()
                    .map(|control| &control.wait)
                    .filter(|wait| wait.ticket == ticket)
            })
    }

    fn wait_mut(&mut self, ticket: PiTicket) -> Option<&mut EditWait> {
        if self
            .edit_wait
            .as_ref()
            .is_some_and(|wait| wait.ticket == ticket)
        {
            self.edit_wait.as_mut()
        } else {
            self.control_wait
                .as_mut()
                .map(|control| &mut control.wait)
                .filter(|wait| wait.ticket == ticket)
        }
    }

    fn receipt_wait_mut(&mut self, ticket: PiTicket) -> Option<&mut EditWait> {
        if self
            .edit_wait
            .as_ref()
            .is_some_and(|wait| wait.ticket == ticket)
        {
            self.edit_wait.as_mut()
        } else {
            self.control_wait
                .as_mut()
                .filter(|control| control.native == ticket)
                .map(|control| &mut control.wait)
        }
    }

    fn native_scope_changed(&mut self) -> Result<()> {
        self.client.advance_pi_session_generation()?;
        self.pending.clear();
        self.edit_fallback = None;
        self.pending_prompt = None;
        self.history_pending = false;
        self.history_completed = false;
        self.dialogs.clear();
        self.publish_dialogs();
        Ok(())
    }

    fn background_request_pending(&self) -> bool {
        self.pending.values().any(|action| {
            matches!(
                action,
                PendingAction::Background(_) | PendingAction::DiscoverBackground
            )
        })
    }

    fn publish_dialogs(&mut self) {
        if let Some(snapshot) = &mut self.projection.snapshot {
            let interactions = self.dialogs.interactions();
            if snapshot.pending_interactions != interactions {
                snapshot.pending_interactions = interactions;
                snapshot.revision += 1;
            }
        }
    }

    fn apply_event(&mut self, event: &Value) {
        if event["type"] == "extension_ui_request"
            && event["method"] == "setStatus"
            && event["statusKey"] == "velune.rewind"
        {
            self.rewind = Some(event.clone());
        }
        if event["type"] == "agent_start" {
            self.pending_prompt_started = true;
        }
        self.dialogs.observe(event);
        self.dialogs.expire();
        self.projection.apply_event(event);
        if self.client.pi_cleanup_required() || !self.projection.can_sync_history() {
            self.pending_prompt = None;
            self.dialogs.clear();
            self.publish_dialogs();
            self.busy = false;
            return;
        }
        if event["type"] == "response"
            && event["command"] == "prompt"
            && event["id"].as_str() == self.pending_prompt.as_deref()
            && self.pending_prompt.is_some()
            && event["data"]["disposition"] != "awaiting_user"
        {
            self.pending_prompt = None;
            if event["success"] == false {
                self.projection
                    .reject_prompt(event["error"].as_str().unwrap_or("Pi 拒绝了输入"));
                self.dialogs.clear();
            } else if event["data"]["disposition"] == "handled" && !self.pending_prompt_started {
                self.busy = false;
                self.projection.finish_handled_prompt();
                self.dialogs.clear();
            }
        }
        self.publish_dialogs();
        if self.client.pi_cleanup_required() {
            // A terminal projection is not a process/cleanup lease release.
            self.busy = false;
        } else if self.projection.turn_ended() {
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
            task_items: BTreeMap::new(),
            task_reads: Default::default(),
            task_issue: None,
            children: BTreeMap::new(),
            pending_notifications: HashSet::new(),
            acknowledged_notifications: HashSet::new(),
            background_pending_sync: false,
            background_refresh_queued: false,
            background_error: None,
            background_diagnostic_error: None,
            conversation_data: None,
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
    pub fn start(&mut self, mut client: Client, mut projection: PiProjection) {
        if let Err(error) = client.enable_port() {
            projection.apply_event(&json!({"type":"velune_error","error":error.to_string()}));
        }
        self.reset_background_session();
        let busy = projection.snapshot.as_ref().is_some_and(|snapshot| {
            matches!(snapshot.run_state, RunState::Running | RunState::Stopping)
        });
        self.running = Some(Running {
            client,
            pending: BTreeMap::new(),
            edit_wait: None,
            control_wait: None,
            edit_fallback: None,
            rewind: None,
            history_pending: false,
            history_completed: false,
            background_frame_revision: 0,
            projection,
            busy,
            dialogs: Default::default(),
            pending_prompt: None,
            pending_prompt_started: false,
        });
        self.background_pending_sync = true;
        if let Err(error) =
            self.enqueue(PiOperation::GetCommands, PendingAction::DiscoverBackground)
        {
            self.set_background_error(Some(error.to_string()));
        }
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
            Some(mut running) => running.client.cleanup_blocking(),
            None => Ok(()),
        }
    }

    /// Failed protocol ownership persists until actual process cleanup.
    pub fn requires_failure_cleanup(&self) -> bool {
        self.running
            .as_ref()
            .is_some_and(|running| running.client.pi_cleanup_required())
    }

    pub fn turn_running(&self) -> bool {
        self.running.as_ref().is_some_and(|running| running.busy)
    }

    /// Known Pi-owned tasks that outlive the current parent turn.
    pub fn work(&self) -> Vec<BackgroundTask> {
        self.background
            .iter()
            .map(|(id, state)| BackgroundTask {
                conversation_id: self
                    .task_items
                    .get(id)
                    .and_then(|task| task.conversation_id.clone()),
                state: *state,
            })
            .chain(self.children.iter().map(|(path, (state, _))| {
                BackgroundTask {
                    conversation_id: self
                        .snapshot()
                        .map(|snapshot| format!("{}:{path}", snapshot.conversation.runtime_id)),
                    state: *state,
                }
            }))
            .collect()
    }

    pub fn can_stop_child(&self, id: &str) -> bool {
        if self
            .background_tasks()
            .tasks
            .iter()
            .any(|task| task.conversation_id.as_deref() == Some(id) && task.can_stop)
        {
            return true;
        }
        self.snapshot()
            .and_then(|snapshot| id.strip_prefix(&format!("{}:", snapshot.conversation.runtime_id)))
            .and_then(|path| self.children.get(path))
            .is_some_and(|(_, can_stop)| *can_stop)
    }

    pub fn background_tasks(&self) -> BackgroundTaskList {
        let live = self.running.is_some()
            && !self.background_pending_sync
            && self.background_error.is_none();
        let tasks = self
            .task_items
            .values()
            .cloned()
            .map(|mut task| {
                if !live {
                    task.status = BackgroundTaskStatus::Unknown;
                    task.can_stop = false;
                    task.can_read_output = false;
                }
                task
            })
            .collect();
        BackgroundTaskList {
            tasks,
            live,
            issue: self
                .task_issue
                .clone()
                .or_else(|| self.background_error.clone()),
        }
    }
    pub fn read_background_task_output(
        &mut self,
        task: &str,
        request: Option<&str>,
    ) -> Result<BackgroundTaskOutput> {
        if let Some(request) = request {
            return self
                .task_reads
                .read(task, request)
                .ok_or_else(|| Error::Sdk("任务输出请求已失效，请重新读取".into()));
        }
        if !self
            .background_tasks()
            .tasks
            .iter()
            .any(|entry| entry.id == task && entry.can_read_output)
        {
            return Err(Error::Sdk("当前不能读取此后台任务的输出".into()));
        }
        let output = self.task_reads.begin(task);
        self.enqueue(
            PiOperation::Background(
                json!({"operation":"task_output","taskId":task,"requestId":output.request_id}),
            ),
            PendingAction::Background(
                self.running
                    .as_ref()
                    .map_or(0, |running| running.background_frame_revision),
            ),
        )?;
        Ok(output)
    }
    pub fn stop_background_task(&mut self, task: &str) -> Result<()> {
        if !self
            .background_tasks()
            .tasks
            .iter()
            .any(|entry| entry.id == task && entry.can_stop)
        {
            return Err(Error::Sdk("当前不能独立停止此后台任务".into()));
        }
        self.enqueue(
            PiOperation::Background(json!({"operation":"stop_task","taskId":task})),
            PendingAction::Background(
                self.running
                    .as_ref()
                    .map_or(0, |running| running.background_frame_revision),
            ),
        )?;
        if let Some(entry) = self.task_items.get_mut(task) {
            entry.status = BackgroundTaskStatus::Stopping;
            entry.can_stop = false;
        }
        Ok(())
    }

    pub fn stop_child(&mut self, id: &str) -> Result<()> {
        if let Some(task) = self
            .background_tasks()
            .tasks
            .iter()
            .find(|task| task.conversation_id.as_deref() == Some(id) && task.can_stop)
            .map(|task| task.id.clone())
        {
            return self.stop_background_task(&task);
        }
        if !self.can_stop_child(id) {
            return Err(Error::Sdk("Pi 子会话不能独立停止".into()));
        }
        let path = id
            .strip_prefix(&format!(
                "{}:",
                self.snapshot().unwrap().conversation.runtime_id
            ))
            .unwrap()
            .to_owned();
        self.enqueue(
            PiOperation::Background(json!({"operation":"stop_child", "sessionFile":&path})),
            PendingAction::Background(
                self.running
                    .as_ref()
                    .map_or(0, |running| running.background_frame_revision),
            ),
        )?;
        if let Some((state, can_stop)) = self.children.get_mut(&path) {
            *state = BackgroundState::Stopping;
            *can_stop = false;
        }
        Ok(())
    }

    /// True while the Host API snapshot, an error, or a completion
    /// notification handoff is not yet settled.
    pub fn has_pending_metadata(&self) -> bool {
        self.background_pending_sync
            || self.running.as_ref().is_some_and(|running| {
                running.client.pi_cleanup_required() || running.background_request_pending()
            })
            || self.background_refresh_queued
            || self.task_reads.pending()
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
        if self
            .running
            .as_ref()
            .is_some_and(|running| running.background_request_pending())
        {
            self.background_refresh_queued = true;
            return;
        }
        self.background_pending_sync = true;
        match self.enqueue(
            PiOperation::Background(command),
            PendingAction::Background(
                self.running
                    .as_ref()
                    .map_or(0, |running| running.background_frame_revision),
            ),
        ) {
            Ok(_) => {}
            Err(error) => self.set_background_error(Some(error.to_string())),
        }
    }

    fn reset_background_session(&mut self) {
        self.background.clear();
        self.task_items.clear();
        self.task_reads = Default::default();
        self.task_issue = None;
        self.children.clear();
        self.pending_notifications.clear();
        self.acknowledged_notifications.clear();
        self.background_pending_sync = true;
        self.background_refresh_queued = false;
        self.set_background_error(None);
        self.background_diagnostic_error = None;
    }

    fn apply_background_frame(&mut self, frame: &Value) {
        // A delayed response from the previous native parent must not mutate
        // the newly addressed session, including output and operation receipts.
        // Bridges predating this owner field remain compatible.
        let expected = self
            .snapshot()
            .and_then(|snapshot| {
                snapshot
                    .conversation
                    .id
                    .strip_prefix(&format!("{}:", snapshot.conversation.runtime_id))
            })
            .map(std::path::Path::new)
            .filter(|path| path.is_absolute())
            .or(self.config.session.as_deref());
        if let (Some(parent), Some(expected)) = (frame["parentSessionFile"].as_str(), expected)
            && std::path::Path::new(parent) != expected
        {
            return;
        }
        if frame["version"] != 1 {
            self.background_pending_sync = true;
            self.set_background_error(Some("Pi 后台工作桥接返回了未知协议版本".into()));
            return;
        }
        if let Some(running) = &mut self.running {
            running.background_frame_revision = running.background_frame_revision.saturating_add(1);
        }
        if let Some(output) = frame.get("taskOutput")
            && let (Some(request), Some(task)) =
                (output["requestId"].as_str(), output["taskId"].as_str())
        {
            self.task_reads.finish(
                request,
                task,
                output["text"].as_str().unwrap_or(""),
                output["truncated"] == true,
                output["error"].as_str().map(str::to_owned),
            );
        }
        if let Some(operation) = frame.get("taskOperation") {
            self.task_issue = operation["error"].as_str().map(str::to_owned);
        }
        if let Some(receipts) = frame["persistedNotificationIds"].as_array() {
            for id in receipts
                .iter()
                .filter_map(Value::as_str)
                .filter(|id| !id.is_empty())
            {
                self.acknowledged_notifications.insert(id.to_owned());
                self.pending_notifications.remove(id);
            }
        }
        match frame["kind"].as_str() {
            Some("unsupported") => {
                self.background.clear();
                self.task_items.clear();
                self.pending_notifications.clear();
                self.background_pending_sync = false;
                self.set_background_error(None);
            }
            Some("snapshot") => {
                self.background.clear();
                self.task_items.clear();
                if let Some(tasks) = frame["tasks"].as_array() {
                    for task in tasks {
                        let Some(id) = task["id"].as_str().filter(|id| !id.is_empty()) else {
                            continue;
                        };
                        let status =
                            crate::background::status(task["status"].as_str().unwrap_or("unknown"));
                        let kind = task["kind"].as_str().unwrap_or("task");
                        let title = task["description"]
                            .as_str()
                            .filter(|value| !value.is_empty())
                            .or_else(|| task["command"].as_str())
                            .unwrap_or(kind);
                        self.task_items.insert(
                            id.into(),
                            BackgroundTaskInfo {
                                id: id.into(),
                                kind: kind.into(),
                                title: crate::background::text(title, 512).0,
                                status,
                                conversation_id: task["childSessionFile"].as_str().and_then(
                                    |path| {
                                        self.snapshot().map(|snapshot| {
                                            format!("{}:{path}", snapshot.conversation.runtime_id)
                                        })
                                    },
                                ),
                                child_session_id: task["childSessionId"]
                                    .as_str()
                                    .map(str::to_owned),
                                can_stop: status == BackgroundTaskStatus::Running
                                    && task["canStop"] == true,
                                can_read_output: task["canReadOutput"] == true,
                            },
                        );
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
        if let Some(running) = &mut self.running {
            running
                .projection
                .update_sub_agents(&frame["subAgents"], true);
            // Older bridges may describe an active child only here. Its
            // fallback must never replace the fuller native agent/task title.
            running
                .projection
                .update_sub_agents(&frame["children"], false);
        }
        if let Some(children) = frame["children"].as_array() {
            self.children.clear();
            for child in children {
                if let Some(path) = child["sessionFile"]
                    .as_str()
                    .filter(|path| std::path::Path::new(path).is_absolute())
                {
                    let state = if child["state"] == "stopping" {
                        BackgroundState::Stopping
                    } else {
                        BackgroundState::Running
                    };
                    self.children.insert(
                        path.into(),
                        (
                            state,
                            child["canStop"] == true && frame["childrenUnknown"] != true,
                        ),
                    );
                }
            }
        }
        if frame["childrenUnknown"] == true {
            self.background_pending_sync = true;
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
        if task_ids.is_empty() && self.children.is_empty() {
            if self.background_pending_sync || self.background_error.is_some() {
                return Err(Error::Sdk(
                    self.background_error
                        .clone()
                        .unwrap_or_else(|| "Pi 后台工作状态尚未同步，无法安全停止".into()),
                ));
            }
            return Ok(());
        }
        self.background_pending_sync = true;
        self.enqueue(
            PiOperation::Background(json!({"operation":"stop_all","taskIds":task_ids})),
            PendingAction::Background(
                self.running
                    .as_ref()
                    .map_or(0, |running| running.background_frame_revision),
            ),
        )
        .map(drop)
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
            snapshot.retained_reply_ids = source.retained_reply_ids.clone();
            for id in &source.work_notification_ids {
                if snapshot.messages.iter().any(|message| &message.id == id)
                    && !snapshot.work_notification_ids.contains(id)
                {
                    snapshot.work_notification_ids.push(id.clone());
                }
            }
        }
    }

    pub fn set_conversation(
        &mut self,
        conversation: crate::creation::Retained<ConversationSummary>,
    ) {
        let (conversation, data) = conversation.into_parts();
        if let Some(running) = &mut self.running {
            running.projection.set_conversation(conversation);
            self.conversation_data = Some(data);
        }
    }

    pub fn owned_snapshot(&self) -> Result<crate::creation::Retained<ConversationSnapshot>> {
        let data = self
            .conversation_data
            .as_ref()
            .map(crate::creation::DataRetention::try_clone)
            .transpose()
            .map_err(Error::Io)?
            .unwrap_or_default();
        let snapshot = self
            .snapshot()
            .ok_or_else(|| Error::Sdk("Pi 会话尚未打开".into()))?;
        Ok(crate::creation::Retained::from_parts(
            snapshot.clone(),
            data,
        ))
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
        running.dialogs.expire();
        running.publish_dialogs();
        let items = running.client.poll_pi();
        let settled = self.apply_port_items(items);
        if settled && !self.turn_running() {
            // Close the admission-coalescing window at the parent turn fence:
            // read the public task snapshot before callers can conclude that
            // an empty cached list is settled and release Pi/the gateway.
            self.background_pending_sync = true;
            self.refresh_background(json!({"operation":"snapshot"}));
        }
    }

    fn apply_port_items(&mut self, items: Vec<PiPortItem>) -> bool {
        let mut settled = false;
        for item in items {
            match item {
                PiPortItem::Event(event) => settled |= self.apply_events(vec![event]),
                PiPortItem::OperationEvent {
                    ticket,
                    event,
                    deadline,
                } => {
                    let Some(running) = &mut self.running else {
                        continue;
                    };
                    if !running.client.pi_owns_ticket(ticket) {
                        running.pending.remove(&ticket);
                        continue;
                    }
                    if let Some(error) = running.client.pi_install_failure(deadline) {
                        running.apply_event(&json!({"type":"velune_error","error":error}));
                    }
                    settled |= self.apply_events(vec![event]);
                }
                PiPortItem::Receipt(receipt) => self.consume_receipt(receipt),
            }
        }
        settled
    }

    fn apply_events(&mut self, events: Vec<Value>) -> bool {
        let mut settled = false;
        for event in events {
            if event["type"] == "extension_ui_request"
                && event["method"] == "setStatus"
                && event["statusKey"] == "velune.session-metadata"
            {
                let frame = event["statusText"]
                    .as_str()
                    .and_then(|text| serde_json::from_str::<Value>(text).ok());
                if !frame
                    .as_ref()
                    .is_some_and(|frame| frame["snapshotHeaderOnly"] == true)
                {
                    let pending = self.running.as_ref().and_then(|running| {
                        if running.client.pi_cleanup_required() {
                            return None;
                        }
                        running
                            .pending
                            .iter()
                            .find_map(|(ticket, action)| match action {
                                PendingAction::HistoryMetadata(request)
                                    if matches!(request.snapshot, HistorySnapshot::Awaiting)
                                        && running.client.pi_owns_ticket(*ticket) =>
                                {
                                    Some((*ticket, request.clone()))
                                }
                                _ => None,
                            })
                    });
                    if let Some((ticket, request)) = pending {
                        let request_id =
                            format!("velune-pi-{}-{}", ticket.generation, ticket.sequence);
                        if frame
                            .as_ref()
                            .and_then(|frame| frame["requestId"].as_str())
                            .is_some_and(|source| source != request_id)
                        {
                            continue; // A failed/old query cannot satisfy a later query.
                        }
                        let outcome = match Client::projection_snapshot(&event, &request_id) {
                            Err(error) => HistorySnapshot::Invalid(error.to_string()),
                            Ok(_) if self.running.as_ref().is_some_and(|running| running.busy) => {
                                HistorySnapshot::Deferred
                            }
                            Ok(snapshot) => {
                                let deadline = request.deadline;
                                match self.install_history(request, snapshot, deadline) {
                                    Ok(()) => HistorySnapshot::Installed,
                                    Err(error) => HistorySnapshot::Invalid(error.to_string()),
                                }
                            }
                        };
                        let installed = matches!(outcome, HistorySnapshot::Installed);
                        if let Some(PendingAction::HistoryMetadata(request)) = self
                            .running
                            .as_mut()
                            .and_then(|running| running.pending.get_mut(&ticket))
                        {
                            request.snapshot = outcome;
                        }
                        if !installed {
                            continue;
                        } // Preserve the old body and header together.
                    } else if frame.as_ref().is_some_and(|frame| {
                        frame.get("snapshotVersion").is_some() || frame.get("requestId").is_some()
                    }) {
                        continue; // Late, duplicate, readiness, or cancelled-generation frame.
                    }
                }
            }
            // Native message events (including ID-less Pi events) do not carry
            // the admission ticket. They may update projection, never settle an
            // input merely because agent_start preceded a user row. Only the
            // exact native prompt response is attributable without guessing.
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
                if event["type"] == "velune_error" && running.client.pi_cleanup_required() {
                    for wait in running.edit_wait.iter_mut().chain(
                        running
                            .control_wait
                            .iter_mut()
                            .map(|control| &mut control.wait),
                    ) {
                        if wait.result.is_none() {
                            wait.result = Some(Err(Error::Transport(
                                event["error"]
                                    .as_str()
                                    .unwrap_or("Pi operation transport failed")
                                    .to_owned(),
                            )));
                        }
                    }
                }
                settled |= event["type"] == "agent_settled";
            }
        }
        settled
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
        if std::mem::take(&mut running.history_completed) {
            return Ok(true);
        }
        if running.busy
            || running.history_pending
            || running.projection.history_synchronized()
            || !running.projection.can_sync_history()
        {
            return Ok(false);
        }
        self.enqueue(
            PiOperation::ProjectionSync,
            PendingAction::HistoryMetadata(HistoryRequest {
                runtime_id: runtime_id.into(),
                model_record_key,
                snapshot: HistorySnapshot::Awaiting,
                deadline: None,
            }),
        )?;
        self.running.as_mut().unwrap().history_pending = true;
        Ok(false)
    }

    /// Admit one edit dependency without waiting on the protocol owner. The
    /// exact ticket remains owned until its validated native result is polled.
    pub fn begin_edit_history(
        &mut self,
        runtime_id: &str,
        model_record_key: Option<String>,
    ) -> Result<PiTicket> {
        let running = self.running()?;
        if running.busy || running.edit_wait.is_some() {
            return Err(Error::Sdk("Pi 消息编辑依赖正在执行".into()));
        }
        let pending = running.pending.iter().find_map(|(ticket, action)| {
            matches!(action, PendingAction::HistoryMetadata(_)).then_some(*ticket)
        });
        let ticket = match pending {
            Some(ticket) => ticket,
            None => self.enqueue(
                PiOperation::ProjectionSync,
                PendingAction::HistoryMetadata(HistoryRequest {
                    runtime_id: runtime_id.into(),
                    model_record_key,
                    snapshot: HistorySnapshot::Awaiting,
                    deadline: None,
                }),
            )?,
        };
        let running = self.running()?;
        running.history_pending = true;
        running.edit_wait = Some(EditWait {
            ticket,
            installed_generation: ticket.session_generation,
            prompt: false,
            prompt_wire_id: None,
            prompt_accepted: false,
            result: None,
        });
        Ok(ticket)
    }

    pub fn take_edit_fallback(&mut self, ticket: PiTicket) -> bool {
        self.running.as_mut().is_some_and(|running| {
            running.client.pi_owns_ticket(ticket) && running.edit_fallback.take() == Some(ticket)
        })
    }

    pub fn begin_edit_branch(&mut self, entry_id: &str) -> Result<PiTicket> {
        if self.running()?.busy || self.running()?.edit_wait.is_some() {
            return Err(Error::Sdk("Pi 消息编辑依赖正在执行".into()));
        }
        self.running()?.rewind = None;
        self.running()?.edit_fallback = None;
        let ticket = self.enqueue(
            PiOperation::Rewind(entry_id.into()),
            PendingAction::Rewind(entry_id.into()),
        )?;
        self.running()?.edit_wait = Some(EditWait {
            ticket,
            installed_generation: ticket.session_generation,
            prompt: false,
            prompt_wire_id: None,
            prompt_accepted: false,
            result: None,
        });
        Ok(ticket)
    }

    pub fn begin_edit_model(&mut self, provider: Option<&str>, model: &str) -> Result<PiTicket> {
        if self.running()?.edit_wait.is_some() {
            return Err(Error::Sdk("Pi 消息编辑依赖正在执行".into()));
        }
        let ticket = self.enqueue(
            PiOperation::SetModel {
                provider: provider.map(str::to_owned),
                model_id: model.into(),
            },
            PendingAction::Control,
        )?;
        self.running()?.edit_wait = Some(EditWait {
            ticket,
            installed_generation: ticket.session_generation,
            prompt: false,
            prompt_wire_id: None,
            prompt_accepted: false,
            result: None,
        });
        Ok(ticket)
    }

    /// A local enqueue/physical Written is not native input acceptance. This
    /// ticket settles only at its exact native response. Pi user events may
    /// lack identity and are not attributable ACKs, even after agent_start.
    pub fn begin_edit_prompt(&mut self, input: &TurnInput) -> Result<PiTicket> {
        if self.running()?.edit_wait.is_some() {
            return Err(Error::Sdk("Pi 消息编辑依赖正在执行".into()));
        }
        self.send(input)?;
        let running = self.running()?;
        let ticket = running
            .pending
            .iter()
            .find_map(|(ticket, action)| matches!(action, PendingAction::Prompt).then_some(*ticket))
            .ok_or_else(|| Error::Sdk("Pi 替换输入未取得票据".into()))?;
        running.edit_wait = Some(EditWait {
            ticket,
            installed_generation: ticket.session_generation,
            prompt: true,
            prompt_wire_id: running.pending_prompt.clone(),
            prompt_accepted: false,
            result: None,
        });
        Ok(ticket)
    }

    /// Polling does no IO wait. Generation changes cannot authorize a successor;
    /// successful rewind is the sole operation allowed to advance its own scope.
    pub fn take_confirmation(&mut self, ticket: PiTicket) -> Result<Option<Result<()>>> {
        let running = self.running()?;
        let wait = running
            .wait(ticket)
            .ok_or_else(|| Error::Sdk("Pi 操作等待身份已失效".into()))?;
        let installed = PiTicket {
            session_generation: wait.installed_generation,
            ..ticket
        };
        let confirmed_control = running
            .control_wait
            .as_ref()
            .is_some_and(|control| control.wait.ticket == ticket && control.confirmed);
        if !running.client.pi_owns_ticket(installed) && !wait.prompt_accepted && !confirmed_control
        {
            self.clear_edit_wait(ticket);
            return Ok(Some(Err(Error::Transport("Pi 编辑来源代际已改变".into()))));
        }
        self.take_edit_wait_result(ticket)
    }

    /// Discard an unverified view without releasing the actual protocol owner.
    pub fn clear_edit_projection(&mut self) {
        if let Some(running) = &mut self.running {
            running.projection.snapshot = None;
        }
    }

    /// Withdraw only never-written input. Otherwise retain the protocol owner
    /// and its receipts; cancellation is not proof that a mutation did not run.
    pub fn cancel_confirmation(&mut self, ticket: PiTicket) -> bool {
        let not_sent = self.running.as_mut().is_some_and(|running| {
            let native = running
                .control_wait
                .as_ref()
                .filter(|control| control.wait.ticket == ticket)
                .map_or(ticket, |control| control.native);
            running.client.abandon_queued_pi(native)
        });
        self.clear_edit_wait(ticket);
        not_sent
    }

    /// Synchronizes Pi history for an edit precondition. Unlike ordinary
    /// projection refresh, this waits for the typed request's validated frame
    /// and receipt because its caller is about to compare/branch/send.
    pub fn sync_history_for_edit(
        &mut self,
        runtime_id: &str,
        model_record_key: Option<String>,
    ) -> Result<()> {
        let (synchronized, busy, pending_ticket, completed) = {
            let running = self.running()?;
            (
                running.projection.history_synchronized(),
                running.busy,
                running.pending.iter().find_map(|(ticket, action)| {
                    matches!(action, PendingAction::HistoryMetadata(_)).then_some(*ticket)
                }),
                running.history_completed,
            )
        };
        if synchronized {
            return Ok(());
        }
        if busy {
            return Err(Error::Sdk("Pi 正在执行，无法为消息编辑确认原生历史".into()));
        }

        let ticket = match pending_ticket {
            Some(ticket) => ticket,
            None => {
                if self.sync_history(runtime_id, model_record_key.clone())? {
                    return Ok(());
                }
                let running = self.running()?;
                if running.projection.history_synchronized() {
                    return Ok(());
                }
                running
                    .pending
                    .iter()
                    .find_map(|(ticket, action)| {
                        matches!(action, PendingAction::HistoryMetadata(_)).then_some(*ticket)
                    })
                    .ok_or_else(|| Error::Sdk("Pi 未能为消息编辑启动原生历史同步".into()))?
            }
        };

        self.wait_for_edit_ticket(ticket)?;
        // A validated frame is immediately followed by its header-only event,
        // which invalidates the projection's cheap freshness bit. The typed
        // owner retains the install completion edge for this exact reason.
        if self.sync_history(runtime_id, model_record_key)? {
            Ok(())
        } else {
            Err(Error::Sdk(if completed {
                "Pi 原生历史同步未完成消息编辑屏障".into()
            } else {
                "Pi 原生历史快照未安装，不能继续消息编辑".into()
            }))
        }
    }

    /// Rewinds an in-place edit and waits for its native outcome. The caller
    /// must then use `sync_history_for_edit` before comparing or sending.
    /// Ordinary `branch` remains nonblocking.
    pub fn branch_for_edit(&mut self, entry_id: &str) -> Result<()> {
        if self.running()?.busy {
            return Err(Error::Sdk("Pi 正在执行，无法准备消息编辑分支".into()));
        }
        self.running()?.rewind = None;
        let ticket = self.enqueue(
            PiOperation::Rewind(entry_id.into()),
            PendingAction::Rewind(entry_id.into()),
        )?;
        self.wait_for_edit_ticket(ticket)
    }

    fn wait_for_edit_ticket(&mut self, ticket: PiTicket) -> Result<()> {
        {
            let running = self.running()?;
            if running.edit_wait.is_some() {
                return Err(Error::Sdk("Pi 消息编辑操作正在等待完成".into()));
            }
            if !running.client.pi_owns_ticket(ticket) {
                return Err(Error::Sdk("Pi 消息编辑操作已失效".into()));
            }
            running.edit_wait = Some(EditWait {
                ticket,
                installed_generation: ticket.session_generation,
                prompt: false,
                prompt_wire_id: None,
                prompt_accepted: false,
                result: None,
            });
        }

        let deadline = Instant::now() + EDIT_OPERATION_TIMEOUT;
        loop {
            if let Some(result) = self.take_edit_wait_result(ticket)? {
                return result;
            }
            if Instant::now() >= deadline {
                self.clear_edit_wait(ticket);
                return Err(Error::Timeout("Pi 消息编辑操作等待超时".into()));
            }
            self.poll();
            if let Some(result) = self.take_edit_wait_result(ticket)? {
                return result;
            }
            std::thread::sleep(EDIT_POLL_INTERVAL);
        }
    }

    fn take_edit_wait_result(&mut self, ticket: PiTicket) -> Result<Option<Result<()>>> {
        let running = self.running()?;
        let confirmed_control = running
            .control_wait
            .as_ref()
            .is_some_and(|control| control.wait.ticket == ticket && control.confirmed);
        let Some(wait) = running.wait_mut(ticket) else {
            return Err(Error::Sdk("Pi 操作等待身份不匹配或已结束".into()));
        };
        let result = if wait.prompt_accepted || confirmed_control {
            // Business acceptance survives connection failure and install fences.
            Ok(())
        } else {
            let Some(result) = wait.result.take() else {
                return Ok(None);
            };
            result
        };
        self.clear_edit_wait(ticket);
        Ok(Some(result))
    }

    fn clear_edit_wait(&mut self, ticket: PiTicket) {
        if let Some(running) = &mut self.running {
            if running
                .edit_wait
                .as_ref()
                .is_some_and(|wait| wait.ticket == ticket)
            {
                running.edit_wait = None;
            }
            if running
                .control_wait
                .as_ref()
                .is_some_and(|control| control.wait.ticket == ticket)
            {
                running.control_wait = None;
            }
        }
    }

    fn install_history(
        &mut self,
        request: HistoryRequest,
        messages: Value,
        deadline: Option<std::time::Instant>,
    ) -> Result<()> {
        self.ensure_pi_install_deadline(deadline)?;
        Self::observe_persisted_background_notifications(
            &messages,
            &mut self.acknowledged_notifications,
            &mut self.pending_notifications,
        );
        let running = self.running.as_mut().ok_or(Error::NotStarted)?;
        let runtime_id = request.runtime_id;
        let model_record_key = request.model_record_key;
        let projection = &mut running.projection;
        if let Some(path) = messages["nativeSession"]["sessionFile"].as_str() {
            let previous = projection.snapshot.as_ref().map(|s| &s.conversation);
            projection.set_conversation(ConversationSummary {
                native_read_state: None,
                can_rename: false,
                can_delete: false,
                id: format!("{runtime_id}:{path}"),
                title: crate::conversation::conversation_title(
                    messages["nativeSession"]["name"].as_str(),
                    None,
                ),
                created_at_unix_ms: previous.and_then(|c| c.created_at_unix_ms),
                updated_at_unix_ms: previous.and_then(|c| c.updated_at_unix_ms),
                runtime_id,
                cwd: previous.and_then(|c| c.cwd.clone()),
                parent_conversation_id: None,
            });
        }
        projection.replace_history(&messages);
        if let Some(snapshot) = projection.snapshot.as_mut() {
            snapshot.pending_interactions = running.dialogs.interactions();
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
        Ok(())
    }

    /// Send a user turn. Pi may handle it at once (an extension command);
    /// otherwise the turn runs until its events end it.
    pub fn send(&mut self, input: &TurnInput) -> Result<()> {
        if self.turn_running() {
            return Err(Error::Sdk("Pi turn already running".into()));
        }
        if let Some(level) = &input.effort {
            let mut dependent = input.clone();
            dependent.effort = None;
            let predecessor = self.enqueue(
                PiOperation::ThinkingLevel(level.clone()),
                PendingAction::Thinking,
            )?;
            let ticket =
                match self
                    .running()?
                    .client
                    .enqueue_pi_after_thinking(dependent, predecessor, None)
                {
                    Ok(ticket) => ticket,
                    Err(error) => {
                        self.running()?.client.abandon_queued_pi(predecessor);
                        return Err(error);
                    }
                };
            let running = self.running()?;
            running.pending.insert(ticket, PendingAction::Prompt);
            running.pending_prompt = Some(format!(
                "velune-pi-{}-{}",
                ticket.generation, ticket.sequence
            ));
        } else {
            self.enqueue_prompt(input.clone())?;
        }
        let running = self.running()?;
        running.pending_prompt_started = false;
        running.busy = true;
        running.projection.begin_turn();
        if let Some(snapshot) = running.projection.snapshot.as_mut() {
            snapshot.run_state = RunState::Running;
            snapshot.actions.can_send = false;
            snapshot.actions.can_cancel = true;
        }
        Ok(())
    }

    fn enqueue_prompt(&mut self, input: TurnInput) -> Result<()> {
        let ticket = self.enqueue(PiOperation::Prompt(input), PendingAction::Prompt)?;
        self.running()?.pending_prompt = Some(format!(
            "velune-pi-{}-{}",
            ticket.generation, ticket.sequence
        ));
        Ok(())
    }

    pub fn steer(&mut self, input: &TurnInput) -> Result<()> {
        self.enqueue(PiOperation::Steer(input.clone()), PendingAction::Steer)
            .map(drop)
    }

    pub fn begin_cancel_confirmation(&mut self) -> Result<PiTicket> {
        if self.running()?.control_wait.is_some() {
            return Err(Error::Sdk("Pi 控制正在等待确认".into()));
        }
        if let Some(error) = self.prepare_cancel()? {
            return Err(error);
        }
        let ticket = self.enqueue(PiOperation::ClearQueue, PendingAction::Control)?;
        let running = self.running()?;
        running.control_wait = Some(ControlWait::new(ticket, true));
        running.dialogs.clear();
        running.publish_dialogs();
        // Abort is admitted only after this exact ClearQueue ACK. The stable
        // root ticket names the bounded two-step control, not the business wait.
        Ok(ticket)
    }

    pub fn cancel_prefix_confirmed(&self, ticket: PiTicket) -> bool {
        self.running
            .as_ref()
            .and_then(|running| running.control_wait.as_ref())
            .is_some_and(|control| {
                control.wait.ticket == ticket
                    && control.cancellation
                    && control.clear_queue_confirmed
            })
    }

    pub fn begin_reply_confirmation(
        &mut self,
        id: &str,
        reply: crate::conversation::RuntimeInteractionReply,
    ) -> Result<PiTicket> {
        if self.running()?.control_wait.is_some() {
            return Err(Error::Sdk("Pi 控制正在等待确认".into()));
        }
        self.reply(id, reply)?;
        let running = self.running()?;
        let ticket = running
            .pending
            .iter()
            .rev()
            .find_map(|(ticket, action)| {
                matches!(action, PendingAction::DialogReply).then_some(*ticket)
            })
            .ok_or_else(|| Error::Sdk("Pi 回复未取得票据".into()))?;
        running.control_wait = Some(ControlWait::new(ticket, false));
        Ok(ticket)
    }

    pub fn reply(
        &mut self,
        interaction_id: &str,
        reply: crate::conversation::RuntimeInteractionReply,
    ) -> Result<()> {
        let running = self.running()?;
        let response = running.dialogs.response(interaction_id, reply)?;
        let ticket = running
            .client
            .enqueue_pi(PiOperation::DialogReply(response), None)?;
        running.pending.insert(ticket, PendingAction::DialogReply);
        running.dialogs.remove(interaction_id);
        running.publish_dialogs();
        Ok(())
    }

    fn prepare_cancel(&mut self) -> Result<Option<Error>> {
        let interactions = self.running()?.dialogs.interactions();
        let mut reply_error = None;
        for interaction in interactions {
            if let Err(error) = self.enqueue(
                PiOperation::DialogReply(
                    json!({"type":"extension_ui_response","id":interaction.id,"cancelled":true}),
                ),
                PendingAction::DialogReply,
            ) {
                reply_error = Some(error);
            }
        }
        // Remove only an unsent dependent prompt; never guess/replay an on-wire prompt.
        let running = self.running()?;
        let unsent = running
            .pending
            .iter()
            .filter_map(|(ticket, action)| {
                matches!(
                    action,
                    PendingAction::Thinking | PendingAction::Prompt | PendingAction::Steer
                )
                .then_some(*ticket)
            })
            .collect::<Vec<_>>();
        for ticket in unsent {
            if running.client.abandon_queued_pi(ticket)
                && matches!(running.pending.get(&ticket), Some(PendingAction::Thinking))
            {
                running.busy = false;
                running.projection.finish_handled_prompt();
            }
        }
        // An in-flight thinking response must not start a new prompt after cancel.
        running
            .pending
            .retain(|_, action| !matches!(action, PendingAction::Thinking));
        Ok(reply_error)
    }

    pub fn cancel(&mut self) -> Result<()> {
        let reply_error = self.prepare_cancel()?;
        self.enqueue(PiOperation::ClearQueue, PendingAction::Control)?;
        self.enqueue(PiOperation::Abort, PendingAction::Control)?;
        let running = self.running()?;
        running.dialogs.clear();
        running.publish_dialogs();
        reply_error.map_or(Ok(()), Err)
    }

    /// Compaction runs like a turn until its result arrives.
    pub fn compact(&mut self) -> Result<()> {
        let running = self.running()?;
        let ticket = running.client.enqueue_pi(PiOperation::Compact, None)?;
        running.pending.insert(ticket, PendingAction::Control);
        let id = format!("velune-pi-{}-{}", ticket.generation, ticket.sequence);
        running.projection.begin_compaction(id);
        running.busy = true;
        Ok(())
    }

    /// Continue from before the user entry `entry_id`, here or in a new
    /// session as `placement` says.
    pub fn branch(&mut self, entry_id: &str, placement: EditPlacement) -> Result<()> {
        match placement {
            EditPlacement::InPlace => {
                self.running()?.rewind = None;
                self.enqueue(
                    PiOperation::Rewind(entry_id.into()),
                    PendingAction::Rewind(entry_id.into()),
                )?;
            }
            EditPlacement::Fork => {
                self.enqueue(PiOperation::Fork(entry_id.into()), PendingAction::Branch)?;
            }
        }
        Ok(())
    }

    /// Enqueue a native branch; its completion, not admission, resets history.
    pub fn fork_at(&mut self, entry_id: Option<&str>) -> Result<()> {
        self.enqueue(
            entry_id.map_or(PiOperation::Clone, |id| PiOperation::Fork(id.into())),
            PendingAction::Branch,
        )
        .map(drop)
    }
    pub fn new_session(&mut self) -> Result<()> {
        self.enqueue(PiOperation::NewSession, PendingAction::Branch)
            .map(drop)
    }
    pub fn set_name(&mut self, name: &str) -> Result<()> {
        self.enqueue(PiOperation::SetName(name.into()), PendingAction::Rename)
            .map(drop)
    }
    pub fn set_model(&mut self, provider: Option<&str>, model_id: &str) -> Result<()> {
        self.enqueue(
            PiOperation::SetModel {
                provider: provider.map(str::to_owned),
                model_id: model_id.into(),
            },
            PendingAction::Control,
        )
        .map(drop)
    }
    fn enqueue(&mut self, operation: PiOperation, action: PendingAction) -> Result<PiTicket> {
        let running = self.running()?;
        let ticket = running.client.enqueue_pi(operation, None)?;
        running.pending.insert(ticket, action);
        Ok(ticket)
    }

    fn consume_receipt(&mut self, receipt: crate::PiReceipt) {
        let Some(running) = &mut self.running else {
            return;
        };
        if let Some(wait) = &mut running.edit_wait
            && wait.ticket == receipt.ticket
            && wait.prompt
            && wait.prompt_wire_id.as_deref() == Some(receipt.wire_id.as_str())
            && let PiReceiptPhase::Accepted(response) | PiReceiptPhase::Outcome(response) =
                &receipt.phase
            && response["type"] == "response"
            && response["command"] == "prompt"
            && response["id"].as_str() == wait.prompt_wire_id.as_deref()
            && response["success"] == true
        {
            // Retain the real acceptance even if EOF/deadline already closed
            // this pump. It grants no install or successor authority.
            wait.prompt_accepted = true;
        }
        if let Some(control) = &mut running.control_wait
            && control.native == receipt.ticket
            && let PiReceiptPhase::Accepted(response) = &receipt.phase
            && response["success"] == true
            && response["id"].as_str() == Some(receipt.wire_id.as_str())
        {
            if control.cancellation
                && control.native == control.wait.ticket
                && response["command"] == "clear_queue"
            {
                control.clear_queue_confirmed = true;
            } else if control.cancellation
                && control.native != control.wait.ticket
                && response["command"] == "abort"
            {
                control.confirmed = true;
            }
        }
        if !running.client.pi_owns_ticket(receipt.ticket) {
            running.pending.remove(&receipt.ticket);
            if let Some(wait) = running.receipt_wait_mut(receipt.ticket) {
                wait.result = Some(Err(Error::Sdk(
                    "Pi 消息编辑操作的原生会话代际已失效".into(),
                )));
            }
            return;
        }
        if matches!(
            receipt.phase,
            PiReceiptPhase::Accepted(_)
                | PiReceiptPhase::Rejected(_)
                | PiReceiptPhase::Outcome(_)
                | PiReceiptPhase::AwaitingUser(_)
        ) && let Some(error) = running.client.pi_install_failure(receipt.deadline)
        {
            running.apply_event(&json!({"type":"velune_error","error":error}));
        }
        if matches!(receipt.phase, PiReceiptPhase::AwaitingUser(_)) {
            return;
        }
        if matches!(receipt.phase, PiReceiptPhase::Written) {
            if let Some(PendingAction::HistoryMetadata(request)) =
                running.pending.get_mut(&receipt.ticket)
            {
                request.deadline = receipt.deadline;
            }
            // One-way UI replies end at Written. Native controls await responses.
            if let Some(running) = &mut self.running
                && matches!(
                    running.pending.get(&receipt.ticket),
                    Some(PendingAction::DialogReply)
                )
            {
                running.pending.remove(&receipt.ticket);
                let timely_write = matches!(receipt.physical_write,
                    Some(crate::WriteFact::Written(completion)) if completion.met_deadline());
                if let Some(control) = &mut running.control_wait
                    && control.native == receipt.ticket
                    && !control.cancellation
                    && timely_write
                {
                    control.confirmed = true;
                }
                let failed = running.client.pi_cleanup_required();
                if let Some(wait) = running.receipt_wait_mut(receipt.ticket)
                    && wait.result.is_none()
                {
                    wait.result = Some(if failed {
                        Err(Error::Transport(
                            "Pi 回复实际写入后连接失败；原生处理未确认".into(),
                        ))
                    } else {
                        Ok(())
                    });
                }
            }
            return;
        }
        let Some(action) = self
            .running
            .as_mut()
            .and_then(|running| running.pending.remove(&receipt.ticket))
        else {
            return;
        };
        if let Some(running) = &mut self.running
            && (running.client.pi_cleanup_required() || !running.projection.can_sync_history())
        {
            running.pending_prompt = None;
            if let Some(wait) = running.receipt_wait_mut(receipt.ticket)
                && wait.result.is_none()
            {
                wait.result = Some(Err(match &receipt.phase {
                    PiReceiptPhase::NotSent => Error::Sdk("Pi 操作尚未发送；连接需要清理".into()),
                    PiReceiptPhase::Rejected(response) => Error::CommandFailed(
                        response["error"]
                            .as_str()
                            .unwrap_or("Pi command rejected")
                            .into(),
                    ),
                    _ => Error::Transport("Pi 编辑连接需要清理".into()),
                }));
            }
            return;
        }
        let scope_action = matches!(action, PendingAction::Branch | PendingAction::Rewind(_));
        let background_action = matches!(
            action,
            PendingAction::Background(_) | PendingAction::DiscoverBackground
        );
        let history_deferred = matches!(
            &action,
            PendingAction::HistoryMetadata(request)
                if matches!(request.snapshot, HistorySnapshot::Deferred)
        );
        let mut result = match receipt.phase {
            PiReceiptPhase::Accepted(response) | PiReceiptPhase::Outcome(response) => {
                self.complete_action(action, response, receipt.deadline, receipt.ticket)
            }
            PiReceiptPhase::Rejected(response) => {
                if matches!(action, PendingAction::HistoryMetadata(_)) {
                    self.running.as_mut().unwrap().history_pending = false;
                }
                if matches!(
                    action,
                    PendingAction::Background(_) | PendingAction::DiscoverBackground
                ) {
                    self.background_pending_sync = true;
                }
                if matches!(action, PendingAction::Thinking) {
                    let running = self.running.as_mut().unwrap();
                    running
                        .projection
                        .reject_prompt(response["error"].as_str().unwrap_or("Pi 拒绝推理强度"));
                    running.busy = false;
                    running.dialogs.clear();
                    running.publish_dialogs();
                }
                Err(Error::CommandFailed(
                    response["error"]
                        .as_str()
                        .unwrap_or("Pi command rejected")
                        .into(),
                ))
            }
            PiReceiptPhase::NotSent => {
                if self.running.as_ref().is_some_and(|running| {
                    running
                        .edit_wait
                        .as_ref()
                        .is_some_and(|wait| wait.ticket == receipt.ticket)
                        || running
                            .control_wait
                            .as_ref()
                            .is_some_and(|control| control.native == receipt.ticket)
                }) {
                    Err(Error::Sdk("Pi 操作尚未发送".into()))
                } else if scope_action {
                    Err(Error::Transport("Pi 分支操作未发送".into()))
                } else {
                    if matches!(action, PendingAction::Prompt) {
                        let running = self.running.as_mut().unwrap();
                        running.pending_prompt = None;
                        running.busy = false;
                        if !running.projection.turn_failed() {
                            running.projection.finish_handled_prompt();
                        }
                    }
                    Ok(())
                }
            }
            PiReceiptPhase::Indeterminate(detail) => Err(Error::Transport(detail)),
            PiReceiptPhase::Written | PiReceiptPhase::AwaitingUser(_) => unreachable!(),
        };
        if history_deferred
            && self.running.as_ref().is_some_and(|running| {
                running
                    .edit_wait
                    .as_ref()
                    .is_some_and(|wait| wait.ticket == receipt.ticket)
            })
            && result.is_ok()
        {
            result = Err(Error::Sdk("Pi 在消息编辑期间延后了原生历史快照".into()));
        }
        if let Err(error) = &result {
            if scope_action {
                self.running
                    .as_mut()
                    .unwrap()
                    .client
                    .release_pi_transition(receipt.ticket);
            }
            if let Some(running) = &mut self.running {
                running.history_pending = false;
                running.projection.apply_event(&json!({"type":"extension_error","event":"Pi operation","error":error.to_string()}));
            }
            if background_action {
                self.set_background_error(Some(error.to_string()));
            }
        }
        let clear_queue_ack = result.is_ok()
            && self.running.as_ref().is_some_and(|running| {
                running.control_wait.as_ref().is_some_and(|control| {
                    control.cancellation
                        && control.native == receipt.ticket
                        && control.native == control.wait.ticket
                })
            });
        if clear_queue_ack {
            let abort = self.enqueue(PiOperation::Abort, PendingAction::Control);
            if let Some(control) = self
                .running
                .as_mut()
                .and_then(|running| running.control_wait.as_mut())
            {
                match abort {
                    Ok(ticket) => control.native = ticket,
                    Err(error) => {
                        control.wait.result = Some(Err(Error::Sdk(format!(
                            "Pi 清队列已确认；Abort 尚未发送：{error}"
                        ))))
                    }
                }
            }
            return;
        }
        if let Some(running) = &mut self.running {
            let generation = running.client.pi_session_generation();
            if let Some(wait) = running.receipt_wait_mut(receipt.ticket) {
                if scope_action && result.is_ok() {
                    wait.installed_generation = generation.unwrap_or(wait.installed_generation);
                }
                if wait.result.is_none() {
                    wait.result = Some(result);
                }
            }
        }
    }

    fn ensure_pi_install_deadline(&mut self, deadline: Option<std::time::Instant>) -> Result<()> {
        let running = self.running()?;
        if let Some(error) = running.client.pi_install_failure(deadline) {
            running.apply_event(&json!({"type":"velune_error","error":error}));
            return Err(Error::Transport(error));
        }
        Ok(())
    }

    fn complete_action(
        &mut self,
        action: PendingAction,
        response: Value,
        deadline: Option<std::time::Instant>,
        ticket: PiTicket,
    ) -> Result<()> {
        // Timeout barriers survive all late native successes; no dependent dispatch.
        self.ensure_pi_install_deadline(deadline)?;
        match action {
            PendingAction::Thinking => {}
            PendingAction::DiscoverBackground => {
                self.background_refresh_queued = false;
                if response["data"]["commands"]
                    .as_array()
                    .is_some_and(|commands| {
                        commands
                            .iter()
                            .any(|entry| entry["name"] == "velune-background-work")
                    })
                {
                    self.refresh_background(json!({"operation":"snapshot"}));
                } else {
                    self.apply_background_frame(
                        &json!({"version":1,"kind":"unsupported","tasks":[]}),
                    );
                }
            }
            PendingAction::Background(revision) => {
                if response["data"]["disposition"] != "handled" {
                    return Err(Error::Sdk("Pi 后台工作查询未被 Velune 扩展处理".into()));
                }
                if self.running()?.background_frame_revision <= revision {
                    return Err(Error::Sdk("Pi 后台工作桥接没有返回结构化状态".into()));
                }
                if std::mem::take(&mut self.background_refresh_queued) {
                    self.refresh_background(json!({"operation":"snapshot"}));
                }
            }
            PendingAction::HistoryMetadata(request) => {
                self.running()?.history_pending = false;
                if response["data"]["disposition"] != "handled" {
                    return Err(Error::Sdk("原生会话元数据查询未被扩展处理".into()));
                }
                match request.snapshot {
                    HistorySnapshot::Installed => self.running()?.history_completed = true,
                    // An extension started a run before the snapshot barrier.
                    // Keep its live-only rows; retry once the run settles.
                    HistorySnapshot::Deferred => {}
                    HistorySnapshot::Invalid(error) => return Err(Error::Sdk(error)),
                    HistorySnapshot::Awaiting => {
                        return Err(Error::Sdk("原生会话投影缺少同源快照".into()));
                    }
                }
            }
            PendingAction::Branch => {
                if response["data"]["cancelled"] == true {
                    return Err(Error::Sdk("Pi 取消了分支操作".into()));
                }
                self.running()?.native_scope_changed()?;
                self.reset_background_session();
                self.invalidate_history();
                self.refresh_background(json!({"operation":"snapshot"}));
            }
            PendingAction::Rewind(id) => {
                let event = self
                    .running()?
                    .rewind
                    .take()
                    .ok_or_else(|| Error::Sdk("消息回退没有结果".into()))?;
                let outcome: Value =
                    serde_json::from_str(event["statusText"].as_str().unwrap_or(""))?;
                if response["data"]["disposition"] == "handled"
                    && outcome["entryId"] == id
                    && outcome["unmodified"] == true
                    && outcome["fallback"].is_string()
                    && matches!(
                        outcome["reason"].as_str(),
                        Some("user_leaf" | "prefix_context_edit")
                    )
                {
                    self.running()?.edit_fallback = Some(ticket);
                    return Err(Error::Sdk(
                        "Pi 编辑边界需要保真原生副本；原会话未修改".into(),
                    ));
                }
                if response["data"]["disposition"] != "handled"
                    || outcome["entryId"] != id
                    || outcome["fallback"].is_string()
                {
                    return Err(Error::Sdk("Pi 未能回到这条消息之前".into()));
                }
                self.ensure_pi_install_deadline(deadline)?;
                self.running()?.native_scope_changed()?;
                self.invalidate_history();
            }
            PendingAction::Rename => {
                self.enqueue(PiOperation::GetState, PendingAction::RenameState)?;
            }
            PendingAction::RenameState => {
                if let Some(snapshot) = self.running()?.projection.snapshot.as_mut() {
                    snapshot.conversation.title = crate::conversation::conversation_title(
                        response["data"]["sessionName"].as_str(),
                        None,
                    );
                    snapshot.revision = snapshot.revision.saturating_add(1);
                }
                self.invalidate_history();
            }
            PendingAction::Control
            | PendingAction::Steer
            | PendingAction::DialogReply
            | PendingAction::Prompt => {}
        }
        Ok(())
    }
}

#[cfg(all(test, unix))]
mod tests {
    use super::*;
    use crate::LiveConversation;

    fn poll_until(session: &mut PiSession, predicate: impl Fn(&PiSession) -> bool) {
        let deadline = std::time::Instant::now() + std::time::Duration::from_secs(2);
        loop {
            session.poll();
            if predicate(session) {
                return;
            }
            assert!(
                std::time::Instant::now() < deadline,
                "async Pi owner did not reach expected state"
            );
            std::thread::sleep(std::time::Duration::from_millis(5));
        }
    }

    #[test]
    fn pi_subagent_live_metadata_keeps_terminal_identities_separate_from_active_work() {
        let mut random = [0; 8];
        getrandom::fill(&mut random).unwrap();
        let root = std::env::temp_dir().join(format!(
            "velune-child-descriptors-{:x}",
            u64::from_le_bytes(random)
        ));
        std::fs::create_dir(&root).unwrap();
        let helper = root.join("rpc.py");
        std::fs::write(&helper, r#"import json,sys
for line in sys.stdin:
 r=json.loads(line)
 data={'commands':[]} if r['type']=='get_commands' else {'messages':[]}
 print(json.dumps({'type':'response','id':r['id'],'command':r['type'],'success':True,'data':data}),flush=True)
"#).unwrap();
        let config: Config = serde_json::from_value(json!({"execution":{"kind":"local"},"os_home":root,"binary":"/usr/bin/python3","node_binary":"/usr/bin/python3","rpc_entry":helper})).unwrap();
        let client = Client::spawn(config.clone()).unwrap();
        let mut session = PiSession::new(config);
        session.start(
            client,
            PiProjection::new(ConversationSummary {
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
        session.config.session = Some("/synthetic/parent-a.jsonl".into());
        session.apply_background_frame(&json!({"version":1,"kind":"snapshot","parentSessionFile":"/synthetic/parent-b.jsonl",
            "tasks":[{"id":"foreign-task","status":"running"}],
            "children":[{"sessionFile":"/synthetic/foreign-child.jsonl","title":"foreign child","canStop":true}],
            "subAgents":[{"sessionFile":"/synthetic/foreign-child.jsonl","title":"foreign descriptor"}],
            "taskOperation":{"error":"foreign operation"},
            "persistedNotificationIds":["foreign-notification"]
        }));
        assert!(session.snapshot().unwrap().sub_agents.is_empty());
        assert!(session.work().is_empty());
        assert!(session.children.is_empty());
        assert!(session.task_items.is_empty());
        assert!(session.task_issue.is_none());
        assert!(
            !session
                .acknowledged_notifications
                .contains("foreign-notification")
        );
        session.apply_background_frame(&json!({"version":1,"kind":"snapshot","parentSessionFile":"/synthetic/parent-a.jsonl","tasks":[
            {"id":"shell","status":"running","kind":"shell","description":"Ordinary shell","childSessionFile":"/synthetic/not-native.jsonl"}
        ],"subAgents":[{"sessionFile":"/synthetic/child.jsonl","title":"worker：缓存检查","failed":false}],
        "children":[{"sessionFile":"/synthetic/child.jsonl","title":"fallback","state":"running","canStop":true}]}));
        let agents = &session.snapshot().unwrap().sub_agents;
        assert_eq!(
            agents.len(),
            1,
            "ordinary background tasks do not create native identities"
        );
        assert_eq!(agents[0].title, "worker：缓存检查");
        assert!(
            session
                .work()
                .iter()
                .any(|task| task.conversation_id.as_deref() == Some("pi:/synthetic/child.jsonl"))
        );
        session.apply_background_frame(&json!({"version":1,"kind":"snapshot","tasks":[],"children":[],"subAgents":[{"sessionFile":"/synthetic/child.jsonl","title":"worker：缓存检查","failed":true}]}));
        assert!(session.work().is_empty());
        assert!(session.snapshot().unwrap().sub_agents[0].failed);
        session.apply_background_frame(
            &json!({"version":1,"kind":"unknown","childrenUnknown":true,"children":[]}),
        );
        assert_eq!(session.snapshot().unwrap().sub_agents.len(), 1);
        assert!(session.has_pending_metadata());
        // The SDK owner is newer than the launch configuration. Follow it in
        // both directions rather than accepting frames for the stale launch A.
        for (owner, rejected) in [("b", "a"), ("a", "b")] {
            let parent = format!("/synthetic/parent-{owner}.jsonl");
            session.apply_events(vec![json!({"type":"extension_ui_request","method":"setStatus",
                "statusKey":"velune.session-metadata","statusText":json!({"sessionFile":parent}).to_string()
            })]);
            assert!(
                session.snapshot().unwrap().sub_agents.is_empty(),
                "switching the SDK owner clears the previous parent's descriptors"
            );
            session.apply_background_frame(&json!({"version":1,"kind":"snapshot",
                "parentSessionFile":format!("/synthetic/parent-{rejected}.jsonl"),
                "tasks":[],"children":[],"subAgents":[{"sessionFile":"/synthetic/stale.jsonl","title":"stale parent"}]
            }));
            assert!(session.snapshot().unwrap().sub_agents.is_empty());
            session.apply_background_frame(&json!({"version":1,"kind":"snapshot","parentSessionFile":parent,
                "tasks":[],"children":[],"subAgents":[{"sessionFile":format!("/synthetic/child-{owner}.jsonl"),"title":format!("owner {owner}")}]
            }));
            assert_eq!(session.snapshot().unwrap().sub_agents.len(), 1);
            assert_eq!(
                session.snapshot().unwrap().sub_agents[0].title,
                format!("owner {owner}")
            );
        }
        session.shutdown().unwrap();
        drop(session);
        std::fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn background_tasks_async_output_and_exact_stop_do_not_block_or_interrupt_parent() {
        let mut random = [0; 8];
        getrandom::fill(&mut random).unwrap();
        let root = std::env::temp_dir().join(format!(
            "velune-task-output-{:x}",
            u64::from_le_bytes(random)
        ));
        std::fs::create_dir(&root).unwrap();
        let helper = root.join("rpc.py");
        std::fs::write(&helper, r#"
import json,sys,time,base64,os
def emit(value): print(json.dumps(value),flush=True)
tasks=[{'id':'a','kind':'shell','description':'Shell A','status':'running','canStop':True,'canReadOutput':True},{'id':'b','kind':'shell','description':'Shell B','status':'running','canStop':True,'canReadOutput':True}]
for line in sys.stdin:
 r=json.loads(line); command=r['type']; data={}
 with open(os.path.join(os.path.dirname(__file__),'requests.jsonl'),'a') as f: f.write(json.dumps(r)+'\n')
 if command=='get_commands': data={'commands':[{'name':'velune-background-work'}]}
 elif command=='get_messages': data={'messages':[]}
 elif command=='prompt' and r['message'].startswith('/velune-projection-sync '):
  emit({'type':'extension_ui_request','method':'setStatus','statusKey':'velune.session-metadata','statusText':json.dumps({'requestId':r['id'],'snapshotVersion':1,'messages':[],'messageIds':[],'identityConfirmations':[]})})
  data={'disposition':'handled'}
 elif command=='prompt':
  encoded=r['message'].split(' ',1)[1]; request=json.loads(base64.urlsafe_b64decode(encoded+'='*((-len(encoded))%4)))
  frame={'version':1,'kind':'snapshot','tasks':tasks}
  if request['operation']=='task_output':
   time.sleep(0.5)
   frame['taskOutput']={'taskId':request['taskId'],'requestId':request['requestId'],'text':'Shell A output','truncated':False}
  elif request['operation']=='stop_task':
   for task in tasks:
    if task['id']==request['taskId']: task['status']='killed'; task['canStop']=False
   frame['tasks']=tasks; frame['taskOperation']={'taskId':request['taskId']}
  emit({'type':'extension_ui_request','method':'setStatus','statusKey':'velune.background-work-v1','statusText':json.dumps(frame)})
  data={'disposition':'handled'}
 emit({'type':'response','id':r['id'],'command':command,'success':True,'data':data})
"#).unwrap();
        let config: Config = serde_json::from_value(json!({"execution":{"kind":"local"},"os_home":root,"binary":"/usr/bin/python3","node_binary":"/usr/bin/python3","rpc_entry":helper})).unwrap();
        let mut client = Client::spawn(config.clone()).unwrap();
        // Production execution performs the patient readiness snapshot before
        // its first ordinary 3s metadata command. Keep the fixture faithful.
        client.state().unwrap();
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
        poll_until(&mut session, |session| {
            !session.background_tasks().tasks.is_empty()
        });
        session.apply_events(vec![json!({"type":"agent_start"})]);
        assert!(
            session
                .background_tasks()
                .tasks
                .iter()
                .any(|task| task.id == "a" && task.can_read_output),
            "initial task frame failed: {:?}; metadata={:?}",
            session.background_error,
            session.background_tasks()
        );
        let start = std::time::Instant::now();
        let output = session.read_background_task_output("a", None).unwrap();
        assert!(
            start.elapsed() < std::time::Duration::from_millis(250),
            "output request must not wait for the host"
        );
        assert_eq!(
            output.status,
            crate::conversation::BackgroundTaskOutputStatus::Checking
        );
        let deadline = std::time::Instant::now() + std::time::Duration::from_secs(3);
        loop {
            session.poll();
            if session
                .read_background_task_output("a", Some(&output.request_id))
                .unwrap()
                .status
                == crate::conversation::BackgroundTaskOutputStatus::Ready
            {
                break;
            }
            assert!(std::time::Instant::now() < deadline);
            std::thread::sleep(std::time::Duration::from_millis(5));
        }
        assert_eq!(
            session
                .read_background_task_output("a", Some(&output.request_id))
                .unwrap()
                .text,
            "Shell A output"
        );
        assert!(session.turn_running());
        session.stop_background_task("a").unwrap();
        while session
            .background_tasks()
            .tasks
            .iter()
            .any(|task| task.id == "a" && task.status != BackgroundTaskStatus::Cancelled)
        {
            session.poll();
            assert!(std::time::Instant::now() < deadline);
            std::thread::sleep(std::time::Duration::from_millis(5));
        }
        assert!(session.turn_running());
        assert_eq!(
            session
                .background_tasks()
                .tasks
                .iter()
                .find(|task| task.id == "b")
                .unwrap()
                .status,
            BackgroundTaskStatus::Running
        );
        let requests = std::fs::read_to_string(root.join("requests.jsonl")).unwrap();
        assert!(
            !requests
                .lines()
                .any(|line| serde_json::from_str::<Value>(line).unwrap()["type"] == "abort")
        );
        session.shutdown().unwrap();
        std::fs::remove_dir_all(root).unwrap();
    }

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
    fn compacted_notification_receipts_settle_without_claiming_undelivered_work() {
        let config: Config = serde_json::from_value(json!({
            "execution":{"kind":"local"}, "binary":"/bin/pi", "node_binary":"/bin/node",
        }))
        .unwrap();
        let mut session = PiSession::new(config);
        // get_messages no longer includes this notification after compaction.
        // The SDK archive still proves it was delivered to this session.
        session.apply_background_frame(&json!({
            "version":1,"kind":"snapshot","tasks":[
                {"id":"before-compaction","status":"completed","notifyOnCompletion":true,"notified":true}
            ],"persistedNotificationIds":["before-compaction"]
        }));
        assert!(session.work_settled());
        assert!(!session.has_pending_metadata());

        session.apply_background_frame(&json!({
            "version":1,"kind":"snapshot","tasks":[
                {"id":"before-compaction","status":"completed","notifyOnCompletion":true,"notified":true},
                {"id":"in-flight","status":"completed","notifyOnCompletion":true,"notified":true},
                {"id":"actual-work","status":"running"}
            ],"persistedNotificationIds":["before-compaction"]
        }));
        assert!(
            session.has_pending_metadata(),
            "notified alone is not delivery proof"
        );
        assert_eq!(session.work().len(), 1);
        session.apply_background_frame(&json!({
            "version":1,"kind":"snapshot","tasks":[
                {"id":"in-flight","status":"completed","notifyOnCompletion":true,"notified":true},
                {"id":"actual-work","status":"running"}
            ],"persistedNotificationIds":["in-flight"]
        }));
        assert!(!session.has_pending_metadata());
        assert!(
            !session.work_settled(),
            "a receipt must not erase active work"
        );
    }

    #[test]
    fn notification_receipt_snapshot_observes_preceding_agent_start() {
        let mut random = [0; 8];
        getrandom::fill(&mut random).unwrap();
        let root = std::env::temp_dir().join(format!(
            "velune-pi-receipt-order-{:x}",
            u64::from_le_bytes(random)
        ));
        std::fs::create_dir(&root).unwrap();
        let helper = root.join("rpc.py");
        std::fs::write(&helper, r#"
import json,sys
def emit(value): print(json.dumps(value),flush=True)
started=False
for line in sys.stdin:
 r=json.loads(line); command=r['type']; data={}
 if command=='get_commands': data={'commands':[{'name':'velune-background-work'}]}
 elif command=='prompt':
  if not started: emit({'type':'agent_start'}); started=True
  frame={'version':1,'kind':'snapshot','tasks':[{'id':'old','status':'completed','notifyOnCompletion':True,'notified':True}],'persistedNotificationIds':['old']}
  emit({'type':'extension_ui_request','method':'setStatus','statusKey':'velune.background-work-v1','statusText':json.dumps(frame)})
  data={'disposition':'handled'}
 elif command=='abort': emit({'type':'agent_settled','aborted':True})
 emit({'type':'response','id':r['id'],'command':command,'success':True,'data':data})
"#).unwrap();
        let config: Config = serde_json::from_value(json!({
            "execution":{"kind":"local"}, "os_home":root,
            "binary":"/usr/bin/python3", "node_binary":"/usr/bin/python3", "rpc_entry":helper,
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
        poll_until(&mut session, |session| !session.has_pending_metadata());
        assert!(!session.has_pending_metadata());
        assert!(
            session.turn_running(),
            "settling old receipts must retain the new run"
        );
        assert_eq!(session.snapshot().unwrap().run_state, RunState::Running);
        assert!(session.can_steer());
        session.cancel().unwrap();
        poll_until(&mut session, |session| !session.turn_running());
        assert!(!session.turn_running());
        assert_eq!(session.snapshot().unwrap().run_state, RunState::Idle);
        session.shutdown().unwrap();
        std::fs::remove_dir_all(root).unwrap();
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
    if command == 'prompt' and request['message'].startswith('/velune-background-work '):
        emit({'type':'extension_ui_request','method':'setStatus','statusKey':'velune.background-work-v1','statusText':json.dumps({'version':1,'kind':'unsupported','supported':False,'tasks':[]})})
        data = {'disposition':'handled'}
    elif command == 'prompt':
        if not started:
            emit({'type':'agent_start'})
            started = True
        emit({'type':'extension_ui_request','method':'setStatus',
              'statusKey':'velune.session-metadata',
              'statusText':json.dumps({'requestId':request['id'],'snapshotVersion':1,'messages':[], 'messageIds':[], 'identityConfirmations':[]})})
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
        assert!(
            !session.sync_history("pi", None).unwrap(),
            "history admission is not a completed fence"
        );
        poll_until(&mut session, |session| {
            !session.running.as_ref().unwrap().history_pending
        });
        assert!(!session.running.as_ref().unwrap().history_completed);
        assert!(
            !session.sync_history("pi", None).unwrap(),
            "the new run defers native history replacement"
        );
        assert!(session.turn_running());
        assert!(session.can_steer());
        let snapshot = session.snapshot().unwrap();
        assert_eq!(snapshot.run_state, RunState::Running);
        assert!(!snapshot.actions.can_send);
        assert!(snapshot.actions.can_cancel);
        session.cancel().unwrap();
        poll_until(&mut session, |session| !session.turn_running());
        assert!(!session.turn_running());
        assert!(!session.can_steer());
        assert!(session.snapshot().unwrap().actions.can_send);
        session.shutdown().unwrap();
        std::fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn preflight_dialog_does_not_wait_for_prompt_ack_and_routes_reply() {
        use crate::conversation::{InteractionAnswer, InteractionKind, RuntimeInteractionReply};
        let mut random = [0; 8];
        getrandom::fill(&mut random).unwrap();
        let root =
            std::env::temp_dir().join(format!("velune-pi-dialog-{:x}", u64::from_le_bytes(random)));
        std::fs::create_dir(&root).unwrap();
        let helper = root.join("rpc.py");
        std::fs::write(&helper, r#"
import json, sys
pending = None
def emit(value): print(json.dumps(value), flush=True)
for line in sys.stdin:
    request = json.loads(line)
    command = request['type']
    data = {}
    if command == 'prompt':
        if request['message'].startswith('/velune'):
            data = {'disposition':'handled'}
        else:
            pending = request
            emit({'type':'extension_ui_request','id':'dialog','method':'editor','title':'Edit','prefill':'initial'})
            continue
    elif command == 'extension_ui_response':
        assert request.get('value') == 'edited' or request.get('cancelled') is True
        failed = pending['message'] == 'reject'
        emit({'type':'response','id':pending['id'],'command':'prompt','success':not failed,
              'error':'synthetic rejection','data':{'disposition':'handled'}})
        continue
    elif command == 'get_messages': data = {'messages':[]}
    elif command == 'abort': emit({'type':'agent_settled','aborted':True})
    emit({'type':'response','id':request['id'],'command':command,'success':True,'data':data})
"#).unwrap();
        let config: Config = serde_json::from_value(json!({
            "execution":{"kind":"local"},"os_home":root,
            "binary":"/usr/bin/python3","node_binary":"/usr/bin/python3","rpc_entry":helper,
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
        for text in ["handled", "reject", "cancel"] {
            session
                .send(&TurnInput {
                    text: text.into(),
                    effort: None,
                    permission_mode: None,
                    images: vec![],
                })
                .unwrap();
            poll_until(&mut session, |session| {
                !session.snapshot().unwrap().pending_interactions.is_empty()
            });
            let snapshot = session.snapshot().unwrap();
            assert_eq!(snapshot.pending_interactions.len(), 1);
            let InteractionKind::UserInput { questions } = &snapshot.pending_interactions[0].kind
            else {
                panic!("editor question")
            };
            assert_eq!(questions[0].default_text.as_deref(), Some("initial"));
            assert!(questions[0].multiline);
            let revision = snapshot.revision;
            if text == "cancel" {
                session.cancel().unwrap();
            } else {
                session
                    .reply(
                        "dialog",
                        RuntimeInteractionReply::Answers {
                            answers: vec![InteractionAnswer {
                                question_id: "dialog".into(),
                                values: vec!["edited".into()],
                            }],
                        },
                    )
                    .unwrap();
            }
            assert!(session.snapshot().unwrap().pending_interactions.is_empty());
            assert!(session.snapshot().unwrap().revision > revision);
            let deadline = std::time::Instant::now() + std::time::Duration::from_secs(2);
            while session.turn_running() && std::time::Instant::now() < deadline {
                session.poll();
                std::thread::sleep(std::time::Duration::from_millis(5));
            }
            assert!(!session.turn_running());
            assert!(session.snapshot().unwrap().actions.can_send);
            if text == "reject" {
                assert!(session.turn_failed());
                assert!(session.snapshot().unwrap().messages.iter().any(|message|
                    message.blocks.iter().any(|block| matches!(block,crate::conversation::MessageBlock::Notice {text} if text == "synthetic rejection"))));
            }
        }
        session.shutdown().unwrap();
        std::fs::remove_dir_all(root).unwrap();
    }
}

#[cfg(all(test, unix))]
mod port_tests;

#[cfg(all(test, unix))]
mod snapshot_tests;

#[cfg(test)]
mod resume_history_tests;

#[cfg(all(test, unix))]
#[path = "pi_session/edit_tests.rs"]
mod edit_tests;
