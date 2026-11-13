//! Task-scoped Claude stream-json observations and nonblocking control.
use super::*;

impl NativeSession {
    pub(super) fn claude_background_record(&mut self, record: &Value) {
        if record["type"] == "control_response" {
            let response = &record["response"];
            if let Some((id, _)) = response["request_id"]
                .as_str()
                .and_then(|id| self.claude.task_stops.remove(id))
                && response["subtype"] != "success"
            {
                self.claude.task_issue =
                    Some("运行时未接受单项停止请求，任务状态将继续同步。".into());
                let detail = response["error"]
                    .as_str()
                    .unwrap_or("")
                    .to_ascii_lowercase();
                if detail.contains("stop_task")
                    && (detail.contains("unsupported")
                        || detail.contains("unknown")
                        || detail.contains("not supported"))
                {
                    self.claude.task_stop_capable = false;
                    for task in self.claude.task_infos.values_mut() {
                        task.can_stop = false;
                    }
                }
                if self.claude.task_runs.values().any(|current| current == &id)
                    && let Some(task) = self.claude.task_infos.get_mut(&id)
                    && task.status == BackgroundTaskStatus::Stopping
                {
                    task.status = BackgroundTaskStatus::Running;
                    task.can_stop = false;
                }
            }
            return;
        }
        if record["type"] != "system" {
            return;
        }
        let subtype = record["subtype"].as_str().unwrap_or("");
        if subtype == "background_tasks_changed" {
            let Some(tasks) = record["tasks"].as_array() else {
                return;
            };
            let mut current = std::collections::BTreeSet::new();
            for task in tasks {
                if let Some(id) = self.claude_observe_task(task) {
                    current.insert(id);
                }
            }
            for (id, task) in &mut self.claude.task_infos {
                if !current.contains(id)
                    && matches!(
                        task.status,
                        BackgroundTaskStatus::Running | BackgroundTaskStatus::Stopping
                    )
                {
                    task.status = BackgroundTaskStatus::Unknown;
                    task.can_stop = false;
                }
            }
            self.claude.tasks.retain(|id| {
                self.claude
                    .task_runs
                    .get(id)
                    .is_some_and(|id| current.contains(id))
            });
            self.claude_prune_background_tasks();
            return;
        }
        if matches!(subtype, "task_started" | "task_progress") {
            self.claude_observe_task(record);
        } else if matches!(subtype, "task_notification" | "task_updated") {
            let Some(native_id) = record["task_id"].as_str() else {
                return;
            };
            let public = record["run_id"]
                .as_str()
                .map(|run| format!("{}:{}:{native_id}{run}", self.epoch, native_id.len()))
                .or_else(|| self.claude.task_runs.get(native_id).cloned())
                .or_else(|| self.claude_observe_task(record));
            let Some(public) = public else { return };
            if !self.claude.task_infos.contains_key(&public) {
                self.claude.task_order.push_back(public.clone());
                self.claude.task_infos.insert(
                    public.clone(),
                    BackgroundTaskInfo {
                        id: public.clone(),
                        kind: record["task_type"].as_str().unwrap_or("task").into(),
                        title: crate::background::text(
                            record["description"].as_str().unwrap_or("后台任务"),
                            512,
                        )
                        .0,
                        status: BackgroundTaskStatus::Unknown,
                        conversation_id: None,
                        child_session_id: None,
                        can_stop: false,
                        can_read_output: false,
                    },
                );
            }
            let state = record["status"]
                .as_str()
                .or_else(|| record["patch"]["status"].as_str());
            if let Some(task) = self.claude.task_infos.get_mut(&public) {
                if let Some(state) = state {
                    task.status = crate::background::status(state);
                }
                task.can_stop = self.claude.task_stop_capable
                    && task.status == BackgroundTaskStatus::Running
                    && self.claude.task_runs.get(native_id) == Some(&public);
            }
            if let Some(summary) = record["summary"]
                .as_str()
                .or_else(|| record["output"].as_str())
            {
                self.claude
                    .task_outputs
                    .insert(public.clone(), crate::background::text(summary, 65_536));
                if let Some(task) = self.claude.task_infos.get_mut(&public) {
                    task.can_read_output = true;
                }
            }
        }
        self.claude_prune_background_tasks();
    }

