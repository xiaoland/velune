//! Codex app-server owns the live terminal inventory for a loaded thread.
use super::*;
use std::time::{Duration, Instant};

const CHECK_INTERVAL: Duration = Duration::from_secs(2);
const LIST_METHOD: &str = "thread/backgroundTerminals/list";
const STOP_METHOD: &str = "thread/backgroundTerminals/terminate";

#[derive(Default)]
pub(super) struct Terminals {
    tasks: BTreeMap<String, Entry>,
    list: Option<List>,
    stops: BTreeMap<u64, (String, Instant)>,
    next_check: Option<Instant>,
    shell_in_turn: bool,
    settle: bool,
    fresh: bool,
    issue: Option<&'static str>,
    unsupported: bool,
}

struct Entry {
    // The app-server process identity, scoped to this connection and thread.
    process: String,
    title: String,
    status: BackgroundTaskStatus,
    stop_confirmed: bool,
}

struct List {
    request: u64,
    required: bool,
    deadline: Instant,
    entries: BTreeMap<String, (Entry, DataRetention)>,
    cursors: BTreeMap<String, DataRetention>,
    ended: BTreeMap<String, (BackgroundTaskStatus, DataRetention)>,
}

impl Entry {
    fn active(&self) -> bool {
        matches!(
            self.status,
            BackgroundTaskStatus::Running | BackgroundTaskStatus::Stopping
        )
    }
}

impl Terminals {
    pub(super) fn contains(&self, task: &str) -> bool {
        self.tasks.contains_key(task)
    }

    pub(super) fn active_work(&self) -> impl Iterator<Item = BackgroundTask> + '_ {
        self.tasks
            .values()
            .filter(|task| task.active())
            .map(|task| BackgroundTask {
                conversation_id: None,
                state: match task.status {
                    BackgroundTaskStatus::Stopping => BackgroundState::Stopping,
                    _ => BackgroundState::Running,
                },
            })
    }

    pub(super) fn settling(&self) -> bool {
        self.settle || self.list.as_ref().is_some_and(|list| list.required)
    }

    fn unavailable(&mut self, issue: &'static str) {
        self.fresh = false;
        self.issue = Some(issue);
    }

    fn task_list(&self, connected: bool) -> BackgroundTaskList {
        let live = connected && self.fresh;
        let tasks = self
            .tasks
            .iter()
            .map(|(id, entry)| BackgroundTaskInfo {
                id: id.clone(),
                kind: "shell".into(),
                title: entry.title.clone(),
                status: if !live && entry.active() {
                    BackgroundTaskStatus::Unknown
                } else {
                    entry.status
                },
                conversation_id: None,
                child_session_id: None,
                can_stop: live && entry.status == BackgroundTaskStatus::Running,
                can_read_output: false,
            })
            .collect();
        BackgroundTaskList {
            tasks,
            live,
            issue: if connected {
                self.issue.map(str::to_owned)
            } else {
                None
            },
        }
    }
}

impl NativeSession {
    pub fn has_active_background_terminals(&self) -> bool {
        self.codex.terminals.active_work().next().is_some()
    }

    pub(super) fn codex_background_tasks(&self) -> Result<Retained<BackgroundTaskList>> {
        let retention = self
            .data
            .try_clone(|owner| matches!(owner, DataOwner::Task(_)))?;
        Ok(Retained::from_parts(
            self.codex
                .terminals
                .task_list(!self.failed && !self.process_ended && self.native_id.is_some()),
            retention,
        ))
    }

    pub fn refresh_background_tasks(&mut self) {
        if self.config.kind == NativeKind::Codex166 {
            self.codex_start_terminal_list(false);
        }
    }

    pub(super) fn codex_stop_background_task(&mut self, task: &str) -> Result<()> {
        let terminals = &mut self.codex.terminals;
        let (Some(thread), Some(entry)) = (&self.native_id, terminals.tasks.get_mut(task)) else {
            return Err(Error::new("当前不能独立停止此 Codex 后台终端"));
        };
        if self.failed
            || self.process_ended
            || !terminals.fresh
            || entry.status != BackgroundTaskStatus::Running
        {
            return Err(Error::new("当前不能独立停止此 Codex 后台终端"));
        }
        let deadline = Instant::now() + rpc::CONTROL_TIMEOUT;
        let request = self.rpc.enqueue_request_with_class(
            STOP_METHOD,
            &json!({"threadId": thread, "processId": entry.process}),
            crate::transport::WriteClass::ProtocolReplyCancel,
            deadline,
        )?;
        entry.status = BackgroundTaskStatus::Stopping;
        terminals.stops.insert(request, (task.into(), deadline));
        Ok(())
    }

    fn codex_start_terminal_list(&mut self, required: bool) {
        let terminals = &mut self.codex.terminals;
        let now = Instant::now();
        let required = required || terminals.settle;
        if self.failed
            || self.process_ended
            || terminals.unsupported
            || terminals.list.is_some()
            || (!required && terminals.next_check.is_some_and(|next| now < next))
        {
            return;
        }
        let Some(thread) = &self.native_id else {
            return;
        };
        let deadline = now + rpc::CONTROL_TIMEOUT;
        terminals.next_check = Some(now + CHECK_INTERVAL);
        terminals.settle = false;
        match self
            .rpc
            .enqueue_request(LIST_METHOD, &json!({"threadId": thread}), deadline)
        {
            Ok(request) => {
                terminals.list = Some(List {
                    request,
                    required,
                    deadline,
                    entries: BTreeMap::new(),
                    cursors: Default::default(),
                    ended: BTreeMap::new(),
                })
            }
            Err(_) => terminals.unavailable("无法查询 Codex 后台终端。"),
        }
    }

    pub(super) fn codex_poll_terminals(&mut self) {
        let now = Instant::now();
        let terminals = &mut self.codex.terminals;
        if terminals
            .list
            .as_ref()
            .is_some_and(|list| list.deadline <= now)
        {
            terminals.list = None;
            terminals.unavailable("Codex 后台终端查询超时。");
        }
        terminals.stops.retain(|_, (id, deadline)| {
            if *deadline > now {
                return true;
            }
            if let Some(task) = terminals.tasks.get_mut(id)
                && task.status == BackgroundTaskStatus::Stopping
            {
                task.status = BackgroundTaskStatus::Running;
                terminals.issue = Some("Codex 后台终端停止请求超时。");
            }
            false
        });
        if terminals.settle || terminals.tasks.values().any(Entry::active) {
            self.codex_start_terminal_list(self.codex.terminals.settle);
        }
    }

    pub(super) fn codex_settle_terminals(&mut self) {
        let terminals = &mut self.codex.terminals;
        terminals.settle |= std::mem::take(&mut terminals.shell_in_turn) && !terminals.unsupported;
    }

    // Observe completions before the foreground turn filter. A shell may exit
    // after its original turn or while the next turn is running.
    pub(super) fn codex_terminal_item(&mut self, method: &str, params: &Value) {
        let item = &params["item"];
        if !matches!(method, "item/started" | "item/completed")
            || item["type"] != "commandExecution"
        {
            return;
        }
        let terminals = &mut self.codex.terminals;
        if self.turn_id.is_some() && params["turnId"].as_str() == self.turn_id.as_deref() {
            terminals.shell_in_turn = true;
        }
        let (Some(item_id), Some(process)) = (item["id"].as_str(), item["processId"].as_str())
        else {
            return;
        };
        let id = terminal_id(&self.epoch, item_id, process);
        if method == "item/started"
            && item["source"] == "unifiedExecStartup"
            && !terminals.unsupported
        {
            // The item proves a PTY was allocated, and the inventory proves
            // whether it is still running independently.
            terminals.settle = true;
        }
        if method != "item/completed" || item["source"] == "unifiedExecInteraction" {
            return;
        }
        let status = match item["status"].as_str() {
            Some("completed") => BackgroundTaskStatus::Completed,
            Some("failed" | "declined") => BackgroundTaskStatus::Failed,
            _ => return,
        };
        if let Some(task) = terminals.tasks.get_mut(&id) {
            task.status = if status == BackgroundTaskStatus::Failed && task.stop_confirmed {
                BackgroundTaskStatus::Cancelled
            } else {
                status
            };
        }
        if let Some(list) = &mut terminals.list {
            list.ended.insert(id, (status, self.data.branch()));
        }
    }