    fn claude_observe_task(&mut self, record: &Value) -> Option<String> {
        if record["ambient"] == true {
            return None;
        }
        let native = record["task_id"].as_str().filter(|id| !id.is_empty())?;
        let run = record["run_id"].as_str();
        let public = run.map_or_else(
            || {
                self.claude
                    .task_runs
                    .get(native)
                    .cloned()
                    .unwrap_or_else(|| format!("{}:{}:{native}", self.epoch, native.len()))
            },
            |run| format!("{}:{}:{native}{run}", self.epoch, native.len()),
        );
        if record["subtype"] == "task_progress"
            && self
                .claude
                .task_runs
                .get(native)
                .is_some_and(|current| current != &public)
        {
            return None;
        }
        if let Some(previous) = self
            .claude
            .task_runs
            .insert(native.into(), public.clone())
            .filter(|old| old != &public)
            && let Some(task) = self.claude.task_infos.get_mut(&previous)
        {
            task.status = BackgroundTaskStatus::Unknown;
            task.can_stop = false;
        }
        let kind = record["task_type"].as_str().unwrap_or("task");
        let conversation_id = if kind == "local_agent" {
            self.claude_agent_conversation(native)
                .filter(|id| self.sub_agents.contains(id))
        } else {
            None
        };
        let title = record["description"]
            .as_str()
            .filter(|value| !value.is_empty())
            .unwrap_or(kind);
        let can_stop = self.claude.task_stop_capable;
        if !self.claude.task_infos.contains_key(&public) {
            self.claude.task_order.push_back(public.clone());
        }
        let entry = self
            .claude
            .task_infos
            .entry(public.clone())
            .or_insert_with(|| BackgroundTaskInfo {
                id: public.clone(),
                kind: kind.into(),
                title: crate::background::text(title, 512).0,
                status: BackgroundTaskStatus::Running,
                conversation_id: conversation_id.clone(),
                child_session_id: None,
                can_stop,
                can_read_output: false,
            });
        if record["subtype"] != "task_progress" && entry.status == BackgroundTaskStatus::Unknown {
            entry.status = BackgroundTaskStatus::Running;
            entry.can_stop = can_stop;
        }
        if record.get("description").is_some() {
            entry.title = crate::background::text(title, 512).0;
        }
        if conversation_id.is_some() {
            entry.conversation_id = conversation_id;
        }
        // A late progress event cannot resurrect a terminal task.
        Some(public)
    }

    pub fn background_tasks(&self) -> BackgroundTaskList {
        if self.config.kind != NativeKind::ClaudeCode {
            return BackgroundTaskList::default();
        }
        let live = !self.failed && !self.process_ended;
        let tasks = self
            .claude
            .task_infos
            .values()
            .cloned()
            .map(|mut task| {
                if !live
                    && matches!(
                        task.status,
                        BackgroundTaskStatus::Running | BackgroundTaskStatus::Stopping
                    )
                {
                    task.status = BackgroundTaskStatus::Unknown;
                    task.can_stop = false;
                }
                task
            })
            .collect();
        BackgroundTaskList {
            tasks,
            live,
            issue: self.claude.task_issue.clone(),
        }
    }
    pub fn read_background_task_output(
        &mut self,
        task: &str,
        request: Option<&str>,
    ) -> Result<BackgroundTaskOutput> {
        if let Some(request) = request {
            return self
                .claude
                .task_reads
                .read(task, request)
                .ok_or_else(|| Error::new("任务输出请求已失效，请重新读取"));
        }
        let (value, truncated) = self
            .claude
            .task_outputs
            .get(task)
            .ok_or_else(|| Error::new("运行时尚未提供此任务的输出"))?
            .clone();
        let output = self.claude.task_reads.begin(task);
        self.claude
            .task_reads
            .finish(&output.request_id, task, &value, truncated, None);
        self.claude
            .task_reads
            .read(task, &output.request_id)
            .ok_or_else(|| Error::new("任务输出请求已失效"))
    }
    pub fn stop_background_task(&mut self, task: &str) -> Result<()> {
        if !self
            .background_tasks()
            .tasks
            .iter()
            .any(|entry| entry.id == task && entry.can_stop)
        {
            return Err(Error::new("当前不能独立停止此后台任务"));
        }
        let native = self
            .claude
            .task_runs
            .iter()
            .find_map(|(native, public)| (public == task).then_some(native.clone()))
            .ok_or_else(|| Error::new("后台任务身份已改变"))?;
        self.claude.task_sequence = self.claude.task_sequence.wrapping_add(1);
        let request = format!("velune-background-stop-{}", self.claude.task_sequence);
        self.rpc.write(&json!({"type":"control_request","request_id":request,"request":{"subtype":"stop_task","task_id":native}}))?;
        self.claude
            .task_stops
            .insert(request, (task.into(), std::time::Instant::now()));
        if let Some(entry) = self.claude.task_infos.get_mut(task) {
            entry.status = BackgroundTaskStatus::Stopping;
            entry.can_stop = false;
        }
        Ok(())
    }
    pub(super) fn claude_task_run_matches(&self, record: &Value, native: &str) -> bool {
        record["run_id"].as_str().is_none_or(|run| {
            self.claude.task_runs.get(native).is_some_and(|current| {
                current == &format!("{}:{}:{native}{run}", self.epoch, native.len())
            })
        })
    }
    pub(super) fn claude_expire_background_controls(&mut self) {
        let before = self.claude.task_stops.len();
        self.claude
            .task_stops
            .retain(|_, (_, started)| started.elapsed() < std::time::Duration::from_secs(10));
        if before != self.claude.task_stops.len() {
            self.claude.task_issue =
                Some("运行时尚未确认停止请求，请刷新任务状态或在原生工具中核对。".into());
            // Keep Stopping and the process lease until a full live snapshot
            // or terminal event proves the job is no longer active.
        }
    }
    fn claude_prune_background_tasks(&mut self) {
        while self.claude.task_infos.len() > 128 {
            let removable = self.claude.task_order.iter().position(|id| {
                self.claude.task_infos.get(id).is_none_or(|task| {
                    !matches!(
                        task.status,
                        BackgroundTaskStatus::Running | BackgroundTaskStatus::Stopping
                    )
                })
            });
            let Some(index) = removable else { break };
            let id = self
                .claude
                .task_order
                .remove(index)
                .expect("known task cache index");
            self.claude.task_infos.remove(&id);
            self.claude.task_outputs.remove(&id);
            self.claude.task_runs.retain(|_, current| current != &id);
        }
    }
}