    pub(super) fn codex_terminal_reply(&mut self, record: &Value) -> bool {
        let Some(request) = record["id"]
            .as_u64()
            .filter(|_| record.get("method").is_none())
        else {
            return false;
        };
        let state = &mut self.codex.terminals;
        if let Some((id, _)) = state.stops.remove(&request) {
            if let Some(task) = state.tasks.get_mut(&id) {
                if record["result"]["terminated"] == true {
                    task.stop_confirmed = true;
                    if task.status == BackgroundTaskStatus::Failed {
                        task.status = BackgroundTaskStatus::Cancelled;
                    }
                } else if task.status == BackgroundTaskStatus::Stopping {
                    task.status = BackgroundTaskStatus::Running;
                    state.issue = Some("Codex 未确认停止此后台终端。");
                }
            }
            state.next_check = None;
            return true;
        }
        if state
            .list
            .as_ref()
            .is_some_and(|list| list.request == request)
        {
            self.codex_terminal_list_page(record);
            return true;
        }
        false
    }

    fn codex_terminal_list_page(&mut self, record: &Value) {
        let state = &mut self.codex.terminals;
        let mut list = state.list.take().expect("correlated terminal list");
        let result = &record["result"];
        let Some(items) = result["data"].as_array() else {
            if record["error"]["code"] == -32601 {
                state.unsupported = true;
                state.settle = false;
                state.unavailable("此版本 Codex 未提供后台终端查询。");
            } else {
                state.unavailable("Codex 未返回可用的后台终端清单。");
            }
            return;
        };
        for value in items {
            let (Some(item), Some(process), Some(title)) = (
                value["itemId"].as_str(),
                value["processId"].as_str(),
                value["command"].as_str(),
            ) else {
                state.unavailable("Codex 返回的后台终端清单不完整。");
                return;
            };
            if item.is_empty() || process.is_empty() {
                state.unavailable("Codex 返回的后台终端缺少身份。");
                return;
            }
            let id = terminal_id(&self.epoch, item, process);
            list.entries.insert(
                id,
                (
                    Entry {
                        process: process.to_owned(),
                        title: title.to_owned(),
                        status: BackgroundTaskStatus::Running,
                        stop_confirmed: false,
                    },
                    self.data.branch(),
                ),
            );
        }
        if let Some(cursor) = result["nextCursor"].as_str() {
            if list
                .cursors
                .insert(cursor.to_owned(), self.data.branch())
                .is_none()
                && let Ok(request) = self.rpc.enqueue_request(
                    LIST_METHOD,
                    &json!({"threadId": self.native_id, "cursor": cursor}),
                    list.deadline,
                )
            {
                list.request = request;
                state.list = Some(list);
            } else {
                state.unavailable("无法读取完整的 Codex 后台终端清单。");
            }
            return;
        }
        for (id, task) in &mut state.tasks {
            if task.active() && !list.entries.contains_key(id) {
                task.status = BackgroundTaskStatus::Unknown;
            }
        }
        for (id, (mut entry, retention)) in list.entries {
            if let Some(previous) = state.tasks.get(&id) {
                entry.stop_confirmed = previous.stop_confirmed;
                entry.status = match previous.status {
                    BackgroundTaskStatus::Unknown if !previous.stop_confirmed => {
                        BackgroundTaskStatus::Running
                    }
                    BackgroundTaskStatus::Unknown => BackgroundTaskStatus::Stopping,
                    status => status,
                };
            }
            if let Some((status, _retention)) = list.ended.get(&id) {
                entry.status = match status {
                    BackgroundTaskStatus::Failed if entry.stop_confirmed => {
                        BackgroundTaskStatus::Cancelled
                    }
                    status => *status,
                };
            }
            self.data.insert(DataOwner::Task(id.clone()), retention);
            state.tasks.insert(id, entry);
        }
        while state.tasks.len() > 128
            && let Some(id) = state
                .tasks
                .iter()
                .find_map(|(id, entry)| (!entry.active()).then(|| id.clone()))
        {
            state.tasks.remove(&id);
        }
        state.fresh = true;
        state.issue = None;
    }
}

fn terminal_id(epoch: &str, item: &str, process: &str) -> String {
    format!("codex-terminal:{epoch}:{}:{item}:{process}", item.len())
}
